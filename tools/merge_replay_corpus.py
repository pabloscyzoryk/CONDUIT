"""Prefer observed Telegram revisions to final-only export snapshots.

This produces a MIXED-coverage research corpus, never a claim of full historical
live parity. An observed message's entire export snapshot is removed. Otherwise
an old/final text might reappear after its recorded revisions. Unknown original
versions are not invented. Input chronicle files must cover disjoint intervals.
"""
from __future__ import annotations
import argparse
from collections import Counter
import copy
import hashlib
import json
from pathlib import Path


def merge(export: dict, chronicles: list[dict]) -> dict:
    if any(m.get('provenance', {}).get('version') == 'final_only'
           and m['ts'] < m.get('provenance', {}).get('telegram_edited_ms', m['ts'])
           for m in export['messages']):
        raise ValueError('Export leaks final edited text before its edit timestamp')
    observed = []
    intervals = []
    for source_index, corpus in enumerate(chronicles):
        messages = corpus['messages']
        if not messages:
            continue
        interval = (min(m['ts'] for m in messages), max(m['ts'] for m in messages))
        if any(interval[0] <= end and begin <= interval[1] for begin, end in intervals):
            raise ValueError('Overlapping chronicle intervals require explicit physical-event reconciliation')
        intervals.append(interval)
        for index, original in enumerate(messages):
            message = copy.deepcopy(original)
            message.setdefault('provenance', {}).update({
                'research_source': 'observed_chronicle',
                'research_source_index': source_index,
                'original_receive_seq': message.get('receive_seq'),
                'original_row_index': index,
            })
            observed.append(message)
    observed_keys = {(m.get('kanal', ''), m['msg_id']) for m in observed}
    retained = []
    for original in export['messages']:
        if (original.get('kanal', ''), original['msg_id']) in observed_keys:
            continue
        message = copy.deepcopy(original)
        message.setdefault('provenance', {})['research_source'] = 'final_only_export'
        retained.append(message)
    combined = observed + retained
    combined.sort(key=lambda m: (m['ts'], m.get('receive_seq', 2**63-1)))
    for index, message in enumerate(combined):
        # Stable merge order, not an invented measured delivery sequence.
        message['receive_seq'] = index
    return {
        'schema': 'conduit.raw-message-stream.v1',
        'provenance': {
            'source_kind': 'mixed_observed_chronicle_and_causal_final_export',
            'historical_coverage_complete': False,
            'receive_seq_semantics': 'stable merge order; original measured sequence retained in provenance',
            'limitations': [
                'Export-only messages lack pre-edit text and measured local arrival.',
                'Observed revisions replace whole export records; missing originals are never synthesized.',
                'Chronicle outages and incomplete first-observation histories remain incomplete.',
                'Older imported chronicles may have omitted empty edits; see each source manifest.',
                'This mixed corpus cannot certify a full-window 1:1 live outcome.',
            ],
            'observed_intervals_ms': intervals,
        },
        'counts': {
            'messages': len(combined), 'observed_revisions': len(observed),
            'observed_message_ids': len(observed_keys), 'export_only_records': len(retained),
            'export_records_replaced': len(export['messages'])-len(retained),
            'per_source': dict(Counter(m['provenance']['research_source'] for m in combined)),
        },
        'messages': combined,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--export', required=True, type=Path)
    parser.add_argument('--chronicle', action='append', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    if args.output.exists():
        raise FileExistsError(args.output)
    paths = [args.export, *args.chronicle]
    sources = [json.loads(p.read_text('utf-8-sig')) for p in paths]
    payload = merge(sources[0], sources[1:])
    payload['provenance']['inputs'] = [
        {'path': str(p.resolve()), 'sha256': hashlib.sha256(p.read_bytes()).hexdigest()}
        for p in paths
    ]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(payload, ensure_ascii=False, separators=(',', ':')), encoding='utf-8')
    print(json.dumps(payload['counts']))


if __name__ == '__main__':
    main()
