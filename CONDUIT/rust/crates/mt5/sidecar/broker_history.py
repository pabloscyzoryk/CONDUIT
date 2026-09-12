"""Private, read-only account history export, isolated from trading IPC.

The worker owns a separate MT5 IPC connection. It never receives credentials,
logs in, selects symbols or submits orders. MT5 still serves both connections:
isolation removes Python head-of-line blocking, not terminal load or all races.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import uuid

SCHEMA = "conduit.broker_history.v1"
PAGE_ROWS = 256
PAGE_BYTES = 256 * 1024
MAX_BYTES = 100 * 1024 * 1024
MAX_NATIVE_ROWS = 500_000
TIMEOUT_S = 90
IDLE_REAP_S = 180
ID_FIELDS = {"ticket", "order", "position_id", "position_by_id", "identifier", "magic"}


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), allow_nan=False)


def atomic_json(path, value):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(compact(value), encoding="utf-8")
    os.replace(temporary, path)


def scope(account):
    if account is None or not int(getattr(account, "login", 0)) or not getattr(account, "server", ""):
        raise HistoryFailure("account_unavailable")
    return {"login": str(int(account.login)), "server": str(account.server),
            "trade_mode": int(account.trade_mode)}


class HistoryFailure(Exception):
    def __init__(self, code, mt5_error_code=None):
        self.code = code
        self.mt5_error_code = mt5_error_code
        super().__init__(code)


def raw_record(row):
    # Namedtuple fields come from the installed MT5 package. No hand-maintained
    # field subset: retain fee, external_id, balance/credit types and future fields.
    data = dict(row._asdict())
    for key in ID_FIELDS.intersection(data):
        data[key] = str(int(data[key]))
    compact(data)  # Non-finite/unsupported values are explicit export failures.
    return data


def year_ranges(end_ms):
    """Inclusive UTC-calendar carriers for raw API seconds, with shared endpoints.

    No timezone adjustment of returned MT5 fields. The shared endpoint prevents
    subsecond gaps and rows are deduplicated by account-local native ticket.
    """
    from datetime import datetime, timezone
    start = 0
    year = 1971
    end_s = (end_ms + 999) // 1000
    while start < end_s:
        boundary = min(end_s, int(datetime(year, 1, 1, tzinfo=timezone.utc).timestamp()))
        yield start, boundary
        start = boundary
        year += 1


class Exporter:
    def __init__(self, api, directory, request, utc_ms=None, monotonic=None):
        self.api = api
        self.directory = Path(directory)
        self.request = request
        self.utc_ms = utc_ms or (lambda: time.time_ns() // 1_000_000)
        self.monotonic = monotonic or time.monotonic
        self.deadline = self.monotonic() + TIMEOUT_S
        self.used_bytes = 0
        self.page = []
        self.page_size = 2
        self.seen = {}
        self.stage = "start"
        self.current_range = None
        self.state = {
            "schema": SCHEMA, "job_id": request["job_id"], "state": "running",
            "complete": False, "partial": False,
            "counts": {kind: 0 for kind in ("order", "deal", "position", "pending")},
            "page_count": 0, "started_utc_ms": request["started_utc_ms"],
            "finished_utc_ms": None, "source": request["source"], "account_verified": False,
            "requested_bounds": {"from_ms": 0, "to_ms": None},
            "effective_bounds": {"from_ms": 0, "to_ms": request["to_ms"],
                                 "upper_bound_reason": "capture_utc_plus_one_day_coverage_cushion"},
            "time_semantics": {
                "raw": "unmodified_mt5_time_and_time_msc", "normalized_utc": None,
                "observation": "unix_utc_ms", "query": "epoch_seconds_no_timezone_shift",
                "atomic_snapshot": False,
                "coverage": "all_rows_supplied_by_terminal_for_successful_intervals",
                "limitations": ["broker_retention_unknown", "late_history_updates_possible",
                                "upper_bound_is_not_an_asof_cutoff", "terminal_load_not_realtime_guaranteed"],
            },
            "completed_ranges": [], "errors": [],
        }

    def publish(self):
        atomic_json(self.directory / "status.json", self.state)

    def check_scope(self):
        if self.monotonic() > self.deadline:
            raise HistoryFailure("worker_deadline")
        if scope(self.api.account_info()) != self.request["source"]["account"]:
            raise HistoryFailure("account_changed")
        terminal = self.api.terminal_info()
        if terminal is None:
            raise HistoryFailure("terminal_unavailable")
        expected = os.path.normcase(os.path.abspath(self.request["source"]["terminal_path"]))
        actual = os.path.normcase(os.path.abspath(os.path.join(terminal.path, "terminal64.exe")))
        if actual != expected:
            raise HistoryFailure("terminal_changed")

    def read(self, method, *args):
        self.check_scope()
        started = self.utc_ms()
        result = getattr(self.api, method)(*args)  # No symbol/group/magic filters.
        if result is None:
            error = self.api.last_error()
            raise HistoryFailure(method + "_none", int(error[0]) if error else None)
        self.check_scope()  # Never publish rows returned across an identity change.
        if len(result) > MAX_NATIVE_ROWS:
            # MT5 has no native row pagination. This protects serialization; it
            # cannot bound the native allocation already made by this API call.
            raise HistoryFailure("native_result_resource_limit")
        return result, started, self.utc_ms()

    def flush(self):
        if not self.page:
            return
        raw = compact(self.page)
        encoded = raw.encode("utf-8")
        if self.used_bytes + len(encoded) > MAX_BYTES:
            raise HistoryFailure("spool_byte_limit")
        index = self.state["page_count"]
        payload = {"job_id": self.state["job_id"], "index": index, "count": len(self.page),
                   "records_json": raw, "sha256": hashlib.sha256(encoded).hexdigest()}
        atomic_json(self.directory / ("page_%08d.json" % index), payload)
        self.used_bytes += len(encoded)
        for record in self.page:
            self.state["counts"][record["kind"]] += 1
        self.state["page_count"] += 1
        self.page = []
        self.page_size = 2

    def append(self, kind, row, observed):
        raw = raw_record(row)
        if "ticket" not in raw:
            raise HistoryFailure("missing_native_ticket")
        key = (kind, raw["ticket"])
        fingerprint = hashlib.sha256(compact(raw).encode("utf-8")).digest()
        if key in self.seen:
            if self.seen[key] != fingerprint:
                raise HistoryFailure("native_ticket_changed_during_export")
            return
        record = {"kind": kind, "observed_utc_ms": observed, "raw": raw}
        size = len(compact(record).encode("utf-8")) + 1
        if size + 2 > PAGE_BYTES:
            raise HistoryFailure("single_record_byte_limit")
        if len(self.page) >= PAGE_ROWS or self.page_size + size > PAGE_BYTES:
            self.flush()
        self.page.append(record)
        self.page_size += size
        self.seen[key] = fingerprint

    def run(self):
        self.publish()
        try:
            for lower, upper in year_ranges(self.request["to_ms"]):
                self.current_range = {"from_ms": lower * 1000, "to_ms": upper * 1000}
                for kind, method in (("order", "history_orders_get"), ("deal", "history_deals_get")):
                    self.stage = method
                    rows, before, after = self.read(method, lower, upper)
                    for row in rows:
                        self.append(kind, row, after)
                    self.flush()
                    self.state["completed_ranges"].append({
                        "kind": kind, **self.current_range, "observed_start_utc_ms": before,
                        "observed_end_utc_ms": after, "returned_count": len(rows),
                    })
                    self.publish()
            self.current_range = None
            # Whole-account current state is a separately timed, non-atomic read.
            for kind, method in (("position", "positions_get"), ("pending", "orders_get")):
                self.stage = method
                rows, before, after = self.read(method)
                for row in rows:
                    self.append(kind, row, after)
                self.flush()
                self.state[kind + "_snapshot"] = {"observed_start_utc_ms": before,
                                                  "observed_end_utc_ms": after, "count": len(rows)}
            self.check_scope()
            self.state["source_after"] = self.request["source"]
            self.state.update(state="complete", complete=True, account_verified=True)
        except Exception as error:
            code = error.code if isinstance(error, HistoryFailure) else "serialization_or_worker_error"
            self.state["errors"].append({"code": code, "stage": self.stage,
                                         "range": self.current_range,
                                         "mt5_error_code": getattr(error, "mt5_error_code", None)})
            self.state.update(state="partial", partial=True, complete=False)
        self.state["finished_utc_ms"] = self.utc_ms()
        self.publish()
        return self.state


def running_path(path):
    """A read-only presence check, not a guarantee against close/initialize races."""
    from terminal_discovery import TerminalDiscoveryError, running_terminal_path
    try:
        running_terminal_path(path)
    except TerminalDiscoveryError:
        # Worker status intentionally excludes local paths and account data.
        raise HistoryFailure("explicit_terminal_not_running") from None


def safe_environment():
    # No inherited CONDUIT_MT5_PASSWORD, Telegram/email/API keys, or Python hooks.
    allowed = {"systemroot", "windir", "path", "temp", "tmp", "localappdata", "appdata", "userprofile"}
    return {key: value for key, value in os.environ.items() if key.lower() in allowed}


def worker_main(directory):
    request = json.loads((directory / "request.json").read_text(encoding="utf-8"))
    api = None
    exporter = None
    try:
        import MetaTrader5 as api
        # Safety net independent of native API completion and client polling.
        def deadline_exit():
            time.sleep(TIMEOUT_S + 5)
            os._exit(2)
        threading.Thread(target=deadline_exit, daemon=True).start()
        exporter = Exporter(api, directory, request)
        exporter.publish()
        running_path(request["source"]["terminal_path"])
        # Explicit attach only; no login/password/server or saved profile lookup.
        # MT5 itself can race a terminal closing after the presence check.
        if not api.initialize(request["source"]["terminal_path"], timeout=5000):
            raise HistoryFailure("readonly_attach_failed")
        exporter.run()
    except Exception as error:
        if exporter is None:
            exporter = Exporter(None, directory, request)
        exporter.state.update(state="failed", complete=False, partial=True,
                              finished_utc_ms=time.time_ns() // 1_000_000)
        exporter.state["errors"].append({"code": getattr(error, "code", "worker_start_failed"),
                                         "stage": "attach"})
        exporter.publish()
    finally:
        if api is not None:
            api.shutdown()


class HistoryJobs:
    """Short control RPCs only. Background process performs every history call."""
    def __init__(self):
        self.job = None

    def start(self, source):
        if self.job is not None:
            # A lost start response must not permanently strand the only slot.
            # Do not reap a live worker or a recently read export.
            if (self.job["process"].poll() is not None
                    and time.monotonic() - self.job["last_access"] >= IDLE_REAP_S):
                self.release(self.job["id"])
            else:
                raise ValueError("broker_history_job_already_active")
        directory = Path(tempfile.mkdtemp(prefix="conduit_broker_history_"))
        now = time.time_ns() // 1_000_000
        request = {"job_id": uuid.uuid4().hex, "source": source, "started_utc_ms": now,
                   "to_ms": ((now + 86_400_000 + 999) // 1000) * 1000}
        atomic_json(directory / "request.json", request)
        Exporter(None, directory, request).publish()
        try:
            process = subprocess.Popen([sys.executable, "-I", str(Path(__file__).resolve()), str(directory)],
                                       env=safe_environment(), stdin=subprocess.DEVNULL,
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                       creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        except Exception:
            shutil.rmtree(directory)
            raise ValueError("broker_history_worker_spawn_failed") from None
        self.job = {"id": request["job_id"], "directory": directory, "process": process,
                    "last_access": time.monotonic()}
        # Enforce the wall deadline even when a native history call never returns
        # and nobody polls status. No MT5 calls on this guardian thread.
        def guard():
            try:
                process.wait(timeout=TIMEOUT_S + 10)
            except subprocess.TimeoutExpired:
                process.kill()
        threading.Thread(target=guard, daemon=True).start()
        return self.status(request["job_id"])

    def require(self, job_id):
        if self.job is None or job_id != self.job["id"]:
            raise ValueError("broker_history_unknown_job")
        self.job["last_access"] = time.monotonic()
        return self.job

    def status(self, job_id):
        job = self.require(job_id)
        state = json.loads((job["directory"] / "status.json").read_text(encoding="utf-8"))
        if job["process"].poll() is not None and state["state"] == "running":
            state.update(state="partial", complete=False, partial=True,
                         finished_utc_ms=time.time_ns() // 1_000_000)
            state["errors"].append({"code": "worker_terminated_or_deadline", "stage": "worker"})
        return state

    def page(self, job_id, index):
        job = self.require(job_id)
        if type(index) is not int or index < 0 or index >= self.status(job_id)["page_count"]:
            raise ValueError("broker_history_page_unavailable")
        return json.loads((job["directory"] / ("page_%08d.json" % index)).read_text(encoding="utf-8"))

    def release(self, job_id):
        job = self.require(job_id)
        self.job = None
        def cleanup():
            process = job["process"]
            if process.poll() is None:
                process.kill()
            process.wait()
            shutil.rmtree(job["directory"], ignore_errors=True)
        threading.Thread(target=cleanup, daemon=True).start()
        return {"released": True, "job_id": job_id}

    def close(self):
        if self.job is not None:
            self.release(self.job["id"])


if __name__ == "__main__":
    worker_main(Path(sys.argv[1]))
