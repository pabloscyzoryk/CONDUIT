//! Metryki wyniku. Liczymy wszystko, co pozwala ocenić konfigurację —
//! nie tylko zysk, bo zysk bez kontekstu obsunięcia niczego nie mówi.

use conduit_core::types::{ClosedTrade, Ts};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DayStat {
    pub day: i64,
    pub date: String,
    pub start_equity: f64,
    pub end_equity: f64,
    pub profit: f64,
    pub max_dd: f64,
    /// Minimum of observed simulation steps, including the day's initial equity.
    /// Old result files do not contain this measurement; no curve-based fallback.
    #[serde(default)]
    pub min_equity: Option<f64>,
    #[serde(default)]
    pub real_dd: Option<f64>,
    #[serde(default)]
    pub real_dd_pct: Option<f64>,
    #[serde(default)]
    pub equity_observation_basis: Option<String>,
    pub trades: u32,
    pub signals: u32,
}

/// An ordered stream of real valuations, independent of saved chart resolution.
#[derive(Debug, Clone)]
pub(crate) struct EquityDrawdown {
    pub peak: f64,
    pub minimum: f64,
    pub max_abs: f64,
    pub max_pct: f64,
}

impl EquityDrawdown {
    pub fn new(equity: f64) -> Self {
        Self { peak: equity, minimum: equity, max_abs: 0.0, max_pct: 0.0 }
    }
    pub fn observe(&mut self, equity: f64) {
        if !equity.is_finite() { return; }
        self.minimum = self.minimum.min(equity);
        self.peak = self.peak.max(equity);
        let amount = (self.peak - equity).max(0.0);
        self.max_abs = self.max_abs.max(amount);
        if self.peak > 0.0 { self.max_pct = self.max_pct.max(amount / self.peak * 100.0); }
    }
    pub fn reset_account_peak(&mut self, equity: f64) {
        self.peak = equity;
        self.observe(equity);
    }
}

impl DayStat {
    /// Reporting only. Call again after a reporting-basis transformation (E−C).
    pub fn qualify_real_drawdown(&mut self) {
        self.real_dd = self.min_equity.filter(|m| m.is_finite())
            .filter(|_| self.start_equity.is_finite())
            .map(|m| (self.start_equity - m).max(0.0))
            .filter(|v| v.is_finite());
        self.real_dd_pct = self.real_dd.filter(|_| self.start_equity > 0.0)
            .map(|v| v / self.start_equity * 100.0).filter(|v| v.is_finite());
    }
}

/// Retrospective concentration diagnostics, never a rule selecting future days.
/// Removing a day's PnL does not replay later position sizing or order state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DailyConcentration {
    pub observed_days: u32,
    pub invalid_profit_days: u32,
    pub positive_days: u32,
    pub sum_daily_profit: f64,
    pub positive_daily_profit: f64,
    pub best_market_day: f64,
    pub best_market_day_date: String,
    pub median_market_day: f64,
    /// Owner's protocol: exclusions are available only at max lot 0.01.
    pub fixed_lot_exclusion_valid: bool,
    pub profit_without_best_1: Option<f64>,
    pub profit_without_best_3: Option<f64>,
    pub profit_without_best_5: Option<f64>,
    pub best_1_share_positive_pct: Option<f64>,
    pub best_3_share_positive_pct: Option<f64>,
    pub effective_positive_days: Option<f64>,
    pub return_observations: u32,
    pub unavailable_return_days: u32,
    pub median_daily_return_pct: Option<f64>,
    pub worst_5_observed_days_profit: Option<f64>,
}

pub fn daily_concentration(daily: &[DayStat]) -> DailyConcentration {
    let valid: Vec<_> = daily.iter().filter(|d| d.profit.is_finite()).collect();
    let mut profits: Vec<_> = valid.iter().map(|d| d.profit).collect();
    let mut positive: Vec<_> = profits.iter().copied().filter(|p| *p > 0.0).collect();
    positive.sort_by(|a, b| b.total_cmp(a));
    let total: f64 = profits.iter().sum();
    let gross: f64 = positive.iter().sum();
    let best: f64 = positive.iter().take(1).sum();
    let best3: f64 = positive.iter().take(3).sum();
    let mut result = DailyConcentration {
        observed_days: daily.len() as u32,
        invalid_profit_days: (daily.len() - valid.len()) as u32,
        positive_days: positive.len() as u32,
        sum_daily_profit: total,
        positive_daily_profit: gross,
        median_market_day: median(&mut profits),
        ..DailyConcentration::default()
    };
    if let Some(day) = valid.iter().max_by(|a, b| a.profit.total_cmp(&b.profit)) {
        result.best_market_day = day.profit;
        result.best_market_day_date = day.date.clone();
    }
    if gross > 0.0 {
        result.best_1_share_positive_pct = Some(100.0 * best / gross);
        result.best_3_share_positive_pct = Some(100.0 * best3 / gross);
        let concentration: f64 = positive.iter().map(|p| (p / gross).powi(2)).sum();
        result.effective_positive_days = Some(1.0 / concentration);
    }
    let mut returns: Vec<f64> = valid.iter().filter_map(|d| {
        if !d.start_equity.is_finite() || d.start_equity <= 0.0 {
            return None;
        }
        let value = d.profit / d.start_equity * 100.0;
        value.is_finite().then_some(value)
    }).collect();
    result.return_observations = returns.len() as u32;
    result.unavailable_return_days = daily.len() as u32 - result.return_observations;
    if !returns.is_empty() {
        result.median_daily_return_pct = Some(median(&mut returns));
    }
    // Do not bridge an invalid row and pretend those observations were adjacent.
    result.worst_5_observed_days_profit = daily.windows(5)
        .filter(|days| days.iter().all(|d| d.profit.is_finite()))
        .map(|days| days.iter().map(|d| d.profit).sum::<f64>())
        .min_by(f64::total_cmp);
    result
}

