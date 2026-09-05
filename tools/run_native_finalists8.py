"""Run an explicit queue of frozen, offline MT5/Rust comparison cases.

The plan pins the engines, terminal/editor and adapter sources by SHA-256. Candidate presets
must already contain the intended native broker cost profile. This runner only
changes the requested lot ceiling and initial deposit. It never selects GOD-X8.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time

ARTIFACTS = ("experiment", "portable", "contract", "mapping", "compare", "btp", "most", "defaults", "native_source", "ticks", "messages", "trade_sessions", "terminal", "metaeditor")
RECHECK_ARTIFACTS = tuple(name for name in ARTIFACTS if name not in {"ticks", "messages"})
SANDBOX_BINARIES = {"terminal": "terminal64.exe", "metaeditor": "MetaEditor64.exe"}
SAFE_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,79}\Z")
WINDOWS_DEVICES = {"con", "prn", "aux", "nul", *(f"com{i}" for i in range(1, 10)), *(f"lpt{i}" for i in range(1, 10))}
QUEUE_FILES = ("queue.lock", "frozen_plan_private.json", "runner_provenance.json", "queue_status.json")


def safe_id(value) -> bool:
    return (isinstance(value, str) and SAFE_ID.fullmatch(value) is not None
            and not value.endswith(".") and value.split(".", 1)[0].casefold() not in WINDOWS_DEVICES)


def verify_artifacts(plan: dict) -> None:
    # Large tick/message inputs are verified by the frozen adapter's final
    # manifest instead of hashing the whole history before every case.
    for name in RECHECK_ARTIFACTS:
        record = plan["artifacts"][name]
        if fingerprint(Path(record["path"]))["sha256"] != record["sha256"]:
            raise ValueError("Frozen artifact changed while the queue was running: " + name)


def fingerprint(path: Path) -> dict:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return {"sha256": digest.hexdigest(), "bytes": path.stat().st_size}


def write_json(path: Path, value) -> None:
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, ensure_ascii=True), encoding="utf-8")
    temporary.replace(path)


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def aware_time(value: str) -> datetime:
    result = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if result.tzinfo is None:
        raise ValueError("Queue deadlines must contain an explicit timezone.")
    return result.astimezone(timezone.utc)


def resolve_input(value: str, base: Path) -> Path:
    path = Path(value)
    return (path if path.is_absolute() else base / path).resolve(strict=True)


def validate_plan(plan: dict, base: Path) -> dict:
    if plan.get("schema") != "conduit.native.queue.v1":
        raise ValueError("Unsupported native queue schema.")
    normalized = json.loads(json.dumps(plan))
    for name in ARTIFACTS:
        record = normalized.get("artifacts", {}).get(name)
        if not isinstance(record, dict) or not isinstance(record.get("path"), str) or not isinstance(record.get("sha256"), str):
            raise ValueError("Missing frozen artifact path/SHA-256: " + name)
        path = resolve_input(record["path"], base)
        expected = record["sha256"]
        if not re.fullmatch(r"[0-9a-f]{64}", expected) or fingerprint(path)["sha256"] != expected:
            raise ValueError("Frozen artifact does not match its expected SHA-256: " + name)
        record["path"] = str(path)
    adapter_directory = Path(normalized["artifacts"]["experiment"]["path"]).parent
    for name in ("portable", "contract", "mapping", "compare"):
        if Path(normalized["artifacts"][name]["path"]) != adapter_directory / (name + ".py"):
            raise ValueError("Pinned adapter dependencies must be the actual sibling modules imported by experiment.py.")
    context = normalized["context"]
    for name in ("sandbox", "common_files"):
        context[name] = str(resolve_input(context[name], base))
    for name, filename in SANDBOX_BINARIES.items():
        actual = (Path(context["sandbox"]) / filename).resolve(strict=True)
        if Path(normalized["artifacts"][name]["path"]) != actual:
            raise ValueError("Pinned terminal/editor must be the exact executable used by the offline sandbox: " + name)
    marker = json.loads((Path(context["sandbox"]) / "conduit_tester_sandbox.json").read_text())
    if marker.get("purpose") != "offline_strategy_tester" or marker.get("live_trading") is not False:
        raise ValueError("Queue requires an explicitly provisioned offline sandbox.")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+", context["symbol"]):
        raise ValueError("Unsafe symbol.")
    ids = set()
    output_names = {name.casefold() for name in QUEUE_FILES}
    output_names.update(name.casefold() + ".tmp" for name in QUEUE_FILES)
    earlier_cases = {}
    if not normalized.get("cases"):
        raise ValueError("Queue must name at least one explicit case.")
    for case in normalized["cases"]:
        if not safe_id(case["id"]):
            raise ValueError("Case IDs must be unique safe Windows directory names.")
        names = {case["id"].casefold(), (case["id"] + "_receipt.json").casefold(),
                 (case["id"] + "_receipt.json.tmp").casefold()}
        if names & output_names:
            raise ValueError("Case IDs must be unique without Windows case or queue-output collisions.")
        output_names.update(names)
        if not safe_id(case["candidate_id"]):
            raise ValueError("Candidate ID must be a safe identifier.")
        for key in ("deposit", "cap"):
            value = case[key]
            if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
                raise ValueError("Case deposit and cap must be finite numeric values.")
        if case["deposit"] <= 0 or case["cap"] < 0:
            raise ValueError("Case deposit must be positive and cap nonnegative.")
        if any(not re.fullmatch(r"\d{4}-\d{2}-\d{2}", case[key]) for key in ("from", "to")):
            raise ValueError("Native windows require complete ISO dates without time components.")
        start = datetime.fromisoformat(case["from"]).date()
        end = datetime.fromisoformat(case["to"]).date()
        if start >= end:
            raise ValueError("Case window must have an exclusive end after its start.")
        if any(dependency not in ids for dependency in case.get("requires", [])):
            raise ValueError("Prerequisite cases must appear earlier in the queue.")
        if case["cap"] == 0 or case["cap"] > 0.01:
            predecessors = [earlier_cases[key] for key in case.get("requires", [])]
            if not any(previous["candidate_id"] == case["candidate_id"]
                       and previous["deposit"] == case["deposit"] and previous["cap"] > 0
                       and previous["preset"]["sha256"] == case["preset"]["sha256"]
                       and previous["from"] == case["from"] and previous["to"] == case["to"]
                       and (case["cap"] == 0 or previous["cap"] < case["cap"])
                       for previous in predecessors):
                raise ValueError("Higher-volume cases require a passing lower-cap case with the same candidate, preset SHA, deposit and exact window.")
        preset = resolve_input(case["preset"]["path"], base)
        if fingerprint(preset)["sha256"] != case["preset"]["sha256"]:
            raise ValueError("Candidate preset changed after selection: " + case["id"])
        value = json.loads(preset.read_text(encoding="utf-8-sig"))
        if not isinstance(value.get("settings"), dict):
            raise ValueError("Candidate preset must contain a settings object.")
        case["preset"]["path"] = str(preset)
        ids.add(case["id"])
        earlier_cases[case["id"]] = case
    return normalized


def command_for(plan: dict, case: dict, preset: Path, output: Path, timeout: int) -> list[str]:
    artifacts, context = plan["artifacts"], plan["context"]
    command = [sys.executable, artifacts["experiment"]["path"]]
    for name in ("btp", "most", "defaults", "ticks", "messages", "trade_sessions", "native_source"):
        command.extend(["--" + name.replace("_", "-"), artifacts[name]["path"]])
    for name in ("sandbox", "common_files", "symbol"):
        command.extend(["--" + name.replace("_", "-"), str(context[name])])
    command.extend(["--out", str(output), "--preset", str(preset), "--from", case["from"], "--to", case["to"],
                    "--deposit", str(case["deposit"]), "--leverage", str(context.get("leverage", 500)),
                    "--price-digits", str(context.get("price_digits", 2)), "--native-swap-cash-digits",
                    str(context.get("swap_cash_digits", 2)), "--channel", context.get("channel", "Synergy"),
                    "--timeout", str(timeout)])
    for name in ("limit_price_improvement", "live_telegram_ingress", "new_pending_sl_next_tick"):
        if context.get(name, True):
            command.append("--" + name.replace("_", "-"))
    if context.get("tester_port") is not None:
        command.extend(["--tester-port", str(context["tester_port"])])
    return command


def verify_experiment(plan: dict, case: dict, directory: Path, requested: dict) -> dict:
    """Qualify the trusted frozen adapter's final input receipt before PASS/FAIL."""
    manifest = json.loads((directory / "experiment.json").read_text(encoding="utf-8-sig"))
    if manifest.get("schema") != "conduit.mt5.experiment.v1":
        raise ValueError("Missing or unsupported final experiment manifest.")
    for name in ("btp", "most", "defaults", "ticks", "messages", "trade_sessions"):
        if manifest.get(name, {}).get("sha256") != plan["artifacts"][name]["sha256"]:
            raise ValueError("Final experiment input differs from the frozen queue: " + name)
    if manifest.get("native_expert", {}).get("source", {}).get("sha256") != plan["artifacts"]["native_source"]["sha256"]:
        raise ValueError("Compiled native source differs from the frozen queue.")
    for key, expected in {"from": case["from"], "to_exclusive": case["to"],
                          "deposit": case["deposit"], "symbol": plan["context"]["symbol"]}.items():
        if manifest.get(key) != expected:
            raise ValueError("Final experiment scope differs from the requested case: " + key)
    effective = directory / "effective_preset.json"
    if manifest.get("settings") != fingerprint(effective)["sha256"]:
        raise ValueError("Final effective preset fingerprint is missing or inconsistent.")
    defaults = json.loads(Path(plan["artifacts"]["defaults"]["path"]).read_text(encoding="utf-8-sig"))
    expected = {**requested, "settings": {**defaults, **requested["settings"]}}
    if json.loads(effective.read_text(encoding="utf-8-sig")) != expected:
        raise ValueError("Adapter changed more than the requested effective preset/default merge.")
    verify_artifacts(plan)
    return manifest


