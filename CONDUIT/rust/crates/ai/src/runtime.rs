
use crate::obs::*;
use crate::policy::*;
use crate::safety::{Deny, DenyCounters, Safety, SafetyCfg};
use conduit_core::broker::{Broker, PendingReq};
use conduit_core::engine::Engine;
use conduit_core::types::*;

#[derive(Debug, Default, Clone, Copy)]
pub struct ActionCounters {
    pub decisions: u64,
    pub closes: u64,
    pub partials: u64,
    pub sl_moves: u64,
    pub tp_moves: u64,
    pub tp_drops: u64,
    pub pendings_added: u64,
    pub pendings_cancelled: u64,
    pub baskets_closed: u64,
    pub liquidations: u64,
    pub trims: u64,
    /// ile razy model przestawił wejście koszyka (i ile jednostek postawił)
    pub repositions: u64,
    pub pauses: u64,
    pub paused_h: f64,
    pub repos_units: u64,
    /// suma wybranych głębokości — do policzenia średniej głębokości wejścia
    pub depth_sum: f64,
}

impl ActionCounters {
    /// Rozbicie akcji w kolejności [`crate::reward::ACTION_NAMES`].
    pub fn breakdown(&self) -> [u64; 7] {
        [
            self.closes,
            self.partials,
            self.sl_moves,
            self.tp_moves,
            self.tp_drops,
            self.pendings_added,
            self.pendings_cancelled,
        ]
    }

    /// Ile razy model faktycznie coś zrobił (bez „trzymaj").
    pub fn actions(&self) -> u64 {
        self.closes
            + self.partials
            + self.sl_moves
            + self.tp_moves
            + self.tp_drops
            + self.pendings_added
            + self.pendings_cancelled
            + self.repositions
    }
}

/// Koszyk widziany przez model: metadane + zakres pozycji w `snaps`.
struct Group {
    id: u32,
    ctx: BasketCtx,
    first: usize,
    count: usize,
}

pub struct AiRuntime {
    pub model: Model,
    pub safety: Safety,
    pub market: MarketWindow,
    pub acts: ActionCounters,
    /// twardy stop zadziałał — od tej chwili runtime pilnuje już tylko ciszy
    pub liquidated: bool,
    /// ryzyko zastępcze pozycji bez SL, w jednostkach ATR (spójne z nagrodą)
    pub risk_no_sl_atr: f64,
    /// do kiedy model wstrzymał otwieranie (jego własna decyzja)
    pause_until: Ts,
    loss_streak: u32,
    last_win_ts: Ts,
    seen_wins: u32,
    seen_losses: u32,

    interval_ms: i64,
    last_ts: Ts,
    started: bool,

    // --- bufory robocze; alokowane raz, zero alokacji w pętli decyzyjnej ---
    feat: Vec<f32>,
    sp: Scratch,
    sb: Scratch,
    groups: Vec<Group>,
    snaps: Vec<PosSnap>,
    tickets: Vec<Ticket>,
    pend_tickets: Vec<Ticket>,
    lone: Vec<u32>,
}

impl AiRuntime {
    pub fn new(model: Model, start_balance: f64, decision_interval_s: f64) -> Self {
        let sp = Scratch::for_net(&model.policy.pos);
        let sb = Scratch::for_net(&model.policy.bsk);
        let risk_no_sl_atr = model.train.reward.no_sl_atr;
        let safety = Safety::new(model.safety.clone(), start_balance);
        AiRuntime {
            model,
            safety,
            market: MarketWindow::new(),
            acts: ActionCounters::default(),
            liquidated: false,
            risk_no_sl_atr,
            pause_until: 0,
            loss_streak: 0,
            last_win_ts: 0,
            seen_wins: 0,
            seen_losses: 0,
            interval_ms: (decision_interval_s.max(0.0) * 1000.0) as i64,
            last_ts: 0,
            started: false,
            feat: vec![0.0; POS_IN],
            sp,
            sb,
            groups: Vec::with_capacity(32),
            snaps: Vec::with_capacity(128),
            tickets: Vec::with_capacity(128),
            pend_tickets: Vec::with_capacity(128),
            lone: Vec::with_capacity(16),
        }
    }

    pub fn load(
        path: impl AsRef<std::path::Path>,
        start_balance: f64,
        interval_s: f64,
    ) -> anyhow::Result<Self> {
        Ok(Self::new(Model::load(path)?, start_balance, interval_s))
    }

    pub fn safety_cfg(&self) -> &SafetyCfg {
        &self.safety.cfg
    }
    pub fn denies(&self) -> DenyCounters {
        self.safety.denies
    }

    /// Rozgrzewka: karmimy samą pamięć rynku, bez decyzji. Używane przed oknem
    /// backtestu, żeby ATR i zwroty były już policzone na pierwszym ticku okna.
    #[inline]
    pub fn warmup_tick(&mut self, q: &Quote) {
        self.market.on_tick(q);
    }

    /// Bieżące otwarte ryzyko rachunku (spójne z funkcją nagrody).
    #[inline]
    pub fn open_risk<B: Broker>(&self, b: &B) -> f64 {
        open_risk(b.positions(), self.market.atr(), self.risk_no_sl_atr)
    }

    // ============================================================
    //  GŁÓWNE WEJŚCIE
    // ============================================================

