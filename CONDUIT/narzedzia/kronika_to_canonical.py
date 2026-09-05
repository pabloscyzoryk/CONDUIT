#!/usr/bin/env python3
"""Buduje przyczynowy, płaski replay z Kroniki Conduita.

Generator nie uruchamia parsera sygnałów i nie próbuje wskazywać koszyka.
Każda niepusta wersja wiadomości przechodzi 1:1, łącznie z Info i
identycznymi ponownymi dostawami. Edycje i skasowania przechodzą również
z pustym tekstem: pusta edycja może wycofać oczekujący sygnał. Bezpośrednie ``reply_to`` i ``edit_of`` są kopiowane wyłącznie wtedy,
gdy źródło naprawdę je zawiera.

Zegar replay to lokalna chwila odbioru ``odebrano_ms``. Telegramowy znacznik
wersji pozostaje w ``provenance.telegram_event_ts_ms`` i nigdy nie steruje
kolejnością. ``receive_seq`` jest globalnym, stabilnym numerem rekordu
Kroniki w kolejności plik/linia; surowy ``seq`` (resetowany po restarcie)
pozostaje osobno jako ``seq_in_session``.

Wejściem może być surowe ``kronika.jsonl`` albo scalony ``alllogs*.txt``, w
którym rekord JSON występuje po znaczniku ``[KRONIKA]``.

Przykład:
  python narzedzia/kronika_to_canonical.py --source local/kronika.jsonl \
    --output local/messages.json --chat-id -1001234567890 --kanal Synergy

"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from dataclasses import dataclass
from datetime import date, datetime, timedelta, timezone
import hashlib
import json
import os
from pathlib import Path
import tempfile
from typing import Any, Iterable, Iterator, Sequence


SCHEMA = "conduit.raw-message-stream.v1"
MANIFEST_SCHEMA = "conduit.canonical-replay-manifest.v1"
GENERATOR_VERSION = 2
KRONIKA_MARKER = "[KRONIKA]"

EVENT_NAMES = {
    "nowa": "new",
    "received": "new",
    "new": "new",
    "edycja": "edit",
    "edited": "edit",
    "edit": "edit",
    "skasowana": "delete",
    "deleted": "delete",
    "delete": "delete",
    "start": "start",
    "stop": "stop",
}


@dataclass(frozen=True)
class SourceRef:
    file_index: int
    path: Path
    line: int
    stream_ordinal: int
    session_index: int


@dataclass(frozen=True)
class ChronicleRecord:
    raw: dict[str, Any]
    ref: SourceRef
    event: str
    received_at_ms: int


@dataclass
class ScanStats:
    source_lines: int = 0
    chronicle_records: int = 0
    malformed_records: int = 0
    session_markers: int = 0
    stop_markers: int = 0


def integer(value: Any, field: str, where: str, *, default: int | None = None) -> int:
    if value in (None, "") and default is not None:
        return default
    if isinstance(value, bool):
        raise ValueError(f"{where}.{field}: bool nie jest liczbą")
    try:
        return int(value)
    except (TypeError, ValueError) as exc:
        raise ValueError(f"{where}.{field}: wymagana poprawna liczba") from exc


def optional_integer(value: Any, field: str, where: str) -> int | None:
    if value in (None, ""):
        return None
    return integer(value, field, where)


def unix_ms(value: Any, field: str, where: str, *, default: int | None = None) -> int:
    raw = integer(value, field, where, default=default)
    return raw * 1000 if 0 < abs(raw) < 1_000_000_000_000 else raw


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def telegram_text(value: Any) -> str:
    """Spłaszcza finalny tekst Telegram Desktop wyłącznie do porównań."""

    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return "".join(telegram_text(part) for part in value)
    if isinstance(value, dict):
        return telegram_text(value.get("text", ""))
    return ""


def _json_from_line(line: str) -> str | None:
    marker_at = line.find(KRONIKA_MARKER)
    if marker_at >= 0:
        payload = line[marker_at + len(KRONIKA_MARKER) :].strip()
        # allLogs v2 dopisuje przed rekordem odwracalna proweniencje pliku,
        # np. ``[PROV kronika.jsonl:123] {..}``.  To nie jest czesc JSON i
        # starszy importer probowal parsowac nawias jako pierwszy znak,
        # przez co caly poprawny allLogs byl nieuzywalny do replayu.  Szukamy
        # poczatku obiektu dopiero PO jednoznacznym markerze KRONIKA; brak
        # obiektu nadal zostanie potraktowany jako uszkodzony rekord, a nie
        # cicho pominiety.
        object_at = payload.find("{")
        if object_at >= 0:
            payload = payload[object_at:]
        return payload or None
    stripped = line.strip()
    if stripped.startswith("{") and stripped.endswith("}"):
        return stripped
    return None


def _event_of(raw: dict[str, Any], where: str) -> str:
    value = raw.get("rodzaj", raw.get("event"))
    if not isinstance(value, str):
        raise ValueError(f"{where}: brak `rodzaj`/`event`")
    normalized = EVENT_NAMES.get(value.casefold())
    if normalized is None:
        raise ValueError(f"{where}: nieznany rodzaj Kroniki {value!r}")
    return normalized


def scan_sources(sources: Sequence[Path]) -> tuple[list[ChronicleRecord], ScanStats]:
    """Czyta źródła bez deduplikacji i nadaje globalny porządek plik/linia."""

    records: list[ChronicleRecord] = []
    stats = ScanStats()
    stream_ordinal = 0
    global_session = 0

    for file_index, source in enumerate(sources):
        local_session = 0
        with source.open("r", encoding="utf-8-sig", errors="strict") as handle:
            for line_number, line in enumerate(handle, 1):
                stats.source_lines += 1
                payload = _json_from_line(line)
                if payload is None:
                    continue
                try:
                    raw = json.loads(payload)
                except json.JSONDecodeError as exc:
                    # Zwykłe linie JSON innej sekcji allLogs nie są Kroniką.
                    # Błąd po jawnym markerze jest natomiast utratą danych.
                    if KRONIKA_MARKER not in line:
                        continue
                    stats.malformed_records += 1
                    raise ValueError(
                        f"{source}:{line_number}: uszkodzony rekord [KRONIKA]: {exc}"
                    ) from exc
                if not isinstance(raw, dict):
                    continue
                try:
                    event = _event_of(raw, f"{source}:{line_number}")
                except ValueError:
                    if KRONIKA_MARKER not in line:
                        continue
                    raise

                stats.chronicle_records += 1
                stream_ordinal += 1
                if event == "start":
                    global_session += 1
                    local_session += 1
                    stats.session_markers += 1
                elif event == "stop":
                    stats.stop_markers += 1

                received = unix_ms(
                    raw.get("odebrano_ms", raw.get("received_at_ms")),
                    "odebrano_ms",
                    f"{source}:{line_number}",
                    default=0,
                )
                records.append(
                    ChronicleRecord(
                        raw=raw,
                        ref=SourceRef(
                            file_index=file_index,
                            path=source,
                            line=line_number,
                            stream_ordinal=stream_ordinal,
                            # Globalny indeks rozróżnia restarty także między
                            # plikami. local_session zostaje w provenance.
                            session_index=global_session,
                        ),
                        event=event,
                        received_at_ms=received,
                    )
                )

        # Surowy JSONL może nie mieć markerów start. Kolejny plik musi mimo to
        # dostać inną przestrzeń sesji, żeby seq=1 nie wyglądało na tę samą.
        if local_session == 0:
            global_session += 1

    return records, stats


def local_day(timestamp_ms: int, utc_offset_minutes: int) -> date:
    tz = timezone(timedelta(minutes=utc_offset_minutes))
    return datetime.fromtimestamp(timestamp_ms / 1000, tz=tz).date()


def _source_name(raw: dict[str, Any]) -> str:
    value = raw.get("chat", raw.get("source_name", ""))
    return value if isinstance(value, str) else str(value or "")


def _telegram_ts(raw: dict[str, Any], where: str) -> int | None:
    value = raw.get("ts_telegram_ms", raw.get("msg_ts_ms"))
    if value in (None, "", 0, "0"):
        value = raw.get("ts_telegram")
    if value in (None, "", 0, "0"):
        return None
    return unix_ms(value, "ts_telegram_ms", where)


def build_messages(
    records: Sequence[ChronicleRecord],
    *,
    chat_id: int | None,
    channel: str | None,
    selected_day: date | None,
    utc_offset_minutes: int,
) -> tuple[list[dict[str, Any]], dict[str, Any], list[ChronicleRecord]]:
    """Buduje flat stream. Zwraca też liczniki i rekordy przed filtrem tekstu."""

    chat_records: list[ChronicleRecord] = []
    for record in records:
        if record.event not in {"new", "edit", "delete"}:
            continue
        raw_chat_id = integer(
            record.raw.get("chat_id"),
            "chat_id",
            f"{record.ref.path}:{record.ref.line}",
            default=0,
        )
        if chat_id is not None and raw_chat_id != chat_id:
            continue
        chat_records.append(record)

    # Metadane historii liczymy na pełnym strumieniu wybranego czatu, zanim
    # zawęzimy go do dnia. Dzięki temu dzienny plik nie udaje, że edycja z
    # północy jest pierwszą wersją wiadomości, której `new` widzieliśmy wczoraj.
    full_ordered = sorted(chat_records, key=lambda r: (r.received_at_ms, r.ref.stream_ordinal))
    full_version_index: Counter[tuple[int, int | None, int]] = Counter()
    full_last_hash: dict[tuple[int, int | None, int], str] = {}
    full_seen_new: set[tuple[int, int | None, int]] = set()
    history_metadata: dict[int, tuple[int, bool, bool]] = {}
    for record in full_ordered:
        raw = record.raw
        where = f"{record.ref.path}:{record.ref.line}"
        raw_chat_id = integer(raw.get("chat_id"), "chat_id", where, default=0)
        topic_id = optional_integer(raw.get("temat", raw.get("topic_id")), "temat", where)
        msg_id = integer(raw.get("msg_id"), "msg_id", where)
        identity = (raw_chat_id, topic_id, msg_id)
        text_value = raw.get("text", "")
        text = text_value if isinstance(text_value, str) else str(text_value or "")
        content_hash = sha256_text(text)
        full_version_index[identity] += 1
        repeated = full_last_hash.get(identity) == content_hash
        full_last_hash[identity] = content_hash
        if record.event == "new":
            full_seen_new.add(identity)
        history_metadata[record.ref.stream_ordinal] = (
            full_version_index[identity],
            repeated,
            identity in full_seen_new,
        )

    candidates = [
        record
        for record in chat_records
        if selected_day is None
        or local_day(record.received_at_ms, utc_offset_minutes) == selected_day
    ]

    ordered = sorted(candidates, key=lambda r: (r.received_at_ms, r.ref.stream_ordinal))
    candidate_identities: set[tuple[int, int | None, int]] = set()
    candidate_new_identities: set[tuple[int, int | None, int]] = set()
    candidate_text_states: set[tuple[int, int | None, int, str]] = set()
    candidate_hashes_by_identity: dict[tuple[int, int | None, int], set[str]] = defaultdict(set)
    candidate_kinds_by_identity: dict[tuple[int, int | None, int], set[str]] = defaultdict(set)
    candidate_last_hash: dict[tuple[int, int | None, int], str] = {}
    candidate_duplicate_content = 0
    candidate_event_counts: Counter[str] = Counter()
    for record in ordered:
        raw = record.raw
        where = f"{record.ref.path}:{record.ref.line}"
        raw_chat_id = integer(raw.get("chat_id"), "chat_id", where, default=0)
        topic_id = optional_integer(raw.get("temat", raw.get("topic_id")), "temat", where)
        msg_id = integer(raw.get("msg_id"), "msg_id", where)
        identity = (raw_chat_id, topic_id, msg_id)
        text_value = raw.get("text", "")
        text = text_value if isinstance(text_value, str) else str(text_value or "")
        content_hash = sha256_text(text)
        candidate_identities.add(identity)
        if record.event == "new":
            candidate_new_identities.add(identity)
        candidate_text_states.add((*identity, content_hash))
        candidate_hashes_by_identity[identity].add(content_hash)
        candidate_kinds_by_identity[identity].add(record.event)
        if candidate_last_hash.get(identity) == content_hash:
            candidate_duplicate_content += 1
        candidate_last_hash[identity] = content_hash
        candidate_event_counts[record.event] += 1

    messages: list[dict[str, Any]] = []
    skipped_empty = 0
    duplicate_content = 0
    event_counts: Counter[str] = Counter()

    # Chwila odbioru jest zegarem, zaś globalny ordinal plik/linia jest
    # stabilnym tie-breakiem. Nie używamy msg_id ani resetowanego seq.
    for record in ordered:
        raw = record.raw
        where = f"{record.ref.path}:{record.ref.line}"
        raw_chat_id = integer(raw.get("chat_id"), "chat_id", where, default=0)
        topic_id = optional_integer(raw.get("temat", raw.get("topic_id")), "temat", where)
        msg_id = integer(raw.get("msg_id"), "msg_id", where)
        identity = (raw_chat_id, topic_id, msg_id)
        text_value = raw.get("text", "")
        text = text_value if isinstance(text_value, str) else str(text_value or "")
        content_hash = sha256_text(text)
        current_version, repeated, history_seen_from_new = history_metadata[
            record.ref.stream_ordinal
        ]

        # Puste NEW z mediami nie zawiera polecenia. Puste EDIT musi pozostać:
        # live może nim wycofać geometrię oczekującego deferred ENTRY.
        if record.event == "new" and not text.strip():
            skipped_empty += 1
            continue
        if repeated:
            duplicate_content += 1

        telegram_ts = _telegram_ts(raw, where)
        seq_in_session = integer(raw.get("seq"), "seq", where, default=0)
        reply_to = optional_integer(
            raw.get("reply_to", raw.get("reply_to_message_id")), "reply_to", where
        )
        edit_of = optional_integer(raw.get("edit_of"), "edit_of", where)
        source_name = _source_name(raw)
        route_channel = channel if channel is not None else source_name
        latency = (
            record.received_at_ms - telegram_ts
            if telegram_ts is not None and record.received_at_ms >= telegram_ts
            else None
        )
        event_counts[record.event] += 1

        messages.append(
            {
                "ts": record.received_at_ms,
                "receive_seq": record.ref.stream_ordinal,
                "event": record.event,
                "msg_id": msg_id,
                "reply_to": reply_to,
                "edit_of": edit_of,
                "text": text,
                "kanal": route_channel,
                "chat_id": raw_chat_id,
                "topic_id": topic_id,
                "content_sha256": content_hash,
                "provenance": {
                    "source_kind": "conduit_chronicle_observed",
                    "source_file_index": record.ref.file_index,
                    "source_line": record.ref.line,
                    "capture_session_index": record.ref.session_index,
                    "seq_in_session": seq_in_session,
                    "telegram_event_ts_ms": telegram_ts,
                    "receive_latency_ms": latency,
                    "source_name": source_name,
                    "monitored": bool(raw.get("nasluchiwany", raw.get("monitored", False))),
                    "parser_observed": bool(raw.get("rozpoznane", False)),
                    "format_observed": raw.get("format"),
                    "observed_version_index": current_version,
                    "history_seen_from_new": history_seen_from_new,
                    "duplicate_of_previous_content": repeated,
                    "fidelity": "observed_exact",
                },
            }
        )

    unique_ids = {(m["chat_id"], m["topic_id"], m["msg_id"]) for m in messages}
    distinct_states = {
        (m["chat_id"], m["topic_id"], m["msg_id"], m["content_sha256"]) for m in messages
    }
    counts = {
        "candidate_message_events": len(candidates),
        "candidate_event_counts": dict(sorted(candidate_event_counts.items())),
        "unique_message_ids_before_text_filter": len(candidate_identities),
        "message_ids_with_new_before_text_filter": len(candidate_new_identities),
        "message_ids_without_new_before_text_filter": len(candidate_identities - candidate_new_identities),
        "edit_only_message_ids_before_text_filter": sum(
            kinds == {"edit"} for kinds in candidate_kinds_by_identity.values()
        ),
        "delete_only_message_ids_before_text_filter": sum(
            kinds == {"delete"} for kinds in candidate_kinds_by_identity.values()
        ),
        "distinct_text_states_before_text_filter": len(candidate_text_states),
        "message_ids_with_multiple_text_states_before_filter": sum(
            len(hashes) > 1 for hashes in candidate_hashes_by_identity.values()
        ),
        "consecutive_duplicate_content_deliveries_before_filter": candidate_duplicate_content,
        "repeated_content_state_deliveries_before_filter": len(candidates)
        - len(candidate_text_states),
        "messages_emitted": len(messages),
        "empty_non_delete_skipped": skipped_empty,
        "delete_events_included": event_counts["delete"],
        "event_counts": dict(sorted(event_counts.items())),
        "unique_message_ids_emitted": len(unique_ids),
        "empty_only_message_ids_skipped": len(candidate_identities - unique_ids),
        "distinct_text_states_emitted": len(distinct_states),
        "consecutive_duplicate_content_deliveries_emitted": duplicate_content,
        "repeated_content_state_deliveries_emitted": len(messages) - len(distinct_states),
        "direct_reply_records": sum(m["reply_to"] is not None for m in messages),
        "edit_of_records": sum(m["edit_of"] is not None for m in messages),
        "parser_observed_true": sum(m["provenance"]["parser_observed"] for m in messages),
        "parser_observed_false": sum(not m["provenance"]["parser_observed"] for m in messages),
    }
    return messages, counts, candidates


def _telegram_scope(
    export_path: Path,
    *,
    selected_day: date | None,
    utc_offset_minutes: int,
) -> dict[int, dict[str, Any]]:
    with export_path.open("r", encoding="utf-8-sig") as handle:
        root = json.load(handle)
    raw_messages = root.get("messages")
    if not isinstance(raw_messages, list):
        raise ValueError(f"{export_path}: brak top-level `messages`")

    scoped: dict[int, dict[str, Any]] = {}
    for source_index, raw in enumerate(raw_messages):
        if not isinstance(raw, dict) or raw.get("type") != "message":
            continue
        where = f"{export_path}:messages[{source_index}]"
        published_ms = unix_ms(raw.get("date_unixtime"), "date_unixtime", where)
        if selected_day is not None and local_day(published_ms, utc_offset_minutes) != selected_day:
            continue
        msg_id = integer(raw.get("id"), "id", where)
        edited_value = raw.get("edited_unixtime")
        edited_ms = (
            unix_ms(edited_value, "edited_unixtime", where)
            if edited_value not in (None, "", 0, "0")
            else None
        )
        scoped[msg_id] = {
            "text": telegram_text(raw.get("text", "")),
            "published_ms": published_ms,
            "final_edit_ms": edited_ms,
        }
    return scoped


def coverage_against_telegram(
    export_path: Path,
    candidates: Sequence[ChronicleRecord],
    *,
    selected_day: date | None,
    utc_offset_minutes: int,
) -> dict[str, Any]:
    raw = _telegram_scope(
        export_path,
        selected_day=selected_day,
        utc_offset_minutes=utc_offset_minutes,
    )
    chronicle: dict[int, list[ChronicleRecord]] = defaultdict(list)
    for record in sorted(candidates, key=lambda r: (r.received_at_ms, r.ref.stream_ordinal)):
        msg_id = integer(
            record.raw.get("msg_id"),
            "msg_id",
            f"{record.ref.path}:{record.ref.line}",
        )
        chronicle[msg_id].append(record)

    raw_ids = set(raw)
    chronicle_ids = set(chronicle)
    overlap = raw_ids & chronicle_ids
    chronicle_only = chronicle_ids - raw_ids
    raw_only = raw_ids - chronicle_ids
    text_matches: list[int] = []
    text_mismatches: list[int] = []
    final_timestamp_matches: list[int] = []
    final_timestamp_mismatches: list[int] = []

    for msg_id in sorted(overlap):
        last = chronicle[msg_id][-1]
        last_text_value = last.raw.get("text", "")
        last_text = last_text_value if isinstance(last_text_value, str) else str(last_text_value or "")
        if raw[msg_id]["text"] == last_text:
            text_matches.append(msg_id)
        else:
            text_mismatches.append(msg_id)
        expected_ts = raw[msg_id]["final_edit_ms"] or raw[msg_id]["published_ms"]
        actual_ts = _telegram_ts(last.raw, f"{last.ref.path}:{last.ref.line}")
        if actual_ts == expected_ts:
            final_timestamp_matches.append(msg_id)
        else:
            final_timestamp_mismatches.append(msg_id)

    def sample(values: Iterable[int], limit: int = 50) -> list[int]:
        return sorted(values)[:limit]

    return {
        "telegram_export_path": str(export_path.resolve()),
        "telegram_export_sha256": sha256_file(export_path),
        "telegram_message_ids_in_scope": len(raw_ids),
        "telegram_nonempty_in_scope": sum(bool(item["text"].strip()) for item in raw.values()),
        "chronicle_message_ids_before_text_filter": len(chronicle_ids),
        "overlap_ids": len(overlap),
        "chronicle_only_ids": len(chronicle_only),
        "raw_only_ids": len(raw_only),
        "raw_final_text_matches": len(text_matches),
        "raw_final_text_mismatches": len(text_mismatches),
        "raw_final_timestamp_observed": len(final_timestamp_matches),
        "raw_final_timestamp_not_observed": len(final_timestamp_mismatches),
        "text_mismatch_ids": text_mismatches,
        "final_timestamp_mismatch_ids": final_timestamp_mismatches,
        "chronicle_only_id_sample": sample(chronicle_only),
        "raw_only_id_sample": sample(raw_only),
        "comparison_note": (
            "Chronicle scope uses local receive day; Telegram scope uses publish day. "
            "For a full-window manifest the sets are directly comparable."
        ),
    }


def build_payload_and_manifest(
    sources: Sequence[Path],
    *,
    chat_id: int | None,
    channel: str | None,
    selected_day: date | None = None,
    utc_offset_minutes: int = 0,
    telegram_export: Path | None = None,
    historical: bool = True,
    dataset_kind: str = "historical_observed_chronicle",
) -> tuple[dict[str, Any], dict[str, Any]]:
    if not sources:
        raise ValueError("wymagane co najmniej jedno --source")
    records, scan = scan_sources(sources)
    messages, message_counts, candidates = build_messages(
        records,
        chat_id=chat_id,
        channel=channel,
        selected_day=selected_day,
        utc_offset_minutes=utc_offset_minutes,
    )

    input_info = [
        {
            "source_file_index": index,
            "path": str(path.resolve()),
            "sha256": sha256_file(path),
            "bytes": path.stat().st_size,
        }
        for index, path in enumerate(sources)
    ]
    selection = {
        "chat_id": chat_id,
        "kanal": channel,
        "day": selected_day.isoformat() if selected_day is not None else None,
        "day_clock": "received_at_ms",
        "utc_offset_minutes": utc_offset_minutes,
    }
    limitations = [
        "Only events actually present in Chronicle are historical observations; recorder downtime is not filled.",
        "Info is retained and parser_observed is audit metadata only, never a replay filter.",
        "No reply root or basket owner is pre-resolved; reply_to is the raw direct Telegram edge.",
        "Identical repeated deliveries are retained; no content or action deduplication is performed.",
        "Only empty NEW text is skipped; empty EDIT and DELETE are retained because they change an observed message.",
    ]
    payload = {
        "schema": SCHEMA,
        "historical": historical,
        "dataset_kind": dataset_kind,
        "provenance": {
            "generator": "kronika_to_canonical.py",
            "generator_version": GENERATOR_VERSION,
            "source_kind": "conduit_chronicle_observed",
            "clock": "local_conduit_received_at_ms",
            "ordering": "(ts, global source file/line ordinal); seq_in_session retained as provenance",
            "selection": selection,
            "inputs": input_info,
            "limitations": limitations,
        },
        "counts": {
            "source_lines": scan.source_lines,
            "chronicle_records_all_chats": scan.chronicle_records,
            "capture_session_markers": scan.session_markers,
            "capture_stop_markers": scan.stop_markers,
            **message_counts,
        },
        "messages": messages,
    }
    manifest: dict[str, Any] = {
        "schema": MANIFEST_SCHEMA,
        "generator": "kronika_to_canonical.py",
        "generator_version": GENERATOR_VERSION,
        "historical": historical,
        "dataset_kind": dataset_kind,
        "selection": selection,
        "inputs": input_info,
        "counts": payload["counts"],
        "fidelity": {
            "clock": "received_at_ms",
            "reply_semantics": "raw_direct_only",
            "deduplication": "none",
            "parser_filter": "none",
            "text_filter": "nonempty_new_plus_all_edit_delete",
            "limitations": limitations,
        },
    }
    if messages:
        manifest["range"] = {
            "first_received_at_ms": messages[0]["ts"],
            "last_received_at_ms": messages[-1]["ts"],
        }
    if telegram_export is not None:
        manifest["telegram_export_coverage"] = coverage_against_telegram(
            telegram_export,
            candidates,
            selected_day=selected_day,
            utc_offset_minutes=utc_offset_minutes,
        )
    return payload, manifest


def _json_bytes(payload: dict[str, Any]) -> bytes:
    return (json.dumps(payload, ensure_ascii=False, separators=(",", ":")) + "\n").encode("utf-8")


def write_outputs(
    output: Path,
    manifest_path: Path,
    payload: dict[str, Any],
    manifest: dict[str, Any],
    *,
    force: bool,
) -> None:
    output = output.resolve()
    manifest_path = manifest_path.resolve()
    if output == manifest_path:
        raise ValueError("output i manifest muszą być różnymi plikami")
    for path in (output, manifest_path):
        if path.exists() and not force:
            raise FileExistsError(f"{path} już istnieje; użyj --force")
        path.parent.mkdir(parents=True, exist_ok=True)

    output_data = _json_bytes(payload)
    manifest = dict(manifest)
    manifest["output"] = {
        "path": str(output),
        "sha256": hashlib.sha256(output_data).hexdigest(),
        "bytes": len(output_data),
        "messages": len(payload.get("messages", [])),
    }
    manifest_data = _json_bytes(manifest)

    temporary: list[tuple[Path, Path]] = []
    try:
        for target, data in ((output, output_data), (manifest_path, manifest_data)):
            with tempfile.NamedTemporaryFile("wb", dir=target.parent, delete=False, suffix=".tmp") as handle:
                handle.write(data)
                temp_path = Path(handle.name)
            temporary.append((temp_path, target))
        for temp_path, target in temporary:
            os.replace(temp_path, target)
    finally:
        for temp_path, _ in temporary:
            temp_path.unlink(missing_ok=True)


def parse_day(value: str) -> date:
    try:
        return date.fromisoformat(value)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("dzień musi mieć format YYYY-MM-DD") from exc


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", action="append", type=Path, required=True, help="Kronika JSONL lub allLogs")
    parser.add_argument("--output", type=Path, required=True, help="canonical raw messages JSON")
    parser.add_argument("--manifest", type=Path, help="manifest JSON; domyślnie OUTPUT.manifest.json")
    parser.add_argument("--chat-id", type=int, help="filtr Telegram chat_id")
    parser.add_argument("--kanal", help="stabilna nazwa routingu/presetu, np. Synergy")
    parser.add_argument("--day", type=parse_day, help="opcjonalny dzień receive clock YYYY-MM-DD")
    parser.add_argument(
        "--utc-offset-minutes",
        type=int,
        default=0,
        help="stały offset kalendarza day; Warszawa 28.08.2026 = 120",
    )
    parser.add_argument("--telegram-export", type=Path, help="opcjonalny result.json do manifestu pokrycia")
    parser.add_argument("--force", action="store_true", help="jawnie pozwól nadpisać oba wyniki")
    args = parser.parse_args(argv)

    sources = [path.resolve() for path in args.source]
    for source in sources:
        if not source.is_file():
            raise FileNotFoundError(source)
    output = args.output.resolve()
    manifest_path = (
        args.manifest.resolve()
        if args.manifest is not None
        else output.with_name(output.stem + ".manifest.json")
    )
    if output in sources or manifest_path in sources:
        raise ValueError("output/manifest nie może nadpisywać źródła")
    telegram_export = args.telegram_export.resolve() if args.telegram_export else None
    if telegram_export is not None and not telegram_export.is_file():
        raise FileNotFoundError(telegram_export)

    payload, manifest = build_payload_and_manifest(
        sources,
        chat_id=args.chat_id,
        channel=args.kanal,
        selected_day=args.day,
        utc_offset_minutes=args.utc_offset_minutes,
        telegram_export=telegram_export,
    )
    write_outputs(output, manifest_path, payload, manifest, force=args.force)
    print(f"zapisano canonical: {output}")
    print(f"zapisano manifest : {manifest_path}")
    print(f"wersji/dostaw      : {len(payload['messages'])}")
    print("zegar              : odebrano_ms (lokalna chwila widoczności dla bota)")
    print("routing            : raw direct reply_to; zero pre-resolve")
    print("dedup              : brak")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
