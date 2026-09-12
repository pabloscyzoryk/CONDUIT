"""Read-only, headless Synergy history export with immutable acquisition evidence.

Uses an already authorized CONDUIT grammers session in memory. It never logs in,
sends, joins, acknowledges reads or downloads media. Only the uniquely configured
Synergy binding is eligible. Each output directory must be new; older snapshots
are never inputs to formatting and are never overwritten. Current server values
are not a historical snapshot and are not automatically qualified for backtests.
Install Telethon 1.45.0 separately; --help/--version do not import it or read auth.
"""
from __future__ import annotations

import argparse
import asyncio
import base64
import datetime as dt
import hashlib
import json
import logging
import os
from pathlib import Path
import stat
import sys
from types import SimpleNamespace

from telegram_desktop_format import canonical_projection, desktop_json, format_export

VERSION = '1.0.0'
MAX_INPUT_BYTES = 16 * 1024 * 1024
MAX_OUTPUT_BYTES = 256 * 1024 * 1024
MAX_RECORDS = 100_000
MAX_PAGES = 1001


class ExportError(ValueError):
    """Messages are fixed categories, never a server response or private value."""


def check(condition, category):
    if not condition:
        raise ExportError(category)


def no_links(path):
    path = Path(os.path.abspath(path))
    for item in (*reversed(path.parents), path):
        info = item.lstat()
        check(not stat.S_ISLNK(info.st_mode)
              and not getattr(info, 'st_file_attributes', 0) & 0x400,
              'symlink_or_reparse_path')
    return path


def read_input(path):
    path = no_links(path)
    check(path.is_file() and path.stat().st_size <= MAX_INPUT_BYTES,
          'input_missing_or_too_large')
    with path.open('rb') as handle:
        raw = handle.read(MAX_INPUT_BYTES + 1)
    check(len(raw) <= MAX_INPUT_BYTES, 'input_too_large')
    return raw


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def safe_raw(value):
    """Private evidence preserves content; access/auth keys are not exported."""
    if isinstance(value, dict):
        return {k: safe_raw(v) for k, v in value.items()
                if k not in ('access_hash', 'auth_key')}
    if isinstance(value, (list, tuple)):
        return [safe_raw(v) for v in value]
    if isinstance(value, dt.datetime):
        return value.isoformat()
    if isinstance(value, bytes):
        return {'bytes_base64': base64.b64encode(value).decode('ascii')}
    return value


class Evidence:
    def __init__(self, directory):
        parent = no_links(directory.parent)
        self.root = parent / directory.name
        self.root.mkdir(exist_ok=False)
        self.bytes = 0
        self.pins = []

    def write(self, relative, value, *, raw=False):
        check(relative and '/' not in relative and '\\' not in relative
              and ':' not in relative and relative not in ('.', '..'),
              'unsafe_output_name')
        data = value if raw else json.dumps(safe_raw(value), ensure_ascii=False,
                                           indent=1, allow_nan=False).encode('utf-8')
        check(self.bytes + len(data) <= MAX_OUTPUT_BYTES, 'output_budget_exceeded')
        with (self.root / relative).open('xb') as handle:
            handle.write(data)
        self.bytes += len(data)
        pin = {'file': relative, 'bytes': len(data), 'sha256': digest(data)}
        self.pins.append(pin)
        return pin


def template_inputs(directory):
    directory = no_links(directory)
    paths = [directory / name for name in ('secrets.json', 'telegram.session', 'channels.json')]
    raw = [read_input(path) for path in paths]
    secrets, session, channels = [json.loads(value.decode('utf-8-sig')) for value in raw]
    telegram = secrets['telegram']
    serialized = json.loads(base64.b64decode(telegram['sessionString'], validate=True))

    def identity(doc):
        return doc['home_dc'], {x['id']: x.get('auth_key') for x in doc['dc_options']}

    check(identity(serialized) == identity(session), 'session_pair_mismatch')
    bindings = channels['bindings']
    values = list(bindings.values()) if isinstance(bindings, dict) else bindings
    selected = [v for v in values if str(v.get('format', '')).casefold() == 'synergy']
    check(len(selected) == 1, 'synergy_binding_not_unique')
    channel_id = int(selected[0]['channelId'])
    # Bot API -100 prefix and bare positive channel identity are supported.
    check(channel_id > 0 or str(channel_id).startswith('-100'), 'invalid_channel_binding')
    target = channel_id if channel_id > 0 else int(str(channel_id)[4:])
    check(target > 0, 'invalid_channel_binding')
    dc = next(x for x in serialized['dc_options'] if x['id'] == serialized['home_dc'])
    key = bytes.fromhex(dc['auth_key'])
    check(len(key) == 256, 'invalid_session_key_size')
    host, port = dc['ipv4'].rsplit(':', 1)
    check(type(telegram['apiId']) is int and telegram['apiId'] > 0
          and isinstance(telegram['apiHash'], str) and len(telegram['apiHash']) == 32,
          'invalid_api_pair')
    return SimpleNamespace(target=target, dc=dc['id'], host=host, port=int(port),
                           key=key, api_id=telegram['apiId'], api_hash=telegram['apiHash'],
                           paths=paths, original=raw)


