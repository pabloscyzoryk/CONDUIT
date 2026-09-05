import contextlib
import copy
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import research_runner_exact8 as exact
from training_contract8 import WINDOWS, require_exact_training, require_screening_selection


class TrainingBoundaryTests(unittest.TestCase):
    def plan(self):
        return {'metadata': {'status_walidacji': 'training_confirmation', 'tryb_obliczen': 'exact'},
            'protocol': {'parameter_search': True},
            'jobs': [{'id': key+'_exact_train', 'window': {'from': window[0], 'to': window[1]},
                'argv': ['btp', '--from', window[0], '--to', window[1], '--quick-tick-stride', '1']}
                for key, window in WINDOWS.items()]}

    def test_names_do_not_authorize_other_dates_stage_or_command(self):
        require_exact_training(self.plan())
        mutations = [lambda p: p['metadata'].update(status_walidacji='full_comparison'),
            lambda p: p['jobs'][0]['window'].update(to='2099-01-01'),
            lambda p: p['jobs'][0]['argv'].__setitem__(4, '2099-01-01')]
        for mutate in mutations:
            plan = self.plan()
            mutate(plan)
            with self.assertRaises(ValueError):
                require_exact_training(plan)

    def test_neighbors_require_screening_boundaries(self):
        valid = {'screening_contract': {'schema': 'conduit.training-boundaries.v1',
            'stage': 'screening', 'windows': WINDOWS, 'deposit': 600, 'lot_cap': 5,
            'later_outcomes_used': False}, 'stage': 'screening'}
        require_screening_selection(valid)
        for bad in ({}, {**valid, 'stage': 'chronological_validation'}):
            with self.assertRaises(ValueError):
                require_screening_selection(bad)


class CompletionBindingTests(unittest.TestCase):
    def run_child(self, root, missing=False):
        results = root/'results'
        results.mkdir()
        child = root/'child.py'
        child.write_text('from pathlib import Path\nimport json\np=Path('+repr(str(results))+')\n'
            +'(p/"wyniki_compound.json").write_text(json.dumps({"candidate": {"start_balance": 600, "total_profit": 1, "end_equity": 601}}))\n'
            +('' if missing else '(p/"candidate_compound_dane.json").write_text("{\\"krzywa\\": []}")\n'), encoding='utf-8')
        plan_path = root/'plan.json'
        plan = {'id': 'binding_test', 'output': str(root/'run'), 'progress_dir': str(root/'progress'),
            'threads': 1, 'jobs': [{'id': 'case', 'argv': [sys.executable, str(child)],
                'result_dir': str(results), 'expected_candidates': 1}],
            'inputs': [{'path': str(p.resolve()), 'sha256': exact.runner.file_hash(p)}
                for p in (Path(exact.__file__), Path(exact.runner.__file__), child)]}
        plan_path.write_text(json.dumps(plan), encoding='utf-8')
        with patch.object(sys, 'argv', ['runner', str(plan_path)]), contextlib.redirect_stdout(io.StringIO()):
            code = exact.main()
        receipt = json.loads((root/'run/case/receipt.json').read_text('utf-8'))
        return code, receipt, results/'candidate_compound_dane.json'

    def test_completed_process_binds_history_and_detects_tampering(self):
        with tempfile.TemporaryDirectory() as folder:
            code, receipt, detail = self.run_child(Path(folder))
            self.assertEqual(code, 0)
            exact.verify_detail_binding(detail, receipt)
            detail.write_text('{"different": true}', encoding='utf-8')
            with self.assertRaisesRegex(ValueError, 'changed'):
                exact.verify_detail_binding(detail, receipt)

    def test_exit_zero_missing_history_is_failed(self):
        with tempfile.TemporaryDirectory() as folder:
            code, receipt, _ = self.run_child(Path(folder), missing=True)
            self.assertEqual(code, 1)
            self.assertEqual(receipt['status'], 'failed')
            self.assertTrue(receipt['validation_errors'])


if __name__ == '__main__':
    unittest.main()
