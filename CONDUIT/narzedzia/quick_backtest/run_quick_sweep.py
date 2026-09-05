"""Checkpointed quick-backtest tournament. Default action is a read-only dry-run."""
from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_POOL = ROOT / "work/quick_sweep/manifest.json"
DEFAULT_RUN = ROOT / "work/quick_sweep/run"
DEFAULT_BT = ROOT / "rust/target/release/btp.exe"
DEFAULT_TICKS = ROOT / "work/input/ticks.bin"
DEFAULT_SIGNALS = ROOT / "work/input/signals.json"
AUTHORIZATION = "START_QUICK_SWEEP1"
CALIBRATION_STRIDES = (1, 10, 12, 20)
APPROX_BUDGET_S = 27 * 60  # three minutes of headroom under the 30 min target


def load(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8-sig"))


def atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(value, ensure_ascii=False, indent=2,
                              allow_nan=False) + "\n", encoding="utf-8")
    tmp.replace(path)


def file_sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def score(m: dict[str, Any]) -> float:
    if m.get("blown"):
        return -math.inf
    risk_pct = float(m.get("max_open_risk_pct", 0.0))
    profit = float(m.get("total_profit", 0.0))
    if risk_pct > 50.0:
        return -1e6 + profit / max(risk_pct, 1.0)
    capital = max(float(m.get("start_balance", 0.0)), 1.0)
    if profit <= 0:
        return profit / capital - 1000.0
    margin = min(1.0, max(0.05, float(m.get("min_equity", 0.0)) / capital))
    typical = max((capital + max(float(m.get("end_equity", capital)), capital)) * 0.5,
                  capital)
    real_loss = abs(min(float(m.get("worst_day", 0.0)), 0.0)) / typical
    ml = float(m.get("min_margin_level", 0.0))
    ml_margin = 1.0 if not math.isfinite(ml) or ml <= 0 else min(1.0, max(0.0, (ml - 20) / 480))
    danger_ticks = float(m.get("ml_pod_150", 0.0))
    danger_time = 1.0 if danger_ticks <= 0 else 0.5 ** (danger_ticks / 1000.0)
    consistency = 0.5 + float(m.get("win_days_pct", 0.0)) / 200.0
    return (profit / capital) * margin * ml_margin * danger_time * consistency / (1 + 3 * real_loss)


def ranked(results: dict[str, dict[str, Any]]) -> list[str]:
    return sorted(results, key=lambda name: (-score(results[name]), name))


def average_ranks(results: dict[str, dict[str, Any]]) -> dict[str, float]:
    order = ranked(results)
    out: dict[str, float] = {}
    i = 0
    while i < len(order):
        value = score(results[order[i]])
        j = i + 1
        while j < len(order) and score(results[order[j]]) == value:
            j += 1
        rank = ((i + 1) + j) / 2.0
        for name in order[i:j]:
            out[name] = rank
        i = j
    return out


def spearman(exact: dict[str, dict[str, Any]], quick: dict[str, dict[str, Any]]) -> float:
    names = sorted(set(exact) & set(quick))
    a, b = average_ranks(exact), average_ranks(quick)
    xs, ys = [a[n] for n in names], [b[n] for n in names]
    mx, my = sum(xs) / len(xs), sum(ys) / len(ys)
    num = sum((x - mx) * (y - my) for x, y in zip(xs, ys))
    dx = sum((x - mx) ** 2 for x in xs)
    dy = sum((y - my) ** 2 for y in ys)
    return num / math.sqrt(dx * dy) if dx and dy else 0.0


def read_results(out: Path, stride: int) -> dict[str, dict[str, Any]]:
    path = out / ("wyniki_compound.json" if stride == 1
                  else f"wyniki_APPROX_N{stride}_compound.json")
    doc = load(path)
    if stride == 1:
        if isinstance(doc, dict) and doc.get("approximate") is True:
            raise RuntimeError(f"N=1 unexpectedly labelled approximate: {path}")
        return doc
    if (doc.get("approximate") is not True or
            doc.get("coronation_eligible") is not False or
            int(doc.get("quick_tick_stride", 0)) != stride):
        raise RuntimeError(f"unsafe/malformed quick result wrapper: {path}")
    return doc["results"]


def read_partial(path: Path) -> dict[str, dict[str, Any]]:
    doc = load(path)
    if isinstance(doc, dict) and doc.get("approximate") is True:
        results = doc.get("results")
        return results if isinstance(results, dict) else {}
    return doc if isinstance(doc, dict) else {}