def load_sdk():
    import telethon
    from telethon import TelegramClient, functions, types, utils, errors
    from telethon.crypto import AuthKey
    from telethon.sessions import MemorySession
    check(telethon.__version__ == '1.45.0', 'unqualified_telethon_version')
    return SimpleNamespace(Client=TelegramClient, functions=functions, types=types,
                           utils=utils, errors=errors, AuthKey=AuthKey,
                           MemorySession=MemorySession, version=telethon.__version__)


def create_client(sdk, template):
    session = sdk.MemorySession()
    session.set_dc(template.dc, template.host, template.port)
    session.auth_key = sdk.AuthKey(template.key)
    return sdk.Client(session, template.api_id, template.api_hash,
                      receive_updates=False, catch_up=False, request_retries=0,
                      connection_retries=0, auto_reconnect=False, flood_sleep_threshold=0,
                      timeout=12, device_model='CONDUIT Synergy history exporter',
                      system_version='headless export', app_version=VERSION)


def ingest_page(page, sdk, target, boundary, cursor, rows):
    raw = []
    for message in page.messages:
        check(isinstance(message, (sdk.types.Message, sdk.types.MessageService)),
              'unavailable_or_unknown_history_record')
        check(isinstance(message.peer_id, sdk.types.PeerChannel)
              and message.peer_id.channel_id == target, 'wrong_peer_in_history')
        check(type(message.id) is int and 0 < message.id <= boundary
              and (not cursor or message.id < cursor), 'history_range_violation')
        check(message.id not in rows, 'duplicate_history_record')
        payload = message.to_dict()
        rows[message.id] = payload
        raw.append(payload)
    check(len(rows) <= MAX_RECORDS, 'history_record_budget_exceeded')
    return raw


def emoji_ids(rows):
    result = set()
    for row in rows:
        for e in row.get('entities') or []:
            if e.get('_') == 'MessageEntityCustomEmoji': result.add(e['document_id'])
        for r in (row.get('reactions') or {}).get('results') or []:
            if r['reaction'].get('_') == 'ReactionCustomEmoji':
                result.add(r['reaction']['document_id'])
    check(len(result) <= 2000, 'emoji_metadata_budget_exceeded')
    return sorted(result)


