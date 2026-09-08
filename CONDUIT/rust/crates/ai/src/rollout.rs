
use crate::obs::open_risk;
use crate::policy::Model;
use crate::reward::WindowOutcome;
use crate::runtime::AiRuntime;
use conduit_backtest::{ReplayMessage, SimBroker, TickData};
use conduit_core::broker::*;
use conduit_core::engine::{Engine, IncomingMessage};
use conduit_core::settings::{Settings, TpSchedule};
use conduit_core::types::*;

// ============================================================
//  OKNA
// ============================================================

#[derive(Clone, Debug)]
pub struct Window {
    pub from: Ts,
    pub to: Ts,
    pub label: String,
}

pub fn day_label(ts: Ts) -> String {
    use chrono::{TimeZone, Utc};
    match Utc.timestamp_millis_opt(ts).single() {
        Some(d) => d.format("%Y-%m-%d").to_string(),
        None => "?".into(),
    }
}

/// Dzieli zakres na `n` równych kawałków i bierze z każdego pierwsze `days` dni.
///
/// Chodzi o to, żeby okna treningowe rozłożyły się po CAŁYM zakresie, a nie
/// skleiły w jeden reżim rynku. Model, który widzi tylko kwiecień, nauczy się
/// kwietnia.
pub fn split_windows(from: Ts, to: Ts, n: usize, days: f64) -> Vec<Window> {
    let n = n.max(1);
    let span = (to - from).max(1);
    let chunk = span / n as i64;
    let len = (days * 86_400_000.0) as i64;
    (0..n)
        .map(|k| {
            let a = from + k as i64 * chunk;
            let b = (a + len).min(a + chunk).min(to);
            Window {
                from: a,
                to: b,
                label: day_label(a),
            }
        })
        .filter(|w| w.to > w.from)
        .collect()
}

/// Podział przeplatany (blokowa walidacja krzyżowa).
///
/// Dzieli zakres na `blocks` bloków i w KAŻDYM wycina najpierw okno treningowe,
/// a zaraz za nim walidacyjne. Zbiory są rozłączne w czasie, ale oba rozciągają
/// się na cały zakres danych.
///
/// Po co, skoro prostszy podział „pierwsza połowa / druga połowa" też jest
/// rozłączny? Bo w tych danych reżim rynku zmienia się w czerwcu. Przy podziale
/// chronologicznym nie da się odróżnić przeuczenia od zwykłej zmiany reżimu —
/// model dostaje do nauki kwiecień–maj, a egzamin z czerwca–lipca. Przeplot
/// mierzy to, co chcemy zmierzyć: uogólnianie, a nie zgodność reżimów.
pub fn split_alternating(
    from: Ts,
    to: Ts,
    blocks: usize,
    train_days: f64,
    valid_days: f64,
) -> (Vec<Window>, Vec<Window>) {
    let n = blocks.max(1);
    let chunk = (to - from).max(1) / n as i64;
    let tl = (train_days * 86_400_000.0) as i64;
    let vl = (valid_days * 86_400_000.0) as i64;
    let mut tr = Vec::with_capacity(n);
    let mut va = Vec::with_capacity(n);
    for k in 0..n {
        let a = from + k as i64 * chunk;
        let t_end = (a + tl).min(a + chunk).min(to);
        if t_end > a {
            tr.push(Window {
                from: a,
                to: t_end,
                label: day_label(a),
            });
        }
        let v_beg = t_end;
        let v_end = (v_beg + vl).min(a + chunk).min(to);
        if v_end > v_beg {
            va.push(Window {
                from: v_beg,
                to: v_end,
                label: day_label(v_beg),
            });
        }
    }
    (tr, va)
}


