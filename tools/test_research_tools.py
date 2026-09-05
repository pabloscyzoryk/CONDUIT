import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from giga_sweep8 import BROKER_FIELDS, FAMILIES, generate
from research_runner import validate_results, write_json, write_progress


class CandidateSpaceTests(unittest.TestCase):
    def test_reproducible_unique_broad_and_capped(self):
        base = {'settings': {'lot_max': 0, 'entry_units': 8, 'lot_percent': .5}}
        broker = {'konto_dzwignia': 500, 'swap_long_points': -75.82}
        rows = generate(base, broker, 1000, 20260905)
        self.assertEqual(rows, generate(base, broker, 1000, 20260905))
        self.assertEqual(len({r['fingerprint'] for r in rows}), 1000)
        self.assertEqual({r['family'] for r in rows}, {*FAMILIES, 'reference'})
        for row in rows:
            self.assertEqual(row['settings']['lot_max'], 5)
            self.assertNotIn('explicit_pending_until_cancel', row['settings'])
            self.assertFalse(set(row['changes']) & BROKER_FIELDS)
            self.assertEqual(row['settings']['konto_dzwignia'], 500)
            self.assertEqual(row['settings']['swap_long_points'], -75.82)
            self.assertNotIn('GOD-X8', row['id'])
        self.assertEqual(base['settings']['lot_max'], 0)

    def test_broker_cannot_smuggle_strategy_axes(self):
        with self.assertRaises(ValueError):
            generate({'settings': {}}, {'lot_percent': 50}, 3, 1)

    def test_optional_families_keep_common_contract_and_native_supported_measure(self):
        base = {'settings': {'lot_max': 10, 'entry_units': 8}}
        default_before = generate(base, {}, 25, 123)
        rows = generate(base, {}, 80, 456, ('daily_bank', 'soft_regime'), 'G8E2')
        self.assertEqual({r['family'] for r in rows}, {'reference', 'daily_bank', 'soft_regime'})
        for row in rows:
            self.assertEqual(row['settings']['lot_max'], 5)
            self.assertNotIn('explicit_pending_until_cancel', row['settings'])
            if row['family'] == 'soft_regime':
                self.assertEqual(row['settings']['regime_miara'], 'Srednia')
                self.assertEqual(row['settings']['regime_gdy_rozerwany'], 'Milcz')
                self.assertTrue(row['settings']['regime_soft'])
        self.assertEqual(default_before, generate(base, {}, 25, 123))

    def test_profit_budget_space_is_opt_in_and_keeps_broker_and_lot_contract(self):
        base = {'settings': {'profit_budget_arm_pct': 0, 'entry_units': 8}}
        rows = generate(base, {'konto_dzwignia': 500}, 257, 20260907,
                        ('budget_reinvest', 'budget_soft_regime'), 'G8B')
        self.assertEqual(rows[0]['settings']['profit_budget_arm_pct'], 0)
        for row in rows[1:]:
            s = row['settings']
            self.assertGreater(s['profit_budget_arm_pct'], 0)
            self.assertTrue(0 < s['profit_budget_keep_pct'] < 100)
            self.assertTrue(0 < s['profit_budget_deploy_pct'] <= 100)
            self.assertIn(s['day_trail_stop_pct'], [0, 100-s['profit_budget_keep_pct']])
            self.assertEqual(s['lot_max'], 5)
            self.assertNotIn('explicit_pending_until_cancel', s)
            self.assertFalse(set(row['changes']) & BROKER_FIELDS)

    def test_reference_preserves_preset_management_and_ingress_policies(self):
        for protect in (False, True):
            settings = {'lot_max': 10, 'explicit_pending_until_cancel': protect,
                        'edycja_sieroty_nie_otwiera': False,
                        'pending_lifetime': 'UntilTp1', 'pending_ttl_h': 4,
                        'day_trail_basis': 'ProfitPeak', 'rearm_grid_on_return': True}
            rows = generate({'settings': settings}, {}, 10, 333, ('basket_harvest',))
            self.assertEqual(rows[0]['settings'], {**settings, 'lot_max': 5.0})
            for row in rows:
                self.assertEqual(row['settings']['explicit_pending_until_cancel'], protect)
                self.assertFalse(row['settings']['edycja_sieroty_nie_otwiera'])
                self.assertEqual(row['settings']['pending_lifetime'], 'UntilTp1')
                self.assertEqual(row['settings']['pending_ttl_h'], 4)
                self.assertEqual(row['settings']['day_trail_basis'], 'ProfitPeak')


class CompletionTests(unittest.TestCase):
    def test_monitor_read_lock_does_not_abort_jobs(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp)/'state.json'
            with patch('research_runner.os.replace', side_effect=[PermissionError('reader lock'), None]) as replace:
                write_json(target, {'active': 2})
                self.assertEqual(replace.call_count, 2)
            with patch('research_runner.write_json', side_effect=PermissionError('locked')):
                write_progress(target, {'active': 2})

    def test_complete_exit_cannot_hide_skipped_presets(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            metrics = {'total_profit': 10, 'end_equity': 610, 'start_balance': 600}
            path = root/'wyniki_compound.json'
            path.write_text(json.dumps({'A': metrics}), encoding='utf-8')
            self.assertEqual(validate_results(root, 1)[1], [])
            self.assertTrue(validate_results(root, 2)[1])
            path.write_text(json.dumps({'approximate': True, 'results': {'A': metrics}}), encoding='utf-8')
            self.assertEqual(validate_results(root, 1)[1], [])
            metrics['total_profit'] = None
            path.write_text(json.dumps({'A': metrics}), encoding='utf-8')
            self.assertTrue(validate_results(root, 1)[1])

    def test_partial_only_is_not_complete(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root/'wyniki_czastkowe.json').write_text('{}', encoding='utf-8')
            self.assertTrue(validate_results(root)[1])


if __name__ == '__main__':
    unittest.main()
