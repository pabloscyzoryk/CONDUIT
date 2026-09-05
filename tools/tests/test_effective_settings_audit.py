import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))
import effective_settings_audit as audit
import package_release


class EffectiveAuditTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / "presets").mkdir()
        self.write("settings.json", {"settings": {"synthetic_private_unused": "DO-NOT-REPORT"}})
        self.write("channels.json", {"bindings": {"-990001": {"monitored": True, "format": "Synergy"}}})
        self.write("lancuchy.json", {"aktywny": "synthetic", "lista": [{"nazwa": "synthetic", "presety": {"Synergy": "TEST"}, "pulapy": {}}]})
        self.preset = self.write("presets/TEST.json", {"name": "TEST", "format": "Synergy", "settings": {"lot_max": 5}})

    def tearDown(self):
        self.temp.cleanup()

    def write(self, name, value):
        p = self.root / name
        p.write_text(json.dumps(value), encoding="utf-8")
        return p

    def fake_probe(self, *_args, **kwargs):
        self.request = json.loads(kwargs["input"])
        return subprocess.CompletedProcess([], 0, json.dumps({"matches": True,
            "core_differences": [], "chain_differences": [], "live_start_blockers": []}), "")

    def test_never_reads_authentication_and_report_does_not_echo_input(self):
        self.write("secrets.json", {"password": "SYNTHETIC-SECRET"})
        original = audit.read
        paths = []
        def spy(path):
            paths.append(path.name)
            self.assertNotIn(path.name, {"secrets.json", "telegram.session"})
            return original(path)
        with patch.object(audit, "read", side_effect=spy), patch.object(audit.subprocess, "run", side_effect=self.fake_probe):
            report = audit.audit(self.root, Path("probe.exe"))
        self.assertFalse(report["authentication_read"])
        self.assertNotIn("DO-NOT-REPORT", json.dumps(report))
        self.assertNotIn("-990001", json.dumps(report))
        self.assertNotIn("secrets.json", paths)

    def test_duplicate_internal_preset_identity_with_different_values_is_rejected(self):
        self.write("presets/00-copy.json", {"name": "test", "settings": {"lot_max": 10}})
        with self.assertRaisesRegex(audit.AuditError, "duplicate_preset_identity"):
            audit.resolve_preset(self.root, "TEST")

    def test_unobserved_synergy_cannot_receive_a_match_stamp(self):
        self.write("channels.json", {"bindings": {}})
        with patch.object(audit.subprocess, "run", side_effect=self.fake_probe):
            report = audit.audit(self.root, Path("probe.exe"))
        self.assertFalse(report["matches"])
        self.assertFalse(report["routing"]["synergy_observed"])

    def test_unapproved_proposal_is_reviewable_but_never_promoted_to_approved(self):
        proposal = {"approved": False, "preset_id": "TEST",
            "preset_sha256": hashlib.sha256(self.preset.read_bytes()).hexdigest(),
            "account_overlay": dict.fromkeys(package_release.ACCOUNT_OVERLAY_KEYS, 0),
            "chain_caps": dict.fromkeys(package_release.CHAIN_CAP_KEYS, 0)}
        selection = self.write("proposal.json", proposal)
        with patch.object(audit.subprocess, "run", side_effect=self.fake_probe):
            report = audit.audit(self.root, Path("probe.exe"), preset_path=self.preset, selection_path=selection)
        self.assertFalse(report["selection_owner_approved"])
        self.assertFalse(audit.read(selection)["approved"])
        self.assertEqual(self.request["expected"]["lot_max"], 5)
        self.assertNotIn("konto_dzwignia", audit.read(self.root / "settings.json")["settings"])


if __name__ == "__main__":
    unittest.main()
