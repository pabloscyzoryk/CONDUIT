
pub mod prices;
pub mod rest;
pub mod sniff;

use crate::coalesce::{Section, Sections};
use crate::state::StateHandle;
use crate::{settings_map, ui};
use conduit_backtest::sim::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::{Engine, IncomingMessage};
use conduit_core::types::*;
use prices::{Feed, FileFeed, SynthCfg, SynthFeed};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Symbol, na którym pracuje bot. Jeden — bo strategia jest o złocie.
pub const SYMBOL: &str = "XAUUSD";

/// Górna granica salda startowego. Wynika wprost z wymagania („do miliarda"),
/// a nie z ograniczenia technicznego — chodzi o to, żeby literówka w polu
/// nie stworzyła konta, przy którym każdy limit ryzyka staje się bez znaczenia.
pub const MAX_BALANCE: f64 = 1_000_000_000.0;

/// Ile ticków przetwarzamy w jednym obrocie pętli, zanim sprawdzimy
/// przerwanie i opublikujemy stan. Przy maksymalnej prędkości to ułamek
/// sekundy pracy, a przy 1× i tak nigdy nie zostanie osiągnięte.
const CHUNK: u64 = 40_000;

/// Odstęp publikacji stanu do interfejsu (ms). 200 ms = 5 odświeżeń na sekundę;
/// delty i tak koalescencjonują się do 10 Hz.
const PUBLISH_EVERY_MS: u128 = 200;

/// Najdłuższe REALNE oczekiwanie na kolejny tick. Powyżej tego progu uznajemy,
/// że trafiliśmy na przerwę sesyjną albo weekend, i przeskakujemy ją — inaczej
/// odtwarzanie przy 1× stałoby godzinę w miejscu każdej doby.
const MAX_WAIT_MS: i64 = 3_000;

/// Ile wiadomości trzymamy w stanie interfejsu.
const MSG_KEEP: usize = 300;

// ============================================================
//  KONFIGURACJA
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PriceSource {
    /// kwotowania z pliku `ticks.bin`
    File,
    /// generator: błądzenie losowe z ziarnem
    Synthetic,
}

impl Default for PriceSource {
    fn default() -> Self {
        PriceSource::File
    }
}

/// Konfiguracja trybu demo — dokument `demo.json` w katalogu roboczym.
///
/// Świadomie NIE jest częścią `conduit_core::Settings`: to nie jest ustawienie
/// strategii, tylko opis stanowiska testowego (skąd dane, jak szybko, od kiedy).
/// Wrzucenie go do `Settings` oznaczałoby, że preset strategii niesie ze sobą
/// ścieżki do plików na czyimś dysku.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DemoConfig {
    /// saldo startowe wirtualnego konta (0 … 1 000 000 000 $)
    pub balance: f64,
    pub price_source: PriceSource,

    pub ticks_path: String,
    /// „RRRR-MM-DD" albo puste = od początku pliku
    pub ticks_from: String,
    pub ticks_to: String,

    pub signals_path: String,
    pub signals_from: String,
    pub signals_to: String,
    /// `false` = graj wyłącznie na sygnałach wysyłanych ręcznie z panelu
    pub use_file_signals: bool,
    /// Opt-in listener replay for file messages; manual injection is current.
    pub live_telegram_ingress: bool,
    /// None inherits the live signal_max_age_min at demo start.
    pub live_ingress_max_age_min: Option<f64>,

    /// mnożnik odtwarzania: 1, 10, 60… `0` = maksymalna prędkość
    pub speed: f64,

    // ---- generator ----
    pub seed: u64,
    pub synth_start_price: f64,
    /// odchylenie w $ na pierwiastek sekundy
    pub synth_vol: f64,
    pub synth_spread: f64,
    pub synth_interval_ms: i64,

    /// Nadpisanie `Settings::msg_offset()` na czas przebiegu.
    /// `None` = weź z ustawień silnika.
    pub msg_clock_offset_ms: Option<i64>,
    pub source_name: String,
}

impl Default for DemoConfig {
    fn default() -> Self {
        DemoConfig {
            balance: 200.0,
            price_source: PriceSource::File,
            ticks_path: String::new(),
            ticks_from: String::new(),
            ticks_to: String::new(),
            signals_path: String::new(),
            signals_from: String::new(),
            signals_to: String::new(),
            use_file_signals: true,
            live_telegram_ingress: false,
            live_ingress_max_age_min: None,
            speed: 60.0,
            seed: 1,
            synth_start_price: 4118.0,
            synth_vol: 0.15,
            synth_spread: 0.24,
            synth_interval_ms: 250,
            msg_clock_offset_ms: None,
            source_name: "ATFX VIP SIGNALS".into(),
        }
    }
}

impl DemoConfig {
    /// Sprawdza to, co da się sprawdzić PRZED uruchomieniem wątku.
    ///
    /// Każdy błąd wraca jako zdanie do pokazania użytkownikowi. Cicha korekta
    /// („saldo poza zakresem, wziąłem 200") byłaby gorsza niż odmowa: człowiek
    /// zobaczyłby wynik przebiegu, którego nie zamawiał.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.live_ingress_max_age_min.is_some_and(|x| !x.is_finite() || x < 0.0) {
            anyhow::bail!("liveIngressMaxAgeMin requires finite minutes >=0 (0=off)");
        }
        if !self.balance.is_finite() || self.balance < 0.0 {
            anyhow::bail!("saldo startowe musi być liczbą ≥ 0");
        }
        if self.balance > MAX_BALANCE {
            anyhow::bail!("saldo startowe nie może przekroczyć {MAX_BALANCE:.0} $");
        }
        if !self.speed.is_finite() || self.speed < 0.0 {
            anyhow::bail!("przyspieszenie musi być liczbą ≥ 0 (0 = maksymalne)");
        }
        if self.price_source == PriceSource::File && self.ticks_path.trim().is_empty() {
            anyhow::bail!("wskaż plik z tickami albo przełącz źródło ceny na generator");
        }
        if self.price_source == PriceSource::Synthetic {
            if self.synth_interval_ms <= 0 {
                anyhow::bail!("odstęp między kwotowaniami generatora musi być większy od zera");
            }
            if !(0.0..=1000.0).contains(&self.synth_vol) {
                anyhow::bail!("zmienność generatora poza sensownym zakresem (0 … 1000 $/√s)");
            }
            if self.synth_spread < 0.0 {
                anyhow::bail!("spread nie może być ujemny");
            }
            if self.synth_start_price <= 0.0 {
                anyhow::bail!("cena startowa generatora musi być dodatnia");
            }
        }
        if self.use_file_signals && !self.signals_path.trim().is_empty() {
            let p = std::path::Path::new(&self.signals_path);
            if !p.is_file() {
                anyhow::bail!("nie ma pliku sygnałów: {}", self.signals_path);
            }
        }
        okno(&self.ticks_from, &self.ticks_to, "ticków")?;
        okno(&self.signals_from, &self.signals_to, "sygnałów")?;
        Ok(())
    }

    fn okno_tickow(&self) -> (Ts, Ts) {
        (
            dzien_ms(&self.ticks_from).unwrap_or(0),
            koniec_dnia(&self.ticks_to),
        )
    }
    fn okno_sygnalow(&self) -> (Ts, Ts) {
        (
            dzien_ms(&self.signals_from).unwrap_or(0),
            koniec_dnia(&self.signals_to),
        )
    }
}

/// Zakres dat: obie granice muszą się parsować i „od" nie może być po „do".
fn okno(od: &str, do_: &str, co: &str) -> anyhow::Result<()> {
    let a = dzien_ms(od);
    let b = dzien_ms(do_);
    if !od.trim().is_empty() && a.is_none() {
        anyhow::bail!("data początkowa {co} ma zły format (oczekiwany RRRR-MM-DD): „{od}”");
    }
    if !do_.trim().is_empty() && b.is_none() {
        anyhow::bail!("data końcowa {co} ma zły format (oczekiwany RRRR-MM-DD): „{do_}”");
    }
    if let (Some(a), Some(b)) = (a, b) {
        if a > b {
            anyhow::bail!("zakres dat {co} jest odwrócony: {od} … {do_}");
        }
    }
    Ok(())
}

/// „RRRR-MM-DD" → ms północy. Puste → `None`.
pub fn dzien_ms(s: &str) -> Option<Ts> {
    if s.trim().is_empty() {
        return None;
    }
    crate::lab::parse_dzien(s).ok()
}