/// Nakładka na `SimBroker`, która po drodze liczy to, czego silnik nie zapisuje.
///
/// `Engine::on_tick` konsumuje `drain_closed()` i zostawia po transakcjach tylko
/// zysk — a nagroda potrzebuje też CZASU TRZYMANIA. Zamiast dotykać
/// `crates/backtest`, przechwytujemy strumień zamknięć tutaj.
pub struct Audit {
    pub sim: SimBroker,
    /// Σ wolumen × godziny trzymania
    pub lot_hours: f64,
    pub gross_win: f64,
    pub gross_loss: f64,
    pub trades: u32,
    pub wins: u32,
    /// zysk KAŻDEJ zamkniętej transakcji — podstawa analizy ryzyka ruiny
    pub profits: Vec<f64>,
    // --- diagnostyka: co silnik w ogóle wysłał do brokera ---
    pub market_ok: u64,
    pub market_err: u64,
    pub pending_ok: u64,
    pub pending_err: u64,
}

impl Audit {
    pub fn new(sim: SimBroker) -> Self {
        Audit {
            sim,
            lot_hours: 0.0,
            gross_win: 0.0,
            gross_loss: 0.0,
            trades: 0,
            wins: 0,
            profits: Vec::with_capacity(256),
            market_ok: 0,
            market_err: 0,
            pending_ok: 0,
            pending_err: 0,
        }
    }
    #[inline]
    pub fn on_quote(&mut self, q: Quote) -> (usize, usize) {
        self.sim.on_quote(q)
    }
}

impl Broker for Audit {
    #[inline]
    fn quote(&self) -> Quote {
        self.sim.quote()
    }
    #[inline]
    fn account(&self) -> Account {
        self.sim.account()
    }
    #[inline]
    fn stops_level(&self) -> f64 {
        self.sim.stops_level()
    }
    #[inline]
    fn volume_min(&self) -> f64 {
        self.sim.volume_min()
    }
    #[inline]
    fn volume_step(&self) -> f64 {
        self.sim.volume_step()
    }
    fn volume_max(&self) -> f64 {
        self.sim.volume_max()
    }
    fn close_receipt_reconciliation_active(&self) -> bool {
        self.sim.close_receipt_reconciliation_active()
    }
    fn close_receipts_pending(&self) -> bool {
        self.sim.close_receipts_pending()
    }
    fn receipt_barrier(&self) -> ReceiptBarrier { self.sim.receipt_barrier() }
    fn execution_session(&self) -> Option<ExecutionSession> { self.sim.execution_session() }
    fn unconfirmed_open(&self) -> Option<UnconfirmedOpen> { self.sim.unconfirmed_open() }
    fn confirmed_open(&self, intent: &UnconfirmedOpen) -> Option<Ticket> { self.sim.confirmed_open(intent) }
    fn position_identifier(&self, ticket: Ticket) -> Option<u64> { self.sim.position_identifier(ticket) }
    fn pending_cancel_snapshot_authoritative(&self) -> bool { self.sim.pending_cancel_snapshot_authoritative() }
    fn cost_net_supported(&self) -> bool { self.sim.cost_net_supported() }
    fn report_cost_consumer_fault(&mut self, reason:&str) {self.sim.report_cost_consumer_fault(reason);}
    #[inline]
    fn positions(&self) -> &[Position] {
        self.sim.positions()
    }
    #[inline]
    fn pendings(&self) -> &[PendingOrder] {
        self.sim.pendings()
    }
    #[inline]
    fn positions_mut(&mut self) -> &mut Vec<Position> {
        self.sim.positions_mut()
    }
    #[inline]
    fn pendings_mut(&mut self) -> &mut Vec<PendingOrder> {
        self.sim.pendings_mut()
    }
    fn open_market(&mut self, r: OrderReq) -> BResult<Ticket> {
        let res = self.sim.open_market(r);
        if res.is_ok() {
            self.market_ok += 1;
        } else {
            self.market_err += 1;
        }
        res
    }
    fn place_pending(&mut self, r: PendingReq) -> BResult<Ticket> {
        let res = self.sim.place_pending(r);
        if res.is_ok() {
            self.pending_ok += 1;
        } else {
            self.pending_err += 1;
        }
        res
    }
    #[inline]
    fn modify_position(&mut self, t: Ticket, sl: Option<Px>, tp: Option<Px>) -> BResult<()> {
        self.sim.modify_position(t, sl, tp)
    }
    #[inline]
    fn modify_pending(
        &mut self,
        t: Ticket,
        price: Px,
        sl: Option<Px>,
        tp: Option<Px>,
    ) -> BResult<()> {
        self.sim.modify_pending(t, price, sl, tp)
    }
    #[inline]
    fn close_position(&mut self, t: Ticket, reason: CloseReason) -> BResult<f64> {
        self.sim.close_position(t, reason)
    }
    #[inline]
    fn close_partial(&mut self, t: Ticket, volume: f64, reason: CloseReason) -> BResult<f64> {
        self.sim.close_partial(t, volume, reason)
    }
    #[inline]
    fn cancel_pending(&mut self, t: Ticket) -> BResult<()> {
        self.sim.cancel_pending(t)
    }

