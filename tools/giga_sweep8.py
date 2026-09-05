"""Generate a reproducible, diversified Synergy candidate space.

Every search candidate has a hard user lot cap of 5. Dates, direction-specific
filters, individual message IDs and future outcomes are never search axes.
Broker assumptions are explicit, common inputs, not optimization parameters.
"""
from __future__ import annotations
import argparse
import copy
import hashlib
import json
from pathlib import Path
import random
import re

FAMILIES = ('profit_retention', 'basket_harvest', 'adaptive_trail', 'breadth',
            'sizing', 'neighborhood', 'hybrid')
EXTRA_FAMILIES = ('daily_bank', 'soft_regime', 'budget_reinvest', 'budget_soft_regime')
BROKER_FIELDS = {
    'konto_dzwignia', 'commission_per_lot', 'swap_enabled', 'swap_long_points',
    'swap_short_points', 'swap_point_value', 'swap_rollover_z_serwera',
    'swap_rollover3days_mt5', 'swap_rollover_weekday', 'swap_rollover_mult',
    'stop_out_level_pct', 'margin_call_level_pct', 'server_tz_offset_ms',
}


def fingerprint(settings: dict) -> str:
    return hashlib.sha256(json.dumps(settings, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def sample_family(family: str, rng: random.Random) -> dict:
    choice = rng.choice
    if family in ('budget_reinvest', 'budget_soft_regime'):
        arm = choice([1., 2., 4., 8., 12., 20.])
        keep = choice([20., 40., 60., 80.])
        axes = {
            'profit_budget_arm_pct': arm,
            'profit_budget_keep_pct': keep,
            'profit_budget_deploy_pct': choice([25., 50., 75., 100.]),
            'day_trail_basis': 'ProfitPeak',
            'day_trail_arm_pct': arm,
            # Half the space sizes new risk only; the other half also closes
            # existing exposure at the matching peak-profit retention floor.
            'day_trail_stop_pct': choice([0., 100. - keep]),
            'lot_percent': choice([.2, .3, .5, .7, 1.]),
            'risk_per_basket_pct': choice([5., 8., 12., 20., 30.]),
            'entry_units': choice([4, 6, 8, 10]),
            'max_portfolio_risk_pct': choice([0., 25., 50., 100.]),
        }
        if family == 'budget_soft_regime':
            axes.update({k:v for k,v in sample_family('soft_regime', rng).items()
                         if k.startswith('regime_')})
        return axes
    if family == 'daily_bank':
        return {
            'day_target_pct': choice([.5, 1., 2., 3., 5., 8., 12., 20.]),
            'day_target_close': True, 'day_target_usd': 0.,
            'day_trail_basis': 'ProfitPeak',
            'day_trail_arm_pct': choice([.5, 1., 2., 4., 8., 12.]),
            'day_trail_stop_pct': choice([10., 20., 30., 40., 50.]),
            'lot_percent': choice([.1, .15, .2, .3, .5]),
            'risk_per_basket_pct': choice([3., 5., 8., 12.]),
            'entry_units': choice([3, 4, 6, 8, 10]),
            'max_portfolio_risk_pct': choice([10., 20., 40., 60.]),
        }
    if family == 'soft_regime':
        return {
            'regime_filter': choice(['TrendMa', 'CounterMa']),
            'regime_ma_hours': choice([1., 3., 6., 12., 24., 48., 72.]),
            'regime_cena': 'Rynkowa',
            'regime_miara': 'Srednia',
            'regime_soft': True, 'regime_gdy_rozerwany': 'Milcz',
            'regime_soft_units_mult': choice([.5, .75, 1.]),
            'regime_soft_lot_mult': choice([.25, .5, .75]),
            'regime_soft_risk_mult': choice([.5, .75, 1.]),
            'regime_soft_max_positions': 0,
            'lot_percent': choice([.1, .15, .2, .3, .5]),
            'risk_per_basket_pct': choice([3., 5., 8., 12.]),
        }
    if family == 'profit_retention':
        return {
            'day_trail_basis': 'ProfitPeak',
            'day_trail_arm_pct': choice([2., 4., 6., 8., 12., 16., 24., 32.]),
            'day_trail_stop_pct': choice([10., 20., 30., 40., 50., 65., 80.]),
            'lot_percent': choice([.15, .2, .3, .5, .7, 1.]),
            'risk_per_basket_pct': choice([5., 8., 12., 16., 20.]),
            'entry_units': choice([4, 6, 8, 10]),
        }
    if family == 'basket_harvest':
        return {
            'riskfree_enabled': True, 'riskfree_trigger_usd': 0.,
            'riskfree_trigger_r': choice([.2, .35, .5, .75, 1., 1.5]),
            'riskfree_keep_units': choice([1, 2, 3]),
            'riskfree_be_offset': choice([0., .2, .4]),
            'riskfree_runner_target': choice(['KeepTp', 'LastTp', 'NextTp']),
            'riskfree_runner_stop': choice(['Be', 'BeOwn', 'TrailGap']),
            'riskfree_runner_gap': choice([3., 5., 8., 12.]),
            'official_pct': choice([[10.,10.,25.,10.], [15.,15.,25.,15.],
                                    [25.,25.,25.,25.], [40.,20.,15.,10.]]),
            'tp_open_offset': choice([10., 15., 20., 25., 35.]),
        }
    if family == 'adaptive_trail':
        return {
            'trail_mode': 'Gap', 'trail_start': choice([3., 5., 8., 12., 16.]),
            'trail_gap': choice([2., 3., 5., 8., 12., 16.]),
            'trail_adaptive_enabled': True,
            'trail_adaptive_runners_only': choice([True, False]),
            'trail_adaptive_window_s': choice([30., 60., 120., 300., 600.]),
            'trail_adaptive_trend_gap_mult': choice([1., 1.3, 1.8, 2.2]),
            'trail_adaptive_chop_gap_mult': choice([.5, .7, 1.]),
            'trail_adaptive_reversal_gap_mult': choice([.35, .5, .75]),
            'trail_adaptive_min_peak': choice([1., 2., 4.]),
            'trail_adaptive_min_gap': choice([.5, 1., 2.]),
            'trail_adaptive_max_gap': choice([8., 12., 20., 30.]),
        }
    if family == 'breadth':
        return {
            'entry_tol_offset': choice([0., .5, 1., 2., 3., 4., 6.]),
            'entry_deep_offset': choice([0., .5, 1., 2., 3.]),
            'entry_units': choice([3, 4, 6, 8, 10, 12]),
            'market_entry_units': choice([1, 2, 3]),
            'pending_lifetime': choice(['UntilTp1', 'UntilTp2', 'UntilTp3']),
            'pending_ttl_h': choice([2., 4., 8., 12.]),
            'pending_drop_keep_n': choice([0, 1, 2]),
            'rearm_grid_on_return': choice([True, False]),
            'risk_per_basket_pct': choice([5., 10., 15., 20.]),
        }
    if family == 'sizing':
        return {
            'lot_percent': choice([.1, .15, .2, .3, .5, .7, 1.]),
            'risk_per_basket_pct': choice([3., 5., 8., 12., 16., 20.]),
            'max_portfolio_risk_pct': choice([0., 20., 40., 60., 100.]),
            'dd_soft_pct': choice([0., 10., 20., 30.]),
            'dd_soft_mult': choice([.25, .5, .75]),
            'lot_base': choice(['Balance', 'Equity', 'MinOfBoth']),
            'entry_depth_curve': choice([.5, 1., 1.5, 2.]),
            'entry_units': choice([4, 6, 8, 10]),
        }
    if family == 'neighborhood':
        axes = {
            'entry_units': [4, 6, 8, 10, 12], 'lot_percent': [.2, .3, .5, .7],
            'entry_tol_offset': [0.,1.,2.,3.,4.], 'entry_deep_offset': [0.,.5,1.,2.],
            'no_tp_after_stage': [0,1,2,3], 'be_od_etapu': [1,2,3],
            'tp_open_offset': [10.,15.,20.,25.,35.],
            'pending_lifetime': ['UntilTp1','UntilTp2','UntilTp3'],
            'day_trail_arm_pct': [0.,4.,8.,12.,24.],
            'day_trail_stop_pct': [0.,10.,20.,30.,40.],
            'risk_per_basket_pct': [5.,10.,15.,20.,25.],
            'rearm_grid_on_return': [True,False],
            'pending_drop_require_zone_touch': [True,False],
        }
        return {key: choice(axes[key]) for key in rng.sample(list(axes), choice([2,3,4,5]))}
    if family == 'hybrid':
        out = sample_family(choice(['basket_harvest', 'adaptive_trail', 'breadth']), rng)
        out.update(sample_family('profit_retention', rng))
        if rng.random() < .5:
            out.update({k:v for k,v in sample_family('sizing', rng).items()
                        if k in ('dd_soft_pct','dd_soft_mult','lot_base')})
        return out
    raise ValueError(family)


def generate(base: dict, broker: dict, count: int, seed: int,
             families: tuple[str, ...] = FAMILIES, prefix: str = 'G8') -> list[dict]:
    if count < 1 or not families or set(families) - set(FAMILIES + EXTRA_FAMILIES):
        raise ValueError('Invalid candidate count or axis family')
    if not re.fullmatch(r'[A-Za-z0-9_-]+', prefix):
        raise ValueError('Candidate prefix must be a safe identifier')
    if set(broker)-BROKER_FIELDS:
        raise ValueError(f'Unexpected broker fields: {sorted(set(broker)-BROKER_FIELDS)}')
    common = copy.deepcopy(base['settings'])
    common.update(broker)
    common['lot_max'] = 5.0
    # Management belongs to the supplied preset. Publisher validity must not
    # silently replace its TP-stage/TTL/rearm or daily-profit interpretation.
    # A phase that changes an ingress or pending policy supplies an explicitly
    # qualified base, whose file hash is recorded in the space manifest.
    rows = [{'id': 'GOD-X7-cap5', 'family': 'reference', 'settings': common,
             'changes': {}, 'fingerprint': fingerprint(common)}]
    seen = {rows[0]['fingerprint']}
    rng = random.Random(seed)
    attempt = 0
    while len(rows) < count:
        family = families[attempt % len(families)]
        attempt += 1
        changes = sample_family(family, rng)
        settings = copy.deepcopy(common)
        settings.update(changes)
        settings['lot_max'] = 5.0
        key = fingerprint(settings)
        if key in seen:
            continue
        seen.add(key)
        changes = {k:v for k,v in changes.items() if v != common.get(k)}
        rows.append({'id': f'{prefix}-{family}-{len(rows):05d}', 'family': family,
                     'settings': settings, 'changes': changes, 'fingerprint': key})
    return rows


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', required=True, type=Path)
    parser.add_argument('--broker', required=True, type=Path)
    parser.add_argument('--settings-source', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--count', type=int, default=10000)
    parser.add_argument('--seed', type=int, default=20260905)
    parser.add_argument('--families', default=','.join(FAMILIES),
                        help='Explicit comma-separated families; use a new output directory for each phase.')
    parser.add_argument('--prefix', default='G8', help='Use a new prefix for a separate search phase.')
    args = parser.parse_args()
    if args.output.exists():
        raise FileExistsError(args.output)
    base = json.loads(args.base.read_text('utf-8-sig'))
    broker = json.loads(args.broker.read_text('utf-8-sig'))
    families = tuple(args.families.split(','))
    rows = generate(base, broker, args.count, args.seed, families, args.prefix)
    known = set(re.findall(r'pub\s+(\w+)\s*:', args.settings_source.read_text('utf-8')))
    for row in rows:
        unknown = set(row['changes'])-known
        if unknown:
            raise ValueError(f'Unknown axes (would be silently ignored by serde): {unknown}')
        assert row['settings']['lot_max'] == 5.0
    args.output.mkdir(parents=True)
    configurations = args.output/'configs'
    configurations.mkdir()
    manifest_rows = []
    for row in rows:
        path = configurations/(row['id']+'.json')
        payload = {'name': row['id'], 'nazwa': row['id'], 'format': 'Synergy',
                   'description': 'Research candidate; not a selected production default.',
                   'settings': row['settings']}
        path.write_text(json.dumps(payload, indent=2), encoding='utf-8')
        manifest_rows.append({k:v for k,v in row.items() if k!='settings'})
    manifest = {'schema': 'conduit.giga-sweep8-space.v2', 'seed': args.seed,
                'candidate_count': len(rows), 'search_lot_cap': 5.0,
                'base_sha256': hashlib.sha256(args.base.read_bytes()).hexdigest(),
                'broker_profile': broker, 'families': list(families), 'prefix': args.prefix,
                'candidates': manifest_rows,
                'invariants': ['No date/hour/message-ID filter axes.',
                               'Pending-order and entry-ingress policies are inherited from the supplied preset.',
                               'Publisher validity is independent of preset-driven order management.',
                               'Broker costs are fixed across all candidates.',
                               'Quick results cannot enter coronation without exact reruns.',
                               'GOD-X8 name is reserved for the owner-selected candidate.']}
    (args.output/'manifest.json').write_text(json.dumps(manifest, indent=2), encoding='utf-8')
    print(json.dumps({'count': len(rows), 'seed': args.seed, 'lot_max': 5.0}))


if __name__ == '__main__':
    main()