/// Górna granica okna: data „do" jest WŁĄCZNIE, więc bierzemy jej koniec doby.
/// `0` = bez ograniczenia.
fn koniec_dnia(s: &str) -> Ts {
    match dzien_ms(s) {
        Some(t) => t + 86_400_000,
        None => 0,
    }
}

// ============================================================
//  STAN WIDOCZNY W INTERFEJSIE
// ============================================================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DemoState {
    pub running: bool,
    /// `idle` | `running` | `finished` | `stopped` | `failed`
    pub phase: String,
    pub config: DemoConfig,
    /// wirtualny czas (zegar ticków) w ms
    pub clock: i64,
    pub clock_label: String,
    pub speed: f64,
    pub start_balance: f64,
    pub balance: f64,
    pub equity: f64,
    pub ticks_done: u64,
    pub ticks_total: u64,
    pub progress: f64,
    pub messages: u64,
    pub signals: u64,
    pub manual_signals: u64,
    pub trades: u64,
    pub open_positions: u32,
    pub open_pendings: u32,
    pub baskets: u32,
    /// `file` | `synthetic`
    pub source: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub elapsed_ms: i64,
    pub note: String,
    pub error: Option<String>,
}

impl DemoState {
    pub fn idle(cfg: DemoConfig) -> Self {
        DemoState {
            phase: "idle".into(),
            config: cfg,
            ..Default::default()
        }
    }
}

// ============================================================
//  STEROWANIE
// ============================================================

/// Polecenie wrzucone do pętli demo z interfejsu.
#[derive(Debug, Clone)]
pub enum DemoCmd {
    Message {
        text: String,
        source_name: String,
        adres: Box<crate::proto::Command>,
    },
    ClosePosition(Ticket),
    CloseAll,
    CloseBasket(u32),
    DeletePending(Ticket),
    DeleteAllPendings,
    ResumeTrading,
    RearmGuard,
}

/// Uchwyt trzymany w [`crate::state::Shared`]. Sam nie liczy — pilnuje, żeby
/// naraz szedł jeden przebieg, i przenosi polecenia z wątku HTTP do pętli.
#[derive(Default)]
pub struct DemoControl {
    running: AtomicBool,
    cancel: parking_lot::Mutex<Option<Arc<AtomicBool>>>,
    inbox: parking_lot::Mutex<Vec<DemoCmd>>,
    /// prędkość odtwarzania jako bity `f64` — zmienialna w trakcie przebiegu
    speed: AtomicU64,
}

impl DemoControl {
    #[inline]
    pub fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn try_claim(&self, cancel: Arc<AtomicBool>) -> bool {
        if self
            .running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        *self.cancel.lock() = Some(cancel);
        self.inbox.lock().clear();
        true
    }

    fn release(&self) {
        *self.cancel.lock() = None;
        self.inbox.lock().clear();
        self.running.store(false, Ordering::SeqCst);
    }

