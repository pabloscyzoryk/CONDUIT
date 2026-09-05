//! Model danych WYSYŁANY DO INTERFEJSU.
//!
//! To jest świadomie osobna warstwa od `conduit_core::types`. Powód:
//! rdzeń ma reprezentację wygodną dla silnika (snake_case, `Ts` w ms,
//! brak symbolu w pozycji, brak zysku — bo zysk zależy od kwotowania),
//! a React ma gotowy, przetestowany model domenowy w `src/types/index.ts`
//! (camelCase, zysk w pozycji, symbol w każdej strukturze).
//!
//! Tłumaczenie robimy TU, raz, po stronie Rusta. Dzięki temu:
//!  * żaden komponent React nie musi znać kształtu rdzenia,
//!  * zmiana w rdzeniu nie łamie 7 widoków, tylko jedną funkcję mapującą,
//!  * kontrakt `AppContextValue` zostaje nietknięty.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ============================================================
//  PODSTAWY
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    #[serde(rename = "BUY")]
    Buy,
    #[serde(rename = "SELL")]
    Sell,
}

impl From<conduit_core::Side> for Direction {
    fn from(s: conduit_core::Side) -> Self {
        match s {
            conduit_core::Side::Buy => Direction::Buy,
            conduit_core::Side::Sell => Direction::Sell,
        }
    }
}

impl From<Direction> for conduit_core::Side {
    fn from(d: Direction) -> Self {
        match d {
            Direction::Buy => conduit_core::Side::Buy,
            Direction::Sell => conduit_core::Side::Sell,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingKind {
    #[serde(rename = "BUY_LIMIT")]
    BuyLimit,
    #[serde(rename = "SELL_LIMIT")]
    SellLimit,
    #[serde(rename = "BUY_STOP")]
    BuyStop,
    #[serde(rename = "SELL_STOP")]
    SellStop,
}

impl From<conduit_core::PendingKind> for PendingKind {
    fn from(k: conduit_core::PendingKind) -> Self {
        use conduit_core::PendingKind as K;
        match k {
            K::BuyLimit => PendingKind::BuyLimit,
            K::SellLimit => PendingKind::SellLimit,
            K::BuyStop => PendingKind::BuyStop,
            K::SellStop => PendingKind::SellStop,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradingMode {
    #[serde(rename = "MANUAL")]
    Manual,
    /// domyślny: bot prowadzi pozycje wg konfiguracji
    #[default]
    #[serde(rename = "AUTO")]
    Auto,
    #[serde(rename = "AUTO-EA")]
    AutoEa,
    #[serde(rename = "AI")]
    Ai,
}

impl TradingMode {
    /// Czy ten tryb czyta WŁASNY wskaźnik łańcucha (`aktywny_ea`).
    ///
    /// Dokładnie jeden tryb tak ma i to jest cała reguła — wypisana raz,
    /// żeby nie rozsypać `== TradingMode::AutoEa` po pięciu plikach.
    pub fn wlasny_lancuch(self) -> bool {
        matches!(self, TradingMode::AutoEa)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum KtoraDrabinka {
    /// drabinka trybów AUTO / MANUAL / AI — pole `drabinka`
    Wspolna,
    /// drabinka trybu AUTO-EA („SKYNET-1") — pole `drabinka_ea`
    Ea,
}

impl KtoraDrabinka {
    pub fn dla(tryb: TradingMode) -> Self {
        if tryb.wlasny_lancuch() {
            KtoraDrabinka::Ea
        } else {
            KtoraDrabinka::Wspolna
        }
    }

    /// Nazwa do DZIENNIKA — komunikat odmowy ma nazywać obie strony sporu
    /// po imieniu, inaczej „niezgodność trybu" nie mówi nic operatorowi.
    pub fn nazwa(self) -> &'static str {
        match self {
            KtoraDrabinka::Wspolna => "drabinka trybów AUTO/MANUAL/AI",
            KtoraDrabinka::Ea => "drabinka trybu AUTO-EA (SKYNET-1)",
        }
    }
}

/// STRAŻ SILNIKOWA DRABINKI — czy wolno tknąć tę drabinkę w tym trybie.
///
/// Jedno miejsce dla wszystkich dróg (komenda z panelu, komenda ze skryptu,
/// krok pętli handlowej). `Err` niesie GOTOWE ZDANIE do dziennika: odmowa,
/// której nie da się przeczytać, jest tylko cichszą wersją wykonania.
pub fn straz_drabinki(tryb: TradingMode, cel: KtoraDrabinka) -> Result<(), String> {
    let moja = KtoraDrabinka::dla(tryb);
    if moja == cel {
        return Ok(());
    }
    Err(format!(
        "ODMOWA: komenda dotyczy „{}”, a bot pracuje w trybie {:?}, w którym obowiązuje „{}”. \
         Każdy tryb ma WŁASNĄ drabinkę i wolno mu ruszyć wyłącznie własną — inaczej ustawienie \
         zrobione w jednym trybie może przestawić skład używany przez drugi.",
        cel.nazwa(),
        tryb,
        moja.nazwa()
    ))
}

/// KTÓRY ŁAŃCUCH OBOWIĄZUJE W TYM TRYBIE (projekt EA-2).
///
/// AUTO-EA prowadzi własny skład: czyta `aktywny_ea`, a gdy ten jest pusty
/// ALBO wskazuje łańcuch spoza listy — wraca na zwykłe `aktywny`. Pozostałe
/// tryby nie wiedzą o istnieniu drugiego wskaźnika.
///
/// # Kontrakt zera
///
/// Plik `lancuchy.json` sprzed tej wersji nie ma pola `aktywnyEa`, więc
/// `aktywny_ea` jest pustym łańcuchem znaków i AUTO-EA gra tym samym, czym
/// grało dotąd. Fallback jest tu, a nie w miejscu wczytania, świadomie:
/// wskaźnik może wskazać łańcuch, który użytkownik SKASOWAŁ po jego
/// zapisaniu, a cichy brak składu jest gorszy niż powrót do wspólnego.
pub fn aktywny_dla<'a>(
    lancuchy: &'a conduit_core::formaty::Lancuchy,
    aktywny_ea: &'a str,
    tryb: TradingMode,
) -> &'a str {
    if tryb.wlasny_lancuch()
        && !aktywny_ea.is_empty()
        && lancuchy.lista.iter().any(|l| l.nazwa == aktywny_ea)
    {
        return aktywny_ea;
    }
    &lancuchy.aktywny
}

/// Ten sam wybór, ale od razu jako łańcuch z listy. `None` = wskazanie
/// pokazuje w pustkę (zbiór bez łańcuchów albo nazwa spoza listy).
pub fn lancuch_dla<'a>(
    lancuchy: &'a conduit_core::formaty::Lancuchy,
    aktywny_ea: &str,
    tryb: TradingMode,
) -> Option<&'a conduit_core::formaty::Lancuch> {
    let nazwa = aktywny_dla(lancuchy, aktywny_ea, tryb).to_string();
    lancuchy.lista.iter().find(|l| l.nazwa == nazwa)
}

/// Powód zamknięcia — nazwy identyczne jak w `src/types/index.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloseReason {
    #[serde(rename = "TP")]
    Tp,
    #[serde(rename = "SL")]
    Sl,
    #[serde(rename = "VSL")]
    Vsl,
    #[serde(rename = "MANUAL")]
    Manual,
    #[serde(rename = "PARTIAL")]
    Partial,
    #[serde(rename = "BASKET")]
    Basket,
    #[serde(rename = "RISK_FREE")]
    RiskFree,
    #[serde(rename = "OAE")]
    Oae,
    #[serde(rename = "HARVEST")]
    Harvest,
    #[serde(rename = "STALE")]
    Stale,
    #[serde(rename = "TRAIL")]
    Trail,
    #[serde(rename = "EOD")]
    Eod,
    #[serde(rename = "DAY_TARGET")]
    DayTarget,
    #[serde(rename = "MAX_DD")]
    MaxDd,
    #[serde(rename = "AI")]
    Ai,
}

impl From<conduit_core::CloseReason> for CloseReason {
    fn from(r: conduit_core::CloseReason) -> Self {
        use conduit_core::CloseReason as R;
        match r {
            R::Tp => CloseReason::Tp,
            R::Sl => CloseReason::Sl,
            R::VirtualSl => CloseReason::Vsl,
            R::Manual => CloseReason::Manual,
            R::Partial => CloseReason::Partial,
            R::RiskFree => CloseReason::RiskFree,
            R::OutAtEntry => CloseReason::Oae,
            R::Harvest => CloseReason::Harvest,
            R::Stale => CloseReason::Stale,
            R::Trail => CloseReason::Trail,
            R::BasketClose => CloseReason::Basket,
            R::EodFlat => CloseReason::Eod,
            R::DayTarget => CloseReason::DayTarget,
            R::MaxDd => CloseReason::MaxDd,
            R::Ai => CloseReason::Ai,
            R::Expired => CloseReason::Manual,
            // panel nie ma osobnego kafla dla reversal-exit — pokazujemy jako
            // żniwa (to jest bank zysku), a rozróżnienie żyje w dzienniku
            // (`close_reason_str` = "RevExit")
            R::RevExit => CloseReason::Harvest,
        }
    }
}

// ============================================================
//  RYNEK
// ============================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quote {
    pub symbol: String,
    pub bid: f64,
    pub ask: f64,
    pub spread: f64,
    pub time: i64,
    pub change_pct: f64,
    pub change: f64,
    pub day_high: f64,
    pub day_low: f64,
}

// ============================================================
//  POZYCJE I ZLECENIA
// ============================================================

/// Skąd pochodzi wiersz pokazywany w panelu.
///
/// CONDUIT jest ogólną platformą tradingową, a kopiowanie sygnałów z Telegrama
/// to jedna z jej funkcji. Panel pokazuje **wszystko, co dzieje się na koncie** —
/// to pole mówi, co z tego prowadzi bot, a co żyje własnym życiem.
///
/// Ważne: `Bot` to jedyna kategoria, którą silnik ZARZĄDZA. Pozostałe są
/// prawidłowymi pozycjami użytkownika i nie wolno ich ruszać — ale ukrywanie
/// ich robiło z panelu kłamcę (equity je liczyło, lista nie pokazywała).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    /// prowadzone przez silnik CONDUIT
    #[serde(rename = "BOT")]
    Bot,
    #[serde(rename = "EXTERNAL")]
    External,
    /// otwarte ręcznie z terminala (`magic` 0)
    #[serde(rename = "MANUAL")]
    Manual,
}

impl Default for Origin {
    fn default() -> Self {
        Origin::Manual
    }
}

impl Origin {
    /// Warstwa serwera świadomie NIE zna `conduit_mt5` (nie zależy od niego),
    /// więc tłumaczenie idzie po nazwie — tej samej, którą zwraca
    /// `conduit_mt5::Origin::as_str`.
    pub fn from_name(s: &str) -> Origin {
        match s {
            "EXTERNAL" => Origin::External,
            "MANUAL" => Origin::Manual,
            _ => Origin::Bot,
        }
    }
}