    pub fn on_tick<B: Broker>(&mut self, eng: &mut Engine, b: &mut B, q: &Quote) {
        self.market.on_tick(q);
        if self.liquidated {
            return;
        }

        // --- bezpiecznik nr 1: podłoga equity, sprawdzana CO TICK ---
        let acc = b.account();
        if self.safety.must_liquidate(&acc) {
            self.liquidate(eng, b, q);
            return;
        }

        // Pauza modelu: silnik nie otwiera nic nowego, dopóki nie minie.
        // Zdejmujemy ją sami, więc `halted` nie zostaje na stałe.
        if self.pause_until > 0 {
            if q.ts < self.pause_until {
                if eng.halted.is_none() {
                    eng.halted = Some("AI: przerwa na decyzję modelu".into());
                }
            } else {
                self.pause_until = 0;
                if eng.halted.as_deref() == Some("AI: przerwa na decyzję modelu") {
                    eng.halted = None;
                }
            }
        }

        if self.started && q.ts - self.last_ts < self.interval_ms {
            return;
        }
        self.started = true;
        self.last_ts = q.ts;
        self.acts.decisions += 1;

        // Seria strat i czas od ostatniej wygranej — najtańsze cechy rozpoznające
        // zły dzień. Czytamy je z przyrostów liczników silnika, bo strumień
        // zamkniętych transakcji konsumuje `Engine::on_tick`, zanim tu dotrzemy.
        let (w_now, l_now) = (eng.stats.wins, eng.stats.losses);
        if w_now > self.seen_wins {
            self.loss_streak = 0;
            self.last_win_ts = q.ts;
        } else if l_now > self.seen_losses {
            self.loss_streak += l_now - self.seen_losses;
        }
        self.seen_wins = w_now;
        self.seen_losses = l_now;
        if self.last_win_ts == 0 {
            self.last_win_ts = q.ts;
        }

        // --- bezpiecznik nr 2: nadmiar marginu → ścinamy potencjalną ekspozycję ---
        if self.safety.over_margin(&acc) {
            self.trim_pendings(eng, b);
        }

        self.decide(eng, b, q, &acc);
    }

    /// Twardy stop: zamknij wszystko i zatrzymaj silnik.
    ///
    /// Samo zamknięcie pozycji nie wystarcza — bez `Engine::halted` kolejna
    /// wiadomość z kanału natychmiast otworzyłaby nowy koszyk na resztkach
    /// kapitału. Zatrzymanie silnika jest częścią bezpiecznika, nie dodatkiem.
    fn liquidate<B: Broker>(&mut self, eng: &mut Engine, b: &mut B, q: &Quote) {
        eng.close_everything(b, q.ts, CloseReason::MaxDd);
        eng.halted = Some(format!(
            "AI: podłoga equity {:.2} $ ({:.0} % kapitału startowego)",
            self.safety.floor(),
            self.safety.cfg.equity_floor_pct
        ));
        self.liquidated = true;
        self.acts.liquidations += 1;
    }

    /// Redukcja POTENCJALNEJ ekspozycji: kasujemy najmłodsze zlecenia oczekujące.
    /// Najstarsze stoją najbliżej strefy z sygnału, więc mają największą wartość.
    fn trim_pendings<B: Broker>(&mut self, eng: &mut Engine, b: &mut B) {
        self.pend_tickets.clear();
        let half = (b.pendings().len() / 2).max(1);
        for _ in 0..half {
            let mut newest: Option<(Ts, Ticket)> = None;
            for o in b.pendings() {
                if self.pend_tickets.contains(&o.ticket) {
                    continue;
                }
                let key = (o.placed_ts, o.ticket);
                if newest.map(|n| key > n).unwrap_or(true) {
                    newest = Some(key);
                }
            }
            match newest {
                Some((_, t)) => self.pend_tickets.push(t),
                None => break,
            }
        }
        for k in 0..self.pend_tickets.len() {
            let t = self.pend_tickets[k];
            if b.cancel_pending(t).is_ok() {
                self.acts.trims += 1;
            }
            for bk in eng.baskets.iter_mut() {
                bk.pendings.retain(|x| *x != t);
            }
        }
        self.pend_tickets.clear();
    }

    // ============================================================
    //  DECYZJA
    // ============================================================

