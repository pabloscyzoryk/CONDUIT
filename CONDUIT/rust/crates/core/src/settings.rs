
use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskFreeMode {
    CloseAllKeepNearest,
    CloseAllKeepBest,
    CloseProfitableOnly,
    MoveSlToBeOnly,
    CloseEverything,
    Ignore,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DayTrailBasis {
    /// Legacy: allowable drawdown is a percentage of the entire equity peak.
    #[default]
    EquityPeak,
    /// Allowable drawdown is a percentage of profit earned above day start.
    ProfitPeak,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskFreeRunnerTarget {
    KeepTp,
    LastTp,
    NoTpTrailOnly,
    NextTp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SppSlMode {
    Off,
    Stop,
    OnlyIfBetter,
    RunnersOnly,
    RunnersOnlyIfBetter,
    BankersOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutAtEntryMode {
    CloseAll,
    CloseLosersOnly,
    CloseFlatOnly,
    MoveSlToBe,
    Ignore,
}

fn oae_pod_woda_domyslna() -> OaePodWoda {
    OaePodWoda::NicNieRob
}

fn runner_krok_domyslny() -> f64 {
    10.0
}
fn sr_tf_min_domyslne() -> u32 {
    1
}
fn sr_fractal_domyslny() -> u32 {
    3
}
fn sr_offset_domyslny() -> f64 {
    0.5
}
fn sr_min_dist_tp_domyslny() -> f64 {
    2.0
}
fn sr_okno_h_domyslne() -> u32 {
    24
}
fn sr_atr_period_domyslny() -> u32 {
    14
}
fn trail_adaptive_runners_only_default() -> bool {
    true
}
fn trail_adaptive_window_s_default() -> f64 {
    90.0
}
fn trail_adaptive_min_samples_default() -> u32 {
    8
}
fn trail_adaptive_trend_er_default() -> f64 {
    0.55
}
fn trail_adaptive_reversal_er_default() -> f64 {
    0.45
}
fn trail_adaptive_trend_gap_mult_default() -> f64 {
    1.60
}
fn trail_adaptive_chop_gap_mult_default() -> f64 {
    0.85
}
fn trail_adaptive_reversal_gap_mult_default() -> f64 {
    0.45
}
fn trail_adaptive_fast_vol_s_default() -> f64 {
    20.0
}
fn trail_adaptive_slow_vol_s_default() -> f64 {
    120.0
}
fn trail_adaptive_vol_ratio_default() -> f64 {
    1.80
}
fn trail_adaptive_vol_favorable_mult_default() -> f64 {
    1.25
}
fn trail_adaptive_vol_adverse_mult_default() -> f64 {
    0.65
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OaePodWoda {
    NicNieRob,
    Zamknij,
    DociagnijStop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlHitMode {
    CancelPendings,
    CloseAll,
    VerifyByPrice,
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingLifetime {
    UntilTp1,
    UntilTp2,
    UntilTp3,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TpSchedule {
    AllRunners,
    ScaleOutPct,
    OfficialPct,
    OfficialCounts,
    Ladder,
    AllAtTp1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrailMode {
    Gap,
    LockPct,
    Tiered,
    Atr,
    Chandelier,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrailSrScope {
    Runner,
    Tp3Up,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrailSrActivation {
    Entry,
    Gain,
    Tp1,
    Tp2,
    Tp3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EaStateSrc {
    FloatR,
    FloatPctEquity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EaRatchet {
    NieLuzujWKoszyku,
    Swobodny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EaStanDnia {
    Off,
    TylkoInkaso,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloseAllScope {
    Global,
    Basket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZoneOffsetMode {
    None,
    Price,
    Directional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingCrossPolicy {
    Market,
    Stop,
    Shift,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketEntryMode {
    GridAtOnce,
    Single,
    Laddered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskFreeRunnerStop {
    Be,
    BeOwn,
    TrailGap,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PodstawaLota {
    Balance,
    Equity,
    MinOfBoth,
}

fn podstawa_lota_domyslna() -> PodstawaLota {
    PodstawaLota::Balance
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrendFilterMode {
    Block,
    Shrink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingScope {
    SameBasket,
    AnyBasket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SideFilter {
    Both,
    BuyOnly,
    SellOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DdGuardScope {
    Daily,
    Lifetime,
    LifetimePeakDailyReset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TpSource {
    PriceOnly,
    SignalOnly,
    Either,
    SignalConfirmedByPrice,
    PriceFirstSignalWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegimeCena {
    Rynkowa,
    Wejscia,
    Obie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegimeMiara {
    Srednia,
    Mediana,
    Kanal,
    Wykladnicza,
    Percentyl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegimeGdyRozerwany {
    #[serde(alias = "Pass")]
    Milcz,
    KrotkieOkno,
    #[serde(alias = "Soft")]
    Miekko,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegimeFilter {
    Off,
    TrendMa,
    CounterMa,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VolSizeMode {
    Off,
    Target,
    Percentile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BankRounding {
    Nearest,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BankFrom {
    Worst,
    Best,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LastRunner {
    Runner,
    NextTp,
    NoTp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SmartSlMode {
    Off,
    Ladder,
    BreakevenOnly,
    LadderWithBe,
}


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Autonomous policy; independent of legacy AUTO-EA and off in old presets.
    pub t100: crate::t100::Config,
    pub lot_mode_percent: bool,
    pub lot_fixed: f64,
    pub lot_percent: f64,
    pub lot_scale_step: f64,
    pub lot_max: f64,
    pub lot_min: f64,
    pub order_volume_contract_v2: bool,

    pub zone_offset_mode: ZoneOffsetMode,
    pub entry_hi_offset: f64,
    pub entry_lo_offset: f64,
    pub entry_deep_offset: f64,
    #[serde(default)]
    pub entry_deep_frac_to_sl: f64,
    pub entry_tol_offset: f64,
    pub only_limit_signals: bool,
    pub auto_limit: bool,
    pub ignore_old_after_min: f64,
    pub skip_if_sl_breached: bool,
    pub max_chase_beyond_zone: f64,

    pub entry_units: u32,
    pub entry_units_limit: u32,
    pub ppm: f64,
    pub ppm_enabled: bool,
    pub entry_risk_budget: f64,
    pub entry_tp1_budget: f64,
    pub entry_weights: String,
    pub entry_uklad: String,
    pub entry_uklad_kotwica: String,
    pub entry_krzywa_kotwica: String,
    pub tp_drabinka_kotwica: String,
    pub entry_depth_curve: f64,

    #[serde(default)]
    pub entry_allowance_usd: f64,
    #[serde(default)]
    pub entry_allowance_units: u32,

    pub risk_per_basket_pct: f64,
    pub toucher_units: u32,
    pub toucher_tp_index: usize,
    pub toucher_tp_one_based: bool,
    pub toucher_bands: String,
    pub pending_lifetime: PendingLifetime,
    /// Explicit source LIMIT/STOP stays valid until source cancellation; risk guards still apply.
    #[serde(default)]
    pub explicit_pending_until_cancel: bool,
    pub pending_drop_on_target: bool,
    pub pending_drop_arm: bool,
    pub pending_ttl_h: f64,
    pub pending_ttl_from_basket: bool,
    pub ppm_for_limits: bool,
    pub grid_anchor_absolute: bool,
    #[serde(default = "default_units_per_level")]
    pub units_per_level: bool,
    #[serde(default = "default_units_per_level_zone")]
    pub units_per_level_zone: bool,
    pub pending_cross_policy: PendingCrossPolicy,
    pub ppm_for_market: bool,
    pub market_entry_step: f64,
    pub market_entry_mode: MarketEntryMode,

    pub vol_window_min: f64,
    pub vol_range_usd: f64,
    pub vol_units_mult: f64,
    pub pending_resize_on_vol: bool,
    pub pending_resize_s: f64,
    #[serde(default)]
    pub pending_relot_on_balance: bool,
    #[serde(default)]
    pub pending_relot_reconcile_target: bool,
    #[serde(default)]
    pub pending_relot_topup: bool,
    #[serde(default = "prawda")]
    pub pending_relot_up: bool,
    #[serde(default = "prawda")]
    pub pending_relot_down: bool,
    #[serde(default)]
    pub pending_relot_up_od_salda: f64,
    #[serde(default = "prawda")]
    pub pending_relot_wg_planu: bool,

    #[serde(default)]
    pub expo_cap_pct: f64,
    #[serde(default)]
    pub expo_cap_close: bool,
    #[serde(default)]
    pub expo_cap_s: f64,
    #[serde(default)]
    pub expo_cap_ml_pct: f64,

    #[serde(default)]
    pub ml_licz_wiszace: bool,
    #[serde(default)]
    pub ml_min_wejscie: f64,
    #[serde(default)]
    pub ml_min_warstwa: f64,
    #[serde(default)]
    pub ml_min_reentry: f64,
    #[serde(default)]
    pub ml_min_rearm: f64,
    #[serde(default)]
    pub ml_min_piramida: f64,
    #[serde(default)]
    pub ml_min_fast_addon: f64,
    #[serde(default)]
    pub ml_min_relot_up: f64,
    #[serde(default)]
    pub ml_min_drabina: f64,
    #[serde(default)]
    pub konto_dzwignia: f64,
    #[serde(default)]
    pub wiek_od_wypelnienia: bool,
    #[serde(default)]
    pub pending_drop_grace_min: f64,
    pub pending_drop_grace_max_dist: f64,
    pub pending_drop_keep_n: u32,
    #[serde(default = "podstawa_lota_domyslna")]
    pub lot_base: PodstawaLota,

    pub sl_min_dist: f64,
    pub sl_max_dist: f64,
    pub entry_sl_dist_limit: f64,
    pub virtual_sl: bool,
    pub virtual_sl_only_when_rejected: bool,
    pub virtual_sl_all: bool,
    pub vsl_eval_s: f64,
    pub vsl_broker_offset: f64,

    pub tp_schedule: TpSchedule,
    pub scale_out_pct: f64,
    pub official_pct: [f64; 4],
    pub official_counts: String,
    pub official_spp: bool,
    pub assign_tp_per_position: bool,
    pub tp_open_offset: f64,
    pub tp_freeze_after_ladder: bool,
    pub tp_open_extra: bool,
    pub tp_source: TpSource,
    pub tp_price_tolerance: f64,
    pub tp_price_front_run_usd: f64,
    pub tp_signal_max_lead_s: f64,
    pub tp_signal_max_lag_s: f64,
    pub tp_stage_from_broker_fill: bool,
    pub tp_hit_fill_stages: bool,
    pub bank_rounding: BankRounding,
    pub bank_from: BankFrom,
    pub bank_close_last: bool,
    pub last_runner: LastRunner,
    pub partial_close: bool,
    pub partial_min_lot: f64,

    pub partial_pct_od_pierwotnego: bool,

    pub cele_na_ostatnim: bool,

    pub retarget_respects_final_target: bool,

    pub sl_polowa_od_konca: usize,

    pub sl_polowa_ulamek: f64,
    pub spp_max_age_h: f64,
    pub spp_keep_tp: bool,
    pub spp_sl_mode: SppSlMode,
    pub spp_sl_pad: f64,

    pub risk_free_mode: RiskFreeMode,
    pub risk_free_runners: u32,
    pub risk_free_runner_target: RiskFreeRunnerTarget,
    pub risk_free_trail: bool,
    #[serde(default)]
    pub risk_free_be_min_profit: f64,
    pub out_at_entry_mode: OutAtEntryMode,
    #[serde(default = "oae_pod_woda_domyslna")]
    pub oae_pod_woda: OaePodWoda,
    pub oae_band_pts: f64,
    pub sl_hit_mode: SlHitMode,
    pub sl_hit_verify_tol: f64,
    pub honor_cancel: bool,
    pub honor_close_all: bool,
    #[serde(default = "default_close_all_scope")]
    pub close_all_scope: CloseAllScope,
    #[serde(default)]
    pub partials_wykonuj: bool,
    #[serde(default)]
    pub partials_pct: f64,
    #[serde(default)]
    pub parser_luz_interpunkcyjny: bool,
    #[serde(default)]
    pub recap_guard: bool,
    #[serde(default)]
    pub profit_update_telemetry_only: bool,
    pub honor_market_open: bool,
    pub basket_hint_tolerance: f64,
    pub dedup_edited_signals: bool,
    #[serde(default = "default_dedup_pelny_status")]
    pub dedup_pelny_status: bool,
    #[serde(default = "default_edycja_wykonuje_reszte_akcji")]
    pub edycja_wykonuje_reszte_akcji: bool,
    #[serde(default = "default_dedup_klucz_z_wartoscia")]
    pub dedup_klucz_z_wartoscia: bool,
    #[serde(default = "default_edycja_sieroty_nie_otwiera")]
    /// true: legacy block. false: recover only a complete protected Entry at
    /// receive time; orphan MarketOpen stays blocked. Source identity is durable.
    pub edycja_sieroty_nie_otwiera: bool,
    #[serde(default = "default_entry_idempotencja")]
    pub entry_idempotencja: bool,
    #[serde(default)]
    pub entry_edit_geometry_v2: bool,
    #[serde(default)]
    pub sr_warmup_exact_ticks: bool,
    #[serde(default)]
    pub dedup_management_po_restarcie: bool,
    #[serde(default = "default_rf_wymaga_wykonania")]
    pub rf_wymaga_wykonania: bool,
    #[serde(default = "default_market_entry_units")]
    pub market_entry_units: u32,
    #[serde(default = "default_market_hybrid_now_units")]
    pub market_hybrid_now_units: u32,
    #[serde(default = "default_market_hybrid_pending_units")]
    pub market_hybrid_pending_units: u32,
    #[serde(default = "default_market_hybrid_lot_mult")]
    pub market_hybrid_lot_mult: f64,
    #[serde(default = "default_market_hybrid_max_chase_usd")]
    pub market_hybrid_max_chase_usd: f64,
    #[serde(default = "default_market_hybrid_tp_stage")]
    pub market_hybrid_tp_stage: u8,
    #[serde(default = "default_market_unfilled_cancel_stage")]
    pub market_unfilled_cancel_stage: u8,
    #[serde(default = "default_pending_cancel_on_riskfree")]
    pub pending_cancel_on_riskfree: bool,
    #[serde(default = "default_bank_all_at_stage")]
    pub bank_all_at_stage: u8,
    #[serde(default)]
    pub confirmed_exit_retry: bool,
    #[serde(default)]
    pub close_receipt_reconcile: bool,
    #[serde(default)]
    pub defer_entry_until_receipts: bool,
    #[serde(default = "default_deferred_entry_max_age_s")]
    pub deferred_entry_max_age_s: f64,
    #[serde(default = "default_stat_be_prog_usd")]
    pub stat_be_prog_usd: f64,
    pub oae_timeout_min: f64,
    pub oae_profit_min: f64,

    pub oae_skip_after_riskfree: bool,
    pub no_tp_after_stage: u8,
    pub no_reenter_from_stage: u8,
    pub day_gate_od_salda: f64,
    pub day_gate_do_salda: f64,
    pub reenter_stop_after_riskfree: bool,

    pub be_lock_pts: f64,
    pub be_at_tp1: bool,
    #[serde(default)]
    pub be_od_etapu: u8,
    #[serde(default)]
    pub be_min_pozycji: u32,
    #[serde(default)]
    pub cele_pomin_za_cena: bool,
    pub entry_jeden_na_glebokiej: bool,
    pub sl_po_tp1_na_krawedz: bool,
    pub sl_wlasny_na_pozycje: f64,
    pub be_offset: f64,
    pub be_never_loosen: bool,
    #[serde(default)]
    pub be_covers_late_fills: bool,
    pub trail_mode: TrailMode,
    pub trail_start: f64,
    pub trail_gap: f64,
    pub trail_lock_pct: f64,
    pub trail_tiers: String,
    pub trail_split: bool,
    pub trail_runners_n: u32,
    pub trail_runner_mode: TrailMode,
    pub trail_runner_start: f64,
    pub trail_runner_gap: f64,
    pub trail_runner_lock_pct: f64,
    pub trail_runner_tiers: String,
    pub trail_min_dist: f64,
    pub ladder_from_tp: usize,
    pub ladder_lag: usize,
    pub ladder_offset: f64,
    pub smart_sl_mode: SmartSlMode,
    pub smart_sl_delay: usize,
    pub smart_sl_only_after_rf: bool,
    pub smart_sl_floor_be_after_rf: bool,
    pub sltp_retry_s: f64,

    #[serde(default = "default_trail_sr_enabled")]
    pub trail_sr_enabled: bool,
    #[serde(default = "default_trail_sr_scope")]
    pub trail_sr_scope: TrailSrScope,
    #[serde(default = "default_trail_sr_activation")]
    pub trail_sr_activation: TrailSrActivation,
    #[serde(default = "default_trail_sr_min_gain")]
    pub trail_sr_min_gain: f64,
    #[serde(default = "default_trail_sr_min_dist_price")]
    pub trail_sr_min_dist_price: f64,

    #[serde(default)]
    pub entry_warstwy_offset: f64,
    #[serde(default)]
    pub entry_warstwy_z_tekstu: bool,
    #[serde(default)]
    pub runner_cele_n: u32,
    #[serde(default = "runner_krok_domyslny")]
    pub runner_cele_krok: f64,
    #[serde(default)]
    pub runner_partial_pct: f64,
    #[serde(default = "sr_tf_min_domyslne")]
    pub trail_sr_tf_min: u32,
    #[serde(default = "sr_fractal_domyslny")]
    pub trail_sr_fractal_n: u32,
    #[serde(default = "sr_offset_domyslny")]
    pub trail_sr_offset: f64,
    #[serde(default = "sr_min_dist_tp_domyslny")]
    pub trail_sr_min_dist_tp: f64,
    #[serde(default = "sr_okno_h_domyslne")]
    pub trail_sr_struct_window_h: u32,
    #[serde(default)]
    pub trail_sr_min_prominence_atr: f64,
    #[serde(default)]
    pub trail_sr_offset_atr_mult: f64,
    #[serde(default)]
    pub trail_sr_offset_spread_mult: f64,
    #[serde(default = "sr_atr_period_domyslny")]
    pub trail_sr_atr_period: u32,

    pub harvest_retrace_pct: f64,
    pub harvest_start: f64,
    pub stale_take_min: f64,
    pub stale_take_profit: f64,
    pub stale_take_min2: f64,
    pub stale_take_profit2: f64,
    pub rev_exit_range: f64,
    pub rev_exit_slope: f64,
    pub rev_exit_profit: f64,
    pub rev_exit_window_min: f64,
    pub reenter_after_tp: bool,
    pub reenter_min_tp_stage: usize,
    pub reenter_max: u32,

    pub session_filter: bool,
    #[serde(default = "sesja_bramka_domyslna")]
    pub sesja_bramka: SesjaBramka,
    pub session_hours: String,
    pub max_open_positions: u32,

    pub enforce_position_limit_on_fill: bool,
    #[serde(default)]
    pub limit_kasuje_tylko_nadmiar: bool,
    pub max_open_baskets: u32,

    pub exposure_bonus_profit_pct: f64,
    pub exposure_bonus_positions: u32,
    pub exposure_bonus_baskets: u32,

    pub exposure_count_pendings: bool,
    pub max_directional_lots: f64,
    pub streak_pause_n: u32,
    pub streak_pause_min: f64,
    pub signal_filter: bool,
    pub side_filter: SideFilter,
    pub skip_tags: String,
    pub require_tags: String,
    pub regime_filter: RegimeFilter,
    pub regime_ma_hours: f64,
    #[serde(default = "regime_cena_domyslna")]
    pub regime_cena: RegimeCena,
    #[serde(default = "regime_miara_domyslna")]
    pub regime_miara: RegimeMiara,
    #[serde(default)]
    pub regime_pilnuj_limitow: bool,
    #[serde(default = "regime_percentyl_domyslny")]
    pub regime_percentyl: f64,
    #[serde(default)]
    pub regime_strefa_martwa: f64,
    #[serde(default)]
    pub regime_okno2_h: f64,
    #[serde(default)]
    pub regime_zmiennosc_min: f64,
    #[serde(default, alias = "regime_range_mute_usd")]
    pub regime_zmiennosc_max: f64,
    #[serde(
        default = "regime_gdy_rozerwany_domyslny",
        alias = "regime_range_mute_mode"
    )]
    pub regime_gdy_rozerwany: RegimeGdyRozerwany,

    #[serde(default)]
    pub regime_soft: bool,
    #[serde(default = "jeden_f64")]
    pub regime_soft_units_mult: f64,
    #[serde(default = "jeden_f64")]
    pub regime_soft_lot_mult: f64,
    #[serde(default)]
    pub regime_soft_max_positions: u32,
    #[serde(default)]
    pub sanity_zone_max: f64,
    #[serde(default)]
    pub sanity_tp_max: f64,
    #[serde(default)]
    pub sanity_tp_rosnace: bool,
    #[serde(default)]
    pub sanity_tp_strona: bool,
    #[serde(default = "jeden_f64")]
    pub regime_soft_risk_mult: f64,
    #[serde(default)]
    pub slhit_pause_n: u32,
    #[serde(default)]
    pub slhit_pause_min: f64,
    #[serde(default)]
    pub slhit_pause_lot_mult: f64,

    pub max_dd_pct: f64,
    pub max_dd_usd: f64,
    pub dd_guard_scope: DdGuardScope,

    /// >0: account-wide live downside cap on every new order, including addons
    /// and manual panel entries; <=0 disables it. Day profit arm is independent.
    pub max_portfolio_risk_pct: f64,
    pub dd_soft_pct: f64,
    pub dd_soft_mult: f64,
    pub dd_hard_pct: f64,
    pub dd_hard_mult: f64,

    pub exit_min_hold_min: f64,
    pub exit_min_profit: f64,
    pub exit_r_multiple: f64,
    pub basket_target_usd: f64,
    pub exit_round_dist: f64,
    pub exit_round_step: f64,
    pub exit_spread_mult: f64,
    pub exit_on_opposite_signal: bool,
    #[serde(default = "cel_z_przeciwnego_domyslny")]
    pub cel_z_przeciwnego: CelZPrzeciwnego,
    #[serde(default)]
    pub cel_z_przeciwnego_zapas: f64,
    pub hold_after_tp_hit_min: f64,

    pub smart_exit: bool,
    pub smart_exit_take: f64,
    pub smart_exit_giveback: f64,
    pub smart_exit_min_peak: f64,
    pub smart_exit_drop_speed: f64,
    pub smart_exit_speed_window_s: f64,
    pub smart_exit_hold_if_pending: f64,
    pub smart_exit_min_pendings: u32,
    pub smart_exit_pending_scope: PendingScope,
    pub smart_exit_pending_min_dist: f64,
    pub day_target_usd: f64,
    pub day_target_close: bool,
    pub day_target_scale_lot: bool,
    pub day_trail_stop_usd: f64,
    pub usd_scale_with_lot: bool,
    pub eod_flat_hour: f64,
    pub flat_weekend: bool,
    pub flat_weekend_hour: f64,
    pub equity_floor_pct: f64,


    pub riskfree_enabled: bool,
    pub riskfree_trigger_usd: f64,
    pub riskfree_trigger_r: f64,
    pub riskfree_keep_units: u32,
    pub riskfree_be_offset: f64,
    pub riskfree_runner_target: RiskFreeRunnerTarget,
    pub riskfree_runner_stop: RiskFreeRunnerStop,
    pub riskfree_runner_gap: f64,
    pub riskfree_runner_max_hold_min: f64,

    pub basket_max_age_min: f64,

    pub fast_fill_reject_s: f64,

    pub fast_fill_layers: u32,

    pub fast_fill_soft_age_min: f64,

    pub zone_exit_adverse_s: f64,

    pub zone_exit_adverse_close: bool,

    pub reenter_min_return_s: f64,

    pub pyramid_after_stage: u32,

    pub pyramid_lot_mult: f64,

    pub pyramid_regime_lookback: u32,

    pub pyramid_regime_max_fast_pct: f64,

    pub fast_addon_move_usd: f64,
    pub fast_addon_window_s: f64,
    pub fast_addon_max: u32,
    pub fast_addon_lot_mult: f64,
    pub fast_addon_min_stage: u32,
    pub fast_addon_cooldown_s: f64,

    pub pyramid_min_equity_mult: f64,

    pub trend_filter_enabled: bool,
    pub trend_filter_window_h: f64,
    pub trend_filter_drop_pct: f64,
    pub trend_filter_mode: TrendFilterMode,
    pub trend_filter_shrink: f64,

    pub pending_drop_require_zone_touch: bool,

    pub trail_runners_by_depth: bool,

    pub entry_weights_from_rr: bool,
    pub entry_weights_rr_power: f64,
    pub entry_weights_rr_cap: f64,

    #[serde(default)]
    pub drop_unplaceable_levels: bool,

    #[serde(default)]
    pub zakaz_ponizej_krawedzi: bool,

    pub adaptive_params: bool,
    pub sl_min_dist_zone_mult: f64,
    pub sl_min_dist_atr_mult: f64,
    pub sl_min_dist_floor: f64,
    pub sl_min_dist_cap: f64,
    pub entry_deep_zone_mult: f64,
    pub entry_units_zone_ref: f64,
    pub adaptive_atr_window_min: f64,
    pub units_by_hour: String,

    pub rearm_grid_on_return: bool,
    pub rearm_keep_empty_alive: bool,
    pub rearm_block_after_secured: bool,
    pub spp_blocks_rearm_when_flat: bool,
    pub rearm_min_basket_profit: f64,
    pub rearm_max_times: u32,
    pub rearm_bez_pozycji: bool,
    pub rearm_bez_pozycji_max_h: f64,
    pub rearm_min_gap_min: f64,

    pub day_target_pct: f64,
    pub day_trail_stop_pct: f64,
    pub day_trail_arm_pct: f64,
    #[serde(default)]
    pub day_trail_basis: DayTrailBasis,
    /// Daily equity-peak reserve for new risk; arm=0 preserves legacy behavior.
    pub profit_budget_arm_pct: f64,
    pub profit_budget_keep_pct: f64,
    pub profit_budget_deploy_pct: f64,

    pub daily_signal_budget: u32,
    pub signal_min_rr: f64,
    pub signal_min_zone_width: f64,
    pub signal_max_zone_width: f64,

    pub merge_same_side: bool,
    pub merge_window_min: f64,
    pub merge_min_overlap: f64,

    pub exit_via_limit: bool,
    pub exit_limit_offset: f64,
    pub exit_limit_wait_s: f64,
    pub exit_limit_min_profit: f64,

    pub stops_level: f64,
    pub commission_per_lot: f64,

    pub swap_enabled: bool,
    pub swap_long_points: f64,
    pub swap_short_points: f64,
    pub swap_point_value: f64,
    pub swap_rollover_weekday: u32,
    pub swap_rollover_mult: f64,
    #[serde(default)]
    pub swap_pomijaj_weekend: bool,
    #[serde(default)]
    pub swap_rollover_z_serwera: bool,
    #[serde(default = "trzy_u32")]
    pub swap_rollover3days_mt5: u32,
    #[serde(default)]
    pub runner_ksiegowanie_v2: bool,
    #[serde(default)]
    pub basket_realized_broker_only: bool,
    #[serde(default)]
    pub closed_profit_net_costs: bool,
    #[serde(default)]
    pub restore_strategy_continuation: bool,
    #[serde(default)]
    pub msg_kurs_sprzed_luki: bool,
    #[serde(default)]
    pub live_tick_order_strict: bool,

    pub slippage_pending_pts: f64,

    pub sim_margin_check_on_fill: bool,

    #[serde(default)]
    pub sim_validate_pending_stops: bool,

    #[serde(default)]
    pub reenter_respect_cap: bool,

    #[serde(default)]
    pub sl_edit_reaches_pendings: bool,

    #[serde(default)]
    pub honor_stop_orders: bool,

    #[serde(default)]
    pub hint_veto: bool,

    #[serde(default)]
    pub reply_veto: bool,

    pub sync_only_live_levels: bool,

    #[serde(default)]
    pub tp_correction_to_broker: bool,

    #[serde(default)]
    pub runner_max_hold_rule_only: bool,

    #[serde(default)]
    pub runner_max_hold_bez_reguly: bool,

    #[serde(default)]
    pub spp_arms_runner_clock: bool,

    #[serde(default)]
    pub tp_hit_match_level: bool,

    #[serde(default)]
    pub tp_unindexed_pips_require_price: bool,

    #[serde(default)]
    pub tp_price_only_strict: bool,

    #[serde(default)]
    pub rf_level_sanity_max_usd: f64,

    #[serde(default)]
    pub reply_graph_transitive: bool,

    pub stop_out_level_pct: f64,
    #[serde(default)]
    pub sim_margin_at_market: bool,
    pub margin_call_level_pct: f64,
    pub server_tz_offset_ms: i64,
    pub msg_clock_offset_ms: Option<i64>,
    pub exec_latency_ms: i64,
    pub slippage_pts: f64,

    pub ai_enabled: bool,
    pub ai_replaces_management: bool,
    pub ai_model: String,
    pub ai_decision_interval_s: f64,

    pub mt5_autostart: bool,
    pub mt5_watchdog: bool,
    pub mt5_terminal_path: String,
    pub mt5_retry_attempts: u32,
    pub mt5_retry_delay_s: f64,
    pub mt5_restart_after: u32,
    pub mt5_health_interval_s: f64,

    pub journal_enabled: bool,
    pub journal_min_level: crate::journal::EventLevel,
    pub journal_snapshots: bool,
    pub journal_excursions: bool,
    pub journal_text_mirror: bool,
    pub journal_retention_days: u32,
    pub journal_buffer_cap: u32,

    pub entry_units_small: u32,
    pub entry_units_small_mult: f64,

    pub risk_per_basket_pct_small: f64,
    pub risk_per_basket_pct_small_mult: f64,

    pub reenter_max_small: u32,
    pub reenter_max_small_mult: f64,

    pub max_open_positions_small: u32,
    pub max_open_positions_small_mult: f64,

    pub max_open_baskets_small: u32,
    pub max_open_baskets_small_mult: f64,

    pub basket_max_age_min_small: f64,
    pub basket_max_age_min_small_mult: f64,

    pub fast_fill_soft_age_min_small: f64,
    pub fast_fill_soft_age_min_small_mult: f64,

    pub market_entry_step_small: f64,
    pub market_entry_step_small_mult: f64,

    pub sl_min_dist_small: f64,
    pub sl_min_dist_small_mult: f64,

    pub lot_percent_small: f64,
    pub lot_percent_small_mult: f64,

    pub lot_max_z_salda: f64,

    pub trail_atr_mult: f64,

    #[serde(default)]
    pub trail_adaptive_enabled: bool,
    #[serde(default = "trail_adaptive_runners_only_default")]
    pub trail_adaptive_runners_only: bool,
    #[serde(default = "trail_adaptive_window_s_default")]
    pub trail_adaptive_window_s: f64,
    #[serde(default = "trail_adaptive_min_samples_default")]
    pub trail_adaptive_min_samples: u32,
    #[serde(default = "trail_adaptive_trend_er_default")]
    pub trail_adaptive_trend_er: f64,
    #[serde(default = "trail_adaptive_reversal_er_default")]
    pub trail_adaptive_reversal_er: f64,
    #[serde(default = "trail_adaptive_trend_gap_mult_default")]
    pub trail_adaptive_trend_gap_mult: f64,
    #[serde(default = "trail_adaptive_chop_gap_mult_default")]
    pub trail_adaptive_chop_gap_mult: f64,
    #[serde(default = "trail_adaptive_reversal_gap_mult_default")]
    pub trail_adaptive_reversal_gap_mult: f64,
    #[serde(default = "trail_adaptive_fast_vol_s_default")]
    pub trail_adaptive_fast_vol_s: f64,
    #[serde(default = "trail_adaptive_slow_vol_s_default")]
    pub trail_adaptive_slow_vol_s: f64,
    #[serde(default = "trail_adaptive_vol_ratio_default")]
    pub trail_adaptive_vol_ratio: f64,
    #[serde(default = "trail_adaptive_vol_favorable_mult_default")]
    pub trail_adaptive_vol_favorable_mult: f64,
    #[serde(default = "trail_adaptive_vol_adverse_mult_default")]
    pub trail_adaptive_vol_adverse_mult: f64,
    #[serde(default)]
    pub trail_adaptive_min_peak: f64,
    #[serde(default)]
    pub trail_adaptive_min_gap: f64,
    #[serde(default)]
    pub trail_adaptive_max_gap: f64,

    pub vol_size_mode: VolSizeMode,

    pub vol_size_target: f64,

    pub vol_size_min_mult: f64,

    pub vol_size_max_mult: f64,

    pub vol_size_percentile_okno: u32,

    pub vol_size_odsezonuj: bool,

    #[serde(default)]
    pub credit_balance_separate: bool,
    pub odlicz_kredyt: bool,
    pub kredyt_reczny: f64,

    pub parser_geometryczny: bool,
    pub parser_min_pewnosc: f64,

    #[serde(default)]
    pub ea_enabled: bool,
    #[serde(default)]
    pub ea_tick_s: f64,
    #[serde(default = "default_ea_state_src")]
    pub ea_state_src: EaStateSrc,
    #[serde(default)]
    pub ea_defense_enter: f64,
    #[serde(default)]
    pub ea_defense_exit: f64,
    #[serde(default)]
    pub ea_offense_enter: f64,
    #[serde(default)]
    pub ea_offense_exit: f64,
    #[serde(default)]
    pub ea_state_dwell_s: f64,
    #[serde(default = "default_ea_state_ratchet")]
    pub ea_state_ratchet: EaRatchet,
    #[serde(default = "default_ea_state_journal")]
    pub ea_state_journal: bool,
    #[serde(default)]
    pub ea_dozor_sl: bool,

    #[serde(default)]
    pub ea_lot_z_wolnego_marginesu: f64,
    #[serde(default)]
    pub ea_stop_dokladek_przy_stracie: f64,
    #[serde(default)]
    pub ea_stop_dokladek_powrot: f64,
    #[serde(default)]
    pub ea_redukcja_przy_zageszczeniu: f64,
    #[serde(default)]
    pub ea_zageszczenie_podloga: f64,
    #[serde(default = "default_ea_stan_dnia")]
    pub ea_stan_dnia: EaStanDnia,
    #[serde(default = "default_ea_stan_dnia_prog_sl")]
    pub ea_stan_dnia_prog_sl: u32,
    #[serde(default = "default_ea_stan_dnia_jednostki_mult")]
    pub ea_stan_dnia_jednostki_mult: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            lot_mode_percent: false,
            lot_fixed: 0.01,
            lot_percent: 1.0,
            lot_scale_step: 0.0,
            lot_max: 100.0,
            lot_min: 0.01,
            order_volume_contract_v2: false,

            zone_offset_mode: ZoneOffsetMode::None,
            entry_hi_offset: 0.0,
            entry_lo_offset: 0.0,
            entry_deep_offset: 4.0,
            entry_deep_frac_to_sl: 0.0,
            entry_tol_offset: 0.3,
            only_limit_signals: false,
            auto_limit: true,
            ignore_old_after_min: 0.0,
            skip_if_sl_breached: true,
            max_chase_beyond_zone: 0.0,

            entry_units: 1,
            entry_units_limit: 0,
            ppm: 1.0,
            ppm_enabled: false,
            entry_risk_budget: 0.0,
            entry_tp1_budget: 0.0,
            entry_depth_curve: 1.0,
            entry_allowance_usd: 0.0,
            entry_allowance_units: 0,
            entry_weights: String::new(),
            entry_uklad: String::new(),
            entry_uklad_kotwica: "Ocalaly".into(),
            entry_krzywa_kotwica: "Ocalaly".into(),
            tp_drabinka_kotwica: "Ocalaly".into(),
            risk_per_basket_pct: 0.0,
            toucher_units: 0,
            toucher_tp_index: 1,
            toucher_tp_one_based: false,
            toucher_bands: String::new(),
            pending_lifetime: PendingLifetime::UntilTp1,
            explicit_pending_until_cancel: false,
            pending_drop_on_target: true,
            pending_drop_arm: false,
            pending_ttl_h: 0.0,
            pending_ttl_from_basket: true,
            ppm_for_limits: true,
            grid_anchor_absolute: false,
            units_per_level: default_units_per_level(),
            units_per_level_zone: default_units_per_level_zone(),
            pending_cross_policy: PendingCrossPolicy::Market,
            ppm_for_market: false,
            market_entry_step: 1.0,
            market_entry_mode: MarketEntryMode::GridAtOnce,

            vol_window_min: 0.0,
            vol_range_usd: 15.0,
            vol_units_mult: 0.7,
            pending_resize_on_vol: false,
            pending_resize_s: 30.0,
            pending_relot_on_balance: false,
            pending_relot_reconcile_target: false,
            pending_relot_topup: false,
            pending_relot_up: true,
            pending_relot_down: true,
            pending_relot_up_od_salda: 0.0,
            pending_relot_wg_planu: true,

            expo_cap_pct: 0.0,
            expo_cap_close: false,
            expo_cap_s: 0.0,
            expo_cap_ml_pct: 0.0,
            ml_licz_wiszace: false,
            ml_min_wejscie: 0.0,
            ml_min_warstwa: 0.0,
            ml_min_reentry: 0.0,
            ml_min_rearm: 0.0,
            ml_min_piramida: 0.0,
            ml_min_fast_addon: 0.0,
            ml_min_relot_up: 0.0,
            ml_min_drabina: 0.0,
            konto_dzwignia: 0.0,
            wiek_od_wypelnienia: false,
            pending_drop_grace_min: 0.0,
            pending_drop_grace_max_dist: 0.0,
            pending_drop_keep_n: 0,
            lot_base: PodstawaLota::Balance,

            sl_min_dist: 0.0,
            sl_max_dist: 0.0,
            entry_sl_dist_limit: 0.0,
            virtual_sl: false,
            virtual_sl_only_when_rejected: true,
            virtual_sl_all: false,
            vsl_eval_s: 0.0,
            vsl_broker_offset: 0.0,

            tp_schedule: TpSchedule::Ladder,
            scale_out_pct: 30.0,
            official_pct: [15.0, 30.0, 30.0, 20.0],
            official_counts: "1,1,1".into(),
            official_spp: false,
            assign_tp_per_position: true,
            tp_open_offset: 5.0,
            tp_freeze_after_ladder: true,
            tp_open_extra: false,
            tp_source: TpSource::Either,
            tp_price_tolerance: 0.30,
            tp_price_front_run_usd: 0.0,
            tp_signal_max_lead_s: 0.0,
            tp_signal_max_lag_s: 0.0,
            tp_stage_from_broker_fill: true,
            tp_hit_fill_stages: true,
            bank_rounding: BankRounding::Up,
            bank_from: BankFrom::Worst,
            bank_close_last: false,
            last_runner: LastRunner::Runner,
            partial_close: false,
            partial_min_lot: 0.02,
            partial_pct_od_pierwotnego: false,
            cele_na_ostatnim: false,
            retarget_respects_final_target: false,
            sl_polowa_od_konca: 0,
            sl_polowa_ulamek: 0.5,
            spp_max_age_h: 12.0,
            spp_keep_tp: false,
            spp_sl_mode: SppSlMode::Off,
            spp_sl_pad: 0.0,

            risk_free_mode: RiskFreeMode::CloseAllKeepNearest,
            risk_free_runners: 1,
            risk_free_runner_target: RiskFreeRunnerTarget::LastTp,
            risk_free_trail: false,
            risk_free_be_min_profit: 0.0,
            out_at_entry_mode: OutAtEntryMode::CloseAll,
            oae_pod_woda: OaePodWoda::NicNieRob,
            oae_band_pts: 1.0,
            sl_hit_mode: SlHitMode::CancelPendings,
            sl_hit_verify_tol: 0.0,
            honor_cancel: true,
            honor_close_all: true,
            close_all_scope: default_close_all_scope(),
            partials_wykonuj: false,
            parser_luz_interpunkcyjny: false,
            recap_guard: false,
            profit_update_telemetry_only: false,
            partials_pct: 0.0,
            honor_market_open: false,
            basket_hint_tolerance: 0.6,
            dedup_edited_signals: true,
            dedup_pelny_status: default_dedup_pelny_status(),
            edycja_wykonuje_reszte_akcji: default_edycja_wykonuje_reszte_akcji(),
            dedup_klucz_z_wartoscia: default_dedup_klucz_z_wartoscia(),
            edycja_sieroty_nie_otwiera: default_edycja_sieroty_nie_otwiera(),
            entry_idempotencja: default_entry_idempotencja(),
            entry_edit_geometry_v2: false,
            sr_warmup_exact_ticks: false,
            dedup_management_po_restarcie: false,
            rf_wymaga_wykonania: default_rf_wymaga_wykonania(),
            market_entry_units: default_market_entry_units(),
            market_hybrid_now_units: default_market_hybrid_now_units(),
            market_hybrid_pending_units: default_market_hybrid_pending_units(),
            market_hybrid_lot_mult: default_market_hybrid_lot_mult(),
            market_hybrid_max_chase_usd: default_market_hybrid_max_chase_usd(),
            market_hybrid_tp_stage: default_market_hybrid_tp_stage(),
            market_unfilled_cancel_stage: default_market_unfilled_cancel_stage(),
            pending_cancel_on_riskfree: default_pending_cancel_on_riskfree(),
            bank_all_at_stage: default_bank_all_at_stage(),
            confirmed_exit_retry: false,
            close_receipt_reconcile: false,
            defer_entry_until_receipts: false,
            deferred_entry_max_age_s: 300.0,
            stat_be_prog_usd: default_stat_be_prog_usd(),
            oae_timeout_min: 0.0,
            oae_profit_min: 0.5,
            oae_skip_after_riskfree: false,
            no_tp_after_stage: 0,
            no_reenter_from_stage: 0,
            day_gate_od_salda: 0.0,
            day_gate_do_salda: 0.0,
            reenter_stop_after_riskfree: false,

            be_lock_pts: 0.0,
            be_at_tp1: false,
            be_od_etapu: 0,
            be_min_pozycji: 0,
            cele_pomin_za_cena: false,
            entry_jeden_na_glebokiej: false,
            sl_po_tp1_na_krawedz: false,
            sl_wlasny_na_pozycje: 0.0,
            be_offset: 0.0,
            be_never_loosen: false,
            be_covers_late_fills: false,
            trail_mode: TrailMode::Off,
            trail_start: 25.0,
            trail_gap: 20.0,
            trail_lock_pct: 50.0,
            trail_tiers: "5:1,10:5,15:9,20:14,30:23,50:42".into(),
            trail_split: false,
            trail_runners_n: 1,
            trail_runner_mode: TrailMode::Tiered,
            trail_runner_start: 5.0,
            trail_runner_gap: 8.0,
            trail_runner_lock_pct: 50.0,
            trail_runner_tiers: "5:1,10:4,20:12,35:26,60:50,100:88".into(),
            trail_min_dist: 0.0,
            ladder_from_tp: 0,
            ladder_lag: 0,
            ladder_offset: 0.0,
            smart_sl_mode: SmartSlMode::Off,
            smart_sl_delay: 0,
            smart_sl_only_after_rf: false,
            smart_sl_floor_be_after_rf: true,
            sltp_retry_s: 3.0,

            trail_sr_enabled: default_trail_sr_enabled(),
            trail_sr_scope: default_trail_sr_scope(),
            trail_sr_activation: default_trail_sr_activation(),
            trail_sr_min_gain: default_trail_sr_min_gain(),
            trail_sr_min_dist_price: default_trail_sr_min_dist_price(),
            entry_warstwy_offset: 0.0,
            entry_warstwy_z_tekstu: false,
            runner_cele_n: 0,
            runner_cele_krok: 10.0,
            runner_partial_pct: 0.0,
            trail_sr_tf_min: 1,
            trail_sr_fractal_n: 3,
            trail_sr_offset: 0.5,
            trail_sr_min_dist_tp: 2.0,
            trail_sr_struct_window_h: 24,
            trail_sr_min_prominence_atr: 0.0,
            trail_sr_offset_atr_mult: 0.0,
            trail_sr_offset_spread_mult: 0.0,
            trail_sr_atr_period: 14,

            harvest_retrace_pct: 0.0,
            harvest_start: 8.0,
            stale_take_min: 0.0,
            stale_take_profit: 15.0,
            stale_take_min2: 0.0,
            stale_take_profit2: 35.0,
            rev_exit_range: 0.0,
            rev_exit_slope: 14.0,
            rev_exit_profit: 4.0,
            rev_exit_window_min: 60.0,
            reenter_after_tp: false,
            reenter_min_tp_stage: 1,
            reenter_max: 0,

            sesja_bramka: SesjaBramka::Sygnal,
            session_filter: false,
            session_hours: "7-20".into(),
            max_open_positions: 0,
            enforce_position_limit_on_fill: false,
            limit_kasuje_tylko_nadmiar: false,
            max_open_baskets: 0,
            exposure_bonus_profit_pct: 0.0,
            exposure_bonus_positions: 0,
            exposure_bonus_baskets: 0,
            exposure_count_pendings: false,
            max_directional_lots: 0.0,
            streak_pause_n: 0,
            streak_pause_min: 60.0,
            signal_filter: false,
            side_filter: SideFilter::Both,
            skip_tags: String::new(),
            require_tags: String::new(),
            regime_filter: RegimeFilter::Off,
            regime_ma_hours: 72.0,
            regime_cena: RegimeCena::Rynkowa,
            regime_miara: RegimeMiara::Srednia,
            regime_pilnuj_limitow: false,
            regime_percentyl: 50.0,
            regime_strefa_martwa: 0.0,
            regime_okno2_h: 0.0,
            regime_zmiennosc_min: 0.0,
            regime_zmiennosc_max: 0.0,
            regime_gdy_rozerwany: RegimeGdyRozerwany::Milcz,
            regime_soft: false,
            regime_soft_units_mult: 1.0,
            regime_soft_lot_mult: 1.0,
            regime_soft_max_positions: 0,
            sanity_zone_max: 0.0,
            sanity_tp_max: 0.0,
            sanity_tp_rosnace: false,
            sanity_tp_strona: false,
            regime_soft_risk_mult: 1.0,
            slhit_pause_n: 0,
            slhit_pause_min: 0.0,
            slhit_pause_lot_mult: 0.0,

            max_dd_pct: 0.0,
            max_dd_usd: 0.0,
            dd_guard_scope: DdGuardScope::Daily,
            max_portfolio_risk_pct: 0.0,
            dd_soft_pct: 0.0,
            dd_soft_mult: 0.5,
            dd_hard_pct: 0.0,
            dd_hard_mult: 0.25,

            exit_min_hold_min: 0.0,
            exit_min_profit: 0.0,
            exit_r_multiple: 0.0,
            basket_target_usd: 0.0,
            exit_round_dist: 0.0,
            exit_round_step: 10.0,
            exit_spread_mult: 0.0,
            exit_on_opposite_signal: false,
            cel_z_przeciwnego: CelZPrzeciwnego::Off,
            cel_z_przeciwnego_zapas: 0.0,
            hold_after_tp_hit_min: 0.0,

            smart_exit: false,
            smart_exit_take: 0.0,
            smart_exit_giveback: 0.30,
            smart_exit_min_peak: 4.0,
            smart_exit_drop_speed: 0.0,
            smart_exit_speed_window_s: 60.0,
            smart_exit_hold_if_pending: 2.0,
            smart_exit_min_pendings: 1,
            smart_exit_pending_scope: PendingScope::SameBasket,
            smart_exit_pending_min_dist: 0.3,
            day_target_usd: 0.0,
            day_target_close: false,
            day_target_scale_lot: false,
            day_trail_stop_usd: 0.0,
            usd_scale_with_lot: false,
            eod_flat_hour: 0.0,
            flat_weekend: false,
            flat_weekend_hour: 20.0,
            equity_floor_pct: 0.0,

            riskfree_enabled: false,
            riskfree_trigger_usd: 0.0,
            riskfree_trigger_r: 0.0,
            riskfree_keep_units: 1,
            riskfree_be_offset: 0.0,
            riskfree_runner_target: RiskFreeRunnerTarget::LastTp,
            riskfree_runner_stop: RiskFreeRunnerStop::Be,
            riskfree_runner_gap: 15.0,
            riskfree_runner_max_hold_min: 90.0,
            basket_max_age_min: 0.0,
            fast_fill_reject_s: 0.0,
            fast_fill_layers: 3,
            fast_fill_soft_age_min: 0.0,
            zone_exit_adverse_s: 0.0,
            zone_exit_adverse_close: false,
            reenter_min_return_s: 0.0,
            pyramid_after_stage: 0,
            pyramid_lot_mult: 1.0,
            pyramid_regime_lookback: 0,
            pyramid_regime_max_fast_pct: 30.0,
            pyramid_min_equity_mult: 0.0,
            fast_addon_move_usd: 0.0,
            fast_addon_window_s: 60.0,
            fast_addon_max: 1,
            fast_addon_lot_mult: 1.0,
            fast_addon_min_stage: 0,
            fast_addon_cooldown_s: 60.0,

            trend_filter_enabled: false,
            trend_filter_window_h: 24.0,
            trend_filter_drop_pct: 0.0,
            trend_filter_mode: TrendFilterMode::Shrink,
            trend_filter_shrink: 0.5,

            pending_drop_require_zone_touch: false,
            trail_runners_by_depth: false,

            entry_weights_from_rr: false,
            entry_weights_rr_power: 1.0,
            entry_weights_rr_cap: 4.0,
            drop_unplaceable_levels: false,
            zakaz_ponizej_krawedzi: false,

            adaptive_params: false,
            sl_min_dist_zone_mult: 0.0,
            sl_min_dist_atr_mult: 0.0,
            sl_min_dist_floor: 0.0,
            sl_min_dist_cap: 0.0,
            entry_deep_zone_mult: 0.0,
            entry_units_zone_ref: 0.0,
            adaptive_atr_window_min: 60.0,
            units_by_hour: String::new(),

            rearm_grid_on_return: false,
            rearm_keep_empty_alive: false,
            rearm_block_after_secured: false,
            spp_blocks_rearm_when_flat: false,
            rearm_min_basket_profit: 0.0,
            rearm_max_times: 1,
            rearm_bez_pozycji: false,
            rearm_bez_pozycji_max_h: 6.0,
            rearm_min_gap_min: 15.0,

            day_target_pct: 0.0,
            day_trail_stop_pct: 0.0,
            day_trail_arm_pct: 0.0,
            day_trail_basis: DayTrailBasis::EquityPeak,
            profit_budget_arm_pct: 0.0,
            profit_budget_keep_pct: 50.0,
            profit_budget_deploy_pct: 100.0,

            daily_signal_budget: 0,
            signal_min_rr: 0.0,
            signal_min_zone_width: 0.0,
            signal_max_zone_width: 0.0,

            merge_same_side: false,
            merge_window_min: 20.0,
            merge_min_overlap: 0.5,

            exit_via_limit: false,
            exit_limit_offset: 0.0,
            exit_limit_wait_s: 60.0,
            exit_limit_min_profit: 0.0,

            stops_level: 0.20,
            commission_per_lot: 0.0,

            swap_enabled: true,
            swap_long_points: -75.82,
            swap_short_points: 27.41,
            swap_point_value: 1.0,
            swap_rollover_weekday: 3,
            swap_rollover_mult: 3.0,
            swap_pomijaj_weekend: false,
            swap_rollover_z_serwera: false,
            swap_rollover3days_mt5: 3,
            runner_ksiegowanie_v2: false,
            basket_realized_broker_only: false,
            closed_profit_net_costs: false,
            restore_strategy_continuation: false,
            msg_kurs_sprzed_luki: false,
            live_tick_order_strict: false,
            slippage_pending_pts: 0.0,
            sim_margin_check_on_fill: true,
            sim_validate_pending_stops: false,
            reenter_respect_cap: false,
            sl_edit_reaches_pendings: false,
            honor_stop_orders: false,
            hint_veto: false,
            reply_veto: false,
            sync_only_live_levels: true,
            tp_correction_to_broker: false,
            runner_max_hold_rule_only: false,
            runner_max_hold_bez_reguly: false,
            spp_arms_runner_clock: false,
            tp_hit_match_level: false,
            tp_unindexed_pips_require_price: false,
            tp_price_only_strict: false,
            rf_level_sanity_max_usd: 0.0,
            reply_graph_transitive: false,
            stop_out_level_pct: 20.0,
            sim_margin_at_market: false,
            margin_call_level_pct: 50.0,
            server_tz_offset_ms: 3 * 3_600_000,
            msg_clock_offset_ms: None,
            exec_latency_ms: 250,
            slippage_pts: 0.0,

            ai_enabled: false,
            ai_replaces_management: true,
            ai_model: String::new(),
            ai_decision_interval_s: 2.0,

            mt5_autostart: true,
            mt5_watchdog: true,
            mt5_terminal_path: String::new(),
            mt5_retry_attempts: 10,
            mt5_retry_delay_s: 5.0,
            mt5_restart_after: 1,
            mt5_health_interval_s: 5.0,

            journal_enabled: true,
            journal_min_level: crate::journal::EventLevel::Info,
            journal_snapshots: true,
            journal_excursions: true,
            journal_text_mirror: true,
            journal_retention_days: 90,
            journal_buffer_cap: 20_000,

            entry_units_small: 1,
            entry_units_small_mult: 0.0,
            risk_per_basket_pct_small: 0.0,
            risk_per_basket_pct_small_mult: 0.0,
            reenter_max_small: 0,
            reenter_max_small_mult: 0.0,
            max_open_positions_small: 0,
            max_open_positions_small_mult: 0.0,
            max_open_baskets_small: 0,
            max_open_baskets_small_mult: 0.0,
            basket_max_age_min_small: 0.0,
            basket_max_age_min_small_mult: 0.0,
            fast_fill_soft_age_min_small: 0.0,
            fast_fill_soft_age_min_small_mult: 0.0,
            market_entry_step_small: 1.0,
            market_entry_step_small_mult: 0.0,
            sl_min_dist_small: 0.0,
            sl_min_dist_small_mult: 0.0,
            lot_percent_small: 1.0,
            lot_percent_small_mult: 0.0,
            lot_max_z_salda: 0.0,
            trail_atr_mult: 0.0,
            trail_adaptive_enabled: false,
            trail_adaptive_runners_only: trail_adaptive_runners_only_default(),
            trail_adaptive_window_s: trail_adaptive_window_s_default(),
            trail_adaptive_min_samples: trail_adaptive_min_samples_default(),
            trail_adaptive_trend_er: trail_adaptive_trend_er_default(),
            trail_adaptive_reversal_er: trail_adaptive_reversal_er_default(),
            trail_adaptive_trend_gap_mult: trail_adaptive_trend_gap_mult_default(),
            trail_adaptive_chop_gap_mult: trail_adaptive_chop_gap_mult_default(),
            trail_adaptive_reversal_gap_mult: trail_adaptive_reversal_gap_mult_default(),
            trail_adaptive_fast_vol_s: trail_adaptive_fast_vol_s_default(),
            trail_adaptive_slow_vol_s: trail_adaptive_slow_vol_s_default(),
            trail_adaptive_vol_ratio: trail_adaptive_vol_ratio_default(),
            trail_adaptive_vol_favorable_mult: trail_adaptive_vol_favorable_mult_default(),
            trail_adaptive_vol_adverse_mult: trail_adaptive_vol_adverse_mult_default(),
            trail_adaptive_min_peak: 0.0,
            trail_adaptive_min_gap: 0.0,
            trail_adaptive_max_gap: 0.0,

            vol_size_mode: VolSizeMode::Off,
            vol_size_target: 0.0,
            vol_size_min_mult: 0.0,
            vol_size_max_mult: 0.0,
            vol_size_percentile_okno: 0,
            vol_size_odsezonuj: false,

            odlicz_kredyt: false,
            credit_balance_separate: false,
            kredyt_reczny: 0.0,

            parser_geometryczny: false,
            parser_min_pewnosc: 0.0,

            t100: crate::t100::Config::default(),
            ea_enabled: false,
            ea_tick_s: 0.0,
            ea_state_src: default_ea_state_src(),
            ea_defense_enter: 0.0,
            ea_defense_exit: 0.0,
            ea_offense_enter: 0.0,
            ea_offense_exit: 0.0,
            ea_state_dwell_s: 0.0,
            ea_state_ratchet: default_ea_state_ratchet(),
            ea_state_journal: default_ea_state_journal(),
            ea_dozor_sl: false,
            ea_lot_z_wolnego_marginesu: 0.0,
            ea_stop_dokladek_przy_stracie: 0.0,
            ea_stop_dokladek_powrot: 0.0,
            ea_redukcja_przy_zageszczeniu: 0.0,
            ea_zageszczenie_podloga: 0.0,
            ea_stan_dnia: default_ea_stan_dnia(),
            ea_stan_dnia_prog_sl: default_ea_stan_dnia_prog_sl(),
            ea_stan_dnia_jednostki_mult: default_ea_stan_dnia_jednostki_mult(),
        }
    }
}

fn default_ea_state_src() -> EaStateSrc {
    EaStateSrc::FloatR
}

fn default_ea_state_ratchet() -> EaRatchet {
    EaRatchet::NieLuzujWKoszyku
}

fn default_ea_state_journal() -> bool {
    true
}

fn default_ea_stan_dnia() -> EaStanDnia {
    EaStanDnia::Off
}

fn default_ea_stan_dnia_prog_sl() -> u32 {
    2
}

fn default_ea_stan_dnia_jednostki_mult() -> f64 {
    1.0
}

impl Settings {
    /// Armed trailing-day basis and cash giveback threshold. Uses only the
    /// observed equity peak and opening equity; no forecast or future prices.
    pub fn day_trail_threshold(&self, day_start: f64, day_peak: f64) -> Option<(f64, f64)> {
        if self.day_trail_stop_pct <= 0.0
            || ![day_start, day_peak, self.day_trail_stop_pct, self.day_trail_arm_pct]
                .into_iter().all(f64::is_finite)
        { return None; }
        let profit_peak = day_peak - day_start;
        let armed = self.day_trail_arm_pct <= 0.0
            || profit_peak >= day_start.max(1.0) * self.day_trail_arm_pct / 100.0;
        if !armed { return None; }
        let basis = match self.day_trail_basis {
            DayTrailBasis::EquityPeak => day_peak.max(1.0),
            DayTrailBasis::ProfitPeak if profit_peak > 0.0 => profit_peak,
            DayTrailBasis::ProfitPeak => return None,
        };
        Some((basis, basis * self.day_trail_stop_pct / 100.0))
    }

    #[inline]
    pub fn kredyt_skuteczny_z(&self, kredyt_brokera: f64) -> f64 {
        if !self.odlicz_kredyt {
            return 0.0;
        }
        let k = if self.kredyt_reczny.is_finite() && self.kredyt_reczny > 0.0 {
            self.kredyt_reczny
        } else {
            kredyt_brokera
        };
        if k.is_finite() && k > 0.0 {
            k
        } else {
            0.0
        }
    }

    #[inline]
    pub fn saldo_wlasne(&self, balance: f64, credit: f64) -> f64 {
        if self.credit_balance_separate {
            balance
        } else {
            balance - self.kredyt_skuteczny_z(credit)
        }
    }

    #[inline]
    pub fn podstawa_lota_z_konta(&self, balance: f64, equity: f64, credit: f64) -> f64 {
        let kredyt = self.kredyt_skuteczny_z(credit);
        if self.credit_balance_separate {
            let own_equity = equity - kredyt;
            match self.lot_base {
                PodstawaLota::Balance => balance,
                PodstawaLota::Equity => own_equity,
                PodstawaLota::MinOfBoth => balance.min(own_equity),
            }
            .max(0.0)
        } else {
            let kapital = match self.lot_base {
                PodstawaLota::Balance => balance,
                PodstawaLota::Equity => equity,
                PodstawaLota::MinOfBoth => balance.min(equity),
            };
            (kapital - kredyt).max(0.0)
        }
    }

    #[inline]
    pub fn msg_offset(&self) -> i64 {
        self.msg_clock_offset_ms.unwrap_or(self.server_tz_offset_ms)
    }

    #[inline]
    pub fn session_offset(&self) -> i64 {
        0
    }

    #[inline]
    pub fn journal_config(&self) -> crate::journal::JournalConfig {
        crate::journal::JournalConfig {
            enabled: self.journal_enabled,
            min_level: self.journal_min_level,
            snapshots: self.journal_snapshots,
            excursions: self.journal_excursions,
            server_offset_ms: self.server_tz_offset_ms,
            session_offset_ms: self.session_offset(),
            cap: self.journal_buffer_cap.max(64) as usize,
        }
    }

    #[inline]
    pub fn units_for(&self, is_limit: bool) -> u32 {
        if is_limit && self.entry_units_limit > 0 {
            self.entry_units_limit
        } else {
            self.entry_units
        }
    }

    #[inline]
    pub fn grid_step(&self) -> f64 {
        if self.ppm_enabled && self.ppm_for_limits && self.ppm > 0.0 {
            1.0 / self.ppm
        } else {
            0.0
        }
    }

    #[inline]
    pub fn market_step(&self) -> f64 {
        self.market_step_from(self.market_entry_step)
    }

    #[inline]
    pub fn market_step_from(&self, base_raw: f64) -> f64 {
        let base = if base_raw > 0.0 { base_raw } else { 1.0 };
        if self.ppm_enabled && self.ppm_for_market && self.ppm > 0.0 {
            base / self.ppm
        } else {
            base
        }
    }

    pub fn uklad_drabinki(&self) -> Vec<u32> {
        let v: Vec<u32> = self
            .entry_uklad
            .split(',')
            .filter_map(|x| x.trim().parse::<i64>().ok())
            .map(|x| x.clamp(0, 9) as u32)
            .collect();
        if v.iter().sum::<u32>() == 0 {
            return Vec::new();
        }
        v
    }

    pub fn depth_multipliers(&self, n: usize) -> Vec<f64> {
        if n == 0 {
            return Vec::new();
        }
        let w: Vec<f64> = self
            .entry_weights
            .split(',')
            .filter_map(|x| x.trim().parse::<f64>().ok())
            .filter(|x| *x > 0.0)
            .collect();
        if w.len() < 2 {
            return vec![1.0; n];
        }
        if n == 1 {
            return vec![1.0];
        }
        let last = w.len() - 1;
        let mut out: Vec<f64> = Vec::with_capacity(n);
        for i in 0..n {
            let depth = (n - 1 - i) as f64 / (n - 1) as f64;
            let idx = (depth * last as f64).round() as usize;
            out.push(w[idx.min(last)]);
        }
        let mean = out.iter().sum::<f64>() / n as f64;
        if mean <= 0.0 {
            return vec![1.0; n];
        }
        for v in out.iter_mut() {
            *v /= mean;
        }
        out
    }

    pub fn rr_multipliers(&self, prices: &[f64], sl: Option<f64>, tp1: Option<f64>) -> Vec<f64> {
        let n = prices.len();
        if n == 0 {
            return Vec::new();
        }
        let (Some(s), Some(t)) = (sl, tp1) else {
            return vec![1.0; n];
        };
        let mut w: Vec<f64> = Vec::with_capacity(n);
        for p in prices {
            let ryzyko = (p - s).abs();
            let nagroda = (t - p).abs();
            if ryzyko <= 1e-9 || nagroda <= 1e-9 {
                return vec![1.0; n];
            }
            w.push(nagroda / ryzyko);
        }
        let pow = if self.entry_weights_rr_power > 0.0 {
            self.entry_weights_rr_power
        } else {
            1.0
        };
        for x in w.iter_mut() {
            *x = x.powf(pow);
            if !x.is_finite() || *x <= 0.0 {
                return vec![1.0; n];
            }
        }
        let min = w.iter().copied().fold(f64::MAX, f64::min);
        if !(min > 0.0) {
            return vec![1.0; n];
        }
        let cap = if self.entry_weights_rr_cap > 0.0 {
            self.entry_weights_rr_cap
        } else {
            f64::INFINITY
        };
        for x in w.iter_mut() {
            *x = (*x / min).min(cap);
        }
        let mean = w.iter().sum::<f64>() / n as f64;
        if !(mean > 0.0) {
            return vec![1.0; n];
        }
        for x in w.iter_mut() {
            *x /= mean;
        }
        w
    }

    pub fn hour_units_mult(&self, hour: u32) -> f64 {
        if self.units_by_hour.trim().is_empty() {
            return 1.0;
        }
        for part in self.units_by_hour.split(',') {
            let mut it = part.split(':');
            let zakres = match it.next() {
                Some(x) => x.trim(),
                None => continue,
            };
            let mult = match it.next().and_then(|x| x.trim().parse::<f64>().ok()) {
                Some(m) if m > 0.0 => m,
                _ => continue,
            };
            let mut g = zakres.split('-');
            let a = g.next().and_then(|x| x.trim().parse::<u32>().ok());
            let b = g.next().and_then(|x| x.trim().parse::<u32>().ok());
            match (a, b) {
                (Some(a), Some(b)) if hour >= a && hour < b => return mult,
                (Some(a), None) if hour == a => return mult,
                _ => {}
            }
        }
        1.0
    }

    #[inline]
    pub fn bank_count(&self, n: usize, pct: f64) -> usize {
        if n == 0 || pct <= 0.0 {
            return 0;
        }
        let raw = n as f64 * pct / 100.0;
        let c = match self.bank_rounding {
            BankRounding::Nearest => raw.round(),
            BankRounding::Up => (raw - 1e-9).ceil(),
            BankRounding::Down => (raw + 1e-9).floor(),
        };
        (c.max(0.0) as usize).min(n)
    }

    pub fn partials_allowed(&self, volumes: &[f64]) -> bool {
        self.partial_close
            && !volumes.is_empty()
            && volumes.iter().all(|v| *v >= self.partial_min_lot - 1e-9)
    }

    #[inline]
    pub fn usd_scale(&self, lot: f64) -> f64 {
        if self.usd_scale_with_lot {
            (lot / 0.01).max(1.0)
        } else {
            1.0
        }
    }

    pub fn parse_counts(&self) -> Vec<u32> {
        self.official_counts
            .split(',')
            .filter_map(|s| s.trim().parse::<u32>().ok())
            .collect()
    }

    pub fn parse_tiers(raw: &str) -> Vec<(f64, f64)> {
        raw.split(',')
            .filter_map(|c| {
                let mut it = c.split(':');
                let a = it.next()?.trim().parse::<f64>().ok()?;
                let b = it.next()?.trim().parse::<f64>().ok()?;
                Some((a, b))
            })
            .collect()
    }

    pub fn parse_toucher_bands(&self) -> Vec<(f64, u32, usize)> {
        if self.toucher_bands.trim().is_empty() {
            if self.toucher_units > 0 {
                let idx = if self.toucher_tp_one_based {
                    self.toucher_tp_index.max(1) - 1
                } else {
                    self.toucher_tp_index
                };
                return vec![(0.0, self.toucher_units, idx)];
            }
            return Vec::new();
        }
        self.toucher_bands
            .split(',')
            .filter_map(|c| {
                let p: Vec<&str> = c.split(':').collect();
                if p.len() < 2 {
                    return None;
                }
                let off = p[0].trim().parse::<f64>().ok()? * crate::types::PIP;
                let units = p[1].trim().parse::<u32>().ok()?;
                let tp = p
                    .get(2)
                    .and_then(|x| x.trim().parse::<usize>().ok())
                    .unwrap_or(2);
                Some((off, units, tp.saturating_sub(1)))
            })
            .collect()
    }

    pub fn hours_ok(&self, hour: u32) -> bool {
        if !self.session_filter {
            return true;
        }
        for part in self.session_hours.split(',') {
            let mut it = part.split('-');
            let a = it.next().and_then(|x| x.trim().parse::<u32>().ok());
            let b = it.next().and_then(|x| x.trim().parse::<u32>().ok());
            match (a, b) {
                (Some(a), Some(b)) if hour >= a && hour < b => return true,
                (Some(a), None) if hour == a => return true,
                _ => {}
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_profit_trail_protects_profit_instead_of_entire_equity() {
        let mut c = Settings::default();
        c.day_trail_stop_pct = 30.0;
        c.day_trail_arm_pct = 12.0;
        assert_eq!(c.day_trail_threshold(300.0, 336.0), Some((336.0, 100.8)));
        c.day_trail_basis = DayTrailBasis::ProfitPeak;
        assert_eq!(c.day_trail_threshold(300.0, 336.0), Some((36.0, 10.8)));
        assert!(336.0 - c.day_trail_threshold(300.0, 336.0).unwrap().1 > 300.0);
        assert!(c.day_trail_threshold(300.0, 335.0).is_none(), "arm threshold not reached");
    }

    #[test]
    fn day_profit_trail_never_arms_without_positive_profit_and_off_is_off() {
        let mut c = Settings::default();
        c.day_trail_basis = DayTrailBasis::ProfitPeak;
        c.day_trail_stop_pct = 30.0;
        for peak in [280.0, 300.0, f64::NAN, f64::INFINITY] {
            assert!(c.day_trail_threshold(300.0, peak).is_none());
        }
        c.day_trail_stop_pct = 0.0;
        assert!(c.day_trail_threshold(300.0, 500.0).is_none());
    }

    #[test]
    fn legacy_day_trail_math_and_missing_field_remain_compatible() {
        let mut c: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(c.day_trail_basis, DayTrailBasis::EquityPeak);
        for start in [0.0_f64, 300.0, 600.0] {
            for peak in [300.0_f64, 336.0, 700.0] {
                for arm in [0.0, 12.0, 30.0] {
                    c.day_trail_stop_pct = 25.0;
                    c.day_trail_arm_pct = arm;
                    let armed = arm <= 0.0 || peak - start >= start.max(1.0) * arm / 100.0;
                    let expected = armed.then_some((peak.max(1.0), peak.max(1.0) * 25.0 / 100.0));
                    assert_eq!(c.day_trail_threshold(start, peak), expected);
                }
            }
        }
    }

    #[test]
    fn zaokraglanie_transzy_ma_trzy_rozne_odpowiedzi() {
        let mut c = Settings::default();
        c.bank_rounding = BankRounding::Nearest;
        assert_eq!(c.bank_count(3, 15.0), 0, "„nearest\" gubi całą transzę TP1");
        c.bank_rounding = BankRounding::Up;
        assert_eq!(c.bank_count(3, 15.0), 1);
        c.bank_rounding = BankRounding::Down;
        assert_eq!(c.bank_count(3, 15.0), 0);
        c.bank_rounding = BankRounding::Up;
        assert_eq!(c.bank_count(4, 50.0), 2);
        assert_eq!(c.bank_count(3, 300.0), 3);
        assert_eq!(c.bank_count(0, 50.0), 0);
        assert_eq!(c.bank_count(5, 0.0), 0);
    }

    #[test]
    fn wagi_glebokosci_rosna_w_strone_lepszych_wejsc() {
        let mut c = Settings::default();
        c.entry_weights = "1,2,4".into();
        let m = c.depth_multipliers(3);
        assert!(m[0] > m[1] && m[1] > m[2], "{m:?}");
        assert!((m[0] / m[2] - 4.0).abs() < 1e-9, "{m:?}");
    }

    #[test]
    fn wagi_redystrybuuja_wolumen_a_nie_go_powiekszaja() {
        let mut c = Settings::default();
        c.entry_weights = "1,2,4".into();
        for n in [2usize, 3, 5, 8] {
            let m = c.depth_multipliers(n);
            let mean = m.iter().sum::<f64>() / n as f64;
            assert!(
                (mean - 1.0).abs() < 1e-9,
                "średnia mnożników musi wynosić 1, jest {mean} dla n={n}"
            );
        }
    }

    #[test]
    fn brak_wag_zostawia_drabinke_rowna() {
        let c = Settings::default();
        assert_eq!(c.depth_multipliers(4), vec![1.0; 4]);
        let mut c2 = Settings::default();
        c2.entry_weights = "3".into();
        assert_eq!(c2.depth_multipliers(4), vec![1.0; 4]);
        let mut c3 = Settings::default();
        c3.entry_weights = "abc,,-2".into();
        assert_eq!(c3.depth_multipliers(3), vec![1.0; 3]);
    }

    #[test]
    fn wagi_rozciagaja_sie_na_dowolna_liczbe_poziomow() {
        let mut c = Settings::default();
        c.entry_weights = "1,4".into();
        let m = c.depth_multipliers(6);
        assert_eq!(m.len(), 6);
        assert!(m[0] > m[5], "najgłębszy poziom nadal największy: {m:?}");
        assert_eq!(c.depth_multipliers(1), vec![1.0]);
        assert!(c.depth_multipliers(0).is_empty());
    }

    #[test]
    fn partiale_wymagaja_zeby_kazda_pozycja_byla_dosc_duza() {
        let mut c = Settings::default();
        c.partial_close = true;
        c.partial_min_lot = 0.02;
        assert!(c.partials_allowed(&[0.05, 0.02, 0.10]));
        assert!(!c.partials_allowed(&[0.05, 0.01]));
        assert!(!c.partials_allowed(&[]));
        c.partial_close = false;
        assert!(!c.partials_allowed(&[0.10, 0.10]));
    }

    #[test]
    fn ppm_ma_dwa_niezalezne_zastosowania() {
        let mut c = Settings::default();
        c.ppm_enabled = true;
        c.ppm = 2.0;
        c.market_entry_step = 1.0;

        c.ppm_for_limits = true;
        c.ppm_for_market = false;
        assert_eq!(c.grid_step(), 0.5, "gęstsza siatka limitów");
        assert_eq!(c.market_step(), 1.0, "krok wejść rynkowych bez zmian");

        c.ppm_for_limits = false;
        c.ppm_for_market = true;
        assert_eq!(c.grid_step(), 0.0);
        assert_eq!(c.market_step(), 0.5);
    }

    #[test]
    fn krok_wejsc_rynkowych_nigdy_nie_jest_zerowy() {
        let mut c = Settings::default();
        c.market_entry_step = 0.0;
        assert_eq!(c.market_step(), 1.0, "zero oznaczałoby dokładanie co tick");
    }


    #[test]
    fn wagi_z_rr_daja_wiecej_szczeblowi_o_lepszym_stosunku() {
        let mut c = Settings::default();
        c.entry_weights_rr_cap = 0.0; // bez sufitu — chcemy zobaczyć czysty stosunek
        let ceny = vec![4000.0, 4005.0];
        let m = c.rr_multipliers(&ceny, Some(3999.0), Some(4008.0));
        assert!(m[0] > m[1], "{m:?}");
        assert!(
            (m[0] / m[1] - 16.0).abs() < 1e-6,
            "stosunek jakości 16× — {m:?}"
        );
        let mean = (m[0] + m[1]) / 2.0;
        assert!(
            (mean - 1.0).abs() < 1e-9,
            "wagi mają redystrybuować, nie powiększać: {mean}"
        );
    }

    #[test]
    fn sufit_wag_rr_nie_pozwala_zawiesic_koszyka_na_jednym_poziomie() {
        let mut c = Settings::default();
        c.entry_weights_rr_cap = 4.0;
        let ceny = vec![4000.0, 4005.0];
        let m = c.rr_multipliers(&ceny, Some(3999.0), Some(4008.0));
        assert!(
            (m[0] / m[1] - 4.0).abs() < 1e-6,
            "sufit 4× musi obciąć 16× — {m:?}"
        );
        let mean = (m[0] + m[1]) / 2.0;
        assert!((mean - 1.0).abs() < 1e-9);
    }

    #[test]
    fn wykladnik_lagodzi_albo_zaostrza_wagi_rr() {
        let ceny = vec![4000.0, 4005.0];
        let mut c = Settings::default();
        c.entry_weights_rr_cap = 0.0;
        c.entry_weights_rr_power = 0.5;
        let m = c.rr_multipliers(&ceny, Some(3999.0), Some(4008.0));
        assert!(
            (m[0] / m[1] - 4.0).abs() < 1e-6,
            "pierwiastek z 16 to 4 — {m:?}"
        );
    }

    #[test]
    fn brak_sl_albo_celu_zostawia_wagi_rowne() {
        let c = Settings::default();
        let ceny = vec![4000.0, 4005.0];
        assert_eq!(c.rr_multipliers(&ceny, None, Some(4008.0)), vec![1.0, 1.0]);
        assert_eq!(c.rr_multipliers(&ceny, Some(3999.0), None), vec![1.0, 1.0]);
        assert_eq!(
            c.rr_multipliers(&ceny, Some(4000.0), Some(4008.0)),
            vec![1.0, 1.0]
        );
        assert_eq!(
            c.rr_multipliers(&ceny, Some(3990.0), Some(4005.0)),
            vec![1.0, 1.0]
        );
        assert!(c.rr_multipliers(&[], Some(1.0), Some(2.0)).is_empty());
    }


    #[test]
    fn pasma_godzinowe_zmieniaja_liczbe_jednostek() {
        let mut c = Settings::default();
        c.units_by_hour = "7-11:0.5,15-17:2".into();
        assert_eq!(c.hour_units_mult(8), 0.5);
        assert_eq!(c.hour_units_mult(15), 2.0);
        assert_eq!(c.hour_units_mult(16), 2.0);
        assert_eq!(c.hour_units_mult(11), 1.0);
        assert_eq!(c.hour_units_mult(17), 1.0);
        assert_eq!(c.hour_units_mult(3), 1.0);
    }

    #[test]
    fn puste_i_bledne_pasma_godzinowe_nic_nie_zmieniaja() {
        let c = Settings::default();
        assert_eq!(c.hour_units_mult(12), 1.0);
        let mut c2 = Settings::default();
        c2.units_by_hour = "abc,7-11,,9-10:-3,12-13:0".into();
        for h in 0..24 {
            assert_eq!(
                c2.hour_units_mult(h),
                1.0,
                "godzina {h} nie może zmienić rozmiaru"
            );
        }
    }

    #[test]
    fn ustawienia_przechodza_przez_json_bez_strat() {
        let mut c = Settings::default();
        c.smart_sl_mode = SmartSlMode::LadderWithBe;
        c.bank_from = BankFrom::Best;
        c.last_runner = LastRunner::NoTp;
        c.reenter_after_tp = true;
        let s = serde_json::to_string(&c).unwrap();
        let back: Settings = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn brak_klucza_w_presecie_bierze_domyslna_z_silnika() {
        let c: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(
            c.sync_only_live_levels,
            Settings::default().sync_only_live_levels,
            "preset bez klucza dostał wartość INNĄ niż domyślna silnika —              sprawdź, czy pole nie ma własnego `#[serde(default)]`"
        );
        assert!(c.sync_only_live_levels, "domyślna od 18.08.2026 to `true`");
        let c: Settings = serde_json::from_str(r#"{"sync_only_live_levels": false}"#).unwrap();
        assert!(
            !c.sync_only_live_levels,
            "jawny zapis w presecie musi być uszanowany"
        );
    }

    #[test]
    fn front_run_tp_ma_zero_contract_i_raportuje_ujemna_martwa_wartosc() {
        let c: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(c.tp_price_front_run_usd, 0.0);
        assert!(!c
            .martwe_ustawienia()
            .iter()
            .any(|x| x.pole == "tp_price_front_run_usd"));

        let mut c = Settings::default();
        c.tp_price_front_run_usd = -0.2;
        assert!(c
            .martwe_ustawienia()
            .iter()
            .any(|x| x.pole == "tp_price_front_run_usd"));

        c.tp_price_front_run_usd = 0.2;
        c.tp_source = TpSource::SignalOnly;
        assert!(c
            .pulapki_konfiguracji()
            .iter()
            .any(|x| x.contains("niezależną drogę CENOWĄ")));
    }

    #[test]
    fn hybryda_market_raportuje_martwe_parametry_i_sprzeczne_bramki() {
        let mut c = Settings::default();
        c.market_hybrid_pending_units = 3;
        c.market_hybrid_lot_mult = 0.5;
        c.market_hybrid_max_chase_usd = 0.7;
        c.market_hybrid_tp_stage = 2;
        let m = c.martwe_ustawienia();
        for pole in [
            "market_hybrid_pending_units",
            "market_hybrid_lot_mult",
            "market_hybrid_max_chase_usd",
            "market_hybrid_tp_stage",
        ] {
            assert!(
                m.iter().any(|x| x.pole == pole),
                "{pole} nie zostało zgłoszone przy wyłączonej hybrydzie: {m:?}"
            );
        }

        c.market_hybrid_now_units = 1;
        let m = c.martwe_ustawienia();
        for pole in [
            "market_hybrid_pending_units",
            "market_hybrid_lot_mult",
            "market_hybrid_max_chase_usd",
            "market_hybrid_tp_stage",
        ] {
            assert!(
                !m.iter().any(|x| x.pole == pole),
                "{pole} pozostaje martwe mimo włączonej hybrydy: {m:?}"
            );
        }

        c.auto_limit = false;
        assert!(c
            .martwe_ustawienia()
            .iter()
            .any(|x| x.pole == "market_hybrid_now_units" && x.wlacznik == "auto_limit"));

        c.auto_limit = true;
        c.only_limit_signals = true;
        c.market_unfilled_cancel_stage = 1;
        let m = c.martwe_ustawienia();
        assert!(m.iter().any(|x| {
            x.pole == "market_hybrid_now_units" && x.wlacznik == "only_limit_signals"
        }));
        assert!(m.iter().any(|x| x.pole == "market_unfilled_cancel_stage"));
    }

    #[test]
    fn cala_rodzina_riskfree_ma_jawna_bramke_aktywacji() {
        let mut c = Settings::default();
        c.riskfree_runner_target = RiskFreeRunnerTarget::NoTpTrailOnly;
        c.riskfree_runner_stop = RiskFreeRunnerStop::BeOwn;
        c.pending_cancel_on_riskfree = true;
        let m = c.martwe_ustawienia();
        for pole in [
            "riskfree_runner_target",
            "riskfree_runner_stop",
            "pending_cancel_on_riskfree",
        ] {
            assert!(
                m.iter().any(|x| x.pole == pole),
                "{pole} nie zostało zgłoszone przy riskfree_enabled=false: {m:?}"
            );
        }

        c.riskfree_enabled = true;
        let m = c.martwe_ustawienia();
        for pole in [
            "riskfree_runner_target",
            "riskfree_runner_stop",
            "pending_cancel_on_riskfree",
        ] {
            assert!(
                !m.iter().any(|x| x.pole == pole),
                "{pole} pozostaje martwe mimo riskfree_enabled=true: {m:?}"
            );
        }
    }

    #[test]
    fn bramka_kapitalowa_bez_progu_jest_martwym_ustawieniem() {
        let mut c = Settings::default();
        c.entry_units = 5;
        c.entry_units_small = 3;
        let m = c.martwe_ustawienia();
        assert!(
            m.iter().any(|x| x.pole == "entry_units_small"),
            "brak ostrzeżenia o bramce bez progu: {m:?}"
        );
        c.entry_units_small_mult = 2.0;
        assert!(!c
            .martwe_ustawienia()
            .iter()
            .any(|x| x.pole == "entry_units_small"));
        let mut c2 = Settings::default();
        c2.sl_min_dist_small = c2.sl_min_dist;
        assert!(!c2
            .martwe_ustawienia()
            .iter()
            .any(|x| x.pole == "sl_min_dist_small"));
    }

    #[test]
    fn rodziny_z_audytu_k4_sa_wykrywane() {
        let nowe = [
            "expo_cap_close",
            "expo_cap_s",
            "pending_resize_s",
            "merge_min_overlap",
            "smart_exit_take",
            "trend_filter_drop_pct",
            "vol_size_target",
            "rearm_max_times",
            "slhit_pause_min",
            "regime_percentyl",
            "pyramid_min_equity_mult",
            "fast_addon_lot_mult",
            "exit_round_step",
            "riskfree_keep_units",
            "official_counts",
            "parser_min_pewnosc",
        ];

        let md = Settings::default().martwe_ustawienia();
        for pole in nowe {
            assert!(
                !md.iter().any(|x| x.pole == pole),
                "{pole} krzyczy na czystym default"
            );
        }

        let mut c = Settings::default();
        c.expo_cap_close = true; //           expo_cap_pct/ml_pct = 0
        c.expo_cap_s = 5.0;
        c.pending_resize_s = 60.0; //         pending_resize_on_vol = false
        c.merge_min_overlap = 0.9; //         merge_same_side = false
        c.smart_exit_take = 12.0; //          smart_exit = false
        c.trend_filter_drop_pct = 3.0; //     trend_filter_enabled = false
        c.vol_size_target = 8.0; //           vol_size_mode = Off
        c.rearm_max_times = 4; //             rearm_grid_on_return = false
        c.slhit_pause_min = 90.0; //          slhit_pause_n = 0
        c.regime_percentyl = 80.0; //         regime_filter = Off
        c.pyramid_min_equity_mult = 1.5; //   pyramid_after_stage = 0
        c.fast_addon_lot_mult = 2.0; //       fast_addon_move_usd = 0
        c.exit_round_step = 25.0; //          exit_round_dist = 0
        c.riskfree_keep_units = 3; //         riskfree_enabled = false
        c.official_counts = "2,2".into(); //  tp_schedule = Ladder
        c.parser_min_pewnosc = 0.6; //        parser_geometryczny = false
        let m = c.martwe_ustawienia();
        for pole in nowe {
            assert!(
                m.iter().any(|x| x.pole == pole),
                "brak zgłoszenia dla {pole}: {m:?}"
            );
        }

        let mut c2 = c.clone();
        c2.smart_exit = true;
        c2.regime_filter = RegimeFilter::TrendMa;
        c2.fast_addon_move_usd = 3.0;
        let m2 = c2.martwe_ustawienia();
        for pole in ["smart_exit_take", "regime_percentyl", "fast_addon_lot_mult"] {
            assert!(
                !m2.iter().any(|x| x.pole == pole),
                "{pole} zgłoszony mimo włącznika: {m2:?}"
            );
        }

        let mut c3 = Settings::default();
        c3.tp_schedule = TpSchedule::OfficialCounts;
        c3.official_pct = [40.0, 30.0, 20.0, 10.0];
        assert!(c3
            .martwe_ustawienia()
            .iter()
            .any(|x| x.pole == "official_pct"));
        assert!(!c3
            .martwe_ustawienia()
            .iter()
            .any(|x| x.pole == "official_counts"));
    }

    #[test]
    fn krok_wejsc_rynkowych_z_podmieniona_baza() {
        let mut c = Settings::default();
        c.market_entry_step = 0.3;
        c.ppm_enabled = true;
        c.ppm_for_market = true;
        c.ppm = 2.0;
        assert_eq!(c.market_step(), 0.15);
        assert_eq!(c.market_step_from(0.6), 0.3);
        assert_eq!(c.market_step_from(0.0), 0.5);
    }

    #[test]
    fn brakujace_klucze_biora_wartosc_domyslna() {
        let c: Settings = serde_json::from_str(r#"{"lot_fixed":0.05}"#).unwrap();
        assert_eq!(c.lot_fixed, 0.05);
        assert_eq!(c.bank_rounding, BankRounding::Up);
        assert_eq!(c.reenter_min_tp_stage, 1);
    }

    #[test]
    fn adaptacyjny_trailing_ma_jawny_wlacznik_i_zywe_parametry() {
        let mut c = Settings::default();
        c.trail_adaptive_window_s = 75.0;
        assert!(
            c.martwe_ustawienia()
                .iter()
                .any(|x| x.pole == "trail_adaptive_window_s"
                    && x.wlacznik == "trail_adaptive_enabled")
        );

        c.trail_adaptive_enabled = true;
        assert!(!c
            .martwe_ustawienia()
            .iter()
            .any(|x| x.pole == "trail_adaptive_window_s"));

        c.trail_mode = TrailMode::LockPct;
        c.trail_runner_mode = TrailMode::Tiered;
        c.riskfree_enabled = false;
        assert!(c
            .martwe_ustawienia()
            .iter()
            .any(|x| x.pole == "trail_adaptive_enabled"));
    }

    #[test]
    fn adaptacyjny_trailing_glosno_zglasza_niejawna_normalizacje() {
        let mut c = Settings::default();
        c.trail_adaptive_enabled = true;
        c.trail_adaptive_window_s = 0.0;
        c.trail_adaptive_min_samples = 1;
        c.trail_adaptive_fast_vol_s = 120.0;
        c.trail_adaptive_slow_vol_s = 20.0;
        c.trail_adaptive_trend_er = 1.5;
        c.trail_adaptive_reversal_gap_mult = -0.5;
        c.trail_adaptive_min_gap = 10.0;
        c.trail_adaptive_max_gap = 5.0;

        let p = c.pulapki_konfiguracji().join("\n");
        for fragment in [
            "trail_adaptive_window_s",
            "trail_adaptive_min_samples",
            "musi być krótsze",
            "progi ER",
            "trail_adaptive_reversal_gap_mult",
            "odwrócone klamry",
        ] {
            assert!(p.contains(fragment), "brak ostrzeżenia `{fragment}` w:\n{p}");
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MartweUstawienie {
    pub pole: &'static str,
    pub wlacznik: &'static str,
    pub opis: String,
}

impl Settings {
    fn zegar_runnera_martwy(&self) -> bool {
        self.riskfree_runner_max_hold_min > 0.0
            && !self.riskfree_enabled
            && (!self.runner_max_hold_bez_reguly || self.runner_max_hold_rule_only)
    }

    pub fn martwe_ustawienia(&self) -> Vec<MartweUstawienie> {
        let mut v = Vec::new();
        let mut zglos = |pole: &'static str, wlacznik: &'static str, co: String| {
            v.push(MartweUstawienie {
                pole,
                wlacznik,
                opis: co,
            });
        };

        if self.runner_partial_pct != 0.0 {
            zglos("runner_partial_pct", "brak_implementacji",
                "runner_partial_pct nie ma wykonawczego czytelnika; ta wartość NIE steruje inkasem. Użyj sprawdzonego harmonogramu/partial_close albo pozostaw 0.".into());
        }
        if self.tp_price_only_strict && self.tp_source != TpSource::PriceOnly {
            zglos(
                "tp_price_only_strict",
                "tp_source=PriceOnly",
                "tp_price_only_strict dotyczy wyłącznie PriceOnly; nie zmienia innych źródeł TP."
                    .into(),
            );
        }
        if self.retarget_respects_final_target && !self.cele_na_ostatnim {
            zglos("retarget_respects_final_target", "cele_na_ostatnim",
                "Ta korekta retarget działa tylko przy cele_na_ostatnim; nie zmienia innych harmonogramów.".into());
        }

        let hybrid_def = Settings::default();
        let hybrid_diff = |a: f64, b: f64| (a - b).abs() > 1e-12;
        if self.market_hybrid_now_units == 0 {
            for (pole, rozna) in [
                (
                    "market_hybrid_pending_units",
                    self.market_hybrid_pending_units != hybrid_def.market_hybrid_pending_units,
                ),
                (
                    "market_hybrid_lot_mult",
                    hybrid_diff(
                        self.market_hybrid_lot_mult,
                        hybrid_def.market_hybrid_lot_mult,
                    ),
                ),
                (
                    "market_hybrid_max_chase_usd",
                    hybrid_diff(
                        self.market_hybrid_max_chase_usd,
                        hybrid_def.market_hybrid_max_chase_usd,
                    ),
                ),
                (
                    "market_hybrid_tp_stage",
                    self.market_hybrid_tp_stage != hybrid_def.market_hybrid_tp_stage,
                ),
            ] {
                if rozna {
                    zglos(
                        pole,
                        "market_hybrid_now_units",
                        format!("{pole} nie działa — ustaw `market_hybrid_now_units > 0`"),
                    );
                }
            }
        } else if !self.auto_limit {
            zglos(
                "market_hybrid_now_units",
                "auto_limit",
                "hybryda `teraz + limity` nie działa przy `auto_limit = false` — włącz `auto_limit`"
                    .into(),
            );
        }
        if self.only_limit_signals && self.market_hybrid_now_units > 0 {
            zglos(
                "market_hybrid_now_units",
                "only_limit_signals",
                "hybryda obsługuje sygnały bez `LIMITS`, ale `only_limit_signals = true` odrzuca je przed wykonaniem"
                    .into(),
            );
        }
        if self.only_limit_signals
            && self.market_unfilled_cancel_stage != hybrid_def.market_unfilled_cancel_stage
        {
            zglos(
                "market_unfilled_cancel_stage",
                "only_limit_signals",
                "oś sprząta wyłącznie niewypełnione sygnały bez `LIMITS`, które `only_limit_signals = true` odrzuca wcześniej"
                    .into(),
            );
        }

        if !self.adaptive_params {
            for (pole, wart) in [
                ("sl_min_dist_zone_mult", self.sl_min_dist_zone_mult),
                ("sl_min_dist_atr_mult", self.sl_min_dist_atr_mult),
                ("sl_min_dist_floor", self.sl_min_dist_floor),
                ("sl_min_dist_cap", self.sl_min_dist_cap),
            ] {
                if wart > 0.0 {
                    zglos(
                        pole,
                        "adaptive_params",
                        format!("{pole} = {wart} nie działa — włącz `adaptive_params`"),
                    );
                }
            }
            if !self.units_by_hour.trim().is_empty() {
                zglos(
                    "units_by_hour",
                    "adaptive_params",
                    "units_by_hour nie działa — włącz `adaptive_params`".into(),
                );
            }
        }

        if self.fast_fill_reject_s <= 0.0 {
            if self.fast_fill_soft_age_min > 0.0 {
                zglos(
                    "fast_fill_soft_age_min",
                    "fast_fill_reject_s",
                    "tryb miękki filtra tempa nie działa — ustaw próg `fast_fill_reject_s`".into(),
                );
            }
        }

        if !self.reenter_after_tp {
            if self.reenter_max > 0 {
                zglos(
                    "reenter_max",
                    "reenter_after_tp",
                    "limit ponownych wejść nie działa — włącz `reenter_after_tp`".into(),
                );
            }
            if self.reenter_min_return_s > 0.0 {
                zglos(
                    "reenter_min_return_s",
                    "reenter_after_tp",
                    "odstęp ponownego wejścia nie działa — włącz `reenter_after_tp`".into(),
                );
            }
        }

        if self.pyramid_after_stage == 0 && (self.pyramid_lot_mult - 1.0).abs() > f64::EPSILON {
            zglos(
                "pyramid_lot_mult",
                "pyramid_after_stage",
                "mnożnik piramidy nie działa — ustaw `pyramid_after_stage`".into(),
            );
        }

        {
            let d = |a: f64, b: f64| (a - b).abs() > 1e-12;
            for (pole, wlacznik, rozna) in [
                (
                    "entry_units_small",
                    "entry_units_small_mult",
                    self.entry_units_small_mult <= 0.0
                        && self.entry_units_small != self.entry_units,
                ),
                (
                    "risk_per_basket_pct_small",
                    "risk_per_basket_pct_small_mult",
                    self.risk_per_basket_pct_small_mult <= 0.0
                        && d(self.risk_per_basket_pct_small, self.risk_per_basket_pct),
                ),
                (
                    "reenter_max_small",
                    "reenter_max_small_mult",
                    self.reenter_max_small_mult <= 0.0
                        && self.reenter_max_small != self.reenter_max,
                ),
                (
                    "max_open_positions_small",
                    "max_open_positions_small_mult",
                    self.max_open_positions_small_mult <= 0.0
                        && self.max_open_positions_small != self.max_open_positions,
                ),
                (
                    "max_open_baskets_small",
                    "max_open_baskets_small_mult",
                    self.max_open_baskets_small_mult <= 0.0
                        && self.max_open_baskets_small != self.max_open_baskets,
                ),
                (
                    "basket_max_age_min_small",
                    "basket_max_age_min_small_mult",
                    self.basket_max_age_min_small_mult <= 0.0
                        && d(self.basket_max_age_min_small, self.basket_max_age_min),
                ),
                (
                    "fast_fill_soft_age_min_small",
                    "fast_fill_soft_age_min_small_mult",
                    self.fast_fill_soft_age_min_small_mult <= 0.0
                        && d(
                            self.fast_fill_soft_age_min_small,
                            self.fast_fill_soft_age_min,
                        ),
                ),
                (
                    "market_entry_step_small",
                    "market_entry_step_small_mult",
                    self.market_entry_step_small_mult <= 0.0
                        && d(self.market_entry_step_small, self.market_entry_step),
                ),
                (
                    "sl_min_dist_small",
                    "sl_min_dist_small_mult",
                    self.sl_min_dist_small_mult <= 0.0
                        && d(self.sl_min_dist_small, self.sl_min_dist),
                ),
                (
                    "lot_percent_small",
                    "lot_percent_small_mult",
                    self.lot_percent_small_mult <= 0.0
                        && d(self.lot_percent_small, self.lot_percent),
                ),
            ] {
                if rozna {
                    zglos(
                        pole,
                        wlacznik,
                        format!(
                            "{pole} nie działa — próg `{wlacznik}` wynosi 0, \
                             więc bramka kapitałowa jest wyłączona"
                        ),
                    );
                }
            }
        }

        if !self.session_filter && !self.session_hours.trim().is_empty() {
            zglos(
                "session_hours",
                "session_filter",
                "godziny sesji nie działają — włącz `session_filter`".into(),
            );
        }

        if !self.swap_enabled && (self.swap_long_points != 0.0 || self.swap_short_points != 0.0) {
            zglos(
                "swap_long_points / swap_short_points",
                "swap_enabled",
                "punkty swapu nie są naliczane — włącz `swap_enabled`".into(),
            );
        }

        if self.zegar_runnera_martwy() {
            zglos(
                "riskfree_runner_max_hold_min",
                if self.runner_max_hold_bez_reguly {
                    "runner_max_hold_rule_only"
                } else {
                    "runner_max_hold_bez_reguly"
                },
                if self.runner_max_hold_bez_reguly {
                    format!(
                        "riskfree_runner_max_hold_min = {} min nie ma czego domykać — \
                         `runner_max_hold_rule_only` zawęża limit do koszyków uwolnionych \
                         REGUŁĄ (`secured_by_rule`), a przy `riskfree_enabled = false` \
                         reguła nie uwalnia ani jednego koszyka. Wyłącz `rule_only` \
                         albo włącz regułę.",
                        self.riskfree_runner_max_hold_min
                    )
                } else {
                    format!(
                        "riskfree_runner_max_hold_min = {} min nie działa — limit siedzi \
                         w regule `riskfree_enabled`, która jest wyłączona. Włącz \
                         `runner_max_hold_bez_reguly`, żeby dotyczył koszyków uwolnionych \
                         KOMUNIKATEM kanału.",
                        self.riskfree_runner_max_hold_min
                    )
                },
            );
        }

        {
            let def = Settings::default();
            let d = |a: f64, b: f64| (a - b).abs() > 1e-12;

            if self.expo_cap_pct <= 0.0 && self.expo_cap_ml_pct <= 0.0 {
                if self.expo_cap_close {
                    zglos(
                        "expo_cap_close",
                        "expo_cap_pct",
                        "domykanie pozycji pod próg nie działa — oba progi \
                         (`expo_cap_pct` i `expo_cap_ml_pct`) są zerowe"
                            .into(),
                    );
                }
                if d(self.expo_cap_s, def.expo_cap_s) {
                    zglos(
                        "expo_cap_s",
                        "expo_cap_pct",
                        "kadencja pułapu ekspozycji nie działa — oba progi \
                         (`expo_cap_pct` i `expo_cap_ml_pct`) są zerowe"
                            .into(),
                    );
                }
            }

            if !self.pending_resize_on_vol && d(self.pending_resize_s, def.pending_resize_s) {
                zglos(
                    "pending_resize_s",
                    "pending_resize_on_vol",
                    "kadencja przeliczania lotów w zleceniach nie działa — \
                     włącz `pending_resize_on_vol`"
                        .into(),
                );
            }

            if !self.merge_same_side {
                for (pole, rozna) in [
                    (
                        "merge_window_min",
                        d(self.merge_window_min, def.merge_window_min),
                    ),
                    (
                        "merge_min_overlap",
                        d(self.merge_min_overlap, def.merge_min_overlap),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "merge_same_side",
                            format!("{pole} nie działa — włącz `merge_same_side`"),
                        );
                    }
                }
            }

            if !self.smart_exit {
                for (pole, rozna) in [
                    (
                        "smart_exit_take",
                        d(self.smart_exit_take, def.smart_exit_take),
                    ),
                    (
                        "smart_exit_giveback",
                        d(self.smart_exit_giveback, def.smart_exit_giveback),
                    ),
                    (
                        "smart_exit_min_peak",
                        d(self.smart_exit_min_peak, def.smart_exit_min_peak),
                    ),
                    (
                        "smart_exit_drop_speed",
                        d(self.smart_exit_drop_speed, def.smart_exit_drop_speed),
                    ),
                    (
                        "smart_exit_speed_window_s",
                        d(
                            self.smart_exit_speed_window_s,
                            def.smart_exit_speed_window_s,
                        ),
                    ),
                    (
                        "smart_exit_hold_if_pending",
                        d(
                            self.smart_exit_hold_if_pending,
                            def.smart_exit_hold_if_pending,
                        ),
                    ),
                    (
                        "smart_exit_min_pendings",
                        self.smart_exit_min_pendings != def.smart_exit_min_pendings,
                    ),
                    (
                        "smart_exit_pending_scope",
                        self.smart_exit_pending_scope != def.smart_exit_pending_scope,
                    ),
                    (
                        "smart_exit_pending_min_dist",
                        d(
                            self.smart_exit_pending_min_dist,
                            def.smart_exit_pending_min_dist,
                        ),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "smart_exit",
                            format!("{pole} nie działa — włącz `smart_exit`"),
                        );
                    }
                }
            }

            if !self.trail_adaptive_enabled {
                for (pole, rozna) in [
                    (
                        "trail_adaptive_runners_only",
                        self.trail_adaptive_runners_only != def.trail_adaptive_runners_only,
                    ),
                    (
                        "trail_adaptive_window_s",
                        d(self.trail_adaptive_window_s, def.trail_adaptive_window_s),
                    ),
                    (
                        "trail_adaptive_min_samples",
                        self.trail_adaptive_min_samples != def.trail_adaptive_min_samples,
                    ),
                    (
                        "trail_adaptive_trend_er",
                        d(self.trail_adaptive_trend_er, def.trail_adaptive_trend_er),
                    ),
                    (
                        "trail_adaptive_reversal_er",
                        d(
                            self.trail_adaptive_reversal_er,
                            def.trail_adaptive_reversal_er,
                        ),
                    ),
                    (
                        "trail_adaptive_trend_gap_mult",
                        d(
                            self.trail_adaptive_trend_gap_mult,
                            def.trail_adaptive_trend_gap_mult,
                        ),
                    ),
                    (
                        "trail_adaptive_chop_gap_mult",
                        d(
                            self.trail_adaptive_chop_gap_mult,
                            def.trail_adaptive_chop_gap_mult,
                        ),
                    ),
                    (
                        "trail_adaptive_reversal_gap_mult",
                        d(
                            self.trail_adaptive_reversal_gap_mult,
                            def.trail_adaptive_reversal_gap_mult,
                        ),
                    ),
                    (
                        "trail_adaptive_fast_vol_s",
                        d(
                            self.trail_adaptive_fast_vol_s,
                            def.trail_adaptive_fast_vol_s,
                        ),
                    ),
                    (
                        "trail_adaptive_slow_vol_s",
                        d(
                            self.trail_adaptive_slow_vol_s,
                            def.trail_adaptive_slow_vol_s,
                        ),
                    ),
                    (
                        "trail_adaptive_vol_ratio",
                        d(self.trail_adaptive_vol_ratio, def.trail_adaptive_vol_ratio),
                    ),
                    (
                        "trail_adaptive_vol_favorable_mult",
                        d(
                            self.trail_adaptive_vol_favorable_mult,
                            def.trail_adaptive_vol_favorable_mult,
                        ),
                    ),
                    (
                        "trail_adaptive_vol_adverse_mult",
                        d(
                            self.trail_adaptive_vol_adverse_mult,
                            def.trail_adaptive_vol_adverse_mult,
                        ),
                    ),
                    (
                        "trail_adaptive_min_peak",
                        d(self.trail_adaptive_min_peak, def.trail_adaptive_min_peak),
                    ),
                    (
                        "trail_adaptive_min_gap",
                        d(self.trail_adaptive_min_gap, def.trail_adaptive_min_gap),
                    ),
                    (
                        "trail_adaptive_max_gap",
                        d(self.trail_adaptive_max_gap, def.trail_adaptive_max_gap),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "trail_adaptive_enabled",
                            format!("{pole} nie działa — włącz `trail_adaptive_enabled`"),
                        );
                    }
                }
            } else {
                let oparty_na_luce = |m: TrailMode| {
                    matches!(m, TrailMode::Gap | TrailMode::Atr | TrailMode::Chandelier)
                };
                let ma_sciezke = oparty_na_luce(self.trail_mode)
                    || oparty_na_luce(self.trail_runner_mode)
                    || (self.riskfree_enabled
                        && self.riskfree_runner_stop == RiskFreeRunnerStop::TrailGap);
                if !ma_sciezke {
                    zglos(
                        "trail_adaptive_enabled",
                        "trail_mode|trail_runner_mode",
                        "adaptacja mnoży lukę tylko w trybach Gap/Atr/Chandelier; obecna konfiguracja nie ma takiej ścieżki"
                            .into(),
                    );
                }
            }

            if !self.trail_sr_enabled {
                for (pole, rozna) in [
                    ("trail_sr_scope", self.trail_sr_scope != def.trail_sr_scope),
                    (
                        "trail_sr_activation",
                        self.trail_sr_activation != def.trail_sr_activation,
                    ),
                    (
                        "trail_sr_min_gain",
                        d(self.trail_sr_min_gain, def.trail_sr_min_gain),
                    ),
                    (
                        "trail_sr_min_dist_price",
                        d(self.trail_sr_min_dist_price, def.trail_sr_min_dist_price),
                    ),
                    (
                        "trail_sr_min_prominence_atr",
                        d(
                            self.trail_sr_min_prominence_atr,
                            def.trail_sr_min_prominence_atr,
                        ),
                    ),
                    (
                        "trail_sr_offset_atr_mult",
                        d(self.trail_sr_offset_atr_mult, def.trail_sr_offset_atr_mult),
                    ),
                    (
                        "trail_sr_offset_spread_mult",
                        d(
                            self.trail_sr_offset_spread_mult,
                            def.trail_sr_offset_spread_mult,
                        ),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "trail_sr_enabled",
                            format!("{pole} nie działa — włącz `trail_sr_enabled`"),
                        );
                    }
                }
            } else if self.trail_sr_activation != TrailSrActivation::Gain
                && d(self.trail_sr_min_gain, def.trail_sr_min_gain)
            {
                zglos(
                    "trail_sr_min_gain",
                    "trail_sr_activation",
                    "próg zysku nie działa — czytany wyłącznie przy \
                     `trail_sr_activation = Gain`"
                        .into(),
                );
            }
            let sr_dynamiczny = self.trail_sr_min_prominence_atr > 0.0
                || self.trail_sr_offset_atr_mult > 0.0
                || self.trail_sr_offset_spread_mult > 0.0;
            if !sr_dynamiczny && self.trail_sr_atr_period != def.trail_sr_atr_period {
                zglos(
                    "trail_sr_atr_period",
                    "trail_sr_min_prominence_atr|trail_sr_offset_atr_mult|trail_sr_offset_spread_mult",
                    "okres ATR jest czytany dopiero po włączeniu co najmniej jednej \
                     dynamicznej osi S/R"
                        .into(),
                );
            }

            if !self.trend_filter_enabled {
                for (pole, rozna) in [
                    (
                        "trend_filter_window_h",
                        d(self.trend_filter_window_h, def.trend_filter_window_h),
                    ),
                    (
                        "trend_filter_drop_pct",
                        d(self.trend_filter_drop_pct, def.trend_filter_drop_pct),
                    ),
                    (
                        "trend_filter_mode",
                        self.trend_filter_mode != def.trend_filter_mode,
                    ),
                    (
                        "trend_filter_shrink",
                        d(self.trend_filter_shrink, def.trend_filter_shrink),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "trend_filter_enabled",
                            format!("{pole} nie działa — włącz `trend_filter_enabled`"),
                        );
                    }
                }
            }

            if self.vol_size_mode == VolSizeMode::Off {
                if d(self.vol_size_target, def.vol_size_target) {
                    zglos(
                        "vol_size_target",
                        "vol_size_mode",
                        "docelowy zasięg nie działa — ustaw `vol_size_mode = Target`".into(),
                    );
                }
                if self.vol_size_percentile_okno != def.vol_size_percentile_okno {
                    zglos(
                        "vol_size_percentile_okno",
                        "vol_size_mode",
                        "okno percentyla nie działa — ustaw `vol_size_mode = Percentile`".into(),
                    );
                }
            }

            if !self.rearm_grid_on_return {
                for (pole, rozna) in [
                    ("rearm_keep_empty_alive", self.rearm_keep_empty_alive),
                    (
                        "spp_blocks_rearm_when_flat",
                        self.spp_blocks_rearm_when_flat,
                    ),
                    (
                        "rearm_min_basket_profit",
                        d(self.rearm_min_basket_profit, def.rearm_min_basket_profit),
                    ),
                    (
                        "rearm_max_times",
                        self.rearm_max_times != def.rearm_max_times,
                    ),
                    (
                        "rearm_min_gap_min",
                        d(self.rearm_min_gap_min, def.rearm_min_gap_min),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "rearm_grid_on_return",
                            format!("{pole} nie działa — włącz `rearm_grid_on_return`"),
                        );
                    }
                }
            }

            if self.slhit_pause_n == 0 {
                for (pole, rozna) in [
                    (
                        "slhit_pause_min",
                        d(self.slhit_pause_min, def.slhit_pause_min),
                    ),
                    (
                        "slhit_pause_lot_mult",
                        d(self.slhit_pause_lot_mult, def.slhit_pause_lot_mult),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "slhit_pause_n",
                            format!("{pole} nie działa — ustaw próg `slhit_pause_n`"),
                        );
                    }
                }
            }

            if self.regime_filter == RegimeFilter::Off {
                for (pole, rozna) in [
                    (
                        "regime_ma_hours",
                        d(self.regime_ma_hours, def.regime_ma_hours),
                    ),
                    ("regime_cena", self.regime_cena != def.regime_cena),
                    ("regime_miara", self.regime_miara != def.regime_miara),
                    ("regime_pilnuj_limitow", self.regime_pilnuj_limitow),
                    (
                        "regime_percentyl",
                        d(self.regime_percentyl, def.regime_percentyl),
                    ),
                    (
                        "regime_strefa_martwa",
                        d(self.regime_strefa_martwa, def.regime_strefa_martwa),
                    ),
                    ("regime_okno2_h", d(self.regime_okno2_h, def.regime_okno2_h)),
                    (
                        "regime_zmiennosc_min",
                        d(self.regime_zmiennosc_min, def.regime_zmiennosc_min),
                    ),
                    (
                        "regime_zmiennosc_max",
                        d(self.regime_zmiennosc_max, def.regime_zmiennosc_max),
                    ),
                    (
                        "regime_gdy_rozerwany",
                        self.regime_gdy_rozerwany != def.regime_gdy_rozerwany,
                    ),
                    ("regime_soft", self.regime_soft),
                    (
                        "regime_soft_units_mult",
                        d(self.regime_soft_units_mult, def.regime_soft_units_mult),
                    ),
                    (
                        "regime_soft_lot_mult",
                        d(self.regime_soft_lot_mult, def.regime_soft_lot_mult),
                    ),
                    (
                        "regime_soft_max_positions",
                        self.regime_soft_max_positions != def.regime_soft_max_positions,
                    ),
                    (
                        "regime_soft_risk_mult",
                        d(self.regime_soft_risk_mult, def.regime_soft_risk_mult),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "regime_filter",
                            format!("{pole} nie działa — ustaw `regime_filter` inny niż Off"),
                        );
                    }
                }
            }

            if self.pyramid_after_stage == 0 {
                for (pole, rozna) in [
                    (
                        "pyramid_regime_lookback",
                        self.pyramid_regime_lookback != def.pyramid_regime_lookback,
                    ),
                    (
                        "pyramid_regime_max_fast_pct",
                        d(
                            self.pyramid_regime_max_fast_pct,
                            def.pyramid_regime_max_fast_pct,
                        ),
                    ),
                    (
                        "pyramid_min_equity_mult",
                        d(self.pyramid_min_equity_mult, def.pyramid_min_equity_mult),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "pyramid_after_stage",
                            format!("{pole} nie działa — ustaw `pyramid_after_stage`"),
                        );
                    }
                }
            }

            if self.fast_addon_move_usd <= 0.0 {
                for (pole, rozna) in [
                    (
                        "fast_addon_window_s",
                        d(self.fast_addon_window_s, def.fast_addon_window_s),
                    ),
                    ("fast_addon_max", self.fast_addon_max != def.fast_addon_max),
                    (
                        "fast_addon_lot_mult",
                        d(self.fast_addon_lot_mult, def.fast_addon_lot_mult),
                    ),
                    (
                        "fast_addon_min_stage",
                        self.fast_addon_min_stage != def.fast_addon_min_stage,
                    ),
                    (
                        "fast_addon_cooldown_s",
                        d(self.fast_addon_cooldown_s, def.fast_addon_cooldown_s),
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "fast_addon_move_usd",
                            format!("{pole} nie działa — ustaw próg `fast_addon_move_usd`"),
                        );
                    }
                }
            }

            if self.exit_round_dist <= 0.0 && d(self.exit_round_step, def.exit_round_step) {
                zglos(
                    "exit_round_step",
                    "exit_round_dist",
                    "krok okrągłych poziomów nie działa — ustaw `exit_round_dist`".into(),
                );
            }

            if !self.riskfree_enabled {
                for (pole, rozna) in [
                    (
                        "riskfree_trigger_usd",
                        d(self.riskfree_trigger_usd, def.riskfree_trigger_usd),
                    ),
                    (
                        "riskfree_trigger_r",
                        d(self.riskfree_trigger_r, def.riskfree_trigger_r),
                    ),
                    (
                        "riskfree_keep_units",
                        self.riskfree_keep_units != def.riskfree_keep_units,
                    ),
                    (
                        "riskfree_be_offset",
                        d(self.riskfree_be_offset, def.riskfree_be_offset),
                    ),
                    (
                        "riskfree_runner_gap",
                        d(self.riskfree_runner_gap, def.riskfree_runner_gap),
                    ),
                    (
                        "riskfree_runner_target",
                        self.riskfree_runner_target != def.riskfree_runner_target,
                    ),
                    (
                        "riskfree_runner_stop",
                        self.riskfree_runner_stop != def.riskfree_runner_stop,
                    ),
                    (
                        "pending_cancel_on_riskfree",
                        self.pending_cancel_on_riskfree != def.pending_cancel_on_riskfree,
                    ),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "riskfree_enabled",
                            format!("{pole} nie działa — włącz `riskfree_enabled`"),
                        );
                    }
                }
            }

            if self.tp_price_front_run_usd < 0.0 {
                zglos(
                    "tp_price_front_run_usd",
                    "tp_price_front_run_usd > 0",
                    "ujemny front-run jest normalizowany do 0 i nie działa — ustaw wartość > 0"
                        .into(),
                );
            }

            if self.tp_schedule != TpSchedule::OfficialPct && self.official_pct != def.official_pct
            {
                zglos(
                    "official_pct",
                    "tp_schedule",
                    "harmonogram procentowy nie działa — ustaw `tp_schedule = OfficialPct`".into(),
                );
            }
            if self.tp_schedule != TpSchedule::OfficialCounts
                && self.official_counts != def.official_counts
            {
                zglos(
                    "official_counts",
                    "tp_schedule",
                    "harmonogram liczbowy nie działa — ustaw `tp_schedule = OfficialCounts`".into(),
                );
            }

            if !self.parser_geometryczny && self.parser_min_pewnosc > 0.0 {
                zglos(
                    "parser_min_pewnosc",
                    "parser_geometryczny",
                    "próg pewności odczytu nie działa — włącz `parser_geometryczny`".into(),
                );
            }

            if !self.ea_enabled {
                for (pole, rozna) in [
                    ("ea_tick_s", d(self.ea_tick_s, def.ea_tick_s)),
                    ("ea_state_src", self.ea_state_src != def.ea_state_src),
                    (
                        "ea_defense_enter",
                        d(self.ea_defense_enter, def.ea_defense_enter),
                    ),
                    (
                        "ea_defense_exit",
                        d(self.ea_defense_exit, def.ea_defense_exit),
                    ),
                    (
                        "ea_offense_enter",
                        d(self.ea_offense_enter, def.ea_offense_enter),
                    ),
                    (
                        "ea_offense_exit",
                        d(self.ea_offense_exit, def.ea_offense_exit),
                    ),
                    (
                        "ea_state_dwell_s",
                        d(self.ea_state_dwell_s, def.ea_state_dwell_s),
                    ),
                    (
                        "ea_state_ratchet",
                        self.ea_state_ratchet != def.ea_state_ratchet,
                    ),
                    (
                        "ea_state_journal",
                        self.ea_state_journal != def.ea_state_journal,
                    ),
                    ("ea_dozor_sl", self.ea_dozor_sl != def.ea_dozor_sl),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "ea_enabled",
                            format!("{pole} nie działa — włącz `ea_enabled` (warstwa EA)"),
                        );
                    }
                }
            }

            if self.ea_enabled {
                if self.ea_defense_enter <= 0.0 && self.ea_defense_exit > 0.0 {
                    zglos(
                        "ea_defense_exit",
                        "ea_defense_enter",
                        "próg wyjścia z obrony nie działa — `ea_defense_enter = 0` \
                         znaczy OBRONA NIGDY, więc nie ma z czego wychodzić"
                            .into(),
                    );
                }
                if self.ea_offense_enter <= 0.0 && self.ea_offense_exit > 0.0 {
                    zglos(
                        "ea_offense_exit",
                        "ea_offense_enter",
                        "próg wyjścia z agresji nie działa — `ea_offense_enter = 0` \
                         znaczy AGRESJA NIGDY, więc nie ma z czego wychodzić"
                            .into(),
                    );
                }
                if self.ea_defense_enter <= 0.0
                    && self.ea_offense_enter <= 0.0
                    && self.ea_state_dwell_s > 0.0
                {
                    zglos(
                        "ea_state_dwell_s",
                        "ea_defense_enter",
                        "minimalny czas trwania stanu nie ma czego pilnować — oba progi \
                         wejścia (`ea_defense_enter`, `ea_offense_enter`) są zerowe, \
                         więc stan zostaje `Neutral` na zawsze"
                            .into(),
                    );
                }
            }

            if !self.ea_enabled {
                for (pole, rozna) in [
                    (
                        "ea_lot_z_wolnego_marginesu",
                        d(
                            self.ea_lot_z_wolnego_marginesu,
                            def.ea_lot_z_wolnego_marginesu,
                        ),
                    ),
                    (
                        "ea_stop_dokladek_przy_stracie",
                        d(
                            self.ea_stop_dokladek_przy_stracie,
                            def.ea_stop_dokladek_przy_stracie,
                        ),
                    ),
                    (
                        "ea_stop_dokladek_powrot",
                        d(self.ea_stop_dokladek_powrot, def.ea_stop_dokladek_powrot),
                    ),
                    (
                        "ea_redukcja_przy_zageszczeniu",
                        d(
                            self.ea_redukcja_przy_zageszczeniu,
                            def.ea_redukcja_przy_zageszczeniu,
                        ),
                    ),
                    ("ea_stan_dnia", self.ea_stan_dnia != def.ea_stan_dnia),
                ] {
                    if rozna {
                        zglos(
                            pole,
                            "ea_enabled",
                            format!(
                                "{pole} (rodzina A) działa tylko przy `ea_enabled` \
                                 ALBO w trybie AUTO-EA — w zwykłym AUTO/MANUAL/AI \
                                 jest martwe co do bitu"
                            ),
                        );
                    }
                }
            }

            if self.ea_stop_dokladek_przy_stracie <= 0.0 && self.ea_stop_dokladek_powrot > 0.0 {
                zglos(
                    "ea_stop_dokladek_powrot",
                    "ea_stop_dokladek_przy_stracie",
                    "próg powrotu nie działa — `ea_stop_dokladek_przy_stracie = 0` \
                     znaczy STOP DOKŁADKOM NIGDY, więc nie ma czego zdejmować"
                        .into(),
                );
            }
            if self.ea_stop_dokladek_przy_stracie > 0.0
                && self.ea_stop_dokladek_powrot > self.ea_stop_dokladek_przy_stracie
            {
                zglos(
                    "ea_stop_dokladek_powrot",
                    "ea_stop_dokladek_przy_stracie",
                    "histereza odwrócona — próg powrotu musi być MNIEJ dotkliwy niż \
                     próg wejścia, inaczej weto zdejmuje się w tej samej chwili, \
                     w której się zatrzasnęło"
                        .into(),
                );
            }
            if self.ea_redukcja_przy_zageszczeniu <= 0.0 && self.ea_zageszczenie_podloga > 0.0 {
                zglos(
                    "ea_zageszczenie_podloga",
                    "ea_redukcja_przy_zageszczeniu",
                    "podłoga mnożnika nie działa — nachylenie zagęszczenia jest zerowe, \
                     więc mnożnik zostaje 1,0 na zawsze"
                        .into(),
                );
            }
            if matches!(self.ea_stan_dnia, EaStanDnia::Off)
                && (self.ea_stan_dnia_prog_sl != def.ea_stan_dnia_prog_sl
                    || d(
                        self.ea_stan_dnia_jednostki_mult,
                        def.ea_stan_dnia_jednostki_mult,
                    ))
            {
                zglos(
                    "ea_stan_dnia_prog_sl",
                    "ea_stan_dnia",
                    "próg i mnożnik stanu dnia nie działają — `ea_stan_dnia = Off`".into(),
                );
            }
            if self.ea_stan_dnia_jednostki_mult > 1.0 {
                zglos(
                    "ea_stan_dnia_jednostki_mult",
                    "ea_stan_dnia",
                    "mnożnik > 1 jest PRZYCINANY do 1,0 — „ryzyko nie rośnie po stracie\" \
                     jest niezmiennikiem osi A4, nie ustawieniem"
                        .into(),
                );
            }
        }

        v
    }

    pub fn pulapki_konfiguracji(&self) -> Vec<String> {
        let mut v = Vec::new();

        if self.trail_adaptive_enabled {
            let nie_dodatnie_lub_nan = |x: f64| !x.is_finite() || x <= 0.0;
            if nie_dodatnie_lub_nan(self.trail_adaptive_window_s) {
                v.push("trail_adaptive_window_s: okno musi być skończone i > 0; w przeciwnym razie adaptacja nie dostaje snapshotu i pozostaje no-opem.".into());
            }
            if self.trail_adaptive_min_samples < 2 {
                v.push("trail_adaptive_min_samples: wartości < 2 są normalizowane do 2; wpisana liczba nie opisuje więc faktycznego działania.".into());
            }
            if nie_dodatnie_lub_nan(self.trail_adaptive_fast_vol_s)
                || nie_dodatnie_lub_nan(self.trail_adaptive_slow_vol_s)
            {
                v.push("trail_adaptive_fast_vol_s/trail_adaptive_slow_vol_s: oba okna prędkości muszą być skończone i > 0, inaczej mnożnik ekspansji zmienności nie zadziała.".into());
            } else if self.trail_adaptive_fast_vol_s >= self.trail_adaptive_slow_vol_s {
                v.push("trail_adaptive_fast_vol_s musi być krótsze niż trail_adaptive_slow_vol_s; inaczej iloraz prędkości nie mierzy ekspansji krótkoterminowej względem tła.".into());
            }
            if !self.trail_adaptive_trend_er.is_finite()
                || !(0.0..=1.0).contains(&self.trail_adaptive_trend_er)
                || !self.trail_adaptive_reversal_er.is_finite()
                || !(0.0..=1.0).contains(&self.trail_adaptive_reversal_er)
            {
                v.push("trail_adaptive_*_er: progi ER muszą należeć do [0, 1]; silnik normalizuje wartości spoza zakresu, więc preset nie działałby dokładnie tak, jak zapisano.".into());
            }
            for (pole, wartosc) in [
                ("trail_adaptive_trend_gap_mult", self.trail_adaptive_trend_gap_mult),
                ("trail_adaptive_chop_gap_mult", self.trail_adaptive_chop_gap_mult),
                ("trail_adaptive_reversal_gap_mult", self.trail_adaptive_reversal_gap_mult),
                ("trail_adaptive_vol_favorable_mult", self.trail_adaptive_vol_favorable_mult),
                ("trail_adaptive_vol_adverse_mult", self.trail_adaptive_vol_adverse_mult),
            ] {
                if !wartosc.is_finite() || wartosc < 0.0 {
                    v.push(format!(
                        "{pole}: mnożnik musi być skończony i >= 0; wartość ujemna/NaN jest normalizowana i nie opisuje faktycznego działania."
                    ));
                }
            }
            if !self.trail_adaptive_vol_ratio.is_finite()
                || self.trail_adaptive_vol_ratio < 0.0
            {
                v.push("trail_adaptive_vol_ratio: próg musi być skończony i >= 0; zero jawnie wyłącza drugi mnożnik zmienności.".into());
            }
            if self.trail_adaptive_min_gap > 0.0
                && self.trail_adaptive_max_gap > 0.0
                && self.trail_adaptive_min_gap > self.trail_adaptive_max_gap
            {
                v.push("trail_adaptive_min_gap > trail_adaptive_max_gap: odwrócone klamry są bezpiecznie normalizowane do min_gap, ale faktycznie tworzą jedną stałą podłogę zamiast zakresu.".into());
            }
        }

        if self.pending_relot_reconcile_target {
            v.push("pending_relot_reconcile_target: pełny checked plan zastępuje pending_relot_wg_planu; sync/rearm rewalidują legalny wolumen obu trybów. RequiresReview blokuje nowe wejścia danego poziomu, nie jest kolejką zleceń po restarcie.".into());
            if !self.pending_relot_on_balance {
                v.push("pending_relot_reconcile_target: cykliczny relot jest wyłączony przez pending_relot_on_balance=false; kontrola bezpiecznego sync/rearm i zapisane RequiresReview nadal obowiązują.".into());
            }
        }

        if self.entry_edit_geometry_v2 {
            v.push("entry_edit_geometry_v2: wymaga źródłowego snapshotu. Kosmetyczna edycja nie zmienia zleceń. Working/partial geometry i niepotwierdzony cancel/fill przechodzą w RequiresReview tylko danego koszyka; obecny live nie potwierdza automatycznej wymiany geometrii. Zapisany review nie jest kolejką replay ani gwarancją atomic restart.".into());
        }

        if self.order_volume_contract_v2 {
            if !self.lot_min.is_finite()
                || self.lot_min <= 0.0
                || !self.lot_max.is_finite()
                || self.lot_max < 0.0
                || !self.lot_max_z_salda.is_finite()
                || self.lot_max_z_salda < 0.0
                || (self.lot_max > 0.0 && self.lot_max < self.lot_min)
            {
                v.push("order_volume_contract_v2: niepoprawne min/max lub dzielnik kapitału; nowe wejścia zostaną odrzucone (fail-closed), bez zamiany granic i bez podnoszenia resztkowego wolumenu.".into());
            }
            v.push("order_volume_contract_v2 wymaga znanego dodatniego min/step/max brokera i kroku reprezentowalnego do 8 miejsc. lot_max=0 wyłącza tylko limit użytkownika, nie limit brokera. Wolumen jest zaokrąglany w dół; zlecenie poniżej minimum nie powstanie.".into());
        }

        if self.tp_source == TpSource::PriceOnly
            && self.tp_unindexed_pips_require_price
            && !self.tp_price_only_strict
        {
            v.push("PriceOnly ma aktywny wyjątek legacy: +N PIPS HIT może awansować etap po kontroli ceny. tp_price_only_strict=true usuwa ten wyjątek, nie wyłączając RF/SPP/SL.".into());
        }

        if self.tp_price_front_run_usd > 0.0 && matches!(self.tp_source, TpSource::SignalOnly) {
            v.push(format!(
                "`tp_source = SignalOnly`, ale `tp_price_front_run_usd = {}` celowo \
                 dodaje niezależną drogę CENOWĄ dla koszyków z pozycją. \
                 Telegram nie jest potrzebny; zero przywraca czyste SignalOnly.",
                self.tp_price_front_run_usd
            ));
        }
        if self.tp_price_front_run_usd > 0.0 && self.assign_tp_per_position {
            v.push(format!(
                "`tp_price_front_run_usd = {}` wykonuje zarządzanie etapem przed TP, \
                 ale `assign_tp_per_position = true` pozostawia brokerowe TP na pełnym \
                 poziomie. Gdy zlecenie front-run zostanie odrzucone, broker zamknie \
                 pozycję dopiero na jej zwykłym TP; dziennik musi pokazać wynik close.",
                self.tp_price_front_run_usd
            ));
        }

        if matches!(self.trail_mode, TrailMode::Off)
            && !matches!(self.trail_runner_mode, TrailMode::Off)
            && (self.trail_split || (self.risk_free_trail && !self.riskfree_enabled))
        {
            v.push(format!(
                "`trail_mode = Off` NIE wyłącza trailingu runnerów — \
                 `trail_runner_mode = {:?}` może działać przez trail_split lub kanałowy risk_free_trail; próg wynosi {} $ zysku. \
                 Zmierzony koszt tej niespodzianki: 1054 $.",
                self.trail_runner_mode, self.trail_runner_start
            ));
        }

        {
            let bez_celu = matches!(
                self.risk_free_runner_target,
                RiskFreeRunnerTarget::NoTpTrailOnly
            );
            let bez_trailingu = matches!(self.trail_mode, TrailMode::Off)
                && matches!(self.trail_runner_mode, TrailMode::Off);
            let be_poza_zasiegiem = self.be_offset >= 9.0;
            let bez_zegara =
                self.riskfree_runner_max_hold_min <= 0.0 || self.zegar_runnera_martwy();
            if bez_celu && bez_trailingu && be_poza_zasiegiem && bez_zegara {
                v.push(format!(
                    "RUNNER NIE MA WYJŚCIA: `risk_free_runner_target = NoTpTrailOnly` \
                     zdejmuje cel i deleguje pilnowanie do trailingu, \
                     `trail_mode`/`trail_runner_mode = Off` trailing wyłączają, \
                     `be_offset = {}` sprawia, że `sl_is_valid` odrzuca stop na BE, \
                     a limit `riskfree_runner_max_hold_min` nie obowiązuje. \
                     Po RISK FREE pozycja nie ma ANI celu, ANI zapadki, ANI terminu. \
                     Najtańsze wyjście: `runner_max_hold_bez_reguly = true` \
                     (nie stawia sufitu i nie dotyka stopu).",
                    self.be_offset
                ));
            }
        }

        if self.equity_floor_pct > 0.0 {
            v.push(format!(
                "`equity_floor_pct = {}` blokuje NOWE wejścia poniżej progu i jest \
                 STANEM POCHŁANIAJĄCYM: konto zamiera i nie wznowi się samo, \
                 wymaga ręcznej decyzji. Na VPS oznacza cichy stop.",
                self.equity_floor_pct
            ));
        }

        if self.max_dd_pct > 0.0 || self.max_dd_usd > 0.0 {
            v.push(format!(
                "`max_dd_pct = {}` / `max_dd_usd = {}` ZAMYKA WSZYSTKO po cenie dna \
                 i wstrzymuje wejścia (także dokładki i odbudowę siatki). Zmierzone \
                 na kwietniu-maju: już DWA zadziałania pogarszają i zysk, i dno equity \
                 (30,76 $ wobec 31,56 $ bez hamulca), a próg 40 % zbija dno do 6,93 $. \
                 Rozważ `max_portfolio_risk_pct` — dławi zamiast zatrzymywać.",
                self.max_dd_pct, self.max_dd_usd
            ));
        }

        if (self.max_dd_pct > 0.0 || self.max_dd_usd > 0.0)
            && matches!(self.dd_guard_scope, DdGuardScope::LifetimePeakDailyReset)
        {
            v.push(
                "`dd_guard_scope = LifetimePeakDailyReset` mierzy obsunięcie od szczytu \
                 WSZECH CZASÓW, a blokadę zdejmuje o północy. Jeśli equity trwale spadło \
                 poniżej progu, strażnik zapala się PONOWNIE na pierwszym ticku każdego \
                 dnia — zwolnienie jest pozorne, blokada dożywotnia."
                    .to_string(),
            );
        }

        if (self.dd_soft_pct > 0.0 || self.dd_hard_pct > 0.0) && self.risk_per_basket_pct <= 0.0 {
            v.push(
                "`dd_soft_pct`/`dd_hard_pct` mnożą BUDŻET RYZYKA KOSZYKA, a \
                 `risk_per_basket_pct = 0` znaczy, że tego budżetu nie ma. \
                 Dławik jest wtedy martwy — ustaw `risk_per_basket_pct` albo użyj \
                 `max_portfolio_risk_pct`."
                    .to_string(),
            );
        }

        if self.max_open_positions > 0 && !self.enforce_position_limit_on_fill {
            v.push(format!(
                "`max_open_positions = {}` jest sprawdzany TYLKO w chwili nadejścia \
                 sygnału. Wypełnienia wiszących limitów już mu nie podlegają, więc \
                 jeden koszyk potrafi mieć więcej pozycji niż limit. \
                 Włącz `enforce_position_limit_on_fill`, jeśli limit ma naprawdę obowiązywać.",
                self.max_open_positions
            ));
        }

        if self.ea_enabled && self.ea_tick_s <= 0.0 {
            v.push(
                "`ea_tick_s = 0` NIE znaczy \"zegar wyłączony\" — znaczy \
                 \"BEZ WŁASNEGO ZEGARA, czyli puls na KAŻDYM tiku\". To jest \
                 najdroższe z możliwych ustawień warstwy EA: zmierzone +162 % \
                 czasu przebiegu (STORM 17–24.08: 2,23 s → 5,83 s), a na żywo \
                 dodatkowo jeden odczyt rachunku z terminala na tik. \
                 Kadencja `1` kosztuje +23 %, `5` — +5 % (podłoga szumu)."
                    .to_string(),
            );
        }

        v
    }
}

pub fn nieznane_pola_ustawien(surowy: &serde_json::Value) -> Vec<String> {
    let Some(fields) = surowy.get("settings").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let known = settings_deserialization_field_names();
    let mut unknown: Vec<String> = fields
        .keys()
        .filter(|key| !known.contains(key.as_str()))
        .cloned()
        .collect();
    unknown.sort();
    unknown
}

fn settings_deserialization_field_names() -> &'static std::collections::HashSet<&'static str> {
    use serde::de::{self, Visitor};
    static KNOWN: std::sync::OnceLock<std::collections::HashSet<&'static str>> =
        std::sync::OnceLock::new();
    KNOWN.get_or_init(|| {
        struct Names<'a>(&'a mut std::collections::HashSet<&'static str>);
        impl<'de> de::Deserializer<'de> for Names<'_> {
            type Error = de::value::Error;
            fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Self::Error> {
                Err(de::Error::custom(
                    "field-name discovery has no input values",
                ))
            }
            fn deserialize_struct<V: Visitor<'de>>(
                self,
                _: &'static str,
                fields: &'static [&'static str],
                _: V,
            ) -> Result<V::Value, Self::Error> {
                self.0.extend(fields.iter().copied());
                Err(de::Error::custom(
                    "field names captured; no Settings value requested",
                ))
            }
            serde::forward_to_deserialize_any! {
                bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
                bytes byte_buf option unit unit_struct newtype_struct seq tuple
                tuple_struct map enum identifier ignored_any
            }
        }
        let mut names = std::collections::HashSet::new();
        let _ = Settings::deserialize(Names(&mut names));
        assert!(
            !names.is_empty(),
            "Settings no longer exposes serde struct field names"
        );
        names
    })
}

#[cfg(test)]
fn legacy_unknown_settings_by_value(surowy: &serde_json::Value) -> Vec<String> {
    let Some(obiekt) = surowy.get("settings").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let wzorzec = match serde_json::to_value(Settings::default()) {
        Ok(serde_json::Value::Object(m)) => m,
        _ => return Vec::new(),
    };
    let podejrzani: Vec<String> = obiekt
        .keys()
        .filter(|k| !wzorzec.contains_key(*k))
        .cloned()
        .collect();
    if podejrzani.is_empty() {
        return Vec::new();
    }

    let pelny: Settings =
        serde_json::from_value(serde_json::Value::Object(obiekt.clone())).unwrap_or_default();
    let mut out = Vec::new();
    for k in podejrzani {
        let mut bez = obiekt.clone();
        bez.remove(&k);
        let okrojony: Settings =
            serde_json::from_value(serde_json::Value::Object(bez)).unwrap_or_default();
        if okrojony == pelny {
            out.push(k);
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod recognized_settings_names_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn genuine_default_alias_reproduces_legacy_false_alarm_but_is_now_known() {
        let defaults = serde_json::to_value(Settings::default()).unwrap();
        for (alias, canonical) in [
            ("regime_range_mute_usd", "regime_zmiennosc_max"),
            ("regime_range_mute_mode", "regime_gdy_rozerwany"),
        ] {
            let doc = json!({"settings": {alias: defaults[canonical].clone()}});
            assert!(serde_json::from_value::<Settings>(doc["settings"].clone()).is_ok());
            assert_eq!(
                legacy_unknown_settings_by_value(&doc),
                vec![alias.to_string()],
                "historical heuristic really misclassified this accepted default alias"
            );
            assert!(nieznane_pola_ustawien(&doc).is_empty());
        }
        assert!(
            nieznane_pola_ustawien(&json!({"settings": defaults})).is_empty(),
            "all canonical serialized names must remain recognized"
        );
    }

    #[test]
    fn name_classification_is_independent_of_bad_values_elsewhere() {
        let doc = json!({"settings": {"lot_fixed": {"invalid": "type"},
            "regime_range_mute_usd": 0.0,
            "regime_range_mute_mode": {"invalid": "enum"},
            "misspelled_runner_axis": 0, "future_unknown_axis": true}});
        assert!(serde_json::from_value::<Settings>(doc["settings"].clone()).is_err());
        assert_eq!(
            nieznane_pola_ustawien(&doc),
            vec!["future_unknown_axis", "misspelled_runner_axis"]
        );
    }

    #[test]
    fn recognized_aliases_do_not_make_ambiguous_duplicates_valid() {
        let doc = json!({"settings": {"regime_range_mute_usd": 0.0,
                                      "regime_zmiennosc_max": 0.0}});
        assert!(nieznane_pola_ustawien(&doc).is_empty());
        assert!(
            serde_json::from_value::<Settings>(doc["settings"].clone()).is_err(),
            "canonical plus alias is still an ambiguous duplicate field"
        );
        assert_eq!(
            nieznane_pola_ustawien(&json!({"settings": {"truely_unknown": 0}})),
            vec!["truely_unknown"]
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "domyslny_format")]
    pub format: String,
    pub settings: Settings,
    #[serde(default)]
    pub ea: Option<serde_json::Value>,
}

fn domyslny_format() -> String {
    "ATFX".to_string()
}

fn prawda() -> bool {
    true
}

fn jeden_f64() -> f64 {
    1.0
}

fn trzy_u32() -> u32 {
    3
}

fn regime_cena_domyslna() -> RegimeCena {
    RegimeCena::Rynkowa
}

fn regime_miara_domyslna() -> RegimeMiara {
    RegimeMiara::Srednia
}

fn regime_percentyl_domyslny() -> f64 {
    50.0
}

fn regime_gdy_rozerwany_domyslny() -> RegimeGdyRozerwany {
    RegimeGdyRozerwany::Milcz
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SesjaBramka {
    Sygnal,
    Wypelnienie,
    Oba,
}

fn sesja_bramka_domyslna() -> SesjaBramka {
    SesjaBramka::Sygnal
}


fn default_dedup_pelny_status() -> bool {
    true
}
fn default_edycja_wykonuje_reszte_akcji() -> bool {
    false
}
fn default_dedup_klucz_z_wartoscia() -> bool {
    false
}
fn default_edycja_sieroty_nie_otwiera() -> bool {
    false
}
fn default_entry_idempotencja() -> bool {
    true
}


fn default_rf_wymaga_wykonania() -> bool {
    false
}
fn default_market_entry_units() -> u32 {
    0
}
fn default_market_hybrid_now_units() -> u32 {
    0
}
fn default_market_hybrid_pending_units() -> u32 {
    0
}
fn default_market_hybrid_lot_mult() -> f64 {
    1.0
}
fn default_market_hybrid_max_chase_usd() -> f64 {
    0.0
}
fn default_market_hybrid_tp_stage() -> u8 {
    0
}
fn default_market_unfilled_cancel_stage() -> u8 {
    0
}
fn default_pending_cancel_on_riskfree() -> bool {
    false
}
fn default_deferred_entry_max_age_s() -> f64 {
    300.0
}
fn default_bank_all_at_stage() -> u8 {
    0
}
fn default_stat_be_prog_usd() -> f64 {
    0.0
}


fn default_units_per_level() -> bool {
    true
}

fn default_units_per_level_zone() -> bool {
    true
}


fn default_trail_sr_enabled() -> bool {
    false
}

fn default_trail_sr_scope() -> TrailSrScope {
    TrailSrScope::Runner
}

fn default_trail_sr_activation() -> TrailSrActivation {
    TrailSrActivation::Tp2
}

fn default_trail_sr_min_gain() -> f64 {
    3.0
}

fn default_trail_sr_min_dist_price() -> f64 {
    6.0
}


fn default_close_all_scope() -> CloseAllScope {
    CloseAllScope::Global
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CelZPrzeciwnego {
    Off,
    BliższaKrawedz,
    DalszaKrawedz,
    Srodek,
}

fn cel_z_przeciwnego_domyslny() -> CelZPrzeciwnego {
    CelZPrzeciwnego::Off
}

#[cfg(test)]
mod credit_balance_separate_tests {
    use super::*;
    use crate::engine::Engine;
    fn cfg() -> Settings {
        Settings {
            credit_balance_separate: true,
            odlicz_kredyt: true,
            ..Settings::default()
        }
    }
    #[test]
    fn credit_balance_separate_pu_actual_snapshot_and_quantized_lot() {
        let mut c = cfg();
        c.lot_mode_percent = true;
        c.lot_percent = 0.5;
        c.lot_min = 0.01;
        c.lot_max = 0.0;
        let mut e = Engine::new(c, 159.8);
        e.stats.equity = 459.8;
        e.stats.credit = 300.0;
        assert_eq!(e.podstawa_lota(), 159.8);
        assert_eq!(e.kredyt_odliczony_od_podstawy(), 0.0);
        assert_eq!(
            e.lot_size(e.podstawa_lota()),
            0.01,
            "minimum lot still quantizes this small account to .01"
        );
        e.cfg.credit_balance_separate = false;
        assert_eq!(e.podstawa_lota(), 0.0);
        assert_eq!(e.lot_size(e.podstawa_lota()), 0.01);
    }
    #[test]
    fn credit_balance_separate_all_capital_selectors_and_floating_loss() {
        let mut c = cfg();
        for (b, e, credit, expected) in [
            (600.0, 900.0, 300.0, [600.0, 600.0, 600.0]),
            (600.0, 800.0, 300.0, [600.0, 500.0, 500.0]),
            (600.0, 950.0, 300.0, [600.0, 650.0, 600.0]),
            (159.8, 459.8, 300.0, [159.8, 159.8, 159.8]),
        ] {
            for (base, want) in [
                PodstawaLota::Balance,
                PodstawaLota::Equity,
                PodstawaLota::MinOfBoth,
            ]
            .into_iter()
            .zip(expected)
            {
                c.lot_base = base;
                assert!((c.podstawa_lota_z_konta(b, e, credit) - want).abs() < 1e-9);
            }
        }
    }
    #[test]
    fn credit_balance_separate_off_matches_legacy_formula() {
        let mut c = cfg();
        c.credit_balance_separate = false;
        for deduct in [false, true] {
            c.odlicz_kredyt = deduct;
            for manual in [0.0, 300.0, 500.0] {
                c.kredyt_reczny = manual;
                for base in [
                    PodstawaLota::Balance,
                    PodstawaLota::Equity,
                    PodstawaLota::MinOfBoth,
                ] {
                    c.lot_base = base;
                    let raw: f64 = match base {
                        PodstawaLota::Balance => 600.0,
                        PodstawaLota::Equity => 450.0,
                        PodstawaLota::MinOfBoth => 450.0,
                    };
                    let credit: f64 = if !deduct {
                        0.0
                    } else if manual > 0.0 {
                        manual
                    } else {
                        300.0
                    };
                    assert_eq!(
                        c.podstawa_lota_z_konta(600.0, 450.0, 300.0).to_bits(),
                        (raw - credit).max(0.0).to_bits()
                    );
                }
            }
        }
    }
    #[test]
    fn credit_balance_separate_zero_credit_bitwise_parity() {
        let mut c = cfg();
        for base in [
            PodstawaLota::Balance,
            PodstawaLota::Equity,
            PodstawaLota::MinOfBoth,
        ] {
            c.lot_base = base;
            for (b, e) in [(0.0, 0.0), (600.0, 500.0), (600.0, 700.0), (-50.0, -50.0)] {
                c.credit_balance_separate = true;
                let on = c.podstawa_lota_z_konta(b, e, 0.0);
                c.credit_balance_separate = false;
                assert_eq!(on.to_bits(), c.podstawa_lota_z_konta(b, e, 0.0).to_bits());
            }
        }
    }
    #[test]
    fn credit_balance_separate_bonus_removal_manual_override_and_reload() {
        let mut c = cfg();
        c.lot_base = PodstawaLota::Equity;
        assert_eq!(c.podstawa_lota_z_konta(600.0, 900.0, 300.0), 600.0);
        assert_eq!(c.podstawa_lota_z_konta(600.0, 600.0, 0.0), 600.0);
        c.kredyt_reczny = 250.0;
        assert_eq!(c.podstawa_lota_z_konta(600.0, 900.0, 300.0), 650.0);
        c.odlicz_kredyt = false;
        assert_eq!(c.podstawa_lota_z_konta(600.0, 900.0, 300.0), 900.0);
        c.odlicz_kredyt = true;
        c.lot_base = PodstawaLota::Balance;
        assert_eq!(c.podstawa_lota_z_konta(600.0, 900.0, 300.0), 600.0);
        let mut e = Engine::new(c, 600.0);
        e.stats.equity = 900.0;
        e.stats.credit = 300.0;
        assert_eq!(e.podstawa_lota(), 600.0);
        e.cfg.credit_balance_separate = false;
        assert_eq!(e.podstawa_lota(), 350.0);
    }
    #[test]
    fn credit_balance_separate_actual_deduction_min_is_not_nominal_credit() {
        let mut e = Engine::new(cfg(), 600.0);
        e.stats.equity = 850.0;
        e.stats.credit = 300.0;
        e.cfg.lot_base = PodstawaLota::MinOfBoth;
        assert_eq!(e.kredyt_odliczony_od_podstawy(), 50.0);
        e.cfg.lot_base = PodstawaLota::Equity;
        assert_eq!(e.kredyt_odliczony_od_podstawy(), 300.0);
        e.cfg.lot_base = PodstawaLota::Balance;
        assert_eq!(e.kredyt_odliczony_od_podstawy(), 0.0);
        assert_eq!(e.cfg.saldo_wlasne(600.0, 300.0), 600.0);
    }
    #[test]
    fn credit_balance_separate_serde_default_and_account_mapping() {
        assert!(!Settings::default().credit_balance_separate);
        let missing: Settings = serde_json::from_str("{}").unwrap();
        assert!(!missing.credit_balance_separate);
        let yes: Settings = serde_json::from_str("{\"credit_balance_separate\":true}").unwrap();
        assert!(yes.credit_balance_separate);
        assert!(crate::wielosilnik::POLA_RACHUNKU.contains(&"credit_balance_separate"));
        let merged = crate::wielosilnik::ustawienia_formatu(&Settings::default(), &yes);
        assert!(merged.credit_balance_separate);
    }
}