impl Metrics {
    /// Max-lot qualification must come from the actual strategy configuration,
    /// not a filename or a guess from the size of the resulting profit.
    pub fn qualify_fixed_lot_daily_exclusion(&mut self, daily: &[DayStat], max_lot: f64) {
        let Some(result) = self.daily_concentration.as_mut() else { return };
        result.fixed_lot_exclusion_valid = false;
        result.profit_without_best_1 = None;
        result.profit_without_best_3 = None;
        result.profit_without_best_5 = None;
        if !max_lot.is_finite() || (max_lot - 0.01).abs() > 1e-12
            || daily.is_empty() || result.invalid_profit_days > 0 {
            return;
        }
        let mut positive: Vec<_> = daily.iter().map(|d| d.profit).filter(|p| *p > 0.0).collect();
        positive.sort_by(|a, b| b.total_cmp(a));
        result.fixed_lot_exclusion_valid = true;
        result.profit_without_best_1 = Some(result.sum_daily_profit - positive.iter().take(1).sum::<f64>());
        result.profit_without_best_3 = Some(result.sum_daily_profit - positive.iter().take(3).sum::<f64>());
        result.profit_without_best_5 = Some(result.sum_daily_profit - positive.iter().take(5).sum::<f64>());
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Metrics {
    /// None identifies historical chart-sampled DD. It does not certify a zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_dd_observation_basis: Option<String>,
    // --- wynik ---
    pub start_balance: f64,
    pub end_balance: f64,
    pub end_equity: f64,
    /// Present only for nonzero separate-credit simulation. The returned equity
    /// curve, daily equity, end_equity, min_equity and DD use E-C (own capital).
    /// Raw broker values remain below; legacy/zero-credit JSON stays unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reporting_equity_basis: Option<String>,
    /// Constant throughout this simulation; credit-change events are not modeled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_credit: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_broker_end_equity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_broker_min_equity: Option<f64>,
    pub total_profit: f64,
    #[serde(deserialize_with = "null_jako_zero")]
    pub return_pct: f64,
    /// zysk na miesiąc przy zachowaniu tempa
    #[serde(deserialize_with = "null_jako_zero")]
    pub monthly_profit: f64,

    // --- dni ---
    pub days: u32,
    pub trading_days: u32,
    pub avg_per_day: f64,
    pub median_day: f64,
    pub best_day: f64,
    pub worst_day: f64,
    /// Data dnia odpowiadającego [`Self::worst_day`]. Puste w archiwalnych
    /// wynikach, które powstały przed dodaniem tej metryki.
    pub worst_day_date: String,
    pub win_days: u32,
    pub win_days_pct: f64,
    /// Positive equity days divided by ALL observed market days. Unlike the
    /// legacy closed-trade denominator, idle or floating-only days cannot
    /// silently disappear from a daily consistency target.
    pub market_days: u32,
    pub positive_market_days: u32,
    pub negative_market_days: u32,
    pub flat_market_days: u32,
    pub positive_market_days_pct: f64,
    pub worst_market_day: f64,
    pub worst_market_day_date: String,
    pub max_losing_streak_days: u32,
    /// All observed equity days, including floating-only and idle days.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_concentration: Option<DailyConcentration>,

    // --- ryzyko ---
    pub max_dd_abs: f64,
    #[serde(deserialize_with = "null_jako_zero")]
    pub max_dd_pct: f64,
    pub max_daily_dd: f64,
    pub min_equity: f64,
    /// Najgorsze CHWILOWE niezrealizowane obsunięcie.
    pub max_floating_loss: f64,
    /// Maksymalne OTWARTE RYZYKO: suma |wejście − SL| × 100 × wolumen po
    /// wszystkich pozycjach otwartych jednocześnie. Odpowiada na pytanie
    /// „ile stracę, jeśli WSZYSTKO naraz trafi w stop-loss".
    /// Bez tej liczby szeroki SL wygląda jak darmowy zysk — bo w krótkiej
    /// próbce stop po prostu nigdy nie zostaje trafiony.
    pub max_open_risk: f64,
    /// Maksymalne otwarte ryzyko jako % kapitału startowego.
    #[serde(deserialize_with = "null_jako_zero")]
    pub max_open_risk_pct: f64,
    /// Ile pozycji było otwartych jednocześnie w szczycie.
    pub max_open_positions: u32,
    /// czy konto zostało wyzerowane — dyskwalifikuje konfigurację
    pub blown: bool,
    #[serde(deserialize_with = "null_jako_zero")]
    pub recovery_factor: f64,
    #[serde(deserialize_with = "null_jako_zero")]
    pub calmar: f64,
    #[serde(deserialize_with = "null_jako_zero")]
    pub ulcer_index: f64,

    // --- transakcje ---
    pub trades: u32,
    pub wins: u32,
    pub losses: u32,
    #[serde(default)]
    pub bes: u32,
    /// Dawna definicja BEZ ZMIAN: wygrane / WSZYSTKIE transakcje. Nie wolno
    /// jej ruszyć — po tej liczbie porównuje się ponad dwieście archiwalnych
    /// plików `wyniki_*.json`.
    #[serde(deserialize_with = "null_jako_zero")]
    pub win_rate: f64,
    /// SKUTECZNOŚĆ BEZ REMISÓW: wygrane / (wygrane + przegrane-bez-BE).
    /// To jest ta liczba, którą chce się porównać ze skutecznością kanału —
    /// sygnalista też nie liczy wyjścia na zero jako przegranej.
    #[serde(default, deserialize_with = "null_jako_zero")]
    pub win_rate_bez_be: f64,
    #[serde(deserialize_with = "null_jako_zero")]
    pub profit_factor: f64,
    #[serde(deserialize_with = "null_jako_zero")]
    pub expectancy: f64,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub largest_win: f64,
    pub largest_loss: f64,
    pub avg_hold_min: f64,
    pub median_hold_min: f64,
    pub max_consecutive_losses: u32,

    // --- rozkład ---
    #[serde(deserialize_with = "null_jako_zero")]
    pub sharpe: f64,
    #[serde(deserialize_with = "null_jako_zero")]
    pub sortino: f64,
    /// mediana czasu do pierwszego zysku koszyka (minuty)
    pub median_time_to_profit: f64,

    // --- wykonanie ---
    pub signals_seen: u32,
    pub signals_taken: u32,
    pub baskets: u32,
    /// Zero in historical summaries; one when entry-source observations were measured.
    pub entry_source_observation_version: u32,
    /// Distinct source identities whose actual configured parser requested an
    /// entry, including the first known revision when NEW is unavailable.
    pub known_entry_sources: u32,
    /// Distinct identities that provided a complete Entry (bare NOW excluded).
    pub known_full_entry_sources: u32,
    /// Entry identities first encountered through EDIT rather than NEW.
    /// This exposes missing source history instead of hiding it in utilization.
    pub entry_sources_first_seen_as_edit: u32,
    pub rejected_stops: u64,
    pub market_instead_of_limit: u64,

    pub rejected_no_money: u64,
    /// ile pozycji zamknął stop out brokera
    pub stop_outs: u64,
    #[serde(default = "nieskonczonosc")]
    pub min_margin_level: f64,
    #[serde(default)]
    pub ml_pod_200: u64,
    #[serde(default)]
    pub ml_pod_150: u64,
    #[serde(default)]
    pub ml_pod_100: u64,
    #[serde(default)]
    pub max_open_volume: f64,
    #[serde(default)]
    pub max_open_margin: f64,
    // ---- CZAS ŻYCIA ZLECENIA OCZEKUJĄCEGO (minuty) ----
    //
    // Drugi czynnik iloczynu przy relocie: reguła zdąży zadziałać wyłącznie na
    // szczeblu, który JESZCZE LEŻY. Bez tego rozkładu „3 wykorzystania na
    // 2 477 okazji" nie rozkłada się na „nie było po co" i „nie było KIEDY".
    #[serde(default)]
    pub pend_zycie_med_min: f64,
    #[serde(default)]
    pub pend_zycie_p90_min: f64,
    #[serde(default)]
    pub pend_fill_n: u64,
    #[serde(default)]
    pub pend_anul_n: u64,
    /// znacznik czasu pierwszego stop-outu (0 = nie było)
    #[serde(default)]
    pub stop_out_ts: i64,
    /// saldo i equity w chwili pierwszego stop-outu — czy drabinka zdążyłaby
    #[serde(default)]
    pub bal_przy_stopoucie: f64,
    #[serde(default)]
    pub eq_przy_stopoucie: f64,
    /// ile zleceń oczekujących broker odrzucił kodem 10016 (SL/TP bliżej ceny
    /// AKTYWACJI niż `stops_level` — „szczeble-widma"). Liczone WYŁĄCZNIE przy
    /// `sim_validate_pending_stops = true`; przy `false` zawsze zero.
    pub rejected_pending_stops: u64,
    /// ile razy bramka wejść odrzuciła sygnał, wg kodu powodu
    pub odrzuty: std::collections::BTreeMap<String, u64>,

    pub relot_up_zdarzen: u32,
    /// ile razy szczebel był ZA DUŻY i relot go zmniejszył
    pub relot_down_zdarzen: u32,
    /// suma lotów dołożonych w górę
    pub relot_up_lotow: f64,
    /// suma lotów zdjętych w dół
    pub relot_down_lotow: f64,
    /// z tego: redukcje, których PLAN przeliczony na bieżące saldo wcale nie
    /// żąda — czyli wywołane wyłącznie spłaszczaniem wag RR
    pub relot_down_bez_spadku: u32,
    /// dokładki podnoszące szczebel POWYŻEJ planu — omijające
    /// `cap_basket_risk` (liczony od equity) i sufit portfela
    pub relot_up_ponad_plan: u32,
    /// ile razy relot w ogóle miał co robić (próby)
    pub relot_prob: u32,
    /// z tego udane / odrzucone przez brokera
    pub relot_udane: u32,
    pub relot_odmowy: u32,
    /// ile razy plan przeliczony na bieżące saldo wyszedł PUSTY
    pub relot_plan_pusty: u32,
    /// ile razy plan wyszedł policzalny
    pub relot_plan_ok: u32,
    /// suma |cel wg planu − cel wg gołego lota| w lotach
    pub relot_rozjazd_lotow: f64,
    /// MIANOWNIK: ile razy w ogóle był żywy szczebel do sprawdzenia
    pub relot_szczebli: u32,
    /// koszyki pominięte, bo przeliczony plan miał inny KSZTAŁT
    pub relot_ksztalt_odmowa: u32,
    // ---- REDUKCJA EKSPOZYCJI (`expo_cap_pct`) ----
    /// najwyższa ekspozycja POTENCJALNA w % equity (mierzona zawsze, gdy
    /// reguła włączona — także przy progu, który nigdy nie wiąże)
    #[serde(default)]
    pub expo_max_pct: f64,
    /// ile ticków przekroczyło próg
    #[serde(default)]
    pub expo_zdarzen: u32,
    /// ile leżących zleceń skasowano i ile to LOTÓW — wariant (a)
    #[serde(default)]
    pub expo_pend_skasowane: u32,
    #[serde(default)]
    pub expo_lotow: f64,
    /// ile pozycji domknięto — wariant (b)
    #[serde(default)]
    pub expo_poz_domkniete: u32,
    /// ile razy sam wariant (a) nie dowiózł (zero leżących, nadal nad progiem)
    #[serde(default)]
    pub expo_niedosyt: u32,

    /// STATYSTYKI PER SYGNAŁ I KOSZYK (Pakiet E): lejek, koszyki, transakcje.
    ///
    /// Siedzi W METRYKACH, a nie obok nich, z jednego powodu: `wyniki_*.json`
    /// to mapa „nazwa presetu → `Metrics`" i wszystko, co ma być porównywalne
    /// między presetami, musi być w TEJ strukturze. Osobny plik obok rozjechałby
    /// się z nią przy pierwszym przebiegu, którego ktoś nie zapisze w całości.
    ///
    /// Stare pliki czytają się dalej (`serde(default)` na kontenerze `Metrics`),
    /// a stare czytniki nowych plików też — serde pomija pola, których nie zna.
    #[serde(default)]
    pub stat_sygnalow: crate::statystyki::StatSygnalow,
}

fn nieskonczonosc() -> f64 {
    f64::INFINITY
}

/// NaN i nieskończoność → 0. Wartość, której nie da się policzyć, ma być
/// zerem, a nie liczbą, której nie da się ZAPISAĆ.
///
/// `serde_json` nie ma reprezentacji dla NaN ani `inf` — zapisuje je jako
/// `null`, a przy odczycie `null` nie jest poprawnym `f64`. Jedna taka wartość
/// unieważnia CAŁY plik wyników, razem z kilkudziesięcioma presetami obok.
fn skonczona(x: f64) -> f64 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

fn null_jako_zero<'de, D>(d: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<f64>::deserialize(d)?
        .filter(|x| x.is_finite())
        .unwrap_or(0.0))
}

