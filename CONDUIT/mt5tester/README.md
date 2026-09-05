# Native MetaTrader 5 comparison

`CONDUIT_XT.mq5` is an independent MQL5 implementation of strategy management.
The bridge uses the Rust parser. Matching inputs do not by themselves establish
matching execution. Compare native deal timestamps, prices, volumes and costs.

Use a private, separate portable terminal. The tools require its explicit
offline marker, disabled chart experts and a dead local proxy. They never
configure the installed terminal. Both supplied experts refuse live charts.
Account caches, terminal profiles, market data and experiment outputs must stay
outside the public repository, in an ignored `VPSREADY_...` directory or a
private work directory. No terminal binaries or credentials are distributed.

## Prepare

1. Build `btp` and `most_eaF22audit` from the same frozen source revision.
2. Export the complete defaults with that `btp --dump-settings` binary.
3. Run `provision.py --help` to create a fresh private sandbox from local
   terminal binaries and an existing cached broker context. Select an explicit
   server, symbol and cached months. The source terminal is read only.
4. Use `CONDUIT_TICK_DUMP` through `portable.py` to export the native tick stream.
   Verify every timestamp and quote against the corresponding CDTK interval.
   A coincidentally equal tick count is insufficient.
5. Match the native `BROKER_SPEC` in the Rust execution model: symbol point,
   digits, stops, volume bounds, leverage, swap and cash precision. Native
   LIMIT price improvement needs `--sim-limit-price-improvement`; quote
   precision is explicit with `--sim-price-digits`. Cost or fill-model changes
   are experimental inputs, not strategy improvements.

## Run and compare

Generate the bridge with the frozen `most_eaF22audit` binary using `--schema v2`,
the same preset, `--from`, exclusive `--to`, channel and explicit `--offset-ms`.
For a live replay, use `--live-telegram-ingress` in **both** bridge and btp. It
uses the shared bounded content deduplication and stale opening gate. The
bridge filters its window using preset execution latency; XT adds that latency
when dispatching records. Do not add it a second time to `--offset-ms`.

`contract.py --defaults ... --preset ... --bridge ... --out ... --diagnostic`
writes explicit EA inputs and a field-by-field contract. Unsupported active
features produce a nonzero exit and remain visible. A diagnostic experiment
can investigate those gaps, but cannot be described as established parity.
Set `In_DiagFile` to a unique relative Common/Files path for each run.

`portable.py --sandbox ... --out ... --expert CONDUIT_XT --symbol ...
--from YYYY-MM-DD --to YYYY-MM-DD --deposit 300 --leverage 500
--parameters ...` compiles the canonical source and runs local real ticks.
It records source/binary/report hashes, the actual symbol specification and
fresh diagnostic records. A per-sandbox lock prevents overlapping launches.

Copy the diagnostic CSV from the terminal's Common/Files location into the
private experiment. Compare it with the matching btp `--dump-trades` output:

```text
python compare.py --native diagnostics.csv --rust transakcje.json --out comparison.json
```

`experiment.py` orchestrates the bridge, native run, Rust run and comparisons.
It also checks final marked equity against a native snapshot captured at the
end of the last strategy tick. MT5 liquidates open positions before `OnDeinit`,
so querying that callback's account state would hide the remaining portfolio.
The native closed ledger excludes this technical liquidation. Its cash result
and the final floating result are reported separately. Individual Rust pending
and open-position states still require additional comparison evidence.

`coverage.py --space ... --defaults ... --out ...` audits every configuration in
a sweep manifest, records its settings hash, classifies changed axes and lists
active unsupported settings. `vol_size_mode=Percentile/Target`, for example,
currently fails; parameters under `vol_size_mode=Off` remain explicitly inactive.
Source references and input mappings alone never establish execution parity.

Begin at a 0.01 lot ceiling and increase it gradually. Verify that larger
ceilings actually result in larger volumes before calling them stress tests.
Inspect the first divergent event before drawing conclusions from total P&L.
Reset deposits with separate terminal runs for each daily, weekly or monthly
window. Quote warmup, publication/receipt times, session boundaries and
effective settings must remain explicit and identical in both systems.

## Implemented contracts and limits

- Schema 1/2 validation, chronological bridge records and a hard 12-action
  limit prevent silent truncation. Harness runs require schema 2.
- Hedging is required. A netting account fails initialization.
- An observed target before the first fill is separate from an executed
  target stage, so it cannot consume a later partial close.
- Realized basket PnL reconciles confirmed partial exits while the remaining
  position is alive. The final close books only its incremental amount.
  The native contract uses broker-only realized accounting; RF command
  estimates cannot book the same receipt a second time.
- An Armed grid edit requires confirmed cancellation and a clean broker
  snapshot before replacement. A refusal, retained order or fill preserves
  committed strategy geometry and retains a session review flag blocking
  new basket risk. Existing position management remains available.
- Explicit pending validity retains publisher LIMIT/STOP plans until a bound
  source cancellation. Risk exits remain authoritative. A cancellation
  tombstone prevents new orders and retries broker-refused pending removal.
- Daily trailing can use total peak equity or peak daily profit. Day, EOD
  and weekend guards include exposure consisting only of pending orders.
- Native fault scenarios 1–13 in XT exercise confirmed exits, broker refusal,
  cancellation/fill races, residual partials, partial accounting, refused
  grid edits and clean replacement, including legacy and no-fault controls.
  Further scenarios cover known special grid legs, profit-budget flooring
  against acknowledged exposure, a standalone portfolio cap before profit
  reserve activation, receive-time source recovery and legacy orphan rejection.
  Source identity and aliases survive basket pruning within
  a native run. XT accepts one source channel per experiment; its fresh tester
  state does not claim the live application's persisted account restart proof.
- Fast addons apply the lot ceiling after their multiplier. Local invalid-TP
  checks preserve capacity and start the configured cooldown; transmitted
  refusals consume an attempt. The
  `OPEN_VOLUME_AUDIT` record measures final transmitted and accepted-request
  maxima, including refusals, rather than inferring volume from partial closes.
- Comparator schema v2 reads SimBroker profit as already including realized
  swap. Canonical receipts include all costs and must reconcile. Commission
  and swap breakdown fields are never added a second time. Earlier immutable
  comparator reports need a separately labelled measurement correction.
- Strategy rankings and gross floating-profit thresholds use the same raw
  mark-to-market formula as core, excluding swap and broker cash rounding.
  Account equity and realized receipts retain their actual costs.
- `experiment.py --trade-sessions profile.json` validates observed native
  sessions against an explicit broker execution profile, without filtering
  quotes or source messages. See [the execution contract](../docs/BROKER_TRADE_SESSIONS.md).
  `--native-source` can pin an immutable MQ5 snapshot for the comparison.
- Adaptive trailing uses the same sampled path efficiency, fast/slow movement
  ratio and directional gap multipliers as core. A contract error rejects a
  requested adaptive window beyond shared history retention. It still needs
  native regression evidence for each tested strategy family.
- Matching closed trades alone does not prove matching pending state,
  restart behavior or all broker failure paths. Preserve separate evidence
  for these contracts and do not label a one-day comparison universal parity.
