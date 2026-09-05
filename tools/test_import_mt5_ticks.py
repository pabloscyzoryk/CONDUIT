import json
from pathlib import Path
import tempfile
import unittest

import numpy as np
from import_mt5_ticks import RECORD, convert


class TickImportTests(unittest.TestCase):
    def test_one_sided_quotes_cross_chunks_and_keep_same_time_order(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            source, target = directory / 'input.tsv', directory / 'ticks.bin'
            source.write_text(
                '<DATE>\t<TIME>\t<BID>\t<ASK>\n'
                '2026.01.05\t10:00:00.000\t3000\t3001\n'
                '2026.01.05\t10:00:00.000\t3000.5\t\n'
                '2026.01.05\t10:00:00.001\t\t3002\n', encoding='utf-8')
            manifest = convert(source, target, chunk_size=2)
            records = np.fromfile(target, dtype=RECORD, offset=64)
            self.assertEqual(list(records['bid']), [3000, 3000.5, 3000.5])
            self.assertEqual(list(records['ask']), [3001, 3001, 3002])
            self.assertEqual(manifest['same_timestamp_pairs'], 1)
            self.assertEqual(manifest['records'], 3)
            self.assertEqual(manifest['per_day_ticks'], {'2026-01-05': 3})

    def test_time_regression_is_rejected_across_chunk_boundary(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            source, target = directory / 'input.tsv', directory / 'ticks.bin'
            source.write_text(
                '<DATE>\t<TIME>\t<BID>\t<ASK>\n'
                '2026.01.05\t10:00:00.002\t3000\t3001\n'
                '2026.01.05\t10:00:00.001\t3000\t3001\n', encoding='utf-8')
            with self.assertRaisesRegex(ValueError, 'time regression'):
                convert(source, target, chunk_size=1)
            self.assertFalse(target.exists())


if __name__ == '__main__':
    unittest.main()
