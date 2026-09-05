# Privacy audit

Audit scope: every file in this source snapshot, after removal of build output.
The audit was designed to detect both ordinary credentials and data copied from
the private development/runtime corpus without printing candidate secret values.

## Result

No unresolved private-data finding was detected in the audited snapshot.

| Category | Result |
| --- | ---: |
| Local owner names and absolute owner paths | 0 hits |
| Email addresses and phone numbers | 0 hits |
| Telegram API IDs, API hashes, bot tokens and session strings/files | 0 hits |
| Private Telegram peer, channel and topic identifiers | 0 hits |
| MT5 login, server, password and account values from private configuration | 0 hits |
| SMTP host/user/password and mail credentials from private configuration | 0 hits |
| Common cloud/API token formats and credential-bearing URLs | 0 hits |
| Exact values extracted from private JSON configurations | 0 hits |
| Twelve-word overlap with the available private chat/export corpus | 0 hits |
| Runtime logs, ticks, chat exports and backtest-result data files | 0 files |
| Compiled/build/cache artifacts | 0 files |

The exact-value comparison examined 124 JSON files outside this public snapshot.
It extracted 8 identifier values and 17 secret/contact values from recognized
credential and runtime fields; none occurred in this snapshot. Values and source
paths are intentionally not reproduced here.

The chat-corpus comparison scanned the available export/attachment corpus using
normalized twelve-word shingles. No repository source line shared such a window
after real-derived fixtures and provenance were replaced with synthetic cases.

## Reviewed candidates

The scanner conservatively reported 24 credential-literal candidates and 30
high-entropy candidates. Each was reviewed. They are synthetic placeholders,
test-only sentinels, normal identifiers/function names, schema explanations or
format examples; unresolved findings: **0**. Obvious placeholder credentials in
tests and documentation remain deliberately nonfunctional.

Five public presets have `SWEEP` in their names, three Python source files
implement the public quick-sweep workflow, and two Rust source/test files
implement the `alllogs` feature. These names triggered a filename heuristic but
are neither sweep outputs nor bundled log data.

## Method and limitations

The audit combined:

1. path, extension and runtime-artifact deny lists;
2. credential/contact/token regular expressions;
3. literal credential-field inspection;
4. entropy-based candidate detection;
5. exact comparison with values extracted from private configuration files;
6. normalized n-gram comparison with the available private Telegram corpus;
7. manual review of all conservative candidates.

No static scanner can prove the absence of every imaginable private fact. In
particular, entropy and regex heuristics can miss novel encodings, and the corpus
comparison covers only files available at audit time. The repository owner should
run a new scan after every modification and inspect the staged Git diff before
publishing. Never commit a populated secrets/configuration file.
