
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[path = "mailer_i18n.rs"]
pub mod i18n;

// ============================================================
//  KATEGORIE ZDARZEŃ
// ============================================================

/// Rodzaj zdarzenia. Każdy ma własny przełącznik i własne okno dławienia.
///
/// Lista pokrywa wszystko, co `bot.py` wysyłał mailem, plus to, czego mu
/// brakowało (start/stop bota jako osobna kategoria, błąd zlecenia).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MailCategory {
    /// bot wystartował albo został zatrzymany
    Lifecycle,
    /// utracono połączenie z MT5 / połączenie wróciło
    Mt5Connection,
    /// seria prób wznowienia MT5 nieudana (`bot.py`: „MT5 nadal niedostepny")
    Mt5RecoveryFailed,
    /// przekroczony limit obsunięcia kapitału (`bot.py`: EMERGENCY STOP)
    Drawdown,
    /// broker odrzucił zlecenie albo modyfikację
    OrderError,
    /// raport okresowy / dzienne podsumowanie
    Summary,
    SignalUnreadable,
    /// mail testowy z przycisku w ustawieniach — NIGDY nie dławiony
    Test,
}

impl MailCategory {
    /// Wszystkie kategorie — do pętli po przełącznikach w UI i w testach.
    pub const ALL: &'static [MailCategory] = &[
        MailCategory::Lifecycle,
        MailCategory::Mt5Connection,
        MailCategory::Mt5RecoveryFailed,
        MailCategory::Drawdown,
        MailCategory::OrderError,
        MailCategory::Summary,
        MailCategory::SignalUnreadable,
        MailCategory::Test,
    ];

    /// Klucz używany w JSON-ie ustawień i w logach.
    pub fn key(self) -> &'static str {
        match self {
            MailCategory::Lifecycle => "lifecycle",
            MailCategory::Mt5Connection => "mt5Connection",
            MailCategory::Mt5RecoveryFailed => "mt5RecoveryFailed",
            MailCategory::Drawdown => "drawdown",
            MailCategory::OrderError => "orderError",
            MailCategory::Summary => "summary",
            MailCategory::SignalUnreadable => "signalUnreadable",
            MailCategory::Test => "test",
        }
    }

    /// Etykieta po polsku — do tematu maila i do dziennika.
    pub fn label(self) -> &'static str {
        match self {
            MailCategory::Lifecycle => "start/stop bota",
            MailCategory::Mt5Connection => "połączenie z MT5",
            MailCategory::Mt5RecoveryFailed => "nieudane wznowienie MT5",
            MailCategory::Drawdown => "limit obsunięcia",
            MailCategory::OrderError => "błąd zlecenia",
            MailCategory::Summary => "podsumowanie",
            MailCategory::SignalUnreadable => "nieczytelny sygnał",
            MailCategory::Test => "test",
        }
    }

    /// Czy tę kategorię wolno dławić?
    ///
    /// Mail testowy — nigdy: użytkownik kliknął przycisk i ma prawo zobaczyć
    /// wynik natychmiast, a nie „scalimy to za 15 minut".
    pub fn throttled(self) -> bool {
        self != MailCategory::Test
    }
}

/// Przełączniki kategorii. Domyślnie włączone jest to, co ratuje pieniądze;
/// podsumowanie okresowe wymaga świadomego włączenia, bo to jedyna kategoria,
/// która sypie mailami przy całkowicie poprawnej pracy bota.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MailCategories {
    pub lifecycle: bool,
    pub mt5_connection: bool,
    pub mt5_recovery_failed: bool,
    pub drawdown: bool,
    pub order_error: bool,
    pub summary: bool,
    /// Domyślnie WŁĄCZONE — patrz `MailCategory::SignalUnreadable`. To jest
    /// jedyny sygnał, po którym da się zauważyć, że kanał zmienił zapis,
    /// zanim minie dzień bez handlu. Pole ma `#[serde(default)]` na całej
    /// strukturze, więc stary `smtp.json` wczyta się bez migracji.
    #[serde(default = "wlaczone")]
    pub signal_unreadable: bool,
}

/// Wartość domyślna dla pól, które mają być włączone także w STARYCH plikach.
/// `#[serde(default)]` na strukturze bierze `Default::default()` dla brakującego
/// pola, czyli `false` — a to dla alarmu jest złą stroną domyślną.
fn wlaczone() -> bool {
    true
}

impl Default for MailCategories {
    fn default() -> Self {
        MailCategories {
            lifecycle: true,
            mt5_connection: true,
            mt5_recovery_failed: true,
            drawdown: true,
            order_error: true,
            summary: false,
            signal_unreadable: true,
        }
    }
}

impl MailCategories {
    pub fn enabled(&self, c: MailCategory) -> bool {
        match c {
            MailCategory::Lifecycle => self.lifecycle,
            MailCategory::Mt5Connection => self.mt5_connection,
            MailCategory::Mt5RecoveryFailed => self.mt5_recovery_failed,
            MailCategory::Drawdown => self.drawdown,
            MailCategory::OrderError => self.order_error,
            MailCategory::Summary => self.summary,
            MailCategory::SignalUnreadable => self.signal_unreadable,
            // mail testowy jest zawsze dozwolony — inaczej przycisk „wyślij
            // mail testowy" milczałby dokładnie wtedy, gdy służy do diagnozy
            MailCategory::Test => true,
        }
    }
}

// ============================================================
//  POLITYKA: DŁAWIENIE I LIMIT GODZINOWY
// ============================================================

/// Decyzja polityki dla pojedynczego zgłoszenia.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// wyślij natychmiast
    Send,
    /// dołóż do zbiorczej wiadomości; wyjdzie, gdy okno się zamknie
    Coalesce { pending: u32 },
    /// kategoria wyłączona w ustawieniach
    Disabled,
    /// przekroczony limit maili na godzinę — trafia do zbiorczej
    RateLimited { pending: u32 },
}

impl Decision {
    pub fn is_send(&self) -> bool {
        matches!(self, Decision::Send)
    }
}

/// Ustawienia dławienia.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThrottleConfig {
    /// Ile minut po wysłaniu maila danej kategorii kolejne zdarzenia tej samej
    /// kategorii są SCALANE zamiast wysyłane. `bot.py` miał tu 10 minut
    /// wspólnych dla wszystkich błędów.
    pub window_min: f64,
    /// Twardy sufit liczby maili na godzinę, liczony łącznie dla wszystkich
    /// kategorii. Zabezpieczenie na wypadek, gdy sypie się kilka rzeczy naraz
    /// i każda mieści się w swoim oknie.
    pub max_per_hour: u32,
}

impl Default for ThrottleConfig {
    fn default() -> Self {
        ThrottleConfig {
            window_min: 10.0,
            max_per_hour: 12,
        }
    }
}

#[derive(Debug, Default, Clone)]
struct Stan {
    /// Kiedy ostatnio wyszedł mail tej kategorii. `None` = jeszcze nigdy.
    ///
    /// Świadomie `Option`, a nie „0 znaczy nigdy": zero jest poprawnym
    /// znacznikiem czasu (1 stycznia 1970) i wartownik z tej wartości psuł
    /// pierwsze okno dławienia w każdym teście liczonym od zera.
    last_sent: Option<i64>,
    /// ile zdarzeń czeka na doklejenie do zbiorczej
    pending: u32,
    /// treści oczekujących zdarzeń (przycinane, żeby mail nie urósł bez granic)
    lines: Vec<String>,
}

/// Ile scalonych zdarzeń wypisujemy w zbiorczym mailu. Reszta jest zliczana.
const MAX_SCALONYCH_LINII: usize = 40;

/// Logika dławienia — BEZ zegara i BEZ SMTP-a, żeby dała się przetestować.
#[derive(Debug)]
pub struct MailPolicy {
    cfg: ThrottleConfig,
    cats: MailCategories,
    stany: HashMap<MailCategory, Stan>,
    /// znaczniki wysyłek z ostatniej godziny — do limitu godzinowego
    wyslane: Vec<i64>,
}

