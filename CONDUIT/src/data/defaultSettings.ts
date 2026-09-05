import type { LogCategory, Settings } from "@/types";

/**
 * Domyslne ustawienia — wartosci przepisane 1:1 ze slownika `settings`
 * w bot.py (linie ~386-860). Kazdy klucz ma tam odpowiednik.
 */
export const DEFAULT_SETTINGS: Settings = {
  /* --- wejscia --- */
  auto_limit: false,
  custom_entry: false,
  entry_high_offset: 0,
  entry_low_offset: 0,
  entry_offset_dir: false,
  entry_deep_offset: 4,
  /* M3: 0 = stala kwota powyzej; ulamek dystansu krawedz->SL, gdy > 0 */
  entry_deep_frac_to_sl: 0,
  entry_tol_offset: 0.3,
  sl_dist_limit: false,
  sl_dist_max: 0,
  sl_min_dist: 0,
  only_limit_signals: false,
  valid_till_tp2: false,
  ignore_old_after_min: 0,

  /* --- jednostki i siatka --- */
  entry_units: 1,
  entry_units_limit: 0,
  entry_risk_budget: 0,
  entry_tp1_budget: 0,
  entry_touch_units: 0,
  entry_touch_tp: 2,
  entry_touch_levels: "",
  ppm_enabled: false,
  ppm: 1,
  ppm_immediate: false,
  ppm_for_limits: false,
  grid_fallback_best_edge: false,
  pending_resize_on_vol: false,
  /* Aktualizacja wolumenu lezacych limitow po zmianie lota bazowego.
     Domyslnie OFF — zaden istniejacy preset nie zmienia zachowania. */
  pending_relot_on_balance: false,
  pending_relot_topup: false,
  /* Kierunki osobno. Oba ON, bo tak dziala relot od poczatku — rozdzielenie
     ma dawac mozliwosc WYLACZENIA jednej strony, a nie zmieniac domyslne. */
  pending_relot_up: true,
  pending_relot_down: true,
  
  pending_relot_wg_planu: true,
  pending_relot_reconcile_target: false,
  /* Prog kapitalu dla kierunku w gore; 0 = bez progu. */
  pending_relot_up_od_salda: 0,
  pending_resize_sec: 30,
  pending_ttl_h: 0,
  pending_never_cancel: false,

  /* --- zarzadzanie TP --- */
  all_runners: true,
  scale_out: false,
  scale_out_pct: 30,
  scale_out_round: "up",
  scale_out_from: "worst",
  scale_out_last_runner: "runner",
  tp_open_offset: 5,
  tp_detect_price: true,
  tp_detect_signal: true,
  tp_freeze_after_ladder: false,
  tp_hit_fill_stages: false,
  spp_max_age_h: 12,

  /* --- trailing --- */
  smart_sl: false,
  breakeven_protection: false,
  trail_after_tp2: false,
  runner_trail: true,
  runner_trail_start: 25,
  runner_trail_gap: 20,
  trail_mode: "gap",
  trail_lock_pct: 50,
  trail_tiers: "5:1,10:5,15:9,20:14,30:23,50:42",
  trail_split: false,
  trail_runners_n: 1,
  trail_runner_mode: "tiered",
  trail_runner_start: 5,
  trail_runner_lock_pct: 50,
  trail_runner_gap: 8,
  trail_runner_tiers: "5:1,10:4,20:12,35:26,60:50,100:88",
  /* trailing S/R po strukturze 1M — wyłączony (kontrakt zera co do centa) */
  trail_sr_enabled: false,
  sr_warmup_exact_ticks: false,
  trail_sr_scope: "Runner",
  trail_sr_activation: "Tp2",
  trail_sr_min_gain: 3,
  trail_sr_min_dist_price: 6,
  /* EA-CORE — WSZYSTKO ZEROWE. To nie jest ostroznosc, tylko warunek bramki
     akceptacji FALI 0: przy tych wartosciach warstwa nie jest wolana ani razu,
     a gdyby ktos wlaczyl sam `ea_enabled`, stan zostaje Neutral na zawsze. */
  ea_enabled: false,
  ea_tick_s: 0,
  ea_state_src: "FloatR",
  ea_defense_enter: 0,
  ea_defense_exit: 0,
  ea_offense_enter: 0,
  ea_offense_exit: 0,
  ea_state_dwell_s: 0,
  ea_state_ratchet: "NieLuzujWKoszyku",
  ea_state_journal: true,
  ea_dozor_sl: false,
  harvest: false,
  harvest_start: 8,
  harvest_retrace_pct: 40,
  trail_min_dist: 0,
  ladder_from_tp: 0,
  ladder_lag: 0,
  ladder_offset: 0,

  /* --- wirtualny SL --- */
  virtual_sl: false,
  virtual_sl_only_when_rejected: true,
  vsl_eval_s: 0,
  virtual_sl_all: false,
  vsl_net_off: 0,

  /* --- filozofia ATFX --- */
  be_lock: false,
  be_lock_points: 2,
  be_at_tp1: false,
  reenter_after_tp: false,
  reenter_min_tp_stage: 0,
  oae_timeout_min: 0,
  oae_profit_min: 0.5,
  ignore_out_at_entry: false,
  ignore_risk_free: false,

  /* --- oficjalny system --- */
  official_mode: false,
  official_pct_tp1: 15,
  official_pct_tp2: 30,
  official_pct_tp3: 30,
  official_pct_spp: 20,
  official_use_counts: false,
  official_counts: "1,2,1",
  official_spp: false,
  official_round: "nearest",
  official_assign_tps: false,
  official_close_last: false,
  spp_keep_tp: false,
  partial_close: false,
  partial_min_lot: 0.02,
  partial_pct_od_pierwotnego: false,
  cele_na_ostatnim: false,
  retarget_respects_final_target: false,
  sl_polowa_od_konca: 0,
  sl_polowa_ulamek: 0.5,

  /* --- risk free --- */
  risk_free_runners: 1,
  risk_free_mode: "scale_out",
  risk_free_smart_sl: false,
  sl_hit_verify_tol: 0,

  /* --- stagnacja --- */
  stale_take_min: 0,
  stale_take_profit: 15,
  stale_take_min2: 0,
  stale_take_profit2: 35,
  rev_exit_range: 0,
  rev_exit_slope: 14,
  rev_exit_profit: 4,

  /* --- reguly doswiadczonego tradera — wszystkie domyslnie WYLACZONE --- */
  exit_min_hold_min: 0,
  exit_min_profit: 0,
  exit_r_multiple: 0,
  basket_target_usd: 0,
  exit_round_dist: 0,
  exit_round_step: 10,
  exit_spread_mult: 0,
  exit_on_opposite_signal: false,
  hold_after_tp_hit_min: 0,
  toucher_tp_one_based: false,
  pending_drop_arm: false,
  ml_licz_wiszace: false,
  ml_min_wejscie: 0,
  ml_min_warstwa: 0,
  ml_min_reentry: 0,
  ml_min_rearm: 0,
  ml_min_piramida: 0,
  ml_min_fast_addon: 0,
  ml_min_relot_up: 0,
  ml_min_drabina: 0,
  konto_dzwignia: 0,
  wiek_od_wypelnienia: false,
  pending_drop_grace_min: 0,
  pending_drop_grace_max_dist: 0,
  pending_drop_keep_n: 0,
  grid_anchor_absolute: false,
  /* G1: true = zachowanie sprzed rozdzielenia pol (kontrakt parytetu). */
  units_per_level: true,
  tp_open_extra: false,
  /* Te dwa istnialy w silniku i w dokumencie ustawien serwera, ale nie mialy
     odpowiednika w typie `Settings` po stronie panelu — a wiec ani kontrolki,
     ani mozliwosci podejrzenia. Wartosci domyslne jak w `conduit_core`. */
  pending_drop_on_target: true,
  smart_sl_floor_be_after_rf: true,

  /* --- madre wyjscie (smart exit) — wartosci 1:1 z conduit_core::Settings --- */
  smart_exit: false,
  smart_exit_take: 0,
  smart_exit_giveback: 0.3,
  smart_exit_min_peak: 4,
  smart_exit_drop_speed: 0,
  smart_exit_speed_window_s: 60,
  smart_exit_hold_if_pending: 2,
  smart_exit_min_pendings: 1,
  smart_exit_pending_scope: "SameBasket",
  smart_exit_pending_min_dist: 0.3,

  /* --- rezim zmiennosci --- */
  vol_window_min: 0,
  vol_range_usd: 15,
  vol_units_mult: 0.7,

  /* --- ochrona kapitalu --- */
  max_dd_pct: 0,
  max_dd_usd: 0,
  /* Hamulec miekki (dlawik): zmniejsza NOWY koszyk zamiast zamykac wszystko.
     Domyslnie wylaczony — zaden istniejacy preset nie zmienia zachowania. */
  max_portfolio_risk_pct: 0,
  dd_soft_pct: 0,
  dd_soft_mult: 0.5,
  dd_hard_pct: 0,
  dd_hard_mult: 0.25,
  /* Ostrzezenie mailem dziala TEZ przy wylaczonym strazniku (0/0) — i wlasnie
     wtedy jest jedyna informacja, ze cos sie dzieje z kontem w nocy. */
  alert_dd_pct: 15,
  
  signal_max_age_min: 5,
  max_open_positions: 0,
  exposure_count_pendings: true,
  lot_scale_step: 0,
  day_target_usd: 0,
  day_target_close: false,
  day_target_scale_lot: false,
  day_trail_stop_usd: 0,
  usd_scale_with_lot: false,
  session_filter: false,
  session_hours: "7-20",
  streak_pause_n: 0,
  streak_pause_min: 60,
  /* --- hamulec SL-HIT (Pakiet F2); 0/0/0 = wyłączony, kontrakt parytetu --- */
  slhit_pause_n: 0,
  slhit_pause_min: 0,
  slhit_pause_lot_mult: 0,
  eod_flat_hour: 0,
  flat_weekend: false,
  flat_weekend_hour: 20,
  day_flat_broker_clock: false,
  signal_filter: false,
  skip_tags: "",
  require_tags: "",

  /* --- audyt / parytet --- */
  /* --- wagi glebokosci i limit ryzyka koszyka ---
     Wartosci musza sie zgadzac z Settings::default() w
     crates/core/src/settings.rs — rozjazd oznacza, ze panel pokazuje co
     innego, niz robi silnik. */
  entry_weights: "",
  risk_per_basket_pct: 0,
  lot_min: 0.01,
  lot_max: 100,

  
  odlicz_kredyt: false,
  credit_balance_separate: false,
  kredyt_reczny: 0,

  /* --- bramki wejscia --- */
  skip_if_sl_breached: true,
  max_chase_beyond_zone: 0,
  sl_max_dist: 0,
  side_filter: "both",
  regime_filter: "off",
  regime_ma_hours: 72,
  max_open_baskets: 0,
  max_directional_lots: 0,
  equity_floor_pct: 0,
  dd_guard_scope: "daily",

  /* --- siatka --- */
  market_entry_step: 1,
  pending_ttl_from_basket: true,

  /* --- zrodlo wiedzy o trafionym celu --- */
  tp_source: "Either",
  tp_price_tolerance: 0.3,
  tp_price_front_run_usd: 0,
  tp_signal_max_lead_s: 0,
  tp_signal_max_lag_s: 0,
  tp_stage_from_broker_fill: true,

  /* --- reakcje na komunikaty --- */
  out_at_entry_mode: "close_all",
  oae_band_pts: 1,
  sl_hit_mode: "cancel_pendings",
  honor_cancel: true,
  honor_close_all: true,
  
  close_all_scope: "Global",
  
  partials_wykonuj: false,
  partials_pct: 0,
  honor_market_open: false,
  dedup_edited_signals: true,
  /* --- Pakiet A: osie dedupu i edycji (domyślne = kontrakt parytetu) --- */
  dedup_pelny_status: true,
  edycja_wykonuje_reszte_akcji: false,
  dedup_klucz_z_wartoscia: false,
  edycja_sieroty_nie_otwiera: false,
  entry_idempotencja: true,
  dedup_management_po_restarcie: false,
  profit_update_telemetry_only: false,
  tp_price_only_strict: false,
  /* --- Pakiet B: osie z audytu TYLER (domyślne = kontrakt parytetu) --- */
  rf_wymaga_wykonania: false,
  market_entry_units: 0,
  market_hybrid_now_units: 0,
  market_hybrid_pending_units: 0,
  market_hybrid_lot_mult: 1,
  market_hybrid_max_chase_usd: 0,
  market_hybrid_tp_stage: 0,
  market_unfilled_cancel_stage: 0,
  pending_cancel_on_riskfree: false,
  bank_all_at_stage: 0,
  /* --- Pakiet E: statystyki (0 = tylko dokładne zero jest remisem) --- */
  stat_be_prog_usd: 0,
  
  reply_veto: false,
  risk_free_runner_target: "last",
  risk_free_trail: true,
  /* M4: 0 = jak dotad; prog zysku dla ruchu stopu na BE po RISK FREE */
  risk_free_be_min_profit: 0,

  /* --- stopy --- */
  be_offset: 0,
  
  be_covers_late_fills: false,
  be_never_loosen: false,
  sltp_retry_s: 3,

  /* --- wyjscia --- */
  rev_exit_window_min: 60,
  reenter_max: 0,

  /* --- model wykonania --- */
  commission_per_lot: 0,
  exec_latency_ms: 250,
  slippage_pts: 0,
  server_tz_offset_h: 3,
  msg_clock_offset_h: null,

  sim_clock_strict: false,
  spp_refresh_after_modify: false,
  sim_stops_level: 0.2,

  /* --- AI --- */
  ai_mode: false,
  ai_model: "BETAZERO_v3",
  ai_decision_interval_s: 2,

  /* --- nadzor nad terminalem MT5 ---
     Wartosci odtwarzaja zachowanie bot.py (10 prob co 5 s, potem
     1/3/15/30 min...), z jedna poprawka: pierwsza proba nie restartuje
     dzialajacego terminala. */
  mt5_autostart: true,
  mt5_watchdog: true,
  close_receipt_reconcile: false,
  closed_profit_net_costs: false,
  restore_strategy_continuation: false,
  order_volume_contract_v2: false,
  mt5_terminal_path: "",
  mt5_retry_attempts: 10,
  mt5_retry_delay_s: 5,
  mt5_restart_after: 1,
  mt5_health_interval_s: 5,

  
  mt5_symbol: "XAUUSD",
  // Runtime connection policy, not a strategy knob. OFF preserves fixed login.
  mt5_follow_terminal_account: false,
  // Follow-terminal mode never enables real-money execution implicitly.
  mt5_allow_real_account: false,
  mt5_magic: 770077,
  mt5_python: "",
  mt5_deviation_points: 30,
  mt5_login: 0,
  mt5_server: "",
  mt5_password: "",
  przerwa_dobowa_od_h: 0,
  przerwa_dobowa_do_h: 1.083,
  puls_h: 6,
  alllogs_dir: "",

  /* --- ogolne / panel --- */
  one_click: true,
  display_currency: "MT5",
  allow_mt5_modify: false,
  poll_ms: 1000,
  price_tol: 0.6,
  show_positions_on_chart: true,
  show_potential_tpsl: false,
  exclude_pending_potential: false,
  comment_mode: "source",
  comment_custom: "",
  comment_include_topic: true,
  price_log: true,
  price_log_interval_s: 10,

  /* --- logi --- */
  merge_config: {
    commands: true,
    events: true,
    messages: true,
    signals: true,
    trades: true,
    unpredicted_signals: true,
    signal_formats: true,
    backup_memory: true,
    poll_interval: true,
    
    update_performance: false,
    price_log: true,
    session_string: false,
    smtp: false,
    
    journal: true,
    kronika: true,
    wiadomosci: true,
    /* Lustro tekstowe dziennika: TE SAME zdarzenia co `journal`, tylko dla
       oka. Wlaczone, bo wymaganie brzmi „wszystko", ale jesli plik ma byc
       dwa razy mniejszy, to jest pierwsze pole do odznaczenia. */
    journal_log: true,
    koszyki: true,
    konfiguracja: true,
    presety: true,
    mail_queue: true,
    lab: true,
  },
  merge_chronological: true,

  /* --- dziennik zdarzen (JSON Lines) ---
     Wartosci musza sie zgadzac z Settings::default() w
     crates/core/src/settings.rs. Domyslnie WLACZONY: zdarzen z przeszlosci
     nie da sie odtworzyc, wiec wylaczony dziennik to bezpowrotna strata. */
  journal_enabled: true,
  journal_min_level: "info",
  journal_snapshots: true,
  journal_excursions: true,
  journal_text_mirror: true,
  journal_retention_days: 90,
  journal_buffer_cap: 20000,
  /* Archiwum wiadomosci trzymamy DLUZEJ niz dziennik zdarzen: surowy tekst
     wazy ulamek tego, co zdarzenia silnika, a jest JEDYNA kopia tresci kanalu
     (eksport z Telegrama zwija edycje i gubi wiadomosci skasowane). */
  archive_retention_days: 365,
  
  riskfree_enabled: false,
  riskfree_trigger_usd: 0,
  riskfree_trigger_r: 0,
  riskfree_keep_units: 1,
  riskfree_be_offset: 0,
  riskfree_runner_target: "LastTp",
  riskfree_runner_stop: "Be",
  riskfree_runner_gap: 12,
  riskfree_runner_max_hold_min: 90,
  /* M15: false = kontrakt zera; true = limit obejmuje takze koszyki
     uwolnione KOMUNIKATEM kanalu, nie tylko regula silnika */
  runner_max_hold_bez_reguly: false,
  basket_max_age_min: 0,

  
  
  ai_replaces_management: true,
  swap_enabled: true,
  swap_long_points: -75.82,
  swap_short_points: 27.41,
  swap_point_value: 1.0,
  
  swap_rollover_weekday: 3,
  swap_rollover_mult: 3,
  /* --- Pakiet D1/D1b: wyłączone = liczby zgodne z archiwum --- */
  swap_pomijaj_weekend: false,
  swap_rollover_z_serwera: false,
  swap_rollover3days_mt5: 3,
  /* --- Pakiet D5/D6/D4 + D3: wyłączone = liczby zgodne z archiwum --- */
  runner_ksiegowanie_v2: false,
  msg_kurs_sprzed_luki: false,
  slippage_pending_pts: 0,
  stop_out_level_pct: 20,
  margin_call_level_pct: 50,
  entry_depth_curve: 1.0,
  /* W30: opcjonalna warstwa allowance przed strefą. Dwa pola, bo kwota
     mowi GDZIE, a jednostki ILE; zero w ktorymkolwiek = warstwy nie ma. */
  entry_allowance_usd: 0,
  entry_allowance_units: 0,

  /* Filtr trendu wyższego rzędu. Domyślnie WYŁĄCZONY, a tryb domyślny to
     `Shrink`, nie `Block`: twarda blokada przeciw dominującemu kierunkowi
     sygnałów może usunąć większość handlu zamiast zmniejszyć ekspozycję. */
  trend_filter_enabled: false,
  trend_filter_window_h: 24,
  trend_filter_drop_pct: 0,
  trend_filter_mode: "Shrink",
  trend_filter_shrink: 0.5,

  /* ===== poprawki istniejacych regul ===== */
  pending_drop_require_zone_touch: false,
  trail_runners_by_depth: false,

  /* ===== wielkosc pozycji wg jakosci szczebla ===== */
  entry_weights_from_rr: false,
  entry_weights_rr_power: 1,
  entry_weights_rr_cap: 4,

  /* ===== parametry liczone z SYGNALU ===== */
  adaptive_params: false,
  sl_min_dist_zone_mult: 0,
  sl_min_dist_atr_mult: 0,
  sl_min_dist_floor: 0,
  sl_min_dist_cap: 0,
  entry_deep_zone_mult: 0,
  entry_units_zone_ref: 0,
  adaptive_atr_window_min: 60,
  units_by_hour: "",

  /* ===== skalowanie po zdarzeniu ===== */
  rearm_grid_on_return: false,
  basket_realized_broker_only: false,
  confirmed_exit_retry: false,
  defer_entry_until_receipts: false,
  entry_edit_geometry_v2: false,
  deferred_entry_max_age_s: 300,
  rearm_keep_empty_alive: false,
  rearm_block_after_secured: false,
  spp_blocks_rearm_when_flat: false,
  rearm_min_basket_profit: 0,
  rearm_max_times: 1,
  rearm_min_gap_min: 15,

  /* ===== konto ===== */
  day_target_pct: 0,
  day_trail_stop_pct: 0,
  day_trail_arm_pct: 0,

  /* ===== budzet transakcji ===== */
  daily_signal_budget: 0,
  signal_min_rr: 0,
  signal_min_zone_width: 0,
  signal_max_zone_width: 0,

  /* ===== laczenie koszykow ===== */
  merge_same_side: false,
  merge_window_min: 20,
  merge_min_overlap: 0.5,

  /* ===== wyjscie limitem ===== */
  exit_via_limit: false,
  exit_limit_offset: 0,
  exit_limit_wait_s: 60,
  exit_limit_min_profit: 0,
};

