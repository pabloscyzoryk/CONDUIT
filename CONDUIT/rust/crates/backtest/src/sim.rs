//! Symulator brokera — odwzorowanie zachowania Vantage na XAUUSD.
//!
//! To jest miejsce, w którym rozstrzyga się wiarygodność całego backtestu.
//! Reguły, których pilnujemy, bo każda z nich potrafi zamienić stratę w
//! fikcyjny zysk:
//!
//!  1. **BUY wchodzi po ASK, wychodzi po BID** (SELL odwrotnie). Spread jest
//!     zapłacony realnie, na każdej pozycji, w obie strony.
//!  2. **SL/TP nie mogą leżeć bliżej ceny niż `stops_level`** ani po złej
//!     stronie rynku. Broker takie zlecenie odrzuca — a symulator, który tego
//!     nie pilnuje, „zamyka" pozycję po cenie, której nigdy nie było.
//!  3. **Zlecenie oczekujące nie realizuje się po cenie gorszej od rynkowej.**
//!     Gdy rynek jest już za poziomem, wchodzimy po rynku.
//!  4. **SL i TP w tym samym ticku**: pierwszeństwo ma SL (założenie
//!     konserwatywne — nie znamy kolejności wewnątrz ticka).
//!  5. Prowizja i swap naliczane jawnie; na koncie STP prowizja = 0, a całym
//!     kosztem jest spread zawarty w danych.

use conduit_core::broker::*;
use conduit_core::settings::Settings;
use conduit_core::types::*;

/// D1b: `SYMBOL_SWAP_ROLLOVER3DAYS` z MT5 → nasza doba WEJŚCIA w potrójny swap.
///
/// Trzeba zrobić DWIE zmiany konwencji naraz i to jest cały powód, dla którego
/// ten kawałek jest funkcją z testem, a nie liczbą w polu:
///
/// 1. **Numeracja dni.** MT5 (`ENUM_DAY_OF_WEEK`) liczy od NIEDZIELI (0),
///    my od poniedziałku. Ten sam dzień: `nasz = (mt5 + 6) % 7`.
/// 2. **Co nazywamy dniem rolowania.** MT5 podaje dzień, którego ROLOWANIE
///    jest potrójne; nasze pole podaje dobę, w którą przy tym rolowaniu
///    WCHODZIMY, czyli dzień następny: `+1`.
///
/// Obie poprawki to razem `+7`, czyli zero modulo 7 — **wartość liczbowa się
/// nie zmienia**. Zbieżność „3 = 3", którą stary komentarz nazywał przypadkiem
/// i źródłem pomyłki, jest więc TOŻSAMOŚCIĄ, a nie zbiegiem okoliczności.
///
/// Jest to zgodne z konwencją MT5: sonda podaje 3 = ŚRODA, a rozliczenie
/// potrójnego swapu przypada na pierwszą chwilę kolejnej doby.
#[inline]
pub fn doba_rolowania_z_mt5(mt5: u32) -> u32 {
    ((mt5 % 7) + 6 + 1) % 7
}

pub struct SimBroker {
    /// None until an explicit empty-model Fresh bind. Never a real MT5 identity.
    synthetic_continuation_session: Option<ExecutionSession>,
    /// Only the B15 indexed driver uses this. A source-row identity is not a
    /// timestamp: separate rows may have identical timestamps AND prices.
    last_physical_observation: Option<(u64, Quote)>,
    /// Explicit swap execution profile, independent of the NET receipt consumer.
    /// None preserves legacy cash-at-rollover arithmetic. Some(0..=8) keeps
    /// accrued swap outside Balance until close and quantizes swap in currency.
    native_swap_cash_digits: Option<u32>,
    trade_sessions: Option<crate::trade_sessions::TradeSessionProfile>,
    pub market_closed_rejections: u64,
    /// Jawna precyzja brokera tylko w symulacji; None zachowuje historyczne f64.
    pub price_digits: Option<u32>,
    pub q: Quote,
    pub balance: f64,
    /// KREDYT BONUSOWY symulowanego rachunku. W modelu MT5 jest oddzielny
    /// od Balance i powiększa Equity. OFF zachowuje stary model zawarty w B.
    /// Offline `Settings::kredyt_reczny` jest źródłem kwoty brokera; zero
    /// oznacza brak zasymulowanego kredytu (nie ma odczytu z terminala).
    pub credit: f64,
    /// OFF: legacy credit included in balance. ON: separate MT5 credit in equity.
    pub credit_balance_separate: bool,
    pub start_balance: f64,
    pub leverage: u32,
    pub stops_level: f64,
    /// Siatka wolumenu instrumentu. Domyslnie XAUUSD 0.01/0.01; pola sa
    /// jawne, aby test/broker-spec mogl odtworzyc inny symbol bez zmiany
    /// logiki silnika.
    pub volume_min: f64,
    pub volume_step: f64,
    /// Explicit simulator contract. ON requires a finite, positive broker cap;
    /// research arithmetic may deliberately select another finite maximum.
    pub volume_max: f64,
    pub order_volume_contract_v2: bool,
    pub commission_per_lot: f64,
    /// Opt-in management net; cash is still charged only in the existing places.
    pub closed_profit_net_costs: bool,
    /// Ephemeral per-run namespace. Set explicitly before combining independent
    /// run receipts; this model does NOT implement persisted consumer replay.
    pub cost_run_id: String,
    cost_ledger: crate::sim_costs::SimCostLedger,
    /// Metadata only: legacy records cannot allocate entry costs to close slices.
    /// Sticky across rate changes; never changes cash or order decisions.
    unallocated_entry_cost_seen: bool,
    pub slippage: f64,
    pub slippage_pending: f64,
    pub limit_price_improvement: bool,
    /// Explicit simulator execution profile, not a strategy setting. Native
    /// Vantage probes close older SL positions on the fill observation, while
    /// the SL of a newly filled pending becomes eligible on the NEXT physical
    /// observation. Indexed replay uses on_tape_quote/on_observation so that
    /// internal repeats of one row cannot advance eligibility. Direct on_quote
    /// callers supply one invocation per observation. Equal timestamps can be
    /// distinct observations. Default OFF
    /// preserves the legacy immediate-new-SL model; TP/stop-out/manual exits
    /// are never postponed by this profile.
    pub defer_new_pending_sl: bool,
    pub swap_per_lot_day: f64,

    pub swap_enabled: bool,
    pub swap_long_points: f64,
    pub swap_short_points: f64,
    pub swap_point_value: f64,
    pub swap_rollover_weekday: u32,
    pub swap_rollover_mult: f64,
    pub swap_pomijaj_weekend: bool,
    /// doba, za którą swap już naliczono (dni od epoki, czas serwera)
    last_swap_day: i64,
    /// swap narosły na pozycji — trafia do `ClosedTrade::swap` przy zamknięciu
    swap_acc: std::collections::HashMap<Ticket, f64>,
    /// suma naliczonego swapu w całym przebiegu — do audytu
    pub swap_total: f64,

    pub margin_check_on_fill: bool,
    /// ile zleceń oczekujących broker skasował z braku depozytu
    pub rejected_no_money: u64,

    pub validate_pending_stops: bool,
    /// ile zleceń oczekujących odrzucono z powodu SL/TP za blisko ceny aktywacji
    pub rejected_pending_stops: u64,

    /// Czy margines liczyć po cenie BIEŻĄCEJ zamiast po cenie otwarcia.
    /// Patrz `used_margin` — to jest różnica modelu, nie parametr strojenia.
    pub margin_at_market: bool,
    pub min_margin_level: f64,
    /// ile ticków poziom marginesu spędził pod 200 / 150 / 100 %
    pub ml_pod_200: u64,
    pub ml_pod_150: u64,
    pub ml_pod_100: u64,
    /// najwyższy ŁĄCZNY wolumen otwartych pozycji (loty) i jego margines
    pub max_open_volume: f64,
    pub max_open_margin: f64,
    // ---- CZAS ŻYCIA ZLECENIA OCZEKUJĄCEGO ----
    //
    // Drugi czynnik iloczynu przy relocie: reguła może zadziałać wyłącznie na
    // szczeblu, który JESZCZE LEŻY. Szczebel wypełniony w dwie sekundy nie
    // dostanie ani jednej szansy na przeliczenie wolumenu, choćby okazji było
    // tysiące. Bez rozkładu życia „3 wykorzystania na 2 477 okazji" nie da się
    // rozłożyć na „nie było po co" i „nie było KIEDY".
    /// czasy życia (ms) zleceń WYPEŁNIONYCH
    pub zycie_pend_fill: Vec<i64>,
    /// czasy życia (ms) zleceń SKASOWANYCH (TTL, redukcja, wymiana relotu)
    pub zycie_pend_anul: Vec<i64>,
    /// Znacznik czasu PIERWSZEGO stop-outu (0 = nie było).
    ///
    /// Bez daty „24 % przebiegów ginie" nie da się odróżnić „ginie zawsze
    /// w tym samym dniu rynku" (jedno zdarzenie, anegdota) od „ginie
    /// w kilku różnych momentach" (mechanizm, który się powtarza) — a to
    /// jest cała różnica między „6 lipca był pech" a „to samo przyjdzie
    /// w sierpniu".
    pub stop_out_ts: Ts,
    /// SALDO i EQUITY w chwili pierwszego stop-outu.
    ///
    /// Odpowiada na pytanie, czy drabinka łańcuchów zdążyłaby zejść szczebel
    /// niżej PRZED zgonem. Drabinka czyta BALANCE, a saldo nie drga podczas
    /// narastania straty pływającej — więc jeśli w chwili likwidacji saldo
    /// stoi wciąż wysoko nad progiem szczebla, to znaczy, że drabinka nie
    /// miała fizycznej możliwości zareagować. Bez tych dwóch liczb zostaje
    /// domysł.
    pub bal_przy_stopoucie: f64,
    pub eq_przy_stopoucie: f64,
    /// poziom marginu, przy którym broker zamyka pozycje (%)
    pub stop_out_level_pct: f64,
    /// ile razy broker wykonał stop out (zamknięcie POJEDYNCZEJ pozycji)
    pub stop_outs: u64,

    positions: Vec<Position>,
    pendings: Vec<PendingOrder>,
    /// kolejka dla silnika — opróżniana przez `drain_closed`
    closed: Vec<ClosedTrade>,
    /// pełna historia, nigdy nie opróżniana — na potrzeby metryk
    pub history: Vec<ClosedTrade>,
    next_ticket: Ticket,

    /// statystyki wykonania — do audytu realizmu
    pub rejected_stops: u64,
    pub filled_pendings: u64,
    pub market_instead_of_limit: u64,
    /// najniższe equity, jakie kiedykolwiek wystąpiło (kontrola wyzerowania)
    pub min_equity: f64,
    /// Read-only account-control valuation before stop-out / engine management.
    /// Consumed by the runner; never used by execution or risk decisions.
    account_control_equity: Option<f64>,
    pub blown: bool,
    /// TEN SAM TICK PRZEBIŁ SL **I** TP tej samej pozycji (Pakiet E4).
    ///
    /// Na danych tickowych, przy poprawnie ustawionych poziomach, ta liczba
    /// MUSI wynosić zero — jedna cena wyjścia nie może być naraz poniżej stopu
    /// i powyżej celu. Licznik istnieje właśnie po to, żeby ta cisza była
    /// SPRAWDZONA, a nie założona: dodatnia wartość znaczy, że jakaś reguła
    /// (trailing, BE, retarget) przestawiła stop na drugą stronę celu, a wynik
    /// takiej pozycji rozstrzyga kolejność w kodzie, nie rynek.
    pub sl_tp_same_tick: u64,
    /// SPREAD ZAPŁACONY przy otwarciach, w dolarach (Pakiet E4).
    ///
    /// `(ask − bid) × 100 × wolumen` w chwili każdego otwarcia — pozycja wchodzi
    /// po gorszej stronie rynku i tyle właśnie oddaje brokerowi, zanim cena
    /// w ogóle drgnie. To jest jedyny koszt tego modelu, który dotąd nigdzie
    /// nie był wykazany: prowizja jest w `commission_per_lot`, swap ma własne
    /// pole, a spread siedział rozpuszczony w cenie wejścia.
    pub spread_paid_usd: f64,
}