impl MailPolicy {
    pub fn new(cfg: ThrottleConfig, cats: MailCategories) -> Self {
        MailPolicy {
            cfg,
            cats,
            stany: HashMap::new(),
            wyslane: Vec::new(),
        }
    }

    pub fn set_config(&mut self, cfg: ThrottleConfig, cats: MailCategories) {
        self.cfg = cfg;
        self.cats = cats;
    }

    fn okno_ms(&self) -> i64 {
        (self.cfg.window_min.max(0.0) * 60_000.0) as i64
    }

    /// Ile maili wyszło w ostatniej godzinie (czyści przy okazji stare wpisy).
    fn w_ostatniej_godzinie(&mut self, now: i64) -> u32 {
        self.wyslane.retain(|t| now - *t < 3_600_000);
        self.wyslane.len() as u32
    }

    /// Zgłoszenie zdarzenia. Zwraca decyzję; wysyłką zajmuje się wołający.
    pub fn offer(&mut self, cat: MailCategory, line: &str, now: i64) -> Decision {
        if !self.cats.enabled(cat) {
            return Decision::Disabled;
        }

        // Mail testowy omija WSZYSTKO — również limit godzinowy. Przycisk
        // diagnostyczny, który czasem nie działa, jest gorszy niż jego brak.
        if !cat.throttled() {
            self.zapisz_wyslanie(now);
            return Decision::Send;
        }

        let okno = self.okno_ms();
        let w_oknie = {
            let st = self.stany.entry(cat).or_default();
            st.last_sent.is_some_and(|t| now - t < okno)
        };

        if w_oknie {
            let st = self.stany.entry(cat).or_default();
            st.pending += 1;
            if st.lines.len() < MAX_SCALONYCH_LINII {
                st.lines.push(line.to_string());
            }
            return Decision::Coalesce {
                pending: st.pending,
            };
        }

        if self.cfg.max_per_hour > 0 && self.w_ostatniej_godzinie(now) >= self.cfg.max_per_hour {
            let st = self.stany.entry(cat).or_default();
            st.pending += 1;
            if st.lines.len() < MAX_SCALONYCH_LINII {
                st.lines.push(line.to_string());
            }
            return Decision::RateLimited {
                pending: st.pending,
            };
        }

        let st = self.stany.entry(cat).or_default();
        st.last_sent = Some(now);
        st.pending = 0;
        st.lines.clear();
        self.zapisz_wyslanie(now);
        Decision::Send
    }

    fn zapisz_wyslanie(&mut self, now: i64) {
        self.wyslane.push(now);
        if self.wyslane.len() > 512 {
            self.wyslane.retain(|t| now - *t < 3_600_000);
        }
    }

    /// Kategorie, których okno dławienia właśnie się zamknęło i które mają
    /// coś zaległego do wysłania. Wołane cyklicznie przez pętlę poczty.
    ///
    /// Zwraca gotowe treści zbiorcze i CZYŚCI stan — dlatego wołający musi
    /// zadbać o wysłanie tego, co dostał (albo o zakolejkowanie).
    pub fn take_due(&mut self, now: i64) -> Vec<(MailCategory, u32, String)> {
        let okno = self.okno_ms();
        let limit_ok =
            self.cfg.max_per_hour == 0 || self.w_ostatniej_godzinie(now) < self.cfg.max_per_hour;
        if !limit_ok {
            return Vec::new();
        }

        let mut gotowe: Vec<MailCategory> = self
            .stany
            .iter()
            .filter(|(_, st)| {
                st.pending > 0 && st.last_sent.map(|t| now - t >= okno).unwrap_or(true)
            })
            .map(|(c, _)| *c)
            .collect();
        // deterministyczna kolejność — inaczej testy i logi tańczą
        gotowe.sort();

        let mut out = Vec::new();
        for cat in gotowe {
            let Some(st) = self.stany.get_mut(&cat) else {
                continue;
            };
            let ile = st.pending;
            let ukryte = ile as usize - st.lines.len();
            let mut body = String::new();
            for l in &st.lines {
                body.push_str("  • ");
                body.push_str(l);
                body.push('\n');
            }
            if ukryte > 0 {
                body.push_str(&format!(
                    "  … oraz {ukryte} dalszych zdarzeń tej kategorii\n"
                ));
            }
            st.pending = 0;
            st.lines.clear();
            st.last_sent = Some(now);
            self.zapisz_wyslanie(now);
            out.push((cat, ile, body));
        }
        out
    }

    /// Ile zdarzeń czeka na scalenie (do panelu diagnostycznego).
    pub fn pending_total(&self) -> u32 {
        self.stany.values().map(|s| s.pending).sum()
    }
}

// ============================================================
//  KOLEJKA WYSYŁKI
// ============================================================

pub const MAX_PROB: u32 = 8;

/// Odstępy między próbami wysyłki (sekundy). Ostatnia wartość powtarza się.
pub const PONOWIENIA_S: &[u64] = &[15, 60, 300, 900, 1800];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedMail {
    pub id: u64,
    pub category: MailCategory,
    pub subject: String,
    pub body: String,
    /// kiedy powstała (ms epoki)
    pub created_at: i64,
    /// nie próbuj wcześniej niż o tej chwili
    pub next_try: i64,
    pub attempts: u32,
    /// ostatni komunikat błędu — trafia do dziennika, nie do maila
    #[serde(default)]
    pub last_error: String,
}

impl QueuedMail {
    /// Odstęp przed kolejną próbą po `attempts` nieudanych.
    pub fn backoff(attempts: u32) -> std::time::Duration {
        let i = (attempts.saturating_sub(1) as usize).min(PONOWIENIA_S.len() - 1);
        std::time::Duration::from_secs(PONOWIENIA_S[i])
    }
}

#[derive(Debug)]
pub struct MailQueue {
    path: Option<PathBuf>,
    items: Mutex<Vec<QueuedMail>>,
    next_id: Mutex<u64>,
}

/// Ile wiadomości trzymamy w kolejce, zanim zaczniemy odrzucać najstarsze.
pub const MAX_KOLEJKI: usize = 200;

impl MailQueue {
    pub fn new(path: Option<PathBuf>) -> Self {
        let items = path.as_ref().and_then(|p| {
            let raw = std::fs::read_to_string(p).ok()?;
            serde_json::from_str::<Vec<QueuedMail>>(&raw).ok()
        });
        let items = items.unwrap_or_default();
        let next = items.iter().map(|i| i.id + 1).max().unwrap_or(1);
        MailQueue {
            path,
            items: Mutex::new(items),
            next_id: Mutex::new(next),
        }
    }

    pub fn len(&self) -> usize {
        self.items.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn push(&self, category: MailCategory, subject: String, body: String, now: i64) -> u64 {
        let id = {
            let mut n = self.next_id.lock();
            let id = *n;
            *n += 1;
            id
        };
        {
            let mut q = self.items.lock();
            q.push(QueuedMail {
                id,
                category,
                subject,
                body,
                created_at: now,
                next_try: now,
                attempts: 0,
                last_error: String::new(),
            });
            // Przy przepełnieniu wypada NAJSTARSZA wiadomość, nie najnowsza:
            // gdy sypie się lawina, świeższy stan świata jest cenniejszy.
            while q.len() > MAX_KOLEJKI {
                q.remove(0);
            }
        }
        self.persist();
        id
    }

    /// Pierwsza wiadomość, której czas ponowienia już minął.
    pub fn next_due(&self, now: i64) -> Option<QueuedMail> {
        self.items
            .lock()
            .iter()
            .find(|i| i.next_try <= now)
            .cloned()
    }

    pub fn remove(&self, id: u64) {
        self.items.lock().retain(|i| i.id != id);
        self.persist();
    }

    /// Odkłada wiadomość na później po nieudanej próbie. Zwraca `false`, gdy
    /// próby się wyczerpały i wiadomość została porzucona.
    pub fn defer(&self, id: u64, now: i64, error: &str) -> bool {
        let zostaje = {
            let mut q = self.items.lock();
            let Some(it) = q.iter_mut().find(|i| i.id == id) else {
                return false;
            };
            it.attempts += 1;
            it.last_error = error.chars().take(300).collect();
            if it.attempts >= MAX_PROB {
                false
            } else {
                it.next_try = now + QueuedMail::backoff(it.attempts).as_millis() as i64;
                true
            }
        };
        if !zostaje {
            self.items.lock().retain(|i| i.id != id);
        }
        self.persist();
        zostaje
    }

    fn persist(&self) {
        let Some(p) = &self.path else { return };
        let items = self.items.lock().clone();
        if items.is_empty() {
            let _ = std::fs::remove_file(p);
            return;
        }
        if let Err(e) = crate::store::write_json_atomic(p, &items) {
            tracing::warn!(blad = %e, "nie udało się zapisać kolejki maili");
        }
    }
}

// ============================================================
//  TRANSPORT
// ============================================================

/// Jak zabezpieczyć połączenie z serwerem poczty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MailSecurity {
    /// port 587 — połączenie jawne, podnoszone do TLS komendą STARTTLS
    #[default]
    Starttls,
    /// port 465 — TLS od pierwszego bajtu
    Ssl,
    /// bez szyfrowania; wyłącznie dla serwera na tej samej maszynie
    None,
}

