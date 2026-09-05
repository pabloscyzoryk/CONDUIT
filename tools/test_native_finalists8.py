"""Offline-only queue qualification; no MT5/Python adapter process is launched."""
from __future__ import annotations

from contextlib import redirect_stdout
from datetime import datetime, timedelta, timezone
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import run_native_finalists8 as queue


class NativeQueueTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        sandbox = self.root / "sandbox"
        sandbox.mkdir()
        (sandbox / "conduit_tester_sandbox.json").write_text(json.dumps({
            "purpose": "offline_strategy_tester", "live_trading": False}))
        common = self.root / "common"
        common.mkdir()
        artifacts = {}
        for name in queue.ARTIFACTS:
            path = sandbox / queue.SANDBOX_BINARIES[name] if name in queue.SANDBOX_BINARIES else self.root / (name + ".py")
            path.write_bytes(b"MZ-synthetic-never-execute" if name in queue.SANDBOX_BINARIES else name.encode())
            if name == "defaults":
                path.write_text(json.dumps({"lot_max": 5, "default_axis": 7}))
            artifacts[name] = {"path": str(path), "sha256": queue.fingerprint(path)["sha256"]}
        self.preset = self.root / "candidate.json"
        self.preset.write_text(json.dumps({"name": "Synthetic", "settings": {"lot_max": 5, "axis": 11}}))
        self.plan = {"schema": "conduit.native.queue.v1", "artifacts": artifacts,
                     "context": {"sandbox": str(sandbox), "common_files": str(common), "symbol": "SYNTHETIC"},
                     "cases": [self.case("small", .01)]}

    def case(self, name, cap, **changes):
        return {"id": name, "candidate_id": "Synthetic", "preset": {"path": str(self.preset),
                "sha256": queue.fingerprint(self.preset)["sha256"]}, "from": "2026-06-01", "to": "2026-09-05",
                "deposit": 600, "cap": cap, **changes}

    def validated(self):
        return queue.validate_plan(self.plan, self.root)

    def add_large(self, cap=10, **changes):
        self.plan["cases"].append(self.case("large", cap, requires=["small"], **changes))

    def write_experiment(self, command, passed=True):
        option = lambda key: command[command.index(key) + 1]
        out = Path(option("--out"))
        out.mkdir()
        requested = json.loads(Path(option("--preset")).read_text())
        defaults = json.loads(Path(option("--defaults")).read_text())
        effective = {**requested, "settings": {**defaults, **requested["settings"]}}
        queue.write_json(out / "effective_preset.json", effective)
        result = {"ledger_and_equity_match": passed}
        queue.write_json(out / "comparison.json", result)
        manifest = {"schema": "conduit.mt5.experiment.v1", "from": option("--from"),
                    "to_exclusive": option("--to"), "symbol": option("--symbol"),
                    "deposit": float(option("--deposit")), "comparison": result,
                    "settings": queue.fingerprint(out / "effective_preset.json")["sha256"],
                    "native_expert": {"source": queue.fingerprint(Path(option("--native-source")))}}
        for name in ("btp", "most", "defaults", "ticks", "messages", "trade_sessions"):
            manifest[name] = queue.fingerprint(Path(option("--" + name.replace("_", "-"))))
        queue.write_json(out / "experiment.json", manifest)
        return out

    def fake_popen(self, passed=True, after_write=None, returncode=0):
        owner = self
        class FakeProcess:
            pid = 987654
            def __init__(self, command, **kwargs):
                owner.last_command = command
                owner.launch_options = kwargs
                out = owner.write_experiment(command, passed)
                if after_write:
                    after_write(out)
                self.returncode = returncode
            def wait(self, timeout):
                return self.returncode
        return FakeProcess

    def run_fake_case(self, **kwargs):
        plan = self.validated()
        with patch.object(queue.subprocess, "Popen", self.fake_popen(**kwargs)):
            return queue.run_case(plan, plan["cases"][0], self.root / "run", 25)

    def edit_manifest(self, out, mutation):
        path = out / "experiment.json"
        value = json.loads(path.read_text())
        mutation(value)
        queue.write_json(path, value)

    def test_valid_lower_higher_unlimited_chain_is_exact(self):
        self.add_large()
        self.plan["cases"].append(self.case("unlimited", 0, requires=["large"]))
        result = self.validated()
        self.assertEqual([x["cap"] for x in result["cases"]], [.01, 10, 0])
        self.assertEqual(self.plan["cases"][0]["preset"]["sha256"], result["cases"][2]["preset"]["sha256"])

    def test_higher_and_unlimited_cannot_skip_prerequisite(self):
        for cap in [10, 0]:
            with self.subTest(cap=cap):
                self.plan["cases"] = [self.case("large", cap)]
                with self.assertRaisesRegex(ValueError, "lower-cap"):
                    self.validated()

    def test_same_alias_different_preset_does_not_qualify(self):
        other = self.root / "other.json"
        other.write_text('{"settings":{"axis":99}}')
        self.add_large(preset={"path": str(other), "sha256": queue.fingerprint(other)["sha256"]})
        with self.assertRaisesRegex(ValueError, "preset SHA"):
            self.validated()

    def test_shorter_or_different_window_does_not_qualify(self):
        for key, value in [("from", "2026-09-01"), ("to", "2026-06-02")]:
            with self.subTest(key=key):
                self.plan["cases"] = [self.case("small", .01, **{key: value}),
                                      self.case("large", 10, requires=["small"])]
                with self.assertRaisesRegex(ValueError, "exact window"):
                    self.validated()

    def test_different_deposit_or_candidate_does_not_qualify(self):
        for changes in [{"deposit": 300}, {"candidate_id": "Other"}]:
            with self.subTest(changes=changes):
                self.plan["cases"] = [self.case("small", .01), self.case("large", 10, requires=["small"], **changes)]
                with self.assertRaisesRegex(ValueError, "lower-cap"):
                    self.validated()

    def test_forward_or_nonexistent_prerequisite_is_rejected(self):
        self.plan["cases"][0]["requires"] = ["later"]
        with self.assertRaisesRegex(ValueError, "earlier"):
            self.validated()

    def test_equal_cap_or_unlimited_predecessor_does_not_qualify(self):
        self.add_large()
        self.plan["cases"].append(self.case("same", 10, requires=["large"]))
        with self.assertRaisesRegex(ValueError, "lower-cap"):
            self.validated()
        self.plan["cases"][-1] = self.case("zero", 0, requires=["large"])
        self.plan["cases"].append(self.case("zero-again", 0, requires=["zero"]))
        with self.assertRaisesRegex(ValueError, "lower-cap"):
            self.validated()

    def test_casefold_and_receipt_names_cannot_collide_in_either_order(self):
        for names in [("Case", "case"), ("a", "a_receipt.json"), ("a_receipt.json", "A"),
                      ("a", "a_receipt.json.tmp")]:
            with self.subTest(names=names):
                self.plan["cases"] = [self.case(n, .01) for n in names]
                with self.assertRaisesRegex(ValueError, "collisions"):
                    self.validated()

    def test_windows_devices_trailing_dot_and_queue_files_are_rejected(self):
        for name in ["CON", "nul.txt", "Lpt9.log", "com1", "case.", "queue_status.json", "QUEUE.LOCK",
                     "frozen_plan_private.json.tmp", "runner_provenance.json", "../escape"]:
            with self.subTest(name=name):
                self.plan["cases"] = [self.case(name, .01)]
                with self.assertRaises(ValueError):
                    self.validated()

    def test_terminal_and_editor_are_pinned_at_exact_sandbox_paths(self):
        for name in queue.SANDBOX_BINARIES:
            with self.subTest(name=name):
                record = self.plan["artifacts"][name]
                original = record["path"]
                elsewhere = self.root / (name + ".exe")
                elsewhere.write_bytes(Path(original).read_bytes())
                record["path"] = str(elsewhere)
                with self.assertRaisesRegex(ValueError, "exact executable"):
                    self.validated()
                record["path"] = original

    def test_missing_terminal_pin_is_rejected_explicitly(self):
        del self.plan["artifacts"]["terminal"]
        with self.assertRaisesRegex(ValueError, "Missing frozen artifact.*terminal"):
            self.validated()

    def test_mapping_dependency_must_be_the_actual_imported_sibling(self):
        record = self.plan["artifacts"]["mapping"]
        other = self.root / "other-mapping.py"
        other.write_bytes(Path(record["path"]).read_bytes())
        record["path"] = str(other)
        with self.assertRaisesRegex(ValueError, "actual sibling"):
            self.validated()

    def test_mapping_changed_after_plan_cannot_launch(self):
        plan = self.validated()
        Path(plan["artifacts"]["mapping"]["path"]).write_bytes(b"changed mapping")
        with patch.object(queue.subprocess, "Popen") as process, self.assertRaisesRegex(ValueError, "mapping"):
            queue.run_case(plan, plan["cases"][0], self.root / "run", 25)
        process.assert_not_called()

    def test_mapping_changed_during_adapter_is_not_qualified(self):
        with self.assertRaisesRegex(ValueError, "mapping"):
            self.run_fake_case(after_write=lambda out: Path(self.plan["artifacts"]["mapping"]["path"]).write_bytes(b"updated mapping"))

    def test_wrong_fingerprint_and_live_marker_rejected_before_launch(self):
        self.plan["artifacts"]["terminal"]["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            self.validated()
        self.plan["artifacts"]["terminal"]["sha256"] = queue.fingerprint(Path(self.plan["artifacts"]["terminal"]["path"]))["sha256"]
        marker = Path(self.plan["context"]["sandbox"]) / "conduit_tester_sandbox.json"
        marker.write_text('{"purpose":"offline_strategy_tester","live_trading":true}')
        with self.assertRaisesRegex(ValueError, "offline"):
            self.validated()

    def test_changed_terminal_before_case_is_not_launched(self):
        plan = self.validated()
        Path(plan["artifacts"]["terminal"]["path"]).write_bytes(b"changed")
        with patch.object(queue.subprocess, "Popen") as process, self.assertRaisesRegex(ValueError, "terminal"):
            queue.run_case(plan, plan["cases"][0], self.root / "run", 25)
        process.assert_not_called()

    def test_fake_success_requires_final_receipt_and_only_changes_lot_cap(self):
        before = self.preset.read_bytes()
        result = self.run_fake_case()
        self.assertEqual(result["status"], "PASS")
        self.assertTrue(result["final_inputs_verified"])
        self.assertEqual(self.preset.read_bytes(), before)
        requested = json.loads((self.root / "run/requested_preset.json").read_text())
        self.assertEqual(requested, {"name": "Synthetic", "settings": {"lot_max": .01, "axis": 11}})
        self.assertEqual(self.last_command[self.last_command.index("--deposit") + 1], "600")
        self.assertNotIn("--initialize", self.last_command)
        self.assertEqual(set(result["sandbox_binaries"]), {"terminal", "metaeditor"})

    def test_final_tick_or_message_hash_mismatch_cannot_pass(self):
        for name in ["ticks", "messages"]:
            with self.subTest(name=name):
                out = self.root / ("run-" + name)
                plan = self.validated()
                def corrupt(path):
                    self.edit_manifest(path, lambda m: m[name].update(sha256="0" * 64))
                with patch.object(queue.subprocess, "Popen", self.fake_popen(after_write=corrupt)), self.assertRaisesRegex(ValueError, name):
                    queue.run_case(plan, plan["cases"][0], out, 25)

    def test_large_input_changed_after_validation_is_detected_by_adapter_receipt(self):
        plan = self.validated()
        Path(plan["artifacts"]["messages"]["path"]).write_bytes(b"changed-after-plan")
        with patch.object(queue.subprocess, "Popen", self.fake_popen()), self.assertRaisesRegex(ValueError, "messages"):
            queue.run_case(plan, plan["cases"][0], self.root / "run", 25)

    def test_missing_final_manifest_cannot_pass_even_if_comparison_matches(self):
        with self.assertRaises(FileNotFoundError):
            self.run_fake_case(after_write=lambda out: (out / "experiment.json").unlink())

    def test_changed_editor_during_case_is_not_qualified(self):
        with self.assertRaisesRegex(ValueError, "metaeditor"):
            self.run_fake_case(after_write=lambda out: Path(self.plan["artifacts"]["metaeditor"]["path"]).write_bytes(b"updated"))

    def test_wrong_native_source_or_final_scope_is_rejected(self):
        mutations = [lambda m: m["native_expert"]["source"].update(sha256="0" * 64),
                     lambda m: m.update(to_exclusive="2026-06-02"), lambda m: m.update(deposit=300)]
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index):
                plan = self.validated()
                with patch.object(queue.subprocess, "Popen", self.fake_popen(after_write=lambda out: self.edit_manifest(out, mutate))), self.assertRaises(ValueError):
                    queue.run_case(plan, plan["cases"][0], self.root / f"run-{index}", 25)

    def test_adapter_cannot_change_an_axis_and_rehash_its_own_effective_preset(self):
        def corrupt(out):
            path = out / "effective_preset.json"
            value = json.loads(path.read_text())
            value["settings"]["axis"] = 999
            queue.write_json(path, value)
            self.edit_manifest(out, lambda m: m.update(settings=queue.fingerprint(path)["sha256"]))
        with self.assertRaisesRegex(ValueError, "changed more"):
            self.run_fake_case(after_write=corrupt)

    def test_nonzero_exit_is_error_even_with_a_written_matching_comparison(self):
        result = self.run_fake_case(returncode=2)
        self.assertEqual(result["status"], "ERROR")
        self.assertFalse(result["final_inputs_verified"])

    def test_qualified_economic_difference_is_fail(self):
        result = self.run_fake_case(passed=False)
        self.assertEqual(result["status"], "FAIL")
        self.assertTrue(result["final_inputs_verified"])

    def test_timeout_bounds_adapter_and_kills_only_owned_process_tree(self):
        plan = self.validated()
        class TimeoutProcess:
            pid = 987654
            returncode = None
            waits = []
            def __init__(self, *args, **kwargs): pass
            def wait(self, timeout):
                self.waits.append(timeout)
                if len(self.waits) == 1:
                    raise subprocess.TimeoutExpired("synthetic-adapter", timeout)
                self.returncode = -9
            def poll(self): return None
        with patch.object(queue.subprocess, "Popen", TimeoutProcess), patch.object(queue.subprocess, "run") as kill, \
             patch.object(queue.os, "killpg", create=True) as kill_group:
            result = queue.run_case(plan, plan["cases"][0], self.root / "run", 7)
        self.assertEqual(result["status"], "ERROR")
        self.assertTrue(result["timed_out"])
        self.assertEqual(TimeoutProcess.waits, [7, 30])
        if os.name == "nt":
            self.assertEqual(kill.call_args.args[0], ["taskkill", "/PID", "987654", "/T", "/F"])
            kill_group.assert_not_called()
        else:
            kill_group.assert_called_once_with(987654, queue.signal.SIGKILL)
            kill.assert_not_called()

    def run_fake_queue(self, effect, continue_after_failure=False, seconds=1000, estimate=100, reserve=10):
        plan = self.validated()
        cutoff = datetime.now(timezone.utc) + timedelta(seconds=seconds)
        with patch.object(queue, "run_case", side_effect=effect) as run, redirect_stdout(io.StringIO()):
            result = queue.run_queue(plan, self.root / "queue", cutoff, 500, estimate, reserve, continue_after_failure)
        return result, run

    def test_error_stops_queue_even_when_continue_after_failure_is_set(self):
        self.plan["cases"].append(self.case("independent", .01))
        result, run = self.run_fake_queue([{"status": "ERROR"}], True)
        self.assertEqual([r["status"] for r in result["cases"]], ["ERROR", "DEFERRED_FAILURE"])
        self.assertEqual(run.call_count, 1)

    def test_failed_low_cap_blocks_high_cap_but_optional_independent_case_can_run(self):
        self.add_large()
        self.plan["cases"].append(self.case("independent", .01, candidate_id="Other"))
        result, run = self.run_fake_queue([{"status": "FAIL"}, {"status": "PASS"}], True)
        self.assertEqual([r["status"] for r in result["cases"]], ["FAIL", "DEFERRED_PREREQUISITE", "PASS"])
        self.assertEqual(run.call_count, 2)

    def test_default_failure_stops_even_independent_cases(self):
        self.plan["cases"].append(self.case("independent", .01))
        result, run = self.run_fake_queue([{"status": "FAIL"}])
        self.assertEqual([r["status"] for r in result["cases"]], ["FAIL", "DEFERRED_FAILURE"])
        self.assertEqual(run.call_count, 1)

    def test_case_exception_is_error_and_private_path_not_copied_to_status(self):
        result, _ = self.run_fake_queue(ValueError("C:/private/sensitive.json"), True)
        self.assertEqual(result["cases"][0]["status"], "ERROR")
        self.assertNotIn("sensitive", json.dumps(result))

    def test_deadline_reserve_prevents_any_process_start(self):
        result, run = self.run_fake_queue([], seconds=109, estimate=100, reserve=10)
        run.assert_not_called()
        self.assertEqual(result["cases"][0]["status"], "DEFERRED_DEADLINE")
        self.assertFalse((self.root / "queue/queue.lock").exists())

    def test_child_timeout_is_capped_to_remaining_time_less_reserve(self):
        result, run = self.run_fake_queue([{"status": "PASS"}], seconds=200, reserve=20)
        self.assertEqual(result["status"], "COMPLETE")
        self.assertLessEqual(run.call_args.args[3], 180)
        self.assertGreaterEqual(run.call_args.args[3], 170)

    def test_deadline_is_rechecked_between_cases(self):
        self.plan["cases"].append(self.case("next", .01))
        current = datetime(2026, 9, 5, 18, tzinfo=timezone.utc)
        class Clock(datetime):
            @classmethod
            def now(cls, tz=None): return current
        def run(*args):
            nonlocal current
            current += timedelta(seconds=950)
            return {"status": "PASS"}
        with patch.object(queue, "datetime", Clock), patch.object(queue, "run_case", side_effect=run), redirect_stdout(io.StringIO()):
            result = queue.run_queue(self.validated(), self.root / "queue", current + timedelta(seconds=1000), 500, 100, 10)
        self.assertEqual([r["status"] for r in result["cases"]], ["PASS", "DEFERRED_DEADLINE"])

    def test_naive_deadline_and_reuse_of_existing_output_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "timezone"):
            queue.aware_time("2026-09-05T22:00:00")
        self.assertEqual(queue.aware_time("2026-09-05T22:00:00+02:00").hour, 20)
        with self.assertRaisesRegex(ValueError, "fresh"):
            queue.run_queue(self.validated(), self.root, datetime.now(timezone.utc), 1, 1, 0)


if __name__ == "__main__":
    unittest.main()
