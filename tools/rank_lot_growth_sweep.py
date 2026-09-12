"""Select sizing candidates using TRAIN only and immutable completion receipts."""
from __future__ import annotations
import argparse
from collections import Counter
import datetime as dt
import math
from pathlib import Path
import statistics

from prepare_lot_growth_sweep import (read, pin, write_new, exact_equal, option,
                                      candidates)
from research_runner_exact8 import verify_detail_binding
from research_runner_sizing import verified_job

def weekly_median(days):
    groups = {}
    for row in days:
        date = dt.date.fromisoformat(row['date'])
        monday = (date-dt.timedelta(days=date.weekday())).isoformat()
        groups.setdefault(monday, []).append(row)
    returns = []
    for rows in groups.values():
        if rows[0]['start_equity'] <= 0: return None
        returns.append(rows[-1]['end_equity']/rows[0]['start_equity']-1)
    return statistics.median(returns) if returns else None

def describe(name, metrics, detail, baskets, sha, trades=None):
    days = detail['dni']
    if trades is None and metrics['trades'] != 0:
        raise ValueError('Closed-trade activity requires its bound execution ledger')
    ledger = [] if trades is None else trades['transakcje']
    if len(ledger) != metrics['trades']:
        raise ValueError('Execution ledger count differs from summary')
    timestamps = set()
    trade_baskets = set()
    for trade in ledger:
        timestamp = trade['otwarcie_ms']
        if type(timestamp) is not int or timestamp <= 0:
            raise ValueError('Invalid recorded entry timestamp')
        timestamps.add(timestamp)
        if trade.get('koszyk') is not None:
            trade_baskets.add(trade['koszyk'])
    populated = []
    for basket in baskets:
        filled = [level for level in basket['warstwy'] if level['filled']]
        for level in filled:
            timestamp = level['fill_ts']
            if type(timestamp) is not int or timestamp <= 0:
                raise ValueError('Filled level lacks a recorded entry timestamp')
            timestamps.add(timestamp)
        if basket['had_positions'] or filled or basket['id'] in trade_baskets:
            populated.append(basket)
    entry_days = {ts//86_400_000 for ts in timestamps}
    keys = ('start_balance','end_equity','total_profit','max_dd_pct','min_equity',
            'blown','stop_outs','positive_market_days_pct','market_days','trades','baskets',
            'profit_factor','min_margin_level','max_open_volume','max_open_risk_pct')
    return {'id':name, 'settings_sha256':sha, **{k:metrics.get(k) for k in keys},
            'filled_baskets':len(populated), 'active_entry_days':len(entry_days),
            'market_dates':[r['date'] for r in days],
            'median_weekly_return':weekly_median(days),
            'log_growth':math.log(metrics['end_equity']/metrics['start_balance'])
                         if metrics['end_equity']>0 else None,
            'max_daily_rdd_pct':(max(r['real_dd_pct'] for r in days)
                                 if days and all(r.get('real_dd_pct') is not None for r in days)
                                 else None),
            'accepted_entry_sources_pct':metrics.get('stat_sygnalow',{}).get('lejek',{}).get('accepted_entry_sources_pct')}

def reasons(row, baseline):
    bad = []
    numeric = ('total_profit','max_dd_pct','min_equity','positive_market_days_pct',
               'median_weekly_return','log_growth')
    if (any(type(row.get(k)) not in (int,float) or not math.isfinite(row[k]) for k in numeric)
            or type(row.get('blown')) is not bool
            or type(row.get('stop_outs')) is not int or row['stop_outs'] < 0):
        return ['invalid_or_incomplete_metrics']
    if row['market_dates'] != baseline['market_dates']: bad.append('incomplete_market_dates')
    if row.get('blown') or row.get('stop_outs') != 0 or row['min_equity'] <= 0:
        bad.append('insolvent_or_stopout')
    if row['total_profit'] <= 0: bad.append('nonpositive_profit')
    if row['max_dd_pct'] > 35: bad.append('dd_above_35')
    if row['positive_market_days_pct'] < 50: bad.append('positive_days_below_50')
    if row['filled_baskets'] < baseline['filled_baskets']*.5: bad.append('insufficient_filled_baskets')
    if row['active_entry_days'] < baseline['active_entry_days']*.8: bad.append('insufficient_active_entry_days')
    return bad

def dd_key(row):
    return (math.ceil(row['max_dd_pct']/2.5), -row['positive_market_days_pct'],
            -row['median_weekly_return'], -row['log_growth'], row['settings_sha256'])

def select(rows, baseline):
    evaluated = [{**row,'rejections':reasons(row,baseline)} for row in rows]
    eligible = [r for r in evaluated if not r['rejections']]
    first = sorted(eligible,key=dd_key)[:5]
    seen = {r['id'] for r in first}
    growth = sorted((r for r in eligible if r['id'] not in seen),
                    key=lambda r:(-r['log_growth'],dd_key(r)))[:5]
    chosen = first+growth
    return {'eligible_count':len(eligible),'selected':chosen,
            'rejection_counts':dict(Counter(reason for r in evaluated for reason in r['rejections'])),
            'all_candidates':evaluated, 'coronation_eligible':False,
            'status':'TRAIN_SELECTION_ONLY' if chosen else 'NO_QUALIFYING_CANDIDATE',
            'selection_does_not_use_august_or_september':True}

def verified_results(root, job):
    root = Path(root).resolve()
    plan, spec, receipt = verified_job(root, job)
    path = Path(spec['result_dir'])/'wyniki_compound.json'
    if pin(path)['sha256'] != receipt['result_sha256'][path.name]:
        raise ValueError('Summary changed after completion')
    rows = read(path)
    if len(rows) != spec['expected_candidates']:
        raise ValueError('Unexpected number of completed candidates')
    expected_names = {read(p)['name'] for p in Path(option(spec['argv'],'--sweep')).glob('*.json')}
    if set(rows) != expected_names:
        raise ValueError('Results do not identify the planned presets')
    details = {}
    for name in rows:
        data = path.parent/(name+'_compound_dane.json')
        verify_detail_binding(data,receipt)
        detail = read(data)
        if (detail['preset'] != name or detail['tryb'] != 'compound'
                or detail['od'] != option(spec['argv'],'--from')
                or detail['do'] != option(spec['argv'],'--to')
                or detail['saldo_start'] != float(option(spec['argv'],'--balance'))
                or not exact_equal(detail['metryki'],rows[name])):
            raise ValueError('Detailed result differs from the planned window, capital or summary')
        dates = [r['date'] for r in detail['dni']]
        if (dates != sorted(set(dates)) or any(not detail['od'] <= day < detail['do'] for day in dates)):
            raise ValueError('Invalid or out-of-window daily history')
        basket_path=path.parent/(name+'_compound_koszyki.json')
        verify_detail_binding(basket_path,receipt)
        detail['koszyki']=read(basket_path)
        ledger_path = path.parent/(name+'_compound_transakcje.json')
        if rows[name]['trades'] > 0 or ledger_path.exists():
            verify_detail_binding(ledger_path,receipt)
            ledger = read(ledger_path)
            if (ledger['preset'] != name or ledger['tryb'] != 'compound'
                    or ledger['od'] != detail['od'] or ledger['do'] != detail['do']
                    or len(ledger['transakcje']) != rows[name]['trades']
                    or ledger['transakcji'] != rows[name]['trades']):
                raise ValueError('Execution ledger does not identify these results')
            detail['transakcje'] = ledger
        else:
            detail['transakcje'] = None
        details[name] = detail
    return rows, details

def verify_training_design(root):
    plan = read(root/'PLAN.json')
    prereg = read(root/'PREREGISTRATION.json')
    if not exact_equal(plan['qualification']['preregistration'],pin(root/'PREREGISTRATION.json')):
        raise ValueError('TRAIN design is not bound by its plan')
    if (prereg['schema'] != 'conduit.sizing300.preregistered.v3'
            or prereg['candidate_count'] != 300 or prereg['capital'] != 600
            or prereg['max_lot'] != 5):
        raise ValueError('Expected the preregistered TRAIN study')
    training = prereg['training']
    if not exact_equal(training,{'from':'2026-06-20','to_exclusive':'2026-08-01'}):
        raise ValueError('This selector accepts TRAIN only')
    jobs = {j['id']:j for j in plan['jobs']}
    if set(jobs) != {'controls','sizing300'}:
        raise ValueError('Unexpected TRAIN jobs')
    for job in jobs.values():
        if (option(job['argv'],'--from') != training['from']
                or option(job['argv'],'--to') != training['to_exclusive']
                or float(option(job['argv'],'--balance')) != 600):
            raise ValueError('Non-TRAIN window or capital must not select candidates')
    base = read(plan['qualification']['materialized_baseline']['path'])
    expected = candidates(base)
    registry = prereg['candidates']
    if len(registry) != len(expected):
        raise ValueError('Unexpected TRAIN candidate registry')
    for actual, generated in zip(registry,expected):
        document = generated.pop('preset')
        if (not exact_equal({k:v for k,v in actual.items() if k != 'file'},generated)
                or not exact_equal(read(actual['file']['path']),document)):
            raise ValueError('Candidate differs from the preregistered sizing-only design')
    return prereg

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('study',type=Path)
    args=parser.parse_args(); root=args.study.resolve()
    prereg=verify_training_design(root)
    summary,details=verified_results(root,'sizing300')
    controls,control_details=verified_results(root,'controls')
    registry={r['id']:r for r in prereg['candidates']}
    if len(registry)!=300 or set(summary)!=set(registry): raise ValueError('Expected all 300 candidates')
    refname='GOD-X7-fixed001'
    refdetail=control_details[refname]
    baseline=describe(refname,controls[refname],refdetail,refdetail['koszyki'],'reference',
                      trades=refdetail['transakcje'])
    rows=[]
    for name,m in summary.items():
        if pin(registry[name]['file']['path']) != registry[name]['file']:
            raise ValueError('Candidate changed since preregistration')
        detail=details[name]
        rows.append(describe(name,m,detail,detail['koszyki'],registry[name]['settings_sha256'],
                             trades=detail['transakcje']))
    result=select(rows,baseline)
    result.update(schema='conduit.sizing300.train-selection.v1',baseline=baseline,
                  preregistration=pin(root/'PREREGISTRATION.json'), plan=pin(root/'PLAN.json'),
                  completion_receipts=[pin(root/job/'receipt.json') for job in ('controls','sizing300')])
    write_new(root/'TRAIN_SELECTION.json',result)
    write_new(root/'SHORTLIST.json',[registry[r['id']] for r in result['selected']])
    print(result['status'],len(result['selected']),'of 300 selected')

if __name__=='__main__':main()
