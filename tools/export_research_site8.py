"""Export an allowlisted, receipt-verified research dataset for the private report.

Never copy replay documents wholesale: they also contain local paths and source
messages. Monetary results from different accounts are never combined here.
"""
from __future__ import annotations
import argparse
import datetime as dt
import json
import math
from pathlib import Path
import re

from research_runner import file_hash
from verified_replay_report import collect
from research_runner_exact8 import verify_detail_binding

CONTRACTS = {'historical_reference', 'observed_receipts', 'mixed_missing_original_stress'}
# This completed public G8i build has a documented summary-only bug:
# metrics::compute initializes end_balance from end_equity, and runner replaces
# it only for the credit-reporting path. Its sampled balance curve is the actual
# broker.balance. Never extend this exception to an unreviewed executable.
LEGACY_BALANCE_ALIAS_EXES = {'f7eae2549d99ac3e4b3be498fb6123dcb146ed4b999468e0af9f19f97996feff'}


def finite(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def close(a, b):
    return finite(a) and finite(b) and math.isclose(a, b, abs_tol=.011, rel_tol=1e-10)


def legacy_balance_view(document, metrics, receipt):
    """Correct only the derived view, using an already completion-bound curve.

    Source summaries, curves, trades and profits are never rewritten. The caller
    must verify_detail_binding before calling this function, then checked_series.
    """
    balance, equity = document.get('saldo', []), document.get('krzywa', [])
    if not balance or not equity or close(balance[-1][1], metrics.get('end_balance')):
        return metrics, None
    if receipt.get('exe_sha256') not in LEGACY_BALANCE_ALIAS_EXES:
        return metrics, None  # Normal strict endpoint validation will reject it.
    own = document.get('metryki', {})
    if (document.get('tryb') != 'compound' or own.get('reporting_equity_basis') is not None
            or not close(document.get('saldo_start'), metrics['start_balance'])
            or not close(own.get('end_balance'), metrics.get('end_balance'))
            or not close(metrics.get('end_balance'), metrics['end_equity'])
            or balance[-1][0] != equity[-1][0] or not finite(balance[-1][1])):
        raise ValueError('Legacy balance alias does not match the audited producer contract')
    correction = {'code': 'legacy_summary_balance_was_equity',
                  'reportedSummaryEndBalance': metrics['end_balance'],
                  'measuredEndBalance': balance[-1][1],
                  'evidence': 'completion-bound actual broker balance curve',
                  'profitAndTradesChanged': False}
    return {**metrics, 'end_balance': balance[-1][1]}, correction


def checked_series(document, metrics):
    """Check account endpoints and every daily movement before exposing a curve."""
    days = document['dni']
    equity, balance = document['krzywa'], document.get('saldo', [])
    if not isinstance(days, list) or not isinstance(equity, list) or not equity:
        raise ValueError('Missing account history')
    if len(days) != metrics['market_days']:
        raise ValueError('Daily history does not match the summary market-day count')
    if len({d['date'] for d in days}) != len(days):
        raise ValueError('Duplicate market dates')
    if [d['date'] for d in days] != sorted(d['date'] for d in days):
        raise ValueError('Market dates move backwards')
    previous = metrics['start_balance']
    for day in days:
        dt.date.fromisoformat(day['date'])
        if not close(day['start_equity'], previous) or not close(day['end_equity']-day['start_equity'], day['profit']):
            raise ValueError('Daily account continuity mismatch')
        previous = day['end_equity']
    if not close(previous, metrics['end_equity']) or not close(sum(d['profit'] for d in days), metrics['total_profit']):
        raise ValueError('Daily history and final summary disagree')
    for points in (equity, balance):
        if any(not isinstance(p, list) or len(p) != 2 or not all(finite(v) for v in p) for p in points):
            raise ValueError('Invalid curve point')
        if any(b[0] < a[0] for a, b in zip(points, points[1:])):
            raise ValueError('Curve time moves backwards')
    if not close(equity[-1][1], metrics['end_equity']):
        raise ValueError('Curve endpoint differs from final equity')
    if not balance or not close(balance[-1][1], metrics.get('end_balance')):
        raise ValueError('Balance curve endpoint differs from final balance')
    balance_at = dict(balance)
    return ([{'timestamp': t, 'equity': v, **({'balance': balance_at[t]} if t in balance_at else {})} for t, v in equity],
            [{'date': d['date'], 'profit': d['profit']} for d in days])


def extrema_preview(points, max_points=1600):
    """Retain endpoints and equity/balance extrema in chronological buckets."""
    if max_points < 10:
        raise ValueError('At least ten preview points required')
    if len(points) <= max_points:
        return points
    buckets = max(1, (max_points-2)//4)
    selected = {0, len(points)-1}
    for bucket in range(buckets):
        start = 1 + (len(points)-2)*bucket//buckets
        end = 1 + (len(points)-2)*(bucket+1)//buckets
        indexes = range(start, end)
        if start >= end:
            continue
        for key in ('equity', 'balance'):
            available = [i for i in indexes if finite(points[i].get(key))]
            if available:
                selected.add(min(available, key=lambda i: points[i][key]))
                selected.add(max(available, key=lambda i: points[i][key]))
    return [points[i] for i in sorted(selected)]


def export(plans, source_calendar=None, series_names=None):
    coverage_document = json.loads(source_calendar.read_text('utf-8')) if source_calendar else {}
    coverage = coverage_document.get('sources', {})
    coverage_by_hash = coverage_document.get('by_input_sha256', {})
    coronation, comparisons, series, evidence, incomplete = [], [], [], [], []
    identities, displayed = {}, {'coronation': set(), 'comparison': set()}
    for plan_path in plans:
        plan = json.loads(plan_path.read_text('utf-8-sig'))
        report = collect(plan_path)
        jobs = {j['id']: j for j in plan['jobs']}
        is_coronation = bool(plan.get('windows'))
        incomplete.extend({'planSha256': report['plan_sha256'], **r} for r in report['incomplete'])
        for result in report['results']:
            job, m = jobs[result['job']], result['metrics']
            name, cap = m['name'], result['lot_cap']
            corpus = job.get('signal_contract', plan.get('protocol', {}).get('signal_contract'))
            if corpus not in CONTRACTS or not re.fullmatch(r'[A-Za-z0-9_-]+', name):
                raise ValueError('Only known corpora and research identifiers may be exported')
            window = result['window']
            kind = window.get('kind', 'full')
            contract = job.get('economic_contract') or 'Common measured broker costs and sessions used in search'
            display_name = 'GOD-X7' if name in ('GOD-X7-cap5', 'GOD-X7-reference') else name
            identity = (display_name, corpus, window['from'], window['to'], result['deposit'], cap, contract, kind)
            signature = (m['total_profit'], m['end_equity'], m['market_days'], m['trades'])
            if identity in identities:
                if not all(close(a, b) for a, b in zip(signature, identities[identity])):
                    raise ValueError('Repeated identical scenario has different measured results')
                if identity in displayed['coronation' if is_coronation else 'comparison']:
                    continue
            identities[identity] = signature
            displayed['coronation' if is_coronation else 'comparison'].add(identity)
            row = {'candidate': display_name, 'corpus': corpus, 'windowKind': kind,
                'from': window['from'], 'to': window['to'], 'deposit': result['deposit'], 'lotCap': cap,
                'profit': m['total_profit'], 'endEquity': m['end_equity'], 'maxDdPct': m['max_dd_pct'],
                'positiveDaysPct': m['positive_market_days_pct'], 'marketDays': m['market_days'],
                'filledBaskets': m['baskets_with_closed_trades'], 'acceptedSourcesPct': m['accepted_entry_sources_pct'],
                'stopOuts': m['stop_outs'], 'blown': m['blown']}
            concentration = m.get('daily_concentration')
            if cap == .01 and kind == 'full' and concentration:
                row['concentration'] = {'profitWithoutBestDay': concentration.get('profit_without_best_1'),
                    'profitWithoutTop3Days': concentration.get('profit_without_best_3'),
                    'profitWithoutTop5Days': concentration.get('profit_without_best_5')}
            data_path = Path(job['result_dir']) / (name + '_compound_dane.json')
            if not data_path.is_file():
                raise ValueError('Detailed history required for every displayed completed result')
            receipt_path = Path(plan['output']) / job['id'] / 'receipt.json'
            if file_hash(receipt_path) != result['receipt_sha256']:
                raise ValueError('Receipt changed during report export')
            receipt = json.loads(receipt_path.read_text('utf-8-sig'))
            detail_sha = verify_detail_binding(data_path, receipt)
            document = json.loads(data_path.read_text('utf-8-sig'))
            m, balance_correction = legacy_balance_view(document, m, receipt)
            points, daily = checked_series(document, m)
            row['endBalance'] = m['end_balance']
            signal_path = job['argv'][job['argv'].index('--signals')+1]
            input_hash = next(a['sha256'] for a in plan['inputs'] if Path(a['path']).resolve() == Path(signal_path).resolve())
            source = coverage_by_hash.get(input_hash, coverage.get(corpus))
            if source:
                if source['input_sha256'] != input_hash:
                    raise ValueError('Source calendar belongs to another signal history')
                after_first = [d for d in daily if d['date'] >= source['first_event_broker_date']]
                row['coveredMarketDays'] = len(after_first)
                row['coveredPositiveDaysPct'] = 100*sum(d['profit'] > 0 for d in after_first)/len(after_first) if after_first else None
            if is_coronation:
                coronation.append(row)
            else:
                comparisons.append({**row, 'contract': contract, 'closes': m['trades']})
            if kind == 'full' and cap in (.01, 10) and (series_names is None or name in series_names):
                sid = '|'.join(map(str, identity))
                if sid not in {s['id'] for s in series}:
                    series.append({'id': sid, 'label': display_name, 'corpus': corpus, 'deposit': result['deposit'],
                        'lotCap': cap, 'contract': contract + ' | ' + window['from'] + ' <= date < ' + window['to'],
                        'from': window['from'], 'to': window['to'],
                        'points': extrema_preview(points), 'daily': daily,
                        'concentration': row.get('concentration'),
                        'originalPointCount': len(points), 'previewPreservesBucketExtrema': True})
            evidence.append({'candidate': name, 'case': job['id'], 'planSha256': report['plan_sha256'],
                'receiptSha256': result['receipt_sha256'], 'summarySha256': result['result_sha256'],
                'curveSha256AtCompletion': detail_sha,
                **({'balanceViewCorrection': balance_correction} if balance_correction else {})})
    return {'schema': 'conduit.site-research-data.v1', 'ready': not incomplete,
        'coronationRows': coronation, 'comparisonRows': comparisons, 'equitySeries': series,
        'incomplete': incomplete, 'evidence': evidence, 'productionChoiceMade': False,
        'notes': ['Daily profit is the movement in account equity.',
            'Open positions remain marked to the last quote; account equity is not automatically realized cash.',
            'Known legacy summary balance aliases are explicitly recorded; the bound actual balance curve is never rewritten.',
            'Preview extrema are preserved; exact account summaries use the complete series.',
            'The source-start view removes only leading dates before any available source event, never interior gaps or flat days.',
            'Source event span does not prove continuous capture.',
            'No local paths, messages, credentials or arbitrary replay fields are exported.']}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--plan', action='append', required=True, type=Path)
    p.add_argument('--source-calendar', type=Path)
    p.add_argument('--series-name', action='append')
    p.add_argument('--out', required=True, type=Path)
    args = p.parse_args()
    result = export(args.plan, args.source_calendar, set(args.series_name) if args.series_name else None)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, ensure_ascii=False, separators=(',', ':'), allow_nan=False), encoding='utf-8')
    print(json.dumps({'ready': result['ready'], 'coronationRows': len(result['coronationRows']),
        'comparisonRows': len(result['comparisonRows']), 'series': len(result['equitySeries']), 'bytes': args.out.stat().st_size}))


if __name__ == '__main__':
    main()
