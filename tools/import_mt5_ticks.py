"""Convert MT5 tab-separated ticks to CDTK without changing row chronology.

One-sided quotes inherit only the preceding known quote. Equal timestamps keep
their original order. Input regressions and invalid/crossed quotes fail closed.
The manifest records data quality, per-day coverage and reproducibility hashes.
"""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import struct
import time

import numpy as np
import pandas as pd

RECORD = np.dtype([('ts', '<i8'), ('bid', '<f4'), ('ask', '<f4')])


def digest(path: Path) -> str:
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def convert(source: Path, output: Path, chunk_size: int = 1_000_000) -> dict:
    if source.resolve() == output.resolve() or output.exists():
        raise ValueError('Output must be a new file separate from the input')
    output.parent.mkdir(parents=True, exist_ok=True)
    partial = output.with_suffix(output.suffix + '.partial')
    if partial.exists():
        raise FileExistsError(partial)
    started = time.monotonic()
    previous_bid = previous_ask = np.nan
    previous_ts = None
    rows = written = skipped = equal_ts = filled_bid = filled_ask = 0
    first_ts = last_ts = None
    days: Counter[str] = Counter()
    minimum_spread = float('inf')
    maximum_spread = 0.0
    with partial.open('xb') as target, pd.read_csv(
        source, sep='\t', chunksize=chunk_size,
        usecols=['<DATE>', '<TIME>', '<BID>', '<ASK>'],
        dtype={'<DATE>': 'string', '<TIME>': 'string'},
    ) as chunks:
        target.write(bytes(64))
        for chunk in chunks:
            ts = pd.to_datetime(chunk['<DATE>'] + ' ' + chunk['<TIME>'],
                                format='%Y.%m.%d %H:%M:%S.%f', utc=True,
                                errors='raise').dt.tz_convert(None).astype('datetime64[ms]').astype('int64').to_numpy()
            if (np.diff(ts) < 0).any() or (previous_ts is not None and ts[0] < previous_ts):
                raise ValueError(f'Input time regression in chunk beginning at row {rows}')
            equal_ts += int((np.diff(ts) == 0).sum()) + int(previous_ts == ts[0])
            previous_ts = int(ts[-1])
            quotes = []
            for key, previous in [('<BID>', previous_bid), ('<ASK>', previous_ask)]:
                values = pd.to_numeric(chunk[key], errors='raise').to_numpy(dtype='float64', copy=True)
                bad = ~np.isfinite(values) | (values <= 0)
                if key == '<BID>':
                    filled_bid += int(bad.sum())
                else:
                    filled_ask += int(bad.sum())
                values[bad] = np.nan
                if np.isnan(values[0]):
                    values[0] = previous
                values = pd.Series(values).ffill().to_numpy()
                quotes.append(values)
            bid, ask = quotes
            previous_bid, previous_ask = float(bid[-1]), float(ask[-1])
            valid = np.isfinite(bid) & np.isfinite(ask)
            if written and not valid.all():
                raise ValueError('Missing quote after initialization')
            skipped += int((~valid).sum())
            bid, ask, ts = bid[valid], ask[valid], ts[valid]
            if (ask < bid).any():
                raise ValueError(f'Crossed bid/ask in chunk beginning at row {rows}')
            record = np.empty(len(ts), dtype=RECORD)
            record['ts'], record['bid'], record['ask'] = ts, bid, ask
            target.write(record.tobytes())
            if len(ts):
                first_ts = int(ts[0]) if first_ts is None else first_ts
                last_ts = int(ts[-1])
                minimum_spread = min(minimum_spread, float((ask-bid).min()))
                maximum_spread = max(maximum_spread, float((ask-bid).max()))
                for day, count in pd.Series(ts.astype('datetime64[ms]').astype('datetime64[D]')).value_counts().items():
                    days[str(day.date())] += int(count)
            rows += len(chunk)
            written += len(record)
            progress = {'stage': 'import_ticks', 'input_rows': rows,
                        'ticks_written': written, 'elapsed_s': round(time.monotonic()-started, 1)}
            output.with_suffix('.progress.json').write_text(json.dumps(progress), encoding='utf-8')
            print(json.dumps(progress), flush=True)
        if not written:
            raise ValueError('No complete quotes')
        header = bytearray(64)
        struct.pack_into('<I', header, 0, 0x4B544443)
        struct.pack_into('<Q', header, 8, written)
        target.seek(0)
        target.write(header)
        target.flush()
        os.fsync(target.fileno())
    os.replace(partial, output)
    manifest = {
        'schema': 'conduit.tick-import.v1',
        'source_sha256': digest(source), 'output_sha256': digest(output),
        'source_bytes': source.stat().st_size, 'output_bytes': output.stat().st_size,
        'source_rows': rows, 'records': written, 'initial_incomplete_rows_skipped': skipped,
        'bid_forward_fills': filled_bid, 'ask_forward_fills': filled_ask,
        'same_timestamp_pairs': equal_ts, 'timestamp_regressions': 0, 'crossed_quotes': 0,
        'first_timestamp_ms': first_ts, 'last_timestamp_ms': last_ts,
        'clock': 'broker wall clock from MT5 export; no UTC shift applied',
        'record_format': 'CDTK: 64 byte header, little endian i64 milliseconds/f32 bid/f32 ask',
        'same_timestamp_order': 'original physical input order preserved',
        'spread_min': minimum_spread, 'spread_max': maximum_spread,
        'per_day_ticks': dict(sorted(days.items())),
        'elapsed_s': round(time.monotonic()-started, 3),
    }
    output.with_suffix('.manifest.json').write_text(json.dumps(manifest, indent=2), encoding='utf-8')
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--chunk-size', type=int, default=1_000_000)
    args = parser.parse_args()
    print(json.dumps(convert(args.source, args.output, args.chunk_size), indent=2))


if __name__ == '__main__':
    main()