/// Dane potrzebne do wysłania maila.
#[derive(Debug, Clone, PartialEq)]
pub struct SmtpTarget {
    pub host: String,
    pub port: u16,
    pub security: MailSecurity,
    pub user: String,
    /// hasło — z `secrets.json`, NIGDY z dokumentu ustawień
    pub password: String,
    /// nadawca; puste = użyj `user`
    pub from: String,
    /// odbiorcy (po przecinku albo średniku w konfiguracji)
    pub to: Vec<String>,
}

impl SmtpTarget {
    /// Czy da się w ogóle próbować wysłać?
    pub fn is_usable(&self) -> Result<(), String> {
        if self.host.trim().is_empty() {
            return Err("brak adresu serwera SMTP".into());
        }
        if self.port == 0 {
            return Err("brak portu SMTP".into());
        }
        if self.to.is_empty() {
            return Err("brak odbiorcy".into());
        }
        // logowanie bywa niepotrzebne dla przekaźnika na localhoście,
        // ale login BEZ hasła to zawsze pomyłka w konfiguracji
        if !self.user.trim().is_empty() && self.password.is_empty() {
            return Err("podano login SMTP bez hasła".into());
        }
        if self.sender().trim().is_empty() {
            return Err("brak adresu nadawcy (pole „od kogo” i login SMTP są puste)".into());
        }
        Ok(())
    }

    pub fn sender(&self) -> String {
        if self.from.trim().is_empty() {
            self.user.clone()
        } else {
            self.from.clone()
        }
    }
}

/// Rozbija listę odbiorców zapisaną jako jeden ciąg.
///
/// Ludzie wpisują adresy rozdzielone przecinkiem, średnikiem albo spacją —
/// wszystkie trzy warianty są tu poprawne, bo alternatywą jest cicha
/// niewysyłka do drugiego adresata.
pub fn parse_recipients(raw: &str) -> Vec<String> {
    raw.split([',', ';', ' ', '\n', '\t'])
        .map(|s| s.trim())
        .filter(|s| s.contains('@'))
        .map(|s| s.to_string())
        .collect()
}

/// Wysyłka. Oddzielona cechą, żeby testy nie potrzebowały serwera poczty.
pub trait MailSender: Send + Sync {
    fn send(&self, target: &SmtpTarget, subject: &str, body: &str) -> anyhow::Result<()>;
    fn name(&self) -> &'static str {
        "smtp"
    }
}

// ============================================================
//  WŁASNY TEMAT MAILA — zmienne `${...}`
// ============================================================
//
// Użytkownik wpisuje w ustawieniach np. `Aktualny balans: ${balance}`
// i dostaje maila o temacie `Aktualny balans: $237.65`.
//
// Trzy reguły, na których stoi cała ta sekcja:
//
// 1. **Jedno źródło prawdy.** Podstawia WYŁĄCZNIE Rust. Panel nie ma własnego
//    silnika szablonów — pyta serwer o gotowy podgląd (`GET /api/email/subject`),
//    więc to, co widać w polu „podgląd", jest tym samym łańcuchem, który
//    wyjdzie w mailu. Druga implementacja w TypeScripcie rozjechałaby się przy
//    pierwszej zmianie formatu liczby.
// 2. **Nieznana zmienna zostaje dosłownie.** `${blans}` (literówka) wypisuje
//    się jako `${blans}`, a nie znika. Ciche znikanie zamienia literówkę
//    w zagadkę „czemu w temacie jest dziura".
// 3. **Wartość liczy się w chwili KOLEJKOWANIA maila**, nie wysyłki. Kolejka
//    leży na dysku i potrafi ponawiać przez godzinę (`MAX_PROB` = 8 prób);
//    temat „balans 237.65" ma opisywać chwilę zdarzenia, a nie chwilę,
//    w której w końcu wróciła sieć.

/// Jedna zmienna dostępna w temacie.
///
/// `label` jest jedynym opisem, jaki widzi użytkownik — panel nie ma osobnej
/// tabelki z objaśnieniami, tylko listę rozwijaną, w której każda pozycja
/// wygląda tak: `Balans konta: ${balance} = $237.65`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZmiennaTematu {
    /// nazwa bez `${}` — to, co użytkownik wpisuje w szablonie
    pub name: String,
    /// opis po polsku do listy rozwijanej
    pub label: String,
    /// wartość „na teraz", już sformatowana
    pub value: String,
}

/// Komplet zmiennych policzony dla jednej chwili.
///
/// Kolejność ma znaczenie: dokładnie w niej panel rysuje listę rozwijaną,
/// więc pola pieniężne stoją na górze, a techniczne na dole.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubjectVars {
    pub items: Vec<ZmiennaTematu>,
}

impl SubjectVars {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.items
            .iter()
            .find(|z| z.name == name)
            .map(|z| z.value.as_str())
    }

    fn dodaj(&mut self, name: &str, label: &str, value: impl Into<String>) {
        self.items.push(ZmiennaTematu {
            name: name.to_string(),
            label: label.to_string(),
            value: value.into(),
        });
    }
}

/// PODSTAWIANIE. Jedyna implementacja w całym programie.
///
/// Zmiennych wolno użyć dowolnie wiele i wolno je powtarzać — każde
/// wystąpienie jest podstawiane osobno, bo szablon jest przemiatany od lewej
/// do prawej, a nie „raz na nazwę".
///
/// Przypadki brzegowe rozstrzygnięte na korzyść WIDOCZNOŚCI błędu:
/// * nieznana nazwa → `${nazwa}` przepisane dosłownie,
/// * `${` bez zamykającej klamry → reszta tekstu przepisana dosłownie,
/// * pusty szablon → pusty wynik (wołający decyduje, że to znaczy
///   „użyj tematu systemowego").
pub fn render_subject(tpl: &str, vars: &SubjectVars) -> String {
    let mut out = String::with_capacity(tpl.len() + 32);
    let mut reszta = tpl;
    while let Some(start) = reszta.find("${") {
        out.push_str(&reszta[..start]);
        let po = &reszta[start + 2..];
        match po.find('}') {
            Some(koniec) => {
                let nazwa = &po[..koniec];
                match vars.get(nazwa) {
                    Some(v) => out.push_str(v),
                    None => {
                        // literówka użytkownika ma zostać WIDOCZNA
                        out.push_str("${");
                        out.push_str(nazwa);
                        out.push('}');
                    }
                }
                reszta = &po[koniec + 1..];
            }
            None => {
                // urwane `${` — przepisujemy resztę i kończymy
                out.push_str(&reszta[start..]);
                return out;
            }
        }
    }
    out.push_str(reszta);
    out
}

// ---------------- formatowanie wartości ----------------

