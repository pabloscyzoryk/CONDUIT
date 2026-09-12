"""Strict, offline Telegram Desktop export comparison; never repairs inputs.

The candidate must be a separately generated export of the exact reference
range. Extra newer messages are a FAIL, not silently sliced or normalized.
PASS proves file equality only: this tool cannot prove that a producer really
downloaded the candidate rather than copying it. Keep independent execution
provenance before admitting an export into another workflow.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import stat
import sys
from pathlib import Path

SCHEMA = 'conduit.telegram-export-byte-verification.v1'
MAX_JSON = 256 * 1024 * 1024
MAX_TOTAL = 16 * 1024 * 1024 * 1024
MAX_FILES = 100_000


class InvalidExport(ValueError):
    pass


def no_reparse(path: Path) -> Path:
    absolute = Path(os.path.abspath(path))
    for current in (*reversed(absolute.parents), absolute):
        info = current.lstat()
        if stat.S_ISLNK(info.st_mode) or getattr(info, 'st_file_attributes', 0) & 0x400:
            raise InvalidExport('reparse_or_symlink_input')
    return absolute


def unique_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise InvalidExport('duplicate_json_key')
        value[key] = item
    return value


def invalid_constant(_):
    raise InvalidExport('nonfinite_json_number')


def positive_id(value):
    if type(value) is not int or value <= 0:
        raise InvalidExport('invalid_message_or_channel_identity')
    return value


def parse_export(path: Path):
    no_reparse(path)
    if path.stat().st_size > MAX_JSON:
        raise InvalidExport('json_size_limit')
    with path.open('rb') as handle:
        raw = handle.read(MAX_JSON + 1)
    if len(raw) > MAX_JSON:
        raise InvalidExport('json_size_limit')
    try:
        value = json.loads(raw.decode('utf-8-sig'),
                           object_pairs_hook=unique_object,
                           parse_constant=invalid_constant)
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise InvalidExport('invalid_utf8_or_json') from None
    if not isinstance(value, dict) or not isinstance(value.get('messages'), list):
        raise InvalidExport('expected_original_desktop_export')
    positive_id(value.get('id'))
    if not isinstance(value.get('name'), str) or not isinstance(value.get('type'), str):
        raise InvalidExport('missing_desktop_channel_metadata')
    ids = []
    for row in value['messages']:
        if not isinstance(row, dict):
            raise InvalidExport('invalid_message_record')
        ids.append(positive_id(row.get('id')))
    if len(set(ids)) != len(ids) or ids != sorted(ids):
        raise InvalidExport('duplicate_or_unordered_messages')
    return value, ids


def file_digest(path: Path):
    no_reparse(path)
    before = path.stat()
    if not stat.S_ISREG(before.st_mode):
        raise InvalidExport('nonregular_input')
    digest = hashlib.sha256()
    length = 0
    with path.open('rb') as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b''):
            length += len(chunk)
            if length > MAX_TOTAL:
                raise InvalidExport('total_size_limit')
            digest.update(chunk)
    after = path.stat()
    if length != before.st_size or (before.st_size, before.st_mtime_ns, before.st_ino) != (after.st_size, after.st_mtime_ns, after.st_ino):
        raise InvalidExport('input_changed_during_comparison')
    return {'bytes': length, 'sha256': digest.hexdigest()}


def inventory(root: Path):
    no_reparse(root)
    if not root.is_dir():
        raise InvalidExport('input_must_be_export_directory')
    records = {}
    total = 0
    entries = 0
    def walk_error(_):
        raise InvalidExport('directory_read_failed')
    for current, dirs, files in os.walk(root, followlinks=False, onerror=walk_error):
        entries += len(dirs) + len(files)
        if entries > MAX_FILES:
            raise InvalidExport('entry_count_limit')
        for name in dirs:
            no_reparse(Path(current) / name)
        for name in sorted(files):
            path = Path(current) / name
            if len(records) >= MAX_FILES:
                raise InvalidExport('file_count_limit')
            record = file_digest(path)
            total += record['bytes']
            if total > MAX_TOTAL:
                raise InvalidExport('total_size_limit')
            records[path.relative_to(root).as_posix()] = record
    if 'result.json' not in records:
        raise InvalidExport('missing_result_json')
    return records


def verify(reference: Path, candidate: Path):
    reference = no_reparse(reference)
    candidate = no_reparse(candidate)
    if reference == candidate or reference in candidate.parents or candidate in reference.parents:
        raise InvalidExport('reference_and_candidate_must_be_separate')
    ref_files = inventory(reference)
    new_files = inventory(candidate)
    ref, ref_ids = parse_export(reference / 'result.json')
    new, new_ids = parse_export(candidate / 'result.json')
    common = ref_files.keys() & new_files.keys()
    mismatches = [key for key in common if ref_files[key] != new_files[key]]
    range_equal = ref_ids == new_ids
    identity_equal = all(ref[key] == new[key] for key in ('id', 'name', 'type'))
    names_equal = ref_files.keys() == new_files.keys()
    # Re-inventory catches additions/removals and changes after reading JSON.
    if ref_files != inventory(reference) or new_files != inventory(candidate):
        raise InvalidExport('input_changed_during_comparison')
    passed = names_equal and not mismatches and range_equal and identity_equal
    return {
        'schema': SCHEMA, 'status': 'PASS' if passed else 'FAIL',
        'literal_file_bytes_equal': passed,
        'same_channel_identity': identity_equal,
        'exact_reference_message_range_and_order': range_equal,
        'reference_records': len(ref_ids), 'candidate_records': len(new_ids),
        'common_message_count': len(set(ref_ids) & set(new_ids)),
        'reference_only_message_count': len(set(ref_ids) - set(new_ids)),
        'candidate_only_message_count': len(set(new_ids) - set(ref_ids)),
        'relative_file_set_equal': names_equal,
        'reference_file_count': len(ref_files), 'candidate_file_count': len(new_files),
        'reference_bytes': sum(v['bytes'] for v in ref_files.values()),
        'candidate_bytes': sum(v['bytes'] for v in new_files.values()),
        'different_file_bytes_count': len(mismatches),
        'reference_only_file_count': len(ref_files.keys() - new_files.keys()),
        'candidate_only_file_count': len(new_files.keys() - ref_files.keys()),
        'reference_result': ref_files['result.json'],
        'candidate_result': new_files['result.json'],
        'network_calls': 0, 'input_mutations': 0, 'normalization_applied': False,
        'generation_provenance_verified': False,
        'eligible_as_independently_generated_export': False,
        'qualification_limit': 'PASS is byte equality only. Independently verify real producer execution; copied references do not qualify.',
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference', required=True, type=Path)
    parser.add_argument('--candidate', required=True, type=Path)
    parser.add_argument('--report', required=True, type=Path)
    args = parser.parse_args(argv)
    destination = Path(os.path.abspath(args.report))
    try:
        for root in (args.reference, args.candidate):
            absolute = Path(os.path.abspath(root))
            if destination == absolute or absolute in destination.parents:
                raise InvalidExport('report_must_be_outside_inputs')
        if destination.exists():
            raise InvalidExport('report_already_exists')
        no_reparse(destination.parent)
        report = verify(args.reference, args.candidate)
        with destination.open('x', encoding='utf-8', newline='\n') as handle:
            json.dump(report, handle, ensure_ascii=False, indent=2, allow_nan=False)
            handle.write('\n')
    except (OSError, InvalidExport) as error:
        category = str(error) if isinstance(error, InvalidExport) else 'filesystem_error'
        print(json.dumps({'status': 'ERROR', 'category': category}))
        return 2
    print(json.dumps({key: report[key] for key in ('status', 'literal_file_bytes_equal', 'reference_records', 'candidate_records', 'different_file_bytes_count', 'generation_provenance_verified')}))
    return 0 if report['status'] == 'PASS' else 3


if __name__ == '__main__':
    sys.exit(main())