def common_args(args: argparse.Namespace) -> list[str]:
    return [str(args.bt), "--ticks", str(args.ticks), "--signals", str(args.signals),
            "--from", args.date_from, "--to", args.date_to,
            "--balance", str(args.balance), "--rozgrzewka-h", "72",
            "--sim-limit-price-improvement", "--sim-new-pending-sl-next-tick",
            "--sim-native-swap-cash-digits", "2", "--summary-only", "--no-charts"]


def run_sweep(args: argparse.Namespace, label: str, configs: Path, out: Path,
              stride: int, checkpoint: dict[str, Any]) -> tuple[dict[str, dict[str, Any]], float]:
    expected = out / ("wyniki_compound.json" if stride == 1
                      else f"wyniki_APPROX_N{stride}_compound.json")
    interrupted = out / "PRZERWANE.txt"
    if expected.is_file() and not interrupted.is_file():
        return read_results(out, stride), float(checkpoint.get("stages", {}).get(label, {}).get("seconds", 0))
    out.mkdir(parents=True, exist_ok=True)
    partial_path = out / "wyniki_czastkowe.json"
    completed = read_partial(partial_path) if partial_path.is_file() else {}
    source_configs = sorted(configs.glob("*.json"))
    pending_configs = []
    for path in source_configs:
        doc = load(path)
        name = doc.get("name") or doc.get("nazwa")
        if name not in completed:
            pending_configs.append(path)
    run_configs = configs
    if completed and pending_configs and len(pending_configs) < len(source_configs):
        run_configs = out / "resume_pending_configs"
        run_configs.mkdir(exist_ok=True)
        expected_files = {path.name for path in pending_configs}
        extra = {path.name for path in run_configs.glob("*.json")} - expected_files
        if extra:
            raise RuntimeError(f"unexpected stale resume configs in {run_configs}: {sorted(extra)[:3]}")
        for path in pending_configs:
            link_or_copy(path, run_configs / path.name)
    elif completed and not pending_configs:
        # A process may die after publishing its final partial row but before
        # writing the final aggregate. Promote the complete checkpoint safely.
        if stride == 1:
            atomic_json(expected, completed)
        else:
            atomic_json(expected, {"schema": "conduit.quick-sweep-results.v1",
                "approximate": True, "coronation_eligible": False,
                "quick_tick_stride": stride,
                "warning": "APPROXIMATE SCREENING ONLY — rerun finalists with N=1",
                "results": completed})
        interrupted.unlink(missing_ok=True)
        return completed, float(checkpoint.get("stages", {}).get(label, {}).get("seconds", 0))
    interrupted.unlink(missing_ok=True)
    cmd = common_args(args) + ["--sweep", str(run_configs), "--quick-tick-stride", str(stride),
                               "--top", "24", "--out", str(out)]
    log = out / "btp.log"
    env = os.environ.copy()
    env["CONDUIT_POSTEP_DIR"] = str(args.run_dir / "monitor")
    started = time.monotonic()
    with log.open("w", encoding="utf-8") as stream:
        process = subprocess.Popen(cmd, cwd=ROOT / "PROJEKT/rust", stdout=stream,
                                   stderr=subprocess.STDOUT, env=env)
        checkpoint.setdefault("stages", {})[label] = {
            "status": "RUNNING", "pid": process.pid, "stride": stride,
            "count": len(source_configs), "pending_count": len(pending_configs),
            "resumed_completed_count": len(completed), "out": str(out), "command": cmd,
            "approximate": stride > 1, "coronation_eligible": stride == 1,
        }
        atomic_json(args.run_dir / "checkpoint.json", checkpoint)
        code = process.wait()
    seconds = time.monotonic() - started
    if code:
        checkpoint["stages"][label].update(status="FAILED", exit_code=code, seconds=seconds)
        atomic_json(args.run_dir / "checkpoint.json", checkpoint)
        raise RuntimeError(f"{label} failed with exit code {code}; see {log}")
    fresh = read_results(out, stride)
    results = {**completed, **fresh}
    if completed:
        if stride == 1:
            atomic_json(expected, results)
            atomic_json(partial_path, results)
        else:
            wrapper = load(expected)
            wrapper["results"] = results
            wrapper["resumed_from_partial_count"] = len(completed)
            atomic_json(expected, wrapper)
            atomic_json(partial_path, wrapper)
    checkpoint["stages"][label].update(
        status="COMPLETE", exit_code=0, seconds=seconds, result_count=len(results),
        result_sha256=file_sha(expected), finished_at=time.time())
    atomic_json(args.run_dir / "checkpoint.json", checkpoint)
    return results, seconds


