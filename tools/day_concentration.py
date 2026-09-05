"""Describe ordinary-day performance and concentration without hindsight trading.

The owner's best-day exclusion diagnostic is enabled only at max lot 0.01.
It subtracts ledger PnL; it does not rerun the account or later order state.
Larger caps report actual cash concentration and percentage returns only.
"""
from __future__ import annotations
import argparse
import datetime as dt
import hashlib
import json
import math
from pathlib import Path
import statistics


def analyze_days(days: list[dict], max_lot: float, mode: str) -> dict:
    if mode not in ('continuous', 'independent_days'):
        raise ValueError('Explicit account mode required')
    if not math.isfinite(max_lot) or max_lot < 0:
        raise ValueError('Invalid lot cap')
    if not days:
        raise ValueError('No daily observations')
    dates = [dt.date.fromisoformat(d['date']) for d in days]
    if dates != sorted(set(dates)):
        raise ValueError('Daily observations must have unique ascending dates')
    for day in days:
        for key in ('profit', 'start_equity', 'end_equity'):
            if not isinstance(day.get(key), (int, float)) or not math.isfinite(day[key]):
                raise ValueError(f'Invalid daily {key}')
        if abs(day['end_equity'] - day['start_equity'] - day['profit']) > max(.02, abs(day['profit']) * 1e-8):
            raise ValueError('Daily equity and PnL do not reconcile')
    pnl = [d['profit'] for d in days]
    winners = sorted((d for d in days if d['profit'] > 0), key=lambda d: d['profit'], reverse=True)
    gross = sum(d['profit'] for d in winners)
    returns = [d['profit'] / d['start_equity'] * 100 for d in days if d['start_equity'] > 0]
    months: dict[str, float] = {}
    weeks: dict[str, float] = {}
    for date, day in zip(dates, days):
        month = date.strftime('%Y-%m')
        week = (date - dt.timedelta(days=date.weekday())).isoformat()
        months[month] = months.get(month, 0) + day['profit']
        weeks[week] = weeks.get(week, 0) + day['profit']
    concentration = {
        f'top_{n}_share_of_positive_pnl_pct': 100 * sum(d['profit'] for d in winners[:n]) / gross if gross > 0 else None
        for n in (1, 3, 5)
    }
    fixed = abs(max_lot - .01) < 1e-12
    exclusions = ({f'profit_without_best_{n}': sum(pnl) - sum(d['profit'] for d in winners[:n])
                   for n in (1, 3, 5)} if fixed else None)
    return {
        'schema': 'conduit.daily-concentration.v1', 'mode': mode, 'max_lot': max_lot,
        'observed_days': len(days), 'positive_days': sum(p > 0 for p in pnl),
        'negative_days': sum(p < 0 for p in pnl), 'flat_days': sum(p == 0 for p in pnl),
        'sum_daily_profit': sum(pnl), 'median_day': statistics.median(pnl),
        'mean_day': statistics.mean(pnl), 'median_daily_return_pct': statistics.median(returns) if returns else None,
        'unavailable_return_days': len(days) - len(returns),
        'worst_five_observed_days_profit': min(sum(pnl[i:i+5]) for i in range(len(pnl)-4)) if len(pnl) >= 5 else None,
        'concentration': concentration,
        'best_days': [{'date': d['date'], 'profit': d['profit'],
                       'return_pct': d['profit'] / d['start_equity'] * 100 if d['start_equity'] > 0 else None}
                      for d in winners[:5]],
        'fixed_lot_exclusion_enabled': fixed, 'fixed_lot_exclusion': exclusions,
        'calendar_month_cash_aggregation': months, 'calendar_week_cash_aggregation': weeks,
        'limitations': [
            'No date is selected or removed by the trading strategy; these are retrospective diagnostics.',
            'Best-day exclusion is unavailable outside the explicit max-lot-0.01 protocol.',
            'Calendar sums are not independent fresh-deposit month/week replays.',
            'A large winning day is beneficial if performance outside exceptional days is also sound.',
            'Cash concentration grows naturally with compounding; examine percentage returns and independent windows.',
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--days', type=Path, required=True)
    parser.add_argument('--max-lot', type=float, required=True)
    parser.add_argument('--mode', choices=('continuous', 'independent_days'), required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    raw = args.days.read_bytes()
    result = analyze_days(json.loads(raw.decode('utf-8-sig')), args.max_lot, args.mode)
    result['source_sha256'] = hashlib.sha256(raw).hexdigest()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps({k: result[k] for k in ('observed_days', 'median_day', 'sum_daily_profit', 'fixed_lot_exclusion')}))


if __name__ == '__main__':
    main()