    /// Prosi przebieg o zatrzymanie. `false` = nic nie chodzi.
    pub fn request_stop(&self) -> bool {
        match self.cancel.lock().as_ref() {
            Some(c) => {
                c.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// Wrzuca polecenie do pętli. `false` = demo nie chodzi.
    pub fn push(&self, cmd: DemoCmd) -> bool {
        if !self.running() {
            return false;
        }
        let mut q = self.inbox.lock();
        // Kolejka jest ograniczona: przy zerwanym połączeniu albo pętli
        // klikania nie chcemy trzymać nieskończonej listy poleceń.
        if q.len() >= 500 {
            return false;
        }
        q.push(cmd);
        true
    }

    fn drain(&self) -> Vec<DemoCmd> {
        std::mem::take(&mut *self.inbox.lock())
    }

    pub fn set_speed(&self, v: f64) {
        self.speed.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn speed(&self) -> f64 {
        let v = f64::from_bits(self.speed.load(Ordering::Relaxed));
        if v.is_finite() && v >= 0.0 {
            v
        } else {
            1.0
        }
    }
}

// ============================================================
//  URUCHOMIENIE I ZATRZYMANIE
// ============================================================

/// Sekcje stanu, które przebieg demo nadpisuje — i które po zatrzymaniu
/// trzeba oddać w takim stanie, w jakim je zastał.
fn sekcje() -> Sections {
    Sections::one(Section::Quotes)
        | Section::Positions
        | Section::Pendings
        | Section::Baskets
        | Section::Closed
        | Section::Stats
        | Section::Messages
        | Section::Connection
        | Section::Demo
}

/// Migawka tego, co demo nadpisze — żeby po zatrzymaniu wrócił obraz konta
/// sprzed przebiegu, a nie wirtualne pozycje udające prawdziwe.
#[derive(Clone)]
struct Zastane {
    positions: Vec<ui::Position>,
    pendings: Vec<ui::PendingOrder>,
    baskets: Vec<ui::Basket>,
    closed: Vec<ui::ClosedPosition>,
    messages: Vec<ui::ChatMessage>,
    quotes: std::collections::BTreeMap<String, ui::Quote>,
    balance: f64,
    stats: ui::Stats,
    mt5: String,
    resolved_symbol: String,
    account: ui::AccountInfo,
}

fn zapamietaj(st: &StateHandle) -> Zastane {
    st.read(|s| Zastane {
        positions: s.positions.clone(),
        pendings: s.pendings.clone(),
        baskets: s.baskets.clone(),
        closed: s.closed.clone(),
        messages: s.messages.clone(),
        quotes: s.quotes.clone(),
        balance: s.balance,
        stats: s.stats.clone(),
        mt5: s.connection.mt5.clone(),
        resolved_symbol: s.connection.resolved_symbol.clone(),
        account: s.connection.account.clone(),
    })
}

fn przywroc(st: &StateHandle, z: &Zastane) {
    st.update_transient(sekcje(), |s| {
        s.positions = z.positions.clone();
        s.pendings = z.pendings.clone();
        s.baskets = z.baskets.clone();
        s.closed = z.closed.clone();
        s.messages = z.messages.clone();
        s.quotes = z.quotes.clone();
        s.balance = z.balance;
        s.stats = z.stats.clone();
        s.connection.mt5 = z.mt5.clone();
        s.connection.resolved_symbol = if z.mt5 == "connected" { z.resolved_symbol.clone() } else { String::new() };
        s.connection.account = z.account.clone();
        s.demo.running = false;
    });
}

/// Startuje przebieg demo w osobnym wątku. Wraca NATYCHMIAST.
///
/// Osobny wątek systemowy, a nie `spawn_blocking`: przebieg trwa tak długo,
/// jak użytkownik zechce (przy 1× może chodzić dobami), a pula blokująca tokia
/// jest współdzielona z obsługą HTTP.
pub fn start(st: &StateHandle, cfg: DemoConfig) -> anyhow::Result<()> {
    cfg.validate()?;
    let cancel = Arc::new(AtomicBool::new(false));
    if !st.demo.try_claim(cancel.clone()) {
        anyhow::bail!("tryb demo już chodzi — najpierw go zatrzymaj");
    }
    st.demo.set_speed(cfg.speed);
    let _ = st.workspace.save_demo(&cfg);

    let zastane = zapamietaj(st);
    let now = crate::now_ms();
    st.update(sekcje(), |s| {
        s.demo = DemoState {
            running: true,
            phase: "running".into(),
            config: cfg.clone(),
            speed: cfg.speed,
            start_balance: cfg.balance,
            balance: cfg.balance,
            equity: cfg.balance,
            source: if cfg.price_source == PriceSource::File {
                "file"
            } else {
                "synthetic"
            }
            .into(),
            started_at: now,
            note: "przygotowanie danych…".into(),
            ..Default::default()
        };
    });

    let st2 = st.clone();
    let cfg_watku = cfg.clone();
    let wynik = std::thread::Builder::new()
        .name("conduit-demo".into())
        .spawn(move || {
            let r = petla(&st2, &cfg_watku, &cancel);
            let przerwane = cancel.load(Ordering::SeqCst);
            let now = crate::now_ms();
            match r {
                Ok(nota) => {
                    st2.update(Sections::one(Section::Demo), |s| {
                        s.demo.running = false;
                        s.demo.phase = if przerwane {
                            "stopped".into()
                        } else {
                            "finished".into()
                        };
                        s.demo.note = nota.clone();
                        s.demo.finished_at = Some(now);
                    });
                    st2.log(
                        "events",
                        if przerwane { "warn" } else { "success" },
                        if przerwane {
                            "Tryb demo zatrzymany"
                        } else {
                            "Tryb demo — koniec danych"
                        },
                        nota,
                    );
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    st2.update(Sections::one(Section::Demo), |s| {
                        s.demo.running = false;
                        s.demo.phase = "failed".into();
                        s.demo.error = Some(msg.clone());
                        s.demo.finished_at = Some(now);
                    });
                    st2.log("events", "error", "Tryb demo nie ruszył", msg);
                }
            }
            przywroc(&st2, &zastane);
            st2.demo.release();
        });

    if let Err(e) = wynik {
        st.demo.release();
        anyhow::bail!("nie udało się uruchomić wątku demo: {e}");
    }
    st.log(
        "events",
        "info",
        "TRYB DEMO",
        format!(
            "wirtualny broker, saldo {:.2} $, źródło ceny: {}, tempo {}",
            cfg.balance,
            if cfg.price_source == PriceSource::File {
                "plik ticków"
            } else {
                "generator"
            },
            opis_tempa(cfg.speed)
        ),
    );
    Ok(())
}

pub fn opis_tempa(v: f64) -> String {
    if v <= 0.0 {
        "maksymalne".into()
    } else if (v - v.round()).abs() < 1e-9 {
        format!("{}×", v.round() as i64)
    } else {
        format!("{v:.2}×")
    }
}

/// „Zatrzymaj". Odpowiedź wraca od razu; pętla domyka bieżącą porcję ticków.
pub fn stop(st: &StateHandle) -> anyhow::Result<()> {
    if !st.demo.request_stop() {
        anyhow::bail!("tryb demo nie jest uruchomiony");
    }
    Ok(())
}

// ============================================================
//  RĘCZNY SYGNAŁ
// ============================================================

/// Wiadomość napisana ręcznie w panelu.
///
/// Działa W OBU TRYBACH i to jest cały sens tej funkcji:
///  * demo chodzi → wiadomość wpada do pętli demo i silnik reaguje na nią
///    dokładnie tak, jakby przyszła z Telegrama,
///  * demo nie chodzi → wiadomość idzie do środowiska uruchomieniowego,
///    czyli do prawdziwego bota.
///
/// Niezależnie od trybu wiadomość jest PARSOWANA I POKAZANA w panelu — także
/// wtedy, gdy nie ma czym jej wykonać. Człowiek ma zobaczyć, co parser z niej
/// zrozumiał, zanim dowie się, że brokera nie ma.
pub fn manual_signal(st: &StateHandle, cmd: &crate::proto::Command) -> anyhow::Result<i64> {
    manual_signal_scoped(st, cmd, None)
}

pub fn manual_signal_scoped(st: &StateHandle, cmd: &crate::proto::Command, account_session: Option<&str>) -> anyhow::Result<i64> {
    crate::commands::validate_account_session(st, cmd, account_session)?;
    let crate::proto::Command::SimulateMessage {
        text,
        channel_id,
        topic_id,
        msg_id,
        edit_of,
        ..
    } = cmd
    else {
        anyhow::bail!("to nie jest polecenie wstrzyknięcia wiadomości");
    };
    let tekst = text.trim().to_string();
    if tekst.is_empty() {
        anyhow::bail!("pusta wiadomość");
    }
    let nazwa = st.read(|s| s.demo.config.source_name.clone());
    let nazwa = if nazwa.is_empty() {
        crate::wstrzykniecie::ZRODLO_PANELU.to_string()
    } else {
        nazwa
    };

    // ---------- NUMER NADAWANY TUTAJ, NIE NIŻEJ ----------
    //
    // Numer musi być znany ZANIM powstanie wpis dla panelu, bo inaczej panel
    // pokazuje wiadomość, której numeru nikt nie zna — a wtedy nie da się
    // wysłać jej edycji, czyli cała reszta F5 jest bezużyteczna. Ta sama
    // liczba jedzie do silnika (patrz `pelne` niżej), więc to, co widać na
    // ekranie, i to, czym operuje silnik, jest jedną i tą samą wiadomością.
    let numer = msg_id.unwrap_or_else(|| crate::wstrzykniecie::nadaj_numer(crate::now_ms()));

    // ---------- KLON I ŁATKA, nigdy budowa od zera ----------
    //
    // Zaczynamy od KLONU całego polecenia i podmieniamy w nim dwa pola.
    // Dzięki temu pola, których ta funkcja nie zna, jadą dalej nietknięte —
    // a `..` w łatce niżej niczego nie gubi, bo nie buduje nowej wartości,
    // tylko wskazuje „reszty nie ruszam".
    //
    // Poprzednia wersja składała polecenie OD NOWA z `text` i `channel_id`
    // i właśnie dlatego reszta pól ginęła po drodze bez śladu w logu.
    let mut pelne = cmd.clone();
    if let crate::proto::Command::SimulateMessage {
        text: t, msg_id: n, ..
    } = &mut pelne
    {
        *t = tekst.clone();
        *n = Some(numer);
    }

    let mut msg = wiadomosc_ui(
        crate::now_ms(),
        channel_id.unwrap_or(crate::wstrzykniecie::KANAL_PANELU),
        &nazwa,
        &tekst,
        format!("manual-{numer}"),
    );
    msg.topic_id = *topic_id;
    msg.topic_name = topic_id.map(|t| format!("temat {t}"));
    msg.edited = edit_of.is_some();

    // Kto dopisuje wiadomość do listy w panelu: gdy demo chodzi, listą zarządza
    // JEGO pętla (publikuje ją przy każdym odświeżeniu), więc dopisanie tutaj
    // zostałoby nadpisane przy najbliższej publikacji i sygnał zniknąłby
    // z ekranu sekundę po wysłaniu.
    let do_demo = st.demo.push(DemoCmd::Message {
        text: tekst.clone(),
        source_name: nazwa,
        adres: Box::new(pelne.clone()),
    });
    if !do_demo {
        st.update(Sections::one(Section::Messages), |s| {
            s.messages.push(msg.clone());
            if s.messages.len() > MSG_KEEP {
                let ile = s.messages.len() - MSG_KEEP;
                s.messages.drain(0..ile);
            }
        });
    }
    st.emit(ui::UiEvent::Signal {
        message: Box::new(msg),
    });

    if do_demo {
        st.log("telegram", "info", "Sygnał ręczny → tryb demo", tekst);
        return Ok(numer);
    }

    // poza demo: do prawdziwego silnika — poleceniem UZUPEŁNIONYM, nie
    // odtworzonym
    let rt = st.runtime.read().clone();
    match rt.command_scoped(&pelne, st, account_session) {
        Ok(()) => {
            st.log(
                "telegram",
                "info",
                format!("Sygnał ręczny → silnik (wiadomość {numer})"),
                tekst,
            );
            Ok(numer)
        }
        Err(e) => Err(e),
    }
}

/// Polecenia handlowe, które w trybie demo obsługuje wirtualny broker.
/// `None` = to nie jest nic, co demo umie (albo demo nie chodzi) — wołający
/// przekazuje komendę dalej, do prawdziwego środowiska.
pub fn try_command(st: &StateHandle, cmd: &crate::proto::Command) -> Option<anyhow::Result<()>> {
    use crate::proto::Command as C;
    if !st.demo.running() {
        return None;
    }
    let d = match cmd {
        C::ClosePosition { ticket } => DemoCmd::ClosePosition(*ticket),
        C::CloseBulk { .. } => DemoCmd::CloseAll,
        C::CloseBasket { id } => DemoCmd::CloseBasket(*id),
        C::DeletePending { ticket } => DemoCmd::DeletePending(*ticket),
        C::DeleteAllPendings => DemoCmd::DeleteAllPendings,
        C::ResumeTrading => DemoCmd::ResumeTrading,
        C::RearmGuard => DemoCmd::RearmGuard,
        _ => return None,
    };
    Some(if st.demo.push(d) {
        Ok(())
    } else {
        Err(anyhow::anyhow!("kolejka poleceń trybu demo jest pełna"))
    })
}

// ============================================================
//  PĘTLA ODTWARZANIA
// ============================================================

fn demo_file_ingress(m: &conduit_backtest::ReplayMessage, im: &IncomingMessage,
    memory: &mut conduit_core::telegram_ingress::ContentMemory, max_age: f64) -> bool {
    use conduit_core::telegram_ingress::{ContentMemory, opens_basket, stale_entry_age_minutes};
    if m.kanal == "__CONDUIT_CONTROL__" && m.text == "__CONDUIT_LIVEBACKTEST_INGRESS_RESTART__" {
        *memory=ContentMemory::new();return false;
    }
    !memory.duplikat_tresci(im) && !m.telegram_published_ts.is_some_and(|published|
        stale_entry_age_minutes(m.ts,published,max_age).is_some() && opens_basket(&m.text,m.edit_of))
}

fn petla(st: &StateHandle, cfg: &DemoConfig, cancel: &Arc<AtomicBool>) -> anyhow::Result<String> {
    // ---------- konfiguracja silnika: dokładnie ta z panelu ----------
    let (doc, lot) = st.read(|s| (s.settings.clone(), s.lot.clone()));
    let max_entry_age = conduit_core::telegram_ingress::normalized_max_entry_age_min(
        cfg.live_ingress_max_age_min.or_else(|| doc.get("signal_max_age_min").and_then(|x|x.as_f64())));
    let mut ingress_memory = conduit_core::telegram_ingress::ContentMemory::new();
    let mut core = settings_map::core_from_ui(&doc);
    core.lot_mode_percent = lot.mode == "percent";
    core.lot_fixed = lot.fixed;
    core.lot_percent = lot.percent;
    if let Some(o) = cfg.msg_clock_offset_ms {
        core.msg_clock_offset_ms = Some(o);
    }

    // ---------- źródło ceny ----------
    let (tf, tt) = cfg.okno_tickow();
    let mut feed: Box<dyn Feed> = match cfg.price_source {
        PriceSource::File => Box::new(FileFeed::open(
            std::path::Path::new(&cfg.ticks_path),
            tf,
            tt,
        )?),
        PriceSource::Synthetic => {
            let start = if tf > 0 {
                tf
            } else {
                crate::now_ms() + core.server_tz_offset_ms
            };
            Box::new(SynthFeed::new(SynthCfg {
                seed: cfg.seed,
                start_price: cfg.synth_start_price,
                vol: cfg.synth_vol,
                spread: cfg.synth_spread,
                interval_ms: cfg.synth_interval_ms,
                start_ts: start,
                end_ts: tt,
            }))
        }
    };

    // ---------- wiadomości z pliku ----------
    let lat = core.msg_offset() + core.exec_latency_ms;
    let (sf, sto) = cfg.okno_sygnalow();
    let mut msgs: Vec<conduit_backtest::ReplayMessage> = Vec::new();
    if cfg.use_file_signals && !cfg.signals_path.trim().is_empty() {
        let wszystkie = conduit_backtest::load_messages(&cfg.signals_path)?;
        msgs = wszystkie
            .into_iter()
            .filter(|m| sf == 0 || m.ts >= sf)
            .filter(|m| sto == 0 || m.ts < sto)
            .collect();
        msgs.sort_by_key(|m| (m.ts, m.msg_id));
    }
    let mut mi = 0usize;

    // ---------- broker i silnik ----------
    let mut broker = SimBroker::new(cfg.balance, core.stops_level, core.commission_per_lot);
    broker.ustaw(&core);
    let mut engine = Engine::new(core.clone(), cfg.balance);
    // Identyfikator przebiegu: prefiks wszystkich `event_id` w dzienniku.
    // Rdzeń nie ma zegara, więc nadaje go warstwa, która go ma — dzięki
    // temu zdarzeń z dwóch uruchomień nie da się pomylić.
    engine.set_run_id(format!("demo-{}", crate::now_ms()));

    // ---------- dziennik zdarzeń ----------
    // Rotacja po dobie handlowej serwera, jeden plik na dobę, nazwa z datą.
    let mut dziennik = if core.journal_enabled {
        Some(crate::journal::JournalWriter::new(
            st.workspace.journal_dir(),
            crate::journal::WriterConfig {
                text_mirror: core.journal_text_mirror,
                retention_days: core.journal_retention_days,
                local_offset_ms: core.server_tz_offset_ms,
                prefix: "demo".into(),
            },
        ))
    } else {
        None
    };
    let source = SourceKey::new(-1_000_000_000_301, None);

    let total = feed.total();
    let pierwszy = feed.peek_ts().unwrap_or(0);
    st.update_transient(Sections::one(Section::Demo), |s| {
        s.demo.ticks_total = total;
        s.demo.note = format!(
            "{} kwotowań, {} wiadomości w oknie",
            if total > 0 {
                total.to_string()
            } else {
                "generator — bez końca".into()
            },
            msgs.len()
        );
    });

    // ---------- zegar odtwarzania ----------
    let mut base_v = pierwszy; // wirtualny punkt odniesienia
    let mut base_r = Instant::now(); // realny punkt odniesienia
    let mut speed = st.demo.speed();
    let start_real = Instant::now();
    let mut ostatnia_publikacja = Instant::now() - Duration::from_secs(60);
    let mut ostatni_ts = pierwszy;
    let mut manual = 0u64;
    let mut przeskoki = 0u64;
    let mut bufor: Vec<ui::ChatMessage> = Vec::new();
    let mut msg_seq = msgs.iter().map(|m| m.msg_id).max().unwrap_or(0) + 1_000_000;

    loop {
        if cancel.load(Ordering::SeqCst) {
            break;
        }

        // zmiana tempa w locie: przestawiamy punkt odniesienia, żeby nowa
        // prędkość liczyła się od TERAZ, a nie od początku przebiegu
        let s_now = st.demo.speed();
        if (s_now - speed).abs() > 1e-9 {
            speed = s_now;
            base_v = ostatni_ts;
            base_r = Instant::now();
        }

        // ---------- polecenia z panelu ----------
        // Dopiero PO pierwszym kwotowaniu. Polecenie obsłużone wcześniej
        // trafiłoby na brokera z ceną 0 — silnik policzyłby strefę i SL
        // względem zera i otworzył coś, czego nikt nie zamawiał. Polecenia
        // wysłane przed startem czekają w skrzynce, zamiast przepadać.
        for c in if broker.quote().ts == 0 {
            Vec::new()
        } else {
            st.demo.drain()
        } {
            match c {
                DemoCmd::Message {
                    text,
                    source_name,
                    adres,
                } => {
                    msg_seq += 1;
                    manual += 1;
                    let mut im = crate::wstrzykniecie::wiadomosc(&adres, ostatni_ts, &source_name)
                        .unwrap_or_else(|| IncomingMessage {
                            ts: ostatni_ts,
                            source: source.clone(),
                            source_name: source_name.clone(),
                            msg_id: msg_seq,
                            reply_to: None,
                            edit_of: None,
                            text: text.clone(),
                        });
                    // ODTWARZANIE MA WŁASNY KANAŁ. Koszyki demo powstają pod
                    // `source` z linii wyżej, więc wstrzyknięcie bez jawnego
                    // `channelId` musi trafić TAM, a nie do pseudokanału
                    // panelu — inaczej edycja nie miałaby czego edytować.
                    if im.source.chat_id == crate::wstrzykniecie::KANAL_PANELU {
                        im.source = SourceKey::new(source.chat_id, im.source.topic_id);
                    }
                    engine.on_message(&mut broker, &im);
                    // do TEGO bufora, a nie wprost do stanu: listę wiadomości
                    // publikuje pętla i nadpisałaby wpis dodany z zewnątrz
                    let mut m = wiadomosc_ui(
                        ostatni_ts,
                        im.source.chat_id,
                        &source_name,
                        &text,
                        format!("m{msg_seq}"),
                    );
                    m.topic_id = im.source.topic_id;
                    m.edited = im.edit_of.is_some();
                    bufor.push(m);
                }
                DemoCmd::ClosePosition(t) => {
                    let _ = broker.close_position(t, CloseReason::Manual);
                }
                DemoCmd::CloseAll => {
                    engine.close_everything(&mut broker, ostatni_ts, CloseReason::Manual);
                }
                DemoCmd::CloseBasket(id) => zamknij_koszyk(&mut broker, id),
                DemoCmd::DeletePending(t) => {
                    let _ = broker.cancel_pending(t);
                }
                DemoCmd::DeleteAllPendings => {
                    let tickets: Vec<Ticket> = broker.pendings().iter().map(|o| o.ticket).collect();
                    for t in tickets {
                        let _ = broker.cancel_pending(t);
                    }
                }
                DemoCmd::ResumeTrading => engine.resume_trading(ostatni_ts),
                DemoCmd::RearmGuard => engine.rearm_guard(ostatni_ts),
            }
        }

        // ---------- ile czasu wirtualnego wolno przerobić ----------
        let target_v = if speed <= 0.0 {
            i64::MAX
        } else {
            base_v + (base_r.elapsed().as_millis() as f64 * speed) as i64
        };

        let mut zrobione = 0u64;
        let mut koniec = false;
        loop {
            match feed.peek_ts() {
                Some(ts) if ts <= target_v => {}
                Some(_) => break,
                None => {
                    koniec = true;
                    break;
                }
            }
            let q = match feed.next() {
                Some(q) => q,
                None => {
                    koniec = true;
                    break;
                }
            };
            ostatni_ts = q.ts;

            // wiadomości, których czas już nadszedł
            while mi < msgs.len() && msgs[mi].ts + lat <= q.ts {
                let m = &msgs[mi];
                mi += 1;
                broker.q = q; // broker musi znać cenę PRZED obsługą wiadomości
                let im = IncomingMessage {
                    ts: q.ts,
                    source: source.clone(),
                    source_name: cfg.source_name.clone(),
                    msg_id: m.msg_id,
                    reply_to: m.reply_to,
                    edit_of: m.edit_of,
                    text: m.text.clone(),
                };
                if cfg.live_telegram_ingress && !demo_file_ingress(m, &im, &mut ingress_memory, max_entry_age) {
                    continue;
                }
                engine.on_message(&mut broker, &im);
                bufor.push(wiadomosc_ui(
                    q.ts,
                    -1_000_000_000_301,
                    &cfg.source_name,
                    &m.text,
                    format!("d{}", m.msg_id),
                ));
                if bufor.len() > MSG_KEEP {
                    let ile = bufor.len() - MSG_KEEP;
                    bufor.drain(0..ile);
                }
            }

            broker.on_quote(q);
            engine.on_tick(&mut broker, &q);

            // Bufor w rdzeniu ma sufit — nieodebrane zdarzenia przepadają,
            // dlatego opróżniamy go w pętli, a nie na końcu przebiegu.
            if let Some(d) = dziennik.as_mut() {
                if engine.journal.len() >= 512 {
                    let mut evs = engine.drain_journal();
                    if let Err(e) = d.write(&mut evs, crate::now_ms()) {
                        tracing::warn!("dziennik zdarzeń: {e}");
                    }
                }
            }

            zrobione += 1;
            if zrobione >= CHUNK {
                break;
            }
            if broker.blown {
                koniec = true;
                break;
            }
        }

        // ---------- publikacja ----------
        if ostatnia_publikacja.elapsed().as_millis() >= PUBLISH_EVERY_MS || koniec {
            ostatnia_publikacja = Instant::now();
            opublikuj(
                st,
                &broker,
                &engine,
                &bufor,
                Postep {
                    clock: ostatni_ts,
                    speed,
                    done: feed.done(),
                    total,
                    manual,
                    elapsed_ms: start_real.elapsed().as_millis() as i64,
                    start_balance: cfg.balance,
                },
            );
        }

        // domknięcie porcji przy każdym obrocie pętli sterowania — dzięki
        // temu plik jest aktualny także wtedy, gdy dane płyną wolno
        if let Some(d) = dziennik.as_mut() {
            if !engine.journal.is_empty() {
                let mut evs = engine.drain_journal();
                let _ = d.write(&mut evs, crate::now_ms());
            }
        }

        if koniec {
            let powod = if broker.blown {
                format!("konto wyzerowane po {} transakcjach", broker.history.len())
            } else {
                format!(
                    "przerobiono {} kwotowań do {}",
                    feed.done(),
                    crate::lab::dzien(ostatni_ts)
                )
            };
            return Ok(format!(
                "{powod}; saldo {:.2} $ (start {:.2} $), transakcji {}, ręcznych sygnałów {manual}",
                broker.balance,
                cfg.balance,
                broker.history.len()
            ));
        }

        // ---------- luka w danych ----------
        if speed > 0.0 {
            if let Some(nts) = feed.peek_ts() {
                let czekanie = ((nts - target_v) as f64 / speed) as i64;
                if czekanie > MAX_WAIT_MS {
                    // przerwa sesyjna albo weekend — przeskakujemy zamiast stać
                    base_v = nts;
                    base_r = Instant::now();
                    przeskoki += 1;
                    if przeskoki <= 8 {
                        st.log(
                            "events",
                            "info",
                            "Tryb demo: przeskok przerwy w notowaniach",
                            format!(
                                "{} → {} (czekanie {} s przy tempie {})",
                                crate::lab::dzien(ostatni_ts),
                                crate::lab::dzien(nts),
                                czekanie / 1000,
                                opis_tempa(speed)
                            ),
                        );
                    }
                } else if czekanie > 0 {
                    std::thread::sleep(Duration::from_millis(czekanie.min(50) as u64));
                }
            }
        }

        if speed > 0.0 && zrobione == 0 {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    opublikuj(
        st,
        &broker,
        &engine,
        &bufor,
        Postep {
            clock: ostatni_ts,
            speed,
            done: feed.done(),
            total,
            manual,
            elapsed_ms: start_real.elapsed().as_millis() as i64,
            start_balance: cfg.balance,
        },
    );
    Ok(format!(
        "zatrzymano na {}; saldo {:.2} $ (start {:.2} $), transakcji {}, ręcznych sygnałów {manual}",
        crate::lab::dzien(ostatni_ts),
        broker.balance,
        cfg.balance,
        broker.history.len()
    ))
}

fn zamknij_koszyk(broker: &mut SimBroker, id: u32) {
    let tickety: Vec<Ticket> = broker
        .positions()
        .iter()
        .filter(|p| p.basket == Some(id))
        .map(|p| p.ticket)
        .collect();
    for t in tickety {
        let _ = broker.close_position(t, CloseReason::BasketClose);
    }
    let oczek: Vec<Ticket> = broker
        .pendings()
        .iter()
        .filter(|o| o.basket == Some(id))
        .map(|o| o.ticket)
        .collect();
    for t in oczek {
        let _ = broker.cancel_pending(t);
    }
}

// ============================================================
//  PUBLIKACJA STANU
// ============================================================

struct Postep {
    clock: Ts,
    speed: f64,
    done: u64,
    total: u64,
    manual: u64,
    elapsed_ms: i64,
    start_balance: f64,
}

fn opublikuj(
    st: &StateHandle,
    broker: &SimBroker,
    engine: &Engine,
    msgs: &[ui::ChatMessage],
    p: Postep,
) {
    let q = broker.quote();
    let acc = broker.account();
    let pozycje: Vec<ui::Position> = broker
        .positions()
        .iter()
        .map(|x| ui::position_from_core(x, SYMBOL, x.profit_usd(&q)))
        .collect();
    let oczekujace: Vec<ui::PendingOrder> = broker
        .pendings()
        .iter()
        .map(|x| ui::pending_from_core(x, SYMBOL))
        .collect();
    let koszyki: Vec<ui::Basket> = engine
        .baskets
        .iter()
        .map(|b| ui::basket_from_core(b, SYMBOL))
        .collect();
    let zamkniete: Vec<ui::ClosedPosition> = broker
        .history
        .iter()
        .rev()
        .take(500)
        .map(|c| ui::closed_from_core(c, SYMBOL))
        .collect();

    let kwotowanie = ui::Quote {
        time_basis: None, time_utc: None,
        symbol: SYMBOL.into(),
        bid: q.bid,
        ask: q.ask,
        spread: q.spread(),
        time: q.ts,
        change: 0.0,
        change_pct: 0.0,
        day_high: q.bid,
        day_low: q.bid,
    };

    // `update_transient`: przebieg demo NIE jest stanem prawdziwego konta,
    // więc nie ma powodu, żeby wymuszał zapis `backup_memory/`.
    st.update_transient(sekcje(), |s| {
        // Szczyt i dołek doby narastają z poprzedniego kwotowania, ale ZERUJĄ
        // się na granicy doby handlowej. Bez tego przy odtwarzaniu czterech
        // miesięcy „maksimum dnia" byłoby maksimum całego przebiegu.
        // Doba liczona z offsetem 0, bo znacznik ticka jest już czasem serwera.
        let stare = s.quotes.get(SYMBOL).cloned();
        let mut k = kwotowanie.clone();
        if let Some(o) = stare {
            if day_of(o.time, 0) == day_of(q.ts, 0) {
                k.day_high = o.day_high.max(q.bid);
                k.day_low = if o.day_low > 0.0 {
                    o.day_low.min(q.bid)
                } else {
                    q.bid
                };
            }
        }
        s.quotes.insert(SYMBOL.into(), k);
        s.positions = pozycje.clone();
        s.pendings = oczekujace.clone();
        s.baskets = koszyki.clone();
        s.closed = zamkniete.clone();
        s.messages = msgs.to_vec();
        s.balance = broker.balance;
        s.stats.balance = broker.balance;
        s.stats.equity = acc.equity;
        s.stats.margin = acc.margin;
        s.stats.free_margin = acc.free_margin;
        s.stats.margin_level = if acc.margin > 0.0 {
            acc.equity / acc.margin * 100.0
        } else {
            0.0
        };
        s.stats.pnl_session = acc.equity - p.start_balance;
        s.stats.pnl_today = acc.equity - p.start_balance;
        s.stats.day_start_equity = p.start_balance;
        s.stats.peak_equity_today = s.stats.peak_equity_today.max(acc.equity);
        s.stats.drawdown_now = (s.stats.peak_equity_today - acc.equity).max(0.0);
        s.stats.max_dd_today = s.stats.max_dd_today.max(s.stats.drawdown_now);
        s.stats.messages = engine.stats.messages;
        s.stats.signals = engine.stats.signals;
        if s.stats
            .equity_curve
            .last()
            .map(|c| q.ts - c.t > 30_000)
            .unwrap_or(true)
        {
            s.stats.equity_curve.push(ui::CurvePoint {
                t: q.ts,
                v: acc.equity,
            });
            if s.stats.equity_curve.len() > 600 {
                s.stats.equity_curve.remove(0);
            }
        }

        s.connection.mt5 = "connected".into();
        s.connection.resolved_symbol = SYMBOL.into();
        s.connection.account = ui::AccountInfo {
            login: 0,
            server: "DEMO (wirtualny broker)".into(),
            broker: "CONDUIT SimBroker".into(),
            currency: "USD".into(),
            leverage: 500,
            kind: "DEMO".into(),
        };

        s.demo.running = true;
        s.demo.clock = p.clock;
        s.demo.clock_label = czas_pelny(p.clock);
        s.demo.speed = p.speed;
        s.demo.balance = broker.balance;
        s.demo.equity = acc.equity;
        s.demo.start_balance = p.start_balance;
        s.demo.ticks_done = p.done;
        s.demo.ticks_total = p.total;
        s.demo.progress = if p.total > 0 {
            (p.done as f64 / p.total as f64).min(1.0)
        } else {
            0.0
        };
        s.demo.messages = engine.stats.messages;
        s.demo.signals = engine.stats.signals;
        s.demo.manual_signals = p.manual;
        s.demo.trades = broker.history.len() as u64;
        s.demo.open_positions = pozycje.len() as u32;
        s.demo.open_pendings = oczekujace.len() as u32;
        s.demo.baskets = koszyki.iter().filter(|b| b.active).count() as u32;
        s.demo.elapsed_ms = p.elapsed_ms;
    });
}

// ============================================================
//  WIADOMOŚCI DLA INTERFEJSU
// ============================================================

/// Buduje wiadomość dla panelu — z rozbiciem na sygnały TYM SAMYM parserem,
/// którego używa silnik. Gdyby panel miał własny parser, pokazywałby coś
/// innego, niż bot wykonał.
pub fn wiadomosc_ui(
    ts: i64,
    channel_id: i64,
    channel: &str,
    text: &str,
    id: String,
) -> ui::ChatMessage {
    let parsed = parsuj(text);
    let types: Vec<String> = parsed.iter().map(|p| p.kind.clone()).collect();
    ui::ChatMessage {
        time_basis: None, received_time_utc: None,        id,
        time: ts,
        channel_id,
        channel_name: channel.to_string(),
        topic_id: None,
        topic_name: None,
        format: None,
        text: text.to_string(),
        types,
        basket_id: None,
        edited: false,
        pending_action: None,
        parsed: Some(parsed),
    }
}

/// Mapuje wynik parsera rdzenia na model interfejsu.
pub fn parsuj(text: &str) -> Vec<ui::ParsedSignal> {
    use conduit_core::parser::Signal as S;
    conduit_core::parser::parse(text)
        .into_iter()
        .map(|s| {
            let mut p = ui::ParsedSignal {
                kind: "INFO".into(),
                direction: None,
                is_limit: None,
                is_stop: None,
                entry_low: None,
                entry_high: None,
                sl: None,
                tps: None,
                tp_index: None,
                level: None,
                raw: text.to_string(),
            };
            match s {
                S::Entry(e) => {
                    p.kind = "ENTRY".into();
                    p.direction = Some(e.side.into());
                    p.is_limit = Some(e.is_limit);
                    p.is_stop = Some(e.is_stop);
                    p.entry_low = Some(e.lo);
                    p.entry_high = Some(e.hi);
                    p.sl = e.sl;
                    p.tps = Some(e.tps);
                }
                S::TpHit { index } => {
                    p.kind = "TP_HIT".into();
                    p.tp_index = index;
                }
                S::SlHit => p.kind = "SL_HIT".into(),
                S::RiskFree { level } => {
                    p.kind = "RISK_FREE".into();
                    p.level = level;
                }
                // `..` ŚWIADOMIE: PARSER dokłada do tego wariantu kolejne
                // pola (m.in. `spp_be_level`), a podgląd rozbioru nie ma
                // powodu padać przy każdym takim dołożeniu.
                S::SecuringPartial { targets, sl, .. } => {
                    p.kind = "PARTIAL".into();
                    p.tps = Some(targets);
                    p.sl = sl;
                }
                S::OutAtEntry => p.kind = "OUT_AT_ENTRY".into(),
                S::CloseAll => p.kind = "CLOSE_ALL".into(),
                S::Cancel => p.kind = "CANCEL".into(),
                S::TpCorrection { index, value } => {
                    p.kind = "TP_CORRECTION".into();
                    p.tp_index = Some(index);
                    p.tps = Some(vec![value]);
                }
                S::SetSl { value } => {
                    p.kind = "SET_SL".into();
                    p.sl = Some(value);
                }
                S::BreakEven => p.kind = "SET_SL".into(),
                // „open now" bez podanej strefy: dla panelu to nadal WEJŚCIE,
                // tylko rynkowe — stąd `isLimit = false` i brak granic strefy
                S::MarketOpen { side } => {
                    p.kind = "ENTRY".into();
                    p.direction = Some(side.into());
                    p.is_limit = Some(false);
                }
                S::Info => p.kind = "INFO".into(),
                // NOWY WARIANT `Signal`, którego ten panel jeszcze nie zna.
                //
                // Świadomie NIE mapujemy go na `INFO`: „info" znaczy
                // „wiadomość bez treści handlowej", a to jest coś wprost
                // przeciwnego — rozpoznany sygnał, którego panel nie umie
                // nazwać. Osobna etykieta sprawia, że brak widać od razu,
                // zamiast go chować wśród pogawędki z kanału.
                _ => p.kind = "UNKNOWN".into(),
            }
            p
        })
        .collect()
}

/// „RRRR-MM-DD GG:MM:SS" z milisekund epoki — do napisu z zegarem demo.
pub fn czas_pelny(ms: i64) -> String {
    let sekundy = ms.div_euclid(1000);
    let pora = sekundy.rem_euclid(86_400);
    format!(
        "{} {:02}:{:02}:{:02}",
        crate::lab::dzien(ms),
        pora / 3600,
        (pora % 3600) / 60,
        pora % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_symbol_demo_publication_and_restore_match_their_quotes() {
        let st = testowy_stan("resolved-symbol");
        st.update(sekcje(), |s| {
            s.connection.mt5 = "connected".into();
            s.connection.resolved_symbol = "XAUUSD.s".into();
            s.connection.account.server = "PUPrime-PUBLIC-DEMO".into();
        });
        let before = zapamietaj(&st);
        let mut broker = SimBroker::new(600.0, 0.2, 0.0);
        broker.on_quote(Quote { ts: 1_775_001_600_000, bid: 4000.0, ask: 4000.2 });
        let engine = Engine::new(conduit_core::Settings::default(), 600.0);
        opublikuj(&st, &broker, &engine, &[], Postep {
            clock: broker.quote().ts, speed: 1.0, done: 1, total: 1,
            manual: 0, elapsed_ms: 0, start_balance: 600.0,
        });
        st.read(|s| {
            assert_eq!(s.connection.resolved_symbol, SYMBOL);
            assert_eq!(s.quotes[SYMBOL].symbol, SYMBOL);
            assert_eq!(s.connection.account.server, "DEMO (wirtualny broker)");
        });
        przywroc(&st, &before);
        st.read(|s| {
            assert_eq!(s.connection.resolved_symbol, "XAUUSD.s");
            assert_eq!(s.connection.account.server, "PUPrime-PUBLIC-DEMO");
            assert_eq!(s.quotes, before.quotes);
        });
        let mut disconnected = before;
        disconnected.mt5 = "disconnected".into();
        przywroc(&st, &disconnected);
        assert!(st.read(|s| s.connection.resolved_symbol.is_empty()));
    }

    #[test]
    fn saldo_poza_zakresem_jest_bledem_a_nie_cicha_korekta() {
        let mut c = DemoConfig {
            price_source: PriceSource::Synthetic,
            ..Default::default()
        };
        c.balance = -1.0;
        assert!(c.validate().is_err());
        c.balance = MAX_BALANCE + 1.0;
        assert!(c.validate().is_err());
        c.balance = 0.0;
        assert!(
            c.validate().is_ok(),
            "zero jest dozwolone — to dolna granica z wymagania"
        );
        c.balance = MAX_BALANCE;
        assert!(c.validate().is_ok());
    }

    #[test]
    fn zakres_dat_musi_byc_poprawny_i_nieodwrocony() {
        let baza = DemoConfig {
            price_source: PriceSource::Synthetic,
            ..Default::default()
        };
        let c = DemoConfig {
            ticks_from: "2026-04-01".into(),
            ticks_to: "2026-04-30".into(),
            ..baza.clone()
        };
        assert!(c.validate().is_ok());
        let c = DemoConfig {
            ticks_from: "2026-04-30".into(),
            ticks_to: "2026-04-01".into(),
            ..baza.clone()
        };
        assert!(c.validate().is_err(), "odwrócony zakres musi być odrzucony");
        let c = DemoConfig {
            signals_from: "01.04.2026".into(),
            ..baza.clone()
        };
        assert!(
            c.validate().is_err(),
            "format DD.MM.RRRR nie jest RRRR-MM-DD"
        );
        // puste granice = cały plik
        assert!(baza.validate().is_ok());
    }

    #[test]
    fn okno_dat_liczy_sie_wlacznie_z_dniem_koncowym() {
        let c = DemoConfig {
            ticks_from: "2026-04-01".into(),
            ticks_to: "2026-04-01".into(),
            ..Default::default()
        };
        let (a, b) = c.okno_tickow();
        assert_eq!(b - a, 86_400_000, "dzień „do” musi wejść do okna w całości");
    }

    #[test]
    fn brak_pliku_tickow_przy_zrodle_plikowym_jest_bledem() {
        let c = DemoConfig {
            price_source: PriceSource::File,
            ticks_path: String::new(),
            ..Default::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn parser_panelu_to_ten_sam_parser_co_silnika() {
        let p = parsuj("🟢 BUY LIMITS GOLD @ 4125/4120 AREA\n🎯 TP 4127\n⛔️ SL 4119");
        let wejscie = p
            .iter()
            .find(|x| x.kind == "ENTRY")
            .expect("sygnał wejścia");
        assert_eq!(wejscie.direction, Some(ui::Direction::Buy));
        assert_eq!(wejscie.is_limit, Some(true));
        assert_eq!(wejscie.entry_low, Some(4120.0));
        assert_eq!(wejscie.entry_high, Some(4125.0));
        assert_eq!(wejscie.sl, Some(4119.0));
        assert_eq!(wejscie.tps, Some(vec![4127.0]));

        let p = parsuj("TP1 HIT +32 PIPS");
        assert!(p.iter().any(|x| x.kind == "TP_HIT"));
        let p = parsuj("OUT AT ENTRY ON THE REST");
        assert!(p.iter().any(|x| x.kind == "OUT_AT_ENTRY"));
        let p = parsuj("dzień dobry");
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].kind, "INFO");
    }

    #[test]
    fn wiadomosc_dla_panelu_niesie_rozpoznane_rodzaje() {
        // wejście BEZ ani jednego celu nie jest wejściem — parser rdzenia je
        // odrzuca, a panel musi pokazywać dokładnie to samo
        let bez_celu = wiadomosc_ui(0, -1, "PANEL", "SELL GOLD @ 4200/4205\nSL 4210", "x".into());
        assert_eq!(bez_celu.types, vec!["INFO".to_string()]);

        let m = wiadomosc_ui(
            1_775_001_600_000,
            -1,
            "PANEL",
            "SELL GOLD @ 4200/4205\nTP 4190\nSL 4210",
            "y".into(),
        );
        assert!(
            m.types.contains(&"ENTRY".to_string()),
            "rodzaje: {:?}",
            m.types
        );
        let e = m
            .parsed
            .as_ref()
            .unwrap()
            .iter()
            .find(|p| p.kind == "ENTRY")
            .unwrap();
        assert_eq!(e.direction, Some(ui::Direction::Sell));
        assert_eq!(e.sl, Some(4210.0));
    }

    #[test]
    fn sterowanie_wpuszcza_jeden_przebieg() {
        let c = DemoControl::default();
        assert!(!c.running());
        assert!(
            !c.push(DemoCmd::CloseAll),
            "bez przebiegu nie ma gdzie wysłać polecenia"
        );
        assert!(c.try_claim(Arc::new(AtomicBool::new(false))));
        assert!(c.running());
        assert!(
            !c.try_claim(Arc::new(AtomicBool::new(false))),
            "drugi przebieg nie ma prawa wejść"
        );
        assert!(c.push(DemoCmd::CloseAll));
        assert_eq!(c.drain().len(), 1);
        assert!(c.drain().is_empty(), "kolejka opróżnia się przy odbiorze");
        assert!(c.request_stop());
        c.release();
        assert!(!c.running());
        assert!(!c.request_stop());
    }

    #[test]
    fn predkosc_przezywa_zapis_i_odczyt() {
        let c = DemoControl::default();
        c.set_speed(60.0);
        assert_eq!(c.speed(), 60.0);
        c.set_speed(0.0);
        assert_eq!(
            c.speed(),
            0.0,
            "zero znaczy „maksymalne tempo”, nie „zatrzymane”"
        );
        c.set_speed(f64::NAN);
        assert_eq!(
            c.speed(),
            1.0,
            "wartość bez sensu nie może zatrzymać odtwarzania"
        );
    }

    #[test]
    fn opis_tempa_jest_czytelny() {
        assert_eq!(opis_tempa(0.0), "maksymalne");
        assert_eq!(opis_tempa(1.0), "1×");
        assert_eq!(opis_tempa(60.0), "60×");
        assert_eq!(opis_tempa(0.5), "0.50×");
    }

    #[test]
    fn zegar_demo_pokazuje_date_i_godzine() {
        assert_eq!(czas_pelny(1_775_001_600_644), "2026-04-01 00:00:00");
        assert_eq!(
            czas_pelny(1_775_001_600_644 + 3_661_000),
            "2026-04-01 01:01:01"
        );
    }

    // ========================================================
    //  RĘCZNY SYGNAŁ → SILNIK → WIRTUALNY BROKER
    // ========================================================

    /// Wiadomość z panelu, przepuszczona przez DOKŁADNIE tę samą ścieżkę, którą
    /// idzie tick demo: silnik + `SimBroker`. Test jest deterministyczny (żadnych
    /// wątków ani zegara), bo sprawdza LOGIKĘ, nie harmonogram.
    #[test]
    fn reczny_sygnal_tworzy_koszyk_u_wirtualnego_brokera() {
        use conduit_backtest::sim::SimBroker;

        let cfg = conduit_core::Settings::default();
        let mut broker = SimBroker::new(1000.0, cfg.stops_level, cfg.commission_per_lot);
        let mut engine = Engine::new(cfg, 1000.0);
        let mut feed = SynthFeed::new(SynthCfg {
            seed: 99,
            start_price: 4118.0,
            interval_ms: 250,
            start_ts: 1_775_001_600_000,
            end_ts: 1_775_001_600_000 + 600_000,
            ..Default::default()
        });

        // pierwsze kwotowanie: broker musi znać cenę, ZANIM przyjdzie wiadomość
        let q0 = feed.next().expect("generator ma dać kwotowanie");
        broker.on_quote(q0);
        assert!(broker.quote().bid > 0.0);

        engine.on_message(
            &mut broker,
            &IncomingMessage {
                ts: q0.ts,
                source: SourceKey::new(-1, None),
                source_name: "PANEL".into(),
                msg_id: 1,
                reply_to: None,
                edit_of: None,
                text: "BUY LIMITS GOLD @ 4116/4110\nTP 4125\nTP 4130\nSL 4100".into(),
            },
        );

        assert_eq!(
            engine.baskets.len(),
            1,
            "ręczny sygnał musi utworzyć koszyk"
        );
        assert_eq!(engine.stats.messages, 1);
        assert_eq!(engine.stats.signals, 1);
        assert!(
            !broker.pendings().is_empty() || !broker.positions().is_empty(),
            "koszyk bez ani jednego zlecenia u brokera to sygnał, który nigdzie nie dotarł"
        );

        // …i strumień cen dalej płynie tą samą pętlą co w demo
        let mut n = 0;
        while let Some(q) = feed.next() {
            broker.on_quote(q);
            engine.on_tick(&mut broker, &q);
            n += 1;
        }
        assert!(n > 2000, "generator powinien wydać całe okno, wydał {n}");
    }

    // ========================================================
    //  PEŁNY PRZEBIEG W TLE
    // ========================================================

    fn testowy_stan(tag: &str) -> StateHandle {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "conduit-demo-{tag}-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let cfg = crate::ServerConfig {
            workspace: dir,
            start_balance: 0.0,
            ..Default::default()
        };
        crate::bootstrap(&cfg, crate::default_auth()).expect("stan testowy")
    }

    /// Czeka na warunek, sprawdzając go co 20 ms. Zwraca `false` po upływie
    /// limitu — test ma wtedy powiedzieć CO się nie stało, a nie wisieć.
    fn czekaj(limit_ms: u64, f: impl Fn() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(limit_ms) {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        f()
    }

    /// Polecenie wstrzyknięcia z SAMYM tekstem — kontrakt zera.
    fn wstrzyk(tekst: &str) -> crate::proto::Command {
        crate::proto::Command::SimulateMessage {
            text: tekst.into(),
            channel_id: None,
            topic_id: None,
            msg_id: None,
            reply_to: None,
            edit_of: None,
        }
    }

    /// Pełna droga: `manual_signal` → skrzynka sterowania → pętla demo →
    /// silnik → `SimBroker` → stan widoczny w interfejsie.
    #[test]
    fn reczny_sygnal_z_panelu_dociera_do_pracujacego_demo() {
        let st = testowy_stan("manual");
        let cfg = DemoConfig {
            price_source: PriceSource::Synthetic,
            balance: 1000.0,
            use_file_signals: false,
            // szybko, ale nie „na maksa": pętla ma zdążyć zajrzeć do skrzynki
            speed: 100.0,
            synth_interval_ms: 250,
            synth_start_price: 4118.0,
            seed: 7,
            ..Default::default()
        };
        start(&st, cfg).expect("demo musi ruszyć");
        assert!(st.demo.running());
        assert!(
            czekaj(5_000, || st.read(|s| s.demo.ticks_done > 0)),
            "pętla nie przerobiła ani jednego kwotowania"
        );

        manual_signal(
            &st,
            &wstrzyk("BUY LIMITS GOLD @ 4116/4110\nTP 4125\nTP 4130\nSL 4100"),
        )
        .expect("ręczny sygnał ma być przyjęty");

        assert!(
            czekaj(5_000, || st
                .read(|s| s.demo.manual_signals == 1 && s.demo.baskets >= 1)),
            "sygnał nie dotarł do silnika: {:?}",
            st.read(|s| (s.demo.manual_signals, s.demo.baskets, s.demo.messages))
        );
        // wiadomość widać też w panelu, z rozbiciem parsera
        assert!(st.read(|s| s
            .messages
            .iter()
            .any(|m| m.types.contains(&"ENTRY".to_string()))));

        stop(&st).expect("zatrzymanie musi się udać");
        assert!(
            czekaj(5_000, || !st.demo.running()),
            "przebieg nie zatrzymał się"
        );

        // po zatrzymaniu wraca obraz sprzed przebiegu — żadnych wirtualnych
        // pozycji udających prawdziwe
        assert!(czekaj(2_000, || st
            .read(|s| s.positions.is_empty() && s.baskets.is_empty())));
        assert!(!st.read(|s| s.demo.running));
        assert_eq!(st.read(|s| s.demo.phase.clone()), "stopped");

        let _ = std::fs::remove_dir_all(&st.workspace.root);
    }

    /// Poza trybem demo ręczny sygnał idzie do prawdziwego środowiska — a gdy
    /// brokera nie ma, kończy się JAWNYM BŁĘDEM. Wiadomość i tak zostaje
    /// sparsowana i pokazana, bo człowiek ma zobaczyć, co parser zrozumiał.
    #[test]
    fn poza_demo_reczny_sygnal_idzie_do_srodowiska_i_nie_ginie_po_cichu() {
        let st = testowy_stan("bezdemo");
        let r = manual_signal(
            &st,
            &wstrzyk("BUY LIMITS GOLD @ 4116/4110\nTP 4125\nSL 4100"),
        );
        assert!(
            r.is_err(),
            "bez podłączonego brokera nie wolno udawać powodzenia"
        );
        assert!(format!("{:#}", r.unwrap_err()).contains("Broker"));
        let m = st.read(|s| s.messages.clone());
        assert_eq!(
            m.len(),
            1,
            "wiadomość musi być widoczna w panelu mimo błędu"
        );
        assert!(m[0].types.contains(&"ENTRY".to_string()));
        let _ = std::fs::remove_dir_all(&st.workspace.root);
    }

    #[test]
    fn dwa_przebiegi_naraz_nie_wejda() {
        let st = testowy_stan("jeden");
        let cfg = DemoConfig {
            price_source: PriceSource::Synthetic,
            use_file_signals: false,
            speed: 10.0,
            ..Default::default()
        };
        start(&st, cfg.clone()).expect("pierwszy przebieg");
        let drugi = start(&st, cfg);
        assert!(drugi.is_err(), "drugi przebieg nie ma prawa wejść");
        stop(&st).ok();
        assert!(czekaj(5_000, || !st.demo.running()));
        let _ = std::fs::remove_dir_all(&st.workspace.root);
    }

    #[test]
    fn konfiguracja_demo_przezywa_zapis_i_odczyt() {
        let st = testowy_stan("cfg");
        let cfg = DemoConfig {
            balance: 12_345.67,
            price_source: PriceSource::Synthetic,
            speed: 0.0,
            seed: 4242,
            ticks_from: "2026-04-01".into(),
            ticks_to: "2026-04-30".into(),
            msg_clock_offset_ms: Some(10_800_000),
            ..Default::default()
        };
        st.workspace.save_demo(&cfg).unwrap();
        let z_dysku = st.workspace.load_demo();
        assert_eq!(z_dysku, cfg);
        assert_eq!(z_dysku.msg_clock_offset_ms, Some(3 * 3_600_000));
        let _ = std::fs::remove_dir_all(&st.workspace.root);
    }

    /// Lista sekcji publikowanych przez demo musi pokrywać się z listą
    /// przywracaną po zatrzymaniu — inaczej po wyjściu z demo zostałaby na
    /// ekranie wirtualna pozycja udająca prawdziwą.
    #[test]
    fn demo_dotyka_wszystkich_sekcji_ktore_przywraca() {
        let s = sekcje();
        for x in [
            Section::Quotes,
            Section::Positions,
            Section::Pendings,
            Section::Baskets,
            Section::Closed,
            Section::Stats,
            Section::Messages,
            Section::Connection,
            Section::Demo,
        ] {
            assert!(s.contains(x), "sekcja {x:?} nie jest publikowana");
        }
    }
}

#[cfg(test)]
mod listener_age_tests {
    use super::*;
    #[test]
    fn demo_file_gate_uses_configured_age_and_preserves_edit_and_dedup_rules() {
        let base=1_800_000_000_000;
        let mut m=conduit_backtest::ReplayMessage {ts:base,telegram_published_ts:Some(base-600000),msg_id:7,
            text:"BUY LIMIT GOLD @ 2000/1999 SL 1990 TP 2020".into(),kanal:"Synergy".into(),..Default::default()};
        let mut im=IncomingMessage {ts:base+10800000,source:SourceKey::new(-100,None),source_name:"Synthetic".into(),
            msg_id:7,reply_to:None,edit_of:None,text:m.text.clone()};
        for (limit,passes) in [(0.,true),(5.,false),(30.,true)] {
            let mut memory=conduit_core::telegram_ingress::ContentMemory::new();
            assert_eq!(demo_file_ingress(&m,&im,&mut memory,limit),passes);
            assert!(!demo_file_ingress(&m,&im,&mut memory,limit));
        }
        m.edit_of=Some(7);im.edit_of=Some(7);
        assert!(demo_file_ingress(&m,&im,&mut conduit_core::telegram_ingress::ContentMemory::new(),5.));
        assert!(!DemoConfig::default().live_telegram_ingress,"legacy demo remains opt-in");
    }
}
