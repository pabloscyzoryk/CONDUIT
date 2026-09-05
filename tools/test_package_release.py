"""Offline packaging regressions using synthetic identities and inert executables.

The only mocked dependency is portable_runtime.verify: runtime authentication
and third-party binary validation belong to that tool's own tests. Every stage,
privacy check, file hash, Git export and package verification here runs for real.
No program, terminal, Telegram session or network transport is started.
"""
from __future__ import annotations

import base64
import contextlib
import copy
import hashlib
import io
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import package_release as pkg


class SyntheticFixture(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="conduit-package-test-")
        self.root = Path(self.temp.name).resolve()
        self.addCleanup(self.cleanup_fixture)
        self.source = self.root / "source"
        self.template = self.root / "VPSREADY_TEMPLATE"
        self.runtime = self.root / "runtime-fixture"
        self.exe = self.root / "inert.exe"
        self.exe.write_bytes(b"MZ-inert-synthetic-package-fixture")
        self.monitor = self.write("inert-monitor.exe", b"MZ-inert-monitor-fixture")
        self.write("runtime-fixture/python.exe", b"MZ-inert-python-fixture")
        self.write("runtime-fixture/LICENSE.txt", b"Synthetic runtime; never executed.\n")
        self.write("source/rust/crates/mt5/sidecar/mt5_sidecar.py", b"import local_receipts\nif False:\n    import local_optional\n")
        self.write("source/rust/crates/mt5/sidecar/local_receipts.py", b"import json\ndef ack(): return True\n")
        self.write("source/rust/crates/mt5/sidecar/local_optional.py", b"import local_receipts\n")
        self.write("source/rust/crates/mt5/sidecar/not_imported.py", b"raise RuntimeError('unused')\n")
        self.caps = {key: 0 for key in pkg.CHAIN_CAP_KEYS}
        self.caps.update(maxLotow=5, maxKoszykow=200, celDniaZamyka=False, blokujPrzeciwneKierunki=False)
        self.overlay = {key: 0 for key in pkg.ACCOUNT_OVERLAY_KEYS}
        self.overlay.update(lot_base="Balance", close_receipt_reconcile=True, server_tz_offset_ms=10800000,
                            msg_clock_offset_ms=None, konto_dzwignia=500, exec_latency_ms=250)
        self.identity = {
            "mt5_login": "88990011", "mt5_server": "SyntheticBroker-TEST",
            "mt5_password": "synthetic-mt-password", "mt5_terminal_path": "C:/SyntheticOnly/terminal64.exe",
            "mt5_follow_terminal_account": True, "mt5_allow_real_account": False,
        }
        self.settings = {"mode": "AUTO", "language": "pl", "presetId": "OLD",
                         "settings": {**self.identity, "close_receipt_reconcile": False, "lot_base": "Equity"}}
        self.chain = {"aktywny": "FixtureChain", "lista": [{"nazwa": "FixtureChain",
                      "presety": {"Synergy": "OLD", "ATFX": "OLD-OTHER"}, "pulapy": {"maxLotow": 0.01}}]}
        self.channels = {"bindings": {"Synergy": {"channelId": "-10088992233", "topicId": 447788}}}
        self.session = {"version": 1, "home_dc": 2, "dc_options": [
            {"id": 2, "auth_key": "synthetic-home-authorization"}, {"id": 4, "auth_key": None}]}
        self.secrets = {"telegram": {"apiId": 77889911, "apiHash": "synthetic-api-hash-material",
                        "sessionString": self.encode_session(self.session)},
                        "mt5": {"password": self.identity["mt5_password"]}}
        self.json("VPSREADY_TEMPLATE/settings.json", self.settings)
        self.json("VPSREADY_TEMPLATE/lancuchy.json", self.chain)
        self.json("VPSREADY_TEMPLATE/channels.json", self.channels)
        self.json("VPSREADY_TEMPLATE/secrets.json", self.secrets)
        self.json("VPSREADY_TEMPLATE/telegram.session", self.session)
        self.write("VPSREADY_TEMPLATE/logs/old.log", b"Must not be copied")
        self.write("VPSREADY_TEMPLATE/koszyki.json", b"[]")
        self.write("VPSREADY_TEMPLATE/old_verifier.exe", b"MZold")
        self.json("source/config/examples/settings.example.json", {
            "mode": "AUTO", "language": "pl", "settings": {"mt5_login": "", "mt5_server": "", "mt5_password": ""}})
        self.json("source/config/examples/lancuchy.example.json", self.chain)
        self.preset = self.root / "selected.json"
        self.selection = self.root / "selection.json"
        self.preset_doc = {"name": "GOD-X8-TEST", "format": "Synergy", "settings": {"lot_max": 5}}
        self.json("selected.json", self.preset_doc)
        self.chosen = {"approved": True, "preset_id": self.preset_doc["name"],
                       "preset_sha256": pkg.sha(self.preset.read_bytes()),
                       "chain_caps": self.caps, "account_overlay": self.overlay}
        self.json("selection.json", self.chosen)
        self.json("source/config/presets/GOD-X7.json", {"name": "GOD-X7", "format": "Synergy", "settings": {"lot_max": 10}})
        self.runtime_check = self.enterContext(patch("portable_runtime.verify", return_value={"ok": True}))

    def cleanup_fixture(self):
        # Only the unique, explicitly created temporary fixture may be removed.
        self.assertEqual(Path(self.temp.name).resolve(), self.root)
        self.assertTrue(self.root.name.startswith("conduit-package-test-"))
        self.temp.cleanup()

    def write(self, relative, data):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def json(self, relative, value):
        return self.write(relative, (json.dumps(value, indent=2) + "\n").encode())

    @staticmethod
    def encode_session(value):
        return base64.b64encode(json.dumps(value).encode()).decode()

    def reset_session(self, value, string_value=None):
        self.json("VPSREADY_TEMPLATE/telegram.session", value)
        self.secrets["telegram"]["sessionString"] = self.encode_session(value if string_value is None else string_value)
        self.json("VPSREADY_TEMPLATE/secrets.json", self.secrets)

    def stage(self, kind="public", name=None):
        destination = self.root / (name or ("VPSREADY_FINAL" if kind == "private" else "PUBLIC_FINAL"))
        result = pkg.stage(self.source, self.template, self.exe, self.preset, self.selection,
                           destination, kind, self.runtime, self.monitor)
        self.assertTrue(result["ok"])
        self.assertFalse(result["live_authentication_verified"])
        self.assertTrue(result["no_services_started"])
        return destination

    def assert_code(self, code, function, *args, **kwargs):
        with self.assertRaises(pkg.PackageError) as raised:
            function(*args, **kwargs)
        self.assertEqual(str(raised.exception), code)

    def revise_selection(self, change):
        value = copy.deepcopy(self.chosen)
        change(value)
        self.json("selection.json", value)

    def update_package_json(self, package, relative, change, rehash=True):
        path = package / relative
        value = pkg.read_json(path)
        change(value)
        pkg.write_json(path, value)
        if rehash and relative != "PACKAGE_MANIFEST.json":
            manifest = pkg.read_json(package / "PACKAGE_MANIFEST.json")
            if relative in manifest["public_files"]:
                data = path.read_bytes()
                manifest["public_files"][relative] = {"sha256": pkg.sha(data), "size": len(data)}
                pkg.write_json(package / "PACKAGE_MANIFEST.json", manifest)


