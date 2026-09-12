import json
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path
from verify_telegram_export_bytes import InvalidExport, inventory, main, parse_export, verify


class ExportBytesTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.a, self.b = self.root / 'a', self.root / 'b'
        self.a.mkdir()
        self.b.mkdir()
        self.doc = {'name': 'synthetic', 'type': 'public_channel', 'id': 1,
                    'messages': [{'id': 1, 'type': 'message', 'text': 'synthetic ą'}]}
        self.write(self.a, self.doc)
        self.write(self.b, self.doc)

    def write(self, folder, value, indent=2):
        (folder / 'result.json').write_text(json.dumps(value, ensure_ascii=False, indent=indent), encoding='utf-8')

    def test_equal_bytes_never_certifies_producer(self):
        got = verify(self.a, self.b)
        self.assertEqual(got['status'], 'PASS')
        self.assertFalse(got['generation_provenance_verified'])
        self.assertFalse(got['eligible_as_independently_generated_export'])

    def test_whitespace_difference_fails(self):
        self.write(self.b, self.doc, None)
        got = verify(self.a, self.b)
        self.assertTrue(got['exact_reference_message_range_and_order'])
        self.assertEqual(got['status'], 'FAIL')

    def test_extra_new_messages_not_silently_sliced(self):
        value = json.loads(json.dumps(self.doc))
        value['messages'].append({'id': 2, 'type': 'message', 'text': 'synthetic'})
        self.write(self.b, value)
        got = verify(self.a, self.b)
        self.assertFalse(got['exact_reference_message_range_and_order'])
        self.assertEqual(got['candidate_only_message_count'], 1)

    def test_reaction_change_fails_with_equal_text(self):
        self.doc['messages'][0]['reactions'] = [{'type': 'emoji', 'count': 1}]
        self.write(self.a, self.doc)
        self.doc['messages'][0]['reactions'][0]['count'] = 2
        self.write(self.b, self.doc)
        self.assertEqual(verify(self.a, self.b)['status'], 'FAIL')

    def test_missing_or_changed_media_fails(self):
        (self.a / 'photos').mkdir()
        (self.a / 'photos/example.jpg').write_bytes(b'synthetic media')
        self.assertFalse(verify(self.a, self.b)['relative_file_set_equal'])
        (self.b / 'photos').mkdir()
        (self.b / 'photos/example.jpg').write_bytes(b'different media')
        self.assertEqual(verify(self.a, self.b)['different_file_bytes_count'], 1)

    def test_same_input_and_nested_input_rejected(self):
        with self.assertRaises(InvalidExport): verify(self.a, self.a)
        nested = self.a / 'nested'
        nested.mkdir()
        with self.assertRaises(InvalidExport): verify(self.a, nested)

    def test_duplicate_keys_ids_bool_and_reorder_rejected(self):
        p = self.b / 'result.json'
        p.write_text('{"name":"synthetic","type":"public_channel","id":1,"id":2,"messages":[]}', encoding='utf-8')
        with self.assertRaises(InvalidExport): parse_export(p)
        for ids in ([1, 1], [2, 1], [True]):
            self.doc['messages'] = [{'id': x} for x in ids]
            self.write(self.b, self.doc)
            with self.assertRaises(InvalidExport): parse_export(p)

    def test_wrong_channel_identity_fails(self):
        self.doc['id'] = 2
        self.write(self.b, self.doc)
        self.assertFalse(verify(self.a, self.b)['same_channel_identity'])

    def test_directory_error_cannot_qualify_as_empty(self):
        def denied(*args, **kwargs):
            kwargs['onerror'](PermissionError('synthetic'))
            return iter(())
        with patch('verify_telegram_export_bytes.os.walk', denied):
            with self.assertRaisesRegex(InvalidExport, 'directory_read_failed'):
                inventory(self.a)

    def test_directory_entries_and_json_are_bounded(self):
        (self.a / 'one').mkdir()
        with patch('verify_telegram_export_bytes.MAX_FILES', 1):
            with self.assertRaisesRegex(InvalidExport, 'entry_count_limit'):
                inventory(self.a)
        with patch('verify_telegram_export_bytes.MAX_JSON', 2):
            with self.assertRaisesRegex(InvalidExport, 'json_size_limit'):
                parse_export(self.a / 'result.json')

    def test_cli_refuses_report_in_inputs_and_overwrite(self):
        self.assertEqual(main(['--reference', str(self.a), '--candidate', str(self.b), '--report', str(self.a/'report.json')]), 2)
        out = self.root / 'report.json'
        self.assertEqual(main(['--reference', str(self.a), '--candidate', str(self.b), '--report', str(out)]), 0)
        old = out.read_bytes()
        self.assertEqual(main(['--reference', str(self.a), '--candidate', str(self.b), '--report', str(out)]), 2)
        self.assertEqual(out.read_bytes(), old)


if __name__ == '__main__': unittest.main()
