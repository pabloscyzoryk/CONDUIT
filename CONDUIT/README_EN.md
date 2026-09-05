# CONDUIT — source code

CONDUIT is a Telegram-to-MetaTrader 5 copy-trading terminal. It combines live
message intake and revision tracking, parsers for multiple formats, one basket
management engine, broker execution, a React panel, and a tick backtester.

This is a public, sanitized **source-only** tree. It contains no Telegram
session, `api_id`/`api_hash`, password, MT5 account details, private channel or
topic identifiers, logs, ticks, chat exports, sweep results, or executables.

## GOD-X7 versus the CONDUIT engine

`config/presets/` contains **109 public presets** accumulated during
development. `GOD-X7.json` is the current recommended reference for the
Synergy format, but it is not a separate bot and it does not exhaust the
project. CONDUIT exposes **498
core axes** covering entries and grids, sizing, exposure, TP/SL, partials,
break-even/risk-free, runners, trailing, re-entry, regime/session filters,
broker costs, pending-order semantics, journaling, and live/backtest parity
contracts. An axis affects behavior only when enabled and not shadowed by a
higher-level rule or chain override.

Built-in formats currently include `ATFX`, `Synergy`, `ZEN`, `PULSEX`, `NOVA`,
`TWP`, `DANGER`, `CLUB1`, and `STORM`. Each source may use an independent
preset or remain listen-only. Modes include `MANUAL`, `AUTO`, `AUTO-EA`, and
`AI`; the UI describes their exact semantics.

## Architecture

```text
Telegram MTProto ──> history/live adapter ──> format parser
                                                │
React UI <──WebSocket/REST── Axum server <───────┤
                                                v
                                       conduit-core Engine
                                       /                 \
                               SimBroker (backtest)   Broker trait (live)
                                                         │
                                              Rust MT5 bridge/TCP
                                                         │
                                              mt5_sidecar.py
                                                         │
                                             MetaTrader5 Python API
                                                         │
                                              MetaTrader 5 terminal
```

- `src/`: TypeScript/React UI, axis schema, and transport handling.
- `rust/crates/core/`: parser, basket state, and shared decision logic.
- `rust/crates/backtest/`: deterministic simulated broker, ticks, and metrics.
- `rust/crates/telegram/`: MTProto login/session, messages, and edits.
- `rust/crates/mt5/`: Rust ↔ Python ↔ MetaTrader 5 bridge.
- `rust/crates/server/`: REST/WebSocket, UI state, configuration, and journal.
- `rust/crates/app/`: the `conduit` executable and Tauri/WebView2 window.
- Other crates provide sweep monitoring, export, chronicle, graphing, and the
  experimental AI layer; all compile from the same workspace.
- `mql5/`: Strategy Tester EA source; no `.ex5` is included.

Backtest and live use the same `conduit-core::Engine`. They differ at the broker
interface: tests use `SimBroker`, while live uses the MT5 bridge. This minimizes
logic drift, but cannot remove physical spread, slippage, network delay, order
rejections, requotes, or broker specification changes.

## Requirements

- Windows 10/11 x64 for the target MT5 terminal and desktop window.
- Rust **1.85+** with the MSVC toolchain, Visual Studio Build Tools (“Desktop
  development with C++”), and a Windows SDK.
- A Vite 7 compatible Node.js: at least 20.19 or 22.12; npm with the lockfile.
- Microsoft Edge WebView2 Runtime for the Tauri window.
- MetaTrader 5 x64.
- 64-bit Python 3.8+ available as `python`, with the `MetaTrader5` package.
- Your own Telegram application `api_id` and `api_hash` from
  <https://my.telegram.org>.
- MetaEditor 5 only if you want to compile `mql5/`.

## Clean build

From PowerShell in the repository root:

```powershell
npm ci
npm run build
npm run build:kronika
npm run build:wiz

Set-Location rust
cargo build --release -p conduit-app --bin conduit
```

Order matters. The UI is embedded with `rust-embed`, so build all frontends
**before** Rust. The main output is `rust/target/release/conduit.exe`.

Headless diagnostic build:

```powershell
Set-Location rust
cargo build --release -p conduit-app --bin conduit --no-default-features
```

Backtest CLI:

```powershell
Set-Location rust
cargo build --release -p conduit-backtest --bin btp
./target/release/btp.exe --help
```

For MQL5, open `mql5/CONDUIT_XT.mq5` in MetaEditor, compile it, and review every
compiler message. The tester EA is not required for the live Python sidecar.

Prepare Python:

```powershell
py -3 -m venv .venv
./.venv/Scripts/python.exe -m pip install --upgrade pip
./.venv/Scripts/python.exe -m pip install MetaTrader5
```

Select that interpreter in CONDUIT or put it on `PATH`. Copy
`rust/crates/mt5/sidecar/mt5_sidecar.py` next to the deployed `conduit.exe`.

## First run with your own accounts

