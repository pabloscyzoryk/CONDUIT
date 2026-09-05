"""Narrow a completed paired exact training replay to a full-comparison queue.

This is not a production selection. Global leaders are retained before family
diversity so a short queue cannot depend on alphabetical family ordering.
"""
from __future__ import annotations
import argparse
from collections import defaultdict
import json
from pathlib import Path

from rank_dual_sweep8 import select_dual
from research_runner import file_hash
from verified_replay_report import collect
from training_contract8 import require_exact_training


def choose(historical, observed, count=12, activity_ratio=.7):
    if count < 6:
        raise ValueError('At least six places including the reference required')
    paired = select_dual(historical, observed, max(2, len(historical)), activity_ratio)
    pool = [r for r in paired['selected_for_exact_replay'] if not r['screening_rejections']]
    reference = next(r for r in paired['selected_for_exact_replay'] if r['name'] == 'GOD-X7-cap5')
    selected, reasons = {}, defaultdict(list)

    def add(row, reason):
        name = row['name']
        if name not in selected and len(selected) >= count:
            return
        selected[name] = row
        if reason not in reasons[name]:
            reasons[name].append(reason)

    add(reference, 'mandatory_exact_reference')
    views = [
        ('paired_capital', sorted(pool, key=lambda r: (-r['worst_capital_ratio_to_reference'], r['name']))),
        ('positive_equity_days', sorted(pool, key=lambda r: (-r['lowest_positive_equity_days_pct'], -r['worst_capital_ratio_to_reference'], r['name']))),
        ('historical_profit', sorted(pool, key=lambda r: (-r['corpora']['historical']['total_profit'], r['name']))),
        ('observed_profit', sorted(pool, key=lambda r: (-r['corpora']['observed']['total_profit'], r['name']))),
        ('activity', sorted(pool, key=lambda r: (-r['worst_activity_ratio_to_reference'], -r['worst_capital_ratio_to_reference'], r['name']))),
    ]
    for label, queue in views:
        if queue:
            add(queue[0], 'global_' + label)
    families = defaultdict(list)
    for row in pool:
        families[row['family']].append(row)
    leaders = [max(rows, key=lambda r: (r['worst_capital_ratio_to_reference'], r['name'])) for rows in families.values()]
    for row in sorted(leaders, key=lambda r: (-r['worst_capital_ratio_to_reference'], r['name'])):
        if len(selected) >= max(6, count * 2 // 3):
            break
        if row['family'] not in {r['family'] for r in selected.values()}:
            add(row, 'additional_family_leader')
    for index in range(len(pool)):
        for label, queue in views:
            add(queue[index], 'exact_' + label)
        if len(selected) >= count:
            break
    return {**{k: v for k, v in paired.items() if k != 'selected_for_exact_replay'},
            'selected_for_exact_replay': [{**row, 'selection_reasons': reasons[name]} for name, row in selected.items()],
            'stage': 'exact_training_to_full_comparison',
            'production_choice_made': False}


def from_verified_report(report, plan, count=12):
    if not report['all_cases_complete']:
        raise ValueError('All paired exact training cases must be complete')
    require_exact_training(plan)
    expected = {'historical_exact_train': 'historical_reference', 'observed_exact_train': 'observed_receipts'}
    if {j['id'] for j in plan['jobs']} != set(expected):
        raise ValueError('Only the two predeclared training windows can narrow this queue')
    for job in plan['jobs']:
        if job['lot_cap'] != 5 or job['deposit'] != 600 or job['signal_contract'] != expected[job['id']]:
            raise ValueError('Paired training must retain cap5, deposit600 and the source contract')
    identities = {r['name']: r for r in plan['selection']}
    groups = {name: {} for name in expected}
    for row in report['results']:
        metrics = row['metrics']
        name = metrics['name']
        if name not in identities or name in groups[row['job']]:
            raise ValueError('Unexpected or duplicate exact candidate')
        groups[row['job']][name] = {**metrics, 'filled_baskets': metrics['baskets_with_closed_trades'],
            'family': identities[name]['family'], 'fingerprint': identities[name]['fingerprint'], 'approximate': False}
    if any(set(rows) != set(identities) for rows in groups.values()):
        raise ValueError('Paired exact training candidate sets differ')
    return choose(groups['historical_exact_train'], groups['observed_exact_train'], count)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--plan', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--count', type=int, default=12)
    args = p.parse_args()
    plan = json.loads(args.plan.read_text('utf-8-sig'))
    report = collect(args.plan)
    result = from_verified_report(report, plan, args.count)
    result['provenance'] = {'plan_sha256': report['plan_sha256'], 'ranker_sha256': file_hash(Path(__file__)),
        'exact_receipts': sorted({r['receipt_sha256'] for r in report['results']}),
        'exact_results': sorted({r['result_sha256'] for r in report['results']})}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2, ensure_ascii=False, allow_nan=False), encoding='utf-8')
    print(json.dumps({'paired_exact': result['paired_candidates'], 'surviving_exact': result['profitable_active_on_both'],
        'full_comparison_queue': len(result['selected_for_exact_replay'])}))


if __name__ == '__main__':
    main()
