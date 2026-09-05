"""Read exact replay results only after receipt and input verification.

Supports full-window comparisons and the complete fresh-account coronation
matrix. Missing or interrupted cases stay explicitly incomplete, never zero.
"""
from __future__ import annotations
import argparse
import json
import math
from pathlib import Path

from research_metrics import compact, read_summary
from research_runner import file_hash


def collect(plan_path: Path, verify_inputs: bool = True) -> dict:
    plan = json.loads(plan_path.read_text('utf-8-sig'))
    plan_sha = file_hash(plan_path)
    if verify_inputs:
        for artifact in plan.get('inputs', []):
            if file_hash(Path(artifact['path'])) != artifact['sha256']:
                raise ValueError('Replay input changed after the plan was prepared')
    results, incomplete, hashes = [], [], {}
    for job in plan['jobs']:
        receipt_path = Path(plan['output']) / job['id'] / 'receipt.json'
        if not receipt_path.exists():
            incomplete.append({'job': job['id'], 'status': 'not_started'})
            continue
        receipt = json.loads(receipt_path.read_text('utf-8-sig'))
        if receipt.get('status') != 'complete':
            incomplete.append({'job': job['id'], 'status': receipt.get('status', 'unknown')})
            continue
        if (receipt.get('plan_sha256') != plan_sha or receipt.get('returncode') != 0
                or receipt.get('partial') is not False or receipt.get('validation_errors')
                or receipt.get('source_revision') != plan.get('source_revision')):
            raise ValueError('Completed receipt does not match the declared replay')
        exe = Path(job['argv'][0]).resolve()
        if str(exe) not in hashes:
            hashes[str(exe)] = file_hash(exe)
        if hashes[str(exe)] != receipt.get('exe_sha256'):
            raise ValueError('Executable identity changed')
        files = receipt.get('result_files', [])
        if len(files) != 1:
            raise ValueError('Expected exactly one final summary per job')
        path = Path(files[0]).resolve()
        if path.parent != Path(job['result_dir']).resolve():
            raise ValueError('Receipt points outside its declared result directory')
        if file_hash(path) != receipt.get('result_sha256', {}).get(path.name):
            raise ValueError('Result changed after completion')
        rows, metadata = read_summary(path)
        if metadata.get('approximate') or len(rows) != job['expected_candidates']:
            raise ValueError('Approximate or incomplete replay cannot enter an exact report')
        cap = job.get('lot_cap')
        if cap is None:
            cap = {'lot001': .01, 'lot10': 10., 'arithmetic': 0.}.get(job.get('cap_mode'))
        for name, metrics in rows.items():
            for key in ('start_balance', 'total_profit', 'end_equity'):
                if not isinstance(metrics.get(key), (int, float)) or not math.isfinite(metrics[key]):
                    raise ValueError('Invalid monetary result')
            if metrics['start_balance'] != job['deposit']:
                raise ValueError('Replay deposit differs from the requested account')
            row = compact(name, metrics)
            funnel = metrics.get('stat_sygnalow', {})
            # Older engine.baskets.len() was pruned. Full ledger aggregates
            # provide the cumulative count without editing the old summary.
            row['created_baskets'] = funnel.get('koszyki', {}).get('total')
            row['baskets_with_closed_trades'] = row.pop('filled_baskets')
            row['closed_signal_utilization_pct'] = row.pop('signal_utilization_pct')
            row['engine_reported_baskets'] = row.pop('baskets')
            row.update({key: metrics.get(key) for key in ('median_day', 'best_day',
                'max_open_volume', 'max_open_positions', 'max_open_margin', 'end_balance',
                'known_entry_sources', 'known_full_entry_sources', 'entry_sources_first_seen_as_edit',
                'entry_source_observation_version')})
            # Old summaries did not count first-known EDIT entries. Preserve
            # unknown as None rather than rendering a false zero or silently
            # treating the legacy NEW-only denominator as complete coverage.
            row['orphan_entry_rejection_events'] = metrics.get('odrzuty', {}).get('EditOrphan', 0)
            concentration = metrics.get('daily_concentration')
            if concentration and cap != .01 and any(concentration.get(key) is not None
                    for key in ('profit_without_best_1', 'profit_without_best_3', 'profit_without_best_5')):
                raise ValueError('Best-day subtraction is prohibited outside max lot 0.01')
            row['daily_concentration'] = concentration
            results.append({'job': job['id'], 'window': job.get('window'), 'lot_cap': cap,
                            'deposit': job['deposit'], 'metrics': row,
                            'receipt_sha256': file_hash(receipt_path), 'result_sha256': file_hash(path)})
    return {'schema': 'conduit.verified-replay-report.v1', 'plan_sha256': plan_sha,
            'all_cases_complete': not incomplete, 'expected_jobs': len(plan['jobs']),
            'completed_jobs': len(plan['jobs']) - len(incomplete), 'incomplete': incomplete,
            'results': results, 'production_choice_made': False,
            'notes': ['Best-day exclusion is permitted only at max lot 0.01.',
                      'Basket activity here requires at least one closed transaction; open-only baskets may be additional.',
                      'Input coverage, execution assumptions and chronological validation limitations remain those of the plan.']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    result = collect(args.plan)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps({key: result[key] for key in ('all_cases_complete', 'expected_jobs', 'completed_jobs')}))


if __name__ == '__main__':
    main()
