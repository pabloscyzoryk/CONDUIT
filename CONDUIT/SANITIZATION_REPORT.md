# Sanitization report

This directory is a source-only public snapshot of CONDUIT. It is intended for
review, compilation and configuration with credentials owned by the person who
runs it. It is not a copy of a working private installation.

## Included

- Rust workspace sources and Cargo manifests/lockfile.
- TypeScript/React frontend sources and npm manifests/lockfile.
- MQL5 and Python helper sources required by the documented workflows.
- Build-time web assets and neutral configuration templates.
- 109 public preset JSON files. `GOD-X7` is marked as the current recommended
  preset, but it is one selectable configuration among many.
- Self-contained synthetic test fixtures needed to test parsing, routing,
  account-following and execution semantics.

## Excluded or neutralized

- Git history and repository metadata.
- Compiled programs, native libraries, debug symbols, source maps and archives.
- `target`, `node_modules`, `dist`, generated web/app directories, TypeScript
  build-info files and Python bytecode caches.
- Runtime state, journals, logs, merged logs, tick data, chat exports, sweep
  outputs and private analysis results.
- Telegram sessions, API credentials, bot tokens, peer/channel/topic identifiers
  originating from a private installation and private message bodies.
- MT5 account numbers, passwords and private broker-login configuration.
- SMTP credentials, email addresses, phone numbers, cloud/API tokens and local
  owner paths.
- Private channel-derived test provenance, real-looking message identifiers,
  handles, dates, prices and verbatim message fixtures. Equivalent parser and
  routing coverage uses explicitly synthetic examples instead.

Names such as `alllogs` and `SWEEP` that remain in source paths are program
features or public preset names, not bundled runtime logs or sweep-result data.

## Validation performed

The clean source snapshot passed the following validation before the final
source-only cleanup:

- npm offline dependency installation: 118 packages, 0 reported vulnerabilities;
- all three documented frontend builds;
- `cargo check --all-targets --offline` for the Rust workspace;
- Node tests: 81 passed;
- Python tests: 86 passed;
- Rust library/test suites: AI 66 passed; backtest 234 passed; core 438 passed
  and 1 ignored; exporter 37 passed; monitor 33 passed; mozg 4 passed; MT5 71
  passed; server 399 passed and 1 ignored; Telegram 156 passed;
- focused regression suites: risk-free chronology 5/5, routing collision 10/10
  and synthetic parser cases 3/3;
- all 109 distributed preset files parsed as valid JSON;
- `cargo metadata --offline --no-deps` succeeded after build artifacts were
  removed;
- Rust parser checks succeeded for every source file modified during the final
  fixture sanitization.

Dependencies installed solely for validation and all generated build output were
then removed from this directory. The snapshot therefore does not ship with
`node_modules`, `dist`, `target`, executables or other generated binaries.

## Known validation limits

- MQL5 source was inspected but was not compiled in MetaEditor in this clean
  snapshot.
- No Telegram login, MT5 login or live order was performed during public-source
  validation.
- A future source change or a user-supplied preset/configuration can introduce
  new sensitive content. Run a fresh secret/privacy scan before every public
  release.
- Build results can vary with toolchain versions; use the versions and commands
  documented in the READMEs.

No license file was invented. Publication or redistribution rights must be
confirmed by the repository owner before publishing the source.
