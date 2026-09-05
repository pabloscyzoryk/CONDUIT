import json
from pathlib import Path
import tempfile
import unittest

from research_runner import file_hash
from verified_replay_report import collect


class VerifiedReplayTests(unittest.TestCase):
    def fixture(self, root):
        exe = root / 'synthetic.bin'
        exe.write_bytes(b'synthetic verification fixture')
        result_dir = root / 'one/results'
        result_dir.mkdir(parents=True)
        result = result_dir / 'wyniki_compound.json'
        row = {'start_balance': 600, 'end_equity': 800, 'total_profit': 200,
               'baskets': 4, 'stat_sygnalow': {'koszyki': {'total': 600, 'z_pozycjami': 300}}}
        result.write_text(json.dumps({'synthetic': row}), encoding='utf-8')
        plan = root / 'plan.json'
        plan.write_text(json.dumps({'output': str(root), 'source_revision': 'synthetic', 'jobs': [
            {'id': 'one', 'argv': [str(exe)], 'result_dir': str(result_dir),
             'expected_candidates': 1, 'lot_cap': 10, 'deposit': 600}]}), encoding='utf-8')
        receipt_path = root / 'one/receipt.json'
        receipt = {'status': 'complete', 'plan_sha256': file_hash(plan), 'returncode': 0,
                   'partial': False, 'source_revision': 'synthetic', 'exe_sha256': file_hash(exe),
                   'result_files': [str(result)], 'result_sha256': {result.name: file_hash(result)}}
        receipt_path.write_text(json.dumps(receipt), encoding='utf-8')
        return plan, result, receipt_path, receipt, row

    def test_uses_cumulative_ledger_count_and_marks_incomplete_without_zeros(self):
        with tempfile.TemporaryDirectory() as temp:
            plan, result, receipt_path, receipt, row = self.fixture(Path(temp))
            report = collect(plan)
            self.assertTrue(report['all_cases_complete'])
            self.assertEqual(report['results'][0]['metrics']['created_baskets'], 600)
            self.assertEqual(report['results'][0]['metrics']['engine_reported_baskets'], 4)
            receipt['status'] = 'running'
            receipt_path.write_text(json.dumps(receipt), encoding='utf-8')
            report = collect(plan)
            self.assertFalse(report['all_cases_complete'])
            self.assertEqual(report['results'], [])

    def test_compound_exclusion_is_rejected_even_in_a_valid_receipt(self):
        with tempfile.TemporaryDirectory() as temp:
            plan, result, receipt_path, receipt, row = self.fixture(Path(temp))
            row['daily_concentration'] = {'profit_without_best_1': 100}
            result.write_text(json.dumps({'synthetic': row}), encoding='utf-8')
            receipt['result_sha256'][result.name] = file_hash(result)
            receipt_path.write_text(json.dumps(receipt), encoding='utf-8')
            with self.assertRaisesRegex(ValueError, 'prohibited'):
                collect(plan)

    def test_modified_result_cannot_enter_exact_comparison(self):
        with tempfile.TemporaryDirectory() as temp:
            plan, result, *_ = self.fixture(Path(temp))
            result.write_text('{}', encoding='utf-8')
            with self.assertRaisesRegex(ValueError, 'changed after completion'):
                collect(plan)


if __name__ == '__main__':
    unittest.main()