    fn decide<B: Broker>(&mut self, eng: &mut Engine, b: &mut B, q: &Quote, acc: &Account) {
        if b.positions().is_empty() && b.pendings().is_empty() {
            return;
        }

        // --- aktualizacja szczytów zysku (w trybie AI silnik tego nie robi) ---
        for p in b.positions_mut().iter_mut() {
            let pts = (q.exit(p.side) - p.open_price) * p.side.sign();
            if pts > p.peak_pts {
                p.peak_pts = pts;
                p.last_peak_ts = q.ts;
            }
        }

        // --- agregaty globalne ---
        let mut buy_lots = 0.0;
        let mut sell_lots = 0.0;
        for p in b.positions() {
            match p.side {
                Side::Buy => buy_lots += p.volume,
                Side::Sell => sell_lots += p.volume,
            }
        }
        // Ta sama podstawa co w silniku: saldo pomniejszone o kredyt bonusowy,
        // gdy `odlicz_kredyt` jest włączone. Inaczej model AI skalowałby się
        // do lota, którego silnik nigdy nie złoży.
        let base_lot = eng.lot_size(eng.podstawa_lota());
        let atr = self.market.atr();
        let sc = Scale {
            q,
            atr,
            cap: self.safety.start_balance.max(1.0),
            base_lot: base_lot.max(0.01),
        };
        {
            let since_win_h = if self.last_win_ts > 0 {
                (q.ts - self.last_win_ts).max(0) as f64 / 3_600_000.0
            } else {
                0.0
            };
            let g = GlobalCtx {
                q,
                mw: &self.market,
                acc,
                start_balance: self.safety.start_balance,
                peak_equity: eng.stats.peak_equity,
                max_dd_abs: eng.stats.max_dd_abs,
                day_start_equity: eng.stats.day_start_equity,
                n_pos: b.positions().len(),
                n_pend: b.pendings().len(),
                buy_lots,
                sell_lots,
                base_lot,
                equity_floor: self.safety.floor(),
                tz_offset_ms: eng.cfg.session_offset(),
                loss_streak: self.loss_streak,
                since_win_h,
                open_risk: open_risk(b.positions(), atr, self.risk_no_sl_atr),
            };
            global_features(&mut self.feat[..G_DIM], &g);
        }

        // --- grupowanie pozycji po koszykach; źródłem prawdy jest BROKER ---
        self.snaps.clear();
        for p in b.positions() {
            if !p.frozen {
                self.snaps.push(PosSnap::of(p));
            }
        }
        self.snaps
            .sort_unstable_by_key(|p| (p.basket_key(), p.ticket));

        self.groups.clear();
        let mut i = 0usize;
        while i < self.snaps.len() {
            let key = self.snaps[i].basket_key();
            let mut j = i;
            while j < self.snaps.len() && self.snaps[j].basket_key() == key {
                j += 1;
            }
            let id = if key == u32::MAX { 0 } else { key };
            let ctx = basket_ctx(eng, b, q, id, &self.snaps[i..j]);
            self.groups.push(Group {
                id,
                ctx,
                first: i,
                count: j - i,
            });
            i = j;
        }

        // koszyki bez otwartych pozycji, ale z żywymi limitami — model musi móc
        // je skasować albo dołożyć do nich wejście
        self.lone.clear();
        for o in b.pendings() {
            let id = match o.basket {
                Some(x) => x,
                None => continue,
            };
            let known = self.groups.iter().any(|gr| gr.id == id) || self.lone.contains(&id);
            if !known {
                self.lone.push(id);
            }
        }
        self.lone.sort_unstable();
        let lone = std::mem::take(&mut self.lone);
        for &id in &lone {
            let ctx = basket_ctx(eng, b, q, id, &[]);
            self.groups.push(Group {
                id,
                ctx,
                first: 0,
                count: 0,
            });
        }
        self.lone = lone;

        // --- pętla decyzyjna ---
        for gi in 0..self.groups.len() {
            let (id, ctx, first, count) = {
                let gr = &self.groups[gi];
                (gr.id, gr.ctx, gr.first, gr.count)
            };
            basket_features(&mut self.feat[G_DIM..G_DIM + B_DIM], &sc, &ctx);
            self.model
                .policy
                .bsk
                .forward(&self.feat[..BSK_IN], &mut self.sb);
            let bd = decode_bsk_mode(&self.sb.out, self.model.actions);
            self.apply_basket(eng, b, q, id, &ctx, bd, base_lot);

            // W trybie zawężonym model NIE dotyka otwartych pozycji — SL, TP i
            // wyjścia prowadzi silnik. Pomijamy więc całą sieć pozycji, co przy
            // okazji skraca rollout o kilkadziesiąt procent.
            if self.model.actions == ActionMode::EntryOnly {
                continue;
            }

            for k in 0..count {
                let snap = self.snaps[first + k];
                if b.find_position(snap.ticket).is_none() {
                    continue; // zamknięta przez akcję koszykową
                }
                position_features(&mut self.feat[G_DIM + B_DIM..POS_IN], &sc, &ctx, &snap);
                self.model
                    .policy
                    .pos
                    .forward(&self.feat[..POS_IN], &mut self.sp);
                let pd = decode_pos(&self.sp.out);
                self.apply_pos(b, q, &snap, pd);
            }
        }
    }

    // ============================================================
    //  WYKONANIE
    // ============================================================

    fn apply_pos<B: Broker>(&mut self, b: &mut B, q: &Quote, p: &PosSnap, d: PosDecision) {
        match d.exit {
            Exit::CloseAll => {
                if b.close_position(p.ticket, CloseReason::Ai).is_ok() {
                    self.acts.closes += 1;
                }
                return;
            }
            Exit::ClosePartial => {
                let vol = ((p.volume * d.partial) * 100.0).round() / 100.0;
                if vol >= 0.01 && vol < p.volume - 1e-9 {
                    if b.close_partial(p.ticket, vol, CloseReason::Partial).is_ok() {
                        self.acts.partials += 1;
                    }
                } else if p.volume <= 0.01 + 1e-9 {
                    // minimalnego lota nie da się podzielić — częściowe = pełne
                    if b.close_position(p.ticket, CloseReason::Ai).is_ok() {
                        self.acts.closes += 1;
                    }
                    return;
                }
            }
            Exit::Hold => {}
        }

        let (cur_sl, cur_tp) = match b.find_position(p.ticket) {
            Some(x) => (x.sl, x.tp),
            None => return,
        };
        let stops = b.stops_level();
        let atr = self.market.atr();
        let s = p.side.sign();

        // --- TP ---
        let mut new_tp = cur_tp;
        match d.tp {
            TpAction::Keep => {}
            TpAction::SetAtr(m) => {
                let want = q.exit(p.side) + s * m * atr;
                match self.safety.sanitize_tp(p.side, want, q, stops) {
                    Ok(v) => {
                        if cur_tp.map(|c| (c - v).abs() > 1e-9).unwrap_or(true) {
                            new_tp = Some(v);
                            self.acts.tp_moves += 1;
                        }
                    }
                    Err(e) => self.safety.denies.bump(e),
                }
            }
            TpAction::Drop => match self.safety.allow_drop_tp(cur_sl.is_some()) {
                Ok(()) => {
                    if cur_tp.is_some() {
                        new_tp = None;
                        self.acts.tp_drops += 1;
                    }
                }
                Err(e) => self.safety.denies.bump(e),
            },
        }

        // --- SL ---
        let mut new_sl = cur_sl;
        if let SlAction::Gap(g) = d.sl {
            // Docelowy SL = bieżąca cena wyjścia − g × ATR. Zapadka w warstwie
            // bezpieczeństwa nie pozwoli poluzować już ustawionego stopa ani
            // zwiększyć otwartego ryzyka, więc model może tym tylko dociskać.
            let want = q.exit(p.side) - s * g * atr;
            match self.safety.sanitize_sl(p.side, cur_sl, want, q, stops) {
                Ok(v) => {
                    if cur_sl.map(|c| (c - v).abs() > 1e-9).unwrap_or(true) {
                        new_sl = Some(v);
                        self.acts.sl_moves += 1;
                    }
                }
                Err(e) => self.safety.denies.bump(e),
            }
        }

        if new_sl != cur_sl || new_tp != cur_tp {
            if b.modify_position(p.ticket, new_sl, new_tp).is_err() {
                self.safety.denies.bump(Deny::BrokerStops);
            }
        }
    }

