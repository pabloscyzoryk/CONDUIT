#!/usr/bin/env python3
"""Build deterministic pseudo-live Telegram streams from the full Synergy export.

The generated files are deliberately marked NON_HISTORICAL.  They exercise the
raw-message replay contract used by the Rust runner: arrival timestamp,
``msg_id``, direct ``reply_to``, ``edit_of`` and ``receive_seq``.  They must not
be used as evidence of historical profit or for preset ranking.

Six profiles are available:

* optimistic -- small delivery jitter and rare harmless redelivery;
* realistic -- the latency/duplicate/edit races observed in live chronicles;
* pessimistic -- stronger reordering, typo/correction and broken-parent races;
* ultra -- adversarial bursts plus deliberately ambiguous/bad feed events.
* edit_recovery -- same-instant first EDIT, duplicate revisions and later NEW;
* reconnect -- unchanged source content redelivered after listener-memory loss.

No artificial delay, typo, reordering or mutation is ever applied to a message
that contains a RISK FREE / break-even instruction. This retains the original
benchmark's controlled disturbance model; it is not a claim that such delays
are impossible in production. Use case tests for protective-command failures.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass, asdict
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile
from typing import Any, Iterable, Sequence


RISK_FREE = re.compile(
    r"\bRISK\s+F+R*E+\b|\bBREAK\s*-?\s*EVEN\b|\bSET\s+(?:THE\s+)?SL\s+TO\s+BE\b|\bSL\s+IS\s+SET\s+TO\s+BE\b",
    re.I,
)
ENTRY = re.compile(
    r"(?is)\b(?:BUY|SELL)\s+(?:LIMITS?|STOPS?|MARKET\s+)?(?:GOLD|XAU\s*/?\s*USD)\b[^\n]{0,100}@"
)
TP_HIT = re.compile(r"\bTP\s*([1-9])\s*(?:HIT|DONE|REACHED|✅|✔️?)\b", re.I)
MANAGEMENT = re.compile(
    r"\b(?:TP\s*\d?\s*HIT|AT\s+TP\s*\d?|SL\s*HIT|CANCEL|INVALID|OUT\s+AT|RISK\s+FREE|BREAK\s*-?\s*EVEN|SECURING\s+PARTIAL)\b",
    re.I,
)


@dataclass(frozen=True)
class Profile:
    name: str
    delays_ms: tuple[int, ...]
    duplicate_mod: int
    noop_edit_mod: int
    typo_mod: int
    orphan_edit_mod: int
    restart_redelivery_mod: int
    broken_parent_mod: int
    at_tp_mod: int
    wrong_parent_tp_mod: int
    plausible_wrong_tp_mod: int
    burst_mod: int
    correction_delay_ms: int


PROFILES = (
    Profile("optimistic", (0, 25, 50, 100, 180, 350), 503, 0, 0, 0, 0, 0, 0, 0, 0, 0, 100),
    # Live sample 02.09: p50 2.83 s, p90 6.39 s, p99 16.59 s, max 37.72 s.
    Profile("realistic", (180, 650, 1_500, 2_830, 4_500, 6_400, 16_600), 37, 17, 59, 0, 131, 83, 47, 0, 0, 149, 900),
    Profile("pessimistic", (500, 2_000, 5_000, 9_000, 16_000, 28_000, 45_000), 13, 11, 23, 71, 67, 41, 29, 53, 0, 47, 2_500),
    Profile("ultra", (0, 2_000, 8_000, 20_000, 45_000, 75_000, 120_000), 7, 5, 13, 31, 29, 19, 17, 23, 61, 23, 5_000),
    Profile("edit_recovery", (0,), 1, 1, 0, 1, 0, 0, 0, 0, 0, 0, 50),
    Profile("reconnect", (0,), 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 50),
)


def stable_u64(profile: str, msg_id: int, source_index: int, salt: str) -> int:
    raw = f"conduit-live-backtest-v1|{profile}|{msg_id}|{source_index}|{salt}".encode()
    return int.from_bytes(hashlib.sha256(raw).digest()[:8], "big")


def chosen(profile: Profile, message: dict[str, Any], modulus: int, salt: str) -> bool:
    if modulus <= 0:
        return False
    return stable_u64(
        profile.name,
        int(message.get("msg_id", 0)),
        int((message.get("provenance") or {}).get("source_index", 0)),
        salt,
    ) % modulus == 0


def latency(profile: Profile, message: dict[str, Any]) -> int:
    text = str(message.get("text") or "")
    if RISK_FREE.search(text):
        return 0
    index = stable_u64(
        profile.name,
        int(message.get("msg_id", 0)),
        int((message.get("provenance") or {}).get("source_index", 0)),
        "latency",
    ) % len(profile.delays_ms)
    return profile.delays_ms[index]


def typo(text: str) -> tuple[str, str] | None:
    """Return (mutated, family); every mutation has a later exact correction."""
    replacements = (
        (r"\bLIMITS\b", "LIMlTS", "entry_keyword"),
        (r"\bGOLD\b", "G0LD", "instrument"),
        (r"\bTP([1-9])\s+HIT\b", r"TP\1 H1T", "tp_hit"),
        (r"\bCANCEL\b", "CANCEI", "cancel"),
        (r"\bINVALID\b", "INVALlD", "invalid"),
        (r"\bOUT\s+AT\s+ENTRY\b", "0UT AT ENTRY", "out_at_entry"),
    )
    for pattern, replacement, family in replacements:
        changed, count = re.subn(pattern, replacement, text, count=1, flags=re.I)
        if count:
            return changed, family
    return None


def at_tp_version(text: str) -> str | None:
    match = TP_HIT.search(text)
    if not match:
        return None
    return text[: match.start()] + f"AT TP{match.group(1)}" + text[match.end() :]


def write_atomic(path: Path, payload: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    encoded = json.dumps(payload, ensure_ascii=False, indent=2, allow_nan=False) + "\n"
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", newline="\n", dir=path.parent, delete=False) as handle:
        handle.write(encoded)
        temporary = Path(handle.name)
    try:
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def emitted(
    base: dict[str, Any],
    *,
    ts: int,
    text: str | None = None,
    msg_id: int | None = None,
    reply_to: int | None | object = ...,
    edit_of: int | None | object = ...,
    scenario: str,
    phase: int,
    original_order: int,
) -> dict[str, Any]:
    provenance = dict(base.get("provenance") or {})
    provenance.update(
        {
            "synthetic": True,
            "synthetic_scenario": scenario,
            "base_ts": int(base["ts"]),
            "base_msg_id": int(base["msg_id"]),
        }
    )
    row = {
        "ts": max(0, int(ts)),
        # The production Telegram listener receives these as two independent
        # facts: Message.date and the local UTC receipt timestamp.  Keeping
        # both lets the replay execute the exact live stale-entry gate.
        "telegram_published_ts": int(base.get("telegram_published_ts") or base["ts"]),
        "msg_id": int(base["msg_id"] if msg_id is None else msg_id),
        "reply_to": base.get("reply_to") if reply_to is ... else reply_to,
        "edit_of": base.get("edit_of") if edit_of is ... else edit_of,
        "text": str(base.get("text") or "") if text is None else text,
        "kanal": str(base.get("kanal") or "Synergy"),
        "latency_ms": max(0, int(ts) - int(base["ts"])),
        "provenance": provenance,
        "_sort": (max(0, int(ts)), phase, original_order),
    }
    return row


def build_profile(base_messages: list[dict[str, Any]], profile: Profile) -> tuple[dict[str, Any], dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    counts: dict[str, int] = {"base": len(base_messages)}
    next_id = max((int(m.get("msg_id", 0)) for m in base_messages), default=0) + 10_000_000
    recent_entries: list[int] = []

    def add(row: dict[str, Any]) -> None:
        scenario = row["provenance"]["synthetic_scenario"]
        counts[scenario] = counts.get(scenario, 0) + 1
        rows.append(row)

    for order, base0 in enumerate(base_messages):
        base = dict(base0)
        # A pseudo-live export begins with a NEW delivery.  The source export
        # contains only final text, so treating it as an EDIT would fabricate
        # an orphan and systematically remove legitimate entries.
        base["edit_of"] = None
        text = str(base.get("text") or "")
        is_rf = bool(RISK_FREE.search(text))
        is_entry = bool(ENTRY.search(text))
        if is_entry:
            recent_entries.append(int(base["msg_id"]))
            recent_entries = recent_entries[-64:]
        delay = latency(profile, base)
        arrival = int(base["ts"]) + delay

        mutation = None if is_rf or not chosen(profile, base, profile.typo_mod, "typo") else typo(text)
        broken_parent = (
            not is_rf
            and base.get("reply_to") is not None
            and MANAGEMENT.search(text)
            and chosen(profile, base, profile.broken_parent_mod, "broken-parent")
        )
        orphan = (
            not is_rf
            and chosen(profile, base, profile.orphan_edit_mod, "orphan")
        )
        at_text = at_tp_version(text)
        at_selected = bool(
            not is_rf
            and mutation is None
            and not broken_parent
            and not orphan
            and at_text
            and base.get("reply_to") is not None
            and chosen(profile, base, profile.at_tp_mod, "at-tp")
        )

        # Edit-before-new race. The selected preset decides whether a complete
        # first EDIT may create exposure; any later NEW must not duplicate it.
        # It is intentionally not used for RF.
        if orphan:
            add(
                emitted(
                    base,
                    ts=max(int(base["ts"]), arrival - 120),
                    edit_of=int(base["msg_id"]),
                    scenario="edit_before_new",
                    phase=5,
                    original_order=order,
                )
            )

        # Real Synergy pattern: one Telegram message starts as AT TP and is
        # edited in place to TP HIT.  It is the SAME msg_id, not a second NEW.
        if at_selected:
            add(
                emitted(
                    base,
                    ts=max(int(base["ts"]), arrival - 250),
                    text=at_text,
                    msg_id=int(base["msg_id"]),
                    reply_to=base.get("reply_to"),
                    edit_of=None,
                    scenario="at_tp_before_hit",
                    phase=7,
                    original_order=order,
                )
            )

        first_text = mutation[0] if mutation else text
        first_parent = 9_000_000_000 + order if broken_parent else base.get("reply_to")
        add(
            emitted(
                base,
                ts=arrival,
                text=first_text,
                reply_to=first_parent,
                edit_of=int(base["msg_id"]) if at_selected else None,
                scenario=(
                    "risk_free_zero_delay"
                    if is_rf
                    else "tp_hit_edit_after_at_tp"
                    if at_selected
                    else "new_delivery"
                ),
                phase=10,
                original_order=order,
            )
        )

        correction_needed = mutation is not None or broken_parent
        if correction_needed:
            correction_ts = arrival + profile.correction_delay_ms
            add(
                emitted(
                    base,
                    ts=correction_ts,
                    text=text,
                    reply_to=base.get("reply_to"),
                    edit_of=int(base["msg_id"]),
                    scenario=("typo_corrected_" + mutation[1]) if mutation else "broken_parent_corrected",
                    phase=20,
                    original_order=order,
                )
            )

        if not is_rf and chosen(profile, base, profile.noop_edit_mod, "noop-edit"):
            add(
                emitted(
                    base,
                    ts=arrival + profile.correction_delay_ms + 50,
                    text=text,
                    edit_of=int(base["msg_id"]),
                    scenario="same_content_edit",
                    phase=30,
                    original_order=order,
                )
            )

        if not is_rf and chosen(profile, base, profile.duplicate_mod, "duplicate"):
            add(
                emitted(
                    base,
                    ts=arrival + 75,
                    text=first_text,
                    reply_to=first_parent,
                    edit_of=int(base["msg_id"]),
                    scenario="duplicate_delivery",
                    phase=15,
                    original_order=order,
                )
            )

        if not is_rf and chosen(profile, base, profile.restart_redelivery_mod, "restart"):
            marker = dict(base)
            marker["msg_id"] = -(next_id + order + 1)
            marker["kanal"] = "__CONDUIT_CONTROL__"
            add(
                emitted(
                    marker,
                    ts=arrival + 29_999,
                    text="__CONDUIT_LIVEBACKTEST_INGRESS_RESTART__",
                    msg_id=int(marker["msg_id"]),
                    reply_to=None,
                    edit_of=None,
                    scenario="ingress_restart_marker",
                    phase=39,
                    original_order=order,
                )
            )
            add(
                emitted(
                    base,
                    ts=arrival + 30_000,
                    text=text,
                    edit_of=None,
                    scenario="restart_new_redelivery",
                    phase=40,
                    original_order=order,
                )
            )

        # A false TP with an impossible parent must fail closed.  This is the
        # pessimistic profile's 'wrong TP HIT' case and is fully decidable.
        if (
            not is_rf
            and recent_entries
            and chosen(profile, base, profile.wrong_parent_tp_mod, "wrong-parent-tp")
        ):
            next_id += 1
            synthetic = dict(base)
            synthetic["msg_id"] = next_id
            add(
                emitted(
                    synthetic,
                    ts=arrival + 33,
                    text="TP3 HIT +999 PIPS — CORRUPTED DELIVERY",
                    msg_id=next_id,
                    reply_to=9_999_999_999,
                    edit_of=None,
                    scenario="wrong_tp_hit_foreign_parent",
                    phase=16,
                    original_order=order,
                )
            )

        # Ultra-only: a syntactically plausible, correctly threaded but false
        # publisher event.  No transport/dedup algorithm can identify it; only
        # price confirmation can.  It is intentionally expected to expose a
        # limitation when tp_source permits signal-only advancement.
        if (
            not is_rf
            and recent_entries
            and chosen(profile, base, profile.plausible_wrong_tp_mod, "plausible-wrong-tp")
        ):
            next_id += 1
            synthetic = dict(base)
            synthetic["msg_id"] = next_id
            add(
                emitted(
                    synthetic,
                    ts=arrival + 40,
                    text="TP3 HIT +999 PIPS",
                    msg_id=next_id,
                    reply_to=recent_entries[-1],
                    edit_of=None,
                    scenario="plausible_wrong_tp_valid_parent",
                    phase=17,
                    original_order=order,
                )
            )

        # Burst of three exact deliveries at one timestamp: receive_seq is the
        # only ordering fact, matching reconnect backlog behavior.
        if (
            not is_rf
            and mutation is None
            and not broken_parent
            and not orphan
            and not at_selected
            and chosen(profile, base, profile.burst_mod, "burst")
        ):
            for part in range(3):
                add(
                    emitted(
                        base,
                        ts=arrival + 125,
                        text=text,
                        edit_of=int(base["msg_id"]) if part else None,
                        scenario="same_timestamp_burst",
                        phase=50 + part,
                        original_order=order,
                    )
                )

    rows.sort(key=lambda row: row.pop("_sort"))
    for sequence, row in enumerate(rows, 1):
        row["receive_seq"] = sequence

    rf_rows = [row for row in rows if RISK_FREE.search(str(row.get("text") or ""))]
    rf_delayed = [row for row in rf_rows if row["ts"] != row["provenance"]["base_ts"]]
    manifest = {
        "schema": "conduit.live-backtest-profile-manifest.v1",
        "profile": profile.name,
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "source_messages": len(base_messages),
        "deliveries": len(rows),
        "counts": counts,
        "risk_free_messages": len(rf_rows),
        "risk_free_artificially_delayed": len(rf_delayed),
        "risk_free_delay_contract": "PASS" if not rf_delayed else "FAIL",
        "configuration": asdict(profile),
        "scope": "synthetic resilience/P&L sensitivity; not historical evidence; forbidden for preset ranking",
        "source_causality": "LIMITED: full export often contains final_only text at publication time; case-level resilience only, never historical P&L evidence",
        "case_expectations": {
            "optimistic": "no crash; no duplicate exposure; near-baseline behavior",
            "realistic": "all transport/edit invariants pass; quantify execution sensitivity",
            "pessimistic": "100% decidable safety cases pass; quantify unavoidable latency damage",
            "ultra": "all decidable case invariants must pass; impossible-to-identify false publisher claims remain explicit limitations",
            "edit_recovery": "complete first EDIT accepted under the enabled policy; duplicate/later NEW must never multiply exposure",
            "reconnect": "listener memory loss cannot duplicate previously accepted source execution",
        },
    }
    if rf_delayed:
        raise AssertionError(f"{profile.name}: benchmark illegally delayed {len(rf_delayed)} RF deliveries")
    payload = {
        "schema": "conduit.raw-message-stream.v1",
        "historical": False,
        "dataset_kind": f"pseudo_live_{profile.name}_NON_HISTORICAL",
        "source": "full Synergy final-state export; synthetic transport/edit disturbance",
        "messages": rows,
        "live_backtest": manifest,
    }
    return payload, manifest


def read_base(path: Path) -> list[dict[str, Any]]:
    root = json.loads(path.read_text(encoding="utf-8-sig"))
    messages = root.get("messages")
    if not isinstance(messages, list) or not messages:
        raise ValueError(f"{path}: expected non-empty raw messages array")
    required = {"ts", "msg_id", "text"}
    seen: set[tuple[str, int]] = set()
    for index, row in enumerate(messages):
        missing = required.difference(row)
        if missing:
            raise ValueError(f"{path}: message {index} missing {sorted(missing)}")
        key = (str(row.get('kanal') or 'Synergy'), int(row['msg_id']))
        if key in seen:
            raise ValueError('Use a unique-message snapshot baseline; observed revision streams must not be flattened by this generator')
        seen.add(key)
    return messages


def generate(source: Path, output: Path, names: Iterable[str]) -> dict[str, Any]:
    if output.exists():
        raise FileExistsError(f"Benchmark output already exists: {output}")
    base = read_base(source)
    requested = set(names)
    selected = {profile.name: profile for profile in PROFILES if profile.name in requested}
    missing = sorted(requested.difference(selected))
    if missing:
        raise ValueError(f"unknown profiles: {missing}")
    summary: dict[str, Any] = {
        "schema": "conduit.live-backtest-profile-set.v1",
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "source": str(source.resolve()),
        "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "generator_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "profiles": {},
    }
    for name in (profile.name for profile in PROFILES if profile.name in selected):
        payload, manifest = build_profile(base, selected[name])
        manifest["source_sha256"] = summary["source_sha256"]
        manifest["generator_sha256"] = summary["generator_sha256"]
        stream_path = output / f"messages_SYN_livebacktest_{name}.json"
        manifest_path = output / f"messages_SYN_livebacktest_{name}.manifest.json"
        write_atomic(stream_path, payload)
        write_atomic(manifest_path, manifest)
        summary["profiles"][name] = {
            "stream": str(stream_path.resolve()),
            "manifest": str(manifest_path.resolve()),
            **manifest,
        }
        print(
            f"{name:12} deliveries={manifest['deliveries']:5} "
            f"RF_delayed={manifest['risk_free_artificially_delayed']} -> {stream_path}"
        )
    write_atomic(output / "LIVE_BACKTEST_PROFILE_SET.json", summary)
    return summary


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--source",
        type=Path,
        required=True,
    )
    parser.add_argument(
        "--out",
        type=Path,
        required=True,
    )
    parser.add_argument(
        "--profiles",
        nargs="+",
        default=[profile.name for profile in PROFILES],
    )
    args = parser.parse_args(argv)
    generate(args.source.resolve(), args.out.resolve(), args.profiles)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
