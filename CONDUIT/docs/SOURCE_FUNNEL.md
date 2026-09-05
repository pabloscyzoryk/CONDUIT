# Source activity and basket activity

`stat_sygnalow.lejek.source_observation_version = 1` identifies the causal
source funnel. Older reports counted only NEW messages without replies, so an
entry whose first observed complete version was EDIT was absent from their
denominator even when it opened a basket or was rejected. Do not compare those
old percentages as though their denominator had the current definition.

The reporting observer uses the selected engine's parser options. The first
observed Entry or MarketOpen source counts once at receipt, after live ingress
deduplication/freshness when that replay mode is selected. Full first EDIT,
NEW draft followed by full EDIT and independent entries in replies count.
Redelivery and material revisions keep their original identity. Known bound
management aliases update the existing source and add no independent intent.
Sources rejected by risk or entry policy still belong to the denominator.

Replay currently preserves the channel/format namespace and original message
ID, but does not retain actual raw Telegram chat/topic identity. The reporting
key is therefore `(channel namespace, edit_of or msg_id)` and the report states
this limitation explicitly. It does not silently change the historical Engine
SourceKey or execution. Ambiguous outcomes caused by an ID collision inside one
legacy synthetic engine are reported as unattributed rather than assigning one
broker outcome to every candidate namespace.

`sygnaly_wejsciowe` counts independent offered sources. `koszyki_sygnaly` counts
independent accepted sources, including two NEW signals merged into one basket.
`odrzucone_sygnaly` counts unique rejected sources that never became accepted.
Their signed difference is `zgubione_bez_sladu`; it is not clamped. Unattributed
entry outcomes have their own counter and must be investigated separately.

Basket fields remain separate: `koszyki` is the complete basket ledger and
`koszyk_z_handlem` means a basket with at least one closed trade, not proof of
every first fill. `wykonanych_pct` retains the basket-with-closed-trade numerator
over the corrected source denominator. `accepted_entry_sources_pct` measures
accepted independent sources over offered independent sources. Neither a
profitable trade count nor accepted management commands is a source denominator.

The observer runs outside the strategy. It reads engine outcomes, does not
authorize sends, does not change broker cash, and survives independent daily
account resets without carrying an old source's trading authority into them.