def run_case(plan: dict, case: dict, output: Path, timeout: int) -> dict:
    output.mkdir()
    # Verify executable/source identity again immediately before the next case.
    verify_artifacts(plan)
    raw = Path(case["preset"]["path"]).read_bytes()
    if hashlib.sha256(raw).hexdigest() != case["preset"]["sha256"]:
        raise ValueError("Candidate preset changed while the queue was running.")
    preset = json.loads(raw.decode("utf-8-sig"))
    preset["settings"]["lot_max"] = case["cap"]
    effective = output / "requested_preset.json"
    write_json(effective, preset)
    command = command_for(plan, case, effective, output / "experiment", timeout)
    write_json(output / "recipe_private.json", {"command": command, "case": case,
                                                "effective_preset": fingerprint(effective)})
    started = time.monotonic()
    timed_out = False
    with (output / "stdout.log").open("wb") as stdout, (output / "stderr.log").open("wb") as stderr:
        launch = ({"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.CREATE_NO_WINDOW}
                  if os.name == "nt" else {"start_new_session": True})
        process = subprocess.Popen(command, stdout=stdout, stderr=stderr, **launch)
        write_json(output / "owned_process.json", {"pid": process.pid, "started_utc": utc_now(),
                                                   "scope": "This queue's private experiment process and its descendants only."})
        try:
            process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            # Bound the entire adapter/native/Rust process tree. Never select
            # processes by a generic executable name or touch another terminal.
            if process.poll() is None:
                if os.name == "nt":
                    subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False,
                                   creationflags=subprocess.CREATE_NO_WINDOW)
                else:
                    os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=30)
    elapsed = time.monotonic() - started
    result_path = output / "experiment/comparison.json"
    comparison = json.loads(result_path.read_text()) if result_path.exists() else None
    qualified = None
    if not timed_out and process.returncode == 0 and comparison is not None:
        qualified = verify_experiment(plan, case, output / "experiment", preset)
        if qualified.get("comparison", {}).get("ledger_and_equity_match") is not comparison.get("ledger_and_equity_match"):
            raise ValueError("Comparison differs from the final experiment receipt.")
    passed = qualified is not None and comparison.get("ledger_and_equity_match") is True
    status = "PASS" if passed else "FAIL" if qualified is not None and comparison.get("ledger_and_equity_match") is False else "ERROR"
    return {"status": status, "finished_utc": utc_now(), "elapsed_seconds": elapsed,
            "returncode": process.returncode, "timed_out": timed_out, "comparison": comparison,
            "final_inputs_verified": qualified is not None,
            "sandbox_binaries": {name: plan["artifacts"][name]["sha256"] for name in SANDBOX_BINARIES},
            "scope": "Native closed execution fields, account and observed sessions; detailed limits remain in comparison."}


