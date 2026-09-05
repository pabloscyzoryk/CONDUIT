/* PLIK GENEROWANY — NIE EDYTOWAĆ RĘCZNIE.
   Źródło: rust/crates/core/src/wielosilnik.rs (POLA_RACHUNKU)
   Generator: narzedzia/pola_rachunku.mjs
   Pól w Ruście: 54 · zna je panel: 54
   Pola wyłącznie po stronie serwera: brak
*/
import type { Settings } from "@/types";

/** Wspólna własność rachunku; preset nogi nie nadpisuje tych pól. */
export const POLA_RACHUNKU = [
  "close_receipt_reconcile",
  "closed_profit_net_costs",
  "restore_strategy_continuation",
  "order_volume_contract_v2",
  "commission_per_lot",
  "swap_enabled",
  "swap_long_points",
  "swap_short_points",
  "swap_point_value",
  "swap_rollover_mult",
  "swap_rollover_weekday",
  "swap_pomijaj_weekend",
  "swap_rollover_z_serwera",
  "swap_rollover3days_mt5",
  "runner_ksiegowanie_v2",
  "msg_kurs_sprzed_luki",
  "slippage_pts",
  "slippage_pending_pts",
  "sim_stops_level",
  "exec_latency_ms",
  "msg_clock_offset_h",
  "server_tz_offset_h",
  "stop_out_level_pct",
  "margin_call_level_pct",
  "expo_cap_pct",
  "sim_margin_check_on_fill",
  "sim_validate_pending_stops",
  "sim_margin_at_market",
  "expo_cap_ml_pct",
  "expo_cap_close",
  "expo_cap_s",
  "mt5_autostart",
  "mt5_watchdog",
  "mt5_health_interval_s",
  "mt5_restart_after",
  "mt5_retry_attempts",
  "mt5_retry_delay_s",
  "mt5_terminal_path",
  "journal_enabled",
  "journal_min_level",
  "journal_text_mirror",
  "journal_retention_days",
  "journal_excursions",
  "journal_snapshots",
  "journal_buffer_cap",
  "ai_mode",
  "ai_model",
  "ai_decision_interval_s",
  "ai_replaces_management",
  "lot_base",
  "odlicz_kredyt",
  "credit_balance_separate",
  "kredyt_reczny",
  "konto_dzwignia",
] as const satisfies readonly (keyof Settings)[];

/** Account overlay corresponding to wielosilnik::ustawienia_formatu. */
export function polaRachunkuZ(doc: Settings): Partial<Settings> {
  const out: Record<string, unknown> = {};
  for (const key of POLA_RACHUNKU) out[key] = doc[key];
  return out as Partial<Settings>;
}
