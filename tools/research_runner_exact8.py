"""Bind detailed histories at completion without changing the screening runner."""
from __future__ import annotations
import json
from pathlib import Path
import re
import sys
import research_runner as runner

SCHEMA = 'conduit.completed-detail-artifacts.v1'


def detail_manifest(files):
    artifacts = {}
    for summary in files:
        rows = json.loads(summary.read_text('utf-8-sig'))
        if rows.get('approximate') is True:
            raise ValueError('Exact receipt cannot bind approximate screening results')
        if summary.stem != 'wyniki_compound':
            raise ValueError('This exact protocol requires compound account histories')
        for name in rows:
            if not re.fullmatch(r'[A-Za-z0-9_-]+', name):
                raise ValueError('Unsafe result candidate identifier')
            path = summary.parent / (name + '_compound_dane.json')
            if not path.is_file():
                raise ValueError('Missing detailed history: ' + path.name)
            artifacts[path.name] = {'sha256': runner.file_hash(path), 'bytes': path.stat().st_size}
    if not artifacts:
        raise ValueError('No detailed account histories to bind')
    return artifacts


def verify_detail_binding(path, receipt):
    record = receipt.get('detail_artifacts', {}).get(path.name)
    if receipt.get('detail_artifacts_schema') != SCHEMA or not record:
        raise ValueError('Detailed history has no completion-bound receipt')
    digest = runner.file_hash(path)
    if record.get('bytes') != path.stat().st_size or record.get('sha256') != digest:
        raise ValueError('Detailed history changed after completion')
    return digest


def main():
    if len(sys.argv) != 2:
        raise ValueError('Exactly one immutable exact plan is required')
    plan = json.loads(Path(sys.argv[1]).read_text('utf-8-sig'))
    inputs = {Path(a['path']).resolve(): a['sha256'] for a in plan['inputs']}
    for path in (Path(__file__).resolve(), Path(runner.__file__).resolve()):
        if inputs.get(path) != runner.file_hash(path):
            raise ValueError('Exact plan must bind both runner modules')
    for job in plan['jobs']:
        path = Path(plan['output']) / job['id'] / 'receipt.json'
        if path.exists():
            receipt = json.loads(path.read_text('utf-8-sig'))
            if receipt.get('status') == 'complete':
                for filename in receipt.get('detail_artifacts', {}):
                    verify_detail_binding(Path(job['result_dir']) / filename, receipt)
                if receipt.get('detail_artifacts_schema') != SCHEMA or not receipt.get('detail_artifacts'):
                    raise ValueError('Cannot resume completed exact receipts without bound histories')
    original_validate, original_write = runner.validate_results, runner.write_json
    manifests = {}

    def validate(out, expected_count=None):
        files, errors = original_validate(out, expected_count)
        if not errors:
            try:
                manifests[out.resolve()] = detail_manifest(files)
            except (OSError, ValueError, TypeError, KeyError) as exc:
                errors.append('Detailed history binding failed: ' + str(exc))
        return files, errors

    def write(path, payload):
        if path.name == 'receipt.json' and payload.get('status') == 'complete':
            directory = Path(payload['result_files'][0]).parent.resolve()
            payload['detail_artifacts_schema'] = SCHEMA
            payload['detail_artifacts'] = manifests[directory]
        original_write(path, payload)

    runner.validate_results, runner.write_json = validate, write
    try:
        return runner.main()
    finally:
        runner.validate_results, runner.write_json = original_validate, original_write


if __name__ == '__main__':
    raise SystemExit(main())
