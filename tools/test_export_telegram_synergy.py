import asyncio
import base64
import contextlib
import datetime as dt
import io
import json
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace as NS
from unittest.mock import patch

import export_telegram_synergy as export


class PeerChannel:
    def __init__(self, channel_id): self.channel_id = channel_id


class InputPeerChannel(PeerChannel):
    def __init__(self, channel_id, access_hash):
        super().__init__(channel_id)
        self.access_hash = access_hash


class Message:
    def __init__(self, mid, peer=99):
        self.id, self.peer_id = mid, PeerChannel(peer)

    def to_dict(self):
        return {'_': 'Message', 'id': self.id, 'peer_id': {'_': 'PeerChannel', 'channel_id': self.peer_id.channel_id},
                'date': dt.datetime.fromtimestamp(1700000000, dt.timezone.utc),
                'message': 'Synthetic plain text', 'entities': []}


class MessageService(Message): pass
class Channel: pass
class Document: pass
class Request:
    def __init__(self, **kw): self.__dict__.update(kw)
class GetDialogsRequest(Request): pass
class GetHistoryRequest(Request): pass
class GetCustomEmojiDocumentsRequest(Request): pass
class FloodWaitError(Exception): pass
class TypeNotFoundError(Exception):
    invalid_constructor_id=0x12345678


class MemorySession:
    def set_dc(self, *args): self.dc = args


class FakeClient:
    instances = []

    def __init__(self, *args, **kwargs):
        self.options = kwargs
        self.calls = []
        self.connected = self.disconnected = False
        self.instances.append(self)

    async def connect(self): self.connected = True
    async def disconnect(self): self.disconnected = True

    async def __call__(self, request):
        self.calls.append(request)
        if isinstance(request, GetDialogsRequest):
            channel = Channel()
            channel.__dict__.update(id=99, title='Synthetic Synergy channel', access_hash=123,
                                    username=None, broadcast=True, megagroup=False)
            return NS(dialogs=[NS()], messages=[], chats=[channel], users=[])
        if isinstance(request, GetHistoryRequest):
            count = sum(isinstance(x, GetHistoryRequest) for x in self.calls)
            return NS(messages=[Message(5), Message(2)] if count == 1 else [], users=[])
        raise AssertionError('unexpected RPC')


def sdk():
    return NS(version='1.45.0', Client=FakeClient, MemorySession=MemorySession, AuthKey=lambda key:key,
              functions=NS(messages=NS(GetDialogsRequest=GetDialogsRequest, GetHistoryRequest=GetHistoryRequest,
                                      GetCustomEmojiDocumentsRequest=GetCustomEmojiDocumentsRequest)),
              types=NS(Message=Message, MessageService=MessageService, PeerChannel=PeerChannel,
                       InputPeerChannel=InputPeerChannel, InputPeerEmpty=lambda:NS(), Channel=Channel,
                       Document=Document, messages=NS(DialogsSlice=type('DialogsSlice', (), {}))),
              utils=NS(), errors=NS(FloodWaitError=FloodWaitError,TypeNotFoundError=TypeNotFoundError))


def make_template(root):
    directory = root / 'template'
    directory.mkdir()
    session = {'home_dc': 2, 'dc_options': [{'id': 2, 'auth_key': '22' * 256, 'ipv4': '127.0.0.1:443'}]}
    secret = {'telegram': {'apiId': 123, 'apiHash': 'aa' * 16,
                          'sessionString': base64.b64encode(json.dumps(session).encode()).decode()}}
    for name, value in [('secrets.json', secret), ('telegram.session', session),
                        ('channels.json', {'bindings': [{'channelId': '-10099', 'format': 'Synergy'}]})]:
        (directory / name).write_text(json.dumps(value), encoding='utf-8')
    return directory