class PackageTests(SyntheticFixture):
    def test_public_stage_is_unconfigured_and_contains_portable_runtime(self):
        package = self.stage()
        settings = pkg.read_json(package / "settings.json")
        self.assertEqual(settings["mode"], "MANUAL")
        self.assertEqual(settings["language"], "en")
        self.assertEqual(settings["settings"]["mt5_python"], "runtime/python.exe")
        self.assertEqual(pkg.read_json(package / "channels.json"), {"bindings": {}})
        for name in ("secrets.json", "telegram.session", "koszyki.json", "old_verifier.exe", "INCOMPLETE"):
            self.assertFalse((package / name).exists())
        for name in ("runtime/python.exe", "runtime/LICENSE.txt", "postep.exe", "START_CONDUIT.vbs", "START_BROWSER.vbs",
                     "local_receipts.py", "local_optional.py", "presets/GOD-X7.json"):
            self.assertTrue((package / name).is_file(), name)
        self.assertFalse((package / "not_imported.py").exists())
        self.assertEqual((package / "postep.exe").read_bytes(), self.monitor.read_bytes())
        self.runtime_check.assert_called_once_with(self.runtime)
        self.assertTrue(pkg.verify(package)["ok"])

    def test_private_stage_preserves_identity_and_only_reviewed_economic_overrides(self):
        package = self.stage("private")
        settings = pkg.read_json(package / "settings.json")
        self.assertEqual(settings["mode"], "AUTO")
        self.assertEqual(settings["language"], "pl")
        for key, value in self.identity.items():
            self.assertEqual(settings["settings"][key], value, key)
        for key, value in self.overlay.items():
            self.assertEqual(settings["settings"][key], value, key)
        for name in ("secrets.json", "telegram.session", "channels.json"):
            self.assertEqual((package / name).read_bytes(), (self.template / name).read_bytes())
        self.assertFalse((package / "logs").exists())
        self.assertTrue(pkg.verify(package, self.template)["ok"])
        report = json.dumps(pkg.verify(package, self.template))
        self.assertNotIn(self.identity["mt5_login"], report)
        self.assertNotIn(hashlib.sha256(self.identity["mt5_login"].encode()).hexdigest(), report)

    def test_selected_preset_caps_current_snapshot_and_synergy_only_are_consistent(self):
        package = self.stage("private")
        active = pkg.read_json(package / "lancuchy.json")["lista"][0]
        self.assertEqual(active["pulapy"], self.caps)
        self.assertEqual({k:v for k,v in active["presety"].items() if v}, {"Synergy":"GOD-X8-TEST"})
        self.assertEqual((package / "presets/BIEZACY.json").read_bytes(), self.preset.read_bytes())
        self.assertEqual((package / "presets/GOD-X8-TEST.json").read_bytes(), self.preset.read_bytes())

    def test_private_home_dc_authorization_allows_other_unauthenticated_dcs(self):
        result = pkg.source_inspection(self.template)
        self.assertIs(result["telegram_authorization_matches"], True)
        self.stage("private")

    def test_empty_home_authorization_and_empty_maps_are_not_equal_authentication(self):
        for options in ([], [{"id":2,"auth_key":None}], [{"id":2,"auth_key":""},{"id":4,"auth_key":"unrelated-dc-key"}]):
            with self.subTest(options=len(options)):
                self.reset_session({"version":1,"home_dc":2,"dc_options":options})
                self.assertIs(pkg.source_inspection(self.template)["telegram_authorization_matches"], False)
                self.assert_code("private_telegram_material_incomplete_or_mismatched", self.stage, "private")

    def test_changed_session_file_is_not_accepted_as_the_string_session(self):
        different = copy.deepcopy(self.session)
        different["dc_options"][0]["auth_key"] = "different-synthetic-authorization"
        self.reset_session(self.session, different)
        self.assertIs(pkg.source_inspection(self.template)["telegram_authorization_matches"], False)
        self.assert_code("private_telegram_material_incomplete_or_mismatched", self.stage, "private")

    def test_destination_cannot_be_overwritten_or_cross_public_private_boundary(self):
        package = self.stage()
        before = (package / "PACKAGE_MANIFEST.json").read_bytes()
        self.assert_code("destination_already_exists", self.stage)
        self.assertEqual((package / "PACKAGE_MANIFEST.json").read_bytes(), before)
        self.assert_code("private_destination_must_be_vpsready", self.stage, "private", "WRONG")
        self.assert_code("public_destination_cannot_be_private", self.stage, "public", "VPSREADY_WRONG")

    def test_unapproved_changed_or_mismatched_preset_is_blocked(self):
        for change, code in (
            (lambda v:v.update(approved=False), "selected_preset_not_approved_or_changed"),
            (lambda v:v.update(preset_sha256="0"*64), "selected_preset_not_approved_or_changed"),
            (lambda v:v.update(preset_id="DIFFERENT"), "selected_preset_identity_mismatch"),
            (lambda v:v.update(preset_id="../escape"), "selected_preset_identity_mismatch"),
        ):
            with self.subTest(code=code):
                self.revise_selection(change)
                self.assert_code(code, pkg.selected_preset, self.preset, self.selection)

    def test_explicit_complete_caps_and_account_overlay_are_required(self):
        for change, code in (
            (lambda v:v.pop("chain_caps"), "explicit_complete_chain_caps_required"),
            (lambda v:v["chain_caps"].pop("maxLotow"), "explicit_complete_chain_caps_required"),
            (lambda v:v["chain_caps"].update(unreviewed=0), "explicit_complete_chain_caps_required"),
            (lambda v:v.pop("account_overlay"), "explicit_complete_account_overlay_required"),
            (lambda v:v["account_overlay"].pop("exec_latency_ms"), "explicit_complete_account_overlay_required"),
        ):
            with self.subTest(code=code):
                self.revise_selection(change)
                self.assert_code(code, pkg.selected_preset, self.preset, self.selection)

    def test_identity_overlay_is_blocked_even_when_empty(self):
        for key in ("mt5_login", "mt5Password", "phone", "channelId", "topicId"):
            with self.subTest(key=key):
                self.revise_selection(lambda v:v["account_overlay"].update({key:""}))
                self.assert_code("account_overlay_cannot_change_identity", pkg.selected_preset, self.preset, self.selection)

    def test_public_config_recognizes_channel_and_topic_identifiers(self):
        for key in ("channelId", "chat_id", "topicId", "mt5Login", "apiHash"):
            with self.subTest(key=key):
                self.assert_code("nonempty_identity_in_public_configuration", pkg.check_public_config, {"nested":[{key:42}]})

    def test_private_values_in_payload_and_derived_account_hash_are_blocked(self):
        values = pkg.sensitive_values([self.template])
        for value in (self.identity["mt5_login"].encode(), self.identity["mt5_password"].encode(),
                      self.identity["mt5_server"].encode("utf-16-le"),
                      hashlib.sha256(self.identity["mt5_login"].encode()).hexdigest().encode(),
                      str(self.channels["bindings"]["Synergy"]["topicId"]).encode()):
            with self.subTest(bytes=len(value)):
                self.assert_code("private_value_in_public_payload", pkg.public_bytes, b"prefix "+value+b" suffix", values)

    def test_public_manifest_is_scanned_for_private_values(self):
        package = self.stage()
        self.update_package_json(package, "PACKAGE_MANIFEST.json", lambda v:v.update(extra=self.identity["mt5_login"]))
        self.assert_code("private_value_in_public_payload", pkg.verify, package, self.template)

    def test_public_manifest_rejects_new_identity_fields_even_without_template(self):
        package = self.stage()
        self.update_package_json(package, "PACKAGE_MANIFEST.json", lambda v:v.update(metadata={"channelId":42,"topicId":19}))
        self.assert_code("nonempty_identity_in_public_configuration", pkg.verify, package)

    def test_unexpected_files_changed_payload_and_incomplete_stage_are_blocked(self):
        package = self.stage()
        (package / "INCOMPLETE").write_text("incomplete")
        self.assert_code("package_incomplete", pkg.verify, package)
        (package / "INCOMPLETE").unlink()
        (package / "extra.txt").write_text("unknown")
        self.assert_code("unexpected_package_file", pkg.verify, package)
        (package / "extra.txt").unlink()
        (package / "conduit.exe").write_bytes(b"MZchanged")
        self.assert_code("runtime_or_public_payload_changed", pkg.verify, package)

    def test_private_authentication_and_account_selection_tampering_are_blocked(self):
        package = self.stage("private")
        self.assert_code("private_verification_requires_explicit_template", pkg.verify, package)
        original = (package / "secrets.json").read_bytes()
        (package / "secrets.json").write_bytes(b"{}")
        self.assert_code("private_material_differs_from_template", pkg.verify, package, self.template)
        (package / "secrets.json").write_bytes(original)
        self.update_package_json(package, "settings.json", lambda v:v["settings"].update(mt5_login="99900022"))
        self.assert_code("private_account_selection_changed", pkg.verify, package, self.template)

    def test_current_snapshot_cannot_be_replaced_even_with_updated_file_hash(self):
        package = self.stage()
        self.update_package_json(package, "presets/BIEZACY.json", lambda v:v["settings"].update(lot_max=0.01))
        self.assert_code("current_preset_snapshot_changed", pkg.verify, package)

    def test_chain_caps_cannot_silently_inherit_or_change(self):
        package = self.stage("private")
        self.update_package_json(package, "lancuchy.json", lambda v:v["lista"][0]["pulapy"].update(maxLotow=0.01))
        self.assert_code("chain_caps_differ_from_reviewed_selection", pkg.verify, package, self.template)

    def test_account_overlay_cannot_silently_revert_to_template(self):
        package = self.stage("private")
        self.update_package_json(package, "settings.json", lambda v:v["settings"].update(lot_base="Equity"))
        self.assert_code("account_overlay_differs_from_reviewed_selection", pkg.verify, package, self.template)

    def test_runtime_interpreter_must_be_portable(self):
        package = self.stage()
        self.update_package_json(package, "settings.json", lambda v:v["settings"].update(mt5_python="python"))
        self.assert_code("portable_interpreter_not_selected", pkg.verify, package)

    def test_public_mode_and_channels_cannot_be_activated_by_rehash(self):
        package = self.stage()
        self.update_package_json(package, "settings.json", lambda v:v.update(mode="AUTO"))
        self.assert_code("public_package_not_unconfigured_manual", pkg.verify, package)
        self.update_package_json(package, "settings.json", lambda v:v.update(mode="MANUAL"))
        self.update_package_json(package, "channels.json", lambda v:v.update(bindings={"Synergy":{}}))
        self.assert_code("public_package_not_unconfigured_manual", pkg.verify, package)

    def test_path_traversal_and_non_windows_payload_are_rejected(self):
        self.assert_code("unsafe_member_path", pkg.checked_file, self.root, "../outside")
        self.exe.write_bytes(b"not-a-windows-executable")
        self.assert_code("runtime_not_windows_executable", self.stage)

    def test_missing_or_invalid_progress_monitor_blocks_the_package(self):
        self.monitor.write_bytes(b"not-a-windows-monitor")
        self.assert_code("monitor_not_windows_executable", self.stage)
        self.monitor.unlink()
        self.assert_code("required_payload_missing", self.stage)
        self.assertFalse((self.root / "PUBLIC_FINAL").exists())

    def test_cli_failure_reports_only_a_stable_code(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            status = pkg.main(["verify", "--package", str(self.root / "missing")])
        self.assertEqual(status, 1)
        result = json.loads(output.getvalue())
        self.assertFalse(result["ok"])
        self.assertFalse(result["private_values_or_hashes_reported"])
        self.assertNotIn(str(self.root), output.getvalue())


@unittest.skipUnless(shutil.which("git"), "Git is required for immutable source export tests")
class SourceExportTests(SyntheticFixture):
    def make_repo(self, extra=None):
        repo = self.root / "git-source"
        repo.mkdir()
        files = {"README.md": b"Synthetic source export\n", "CONDUIT/src/example.ts": b"export const value = 1;\n",
                 "CONDUIT/config/examples/secrets.example.json": b'{"telegram":{"apiId":0,"apiHash":"","sessionString":""},"mt5":{"password":""}}\n'}
        files.update(extra or {})
        for relative, data in files.items():
            self.write("git-source/"+relative, data)
        def git(*argv):
            return subprocess.check_output(["git", "-C", str(repo), *argv], stderr=subprocess.PIPE).decode().strip()
        git("init", "--quiet")
        git("add", ".")
        git("-c", "user.name=Synthetic Test", "-c", "user.email=synthetic@example.com", "commit", "--quiet", "-m", "Synthetic fixture")
        return repo, git("rev-parse", "HEAD")

    def test_source_export_uses_only_immutable_commit_and_allows_sanitized_example(self):
        repo, revision = self.make_repo()
        (repo / "README.md").write_text("uncommitted change")
        (repo / "untracked.txt").write_text("must not escape")
        target = self.root / "PUBLIC_SOURCE"
        result = pkg.export_source(repo, revision, target, self.template)
        self.assertEqual(result["commit"], revision)
        self.assertEqual((target / "README.md").read_text(), "Synthetic source export\n")
        self.assertFalse((target / "untracked.txt").exists())
        self.assertTrue((target / "CONDUIT/config/examples/secrets.example.json").is_file())
        self.assert_code("destination_already_exists", pkg.export_source, repo, revision, target, self.template)

    def test_source_export_rejects_nonempty_secrets_example(self):
        repo, revision = self.make_repo({"CONDUIT/config/examples/secrets.example.json":b'{"telegram":{"apiHash":"nonempty-synthetic-value"}}'})
        target = self.root / "PUBLIC_SOURCE"
        self.assert_code("nonempty_identity_in_public_configuration", pkg.export_source, repo, revision, target, self.template)
        self.assertFalse(target.exists())

    def test_source_export_rejects_private_artifact(self):
        repo, revision = self.make_repo({"CONDUIT/secrets.json":b"{}"})
        self.assert_code("private_or_build_artifact_in_source", pkg.export_source, repo, revision, self.root/"PUBLIC_SOURCE", self.template)

    def test_source_export_rejects_embedded_binary(self):
        repo, revision = self.make_repo({"CONDUIT/private.exe":b"MZfixture"})
        self.assert_code("private_or_build_artifact_in_source", pkg.export_source, repo, revision, self.root/"PUBLIC_SOURCE", self.template)

    def test_source_export_rejects_vpsready_directory(self):
        repo, revision = self.make_repo({"VPSREADY_OLD/README.md":b"private"})
        self.assert_code("source_member_outside_allowlist", pkg.export_source, repo, revision, self.root/"PUBLIC_SOURCE", self.template)

    def test_source_export_rejects_identity_embedded_in_source(self):
        repo, revision = self.make_repo({"CONDUIT/src/leak.ts":self.identity["mt5_login"].encode()})
        self.assert_code("private_value_in_public_payload", pkg.export_source, repo, revision, self.root/"PUBLIC_SOURCE", self.template)


if __name__ == "__main__":
    unittest.main(verbosity=2)