    fn apply_basket<B: Broker>(
        &mut self,
        eng: &mut Engine,
        b: &mut B,
        q: &Quote,
        id: u32,
        ctx: &BasketCtx,
        d: BasketDecision,
        base_lot: f64,
    ) {
        match d {
            BasketDecision::Hold => {}

            BasketDecision::CancelPendings => {
                self.collect_pendings(b, id);
                self.flush_cancel(eng, b);
            }

            BasketDecision::CloseAll => {
                self.tickets.clear();
                for p in b.positions() {
                    if p.basket == Some(id) && !p.frozen {
                        self.tickets.push(p.ticket);
                    }
                }
                let mut n = 0u64;
                for k in 0..self.tickets.len() {
                    if b.close_position(self.tickets[k], CloseReason::Ai).is_ok() {
                        n += 1;
                    }
                }
                self.tickets.clear();
                if n > 0 {
                    self.acts.baskets_closed += 1;
                    self.acts.closes += n;
                }
                self.collect_pendings(b, id);
                self.flush_cancel(eng, b);
            }

            BasketDecision::Pause { hours } => {
                let until = q.ts + (hours * 3_600_000.0) as i64;
                if until > self.pause_until {
                    self.pause_until = until;
                    self.acts.pauses += 1;
                    self.acts.paused_h += hours;
                }
            }

            BasketDecision::Reposition {
                depth,
                sl_mult,
                units,
                tp_frac,
            } => {
                self.reposition(eng, b, q, id, ctx, depth, sl_mult, units, tp_frac, base_lot);
            }

            BasketDecision::AddPending {
                dist_atr,
                lot_mult,
                tp_atr,
            } => {
                if id == 0 {
                    return; // pozycje-sieroty nie dostają dokładek
                }
                let atr = self.market.atr();
                let stops = b.stops_level();
                let s = ctx.side.sign();
                let dist = (dist_atr * atr).max(stops * 2.0).max(0.05);
                let price = q.entry(ctx.side) - s * dist;
                let vol = ((base_lot * lot_mult) * 100.0).round() / 100.0;
                if vol < 0.01 {
                    return;
                }
                // JEDYNA droga do zwiększenia ekspozycji prowadzi przez tę bramkę
                if let Err(e) = self.safety.allow_new_exposure(b, vol, price) {
                    self.safety.denies.bump(e);
                    return;
                }
                let tp = price + s * (tp_atr * atr).max(stops * 2.0);
                let req = PendingReq {
                    kind: PendingKind::limit(ctx.side),
                    volume: vol,
                    price,
                    sl: ctx.sl,
                    tp: Some(tp),
                    basket: Some(id),
                    level: 99,
                    is_toucher: false,
                    comment: String::new(),
                    // Ścieżka AI nie zna dokładek — te powstają wyłącznie
                    // w relocie siatki (`engine.rs`), a model składa zlecenia
                    // sam, od zera. `false` znaczy „zwykłe zlecenie", czyli
                    // liczy się do LICZBY sztuk na szczeblu, nie tylko do
                    // wolumenu.
                    is_topup: false,
                    no_market_fallback: false,
                };
                if let Ok(t) = b.place_pending(req) {
                    self.acts.pendings_added += 1;
                    if let Some(bk) = eng.baskets.iter_mut().find(|x| x.id == id) {
                        bk.pendings.push(t);
                    }
                }
            }
        }
    }