async def acquire(args, evidence, report, template, sdk):
    client = create_client(sdk, template)
    f, types, utils = sdk.functions, sdk.types, sdk.utils
    target, boundary = template.target, args.through_message_id
    permitted_emoji = set()

    async def request(req):
        if isinstance(req, f.messages.GetDialogsRequest):
            kind = 'dialog_metadata'
            check(report['metadata_requests'] < 25 and req.limit == 20,
                  'metadata_request_budget')
            report['metadata_requests'] += 1
        elif isinstance(req, f.messages.GetHistoryRequest):
            kind = 'synergy_history'
            check(req.peer.channel_id == target and req.limit == 100
                  and req.max_id == (boundary + 1 if boundary is not None else 0),
                  'history_request_outside_scope')
            check(report['history_requests'] < MAX_PAGES, 'history_request_budget')
            report['history_requests'] += 1
        elif isinstance(req, f.messages.GetCustomEmojiDocumentsRequest):
            kind = 'related_emoji_metadata'
            check(0 < len(req.document_id) <= 100
                  and set(req.document_id) <= permitted_emoji, 'emoji_request_outside_scope')
            report['emoji_metadata_requests'] += 1
        else:
            raise ExportError('request_outside_readonly_allowlist')
        for attempt in range(3):
            report['rpc_attempts_by_kind'][kind] = report['rpc_attempts_by_kind'].get(kind, 0) + 1
            try:
                return await asyncio.wait_for(client(req), timeout=30)
            except sdk.errors.FloodWaitError as error:
                check(attempt < 2 and 0 <= error.seconds <= 60, 'flood_wait_exceeds_budget')
                report['bounded_flood_waits'] += 1
                await asyncio.sleep(error.seconds + 1)
            except sdk.errors.TypeNotFoundError as error:
                # A fresh request for the exact same read-only page can succeed
                # after a malformed/unrecognized RPC response. No decoded rows
                # or cursor are committed before this call returns successfully.
                # Repeated failure stops the export; never skip the page or log
                # the exception's remaining bytes, which contain private data.
                report['tl_decode_failures'].append({'request_kind': kind,
                    'constructor_hex': f'{error.invalid_constructor_id:08x}'})
                check(attempt < 2, 'unsupported_tl_response_after_retries')
                await asyncio.sleep(1)

    try:
        await asyncio.wait_for(client.connect(), timeout=25)
        query = f.messages.GetDialogsRequest(offset_date=None, offset_id=0,
            offset_peer=types.InputPeerEmpty(), limit=20, hash=0, exclude_pinned=False)
        peer = None
        for _ in range(25):
            page = await request(query)
            report['metadata_entries_received'] += len(page.dialogs)
            report['incidental_top_messages_not_exported'] += len(page.messages)
            found = [c for c in page.chats if getattr(c, 'id', None) == target]
            if found:
                check(len(found) == 1 and isinstance(found[0], types.Channel)
                      and found[0].access_hash and 'synergy' in found[0].title.casefold(),
                      'resolved_peer_not_synergy')
                channel = found[0]
                peer = types.InputPeerChannel(channel.id, channel.access_hash)
                break
            if not page.dialogs or not isinstance(page, types.messages.DialogsSlice): break
            last = page.dialogs[-1]
            emap = {utils.get_peer_id(e): e for e in [*page.users, *page.chats]}
            message = next((m for m in page.messages if m.id == last.top_message
                and utils.get_peer_id(m.peer_id) == utils.get_peer_id(last.peer)), None)
            check(message is not None, 'metadata_cursor_missing')
            query = f.messages.GetDialogsRequest(offset_date=message.date, offset_id=last.top_message,
                offset_peer=utils.get_input_peer(emap[utils.get_peer_id(last.peer)]),
                limit=20, hash=0, exclude_pinned=True)
        check(peer is not None, 'synergy_not_resolved_within_budget')
        rows, users = {}, {}
        cursor = boundary + 1 if boundary is not None else 0
        for sequence in range(MAX_PAGES):
            page = await request(f.messages.GetHistoryRequest(peer=peer, offset_id=cursor,
                offset_date=None, add_offset=0, limit=100,
                max_id=boundary + 1 if boundary is not None else 0, min_id=0, hash=0))
            if boundary is None:
                check(bool(page.messages), 'empty_latest_history')
                boundary = max(m.id for m in page.messages)
                report['boundary_frozen_from_first_history_page'] = True
            raw = ingest_page(page, sdk, target, boundary, cursor, rows)
            for user in page.users: users[user.id] = user.to_dict()
            evidence.write(f'page_{sequence:04}.private.json', {
                'messages': raw, 'users': [] if not raw else [u.to_dict() for u in page.users],
                'observed_utc': dt.datetime.now(dt.timezone.utc).isoformat()})
            if not raw:
                report['available_history_eof_observed'] = True
                break
            cursor = min(r['id'] for r in raw)
            if sequence % 10 == 0:
                print(json.dumps({'stage': 'Synergy history', 'pages': sequence + 1,
                                  'records': len(rows)}), flush=True)
            await asyncio.sleep(.3)
        check(report.get('available_history_eof_observed'), 'history_page_budget_exceeded')
        check(boundary in rows, 'requested_boundary_unavailable')
        permitted_emoji.update(emoji_ids(rows.values()))
        emoji_status = {}
        ordered = sorted(permitted_emoji)
        for start in range(0, len(ordered), 100):
            batch = ordered[start:start + 100]
            documents = await request(f.messages.GetCustomEmojiDocumentsRequest(document_id=batch))
            check(all(getattr(d, 'id', None) in batch for d in documents),
                  'unexpected_emoji_document')
            available = {d.id for d in documents if isinstance(d, types.Document)}
            emoji_status.update({str(i): i in available for i in batch})
            evidence.write(f'emoji_{start // 100:03}.private.json',
                           {'documents': [d.to_dict() for d in documents]})
        snapshot = safe_raw({'channel': {'id': channel.id, 'title': channel.title,
            'username': channel.username, 'broadcast': channel.broadcast,
            'megagroup': channel.megagroup}, 'messages': [rows[i] for i in sorted(rows)],
            'users': list(users.values()), 'custom_emoji_status': emoji_status})
        report['snapshot'] = evidence.write('network_history.private.json', snapshot)
        report.update(records=len(rows), service_records=sum(r['_'] == 'MessageService' for r in rows.values()),
                      end_message_id=boundary, independently_generated_snapshot=True)
        document = format_export(snapshot, args.utc_offset_minutes)
        formatted = desktop_json(document).encode('utf-8')
        desktop = evidence.root / 'desktop'
        desktop.mkdir(exist_ok=False)
        check(evidence.bytes + len(formatted) <= MAX_OUTPUT_BYTES, 'output_budget_exceeded')
        with (desktop / 'result.json').open('xb') as handle: handle.write(formatted)
        evidence.bytes += len(formatted)
        report['desktop'] = {'file': 'desktop/result.json', 'bytes': len(formatted), 'sha256': digest(formatted)}
        if args.canonical_time_mode:
            report['canonical'] = evidence.write('canonical.private.json',
                canonical_projection(document, formatted, args.canonical_time_mode))
        report['status'] = 'COMPLETE_AVAILABLE_HISTORY_EXPORT'
    finally:
        await asyncio.wait_for(client.disconnect(), timeout=10)