@contextmanager
def queue_lock(output: Path):
    lock = output / "queue.lock"
    with lock.open("x") as stream:
        stream.write(utc_now())
    try:
        yield
    finally:
        lock.unlink(missing_ok=True)


def run_queue(plan: dict, output: Path, cutoff: datetime, case_timeout: int,
              estimated_case_seconds: int, reserve_seconds: int, continue_after_failure: bool = False) -> dict:
    if output.exists():
        raise ValueError("Use a fresh queue output directory; previous evidence is immutable.")
    output.mkdir(parents=True)
    write_json(output / "frozen_plan_private.json", plan)
    write_json(output / "runner_provenance.json", fingerprint(Path(__file__)))
    status = {"schema": "conduit.native.queue-status.v1", "started_utc": utc_now(),
              "status": "RUNNING", "active_case": None,
              "cases": [{"id": c["id"], "candidate_id": c["candidate_id"], "status": "PENDING"} for c in plan["cases"]]}
    results = {}
    with queue_lock(output):
        for case, row in zip(plan["cases"], status["cases"]):
            remaining = (cutoff - datetime.now(timezone.utc)).total_seconds()
            if remaining < estimated_case_seconds + reserve_seconds:
                row.update(status="DEFERRED_DEADLINE", reason="Insufficient reserved time before queue cutoff.")
                continue
            if any(results.get(key) != "PASS" for key in case.get("requires", [])):
                row.update(status="DEFERRED_PREREQUISITE", reason="An earlier required native case did not pass.")
                continue
            row.update(status="RUNNING", started_utc=utc_now())
            status["active_case"] = case["id"]
            write_json(output / "queue_status.json", status)
            print(json.dumps({"case": case["id"], "status": "RUNNING"}), flush=True)
            try:
                receipt = run_case(plan, case, output / case["id"], min(case_timeout, max(1, int(remaining-reserve_seconds))))
            except Exception as error:
                receipt = {"status": "ERROR", "finished_utc": utc_now(), "error_type": type(error).__name__,
                           "reason": "Inspect private case artifacts; queue never retries unknown native execution automatically."}
            row.update({key: value for key, value in receipt.items() if key != "comparison"})
            results[case["id"]] = row["status"]
            write_json(output / (case["id"] + "_receipt.json"), {"case": case, **receipt})
            status["active_case"] = None
            write_json(output / "queue_status.json", status)
            print(json.dumps({"case": case["id"], "status": row["status"], "elapsed_seconds": receipt.get("elapsed_seconds")}), flush=True)
            if row["status"] == "ERROR" or (row["status"] != "PASS" and not continue_after_failure):
                for later in status["cases"]:
                    if later["status"] == "PENDING":
                        later.update(status="DEFERRED_FAILURE", reason="Earlier native execution requires diagnosis.")
                break
        status.update(status="COMPLETE" if all(x["status"] == "PASS" for x in status["cases"]) else "INCOMPLETE",
                      finished_utc=utc_now(), active_case=None)
        write_json(output / "queue_status.json", status)
    return status


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--finish-before", required=True, help="ISO time with timezone; reserve prevents late starts.")
    parser.add_argument("--case-timeout", type=int, default=2700)
    parser.add_argument("--estimated-case-seconds", type=int, default=2100)
    parser.add_argument("--reserve-seconds", type=int, default=300)
    parser.add_argument("--continue-after-failure", action="store_true")
    parser.add_argument("--validate-only", action="store_true")
    args = parser.parse_args()
    if args.case_timeout <= 0 or args.estimated_case_seconds <= 0 or args.reserve_seconds < 0:
        raise ValueError("Queue timing values must be positive with nonnegative reserve.")
    plan = validate_plan(json.loads(args.plan.read_text(encoding="utf-8-sig")), args.plan.resolve().parent)
    cutoff = aware_time(args.finish_before)
    if args.validate_only:
        print(json.dumps({"valid": True, "cases": len(plan["cases"]), "artifacts": len(ARTIFACTS)}))
        return
    result = run_queue(plan, args.out.resolve(), cutoff, args.case_timeout,
                       args.estimated_case_seconds, args.reserve_seconds, args.continue_after_failure)
    raise SystemExit(0 if result["status"] == "COMPLETE" else 2)


if __name__ == "__main__":
    main()