impl SimBroker {
    #[inline]
    fn norm_price(&self, px: f64) -> f64 {
        self.price_digits.map_or(px, |d| {
            let factor = 10f64.powi(d as i32);
            (px * factor).round() / factor
        })
    }
    #[inline]
    fn norm_quote(&self, q: Quote) -> Quote {
        Quote { ts: q.ts, bid: self.norm_price(q.bid), ask: self.norm_price(q.ask) }
    }
    pub fn new(balance: f64, stops_level: f64, commission_per_lot: f64) -> Self {
        SimBroker {
            synthetic_continuation_session: None,
            last_physical_observation: None,
            native_swap_cash_digits: None,
            trade_sessions: None,
            market_closed_rejections: 0,
            price_digits: None,
            q: Quote {
                ts: 0,
                bid: 0.0,
                ask: 0.0,
            },
            balance,
            credit: 0.0,
            credit_balance_separate: false,
            start_balance: balance,
            leverage: 500,
            stops_level,
            volume_min: 0.01,
            volume_step: 0.01,
            volume_max: 100.0,
            order_volume_contract_v2: false,
            commission_per_lot,
            closed_profit_net_costs: false,
            cost_run_id: "ephemeral-sim-v1".into(),
            cost_ledger: crate::sim_costs::SimCostLedger::default(),
            unallocated_entry_cost_seen: false,
            slippage: 0.0,
            slippage_pending: 0.0,
            limit_price_improvement: false,
            defer_new_pending_sl: false,
            swap_per_lot_day: 0.0,
            // Domyślnie WYŁĄCZONY w samym `SimBroker`, żeby nie zmienić
            // zachowania kodu, który konstruuje go wprost (panel, demo).
            // Backtest włącza go z `Settings`, gdzie domyślną jest `true`.
            swap_enabled: false,
            swap_long_points: -75.82,
            swap_short_points: 27.41,
            swap_point_value: 1.0,
            swap_rollover_weekday: 3,
            swap_rollover_mult: 3.0,
            swap_pomijaj_weekend: false,
            last_swap_day: i64::MIN,
            swap_acc: std::collections::HashMap::new(),
            swap_total: 0.0,
            margin_check_on_fill: true,
            rejected_no_money: 0,
            // Domyślnie WYŁĄCZONA — jak przy `swap_enabled`, konstruktor nie ma
            // prawa zmienić zachowania kodu, który buduje brokera wprost.
            // Backtest włącza ją z `Settings` (`sim_validate_pending_stops`).
            validate_pending_stops: false,
            rejected_pending_stops: 0,
            margin_at_market: false,
            min_margin_level: f64::INFINITY,
            ml_pod_200: 0,
            ml_pod_150: 0,
            ml_pod_100: 0,
            max_open_volume: 0.0,
            max_open_margin: 0.0,
            zycie_pend_fill: Vec::new(),
            zycie_pend_anul: Vec::new(),
            stop_out_ts: 0,
            bal_przy_stopoucie: 0.0,
            eq_przy_stopoucie: 0.0,
            stop_out_level_pct: 20.0,
            stop_outs: 0,
            positions: Vec::with_capacity(64),
            pendings: Vec::with_capacity(64),
            closed: Vec::with_capacity(256),
            history: Vec::with_capacity(1024),
            next_ticket: 1,
            rejected_stops: 0,
            filled_pendings: 0,
            market_instead_of_limit: 0,
            min_equity: balance,
            account_control_equity: None,
            blown: false,
            sl_tp_same_tick: 0,
            spread_paid_usd: 0.0,
        }
    }

    pub fn native_swap_cash_digits(&self) -> Option<u32> { self.native_swap_cash_digits }

    pub fn set_trade_sessions(&mut self, profile: Option<crate::trade_sessions::TradeSessionProfile>) -> Result<(), String> {
        if let Some(p) = &profile { p.validate()?; }
        if !self.positions.is_empty() || !self.pendings.is_empty() {
            return Err("execution sessions may only be configured before exposure".into());
        }
        self.trade_sessions = profile;
        Ok(())
    }
    fn trade_is_open(&self) -> bool {
        self.trade_sessions.as_ref().is_none_or(|p| p.is_open(self.q.ts))
    }
    fn require_trade_open(&mut self) -> BResult<()> {
        if self.trade_is_open() { Ok(()) } else {
            self.market_closed_rejections += 1;
            Err(BrokerError::MarketClosed)
        }
    }

    /// Bind before the first observation/exposure; never migrate an active
    /// account's cash convention. Repeating the already-bound value is a no-op.
    pub fn set_native_swap_cash_digits(&mut self, digits: Option<u32>) -> Result<(), String> {
        if digits.is_some_and(|d| d > 8) {
            return Err("native swap currency digits must be in 0..=8".into());
        }
        if digits == self.native_swap_cash_digits { return Ok(()); }
        if self.q.ts != 0 || self.q.bid != 0.0 || self.q.ask != 0.0
            || self.last_physical_observation.is_some() || !self.positions.is_empty() || !self.pendings.is_empty()
            || !self.history.is_empty() || !self.closed.is_empty() || !self.swap_acc.is_empty()
            || self.last_swap_day != i64::MIN || self.swap_total != 0.0
            || self.balance != self.start_balance || self.cost_ledger.fault.is_some() {
            return Err("native swap cash profile can only bind to a pristine simulator".into());
        }
        self.native_swap_cash_digits = digits;
        Ok(())
    }

    fn native_swap_round(&self, value: f64) -> Option<f64> {
        let factor = 10f64.powi(self.native_swap_cash_digits? as i32);
        let rounded = (value * factor).round() / factor;
        (value.is_finite() && rounded.is_finite()).then_some(rounded)
    }

    /// The same settlement amounts feed cash, remaining exposure and receipts.
    fn native_swap_allocation(&mut self, ticket: Ticket, volume: f64, current: f64)
        -> crate::sim_costs::NativeSwapAllocation {
        let before = self.swap_acc.get(&ticket).copied().unwrap_or(0.0);
        let full = volume >= current;
        let realized = if full { before } else {
            self.native_swap_round(before * (volume / current)).expect("validated native swap accrual")
        };
        let remaining = if full { 0.0 } else {
            self.native_swap_round(before - realized).expect("validated native swap residual")
        };
        if full { self.swap_acc.remove(&ticket); }
        else { self.swap_acc.insert(ticket, remaining); }
        crate::sim_costs::NativeSwapAllocation { realized, remaining }
    }

    pub fn z_ustawien(balance: f64, s: &Settings) -> Self {
        let mut b = SimBroker::new(balance, s.stops_level, s.commission_per_lot);
        b.ustaw(s);
        if b.credit_balance_separate {
            b.min_equity = b.equity();
        }
        b
    }

    pub fn bind_synthetic_continuation_scope(&mut self) -> Result<ExecutionSession,String> {
        if self.synthetic_continuation_session.is_some() || !self.positions.is_empty()
            || !self.pendings.is_empty() || !self.history.is_empty() || !self.closed.is_empty()
            || self.next_ticket!=1
            || self.receipt_barrier()!=ReceiptBarrier::Clear {
            return Err("synthetic Fresh requires an unbound model without prior exposure/history/receipts".into());
        }
        static NEXT_SCOPE:std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(1);
        let serial=NEXT_SCOPE.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
        let session=ExecutionSession{scope:format!("SYNTHETIC_SIM_NOT_MT5:{}:{serial}",std::process::id()),generation:1};
        self.synthetic_continuation_session=Some(session.clone());Ok(session)
    }

    /// Przepisuje ustawienia na już zbudowanego brokera (cena i prowizja idą
    /// przez konstruktor, więc tu ich nie ma).
    pub fn ustaw(&mut self, s: &Settings) {
        if self.closed_profit_net_costs != s.closed_profit_net_costs
            && (!self.positions.is_empty() || !self.pendings.is_empty() || !self.history.is_empty()) {
            self.cost_ledger.latch("accounting mode change with existing state requires review");
        }
        self.closed_profit_net_costs=s.closed_profit_net_costs;
        if s.closed_profit_net_costs && !s.basket_realized_broker_only {
            self.cost_ledger.latch("closed net requires basket_realized_broker_only");
        }
        self.order_volume_contract_v2 = s.order_volume_contract_v2;
        self.slippage = s.slippage_pts;
        // Model kosztów odpytany z serwera Vantage, nie z dokumentacji.
        self.slippage_pending = s.slippage_pending_pts;
        self.swap_enabled = s.swap_enabled;
        self.swap_long_points = s.swap_long_points;
        self.swap_short_points = s.swap_short_points;
        self.swap_point_value = s.swap_point_value;
        // D1b: doba potrójnego rolowania WYLICZONA z wartości serwera zamiast
        // wpisanej ręcznie. Przy `swap_rollover3days_mt5 = 3` (sonda Vantage)
        // wynik jest identyczny z zaszytym 3 — patrz `doba_rolowania_z_mt5`.
        self.swap_rollover_weekday = if s.swap_rollover_z_serwera {
            doba_rolowania_z_mt5(s.swap_rollover3days_mt5)
        } else {
            s.swap_rollover_weekday
        };
        self.swap_rollover_mult = s.swap_rollover_mult;
        self.swap_pomijaj_weekend = s.swap_pomijaj_weekend;
        self.stop_out_level_pct = s.stop_out_level_pct;
        self.margin_at_market = s.sim_margin_at_market;
        self.margin_check_on_fill = s.sim_margin_check_on_fill;
        self.validate_pending_stops = s.sim_validate_pending_stops;
        // KREDYT BONUSOWY symulowanego rachunku.
        //
        // W backteście nie ma terminala, więc kwota wpisana przez człowieka
        // JEST odczytem z brokera — i musi nim być, inaczej silnik zgłaszałby
        // rozjazd „ręcznie 300 vs terminal 0" w każdym przemiecie.
        //
        // Legacy OFF zachowuje stare powiązanie z odliczaniem. W modelu MT5
        // kredyt istnieje na rachunku niezależnie od wyboru podstawy lota.
        self.credit_balance_separate = s.credit_balance_separate;
        self.credit = if (s.odlicz_kredyt || s.credit_balance_separate) && s.kredyt_reczny > 0.0 {
            s.kredyt_reczny
        } else {
            0.0
        };
        if s.konto_dzwignia > 0.0 {
            self.leverage = s.konto_dzwignia.clamp(1.0, u32::MAX as f64).round() as u32;
        }
    }

    #[inline]
    fn ticket(&mut self) -> Ticket {
        let t = self.next_ticket;
        self.next_ticket += 1;
        t
    }

    #[inline]
    pub fn equity(&self) -> f64 {
        let mut e = self.balance;
        if self.credit_balance_separate && self.credit.is_finite() && self.credit > 0.0 {
            e += self.credit;
        }
        for p in &self.positions {
            e += p.profit_usd(&self.q);
        }
        if self.native_swap_cash_digits.is_some() {
            for p in &self.positions { e += self.swap_acc.get(&p.ticket).copied().unwrap_or(0.0); }
        }
        e
    }

    #[inline]
    pub fn used_margin(&self) -> f64 {
        let mut m = 0.0;
        for p in &self.positions {
            let px = if self.margin_at_market {
                // cena wyceny pozycji: strona, po której zostałaby zamknięta
                let c = if p.side == Side::Buy {
                    self.q.bid
                } else {
                    self.q.ask
                };
                if c > 0.0 {
                    c
                } else {
                    p.open_price
                }
            } else {
                p.open_price
            };
            m += p.volume * XAU_CONTRACT * px / self.leverage as f64;
        }
        m
    }

    /// POZIOM MARGINESU w % (`equity / margines × 100`).
    ///
    /// `f64::INFINITY` przy zerowym marginesie — brak pozycji to nie jest
    /// „poziom 0 %", tylko brak zagrożenia, a zwrócenie zera kazałoby każdemu
    /// licznikowi „prawie-śmierci" liczyć puste konto jako katastrofę.
    #[inline]
    pub fn margin_level_pct(&self) -> f64 {
        let m = self.used_margin();
        if m <= 0.0 {
            f64::INFINITY
        } else {
            self.equity() / m * 100.0
        }
    }

    /// Krok symulacji: nowe kwotowanie → wypełnienia i egzekucje.
    /// Zwraca liczbę zafillowanych zleceń i zamkniętych pozycji.
    /// Nalicza punkty swapowe za każdą przekroczoną północ czasu serwera.
    ///
    /// Znaczniki ticków są JUŻ w czasie serwera brokera, więc granica doby to
    /// `day_of(ts, 0)` — ta sama, po której silnik resetuje statystyki dnia.
    /// Środa liczy się potrójnie (`SWAP_ROLLOVER3DAYS = 3`), bo weekend
    /// rozlicza się z góry.
    ///
    /// Pętla po dobach, a nie pojedyncze naliczenie: przerwa świąteczna albo
    /// luka w danych potrafi przeskoczyć kilka dni naraz, a broker naliczy
    /// każdy z nich.
    fn nalicz_swap(&mut self, ts: Ts) {
        if !self.swap_enabled {
            return;
        }
        let dzis = day_of(ts, 0);
        if self.last_swap_day == i64::MIN {
            self.last_swap_day = dzis;
            return;
        }
        while self.last_swap_day < dzis {
            self.last_swap_day += 1;
            let d = self.last_swap_day;
            // D1: NOCE WEEKENDOWE SĄ JUŻ ZAPŁACONE.
            //
            // Potrójne rolowanie (wejście w czwartek) pokrywa sobotę
            // i niedzielę Z GÓRY — mówi o tym własny komentarz tej funkcji.
            // Mimo to pętla szła po każdej północy KALENDARZOWEJ i doliczała
            // te dwie noce drugi raz: tydzień kosztował 9 jednostek zamiast 7.
            // Na OMEGA-X2 to 20 671 $ z 46 681 $ całego swapu (44 %), i to
            // asymetrycznie — po stronie BUY, czyli 93 % sygnałów.
            //
            // Konwencja doby WEJŚCIA, spójna z `swap_rollover_weekday` obok.
            if self.swap_pomijaj_weekend {
                let wd = weekday_of(d * 86_400_000, 0);
                if wd == 5 || wd == 6 {
                    continue;
                }
            }
            let mult = if weekday_of(d * 86_400_000, 0) == self.swap_rollover_weekday {
                self.swap_rollover_mult
            } else {
                1.0
            };
            if self.native_swap_cash_digits.is_some() {
                // Compute the whole day first: malformed rate/overflow cannot
                // publish a partly-mutated account. Fault remains an entry HOLD.
                let mut accrued = Vec::with_capacity(self.positions.len());
                for p in &self.positions {
                    let points = if p.side == Side::Buy { self.swap_long_points } else { self.swap_short_points };
                    let before = self.swap_acc.get(&p.ticket).copied().unwrap_or(0.0);
                    let Some(amount) = self.native_swap_round(points * self.swap_point_value * p.volume * mult) else {
                        self.cost_ledger.latch("invalid native swap accrual/overflow"); return;
                    };
                    let Some(after) = self.native_swap_round(before + amount) else {
                        self.cost_ledger.latch("invalid native swap accumulated amount"); return;
                    };
                    accrued.push((p.ticket, before, amount, after));
                }
                let total = accrued.iter().fold(self.swap_total, |sum, item| sum + item.2);
                if !total.is_finite() {
                    self.cost_ledger.latch("native swap aggregate overflow"); return;
                }
                for (ticket, before, amount, after) in accrued {
                    self.swap_acc.insert(ticket, after);
                    if self.closed_profit_net_costs { self.cost_ledger.swap_native(ticket, before, after); }
                    self.swap_total += amount;
                }
                // Native observed contract: no Balance posting before close.
                continue;
            }
            let mut suma = 0.0;
            for p in &self.positions {
                let punkty = match p.side {
                    Side::Buy => self.swap_long_points,
                    Side::Sell => self.swap_short_points,
                };
                let kwota = punkty * self.swap_point_value * p.volume * mult;
                *self.swap_acc.entry(p.ticket).or_insert(0.0) += kwota;
                if self.closed_profit_net_costs {self.cost_ledger.swap(p.ticket,kwota);}
                suma += kwota;
            }
            self.balance += suma;
            self.swap_total += suma;
        }
    }