/// Znak waluty rachunku i to, czy stoi PRZED kwotą.
///
/// Lista jest krótka celowo — to waluty, w których brokerzy CFD realnie
/// prowadzą rachunki. Dla reszty wypisujemy kod ISO za kwotą (`237.65 HUF`),
/// bo wymyślony znaczek byłby gorszy niż jawny kod.
fn znak_waluty(kod: &str) -> (String, bool) {
    match kod.trim().to_ascii_uppercase().as_str() {
        // pusty kod = konto jeszcze nie odpowiedziało; `AccountInfo::default`
        // ma „USD", więc dolar jest tu spójnym domysłem, a nie zgadywaniem
        "USD" | "" => ("$".to_string(), true),
        "EUR" => ("€".to_string(), true),
        "GBP" => ("£".to_string(), true),
        "JPY" => ("¥".to_string(), true),
        "PLN" => ("zł".to_string(), false),
        inny => (inny.to_string(), false),
    }
}

/// Kwota z dwoma miejscami i symbolem waluty rachunku: `$237.65`, `237.65 zł`.
pub fn pieniadze(v: f64, waluta: &str) -> String {
    formatuj_kwote(v, waluta, false)
}

/// To samo, ale ZAWSZE ze znakiem: `+$18.45` / `-$4.50`.
///
/// Osobna funkcja, bo dla salda znak plus jest hałasem, a dla wyniku dnia
/// jego brak jest ubytkiem informacji — `Wynik: $18.45` nie mówi nic o tym,
/// czy dzień był dodatni.
pub fn pieniadze_ze_znakiem(v: f64, waluta: &str) -> String {
    formatuj_kwote(v, waluta, true)
}

fn formatuj_kwote(v: f64, waluta: &str, zawsze_znak: bool) -> String {
    if !v.is_finite() {
        return "—".to_string();
    }
    let (sym, przedrostek) = znak_waluty(waluta);
    // Zaokrąglamy PRZED sprawdzeniem znaku: bez tego −0,004 wypisywało się
    // jako „-$0.00", czyli strata, której nie ma.
    let zaokr = (v * 100.0).round() / 100.0;
    let znak = if zaokr < 0.0 {
        "-"
    } else if zawsze_znak {
        "+"
    } else {
        ""
    };
    let liczba = format!("{:.2}", zaokr.abs());
    if przedrostek {
        format!("{znak}{sym}{liczba}")
    } else {
        format!("{znak}{liczba} {sym}")
    }
}

pub fn procent(v: f64) -> String {
    if !v.is_finite() {
        return "—".to_string();
    }
    format!("{v:.1}%")
}

/// Cena instrumentu. Liczba miejsc z rzędu wielkości, bo migawka nie niesie
/// `digits` symbolu: XAUUSD (4665,32) dostaje 2 miejsca, EURUSD (1,08421) — 5.
/// Wypisanie złota z pięcioma miejscami zaśmieca temat, a waluty z dwoma
/// gubi całą zmienność.
pub fn cena(v: f64) -> String {
    if !v.is_finite() || v <= 0.0 {
        return "—".to_string();
    }
    cena_wg(v, v)
}

/// Odległość cenowa formatowana precyzją INSTRUMENTU, nie własną.
///
/// Spread złota to 0,29 — z własnego rzędu wielkości wyszłoby „0.29000",
/// czyli zapis pary walutowej doklejony do złota. Odniesieniem jest bid.
pub fn cena_wg(v: f64, odniesienie: f64) -> String {
    if !v.is_finite() || !odniesienie.is_finite() {
        return "—".to_string();
    }
    if odniesienie >= 100.0 {
        format!("{v:.2}")
    } else {
        format!("{v:.5}")
    }
}

// ---------------- katalog zmiennych ----------------

