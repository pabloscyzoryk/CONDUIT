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
            self.assertTrue(row['settings']['explicit_pending_until_cancel'])
            self.assertFalse(set(row['changes']) & BROKER_FIELDS)
            self.assertEqual(row['settings']['konto_dzwignia'], 500)
            self.assertEqual(row['settings']['swap_long_points'], -75.82)
            self.assertNotIn('GOD-X8', row['id'])
        self.assertEqual(base['settings']['lot_max'], 0)

    def test_broker_cannot_smuggle_strategy_axes(self):
        with self.assertRaises(ValueError):
            generate({'settings': {}}, {'lot_percent': 50}, 3, 1)


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