/// Podsumowanie tego, co na rachunku NIE należy do bota.
///
/// Panel pokazuje je jako pasek informacyjny nad listą: użytkownik ma od razu
/// widzieć, ile z jego equity i marginesu pochodzi z rzeczy, których CONDUIT
/// nie prowadzi.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignSummary {
    /// liczba otwartych pozycji spoza bota
    pub positions: usize,
    /// liczba zleceń oczekujących spoza bota
    pub pendings: usize,
    /// łączny wolumen tych pozycji (loty)
    pub volume: f64,
    /// łączny wynik pływający tych pozycji
    pub profit: f64,
    /// napotkane numery `magic` (bez naszego), rosnąco
    pub magics: Vec<i64>,
    /// napotkane symbole, alfabetycznie
    pub symbols: Vec<String>,
    /// przykładowe komentarze (do 3) — pomagają rozpoznać, czyj to automat
    pub comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub ticket: u64,
    pub symbol: String,
    pub direction: Direction,
    pub volume: f64,
    pub open_price: f64,
    pub open_time: i64,
    pub sl: Option<f64>,
    pub tp: Option<f64>,
    pub vsl: Option<f64>,
    pub profit: f64,
    pub swap: f64,
    pub commission: f64,
    pub comment: String,
    #[serde(default)]
    pub magic: Option<i64>,
    pub basket_id: Option<u32>,
    pub level: i32,
    pub frozen: bool,
    pub peak_pts: f64,
    pub runner: bool,
    pub toucher: bool,
    pub last_peak_time: i64,
    /// Kto to otworzył. `BOT` = prowadzone przez silnik; reszta jest widoczna,
    /// ale niezarządzana. `default` dla zgodności ze starymi backupami.
    #[serde(default)]
    pub source: Origin,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingOrder {
    pub ticket: u64,
    pub symbol: String,
    pub kind: PendingKind,
    pub volume: f64,
    pub price: f64,
    pub sl: Option<f64>,
    pub tp: Option<f64>,
    pub placed_time: i64,
    pub comment: String,
    pub basket_id: Option<u32>,
    pub level: i32,
    pub frozen: bool,
    #[serde(default)]
    pub source: Origin,
    /// `magic` zlecenia — dla wierszy spoza bota mówi, czyj to automat.
    /// `None` = wartość nieznana (patrz `Position::magic`), nie zero.
    #[serde(default)]
    pub magic: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosedPosition {
    pub ticket: u64,
    pub symbol: String,
    pub direction: Direction,
    pub volume: f64,
    pub open_price: f64,
    pub close_price: f64,
    pub open_time: i64,
    pub close_time: i64,
    pub profit: f64,
    pub swap: f64,
    pub commission: f64,
    pub reason: CloseReason,
    pub comment: String,
    pub basket_id: Option<u32>,
    #[serde(default)]
    pub source: Origin,
    /// `None` = wartość nieznana, nie zero (patrz `Position::magic`).
    #[serde(default)]
    pub magic: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingHistoryStatus {
    #[serde(rename = "FILLED")]
    Filled,
    #[serde(rename = "CANCELLED")]
    Cancelled,
    #[serde(rename = "EXPIRED")]
    Expired,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingHistoryItem {
    pub ticket: u64,
    pub symbol: String,
    pub kind: PendingKind,
    pub volume: f64,
    pub price: f64,
    pub sl: Option<f64>,
    pub tp: Option<f64>,
    pub placed_time: i64,
    pub end_time: i64,
    pub status: PendingHistoryStatus,
    pub basket_id: Option<u32>,
}

// ============================================================
//  KOSZYKI
// ============================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasketEvent {
    pub t: i64,
    pub text: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Basket {
    pub id: u32,
    pub symbol: String,
    pub direction: Direction,
    pub is_limit: bool,
    pub entry_low: f64,
    pub entry_high: f64,
    pub zone_low: f64,
    pub zone_high: f64,
    pub sl: Option<f64>,
    pub tps: Vec<f64>,
    pub tp_stage: usize,
    pub created_at: i64,
    pub source: String,
    pub source_key: String,
    pub active: bool,
    pub tickets: Vec<u64>,
    pub pending_tickets: Vec<u64>,
    pub events: Vec<BasketEvent>,
    pub risk_free: bool,
}

// ============================================================
//  STATYSTYKI
// ============================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LotNogi {
    pub format: String,
    pub preset: String,
    /// bieżący lot nogi = `lot_size(podstawa)` JEJ silnika (pola nogi!)
    pub lot: f64,
    /// Σ wolumenu zamkniętych transakcji tej nogi w pamięci panelu
    pub wolumen_wykonany: f64,
    /// czy noga tylko zarządza (zamrożona) — dymek ją podpisuje, kafel
    /// NIE wlicza jej lota do sumy (nie otwiera nowych pozycji)
    pub zamrozona: bool,
    #[serde(default)]
    pub handluje: bool,
    /// Czy pola HANDLU tej nogi przyszły z PLIKU presetu (`true`), czy
    /// z dokumentu panelu (`false`). Patrz [`conduit_core::routing::Silnik`].
    #[serde(default)]
    pub z_pliku: bool,
    /// Sufit lota POJEDYNCZEGO zlecenia z presetu nogi (`0` = brak).
    #[serde(default)]
    pub lot_max: f64,
    #[serde(default)]
    pub lot_koszyka: f64,
    /// Ile szczebli stawia jeden sygnał (`entry_units` po bramce kapitałowej
    /// i reżimie miękkim). Panel mnoży przez to `lot` w podpisie.
    #[serde(default)]
    pub poziomy_wejscia: u32,
    /// Pułap `max_lotow` aktywnego ŁAŃCUCHA (`0` = brak).
    ///
    /// ⚠ To jest **bramka wejścia**, nie ogranicznik wolumenu: silnik
    /// (`engine.rs`, `Gate::Blocked` „pułap łańcucha") odmawia OTWARCIA, gdy
    /// suma lotów na rachunku sięgnęła progu — ale nigdy nie zmniejsza lota
    /// zlecenia, które właśnie składa. Panel ma to nazywać po imieniu.
    #[serde(default)]
    pub pulap_lancucha: f64,

    #[serde(default)]
    pub stan: String,
    /// Dlaczego noga nie handluje. Pusty łańcuch = handluje.
    /// Kod, nie zdanie — tłumaczenie należy do panelu.
    #[serde(default)]
    pub powod: String,
    /// Łańcuch, z którego pochodzi noga (`""` = spoza łańcuchów).
    #[serde(default)]
    pub lancuch: String,
    /// Próg BALANCE szczebla drabinki, na którym stoi ten łańcuch.
    /// `-1` = łańcuch spoza drabinki (albo drabinka wyłączona).
    #[serde(default = "prog_spoza_drabinki")]
    pub prog: f64,
}

/// Wartownik dla `LotNogi::prog` — `-1` znaczy „nie ma tego w drabince".
/// Zero byłoby kłamstwem: `0` to PRAWDZIWY próg szczebla bazowego.
fn prog_spoza_drabinki() -> f64 {
    -1.0
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub t: i64,
    pub v: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub balance: f64,
    pub equity: f64,
    pub margin: f64,
    pub free_margin: f64,
    pub margin_level: f64,

    // ---------- KREDYT BONUSOWY: trzy liczby i ŹRÓDŁO ----------
    //
    // Panel ma pokazywać saldo brokera, kredyt i podstawę lota OSOBNO, bo
    // inaczej użytkownik nie ma jak sprawdzić, od czego bot naprawdę liczy
    // wolumen — a to jest różnica 2× na koncie z bonusem 100 %.
    /// Kredyt bonusowy raportowany przez TERMINAL (`ACCOUNT_CREDIT`).
    /// Zero = brak bonusu albo sidecar starszy niż ta wersja.
    #[serde(default)]
    pub credit: f64,
    /// Kredyt faktycznie ODLICZANY od podstawy lota. Zero, gdy przełącznik
    /// `odlicz_kredyt` jest wyłączony — niezależnie od `credit`.
    #[serde(default)]
    pub credit_applied: f64,
    /// `"terminal"` · `"reczny"` · `"off"` — skąd wzięła się liczba wyżej.
    /// Bez tego pola „kredyt 300 $" w panelu nie odróżnia odczytu z terminala
    /// od kwoty, którą ktoś wpisał kwartał temu i zapomniał.
    #[serde(default)]
    pub credit_source: String,
    /// PODSTAWA WIELKOŚCI POZYCJI: `max(balance - credit_applied, 0)`.
    #[serde(default)]
    pub lot_base: f64,
    #[serde(default)]
    pub lot_nogi: Vec<LotNogi>,
    /// Kwota ręczna nie zgadza się z terminalem — do pokazania jako
    /// OSTRZEŻENIE, nie do cichego wyboru jednej z nich. Najczęstsza
    /// przyczyna: broker zdjął bonus, a w ustawieniach została stara kwota.
    #[serde(default)]
    pub credit_mismatch: bool,
    pub pnl_today: f64,
    pub pnl_session: f64,
    pub session_start: i64,
    pub drawdown_now: f64,
    pub max_dd_today: f64,
    pub peak_equity_today: f64,
    pub drawdown_balance_now: f64,
    pub max_dd_balance_today: f64,
    pub peak_balance_today: f64,
    pub day_start_equity: f64,
    /// Numer doby w czasie SERWERA BROKERA (dni od epoki), do której odnoszą
    /// się pola `*_today`.
    ///
    /// Bez tego pola dobowe liczniki nie miały jak się przestawić: bot na
    /// żywo startował z saldem wziętym z `--balance` (domyślnie 2000 $) i przy
    /// realnym koncie na 410 $ pokazywał obsunięcie 1589 $ zaraz po
    /// uruchomieniu, a `pnlToday` zostawało na zerze na zawsze. Zero oznacza
    /// „jeszcze nie widzieliśmy brokera" — pierwsza publikacja przestawi.
    #[serde(default)]
    pub day_key: i64,
    /// Kapitał w chwili startu sesji — odniesienie dla `pnl_session`.
    /// Zero = jeszcze nieustalony; ustala go pierwsza publikacja z brokera.
    #[serde(default)]
    pub session_start_equity: f64,
    #[serde(default)]
    pub konto_kotwic: String,
    pub messages: u64,
    pub signals: u64,
    pub equity_curve: Vec<CurvePoint>,
}

impl Stats {
    pub fn new(balance: f64, now: i64) -> Self {
        Stats {
            balance,
            equity: balance,
            margin: 0.0,
            free_margin: balance,
            margin_level: 0.0,
            credit: 0.0,
            credit_applied: 0.0,
            credit_source: "off".into(),
            lot_base: balance,
            lot_nogi: Vec::new(),
            credit_mismatch: false,
            pnl_today: 0.0,
            pnl_session: 0.0,
            session_start: now,
            drawdown_now: 0.0,
            max_dd_today: 0.0,
            peak_equity_today: balance,
            day_start_equity: balance,
            day_key: 0,
            drawdown_balance_now: 0.0,
            max_dd_balance_today: 0.0,
            peak_balance_today: 0.0,
            session_start_equity: 0.0,
            konto_kotwic: String::new(),
            messages: 0,
            signals: 0,
            equity_curve: vec![CurvePoint { t: now, v: balance }],
        }
    }

    /// PRZYPISANIE KOTWIC PnL DO RACHUNKU. Zwraca `true`, gdy wykryto ZMIANĘ
    /// konta i kotwice zostały wyzerowane.
    ///
    /// * pierwsza publikacja (`konto_kotwic` puste) — tylko przypisuje klucz,
    ///   niczego nie zeruje: to jest ten sam rachunek, na którym powstały
    ///   kotwice z backupu;
    /// * ten sam klucz — nic;
    /// * INNY klucz — dzień i sesja startują od bieżącego equity, szczyt dnia
    ///   i obsunięcie od zera, krzywa od jednego punktu. Liczby liczone od
    ///   kotwic cudzego konta są gorsze niż brak liczb (dowód: „PNL SESJI
    ///   −107,80" na koncie bez jednej pozycji).
    ///
    /// ŚWIADOMIE bez pamięci kotwic starego konta: powrót na poprzedni
    /// rachunek TEŻ zeruje. Przechowywanie kompletu kotwic per konto to mapa,
    /// której nikt nie sprząta, i dzień „wznowiony" po tygodniu przerwy —
    /// mniejsze zło to uczciwe „liczę od teraz".
    pub fn przelacz_konto(&mut self, klucz: &str, equity: f64, doba: i64, now: i64) -> bool {
        if self.konto_kotwic == klucz {
            return false;
        }
        let pierwsze = self.konto_kotwic.is_empty();
        self.konto_kotwic = klucz.to_string();
        if pierwsze {
            return false;
        }
        self.day_key = doba;
        self.day_start_equity = equity;
        self.peak_equity_today = equity;
        self.max_dd_today = 0.0;
        self.drawdown_now = 0.0;
        self.pnl_today = 0.0;
        self.session_start = now;
        self.session_start_equity = equity;
        self.pnl_session = 0.0;
        self.equity_curve = vec![CurvePoint { t: now, v: equity }];
        true
    }
}

// ============================================================
//  TELEGRAM / LOGI
// ============================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedSignal {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_limit: Option<bool>,
    /// Zlecenie oczekujące na PRZEBICIE poziomu, a nie na powrót do niego.
    /// `BUY STOP` i `BUY LIMIT` to dwie różne strony rynku wobec ceny —
    /// bez tego pola podgląd rozbioru pokazywał je identycznie.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_stop: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_low: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_high: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sl: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tps: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tp_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<f64>,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub time: i64,
    pub channel_id: i64,
    pub channel_name: String,
    /// ID TEMATU FORUM — osobne pole, bo `channel_id` NIE JEST tożsamością
    /// źródła.
    ///
    /// Na forum wszystkie tematy jednej grupy mają ten sam `chat_id` i tę samą
    /// nazwę (Telegram oddaje nazwę per dialog, nie per temat). Panel budował
    /// listę filtra po samym `chat_id`, więc ZEN, NOVA i PulseX — trzy różne
    /// formaty, trzy różne presety — zlewały się w JEDNĄ pozycję rozwijanej
    /// listy, a wybranie jej nie zmieniało niczego. Tożsamością źródła jest
    /// `SourceKey { chat_id, topic_id }` (patrz `types.rs`) i taką samą parę
    /// musi mieć pod ręką panel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_name: Option<String>,
    /// Format przypisany do tego źródła (`ZEN`, `Synergy`…) albo `None`, gdy
    /// źródło jest tylko nasłuchiwane. Panel pokazuje go przy wiadomości —
    /// inaczej „dlaczego ten sygnał nie zagrał" wymaga wejścia w Kanały.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    pub text: String,
    pub types: Vec<String>,
    pub basket_id: Option<u32>,
    pub edited: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parsed: Option<Vec<ParsedSignal>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub id: u64,
    pub t: i64,
    pub category: String,
    pub title: String,
    pub content: String,
    pub level: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", from = "SurowePowiazanie")]
pub struct ChannelBinding {
    pub channel_id: i64,
    pub monitored: bool,
    pub notify: bool,
    /// JEDEN format kanału zwykłego. Pusty = kanał nie handluje.
    #[serde(default)]
    pub format: String,
    /// temat forum → JEDEN format; klucz to id tematu
    /// (JSON wymaga stringów w kluczach obiektu)
    #[serde(default)]
    pub topics: BTreeMap<String, String>,
}

impl ChannelBinding {
    /// FORMAT dla wiadomości z tego kanału (i ewentualnie tematu).
    ///
    /// To jest JEDYNE miejsce, w którym rozstrzyga się „którym presetem gra
    /// to źródło" — `live.rs::format_zrodla` deleguje tutaj. Wyciągnięte
    /// z domknięcia na stanie po to, żeby dało się przetestować dokładnie
    /// typowy przypadek: dwa formaty to DWA TEMATY JEDNEJ grupy forum
    /// (zanonimizowany identyfikator), więc routing po samym `chat_id` by je pomieszał.
    ///
    /// Reguły, obie świadome:
    /// * niepusta mapa tematów = gra WYŁĄCZNIE to, co wymienione; temat
    ///   spoza mapy (np. PulseX obok ZEN) nie handluje, choć kanał tak;
    /// * pusta mapa tematów = całe źródło gra formatem kanału — także gdy
    ///   wiadomość niesie temat (kanał mógł stać się forum po konfiguracji).
    pub fn format_dla(&self, topic_id: Option<i64>) -> Option<String> {
        match topic_id {
            Some(t) if !self.topics.is_empty() => self
                .topics
                .get(&t.to_string())
                .filter(|f| !f.is_empty())
                .cloned(),
            _ => Some(self.format.clone()).filter(|f| !f.is_empty()),
        }
    }

    /// Czy to źródło jest w ogóle OBSERWOWANE (nasłuch, nie handel).
    /// `live.rs::kanal_obserwowany` deleguje tutaj.
    pub fn obserwuje(&self, topic_id: Option<i64>) -> bool {
        if !self.monitored {
            return false;
        }
        match topic_id {
            Some(t) if !self.topics.is_empty() => self.topics.contains_key(&t.to_string()),
            _ => true,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SurowePowiazanie {
    channel_id: i64,
    #[serde(default)]
    monitored: bool,
    #[serde(default)]
    notify: bool,
    #[serde(default)]
    format: Option<serde_json::Value>,
    #[serde(default)]
    formats: Option<serde_json::Value>,
    #[serde(default)]
    topics: BTreeMap<String, serde_json::Value>,
}

impl From<SurowePowiazanie> for ChannelBinding {
    fn from(r: SurowePowiazanie) -> Self {
        let format = r
            .format
            .as_ref()
            .map(|v| pierwszy_format(v, "kanał"))
            .filter(|f| !f.is_empty())
            .or_else(|| r.formats.as_ref().map(|v| pierwszy_format(v, "kanał")))
            .unwrap_or_default();
        ChannelBinding {
            channel_id: r.channel_id,
            monitored: r.monitored,
            notify: r.notify,
            format,
            topics: r
                .topics
                .iter()
                .map(|(k, v)| (k.clone(), pierwszy_format(v, &format!("temat {k}"))))
                .filter(|(_, f)| !f.is_empty())
                .collect(),
        }
    }
}

/// Ta sama migracja dla wartości, której nie mamy na własność — używa jej
/// obsługa polecenia `setBinding` (`commands.rs`), bo panel do czasu pełnego
/// przejścia wysyła OBA kształty naraz.
pub fn pierwszy_format(v: &serde_json::Value, gdzie: &str) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => {
            if a.len() > 1 {
                let nazwy: Vec<String> = a
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect();
                tracing::warn!(
                    gdzie,
                    formaty = ?nazwy,
                    "{gdzie} ma {} formatów, a wolno mieć JEDEN — biorę pierwszy",
                    a.len()
                );
            }
            a.first()
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string()
        }
        _ => String::new(),
    }
}

// ============================================================
//  KONFIGURACJA WIDOCZNA W UI
// ============================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LotConfig {
    pub mode: String,
    pub fixed: f64,
    pub percent: f64,
}

impl Default for LotConfig {
    fn default() -> Self {
        LotConfig {
            mode: "fixed".into(),
            fixed: 0.01,
            percent: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EmailConfig {
    pub enabled: bool,
    /// odbiorcy — jeden lub wielu, po przecinku/średniku/spacji
    pub to: String,
    pub interval_min: f64,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Hasło SMTP NIGDY nie wychodzi do UI — serwer wysyła pustą wartość,
    /// a przy zapisie pusta wartość oznacza „zostaw stare hasło".
    /// Właściwym magazynem jest `secrets.json`, nie ten dokument.
    pub pass: String,

    /// Nadawca. Puste = adres logowania (`user`). Osobne pole, bo część
    /// serwerów firmowych wymaga innego „From" niż konta logowania.
    pub from: String,

    pub subject: String,
    /// STARTTLS (587) / SSL (465) / bez szyfrowania.
    pub security: crate::mailer::MailSecurity,
    /// Które zdarzenia w ogóle wysyłać.
    pub categories: crate::mailer::MailCategories,
    /// Dławienie: okno scalania i sufit maili na godzinę.
    pub throttle: crate::mailer::ThrottleConfig,
}

impl Default for EmailConfig {
    fn default() -> Self {
        EmailConfig {
            enabled: false,
            to: String::new(),
            interval_min: 60.0,
            host: "smtp.example.com".into(),
            port: 587,
            user: String::new(),
            pass: String::new(),
            from: String::new(),
            subject: String::new(),
            security: crate::mailer::MailSecurity::Starttls,
            categories: crate::mailer::MailCategories::default(),
            throttle: crate::mailer::ThrottleConfig::default(),
        }
    }
}

impl EmailConfig {
    /// Kopia BEZ hasła — jedyna postać, w jakiej ta struktura opuszcza serwer.
    ///
    /// Wołane w jednym miejscu (budowa migawki), żeby nie dało się wysłać
    /// hasła przez przeoczenie w nowym endpoincie.
    pub fn redacted(&self) -> EmailConfig {
        EmailConfig {
            pass: String::new(),
            ..self.clone()
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifyConfig {
    pub channels: Vec<i64>,
    pub summary_enabled: bool,
    pub summary_interval_min: f64,
}

/// Jeden szczebel DRABINKI ŁAŃCUCHÓW — „od tego BALANCE graj tym łańcuchem".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SzczebelDrabinki {
    /// BALANCE w dolarach, od którego ten łańcuch obowiązuje.
    ///
    /// BALANCE, nigdy equity — wymóg użytkownika wprost: equity oddycha
    /// z otwartymi pozycjami i drabinka tańczyłaby w dołku floatingu.
    /// Balance zmienia się wyłącznie przy rozliczeniu transakcji.
    pub prog_balance: f64,
    /// Nazwa łańcucha z listy (`Lancuchy::lista`).
    pub lancuch: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DrabinkaLancuchow {
    pub enabled: bool,
    /// WYŁĄCZNIK UŻYTKOWNIKA — intencja, nie skutek. `None` = dokument sprzed
    /// projektu EA-2c; wtedy intencją jest to, co stoi w `enabled` (kontrakt
    /// zera: stary `backup_memory` zachowuje się co do bitu jak dotąd).
    ///
    /// Przelicza to [`przelicz_izolacje_drabinek`] — jedyne miejsce, które ma
    /// prawo ruszyć `enabled` bez udziału użytkownika.
    #[serde(default)]
    pub wlacznik: Option<bool>,
    /// Szczeble; kolejność dowolna, silnik sortuje malejąco po `prog_balance`.
    pub szczeble: Vec<SzczebelDrabinki>,
    /// Ile procent PONIŻEJ progu bieżącego szczebla musi spaść balance,
    /// żeby zejść niżej. Po to, żeby saldo drgające wokół progu nie
    /// przełączało łańcucha co pętlę — każde przełączenie to przebudowa
    /// silników i adopcja koszyków. `0` = zejście natychmiast pod progiem
    /// (użytkownik zawsze może tak ustawić; testy flappingu jadą na obu).
    pub histereza_pct: f64,
    /// Próg szczebla, na którym drabinka OSTATNIO stała (persystowany
    /// w backupie). `-1` = jeszcze nigdzie — pierwszy wybór bez histerezy.
    ///
    /// Bez tego pola restart w środku drabinki nie wiedziałby, wobec czego
    /// liczyć histerezę, i konto tuż pod progiem oscylowałoby przy każdym
    /// starcie.
    pub biezacy_prog: f64,
    /// Kiedy drabinka ostatnio przełączyła łańcuch (ms epoki, 0 = nigdy).
    pub ostatnia_zmiana_ts: i64,
}

impl Default for DrabinkaLancuchow {
    fn default() -> Self {
        DrabinkaLancuchow {
            enabled: false,
            wlacznik: None,
            szczeble: vec![SzczebelDrabinki {
                prog_balance: 0.0,
                lancuch: "MONOLIT-4".into(),
            }],
            histereza_pct: 2.0,
            biezacy_prog: -1.0,
            ostatnia_zmiana_ts: 0,
        }
    }
}

/// DRABINKA TRYBU AUTO-EA W INSTALACJI, KTÓRA JEJ NIE MIAŁA — pusta
/// i wyłączona (projekt EA-2c, kontrakt zera).
///
/// Świadomie NIE `Default::default()`: tamta niesie szczebel z koroną, więc
/// dołożenie pola `drabinka_ea` do istniejącego bota oznaczałoby, że AUTO-EA
/// nagle MA plan zmiany składu, którego właściciel nigdy nie ułożył. Pusta
/// drabinka to jedyna wartość, przy której zmiana wersji binarki nie zmienia
/// zachowania rachunku ani o krok.
pub fn drabinka_ea_domyslna() -> DrabinkaLancuchow {
    DrabinkaLancuchow::pusta()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NazwanaDrabinka {
    pub nazwa: String,
    /// Opis dla panelu — CO ta drabinka robi i JAKIE liczby za nią stoją.
    pub opis: String,
    /// Rekomendowana. Panel rysuje przy niej koronę.
    ///
    /// Dokładnie JEDNA pozycja listy ma tu `true` — bo „rekomendacja" znaczy
    /// „ta, którą wziąć, jeśli nie masz powodu wybrać innej", a dwie takie
    /// odpowiedzi to żadna odpowiedź.
    pub korona: bool,
    pub szczeble: Vec<SzczebelDrabinki>,
    pub histereza_pct: f64,
}

pub fn drabinki_wbudowane() -> Vec<NazwanaDrabinka> {
    vec![
        NazwanaDrabinka {
            nazwa: "MONOLIT-4".into(),
            opis: "Built-in ladder. Review every threshold and risk setting before use.".into(),
            korona: true,
            szczeble: vec![
                SzczebelDrabinki { prog_balance: 0.0, lancuch: "MONOLIT-4".into() },
            ],
            histereza_pct: 2.0,
        },
        NazwanaDrabinka {
            nazwa: "MONOLIT-3".into(),
            opis: "Built-in ladder. Review every threshold and risk setting before use.".into(),
            korona: false,
            szczeble: vec![
                SzczebelDrabinki { prog_balance: 0.0, lancuch: "MONOLIT-3".into() },
            ],
            histereza_pct: 2.0,
        },
        NazwanaDrabinka {
            nazwa: "MONOLIT-2".into(),
            opis: "Built-in ladder. Review every threshold and risk setting before use.".into(),
            korona: false,
            szczeble: vec![
                SzczebelDrabinki { prog_balance: 0.0, lancuch: "MONOLIT-2".into() },
            ],
            histereza_pct: 2.0,
        },
        NazwanaDrabinka {
            nazwa: "MONOLIT-1".into(),
            opis: "Built-in ladder. Review every threshold and risk setting before use.".into(),
            korona: false,
            szczeble: vec![
                SzczebelDrabinki { prog_balance: 0.0, lancuch: "MONOLIT-1".into() },
            ],
            histereza_pct: 2.0,
        },
        NazwanaDrabinka {
            nazwa: "OMEGA-3".into(),
            opis: "Built-in ladder. Review every threshold and risk setting before use.".into(),
            korona: false,
            szczeble: vec![
                SzczebelDrabinki { prog_balance: 0.0, lancuch: "OMEGA-3".into() },
            ],
            histereza_pct: 2.0,
        },
        NazwanaDrabinka {
            nazwa: "SENTINEL-3".into(),
            opis: "Built-in ladder. Review every threshold and risk setting before use.".into(),
            korona: false,
            szczeble: vec![
                SzczebelDrabinki { prog_balance: 0.0, lancuch: "SENTINEL-3".into() },
            ],
            histereza_pct: 2.0,
        },
        NazwanaDrabinka {
            nazwa: "FS-M3 CZOLOWKA".into(),
            opis: "Built-in ladder. Review every threshold and risk setting before use.".into(),
            korona: false,
            szczeble: vec![
                SzczebelDrabinki { prog_balance: 0.0, lancuch: "FS-M3-PARA".into() },
            ],
            histereza_pct: 2.0,
        },
        // Wersja ostrożna dla użytkownika, który chce prowadzić jedną nogę
        // strategii albo ograniczyć ekspozycję przy spadku jakości źródła.
        NazwanaDrabinka {
            nazwa: "FS-M3 SAMA SYNERGY".into(),
            opis: "Built-in ladder. Review every threshold and risk setting before use.".into(),
            korona: false,
            szczeble: vec![
                SzczebelDrabinki { prog_balance: 0.0, lancuch: "FS-M3-SOLO".into() },
            ],
            histereza_pct: 2.0,
        },
        NazwanaDrabinka {
            nazwa: "FS-M2 PEŁNA".into(),
            opis: "Built-in ladder. Review every threshold and risk setting before use.".into(),
            korona: false,
            szczeble: vec![
                SzczebelDrabinki { prog_balance: 0.0, lancuch: "FS-M2-SOLO".into() },
                SzczebelDrabinki { prog_balance: 1000.0, lancuch: "FS-M2-PARA".into() },
            ],
            histereza_pct: 2.0,
        },
        NazwanaDrabinka {
            nazwa: "FS-M2 OSTROŻNA".into(),
            opis: "Built-in ladder. Review every threshold and risk setting before use.".into(),
            korona: false,
            szczeble: vec![
                SzczebelDrabinki { prog_balance: 0.0, lancuch: "FS-M2-ZEN-SOLO".into() },
                SzczebelDrabinki { prog_balance: 1000.0, lancuch: "FS-M2-PARA".into() },
            ],
            histereza_pct: 2.0,
        },
        NazwanaDrabinka {
            nazwa: "FS-M2 SAMA SYNERGY".into(),
            opis: "Built-in ladder. Review every threshold and risk setting before use.".into(),
            korona: false,
            szczeble: vec![
                SzczebelDrabinki { prog_balance: 0.0, lancuch: "FS-M2-SOLO".into() },
            ],
            histereza_pct: 2.0,
        },
        NazwanaDrabinka {
            nazwa: "FFS-1C (poprzednia)".into(),
            opis: "Built-in ladder. Review every threshold and risk setting before use.".into(),
            korona: false,
            szczeble: vec![
                SzczebelDrabinki { prog_balance: 0.0, lancuch: "ZENONLY5".into() },
                SzczebelDrabinki { prog_balance: 500.0, lancuch: "ZENONLY3".into() },
                SzczebelDrabinki { prog_balance: 2000.0, lancuch: "SENTINEL-0C".into() },
            ],
            histereza_pct: 2.0,
        },
    ]
}

impl DrabinkaLancuchow {
    /// Drabinka BEZ SZCZEBLI i wyłączona — stan „tej drabinki nikt nie ułożył".
    pub fn pusta() -> Self {
        DrabinkaLancuchow {
            enabled: false,
            wlacznik: Some(false),
            szczeble: Vec::new(),
            histereza_pct: 2.0,
            biezacy_prog: -1.0,
            ostatnia_zmiana_ts: 0,
        }
    }

    /// Wyłącznik USTAWIONY PRZEZ UŻYTKOWNIKA (patrz [`Self::wlacznik`]).
    pub fn wlacznik_uzytkownika(&self) -> bool {
        self.wlacznik.unwrap_or(self.enabled)
    }

    /// PRZELICZENIE SKUTECZNOŚCI: `moj_tryb` mówi, czy bot pracuje właśnie
    /// w trybie, do którego ta drabinka należy.
    ///
    /// Idempotentne — wolno wołać po każdej zmianie stanu, nie trzeba pilnować
    /// „czy już". Po wywołaniu `wlacznik` jest zawsze jawny, więc kolejne
    /// przeliczenie nie ma jak zgubić intencji.
    pub fn ustaw_skutecznosc(&mut self, moj_tryb: bool) {
        let w = self.wlacznik_uzytkownika();
        self.wlacznik = Some(w);
        self.enabled = w && moj_tryb;
    }

    /// SPRAWDZENIE DRABINKI PRZED ZAPISEM — jedno miejsce dla wszystkich dróg.
    ///
    /// Reguły są trzy i każda ma powód mierzalny na rachunku, nie estetyczny:
    ///
    /// 1. **Progi ściśle rosnące w kolejności tablicy.** Dwa szczeble o tym
    ///    samym progu to pytanie „który wygrywa", na które nie ma dobrej
    ///    odpowiedzi — wybór zależałby od kolejności sortowania, czyli od
    ///    szczegółu implementacji. Próg mniejszy od poprzedniego znaczy, że
    ///    to, co użytkownik widzi jako kolejność drabinki, nie jest tym, czym
    ///    drabinka jest. **Nie sortujemy po cichu**: cichy sort zamienia
    ///    literówkę w inną konfigurację i nikt się o tym nie dowiaduje.
    /// 2. **Szczebel bazowy `0`.** Bez niego konto poniżej najniższego progu
    ///    nie ma przypisanego łańcucha — `wybierz` zwraca `None`, drabinka
    ///    zostawia to, co akurat było aktywne, i bot gra konfiguracją, której
    ///    nikt świadomie nie wybrał. Przy koncie startowym 200 $ i najniższym
    ///    progu 500 $ to jest stan domyślny, nie skrajny.
    /// 3. **Każdy łańcuch musi istnieć na liście.** Literówka w nazwie ma
    ///    krzyczeć przy zapisie, a nie cicho zjadać szczebel w pętli.
    ///
    /// Liczba szczebli jest DOWOLNA — świadomie nie ma tu limitu. Użytkownik
    /// wyjmuje i wstawia szczeble w zależności od tego, jak zachowuje się
    /// dana noga, i to jest jego bieżący sposób pracy z botem.
    pub fn sprawdz(&self, znane: &[String]) -> Result<(), String> {
        if !self.histereza_pct.is_finite() || self.histereza_pct < 0.0 {
            return Err("histereza musi być liczbą ≥ 0 %".into());
        }
        if self.szczeble.is_empty() {
            // Pusta drabinka jest dopuszczalna WYŁĄCZNIE wyłączona: nie ma
            // czego walidować i nic się nie przełączy.
            return if self.enabled {
                Err("drabinka włączona, ale nie ma ani jednego szczebla — \
                     dodaj szczebel bazowy (próg 0 $) albo wyłącz drabinkę"
                    .into())
            } else {
                Ok(())
            };
        }
        for (i, sz) in self.szczeble.iter().enumerate() {
            if !sz.prog_balance.is_finite() || sz.prog_balance < 0.0 {
                return Err(format!(
                    "szczebel {}: próg musi być liczbą ≥ 0 (jest: {})",
                    i + 1,
                    sz.prog_balance
                ));
            }
            if !znane.iter().any(|n| n == &sz.lancuch) {
                return Err(format!(
                    "szczebel {:.0} $ wskazuje łańcuch „{}”, którego nie ma na liście. \
                     Dostępne: {}",
                    sz.prog_balance,
                    sz.lancuch,
                    znane.join(", ")
                ));
            }
            if i > 0 {
                let poprzedni = self.szczeble[i - 1].prog_balance;
                if sz.prog_balance == poprzedni {
                    return Err(format!(
                        "dwa szczeble mają ten sam próg {:.0} $ („{}” i „{}”). \
                         Progi muszą być różne — inaczej nie da się powiedzieć, \
                         który łańcuch obowiązuje przy tym saldzie.",
                        sz.prog_balance,
                        self.szczeble[i - 1].lancuch,
                        sz.lancuch
                    ));
                }
                if sz.prog_balance < poprzedni {
                    return Err(format!(
                        "szczebel {} ma próg {:.0} $, czyli MNIEJSZY niż poprzedni ({:.0} $). \
                         Progi muszą rosnąć od bazowego w górę. Popraw wartość albo \
                         przestaw szczeble — świadomie nie sortuję ich za Ciebie, \
                         bo cichy sort zamieniłby literówkę w inną konfigurację.",
                        i + 1,
                        sz.prog_balance,
                        poprzedni
                    ));
                }
            }
        }
        if self.szczeble[0].prog_balance != 0.0 {
            return Err(format!(
                "brakuje szczebla BAZOWEGO: najniższy próg to {:.0} $, a musi być 0 $. \
                 Poniżej najniższego progu drabinka nie ma czego wybrać i bot zostaje \
                 przy łańcuchu, którego nikt świadomie nie wskazał.",
                self.szczeble[0].prog_balance
            ));
        }
        Ok(())
    }

    /// Który szczebel obowiązuje przy tym BALANCE? `None`, gdy drabinka
    /// wyłączona, pusta albo balance nie sięga najniższego progu.
    ///
    /// Histereza działa WYŁĄCZNIE przy schodzeniu: szczebel niższy niż ten,
    /// na którym stoimy (`biezacy_prog`), bierzemy dopiero gdy balance
    /// spadnie pod `prog_bieżącego · (1 − histereza%)`. Wejście w górę jest
    /// natychmiastowe — wyższy szczebel to z definicji przekroczony próg,
    /// a nie drganie wokół niego.
    pub fn wybierz(&self, balance: f64) -> Option<&SzczebelDrabinki> {
        if !self.enabled || self.szczeble.is_empty() {
            return None;
        }
        let mut posortowane: Vec<&SzczebelDrabinki> = self.szczeble.iter().collect();
        posortowane.sort_by(|a, b| {
            b.prog_balance
                .partial_cmp(&a.prog_balance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // kandydat wprost z balance: najwyższy próg ≤ balance
        let kandydat = posortowane
            .iter()
            .find(|s| balance >= s.prog_balance)
            .copied()?;

        // schodzimy PONIŻEJ szczebla, na którym stoimy → wymagaj histerezy
        if self.biezacy_prog >= 0.0 && kandydat.prog_balance < self.biezacy_prog {
            let granica = self.biezacy_prog * (1.0 - self.histereza_pct / 100.0);
            if balance >= granica {
                // drganie, nie spadek: zostań na bieżącym szczeblu
                return posortowane
                    .iter()
                    .find(|s| (s.prog_balance - self.biezacy_prog).abs() < 1e-9)
                    .copied()
                    .or(Some(kandydat));
            }
        }
        Some(kandydat)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub login: i64,
    pub server: String,
    pub broker: String,
    pub currency: String,
    pub leverage: u32,
    #[serde(rename = "type")]
    pub kind: String,
}

impl Default for AccountInfo {
    fn default() -> Self {
        AccountInfo {
            login: 0,
            server: String::new(),
            broker: String::new(),
            currency: "USD".into(),
            leverage: 0,
            kind: "DEMO".into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInfo {
    pub name: String,
    pub handle: String,
    pub phone: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionState {
    /// "connected" | "connecting" | "disconnected"
    pub telegram: String,
    pub mt5: String,
    pub account: AccountInfo,
    #[serde(default)]
    pub account_verified: String,
    /// Opaque, non-persisted broker binding generation. Commands must carry the
    /// token of the snapshot that created the user's intent, never a fresh one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub account_session: String,
    /// Instrument confirmed by the current broker session, not the requested
    /// settings/preset symbol. Empty until publication and after disconnect.
    /// Runtime-only: BackupMemory deliberately does not persist ConnectionState.
    #[serde(default)]
    pub resolved_symbol: String,
    pub user: UserInfo,
    pub latency_ms: u32,

    // ---------- ZDROWIE SESJI TELEGRAMA ----------
    //
    // Sama plakietka „connected" nie odróżnia spokojnej nocy od MARTWEGO
    // GNIAZDA MTProto: `next_message()` przy zerwanej sesji nie zwraca błędu,
    // tylko nigdy nic nie oddaje. Panel pokazywał wtedy „połączony" przy
    // sesji, która nie żyła — a to jest gorsze niż jawne rozłączenie, bo
    // użytkownik nie ma powodu niczego sprawdzać.
    //
    // Wypełnia je `live.rs` z `TelegramService::zdrowie()`.
    /// kiedy ostatnio przyszła JAKAKOLWIEK wiadomość — **ms epoki, nie „minut temu"**.
    /// `None` = nie wiemy. Wartość względna zapisana w migawce zestarzałaby się
    /// w `backup_memory` i po restarcie kłamała; panel liczy „min temu" przy rysowaniu.
    #[serde(default)]
    pub telegram_last_message_ms: Option<i64>,
    /// kiedy ostatni keepalive potwierdził żywą sesję (ms epoki). `None` = jeszcze nie było.
    #[serde(default)]
    pub telegram_last_ping_ok_ms: Option<i64>,
    /// nieudane pingi POD RZĄD — jedyna liczba, która odróżnia ciszę
    /// od awarii, i na której stoi reguła alarmu (koniunkcja: cisza ORAZ
    /// niepotwierdzony keepalive)
    #[serde(default)]
    pub telegram_ping_failures: u32,
    /// ile razy usługa musiała zalogować się od nowa z zapisanej sesji
    #[serde(default)]
    pub telegram_reconnects: u32,
}

impl Default for ConnectionState {
    fn default() -> Self {
        ConnectionState {
            telegram: "disconnected".into(),
            mt5: "disconnected".into(),
            account: AccountInfo::default(),
            account_verified: String::new(),
            account_session: String::new(),
            resolved_symbol: String::new(),
            user: UserInfo::default(),
            latency_ms: 0,
            telegram_last_message_ms: None,
            telegram_last_ping_ok_ms: None,
            telegram_ping_failures: 0,
            telegram_reconnects: 0,
        }
    }
}

// ============================================================
//  KLASA ZATRZYMANIA HANDLU
// ============================================================

/// Separator, którym sklejamy powody zatrzymania w jedno zdanie.
/// Ten sam, którego używa [`conduit_core::routing::Silniki::halted`].
pub const HALT_SEP: &str = " · ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KlasaHaltu {
    Diagnoza,
    Ryzyko,
}

impl KlasaHaltu {
    pub fn nazwa(self) -> &'static str {
        match self {
            KlasaHaltu::Diagnoza => "DIAGNOZA",
            KlasaHaltu::Ryzyko => "RYZYKO",
        }
    }
}

/// ZNANE ZDANIA KLASY DIAGNOZA — wyłącznie do migracji starych zapisów.
///
/// To NIE jest heurystyka po dowolnym tekście: zbiór jest ZAMKNIĘTY i pochodzi
/// z naszego własnego kodu (`conduit_server::bootstrap` i bramka startowa
/// w `live.rs`). Pełni jedną rolę — `backup_memory/latest.json` zapisany przez
/// starą binarkę nie ma pól klas, a bez migracji halt konfiguracyjny z takiego
/// pliku zostałby uznany za RYZYKO i dalej zamykałby bota w pętli.
const ZDANIA_DIAGNOZY: &[&str] = &[
    // conduit_server::bootstrap
    "konfiguracja nie została wczytana",
    "paczka nie zgadza się z konfiguracją",
    // live.rs — bramka startowa wielonogowa
    "bramka startowa serwera",
    "rachunek zdejmuje bezpiecznik nodze",
    "TERMINAL NA INNYM KONCIE",
];

/// ZATRZYMANIE HANDLU — rozbite na KLASY (patrz [`KlasaHaltu`]).
///
/// `active` i `reason` zostają tym, czym były: włącznikiem banera w panelu
/// i zdaniem dla człowieka. Są jednak **wyliczane** z pól klasowych
/// ([`HaltState::przelicz`]), a nie ustawiane niezależnie — inaczej dałoby się
/// dostać stan „powód mówi o obsunięciu, a klasa mówi diagnoza", czyli dokładnie
/// tę sprzeczność, dla której ta struktura powstaje.
///
/// # Dlaczego DWA POLA, a nie jedno `klasa: KlasaHaltu`
///
/// Bo obie klasy potrafią być aktywne NARAZ: strażnik obsunięcia zatrzymał
/// konto wczoraj (RYZYKO, przeżywa restart), a dziś rano paczka przyjechała
/// ze starą konfiguracją (DIAGNOZA, wyprowadzona od nowa). Jedno pole
/// enumeryczne musiałoby wtedy jedną z tych prawd po cichu wyrzucić — a to jest
/// ta sama klasa błędu, którą tu naprawiamy. Przy jednej aktywnej klasie
/// `reason` wychodzi znak w znak takie samo jak przed zmianą.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HaltState {
    pub active: bool,
    pub reason: String,
    /// powód klasy DIAGNOZA; pusty = takiego zatrzymania nie ma
    #[serde(default)]
    pub diagnoza: String,
    /// powód klasy RYZYKO; pusty = takiego zatrzymania nie ma
    #[serde(default)]
    pub ryzyko: String,
}

impl HaltState {
    pub fn nowy(klasa: KlasaHaltu, powod: impl Into<String>) -> Self {
        let mut h = HaltState::default();
        h.ustaw(klasa, powod);
        h
    }

    pub fn ma(&self, klasa: KlasaHaltu) -> bool {
        !self.powod(klasa).is_empty()
    }

    pub fn powod(&self, klasa: KlasaHaltu) -> &str {
        match klasa {
            KlasaHaltu::Diagnoza => &self.diagnoza,
            KlasaHaltu::Ryzyko => &self.ryzyko,
        }
    }

    fn powod_mut(&mut self, klasa: KlasaHaltu) -> &mut String {
        match klasa {
            KlasaHaltu::Diagnoza => &mut self.diagnoza,
            KlasaHaltu::Ryzyko => &mut self.ryzyko,
        }
    }

    /// Ustawia powód klasy, NADPISUJĄC to, co w niej stało. Pusty tekst
    /// znaczy „tej klasy nie ma" — czyli to samo, co [`HaltState::zdejmij`].
    pub fn ustaw(&mut self, klasa: KlasaHaltu, powod: impl Into<String>) {
        *self.powod_mut(klasa) = powod.into();
        self.przelicz();
    }

    /// DOKŁADA powód do klasy, jeśli jeszcze go tam nie ma.
    ///
    /// Potrzebne przy starcie serwera: nieodczytany `settings.json` i rozjazd
    /// pieczęci `PACZKA.json` to DWIE osobne diagnozy tego samego uruchomienia
    /// i żadna nie ma prawa skasować drugiej.
    pub fn dolacz(&mut self, klasa: KlasaHaltu, powod: impl Into<String>) {
        let p = powod.into();
        if p.is_empty() {
            return;
        }
        let stary = self.powod_mut(klasa);
        if stary.is_empty() {
            *stary = p;
        } else if !stary.contains(&p) {
            stary.push_str(HALT_SEP);
            stary.push_str(&p);
        }
        self.przelicz();
    }

    pub fn zdejmij(&mut self, klasa: KlasaHaltu) {
        self.powod_mut(klasa).clear();
        self.przelicz();
    }

    /// `active` i `reason` liczone ze składowych. Jedno źródło prawdy.
    pub fn przelicz(&mut self) {
        let mut czesci: Vec<String> = Vec::new();
        if !self.diagnoza.is_empty() {
            czesci.push(self.diagnoza.clone());
        }
        if !self.ryzyko.is_empty() {
            czesci.push(self.ryzyko.clone());
        }
        self.active = !czesci.is_empty();
        self.reason = czesci.join(HALT_SEP);
    }

    /// ZGODNOŚĆ WSTECZ: `latest.json` zapisany przez STARĄ binarkę.
    ///
    /// Taki plik ma `active` i `reason`, nie ma ani `diagnoza`, ani `ryzyko`.
    /// Rozpoznajemy go po sygnaturze „aktywny, a obie klasy puste" — nowy kod
    /// nigdy takiego zapisu nie wyprodukuje, bo `active` jest WYLICZANE
    /// z klas.
    ///
    /// # Dlaczego migracja po treści, a nie „brak pola = RYZYKO"
    ///
    /// Bo te dwa błędy nie kosztują tyle samo. Błędne uznanie za DIAGNOZĘ jest
    /// SAMONAPRAWIALNE: przyczyna jest sprawdzana przy starcie od nowa i jeśli
    /// dalej istnieje, zatrzymanie wraca z tym samym zdaniem, tylko świeżym.
    /// Błędne uznanie za RYZYKO jest PUŁAPKĄ: jedyne wyjście z niego rozbraja
    /// `max_dd_pct`, `max_dd_usd` i pułapy łańcucha — czyli za odklikanie
    /// fałszywej diagnozy użytkownik płaci całą strażą ryzyka. Dlatego RYZYKO
    /// jest domyślne (zachowawczo), ale zdania, które NA PEWNO wyprodukował
    /// nasz własny kod diagnostyczny, przechodzą na DIAGNOZĘ.
    ///
    /// Wystarczy JEDEN segment spoza zamkniętego zbioru, żeby całość poszła
    /// na RYZYKO — mieszany zapis („STORM: MAX DD 40% · Synergy: konfiguracja
    /// nie została wczytana") ma zostać potraktowany po najostrożniejszej
    /// stronie.
    pub fn migruj_stary_zapis(&mut self) -> bool {
        if !self.active || !self.diagnoza.is_empty() || !self.ryzyko.is_empty() {
            return false;
        }
        if self.reason.trim().is_empty() {
            // Zatrzymanie bez zapisanego powodu. Nie ma czego klasyfikować,
            // więc zostaje przy RYZYKU — z powodem nazywającym sytuację.
            self.ryzyko = "zatrzymanie zapisane przez starszą wersję (bez powodu)".into();
        } else if Self::cale_zdanie_diagnostyczne(&self.reason) {
            self.diagnoza = self.reason.clone();
        } else {
            self.ryzyko = self.reason.clone();
        }
        self.przelicz();
        true
    }

    fn cale_zdanie_diagnostyczne(reason: &str) -> bool {
        let mut byl_segment = false;
        for seg in reason.split(HALT_SEP) {
            let s = seg.trim();
            if s.is_empty() {
                continue;
            }
            byl_segment = true;
            if !ZDANIA_DIAGNOZY.iter().any(|m| s.contains(m)) {
                return false;
            }
        }
        byl_segment
    }

    pub fn wyprowadz_od_nowa(&mut self) -> Option<String> {
        self.migruj_stary_zapis();
        let stara = std::mem::take(&mut self.diagnoza);
        self.przelicz();
        (!stara.is_empty()).then_some(stara)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskOverride {
    pub active: bool,
    pub since: i64,
    pub reason: String,
}

// ============================================================
//  PEŁNY STAN
// ============================================================

/// Wszystko, co interfejs musi wiedzieć po podłączeniu.
///
/// `settings` i `sims` celowo są `serde_json::Value`: w React żyje 149 kluczy
/// konfiguracji odwzorowanych z `bot.py`, a `conduit_core::Settings` ma inny,
/// mniejszy zestaw. Serwer trzyma dokument UI w oryginale (to on jest zapisywany
/// do `settings.json`) i osobno wylicza z niego konfigurację silnika
/// (`settings_map::core_from_ui`). Gdybyśmy odwzorowywali 149 pól w Ruście,
/// każde nowe ustawienie wymagałoby zmiany w trzech miejscach.

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostepScalania {
    /// czy scalanie trwa W TEJ CHWILI
    pub aktywne: bool,
    /// `""` (nigdy nie uruchamiane) | `trwa` | `gotowe` | `blad`
    pub faza: String,
    /// co robi teraz — np. „dziennik decyzji · 2026-07-30.jsonl"
    pub etap: String,
    /// 0…1
    pub postep: f64,
    /// BAJTÓW wejścia przetworzonych (nie plików — sekcje są skrajnie
    /// nierówne i licznik plikowy stał w miejscu przez najdłuższą część pracy)
    pub zrobione: u64,
    /// bajtów wejścia do przetworzenia łącznie
    pub wszystkich: u64,
    /// czytelna prędkość, np. „14,2 MB/s"
    pub predkosc: String,
    /// szacowany czas do końca w ms; 0 = jeszcze nie wiadomo
    pub eta_ms: i64,
    pub czas_ms: i64,
    /// nazwa gotowego pliku (po zakończeniu)
    pub plik: String,
    /// rozmiar gotowego pliku w znakach
    pub znakow: u64,
    /// ile ŹRÓDEŁ weszło do pliku (dziennik, kronika, archiwum wiadomości…).
    /// Bez tej liczby podsumowanie „gotowe" nie odpowiada na jedyne pytanie,
    /// które ma sens po scaleniu: czy na pewno wszystko tam jest.
    #[serde(default)]
    pub zrodel: u64,
    /// ile źródeł ODRZUCONO (odznaczone w panelu albo puste) — bo „nie ma
    /// w pliku" i „nie było czego dołożyć" to dwie różne diagnozy.
    #[serde(default)]
    pub pominietych: u64,
    /// pełna ścieżka — do pokazania użytkownikowi, gdzie tego szukać
    pub sciezka: String,
    pub blad: Option<String>,
}

impl PostepScalania {
    /// Procent do pokazania na pasku. Osobno, żeby panel nie musiał pamiętać
    /// o mnożeniu i o tym, że przy nieznanej całości postęp to nie zero.
    pub fn procent(&self) -> f64 {
        (self.postep * 100.0).clamp(0.0, 100.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiSnapshot {
    pub rev: u64,
    pub server_time: i64,
    pub mode: TradingMode,
    pub connection: ConnectionState,
    pub quotes: BTreeMap<String, Quote>,
    pub positions: Vec<Position>,
    pub pendings: Vec<PendingOrder>,
    pub baskets: Vec<Basket>,
    pub closed: Vec<ClosedPosition>,
    pub pending_history: Vec<PendingHistoryItem>,
    /// Ile z tego, co widać na rachunku, NIE jest prowadzone przez bota.
    /// Zera oznaczają rachunek, na którym gra wyłącznie CONDUIT.
    #[serde(default)]
    pub foreign: ForeignSummary,
    pub balance: f64,
    pub stats: Stats,
    pub halt: HaltState,
    pub risk_override: RiskOverride,
    pub settings: serde_json::Value,
    pub lot: LotConfig,
    pub preset_id: String,
    pub messages: Vec<ChatMessage>,
    pub logs: Vec<LogEntry>,
    pub bindings: BTreeMap<String, ChannelBinding>,
    /// FORMATY SYGNAŁÓW — słownik, z którego panel bierze listę wyboru.
    /// Wbudowane plus (docelowo) własne użytkownika.
    #[serde(default = "conduit_core::formaty::formaty_wbudowane")]
    pub formaty: Vec<conduit_core::formaty::Format>,
    /// ŁAŃCUCHY: mapa `format → preset` plus pułapy obowiązujące ponad
    /// presetami. Zawsze dokładnie jeden jest aktywny i to on wyznacza,
    /// ile silników pracuje na rachunku.
    #[serde(default)]
    pub lancuchy: conduit_core::formaty::Lancuchy,
    /// WSKAŹNIK AKTYWNEGO ŁAŃCUCHA DLA TRYBU AUTO-EA (projekt EA-2).
    /// Pusty = brak własnego wskazania, czyli AUTO-EA gra tym samym, co reszta
    /// trybów (`lancuchy.aktywny`). Rozstrzyga [`aktywny_dla`].
    #[serde(default)]
    pub aktywny_ea: String,
    #[serde(default)]
    pub pieczec_lancuch: String,
    pub email: EmailConfig,
    pub notify: NotifyConfig,
    #[serde(default)]
    pub drabinka: DrabinkaLancuchow,
    /// DRABINKA ŁAŃCUCHÓW TRYBU AUTO-EA — „SKYNET-1" (nazwa właściciela).
    ///
    /// Przełącza WYŁĄCZNIE `aktywny_ea`, czyli skład warstwy EA. Kontrakt
    /// zera: instalacja sprzed EA-2c nie ma tego pola w `backup_memory`, więc
    /// wchodzi pusta i wyłączona ([`drabinka_ea_domyslna`]) — AUTO-EA
    /// zachowuje się dokładnie jak przed podmianą binarki.
    #[serde(default = "drabinka_ea_domyslna")]
    pub drabinka_ea: DrabinkaLancuchow,
    pub sims: Vec<serde_json::Value>,
    pub favorites: Vec<String>,
    /// Język interfejsu (`"en"`/`"pl"`) — front czyta z migawki przy starcie
    /// (`B.language`), zapisuje łatką ustawień; klucz GŁÓWNY, poza `settings`.
    #[serde(default = "jezyk_domyslny_ui")]
    pub language: String,
    pub auth: crate::auth::AuthState,
    /// Postęp backtestów i treningu AI. Trzymamy go w stanie, a nie w osobnym
    /// kanale, bo dzięki temu karta otwarta w połowie dziesięciominutowego
    /// przemiału od razu widzi, co się liczy — zamiast czekać na pierwszą deltę.
    #[serde(default)]
    pub lab: crate::lab::LabState,
    #[serde(default)]
    pub scalanie: PostepScalania,
    /// Stan trybu demo: wirtualny broker, zegar odtwarzania, wybrane pliki.
    /// Trzymamy go w stanie, a nie tylko w REST-cie, bo demo zmienia pozycje
    /// i saldo — panel musi wiedzieć, że patrzy na symulację, nie na konto.
    #[serde(default)]
    pub demo: crate::demo::DemoState,
}

impl UiSnapshot {
    pub fn empty(now: i64) -> Self {
        UiSnapshot {
            rev: 0,
            server_time: now,
            mode: TradingMode::Auto,
            connection: ConnectionState::default(),
            quotes: BTreeMap::new(),
            positions: Vec::new(),
            pendings: Vec::new(),
            baskets: Vec::new(),
            closed: Vec::new(),
            pending_history: Vec::new(),
            foreign: ForeignSummary::default(),
            balance: 0.0,
            stats: Stats::new(0.0, now),
            halt: HaltState::default(),
            risk_override: RiskOverride::default(),
            settings: serde_json::Value::Object(Default::default()),
            lot: LotConfig::default(),
            preset_id: String::new(),
            messages: Vec::new(),
            logs: Vec::new(),
            bindings: BTreeMap::new(),
            formaty: conduit_core::formaty::formaty_wbudowane(),
            lancuchy: conduit_core::formaty::Lancuchy::default(),
            aktywny_ea: String::new(),
            pieczec_lancuch: String::new(),
            email: EmailConfig::default(),
            notify: NotifyConfig::default(),
            drabinka: DrabinkaLancuchow::default(),
            drabinka_ea: drabinka_ea_domyslna(),
            sims: Vec::new(),
            favorites: Vec::new(),
            language: jezyk_domyslny_ui(),
            auth: crate::auth::AuthState::default(),
            lab: crate::lab::LabState::default(),
            scalanie: PostepScalania::default(),
            demo: crate::demo::DemoState::default(),
        }
    }

    /// Nazwa łańcucha, którym bot gra W TYM STANIE — z uwzględnieniem trybu.
    /// Jedyne miejsce, o które warstwy wyżej mają pytać (projekt EA-2).
    pub fn aktywny_lancuch_nazwa(&self) -> &str {
        aktywny_dla(&self.lancuchy, &self.aktywny_ea, self.mode)
    }

    /// Ten sam łańcuch jako rekord z listy; `None` = wskazanie w pustkę.
    pub fn aktywny_lancuch(&self) -> Option<&conduit_core::formaty::Lancuch> {
        lancuch_dla(&self.lancuchy, &self.aktywny_ea, self.mode)
    }

    /// DRABINKA OBOWIĄZUJĄCA W TYM TRYBIE (projekt EA-2c). Jedyne miejsce,
    /// o które warstwy wyżej mają pytać — `s.drabinka` czytane wprost jest
    /// odpowiedzią na inne pytanie („co ma AUTO"), nie na to.
    pub fn drabinka_biezaca(&self) -> &DrabinkaLancuchow {
        match KtoraDrabinka::dla(self.mode) {
            KtoraDrabinka::Wspolna => &self.drabinka,
            KtoraDrabinka::Ea => &self.drabinka_ea,
        }
    }

    /// To samo do zapisu — używa tego krok drabinki (`biezacy_prog`,
    /// `ostatnia_zmiana_ts`), żeby postęp jednego trybu nie zapisywał się
    /// w pamięci drugiego.
    pub fn drabinka_biezaca_mut(&mut self) -> &mut DrabinkaLancuchow {
        match KtoraDrabinka::dla(self.mode) {
            KtoraDrabinka::Wspolna => &mut self.drabinka,
            KtoraDrabinka::Ea => &mut self.drabinka_ea,
        }
    }

    /// Drabinka WSKAZANA Z NAZWY — do komend, które mówią wprost, czego
    /// dotyczą (`SetDrabinka { tryb }`).
    pub fn drabinka_mut(&mut self, ktora: KtoraDrabinka) -> &mut DrabinkaLancuchow {
        match ktora {
            KtoraDrabinka::Wspolna => &mut self.drabinka,
            KtoraDrabinka::Ea => &mut self.drabinka_ea,
        }
    }
}

/// IZOLACJA DRABINEK — dokładnie jedna z nich jest SKUTECZNA: ta, która
/// należy do bieżącego trybu (projekt EA-2c).
///
/// Wołać po KAŻDEJ zmianie trybu i po każdym zapisie drabinki, oraz raz przy
/// starcie (po wznowieniu `backup_memory`). Funkcja jest idempotentna.
///
/// # Dlaczego to jest tu, a nie „w pętli, która i tak pyta o tryb"
///
/// Bo pytać o tryb musiałby KAŻDY czytelnik `drabinka.enabled` — dziś jest
/// ich pięciu w trzech skrzyniach (pętla handlowa, lista szczebli dla panelu,
/// pieczęć paczki, synchronizacja przy włączeniu, sam panel), a jutro będzie
/// szósty, który zapomni. Gasząc nie-swoją drabinkę u ŹRÓDŁA sprawiamy, że
/// czytelnik, który o trybie nie wie, i tak nie może zrobić szkody: dostaje
/// drabinkę wyłączoną, czyli bezczynną. Wyłącznik użytkownika zostaje
/// nietknięty w `wlacznik` i wraca, gdy wróci jego tryb.
pub fn przelicz_izolacje_drabinek(s: &mut UiSnapshot) {
    let ea = KtoraDrabinka::dla(s.mode) == KtoraDrabinka::Ea;
    s.drabinka.ustaw_skutecznosc(!ea);
    s.drabinka_ea.ustaw_skutecznosc(ea);
}

// ============================================================
//  ZDARZENIA (nie koalescencjonowane — lecą natychmiast)
// ============================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum UiEvent {
    /// nowa wiadomość / rozpoznany sygnał
    Signal { message: Box<ChatMessage> },
    /// wypełnienie zlecenia
    Fill {
        ticket: u64,
        price: f64,
        volume: f64,
        direction: Direction,
    },
    /// zamknięcie pozycji
    Closed { trade: Box<ClosedPosition> },
    /// komunikat do wyświetlenia (toast w UI)
    Toast {
        level: String,
        title: String,
        text: String,
    },
    /// wpis dziennika
    Log { entry: Box<LogEntry> },
    /// alarm strażnika ryzyka
    Alert { reason: String, halted: bool },
}

// ============================================================
//  MAPOWANIE Z RDZENIA
// ============================================================

/// Mapuje pozycję z rdzenia na model UI. Zysk podajemy z zewnątrz, bo rdzeń
/// go nie trzyma — liczy się go z bieżącego kwotowania.
pub fn position_from_core(p: &conduit_core::Position, symbol: &str, profit: f64) -> Position {
    Position {
        ticket: p.ticket,
        symbol: symbol.to_string(),
        direction: p.side.into(),
        volume: p.volume,
        open_price: p.open_price,
        open_time: p.open_ts,
        sl: p.sl,
        tp: p.tp,
        vsl: p.vsl,
        profit,
        swap: 0.0,
        commission: 0.0,
        comment: p.comment.clone(),
        // Rdzeń nie zna `magic` — i nie ma go skąd znać. NIE podstawiamy
        // tu `mt5_magic`: tę samą funkcję woła tryb demo, gdzie gra
        // wirtualny broker i żadnego `magic` nie ma.
        magic: None,
        basket_id: p.basket,
        level: p.level,
        frozen: p.frozen,
        peak_pts: p.peak_pts,
        runner: p.is_runner,
        toucher: p.is_toucher,
        last_peak_time: p.last_peak_ts,
        source: Origin::Bot,
    }
}

pub fn pending_from_core(o: &conduit_core::PendingOrder, symbol: &str) -> PendingOrder {
    PendingOrder {
        ticket: o.ticket,
        symbol: symbol.to_string(),
        kind: o.kind.into(),
        volume: o.volume,
        price: o.price,
        sl: o.sl,
        tp: o.tp,
        placed_time: o.placed_ts,
        comment: o.comment.clone(),
        basket_id: o.basket,
        level: o.level,
        frozen: o.frozen,
        source: Origin::Bot,
        magic: None,
    }
}

pub fn basket_from_core(b: &conduit_core::Basket, symbol: &str) -> Basket {
    Basket {
        id: b.id,
        symbol: symbol.to_string(),
        direction: b.side.into(),
        is_limit: b.is_limit,
        entry_low: b.entry_lo,
        entry_high: b.entry_hi,
        zone_low: b.zone_lo,
        zone_high: b.zone_hi,
        sl: b.sl,
        tps: b.tps.clone(),
        tp_stage: b.tp_stage,
        created_at: b.created_ts,
        source: b.source_name.clone(),
        source_key: b.source.as_string(),
        active: b.alive(),
        tickets: b.tickets.clone(),
        pending_tickets: b.pendings.clone(),
        events: b
            .events
            .iter()
            .map(|e| BasketEvent {
                t: e.ts,
                text: e.text.clone(),
                kind: "info".into(),
            })
            .collect(),
        risk_free: matches!(b.state, conduit_core::BasketState::RiskFree),
    }
}

pub fn closed_from_core(c: &conduit_core::ClosedTrade, symbol: &str) -> ClosedPosition {
    ClosedPosition {
        ticket: c.ticket,
        symbol: symbol.to_string(),
        direction: c.side.into(),
        volume: c.volume,
        open_price: c.open_price,
        close_price: c.close_price,
        open_time: c.open_ts,
        close_time: c.close_ts,
        profit: c.profit,
        swap: c.swap,
        commission: c.commission,
        reason: c.reason.into(),
        comment: String::new(),
        basket_id: c.basket,
        source: Origin::Bot,
        magic: None,
    }
}

// ============================================================
//  TESTY — MIGRACJA POWIAZAN KANALOW
// ============================================================

#[cfg(test)]
mod testy_powiazan {
    use super::*;

    #[test]
    fn stary_ksztalt_z_lista_wczytuje_sie_jako_jeden_format() {
        let stary = serde_json::json!({
            "channelId": -1001,
            "monitored": true,
            "notify": false,
            "formats": ["ATFX"],
            "topics": { "3": ["Synergy"] }
        });
        let b: ChannelBinding = serde_json::from_value(stary).unwrap();
        assert_eq!(b.format, "ATFX");
        assert_eq!(b.topics.get("3").map(String::as_str), Some("Synergy"));
    }

    /// Pusta lista znaczy „ten kanal nie handluje" - stan POPRAWNY, nie
    /// usterka: nowe zrodlo warto najpierw obserwowac.
    #[test]
    fn pusta_lista_znaczy_brak_formatu() {
        let b: ChannelBinding = serde_json::from_value(serde_json::json!({
            "channelId": -1002, "monitored": true, "notify": false,
            "formats": [], "topics": {}
        }))
        .unwrap();
        assert_eq!(b.format, "");
        assert!(b.topics.is_empty());
    }

    /// Dwa formaty na jednym kanale to podwojna ekspozycja z jednego sygnalu.
    /// Bierzemy pierwszy - ale nie wolno tego przemilczec (patrz
    /// `Workspace::load_channels_z_ostrzezeniami`).
    #[test]
    fn kilka_formatow_daje_pierwszy() {
        let b: ChannelBinding = serde_json::from_value(serde_json::json!({
            "channelId": -1003, "monitored": true, "notify": false,
            "formats": ["Synergy", "ATFX"], "topics": { "7": ["A", "B"] }
        }))
        .unwrap();
        assert_eq!(b.format, "Synergy");
        assert_eq!(b.topics.get("7").map(String::as_str), Some("A"));
    }

    /// Nowy ksztalt (napis) czyta sie bez zmian - to jest docelowa postac.
    #[test]
    fn nowy_ksztalt_z_napisem_dziala_wprost() {
        let b: ChannelBinding = serde_json::from_value(serde_json::json!({
            "channelId": -1004, "monitored": true, "notify": true,
            "format": "Synergy", "topics": { "1": "ATFX" }
        }))
        .unwrap();
        assert_eq!(b.format, "Synergy");
        assert_eq!(b.topics.get("1").map(String::as_str), Some("ATFX"));
    }

    /// Brak obu kluczy (plik sprzed pola `formats`) nie moze wywalic startu.
    #[test]
    fn brak_pola_to_brak_formatu_a_nie_blad() {
        let b: ChannelBinding = serde_json::from_value(serde_json::json!({
            "channelId": -1005, "monitored": false, "notify": false
        }))
        .unwrap();
        assert_eq!(b.format, "");
    }

    /// Runda w obie strony: to, co zapiszemy, musi sie wczytac tak samo.
    #[test]
    fn zapis_i_odczyt_daja_to_samo() {
        let b = ChannelBinding {
            channel_id: -1006,
            monitored: true,
            notify: false,
            format: "ATFX".into(),
            topics: [("2".to_string(), "Synergy".to_string())]
                .into_iter()
                .collect(),
        };
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(v["format"], "ATFX", "na drut idzie NAPIS, nie lista");
        let z: ChannelBinding = serde_json::from_value(v).unwrap();
        assert_eq!(z, b);
    }
}

// ============================================================
//  TESTY ROUTINGU ŹRÓDEŁ — wyłącznie syntetyczne identyfikatory
// ============================================================
#[cfg(test)]
mod testy_routingu_zrodel {
    use super::*;

    /// Syntetyczna grupa forum:
    /// ZEN i NOVA to DWA TEMATY tej samej grupy, PulseX to trzeci temat,
    /// którego świadomie NIE podpinamy. Synergy to osobny, zwykły kanał.
    fn forum_pro_trader() -> ChannelBinding {
        ChannelBinding {
            channel_id: 9_000_000_101,
            monitored: true,
            notify: false,
            format: String::new(), // forum nie ma formatu kanałowego
            topics: [
                (2.to_string(), "ZEN".to_string()),
                (37.to_string(), "NOVA".to_string()),
            ]
            .into_iter()
            .collect(),
        }
    }

    fn kanal_synergy() -> ChannelBinding {
        ChannelBinding {
            channel_id: 9_000_000_202,
            monitored: true,
            notify: false,
            format: "Synergy".to_string(),
            topics: Default::default(),
        }
    }

    #[test]
    fn dwa_tematy_jednej_grupy_dostaja_rozne_formaty() {
        let b = forum_pro_trader();
        assert_eq!(b.format_dla(Some(2)).as_deref(), Some("ZEN"));
        assert_eq!(b.format_dla(Some(37)).as_deref(), Some("NOVA"));
    }

    #[test]
    fn temat_niepodpiety_nie_handluje_chocbys_podpial_sasiednie() {
        // PulseX (temat 99) istnieje na forum, ale nie ma przypisanego
        // formatu. Wiadomość stamtąd nie może dostać formatu ZEN ani NOVA
        // „przez sąsiedztwo" — to byłby handel na niezbadanym formacie.
        let b = forum_pro_trader();
        assert_eq!(b.format_dla(Some(99)), None);
        assert!(
            !b.obserwuje(Some(99)),
            "temat spoza mapy nie jest nawet nasłuchiwany"
        );
        assert!(b.obserwuje(Some(2)), "podpięty temat jest nasłuchiwany");
    }

    #[test]
    fn forum_bez_tematu_nie_wpada_w_format_kanalowy() {
        // Wiadomość forum bez tematu (nie powinno się zdarzyć — warstwa
        // Telegrama domyka temat ogólny) nie może dostać formatu, bo pole
        // kanałowe forum jest puste. Cisza, nie zgadywanie.
        let b = forum_pro_trader();
        assert_eq!(b.format_dla(None), None);
    }

    #[test]
    fn zwykly_kanal_gra_formatem_kanalu() {
        let b = kanal_synergy();
        assert_eq!(b.format_dla(None).as_deref(), Some("Synergy"));
    }

    #[test]
    fn zwykly_kanal_z_nagle_tematem_dalej_gra_formatem_kanalu() {
        // Kanał może stać się forum PO konfiguracji. Pusta mapa tematów
        // znaczy „całe źródło gra formatem kanału" — wiadomość z tematem
        // nie może przez to zamilknąć.
        let b = kanal_synergy();
        assert_eq!(b.format_dla(Some(123)).as_deref(), Some("Synergy"));
        assert!(b.obserwuje(Some(123)));
    }

    #[test]
    fn wylaczony_nasluch_wylacza_wszystko() {
        let mut b = forum_pro_trader();
        b.monitored = false;
        assert!(!b.obserwuje(Some(2)));
        assert!(!b.obserwuje(None));
    }
}

#[cfg(test)]
mod testy_kotwic_konta {
    use super::*;

    #[test]
    fn zmiana_konta_zeruje_kotwice_dnia_i_sesji() {
        let mut s = Stats::new(200.0, 1_000);
        // pierwsza publikacja: Vantage — przypisanie bez zerowania
        assert!(!s.przelacz_konto("10000001@Vantage-PUBLIC-DEMO", 406.56, 100, 2_000));
        s.session_start_equity = 406.56;
        s.day_start_equity = 406.56;
        s.peak_equity_today = 420.0;
        s.max_dd_today = 55.0;
        s.pnl_session = -107.80;
        s.pnl_today = -3.0;

        // ta sama para login@serwer — nic się nie dzieje
        assert!(!s.przelacz_konto("10000001@Vantage-PUBLIC-DEMO", 298.76, 101, 3_000));
        assert_eq!(s.session_start_equity, 406.56);

        // PUPrime — dzień i sesja od bieżącego equity, obsunięcie od zera
        assert!(s.przelacz_konto("10000002@PUPrime-PUBLIC-DEMO", 300.0, 102, 4_000));
        assert_eq!(s.session_start_equity, 300.0);
        assert_eq!(s.day_start_equity, 300.0);
        assert_eq!(s.peak_equity_today, 300.0);
        assert_eq!(s.max_dd_today, 0.0);
        assert_eq!(s.pnl_session, 0.0);
        assert_eq!(s.pnl_today, 0.0);
        assert_eq!(s.day_key, 102);
        assert_eq!(
            s.equity_curve.len(),
            1,
            "krzywa zaczyna się od jednego punktu"
        );

        // ⚠ POWRÓT na stare konto NIE odzyskuje tamtych kotwic — świadomie:
        // mapa kotwic per konto to struktura, której nikt nie sprząta,
        // a „dzień wznowiony" po tygodniu przerwy kłamałby tak samo.
        assert!(s.przelacz_konto("10000001@Vantage-PUBLIC-DEMO", 298.76, 103, 5_000));
        assert_eq!(
            s.session_start_equity, 298.76,
            "stara kotwica 406,56 NIE wraca"
        );
    }

    /// Ten sam login u DWÓCH brokerów to dwa różne rachunki — klucz musi
    /// nieść serwer, nie sam numer.
    #[test]
    fn ten_sam_login_inny_serwer_to_inne_konto() {
        let mut s = Stats::new(200.0, 1_000);
        assert!(!s.przelacz_konto("123@Alfa-Demo", 200.0, 1, 1_000));
        assert!(s.przelacz_konto("123@Beta-Demo", 500.0, 2, 2_000));
        assert_eq!(s.day_start_equity, 500.0);
    }
}

#[cfg(test)]
mod testy_drabinki {
    use super::*;

    fn ffss0_wlaczona() -> DrabinkaLancuchow {
        DrabinkaLancuchow {
            enabled: true,
            wlacznik: Some(true),
            szczeble: vec![
                SzczebelDrabinki {
                    prog_balance: 0.0,
                    lancuch: "ZENONLY5".into(),
                },
                SzczebelDrabinki {
                    prog_balance: 500.0,
                    lancuch: "ZENONLY3".into(),
                },
                SzczebelDrabinki {
                    prog_balance: 2000.0,
                    lancuch: "SENTINEL-0C".into(),
                },
            ],
            histereza_pct: 2.0,
            biezacy_prog: -1.0,
            ostatnia_zmiana_ts: 0,
        }
    }

    #[test]
    fn ffs1c_ma_szczeble_uzytkownika() {
        let n = drabinki_wbudowane();
        let d = n
            .iter()
            .find(|x| x.nazwa.starts_with("FFS-1C"))
            .expect("FFS-1C musi zostać na liście nazwanych drabinek");
        let pary: Vec<(f64, &str)> = d
            .szczeble
            .iter()
            .map(|s| (s.prog_balance, s.lancuch.as_str()))
            .collect();
        assert_eq!(
            pary,
            vec![
                (0.0, "ZENONLY5"),
                (500.0, "ZENONLY3"),
                (2000.0, "SENTINEL-0C")
            ]
        );
        assert_eq!(d.histereza_pct, 2.0);
        assert!(!d.korona, "FFS-1C to pozycja historyczna, nie korona");
    }

    /// Wbudowana domyślna odpowiada KORONIE (uzasadnienie przy `impl
    /// Default`): jeden szczebel od pierwszego dolara, wyłączona, histereza
    /// 2 %. Nazwy szczebla NIE wbijamy — pilnuje jej zgodność z pozycją
    /// koronną `drabinki_wbudowane()`, więc zmiana czempiona nie zabija testu.
    #[test]
    fn domyslna_drabinka_odpowiada_koronie() {
        let d = DrabinkaLancuchow::default();
        assert!(
            !d.enabled,
            "wbudowana drabinka jest WYŁĄCZONA — włącza ją człowiek"
        );
        assert_eq!(d.szczeble.len(), 1, "domyślna = jeden szczebel korony");
        assert_eq!(d.szczeble[0].prog_balance, 0.0);
        assert_eq!(d.histereza_pct, 2.0);
        let korona = drabinki_wbudowane()
            .into_iter()
            .find(|x| x.korona)
            .expect("lista nazwanych drabinek musi mieć koronę");
        assert_eq!(
            d.szczeble[0].lancuch, korona.szczeble[0].lancuch,
            "domyślna i korona muszą wskazywać ten sam łańcuch startowy"
        );
    }

    #[test]
    fn wybor_po_balance_w_gore_natychmiast() {
        let d = ffss0_wlaczona();
        assert_eq!(d.wybierz(499.99).unwrap().lancuch, "ZENONLY5");
        assert_eq!(
            d.wybierz(500.0).unwrap().lancuch,
            "ZENONLY3",
            "próg jest domknięty (≥)"
        );
        assert_eq!(
            d.wybierz(1999.99).unwrap().lancuch,
            "ZENONLY3",
            "Synergy dopiero od 2000"
        );
        assert_eq!(d.wybierz(2000.0).unwrap().lancuch, "SENTINEL-0C");
        assert_eq!(d.wybierz(99_999.0).unwrap().lancuch, "SENTINEL-0C");
    }

    /// FLAPPING 498↔502 przy histerezie 2 % — scenariusz z baterii dowodowej.
    ///
    /// Stoimy na szczeblu 500 (ZENONLY3). Zejście wymaga balance <
    /// 500·(1−2 %) = 490. Oscylacja 498↔502 NIE MA PRAWA przełączyć niczego.
    #[test]
    fn flapping_498_502_przy_histerezie_2pct_zero_przelaczen() {
        let mut d = ffss0_wlaczona();
        d.biezacy_prog = 500.0; // drabinka stoi na ZENONLY3
        for balance in [498.0, 502.0, 498.0, 502.0, 499.9, 501.1] {
            let w = d.wybierz(balance).unwrap();
            assert_eq!(
                w.lancuch, "ZENONLY3",
                "balance {balance}: histereza 2 % ma trzymać szczebel 500"
            );
        }
        // dopiero WYRAŹNY spadek schodzi — i to o szczebel właściwy
        assert_eq!(d.wybierz(489.9).unwrap().lancuch, "ZENONLY5");
        // powrót w górę: natychmiast, bez histerezy
        d.biezacy_prog = 0.0;
        assert_eq!(d.wybierz(502.0).unwrap().lancuch, "ZENONLY3");
    }

    /// Histereza 0 % = przełącza od razu, ale ZAWSZE spójnie (bez utknięcia
    /// między szczeblami) — druga połowa scenariusza flappingu.
    #[test]
    fn histereza_zero_przelacza_natychmiast_i_spojnie() {
        let mut d = ffss0_wlaczona();
        d.histereza_pct = 0.0;
        d.biezacy_prog = 500.0;
        assert_eq!(
            d.wybierz(499.99).unwrap().lancuch,
            "ZENONLY5",
            "pod progiem od razu w dół"
        );
        assert_eq!(
            d.wybierz(500.0).unwrap().lancuch,
            "ZENONLY3",
            "na progu od razu w górę"
        );
    }

    /// Restart w środku drabinki: `biezacy_prog` wraca z backupu, więc konto
    /// 1200 wstaje na SENTINEL-0 i histereza liczy się od właściwego progu.
    #[test]
    fn restart_w_srodku_drabinki_nie_oscyluje() {
        let mut d = ffss0_wlaczona();
        d.biezacy_prog = 2000.0; // stan z backupu
        assert_eq!(d.wybierz(2400.0).unwrap().lancuch, "SENTINEL-0C");
        // tuż pod progiem, ale nad granicą histerezy (1960): zostajemy
        assert_eq!(d.wybierz(1990.0).unwrap().lancuch, "SENTINEL-0C");
        // pod granicą histerezy: legalne zejście
        assert_eq!(d.wybierz(1959.0).unwrap().lancuch, "ZENONLY3");
    }

    /// WALIDACJA DRABINKI — progi ściśle rosnące, szczebel bazowy, dowolna
    /// liczba szczebli. Test pilnuje KOMUNIKATÓW, nie tylko odmowy: użytkownik
    /// ma z panelu wiedzieć, co poprawić, bez czytania kodu.
    #[test]
    fn drabinka_wymaga_progow_scisle_rosnacych_i_szczebla_bazowego() {
        let znane: Vec<String> = ["A", "B", "C"].iter().map(|s| s.to_string()).collect();
        let sz = |p: f64, l: &str| SzczebelDrabinki {
            prog_balance: p,
            lancuch: l.into(),
        };
        let drab = |szczeble: Vec<SzczebelDrabinki>| DrabinkaLancuchow {
            enabled: true,
            wlacznik: Some(true),
            szczeble,
            histereza_pct: 2.0,
            biezacy_prog: -1.0,
            ostatnia_zmiana_ts: 0,
        };

        // DOBRE: rosnące od zera, dowolna długość (tu siedem szczebli —
        // sztywnego limitu nie ma i ten test jest tego dowodem)
        let dobra = drab(vec![
            sz(0.0, "A"),
            sz(300.0, "B"),
            sz(500.0, "C"),
            sz(800.0, "A"),
            sz(1000.0, "B"),
            sz(1500.0, "C"),
            sz(2500.0, "A"),
        ]);
        assert!(
            dobra.sprawdz(&znane).is_ok(),
            "siedem rosnących szczebli musi przejść"
        );

        // DUPLIKAT progu
        let e = drab(vec![sz(0.0, "A"), sz(500.0, "B"), sz(500.0, "C")])
            .sprawdz(&znane)
            .unwrap_err();
        assert!(
            e.contains("ten sam próg"),
            "komunikat ma nazwać duplikat: {e}"
        );

        // PRÓG MNIEJSZY OD POPRZEDNIEGO — bez cichego sortowania
        let e = drab(vec![sz(0.0, "A"), sz(900.0, "B"), sz(400.0, "C")])
            .sprawdz(&znane)
            .unwrap_err();
        assert!(e.contains("MNIEJSZY"), "komunikat ma nazwać kolejność: {e}");

        // BRAK SZCZEBLA BAZOWEGO
        let e = drab(vec![sz(500.0, "A"), sz(900.0, "B")])
            .sprawdz(&znane)
            .unwrap_err();
        assert!(e.contains("BAZOWEGO"), "komunikat ma nazwać brak zera: {e}");

        // NIEISTNIEJĄCY ŁAŃCUCH
        let e = drab(vec![sz(0.0, "A"), sz(500.0, "NIE-MA")])
            .sprawdz(&znane)
            .unwrap_err();
        assert!(e.contains("NIE-MA"), "komunikat ma zacytować nazwę: {e}");

        // PUSTA: wyłączona wolno, włączona nie
        let mut pusta = drab(vec![]);
        assert!(
            pusta.sprawdz(&znane).is_err(),
            "włączona pusta drabinka nie ma czego wybrać"
        );
        pusta.enabled = false;
        assert!(
            pusta.sprawdz(&znane).is_ok(),
            "wyłączona pusta jest stanem poprawnym"
        );

        // WBUDOWANA FFS-1C musi przechodzić własną walidację — inaczej
        // domyślna konfiguracja programu byłaby nie do zapisania z panelu.
        let ffss = DrabinkaLancuchow::default();
        let wbudowane: Vec<String> = conduit_core::formaty::lancuchy_wbudowane()
            .iter()
            .map(|l| l.nazwa.clone())
            .collect();
        assert!(
            ffss.sprawdz(&wbudowane).is_ok(),
            "FFS-1C musi przechodzić walidację"
        );
    }

    #[test]
    fn wylaczona_albo_pusta_nie_wybiera_niczego() {
        let d = DrabinkaLancuchow::default(); // enabled=false
        assert!(d.wybierz(1000.0).is_none());
        let mut d2 = ffss0_wlaczona();
        d2.szczeble.clear();
        assert!(d2.wybierz(1000.0).is_none());
    }
}

fn jezyk_domyslny_ui() -> String {
    "en".into()
}