    fn drain_closed(&mut self) -> Vec<ClosedTrade> {
        let v = self.sim.drain_closed();
        for c in &v {
            self.lot_hours += c.volume * ((c.close_ts - c.open_ts).max(0) as f64 / 3_600_000.0);
            self.profits.push(c.profit);
            self.trades += 1;
            if c.profit > 0.0 {
                self.wins += 1;
                self.gross_win += c.profit;
            } else {
                self.gross_loss += -c.profit;
            }
        }
        v
    }
}

// ============================================================
//  PRZEBIEG
// ============================================================

#[derive(Clone, Debug)]
pub struct RunCfg {
    pub start_balance: f64,
    pub settings: Settings,
    /// ile minut ticków przed oknem karmić samą pamięć rynku (ATR, zwroty)
    pub warmup_min: i64,
    /// ryzyko zastępcze pozycji bez SL, w ATR — musi być spójne z nagrodą
    pub no_sl_atr: f64,
    /// domknąć pozycje na końcu okna? (rozliczenie zamknięte, porównywalne okna)
    pub flat_at_end: bool,
    pub msg_offset_ms: i64,
}

impl Default for RunCfg {
    fn default() -> Self {
        let mut s = Settings::default();
        s.ai_enabled = true;
        let off = s.msg_offset();
        RunCfg {
            start_balance: 1000.0,
            settings: s,
            warmup_min: 120,
            no_sl_atr: 40.0,
            flat_at_end: true,
            msg_offset_ms: off,
        }
    }
}

/// Ustawienia odniesienia dla treningu na koncie ATFX/Vantage.
///
/// Jedna jednostka na koszyk to nie jest ostrożność „na wszelki wypadek" —
/// przy locie 0.01 i koncie rzędu kilkuset dolarów sześć jednostek zeruje
/// rachunek w tydzień. Model ma się nauczyć zarządzania, a nie kompensować
/// ustawienie, które i tak jest nie do obrony.
pub fn training_settings() -> Settings {
    training_settings_with(TpSchedule::AllRunners)
}

pub fn deep_settings() -> Settings {
    let mut s = training_settings_with(TpSchedule::Ladder);
    s.zone_offset_mode = conduit_core::settings::ZoneOffsetMode::Directional;
    s.entry_tol_offset = -5.0;
    s.entry_deep_offset = 2.0;
    s.sl_min_dist = 4.0;
    s.max_open_positions = 1;
    s.entry_units = 1;
    s.entry_units_limit = 1;
    s.max_dd_pct = 30.0;
    s
}

/// Jak wyżej, ale z jawnym wyborem harmonogramu celów.
///
/// Wybór ma duże znaczenie dla tego, CZEGO model może się nauczyć:
///  * `Ladder`/`AllAtTp1` — pozycja wychodzi sama na TP1, więc o wyniku
///    decyduje głównie sygnał, a model ma niewiele do powiedzenia,
///  * `AllRunners` — pozycja dostaje najdalszy cel, a całe zarządzanie po
///    drodze (kiedy brać zysk, kiedy ciąć) spada na model. To jest właściwe
///    zadanie dla AI i taką konfigurację przyjmujemy domyślnie.
pub fn training_settings_with(sched: TpSchedule) -> Settings {
    let mut s = Settings::default();
    s.ai_enabled = true;
    s.ai_decision_interval_s = 2.0;
    s.lot_fixed = 0.01;
    s.entry_units = 1;
    s.entry_units_limit = 1;
    s.ppm_enabled = false;
    s.toucher_units = 0;
    s.tp_schedule = sched;
    s.assign_tp_per_position = true;
    s.tp_source = conduit_core::settings::TpSource::Either;
    // strażniki silnika WYŁĄCZONE — ochronę kapitału realizuje warstwa
    // bezpieczeństwa AI, żeby wynik mierzył model, a nie preset
    s.max_dd_pct = 0.0;
    s.max_dd_usd = 0.0;
    s.equity_floor_pct = 0.0;
    s
}

