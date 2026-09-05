"""Select an exact-replay queue that survives two separately reported corpora.

Overlapping training histories are never added or described as independent
out-of-sample evidence. This produces a research queue, not a production preset.
"""
from __future__ import annotations
import argparse
from collections import Counter, defaultdict
import json
import math
from pathlib import Path

from rank_giga_sweep8 import collect
from research_runner import file_hash
from training_contract8 import screening_contract


def finite_metric(value):
    return isinstance(value, (float, int)) and not isinstance(value, bool) and math.isfinite(value)


def validate_instruments(*provenances):
    hashes = set()
    for provenance in provenances:
        receipts = provenance.get('receipts', [])
        if not receipts:
            raise ValueError('Both corpora require completed instrument receipts')
        for receipt in receipts:
            digest = receipt.get('exe_sha256')
            if not isinstance(digest, str) or len(digest) != 64 or any(c not in '0123456789abcdef' for c in digest.lower()):
                raise ValueError('A valid instrument SHA256 is required in every receipt')
            hashes.add(digest.lower())
    if len(hashes) != 1:
        raise ValueError('Historical and observed corpora must use the same research instrument')


def select_dual(historical, observed, count=48, activity_ratio=.7):
    if count < 2 or not 0 <= activity_ratio <= 1:
        raise ValueError('Invalid selection size or activity ratio')
    reference_id = 'GOD-X7-cap5'
    if reference_id not in historical or reference_id not in observed:
        raise ValueError('Both completed reference runs are required')
    references = {'historical': historical[reference_id], 'observed': observed[reference_id]}
    metric_keys = ('total_profit', 'end_equity', 'filled_baskets', 'accepted_entry_sources_pct', 'min_equity', 'positive_market_days_pct')
    for corpus, reference in references.items():
        if any(not finite_metric(reference.get(k)) for k in metric_keys):
            raise ValueError(corpus + ':reference_requires_measured_acceptance_and_equity_days')
        if any(not 0 <= reference[k] <= 100 for k in ('accepted_entry_sources_pct', 'positive_market_days_pct')):
            raise ValueError(corpus + ':invalid_reference_percentage')
    shared = historical.keys() & observed.keys()
    pool, rejected, paired = [], Counter(), {}
    for name in sorted(shared):
        a, b = historical[name], observed[name]
        if a['family'] != b['family'] or a['fingerprint'] != b['fingerprint']:
            raise ValueError('The paired candidate settings differ')
        values = {'historical': a, 'observed': b}
        reasons, capital_ratios, activity, green = [], [], [], []
        for corpus, row in values.items():
            ref = references[corpus]
            if any(not finite_metric(row.get(k)) for k in metric_keys):
                reasons.append(corpus + ':missing_or_invalid_metrics')
                continue
            if any(not 0 <= row[k] <= 100 for k in ('accepted_entry_sources_pct', 'positive_market_days_pct')):
                reasons.append(corpus + ':invalid_percentage')
                continue
            if row['total_profit'] <= 0:
                reasons.append(corpus + ':nonpositive_profit')
            if row['blown'] or row['stop_outs'] or row['min_equity'] <= 0:
                reasons.append(corpus + ':insolvent_or_stop_out')
            for key in ('filled_baskets', 'accepted_entry_sources_pct'):
                if row[key] < ref[key] * activity_ratio:
                    reasons.append(corpus + ':below_activity_floor')
                activity.append(row[key] / max(ref[key], 1e-12))
            capital_ratios.append(row['end_equity'] / max(ref['end_equity'], 1e-12))
            green.append(row['positive_market_days_pct'])
        item = {'name': name, 'family': a['family'], 'fingerprint': a['fingerprint'],
                'corpora': values, 'screening_rejections': sorted(set(reasons)),
                'worst_capital_ratio_to_reference': min(capital_ratios, default=0),
                'worst_activity_ratio_to_reference': min(activity, default=0),
                'lowest_positive_equity_days_pct': min(green) if len(green) == 2 else None}
        paired[name] = item
        if reasons:
            rejected.update(set(reasons))
        else:
            pool.append(item)
    selected, why = {}, defaultdict(list)

    def add(item, reason):
        if item['name'] not in selected and len(selected) >= count:
            return
        selected[item['name']] = item
        if reason not in why[item['name']]:
            why[item['name']].append(reason)

    add(paired[reference_id], 'mandatory_paired_reference')
    by_family = defaultdict(list)
    for item in pool:
        by_family[item['family']].append(item)
    for family, members in sorted(by_family.items()):
        add(max(members, key=lambda r: r['worst_capital_ratio_to_reference']),
            'family_relative_capital_leader')
        add(max(members, key=lambda r: (r['lowest_positive_equity_days_pct'],
                                       r['worst_capital_ratio_to_reference'])),
            'family_positive_equity_day_leader')
    views = [
        ('relative_capital', sorted(pool, key=lambda r: r['worst_capital_ratio_to_reference'], reverse=True)),
        ('positive_equity_days', sorted(pool, key=lambda r: (r['lowest_positive_equity_days_pct'], r['worst_capital_ratio_to_reference']), reverse=True)),
        ('activity', sorted(pool, key=lambda r: (r['worst_activity_ratio_to_reference'], r['worst_capital_ratio_to_reference']), reverse=True)),
        ('historical_profit', sorted(pool, key=lambda r: r['corpora']['historical']['total_profit'], reverse=True)),
        ('observed_profit', sorted(pool, key=lambda r: r['corpora']['observed']['total_profit'], reverse=True)),
    ]
    for i in range(len(pool)):
        for reason, queue in views:
            add(queue[i], reason)
        if len(selected) >= count:
            break
    return {'coronation_eligible': False, 'paired_candidates': len(shared),
            'unpaired_historical': sorted(historical.keys() - observed.keys()),
            'unpaired_observed': sorted(observed.keys() - historical.keys()),
            'profitable_active_on_both': len(pool), 'rejection_counts': dict(rejected),
            'activity_ratio_of_each_reference': activity_ratio,
            'activity_metrics': ['filled_baskets', 'accepted_entry_sources_pct'],
            'positive_day_basis': 'all_market_equity_days',
            'selected_for_exact_replay': [{**item, 'selection_reasons': why[name]}
                                           for name, item in selected.items()],
            'notes': ['Results remain separate; overlapping windows are never added together.',
                      'Historical publication-final data preserves the reference assumption; observed receipts are a distinct causal evidence set.',
                      'This training queue does not use later chronological-validation outcomes.',
                      'Source acceptance is the unique accepted entry-source measurement; legacy signal_utilization_pct is only a closed-basket proxy.',
                      'Positive equity-day ranking requires a measured all-market-day percentage; days with closing trades are never substituted.',
                      'Mixed final-only/observed history is reserved for an explicit missing-original stress test.',
                      'Exact full-window replay, activity, concentration at 0.01 and owner coronation remain required.']}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--plan', required=True, type=Path)
    p.add_argument('--manifest', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--count', type=int, default=48)
    p.add_argument('--min-activity-ratio', type=float, default=.7)
    args = p.parse_args()
    contract = screening_contract(json.loads(args.plan.read_text('utf-8-sig')))
    a, pa = collect(args.plan, args.manifest, prefix='historical_screen_')
    b, pb = collect(args.plan, args.manifest, prefix='observed_screen_')
    validate_instruments(pa, pb)
    result = select_dual(a, b, args.count, args.min_activity_ratio)
    result['screening_contract'] = contract
    result['stage'] = 'screening'
    result['provenance'] = {'historical': pa, 'observed': pb,
                            'ranker_sha256': file_hash(Path(__file__))}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, ensure_ascii=False, indent=2, allow_nan=False), encoding='utf-8')
    print(json.dumps({'paired_candidates': result['paired_candidates'],
                      'profitable_active_on_both': result['profitable_active_on_both'],
                      'exact_queue': len(result['selected_for_exact_replay'])}))


if __name__ == '__main__':
    main()
