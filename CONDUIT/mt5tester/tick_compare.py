"""Compare every native tester tick with an explicit CDTK time interval."""
from __future__ import annotations

import argparse
import csv
from datetime import datetime, timezone
import hashlib
import json
import mmap
import math
from pathlib import Path
import struct

RECORD = struct.Struct("<qff")


def date_ms(value: str) -> int:
    return int(datetime.strptime(value, "%Y-%m-%d").replace(tzinfo=timezone.utc).timestamp()) * 1000


def compare(csv_path: Path, tape_path: Path, start: int, end: int) -> dict:
    if start >= end:
        raise ValueError("The requested native server-clock interval must be nonempty.")
    native_hash, source_hash = hashlib.sha256(), hashlib.sha256()
    differences = []
    native_count = 0
    previous_ts = None
    with tape_path.open("rb") as file, mmap.mmap(file.fileno(), 0, access=mmap.ACCESS_READ) as tape:
        if tape[:4] != b"CDTK" or len(tape) < 64:
            raise ValueError("Invalid CDTK header.")
        count = struct.unpack_from("<Q", tape, 8)[0]
        if len(tape) != 64 + count * RECORD.size:
            raise ValueError("CDTK file size does not match its declared record count.")
        def lower_bound(ts):
            left, right = 0, count
            while left < right:
                mid = (left + right) // 2
                if struct.unpack_from("<q", tape, 64 + mid * RECORD.size)[0] < ts:
                    left = mid + 1
                else:
                    right = mid
            return left
        first, last = lower_bound(start), lower_bound(end)
        with csv_path.open("r", encoding="utf-8-sig", newline="") as stream:
            reader = csv.reader(stream, delimiter=";")
            if next(reader, None) != ["server_time_ms", "bid", "ask"]:
                raise ValueError("Unexpected native tick CSV header.")
            for row in reader:
                if len(row) != 3:
                    raise ValueError("Malformed native tick row.")
                ts, bid, ask = int(row[0]), float(row[1]), float(row[2])
                if not math.isfinite(bid) or not math.isfinite(ask) or bid <= 0 or ask < bid:
                    raise ValueError("Invalid native quote.")
                if ts < start or ts >= end or (previous_ts is not None and ts < previous_ts):
                    raise ValueError("Native ticks are outside the interval or out of order.")
                previous_ts = ts
                record = RECORD.pack(ts, bid, ask)
                native_hash.update(record)
                source_index = first + native_count
                expected = tape[64 + source_index * RECORD.size:64 + (source_index + 1) * RECORD.size] if source_index < last else b""
                if record != expected and len(differences) < 10:
                    differences.append({"ordinal": native_count, "native": [ts, bid, ask],
                                        "source": list(RECORD.unpack(expected)) if expected else None})
                native_count += 1
        # Hash the full expected slice, including any native missing tail.
        for offset in range(64 + first * RECORD.size, 64 + last * RECORD.size, 1024 * 1024):
            source_hash.update(tape[offset:min(offset + 1024 * 1024, 64 + last * RECORD.size)])
    exact = native_count > 0 and native_count == last - first and native_hash.digest() == source_hash.digest()
    return {"schema": "conduit.mt5.tick-equality.v1", "native_count": native_count,
            "source_count": last - first, "start_ms": start, "end_ms_exclusive": end,
            "native_sha256": native_hash.hexdigest(), "source_sha256": source_hash.hexdigest(),
            "bitwise_equal": exact, "first_differences": differences,
            "record_format": "little-endian i64 timestamp, f32 bid, f32 ask"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", type=Path, required=True)
    parser.add_argument("--ticks", type=Path, required=True)
    parser.add_argument("--from", dest="start", required=True)
    parser.add_argument("--to", dest="end", required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = compare(args.native, args.ticks, date_ms(args.start), date_ms(args.end))
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps({k:v for k,v in result.items() if k != "first_differences"}))
    raise SystemExit(0 if result["bitwise_equal"] else 2)


if __name__ == "__main__":
    main()