/// Buduje komplet zmiennych z migawki stanu.
///
/// `cat` i `zdarzenie` opisują MAILA, który właśnie powstaje — dzięki nim
/// szablon `${kategoria}: ${zdarzenie}` odtwarza temat systemowy własnymi
/// słowami użytkownika. Podgląd w panelu podstawia tu przykład, bo w chwili
/// oglądania ustawień żaden mail nie powstaje.
///
/// `now_ms` jest argumentem, a nie odczytem zegara, żeby test mógł sprawdzić
/// datę bez czekania na północ.
pub fn zmienne_tematu(
    snap: &crate::ui::UiSnapshot,
    cat: MailCategory,
    zdarzenie: &str,
    now_ms: i64,
) -> SubjectVars {
    use crate::ui::Origin;

    let mut v = SubjectVars::default();
    let s = &snap.stats;
    let acc = &snap.connection.account;
    let waluta = if acc.currency.trim().is_empty() {
        "USD"
    } else {
        acc.currency.as_str()
    };

    // ---------- doba handlowa ----------
    // Ta sama definicja, co w `live.rs`: doba liczona w zegarze SERWERA
    // BROKERA (`server_tz_offset_h`), nie maszyny. Przy offsecie +3 h doba
    // kalendarzowa Windows przestawiałaby liczniki trzy godziny za wcześnie.
    let offset_ms = snap
        .settings
        .get("server_tz_offset_h")
        .and_then(|x| x.as_f64())
        .map(|h| (h * 3_600_000.0) as i64)
        .unwrap_or(0);
    // `day_key == 0` znaczy „broker jeszcze się nie odezwał" — wtedy bierzemy
    // dobę z zegara, żeby liczniki nie pokazywały wszystkiego jako „dzisiaj".
    let doba = if s.day_key != 0 {
        s.day_key
    } else {
        (now_ms + offset_ms).div_euclid(86_400_000)
    };
    let dzis = |ts: i64| (ts + offset_ms).div_euclid(86_400_000) == doba;

    // ---------- transakcje bota z dzisiaj ----------
    // Tylko `Origin::Bot`: na rachunku bywają pozycje z terminala i ze starego
    // automatu, a „skuteczność bota" policzona z cudzych transakcji byłaby
    // liczbą bez znaczenia.
    let mut ile = 0u32;
    let mut wygrane = 0u32;
    let mut zysk_brutto = 0.0f64;
    let mut strata_brutto = 0.0f64;
    let mut zrealizowany = 0.0f64;
    let mut net_complete = true;
    for c in snap
        .closed
        .iter()
        .filter(|c| c.source == Origin::Bot && dzis(c.close_time))
    {
        ile += 1;
        let Some(netto) = c.net_result() else { net_complete = false; continue; };
        zrealizowany += netto;
        if netto > 0.0 {
            wygrane += 1;
            zysk_brutto += netto;
        } else {
            strata_brutto += -netto;
        }
    }
    let skutecznosc = if ile > 0 {
        wygrane as f64 / ile as f64 * 100.0
    } else {
        0.0
    };
    let pf = if !net_complete { "—".to_string() } else if strata_brutto > 0.0 {
        format!("{:.2}", zysk_brutto / strata_brutto)
    } else if zysk_brutto > 0.0 {
        // Dzień bez ani jednej straty. „inf" w temacie maila wygląda jak
        // awaria, więc mówimy to słowem.
        "bez strat".to_string()
    } else {
        "—".to_string()
    };

    // ---------- pozycje, zlecenia, koszyki ----------
    let poz_bota: Vec<_> = snap
        .positions
        .iter()
        .filter(|p| p.source == Origin::Bot)
        .collect();
    let plywajacy: f64 = poz_bota.iter().map(|p| p.profit).sum();
    let zlec_bota = snap
        .pendings
        .iter()
        .filter(|p| p.source == Origin::Bot)
        .count();
    let koszyki = snap.baskets.iter().filter(|b| b.active).count();

    // ---------- instrument ----------
    // Bot gra jednym symbolem naraz, ale `quotes` po restarcie potrafi mieć
    // wpis sprzed przełączenia. Pierwszeństwo ma to, co bot AKTUALNIE trzyma.
    let symbol = poz_bota
        .first()
        .map(|p| p.symbol.clone())
        .or_else(|| {
            snap.pendings
                .iter()
                .find(|p| p.source == Origin::Bot)
                .map(|p| p.symbol.clone())
        })
        .or_else(|| {
            snap.baskets
                .iter()
                .find(|b| b.active)
                .map(|b| b.symbol.clone())
        })
        .or_else(|| snap.quotes.keys().next().cloned())
        .unwrap_or_else(|| "—".to_string());
    let kwot = snap.quotes.get(&symbol);

    // ---------- czas ----------
    let (data, godzina) = data_i_godzina(now_ms);

    // ---------- PIENIĄDZE ----------
    v.dodaj("balance", "Balans konta", pieniadze(s.balance, waluta));
    v.dodaj("equity", "Kapitał (equity)", pieniadze(s.equity, waluta));
    v.dodaj("margines", "Margines użyty", pieniadze(s.margin, waluta));
    v.dodaj(
        "wolny_margines",
        "Wolny margines",
        pieniadze(s.free_margin, waluta),
    );
    v.dodaj("poziom_marginu", "Poziom marginu", procent(s.margin_level));
    v.dodaj(
        "zysk_dzis",
        "Zysk ZREALIZOWANY dzisiaj",
        if net_complete { pieniadze_ze_znakiem(zrealizowany, waluta) } else { "—".into() },
    );
    v.dodaj(
        "wynik_dzis",
        "Wynik dnia razem z pozycjami otwartymi",
        pieniadze_ze_znakiem(s.pnl_today, waluta),
    );
    v.dodaj(
        "zysk_plywajacy",
        "Wynik pływający otwartych pozycji",
        pieniadze_ze_znakiem(plywajacy, waluta),
    );
    v.dodaj(
        "obsuniecie",
        "Obsunięcie dnia w dolarach",
        pieniadze(s.max_dd_today, waluta),
    );
    // Mianownikiem jest SZCZYT dnia, nie saldo startowe — obsunięcie mierzy się
    // od wierzchołka, bo tyle realnie „zjadł" rynek z najlepszego stanu konta.
    let dd_pct = if s.peak_equity_today > 0.0 {
        s.max_dd_today / s.peak_equity_today * 100.0
    } else {
        0.0
    };
    v.dodaj(
        "obsuniecie_pct",
        "Obsunięcie dnia w procentach",
        procent(dd_pct),
    );

    // ---------- LICZNIKI ----------
    v.dodaj(
        "pozycje",
        "Otwarte pozycje bota",
        poz_bota.len().to_string(),
    );
    v.dodaj(
        "zlecenia",
        "Zlecenia oczekujące bota",
        zlec_bota.to_string(),
    );
    v.dodaj("koszyki", "Aktywne koszyki", koszyki.to_string());
    v.dodaj(
        "transakcje",
        "Transakcje zamknięte dzisiaj",
        ile.to_string(),
    );
    v.dodaj("skutecznosc", "Skuteczność dzisiaj", if net_complete { procent(skutecznosc) } else { "—".into() });
    v.dodaj("profit_factor", "Profit factor dzisiaj", pf);
    v.dodaj(
        "sygnaly",
        "Rozpoznane sygnały od startu",
        s.signals.to_string(),
    );

    // ---------- RYNEK ----------
    v.dodaj("symbol", "Instrument", symbol.clone());
    v.dodaj(
        "bid",
        "Cena bid",
        kwot.map(|q| cena(q.bid)).unwrap_or_else(|| "—".into()),
    );
    v.dodaj(
        "ask",
        "Cena ask",
        kwot.map(|q| cena(q.ask)).unwrap_or_else(|| "—".into()),
    );
    v.dodaj(
        "spread",
        "Spread",
        kwot.map(|q| cena_wg(q.spread, q.bid))
            .unwrap_or_else(|| "—".into()),
    );

    // ---------- RACHUNEK ----------
    v.dodaj(
        "user",
        "Użytkownik Telegrama",
        nazwa_uzytkownika(&snap.connection.user),
    );
    v.dodaj(
        "login",
        "Numer rachunku",
        if acc.login != 0 {
            acc.login.to_string()
        } else {
            "—".into()
        },
    );
    v.dodaj("broker", "Nazwa brokera", pusty_na_kreske(&acc.broker));
    v.dodaj("serwer", "Serwer brokera", pusty_na_kreske(&acc.server));
    v.dodaj("waluta", "Waluta rachunku", waluta.to_string());
    v.dodaj(
        "dzwignia",
        "Dźwignia",
        if acc.leverage > 0 {
            format!("1:{}", acc.leverage)
        } else {
            "—".into()
        },
    );
    v.dodaj(
        "konto",
        "Rodzaj rachunku (DEMO/REAL)",
        pusty_na_kreske(&acc.kind),
    );

    // ---------- KONFIGURACJA ----------
    // Pusta etykieta presetu NIE znaczy „brak konfiguracji" — znaczy, że ktoś
    // zmienił pojedyncze ustawienie ręcznie (`apply_settings_patch` czyści
    // `preset_id`). Mówimy to wprost, zamiast wypisywać pustkę.
    v.dodaj(
        "preset",
        "Nazwa presetu",
        if snap.preset_id.trim().is_empty() {
            "(ustawienia własne)".into()
        } else {
            snap.preset_id.clone()
        },
    );
    v.dodaj(
        "tryb",
        "Tryb pracy bota",
        format!("{:?}", snap.mode).to_uppercase(),
    );

    // ---------- MAIL ----------
    v.dodaj("kategoria", "Kategoria zdarzenia", cat.label().to_string());
    v.dodaj(
        "zdarzenie",
        "Systemowy temat zdarzenia",
        zdarzenie.to_string(),
    );

    // ---------- CZAS ----------
    v.dodaj("data", "Data (czas lokalny)", data);
    v.dodaj("godzina", "Godzina (czas lokalny)", godzina);

    v
}

fn pusty_na_kreske(s: &str) -> String {
    if s.trim().is_empty() {
        "—".to_string()
    } else {
        s.to_string()
    }
}

/// Nazwa użytkownika Telegrama: imię, a jak go nie ma — `@handle`.
///
/// Konta bez ustawionego imienia istnieją i nie są rzadkie; podstawienie
/// pustego łańcucha dawałoby temat „Raport dla : +18.45 $".
fn nazwa_uzytkownika(u: &crate::ui::UserInfo) -> String {
    if !u.name.trim().is_empty() {
        return u.name.clone();
    }
    if !u.handle.trim().is_empty() {
        let h = u.handle.trim();
        return if h.starts_with('@') {
            h.to_string()
        } else {
            format!("@{h}")
        };
    }
    "—".to_string()
}

