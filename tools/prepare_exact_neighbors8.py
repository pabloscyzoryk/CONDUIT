"""Add nearby existing cap-5 configurations to an exact training queue.

This probes local parameter sensitivity before later/full-window results are
used. Neighbors are research cases, not automatically qualified candidates.
"""
from __future__ import annotations
import argparse
import copy
import json
import math
from pathlib import Path

from giga_sweep8 import BROKER_FIELDS, fingerprint
from research_runner import file_hash
from training_contract8 import require_screening_selection


def local_distance(a, b):
    changed = [key for key in a.keys() | b.keys() if a.get(key) != b.get(key)]
    if not 1 <= len(changed) <= 2 or set(changed) & (BROKER_FIELDS | {'lot_max'}):
        return None
    distance = 0.
    for key in changed:
        x, y = a.get(key), b.get(key)
        if any(isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v) for v in (x, y)):
            return None
        relative = abs(x-y)/max(abs(x), abs(y), 1e-12)
        if relative > .5:
            return None
        distance += relative
    return (len(changed), distance, sorted(changed))


def augment(selection, manifest, base, maximum=60):
    require_screening_selection(selection)
    result = copy.deepcopy(selection)
    selected = result['selected_for_exact_replay']
    if maximum < len(selected):
        raise ValueError('Neighborhood audit cannot remove the existing exact queue')
    by_name = {row['id']: row for row in manifest['candidates']}
    reconstructed = {name: {**base, **row['changes']} for name, row in by_name.items()}
    for name, settings in reconstructed.items():
        if settings['lot_max'] != 5 or fingerprint(settings) != by_name[name]['fingerprint']:
            raise ValueError('Manifest deltas do not reproduce the immutable cap-5 configuration')
    measured = [r for r in selected if r['name'] != 'GOD-X7-cap5' and not r.get('screening_rejections')]
    anchors = {}
    for key in ('worst_capital_ratio_to_reference', 'lowest_positive_equity_days_pct'):
        for row in sorted(measured, key=lambda r: (-r[key], r['name']))[:3]:
            anchors[row['name']] = row
    chosen = {r['name'] for r in selected}
    added = []
    for name, parent in anchors.items():
        neighbors = []
        for child, settings in reconstructed.items():
            if child in chosen or by_name[child]['family'] != parent['family']:
                continue
            distance = local_distance(reconstructed[name], settings)
            if distance is not None:
                neighbors.append((distance[0], distance[1], child, distance[2]))
        for fields, distance, child, keys in sorted(neighbors)[:2]:
            if len(selected) >= maximum:
                break
            row = by_name[child]
            selected.append({'name': child, 'family': row['family'], 'fingerprint': row['fingerprint'],
                'selection_reasons': ['nearby_parameter_sensitivity_exact_training'],
                'neighbor_of': name, 'changed_fields': keys,
                'relative_parameter_distance': distance,
                'prior_screening_required': False})
            chosen.add(child)
            added.append({'name': child, 'neighbor_of': name, 'changed_fields': keys})
    result['local_sensitivity_plan'] = {'added': added, 'maximum_queue': maximum,
        'rules': 'Same family; one or two numeric strategy axes; unchanged categories, lot cap and broker profile.',
        'interpretation': 'Equal outcomes may indicate an inactive branch. Neighbor results are not out-of-sample evidence.',
        'all_training_at_cap5': True, 'later_results_used_to_choose_neighbors': False}
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--selection', required=True, type=Path)
    p.add_argument('--space', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--maximum', type=int, default=60)
    args = p.parse_args()
    if args.out.exists():
        raise FileExistsError('Exact queue is immutable once prepared')
    manifest = args.space/'manifest.json'
    base = args.space/'configs/GOD-X7-cap5.json'
    result = augment(json.loads(args.selection.read_text('utf-8-sig')),
        json.loads(manifest.read_text('utf-8-sig')), json.loads(base.read_text('utf-8-sig'))['settings'], args.maximum)
    result['neighbor_provenance'] = {'selection_sha256': file_hash(args.selection), 'manifest_sha256': file_hash(manifest),
        'base_sha256': file_hash(base), 'tool_sha256': file_hash(Path(__file__))}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2, ensure_ascii=False, allow_nan=False), encoding='utf-8')
    print(json.dumps({'exact_queue': len(result['selected_for_exact_replay']),
        'sensitivity_neighbors': len(result['local_sensitivity_plan']['added']), 'launched': False}))


if __name__ == '__main__':
    main()
