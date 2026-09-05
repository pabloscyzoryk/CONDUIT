# Live benchmark

The benchmark checks execution decisions at several boundaries. Passing a
synthetic replay alone does not certify a live broker or historical data.

| Layer | Evidence | Failure to investigate |
|---|---|---|
| Frozen historical reference | Input, preset and executable hashes; every closed transaction and daily ledger | First different request, fill, modification or close |
| Telegram ingress and strategy | `live_benchmark_contract` and `telegram_adversarial` Rust integration tests | Missing first complete edit, duplicated exposure, stale NEW rollback, wrong reply ownership |
| Application restart | Application restart differential tests with persisted account scope and source records | Lost ownership, repeated entry, reset profit anchor or repeated settlement |
| Native MT5 tester | Immutable EA build, matching physical ticks, broker contract and transaction comparison | Unsupported broker session, rejected requests, fill sequencing, price or cash rounding |

Management follows the selected preset. Source-message validity and cancellation
memory do not force a different lifetime for the preset's pending orders. The
explicit keep-pending option remains a separately selected management rule.

## Deterministic checks

From the repository root:

```powershell
cargo test --manifest-path CONDUIT/rust/Cargo.toml -p conduit-backtest --test live_benchmark_contract
cargo test --manifest-path CONDUIT/rust/Cargo.toml -p conduit-backtest --test telegram_adversarial
python -m unittest discover -s tools -p test_live_benchmark_profiles.py
```

The first suite compares full transaction ledgers, balance and equity paths,
and basket counts. It exercises lot caps 0.01, 0.1, 1 and 10. It includes a
first complete EDIT followed by duplicate EDIT and NEW deliveries, volatile
listener restart, an earlier source-bound CANCEL, and a stale NEW following
a newer accepted entry revision. Each trading fixture must produce trades;
two silently inactive runs cannot pass as a positive execution proof.

An edit cannot precede its original publication on Telegram. "First EDIT"
means the first version observed by this bot, for example after it missed the
original message while offline. A later NEW in the stress fixture represents
application-level redelivery of that earlier publication; it does not assert
that Telegram normally delivers its ordered update stream backwards. The bot
acts at observation time and never backdates an order to the missed original.
The recovery suite also enables the ordinary five-minute stale-NEW gate and
checks a complete EDIT whose original publication was fourteen days earlier.
Its current-observation execution must match a fresh EDIT; the stale NEW
control must remain rejected.

The special listener-restart marker clears only content-dedup memory. It is
not an application restart; the separate application tests cover durable state.

## Full-window transport stress

```powershell
python tools/live_benchmark_profiles.py --source <snapshot.json> --out <new-output-directory> --profiles optimistic realistic pessimistic ultra edit_recovery reconnect
```

This retains the original four disturbance profiles and adds focused
edit-recovery and reconnect profiles. The input must contain one snapshot per
source message. Observed streams with several revisions of one message are
rejected rather than flattened. Existing output directories are never replaced.
Each manifest records the source and generator hashes.

These generated streams are synthetic sensitivity experiments. They are never
historical evidence and must not be used to rank strategy parameters. The
original four profiles include artificial message delays, typos, corrections,
reply errors and bursts. RF/BE commands retain the original benchmark's
zero-added-delay control; this does not claim such delays are impossible live.

All decidable execution invariants must pass. A plausible but false statement
from a publisher cannot always be identified from transport metadata; that
limitation is reported separately rather than hidden in an aggregate pass rate.

## Broker comparison

Start with a 0.01 lot cap and compare physical tick identities and ordered
requests before comparing money. Increase the cap only after explaining each
first divergence. Broker trading sessions are separate from quote sessions;
receiving a quote does not prove an order can be executed. Net realized profit
already includes the source-defined swap settlement: do not add swap twice.

The historical reference keeps its original execution contract. A new broker
profile is measured as a separate comparison, with all shared assumptions
fixed across presets. No compensation is applied merely to make a final
profit number match.
