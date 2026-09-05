import math
import unittest

from choose_coronation_queue8 import assess, groups, validate_plans, FULL_CASES, CHRONOLOGICAL_CASES, REFERENCE


def metric(name, profit=2000):
    return {'name': name, 'total_profit': profit, 'end_equity': 600+profit,
            'min_equity': 550, 'baskets_with_closed_trades': 500,
            'accepted_entry_sources_pct': 80, 'positive_market_days_pct': 90,
            'max_dd_pct': 20, 'blown': False, 'stop_outs': 0, 'trades': 4864,
            'daily_concentration': {'profit_without_best_5': 300}}


def fixture():
    names = [REFERENCE, *(f'C{i}' for i in range(7))]
    full = {case: {n: metric(n) for n in names} for case in FULL_CASES}
    full['historical_owner_recipe_full10'][REFERENCE] = metric(REFERENCE, 1522404.67)
    for i, name in enumerate(names[1:]):
        full['historical_owner_recipe_full10'][name] = metric(name, 1600000+i)
    later = {case: {n: metric(n) for n in names} for case in CHRONOLOGICAL_CASES}
    return full, later


class CoronationQueueTests(unittest.TestCase):
    def test_five_distinct_without_production_choice(self):
        a,b=fixture(); result=assess(a,b)
        self.assertEqual(len({r['id'] for r in result['selected']}),5)
        self.assertNotIn(REFERENCE,{r['id'] for r in result['selected']})
        self.assertFalse(result['production_choice_made'])
        self.assertFalse(result['untouched_holdout_claim'])

    def test_broken_owner_reference_cannot_qualify_candidates(self):
        a,b=fixture(); a['historical_owner_recipe_full10'][REFERENCE]['total_profit']=160
        with self.assertRaisesRegex(ValueError,'benchmark'): assess(a,b)

    def test_reference_trade_count_checked(self):
        a,b=fixture(); a['historical_owner_recipe_full10'][REFERENCE]['trades']=4863
        with self.assertRaisesRegex(ValueError,'benchmark'): assess(a,b)

    def test_few_basket_jackpot_cannot_displace_eligible_rows(self):
        a,b=fixture()
        for case in ('historical_latest_full_5','observed_full_5'):
            a[case]['C0'].update(baskets_with_closed_trades=20,end_equity=1e12,total_profit=1e12)
        result=assess(a,b)
        self.assertNotIn('C0',{r['id'] for r in result['selected']})
        row=next(r for r in result['assessments'] if r['id']=='C0')
        self.assertTrue(any('activity' in x for x in row['core_requirements_failed']))

    def test_failed_chronology_remains_visible(self):
        a,b=fixture(); b['observed_later5']['C0']['total_profit']=-1
        row=next(r for r in assess(a,b)['assessments'] if r['id']=='C0')
        self.assertTrue(any('observed_later5' in x for x in row['cautions']))
        self.assertFalse(row['all_reported_targets_met'])

    def test_every_completed_full_case_can_disprove_targets(self):
        for case in FULL_CASES:
            a,b=fixture(); a[case]['C0'].update(blown=True,stop_outs=1)
            row=next(r for r in assess(a,b)['assessments'] if r['id']=='C0')
            self.assertFalse(row['all_reported_targets_met'],case)
            self.assertTrue(row['core_requirements_failed'] or row['cautions'])

    def test_cap_ten_green_days_are_part_of_target(self):
        a,b=fixture(); a['historical_latest_full_10']['C0']['positive_market_days_pct']=55
        row=next(r for r in assess(a,b)['assessments'] if r['id']=='C0')
        self.assertFalse(row['all_reported_targets_met'])
        self.assertTrue(any('full_10:below_90' in x for x in row['cautions']))

    def test_no_best_day_subtraction_from_compounding(self):
        a,b=fixture(); a['historical_latest_full_10']['C0']['daily_concentration']['profit_without_best_5']=-1e15
        row=next(r for r in assess(a,b)['assessments'] if r['id']=='C0')
        self.assertEqual(row['fixed_profit_without_best_5'],300)

    def test_unknown_concentration_is_not_zero(self):
        a,b=fixture(); a['historical_latest_full_0p01']['C0']['daily_concentration']={}
        with self.assertRaisesRegex(ValueError,'concentration'): assess(a,b)

    def test_observed_and_stress_fixed_lot_concentration_stays_visible(self):
        for case in ('observed_full_0p01', 'mixed_stress_full_0p01'):
            a,b=fixture(); a[case]['C0']['daily_concentration']['profit_without_best_5']=-12
            row=next(r for r in assess(a,b)['assessments'] if r['id']=='C0')
            self.assertEqual(row['fixed_profit_without_best_5_by_history'][case],-12)
            self.assertTrue(any(case+':nonpositive_after_removing_best_5' in x for x in row['cautions']))
            self.assertFalse(row['all_reported_targets_met'])
            self.assertEqual(row['core_requirements_failed'],[])

    def test_nan_metrics_rejected(self):
        a,b=fixture(); a['observed_full_5']['C0']['positive_market_days_pct']=math.nan
        with self.assertRaisesRegex(ValueError,'metric'): assess(a,b)

    def test_invalid_percentage_rejected(self):
        a,b=fixture(); a['observed_full_5']['C0']['positive_market_days_pct']=101
        with self.assertRaisesRegex(ValueError,'percentage'): assess(a,b)

    def test_loss_outside_best_days_disqualifies_jackpot(self):
        a,b=fixture()
        a['historical_latest_full_0p01']['C0']['daily_concentration']['profit_without_best_5']=-1
        a['historical_owner_recipe_full10']['C0']['total_profit']=1e12
        result=assess(a,b)
        self.assertNotIn('C0',{r['id'] for r in result['selected']})
        self.assertTrue(next(r for r in result['assessments'] if r['id']=='C0')['core_requirements_failed'])

    def test_diagnostic_fallback_preserves_failures(self):
        a,b=fixture()
        for name,m in a['observed_full_5'].items():
            if name!=REFERENCE: m['total_profit']=-1
        result=assess(a,b)
        self.assertEqual(len(result['selected']),5)
        self.assertTrue(all(r['core_requirements_failed'] for r in result['selected']))
        self.assertEqual(result['fully_met_reported_targets'],0)

    def test_partial_or_mismatched_sets_rejected(self):
        with self.assertRaisesRegex(ValueError,'complete'): groups({'all_cases_complete':False})
        a,b=fixture(); del b['observed_later5']['C0']
        with self.assertRaisesRegex(ValueError,'same candidates'): assess(a,b)

    def test_duplicate_receipt_row_rejected(self):
        row={'job':'one','metrics':metric(REFERENCE)}
        with self.assertRaisesRegex(ValueError,'Duplicate'):
            groups({'all_cases_complete':True,'incomplete':[],'results':[row,row]})


if __name__=='__main__': unittest.main()
