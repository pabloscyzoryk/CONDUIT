# Offline effective configuration audit

The checked live configuration consists of the selected leg's strategy settings,
the shared account fields from the settings document, and the active chain's
16 independent limits. A preset file alone does not describe all three layers.

Build the offline helper with `cargo build -p conduit-server --example
effective_settings_probe` in `CONDUIT/rust`. Run `tools/effective_settings_audit.py`
with `--workspace`, `--probe` and `--output`. The wrapper resolves the active
Synergy preset by its internal name. Conflicting duplicate identities fail the
audit. It reads settings, chain, channel bindings and preset files only, and
starts no application service. Credentials and Telegram sessions are not read.

The helper compares every typed `Settings` field and every chain limit. It calls
the production UI mapper, lot mapper, account overlay function and routing
constructor. Both single-leg and multiple-leg construction must give the same
configuration. The audited app call sites are `server::bootstrap`,
`app::live_core_from_ui`, `app::zbuduj_silniki`, live start, reload and account
switch. Bootstrap gives embedded canonical lot fields precedence over the outer
lot card. The selected preset then owns lot mode, fraction, fixed size and
minimum/maximum; `lot_base` and credit policy belong to the account layer.

Use `--preset` and `--selection` to inspect a proposed package recipe in memory.
The preset hash, all required account overlay fields and all chain limits must
be explicit. An unapproved proposal may be audited; this never changes its
approval or creates a package. Without `--expected`, the comparison target is
the selected preset plus the proposed account overlay, with the proposed caps.
For an existing package without a proposal, the comparison target is its preset
with zero chain caps. `--expected` and `--expected-caps` provide an independent
research target. `--broker-stops` applies an explicitly observed broker stop
distance as live startup does; it does not pretend to discover terminal facts.

Results disclose differences and their layers, without raw settings or channel
identities. Terminal paths are redacted. A missing observed Synergy binding or
another active observed leg prevents the Synergy-only match stamp. Live NET
accounting and the research-only exact-tick S/R producer are explicit blockers.
Operational differences, such as watchdog preferences, remain visible among
the full-field differences; they must not be misreported as trading decisions.

Configuration equality is necessary but is not an execution parity certificate.
Ticks, receive times, source revisions, costs, broker session rules, fills and
restart receipts still need their independent replay and broker tests. A fresh
package excludes old backup state; auditing a later resumed installation also
requires checking its persisted chain ladder and account-scoped risk state.
