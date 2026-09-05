import copy
import unittest
import contextlib
import io
import tempfile
from pathlib import Path
from unittest.mock import patch

from rank_dual_sweep8 import select_dual, validate_instruments, main
from research_metrics import compact


def row(name, profit=100, filled=100, green=70):
    return {'name': name, 'family': 'reference' if name == 'GOD-X7-cap5' else 'test',
            'fingerprint': name, 'total_profit': profit, 'end_equity': 600 + profit,
            'filled_baskets': filled, 'signal_utilization_pct': filled * .8,
            'accepted_entry_sources_pct': filled * .8,
            'min_equity': 400, 'blown': False, 'stop_outs': 0,
            'positive_market_days_pct': green, 'win_days_pct': green}


class DualRankingTests(unittest.TestCase):
    def test_one_corpus_jackpot_cannot_hide_failure_in_the_other(self):
        a = {n: row(n) for n in ['GOD-X7-cap5', 'robust', 'jackpot']}
        b = copy.deepcopy(a)
        a['jackpot'] = row('jackpot', profit=1_000_000)
        b['jackpot'] = row('jackpot', profit=-100)
        result = select_dual(a, b)
        selected = {r['name'] for r in result['selected_for_exact_replay']}
        self.assertEqual(selected, {'GOD-X7-cap5', 'robust'})
        self.assertEqual(result['rejection_counts']['observed:nonpositive_profit'], 1)
        self.assertFalse(result['coronation_eligible'])

    def test_high_activity_in_one_corpus_cannot_hide_sparse_other_history(self):
        a = {n: row(n) for n in ['GOD-X7-cap5', 'sparse']}
        b = copy.deepcopy(a)
        b['sparse'] = row('sparse', profit=10000, filled=20)
        result = select_dual(a, b)
        self.assertEqual([r['name'] for r in result['selected_for_exact_replay']], ['GOD-X7-cap5'])

    def test_missing_pair_and_settings_mismatch_are_not_merged(self):
        a = {n: row(n) for n in ['GOD-X7-cap5', 'pending_other_corpus']}
        b = {'GOD-X7-cap5': row('GOD-X7-cap5')}
        result = select_dual(a, b)
        self.assertEqual(result['unpaired_historical'], ['pending_other_corpus'])
        self.assertEqual(result['paired_candidates'], 1)
        ref = result['selected_for_exact_replay'][0]
        self.assertEqual(ref['corpora']['historical']['total_profit'], 100)
        self.assertEqual(ref['corpora']['observed']['total_profit'], 100)
        self.assertNotIn('total_profit', ref)
        b['GOD-X7-cap5']['fingerprint'] = 'different'
        with self.assertRaises(ValueError):
            select_dual(a, b)

    def test_missing_equity_measurement_is_not_replaced_with_days_having_closes(self):
        a = {n: row(n) for n in ['GOD-X7-cap5', 'legacy']}
        b = copy.deepcopy(a)
        a['legacy']['positive_market_days_pct'] = None
        a['legacy']['win_days_pct'] = 100
        result = select_dual(a, b)
        self.assertEqual([r['name'] for r in result['selected_for_exact_replay']], ['GOD-X7-cap5'])
        self.assertIn('historical:missing_or_invalid_metrics', result['rejection_counts'])
        a['GOD-X7-cap5']['positive_market_days_pct'] = None
        with self.assertRaisesRegex(ValueError, 'equity_days'):
            select_dual(a, b)

    def test_actual_source_acceptance_cannot_be_replaced_by_the_closed_basket_proxy(self):
        a = {n: row(n) for n in ['GOD-X7-cap5', 'sparse', 'missing']}
        b = copy.deepcopy(a)
        a['sparse']['signal_utilization_pct'] = 1000
        a['sparse']['accepted_entry_sources_pct'] = 10
        a['missing']['accepted_entry_sources_pct'] = None
        result = select_dual(a, b)
        self.assertEqual([r['name'] for r in result['selected_for_exact_replay']], ['GOD-X7-cap5'])
        self.assertEqual(result['activity_metrics'], ['filled_baskets', 'accepted_entry_sources_pct'])

    def test_cross_corpus_instrument_identity_is_required_even_for_one_receipt_each(self):
        a = {'receipts': [{'exe_sha256': 'a' * 64}]}
        validate_instruments(a, copy.deepcopy(a))
        for b in [{'receipts': [{'exe_sha256': 'b' * 64}]}, {'receipts': []}, {'receipts': [{}]}]:
            with self.assertRaises(ValueError):
                validate_instruments(a, b)

    def test_main_refuses_cross_corpus_engine_mismatch_before_writing_a_ranking(self):
        rows = {'GOD-X7-cap5': row('GOD-X7-cap5')}
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'ranking.json'
            provenance = lambda digest: {'receipts': [{'exe_sha256': digest * 64}]}
            with patch('rank_dual_sweep8.collect', side_effect=[(rows, provenance('a')), (rows, provenance('b'))]), \
                 patch('sys.argv', ['rank_dual_sweep8', '--plan', 'synthetic-plan.json', '--manifest', 'synthetic-manifest.json', '--out', str(output)]), \
                 contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaisesRegex(ValueError, 'same research instrument'):
                    main()
            self.assertFalse(output.exists())

    def test_compact_preserves_both_activity_denominators_and_missing_observation(self):
        metrics = {'entry_source_observation_version': 1,
                   'stat_sygnalow': {'lejek': {'accepted_entry_sources_pct': 91, 'wykonanych_pct': 72}}}
        result = compact('candidate', metrics)
        self.assertEqual(result['accepted_entry_sources_pct'], 91)
        self.assertEqual(result['closed_basket_to_entry_sources_pct'], 72)
        self.assertEqual(result['signal_utilization_basis'], 'legacy_closed_baskets_per_entry_source_proxy')
        metrics['entry_source_observation_version'] = 0
        self.assertIsNone(compact('legacy', metrics)['accepted_entry_sources_pct'])
        metrics['entry_source_observation_version'] = 1
        metrics['stat_sygnalow']['lejek'].pop('accepted_entry_sources_pct')
        self.assertIsNone(compact('legacy', metrics)['accepted_entry_sources_pct'])


if __name__ == '__main__':
    unittest.main()
