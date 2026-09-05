"""Choose five diagnostic finalists from completed, comparable exact replays.

This is a validation-informed queue for the owner's reset-account tests, not an
out-of-sample claim or a production choice. Every failed requirement is retained.
"""
from __future__ import annotations

import argparse
import json
import math
from pathlib import Path

from giga_sweep8 import fingerprint
from research_runner import file_hash
from verified_replay_report import collect

REFERENCE = 'GOD-X7-cap5'
FULL_CASES = {
    'historical_owner_recipe_full10',
    *(f'{prefix}_full_{cap}' for prefix in ('historical_latest', 'observed', 'mixed_stress')
      for cap in ('5', '10', '0p01')),
}
CHRONOLOGICAL_CASES = {'historical_latest_later5', 'observed_later5'}


def finite(value):
    return isinstance(value, (float, int)) and not isinstance(value, bool) and math.isfinite(value)


def validate_plans(full, later):
    instruments = set()
    for plan, expected, stage in ((full, FULL_CASES, 'full_comparison'),
                                  (later, CHRONOLOGICAL_CASES, 'chronological_validation')):
        if plan.get('metadata', {}).get('etap_badania') != stage or {j['id'] for j in plan['jobs']} != expected:
            raise ValueError('Only the declared full and later validation stages are accepted')
        for job in plan['jobs']:
            name = job['id']
            if stage == 'chronological_validation':
                start, end, cap = '2026-08-17', '2026-09-05', 5
            elif name == 'historical_owner_recipe_full10':
                start, end, cap = '2026-06-20', '2026-09-02', 10
            else:
                start = ('2026-06-20' if name.startswith('historical_') else
                         '2026-08-05' if name.startswith('observed_') else '2026-06-01')
                end, cap = '2026-09-05', {'5': 5, '10': 10, '0p01': .01}[name.rsplit('_', 1)[-1]]
            corpus = ('historical_reference' if name.startswith('historical_') else
                      'observed_receipts' if name.startswith('observed_') else 'mixed_missing_original_stress')
            if (job['window'] != {'from': start, 'to': end} or job['deposit'] != 600
                    or job['lot_cap'] != cap or job['signal_contract'] != corpus):
                raise ValueError('Validation window, account or source contract changed')
            argv = job['argv']
            expected_flags = {'--from': start, '--to': end, '--quick-tick-stride': '1',
                              '--signal-time-offset-min': '0'}
            for flag, value in expected_flags.items():
                if argv.count(flag) != 1 or argv.index(flag)+1 >= len(argv) or argv[argv.index(flag)+1] != value:
                    raise ValueError('Actual replay argv differs from its declared contract: ' + flag)
            if (argv.count('--balance') != 1 or argv.index('--balance')+1 >= len(argv)
                    or float(argv[argv.index('--balance')+1]) != 600):
                raise ValueError('Actual replay deposit differs from its declared account')
            if ('--live-telegram-ingress' in argv) != (corpus != 'historical_reference'):
                raise ValueError('Actual ingress mode differs from its declared source contract')
            instruments.add(str(Path(argv[0]).resolve()))
    if len(instruments) != 1:
        raise ValueError('The same immutable engine must perform both validation stages')
    a = {r['name']: r['fingerprint'] for r in full['selection']}
    b = {r['name']: r['fingerprint'] for r in later['selection']}
    if a != b:
        raise ValueError('Preset identities changed between validation stages')
    return a


def groups(report, expected_cases=None):
    if not report.get('all_cases_complete') or report.get('incomplete'):
        raise ValueError('Every requested exact case must be complete')
    result = {}
    for row in report['results']:
        name = row['metrics']['name']
        case = result.setdefault(row['job'], {})
        if name in case:
            raise ValueError('Duplicate candidate result')
        case[name] = row['metrics']
    if expected_cases is not None and set(result) != set(expected_cases):
        raise ValueError('Unexpected full-comparison cases')
    if not result or any(REFERENCE not in values for values in result.values()):
        raise ValueError('Same-window GOD-X7 reference required in every case')
    names = set(next(iter(result.values())))
    if any(set(values) != names for values in result.values()):
        raise ValueError('Candidate sets differ between exact cases')
    return result


