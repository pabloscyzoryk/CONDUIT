# Strict Telegram export verification

`tools/verify_telegram_export_bytes.py` compares a separately generated, bounded Telegram export with a Telegram Desktop reference. It runs offline with Python's standard library and does not read credentials or change either input.

```powershell
python -B tools/verify_telegram_export_bytes.py --reference "C:/exports/desktop-reference" --candidate "C:/exports/generated-overlap" --report "C:/exports/overlap-verification.json"
```

Both directories must contain original-format `result.json`. The candidate must cover exactly the reference message range. Generate that bounded candidate independently; do not copy the reference or repair its metadata into the candidate. A newer full export containing additional records fails this comparison. No messages, fields or whitespace are removed to make it pass.

PASS requires the same channel metadata, exact message IDs and order, the same relative file set, and matching bytes for every file. Thus a reference with photos also requires those exact media files. A JSON-only reference requires a separately generated JSON-only candidate. The tool rejects duplicate JSON keys or message IDs, nonfinite JSON values, reordered records, symlinks/reparse points, unreadable directories, oversized inputs, input changes observed during comparison, an existing report, and reports located inside either input directory.

Exit codes are `0` for byte equality, `3` for a completed comparison with differences, and `2` for invalid inputs or an operational error. Output contains aggregate counts and whole-result file hashes, without message text, channel IDs or per-message IDs.

**PASS proves byte equality, not origin.** A copied reference could pass a byte comparison. The receipt therefore always reports `generation_provenance_verified: false` and `eligible_as_independently_generated_export: false`. A caller must separately verify the real producer invocation, source/binary identity, exact allowed channel, pagination completion and generated output hashes before admitting a new export to research. If literal overlap equality is required, a failed comparison must block use of the candidate.

Telegram history exports describe a snapshot. Matching a Desktop snapshot does not establish the original text of every edit or the bot's observation times. Preserve observed NEW/EDIT delivery histories separately.

The existing `eksport` application is in `CONDUIT/rust/crates/eksporter`. Its `--help` and `--version` flags return before reading configuration, restoring a session, connecting to Telegram or opening the interface. The normal interface's current serializer omits some Desktop fields and flattens formatted text, so its output is not a byte-compatible replacement. Use the headless tool below for the separately tested Desktop-style JSON path. Even with a matching serializer, changed reaction counts or later edits can make a fresh snapshot differ from an older Desktop reference. Do not copy older values into the candidate to obtain a PASS.

## Headless Synergy export

`tools/export_telegram_synergy.py` uses an existing authorized CONDUIT session. Install its dependencies into a separate virtual environment; do not modify a deployed application's Python runtime:

```powershell
python -m venv .venv-telegram-export
.venv-telegram-export/Scripts/python.exe -m pip install -r tools/requirements-telegram-export.txt
.venv-telegram-export/Scripts/python.exe -B tools/export_telegram_synergy.py --help
```

The template directory must contain `secrets.json`, `telegram.session` and `channels.json`. The session pair must agree, and exactly one binding must have format `Synergy`. The exporter resolves only that peer using minimal dialog metadata, then requests only its history. A missing authorization or ambiguous binding stops the run; there is no login prompt. Neither credentials nor their hashes are printed or copied to the export. Output remains private because it contains message bodies and identifiers.

Run a bounded export by supplying the inclusive last message ID from the source being compared. Replace the illustrative ID below with that boundary:

```powershell
.venv-telegram-export/Scripts/python.exe -B tools/export_telegram_synergy.py --template-dir "C:/private/conduit-template" --output-dir "C:/private/exports/synergy-bounded-v1" --through-message-id 123456 --utc-offset-minutes 120 --canonical-time-mode publication-final
```

For a new current snapshot, use `--latest` instead of `--through-message-id`. The first history page freezes its final ID; later arrivals are excluded from that run. The displayed local-time offset is explicit, with no automatic profit-based inference. Unix publication and edit times are never shifted. A fixed offset does not represent a timezone's DST transitions; use the reference's actual export convention when testing byte equality.

The output directory must be new and its parent must exist. It contains:

- `desktop/result.json`: current API values in the supported Desktop JSON layout, with no media downloads.
- `page_*.private.json`, `network_history.private.json` and optional emoji metadata: the independently acquired evidence, with acquisition times and access/auth keys removed.
- `EXPORT_RECEIPT.json`: producer and formatter hashes, artifact hashes, pagination completion, scope counts, unchanged input checks and explicit limitations.
- Optional `canonical.private.json`: the selected final-text timing model for the historical loader. This sidecar stays outside the Desktop directory so it does not change the strict file-set comparison.

No history is fetched from other channels. The metadata lookup may incidentally return other dialogs' latest-message objects, which are neither exported nor printed. The tool does not send messages, join channels, mark history read, download media, write session files or open a browser. Custom-emoji document metadata is queried only for IDs referenced by the selected history; no emoji files are downloaded. Budgets limit requests, records, serialized bytes and elapsed time. An unsupported media/service/rich-text shape stops formatting explicitly; it does not produce a qualified partial export.

The formatter follows the public Telegram Desktop [JSON writer](https://github.com/telegramdesktop/tdesktop/blob/dev/Telegram/SourceFiles/export/output/export_output_json.cpp), [text parser](https://github.com/telegramdesktop/tdesktop/blob/dev/Telegram/SourceFiles/export/data/export_data_types.cpp) and [custom-emoji export policy](https://github.com/telegramdesktop/tdesktop/blob/dev/Telegram/SourceFiles/export/export_api_wrap.cpp). The supported path preserves UTF-16 entity offsets, structured text, replies, edits, service records, available media metadata, field order and JSON spacing. Compatibility with every Desktop version or export option is not assumed; run the strict comparison against the actual reference.

`publication-final` emits each current final text at its original publication time. This deliberately reproduces a historical benchmark convention, including its possible look-ahead. `last-edit-final` emits an edited final version at its known edit time, but still lacks the original version and actual local receipt time. Neither creates an observed NEW/EDIT chronicle. Selecting a mode is explicit; the exporter does not automatically replace research inputs or declare them qualified.

If an original raw reference is missing, whole-file byte equality is unverified. A separate canonical/parser comparison can establish which consumed fields agree, and an identical-binary replay can establish economic equality for a specified time window. These are different proofs: an out-of-window edit or a metadata-only difference must still be reported as a raw mismatch, even when replay results agree. A policy requiring literal bytes remains unsatisfied until the literal comparison passes.

Keep each export in a new version directory. The raw acquisition and resulting bytes are preserved without rewriting earlier snapshots. Later changes are visible by comparing those versions; this archive does not pretend to recover a snapshot that was never captured.

Run the synthetic tests with:

```powershell
python -B -m unittest discover -s tools -p "test*telegram*.py" -v
```