    /// D4: ZNACZNIK CZASU I CENY BEZ EGZEKUCJI.
    ///
    /// Robi dokładnie dwie rzeczy z `on_quote`: nalicza swap za przekroczone
    /// północe i przestawia bieżące kwotowanie. **Nie wypełnia zleceń, nie
    /// realizuje SL/TP, nie sprawdza stop-outu i nie rusza liczników poziomu
    /// marginesu.**
    ///
    /// Istnieje dla GRANICY DOBY w `runner.rs` i `okna.rs`. Tam potrzebny był
    /// wyłącznie efekt „noc kosztuje" (S-3): broker ma zobaczyć pierwszy kurs
    /// nowej doby, zanim policzymy wynik dnia. Użyty do tego `on_quote` robił
    /// przy okazji dwie rzeczy, o które nikt nie prosił:
    ///
    /// 1. **Odwracał kolejność.** Na zwykłym ticku pętla najpierw obsługuje
    ///    komunikaty, a potem egzekwuje; na pierwszym ticku doby egzekwowała
    ///    PRZED komunikatami nocnymi. Ten sam kurs znaczył więc co innego
    ///    w zależności od tego, czy wypadł na granicy doby.
    /// 2. **Liczyła ten sam tick po kilka razy.** Przy `daily_reset` tick
    ///    granicy przechodził przez `on_quote` trzykrotnie (granica, po
    ///    `EodFlat`, koniec pętli), więc liczniki `ml_pod_200/150/100`
    ///    dostawały trzy próbki zamiast jednej.
    ///
    /// Cel S-3 zostaje spełniony co do centa: swap i cena to jedyne, czego
    /// „noc kosztuje" wymaga.
    pub fn mark(&mut self, q: Quote) {
        let q = self.norm_quote(q);
        self.nalicz_swap(q.ts);
        self.q = q;
        self.account_control_equity = Some(self.equity());
    }

    pub(crate) fn take_account_control_equity(&mut self) -> Option<f64> {
        self.account_control_equity.take()
    }

    /// Execute one physical source-row at most once. Re-entering bookkeeping
    /// for that SAME row cannot fill newly placed orders or activate a new SL.
    /// The next distinct row executes even if all quote fields are identical.
    /// Invalid/reused identity fails before any price, swap or exposure change.
    pub fn on_observation(&mut self, observation_id: u64, q: Quote)
        -> Result<(usize, usize), &'static str>
    {
        if !q.bid.is_finite() || !q.ask.is_finite() {
            return Err("physical observation has a non-finite quote");
        }
        if let Some((previous_id, previous_quote)) = self.last_physical_observation {
            if observation_id < previous_id {
                return Err("physical observation identity moved backwards");
            }
            if observation_id == previous_id {
                if q.ts != previous_quote.ts
                    || q.bid.to_bits() != previous_quote.bid.to_bits()
                    || q.ask.to_bits() != previous_quote.ask.to_bits()
                {
                    return Err("physical observation identity reused for different quote");
                }
                // A message may temporarily expose an older quote (D3).
                // Restore the already observed current price for management,
                // but do not replay swap, fills, stops or margin sampling.
                self.q = self.norm_quote(q);
                return Ok((0, 0));
            }
        }
        self.last_physical_observation = Some((observation_id, q));
        Ok(self.on_quote(q))
    }

    /// B15 opt-in path for drivers with an immutable TickData row index.
    /// OFF never reads/writes observation state and preserves legacy calls.
    #[inline]
    pub(crate) fn on_tape_quote(&mut self, q: Quote, row: usize) -> (usize, usize) {
        if self.defer_new_pending_sl {
            self.on_observation(row as u64, q)
                .expect("driver must use the same immutable quote for one monotone tape row")
        } else {
            self.on_quote(q)
        }
    }

    pub fn on_quote(&mut self, q: Quote) -> (usize, usize) {
        let q = self.norm_quote(q);
        // Swap PRZED wypełnieniami i egzekucjami: pozycja, która przeżyła
        // północ, ma zapłacić za nią także wtedy, gdy w tym samym ticku
        // wychodzi na stopie.
        self.nalicz_swap(q.ts);
        self.q = q;
        let mut filled = 0;
        let mut closed = 0;
        // Per invocation, NOT open_ts == q.ts: two distinct quotes may have
        // the same millisecond. OFF creates no collection and changes no fill.
        let mut new_pending_tickets = self.defer_new_pending_sl
            .then(std::collections::HashSet::<Ticket>::new);

        // ---------- 1. wypełnienia zleceń oczekujących ----------
        // Quotes remain observable during a closed trade session; native
        // pending fills and server SL/TP wait for the first tradable tick.
        if self.trade_is_open() {
        let mut i = 0;
        while i < self.pendings.len() {
            let o = self.pendings[i].clone();
            let hit = match o.kind {
                PendingKind::BuyLimit => q.ask <= o.price,
                PendingKind::SellLimit => q.bid >= o.price,
                PendingKind::BuyStop => q.ask >= o.price,
                PendingKind::SellStop => q.bid <= o.price,
            };
            if !hit {
                i += 1;
                continue;
            }
            let usuniete = self.pendings.remove(i);
            if usuniete.placed_ts > 0 && q.ts >= usuniete.placed_ts {
                self.zycie_pend_fill.push(q.ts - usuniete.placed_ts);
            }
            let o = usuniete;

            let side = o.kind.side();
            let mkt = q.entry(side);
            let px = match o.kind {
                PendingKind::BuyLimit if self.limit_price_improvement => o.price.min(mkt),
                PendingKind::SellLimit if self.limit_price_improvement => o.price.max(mkt),
                PendingKind::BuyLimit | PendingKind::SellLimit => o.price,
                PendingKind::BuyStop => o.price.max(mkt),
                PendingKind::SellStop => o.price.min(mkt),
            };
            // Poślizg zleceń oczekujących jest jawnym parametrem. Domyślnie
            // wynosi zero, dopóki użytkownik nie poda modelu własnego brokera.
            let px = px + side.sign() * self.slippage_pending;

            if self.margin_check_on_fill {
                let need = o.volume * XAU_CONTRACT * px / self.leverage as f64;
                if self.equity() - self.used_margin() < need {
                    self.rejected_no_money += 1;
                    continue;
                }
            }
            let t = self.ticket();
            if self.closed_profit_net_costs {
                match crate::sim_costs::SimCostLedger::entry_pool(o.volume,self.volume_step,-o.volume*self.commission_per_lot) {
                    Ok(pool)=>self.cost_ledger.insert(t,pool),
                    Err(e)=>self.cost_ledger.latch(format!("pending fill cost pool: {e}")),
                }
            }
            self.positions.push(Position {
                ticket: t,
                side,
                volume: o.volume,
                open_price: px,
                open_ts: q.ts,
                sl: o.sl,
                tp: o.tp,
                vsl: None,
                basket: o.basket,
                level: o.level,
                frozen: false,
                peak_pts: 0.0,
                last_peak_ts: q.ts,
                is_runner: o.tp.is_none(),
                is_toucher: o.is_toucher,
                comment: o.comment.clone(),
            });
            if let Some(tickets)=&mut new_pending_tickets { tickets.insert(t); }
            self.unallocated_entry_cost_seen |= o.volume * self.commission_per_lot != 0.0;
            self.balance -= o.volume * self.commission_per_lot;
            self.spread_paid_usd += (q.ask - q.bid).max(0.0) * XAU_CONTRACT * o.volume;
            self.filled_pendings += 1;
            filled += 1;
        }

        // ---------- 2. SL / TP ----------
        let mut j = 0;
        while j < self.positions.len() {
            let p = self.positions[j].clone();
            let exit = q.exit(p.side);

            // SL ma pierwszeństwo przed TP w tym samym ticku
            let sl_hit = p.sl.map_or(false, |s| match p.side {
                Side::Buy => exit <= s,
                Side::Sell => exit >= s,
            });
            let tp_tez = p.tp.map_or(false, |t| match p.side {
                Side::Buy => exit >= t,
                Side::Sell => exit <= t,
            });
            if sl_hit && tp_tez {
                self.sl_tp_same_tick += 1;
            }

            let defer_this_sl = new_pending_tickets.as_ref()
                .is_some_and(|tickets|tickets.contains(&p.ticket));
            if sl_hit && !defer_this_sl {
                // Stop realizuje się po CENIE RYNKOWEJ, jeśli rynek zdążył
                // przeskoczyć poziom. Rozliczanie po poziomie stopa było dziurą
                // drukującą pieniądze: limit zrealizowany luką PONIŻEJ własnego
                // SL (kupno) zamykał się „na stopie" powyżej ceny otwarcia,
                // czyli z fikcyjnym zyskiem. Broker wykonuje wtedy zlecenie po
                // pierwszej dostępnej cenie rynkowej, co może oznaczać stratę.
                let lvl = p.sl.unwrap();
                let px = match p.side {
                    Side::Buy => lvl.min(exit),
                    Side::Sell => lvl.max(exit),
                };
                self.settle(j, px, CloseReason::Sl);
                closed += 1;
                continue;
            }

            let tp_hit = p.tp.map_or(false, |t| match p.side {
                Side::Buy => exit >= t,
                Side::Sell => exit <= t,
            });
            if tp_hit {
                let target = p.tp.unwrap();
                let improve = self.trade_sessions.as_ref().is_some_and(|p| p.take_profit_price_improvement);
                let px = if improve { match p.side {
                    Side::Buy => target.max(exit), Side::Sell => target.min(exit),
                }} else { target };
                self.settle(j, px, CloseReason::Tp);
                closed += 1;
                continue;
            }
            j += 1;
        }

        // ---------- 3. kontrola konta ----------
        }
        let eq = self.equity();
        if eq < self.min_equity {
            self.min_equity = eq;
        }
        self.account_control_equity = Some(eq);
        {
            let m = self.used_margin();
            if m > 0.0 {
                let ml = self.equity() / m * 100.0;
                if ml < self.min_margin_level {
                    self.min_margin_level = ml;
                }
                if ml < 200.0 {
                    self.ml_pod_200 += 1;
                }
                if ml < 150.0 {
                    self.ml_pod_150 += 1;
                }
                if ml < 100.0 {
                    self.ml_pod_100 += 1;
                }
                if m > self.max_open_margin {
                    self.max_open_margin = m;
                }
            }
            let vol: f64 = self.positions.iter().map(|p| p.volume).sum();
            if vol > self.max_open_volume {
                self.max_open_volume = vol;
            }
        }

        // ---------- STOP OUT ----------
        //
        // MT5 NIE zamyka wszystkiego naraz. Zamyka NAJBARDZIEJ STRATNĄ
        // pozycję, przelicza poziom marginu i powtarza, dopóki poziom nie
        // wróci ponad próg. Przy siatce wielu małych pozycji to zupełnie inny
        // przebieg niż jednorazowa likwidacja: zamknięcie jednej nogi zwalnia
        // margines i często wystarcza, żeby reszta przeżyła.
        //
        // To dotyczy JEDYNEGO progu bezwzględnego, jaki mamy („konto nie może
        // zostać wyzerowane"), więc różnica modelu zmienia nie wynik, tylko
        // odpowiedź na pytanie, czy wariant w ogóle wolno dopuścić.
        let prog = self.stop_out_level_pct / 100.0;
        loop {
            let m = self.used_margin();
            if m <= 0.0 || self.positions.is_empty() {
                break;
            }
            if self.equity() / m >= prog {
                break;
            }
            // najbardziej stratna pozycja — tak wybiera MT5
            let mut idx = 0usize;
            let mut najgorsza = f64::MAX;
            for (i, p) in self.positions.iter().enumerate() {
                let z = p.profit_usd(&self.q);
                if z < najgorsza {
                    najgorsza = z;
                    idx = i;
                }
            }
            let t = self.positions[idx].ticket;
            if self.close_position(t, CloseReason::MaxDd).is_err() {
                break;
            }
            self.stop_outs += 1;
            if self.stop_out_ts == 0 {
                self.stop_out_ts = self.q.ts;
                // saldo PRZED tą likwidacją już się zmieniło (`close_position`
                // rozliczyło pozycję), ale to jest właśnie stan, w którym
                // drabinka po raz PIERWSZY zobaczyłaby cokolwiek — wcześniej
                // saldo stało w miejscu przez całe zdarzenie.
                self.bal_przy_stopoucie = self.balance;
                self.eq_przy_stopoucie = self.equity();
            }
            closed += 1;
        }
        if self.positions.is_empty() && self.stop_outs > 0 {
            // po pełnej likwidacji broker kasuje też zlecenia oczekujące
            let m = self.used_margin();
            if m <= 0.0 && self.equity() < self.start_balance * 0.05 {
                self.pendings.clear();
            }
        }
        if eq <= 0.0 {
            self.blown = true;
        }

        (filled, closed)
    }