def assess(full, chronological, count=5):
    if not 5 <= count <= 10:
        raise ValueError('The owner requires five to ten finalists')
    names = set(next(iter(full.values())))
    if any(set(values) != names for values in chronological.values()):
        raise ValueError('Chronological validation must contain the same candidates')
    baseline = full['historical_owner_recipe_full10'][REFERENCE]
    if not math.isclose(baseline['total_profit'], 1522404.67, abs_tol=.011, rel_tol=0) or baseline['trades'] != 4864:
        raise ValueError('The owner GOD-X7 benchmark has not been reproduced')
    rows = []
    for name in sorted(names - {REFERENCE}):
        failures, cautions, ratios, activities, green, drawdown = [], [], [], [], [], []
        for identity in ('historical_latest_full_5', 'observed_full_5'):
            m, ref = full[identity][name], full[identity][REFERENCE]
            for key in ('total_profit', 'end_equity', 'min_equity', 'baskets_with_closed_trades',
                        'accepted_entry_sources_pct', 'positive_market_days_pct', 'max_dd_pct'):
                if not finite(m.get(key)) or not finite(ref.get(key)):
                    raise ValueError('Missing measured metric: ' + identity + ':' + key)
            if any(not 0 <= m[key] <= 100 or not 0 <= ref[key] <= 100
                   for key in ('accepted_entry_sources_pct', 'positive_market_days_pct')):
                raise ValueError('Invalid measured percentage')
            if m['total_profit'] <= 0:
                failures.append(identity + ':nonpositive_profit')
            if m['blown'] or m['stop_outs'] or m['min_equity'] <= 0:
                failures.append(identity + ':insolvent_or_stop_out')
            for key in ('baskets_with_closed_trades', 'accepted_entry_sources_pct'):
                ratio = m[key] / max(ref[key], 1e-12)
                activities.append(ratio)
                if ratio < .7:
                    failures.append(identity + ':activity_below_70pct_reference:' + key)
            ratios.append(m['end_equity'] / max(ref['end_equity'], 1e-12))
            green.append(m['positive_market_days_pct'])
            drawdown.append(m['max_dd_pct'])
        fixed = full['historical_latest_full_0p01'][name].get('daily_concentration', {})
        without5 = fixed.get('profit_without_best_5')
        if not finite(without5):
            raise ValueError('Fixed-lot concentration evidence required')
        if without5 <= 0:
            failures.append('fixed_0p01:nonpositive_after_removing_best_5_days')
        fixed_by_history = {'historical_latest_full_0p01': without5}
        for identity in ('observed_full_0p01', 'mixed_stress_full_0p01'):
            value = (full[identity][name].get('daily_concentration') or {}).get('profit_without_best_5')
            if not finite(value):
                raise ValueError('Fixed-lot concentration evidence required: ' + identity)
            fixed_by_history[identity] = value
            if value <= 0:
                cautions.append(identity + ':nonpositive_after_removing_best_5_days')
        owner = full['historical_owner_recipe_full10'][name]
        if owner['total_profit'] <= baseline['total_profit']:
            cautions.append('owner_recipe_10:does_not_exceed_GOD_X7_profit')
        if min(green) < 90:
            cautions.append('full_cap5:below_90pct_positive_equity_days')
        for identity, values in chronological.items():
            m = values[name]
            if m['total_profit'] <= 0 or m['blown'] or m['stop_outs']:
                cautions.append(identity + ':nonpositive_or_insolvent_fresh_validation')
        for identity in sorted(FULL_CASES - {'historical_latest_full_5', 'observed_full_5'}):
            m = full[identity][name]
            if m['total_profit'] <= 0 or m['blown'] or m['stop_outs']:
                cautions.append(identity + ':nonpositive_or_insolvent')
        for identity in ('historical_owner_recipe_full10', 'historical_latest_full_10', 'observed_full_10'):
            pct = full[identity][name].get('positive_market_days_pct')
            if not finite(pct) or not 0 <= pct <= 100:
                raise ValueError('Missing or invalid cap-ten positive-day percentage')
            if pct < 90:
                cautions.append(identity + ':below_90pct_positive_equity_days')
        rows.append({'id': name, 'core_requirements_failed': failures, 'cautions': cautions,
                     'worst_capital_ratio_to_reference': min(ratios),
                     'worst_activity_ratio_to_reference': min(activities),
                     'lowest_positive_equity_days_pct': min(green), 'worst_max_dd_pct': max(drawdown),
                     'owner_recipe_profit': owner['total_profit'], 'fixed_profit_without_best_5': without5,
                     'fixed_profit_without_best_5_by_history': fixed_by_history,
                     'all_reported_targets_met': not failures and not cautions})
    if len(rows) < count:
        raise ValueError('Not enough completed candidates for the requested matrix')
    # Eligibility always takes precedence over a ranking metric. A queue can
    # include diagnostic alternatives when fewer than five clear the gates.
    def order(metric):
        return sorted(rows, key=lambda r: (len(r['core_requirements_failed']),
                      -r[metric], -r['worst_capital_ratio_to_reference'], r['id']))
    views = [
        ('paired_relative_capital', order('worst_capital_ratio_to_reference')),
        ('positive_equity_days', order('lowest_positive_equity_days_pct')),
        ('owner_benchmark_profit', order('owner_recipe_profit')),
        ('fixed_lot_outside_best_days', order('fixed_profit_without_best_5')),
    ]
    selected, reasons = {}, {}
    for index in range(len(rows)):
        for reason, ranked in views:
            row = ranked[index]
            if row['id'] not in selected and len(selected) < count:
                selected[row['id']] = row
            if row['id'] in selected:
                reasons.setdefault(row['id'], [])
                if reason not in reasons[row['id']]:
                    reasons[row['id']].append(reason)
        if len(selected) >= count:
            break
    return {'production_choice_made': False, 'uses_later_validation_for_queue': True,
            'untouched_holdout_claim': False, 'activity_floor_ratio': .7,
            'fully_met_reported_targets': sum(r['all_reported_targets_met'] for r in rows),
            'selected': [{**row, 'selection_reasons': reasons[name]} for name, row in selected.items()],
            'assessments': rows,
            'notes': ['The five-to-ten queue includes explicitly flagged diagnostic alternatives if necessary.',
                      'Full and chronological accounts remain separate; their money is never added.',
                      'Removing best days is used only for the 0.01-lot replay.',
                      'The reset-account coronation and owner decision are still required.']}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--full-plan', required=True, type=Path)
    p.add_argument('--chronological-plan', required=True, type=Path)
    p.add_argument('--space', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--count', type=int, default=5)
    args = p.parse_args()
    plans = [json.loads(path.read_text('utf-8-sig')) for path in (args.full_plan, args.chronological_plan)]
    identities = validate_plans(*plans)
    reports = [collect(args.full_plan), collect(args.chronological_plan)]
    result = assess(groups(reports[0], FULL_CASES), groups(reports[1], CHRONOLOGICAL_CASES), args.count)
    finalists = []
    for row in result['selected']:
        path = args.space/'configs'/(row['id']+'.json')
        document = json.loads(path.read_text('utf-8-sig'))
        if document['settings']['lot_max'] != 5 or fingerprint(document['settings']) != identities.get(row['id']):
            raise ValueError('Original, unchanged cap-five candidate required')
        finalists.append({'id': row['id'], 'preset_path': str(path.resolve()),
                          'fingerprint': fingerprint(document['settings'])})
    result['provenance'] = {'full_plan_sha256': reports[0]['plan_sha256'],
        'chronological_plan_sha256': reports[1]['plan_sha256'], 'selector_sha256': file_hash(Path(__file__)),
        'receipt_sha256': sorted({r['receipt_sha256'] for report in reports for r in report['results']})}
    args.out.mkdir(parents=True, exist_ok=False)
    (args.out/'assessment.json').write_text(json.dumps(result, indent=2, allow_nan=False), encoding='utf-8')
    (args.out/'finalists.json').write_text(json.dumps(finalists, indent=2), encoding='utf-8')
    print(json.dumps({'finalists': len(finalists), 'all_targets_met': result['fully_met_reported_targets'],
                      'production_choice_made': False}))


if __name__ == '__main__':
    main()