def link_or_copy(source: Path, target: Path) -> None:
    if target.exists():
        return
    try:
        os.link(source, target)
    except OSError:
        shutil.copy2(source, target)


def materialize(rows: list[dict[str, Any]], source_dir: Path, target: Path) -> None:
    target.mkdir(parents=True, exist_ok=True)
    expected = {row["file"] for row in rows}
    present = {p.name for p in target.glob("*.json")}
    if present - expected:
        raise RuntimeError(f"unexpected configs in {target}: {sorted(present - expected)[:3]}")
    for row in rows:
        source = source_dir / row["file"]
        if file_sha(source) != row["payload_sha256"]:
            raise RuntimeError(f"candidate hash mismatch: {source}")
        link_or_copy(source, target / row["file"])


def stratified_pool(queue: list[dict[str, Any]], count: int) -> list[dict[str, Any]]:
    by: dict[str, list[dict[str, Any]]] = {}
    for row in queue:
        by.setdefault(row["family"], []).append(row)
    selected: list[dict[str, Any]] = []
    remaining = count
    families = sorted(by)
    for index, family in enumerate(families):
        target = remaining if index == len(families) - 1 else round(count * len(by[family]) / len(queue))
        target = min(target, len(by[family]))
        source = by[family]
        indices = sorted({round(i * (len(source) - 1) / max(target - 1, 1)) for i in range(target)})
        selected.extend(source[i] for i in indices)
        remaining -= len(indices)
    if len(selected) < count:
        have = {r["name"] for r in selected}
        selected.extend(
            [r for r in queue if r["name"] not in have][:count - len(selected)]
        )
    return sorted(selected[:count], key=lambda r: r["queue_index"])


def select_survivors(rows: list[dict[str, Any]], results: dict[str, dict[str, Any]],
                     keep: int) -> list[dict[str, Any]]:
    available = [r for r in rows if r["name"] in results]
    by_name = {r["name"]: r for r in available}
    global_order = [n for n in ranked(results) if n in by_name]
    chosen: list[str] = []
    # Reserve 75% proportionally by family; final 25% is global merit.
    quota_total = math.floor(keep * 0.75)
    for family in sorted({r["family"] for r in available}):
        family_names = [r["name"] for r in available if r["family"] == family]
        quota = round(quota_total * len(family_names) / len(available))
        family_set = set(family_names)
        chosen.extend([n for n in global_order if n in family_set][:quota])
    for name in global_order:
        if name not in chosen:
            chosen.append(name)
        if len(chosen) >= keep:
            break
    return [by_name[n] for n in chosen[:keep]]


def calibration_report(exact: dict[str, dict[str, Any]], quick_by_n: dict[int, dict[str, dict[str, Any]]],
                       walls: dict[int, float]) -> dict[str, Any]:
    exact_order = ranked(exact)
    report: dict[str, Any] = {"count": len(exact), "rows": []}
    for n in sorted(quick_by_n):
        quick_order = ranked(quick_by_n[n])
        top12_in_24 = len(set(exact_order[:12]) & set(quick_order[:24])) / 12
        top24_in_48 = len(set(exact_order[:24]) & set(quick_order[:48])) / 24
        report["rows"].append({"stride": n, "seconds": walls[n],
            "speedup_vs_n1": walls[1] / walls[n], "spearman": spearman(exact, quick_by_n[n]),
            "exact_top12_recall_in_quick_top24": top12_in_24,
            "exact_top24_recall_in_quick_top48": top24_in_48,
            "exact_top1_rank": quick_order.index(exact_order[0]) + 1,
            "quick_top1": quick_order[0]})
    viable = [row for row in report["rows"]
              if row["spearman"] >= 0.25
              and row["exact_top12_recall_in_quick_top24"] >= 0.83
              and row["exact_top24_recall_in_quick_top48"] >= 0.83]
    report["selected_stride"] = max((row["stride"] for row in viable), default=10)
    report["quality_gate"] = "Spearman>=0.25 and top12->24/top24->48 recall>=0.83"
    return report