/// Jeden przebieg przez jedno okno danych.
pub fn run_window(
    td: &TickData,
    msgs: &[ReplayMessage],
    w: &Window,
    rc: &RunCfg,
    model: &Model,
) -> WindowOutcome {
    let mut eng = Engine::new(rc.settings.clone(), rc.start_balance);
    let sim = SimBroker::z_ustawien(rc.start_balance, &rc.settings);
    let mut br = Audit::new(sim);
    let mut rt = AiRuntime::new(
        model.clone(),
        rc.start_balance,
        rc.settings.ai_decision_interval_s,
    );
    rt.risk_no_sl_atr = rc.no_sl_atr;

    let i0 = td.index_at(w.from);
    let i1 = td.index_at(w.to).min(td.len());
    let iw = td.index_at(w.from - rc.warmup_min * 60_000);

    // --- rozgrzewka pamięci rynku (bez brokera, bez decyzji) ---
    for i in iw..i0 {
        let q = td.quote(i);
        if q.bid > 0.0 && q.ask >= q.bid {
            rt.warmup_tick(&q);
        }
    }

    // czas wiadomości → czas ticków: przesunięcie zegara + modelowane opóźnienie
    let lat = rc.settings.exec_latency_ms + rc.msg_offset_ms;
    let mut mi = msgs.partition_point(|m| m.ts + lat < w.from);
    let source = SourceKey::new(1, None);

    // --- dekompozycja dobowa (potrzebna trybowi compounding) ---
    let tz = rc.settings.session_offset();
    let mut daily_pnl: Vec<f64> = Vec::with_capacity(128);
    let mut daily_ret: Vec<f64> = Vec::with_capacity(128);
    let mut daily_tr: Vec<u32> = Vec::with_capacity(128);
    let mut cur_day = i64::MIN;
    let mut day_open_eq = rc.start_balance;
    let mut day_open_tr = 0u32;

    let mut max_risk = 0.0f64;
    let mut next_risk_ts = i64::MIN;
    let risk_step = (rc.settings.ai_decision_interval_s * 1000.0) as i64;

    for i in i0..i1 {
        let q = td.quote(i);
        if !(q.bid > 0.0 && q.ask >= q.bid) {
            continue;
        }
        let d = conduit_core::types::day_of(q.ts, tz);
        if d != cur_day {
            if cur_day != i64::MIN {
                let eq = br.account().equity;
                daily_pnl.push(eq - day_open_eq);
                daily_ret.push((eq - day_open_eq) / day_open_eq.max(1.0) * 100.0);
                daily_tr.push(br.trades - day_open_tr);
                day_open_eq = eq;
                day_open_tr = br.trades;
            } else {
                day_open_eq = br.account().equity;
                day_open_tr = br.trades;
            }
            cur_day = d;
        }
        br.on_quote(q);

        while mi < msgs.len() && msgs[mi].ts + lat <= q.ts {
            let m = &msgs[mi];
            let im = IncomingMessage {
                ts: q.ts,
                source: source.clone(),
                source_name: "ATFX".into(),
                msg_id: m.msg_id,
                reply_to: m.reply_to,
                edit_of: m.edit_of,
                text: m.text.clone(),
            };
            eng.on_message(&mut br, &im);
            mi += 1;
        }

        eng.on_tick(&mut br, &q);
        rt.on_tick(&mut eng, &mut br, &q);

        if q.ts >= next_risk_ts {
            next_risk_ts = q.ts + risk_step;
            let r = open_risk(br.positions(), rt.market.atr(), rc.no_sl_atr);
            if r > max_risk {
                max_risk = r;
            }
        }
    }

    // --- domknięcie okna ---
    if rc.flat_at_end && i1 > i0 {
        let q = td.quote(i1 - 1);
        eng.close_everything(&mut br, q.ts, CloseReason::EodFlat);
        eng.on_tick(&mut br, &q);
    }

    let acc = br.account();
    if cur_day != i64::MIN {
        daily_pnl.push(acc.equity - day_open_eq);
        daily_ret.push((acc.equity - day_open_eq) / day_open_eq.max(1.0) * 100.0);
        daily_tr.push(br.trades - day_open_tr);
    }
    WindowOutcome {
        start_balance: rc.start_balance,
        end_equity: acc.equity,
        max_dd_abs: eng.stats.max_dd_abs,
        max_open_risk: max_risk,
        min_equity: br.sim.min_equity,
        daily_pnl,
        daily_trades: daily_tr,
        daily_ret_pct: daily_ret,
        lot_hours: br.lot_hours,
        trades: br.trades,
        wins: br.wins,
        gross_win: br.gross_win,
        gross_loss: br.gross_loss,
        blown: br.sim.blown,
        floor_hit: rt.liquidated,
        decisions: rt.acts.decisions,
        actions: rt.acts.actions(),
        signals: eng.stats.signals,
        baskets: eng.baskets.len() as u32,
        rejected_stops: br.sim.rejected_stops,
        filled_pendings: br.sim.filled_pendings,
        orders: [br.market_ok, br.market_err, br.pending_ok, br.pending_err],
        acts: rt.acts.breakdown(),
        profits: std::mem::take(&mut br.profits),
        repositions: rt.acts.repositions,
        repos_units: rt.acts.repos_units,
        depth_sum: rt.acts.depth_sum,
    }
}

