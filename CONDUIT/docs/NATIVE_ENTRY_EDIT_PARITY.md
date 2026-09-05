# Native legacy EntryEdit contract

The native expert implements the legacy edit path (`entry_edit_geometry_v2=false`).
The experimental V2 edit strategy remains an explicitly unsupported active setting.
This checklist records source coverage; a matching full-window ledger is a separate
validation requirement.

| Stage | Rust contract | Native implementation |
|---|---|---|
| Source identity | Original message and aliases resolve the existing basket; source cancellation wins | Existing source ledger and reply map, without latest-basket fallback for an unknown cancellation |
| Accepted source snapshot | `EntryEditState.source` contains the raw EntrySignal | `NativeEntryPlanSource`, captured before target filtering or runner expansion and kept with the basket |
| Exact source equality | Side, LIMIT/STOP, original lo/hi, optional SL, raw TP sequence, TP OPEN and optional text layer offset | `NativeSameEntrySource`; metadata and delivery keys are excluded |
| Repeated source after management | Exact source equality returns NoOp before changing a tightened SL, target progress or broker orders | `ApplySourceEntryEdit` returns before any execution call; current managed geometry is not the equality key |
| Rejected source | Wrong side or directionally unordered targets do not commit a revision | Wrapper returns false; source edit acknowledgement is recorded only for handled changes or NoOp |
| Existing review/exit/cancellation | No new edit transaction while confirmation is unresolved | Existing execution guards precede source and geometry changes |
| Effective zone | Original zone plus configured price/directional offsets, ordered boundaries | Existing native offset calculation; unsupported distance-to-SL offset remains rejected by the preset contract |
| Effective SL | Optional source SL, minimum distance then maximum distance | Same calculation; absent source SL clears only basket SL state and does not issue a broker stop-removal request |
| Effective targets | Price filtering then runner expansion | `PrzygotujCele`; preflight rejects a raw/expanded target bound beyond the native capacity |
| Material-plan progress | Exact change of effective zone, optional SL or TP sequence resets progress, including Working baskets | `NativeEntryPlanChanged` and `NativeResetEntryProgress`, independent of the SPP compatibility switch |
| Reset scope | TP stage, observed plan stage, zone touch, drop arming/deadline, last TP and target/SL observations | All stored native counterparts are reset; native does not retain separate target-price/SL-touch buffers. Security state, addon count, economics and source delivery memory remain intact |
| Armed transaction precondition | Clear broker receipt barrier before edit mutation | Incomplete native ownership snapshot enters review before any stop modification or cancellation |
| Armed replacement threshold | Zone tolerance below 1e-9, equal optional SL truncated at 1e-9, exact TP sequence | `NativeEntryGridUnchanged` is separate from exact material-plan reset |
| Replacement confirmation | All owned pending cancellations confirmed; no new fill, remaining pending, session change or unresolved receipt | Existing native before/after ownership snapshots and confirmed cancellation count |
| Failed replacement | Restore committed geometry and progress, reconcile actual broker state, retain review | Snapshot restore includes raw source state; successful broker changes are not blindly undone |
| Successful revision | Commit accepted raw source only after successful handling | Wrapper stores the source after the review check |
| SPP | Separate target/stop management path | Remains separate; ENTRY NoOp and reset do not depend on the SPP reset switch |
| Lifetime retention | Source state survives basket compaction while active ownership remains | Source snapshot is a Basket field copied by stable compaction; this fresh-tester contract does not claim native live-account restart reconstruction |

Validation includes actual native fixtures for changed TP/zone/SL, identical plans,
SPP ON/OFF, repeated raw source after a confirmed broker SL change, incomplete
ownership before mutation, failed and successful Armed replacement, source
recovery and replay. Fixture and full-window outcomes belong to immutable private
experiment receipts; the existence of a fixture is not itself a passing result.

The target preflight runs after bridge generation and before native execution.
It fails the comparison when raw targets plus enabled runner expansion exceed
`MAXTP`, without removing signals or quietly changing target geometry. This is a
conservative bound because future price filtering can reduce the target count.
