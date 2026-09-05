"""Read complete research summaries and compare approximate/exact replay."""
from __future__ import annotations
import argparse
import json
import math
from pathlib import Path
import statistics


def read_summary(path: Path, allow_partial: bool = False) -> tuple[dict, dict]:
    if not allow_partial and (path.name == 'wyniki_czastkowe.json' or (path.parent/'PRZERWANE.txt').exists()):
        raise ValueError('Partial results cannot be ranked as complete')
    doc = json.loads(path.read_text('utf-8-sig'))
    if doc.get('approximate') is True:
        return doc['results'], {k:v for k,v in doc.items() if k!='results'}
    return doc, {'approximate': False}


def compact(name: str, metrics: dict) -> dict:
    funnel = metrics.get('stat_sygnalow', {}).get('lejek', {})
    baskets = metrics.get('stat_sygnalow', {}).get('koszyki', {})
    return {'name': name,
            **{key: metrics.get(key) for key in (
                'total_profit', 'start_balance', 'end_equity', 'market_days',
                'positive_market_days', 'negative_market_days', 'flat_market_days',
                'positive_market_days_pct', 'worst_market_day', 'worst_market_day_date',
                'win_days_pct', 'win_days', 'trading_days',
                'baskets', 'trades', 'max_dd_pct', 'max_daily_dd', 'min_equity',
                'blown', 'stop_outs', 'profit_factor')},
            'filled_baskets': baskets.get('z_pozycjami'),
            'entry_signals': funnel.get('sygnaly_wejsciowe'),
            'signal_utilization_pct': funnel.get('wykonanych_pct'),
            'rejected_signals': funnel.get('odrzucone_sygnaly'),
            'no_fill_baskets': funnel.get('koszyk_bez_fillu')}


def average_ranks(values: list[float]) -> list[float]:
    indexes = sorted(range(len(values)), key=values.__getitem__)
    ranks = [0.] * len(values)
    start = 0
    while start < len(indexes):
        end = start+1
        while end < len(indexes) and values[indexes[end]] == values[indexes[start]]:
            end += 1
        rank = (start+end-1)/2
        for pos in range(start, end):
            ranks[indexes[pos]] = rank
        start = end
    return ranks


def correlation(x: list[float], y: list[float]) -> float | None:
    if len(x) < 2:
        return None
    xm, ym = statistics.mean(x), statistics.mean(y)
    numerator = sum((a-xm)*(b-ym) for a,b in zip(x,y))
    denominator = math.sqrt(sum((a-xm)**2 for a in x)*sum((b-ym)**2 for b in y))
    return numerator/denominator if denominator else None


def compare(exact: dict, quick: dict) -> dict:
    shared = sorted(exact.keys() & quick.keys())
    if not shared:
        raise ValueError('No shared candidates')
    comparisons = {}
    for key in ('total_profit', 'positive_market_days_pct', 'baskets', 'max_dd_pct'):
        a, b = [exact[n][key] for n in shared], [quick[n][key] for n in shared]
        delta = [abs(x-y) for x,y in zip(a,b)]
        comparisons[key] = {'spearman': correlation(average_ranks(a), average_ranks(b)),
                            'median_absolute_error': statistics.median(delta),
                            'max_absolute_error': max(delta)}
    k = min(10, max(1, len(shared)//3))
    exact_top = set(sorted(shared, key=lambda n: exact[n]['total_profit'], reverse=True)[:k])
    quick_top = set(sorted(shared, key=lambda n: quick[n]['total_profit'], reverse=True)[:k])
    return {'common_candidates': len(shared), 'metrics': comparisons,
            'profit_sign_disagreements': sum((exact[n]['total_profit']>0)!=(quick[n]['total_profit']>0) for n in shared),
            'top_k': k, 'top_k_profit_overlap': len(exact_top & quick_top),
            'coronation_eligible': False,
            'warning': 'Calibration does not certify approximate replay for selecting a production default.'}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--exact', required=True, type=Path)
    parser.add_argument('--quick', type=Path)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    rows, metadata = read_summary(args.exact)
    if args.quick:
        if metadata['approximate']:
            raise ValueError('--exact must contain exact replay')
        approximate, quick_metadata = read_summary(args.quick)
        if not quick_metadata['approximate']:
            raise ValueError('--quick must contain approximate replay')
        result = compare(rows, approximate)
    else:
        result = {'metadata': metadata, 'candidates': [compact(n,m) for n,m in rows.items()]}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps({'output': str(args.out), 'candidates': len(rows)}))


if __name__ == '__main__':
    main()