fn median(v: &mut Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) * 0.5
    }
}

pub fn compute(
    start_balance: f64,
    equity_curve: &[(Ts, f64)],
    daily: &[DayStat],
    trades: &[ClosedTrade],
    min_equity: f64,
    blown: bool,
    prog_be: f64,
) -> Metrics {
    let mut m = Metrics {
        start_balance,
        min_equity,
        blown,
        ..Default::default()
    };

    m.end_equity = equity_curve
        .last()
        .map(|(_, e)| *e)
        .unwrap_or(start_balance);
    m.end_balance = m.end_equity;
    m.total_profit = m.end_equity - start_balance;
    m.return_pct = m.total_profit / start_balance.max(1.0) * 100.0;

    // --- obsunięcie na krzywej equity ---
    let mut peak = start_balance;
    let mut dd_abs: f64 = 0.0;
    let mut dd_pct: f64 = 0.0;
    let mut ulcer_sum = 0.0;
    for &(_, e) in equity_curve {
        if e > peak {
            peak = e;
        }
        let d = peak - e;
        if d > dd_abs {
            dd_abs = d;
        }
        let dp = d / peak.max(1.0) * 100.0;
        if dp > dd_pct {
            dd_pct = dp;
        }
        ulcer_sum += dp * dp;
    }
    m.max_dd_abs = dd_abs;
    m.max_dd_pct = dd_pct;
    m.ulcer_index = if equity_curve.is_empty() {
        0.0
    } else {
        (ulcer_sum / equity_curve.len() as f64).sqrt()
    };

    // --- dni ---
    m.days = daily.len() as u32;
    m.market_days = daily.len() as u32;
    m.daily_concentration = (!daily.is_empty()).then(|| daily_concentration(daily));
    m.positive_market_days = daily.iter().filter(|d| d.profit > 0.0).count() as u32;
    m.negative_market_days = daily.iter().filter(|d| d.profit < 0.0).count() as u32;
    m.flat_market_days = daily.iter().filter(|d| d.profit == 0.0).count() as u32;
    if m.market_days > 0 {
        m.positive_market_days_pct = 100.0 * m.positive_market_days as f64 / m.market_days as f64;
        if let Some(day) = daily.iter().min_by(|a,b| a.profit.total_cmp(&b.profit)) {
            m.worst_market_day = day.profit;
            m.worst_market_day_date = day.date.clone();
        }
    }
    let active: Vec<&DayStat> = daily.iter().filter(|d| d.trades > 0).collect();
    m.trading_days = active.len() as u32;
    let profits: Vec<f64> = active.iter().map(|d| d.profit).collect();
    if !profits.is_empty() {
        m.avg_per_day = profits.iter().sum::<f64>() / profits.len() as f64;
        m.best_day = profits.iter().cloned().fold(f64::MIN, f64::max);
        m.worst_day = profits.iter().cloned().fold(f64::MAX, f64::min);
        if let Some(day) = active.iter().min_by(|a, b| a.profit.total_cmp(&b.profit)) {
            m.worst_day_date = day.date.clone();
        }
        m.win_days = profits.iter().filter(|p| **p > 0.0).count() as u32;
        m.win_days_pct = m.win_days as f64 / profits.len() as f64 * 100.0;
        m.median_day = median(&mut profits.clone());
        m.monthly_profit = m.avg_per_day * 21.0;
    }
    m.max_daily_dd = daily.iter().map(|d| d.max_dd).fold(0.0, f64::max);

    let mut streak = 0u32;
    for d in &active {
        if d.profit < 0.0 {
            streak += 1;
            m.max_losing_streak_days = m.max_losing_streak_days.max(streak);
        } else {
            streak = 0;
        }
    }

    // --- transakcje ---
    m.trades = trades.len() as u32;
    let mut gw = 0.0;
    let mut gl = 0.0;
    let mut holds: Vec<f64> = Vec::with_capacity(trades.len());
    let mut cons = 0u32;
    // Próg BE jest OSOBNYM przebiegiem po tej samej liście, świadomie NIE
    // wpleciony w gałąź wygrana/przegrana. Wplecenie zmieniłoby `losses`,
    // a razem z nimi `avg_loss`, `profit_factor` i `max_consecutive_losses` —
    // czyli liczby, po których porównuje się całe archiwum. Remis ma być
    // NOWYM odczytem tych samych transakcji, nie przesunięciem starych.
    let prog = prog_be.abs();
    for t in trades {
        if t.profit > 0.0 {
            m.wins += 1;
            gw += t.profit;
            m.largest_win = m.largest_win.max(t.profit);
            cons = 0;
        } else {
            m.losses += 1;
            gl += -t.profit;
            m.largest_loss = m.largest_loss.min(t.profit);
            cons += 1;
            m.max_consecutive_losses = m.max_consecutive_losses.max(cons);
        }
        if t.profit.abs() <= prog {
            m.bes += 1;
        }
        holds.push((t.close_ts - t.open_ts) as f64 / 60_000.0);
    }
    if m.trades > 0 {
        m.win_rate = m.wins as f64 / m.trades as f64 * 100.0;
        // Mianownik bez remisów. Wygrana o zysku <= progu (możliwe przy progu
        // dodatnim) też jest remisem, więc licznik też ją traci — inaczej
        // suma kubełków przestałaby się zgadzać z liczbą transakcji.
        let wygrane_bez_be = trades
            .iter()
            .filter(|t| t.profit > 0.0 && t.profit.abs() > prog)
            .count() as f64;
        let bez_be = m.trades - m.bes;
        if bez_be > 0 {
            m.win_rate_bez_be = wygrane_bez_be / bez_be as f64 * 100.0;
        }
        m.expectancy = m.total_profit / m.trades as f64;
        m.avg_hold_min = holds.iter().sum::<f64>() / holds.len() as f64;
        m.median_hold_min = median(&mut holds);
    }
    m.avg_win = if m.wins > 0 { gw / m.wins as f64 } else { 0.0 };
    m.avg_loss = if m.losses > 0 {
        gl / m.losses as f64
    } else {
        0.0
    };
    m.profit_factor = if gl > 0.0 {
        gw / gl
    } else if gw > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };

    // --- miary ryzyka skorygowanego ---
    if dd_abs > 0.0 {
        m.recovery_factor = m.total_profit / dd_abs;
    }
    // OKNO KROTSZE NIZ POL ROKU NIE NADAJE SIE DO ANUALIZACJI.
    //
    // CAGR podnosi krotnosc wzrostu do potegi `1/lata`. Przy oknie 40 dni
    // wykladnik wynosi 9,12, wiec konto, ktore urosло 12,72-krotnie, dostaje
    // CAGR 1,2e12 % i `calmar` rzedu 6,0e10. Liczba jest poprawna arytmetycznie
    // i fizycznie absurdalna: znaczy „gdyby to tempo utrzymalo sie CALY ROK,
    // z 600 $ zrobiloby sie 7 bilionow".
    //
    // Gorzej: taki wskaznik SYSTEMATYCZNIE PREMIUJE krotkie okna i szczesliwe
    // serie — im krotszy przebieg, tym wyzszy wykladnik i tym bardziej wynik
    // odjezdza. A `calmar` lezy w raportach obok `expectancy` i `profit_factor`,
    // wiec przy porownywaniu presetow kolumnami wyglada na miare jakosci.
    //
    // Ponizej progu zwracamy 0 („nie da sie policzyc"), tak samo jak przy
    // koncie zbankrutowanym nizej. Do porownywania krotkich okien sluzy
    // `recovery_factor` — zysk na jednostke obsuniecia, bez anualizacji.
    const MIN_DNI_DO_ANUALIZACJI: u32 = 180;
    if dd_pct > 0.0 && m.days >= MIN_DNI_DO_ANUALIZACJI {
        let years = m.days as f64 / 365.0;
        let cagr = if years > 0.0 && start_balance > 0.0 {
            ((m.end_equity / start_balance).powf(1.0 / years.max(1e-9)) - 1.0) * 100.0
        } else {
            0.0
        };
        m.calmar = skonczona(cagr / dd_pct);
    }

    let dr: Vec<f64> = active.iter().map(|d| d.profit).collect();
    if dr.len() > 1 {
        let mean = dr.iter().sum::<f64>() / dr.len() as f64;
        let var = dr.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (dr.len() - 1) as f64;
        let sd = var.sqrt();
        if sd > 0.0 {
            m.sharpe = mean / sd * (252.0f64).sqrt();
        }
        let dn: Vec<f64> = dr.iter().filter(|x| **x < 0.0).cloned().collect();
        if !dn.is_empty() {
            let dv = dn.iter().map(|x| x * x).sum::<f64>() / dn.len() as f64;
            let dsd = dv.sqrt();
            if dsd > 0.0 {
                m.sortino = mean / dsd * (252.0f64).sqrt();
            }
        }
    }

    m
}