    fn settle(&mut self, idx: usize, px: Px, reason: CloseReason) {
        let p = self.positions.remove(idx);
        let profit = (px - p.open_price) * p.side.sign() * XAU_CONTRACT * p.volume;
        let native = self.native_swap_cash_digits.map(|_| self.native_swap_allocation(p.ticket,p.volume,p.volume));
        let swap = if let Some(a) = native {
            self.balance += profit + a.realized;
            a.realized
        } else {
            // Legacy swap was already posted at rollover; preserve its cash
            // and reporting operation order exactly when the profile is OFF.
            self.balance += profit;
            self.swap_acc.remove(&p.ticket).unwrap_or(0.0)
        };
        let tr = ClosedTrade {
            profit_basis: (!self.unallocated_entry_cost_seen).then_some(conduit_core::cost_receipt::ProfitBasis::PricePlusSwap), cost_receipt: None,
            ticket: p.ticket,
            side: p.side,
            volume: p.volume,
            open_price: p.open_price,
            close_price: px,
            open_ts: p.open_ts,
            close_ts: self.q.ts,
            profit: profit + swap,
            commission: 0.0,
            swap,
            reason,
            basket: p.basket,
        };
        self.record_cost_close(tr,profit,true,native);
    }

    /// Visible quarantine, not an automatic resume mechanism.
    pub fn cost_reconciliation_required(&self) -> Option<&str> {self.cost_ledger.fault.as_deref()}
    pub fn quarantined_cost_trades(&self) -> &[ClosedTrade] {&self.cost_ledger.quarantined}

    fn cost_spec_fingerprint(&self)->String {
        // Deterministic FNV-1a descriptor hash, explicitly not a broker signature.
        let mut data=format!("sim-cost-v1|{:x}|{:x}|{}|{:x}|{:x}|{:x}|{}|{:x}|{}|USD|exit-fee0|entry-fee0",
            self.commission_per_lot.to_bits(),self.volume_step.to_bits(),self.swap_enabled,
            self.swap_long_points.to_bits(),self.swap_short_points.to_bits(),self.swap_point_value.to_bits(),
            self.swap_rollover_weekday,self.swap_rollover_mult.to_bits(),self.swap_pomijaj_weekend);
        if let Some(digits) = self.native_swap_cash_digits { data.push_str(&format!("|native-swap-cash-v1|digits{digits}")); }
        let hash=data.bytes().fold(0xcbf29ce484222325u64,|h,b|(h^b as u64).wrapping_mul(0x100000001b3));
        format!("fnv1a64:{hash:016x}")
    }
    fn record_cost_close(&mut self,tr:ClosedTrade,gross:f64,full:bool,native_swap:Option<crate::sim_costs::NativeSwapAllocation>) {
        if native_swap.is_some() && self.cost_ledger.fault.is_some() {
            // Unknown native accrual stays unknown also with NET consumer OFF.
            // Protective execution already happened; do not publish fake profit.
            self.cost_ledger.quarantined.push(tr); return;
        }
        let trade=if self.closed_profit_net_costs {
            let spec=self.cost_spec_fingerprint();
            self.cost_ledger.project(tr,gross,full,spec,&self.cost_run_id,native_swap)
        } else {Some(tr)};
        if let Some(tr)=trade {self.history.push(tr.clone());self.closed.push(tr);}
    }

    fn cost_entry_guard(&mut self,volume:f64)->BResult<()> {
        if self.cost_ledger.fault.is_some() {return Err(BrokerError::Rejected);}
        if self.native_swap_cash_digits.is_some() && (!volume.is_finite() || volume<=0.0) {
            self.cost_ledger.latch("invalid volume under native swap cash profile");
            return Err(BrokerError::InvalidVolume);
        }
        if self.closed_profit_net_costs {
            if let Err(error)=crate::sim_costs::SimCostLedger::entry_pool(volume,self.volume_step,-volume*self.commission_per_lot) {
                self.cost_ledger.latch(format!("entry cost allocation is not representable: {error}"));
                return Err(BrokerError::InvalidVolume);
            }
        }
        Ok(())
    }
}

impl Broker for SimBroker {
    fn execution_session(&self) -> Option<ExecutionSession> { self.synthetic_continuation_session.clone() }
    fn quote(&self) -> Quote {
        self.q
    }

    fn account(&self) -> Account {
        let eq = self.equity();
        let m = self.used_margin();
        Account {
            balance: self.balance,
            equity: eq,
            margin: m,
            free_margin: eq - m,
            leverage: self.leverage,
            // Model MT5 dodaje tę osobną kwotę w equity(), nie w balance.
            credit: self.credit,
        }
    }

    fn stops_level(&self) -> f64 {
        self.stops_level
    }

    fn volume_min(&self) -> f64 {
        self.volume_min
    }

    fn volume_step(&self) -> f64 {
        self.volume_step
    }

    fn volume_max(&self) -> f64 { self.volume_max }
    fn normalize_order_price(&self, price:f64)->f64 { self.norm_price(price) }
    fn pending_cancel_snapshot_authoritative(&self) -> bool { true }
    // In this model tickets are the immutable synthetic position IDs. This is
    // explicitly NOT the live MT5 alias-to-POSITION_IDENTIFIER convention.
    fn position_identifier(&self, ticket: Ticket) -> Option<u64> {
        self.find_position(ticket).map(|p|p.ticket)
    }
    fn cost_net_supported(&self) -> bool {self.closed_profit_net_costs && self.cost_ledger.fault.is_none()}
    fn report_cost_consumer_fault(&mut self,reason:&str) {self.cost_ledger.latch(reason);}
    fn receipt_barrier(&self) -> ReceiptBarrier {
        if self.cost_ledger.fault.is_some() {ReceiptBarrier::RequiresReview} else {ReceiptBarrier::Clear}
    }

    fn positions(&self) -> &[Position] {
        &self.positions
    }
    fn pendings(&self) -> &[PendingOrder] {
        &self.pendings
    }
    fn positions_mut(&mut self) -> &mut Vec<Position> {
        &mut self.positions
    }
    fn pendings_mut(&mut self) -> &mut Vec<PendingOrder> {
        &mut self.pendings
    }

    fn open_market(&mut self, mut r: OrderReq) -> BResult<Ticket> {
        self.cost_entry_guard(r.volume)?;
        r.sl = r.sl.map(|p| self.norm_price(p));
        r.tp = r.tp.map(|p| self.norm_price(p));
        if self.order_volume_contract_v2 {
            conduit_core::volume_contract::validate_broker_volume(r.volume,
                conduit_core::volume_contract::VolumeSpec {
                    minimum: self.volume_min, step: self.volume_step, maximum: self.volume_max,
                }).map_err(|_| BrokerError::InvalidVolume)?;
        } else if r.volume < 0.01 {
            return Err(BrokerError::InvalidVolume);
        }
        let px = self.q.entry(r.side) + r.side.sign() * self.slippage;

        // broker odrzuca zlecenie z niewykonalnym SL/TP
        if let Some(s) = r.sl {
            if !sl_is_valid(r.side, s, &self.q, self.stops_level) {
                self.rejected_stops += 1;
                return Err(BrokerError::InvalidStops);
            }
        }
        if let Some(t) = r.tp {
            if !tp_is_valid(r.side, t, &self.q, self.stops_level) {
                self.rejected_stops += 1;
                return Err(BrokerError::InvalidStops);
            }
        }

        self.require_trade_open()?;
        let need = r.volume * XAU_CONTRACT * px / self.leverage as f64;
        if self.equity() - self.used_margin() < need {
            return Err(BrokerError::NotEnoughMargin);
        }

        let t = self.ticket();
        if self.closed_profit_net_costs {
            let pool=crate::sim_costs::SimCostLedger::entry_pool(r.volume,self.volume_step,-r.volume*self.commission_per_lot)
                .map_err(|_|BrokerError::InvalidVolume)?;
            self.cost_ledger.insert(t,pool);
        }
        self.positions.push(Position {
            ticket: t,
            side: r.side,
            volume: r.volume,
            open_price: px,
            open_ts: self.q.ts,
            sl: r.sl,
            tp: r.tp,
            vsl: None,
            basket: r.basket,
            level: r.level,
            frozen: false,
            peak_pts: 0.0,
            last_peak_ts: self.q.ts,
            is_runner: r.tp.is_none(),
            is_toucher: r.is_toucher,
            comment: r.comment,
        });
        self.unallocated_entry_cost_seen |= r.volume * self.commission_per_lot != 0.0;
        self.balance -= r.volume * self.commission_per_lot;
        self.spread_paid_usd += (self.q.ask - self.q.bid).max(0.0) * XAU_CONTRACT * r.volume;
        Ok(t)
    }

    fn place_pending(&mut self, mut r: PendingReq) -> BResult<Ticket> {
        self.cost_entry_guard(r.volume)?;
        r.price = self.norm_price(r.price);
        r.sl = r.sl.map(|p| self.norm_price(p));
        r.tp = r.tp.map(|p| self.norm_price(p));
        if self.order_volume_contract_v2 {
            conduit_core::volume_contract::validate_broker_volume(r.volume,
                conduit_core::volume_contract::VolumeSpec {
                    minimum: self.volume_min, step: self.volume_step, maximum: self.volume_max,
                }).map_err(|_| BrokerError::InvalidVolume)?;
        } else if r.volume < 0.01 {
            return Err(BrokerError::InvalidVolume);
        }
        let side = r.kind.side();

        // Poziom, na którym zlecenie oczekujące NIE MOŻE leżeć: albo rynek już
        // je minął, albo leży bliżej ceny niż `stops_level`. Drugiego warunku
        // przez długi czas tu nie było i to jest dokładnie ta różnica, przez
        // którą backtest pokazywał wejście, a żywy broker zwracał `10015`.
        // Siatką ratunkową jest wejście po rynku — tak samo jak w moście.
        let crossed = match r.kind {
            PendingKind::BuyLimit | PendingKind::SellLimit => {
                !limit_price_is_valid(side, r.price, &self.q, self.stops_level)
            }
            PendingKind::BuyStop | PendingKind::SellStop => {
                !stop_price_is_valid(side, r.price, &self.q, self.stops_level)
            }
        };
        if crossed {
            self.market_instead_of_limit += 1;
            return self.open_market(OrderReq {
                side,
                volume: r.volume,
                sl: r.sl,
                tp: r.tp,
                basket: r.basket,
                level: r.level,
                is_toucher: r.is_toucher,
                comment: r.comment,
            });
        }

        // ---------- WALIDACJA SL/TP WZGLĘDEM CENY ZLECENIA (10016) ----------
        //
        // Dopiero TU, po gałęzi `crossed`: zlecenie, które rynek już minął,
        // idzie po rynku i jego SL/TP ocenia `open_market` względem kursu —
        // tak samo jak u brokera. Walidacja 10016 dotyczy wyłącznie zlecenia,
        // które naprawdę ma zostać POŁOŻONE.
        if self.validate_pending_stops {
            let zle_sl =
                r.sl.map(|s| !pending_sl_is_valid(side, s, r.price, self.stops_level))
                    .unwrap_or(false);
            let zle_tp =
                r.tp.map(|t| !pending_tp_is_valid(side, t, r.price, self.stops_level))
                    .unwrap_or(false);
            if zle_sl || zle_tp {
                self.rejected_pending_stops += 1;
                return Err(BrokerError::InvalidStops);
            }
        }

        self.require_trade_open()?;
        let t = self.ticket();
        self.pendings.push(PendingOrder {
            ticket: t,
            kind: r.kind,
            volume: r.volume,
            price: r.price,
            sl: r.sl,
            tp: r.tp,
            placed_ts: self.q.ts,
            basket: r.basket,
            level: r.level,
            frozen: false,
            is_toucher: r.is_toucher,
            is_topup: r.is_topup,
            comment: r.comment,
        });
        Ok(t)
    }