def exact_job(args: argparse.Namespace, row: dict[str, Any], rank: int) -> dict[str, Any]:
    out = args.run_dir / "exact_top24" / f"{rank:02d}-{row['name']}"
    expected = out / "wyniki_compound.json"
    if expected.is_file():
        result = read_results(out, 1)
        return {"name": row["name"], "rank_in": rank, "pid": None,
                "seconds": 0.0, "result": result[row["name"]], "resumed": True}
    out.mkdir(parents=True, exist_ok=True)
    cmd = common_args(args) + ["--preset", str(args.config_dir / row["file"]),
                               "--quick-tick-stride", "1", "--top", "1", "--out", str(out)]
    env = os.environ.copy()
    env["RAYON_NUM_THREADS"] = "1"
    env["CONDUIT_POSTEP_DIR"] = str(args.run_dir / "monitor")
    started = time.monotonic()
    with (out / "btp.log").open("w", encoding="utf-8") as stream:
        p = subprocess.Popen(cmd, cwd=ROOT / "PROJEKT/rust", stdout=stream,
                             stderr=subprocess.STDOUT, env=env)
        code = p.wait()
    if code:
        raise RuntimeError(f"exact job {row['name']} failed: {code}")
    result = read_results(out, 1)
    return {"name": row["name"], "rank_in": rank, "pid": p.pid,
            "seconds": time.monotonic() - started, "result": result[row["name"]], "resumed": False}


def execute(args: argparse.Namespace, manifest: dict[str, Any]) -> None:
    args.run_dir.mkdir(parents=True, exist_ok=True)
    (args.run_dir / "monitor").mkdir(exist_ok=True)
    # A previous failed attempt is preserved in checkpoint/history; this
    # sentinel describes only the currently active attempt.
    (args.run_dir / "FAILED.json").unlink(missing_ok=True)
    checkpoint_path = args.run_dir / "checkpoint.json"
    checkpoint = load(checkpoint_path) if checkpoint_path.is_file() else {
        "schema": "conduit.quick-sweep1-checkpoint.v1", "status": "RUNNING",
        "started_at": time.time(), "pool_manifest": str(args.manifest),
        "pool_manifest_sha256": file_sha(args.manifest), "stages": {},
        "automatic_coronation": False,
    }
    if checkpoint["pool_manifest_sha256"] != file_sha(args.manifest):
        raise RuntimeError("resume refused: pool manifest changed")
    atomic_json(checkpoint_path, checkpoint)

    pilot_dir = Path(manifest["pilot"]["config_dir"])
    calibration: dict[int, dict[str, dict[str, Any]]] = {}
    walls: dict[int, float] = {}
    for n in CALIBRATION_STRIDES:
        calibration[n], walls[n] = run_sweep(
            args, f"calibration_n{n}", pilot_dir,
            args.run_dir / "calibration_full96" / f"n{n}", n, checkpoint)
    report = calibration_report(calibration[1], {n: calibration[n] for n in CALIBRATION_STRIDES if n > 1}, walls)
    atomic_json(args.run_dir / "calibration_full96" / "quality_report.json", report)
    stride1 = int(report["selected_stride"])
    stride2 = 10 if stride1 > 10 else 5
    per96_1 = walls[stride1]
    per96_2 = walls.get(stride2, walls[1] / 1.54)

    # Largest meaningful initial pool fitting the measured 27-minute envelope:
    # stage1 all -> stage2 best 25% -> exact N=1 top24 (outside approx budget).
    initial_count = 96
    for candidate in range(96, len(manifest["queue"]) + 1, 24):
        keep = max(48, math.ceil(candidate * 0.25 / 24) * 24)
        estimate = per96_1 * candidate / 96 + per96_2 * keep / 96
        if estimate <= APPROX_BUDGET_S:
            initial_count = candidate
        else:
            break
    keep1 = max(48, math.ceil(initial_count * 0.25 / 24) * 24)
    estimate = per96_1 * initial_count / 96 + per96_2 * keep1 / 96
    plan = {"selected_stride_stage1": stride1, "selected_stride_stage2": stride2,
            "initial_count": initial_count, "stage2_count": keep1, "exact_count": 24,
            "approx_estimate_seconds": estimate, "budget_seconds": APPROX_BUDGET_S,
            "full_10000_stage1_estimate_seconds": per96_1 * 10_000 / 96,
            "why_not_forced_10000": "measured budget and ranking-quality gate",
            "final_exact_processes": 24, "rayon_threads_per_exact_process": 1}
    atomic_json(args.run_dir / "tournament_plan.json", plan)

    initial_rows = stratified_pool(manifest["queue"], initial_count)
    stage1_dir = args.run_dir / "stage1" / "configs"
    materialize(initial_rows, args.config_dir, stage1_dir)
    stage1_results, _ = run_sweep(args, "stage1", stage1_dir,
                                  args.run_dir / "stage1" / "results", stride1, checkpoint)
    stage2_rows = select_survivors(initial_rows, stage1_results, keep1)
    stage2_dir = args.run_dir / "stage2" / "configs"
    materialize(stage2_rows, args.config_dir, stage2_dir)
    stage2_results, _ = run_sweep(args, "stage2", stage2_dir,
                                  args.run_dir / "stage2" / "results", stride2, checkpoint)
    top24 = select_survivors(stage2_rows, stage2_results, 24)
    atomic_json(args.run_dir / "top24_approx_selection.json", {
        "schema": "conduit.quick-sweep1-top24-screening.v1", "approximate": True,
        "coronation_eligible": False, "source_stride": stride2,
        "warning": "Selection only; exact N=1 jobs below decide final ranking.",
        "rows": [{k: row[k] for k in ("name", "family", "file", "queue_index")} for row in top24]})

    checkpoint["exact_top24"] = {"status": "RUNNING", "processes": 24,
                                  "threads_each": 1, "started_at": time.time()}
    atomic_json(checkpoint_path, checkpoint)
    with concurrent.futures.ThreadPoolExecutor(max_workers=24) as pool:
        futures = [pool.submit(exact_job, args, row, rank)
                   for rank, row in enumerate(top24, 1)]
        exact = [future.result() for future in futures]
    merged = {job["name"]: job["result"] for job in sorted(exact, key=lambda j: j["rank_in"])}
    exact_order = ranked(merged)
    atomic_json(args.run_dir / "exact_top24" / "wyniki_exact_merged.json", merged)
    atomic_json(args.run_dir / "exact_top24" / "manifest.json", {
        "schema": "conduit.quick-sweep1-exact-finalists.v1", "approximate": False,
        "coronation_eligible": True, "stride": 1, "jobs": exact,
        "deterministic_exact_ranking": exact_order,
        "automatic_coronation": False,
        "warning": "Exact ranking is evidence, not automatic coronation."})
    checkpoint["exact_top24"].update(status="COMPLETE", finished_at=time.time(),
                                      ranking=exact_order)
    checkpoint["status"] = "COMPLETE_EXACT_TOP24"
    checkpoint["finished_at"] = time.time()
    atomic_json(checkpoint_path, checkpoint)


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--manifest", type=Path, default=DEFAULT_POOL)
    p.add_argument("--run-dir", type=Path, default=DEFAULT_RUN)
    p.add_argument("--bt", type=Path, default=DEFAULT_BT)
    p.add_argument("--ticks", type=Path, default=DEFAULT_TICKS)
    p.add_argument("--signals", type=Path, default=DEFAULT_SIGNALS)
    p.add_argument("--from", dest="date_from", required=True)
    p.add_argument("--to", dest="date_to", required=True)
    p.add_argument("--balance", type=float, default=600.0)
    p.add_argument("--execute", action="store_true")
    p.add_argument("--authorization", default="")
    return p.parse_args()