1. Create an empty deployment directory and copy `conduit.exe` and
   `mt5_sidecar.py` into it.
2. Copy files from `config/examples/` next to the executable and remove the
   `.example` suffix. Copy the complete `config/presets/` directory as the
   deployment's `presets/` directory. GOD-X7 is recommended, while the UI can
   discover every JSON file in that directory.
3. Start MT5, sign in to **your demo account**, enable Algo Trading, and expose
   the correct gold symbol in Market Watch.
4. Start CONDUIT. Enter your own `api_id`/`api_hash`, scan its Telegram QR code,
   and complete login. The secret/session is generated locally—never commit it.
5. Refresh dialogs in Channels, select a channel or forum topic, enable
   monitoring, and assign the **correct message format**. IDs come from your
   account; this repository ships no private binding.
6. Map a chosen preset to that format in the chain. The neutral template does
   not start automated trading; GOD-X7 is the reference recommendation and
   other formats may remain listen-only.
7. Stay in `MANUAL`, inspect parsing and orders on demo, then test `AUTO` on
   demo. Consider a real account only after your own validation.

## Preset workflow

A preset is JSON containing axis values. A defensible workflow is:

1. Copy and rename a preset; never mutate the reference in place.
2. Change one axis family at a time and record the config version/hash.
3. Keep ticks, message revisions, costs, and clock identical between runs.
4. Test the full window, isolated days/weeks, multiple deposits, and lot caps.
5. Run robustness checks, then demo-live observation.
6. Before production, verify panel/account/chain overrides do not shadow preset
   fields and that the runtime file hashes match the reviewed candidate.

No included preset proves an optimum for another broker, month, format, or
risk budget. GOD-X7 is the current recommendation, not a performance
guarantee. A 498-axis search space has strong interaction and overfitting risk.

## Safety: DEMO → REAL

The supplied template starts in `MANUAL` and sets
`mt5_allow_real_account=false`.

- Never enable REAL merely because the project compiled.
- Verify account identity, DEMO/REAL mode, server, symbol, `digits`,
  `volume_min/step/max`, `stops_level`, fill policy, spread, and swap.
- Do not run two instances controlling the same magic/symbol.
- Configure risk limits for your account. A zero often disables a limit; it
  does not necessarily mean maximum safety.
- Exercise new signal, SL/TP edit, cancel, TP1/TP2/TP3, partial, restart,
  sidecar loss, and manual account switching on demo.
- Authorize REAL deliberately only after documented demo parity.

## Tests

After building the frontends:

```powershell
npm run typecheck

# MQL/UI source-contract tests (build the Rust probe first):
Set-Location rust
cargo build -p conduit-server --example ui_alias_probe
Set-Location ..
npm run test:node

Set-Location rust
cargo test --workspace --lib --offline
cargo test -p conduit-mt5 --offline
```

Tests that depend on private corpora, ticks, or an external terminal are not in
the public repository and should either report missing data explicitly or be
skipped. Python sidecar tests:

```powershell
Set-Location rust/crates/mt5/sidecar
python -m unittest discover -p "test_*.py"
```

Cargo `--offline` works only after the local cache contains every crate locked
by `Cargo.lock`. This repository does not vendor Rust or npm dependencies. On
a clean machine, run the first `cargo fetch`/`cargo test` and `npm ci` with
network access, then use offline mode for reproducible repeat checks.

## Troubleshooting

- **New binary serves an old UI:** remove `dist`/generated `web`, run all three
  npm builds, then rebuild Rust.
- **`rust-embed` cannot find `web/`:** the frontend build step was skipped.
- **MT5 disconnected:** start the x64 terminal, verify the chosen Python and
  `python -c "import MetaTrader5"`, terminal path, and that only one sidecar is
  active.
- **Wrong `XAUUSD`/`XAUUSD.s`:** keep auto-detection or select the exact symbol
  exposed by the current server; verify again after switching accounts.
- **Telegram shows no channels:** verify your own API credentials, log in
  again if needed, then refresh dialogs.
- **Messages arrive but no trade is placed:** check monitoring, topic, format,
  preset mapping in the active chain, mode, halt state, and signal validation.
  Listen-only is a valid state.
- **Backtest differs from live:** compare clock, edit history, symbol, spread,
  costs, pending-fill semantics, stops level, latency, and account/chain
  overrides first. A backtest cannot know future slippage or broker rejection.

## Never commit these data

Do not commit `secrets.json`, `*.session`, `.env`, settings containing a login,
mapped `channels.json`, private chain bindings, logs, chronicles, Telegram
exports, MT5 history, ticks, reports, dumps, private-data models, or binaries.
`.gitignore` covers common cases, but scan the complete Git history yourself
before publishing.

## License and risk

The source project did not contain an unambiguous public license, so this copy
does not invent one. Default copyright rules still apply. This software makes
no profit guarantee and does not establish regulatory compliance. Leveraged
trading can rapidly lose the entire account.
