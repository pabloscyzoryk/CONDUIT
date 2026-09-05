"""Run reproducible backtest jobs with a shared CPU budget and visible progress.

Job plans and results belong outside the public repository. Commands are argv
arrays (never shell strings). Every completed job records its executable hash,
parameters, return code and elapsed time; incomplete runs remain unrankable.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import math
import os
from pathlib import Path
import re
import subprocess
import time


def write_json(path: Path, payload: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    partial = path.with_suffix(path.suffix + '.tmp')
    partial.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding='utf-8')
    # Windows readers (including PowerShell and some preview tools) can briefly
    # open an existing file without FILE_SHARE_DELETE. Keep the previous whole
    # document visible and retry the atomic replacement, never truncate it.
    for attempt in range(24):
        try:
            os.replace(partial, path)
            return
        except PermissionError:
            if attempt == 23:
                raise
            time.sleep(min(.025 * (attempt+1), .25))


def write_progress(path: Path, payload: dict) -> None:
    """A locked monitor file must never orphan the research child processes."""
    try:
        write_json(path, payload)
    except OSError as exc:
        print(json.dumps({'warning': 'progress_write_failed', 'path': str(path),
                          'error': str(exc)}), flush=True)


def file_hash(path: Path) -> str:
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def validate_results(out: Path, expected_count: int | None = None) -> tuple[list[Path], list[str]]:
    """An exit code alone does not prove that every requested preset ran."""
    files = [p for p in out.glob('wyniki_*.json') if p.name != 'wyniki_czastkowe.json']
    errors = []
    if not files:
        errors.append('No final result file')
    for path in files:
        try:
            doc = json.loads(path.read_text('utf-8-sig'))
            rows = doc['results'] if doc.get('approximate') is True else doc
            if not isinstance(rows, dict) or not rows:
                raise ValueError('Empty or invalid result mapping')
            if expected_count is not None and len(rows) != expected_count:
                raise ValueError(f'Expected {expected_count} candidates, got {len(rows)}')
            for name, metrics in rows.items():
                if not isinstance(metrics, dict):
                    raise ValueError(f'Invalid metrics for {name}')
                for key in ('total_profit', 'end_equity', 'start_balance'):
                    value = metrics.get(key)
                    if not isinstance(value, (int, float)) or not math.isfinite(value):
                        raise ValueError(f'Invalid {key} for {name}')
        except (ValueError, KeyError, TypeError, OSError) as exc:
            errors.append(f'{path.name}: {exc}')
    return files, errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('plan', type=Path)
    args = parser.parse_args()
    plan_path = args.plan.resolve()
    plan = json.loads(plan_path.read_text('utf-8-sig'))
    for artifact in plan.get('inputs', []):
        if file_hash(Path(artifact['path'])) != artifact['sha256']:
            raise ValueError(f"Input hash changed: {artifact['path']}")
    root = Path(plan['output']).resolve()
    progress_dir = Path(plan['progress_dir']).resolve()
    root.mkdir(parents=True, exist_ok=True)
    progress_dir.mkdir(parents=True, exist_ok=True)
    study_id = plan['id']
    if not re.fullmatch(r'[A-Za-z0-9_-]+', study_id):
        raise ValueError('Study id must be a safe filename')
    budget = int(plan.get('threads', 24))
    jobs = list(plan['jobs'])
    ids = [job['id'] for job in jobs]
    if len(set(ids)) != len(ids) or any(not re.fullmatch(r'[A-Za-z0-9_-]+', i) for i in ids):
        raise ValueError('Job ids must be unique safe filenames')
    for job in jobs:
        if not 1 <= int(job.get('threads', 1)) <= budget:
            raise ValueError('Invalid per-job thread allocation')
    deadline = dt.datetime.fromisoformat(plan['stop_at']).timestamp() if plan.get('stop_at') else float('inf')
    started_ms = int(time.time()*1000)
    completed: list[dict] = []
    active: dict[str, tuple] = {}
    hashes: dict[str, str] = {}
    stopped = False
    plan_hash = file_hash(plan_path)
    state_file = root / 'runner_state.json'
    if state_file.exists():
        old = json.loads(state_file.read_text('utf-8'))
        if old.get('plan_sha256') != plan_hash:
            raise ValueError('Plan changed: use a new output directory')
        completed = [r for r in old.get('completed', []) if r.get('status') == 'complete']
    done_ids = {r['id'] for r in completed}
    pending = [j for j in jobs if j['id'] not in done_ids]
    def ask_children_to_stop() -> None:
        for _, process, _, _, _ in active.values():
            for state in progress_dir.glob(f'backtest-{process.pid}-*.json'):
                state.with_suffix('.stop').write_text('stop', encoding='ascii')
    while pending or active:
        now = time.time()
        if now >= deadline or (progress_dir / f'{study_id}.stop').exists():
            stopped = True
            ask_children_to_stop()
        used = sum(int(j.get('threads', 1)) for j, *_ in active.values())
        while pending and not stopped:
            job = pending[0]
            allocation = int(job.get('threads', 1))
            if used + allocation > budget:
                break
            pending.pop(0)
            job_dir = root / job['id']
            job_dir.mkdir(exist_ok=True)
            receipt_path = job_dir/'receipt.json'
            if receipt_path.exists():
                previous = json.loads(receipt_path.read_text('utf-8'))
                if previous.get('status') == 'running':
                    raise RuntimeError(f"Unreconciled running receipt for {job['id']}; recover the existing process before relaunching")
            argv = list(job['argv'])
            executable = str(Path(argv[0]).resolve())
            argv[0] = executable
            env = os.environ.copy()
            env.update({str(k): str(v) for k, v in job.get('env', {}).items()})
            env['RAYON_NUM_THREADS'] = str(allocation)
            env['CONDUIT_POSTEP_DIR'] = str(progress_dir)
            env.pop('CONDUIT_BEZ_OKNA', None)
            log = (job_dir/'process.log').open('w', encoding='utf-8')
            launched = time.time()
            receipt = {'id': job['id'], 'argv': argv,
                       'source_revision': plan.get('source_revision', 'unspecified'),
                       'threads': allocation, 'started_at': launched,
                       'status': 'running', 'plan_sha256': plan_hash}
            try:
                if executable not in hashes:
                    hashes[executable] = file_hash(Path(executable))
                receipt['exe_sha256'] = hashes[executable]
                process = subprocess.Popen(argv, cwd=job.get('cwd'), env=env,
                                           stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
                                           creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
            except OSError as exc:
                log.close()
                receipt.update(status='failed', error=str(exc), elapsed_s=round(time.time()-launched, 3))
                write_json(job_dir/'receipt.json', receipt)
                completed.append(receipt)
                print(json.dumps({'job': job['id'], 'status': 'failed', 'error': str(exc)}), flush=True)
                continue
            receipt['pid'] = process.pid
            write_json(job_dir/'receipt.json', receipt)
            active[job['id']] = (job, process, log, launched, receipt)
            used += allocation
        for identity, (job, process, log, launched, receipt) in list(active.items()):
            code = process.poll()
            if code is None:
                continue
            log.close()
            out = Path(job['result_dir'])
            final_results, validation_errors = validate_results(out, job.get('expected_candidates'))
            partial = (out/'PRZERWANE.txt').exists()
            status = 'complete' if code == 0 and not validation_errors and not partial and not stopped else 'failed'
            receipt.update(returncode=code, status=status, elapsed_s=round(time.time()-launched, 3),
                           result_files=[str(p) for p in final_results], partial=partial,
                           validation_errors=validation_errors,
                           result_sha256={p.name: file_hash(p) for p in final_results})
            write_json(root/identity/'receipt.json', receipt)
            completed.append(receipt)
            del active[identity]
            print(json.dumps({'job': identity, 'status': status, 'elapsed_s': receipt['elapsed_s']}), flush=True)
        successful = sum(r['status'] == 'complete' for r in completed)
        failed = sum(r['status'] != 'complete' for r in completed)
        state = {'schema': 'conduit.research-runner.v1', 'id': study_id,
                 'plan_sha256': plan_hash, 'stopped': stopped,
                 'completed': completed, 'active': list(active),
                 'pending': [j['id'] for j in pending], 'updated_at': time.time()}
        write_json(state_file, state)
        stats = {**plan.get('metadata', {}), 'watki': budget,
                 'zaplanowane': len(jobs), 'ukonczone': successful, 'nieudane': failed,
                 'aktywne_procesy': len(active),
                 'watki_przydzielone': sum(int(j.get('threads', 1)) for j, *_ in active.values())}
        elapsed = time.time() - started_ms/1000
        finished = len(completed)
        eta = elapsed/finished*(len(jobs)-finished) if finished else -1
        write_progress(progress_dir/f'{study_id}.json', {
            'id': study_id, 'nazwa': plan.get('name', study_id), 'rodzaj': 'backtest',
            'postep': finished/max(len(jobs),1), 'szybkosc': finished/max(elapsed,1),
            'jednostka_szybkosci': 'przebiegów/s', 'eta_s': eta,
            'co_teraz': ' | '.join(active) if active else ('Zatrzymano' if stopped else 'Zakończono'),
            'start_ts': started_ms, 'aktualizacja_ts': int(time.time()*1000),
            'zrobione': finished, 'calosc': len(jobs), 'jednostka': 'przebiegów',
            'przerywanie': stopped, 'statystyki': stats, 'katalog_wynikow': str(root),
        })
        if stopped and not active:
            break
        if active or pending:
            time.sleep(0.5)
    # Let the monitor retain a receipt instead of a stale, apparently crashed job.
    final_progress = progress_dir/f'{study_id}.json'
    if final_progress.exists():
        write_json(root/'completed_progress.json', json.loads(final_progress.read_text('utf-8')))
        try:
            final_progress.unlink()
        except OSError as exc:
            print(json.dumps({'warning': 'completed_progress_cleanup', 'error': str(exc)}), flush=True)
    return 1 if stopped or any(r['status'] != 'complete' for r in completed) else 0


if __name__ == '__main__':
    raise SystemExit(main())