def main() -> None:
    args = parse_args()
    args.manifest, args.run_dir = args.manifest.resolve(), args.run_dir.resolve()
    args.bt, args.ticks, args.signals = args.bt.resolve(), args.ticks.resolve(), args.signals.resolve()
    for path in (args.manifest, args.bt, args.ticks, args.signals):
        if not path.is_file():
            raise SystemExit(f"missing input: {path}")
    manifest = load(args.manifest)
    args.config_dir = args.manifest.parent / "configs"
    if (manifest.get("count") != 10_000 or manifest.get("lot_max") != 5.0 or
            len(manifest.get("queue", [])) != 10_000):
        raise SystemExit("pool manifest is not the audited 10,000-row cap-5 queue")
    if not args.execute:
        print(json.dumps({"status": "DRY_RUN_OK", "pool_count": 10_000,
                          "pilot_count": manifest["pilot"]["count"],
                          "calibration_strides": CALIBRATION_STRIDES,
                          "full_window": [args.date_from, args.date_to],
                          "approx_budget_seconds": APPROX_BUDGET_S,
                          "will_auto_reduce_pool_if_needed": True,
                          "will_run_exact_top24_as": "24 processes x RAYON_NUM_THREADS=1",
                          "monitor_dir": str(args.run_dir / "monitor"),
                          "execute_requires": AUTHORIZATION}, indent=2))
        return
    if args.authorization != AUTHORIZATION:
        raise SystemExit(f"--execute requires --authorization {AUTHORIZATION}")
    try:
        execute(args, manifest)
    except Exception as exc:
        if args.run_dir.exists():
            atomic_json(args.run_dir / "FAILED.json", {"error": str(exc), "time": time.time()})
        raise


if __name__ == "__main__":
    main()
