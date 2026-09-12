"""Synthetic completed artifacts; no market, Telegram or executable launch."""
import contextlib
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import prepare_lot_growth_comparison as subject
import prepare_lot_growth_sweep as prep
import rank_lot_growth_sweep as rank
import validate_lot_growth_finalists as august
import test_validate_lot_growth_finalists as fixtures


def rewrite(path, value):
    Path(path).write_text(json.dumps(value, ensure_ascii=True, allow_nan=False), encoding='utf-8')


@contextlib.contextmanager
def restore(*paths):
    original = {Path(path): Path(path).read_bytes() for path in paths}
    try:
        yield
    finally:
        for path, data in original.items():
            path.write_bytes(data)


class ComparisonTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.root = Path(cls.temporary.name)
        cls.train = fixtures.fixture(cls.root/'train')
        cls.august = cls.root/'august'
        august.prepare(cls.train, cls.august)
        plan = prep.read(cls.august/'PLAN.json')
        for job in plan['jobs']:
            fixtures.completed(cls.august/'PLAN.json', job['id'], plan['finalists_contract']['registry'],
                               ['2026-08-03', '2026-08-04'])
        cls.final = cls.root/'final'
        august.select(cls.august, cls.final)
        cls.full = cls.root/'full'
        subject.prepare(cls.final, cls.full, cpu_budget=8)

    def destination(self):
        return self.root/self._testMethodName

    def test_full_actual_contract_is_33_independent_accounts_and_only_cap_changes(self):
        proof = subject.check(self.full)
        self.assertEqual(proof['status'], 'PASS')
        self.assertEqual(proof['total_runs'], 33)
        plan = prep.read(self.full/'PLAN.json')
        self.assertEqual([j['threads'] for j in plan['jobs']], [8]*3)
        contract = plan['comparison_contract']
        self.assertEqual(len(contract['train_shortlist_ids']), 10)
        self.assertEqual(len(contract['august_finalist_ids']), 5)
        self.assertFalse(contract['used_for_selection'])
        for job, (_, cap) in zip(plan['jobs'], subject.CAPS):
            self.assertEqual(job['expected_candidates'], 11)
            self.assertEqual(august.flag(job['argv'], '--balance'), '600')
            self.assertEqual((august.flag(job['argv'], '--from'), august.flag(job['argv'], '--to')), subject.WINDOW)
            records = contract['registry'][job['id']]
            self.assertEqual(sum(r['control'] for r in records.values()), 1)
            for name, record in records.items():
                actual, original = prep.read(record['file']['path']), prep.read(record['source']['path'])
                expected = copy.deepcopy(original)
                expected['name'] = name
                expected['settings']['lot_max'] = cap
                self.assertTrue(prep.exact_equal(actual, expected))

    def test_full_receipts_and_detail_readers_work_without_config_directory_collision(self):
        out = self.destination()
        subject.prepare(self.final, out)
        plan = prep.read(out/'PLAN.json')
        for job in plan['jobs']:
            fixtures.completed(out/'PLAN.json', job['id'], plan['comparison_contract']['registry'][job['id']],
                               ['2026-06-22', '2026-09-01'])
            summary, details = rank.verified_results(out, job['id'])
            self.assertEqual(len(summary), 11)
            self.assertEqual(len(details), 11)
        self.assertEqual(subject.check(out)['total_runs'], 33)

    def test_no_august_finalists_still_compares_frozen_train_ten(self):
        validation, final, out = (self.root/name for name in ('august_none', 'final_none', 'full_none'))
        august.prepare(self.train, validation)
        plan = prep.read(validation/'PLAN.json')
        original = fixtures.describe_fixture
        def rejected_candidates(name, balance, dates, sha, **kwargs):
            return original(name, balance, dates, sha, dd=20 if name in august.CONTROL_IDS else 40)
        with mock.patch.object(fixtures, 'describe_fixture', side_effect=rejected_candidates):
            for job in plan['jobs']:
                fixtures.completed(validation/'PLAN.json', job['id'], plan['finalists_contract']['registry'],
                                   ['2026-08-03', '2026-08-04'])
        frozen = august.select(validation, final)
        self.assertEqual(frozen['status'], 'NO_QUALIFYING_FINALIST')
        self.assertEqual(prep.read(final/'FINALISTS.json'), [])
        receipt = subject.prepare(final, out)
        self.assertEqual(receipt['august_status'], 'NO_QUALIFYING_FINALIST')
        self.assertEqual(receipt['total_runs'], 33)
        self.assertEqual(subject.check(out)['status'], 'PASS')

    def test_repaired_finalist_list_hash_cannot_promote_or_remove_candidates(self):
        file, receipt_path = self.final/'FINALISTS.json', self.final/'SELECTION_RECEIPT.json'
        with restore(file, receipt_path):
            rewrite(file, [])
            receipt = prep.read(receipt_path)
            receipt['finalists'] = prep.pin(file)
            rewrite(receipt_path, receipt)
            with self.assertRaisesRegex(ValueError, 'finalist configurations'):
                subject.prepare(self.final, self.destination())
            self.assertFalse(self.destination().exists())

    def test_typed_false_is_not_numeric_zero_even_with_repaired_receipt(self):
        file, receipt_path = self.final/'FINAL_SELECTION.json', self.final/'SELECTION_RECEIPT.json'
        with restore(file, receipt_path):
            value = prep.read(file)
            value['gates_relaxed'] = 0
            rewrite(file, value)
            receipt = prep.read(receipt_path)
            receipt['selection'] = prep.pin(file)
            rewrite(receipt_path, receipt)
            with self.assertRaisesRegex(ValueError, 'recomputed'):
                subject.prepare(self.final, self.destination())

    def test_missing_second_august_completion_prevents_any_output(self):
        receipt_path = self.august/'august_600/receipt.json'
        with restore(receipt_path):
            rewrite(receipt_path, {'status': 'running'})
            with self.assertRaises(ValueError):
                subject.prepare(self.final, self.destination())
            self.assertFalse(self.destination().exists())

    def test_missing_final_selection_receipt_prevents_any_output(self):
        path = self.final/'SELECTION_RECEIPT.json'
        with restore(path):
            rewrite(path, {})
            with self.assertRaisesRegex(ValueError, 'Final selection receipt'):
                subject.prepare(self.final, self.destination())
            self.assertFalse(self.destination().exists())

    def test_modified_completed_ledger_is_rejected(self):
        path = next((self.august/'august_600/results').glob('*_transakcje.json'))
        with restore(path):
            rewrite(path, {})
            with self.assertRaises(ValueError):
                subject.prepare(self.final, self.destination())

    def test_repaired_strategy_settings_and_hashes_are_rejected(self):
        plan_path, receipt_path = self.full/'PLAN.json', self.full/'PREPARATION_RECEIPT.json'
        plan = prep.read(plan_path)
        job_id = plan['jobs'][0]['id']
        file = Path(plan['comparison_contract']['registry'][job_id][subject.CONTROL]['file']['path'])
        with restore(file, plan_path, receipt_path):
            value = prep.read(file)
            value['settings']['risk_per_basket_pct'] += 1
            rewrite(file, value)
            new_pin = prep.pin(file)
            plan['inputs'] = [new_pin if p['path'] == new_pin['path'] else p for p in plan['inputs']]
            record = plan['comparison_contract']['registry'][job_id][subject.CONTROL]
            record.update(file=new_pin, settings_sha256=prep.fingerprint(value['settings']))
            rewrite(plan_path, plan)
            rewrite(receipt_path, subject.expected_receipt(plan_path, plan))
            with self.assertRaisesRegex(ValueError, 'strategy setting'):
                subject.check(self.full)

    def test_repaired_window_change_is_not_accepted_as_full_owner_recipe(self):
        plan_path, receipt_path = self.full/'PLAN.json', self.full/'PREPARATION_RECEIPT.json'
        with restore(plan_path, receipt_path):
            plan = prep.read(plan_path)
            argv = plan['jobs'][0]['argv']
            argv[argv.index('--from')+1] = '2026-06-22'
            rewrite(plan_path, plan)
            rewrite(receipt_path, subject.expected_receipt(plan_path, plan))
            # This remains a legal generic runner window; the new full contract rejects it.
            subject.sizing.validate_plan_inputs(plan)
            with self.assertRaisesRegex(ValueError, 'Full owner window'):
                subject.check(self.full)

    def test_reference_flags_are_preserved_without_extra_ingress_offset(self):
        plan = prep.read(self.full/'PLAN.json')
        reference = prep.read(plan['qualification']['reference_binding']['path'])['cases'][1]['argv_exact']
        mutable = {reference.index(k)+1 for k in ('--sweep', '--out', '--from', '--to', '--balance')}
        for job in plan['jobs']:
            self.assertEqual(len(reference), len(job['argv']))
            for index, (before, after) in enumerate(zip(reference, job['argv'])):
                if index not in mutable:
                    self.assertEqual(before, after)

    def test_output_reuse_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'already exists'):
            subject.prepare(self.final, self.full)

    def test_cli_help_is_offline_and_windows_console_safe(self):
        env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', PYTHONIOENCODING='cp1250')
        result = subprocess.run([sys.executable, subject.__file__, '--help'], env=env,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20, check=False)
        self.assertEqual(result.returncode, 0, result.stderr.decode('cp1250'))
        self.assertIn(b'full', result.stdout)
        self.assertIn(b'check', result.stdout)


if __name__ == '__main__':
    unittest.main()
