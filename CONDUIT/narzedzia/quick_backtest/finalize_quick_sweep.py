"""Finalize a time-boxed quick sweep without making it coronation eligible."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import time
from typing import Any

from run_quick_sweep import ranked


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_POOL = ROOT / "work/quick_sweep/manifest.json"


def load(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8-sig"))


def atomic_json(path: Path, value: Any) -> None:
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(
        json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False) + "\n",
        encoding="utf-8",
    )
    tmp.replace(path)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def preset_name(path: Path) -> str:
    doc = load(path)
    name = doc.get("name") or doc.get("nazwa")
    if not isinstance(name, str) or not name:
        raise RuntimeError(f"preset has no name: {path}")
    return name


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run_dir", type=Path)
    parser.add_argument("--pool-manifest", type=Path, default=DEFAULT_POOL)
    parser.add_argument("--stride", type=int, default=20)
    parser.add_argument("--started-at", help="ISO-local start written to the manifest")
    parser.add_argument("--process-exit", default="normal")
    parser.add_argument("--from", dest="date_from", required=True)
    parser.add_argument("--to", dest="date_to", required=True)
    args = parser.parse_args()

    run_dir = args.run_dir.resolve()
    pool_manifest = args.pool_manifest.resolve()
    configs_dir = run_dir / "configs"
    results_dir = run_dir / "results"
    final_path = results_dir / f"wyniki_APPROX_N{args.stride}_compound.json"
    partial_path = results_dir / "wyniki_czastkowe.json"
    source_path = final_path if final_path.is_file() else partial_path
    if not source_path.is_file():
        raise SystemExit(f"no quick results to finalize: {results_dir}")
    wrapper = load(source_path)
    if (
        wrapper.get("approximate") is not True
        or wrapper.get("coronation_eligible") is not False
        or int(wrapper.get("quick_tick_stride", 0)) != args.stride
        or not isinstance(wrapper.get("results"), dict)
    ):
        raise SystemExit(f"unsafe quick wrapper: {source_path}")

    results: dict[str, dict[str, Any]] = wrapper["results"]
    config_paths = sorted(configs_dir.glob("*.json"))
    requested = {preset_name(path): path for path in config_paths}
    unexpected = sorted(set(results) - set(requested))
    if unexpected:
        raise SystemExit(f"results contain unexpected presets: {unexpected[:3]}")
    order = ranked(results)
    pool = load(pool_manifest)
    pool_by_name = {row["name"]: row for row in pool["queue"]}

    top_rows = []
    for rank, name in enumerate(order[:24], 1):
        row = pool_by_name.get(name, {})
        top_rows.append(
            {
                "approx_rank": rank,
                "name": name,
                "family": row.get("family"),
                "file": requested[name].name,
                "queue_index": row.get("queue_index"),
                "metrics": results[name],
            }
        )

    completed = set(results)
    remaining = [
        {
            "name": name,
            "file": path.name,
            "family": pool_by_name.get(name, {}).get("family"),
            "queue_index": pool_by_name.get(name, {}).get("queue_index"),
        }
        for name, path in requested.items()
        if name not in completed
    ]
    complete = len(completed) == len(requested)
    status = "COMPLETE_APPROX_ONLY" if complete else "TIMEBOXED_PARTIAL_APPROX_ONLY"
    warning = (
        "APPROXIMATE SCREENING ONLY — sampled results cannot crown a release preset; "
        "rerun selected candidates with exact N=1."
    )
    now = time.time()
    common = {
        "schema": "conduit.quick-sweep1-timebox.v1",
        "status": status,
        "approximate": True,
        "coronation_eligible": False,
        "automatic_coronation": False,
        "quick_tick_stride": args.stride,
        "full_window": [args.date_from, args.date_to],
        "max_lot": 5.0,
        "requested_count": len(requested),
        "completed_count": len(completed),
        "remaining_count": len(remaining),
        "process_exit": args.process_exit,
        "started_at_local": args.started_at,
        "finished_at_unix": now,
        "pool_manifest": str(pool_manifest),
        "pool_manifest_sha256": sha256(pool_manifest),
        "source_results": str(source_path),
        "source_results_sha256": sha256(source_path),
        "warning": warning,
    }
    atomic_json(
        run_dir / "top24_approx_selection.json",
        {
            **common,
            "schema": "conduit.quick-sweep1-top24-screening.v1",
            "selected_count": len(top_rows),
            "rows": top_rows,
        },
    )
    atomic_json(
        run_dir / "checkpoint.json",
        {
            **common,
            "schema": "conduit.quick-sweep1-timebox-checkpoint.v1",
            "resume": {
                "supported": bool(remaining),
                "remaining": remaining,
                "completed_names": sorted(completed),
            },
            "next_required_stage": "exact N=1 validation of top24",
        },
    )
    atomic_json(run_dir / "run_manifest.json", common)
    print(json.dumps({"status": status, "completed": len(completed),
                      "top24": [row["name"] for row in top_rows]}, indent=2))


if __name__ == "__main__":
    main()
