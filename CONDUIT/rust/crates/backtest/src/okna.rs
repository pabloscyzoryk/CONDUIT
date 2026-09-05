
use crate::data::{ReplayMessage, TickData};
use crate::runner::FormatCfg;
use crate::sim::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::{Engine, IncomingMessage};
use conduit_core::formaty::{Lancuch, PulapyGlobalne};
use conduit_core::routing::Silniki;
use conduit_core::settings::Settings;
use conduit_core::types::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct KonfOkien {
    pub from: Ts,
    pub to: Ts,
    pub start_balance: f64,
    /// Ten sam jawny model wykonania LIMIT co w zwykłym runnerze.
    pub sim_limit_price_improvement: bool,
    /// Ten sam jawny profil pending/SL co w zwykłym runnerze; default OFF.
    pub sim_new_pending_sl_next_tick: bool,
    /// Same explicit account-money/swap settlement model as RunConfig.
    /// None is the immutable legacy path; Some(0..=8) enables native settlement.
    pub sim_native_swap_cash_digits: Option<u32>,
    pub settings: Settings,
    /// długość okna w DNIACH HANDLOWYCH (dniach, w których są ticki).
    /// 1 = dokładnie dzisiejszy `--daily-reset`.
    pub n_dni: u32,
    /// Z ZACHOWANIEM NOCNYM: po granicy okna nie bierzemy nowych sygnałów,
    /// ale pozwalamy koszykom dojść do naturalnego końca.
    pub zzn: bool,
    pub zzn_max_dni: u32,
    pub source_name: String,
    /// Nogi przebiegu WIELOFORMATOWEGO (`--preset-format`). Puste = jeden
    /// silnik, slot 0, zerowe obce obciążenie — czyli ścieżka parytetu.
    ///
    /// Każde okno dostaje WŁASNY komplet silników, tak samo jak dostaje własny
    /// komplet brokera: okno jest osobnym rachunkiem od `start_balance`.
    pub formaty: Vec<FormatCfg>,
    /// Pułapy ponad presetami nóg. Działają także przy jednej nodze — inaczej
    /// `--pulapy` z jednym `--preset-format` byłoby cicho ignorowane. Domyślne
    /// zera niczego nie zmieniają (`sufit_u32(0, p) == p`), więc parytet stoi.
    pub pulapy: PulapyGlobalne,
}

impl Default for KonfOkien {
    fn default() -> Self {
        KonfOkien {
            from: 0,
            to: i64::MAX,
            start_balance: 200.0,
            sim_limit_price_improvement: false,
            sim_new_pending_sl_next_tick: false,
            sim_native_swap_cash_digits: None,
            settings: Settings::default(),
            n_dni: 1,
            zzn: false,
            zzn_max_dni: 7,
            source_name: "ATFX VIP SIGNALS".into(),
            formaty: Vec::new(),
            pulapy: PulapyGlobalne::default(),
        }
    }
}

/// Wynik jednego okna.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WynikOkna {
    pub nr: u32,
    /// pierwszy i ostatni dzień HANDLOWY okna
    pub od: String,
    pub do_dnia: String,
    /// zysk okna w $ (equity na końcu okna minus kapitał startowy)
    pub zysk: f64,
    /// najniższe equity, jakie w tym oknie wystąpiło
    pub min_equity: f64,
    /// najgłębsze obsunięcie od szczytu wewnątrz okna
    pub max_dd: f64,
    /// ON with positive constant broker credit: result/min/DD use own E-C.
    /// Absent for legacy OFF and C=0; historical JSON shape remains unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reporting_equity_basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_credit: Option<f64>,
    /// Raw broker values distinguish boundary valuation from final settlement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_broker_boundary_equity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_broker_end_equity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_broker_min_equity: Option<f64>,
    pub trejdy: u32,
    pub koszyki: u32,

    pub pozycje_na_granicy: u32,
    /// ich łączny niezrealizowany wynik w tej chwili ($)
    pub floating_na_granicy: f64,
    /// zlecenia oczekujące skasowane przez granicę
    pub pendingi_na_granicy: u32,
    /// koszyki, które na granicy jeszcze żyły
    pub koszyki_zywe_na_granicy: u32,
    /// zysk okna liczony DO GRANICY (bez ogona) — przy ZZN pozwala rozdzielić
    /// „co okno zarobiło" od „co dołożył ogon"
    pub zysk_do_granicy: f64,
    /// ile milisekund trwał ogon ZZN (0 bez ZZN albo gdy nie było czego dokańczać)
    pub ogon_ms: i64,
    /// czy ogon został UCIĘTY sufitem `zzn_max_dni` albo końcem danych
    pub ogon_uciety: bool,
}