/** Kolejnosc i etykiety checkboxow LOGI.
 *
 *  `grupa` porzadkuje liste w panelu: najpierw STRUMIENIE Z DYSKU (to, czego
 *  brakowalo i po co ten plik w ogole istnieje), potem log panelu z podzialem
 *  na kategorie, potem stan i konfiguracja, na koncu dane wrazliwe.
 *
 *  `martwe: true` znaczy „ta wersja silnika tego NIE PRODUKUJE" — pole
 *  zostaje widoczne (jest tez filtrem kategorii dziennika i wystepuje
 *  w starych `settings.json`), ale jest wyszarzone i podpisane, zamiast
 *  udawac, ze cos przelacza. Pozostalosc po `bot.py`.
 */
export const MERGE_KEYS: {
  key: LogCategory;
  label: string;
  sensitive?: boolean;
  martwe?: boolean;
  grupa?: string;
  note?: string;
}[] = [
  /* --- strumienie z dysku: caly material do odtworzenia przebiegu --- */
  { key: "journal", label: "dziennik decyzji", grupa: "Strumienie z dysku", note: "logs/journal/*.jsonl — CO bot zrobił i dlaczego" },
  { key: "kronika", label: "kronika", grupa: "Strumienie z dysku", note: "cały strumień z Telegrama, przed filtrem kanałów" },
  { key: "wiadomosci", label: "archiwum wiadomości", grupa: "Strumienie z dysku", note: "logs/wiadomosci/*.jsonl — z edycjami i skasowaniami" },
  { key: "journal_log", label: "dziennik (lustro .log)", grupa: "Strumienie z dysku", note: "te same zdarzenia dla oka — DUBLUJE rozmiar pliku" },

  /* --- log panelu, po kategoriach --- */
  { key: "commands", label: "commands", grupa: "Dziennik panelu", note: "komendy wykonane przez bota" },
  { key: "events", label: "events", grupa: "Dziennik panelu", note: "zdarzenia cyklu życia + kategorie bez własnego pola" },
  { key: "messages", label: "messages", grupa: "Dziennik panelu", note: "wiadomości w pamięci procesu (po restarcie puste)" },
  { key: "signals", label: "signals", grupa: "Dziennik panelu", note: "rozpoznane sygnały" },
  { key: "trades", label: "trades", grupa: "Dziennik panelu", note: "historia pozycji i zleceń" },
  { key: "unpredicted_signals", label: "unpredicted_signals", grupa: "Dziennik panelu", note: "wiadomości bez dopasowania" },

  /* --- stan i konfiguracja na dysku --- */
  { key: "backup_memory", label: "backup_memory", grupa: "Stan i konfiguracja", note: "migawki stanu: ostatnia w całości + spis reszty" },
  { key: "koszyki", label: "koszyki.json", grupa: "Stan i konfiguracja", note: "zrzut koszyków silnika" },
  { key: "signal_formats", label: "łańcuchy i formaty", grupa: "Stan i konfiguracja", note: "lancuchy.json — przypisanie format → preset" },
  { key: "konfiguracja", label: "konfiguracja", grupa: "Stan i konfiguracja", note: "settings.json, channels.json, demo.json, kronika.json" },
  { key: "presety", label: "presety (spis)", grupa: "Stan i konfiguracja", note: "biblioteka presetów — nazwy i rozmiary" },
  { key: "mail_queue", label: "kolejka poczty", grupa: "Stan i konfiguracja", note: "alerty, których nie udało się wysłać" },
  { key: "lab", label: "laboratorium (spis)", grupa: "Stan i konfiguracja", note: "wyniki backtestów i treningu" },

  /* --- pozostalosci po bot.py: silnik tego nie produkuje --- */
  { key: "poll_interval", label: "poll_interval", grupa: "Nieprodukowane w tej wersji", martwe: true, note: "bot.py zapisywał zmiany częstotliwości pętli — ten silnik nie" },
  { key: "update_performance", label: "update_performance", grupa: "Nieprodukowane w tej wersji", martwe: true, note: "czas pętli w ms — pozostałość po bot.py" },
  { key: "price_log", label: "XAUUSD_price", grupa: "Nieprodukowane w tej wersji", martwe: true, note: "pliku ceny ten silnik nie pisze (przełącznik niżej też jest po bot.py)" },

  /* --- wrazliwe: domyslnie ODZNACZONE --- */
  { key: "smtp", label: "smtp", grupa: "Dane wrażliwe", sensitive: true, note: "smtp.json — login i serwer nadawcy (hasło NIGDY)" },
  { key: "session_string", label: "session_string", grupa: "Dane wrażliwe", sensitive: true, note: "sam FAKT istnienia poświadczeń — klucz sesji nie wychodzi nigdy" },
];