    fn modify_position(&mut self, t: Ticket, sl: Option<Px>, tp: Option<Px>) -> BResult<()> {
        let trade_open = self.trade_is_open();
        let sl = sl.map(|p| self.norm_price(p));
        let tp = tp.map(|p| self.norm_price(p));
        let q = self.q;
        let stops = self.stops_level;
        let p = self
            .positions
            .iter_mut()
            .find(|p| p.ticket == t)
            .ok_or(BrokerError::NoSuchTicket)?;
        if let Some(s) = sl {
            if !sl_is_valid(p.side, s, &q, stops) {
                return Err(BrokerError::InvalidStops);
            }
        }
        if let Some(v) = tp {
            if !tp_is_valid(p.side, v, &q, stops) {
                return Err(BrokerError::InvalidStops);
            }
        }
        if !trade_open {
            self.market_closed_rejections += 1;
            return Err(BrokerError::MarketClosed);
        }
        p.sl = sl;
        p.tp = tp;
        Ok(())
    }

    fn modify_pending(
        &mut self,
        t: Ticket,
        price: Px,
        sl: Option<Px>,
        tp: Option<Px>,
    ) -> BResult<()> {
        // Modyfikacja jest u brokera osobnym żądaniem i osobno przechodzi
        let price = self.norm_price(price);
        let sl = sl.map(|p| self.norm_price(p));
        let tp = tp.map(|p| self.norm_price(p));
        // walidację: NOWA cena wobec rynku, NOWE SL/TP wobec NOWEJ ceny.
        // Odmowa NIE zmienia zlecenia — zostaje takie, jakie było.
        if self.validate_pending_stops {
            let kind = self
                .pendings
                .iter()
                .find(|o| o.ticket == t)
                .map(|o| o.kind)
                .ok_or(BrokerError::NoSuchTicket)?;
            let side = kind.side();
            let cena_ok = match kind {
                PendingKind::BuyLimit | PendingKind::SellLimit => {
                    limit_price_is_valid(side, price, &self.q, self.stops_level)
                }
                PendingKind::BuyStop | PendingKind::SellStop => {
                    stop_price_is_valid(side, price, &self.q, self.stops_level)
                }
            };
            if !cena_ok {
                return Err(BrokerError::InvalidPrice);
            }
            let sl_ok = sl.map_or(true, |s| {
                pending_sl_is_valid(side, s, price, self.stops_level)
            });
            let tp_ok = tp.map_or(true, |v| {
                pending_tp_is_valid(side, v, price, self.stops_level)
            });
            if !sl_ok || !tp_ok {
                self.rejected_pending_stops += 1;
                return Err(BrokerError::InvalidStops);
            }
        }
        self.require_trade_open()?;
        let o = self
            .pendings
            .iter_mut()
            .find(|o| o.ticket == t)
            .ok_or(BrokerError::NoSuchTicket)?;
        o.price = price;
        o.sl = sl;
        o.tp = tp;
        Ok(())
    }

    fn close_position(&mut self, t: Ticket, reason: CloseReason) -> BResult<f64> {
        let idx = self
            .positions
            .iter()
            .position(|p| p.ticket == t)
            .ok_or(BrokerError::NoSuchTicket)?;
        let side = self.positions[idx].side;
        self.require_trade_open()?;
        let px = self.q.exit(side);
        let before = self.balance;
        self.settle(idx, px, reason);
        Ok(self.balance - before)
    }

    fn close_partial(&mut self, t: Ticket, volume: f64, reason: CloseReason) -> BResult<f64> {
        let idx = self
            .positions
            .iter()
            .position(|p| p.ticket == t)
            .ok_or(BrokerError::NoSuchTicket)?;
        if !volume.is_finite() || volume <= 0.0 {
            return Err(BrokerError::InvalidVolume);
        }
        self.require_trade_open()?;
        let current = self.positions[idx].volume;
        let Some(vol) = partial_close_volume(current, volume, self.volume_min, self.volume_step)
        else {
            // Taki sam kontrakt jak most live: gdy zadana transza zamknelaby
            // calosc albo zostawila resztke < minimum, zamykamy caly ticket.
            return self.close_position(t, reason);
        };
        let p = self.positions[idx].clone();
        let px = self.q.exit(p.side);
        let profit = (px - p.open_price) * p.side.sign() * XAU_CONTRACT * vol;
        let native = self.native_swap_cash_digits.map(|_| self.native_swap_allocation(t,vol,current));
        if let Some(a) = native { self.balance += profit + a.realized; }
        else { self.balance += profit; }
        self.positions[idx].volume -= vol;
        let tr = ClosedTrade {
            profit_basis: (!self.unallocated_entry_cost_seen).then_some(conduit_core::cost_receipt::ProfitBasis::PricePlusSwap), cost_receipt: None,
            ticket: p.ticket,
            side: p.side,
            volume: vol,
            open_price: p.open_price,
            close_price: px,
            open_ts: p.open_ts,
            close_ts: self.q.ts,
            profit: if let Some(a) = native { profit + a.realized } else { profit },
            commission: 0.0,
            swap: native.map_or(0.0, |a| a.realized),
            reason,
            basket: p.basket,
        };
        self.record_cost_close(tr,profit,false,native);
        Ok(if let Some(a) = native { profit + a.realized } else { profit })
    }

    fn cancel_pending(&mut self, t: Ticket) -> BResult<()> {
        self.require_trade_open()?;
        let i = self
            .pendings
            .iter()
            .position(|o| o.ticket == t)
            .ok_or(BrokerError::NoSuchTicket)?;
        let usuniete = self.pendings.remove(i);
        if usuniete.placed_ts > 0 && self.q.ts >= usuniete.placed_ts {
            self.zycie_pend_anul.push(self.q.ts - usuniete.placed_ts);
        }
        Ok(())
    }

    fn drain_closed(&mut self) -> Vec<ClosedTrade> {
        std::mem::take(&mut self.closed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use conduit_core::engine::IncomingMessage;

    fn q(bid: f64, spread: f64) -> Quote {
        Quote {
            ts: 1_000,
            bid,
            ask: bid + spread,
        }
    }

    fn realized_fixture(enabled: bool) -> (conduit_core::engine::Engine, SimBroker, u32, Ticket) {
        use conduit_core::{engine::Engine, settings::RiskFreeMode};
        let mut cfg = Settings::default();
        cfg.basket_realized_broker_only = enabled;
        cfg.risk_free_mode = RiskFreeMode::CloseEverything;
        cfg.server_tz_offset_ms = 0;
        cfg.exec_latency_ms = 0;
        let mut e = Engine::new(cfg, 1000.0);
        let mut b = SimBroker::new(1000.0, 0.2, 7.0);
        b.on_quote(q(4010.0, 0.2));
        e.on_message(&mut b, &IncomingMessage {
            ts: 1000, source: SourceKey::new(1, None), source_name: "TEST".into(),
            msg_id: 1, reply_to: None, edit_of: None,
            text: "BUY LIMITS GOLD @ 4000/3995\nTP 4030\nTP 4040\nSL 3980".into(),
        });
        assert_eq!(e.baskets.len(), 1, "fixture must create a real parsed basket");
        let id = e.baskets[0].id;
        let pending_ids: Vec<_> = b.pendings().iter().map(|p| p.ticket).collect();
        for t in pending_ids { b.cancel_pending(t).unwrap(); }
        b.mark(q(4000.0, 0.2));
        let t = b.open_market(OrderReq {
            side: Side::Buy, volume: 0.08, sl: None, tp: None,
            basket: Some(id), level: 0, is_toucher: false, comment: "ledger-regression".into(),
        }).unwrap();
        e.on_tick(&mut b, &q(4000.0, 0.2));
        assert_eq!(b.positions().len(), 1);
        (e, b, id, t)
    }

    #[test]
    fn basket_realized_broker_only_riskfree_books_once_and_legacy_stays() {
        for enabled in [false, true] {
            let (mut e, mut b, id, _) = realized_fixture(enabled);
            let profitable = Quote { ts: 2000, bid: 4005.0, ask: 4005.2 };
            b.mark(profitable);
            e.on_message(&mut b, &IncomingMessage {
                ts: 2000, source: SourceKey::new(1, None), source_name: "TEST".into(),
                msg_id: 2, reply_to: Some(1), edit_of: None, text: "RISK FREE".into(),
            });
            assert!(b.positions().is_empty(), "RF must execute the real close_position path");
            assert_eq!(b.history.len(), 1);
            let truth = b.history[0].profit;
            assert!(truth > 0.0);
            // SIM pobiera prowizję przy wejściu, poza ClosedTrade.profit.
            // Ta poprawka księguje dokładnie kontrakt ledgera, nie zmienia
            // konwencji kosztów ani wyniku rachunku.
            assert!((b.account().balance - 1000.0 - truth + 0.08 * 7.0).abs() < 1e-9);
            let before_drain = e.baskets.iter().find(|x| x.id == id).unwrap().realized;
            assert!((before_drain - if enabled { 0.0 } else { truth }).abs() < 1e-9);
            e.on_tick(&mut b, &profitable);
            let expected = truth * if enabled { 1.0 } else { 2.0 };
            assert!((e.baskets.iter().find(|x| x.id == id).unwrap().realized - expected).abs() < 1e-9);
            assert_eq!(e.stats.trades, 1, "global ledger never counts command return twice");
            e.on_tick(&mut b, &Quote { ts: 3000, ..profitable });
            assert!((e.baskets.iter().find(|x| x.id == id).unwrap().realized - expected).abs() < 1e-9,
                "an empty drain must be idempotent");
        }
    }

    #[test]
    fn basket_realized_broker_only_preserves_multiple_partials_same_ticket() {
        let (mut e, mut b, id, t) = realized_fixture(true);
        let profitable = Quote { ts: 2000, bid: 4005.0, ask: 4005.2 };
        b.mark(profitable);
        b.close_partial(t, 0.02, CloseReason::Partial).unwrap();
        b.close_partial(t, 0.02, CloseReason::Partial).unwrap();
        e.on_tick(&mut b, &profitable);
        assert_eq!(e.stats.trades, 2, "different executions of one ticket must not be deduplicated");
        assert!(e.baskets.iter().find(|x| x.id == id).unwrap().tickets.contains(&t));
        b.close_position(t, CloseReason::BasketClose).unwrap();
        e.on_tick(&mut b, &Quote { ts: 3000, ..profitable });
        let truth: f64 = b.history.iter().map(|c| c.profit).sum();
        assert_eq!(b.history.len(), 3);
        assert_eq!(e.stats.trades, 3);
        assert!((e.baskets.iter().find(|x| x.id == id).unwrap().realized - truth).abs() < 1e-9);
    }

    /// Zlecenie oczekujące z pełnym kompletem pól — żeby testy niżej różniły
    /// się WYŁĄCZNIE tym, co badają.
    fn pending(price: f64, sl: Option<f64>, tp: Option<f64>) -> PendingReq {
        PendingReq {
            kind: PendingKind::BuyLimit,
            price,
            volume: 0.01,
            sl,
            tp,
            basket: None,
            level: 0,
            is_toucher: false,
            comment: String::new(),
            is_topup: false,
        }
    }

    #[test]
    fn broker_price_precision_is_explicit_and_recovers_decimal_tick_touch() {
        for enabled in [false, true] {
            for (kind, level, initial_bid, touch_bid, touch_ask) in [
                (PendingKind::BuyLimit, 4599.428571428572, 4610.0, 4599.21, 4599.43),
                (PendingKind::SellLimit, 4610.714285714285, 4600.0, 4610.71, 4610.93),
            ] {
                let mut b = SimBroker::new(1000.0, 0.20, 0.0);
                assert_eq!(b.price_digits, None);
                b.price_digits = enabled.then_some(2);
                b.on_quote(q(initial_bid, 0.22));
                let mut req = pending(level, None, None);
                req.kind = kind;
                let id = b.place_pending(req).unwrap();
                assert_eq!(b.pendings()[0].price, if enabled { (level * 100.0).round()/100.0 } else { level });
                b.modify_pending(id, level, None, None).unwrap();
                let tick = Quote { ts: 2000, bid: touch_bid as f32 as f64, ask: touch_ask as f32 as f64 };
                let (fills, _) = b.on_quote(tick);
                assert_eq!(fills, usize::from(enabled), "{kind:?} enabled={enabled}");
                assert_eq!(b.q.ask, if enabled { touch_ask } else { tick.ask });
            }
        }
    }

    /// SZCZEBEL-WIDMO: SL leży DOKŁADNIE na cenie szczebla, czyli w odległości
    /// zero od ceny aktywacji. MT5 odrzuca to kodem 10016 i taki poziom nigdy
    /// nie zaistnieje — a symulator liczył z niego wynik.
    #[test]
    fn widmo_sl_na_cenie_zlecenia_jest_odrzucane() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.validate_pending_stops = true;
        b.on_quote(q(4010.0, 0.30));
        let r = b.place_pending(pending(4000.0, Some(4000.0), Some(4020.0)));
        assert_eq!(
            r,
            Err(BrokerError::InvalidStops),
            "SL w odległości 0 od ceny zlecenia"
        );
        assert_eq!(b.rejected_pending_stops, 1);
        assert!(
            b.pendings.is_empty(),
            "odrzucone zlecenie nie może zostać w księdze"
        );

        // to samo dla CELU po złej stronie ceny aktywacji
        let r = b.place_pending(pending(4000.0, Some(3990.0), Some(4000.1)));
        assert_eq!(
            r,
            Err(BrokerError::InvalidStops),
            "TP bliżej niż stops_level"
        );
        assert_eq!(b.rejected_pending_stops, 2);
    }

    #[test]
    fn bez_flagi_widmo_przechodzi_jak_dawniej() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.on_quote(q(4010.0, 0.30));
        let t = b
            .place_pending(pending(4000.0, Some(4000.0), Some(4020.0)))
            .expect("bez flagi widmo ma przechodzić");
        assert!(b.pendings.iter().any(|p| p.ticket == t));
        assert_eq!(
            b.rejected_pending_stops, 0,
            "licznik jest martwy przy wyłączonej fladze"
        );
    }