impl WynikOkna {
    /// Ile ogon zmienił wynik okna. Bez ZZN zawsze 0.
    pub fn wklad_ogona(&self) -> f64 {
        self.zysk - self.zysk_do_granicy
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WynikOkien {
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub continuation_reconciliation_required: Option<String>,
    #[serde(default, skip_serializing_if="Option::is_none")]
    pub sim_execution_reconciliation_required: Option<String>,
    pub n_dni: u32,
    pub zzn: bool,
    pub okna: Vec<WynikOkna>,

    // ---------- rozkład ----------
    pub suma: f64,
    pub srednia: f64,
    pub mediana: f64,
    pub pct_dodatnich: f64,

    // ---------- rozkład po oknach, W KTÓRYCH BYŁ HANDEL ----------
    //
    // `Metrics::win_days_pct` i `Metrics::median_day` liczą się WYŁĄCZNIE po
    // dniach z transakcjami (`metrics.rs`: `daily.iter().filter(|d| d.trades > 0)`).
    // Bez tych trzech pól nie da się poprawnie porównać wyniku okien z bazową
    // metodą: udział dodatnich okien i dodatnich dni ma inny mianownik.
    // Podajemy obie wartości jawnie.
    pub okien_z_handlem: u32,
    pub mediana_z_handlem: f64,
    pub pct_dodatnich_z_handlem: f64,
    pub najgorsze_okno: f64,
    pub najlepsze_okno: f64,
    /// minimalne equity W NAJGORSZYM oknie
    pub min_equity_najgorszego: f64,
    /// najniższe equity, jakie wystąpiło w JAKIMKOLWIEK oknie
    pub min_equity_globalne: f64,

    // ---------- skala ucięcia na granicy ----------
    pub ucietych_pozycji: u64,
    pub uciety_floating: f64,
    pub ucietych_pendingow: u64,
    pub ucietych_koszykow: u64,
    /// ile okien miało cokolwiek żywego na granicy
    pub okien_z_ucieciem: u32,

    // ---------- ZZN ----------
    pub okien_z_ucietym_ogonem: u32,
    pub najdluzszy_ogon_min: i64,
    pub sredni_ogon_min: f64,

    pub dni_handlowych: u32,
    pub tickow: u64,
    pub czas_ms: u64,
    /// Wiadomości z `ts + lat` PO OSTATNIM TICKU danych — pętla nigdy do nich
    /// nie doszła, więc żaden tor ich nie wydał; dotąd nigdzie nie liczone
    /// (audyt: okna.rs:389). `serde(default)` obowiązkowe — stare pliki
    /// wyników nie mają tego pola.
    #[serde(default)]
    pub po_ostatnim_ticku: u64,
    /// Ostrzeżenie merytoryczne dla wywołującego (pusty = brak zastrzeżeń).
    pub zastrzezenia: Vec<String>,
}

/// Jeden tor: jedno okno w trakcie liczenia.
struct Tor {
    nr: u32,
    dzien_start: i64,
    /// ostatni dzień HANDLOWY okna (włącznie)
    dzien_koniec: i64,
    /// Komplet silników TEGO okna. Przy przebiegu jednoformatowym ma dokładnie
    /// jeden element ze slotem 0 — patrz `zbuduj_zespol_okna`.
    zespol: Silniki,
    /// `true` = jeden silnik bez routingu, ścieżka parytetu (bez widoku brokera)
    pojedynczy: bool,
    broker: SimBroker,
    /// Fixed credit supplied at construction, as in the standard runner.
    /// This is reporting normalization, independent of lot-basis deduction.
    reporting_credit: f64,
    /// kursor po wspólnej liście wiadomości
    mi: usize,
    /// czy przekroczyliśmy już granicę okna (tryb ZZN — trwa ogon)
    ogon: bool,
    granica_ts: Ts,
    /// equity dokładnie na granicy okna
    equity_na_granicy: f64,
    poz_na_granicy: u32,
    float_na_granicy: f64,
    pend_na_granicy: u32,
    kosz_na_granicy: u32,
    peak_equity: f64,
    max_dd: f64,
}

impl Tor {
    /// Jeden pełny krok rynku. Miejsce wywołania wobec wiadomości zależy
    /// od tej samej osi `live_tick_order_strict`, którą czyta runner.
    fn rozegraj_tick(&mut self, q: Quote, dispatch_utc: Ts, row: usize) {
        self.broker.on_tape_quote(q, row);
        if self.pojedynczy {
            self.zespol.lista[0].engine.on_tick_received(&mut self.broker, &q, dispatch_utc);
        } else {
            self.zespol.przelicz_obce(&self.broker, None);
            let br = &mut self.broker;
            self.zespol.kazdy(br, |e, w| e.on_tick_received(w, &q, dispatch_utc));
        }
    }

    fn zywy_rynek(&self) -> bool {
        !self.broker.positions().is_empty()
            || !self.broker.pendings().is_empty()
            || self
                .zespol
                .lista
                .iter()
                .any(|si| si.engine.baskets.iter().any(|b| b.alive()))
    }

    /// Wszystkie koszyki wszystkich nóg — do liczby w raporcie okna.
    fn koszykow(&self) -> usize {
        self.zespol
            .lista
            .iter()
            .map(|si| si.engine.baskets.len())
            .sum()
    }

    fn zywych_koszykow(&self) -> usize {
        self.zespol
            .lista
            .iter()
            .map(|si| si.engine.baskets.iter().filter(|b| b.alive()).count())
            .sum()
    }
}

fn zbuduj_zespol_okna(cfg: &KonfOkien) -> (Silniki, bool) {
    if cfg.formaty.is_empty() {
        let mut e = Engine::new(cfg.settings.clone(), cfg.start_balance);
        e.set_run_id("okno");
        e.journal.cfg.enabled = false;
        e.pulapy = cfg.pulapy.clone();
        let l = Lancuch {
            nazwa: "OKNA".into(),
            pulapy: cfg.pulapy.clone(),
            ..Default::default()
        };
        return (
            Silniki::pojedynczy(e, String::new(), String::new(), l, true),
            true,
        );
    }
    let mut l = Lancuch {
        nazwa: "OKNA".into(),
        pulapy: cfg.pulapy.clone(),
        ..Default::default()
    };
    let mut presety: BTreeMap<String, Settings> = BTreeMap::new();
    for f in cfg.formaty.iter() {
        // Klucz to NAZWA FORMATU, nie nazwa pliku — dwie nogi wolno puścić na
        // tym samym presecie, a mapa po pliku skleiłaby je w jeden wpis.
        l.presety.insert(f.format.clone(), f.format.clone());
        presety.insert(f.format.clone(), f.settings.clone());
    }
    let (mut z, braki) = Silniki::zbuduj(&l, &presety, &cfg.settings, cfg.start_balance);
    for b in braki {
        // Nieosiągalne (mapę budujemy wiersz w wiersz), ale cisza jest zakazana.
        eprintln!("routing okien: {}", b.opis());
    }
    for si in z.lista.iter_mut() {
        si.engine.set_run_id("okno");
        si.engine.journal.cfg.enabled = false;
        if let Some(f) = cfg.formaty.iter().find(|f| f.format == si.format) {
            si.preset = f.preset.clone();
        }
    }
    (z, false)
}

/// Silnik-kronikarz: JEDYNE źródło historii rynku dla wszystkich okien.
///
/// Nie dostaje ani jednej wiadomości, więc nigdy nie handluje — a `price_hist`
/// i `vol_hist` są czystą funkcją strumienia kwotowań, nie stanu konta. Dzięki
/// temu okno zaczynające się w środę dostaje dokładnie tę wiedzę o rynku, którą
/// ciągły przebieg miałby w środę rano. To nie jest wygoda, tylko warunek
/// porównywalności z bazą: `--daily-reset` przenosi te bufory przez reset od
/// czasu, gdy okazało się, że bez tego filtr reżimu jest martwy.
struct Kronikarz {
    engine: Engine,
    broker: SimBroker,
}

pub fn uruchom(ticks: &TickData, messages: &[ReplayMessage], cfg: &KonfOkien) -> WynikOkien {
    if cfg.sim_native_swap_cash_digits.is_some_and(|digits| digits > 8) {
        let mut result = pusty(cfg, 0, 0);
        result.sim_execution_reconciliation_required =
            Some("native swap cash digits must be in 0..=8".into());
        return result;
    }
    if cfg.settings.restore_strategy_continuation {
        let mut result=pusty(cfg,0,0);
        result.continuation_reconciliation_required=Some("continuation A: the separate windows/restart pipeline has no Fresh/continuation proof".into());
        return result;
    }
    let t0 = std::time::Instant::now();
    let n = cfg.n_dni.max(1) as usize;
    let tz = cfg.settings.session_offset();

    let i0 = ticks.index_at(cfg.from);
    let i_handel = ticks.index_at(cfg.to).min(ticks.len());
    if i_handel <= i0 {
        return pusty(cfg, 0, 0);
    }

    // ---------- DNI HANDLOWE ----------
    // Tylko doby, w których naprawdę są ticki. Weekend nie jest dniem okna:
    // „okno 3-dniowe" ma znaczyć trzy dni HANDLU, tak samo jak `--daily-reset`
    // liczy wyłącznie dni, które trafiły do `daily`.
    let dni = dni_handlowe(ticks, i0, i_handel, tz);
    if dni.is_empty() {
        return pusty(cfg, 0, 0);
    }
    let ostatni_dzien = *dni.last().unwrap();

    // Okna, które MIESZCZĄ SIĘ w całości. Okno urwane przez koniec zakresu nie
    // jest oknem N-dniowym i nie wolno go wrzucać do tego samego rozkładu.
    let ile_okien = dni.len().saturating_sub(n - 1);
    if ile_okien == 0 {
        let mut w = pusty(cfg, dni.len() as u32, 0);
        w.zastrzezenia.push(format!(
            "zakres ma {} dni handlowych, a okno ma {n} — nie powstało ani jedno pełne okno",
            dni.len()
        ));
        return w;
    }

    // Dzień startu okna k to `dni[k]`, a jego ostatni dzień to `dni[k + n - 1]`.
    // Mapa „dzień → numer okna" pozwala w pętli po tickach odpowiedzieć w O(1),
    // czy dziś zaczyna się nowe okno.
    let mut start_w_dniu: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    for k in 0..ile_okien {
        start_w_dniu.insert(dni[k], k);
    }

    // ---------- OGON ZZN ----------
    // Ogon wolno czytać PONAD `--to`, bo to nadal są dane, które mamy, a pytanie
    // brzmi „jak koszyk skończył NAPRAWDĘ". Bez tego ostatnie okna miałyby ogon
    // ucięty samą ramką raportu — czyli dokładnie tę wadę, którą ZZN usuwa.
    let i_koniec = if cfg.zzn {
        let sufit = cfg
            .to
            .saturating_add(cfg.zzn_max_dni.max(1) as i64 * 86_400_000);
        ticks.index_at(sufit).min(ticks.len()).max(i_handel)
    } else {
        i_handel
    };
    // Every trading path below passes the immutable source-row index. B15 can
    // therefore cross days and ZZN tails without re-executing a boundary row.

    // ---------- wiadomości ----------
    // Ogon ZZN też słucha kanału (komunikaty do SWOICH koszyków), więc okno
    // wiadomości sięga tak daleko jak okno ticków.
    let lat = cfg.settings.msg_offset() + cfg.settings.exec_latency_ms;
    let t_kon = ticks.ts(i_koniec.saturating_sub(1)) + 1;
    let mut msgs: Vec<&ReplayMessage> = messages
        .iter()
        .filter(|m| m.ts + lat >= cfg.from && m.ts + lat < t_kon.max(cfg.to))
        .collect();
    msgs.sort_by_key(|m| m.ts);

    let source = SourceKey::new(-1_000_000_000_301, None);

    // ---------- kronikarz ----------
    let mut kron = Kronikarz {
        engine: {
            let mut e = Engine::new(cfg.settings.clone(), cfg.start_balance);
            e.journal.cfg.enabled = false;
            e.set_run_id("kron");
            e
        },
        broker: SimBroker::new(cfg.start_balance, cfg.settings.stops_level, 0.0),
    };
    kron.broker.price_digits = ticks.price_digits();

    // Wiadomości, których kanał nie trafił na żadną nogę. Liczymy je i
    // raportujemy w zastrzeżeniach — cichy brak trasy znaczyłby, że okno
    // mierzy strategię, do której połowa sygnałów nie doszła.
    let mut bez_trasy: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    // Ta sama wiadomość idzie przez KAŻDY żywy tor (dnia 5. żyją okna zaczęte
    // 3., 4. i 5.), więc licznik bez dedupe liczył ją do n razy — przy oknie
    // 3-dniowym potrajał stratę (audyt: okna.rs:565). Klucz (kanał, msg_id),
    // nie samo msg_id: identyfikatory Telegrama są PER KANAŁ (loader dedupuje
    // po tej samej parze, data.rs), więc dwa kanały bez trasy potrafią nieść
    // ten sam numer i zbiór po samym numerze zliczałby dwie wiadomości jako
    // jedną. Jedna wiadomość = jedna sztuka, niezależnie od liczby torów.
    let mut bez_trasy_policzone: std::collections::HashSet<(String, i64)> =
        std::collections::HashSet::new();
    let mut tory: Vec<Tor> = Vec::with_capacity(n + 2);
    let mut wyniki: Vec<WynikOkna> = Vec::with_capacity(ile_okien);
    // regime_hist wędruje łańcuchem po oknach — patrz nagłówek modułu.
    // PO NOGACH: każdy silnik ma własną historię decyzji filtra reżimu, bo
    // każda noga ma własny preset (a więc własne `regime_ma_hours`). Jeden
    // wspólny wektor przenosiłby ustalenia jednej nogi do drugiej.
    // Indeks = pozycja w `zespol.lista`, a ta jest deterministyczna
    // (`sloty_formatow` sortuje po nazwie).
    let mut regime_chain: Vec<Vec<(Ts, bool)>> = Vec::new();
    let mut cur_day = i64::MIN;
    // Znacznik POPRZEDNIEGO ticka. Potrzebny wyłącznie po to, żeby nowe okno
    // dostało dokładnie te wiadomości, które w przebiegu ciągłym dostałby nowy
    // silnik: te, których czas jeszcze nie nadszedł na poprzednim ticku.
    // Odcięcie po `q.ts` gubiłoby wiadomość przypadającą CO DO MILISEKUNDY
    // na tick graniczny — a to jest pierwsza wiadomość nowego dnia.
    let mut prev_ts: Ts = i64::MIN;
    let causal_tick_before_messages = cfg.settings.live_tick_order_strict;
    let check_cost_faults = cfg.sim_native_swap_cash_digits.is_some()
        || cfg.settings.closed_profit_net_costs
        || cfg.formaty.iter().any(|f| f.settings.closed_profit_net_costs);

    for i in i0..i_koniec {
        let q = ticks.quote(i);
        let day = day_of(q.ts, tz);

        if day != cur_day {
            // ---- 1. domknięcia okien, których ostatni dzień właśnie minął ----
            //
            // Kolejność jak w `runner.rs`: NAJPIERW broker widzi pierwszy kurs
            // nowej doby (nalicza swap, odświeża cenę), DOPIERO POTEM liczymy
            // wynik. Bez tego okno nie płaci ani swapu, ani luki nocnej.
            if cur_day != i64::MIN {
                // NOC KOSZTUJE. Kurs nowej doby idzie do KAŻDEGO żywego toru,
                // nie tylko do tego, który się właśnie kończy — tak samo jak
                // `runner.rs` robi to przy każdej granicy doby, także
                // w compoundingu. Bez tego ogon ZZN i okno wielodniowe
                // przechodziłyby przez północ, nie płacąc ani swapu, ani luki.
                //
                // D4: `mark` zamiast `on_quote` — swap i cena to wszystko,
                // czego wymaga „noc kosztuje". Pełne `on_quote` egzekwowało
                // tu wypełnienia i SL/TP PRZED komunikatami nocnymi, czyli
                // odwrotnie niż na zwykłym ticku, i liczyło ten sam tick
                // drugi raz w licznikach poziomu marginesu. Za osią, bo
                // zmienia liczby historyczne.
                for t in tory.iter_mut() {
                    if cfg.settings.runner_ksiegowanie_v2 {
                        t.broker.mark(q);
                    } else {
                        t.broker.on_tape_quote(q, i);
                    }
                    if check_cost_faults {
                        if let Some(result) = unreconciled_window(cfg, t, dni.len(), i-i0+1) {
                            return result;
                        }
                    }
                }
                let mut do_domkniecia: Vec<usize> = Vec::new();
                for (idx, t) in tory.iter_mut().enumerate() {
                    if t.granica_ts != 0 || t.dzien_koniec != cur_day {
                        continue;
                    }
                    zamknij_granice(t, &q, cfg);
                    // łańcuch wiedzy o reżimie idzie do kolejnych okien
                    regime_chain = t
                        .zespol
                        .lista
                        .iter()
                        .map(|si| si.engine.regime_hist.clone())
                        .collect();
                    if !cfg.zzn {
                        do_domkniecia.push(idx);
                    }
                }
                for idx in do_domkniecia.into_iter().rev() {
                    let mut t = tory.remove(idx);
                    domknij_ogon(&mut t, &q, cfg, false, &mut wyniki, i);
                    if check_cost_faults {
                        if let Some(result) = unreconciled_window(cfg, &t, dni.len(), i-i0+1) {
                            return result;
                        }
                    }
                }
            }

            // ---- 2. nowe okno, jeśli dziś któreś się zaczyna ----
            if let Some(&k) = start_w_dniu.get(&day) {
                let (mut zespol, pojedynczy) = zbuduj_zespol_okna(cfg);
                let (ph, vh) = kron.engine.market_history();
                // Stan trailingu S/R z kronikarza — ten sam powód co historia
                // rynku niżej: agregator 1M i potwierdzone swingi to wiedza
                // o RYNKU; okno startujące z pustą strukturą mierzyłoby oś
                // słabszą, niż widzi ją bot ciągły.
                let sr = kron.engine.stan_sr();
                for (idx, si) in zespol.lista.iter_mut().enumerate() {
                    // Ta sama historia rynku dla każdej nogi — tak samo jak
                    // `runner.rs:575`. To jest wiedza o RYNKU, nie o koncie.
                    si.engine.set_market_history(ph.clone(), vh.clone());
                    si.engine.set_stan_sr(sr.clone());
                    if let Some(h) = regime_chain.get(idx) {
                        si.engine.regime_hist = h.clone();
                    }
                }
                let mut b = if cfg.settings.credit_balance_separate {
                    // Seeds RAW minimum at B+C before the first observation.
                    // Merely subtracting C from a B-initialized minimum would
                    // fabricate a drawdown on an otherwise empty account.
                    SimBroker::z_ustawien(cfg.start_balance, &cfg.settings)
                } else {
                    // Preserve the exact historical OFF construction.
                    let mut b = SimBroker::new(
                        cfg.start_balance,
                        cfg.settings.stops_level,
                        cfg.settings.commission_per_lot,
                    );
                    ustaw_brokera(&mut b, &cfg.settings);
                    b
                };
                let reporting_credit = if b.credit_balance_separate
                    && b.credit.is_finite() && b.credit > 0.0 { b.credit } else { 0.0 };
                b.limit_price_improvement = cfg.sim_limit_price_improvement;
                b.defer_new_pending_sl = cfg.sim_new_pending_sl_next_tick;
                b.price_digits = ticks.price_digits();
                if let Some(digits) = cfg.sim_native_swap_cash_digits {
                    if let Err(reason) = b.set_native_swap_cash_digits(Some(digits)) {
                        let mut result = pusty(cfg, 0, 0);
                        result.sim_execution_reconciliation_required = Some(reason);
                        return result;
                    }
                }
                // Kursor wiadomości: okno widzi kanał od swojej pierwszej doby.
                let mi = msgs.partition_point(|m| m.ts + lat <= prev_ts);
                tory.push(Tor {
                    nr: k as u32 + 1,
                    dzien_start: dni[k],
                    dzien_koniec: dni[k + n - 1],
                    zespol,
                    pojedynczy,
                    broker: b,
                    reporting_credit,
                    mi,
                    ogon: false,
                    granica_ts: 0,
                    equity_na_granicy: cfg.start_balance,
                    poz_na_granicy: 0,
                    float_na_granicy: 0.0,
                    pend_na_granicy: 0,
                    kosz_na_granicy: 0,
                    peak_equity: cfg.start_balance,
                    max_dd: 0.0,
                });
            }
            cur_day = day;
        }

        // ---- kronikarz: sam rynek, żadnych wiadomości ----
        kron.broker.on_quote(q);
        kron.engine.on_tick_received(&mut kron.broker, &q, q.ts - cfg.settings.server_tz_offset_ms);

        // ---- tory ----
        for t in tory.iter_mut() {
            // Parytet z runner/live/XT: najpierw istniejące zlecenia oraz
            // CAŁE zarządzanie tickiem, dopiero potem nowa wiadomość.
            if causal_tick_before_messages {
                t.rozegraj_tick(q, q.ts - cfg.settings.server_tz_offset_ms, i);
            }
            while t.mi < msgs.len() && msgs[t.mi].ts + lat <= q.ts {
                let m = msgs[t.mi];
                t.mi += 1;
                t.broker.q = q;
                let im = IncomingMessage {
                    ts: q.ts,
                    source: source.clone(),
                    source_name: cfg.source_name.clone(),
                    msg_id: m.msg_id,
                    reply_to: m.reply_to,
                    edit_of: m.edit_of,
                    text: m.text.clone(),
                };
                if t.pojedynczy {
                    // ŚCIEŻKA PARYTETU. Przebieg jednoformatowy bierze CAŁY
                    // strumień niezależnie od pola `kanal` — tak samo jak brał
                    // przed podpięciem wielosilnika i tak samo jak robi to
                    // `runner.rs`. Bez widoku brokera, bez obcego obciążenia.
                    t.zespol.lista[0].engine.on_message_received(&mut t.broker, &im, m.ts + lat - cfg.settings.server_tz_offset_ms);
                } else {
                    // ROUTING PO KANALE. Brak trasy to POLICZONA strata,
                    // nie cisza — liczba ląduje w zastrzeżeniach wyniku.
                    match t.zespol.indeks_formatu(&m.kanal) {
                        Some(i) => {
                            // Kierunek sygnału musi być znany PRZED bramką,
                            // żeby pułap „nie otwieraj przeciwnie do innej
                            // nogi" miał czego pilnować. Ta sama kolejność
                            // co w `runner.rs` i w `live.rs::skieruj`.
                            let strona = conduit_core::parser::parse(&im.text)
                                .into_iter()
                                .find_map(|sg| match sg {
                                    conduit_core::parser::Signal::Entry(e) => Some(e.side),
                                    conduit_core::parser::Signal::MarketOpen { side } => Some(side),
                                    _ => None,
                                });
                            t.zespol.przelicz_obce(&t.broker, strona);
                            let br = &mut t.broker;
                            t.zespol.z_widokiem(i, br, |e, w| e.on_message_received(w, &im, m.ts + lat - cfg.settings.server_tz_offset_ms));
                        }
                        None => {
                            if bez_trasy_policzone.insert((m.kanal.clone(), m.msg_id)) {
                                let klucz = if m.kanal.is_empty() {
                                    "(bez kanału)".to_string()
                                } else {
                                    m.kanal.clone()
                                };
                                *bez_trasy.entry(klucz).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
            if !causal_tick_before_messages {
                // false zachowuje historyczne wiadomość→tick bez zmian.
                t.rozegraj_tick(q, q.ts - cfg.settings.server_tz_offset_ms, i);
            }
            if check_cost_faults {
                if let Some(result) = unreconciled_window(cfg, t, dni.len(), i-i0+1) {
                    return result;
                }
            }
            let eq = reporting_equity(t, t.broker.equity());
            if eq > t.peak_equity {
                t.peak_equity = eq;
            }
            let d = t.peak_equity - eq;
            if d > t.max_dd {
                t.max_dd = d;
            }
        }

        // ---- domknięcia ogonów ZZN ----
        if cfg.zzn && tory.iter().any(|t| t.ogon) {
            let sufit_ms = cfg.zzn_max_dni.max(1) as i64 * 86_400_000;
            let mut gotowe: Vec<usize> = Vec::new();
            for (idx, t) in tory.iter().enumerate() {
                if !t.ogon {
                    continue;
                }
                if !t.zywy_rynek() || q.ts - t.granica_ts >= sufit_ms {
                    gotowe.push(idx);
                }
            }
            for idx in gotowe.into_iter().rev() {
                let mut t = tory.remove(idx);
                let uciety = t.zywy_rynek();
                domknij_ogon(&mut t, &q, cfg, uciety, &mut wyniki, i);
                if check_cost_faults {
                    if let Some(result) = unreconciled_window(cfg, &t, dni.len(), i-i0+1) {
                        return result;
                    }
                }
            }
        }

        prev_ts = q.ts;
    }

    // ---------- koniec danych ----------
    //
    // Okno, którego ostatni dzień jest zarazem ostatnim dniem zakresu, nie
    // doczeka się „pierwszego ticka następnej doby" — a jest oknem PEŁNYM
    // i musi wejść do rozkładu. `runner.rs` robi dokładnie to samo: dopisuje
    // ostatnią dobę po wyjściu z pętli, licząc equity z ostatniego ticka.
    let qk = ticks.quote(i_koniec.saturating_sub(1));
    for mut t in tory.into_iter() {
        if t.granica_ts == 0 {
            if t.dzien_koniec != ostatni_dzien {
                // okno niepełne — nie ma prawa wejść do rozkładu
                continue;
            }
            zamknij_granice(&mut t, &qk, cfg);
        }
        let uciety = t.zywy_rynek();
        domknij_ogon(&mut t, &qk, cfg, uciety, &mut wyniki, i_koniec.saturating_sub(1));
        if check_cost_faults {
            if let Some(result) = unreconciled_window(cfg, &t, dni.len(), i_koniec-i0) {
                return result;
            }
        }
    }

    wyniki.sort_by_key(|w| w.nr);
    let mut w = podsumuj(
        cfg,
        wyniki,
        dni.len() as u32,
        (i_koniec - i0) as u64,
        t0,
        ostatni_dzien,
    );
    // Licznik PoOstatnimTicku (poz. 21): wiadomości spoza zasięgu pętli.
    // Filtr wyżej wpuszcza je do `msgs` (górna granica `t_kon.max(cfg.to)`),
    // ale żaden tor nigdy ich nie wyda — bez licznika znikały bez śladu.
    let ostatni_ts = ticks.ts(i_koniec.saturating_sub(1));
    w.po_ostatnim_ticku = msgs.iter().filter(|m| m.ts + lat > ostatni_ts).count() as u64;
    if w.po_ostatnim_ticku > 0 {
        w.zastrzezenia.push(format!(
            "{} wiadomości przypada PO ostatnim ticku danych — żadne okno ich nie wydało",
            w.po_ostatnim_ticku
        ));
    }
    for (kanal, ile) in bez_trasy {
        w.zastrzezenia.push(format!(
            "{ile} wiadomości z kanału „{kanal}” nie trafiło na żadną nogę —              te sygnały NIE były handlowane w żadnym oknie"
        ));
    }
    w
}

/// A broken cost proof is not a zero/partial-profit candidate. Check before
/// dropping every completed Tor as well as after ordinary ticks, including
/// native cash settlement with the NET receipt consumer disabled. Reuse the
/// existing non-rankable execution field so existing CLI consumers reject it.
fn unreconciled_window(cfg: &KonfOkien, t: &Tor, days: usize, ticks: usize)
    -> Option<WynikOkien> {
    let fault = t.broker.cost_reconciliation_required()
        .or_else(|| t.zespol.lista.iter()
            .find_map(|s| s.engine.cost_reconciliation_required.as_deref()))?;
    let mut result = pusty(cfg, days as u32, ticks as u64);
    result.sim_execution_reconciliation_required =
        Some(format!("COST HOLD in window #{}: {fault}", t.nr));
    result.zastrzezenia.push("Unreconciled costs: no windows published for ranking".into());
    Some(result)
}

fn zamknij_granice(t: &mut Tor, q: &Quote, cfg: &KonfOkien) {
    t.granica_ts = q.ts;
    t.equity_na_granicy = t.broker.equity();
    if t.reporting_credit > 0.0 {
        // The first tick of the next day can be the deepest observation.
        // A finishing window will not reach the ordinary tick/DD loop again.
        observe_reporting_equity(t);
    }
    t.poz_na_granicy = t.broker.positions().len() as u32;
    t.float_na_granicy = t.broker.positions().iter().map(|p| p.profit_usd(q)).sum();
    t.pend_na_granicy = t.broker.pendings().len() as u32;
    t.kosz_na_granicy = t.zywych_koszykow() as u32;
    if cfg.zzn {
        // ogon: nowych koszyków już nie zakładamy, stare dokańczamy —
        // KAŻDA noga osobno, inaczej jedna dalej otwierałaby po granicy
        for si in t.zespol.lista.iter_mut() {
            si.engine.wygaszanie = true;
        }
        t.ogon = true;
    }
}

/// Domknięcie toru — zarówno bez ZZN (natychmiast po granicy), jak i z ZZN
/// (gdy rynek toru wygasł albo gdy ogon trzeba było uciąć).
fn domknij_ogon(
    t: &mut Tor,
    q: &Quote,
    cfg: &KonfOkien,
    uciety: bool,
    wyniki: &mut Vec<WynikOkna>,
    row: usize,
) {
    t.broker.q = *q;
    if uciety || !cfg.zzn {
        if t.pojedynczy {
            t.zespol.lista[0]
                .engine
                .close_everything(&mut t.broker, q.ts, CloseReason::EodFlat);
        } else {
            let br = &mut t.broker;
            t.zespol
                .kazdy(br, |e, w| e.close_everything(w, q.ts, CloseReason::EodFlat));
        }
        t.broker.on_tape_quote(*q, row);
    }
    if t.reporting_credit > 0.0 {
        // Final settlement may change equity (e.g. a cost at forced exit).
        // Do not silently mix raw broker E with the own-capital peak.
        observe_reporting_equity(t);
    }
    let zysk_do_granicy = reporting_equity(t, t.equity_na_granicy) - cfg.start_balance;
    let zysk = if cfg.zzn {
        reporting_equity(t, t.broker.equity()) - cfg.start_balance
    } else {
        // bez ZZN wynik okna to STAN NA GRANICY — parytet z `--daily-reset`
        zysk_do_granicy
    };
    wyniki.push(WynikOkna {
        nr: t.nr,
        od: fmt_day(t.dzien_start),
        do_dnia: fmt_day(t.dzien_koniec),
        zysk,
        min_equity: reporting_equity(t, t.broker.min_equity),
        max_dd: t.max_dd,
        reporting_equity_basis: (t.reporting_credit > 0.0)
            .then(|| "own_equity_excluding_constant_credit".into()),
        initial_credit: (t.reporting_credit > 0.0).then_some(t.reporting_credit),
        raw_broker_boundary_equity: (t.reporting_credit > 0.0).then_some(t.equity_na_granicy),
        raw_broker_end_equity: (t.reporting_credit > 0.0).then(|| t.broker.equity()),
        raw_broker_min_equity: (t.reporting_credit > 0.0).then_some(t.broker.min_equity),
        trejdy: t.broker.history.len() as u32,
        koszyki: t.koszykow() as u32,
        pozycje_na_granicy: t.poz_na_granicy,
        floating_na_granicy: t.float_na_granicy,
        pendingi_na_granicy: t.pend_na_granicy,
        koszyki_zywe_na_granicy: t.kosz_na_granicy,
        zysk_do_granicy,
        ogon_ms: if cfg.zzn { q.ts - t.granica_ts } else { 0 },
        ogon_uciety: cfg.zzn && uciety,
    });
}

/// Explicit subtraction only for ON/positive C. OFF and C=0 retain the raw
/// value bit-for-bit (including signed zero), rather than computing E-0.
#[inline]
fn reporting_equity(t: &Tor, raw: f64) -> f64 {
    if t.reporting_credit > 0.0 { raw - t.reporting_credit } else { raw }
}

fn observe_reporting_equity(t: &mut Tor) {
    let eq = reporting_equity(t, t.broker.equity());
    t.peak_equity = t.peak_equity.max(eq);
    t.max_dd = t.max_dd.max(t.peak_equity - eq);
}

fn ustaw_brokera(b: &mut SimBroker, s: &Settings) {
    b.ustaw(s);
}

/// Doby, w których naprawdę są ticki.
fn dni_handlowe(ticks: &TickData, i0: usize, i1: usize, tz: i64) -> Vec<i64> {
    // Skok po granicach dób zamiast przeglądania 27 mln ticków: `index_at` to
    // wyszukiwanie binarne po zamapowanym pliku, więc kosztuje tyle co nic,
    // a pętla po tickach i tak nas czeka.
    let mut out = Vec::new();
    let mut ts = ticks.ts(i0);
    let koniec = ticks.ts(i1 - 1);
    loop {
        let d = day_of(ts, tz);
        out.push(d);
        let nast = (d + 1) * 86_400_000 - tz;
        if nast > koniec {
            break;
        }
        let j = ticks.index_at(nast);
        if j >= i1 {
            break;
        }
        ts = ticks.ts(j);
        if day_of(ts, tz) == d {
            break; // zabezpieczenie przed pętlą nieskończoną
        }
    }
    out
}

fn pusty(cfg: &KonfOkien, dni: u32, tickow: u64) -> WynikOkien {
    WynikOkien {
        continuation_reconciliation_required: None,
        sim_execution_reconciliation_required: None,
        n_dni: cfg.n_dni,
        zzn: cfg.zzn,
        okna: Vec::new(),
        suma: 0.0,
        srednia: 0.0,
        mediana: 0.0,
        pct_dodatnich: 0.0,
        okien_z_handlem: 0,
        mediana_z_handlem: 0.0,
        pct_dodatnich_z_handlem: 0.0,
        najgorsze_okno: 0.0,
        najlepsze_okno: 0.0,
        min_equity_najgorszego: cfg.start_balance,
        min_equity_globalne: cfg.start_balance,
        ucietych_pozycji: 0,
        uciety_floating: 0.0,
        ucietych_pendingow: 0,
        ucietych_koszykow: 0,
        okien_z_ucieciem: 0,
        okien_z_ucietym_ogonem: 0,
        najdluzszy_ogon_min: 0,
        sredni_ogon_min: 0.0,
        dni_handlowych: dni,
        tickow,
        czas_ms: 0,
        po_ostatnim_ticku: 0,
        zastrzezenia: Vec::new(),
    }
}

fn podsumuj(
    cfg: &KonfOkien,
    okna: Vec<WynikOkna>,
    dni: u32,
    tickow: u64,
    t0: std::time::Instant,
    _ostatni: i64,
) -> WynikOkien {
    let mut w = pusty(cfg, dni, tickow);
    if okna.is_empty() {
        w.czas_ms = t0.elapsed().as_millis() as u64;
        return w;
    }
    let mediana = |v: &mut Vec<f64>| -> f64 {
        if v.is_empty() {
            return 0.0;
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        if v.len() % 2 == 1 {
            v[v.len() / 2]
        } else {
            (v[v.len() / 2 - 1] + v[v.len() / 2]) / 2.0
        }
    };
    let mut z: Vec<f64> = okna.iter().map(|o| o.zysk).collect();
    w.suma = z.iter().sum();
    w.srednia = w.suma / z.len() as f64;
    w.mediana = mediana(&mut z);
    w.pct_dodatnich =
        100.0 * okna.iter().filter(|o| o.zysk > 0.0).count() as f64 / okna.len() as f64;

    // to samo, ale po oknach z handlem — mianownik zgodny z `Metrics`
    let mut za: Vec<f64> = okna
        .iter()
        .filter(|o| o.trejdy > 0)
        .map(|o| o.zysk)
        .collect();
    w.okien_z_handlem = za.len() as u32;
    w.pct_dodatnich_z_handlem = if za.is_empty() {
        0.0
    } else {
        100.0 * za.iter().filter(|x| **x > 0.0).count() as f64 / za.len() as f64
    };
    w.mediana_z_handlem = mediana(&mut za);
    w.najgorsze_okno = z[0];
    w.najlepsze_okno = z[z.len() - 1];
    let najg = okna
        .iter()
        .min_by(|a, b| {
            a.zysk
                .partial_cmp(&b.zysk)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap();
    w.min_equity_najgorszego = najg.min_equity;
    w.min_equity_globalne = okna
        .iter()
        .map(|o| o.min_equity)
        .fold(f64::INFINITY, f64::min);

    w.ucietych_pozycji = okna.iter().map(|o| o.pozycje_na_granicy as u64).sum();
    w.uciety_floating = okna.iter().map(|o| o.floating_na_granicy).sum();
    w.ucietych_pendingow = okna.iter().map(|o| o.pendingi_na_granicy as u64).sum();
    w.ucietych_koszykow = okna.iter().map(|o| o.koszyki_zywe_na_granicy as u64).sum();
    w.okien_z_ucieciem = okna
        .iter()
        .filter(|o| {
            o.pozycje_na_granicy > 0 || o.pendingi_na_granicy > 0 || o.koszyki_zywe_na_granicy > 0
        })
        .count() as u32;

    w.okien_z_ucietym_ogonem = okna.iter().filter(|o| o.ogon_uciety).count() as u32;
    w.najdluzszy_ogon_min = okna.iter().map(|o| o.ogon_ms).max().unwrap_or(0) / 60_000;
    w.sredni_ogon_min =
        okna.iter().map(|o| o.ogon_ms as f64).sum::<f64>() / okna.len() as f64 / 60_000.0;

    if cfg.settings.pyramid_after_stage > 0 && cfg.settings.pyramid_regime_lookback > 0 {
        w.zastrzezenia.push(
            "preset ma włączoną piramidę z bramką reżimu — `regime_hist` dziedziczy się \
             po oknie zakończonym ostatnio, więc dla N > 1 łańcuch jest o N dni starszy \
             niż w przebiegu ciągłym. Różnica dotyczy WYŁĄCZNIE dokładek piramidy."
                .into(),
        );
    }
    if w.okien_z_ucietym_ogonem > 0 {
        w.zastrzezenia.push(format!(
            "{} z {} okien miało ogon UCIĘTY sufitem {} dób albo końcem danych — \
             te okna są mierzone tak samo niedokładnie jak bez ZZN",
            w.okien_z_ucietym_ogonem,
            okna.len(),
            cfg.zzn_max_dni
        ));
    }
    w.okna = okna;
    w.czas_ms = t0.elapsed().as_millis() as u64;
    w
}

fn fmt_day(day: i64) -> String {
    let z = day + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mediana_parzysta_i_nieparzysta() {
        let cfg = KonfOkien::default();
        let mk = |z: f64, nr: u32| WynikOkna {
            nr,
            od: String::new(),
            do_dnia: String::new(),
            zysk: z,
            min_equity: 200.0,
            max_dd: 0.0,
            reporting_equity_basis: None,
            initial_credit: None,
            raw_broker_boundary_equity: None,
            raw_broker_end_equity: None,
            raw_broker_min_equity: None,
            trejdy: 0,
            koszyki: 0,
            pozycje_na_granicy: 0,
            floating_na_granicy: 0.0,
            pendingi_na_granicy: 0,
            koszyki_zywe_na_granicy: 0,
            zysk_do_granicy: z,
            ogon_ms: 0,
            ogon_uciety: false,
        };
        let t0 = std::time::Instant::now();
        let w = podsumuj(&cfg, vec![mk(1.0, 1), mk(5.0, 2), mk(3.0, 3)], 3, 0, t0, 0);
        assert_eq!(w.mediana, 3.0);
        assert_eq!(w.suma, 9.0);
        assert_eq!(w.najgorsze_okno, 1.0);
        let w = podsumuj(
            &cfg,
            vec![mk(1.0, 1), mk(5.0, 2), mk(3.0, 3), mk(-1.0, 4)],
            4,
            0,
            t0,
            0,
        );
        assert_eq!(w.mediana, 2.0);
        assert_eq!(w.pct_dodatnich, 75.0);
    }

    /// Okna bez handlu rozwadniają medianę do zera — dlatego podajemy OBA
    /// mianowniki. Ten test przypina, że drugi liczy się po oknach z handlem.
    #[test]
    fn okna_bez_handlu_maja_wlasny_mianownik() {
        let cfg = KonfOkien::default();
        let mk = |z: f64, nr: u32, t: u32| WynikOkna {
            nr,
            od: String::new(),
            do_dnia: String::new(),
            zysk: z,
            min_equity: 200.0,
            max_dd: 0.0,
            reporting_equity_basis: None,
            initial_credit: None,
            raw_broker_boundary_equity: None,
            raw_broker_end_equity: None,
            raw_broker_min_equity: None,
            trejdy: t,
            koszyki: 0,
            pozycje_na_granicy: 0,
            floating_na_granicy: 0.0,
            pendingi_na_granicy: 0,
            koszyki_zywe_na_granicy: 0,
            zysk_do_granicy: z,
            ogon_ms: 0,
            ogon_uciety: false,
        };
        // trzy okna bezczynne, dwa z handlem (jedno na plus, jedno na minus)
        let w = podsumuj(
            &cfg,
            vec![
                mk(0.0, 1, 0),
                mk(0.0, 2, 0),
                mk(0.0, 3, 0),
                mk(10.0, 4, 3),
                mk(-4.0, 5, 2),
            ],
            5,
            0,
            std::time::Instant::now(),
            0,
        );
        assert_eq!(w.mediana, 0.0, "po wszystkich oknach mediana jest zerem");
        assert_eq!(w.pct_dodatnich, 20.0);
        assert_eq!(w.okien_z_handlem, 2);
        assert_eq!(w.mediana_z_handlem, 3.0, "mediana z {{10, -4}} to 3");
        assert_eq!(w.pct_dodatnich_z_handlem, 50.0);
    }

    #[test]
    fn wklad_ogona_to_roznica() {
        let mut o = WynikOkna {
            nr: 1,
            od: String::new(),
            do_dnia: String::new(),
            zysk: 12.0,
            min_equity: 200.0,
            max_dd: 0.0,
            reporting_equity_basis: None,
            initial_credit: None,
            raw_broker_boundary_equity: None,
            raw_broker_end_equity: None,
            raw_broker_min_equity: None,
            trejdy: 0,
            koszyki: 0,
            pozycje_na_granicy: 2,
            floating_na_granicy: -3.0,
            pendingi_na_granicy: 5,
            koszyki_zywe_na_granicy: 1,
            zysk_do_granicy: 7.0,
            ogon_ms: 3_600_000,
            ogon_uciety: false,
        };
        assert_eq!(o.wklad_ogona(), 5.0);
        o.zysk = 4.0;
        assert_eq!(o.wklad_ogona(), -3.0);
    }
}
