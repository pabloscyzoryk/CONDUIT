import unittest
from merge_replay_corpus import merge


class CorpusMergeTests(unittest.TestCase):
    def test_observation_replaces_snapshot_and_keeps_empty_edit(self):
        export = {'messages': [
            {'ts': 300, 'msg_id': 1, 'text': 'final', 'kanal': 'Synergy'},
            {'ts': 400, 'msg_id': 2, 'text': 'other', 'kanal': 'Synergy'},
        ]}
        chronicle = {'messages': [
            {'ts': 100, 'msg_id': 1, 'text': 'first', 'kanal': 'Synergy'},
            {'ts': 200, 'msg_id': 1, 'text': '', 'kanal': 'Synergy', 'edit_of': 1},
        ]}
        result = merge(export, [chronicle])
        self.assertEqual([m['text'] for m in result['messages']], ['first', '', 'other'])
        self.assertEqual(result['counts']['export_records_replaced'], 1)
        self.assertFalse(result['provenance']['historical_coverage_complete'])
        self.assertNotIn('provenance', chronicle['messages'][0])

    def test_final_text_cannot_be_backdated(self):
        export = {'messages': [{'ts': 100, 'msg_id': 1, 'text': 'future',
                               'provenance': {'version': 'final_only', 'telegram_edited_ms': 200}}]}
        with self.assertRaisesRegex(ValueError, 'leaks'):
            merge(export, [])

    def test_overlapping_chronicles_are_not_silently_deduplicated(self):
        corpus = {'messages': [{'ts': 100, 'msg_id': 1, 'text': 'observed'}]}
        with self.assertRaisesRegex(ValueError, 'Overlapping'):
            merge({'messages': []}, [corpus, corpus])


if __name__ == '__main__':
    unittest.main()
