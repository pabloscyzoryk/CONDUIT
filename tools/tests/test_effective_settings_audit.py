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
        self.preset = self.write("presets/TEST.json", {"name": "TEST", "format": "Synergy", "settings": {"lot_max": 5, **{key: False if key in package_release.OVERLAY_BOOL_KEYS else "Balance" if key == "lot_base" else 0 for key in package_release.ACCOUNT_OVERLAY_KEYS}}})
        preset = audit.read(self.preset)
        preset['settings'].update(ai_enabled=False, ai_model='', ai_decision_interval_s=2.0, ai_replaces_management=True)
        self.write('presets/TEST.json', preset)

    def tearDown(self):
        self.temp.cleanup()

    def write(self, name, value):
        p = self.root / name
        p.write_text(json.dumps(value), encoding="utf-8")
        return p

    def fake_probe(self, *_args, **kwargs):
        self.request = json.loads(kwargs["input"])
        return subprocess.CompletedProcess([], 0, json.dumps({"matches": True,
            "core_differences": [], "chain_differences": [], "live_start_blockers": [],
            "single_and_multiple_routing_equal": True, "preset_to_live_overrides": []}), "")

    def test_invalid_age_types_do_not_default_or_compare_equal_to_zero(self):
        for age in (False, True, None, '0', float('nan'), float('inf'), -1):
            with self.subTest(age=repr(age)):
                self.write('settings.json', {'settings': {'signal_max_age_min': age}})
                with patch.object(audit.subprocess, 'run', side_effect=self.fake_probe):
                    with self.assertRaisesRegex(audit.AuditError, 'invalid_live_ingress_max_age'):
                        audit.audit(self.root, Path('probe.exe'))

    def test_recipe_match_is_separate_from_all_36_tested_fields(self):
        raw = audit.read(self.preset)['settings']
        for key in package_release.ACCOUNT_OVERLAY_KEYS:
            overlay = package_release.tested_overlay(raw)
            old = overlay[key]
            overlay[key] = not old if type(old) is bool else 'Equity' if key == 'lot_base' else 1 if old is None else old + 1
            proposal = {'approved': True, 'preset_id': 'TEST', 'preset_sha256':hashlib.sha256(self.preset.read_bytes()).hexdigest(),
                'account_overlay':overlay, 'chain_caps': {k: False if k in package_release.CHAIN_BOOL_KEYS else 0 for k in package_release.CHAIN_CAP_KEYS},
                'ingress': {'live_telegram_ingress':True, 'live_ingress_max_age_min':5}}
            with self.subTest(field=key):
                selection = self.write('proposal.json', proposal)
                with patch.object(audit.subprocess, 'run', side_effect=self.fake_probe):
                    result = audit.audit(self.root, Path('probe.exe'), preset_path=self.preset, selection_path=selection)
                self.assertTrue(result['recipe_matches'])
                self.assertFalse(result['strategy_matches_tested_preset'])
                self.assertFalse(result['matches'])

    def test_only_explicit_technical_fields_can_differ(self):
        for field in ('mt5_terminal_path', 'journal_enabled', 'ai_enabled', 'konto_dzwignia', 'lot_base', 'lot_percent'):
            difference = {'field':field, 'expected':'redacted', 'actual':'redacted'}
            def probe(*args, **kwargs):
                result = self.fake_probe(*args, **kwargs)
                body = json.loads(result.stdout)
                body.update(matches=False, core_differences=[difference], preset_to_live_overrides=[difference])
                result.stdout = json.dumps(body)
                return result
            with self.subTest(field=field), patch.object(audit.subprocess, 'run', side_effect=probe):
                report = audit.audit(self.root, Path('probe.exe'))
                self.assertEqual(report['matches'], field in package_release.TECHNICAL_ACCOUNT_KEYS)
                self.assertEqual(report['core_differences'], [difference])

    def test_routing_or_live_blocker_never_overridden_by_technical_exception(self):
        for key, value in [('single_and_multiple_routing_equal', False), ('live_start_blockers', ['TEST_HOLD'])]:
            def probe(*args, **kwargs):
                result = self.fake_probe(*args, **kwargs)
                body = json.loads(result.stdout)
                body[key] = value
                result.stdout = json.dumps(body)
                return result
            with self.subTest(field=key), patch.object(audit.subprocess, 'run', side_effect=probe):
                self.assertFalse(audit.audit(self.root, Path('probe.exe'))['matches'])

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
            "account_overlay": {key: False if key in package_release.OVERLAY_BOOL_KEYS else "Balance" if key == "lot_base" else 0 for key in package_release.ACCOUNT_OVERLAY_KEYS},
            "chain_caps": {key: False if key in package_release.CHAIN_BOOL_KEYS else 0 for key in package_release.CHAIN_CAP_KEYS},
            "ingress": {"live_telegram_ingress": True, "live_ingress_max_age_min": 30}}
        selection = self.write("proposal.json", proposal)
        with patch.object(audit.subprocess, "run", side_effect=self.fake_probe):
            report = audit.audit(self.root, Path("probe.exe"), preset_path=self.preset, selection_path=selection)
        self.assertFalse(report["selection_owner_approved"])
        self.assertFalse(audit.read(selection)["approved"])
        self.assertEqual(self.request["expected"]["lot_max"], 5)
        self.assertEqual(self.request['settings_doc']['settings']['signal_max_age_min'],30)
        self.assertTrue(report['ingress']['matches'])
        self.assertNotIn("konto_dzwignia", audit.read(self.root / "settings.json")["settings"])

    def test_package_ingress_mismatch_cannot_receive_a_match_stamp(self):
        self.write('settings.json', {'settings': {**audit.read(self.preset)['settings'], **audit.read(self.root/'settings.json')['settings']}})
        self.write('PACKAGE_MANIFEST.json',{'ingress_explicit':{'live_telegram_ingress':True,'live_ingress_max_age_min':30}})
        with patch.object(audit.subprocess,'run',side_effect=self.fake_probe):
            report=audit.audit(self.root,Path('probe.exe'))
        self.assertFalse(report['matches'])
        self.assertFalse(report['ingress']['matches'])
        self.assertEqual(report['ingress']['live_ingress_max_age_min'],5)

    def test_explicit_zero_ingress_remains_disabled_age_gate(self):
        self.write('settings.json',{'settings':{'signal_max_age_min':0}})
        self.write('settings.json', {'settings': {**audit.read(self.preset)['settings'], **audit.read(self.root/'settings.json')['settings']}})
        self.write('PACKAGE_MANIFEST.json',{'ingress_explicit':{'live_telegram_ingress':True,'live_ingress_max_age_min':0}})
        with patch.object(audit.subprocess,'run',side_effect=self.fake_probe):
            report=audit.audit(self.root,Path('probe.exe'))
        self.assertTrue(report['matches'])
        self.assertEqual(report['ingress']['live_ingress_max_age_min'],0)


if __name__ == "__main__":
    unittest.main()