    /// Przestawienie wejścia koszyka — najważniejsza akcja modelu.
    ///
    /// Wolno ją wykonać TYLKO na koszyku, w którym nic jeszcze nie zostało
    /// zafillowane. Przestawianie wejścia, gdy pozycje już istnieją, nie
    /// znaczyłoby nic — cena wejścia jest wtedy faktem.
    #[allow(clippy::too_many_arguments)]
    fn reposition<B: Broker>(
        &mut self,
        eng: &mut Engine,
        b: &mut B,
        q: &Quote,
        id: u32,
        ctx: &BasketCtx,
        depth: f64,
        sl_mult: f64,
        units: u32,
        tp_frac: f64,
        base_lot: f64,
    ) {
        if id == 0 || ctx.n_open > 0 || ctx.n_pend == 0 {
            return;
        }
        let (lo, hi, side) = (ctx.zone_lo, ctx.zone_hi, ctx.side);
        let atr = self.market.atr();
        let width = (hi - lo).max(atr * 0.5);
        if !width.is_finite() || width <= 0.0 {
            return;
        }
        let s = side.sign();
        let worse = side.worse_edge(lo, hi);
        // głębokość liczona od krawędzi GORSZEJ w stronę LEPSZEJ i dalej
        let price = worse - s * depth * width;
        let sl = price - s * sl_mult * width;
        if !price.is_finite() || !sl.is_finite() {
            return;
        }

        // W trybie zawężonym stop jest ustalany przez SILNIK (SL koszyka po
        // `sl_min_dist`), a model wybiera wyłącznie GDZIE wejść i ILE. Dzięki temu
        // przestrzeń poszukiwań ma dwie osie zamiast siedmiu.
        let sl = match self.model.actions {
            ActionMode::EntryOnly => match ctx.sl {
                Some(v) if (price - v) * s > 0.0 => v,
                _ => sl,
            },
            ActionMode::Full => sl,
        };

        // cel z drabinki SYGNAŁU — geometrię wybiera model, cele zostają autorskie
        let tps: Vec<Px> = eng
            .baskets
            .iter()
            .find(|x| x.id == id)
            .map(|x| x.tps.clone())
            .unwrap_or_default();
        if tps.is_empty() {
            return;
        }
        let ti = ((tp_frac * tps.len() as f64).floor() as usize).min(tps.len() - 1);
        let tp = tps[ti];
        // cel musi leżeć po stronie zysku względem wybranego wejścia
        if (tp - price) * s <= 0.0 {
            return;
        }

        let lot = ((base_lot * 100.0).round() / 100.0).max(0.01);
        let risk_per_unit = (price - sl).abs() * XAU_CONTRACT * lot;
        let acc = b.account();
        let open_risk_now = self.open_risk(b);

        // ile jednostek przejdzie przez bramki ryzyka
        let mut placed = 0u32;
        self.collect_pendings(b, id);
        let had = self.pend_tickets.len();
        for k in 0..units {
            let add = risk_per_unit;
            let cur = open_risk_now + risk_per_unit * k as f64;
            if let Err(e) = self.safety.allow_risk(&acc, add, cur) {
                self.safety.denies.bump(e);
                break;
            }
            if let Err(e) = self.safety.allow_new_exposure(b, lot, price) {
                self.safety.denies.bump(e);
                break;
            }
            // pierwsze udane wejście kasuje limity postawione przez silnik
            if placed == 0 && had > 0 {
                self.flush_cancel(eng, b);
            }
            // SL jedzie razem ze zleceniem. Było to wcześniej niebezpieczne:
            // `SimBroker` rozliczał stop po POZIOMIE, więc limit zrealizowany luką
            // pod własnym stopem zamykał się na nim z zyskiem (optymalizator zrobił
            // na tym 419 348 $ z konta 200 $). Symulator rozlicza teraz stop po
            // cenie rynkowej z ticka, który przebił poziom, więc luka nie może już
            // wyprodukować zysku i stop przy zleceniu jest bezpieczny — a przy tym
            // realistyczny, bo chroni pozycję od pierwszej milisekundy.
            let req = PendingReq {
                kind: PendingKind::limit(side),
                volume: lot,
                price,
                sl: Some(sl),
                tp: Some(tp),
                basket: Some(id),
                level: 50 + k as i32,
                is_toucher: false,
                comment: String::new(),
                // Ścieżka AI nie zna dokładek — te powstają wyłącznie
                // w relocie siatki (`engine.rs`), a model składa zlecenia
                // sam, od zera. `false` znaczy „zwykłe zlecenie", czyli
                // liczy się do LICZBY sztuk na szczeblu, nie tylko do
                // wolumenu.
                is_topup: false,
                no_market_fallback: false,
            };
            match b.place_pending(req) {
                Ok(t) => {
                    if let Some(bk) = eng.baskets.iter_mut().find(|x| x.id == id) {
                        bk.pendings.push(t);
                    }
                    placed += 1;
                }
                Err(_) => break,
            }
        }
        self.pend_tickets.clear();
        if placed > 0 {
            self.acts.repositions += 1;
            self.acts.repos_units += placed as u64;
            self.acts.depth_sum += depth;
        }
    }

    fn collect_pendings<B: Broker>(&mut self, b: &B, id: u32) {
        self.pend_tickets.clear();
        for o in b.pendings() {
            if o.basket == Some(id) && !o.frozen {
                self.pend_tickets.push(o.ticket);
            }
        }
    }

    fn flush_cancel<B: Broker>(&mut self, eng: &mut Engine, b: &mut B) {
        for k in 0..self.pend_tickets.len() {
            let t = self.pend_tickets[k];
            if b.cancel_pending(t).is_ok() {
                self.acts.pendings_cancelled += 1;
            }
            for bk in eng.baskets.iter_mut() {
                bk.pendings.retain(|x| *x != t);
            }
        }
        self.pend_tickets.clear();
    }
}

// ============================================================
//  KONTEKST KOSZYKA
// ============================================================