#[cfg(test)]
mod testy_zgodnosci {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn stary_format_wynikow_wczytuje_sie() {
        let stary = r#"{"start_balance":200.0,"end_balance":1234.5,"end_equity":1234.5,
          "total_profit":1034.5,"return_pct":517.25,"monthly_profit":300.0,"days":60,
          "trading_days":42,"avg_per_day":24.6,"median_day":3.1,"best_day":400.0,
          "worst_day":-120.0,"win_days":31,"win_days_pct":73.8,"max_losing_streak_days":4,
          "max_dd_abs":300.0,"max_dd_pct":25.0,"max_daily_dd":80.0,"min_equity":150.0,
          "max_open_risk":0.0,"blown":false,"recovery_factor":3.4,"calmar":12.0,
          "ulcer_index":8.0,"trades":428,"wins":138,"losses":290,"win_rate":32.2,
          "profit_factor":7.86,"expectancy":2.4,"avg_win":30.0,"avg_loss":-8.0,
          "largest_win":200.0,"largest_loss":-60.0,"avg_hold_min":22.8,
          "median_hold_min":12.0,"max_consecutive_losses":9,"sharpe":1.2,"sortino":2.1,
          "median_time_to_profit":6.0,"signals_seen":1865,"signals_taken":389,
          "baskets":389,"rejected_stops":0,"market_instead_of_limit":0}"#;
        let m: Metrics = serde_json::from_str(stary).expect("stary format musi się wczytać");
        assert_eq!(m.trades, 428);
        assert_eq!(m.total_profit, 1034.5);
        // pola, których w pliku NIE BYŁO: zero znaczy „nie mierzono"
        assert_eq!(m.rejected_no_money, 0);
        assert_eq!(m.stop_outs, 0);
        assert_eq!(m.max_open_positions, 0);
        assert!(m.odrzuty.is_empty());
    }

    #[test]
    fn null_zamiast_liczby_nie_wywraca_pliku() {
        let z_nullem = r#"{"start_balance":200.0,"calmar":null,"profit_factor":null,
          "sharpe":null,"trades":182,"blown":true}"#;
        let m: Metrics = serde_json::from_str(z_nullem).expect("null musi być tolerowany");
        assert_eq!(m.calmar, 0.0, "niepoliczalny wskaźnik ma być zerem");
        assert_eq!(m.profit_factor, 0.0);
        assert_eq!(m.trades, 182);
        assert!(m.blown);
    }

    /// Archiwum to MAPA „nazwa presetu → metryki", nie pojedynczy wynik.
    #[test]
    fn mapa_presetow_wczytuje_sie() {
        let plik = r#"{"K-ultra":{"start_balance":200.0,"trades":10,"calmar":null},
                       "ULTRA-X3":{"start_balance":200.0,"trades":428}}"#;
        let m: BTreeMap<String, Metrics> = serde_json::from_str(plik).expect("mapa presetów");
        assert_eq!(m.len(), 2);
        assert_eq!(m["ULTRA-X3"].trades, 428);
        assert_eq!(m["K-ultra"].calmar, 0.0);
    }

    // ============ PAKIET E1: KATEGORIA BREAK-EVEN ============

    fn trejd(profit: f64) -> ClosedTrade {
        ClosedTrade {
            profit_basis: None, cost_receipt: None,
            ticket: 1,
            side: conduit_core::types::Side::Buy,
            volume: 0.01,
            open_price: 4000.0,
            close_price: 4000.0,
            open_ts: 0,
            close_ts: 60_000,
            profit,
            commission: 0.0,
            swap: 0.0,
            reason: conduit_core::types::CloseReason::Tp,
            basket: Some(1),
        }
    }

    /// Trzy transakcje — wygrana, przegrana i wyjście po cenie wejścia —
    /// trafiają w trzy kubełki, ale STARE liczby zostają nietknięte.
    #[test]
    fn remis_jest_nowym_odczytem_a_nie_przesunieciem_starych_liczb() {
        let t = vec![trejd(10.0), trejd(-4.0), trejd(0.0)];
        let m = compute(400.0, &[], &[], &t, 400.0, false, 0.0);
        assert_eq!(m.trades, 3);
        assert_eq!(m.bes, 1, "wyjście na zero to remis");
        // stary podział BEZ ZMIAN: remis nadal siedzi w przegranych
        assert_eq!((m.wins, m.losses), (1, 2));
        assert!(
            (m.win_rate - 100.0 / 3.0).abs() < 1e-9,
            "dawna definicja nietknięta"
        );
        // ...a nowa liczba pomija remis po OBU stronach ułamka
        assert!((m.win_rate_bez_be - 50.0).abs() < 1e-9);
    }

    /// Próg dodatni przesuwa granicę remisu i tylko ją.
    #[test]
    fn prog_be_lapie_wyjscia_zjedzone_spreadem() {
        let t = vec![trejd(0.03), trejd(-0.02), trejd(20.0)];
        let zero = compute(400.0, &[], &[], &t, 400.0, false, 0.0);
        assert_eq!(zero.bes, 0, "przy progu 0 tylko dokładne zero jest remisem");
        let szeroki = compute(400.0, &[], &[], &t, 400.0, false, 0.05);
        assert_eq!(szeroki.bes, 2);
        assert_eq!(
            (szeroki.wins, szeroki.losses),
            (zero.wins, zero.losses),
            "stary podział stały"
        );
        assert!(
            (szeroki.win_rate - zero.win_rate).abs() < 1e-12,
            "win_rate stały"
        );
        assert!((szeroki.win_rate_bez_be - 100.0).abs() < 1e-9);
    }

    /// Suma kubełków nie może przekroczyć liczby transakcji — inaczej raport
    /// zaczyna liczyć te same wyjścia dwa razy.
    #[test]
    fn suma_kubelkow_nie_przekracza_liczby_transakcji() {
        let t = vec![trejd(5.0), trejd(0.0), trejd(-1.0), trejd(0.01)];
        let m = compute(400.0, &[], &[], &t, 400.0, false, 0.02);
        assert!(m.bes <= m.trades);
        assert_eq!(
            m.wins + m.losses,
            m.trades,
            "stary podział nadal wyczerpuje całość"
        );
    }

    /// Wartość niepoliczalna nie może w ogóle TRAFIĆ do pliku.
    #[test]
    fn skonczona_tnie_nan_i_nieskonczonosc() {
        assert_eq!(skonczona(f64::NAN), 0.0);
        assert_eq!(skonczona(f64::INFINITY), 0.0);
        assert_eq!(skonczona(f64::NEG_INFINITY), 0.0);
        assert_eq!(skonczona(12.5), 12.5);
    }

    #[test]
    fn daily_consistency_cannot_hide_idle_or_floating_loss_days() {
        let daily: Vec<DayStat> = [(12.0, 1), (-20.0, 0), (0.0, 0)]
            .into_iter().enumerate().map(|(day, (profit, trades))| DayStat {
                day: day as i64, date: format!("synthetic-day-{day}"),
                start_equity: 300.0, end_equity: 300.0 + profit,
                profit, max_dd: 0.0, trades, signals: 1,
                min_equity: None, real_dd: None, real_dd_pct: None, equity_observation_basis: None,
            }).collect();
        let metrics = compute(300.0, &[], &daily, &[], 280.0, false, 0.0);
        assert_eq!(metrics.win_days_pct, 100.0, "legacy metric is kept explicit");
        assert_eq!(metrics.market_days, 3);
        assert_eq!((metrics.positive_market_days, metrics.negative_market_days, metrics.flat_market_days), (1,1,1));
        assert!((metrics.positive_market_days_pct - 100.0/3.0).abs() < 1e-10);
        assert_eq!(metrics.worst_market_day, -20.0);
        assert_eq!(metrics.worst_market_day_date, "synthetic-day-1");
    }

    fn concentration_days(profits: &[f64]) -> Vec<DayStat> {
        profits.iter().enumerate().map(|(day, profit)| DayStat {
            day: day as i64, date: format!("day-{day}"), start_equity: 600.0,
            end_equity: 600.0 + profit, profit: *profit, max_dd: 0.0,
            trades: 0, signals: 0,
            min_equity: None, real_dd: None, real_dd_pct: None, equity_observation_basis: None,
        }).collect()
    }

    #[test]
    fn rdd_daily_contract_and_legacy_unknown() {
        let mut day = concentration_days(&[30.0]).remove(0);
        day.start_equity = 200.0; day.end_equity = 230.0;
        day.min_equity = Some(180.0); day.max_dd = 70.0;
        day.qualify_real_drawdown();
        assert_eq!(day.real_dd, Some(20.0));
        assert_eq!(day.real_dd_pct, Some(10.0));
        assert_eq!(day.max_dd, 70.0);
        day.min_equity = Some(220.0); day.qualify_real_drawdown();
        assert_eq!(day.real_dd, Some(0.0));
        day.start_equity = 0.0; day.min_equity = Some(-20.0); day.qualify_real_drawdown();
        assert_eq!(day.real_dd, Some(20.0)); assert_eq!(day.real_dd_pct, None);
        let mut json = serde_json::to_value(&day).unwrap();
        for key in ["min_equity", "real_dd", "real_dd_pct"] { json.as_object_mut().unwrap().remove(key); }
        let mut legacy: DayStat = serde_json::from_value(json).unwrap();
        legacy.qualify_real_drawdown();
        assert_eq!(legacy.min_equity, None); assert_eq!(legacy.real_dd, None); assert_eq!(legacy.real_dd_pct, None);
        day.min_equity = Some(f64::NAN); day.qualify_real_drawdown();
        assert_eq!(day.real_dd, None); assert_eq!(day.real_dd_pct, None);
    }

    #[test]
    fn observed_dd_keeps_ordered_peaks_troughs_and_independent_account_resets() {
        let mut dd = EquityDrawdown::new(200.0);
        for equity in [250.0,180.0,230.0] { dd.observe(equity); }
        assert_eq!(dd.minimum,180.0); assert_eq!(dd.max_abs,70.0);
        assert!((dd.max_pct-28.0).abs()<1e-12);
        assert!(dd.max_abs >= 200.0-dd.minimum);
        dd.reset_account_peak(200.0);
        dd.observe(180.0);
        assert_eq!(dd.max_abs,70.0,"a new independent deposit is not a loss from yesterday's peak");
        assert!((dd.max_pct-28.0).abs()<1e-12);
    }

    #[test]
    fn concentration_distinguishes_bonus_day_from_only_profitable_day() {
        let healthy_days = concentration_days(&[10., 10., 1000., 10., 10.]);
        let fragile_days = concentration_days(&[-10., -10., 1000., -10., -10.]);
        let mut healthy_metrics = Metrics { daily_concentration: Some(daily_concentration(&healthy_days)), ..Metrics::default() };
        let mut fragile_metrics = Metrics { daily_concentration: Some(daily_concentration(&fragile_days)), ..Metrics::default() };
        healthy_metrics.qualify_fixed_lot_daily_exclusion(&healthy_days, 0.01);
        fragile_metrics.qualify_fixed_lot_daily_exclusion(&fragile_days, 0.01);
        let healthy = healthy_metrics.daily_concentration.unwrap();
        let fragile = fragile_metrics.daily_concentration.unwrap();
        assert_eq!(healthy.profit_without_best_1, Some(40.0));
        assert_eq!(healthy.profit_without_best_3, Some(20.0));
        assert_eq!(healthy.median_market_day, 10.0);
        assert_eq!(fragile.profit_without_best_1, Some(-40.0));
        assert_eq!(fragile.profit_without_best_3, Some(-40.0));
        assert_eq!(fragile.median_market_day, -10.0);
        assert_eq!(healthy.positive_days, 5, "floating-only days count too");
    }

    #[test]
    fn concentration_normalizes_compounding_and_does_not_invent_empty_returns() {
        let mut days = concentration_days(&[600., 1200., 2400., 4800.]);
        for day in &mut days {
            day.start_equity = day.profit;
            day.end_equity = 2.0 * day.profit;
        }
        let result = daily_concentration(&days);
        assert_eq!(result.median_daily_return_pct, Some(100.0));
        assert_eq!(result.profit_without_best_1, None);
        assert!(result.best_1_share_positive_pct.unwrap() > 50.0);
        days[0].start_equity = 0.0;
        assert_eq!(daily_concentration(&days).unavailable_return_days, 1);
        let flat = daily_concentration(&concentration_days(&[0.0, 0.0]));
        assert_eq!(flat.best_1_share_positive_pct, None);
        assert_eq!(flat.effective_positive_days, None);
        assert_eq!(flat.profit_without_best_1, None);
        let mut metrics = Metrics { daily_concentration: Some(daily_concentration(&days)), ..Metrics::default() };
        metrics.qualify_fixed_lot_daily_exclusion(&days, 0.01);
        assert!(metrics.daily_concentration.as_ref().unwrap().fixed_lot_exclusion_valid);
        for cap in [0.0, 0.1, 5.0, 10.0, f64::NAN] {
            metrics.qualify_fixed_lot_daily_exclusion(&days, cap);
            let result = metrics.daily_concentration.as_ref().unwrap();
            assert!(!result.fixed_lot_exclusion_valid);
            assert_eq!(result.profit_without_best_1, None);
            assert_eq!(result.profit_without_best_3, None);
        }
    }
}