    /// Flaga nie może odrzucać zleceń POPRAWNYCH — to jest jej granica.
    #[test]
    fn poprawny_pending_przechodzi_z_flaga() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.validate_pending_stops = true;
        b.on_quote(q(4010.0, 0.30));
        let t = b
            .place_pending(pending(4000.0, Some(3995.0), Some(4020.0)))
            .expect("SL 5 $ pod ceną zlecenia jest wykonalny");
        assert!(b.pendings.iter().any(|p| p.ticket == t));
        assert_eq!(b.rejected_pending_stops, 0);

        // zlecenie BEZ stopów też, i to w obu ustawieniach flagi
        assert!(b.place_pending(pending(3990.0, None, None)).is_ok());
        assert_eq!(b.rejected_pending_stops, 0);
    }

    /// Modyfikacja to u brokera osobne żądanie i osobno przechodzi walidację.
    /// Odmowa NIE MOŻE zmienić zlecenia — inaczej symulator trzymałby stan,
    /// którego u brokera nie ma.
    #[test]
    fn modyfikacja_pendingu_jest_walidowana() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.validate_pending_stops = true;
        b.on_quote(q(4010.0, 0.30));
        let t = b
            .place_pending(pending(4000.0, Some(3995.0), Some(4020.0)))
            .expect("wyjściowe zlecenie jest poprawne");

        // SL dosunięty na cenę zlecenia — odmowa 10016
        let r = b.modify_pending(t, 4000.0, Some(4000.0), Some(4020.0));
        assert_eq!(r, Err(BrokerError::InvalidStops));
        assert_eq!(b.rejected_pending_stops, 1);
        let o = b
            .pendings
            .iter()
            .find(|o| o.ticket == t)
            .expect("zlecenie zostaje");
        assert_eq!(o.sl, Some(3995.0), "odmowa nie zmienia stopu");
        assert_eq!(o.price, 4000.0, "ani ceny");

        // cena przesunięta nad rynek — to już InvalidPrice, nie InvalidStops
        let r = b.modify_pending(t, 4015.0, Some(4010.0), Some(4030.0));
        assert_eq!(r, Err(BrokerError::InvalidPrice));
        assert_eq!(
            b.rejected_pending_stops, 1,
            "zły powód nie może podbijać licznika 10016"
        );

        // poprawna modyfikacja przechodzi
        b.modify_pending(t, 3998.0, Some(3990.0), Some(4020.0))
            .expect("poprawna zmiana");
        let o = b
            .pendings
            .iter()
            .find(|o| o.ticket == t)
            .expect("zlecenie zostaje");
        assert_eq!(o.price, 3998.0);
        assert_eq!(o.sl, Some(3990.0));
    }

    /// REGRESJA: stop nie może dać ZYSKU.
    ///
    /// Scenariusz: limit kupna na 4000 ze stopem 3995. Rynek nie schodzi
    /// łagodnie, tylko przeskakuje od razu na 3990 — czyli limit realizuje się
    /// PONIŻEJ własnego stopa. Rozliczenie po poziomie stopa (3995) dawało
    /// wtedy +5 $ zysku z pozycji, która w rzeczywistości jest pod wodą.
    /// Taki model tworzyłby sztuczny zysk i nie może być używany jako wzorzec.
    #[test]
    fn stop_nie_moze_dac_zysku_po_luce() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.on_quote(q(4010.0, 0.30));
        let t = b
            .place_pending(PendingReq {
                kind: PendingKind::BuyLimit,
                price: 4000.0,
                volume: 0.01,
                sl: Some(3995.0),
                tp: Some(4020.0),
                basket: None,
                level: 0,
                is_toucher: false,
                comment: String::new(),
                is_topup: false,
            })
            .expect("limit powinien zostać przyjęty");
        assert!(b.pendings.iter().any(|p| p.ticket == t));

        // luka: cena przeskakuje pod stopa
        let (filled, closed) = b.on_quote(q(3990.0, 0.30));
        assert_eq!(filled, 1, "limit powinien się zrealizować luką");
        assert_eq!(closed, 1, "pozycja powinna od razu wypaść na stopie");

        let saldo = b.balance;
        assert!(
            saldo < 1000.0,
            "stop po luce musi dać STRATĘ, a saldo wyszło {saldo:.2} $ \
             (rozliczenie po poziomie stopa drukowałoby zysk)"
        );
    }

    #[test]
    fn spread_jest_placony_realnie() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.on_quote(q(4000.0, 0.24));
        let t = b
            .open_market(OrderReq {
                side: Side::Buy,
                volume: 0.01,
                sl: None,
                tp: None,
                basket: None,
                level: 0,
                is_toucher: false,
                comment: String::new(),
            })
            .unwrap();
        // wejście po ASK 4000.24, wyjście po BID 4000.00 → strata = spread
        let p = b.close_position(t, CloseReason::Manual).unwrap();
        assert!((p - (-0.24)).abs() < 1e-6, "zysk {p}");
    }

    #[test]
    fn sl_po_zlej_stronie_jest_odrzucany() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.on_quote(q(4000.0, 0.20));
        // BUY z SL POWYŻEJ ceny — broker musi odmówić
        let r = b.open_market(OrderReq {
            side: Side::Buy,
            volume: 0.01,
            sl: Some(4005.0),
            tp: None,
            basket: None,
            level: 0,
            is_toucher: false,
            comment: String::new(),
        });
        assert_eq!(r, Err(BrokerError::InvalidStops));
    }

    #[test]
    fn sl_zbyt_blisko_ceny_jest_odrzucany() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.on_quote(q(4000.0, 0.20));
        let r = b.open_market(OrderReq {
            side: Side::Buy,
            volume: 0.01,
            sl: Some(3999.95), // 5 centów pod BID, a stops level to 20
            tp: None,
            basket: None,
            level: 0,
            is_toucher: false,
            comment: String::new(),
        });
        assert_eq!(r, Err(BrokerError::InvalidStops));
        assert_eq!(b.rejected_stops, 1);
    }

    #[test]
    fn limit_nie_realizuje_sie_gorzej_niz_rynek() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.on_quote(q(4000.0, 0.20)); // ask 4000.20
                                     // BUY LIMIT 4010 przy rynku 4000.20 — rynek już jest lepszy
        let t = b
            .place_pending(PendingReq {
                kind: PendingKind::BuyLimit,
                volume: 0.01,
                price: 4010.0,
                sl: None,
                tp: None,
                basket: None,
                level: 0,
                is_toucher: false,
                comment: String::new(),
                is_topup: false,
            })
            .unwrap();
        let p = b.find_position(t).expect("powinna powstać pozycja rynkowa");
        assert!(
            (p.open_price - 4000.20).abs() < 1e-9,
            "cena {}",
            p.open_price
        );
        assert_eq!(b.market_instead_of_limit, 1);
    }

    #[test]
    fn limit_fillowany_po_cenie_zlecenia() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.on_quote(q(4010.0, 0.20));
        b.place_pending(PendingReq {
            kind: PendingKind::BuyLimit,
            volume: 0.01,
            price: 4000.0,
            sl: None,
            tp: None,
            basket: None,
            level: 0,
            is_toucher: false,
            comment: String::new(),
            is_topup: false,
        })
        .unwrap();
        assert_eq!(b.pendings().len(), 1);
        // cena schodzi tak, że ASK dotyka 4000
        let (f, _) = b.on_quote(q(3999.80, 0.20));
        assert_eq!(f, 1);
        assert_eq!(b.positions().len(), 1);
        assert!((b.positions()[0].open_price - 4000.0).abs() < 1e-9);
    }

    #[test]
    fn limit_price_improvement_is_optional_and_stop_execution_is_unchanged() {
        let cases = [
            (PendingKind::BuyLimit, 4010.0, 3999.0, 3999.20),
            (PendingKind::SellLimit, 3990.0, 4001.0, 4001.0),
            (PendingKind::BuyStop, 3990.0, 4001.0, 4001.20),
            (PendingKind::SellStop, 4010.0, 3999.0, 3999.0),
        ];
        for (kind, initial_bid, fill_bid, improved) in cases {
            for enabled in [false, true] {
                let mut b = SimBroker::new(1000.0, 0.20, 0.0);
                assert!(!b.limit_price_improvement);
                b.limit_price_improvement = enabled;
                b.on_quote(q(initial_bid, 0.20));
                let mut req = pending(4000.0, None, None);
                req.kind = kind;
                b.place_pending(req).unwrap();
                let (filled, closed) = b.on_quote(q(fill_bid, 0.20));
                assert_eq!((filled, closed), (1, 0));
                let is_limit = matches!(kind, PendingKind::BuyLimit | PendingKind::SellLimit);
                let expected = if is_limit && !enabled { 4000.0 } else { improved };
                assert!((b.positions()[0].open_price - expected).abs() < 1e-8,
                    "{kind:?} enabled={enabled}: {} vs {expected}", b.positions()[0].open_price);
            }
        }
    }

    #[test]
    fn limit_price_improvement_does_not_create_profit_on_gap_through_sl() {
        for enabled in [false, true] {
            let mut b = SimBroker::new(1000.0, 0.20, 0.0);
            b.limit_price_improvement = enabled;
            b.on_quote(q(4010.0, 0.20));
            b.place_pending(pending(4000.0, Some(3995.0), None)).unwrap();
            assert_eq!(b.on_quote(q(3990.0, 0.20)), (1, 1));
            assert!(b.balance < 1000.0, "SL must execute at the gap quote, not at its old level");
            assert!(b.positions().is_empty());
        }
    }

    /// Stop wykonuje się po cenie rynkowej z ticka, który przebił poziom —
    /// nigdy lepiej niż poziom.
    ///
    /// Wcześniej test wymuszał zamknięcie DOKŁADNIE na 3990 nawet wtedy, gdy
    /// rynek był już na 3989. To model bez poślizgu, wygodny, ale nieprawdziwy:
    /// SL w MT5 jest zleceniem stop i realizuje się po pierwszej dostępnej
    /// cenie. Idealizacja miała też groźniejszą konsekwencję — pozwalała
    /// zamknąć „na stopie" pozycję otwartą luką pod stopem, czyli z zyskiem
    /// (patrz `stop_nie_moze_dac_zysku_po_luce`).
    #[test]
    fn sl_zamyka_po_cenie_rynkowej_nie_lepiej_niz_poziom() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.on_quote(q(4000.0, 0.20));
        b.open_market(OrderReq {
            side: Side::Buy,
            volume: 0.01,
            sl: Some(3990.0),
            tp: Some(4010.0),
            basket: None,
            level: 0,
            is_toucher: false,
            comment: String::new(),
        })
        .unwrap();
        let (_, c) = b.on_quote(q(3989.0, 0.20));
        assert_eq!(c, 1);
        let tr = &b.closed[0];
        assert_eq!(tr.reason, CloseReason::Sl);
        // tick przebił poziom o 1 $ → wykonanie po 3989, nie po 3990
        assert!(
            (tr.close_price - 3989.0).abs() < 1e-9,
            "cena {}",
            tr.close_price
        );
        // wejście 4000.20, wyjście 3989 → −11.20 pkt × 100 × 0.01 = −11.20 $
        assert!((tr.profit - (-11.20)).abs() < 1e-6, "zysk {}", tr.profit);
    }

    #[test]
    fn sl_ma_pierwszenstwo_przed_tp_w_tym_samym_ticku() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.on_quote(q(4000.0, 0.20));
        b.open_market(OrderReq {
            side: Side::Buy,
            volume: 0.01,
            sl: Some(3990.0),
            tp: Some(4010.0),
            basket: None,
            level: 0,
            is_toucher: false,
            comment: String::new(),
        })
        .unwrap();
        // ogromna luka przeskakująca oba poziomy
        b.on_quote(q(3980.0, 0.20));
        assert_eq!(b.closed[0].reason, CloseReason::Sl);
    }


    /// Doba 0 od epoki to 1970-01-01, czyli CZWARTEK. `weekday_of` liczy
    /// poniedziałek jako 0, więc środa (=2) wypada tam, gdzie `(d + 3) % 7 == 2`,
    /// czyli w dobach 6, 13, 20…
    fn ts_dnia(dzien: i64, godz: i64) -> Ts {
        dzien * 86_400_000 + godz * 3_600_000
    }

    #[test]
    fn kontrola_arytmetyki_dnia_tygodnia() {
        assert_eq!(weekday_of(ts_dnia(6, 0), 0), 2, "doba 6 to środa");
        assert_eq!(weekday_of(ts_dnia(7, 0), 0), 3, "doba 7 to czwartek");
    }

    fn kup_jedna(b: &mut SimBroker, side: Side) -> Ticket {
        b.open_market(OrderReq {
            side,
            volume: 0.01,
            sl: None,
            tp: None,
            basket: None,
            level: 0,
            is_toucher: false,
            comment: String::new(),
        })
        .unwrap()
    }

    /// SWAP: BUY 0,01 lota kosztuje −0,7582 $ za noc.
    /// Potwierdzone dealami z zanonimizowanego rachunku demo: −0,75 i −0,76.
    #[test]
    fn swap_obciaza_pozycje_dluga_za_kazda_noc() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.swap_enabled = true;
        b.on_quote(Quote {
            ts: ts_dnia(100, 12),
            bid: 4000.0,
            ask: 4000.20,
        });
        let t = kup_jedna(&mut b, Side::Buy);
        let saldo_przed = b.balance;
        // przekraczamy JEDNĄ północ; dzień 101 nie jest środą
        b.on_quote(Quote {
            ts: ts_dnia(101, 12),
            bid: 4000.0,
            ask: 4000.20,
        });
        let naliczone = b.balance - saldo_przed;
        assert!(
            (naliczone - (-0.7582)).abs() < 1e-9,
            "jedna noc BUY 0,01 lota ma kosztować −0,7582 $, a naliczono {naliczone}"
        );
        b.close_position(t, CloseReason::Manual).unwrap();
        let tr = b.history.last().unwrap();
        assert!(
            (tr.swap - (-0.7582)).abs() < 1e-9,
            "swap w transakcji: {}",
            tr.swap
        );
    }

    /// SELL może dostać swap dodatni. Kierunek ma znaczenie, ponieważ stawki
    /// long i short nie muszą być symetryczne.
    #[test]
    fn swap_pozycji_krotkiej_jest_dodatni() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.swap_enabled = true;
        b.on_quote(Quote {
            ts: ts_dnia(100, 12),
            bid: 4000.0,
            ask: 4000.20,
        });
        kup_jedna(&mut b, Side::Sell);
        let przed = b.balance;
        b.on_quote(Quote {
            ts: ts_dnia(101, 12),
            bid: 4000.0,
            ask: 4000.20,
        });
        assert!(
            (b.balance - przed - 0.2741).abs() < 1e-9,
            "{}",
            b.balance - przed
        );
    }

    #[test]
    fn potrojny_swap_wypada_przy_wejsciu_w_czwartek() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.swap_enabled = true;
        b.on_quote(Quote {
            ts: ts_dnia(6, 12),
            bid: 4000.0,
            ask: 4000.20,
        });
        kup_jedna(&mut b, Side::Buy);
        let przed = b.balance;
        // doba 6 to środa, doba 7 to czwartek
        b.on_quote(Quote {
            ts: ts_dnia(7, 12),
            bid: 4000.0,
            ask: 4000.20,
        });
        let n = b.balance - przed;
        assert!(
            (n - (-0.7582 * 3.0)).abs() < 1e-9,
            "wejście w czwartek ma być potrójne, a jest {n}"
        );
    }

    /// Kontrola w drugą stronę: wejście w ŚRODĘ jest pojedyncze.
    #[test]
    fn wejscie_w_srode_jest_pojedyncze() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.swap_enabled = true;
        b.on_quote(Quote {
            ts: ts_dnia(5, 12),
            bid: 4000.0,
            ask: 4000.20,
        });
        kup_jedna(&mut b, Side::Buy);
        let przed = b.balance;
        b.on_quote(Quote {
            ts: ts_dnia(6, 12),
            bid: 4000.0,
            ask: 4000.20,
        });
        let n = b.balance - przed;
        assert!((n - (-0.7582)).abs() < 1e-9, "środa pojedynczo, a jest {n}");
    }

    /// Luka w danych nie może zgubić nocy — broker naliczy każdą.
    #[test]
    fn przerwa_w_danych_nie_gubi_nocy() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.swap_enabled = true;
        b.on_quote(Quote {
            ts: ts_dnia(100, 12),
            bid: 4000.0,
            ask: 4000.20,
        });
        kup_jedna(&mut b, Side::Buy);
        let przed = b.balance;
        // skok o trzy doby: 101, 102, 103 — żadna nie jest środą
        b.on_quote(Quote {
            ts: ts_dnia(103, 12),
            bid: 4000.0,
            ask: 4000.20,
        });
        let n = b.balance - przed;
        assert!(
            (n - (-0.7582 * 3.0)).abs() < 1e-9,
            "trzy noce = trzy naliczenia, jest {n}"
        );
    }

    /// D1: TYDZIEŃ KOSZTUJE 7 JEDNOSTEK, NIE 9.
    ///
    /// Pozycja otwarta w poniedziałek i trzymana do następnego poniedziałku
    /// przekracza 7 północy: wt, śr, cz(×3), pt, sob, nd, pn. Potrójny czwartek
    /// pokrywa sobotę i niedzielę Z GÓRY, więc bez osi płacimy je drugi raz.
    #[test]
    fn d1_weekend_nie_jest_liczony_dwa_razy() {
        // doba 6 to środa, 7 czwartek → doba 4 to poniedziałek
        let pon = 4;
        assert_eq!(
            weekday_of(ts_dnia(pon, 0), 0),
            0,
            "kontrola: doba 4 to poniedziałek"
        );

        for (os, jednostek) in [(false, 9.0), (true, 7.0)] {
            let mut b = SimBroker::new(1000.0, 0.20, 0.0);
            b.swap_enabled = true;
            b.swap_pomijaj_weekend = os;
            b.on_quote(Quote {
                ts: ts_dnia(pon, 12),
                bid: 4000.0,
                ask: 4000.20,
            });
            kup_jedna(&mut b, Side::Buy);
            let przed = b.balance;
            b.on_quote(Quote {
                ts: ts_dnia(pon + 7, 12),
                bid: 4000.0,
                ask: 4000.20,
            });
            let n = b.balance - przed;
            assert!(
                (n - (-0.7582 * jednostek)).abs() < 1e-9,
                "oś swap_pomijaj_weekend = {os}: tydzień ma kosztować {jednostek} jednostek, a kosztuje {}",
                n / -0.7582
            );
        }
    }

    /// D1: sam WEEKEND bez potrójnego czwartku — pt→pn płaci 1 jednostkę
    /// (wejście w poniedziałek), a nie 3.
    #[test]
    fn d1_przejscie_przez_weekend_bez_czwartku() {
        let pt = 8; // doba 7 to czwartek, więc 8 to piątek
        assert_eq!(
            weekday_of(ts_dnia(pt, 0), 0),
            4,
            "kontrola: doba 8 to piątek"
        );
        for (os, jednostek) in [(false, 3.0), (true, 1.0)] {
            let mut b = SimBroker::new(1000.0, 0.20, 0.0);
            b.swap_enabled = true;
            b.swap_pomijaj_weekend = os;
            b.on_quote(Quote {
                ts: ts_dnia(pt, 12),
                bid: 4000.0,
                ask: 4000.20,
            });
            kup_jedna(&mut b, Side::Buy);
            let przed = b.balance;
            b.on_quote(Quote {
                ts: ts_dnia(pt + 3, 12),
                bid: 4000.0,
                ask: 4000.20,
            });
            let n = b.balance - przed;
            assert!(
                (n - (-0.7582 * jednostek)).abs() < 1e-9,
                "oś = {os}: pt→pn ma kosztować {jednostek}, a kosztuje {}",
                n / -0.7582
            );
        }
    }

    /// D1b: przeliczenie doby rolowania z enumu MT5 na naszą konwencję.
    ///
    /// Sonda brokera podaje 3 (środa w MT5) — u nas to czwartek, też 3.
    /// Test pilnuje, żeby ta równość była WYLICZONA, a nie zgadnięta: dla
    /// każdej innej wartości wynik ma być tożsamościowy modulo 7, a broker
    /// z potrójnym swapem w piątek (MT5 = 5) ma dawać naszą sobotę (5).
    #[test]
    fn d1b_doba_rolowania_z_enumu_mt5() {
        assert_eq!(
            doba_rolowania_z_mt5(3),
            3,
            "MT5 środa → nasz czwartek; obie zmiany konwencji się znoszą"
        );
        for d in 0..7u32 {
            assert_eq!(
                doba_rolowania_z_mt5(d),
                d,
                "przeliczenie jest tożsamością modulo 7"
            );
        }
        // ta sama liczba, ale INNE ZNACZENIE po obu stronach — kontrola opisu
        assert_eq!(doba_rolowania_z_mt5(5), 5, "MT5 piątek → nasza sobota");
    }

    /// D1b: oś włączona przy wartości z sondy NIE ZMIENIA ani jednej liczby.
    #[test]
    fn d1b_os_z_serwera_zachowuje_parytet() {
        let mut s = Settings::default();
        s.swap_enabled = true;
        let bez = {
            let mut b = SimBroker::z_ustawien(1000.0, &s);
            b.on_quote(Quote {
                ts: ts_dnia(6, 12),
                bid: 4000.0,
                ask: 4000.20,
            });
            kup_jedna(&mut b, Side::Buy);
            let przed = b.balance;
            b.on_quote(Quote {
                ts: ts_dnia(7, 12),
                bid: 4000.0,
                ask: 4000.20,
            });
            b.balance - przed
        };
        s.swap_rollover_z_serwera = true;
        s.swap_rollover3days_mt5 = 3;
        let z_serwera = {
            let mut b = SimBroker::z_ustawien(1000.0, &s);
            b.on_quote(Quote {
                ts: ts_dnia(6, 12),
                bid: 4000.0,
                ask: 4000.20,
            });
            kup_jedna(&mut b, Side::Buy);
            let przed = b.balance;
            b.on_quote(Quote {
                ts: ts_dnia(7, 12),
                bid: 4000.0,
                ask: 4000.20,
            });
            b.balance - przed
        };
        assert!(
            (bez - z_serwera).abs() < 1e-12,
            "oś D1b przy wartości z sondy musi być no-opem: {bez} vs {z_serwera}"
        );
        assert!(
            (bez - (-0.7582 * 3.0)).abs() < 1e-9,
            "kontrola: to ma być potrójny czwartek"
        );
    }

    /// D4: `mark` nalicza swap i przestawia cenę, ale NIE egzekwuje.
    ///
    /// To jest cały kontrakt tej metody: granica doby ma zapłacić za noc,
    /// a nie wykonać handlu przed komunikatami nocnymi.
    #[test]
    fn d4_mark_placi_za_noc_ale_nie_egzekwuje() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.swap_enabled = true;
        b.on_quote(Quote {
            ts: ts_dnia(100, 12),
            bid: 4000.0,
            ask: 4000.20,
        });
        // limit kupna PONIŻEJ rynku + pozycja ze stopem poniżej
        b.place_pending(PendingReq {
            kind: PendingKind::BuyLimit,
            volume: 0.01,
            price: 3990.0,
            sl: None,
            tp: None,
            basket: None,
            level: 0,
            is_toucher: false,
            is_topup: false,
            comment: String::new(),
        })
        .unwrap();
        b.open_market(OrderReq {
            side: Side::Buy,
            volume: 0.01,
            sl: Some(3995.0),
            tp: None,
            basket: None,
            level: 0,
            is_toucher: false,
            comment: String::new(),
        })
        .unwrap();
        let saldo = b.balance;
        let ml_przed = (b.ml_pod_200, b.ml_pod_150, b.ml_pod_100);

        // kurs nowej doby PO LUCE w dół: bez `mark` wypełniłby limit i zabił
        // pozycję stopem
        b.mark(Quote {
            ts: ts_dnia(101, 0),
            bid: 3985.0,
            ask: 3985.20,
        });

        assert_eq!(b.pendings().len(), 1, "`mark` nie ma prawa wypełnić limitu");
        assert_eq!(
            b.positions().len(),
            1,
            "`mark` nie ma prawa zamknąć pozycji na stopie"
        );
        assert_eq!(b.history.len(), 0, "`mark` nie dopisuje transakcji");
        assert_eq!(
            (b.ml_pod_200, b.ml_pod_150, b.ml_pod_100),
            ml_przed,
            "`mark` nie rusza liczników poziomu marginesu"
        );
        assert!(
            (b.balance - saldo - (-0.7582)).abs() < 1e-9,
            "noc ma być zapłacona"
        );
        assert_eq!(b.q.bid, 3985.0, "`mark` przestawia bieżące kwotowanie");

        // ten sam tick przez `on_quote` egzekwuje już normalnie
        b.on_quote(Quote {
            ts: ts_dnia(101, 0),
            bid: 3985.0,
            ask: 3985.20,
        });
        assert!(
            b.pendings().is_empty(),
            "kontrola: `on_quote` wypełnia limit"
        );
        assert!(
            !b.history.is_empty(),
            "kontrola: `on_quote` egzekwuje stopa"
        );
    }

    #[test]
    fn bez_wlacznika_swap_nie_jest_naliczany() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        assert!(!b.swap_enabled, "sam SimBroker startuje bez swapu");
        b.on_quote(Quote {
            ts: ts_dnia(100, 12),
            bid: 4000.0,
            ask: 4000.20,
        });
        kup_jedna(&mut b, Side::Buy);
        let przed = b.balance;
        b.on_quote(Quote {
            ts: ts_dnia(103, 12),
            bid: 4000.0,
            ask: 4000.20,
        });
        assert_eq!(b.balance, przed);
    }

    /// REGRESJA: limit NIE MOŻE zrealizować się po cenie lepszej niż własny
    /// poziom. Było `min(o.price, mkt)`, co przy luce dawało darmowy zysk —
    /// ta sama klasa błędu co „stop rozliczany po poziomie", tylko na wejściu.
    #[test]
    fn limit_nie_realizuje_sie_lepiej_niz_wlasny_poziom() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.on_quote(q(4010.0, 0.20));
        b.place_pending(PendingReq {
            kind: PendingKind::BuyLimit,
            volume: 0.01,
            price: 4000.0,
            sl: None,
            tp: None,
            basket: None,
            level: 0,
            is_toucher: false,
            comment: String::new(),
            is_topup: false,
        })
        .unwrap();
        // luka: rynek przeskakuje z 4010 na 3990, czyli daleko pod limit
        b.on_quote(q(3990.0, 0.20));
        let p = &b.positions()[0];
        assert!(
            (p.open_price - 4000.0).abs() < 1e-9,
            "limit ma wejść po SWOIM poziomie 4000, a wszedł po {}",
            p.open_price
        );
    }

    #[test]
    fn poslizg_zlecenia_oczekujacego_pogarsza_cene_wejscia() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.slippage_pending = 0.092;
        b.on_quote(q(4010.0, 0.20));
        b.place_pending(PendingReq {
            kind: PendingKind::BuyLimit,
            volume: 0.01,
            price: 4000.0,
            sl: None,
            tp: None,
            basket: None,
            level: 0,
            is_toucher: false,
            comment: String::new(),
            is_topup: false,
        })
        .unwrap();
        b.on_quote(q(3999.80, 0.20));
        let p = &b.positions()[0];
        assert!(
            (p.open_price - 4000.092).abs() < 1e-9,
            "kupno ma wejść DROŻEJ o poślizg: {}",
            p.open_price
        );
    }

    /// STOP OUT: broker zamyka NAJBARDZIEJ STRATNĄ pozycję pojedynczo
    /// i przelicza poziom marginu — nie likwiduje wszystkiego naraz.
    #[test]
    fn stop_out_zamyka_pojedynczo_a_nie_wszystko_naraz() {
        let mut b = SimBroker::new(100.0, 0.20, 0.0);
        b.leverage = 500;
        b.on_quote(q(4000.0, 0.20));
        for _ in 0..4 {
            kup_jedna(&mut b, Side::Buy);
        }
        assert_eq!(b.positions().len(), 4);
        // cena spada tak, żeby poziom marginu zszedł pod 20 %
        b.on_quote(q(3960.0, 0.20));
        assert!(b.stop_outs > 0, "stop out musi zadziałać");
        assert!(
            b.positions().len() < 4,
            "broker ma zamknąć CZĘŚĆ pozycji, a zostało {}",
            b.positions().len()
        );
    }

    /// REGRESJA S-1: wypełnienie zlecenia oczekującego BEZ wolnego depozytu
    /// musi skasować zlecenie, a nie otworzyć pozycję na kredyt.
    ///
    /// Konto 15 $, dwa BUY LIMIT po 0,01 lota na 4000, dźwignia 500 →
    /// margines jednej pozycji to 4000 × 100 × 0,01 / 500 = 8 $. Pierwsza
    /// pozycja się mieści, druga już nie. Realny MT5 zwraca wtedy „No money".
    #[test]
    fn fill_bez_marginesu_kasuje_zlecenie() {
        let mut b = SimBroker::new(15.0, 0.20, 0.0);
        b.leverage = 500;
        b.on_quote(q(4010.0, 0.20));
        for _ in 0..2 {
            b.place_pending(PendingReq {
                kind: PendingKind::BuyLimit,
                volume: 0.01,
                price: 4000.0,
                sl: None,
                tp: None,
                basket: None,
                level: 0,
                is_toucher: false,
                comment: String::new(),
                is_topup: false,
            })
            .unwrap();
        }
        assert_eq!(b.pendings().len(), 2);

        b.on_quote(q(3999.80, 0.20));
        assert_eq!(
            b.positions().len(),
            1,
            "drugie zlecenie ma zostać skasowane z braku depozytu"
        );
        assert_eq!(b.rejected_no_money, 1);
        assert!(
            b.pendings().is_empty(),
            "skasowane zlecenie nie wraca do kolejki"
        );
    }

    #[test]
    fn bez_kontroli_marginesu_symulator_otwiera_na_kredyt() {
        let mut b = SimBroker::new(15.0, 0.20, 0.0);
        b.leverage = 500;
        b.margin_check_on_fill = false;
        b.on_quote(q(4010.0, 0.20));
        for _ in 0..2 {
            b.place_pending(PendingReq {
                kind: PendingKind::BuyLimit,
                volume: 0.01,
                price: 4000.0,
                sl: None,
                tp: None,
                basket: None,
                level: 0,
                is_toucher: false,
                comment: String::new(),
                is_topup: false,
            })
            .unwrap();
        }
        b.on_quote(q(3999.80, 0.20));
        assert_eq!(b.positions().len(), 2, "stary błąd: obie pozycje wchodzą");
        assert_eq!(b.rejected_no_money, 0);
    }

    #[test]
    fn partial_zmniejsza_wolumen() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.on_quote(q(4000.0, 0.20));
        let t = b
            .open_market(OrderReq {
                side: Side::Buy,
                volume: 0.05,
                sl: None,
                tp: None,
                basket: None,
                level: 0,
                is_toucher: false,
                comment: String::new(),
            })
            .unwrap();
        b.close_partial(t, 0.02, CloseReason::Partial).unwrap();
        assert!((b.find_position(t).unwrap().volume - 0.03).abs() < 1e-9);
    }

    #[test]
    fn partial_respektuje_siatke_brokera_i_nie_zostawia_ogarka() {
        let mut b = SimBroker::new(1000.0, 0.20, 0.0);
        b.volume_min = 0.10;
        b.volume_step = 0.10;
        b.on_quote(q(4000.0, 0.20));
        let t = b
            .open_market(OrderReq {
                side: Side::Buy,
                volume: 0.20,
                sl: None,
                tp: None,
                basket: None,
                level: 0,
                is_toucher: false,
                comment: String::new(),
            })
            .unwrap();

        // 0.15 zaokragla sie do 0.20, ale to zamkneloby calosc; broker ma
        // zamiast tego odciac maksymalne 0.10 i zostawic handlowalne 0.10.
        b.close_partial(t, 0.15, CloseReason::Partial).unwrap();
        assert!((b.find_position(t).unwrap().volume - 0.10).abs() < 1e-12);

        // Kolejnego partiala nie da sie wykonac bez resztki < minimum, wiec
        // kontrakt live/backtest domyka ticket zamiast tworzyc ogarek.
        b.close_partial(t, 0.05, CloseReason::Partial).unwrap();
        assert!(b.find_position(t).is_none());
    }

    // ============ PAKIET C3: DŹWIGNIA RACHUNKU Z USTAWIEŃ ============

    /// Broker musi liczyć margines dźwignią z `Settings`, a nie zaszytą 1:500.
    ///
    /// Dotąd `konto_dzwignia` widziały wyłącznie bramki silnika, więc odpowiedź
    /// na „co, gdyby broker ściął dźwignię" była liczona przez pół systemu.
    /// Margines 1,0 lota po 4000 $: przy 1:500 → 800 $, przy 1:1000 → 400 $,
    /// przy 1:100 → 4000 $.
    #[test]
    fn ustaw_przepisuje_dzwignie_z_ustawien() {
        for (dzw, oczekiwany) in [(1000.0, 400.0), (100.0, 4000.0)] {
            let mut c = Settings::default();
            c.konto_dzwignia = dzw;
            let mut b = SimBroker::new(100_000.0, 0.20, 0.0);
            b.ustaw(&c);
            assert_eq!(b.leverage, dzw as u32);
            b.on_quote(q(4000.0, 0.0));
            b.open_market(OrderReq {
                side: Side::Buy,
                volume: 1.0,
                sl: None,
                tp: None,
                basket: None,
                level: 0,
                is_toucher: false,
                comment: String::new(),
            })
            .unwrap();
            assert!(
                (b.used_margin() - oczekiwany).abs() < 1e-6,
                "dźwignia 1:{dzw}: margines {} zamiast {oczekiwany}",
                b.used_margin()
            );
        }
    }

    /// PARYTET: zero znaczy „dźwignia z rachunku" — zostaje 1:500 z `new`,
    /// czyli wszystkie dotychczasowe przebiegi liczą się co do centa tak samo.
    /// Ta gałąź jest warunkiem, pod którym wolno było pole w ogóle podpiąć.
    #[test]
    fn zerowa_dzwignia_zostawia_domyslne_500() {
        let mut b = SimBroker::new(100_000.0, 0.20, 0.0);
        let przed = b.leverage;
        let c = Settings::default();
        assert_eq!(c.konto_dzwignia, 0.0, "domyślną musi być zero");
        b.ustaw(&c);
        assert_eq!(b.leverage, przed);
        assert_eq!(b.leverage, 500);
    }

    #[test]
    fn credit_balance_separate_sim_equity_margin_and_live_account_contract() {
        let c=Settings {credit_balance_separate:true,odlicz_kredyt:true,kredyt_reczny:300.0,..Settings::default()};
        let b=SimBroker::z_ustawien(159.8,&c);let a=b.account();
        assert_eq!(a.balance,159.8);assert_eq!(a.credit,300.0);assert!((a.equity-459.8).abs()<1e-9);
        assert_eq!(a.free_margin,a.equity);assert_eq!(c.podstawa_lota_z_konta(a.balance,a.equity,a.credit),159.8);
        assert_eq!(b.min_equity,a.equity);
        let state=conduit_core::ea::WektorStanu::zbierz(&b,&c,0,1);
        assert!(state.floating.abs()<1e-9,"bonus is not floating trading profit");
        assert!((state.wlasne-159.8).abs()<1e-9);
    }
    #[test]
    fn credit_balance_separate_sim_credit_exists_when_sizing_deduction_off() {
        let c=Settings {credit_balance_separate:true,odlicz_kredyt:false,kredyt_reczny:300.0,..Settings::default()};
        let mut b=SimBroker::z_ustawien(600.0,&c);assert_eq!(b.credit,300.0);assert_eq!(b.equity(),900.0);
        b.credit=0.0;assert_eq!(b.equity(),600.0);assert_eq!(b.balance,600.0);
    }
    #[test]
    fn credit_balance_separate_live_refresh_floating_and_manual_override() {
        use conduit_core::{engine::Engine, settings::PodstawaLota};
        let mut c=Settings {credit_balance_separate:true,odlicz_kredyt:true,kredyt_reczny:0.0,commission_per_lot:0.0,lot_mode_percent:true,lot_percent:0.5,lot_max:0.0,..Settings::default()};
        let mut b=SimBroker::z_ustawien(600.0,&c);b.credit=300.0;
        b.mark(q(4000.0,0.2));
        b.open_market(OrderReq {side:Side::Buy,volume:0.1,sl:None,tp:None,basket:None,level:0,is_toucher:false,comment:"credit-contract-test".into()}).unwrap();
        let quote=q(4002.0,0.2);b.mark(quote);
        let pnl=b.positions()[0].profit_usd(&quote);
        let mut e=Engine::new(c.clone(),600.0);e.on_tick(&mut b,&quote);
        assert_eq!(e.stats.credit,300.0);assert!((e.stats.equity-900.0-pnl).abs()<1e-9);
        assert_eq!(e.podstawa_lota(),600.0);assert_eq!(e.lot_size(e.podstawa_lota()),0.03);
        e.cfg.credit_balance_separate=false;
        assert_eq!(e.podstawa_lota(),300.0);assert_eq!(e.lot_size(e.podstawa_lota()),0.02);
        e.cfg.credit_balance_separate=true;e.cfg.lot_base=PodstawaLota::Equity;
        assert!((e.podstawa_lota()-600.0-pnl).abs()<1e-9);
        c.kredyt_reczny=250.0;
        let state=conduit_core::ea::WektorStanu::zbierz(&b,&c,0,quote.ts);
        assert!((state.floating-pnl).abs()<1e-9);
        assert!((state.wlasne-650.0-pnl).abs()<1e-9);
        // Bonus removal changes equity, not balance or true floating PnL;
        // AUTO deduction follows the broker's new C without a stale override.
        b.credit=0.0;e.on_tick(&mut b,&quote);
        assert_eq!(e.stats.balance,600.0);
        assert!((e.podstawa_lota()-600.0-pnl).abs()<1e-9);
    }
    #[test]
    fn credit_balance_separate_sim_off_and_zero_preserve_legacy() {
        let mut c=Settings {odlicz_kredyt:true,kredyt_reczny:300.0,..Settings::default()};
        let legacy=SimBroker::z_ustawien(600.0,&c);assert_eq!(legacy.equity(),600.0);assert_eq!(legacy.credit,300.0);
        c.odlicz_kredyt=false;let off=SimBroker::z_ustawien(600.0,&c);assert_eq!(off.credit,0.0);
        c.kredyt_reczny=0.0;let a=SimBroker::z_ustawien(600.0,&c);
        c.credit_balance_separate=true;let b=SimBroker::z_ustawien(600.0,&c);
        assert_eq!(a.equity().to_bits(),b.equity().to_bits());assert_eq!(a.account().free_margin.to_bits(),b.account().free_margin.to_bits());
    }
}