fn basket_ctx<B: Broker>(
    eng: &Engine,
    b: &B,
    q: &Quote,
    id: u32,
    members: &[PosSnap],
) -> BasketCtx {
    let n_pend = b.pendings().iter().filter(|o| o.basket == Some(id)).count();
    let floating: f64 = members.iter().map(|p| p.profit_usd(q)).sum();

    if let Some(bk) = eng.baskets.iter().find(|x| x.id == id) {
        return BasketCtx {
            side: bk.side,
            zone_lo: bk.zone_lo,
            zone_hi: bk.zone_hi,
            sl: bk.sl,
            first_tp: bk.tps.first().copied(),
            next_tp: bk.tps.get(bk.tp_stage).copied(),
            last_tp: bk.tps.last().copied(),
            tp_stage: bk.tp_stage,
            n_tps: bk.tps.len(),
            created_ts: bk.created_ts,
            is_limit: bk.is_limit,
            armed: bk.state == BasketState::Armed,
            risk_free: bk.state == BasketState::RiskFree,
            realized: bk.realized,
            floating,
            n_open: members.len(),
            n_pend,
        };
    }

    // koszyk nieznany (zamknięty albo pozycja-sierota) — kontekst zastępczy
    let side = members
        .first()
        .map(|p| p.side)
        .or_else(|| {
            b.pendings()
                .iter()
                .find(|o| o.basket == Some(id))
                .map(|o| o.kind.side())
        })
        .unwrap_or(Side::Buy);
    let mut c = BasketCtx::orphan(side, q.mid());
    c.floating = floating;
    c.n_open = members.len();
    c.n_pend = n_pend;
    if let Some(p) = members.first() {
        c.zone_lo = p.open_price;
        c.zone_hi = p.open_price;
        c.created_ts = p.open_ts;
        c.sl = p.sl;
        c.first_tp = p.tp;
        c.next_tp = p.tp;
        c.last_tp = p.tp;
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::SafetyCfg;
    use conduit_backtest::SimBroker;
    use conduit_core::broker::OrderReq;
    use conduit_core::settings::Settings;

    fn quote(bid: f64, ts: Ts) -> Quote {
        Quote {
            ts,
            bid,
            ask: bid + 0.23,
        }
    }

    fn setup(balance: f64) -> (Engine, SimBroker, AiRuntime) {
        let mut cfg = Settings::default();
        cfg.ai_enabled = true;
        let eng = Engine::new(cfg, balance);
        let mut b = SimBroker::new(balance, 0.20, 0.0);
        b.on_quote(quote(4000.0, 0));
        let model = Model::fresh(&[16, 12], &[12, 8], 1, SafetyCfg::default());
        let rt = AiRuntime::new(model, balance, 2.0);
        (eng, b, rt)
    }

    #[test]
    fn twardy_stop_likwiduje_i_zatrzymuje_silnik() {
        let (mut eng, mut b, mut rt) = setup(1000.0);
        b.open_market(OrderReq {
            side: Side::Buy,
            volume: 0.02,
            sl: None,
            tp: None,
            basket: Some(1),
            level: 0,
            is_toucher: false,
            comment: String::new(),
        })
        .unwrap();

        // zdrowe konto — nic się nie dzieje
        let q1 = quote(4000.0, 1_000);
        b.on_quote(q1);
        rt.on_tick(&mut eng, &mut b, &q1);
        assert!(!rt.liquidated);
        assert_eq!(b.positions().len(), 1);

        // equity spada pod podłogę 60 % (1000 → poniżej 600)
        b.balance = 500.0;
        let q2 = quote(4000.0, 3_000);
        b.on_quote(q2);
        rt.on_tick(&mut eng, &mut b, &q2);

        assert!(rt.liquidated, "podłoga equity nie zadziałała");
        assert!(b.positions().is_empty(), "pozycje nie zostały zlikwidowane");
        assert!(eng.halted.is_some(), "silnik nie został zatrzymany");
        assert_eq!(rt.acts.liquidations, 1);
    }

    #[test]
    fn model_nie_moze_otworzyc_pozycji_zerujacej_konto() {
        // Warstwa wykonania musi odmówić NIEZALEŻNIE od tego, co wyliczy sieć.
        let (_eng, mut b, rt) = setup(1000.0);
        // model chciałby dołożyć 1 lot: margin 800 $ przy equity 1000 $
        assert!(rt.safety.allow_new_exposure(&b, 1.0, 4000.0).is_err());
        // i nawet 0.10 lota po zapchaniu rachunku
        for _ in 0..3 {
            let _ = b.open_market(OrderReq {
                side: Side::Buy,
                volume: 0.10,
                sl: None,
                tp: None,
                basket: Some(1),
                level: 0,
                is_toucher: false,
                comment: String::new(),
            });
        }
        assert!(rt.safety.allow_new_exposure(&b, 0.10, 4000.0).is_err());
    }

    #[test]
    fn interwal_decyzji_jest_respektowany() {
        let (mut eng, mut b, mut rt) = setup(1000.0);
        b.open_market(OrderReq {
            side: Side::Buy,
            volume: 0.01,
            sl: None,
            tp: None,
            basket: Some(1),
            level: 0,
            is_toucher: false,
            comment: String::new(),
        })
        .unwrap();
        // 2 s interwału przy tickach co 100 ms → 1 decyzja na 20 ticków
        for k in 0..201i64 {
            let q = quote(4000.0 + (k % 7) as f64 * 0.1, k * 100);
            b.on_quote(q);
            rt.on_tick(&mut eng, &mut b, &q);
        }
        assert_eq!(rt.acts.decisions, 11, "decyzje: {}", rt.acts.decisions);
    }

    #[test]
    fn swiezy_model_niczego_nie_rusza() {
        let (mut eng, mut b, mut rt) = setup(1000.0);
        let t = b
            .open_market(OrderReq {
                side: Side::Buy,
                volume: 0.01,
                sl: Some(3980.0),
                tp: Some(4020.0),
                basket: Some(1),
                level: 0,
                is_toucher: false,
                comment: String::new(),
            })
            .unwrap();
        for k in 0..500i64 {
            let q = quote(4000.0 + ((k % 40) as f64 - 20.0) * 0.2, k * 500);
            b.on_quote(q);
            rt.on_tick(&mut eng, &mut b, &q);
        }
        assert_eq!(
            rt.acts.actions(),
            0,
            "świeży model wykonał akcje: {:?}",
            rt.acts
        );
        let p = b.find_position(t).expect("pozycja powinna żyć");
        assert_eq!(p.sl, Some(3980.0));
        assert_eq!(p.tp, Some(4020.0));
    }

    /// Model zmusza się do ustawienia SL, ustawiając bias warstwy wyjściowej.
    /// Wagi są zerowe, więc wyjście = bias — polityka jest w pełni sterowalna.
    fn model_ktory_zawsze_dociska_sl() -> Model {
        let mut m = Model::fresh(&[8, 8], &[8, 8], 3, SafetyCfg::default());
        m.actions = ActionMode::Full; // te testy dotyczą zarządzania pozycją
        let last = m.policy.pos.b.len() - 1;
        let b = &mut m.policy.pos.b[last];
        b[O_SL_SET] = 2.0; // > O_SL_KEEP (1.0)
        b[O_SL_GAP] = 0.0; // sigmoid(0) = 0.5 → luka 4.125 × ATR
        m
    }

    /// Karmi rynek tak, żeby ATR wyszedł ≈ `amp`, i zwraca ostatnie kwotowanie.
    fn rozgrzej(rt: &mut AiRuntime, b: &mut SimBroker, amp: f64, minutes: i64) -> Quote {
        let mut q = quote(4000.0, 0);
        for k in 0..minutes {
            let px = 4000.0 + if k % 2 == 0 { amp } else { 0.0 };
            q = quote(px, k * 60_000);
            b.on_quote(q);
            rt.warmup_tick(&q);
        }
        q
    }

    #[test]
    fn model_moze_docisnac_sl_pozycji_pod_woda() {
        // REGRESJA: przy poprzedniej parametryzacji (ułamek szczytu zysku)
        // pozycja stratna miała szczyt = 0, więc docelowy SL zawsze wypadał na
        // wejściu — po złej stronie rynku — i broker go odrzucał. Model nie miał
        // wtedy ŻADNEGO sposobu na zmniejszenie otwartego ryzyka poza
        // zamknięciem pozycji.
        let mut cfg = Settings::default();
        cfg.ai_enabled = true;
        let mut eng = Engine::new(cfg, 1000.0);
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        let mut rt = AiRuntime::new(model_ktory_zawsze_dociska_sl(), 1000.0, 2.0);

        let q = rozgrzej(&mut rt, &mut b, 1.0, 60);
        let t = b
            .open_market(OrderReq {
                side: Side::Buy,
                volume: 0.01,
                sl: Some(3980.0), // ryzyko 20 $ na pozycji 0.01 lota
                tp: None,
                basket: Some(1),
                level: 0,
                is_toucher: false,
                comment: String::new(),
            })
            .unwrap();
        let ryzyko_przed = rt.open_risk(&b);

        rt.on_tick(&mut eng, &mut b, &q);

        let p = b.find_position(t).expect("pozycja miała zostać otwarta");
        let sl = p.sl.expect("SL zniknął");
        assert!(sl > 3980.0, "SL nie został dociśnięty: {sl}");
        assert!(
            sl < q.bid,
            "SL po złej stronie rynku: {sl} vs bid {}",
            q.bid
        );
        assert!(rt.acts.sl_moves >= 1);
        assert!(rt.open_risk(&b) < ryzyko_przed, "otwarte ryzyko nie spadło");
    }

    #[test]
    fn zapadka_blokuje_rozluznienie_sl_w_pelnej_sciezce() {
        // ta sama polityka, ale przy dużym ATR luka 4.125 × ATR wypada PONIŻEJ
        // istniejącego stopa — czyli byłoby to poszerzenie straty
        let mut cfg = Settings::default();
        cfg.ai_enabled = true;
        let mut eng = Engine::new(cfg, 1000.0);
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        let mut rt = AiRuntime::new(model_ktory_zawsze_dociska_sl(), 1000.0, 2.0);

        let q = rozgrzej(&mut rt, &mut b, 6.0, 60);
        let t = b
            .open_market(OrderReq {
                side: Side::Buy,
                volume: 0.01,
                sl: Some(3999.0),
                tp: None,
                basket: Some(1),
                level: 0,
                is_toucher: false,
                comment: String::new(),
            })
            .unwrap();
        assert!(rt.market.atr() > 2.0, "atr {}", rt.market.atr());

        rt.on_tick(&mut eng, &mut b, &q);

        assert_eq!(
            b.find_position(t).unwrap().sl,
            Some(3999.0),
            "zapadka puściła"
        );
        assert_eq!(rt.acts.sl_moves, 0);
        assert!(rt.denies().ratchet >= 1, "odmowa nie została policzona");
    }

    fn model_ktory_przestawia_wejscie() -> Model {
        let mut m = Model::fresh(&[8, 8], &[8, 8], 5, SafetyCfg::default());
        m.actions = ActionMode::Full; // sl_mult modelu działa tylko w trybie pełnym
        let last = m.policy.bsk.b.len() - 1;
        m.policy.bsk.b[last][B_REPOS] = 2.0; // > B_HOLD (1.0)
        m
    }

    /// Buduje koszyk przez PRAWDZIWĄ ścieżkę silnika: wiadomość z kanału.
    fn koszyk_z_sygnalu(balance: f64) -> (Engine, SimBroker, AiRuntime) {
        let mut cfg = Settings::default();
        cfg.ai_enabled = true;
        cfg.entry_units = 1;
        let mut eng = Engine::new(cfg, balance);
        let mut b = SimBroker::new(balance, 0.20, 0.0);
        let mut rt = AiRuntime::new(model_ktory_przestawia_wejscie(), balance, 2.0);

        // rozgrzewka rynku przy cenie POWYŻEJ strefy, żeby limit nie fillował się od razu
        for k in 0..90i64 {
            let q = quote(4010.0 + (k % 3) as f64 * 0.5, k * 60_000);
            b.on_quote(q);
            rt.warmup_tick(&q);
        }
        let q = quote(4010.0, 90 * 60_000);
        b.on_quote(q);
        let m = conduit_core::engine::IncomingMessage {
            ts: q.ts,
            source: SourceKey::new(1, None),
            source_name: "T".into(),
            msg_id: 1,
            reply_to: None,
            edit_of: None,
            text: "BUY GOLD @ 4005/4000\nTP 4008\nTP 4012\nSL 3998".into(),
        };
        eng.on_message(&mut b, &m);
        (eng, b, rt)
    }

    #[test]
    fn model_przestawia_wejscie_glebiej_w_strefe() {
        // To jest sedno modelu: silnik stawia limit na krawędzi GORSZEJ (4005),
        // gdzie R:R wynosi ~0.4; model ma prawo przesunąć go w głąb strefy.
        let (mut eng, mut b, mut rt) = koszyk_z_sygnalu(1000.0);
        let plytkie: Vec<f64> = b.pendings().iter().map(|o| o.price).collect();
        assert_eq!(plytkie.len(), 1, "silnik miał postawić jeden limit");
        assert!(
            (plytkie[0] - 4005.0).abs() < 1e-6,
            "silnik postawił na {}",
            plytkie[0]
        );

        let q = quote(4010.0, 91 * 60_000);
        b.on_quote(q);
        rt.on_tick(&mut eng, &mut b, &q);

        assert_eq!(rt.acts.repositions, 1, "brak przestawienia: {:?}", rt.acts);
        let po: Vec<f64> = b.pendings().iter().map(|o| o.price).collect();
        assert_eq!(po.len(), 2, "model miał postawić 2 jednostki, jest {po:?}");
        // strefa 4000–4005, głębokość 0.85 × 5 $ → 4005 − 4.25 = 4000.75
        for px in &po {
            assert!(*px < 4005.0, "wejście nie zeszło w głąb strefy: {px}");
            assert!(
                (*px - 4000.75).abs() < 0.01,
                "wejście na {px}, oczekiwano 4000.75"
            );
        }
        // stary, płytki limit silnika zniknął
        assert!(!po.iter().any(|p| (*p - 4005.0).abs() < 1e-6));
        let o = &b.pendings()[0];
        assert!(o.sl.unwrap() < o.price, "SL po złej stronie");
        assert!(o.tp.unwrap() > o.price, "TP po złej stronie");
        // koszyk zna nowe zlecenia — inaczej silnik nie umiałby ich skasować
        assert_eq!(eng.baskets[0].pendings.len(), 2);
    }

    #[test]
    #[allow(unused)]
    fn zlecenia_modelu_niosa_stop() {
        // REGRESJA na darmowy zysk. `SimBroker::place_pending` nie waliduje SL
        // (robi to tylko `open_market`), a przy realizacji przypisuje go bez
        // sprawdzenia. Limit zrealizowany LUKĄ poniżej własnego stopa dawał
        // pozycję zamykaną natychmiast NA STOPIE Z ZYSKIEM — optymalizator zrobił
        // na tym 419 348 $ z konta 200 $. Zlecenie modelu nie ma prawa wyjść ze
        // stopem; stop zakładamy po realizacji, od faktycznej ceny wejścia.
        let (mut eng, mut b, mut rt) = koszyk_z_sygnalu(1000.0);
        let q = quote(4010.0, 91 * 60_000);
        b.on_quote(q);
        rt.on_tick(&mut eng, &mut b, &q);
        assert_eq!(rt.acts.repositions, 1);
        for o in b.pendings() {
            assert!(o.sl.is_some(), "zlecenie modelu musi nieść stop");
            assert!(o.tp.is_some(), "i cel");
        }
    }

    #[test]
    fn luka_pod_stopem_nie_daje_darmowego_zysku() {
        // REGRESJA na darmowy zysk. Limit realizuje się LUKĄ daleko poniżej
        // swojej ceny, więc pozycja powstaje już za własnym stopem. Dopóki
        // symulator rozliczał stop po POZIOMIE, zamykała się natychmiast z
        // zyskiem i optymalizator zrobił na tym 419 348 $ z konta 200 $. Po
        // poprawce stop wykonuje się po cenie rynkowej z ticka, który przebił
        // poziom — luka nie może już wyprodukować zysku.
        let (mut eng, mut b, mut rt) = koszyk_z_sygnalu(1000.0);
        let q = quote(4010.0, 91 * 60_000);
        b.on_quote(q);
        rt.on_tick(&mut eng, &mut b, &q);
        assert_eq!(rt.acts.repositions, 1);
        let saldo_przed = b.balance;

        // rynek spada na 3985 — poniżej limitu 4000.75 i poniżej stopa koszyka
        let q2 = quote(3985.0, 92 * 60_000);
        b.on_quote(q2);

        assert!(
            b.balance <= saldo_przed + 1e-9,
            "luka wyprodukowała zysk z niczego: {saldo_przed} → {}",
            b.balance
        );
        for t in b.drain_closed() {
            assert!(
                t.profit <= 1e-9,
                "transakcja z luki dała zysk {:.4}",
                t.profit
            );
        }
    }

    #[test]
    fn limit_ryzyka_zatrzymuje_przestawienie_na_malym_koncie() {
        let (mut eng, mut b, mut rt) = koszyk_z_sygnalu(100.0);
        let przed = b.pendings().len();
        let q = quote(4010.0, 91 * 60_000);
        b.on_quote(q);
        rt.on_tick(&mut eng, &mut b, &q);

        assert_eq!(
            rt.acts.repositions, 0,
            "przestawienie przeszło mimo limitu ryzyka"
        );
        assert!(
            rt.denies().risk >= 1,
            "odmowa nie została policzona: {:?}",
            rt.denies()
        );
        assert_eq!(
            b.pendings().len(),
            przed,
            "limity silnika zostały skasowane mimo odmowy"
        );
    }

    #[test]
    fn otwarte_ryzyko_liczy_sie_poprawnie() {
        let (_eng, mut b, rt) = setup(1000.0);
        // BUY 0.02 @ 4000.23, SL 3990 → ryzyko ≈ 10.23 × 100 × 0.02 = 20.46 $
        b.open_market(OrderReq {
            side: Side::Buy,
            volume: 0.02,
            sl: Some(3990.0),
            tp: None,
            basket: Some(1),
            level: 0,
            is_toucher: false,
            comment: String::new(),
        })
        .unwrap();
        let r = rt.open_risk(&b);
        assert!((r - 20.46).abs() < 0.05, "ryzyko {r}");

        // pozycja BEZ SL wnosi ryzyko zastępcze, nie zero
        b.open_market(OrderReq {
            side: Side::Sell,
            volume: 0.01,
            sl: None,
            tp: None,
            basket: Some(1),
            level: 0,
            is_toucher: false,
            comment: String::new(),
        })
        .unwrap();
        assert!(rt.open_risk(&b) > r + 1.0, "brak SL musi podnosić ryzyko");
    }
}