class ExportTests(unittest.TestCase):
    def test_single_tl_failure_retries_same_page_without_skip(self):
        class RetryClient(FakeClient):
            async def __call__(self, request):
                if isinstance(request,GetHistoryRequest) and not getattr(self,'failed_once',False):
                    self.failed_once=True
                    self.failed_request=request
                    raise TypeNotFoundError()
                return await super().__call__(request)
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);template=make_template(root); fake=sdk();fake.Client=RetryClient
            with patch.object(export,'load_sdk',return_value=fake),contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(export.main(['--template-dir',str(template),'--output-dir',str(root/'out'),
                                               '--through-message-id','5','--utc-offset-minutes','0']),0)
            client=FakeClient.instances[-1]
            history=[x for x in client.calls if isinstance(x,GetHistoryRequest)]
            self.assertIs(client.failed_request,history[0])
            receipt=json.loads((root/'out/EXPORT_RECEIPT.json').read_text())
            self.assertEqual(receipt['history_requests'],2)
            self.assertEqual(receipt['rpc_attempts_by_kind']['synergy_history'],3)
            self.assertEqual(len(receipt['tl_decode_failures']),1)

    def test_repeated_tl_failure_is_not_complete(self):
        class BadClient(FakeClient):
            async def __call__(self,request):
                if isinstance(request,GetHistoryRequest):raise TypeNotFoundError()
                return await super().__call__(request)
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);template=make_template(root);fake=sdk();fake.Client=BadClient
            with patch.object(export,'load_sdk',return_value=fake),contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(export.main(['--template-dir',str(template),'--output-dir',str(root/'out'),
                                               '--latest','--utc-offset-minutes','0']),2)
            receipt=json.loads((root/'out/EXPORT_RECEIPT.json').read_text())
            self.assertEqual(receipt['status'],'FAILED')
            self.assertEqual(receipt['guard_category'],'unsupported_tl_response_after_retries')
            self.assertFalse((root/'out/desktop/result.json').exists())

    def test_help_and_version_do_not_read_inputs_or_load_sdk(self):
        for flag in ['--help', '--version']:
            with patch.object(export, 'template_inputs', side_effect=AssertionError('auth read')), \
                 patch.object(export, 'load_sdk', side_effect=AssertionError('network module')), \
                 contextlib.redirect_stdout(io.StringIO()), self.assertRaises(SystemExit) as result:
                export.main([flag])
            self.assertEqual(result.exception.code, 0)

    def test_scope_unique_and_session_pair(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = make_template(Path(tmp))
            self.assertEqual(export.template_inputs(directory).target, 99)
            path = directory/'channels.json'
            path.write_text(json.dumps({'bindings': [{'channelId':99,'format':'Synergy'}] * 2}))
            with self.assertRaisesRegex(export.ExportError, 'not_unique'): export.template_inputs(directory)
            path.write_text(json.dumps({'bindings': [{'channelId':99,'format':'Synergy'}]}))
            other = json.loads((directory/'telegram.session').read_text())
            other['dc_options'][0]['auth_key'] = '33' * 256
            (directory/'telegram.session').write_text(json.dumps(other))
            with self.assertRaisesRegex(export.ExportError, 'pair_mismatch'): export.template_inputs(directory)

    def test_readonly_full_fake_acquisition_inclusive_boundary(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            template = make_template(root)
            before = {p.name:p.read_bytes() for p in template.iterdir()}
            out = root/'snapshot_1'
            with patch.object(export, 'load_sdk', return_value=sdk()), contextlib.redirect_stdout(io.StringIO()):
                status = export.main(['--template-dir', str(template), '--output-dir', str(out),
                                      '--through-message-id', '5', '--utc-offset-minutes', '120',
                                      '--canonical-time-mode', 'publication-final'])
            self.assertEqual(status, 0)
            receipt = json.loads((out/'EXPORT_RECEIPT.json').read_text())
            self.assertEqual(receipt['records'], 2)
            self.assertTrue(receipt['available_history_eof_observed'])
            self.assertFalse(receipt['historical_byte_parity_verified'])
            self.assertFalse(receipt['eligible_for_backtest_replacement'])
            self.assertTrue(receipt['source_auth_session_bindings_unchanged'])
            self.assertEqual(json.loads((out/'canonical.private.json').read_text())['counts']['nonempty_messages'],2)
            client = FakeClient.instances[-1]
            history = [r for r in client.calls if isinstance(r, GetHistoryRequest)]
            self.assertEqual([(r.offset_id, r.max_id) for r in history], [(6,6),(2,6)])
            self.assertEqual(client.options['receive_updates'], False)
            self.assertTrue(client.disconnected)
            self.assertEqual(before, {p.name:p.read_bytes() for p in template.iterdir()})
            self.assertEqual(len(list((out/'desktop').iterdir())), 1)

    def test_latest_freezes_first_page_boundary(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); template=make_template(root)
            with patch.object(export, 'load_sdk', return_value=sdk()), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(export.main(['--template-dir',str(template),'--output-dir',str(root/'latest'),
                                               '--latest','--utc-offset-minutes','0']), 0)
            history=[r for r in FakeClient.instances[-1].calls if isinstance(r,GetHistoryRequest)]
            self.assertEqual([(r.offset_id,r.max_id) for r in history],[(0,0),(2,6)])

    def test_wrong_peer_out_of_range_duplicate_unavailable_rejected(self):
        cases=[([Message(4,100)],{},'wrong_peer'),([Message(6)],{},'range'),
               ([Message(4)],{4:{}},'duplicate'),([NS(id=4)],{},'unavailable')]
        for messages, rows, reason in cases:
            with self.subTest(reason=reason), self.assertRaisesRegex(export.ExportError,reason):
                export.ingest_page(NS(messages=messages),sdk(),99,5,6,rows)

    def test_output_existing_and_inside_template_refused_before_network(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); template=make_template(root); existing=root/'existing';existing.mkdir()
            for output in [existing,template/'new']:
                with patch.object(export,'load_sdk',side_effect=AssertionError('SDK must not load')), \
                     contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(export.main(['--template-dir',str(template),'--output-dir',str(output),
                                                  '--latest','--utc-offset-minutes','0']),2)
            self.assertEqual(list(existing.iterdir()),[])

    def test_safe_raw_never_persists_access_or_auth_keys(self):
        self.assertEqual(export.safe_raw({'access_hash':123,'child':[{'auth_key':'synthetic','message':'plain'}]}),
                         {'child':[{'message':'plain'}]})

    def test_output_budget_and_name_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            evidence=export.Evidence(Path(tmp)/'new')
            with self.assertRaises(export.ExportError): evidence.write('../escape',{})
            with patch.object(export,'MAX_OUTPUT_BYTES',3), self.assertRaises(export.ExportError):
                evidence.write('large.json',{'long':'data'})

    def test_new_snapshot_does_not_mutate_previous(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)
            a=export.Evidence(root/'first');a.write('raw.json',{'version':1})
            old=(a.root/'raw.json').read_bytes()
            b=export.Evidence(root/'second');b.write('raw.json',{'version':2})
            self.assertEqual((a.root/'raw.json').read_bytes(),old)
            with self.assertRaises(FileExistsError):export.Evidence(a.root)


if __name__ == '__main__': unittest.main()