/// Wynik jednego pola raportu sześciopolowego.
#[derive(Clone, Debug, Default)]
pub struct Pole {
    pub pnl: f64,
    /// krotność kapitału (tylko compounding)
    pub mult: f64,
    pub max_dd: f64,
    pub pf: f64,
    pub trades: u32,
    pub dni_plus: usize,
    pub dni: usize,
    pub min_equity_pct: f64,
    /// najgorsza doba jako % equity NA POCZĄTKU TEJ DOBY
    pub worst_day_pct: f64,
}

impl Pole {
    pub fn pct_dni_plus(&self) -> f64 {
        if self.dni == 0 {
            0.0
        } else {
            self.dni_plus as f64 / self.dni as f64 * 100.0
        }
    }
}

/// **Tryb compounding** — jeden ciągły przebieg, saldo przechodzi między dobami.
///
/// To jest tryb, w którym lot liczony jako % kapitału ma w ogóle sens i w którym
/// widać krotność konta. Metryki dobowe odnoszą się do equity NA POCZĄTKU DOBY,
/// nie do kapitału startowego — inaczej przy rosnącym koncie strata 15 % pokazuje
/// się jako setki procent i wyrzuca z rankingu każdy wariant, który urósł.
pub fn pole_compound(
    td: &TickData,
    msgs: &[ReplayMessage],
    from: Ts,
    to: Ts,
    rc: &RunCfg,
    model: &Model,
) -> Pole {
    let w = Window {
        from,
        to,
        label: day_label(from),
    };
    let o = run_window(td, msgs, &w, rc, model);
    Pole {
        pnl: o.pnl(),
        mult: o.end_equity / o.start_balance.max(1.0),
        max_dd: o.max_dd_abs,
        pf: o.profit_factor(),
        trades: o.trades,
        // Doba bez ANI JEDNEJ transakcji nie jest dobą stratną — jest dobą, w
        // której rynek stał (weekend, święto) albo filtr sesji nie wpuścił nic.
        // Wliczanie ich do mianownika zaniżało odsetek dni dodatnich z 59 % do
        // 37 % i uniemożliwiało uczciwe porównanie z rankingiem presetów.
        dni_plus: o
            .daily_pnl
            .iter()
            .zip(o.daily_trades.iter())
            .filter(|(p, t)| **t > 0 && **p > 0.0)
            .count(),
        dni: o.daily_trades.iter().filter(|t| **t > 0).count(),
        min_equity_pct: o.min_equity / o.start_balance.max(1.0) * 100.0,
        worst_day_pct: o
            .daily_ret_pct
            .iter()
            .zip(o.daily_trades.iter())
            .filter(|(_, t)| **t > 0)
            .map(|(r, _)| *r)
            .fold(f64::INFINITY, f64::min),
    }
}