export const CURRENCIES = [
  { value: "OFF", label: "bez waluty (zwykłe liczby)" },
  { value: "MT5", label: "z MT5 (natywna konta)" },
  { value: "PLN", label: "PLN — zł" },
  { value: "USD", label: "USD — $" },
  { value: "EUR", label: "EUR — €" },
  { value: "GBP", label: "GBP — £" },
  { value: "CHF", label: "CHF — Fr" },
  { value: "JPY", label: "JPY — ¥" },
  { value: "CZK", label: "CZK — Kč" },
  { value: "SEK", label: "SEK — kr" },
  { value: "NOK", label: "NOK — kr" },
  { value: "CAD", label: "CAD — C$" },
  { value: "AUD", label: "AUD — A$" },
];

/** Kursy do przeliczania waluty panelu (mock — w bot.py pobierane z internetu). */
export const FX_RATES: Record<string, { rate: number; symbol: string }> = {
  MT5: { rate: 1, symbol: "$" },
  USD: { rate: 1, symbol: "$" },
  PLN: { rate: 3.94, symbol: "zł" },
  EUR: { rate: 0.92, symbol: "€" },
  GBP: { rate: 0.78, symbol: "£" },
  CHF: { rate: 0.88, symbol: "Fr" },
  JPY: { rate: 151.4, symbol: "¥" },
  CZK: { rate: 23.1, symbol: "Kč" },
  SEK: { rate: 10.6, symbol: "kr" },
  NOK: { rate: 10.9, symbol: "kr" },
  CAD: { rate: 1.36, symbol: "C$" },
  AUD: { rate: 1.52, symbol: "A$" },
  OFF: { rate: 1, symbol: "" },
};
