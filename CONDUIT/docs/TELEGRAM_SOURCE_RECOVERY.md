# Telegram source recovery and order lifetime

The source key is `(chat_id, topic_id, original_message_id)`. Telegram originally publishes NEW before its edits. A listener can be offline and
observe EDIT as its first available version; synthetic tests also exercise delayed
redelivery of a previously published NEW. Receiving edited text does not
provide evidence of its earlier contents or prices.

## Existing preset policy

`edycja_sieroty_nie_otwiera = true` retains the legacy rejection of entry actions in
an EDIT whose source has no known basket. This setting remains available for
historical reference reproduction.

`false` permits the first complete `Entry` to follow the normal entry path at the
current receive timestamp. It does not change that timestamp, quotation, execution
latency, filters, margin checks or risk settings. Recovery requires:

- finite, positive, ordered entry geometry;
- an explicit finite, positive SL beyond the adverse edge of the source zone;
- finite, directionally valid TP levels, or an explicit TP OPEN instruction;
- no conflicting LIMIT and STOP flags and no invalid layer offset.

A bare `MarketOpen` / BUY NOW orphan is still rejected because it does not contain
a complete protected entry. An ordinary fully specified market `Entry` is eligible
for the same validation as a pending `Entry`. A previously received informational
NEW followed by its first complete EDIT uses the same recovery path.

## Persistent identity

Accepted NEW, market commands, recovered EDIT and explicitly merged sources are
recorded before their order sends in the Engine. The source record contains its
basket identity, known reply aliases, whether the first accepted entry was an EDIT,
the last accepted entry-edit time and any publisher withdrawal timestamp. The live
risk snapshot retains these records independently of optional strategy continuation
and independently of retained basket history. Account/server/mode/magic/symbol scope
is the same as the existing live account risk snapshot.

Repeated EDIT manages its existing basket and cannot create another one. A recovered
source remains idempotent even with `entry_idempotencja = false`. A later NEW cannot
replace geometry already accepted through EDIT, including after a listener restart.
An accepted source whose basket has been pruned is consumed; it cannot be recreated
by a later EDIT. A pre-basket risk rejection is not acceptance: another complete
revision may be evaluated against the then-current risk state.

## Publisher withdrawal

A CANCEL replying to a source message records a withdrawal even if that source's
NEW was never received. When recovery and transitive replies are enabled, observed non-entry
reply links preserve the bound source identity through restart. An unrelated
unbound CANCEL does not create a guessed source tombstone. A bound CANCEL whose
source has no basket never selects an unrelated latest basket.

A withdrawal blocks later source-entry recovery or redelivery. Existing broker
cancellation, filled-position management and rearm behavior remain controlled by
the existing preset axes. This source ledger does not authorize closing filled
positions, bypassing risk, or automatically recreating an old order.

`explicit_pending_until_cancel` remains a separate optional preset order-lifetime
policy. Its existing true behavior protects remaining explicit-source pending
orders from the preset's ordinary expiry paths. False retains ordinary preset
TTL/TP/rearm management. Source identity and cancellation memory do not depend on
that option. The option must not be forced onto a preset that was tested with false.

## Limits and evidence

A fresh installation cannot know entries or withdrawals that are absent from all
available history. Recovery accepts the currently observed complete intent under
the selected policy; it does not reconstruct missing earlier fills or source text.
The engine's record plus account snapshot is not a transactional broker/consumer
checkpoint: an abrupt crash during a send still requires broker reconciliation.
No feature can infer unobserved historical originals from Telegram's final export.

Synthetic regressions cover receive-time entry, duplicate EDIT, late NEW, material
revision followed by stale NEW, informational NEW followed by complete EDIT, missing
protection, legacy policy, risk rejection, bound unknown CANCEL, transitive reply,
source separation, basket pruning, persisted account scope and source cache revision.
The live benchmark compares complete trades and equity paths with ingress dedup
both enabled and disabled, using the same source policy.