/// Data i godzina w czasie LOKALNYM maszyny — tym samym, którym `lib.rs`
/// stempluje wznowienie stanu. Mail czyta człowiek, a nie serwer brokera.
fn data_i_godzina(ms: i64) -> (String, String) {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(ms).single() {
        Some(t) => (
            t.format("%Y-%m-%d").to_string(),
            t.format("%H:%M:%S").to_string(),
        ),
        None => ("—".to_string(), "—".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn polityka() -> MailPolicy {
        MailPolicy::new(
            ThrottleConfig {
                window_min: 10.0,
                max_per_hour: 12,
            },
            MailCategories {
                summary: true,
                ..Default::default()
            },
        )
    }

    #[test]
    fn pierwsze_zdarzenie_idzie_od_razu() {
        let mut p = polityka();
        assert_eq!(
            p.offer(MailCategory::Mt5Connection, "utrata", 0),
            Decision::Send
        );
    }

    #[test]
    fn powtorka_w_oknie_jest_scalana_a_nie_gubiona() {
        let mut p = polityka();
        assert!(p
            .offer(MailCategory::Mt5Connection, "utrata 1", 0)
            .is_send());
        // cztery kolejne w ciągu okna
        for i in 1..=4 {
            let d = p.offer(MailCategory::Mt5Connection, &format!("utrata {i}"), i * MIN);
            assert_eq!(d, Decision::Coalesce { pending: i as u32 });
        }
        assert_eq!(p.pending_total(), 4, "żadne zdarzenie nie może zniknąć");

        // przed zamknięciem okna nic nie wychodzi
        assert!(p.take_due(5 * MIN).is_empty());

        // po zamknięciu wychodzi JEDEN mail z czterema zdarzeniami
        let due = p.take_due(11 * MIN);
        assert_eq!(due.len(), 1);
        let (cat, ile, body) = &due[0];
        assert_eq!(*cat, MailCategory::Mt5Connection);
        assert_eq!(*ile, 4);
        assert!(body.contains("utrata 1"), "{body}");
        assert!(body.contains("utrata 4"), "{body}");
        assert_eq!(p.pending_total(), 0);
    }

    #[test]
    fn dlawienie_jest_osobne_dla_kazdej_kategorii() {
        // REGRESJA wobec bot.py: tam JEDEN licznik dla wszystkich błędów
        // powodował, że alert o obsunięciu ginął, bo 9 minut wcześniej
        // poszedł zupełnie niezwiązany alert o MT5.
        let mut p = polityka();
        assert!(p.offer(MailCategory::Mt5Connection, "mt5", 0).is_send());
        assert!(
            p.offer(MailCategory::Drawdown, "obsunięcie 40%", MIN)
                .is_send(),
            "inna kategoria nie może być dławiona cudzym oknem"
        );
        assert!(p
            .offer(MailCategory::OrderError, "invalid stops", 2 * MIN)
            .is_send());
    }

    #[test]
    fn kategoria_wylaczona_nie_wysyla_nic() {
        let mut p = MailPolicy::new(
            ThrottleConfig::default(),
            MailCategories {
                drawdown: false,
                ..Default::default()
            },
        );
        assert_eq!(p.offer(MailCategory::Drawdown, "x", 0), Decision::Disabled);
        assert_eq!(
            p.pending_total(),
            0,
            "wyłączone zdarzenie nie może się kumulować"
        );
        // a sąsiednia kategoria działa dalej
        assert!(p.offer(MailCategory::OrderError, "x", 0).is_send());
    }

    #[test]
    fn limit_godzinowy_zatrzymuje_lawine_ale_nie_gubi_tresci() {
        let mut p = MailPolicy::new(
            ThrottleConfig {
                window_min: 0.0,
                max_per_hour: 3,
            },
            MailCategories {
                summary: true,
                ..Default::default()
            },
        );
        // okno zerowe = każde zdarzenie kwalifikuje się do wysyłki,
        // więc jedynym hamulcem jest limit godzinowy
        for i in 0..3 {
            assert!(
                p.offer(MailCategory::OrderError, "błąd", i * 1000)
                    .is_send(),
                "i={i}"
            );
        }
        let d = p.offer(MailCategory::OrderError, "błąd 4", 4000);
        assert!(matches!(d, Decision::RateLimited { .. }), "{d:?}");
        assert_eq!(
            p.pending_total(),
            1,
            "nadmiarowe zdarzenie czeka, a nie znika"
        );

        // po godzinie limit się odnawia i zaległość wychodzi
        let due = p.take_due(3_600_001);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].1, 1);
    }

    #[test]
    fn mail_testowy_omija_dlawienie_i_limit() {
        let mut p = MailPolicy::new(
            ThrottleConfig {
                window_min: 60.0,
                max_per_hour: 1,
            },
            MailCategories::default(),
        );
        assert!(p.offer(MailCategory::Test, "test", 0).is_send());
        assert!(
            p.offer(MailCategory::Test, "test", 1000).is_send(),
            "drugi test też ma wyjść"
        );
        assert!(p.offer(MailCategory::Test, "test", 2000).is_send());
    }

    #[test]
    fn zbiorczy_mail_przycina_liste_ale_podaje_pelna_liczbe() {
        let mut p = polityka();
        assert!(p.offer(MailCategory::OrderError, "start", 0).is_send());
        for i in 1..=100 {
            p.offer(MailCategory::OrderError, &format!("błąd {i}"), i * 1000);
        }
        let due = p.take_due(11 * MIN);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].1, 100, "licznik musi znać wszystkie zdarzenia");
        assert!(due[0].2.contains("dalszych zdarzeń"), "{}", due[0].2);
        assert!(due[0].2.lines().count() <= MAX_SCALONYCH_LINII + 1);
    }

    #[test]
    fn take_due_nie_zwraca_kategorii_bez_zaleglosci() {
        let mut p = polityka();
        assert!(p.offer(MailCategory::Lifecycle, "start", 0).is_send());
        assert!(
            p.take_due(60 * MIN).is_empty(),
            "sam upływ czasu nie tworzy maila"
        );
    }

    // ---------------- kolejka ----------------

    fn tmp(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "conduit-mail-{tag}-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        p
    }

    #[test]
    fn kolejka_ponawia_z_rosnacym_odstepem() {
        let q = MailQueue::new(None);
        let id = q.push(
            MailCategory::Mt5Connection,
            "temat".into(),
            "treść".into(),
            0,
        );
        assert_eq!(q.len(), 1);
        assert!(q.next_due(0).is_some());

        assert!(q.defer(id, 0, "sieć nie działa"));
        // po pierwszej porażce następna próba jest ODŁOŻONA
        assert!(
            q.next_due(0).is_none(),
            "ponowienie natychmiast to pętla, nie ponowienie"
        );
        assert!(q.next_due(15_000).is_some());

        assert!(q.defer(id, 15_000, "nadal nie"));
        assert!(
            q.next_due(20_000).is_none(),
            "drugi odstęp musi być dłuższy"
        );
        assert!(q.next_due(15_000 + 60_000).is_some());
    }

    #[test]
    fn kolejka_porzuca_wiadomosc_po_wyczerpaniu_prob() {
        let q = MailQueue::new(None);
        let id = q.push(MailCategory::OrderError, "t".into(), "b".into(), 0);
        for i in 1..MAX_PROB {
            assert!(
                q.defer(id, i as i64 * 1000, "błąd"),
                "próba {i} nie może być ostatnia"
            );
        }
        assert!(
            !q.defer(id, 99_000, "błąd"),
            "po {MAX_PROB} próbach odpuszczamy"
        );
        assert!(q.is_empty());
    }

    #[test]
    fn kolejka_przezywa_restart_procesu() {
        let p = tmp("restart");
        {
            let q = MailQueue::new(Some(p.clone()));
            q.push(
                MailCategory::Mt5Connection,
                "utrata MT5".into(),
                "treść".into(),
                1000,
            );
        }
        let q2 = MailQueue::new(Some(p.clone()));
        assert_eq!(q2.len(), 1);
        let m = q2.next_due(1000).unwrap();
        assert_eq!(m.subject, "utrata MT5");
        // a po dostarczeniu plik znika
        q2.remove(m.id);
        assert!(!p.exists());
    }

    #[test]
    fn identyfikatory_nie_powtarzaja_sie_po_wczytaniu() {
        let p = tmp("ident");
        {
            let q = MailQueue::new(Some(p.clone()));
            q.push(MailCategory::Test, "a".into(), String::new(), 0);
            q.push(MailCategory::Test, "b".into(), String::new(), 0);
        }
        let q2 = MailQueue::new(Some(p.clone()));
        let id = q2.push(MailCategory::Test, "c".into(), String::new(), 0);
        assert_eq!(id, 3, "nowy wpis nie może dostać zajętego numeru");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn przepelniona_kolejka_gubi_najstarsze() {
        let q = MailQueue::new(None);
        for i in 0..(MAX_KOLEJKI + 10) {
            q.push(
                MailCategory::OrderError,
                format!("temat {i}"),
                String::new(),
                i as i64,
            );
        }
        assert_eq!(q.len(), MAX_KOLEJKI);
        let pierwszy = q.next_due(i64::MAX).unwrap();
        assert_eq!(
            pierwszy.subject, "temat 10",
            "wypadły najstarsze, nie najnowsze"
        );
    }

    // ---------------- adresaci ----------------

    #[test]
    fn odbiorcy_rozdzielaja_sie_przecinkiem_srednikiem_i_spacja() {
        assert_eq!(
            parse_recipients("a@example.com, b@example.com;c@example.com d@example.com"),
            vec!["a@example.com", "b@example.com", "c@example.com", "d@example.com"]
        );
        // śmieci bez małpy odpadają, zamiast wywracać całą wysyłkę
        assert_eq!(parse_recipients("a@example.com, , nieadres"), vec!["a@example.com"]);
        assert!(parse_recipients("").is_empty());
    }

    #[test]
    fn konfiguracja_bez_odbiorcy_jest_odrzucana_zanim_dotkniemy_sieci() {
        let t = SmtpTarget {
            host: "smtp.example.com".into(),
            port: 587,
            security: MailSecurity::Starttls,
            user: "sender@example.com".into(),
            password: "x".into(),
            from: String::new(),
            to: Vec::new(),
        };
        assert!(t.is_usable().is_err());

        let t2 = SmtpTarget {
            to: vec!["recipient@example.com".into()],
            ..t.clone()
        };
        assert!(t2.is_usable().is_ok());
        assert_eq!(t2.sender(), "sender@example.com", "puste `from` = adres logowania");

        let t3 = SmtpTarget {
            password: String::new(),
            ..t2.clone()
        };
        assert!(
            t3.is_usable().is_err(),
            "login bez hasła to pomyłka, nie konfiguracja"
        );

        let t4 = SmtpTarget {
            host: "  ".into(),
            ..t2
        };
        assert!(t4.is_usable().is_err());
    }

    // ---------------- własny temat maila ----------------

    fn vars() -> SubjectVars {
        let mut v = SubjectVars::default();
        v.dodaj("balance", "Balans konta", "$237.65");
        v.dodaj("equity", "Kapitał", "$241.02");
        v.dodaj("user", "Użytkownik Telegrama", "Demo User");
        v
    }

    #[test]
    fn podstawia_zmienna_w_srodku_zdania() {
        assert_eq!(
            render_subject("Aktualny balans: ${balance}", &vars()),
            "Aktualny balans: $237.65"
        );
    }

    #[test]
    fn ta_sama_zmienna_moze_wystapic_wielokrotnie() {
        assert_eq!(
            render_subject("${balance} → ${balance} (${user}/${user})", &vars()),
            "$237.65 → $237.65 (Demo User/Demo User)"
        );
    }

    #[test]
    fn nieznana_zmienna_zostaje_doslownie() {
        // TO JEST SEDNO: literówka ma być WIDOCZNA. Ciche znikanie zamienia
        // pomyłkę w zagadkę „czemu w temacie jest dziura".
        assert_eq!(
            render_subject("Saldo ${blans} i ${balance}", &vars()),
            "Saldo ${blans} i $237.65"
        );
        assert_eq!(render_subject("${}", &vars()), "${}");
    }

    #[test]
    fn pusty_szablon_daje_pusty_wynik() {
        // Wołający (Notifier) czyta z tego „użyj tematu systemowego".
        assert_eq!(render_subject("", &vars()), "");
    }

    #[test]
    fn tekst_bez_zmiennych_przechodzi_bez_zmian() {
        assert_eq!(render_subject("Alarm z bota", &vars()), "Alarm z bota");
        // sam dolar bez klamry to zwykły znak, nie początek zmiennej
        assert_eq!(
            render_subject("koszt 5$ za sztukę", &vars()),
            "koszt 5$ za sztukę"
        );
    }

    #[test]
    fn urwana_klamra_nie_gubi_reszty_tekstu() {
        assert_eq!(
            render_subject("saldo ${balance i koniec", &vars()),
            "saldo ${balance i koniec"
        );
        assert_eq!(render_subject("${balance} ${equ", &vars()), "$237.65 ${equ");
    }

    #[test]
    fn polskie_znaki_i_emoji_nie_rozwalaja_ciecia() {
        // `find` zwraca indeks BAJTOWY; cięcie po nim na tekście z ogonkami
        // to klasyczne miejsce na panikę „byte index is not a char boundary".
        assert_eq!(
            render_subject("Zażółć ${user} 📈 gęślą jaźń ${balance}", &vars()),
            "Zażółć Demo User 📈 gęślą jaźń $237.65"
        );
    }

    #[test]
    fn kwoty_maja_dwa_miejsca_i_symbol_waluty_rachunku() {
        assert_eq!(pieniadze(237.6543, "USD"), "$237.65");
        assert_eq!(pieniadze(237.6543, "EUR"), "€237.65");
        assert_eq!(pieniadze(1234.5, "PLN"), "1234.50 zł");
        // waluta spoza krótkiej listy → kod ISO za kwotą, zamiast wymyślonego znaczka
        assert_eq!(pieniadze(10.0, "HUF"), "10.00 HUF");
        // brak odpowiedzi rachunku → dolar (tyle mówi `AccountInfo::default`)
        assert_eq!(pieniadze(1.0, ""), "$1.00");
    }

    #[test]
    fn wynik_dnia_zawsze_ze_znakiem_a_minus_zero_nie_udaje_straty() {
        assert_eq!(pieniadze_ze_znakiem(18.45, "USD"), "+$18.45");
        assert_eq!(pieniadze_ze_znakiem(-4.5, "USD"), "-$4.50");
        assert_eq!(pieniadze_ze_znakiem(0.0, "USD"), "+$0.00");
        // −0,004 $ to zero po zaokrągleniu; „-$0.00" wyglądałoby na stratę
        assert_eq!(pieniadze_ze_znakiem(-0.004, "USD"), "+$0.00");
    }

    #[test]
    fn procenty_maja_jedno_miejsce_a_ceny_zaleza_od_rzedu_wielkosci() {
        assert_eq!(procent(4.2499), "4.2%");
        assert_eq!(procent(1943.71), "1943.7%");
        // złoto 2 miejsca, waluty 5 — migawka nie niesie `digits` symbolu
        assert_eq!(cena(4665.3211), "4665.32");
        assert_eq!(cena(1.084213), "1.08421");
        assert_eq!(cena(0.0), "—");
    }

    // ---------------- katalog zmiennych z migawki ----------------

    fn migawka() -> crate::ui::UiSnapshot {
        use crate::ui::*;
        let mut s = UiSnapshot::empty(0);
        s.connection.account = AccountInfo {
            login: 10_000_001,
            server: "VantageInternational-Demo".into(),
            broker: "Vantage Global Prime LLP".into(),
            currency: "USD".into(),
            leverage: 500,
            kind: "DEMO".into(),
        };
        s.connection.user = UserInfo {
            name: "Demo User".into(),
            handle: "demo_user".into(),
            phone: String::new(),
        };
        s.stats.balance = 237.6543;
        s.stats.equity = 241.02;
        s.stats.margin = 12.4;
        s.stats.free_margin = 228.62;
        s.stats.margin_level = 1943.7;
        s.stats.pnl_today = 41.02;
        s.stats.max_dd_today = 10.0;
        s.stats.peak_equity_today = 250.0;
        s.stats.signals = 14;
        s.stats.day_key = 20_664;
        s.preset_id = "ULTRA-X3".into();
        s.quotes.insert(
            "XAUUSD".into(),
            Quote {
                symbol: "XAUUSD".into(),
                bid: 4665.32,
                ask: 4665.61,
                spread: 0.29,
                time: 0,
                change_pct: 0.0,
                change: 0.0,
                day_high: 0.0,
                day_low: 0.0,
            },
        );
        s
    }

    fn zamknieta(
        profit: f64,
        close_time: i64,
        source: crate::ui::Origin,
    ) -> crate::ui::ClosedPosition {
        crate::ui::ClosedPosition {
            ticket: 1,
            symbol: "XAUUSD".into(),
            direction: crate::ui::Direction::Buy,
            volume: 0.01,
            open_price: 4660.0,
            close_price: 4665.0,
            open_time: 0,
            close_time,
            profit,
            profit_basis: crate::ui::ClosedProfitBasis::PriceOnlyGross,
            net_profit: Some(profit),
            swap: 0.0,
            commission: 0.0,
            reason: crate::ui::CloseReason::Tp,
            comment: String::new(),
            basket_id: None,
            source,
            magic: None,
        }
    }

    #[test]
    fn katalog_wypelnia_sie_z_migawki() {
        let snap = migawka();
        let v = zmienne_tematu(&snap, MailCategory::Drawdown, "MAX DD 40%", 0);

        assert_eq!(v.get("balance"), Some("$237.65"));
        assert_eq!(v.get("equity"), Some("$241.02"));
        assert_eq!(v.get("wolny_margines"), Some("$228.62"));
        assert_eq!(v.get("poziom_marginu"), Some("1943.7%"));
        assert_eq!(v.get("user"), Some("Demo User"));
        assert_eq!(v.get("login"), Some("10000001"));
        assert_eq!(v.get("broker"), Some("Vantage Global Prime LLP"));
        assert_eq!(v.get("dzwignia"), Some("1:500"));
        assert_eq!(v.get("preset"), Some("ULTRA-X3"));
        assert_eq!(v.get("tryb"), Some("AUTO"));
        assert_eq!(v.get("symbol"), Some("XAUUSD"));
        assert_eq!(v.get("bid"), Some("4665.32"));
        assert_eq!(v.get("spread"), Some("0.29"));
        assert_eq!(v.get("wynik_dzis"), Some("+$41.02"));
        assert_eq!(v.get("obsuniecie"), Some("$10.00"));
        // 10 z 250 na szczycie dnia = 4,0 %
        assert_eq!(v.get("obsuniecie_pct"), Some("4.0%"));
        assert_eq!(v.get("kategoria"), Some("limit obsunięcia"));
        assert_eq!(v.get("zdarzenie"), Some("MAX DD 40%"));

        // KAŻDA zmienna ma opis — panel nie ma innej listy objaśnień
        for z in &v.items {
            assert!(
                !z.label.trim().is_empty(),
                "zmienna ${{{}}} bez opisu",
                z.name
            );
            assert!(!z.value.is_empty(), "zmienna ${{{}}} bez wartości", z.name);
        }
        assert!(
            v.items.len() >= 20,
            "katalog ma mieć kilkanaście pozycji, ma {}",
            v.items.len()
        );
    }

    #[test]
    fn liczniki_dnia_biora_tylko_transakcje_bota_z_dzisiaj() {
        use crate::ui::Origin;
        let mut snap = migawka();
        // doba 20 664 w zegarze serwera; offset zostaje domyślny (0 h)
        let dzis = 20_664 * 86_400_000 + 3_600_000;
        let wczoraj = dzis - 86_400_000;
        snap.closed = vec![
            zamknieta(12.0, dzis, Origin::Bot),
            zamknieta(-4.0, dzis, Origin::Bot),
            zamknieta(6.0, dzis, Origin::Bot),
            // cudza transakcja z dzisiaj — NIE nasza skuteczność
            zamknieta(500.0, dzis, Origin::External),
            // nasza, ale wczorajsza
            zamknieta(99.0, wczoraj, Origin::Bot),
        ];
        let v = zmienne_tematu(&snap, MailCategory::Summary, "raport", dzis);

        assert_eq!(v.get("transakcje"), Some("3"));
        assert_eq!(
            v.get("zysk_dzis"),
            Some("+$14.00"),
            "12 − 4 + 6, bez cudzych i bez wczoraj"
        );
        // 2 z 3 na plusie
        assert_eq!(v.get("skutecznosc"), Some("66.7%"));
        // (12 + 6) / 4
        assert_eq!(v.get("profit_factor"), Some("4.50"));
    }

    #[test]
    fn mail_net_uses_explicit_basis_once_and_propagates_unknown() {
        use crate::ui::{Origin, ClosedProfitBasis};
        let mut snap = migawka();
        let dzis = 20_664 * 86_400_000;
        let mut gross = zamknieta(20.0, dzis, Origin::Bot);
        gross.swap = -3.0; gross.commission = -2.0;
        let mut sim = gross.clone(); sim.profit = 17.0; sim.profit_basis = ClosedProfitBasis::PricePlusSwap;
        let mut canonical = gross.clone(); canonical.profit = 14.0;
        canonical.profit_basis = ClosedProfitBasis::CanonicalClosedNetV1; canonical.net_profit = Some(14.0);
        snap.closed = vec![gross, sim, canonical];
        let v = zmienne_tematu(&snap, MailCategory::Summary, "report", dzis);
        assert_eq!(v.get("zysk_dzis"), Some("+$44.00"));
        snap.closed[0].profit_basis = ClosedProfitBasis::Unknown;
        let v = zmienne_tematu(&snap, MailCategory::Summary, "report", dzis);
        assert_eq!(v.get("zysk_dzis"), Some("—"));
        assert_eq!(v.get("skutecznosc"), Some("—"));
        assert_eq!(v.get("profit_factor"), Some("—"));
        assert_eq!(v.get("transakcje"), Some("3"));
    }

    #[test]
    fn dzien_bez_strat_mowi_to_slowem_zamiast_pokazywac_nieskonczonosc() {
        use crate::ui::Origin;
        let mut snap = migawka();
        let dzis = 20_664 * 86_400_000;
        snap.closed = vec![zamknieta(3.0, dzis, Origin::Bot)];
        let v = zmienne_tematu(&snap, MailCategory::Summary, "raport", dzis);
        assert_eq!(v.get("profit_factor"), Some("bez strat"));

        // a dzień bez żadnej transakcji nie udaje zera skuteczności bez treści
        let pusty = zmienne_tematu(&migawka(), MailCategory::Summary, "raport", dzis);
        assert_eq!(pusty.get("transakcje"), Some("0"));
        assert_eq!(pusty.get("profit_factor"), Some("—"));
    }

    #[test]
    fn brak_danych_daje_kreske_a_nie_pustke_ani_zera() {
        // Świeży start: broker milczy, Telegram niezalogowany. Temat
        // „Rachunek 0 u : 0.00" byłby gorszy niż jawne „—".
        let snap = crate::ui::UiSnapshot::empty(0);
        let v = zmienne_tematu(&snap, MailCategory::Lifecycle, "start bota", 0);
        assert_eq!(v.get("login"), Some("—"));
        assert_eq!(v.get("broker"), Some("—"));
        assert_eq!(v.get("user"), Some("—"));
        assert_eq!(v.get("dzwignia"), Some("—"));
        assert_eq!(v.get("symbol"), Some("—"));
        assert_eq!(v.get("bid"), Some("—"));
        // brak etykiety presetu ≠ brak konfiguracji — mówimy to wprost
        assert_eq!(v.get("preset"), Some("(ustawienia własne)"));
    }

    #[test]
    fn uzytkownik_bez_imienia_dostaje_swoj_uchwyt() {
        let mut snap = migawka();
        snap.connection.user.name = String::new();
        let v = zmienne_tematu(&snap, MailCategory::Test, "test", 0);
        assert_eq!(v.get("user"), Some("@demo_user"));
    }

    #[test]
    fn calosc_dziala_na_przykladzie_z_zadania() {
        let snap = migawka();
        let v = zmienne_tematu(&snap, MailCategory::Summary, "raport", 0);
        assert_eq!(
            render_subject("Aktualny balans: ${balance}", &v),
            "Aktualny balans: $237.65"
        );
        assert_eq!(
            render_subject(
                "${user} · ${preset} · ${balance} / ${equity} · ${symbol}",
                &v
            ),
            "Demo User · ULTRA-X3 · $237.65 / $241.02 · XAUUSD"
        );
    }

    #[test]
    fn kategorie_maja_stabilne_klucze() {
        // klucze trafiają do smtp.json — zmiana zepsułaby zapisane ustawienia
        assert_eq!(MailCategory::Mt5Connection.key(), "mt5Connection");
        assert_eq!(MailCategory::SignalUnreadable.key(), "signalUnreadable");
        assert_eq!(MailCategory::ALL.len(), 8);
        let cats = MailCategories::default();
        assert!(cats.enabled(MailCategory::Drawdown));
        assert!(
            !cats.enabled(MailCategory::Summary),
            "raport okresowy domyślnie wyłączony"
        );
        assert!(
            cats.enabled(MailCategory::Test),
            "mail testowy zawsze dozwolony"
        );
        assert!(
            cats.enabled(MailCategory::SignalUnreadable),
            "alarm o nieczytelnym sygnale domyślnie WŁĄCZONY — cisza kosztowała już dzień handlu"
        );
        // STARY smtp.json nie ma tego klucza. `#[serde(default)]` na strukturze
        // dałby `false`, czyli alarm martwy u każdego, kto już raz zapisał
        // ustawienia — stąd własna wartość domyślna na polu.
        let stary: MailCategories =
            serde_json::from_str(r#"{"lifecycle":true,"mt5Connection":true,"summary":false}"#)
                .unwrap();
        assert!(
            stary.enabled(MailCategory::SignalUnreadable),
            "brak klucza = alarm WŁĄCZONY"
        );
    }
}