def parser():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--version', action='version', version=VERSION)
    p.add_argument('--template-dir', type=Path, required=True,
                   help='Existing CONDUIT folder containing matching secrets.json, telegram.session and channels.json.')
    p.add_argument('--output-dir', type=Path, required=True, help='New private directory; parent must already exist.')
    boundary = p.add_mutually_exclusive_group(required=True)
    boundary.add_argument('--through-message-id', type=int, help='Inclusive final Synergy message; must still be available.')
    boundary.add_argument('--latest', action='store_true', help='Freeze latest available message from the first history page.')
    p.add_argument('--utc-offset-minutes', type=int, required=True,
                   help='Explicit display-time offset. Unix timestamps are never shifted.')
    p.add_argument('--canonical-time-mode', choices=['publication-final', 'last-edit-final'],
                   help='Optional loader sidecar; explicit final-text timing, never an observed event chronicle.')
    return p


def main(argv=None):
    args = parser().parse_args(argv)
    logging.disable(logging.CRITICAL)
    report = {'schema': 'conduit.synergy-headless-export.v1', 'status': 'STARTED',
        'started_utc': dt.datetime.now(dt.timezone.utc).isoformat(),
        'producer_sha256': digest(Path(__file__).read_bytes()),
        'formatter_sha256': digest(Path(__file__).with_name('telegram_desktop_format.py').read_bytes()),
        'metadata_requests': 0, 'history_requests': 0, 'emoji_metadata_requests': 0,
        'metadata_entries_received': 0, 'incidental_top_messages_not_exported': 0,
        'rpc_attempts_by_kind': {}, 'tl_decode_failures': [],
        'bounded_flood_waits': 0, 'other_channel_history_requests': 0,
        'sends_joins_read_acknowledgements': 0, 'media_downloads': 0,
        'session_material_written': False, 'historical_byte_parity_verified': False,
        'eligible_for_backtest_replacement': False,
        'coverage': 'Available server history to inclusive boundary; deleted records and prior edits cannot be recovered. Pagination is not an atomic historical snapshot.',
        'utc_offset_minutes': args.utc_offset_minutes}
    evidence = None
    template = None
    try:
        check(args.through_message_id is None or 0 < args.through_message_id < 2**31 - 1,
              'invalid_boundary')
        check(abs(args.utc_offset_minutes) <= 840, 'invalid_display_offset')
        template_path = no_links(args.template_dir)
        output = Path(os.path.abspath(args.output_dir))
        check(not output.is_relative_to(template_path) and not template_path.is_relative_to(output),
              'output_overlaps_template')
        check(not output.exists(), 'output_already_exists')
        template = template_inputs(template_path)
        sdk = load_sdk()
        evidence = Evidence(output)
        report['telethon_version'] = sdk.version
        asyncio.run(asyncio.wait_for(acquire(args, evidence, report, template, sdk), timeout=1800))
    except Exception as error:
        report.update(status='FAILED', error_class=type(error).__name__)
        if isinstance(error, ExportError): report['guard_category'] = str(error)
    finally:
        if template is not None:
            try:
                report['source_auth_session_bindings_unchanged'] = all(
                    read_input(path) == raw for path, raw in zip(template.paths, template.original))
            except Exception:
                report['source_auth_session_bindings_unchanged'] = False
            if not report['source_auth_session_bindings_unchanged']:
                report['status'] = 'FAILED'
                report['guard_category'] = 'source_inputs_changed'
    report['finished_utc'] = dt.datetime.now(dt.timezone.utc).isoformat()
    if evidence is not None:
        report['artifacts'] = evidence.pins
        evidence.write('EXPORT_RECEIPT.json', report)
    print(json.dumps({k: report[k] for k in ('status', 'records', 'history_requests',
                      'error_class', 'guard_category') if k in report}))
    return 0 if report['status'] == 'COMPLETE_AVAILABLE_HISTORY_EXPORT' else 2


if __name__ == '__main__':
    sys.exit(main())