/// **Tryb dzień-po-dniu** — każda doba startuje ze świeżym kapitałem.
///
/// Mierzy samą jakość decyzji, bez dźwigni składanej. Wariant, który wygląda
/// świetnie w compoundingu, a tutaj jest stratny, zarabia wyłącznie na
/// rosnącym locie — i to jest pierwszy test, który taki preset oblewa.
pub fn pole_daily(
    td: &TickData,
    msgs: &[ReplayMessage],
    from: Ts,
    to: Ts,
    rc: &RunCfg,
    model: &Model,
) -> Pole {
    let d = 86_400_000i64;
    let mut p = Pole {
        mult: 1.0,
        min_equity_pct: 100.0,
        worst_day_pct: 0.0,
        ..Default::default()
    };
    let (mut gw, mut gl) = (0.0f64, 0.0f64);
    let mut t = from;
    while t < to {
        let end = (t + d).min(to);
        let o = run_window(
            td,
            msgs,
            &Window {
                from: t,
                to: end,
                label: day_label(t),
            },
            rc,
            model,
        );
        if o.trades > 0 || o.pnl().abs() > 1e-9 {
            p.dni += 1;
            if o.pnl() > 0.0 {
                p.dni_plus += 1;
            }
            let pct = o.pnl() / o.start_balance.max(1.0) * 100.0;
            p.worst_day_pct = p.worst_day_pct.min(pct);
        }
        p.pnl += o.pnl();
        p.max_dd = p.max_dd.max(o.max_dd_abs);
        p.trades += o.trades;
        gw += o.gross_win;
        gl += o.gross_loss;
        p.min_equity_pct = p
            .min_equity_pct
            .min(o.min_equity / o.start_balance.max(1.0) * 100.0);
        t = end;
    }
    p.pf = if gl > 1e-9 {
        gw / gl
    } else if gw > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };
    p
}

/// Wygodny skrót: przebieg po wielu oknach.
pub fn run_windows(
    td: &TickData,
    msgs: &[ReplayMessage],
    ws: &[Window],
    rc: &RunCfg,
    model: &Model,
) -> Vec<WindowOutcome> {
    ws.iter()
        .map(|w| run_window(td, msgs, w, rc, model))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn podzial_okien_rozklada_sie_po_zakresie() {
        let d = 86_400_000i64;
        let ws = split_windows(0, 100 * d, 5, 4.0);
        assert_eq!(ws.len(), 5);
        assert_eq!(ws[0].from, 0);
        assert_eq!(ws[0].to, 4 * d);
        assert_eq!(ws[1].from, 20 * d);
        assert_eq!(ws[4].from, 80 * d);
        // okna są rozłączne
        for k in 1..ws.len() {
            assert!(ws[k].from >= ws[k - 1].to);
        }
    }

    #[test]
    fn przeplot_daje_rozlaczne_zbiory_na_calym_zakresie() {
        let d = 86_400_000i64;
        let (tr, va) = split_alternating(0, 60 * d, 6, 6.0, 4.0);
        assert_eq!(tr.len(), 6);
        assert_eq!(va.len(), 6);
        // żadne okno treningowe nie zachodzi na walidacyjne
        for t in &tr {
            for v in &va {
                assert!(t.to <= v.from || v.to <= t.from, "kolizja {t:?} × {v:?}");
            }
        }
        // oba zbiory sięgają początku i końca zakresu
        assert_eq!(tr[0].from, 0);
        assert!(va.last().unwrap().to >= 54 * d);
    }

    #[test]
    fn okno_dluzsze_niz_kawalek_jest_przycinane() {
        let d = 86_400_000i64;
        let ws = split_windows(0, 10 * d, 5, 30.0);
        assert!(ws.iter().all(|w| w.to - w.from <= 2 * d));
    }
}
