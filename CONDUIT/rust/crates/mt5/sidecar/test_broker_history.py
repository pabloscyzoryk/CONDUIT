"""Production exporter tests. No terminal, credentials or broker connection."""
from collections import namedtuple
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import broker_history as history

Row = namedtuple("Row", "ticket order position_id time time_msc type entry volume price profit commission swap fee magic reason symbol comment external_id")


def row(ticket=1, **changes):
    values = dict(ticket=ticket, order=2, position_id=3, time=100, time_msc=100123,
                  type=0, entry=1, volume=0.01, price=1234.56, profit=-1.25,
                  commission=-0.03, swap=0.02, fee=-0.01, magic=999,
                  reason=3, symbol="OTHER_SYMBOL", comment="synthetic manual", external_id="native-external")
    values.update(changes)
    return Row(**values)


class FakeMT5:
    def __init__(self):
        self.identity = SimpleNamespace(login=123, server="SyntheticBroker", trade_mode=0)
        self.terminal = SimpleNamespace(path=str(Path("synthetic_terminal").absolute()))
        self.orders = []
        self.deals = []
        self.positions = []
        self.pending = []
        self.calls = []
        self.after_read = None

    def account_info(self):
        return self.identity

    def terminal_info(self):
        return self.terminal

    def last_error(self):
        return (1, "synthetic API returned None even though old success code remained")

    def get(self, name, values, args):
        self.calls.append((name, args))
        if self.after_read:
            self.after_read(name)
        return values

    def history_orders_get(self, *args):
        return self.get("history_orders_get", self.orders, args)

    def history_deals_get(self, *args):
        return self.get("history_deals_get", self.deals, args)

    def positions_get(self, *args):
        return self.get("positions_get", self.positions, args)

    def orders_get(self, *args):
        return self.get("orders_get", self.pending, args)


class ExportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.directory = Path(self.temp.name)
        self.api = FakeMT5()
        self.request = {"job_id": "a" * 32, "started_utc_ms": 200000,
                        "to_ms": 86400200000,
                        "source": {"account": history.scope(self.api.identity),
                                   "terminal_path": str(Path(self.api.terminal.path) / "terminal64.exe")}}

    def tearDown(self):
        self.temp.cleanup()

    def run_export(self):
        exporter = history.Exporter(self.api, self.directory, self.request, utc_ms=lambda: 200111)
        return exporter.run()

    def records(self, result):
        output = []
        for index in range(result["page_count"]):
            page = json.loads((self.directory / ("page_%08d.json" % index)).read_text("utf-8"))
            self.assertEqual(page["index"], index)
            self.assertEqual(page["sha256"], hashlib.sha256(page["records_json"].encode("utf-8")).hexdigest())
            records = json.loads(page["records_json"])
            self.assertEqual(len(records), page["count"])
            self.assertLessEqual(page["count"], history.PAGE_ROWS)
            self.assertLessEqual(len(page["records_json"].encode("utf-8")), history.PAGE_BYTES)
            output.extend(records)
        return output

    def test_full_account_preserves_all_fields_ids_balance_types_and_raw_times(self):
        self.api.orders = [row(2**63 + 7, magic=-123)]
        self.api.deals = [row(6, type=2, volume=0.0, position_id=0, symbol="", profit=300.0),
                          row(7, type=3, volume=0.0, profit=100.0), row(8, type=6, volume=0.0, fee=-5.0)]
        self.api.positions = [row(10)]
        self.api.pending = [row(11)]
        result = self.run_export()
        self.assertTrue(result["complete"])
        self.assertTrue(result["account_verified"])
        self.assertEqual(result["source"], result["source_after"])
        records = self.records(result)
        self.assertEqual(len(records), 6)
        raw = records[0]["raw"]
        self.assertEqual(raw["ticket"], str(2**63 + 7))
        self.assertEqual(raw["magic"], "-123")
        self.assertEqual(set(raw), set(Row._fields))
        self.assertEqual(raw["time_msc"], 100123)
        self.assertEqual(raw["time"], 100)
        self.assertEqual(raw["external_id"], "native-external")
        self.assertEqual(raw["fee"], -0.01)
        self.assertTrue(all(len(args) == (2 if name.startswith("history_") else 0) for name, args in self.api.calls))
        self.assertEqual(result["counts"], {"order": 1, "deal": 3, "position": 1, "pending": 1})

    def test_empty_success_is_complete_but_none_with_success_last_error_is_partial(self):
        self.assertTrue(self.run_export()["complete"])
        self.api.deals = None
        result = self.run_export()
        self.assertFalse(result["complete"])
        self.assertTrue(result["partial"])
        self.assertEqual(result["errors"][0]["code"], "history_deals_get_none")
        self.assertEqual(result["errors"][0]["mt5_error_code"], 1)

    def test_more_than_old_5000_limit_is_exported_without_slice(self):
        self.api.deals = [row(i) for i in range(6001)]
        result = self.run_export()
        self.assertTrue(result["complete"])
        self.assertEqual(result["counts"]["deal"], 6001)
        self.assertEqual(len(self.records(result)), 6001)

    def test_all_years_queried_including_empty_ranges_no_gaps_boundary_dedup(self):
        self.request["to_ms"] = 1788861600000  # fixed synthetic cutoff
        self.api.deals = [row(10)]  # repeated inclusive boundary receipt
        result = self.run_export()
        self.assertTrue(result["complete"])
        self.assertEqual(result["counts"]["deal"], 1)
        ranges = [args for name, args in self.api.calls if name == "history_deals_get"]
        self.assertEqual(ranges[0][0], 0)
        self.assertEqual(ranges[-1][1] * 1000, self.request["to_ms"])
        self.assertGreater(len(ranges), 50)
        self.assertTrue(all(a[1] == b[0] for a, b in zip(ranges, ranges[1:])))

    def test_account_switch_during_query_never_publishes_changed_account_rows(self):
        self.api.deals = [row(1)]
        self.api.after_read = lambda name: setattr(self.api.identity, "login", 124) if name == "history_deals_get" else None
        result = self.run_export()
        self.assertFalse(result["complete"])
        self.assertEqual(result["errors"][0]["code"], "account_changed")
        self.assertEqual(result["counts"]["deal"], 0)

    def test_terminal_switch_partial_and_snapshot_none_not_empty(self):
        self.api.positions = None
        result = self.run_export()
        self.assertEqual(result["errors"][0]["code"], "positions_get_none")
        self.api.positions = []
        self.api.after_read = lambda name: setattr(self.api.terminal, "path", "other-terminal")
        result = self.run_export()
        self.assertEqual(result["errors"][0]["code"], "terminal_changed")

    def test_resource_limits_are_explicit_partial_not_complete_truncation(self):
        self.api.deals = [row(1), row(2)]
        with patch.object(history, "MAX_NATIVE_ROWS", 1):
            result = self.run_export()
        self.assertEqual(result["errors"][0]["code"], "native_result_resource_limit")
        with patch.object(history, "MAX_BYTES", 1):
            result = self.run_export()
        self.assertFalse(result["complete"])
        self.assertEqual(result["errors"][0]["code"], "spool_byte_limit")

    def test_one_oversized_or_invalid_record_is_not_silently_dropped(self):
        self.api.deals = [row(1, comment="x" * history.PAGE_BYTES)]
        self.assertEqual(self.run_export()["errors"][0]["code"], "single_record_byte_limit")
        self.api.deals = [row(1, price=float("nan"))]
        self.assertEqual(self.run_export()["errors"][0]["code"], "serialization_or_worker_error")

    def test_ticket_conflicting_versions_make_result_partial(self):
        self.api.deals = [row(1), row(1, price=44.0)]
        self.assertEqual(self.run_export()["errors"][0]["code"], "native_ticket_changed_during_export")

    def test_deadline_reports_unread_range(self):
        clock = iter([0, history.TIMEOUT_S + 1])
        exporter = history.Exporter(self.api, self.directory, self.request, monotonic=lambda: next(clock))
        result = exporter.run()
        self.assertEqual(result["errors"][0]["code"], "worker_deadline")
        self.assertFalse(self.api.calls)

    def test_environment_does_not_forward_authentication_or_python_hooks(self):
        with patch.dict(history.os.environ, {"CONDUIT_MT5_PASSWORD": "synthetic", "TELEGRAM_TOKEN": "synthetic", "PYTHONPATH": "synthetic"}):
            env = history.safe_environment()
        self.assertNotIn("CONDUIT_MT5_PASSWORD", env)
        self.assertNotIn("TELEGRAM_TOKEN", env)
        self.assertNotIn("PYTHONPATH", env)

    def test_control_pages_are_job_bound_and_read_only_even_while_worker_busy(self):
        self.api.deals = [row(1)]
        state = self.run_export()
        jobs = history.HistoryJobs()
        class BusyProcess:
            def poll(self):
                return None
        jobs.job = {"id": self.request["job_id"], "directory": self.directory, "process": BusyProcess()}
        # Control methods cannot call fake MT5: it may be busy forever.
        with patch.object(self.api, "history_deals_get", side_effect=AssertionError("no RPC")):
            self.assertEqual(jobs.status(self.request["job_id"]), state)
            self.assertEqual(jobs.page(self.request["job_id"], 0)["index"], 0)
        for job_id, index in [("b" * 32, 0), (self.request["job_id"], -1), (self.request["job_id"], True), (self.request["job_id"], 999)]:
            with self.assertRaises(ValueError):
                jobs.page(job_id, index)

    def test_worker_attaches_explicit_path_without_login_or_mutations(self):
        calls = []
        self.api.initialize = lambda **kwargs: calls.append(("initialize", kwargs)) or True
        self.api.shutdown = lambda: calls.append(("shutdown", {}))
        history.atomic_json(self.directory / "request.json", self.request)
        with patch.dict(sys.modules, {"MetaTrader5": self.api}), patch.object(history, "running_path") as presence, patch.object(history.threading, "Thread"):
            history.worker_main(self.directory)
        presence.assert_called_once_with(self.request["source"]["terminal_path"])
        self.assertEqual(calls, [("initialize", {"path": self.request["source"]["terminal_path"], "timeout": 5000}), ("shutdown", {})])
        self.assertTrue(json.loads((self.directory / "status.json").read_text("utf-8"))["complete"])

    def test_absent_terminal_does_not_initialize_and_failure_does_not_leak_exception(self):
        self.api.initialize = lambda **kwargs: self.fail("must not initialize absent terminal")
        self.api.shutdown = lambda: None
        history.atomic_json(self.directory / "request.json", self.request)
        with patch.dict(sys.modules, {"MetaTrader5": self.api}), patch.object(history, "running_path", side_effect=history.HistoryFailure("explicit_terminal_not_running")), patch.object(history.threading, "Thread"):
            history.worker_main(self.directory)
        status = json.loads((self.directory / "status.json").read_text("utf-8"))
        self.assertTrue(status["partial"])
        self.assertEqual(status["errors"][0]["code"], "explicit_terminal_not_running")

    def test_lost_start_response_can_be_reaped_after_idle_ttl_but_not_during_read(self):
        jobs = history.HistoryJobs()
        class ExitedProcess:
            def poll(self):
                return 0
        old = {"id": "b" * 32, "directory": self.directory, "process": ExitedProcess(), "last_access": 100.0}
        jobs.job = old
        with patch.object(history.time, "monotonic", return_value=101.0):
            with self.assertRaisesRegex(ValueError, "already_active"):
                jobs.start(self.request["source"])
        # Avoid any subprocess. Prove the old slot is released BEFORE attempting
        # a new spool creation, which is deliberately stopped here.
        released = []
        def release(job_id):
            released.append(job_id)
            jobs.job = None
        with patch.object(history.time, "monotonic", return_value=100.0 + history.IDLE_REAP_S), patch.object(jobs, "release", side_effect=release), patch.object(history.tempfile, "mkdtemp", side_effect=RuntimeError("stop before spawn")):
            with self.assertRaisesRegex(RuntimeError, "stop before spawn"):
                jobs.start(self.request["source"])
        self.assertEqual(released, ["b" * 32])

    def test_actual_sidecar_endpoint_reads_spool_and_rejects_scope_change_or_filters(self):
        self.api.deals = [row(1)]
        self.run_export()
        with patch.dict(sys.modules, {"MetaTrader5": self.api}):
            spec = importlib.util.spec_from_file_location("history_sidecar_fixture", Path(__file__).with_name("mt5_sidecar.py"))
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
        sidecar = module.Sidecar(module.parse_args(["--port", "1"]))
        process = SimpleNamespace(poll=lambda: None)
        sidecar.history_jobs.job = {"id": self.request["job_id"], "directory": self.directory, "process": process}
        result = sidecar.cmd_broker_history({"op": "page", "job_id": self.request["job_id"], "index": 0})
        self.assertEqual(result["count"], 1)
        with self.assertRaisesRegex(module.BrokerError, "unexpected arguments"):
            sidecar.cmd_broker_history({"op": "start", "magic": 77})
        self.api.identity.login = 124
        with self.assertRaisesRegex(module.BrokerError, "account_changed"):
            sidecar.cmd_broker_history({"op": "status", "job_id": self.request["job_id"]})


if __name__ == "__main__":
    unittest.main()
