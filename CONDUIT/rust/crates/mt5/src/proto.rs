//! Protokół liniowy między Rustem a sidecarem Pythona.
//!
//! Jedna linia = jeden dokument JSON zakończony `\n`. Kodowanie UTF-8.
//! Trzy rodzaje ramek:
//!
//!  * **żądanie** (Rust → sidecar): `{"id":7,"cmd":"account","args":{...}}`
//!  * **odpowiedź** (sidecar → Rust): `{"id":7,"ok":true,"result":{...}}`
//!    albo `{"id":7,"ok":false,"error":{"code":10016,"msg":"Invalid stops"}}`
//!  * **zdarzenie** (sidecar → Rust, bez `id`): `{"ev":"tick","bid":...,"ask":...}`
//!
//! Zdarzenia i odpowiedzi lecą tym samym strumieniem, dlatego rozróżnia je
//! obecność pola `id`. Ramka bez `id` i bez `ev` jest błędem protokołu — nie
//! wolno jej po cichu zignorować, bo to znaczy, że sidecar i Rust rozjechały
//! się wersjami.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Wersja protokołu. Sidecar podaje swoją w ramce `hello`; różnica = twardy błąd.
pub const PROTO_VERSION: u32 = 1;

// ============================================================
//  ŻĄDANIA
// ============================================================

#[derive(Debug, Clone, Serialize)]
pub struct Request {
    pub id: u64,
    pub cmd: &'static str,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub args: Value,
}

impl Request {
    pub fn new(id: u64, cmd: &'static str, args: Value) -> Self {
        Request { id, cmd, args }
    }

    /// Serializuje do pojedynczej linii zakończonej `\n`.
    pub fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).expect("Request zawsze serializowalny");
        s.push('\n');
        s
    }
}

// ============================================================
//  RAMKI PRZYCHODZĄCE
// ============================================================

/// Błąd zwrócony przez sidecar. `code` to retcode MT5 (10004, 10016, …) albo
/// kod ujemny dla błędów po stronie samego sidecara.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireError {
    pub code: i64,
    #[serde(default)]
    pub msg: String,
}

/// Kody własne sidecara (poza przestrzenią retcodes MT5, które są dodatnie).
pub mod local_code {
    /// terminal nieosiągalny / `initialize()` się nie powiodło
    pub const NOT_INITIALIZED: i64 = -1;
    /// nieznana komenda — rozjazd wersji
    pub const UNKNOWN_CMD: i64 = -2;
    /// zła treść żądania
    pub const BAD_ARGS: i64 = -3;
    /// nie ma takiego tiketu na koncie
    pub const NO_TICKET: i64 = -4;
    /// wyjątek Pythona
    pub const EXCEPTION: i64 = -5;
}

#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// odpowiedź na żądanie o danym `id`
    Response {
        id: u64,
        result: Result<Value, WireError>,
    },
    /// zdarzenie strumieniowe
    Event { kind: String, body: Value },
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProtoError {
    #[error("niepoprawny JSON w linii: {0}")]
    BadJson(String),
    #[error("ramka bez pola `id` ani `ev`: {0}")]
    Unknown(String),
    #[error("odpowiedź ok=false bez pola `error`")]
    MissingError,
}

/// Parsuje jedną linię strumienia. Pusta linia (albo sam biały znak) → `None`;
/// sidecar wysyła je jako keepalive przy długiej ciszy.
pub fn parse_line(line: &str) -> Result<Option<Frame>, ProtoError> {
    let t = line.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let v: Value = serde_json::from_str(t).map_err(|e| ProtoError::BadJson(e.to_string()))?;

    if let Some(id) = v.get("id").and_then(|x| x.as_u64()) {
        let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
        if ok {
            let result = v.get("result").cloned().unwrap_or(Value::Null);
            return Ok(Some(Frame::Response {
                id,
                result: Ok(result),
            }));
        }
        let err = v.get("error").ok_or(ProtoError::MissingError)?;
        let we: WireError =
            serde_json::from_value(err.clone()).map_err(|e| ProtoError::BadJson(e.to_string()))?;
        return Ok(Some(Frame::Response {
            id,
            result: Err(we),
        }));
    }

    if let Some(ev) = v.get("ev").and_then(|x| x.as_str()) {
        return Ok(Some(Frame::Event {
            kind: ev.to_string(),
            body: v,
        }));
    }

    Err(ProtoError::Unknown(t.chars().take(200).collect()))
}

// ============================================================
//  ŁADUNKI (kształt danych z sidecara)
// ============================================================

/// Powitanie sidecara — wysyłane raz, zaraz po nawiązaniu połączenia.
#[derive(Debug, Clone, Deserialize)]
pub struct Hello {
    pub proto: u32,
    #[serde(default)]
    pub sidecar: String,
    #[serde(default)]
    pub mt5_version: String,
    /// New sidecars certify successful initialization explicitly. Legacy v1
    /// is accepted only with a nonempty terminal version, never a failed hello.
    #[serde(default)]
    pub ready: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, thiserror::Error)]
#[error("MT5: start nieudany [{stage}], kod {code}: {msg}")]
pub struct StartupFailure {
    pub stage: String,
    pub code: i64,
    pub msg: String,
}

/// Parametry instrumentu — WSZYSTKIE z serwera, żadnej wartości zaszytej w kodzie.
#[derive(Debug, Clone, Deserialize)]
pub struct SymbolInfo {
    pub symbol: String,
    pub digits: u32,
    pub point: f64,
    /// `SYMBOL_TRADE_STOPS_LEVEL` w punktach
    pub stops_level_points: f64,
    /// `SYMBOL_TRADE_FREEZE_LEVEL` w punktach
    #[serde(default)]
    pub freeze_level_points: f64,
    pub volume_min: f64,
    pub volume_max: f64,
    pub volume_step: f64,
    pub contract_size: f64,
    /// maska `SYMBOL_FILLING_MODE`
    #[serde(default)]
    pub filling_mask: u32,
    /// wybrany przez sidecar tryb wypełnienia dla zleceń rynkowych
    #[serde(default)]
    pub filling_market: u32,
    /// tryb wypełnienia dla zleceń oczekujących
    #[serde(default)]
    pub filling_pending: u32,
    #[serde(default)]
    pub trade_mode: i32,
    /// czy instrument jest w Podglądzie rynku — niewidoczny potrafi oddać
    /// parametry i nie oddać świec
    #[serde(default = "domyslnie_widoczny")]
    pub visible: bool,
    #[serde(default)]
    pub description: String,

    // --- KOSZT PRZETRZYMANIA ---
    /// `SYMBOL_SWAP_LONG` — koszt nocny pozycji długiej, w jednostce zależnej
    /// od [`SymbolInfo::swap_mode`]
    #[serde(default)]
    pub swap_long: f64,
    /// `SYMBOL_SWAP_SHORT`
    #[serde(default)]
    pub swap_short: f64,
    /// `SYMBOL_SWAP_MODE`. 1 = punkty (tak jest na Vantage). Inne tryby
    /// (procent, waluta bazowa, odsetki) liczy się INACZEJ — dlatego pole
    /// jedzie razem z wartościami, a nie jest zakładane.
    #[serde(default)]
    pub swap_mode: i32,
    /// Dzień tygodnia, w którym swap naliczany jest potrójnie.
    /// 0 = niedziela … 3 = **środa** (tak jest na Vantage).
    #[serde(default)]
    pub swap_rollover3days: i32,
    /// `SYMBOL_TRADE_TICK_VALUE` — ile waluty rachunku daje ruch o `tick_size`
    #[serde(default)]
    pub tick_value: f64,
    /// `SYMBOL_TRADE_TICK_SIZE`
    #[serde(default)]
    pub tick_size: f64,
}

fn domyslnie_widoczny() -> bool {
    true
}

impl SymbolInfo {
    /// Minimalny dystans SL/TP od ceny **w jednostkach ceny** (nie w punktach).
    #[inline]
    pub fn stops_level_price(&self) -> f64 {
        self.stops_level_points * self.point
    }

    /// Zaokrągla cenę do liczby miejsc dziesiętnych instrumentu.
    #[inline]
    pub fn round_price(&self, px: f64) -> f64 {
        let f = 10f64.powi(self.digits as i32);
        (px * f).round() / f
    }

    #[inline]
    pub fn usd_per_point(&self) -> f64 {
        if self.tick_size <= 0.0 || self.point <= 0.0 {
            return 0.0;
        }
        self.tick_value * (self.point / self.tick_size)
    }

    pub fn swap_usd_per_lot_day(&self, long: bool) -> Option<f64> {
        let raw = if long {
            self.swap_long
        } else {
            self.swap_short
        };
        match self.swap_mode {
            // SYMBOL_SWAP_MODE_POINTS
            1 => Some(raw * self.usd_per_point()),
            // SYMBOL_SWAP_MODE_CURRENCY_* — już w walucie
            2 | 3 | 4 => Some(raw),
            _ => None,
        }
    }

    /// Dzień UTRZYMANIA pozycji w numeracji 0 = poniedziałek.
    /// To pole nie jest dobą WEJŚCIA po północy używaną przez SimBroker.
    /// MT5 Wednesday=3 daje tutaj Wednesday=2; obciążenie następuje na
    /// przejściu Wednesday→Thursday, czyli w dobie wejścia Thursday=3.
    #[inline]
    pub fn swap_rollover_weekday_mon0(&self) -> u32 {
        // niedziela 0 → poniedziałek 0: przesunięcie o 6 modulo 7
        ((self.swap_rollover3days + 6).rem_euclid(7)) as u32
    }

    /// Doba wejścia przy naliczeniu swapu, zgodna z Settings/SimBroker.
    /// Nieprawidłowej odpowiedzi serwera nie zamieniamy w domyślny dzień.
    pub fn swap_rollover_entry_weekday_mon0(&self) -> Option<u32> {
        (0..=6).contains(&self.swap_rollover3days)
            .then(|| (self.swap_rollover_weekday_mon0() + 1) % 7)
    }

    /// Dosuwa wolumen do siatki brokera (min / max / krok).
    #[inline]
    pub fn round_volume(&self, v: f64) -> f64 {
        if self.volume_step <= 0.0 {
            return v;
        }
        let n = (v / self.volume_step).round();
        let r = n * self.volume_step;
        // krok bywa 0.01 → f64 gubi ostatnią cyfrę, więc jeszcze zaokrąglenie
        let r = (r * 1e8).round() / 1e8;
        r.clamp(self.volume_min, self.volume_max)
    }
}

// ============================================================
//  LISTA SYMBOLI BROKERA
// ============================================================

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BrokerSymbol {
    pub name: String,
    /// czy instrument jest w Podglądzie rynku terminala
    #[serde(default)]
    pub visible: bool,
    /// liczba miejsc po przecinku — panel formatuje cenę bez zgadywania
    #[serde(default)]
    pub digits: u32,
    /// `SYMBOL_TRADE_MODE_*` jako liczba: 0 = handel wyłączony, 4 = pełny
    #[serde(default)]
    pub trade_mode: i32,
}

/// Odpowiedź na `symbols`. Sidecar sortuje: widoczne najpierw, potem
/// alfabetycznie; przy brokerze z > 5000 instrumentów lista jest przycięta
/// do widocznych + trafień filtra (patrz `MAX_SYMBOLI` w sidecarze).
#[derive(Debug, Clone, Deserialize)]
pub struct SymbolsList {
    pub symbols: Vec<BrokerSymbol>,
    /// ile instrumentów broker ma ŁĄCZNIE — mówi, czy lista jest pełna,
    /// czy przycięta obroną. Starszy sidecar tego pola nie wysyła (0).
    #[serde(default)]
    pub total: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawAccount {
    /// Qualified by the sidecar's already-observed advancing quote clock.
    /// None for legacy/stale/ambiguous-day samples; never infer from cached q.ts.
    #[serde(default)]
    pub observation_broker_day: Option<i64>,
    pub balance: f64,
    pub equity: f64,
    pub margin: f64,
    pub margin_free: f64,
    pub leverage: u32,
    #[serde(default)]
    pub credit: f64,
    #[serde(default)]
    pub currency: String,
    // --- tożsamość rachunku ---
    // Wszystkie z `serde(default)`: starszy sidecar tych pól nie wysyła,
    // a brak numeru konta nie jest powodem, żeby most nie wstał.
    #[serde(default)]
    pub login: i64,
    #[serde(default)]
    pub server: String,
    #[serde(default)]
    pub company: String,
    #[serde(default)]
    pub holder: String,
    /// `ACCOUNT_TRADE_MODE`: 0 = demo, 1 = konkurs, 2 = rachunek realny
    #[serde(default)]
    pub trade_mode: u8,
}

/// Tożsamość rachunku — do pokazania w panelu, nie do liczenia.
///
/// Rdzeń (`conduit_core::broker::Account`) świadomie tego nie zna: silnikowi
/// do decyzji nie jest potrzebny numer konta, a każde pole widoczne w rdzeniu
/// to pole, które mogłoby wejść do decyzji i zepsuć determinizm backtestu.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct AccountIdent {
    pub login: i64,
    pub server: String,
    pub company: String,
    pub holder: String,
    pub currency: String,
    pub leverage: u32,
    pub trade_mode: u8,
}

/// Separate, opt-in complete BID candles. Raw broker stamps are not UTC-shifted.
#[derive(Debug, Clone, Deserialize)]
pub struct RawM1Bars {
    pub schema: u32,
    pub symbol: String,
    pub account: M1Account,
    pub observed_utc_ms: i64,
    pub available_at_ms: i64,
    pub complete: bool,
    pub error: Option<String>,
    #[serde(default)]
    pub catchup_truncated: bool,
    pub bars: Vec<conduit_core::t100::Bar>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct M1Account { pub login: i64, pub server: String, pub trade_mode: u8 }

impl AccountIdent {
    /// `DEMO` / `KONKURS` / `REAL` — etykieta dla panelu.
    pub fn kind(&self) -> &'static str {
        match self.trade_mode {
            2 => "REAL",
            1 => "CONTEST",
            _ => "DEMO",
        }
    }

    /// Czy to jest rachunek na PRAWDZIWYCH pieniądzach.
    pub fn is_real(&self) -> bool {
        self.trade_mode == 2
    }
}

// ============================================================
//  ŚWIECE
// ============================================================

/// Jedna świeca — na drucie leci jako TABLICA, nie jako obiekt.
///
/// `[t, o, h, l, c, tick_volume, spread]`. Powód jest mierzalny: 5000 świec
/// w postaci obiektów to 273 kB w jednej linii protokołu, w postaci tablic
/// niecałe 100 kB. Sidecar buduje je w pętli listowej, a nie słownikowej,
/// więc i po jego stronie jest taniej. Nazwane pola pojawiają się dopiero
/// na granicy REST-a, gdzie czyta je człowiek.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Bar(
    pub i64,
    pub f64,
    pub f64,
    pub f64,
    pub f64,
    pub i64,
    pub i64,
);

impl Bar {
    /// Początek świecy w czasie SERWERA BROKERA, jako epoka w ms.
    #[inline]
    pub fn t(&self) -> i64 {
        self.0
    }
    #[inline]
    pub fn open(&self) -> f64 {
        self.1
    }
    #[inline]
    pub fn high(&self) -> f64 {
        self.2
    }
    #[inline]
    pub fn low(&self) -> f64 {
        self.3
    }
    #[inline]
    pub fn close(&self) -> f64 {
        self.4
    }
    /// `tick_volume` — liczba zmian ceny. Realnego wolumenu broker nie podaje.
    #[inline]
    pub fn volume(&self) -> i64 {
        self.5
    }
    /// spread w punktach w chwili domknięcia świecy
    #[inline]
    pub fn spread(&self) -> i64 {
        self.6
    }
}

/// Odpowiedź na `candles`.
///
/// # Zegar
///
/// `Bar::t` jest w czasie SERWERA BROKERA (Vantage: UTC+3), podanym jako
/// epoka w milisekundach — DOKŁADNIE tak samo jak `RawTick::ts`. To nie jest
/// przeoczenie, tylko warunek tego, żeby świeca bieżąca i kwotowanie trafiały
/// do tego samego kubełka czasu. Metadane offsetu wymagają świeżej pary
/// zaobserwowanego postępu ticka i UTC; stary tick nie określa czasu teraz.
#[derive(Debug, Clone, Deserialize)]
pub struct Candles {
    pub symbol: String,
    /// znormalizowana nazwa interwału (`M5`, `H1`, …) — sidecar przyjmuje też
    /// zapis panelu (`5m`, `1h`) i zwraca postać kanoniczną
    pub tf: String,
    /// długość świecy w ms; dla `MN1` wartość poglądowa (30 dni)
    pub bar_ms: i64,
    pub digits: u32,
    pub point: f64,
    /// Surowy czas ostatniego ticka; może pochodzić z poprzedniej sesji.
    #[serde(default)]
    pub server_time_ms: Option<i64>,
    /// UTC przygotowania odpowiedzi, nie chwili powstania starego ticka.
    pub utc_time_ms: i64,
    /// UTC obserwacji postępu time_msc w bieżącym kontekście konta/symbolu.
    /// Pierwszy cached tick i starszy sidecar nie dostarczają tego dowodu.
    #[serde(default)]
    pub quote_observed_utc_ms: Option<i64>,
    /// Wiek tego postępu zmierzony zegarem monotonicznym sidecara.
    #[serde(default)]
    pub quote_observation_age_ms: Option<i64>,
    pub bars: Vec<Bar>,
}

impl Candles {
    /// Ostatni tick tylko z dowodem postępu nie starszym niż30s.
    /// Obie domeny czasu muszą być spójne: skok zegara UTC unieważnia dowód.
    pub fn fresh_server_time_ms(&self) -> Option<i64> {
        let age = self.quote_observation_age_ms?;
        let wall_age = self.utc_time_ms.checked_sub(self.quote_observed_utc_ms?)?;
        if !(0..=30_000).contains(&age) || !(0..=30_000).contains(&wall_age) {
            return None;
        }
        self.server_time_ms
    }

    /// Przesunięcie wyłącznie ze świeżej, sparowanej obserwacji tick/UTC.
    /// Nie odejmujemy obecnego UTC od ticka sprzed zamknięcia rynku.
    pub fn server_offset_ms(&self) -> Option<i64> {
        self.fresh_server_time_ms()?.checked_sub(self.quote_observed_utc_ms?)
    }

    /// Czas spędzony w cache również postarza dowód świeżości.
    pub(crate) fn age_clock_by(&mut self, elapsed: std::time::Duration) {
        let ms = elapsed.as_millis().min(i64::MAX as u128) as i64;
        self.utc_time_ms = self.utc_time_ms.saturating_add(ms);
        self.quote_observation_age_ms = self.quote_observation_age_ms.map(|a| a.saturating_add(ms));
    }

    /// Czas otwarcia najstarszej zwróconej świecy — kursor do doładowania.
    pub fn oldest(&self) -> Option<i64> {
        self.bars.first().map(|b| b.t())
    }

    pub fn newest(&self) -> Option<i64> {
        self.bars.last().map(|b| b.t())
    }

    /// Czy OSTATNIA świeca jest już domknięta.
    ///
    /// Domknięta = minął cały jej interwał. Przy `copy_rates_from_pos` ostatnia
    /// świeca jest zwykle tą, która się właśnie formuje, więc odpowiedź brzmi
    /// „nie" — i panel wie, że wolno mu ją dolepiać z ticków.
    pub fn last_closed(&self) -> bool {
        match (self.newest(), self.server_time_ms) {
            (Some(t), Some(now)) => now >= t + self.bar_ms,
            _ => false,
        }
    }

    /// Czy rynek żyje.
    ///
    /// Kryterium: ostatnia świeca zaczęła się nie dawniej niż trzy interwały
    /// temu. Trzy, a nie jeden, bo instrument bywa cienki i pojedyncza świeca
    /// potrafi się nie utworzyć przy braku ticków — a „rynek zamknięty"
    /// wyświetlone w środku sesji jest gorsze niż spóźnione o trzy minuty.
    pub fn market_open(&self) -> bool {
        match (self.newest(), self.fresh_server_time_ms()) {
            (Some(t), Some(now)) if self.bar_ms > 0 => now.checked_sub(t)
                .map(|age| age >= 0 && age < self.bar_ms.saturating_mul(3)).unwrap_or(false),
            _ => false,
        }
    }
}

// ============================================================
//  HISTORIA RACHUNKU
// ============================================================

/// Jeden deal z historii. Na drucie jako TABLICA — kolejność kolumn
/// z `KOLUMNY_DEALA` w sidecarze, powtórzona w `Deals::columns`.
///
/// 1995 dealów tego konta to 224 kB w postaci tablic; jako obiekty byłoby
/// blisko trzy razy tyle.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Deal(
    /// ticket
    pub i64,
    /// order
    pub i64,
    /// position_id
    pub i64,
    /// time_msc — czas SERWERA BROKERA
    pub i64,
    /// `DEAL_TYPE_*`: 0 BUY, 1 SELL, 2 BALANCE (wpłata/wypłata)
    pub i32,
    /// `DEAL_ENTRY_*`: 0 IN, 1 OUT, 2 INOUT, 3 OUT_BY
    pub i32,
    pub f64,
    /// price
    pub f64,
    /// profit
    pub f64,
    /// commission
    pub f64,
    /// swap
    pub f64,
    /// fee
    pub f64,
    /// magic
    pub i64,
    /// `DEAL_REASON_*`
    pub i32,
    pub String,
    pub String,
);

impl Deal {
    #[inline]
    pub fn ticket(&self) -> i64 {
        self.0
    }
    #[inline]
    pub fn position(&self) -> i64 {
        self.2
    }
    #[inline]
    pub fn time_msc(&self) -> i64 {
        self.3
    }
    #[inline]
    pub fn kind(&self) -> i32 {
        self.4
    }
    #[inline]
    pub fn entry(&self) -> i32 {
        self.5
    }
    #[inline]
    pub fn volume(&self) -> f64 {
        self.6
    }
    #[inline]
    pub fn price(&self) -> f64 {
        self.7
    }
    #[inline]
    pub fn profit(&self) -> f64 {
        self.8
    }
    #[inline]
    pub fn commission(&self) -> f64 {
        self.9
    }
    #[inline]
    pub fn swap(&self) -> f64 {
        self.10
    }
    #[inline]
    pub fn magic(&self) -> i64 {
        self.12
    }
    #[inline]
    pub fn symbol(&self) -> &str {
        &self.14
    }
    #[inline]
    pub fn comment(&self) -> &str {
        &self.15
    }

    /// Wynik NETTO tej transakcji: zysk plus prowizja plus swap.
    ///
    /// Osobna metoda, bo sam `profit` nie jest tym, co ubyło z konta —
    /// a to właśnie jego ludzie porównują z saldem i nie mogą się doliczyć.
    #[inline]
    pub fn net(&self) -> f64 {
        self.profit() + self.commission() + self.swap() + self.11
    }

    /// Czy to operacja salda (wpłata, wypłata, korekta), a nie handel.
    #[inline]
    pub fn is_balance(&self) -> bool {
        self.kind() == 2
    }
}

/// Odpowiedź na `history_deals`.
#[derive(Debug, Clone, Deserialize)]
pub struct Deals {
    /// nazwy kolumn w kolejności, w jakiej stoją w wierszu
    #[serde(default)]
    pub columns: Vec<String>,
    /// ile rekordów spełnia warunki ŁĄCZNIE (nie tylko w tym oknie)
    pub total: usize,
    pub offset: usize,
    pub count: usize,
    /// czy za tym oknem są jeszcze rekordy
    pub more: bool,
    pub deals: Vec<Deal>,
}


#[derive(Debug, Clone, Default, Deserialize)]
pub struct SlipStats {
    /// liczba próbek — bez niej średnia nie jest wynikiem
    pub n: usize,
    /// średni poślizg w JEDNOSTKACH CENY, dodatni = na naszą niekorzyść
    pub mean: f64,
    #[serde(default)]
    pub sd: f64,
    /// połowa przedziału ufności 95 %
    #[serde(default)]
    pub ci95: f64,
    /// ile próbek było dokładnie zerowych
    #[serde(default)]
    pub exact: usize,
    /// ile wypełnień padło dokładnie na poziomie (dla oczekujących)
    #[serde(default)]
    pub exact_at_level: usize,
    #[serde(default)]
    pub max_abs: f64,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Costs {
    pub symbol: String,
    pub window_days: f64,
    pub pending: SlipStats,
    pub market: SlipStats,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawTick {
    /// `time_msc` — epoka w milisekundach
    pub ts: i64,
    pub bid: f64,
    pub ask: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawPosition {
    pub ticket: u64,
    /// POSITION_IDENTIFIER is stable when the current broker ticket changes.
    #[serde(default)]
    pub identifier: u64,
    /// 0 = BUY, 1 = SELL (`POSITION_TYPE_*`)
    pub kind: i32,
    pub volume: f64,
    pub price_open: f64,
    pub time_msc: i64,
    #[serde(default)]
    pub sl: f64,
    #[serde(default)]
    pub tp: f64,
    #[serde(default)]
    pub profit: f64,
    #[serde(default)]
    pub magic: i64,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub symbol: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawOrder {
    pub ticket: u64,
    /// `ORDER_TYPE_*`: 2 BUY_LIMIT, 3 SELL_LIMIT, 4 BUY_STOP, 5 SELL_STOP
    pub kind: i32,
    pub volume: f64,
    pub price_open: f64,
    pub time_msc: i64,
    #[serde(default)]
    pub sl: f64,
    #[serde(default)]
    pub tp: f64,
    #[serde(default)]
    pub magic: i64,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub symbol: String,
}

/// Wynik `order_send` po stronie sidecara.
#[derive(Debug, Clone, Deserialize)]
pub struct SendResult {
    pub retcode: i64,
    #[serde(default)]
    pub order: u64,
    #[serde(default)]
    pub deal: u64,
    /// tiket POZYCJI — sidecar dociąga go z deala, bo `order` to nie to samo
    #[serde(default)]
    pub position: u64,
    /// Stable position identifier, separate from the ticket used for trade RPCs.
    #[serde(default)]
    pub position_identifier: u64,
    #[serde(default)]
    pub volume: f64,
    #[serde(default)]
    pub price: f64,
    #[serde(default)]
    pub profit: f64,
    #[serde(default)]
    pub comment: String,
}

/// Zamknięty deal (`DEAL_ENTRY_OUT` / `OUT_BY`) wypchnięty jako zdarzenie.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct RawClosed {
    pub deal: u64,
    pub position: u64,
    /// 0 = BUY, 1 = SELL — strona DEALA (odwrotna do strony pozycji!)
    pub deal_type: i32,
    pub volume: f64,
    pub price: f64,
    pub time_msc: i64,
    #[serde(default)]
    pub profit: f64,
    #[serde(default)]
    pub commission: f64,
    #[serde(default)]
    pub swap: f64,
    /// `DEAL_REASON_*`: 0 CLIENT, 1 MOBILE, 2 WEB, 3 EXPERT, 4 SL, 5 TP, 6 SO
    #[serde(default)]
    pub reason: i32,
    #[serde(default)]
    pub magic: i64,
    #[serde(default)]
    pub comment: String,
    /// instrument deala — potrzebny przy transakcjach spoza bota, które mogą
    /// być na zupełnie innym symbolu niż ten, którym handluje silnik
    #[serde(default)]
    pub symbol: String,
    /// cena otwarcia pozycji — sidecar dokleja ją z deala wejściowego, jeśli zna
    #[serde(default)]
    pub price_open: f64,
    #[serde(default)]
    pub time_open_msc: i64,
    /// Separate optional proof. Legacy serde defaults above are NOT evidence
    /// that an absent commission/swap field was really zero.
    #[serde(default)]
    pub cost_receipt: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zadanie_ma_dokladnie_jedna_linie() {
        let r = Request::new(3, "account", Value::Null);
        let l = r.to_line();
        assert!(l.ends_with('\n'));
        assert_eq!(l.matches('\n').count(), 1);
        assert!(l.contains("\"cmd\":\"account\""));
        // args == null nie jest wysyłane
        assert!(!l.contains("args"));
    }

    #[test]
    fn odpowiedz_ok_parsuje_sie_na_result() {
        let f = parse_line(r#"{"id":7,"ok":true,"result":{"balance":200.0}}"#)
            .unwrap()
            .unwrap();
        match f {
            Frame::Response { id, result } => {
                assert_eq!(id, 7);
                assert_eq!(result.unwrap()["balance"], 200.0);
            }
            _ => panic!("miała być odpowiedź"),
        }
    }

    #[test]
    fn odpowiedz_bledna_niesie_retcode() {
        let f = parse_line(r#"{"id":8,"ok":false,"error":{"code":10016,"msg":"Invalid stops"}}"#)
            .unwrap()
            .unwrap();
        match f {
            Frame::Response { id, result } => {
                assert_eq!(id, 8);
                let e = result.unwrap_err();
                assert_eq!(e.code, 10016);
                assert_eq!(e.msg, "Invalid stops");
            }
            _ => panic!("miała być odpowiedź"),
        }
    }

    #[test]
    fn zdarzenie_rozpoznawane_po_polu_ev() {
        let f = parse_line(r#"{"ev":"tick","ts":1700000000000,"bid":4000.0,"ask":4000.24}"#)
            .unwrap()
            .unwrap();
        match f {
            Frame::Event { kind, body } => {
                assert_eq!(kind, "tick");
                let t: RawTick = serde_json::from_value(body).unwrap();
                assert_eq!(t.ts, 1_700_000_000_000);
                assert!((t.ask - t.bid - 0.24).abs() < 1e-9);
            }
            _ => panic!("miało być zdarzenie"),
        }
    }

    #[test]
    fn pusta_linia_to_keepalive_a_nie_blad() {
        assert_eq!(parse_line("   \r\n").unwrap(), None);
        assert_eq!(parse_line("").unwrap(), None);
    }

    #[test]
    fn ramka_bez_id_i_bez_ev_to_blad_protokolu() {
        let e = parse_line(r#"{"cos":1}"#).unwrap_err();
        assert!(matches!(e, ProtoError::Unknown(_)));
    }

    #[test]
    fn smieci_to_blad_json_a_nie_panika() {
        let e = parse_line("to nie jest json").unwrap_err();
        assert!(matches!(e, ProtoError::BadJson(_)));
    }

    #[test]
    fn swieca_parsuje_sie_z_tablicy_a_nie_z_obiektu() {
        let v: Candles = serde_json::from_str(
            r#"{"symbol":"XAUUSD","tf":"M5","bar_ms":300000,"digits":2,"point":0.01,
                "server_time_ms":1785338162980,"utc_time_ms":1785327361425,
                "quote_observed_utc_ms":1785327361425,"quote_observation_age_ms":0,
                "bars":[[1785337800000,4029.93,4030.95,4026.18,4027.0,1594,23],
                        [1785338100000,4027.01,4027.48,4025.83,4027.12,510,23]]}"#,
        )
        .unwrap();
        assert_eq!(v.bars.len(), 2);
        let b = v.bars[1];
        assert_eq!(b.t(), 1_785_338_100_000);
        assert!((b.close() - 4027.12).abs() < 1e-9);
        assert_eq!(b.volume(), 510);
        assert_eq!(b.spread(), 23);
        // to jest ta liczba, o którą rozjeżdżają się zegary: 3 godziny
        assert_eq!(v.server_offset_ms(), Some(10_801_555));
        assert_eq!(v.oldest(), Some(1_785_337_800_000));
    }

    #[test]
    fn swieca_biezaca_nie_jest_domknieta_a_rynek_zyje() {
        let mk = |ostatnia: i64, teraz: i64| Candles {
            symbol: "XAUUSD".into(),
            tf: "M5".into(),
            bar_ms: 300_000,
            digits: 2,
            point: 0.01,
            server_time_ms: Some(teraz),
            utc_time_ms: 0,
            quote_observed_utc_ms: Some(0),
            quote_observation_age_ms: Some(0),
            bars: vec![Bar(ostatnia, 1.0, 1.0, 1.0, 1.0, 1, 1)],
        };
        // świeca zaczęta minutę temu: żyje i nie jest domknięta
        let c = mk(1_000_000, 1_060_000);
        assert!(!c.last_closed());
        assert!(c.market_open());
        // ostatnia świeca sprzed pół godziny przy interwale 5 min = rynek stoi
        let z = mk(1_000_000, 1_000_000 + 1_800_000);
        assert!(z.last_closed());
        assert!(!z.market_open(), "weekend musi być rozpoznany");
        // brak czasu serwera = nie wiemy, więc NIE twierdzimy, że rynek działa
        let mut n = mk(1_000_000, 0);
        n.server_time_ms = None;
        assert!(!n.market_open());
    }

    #[test]
    fn cached_or_legacy_quote_never_claims_a_current_clock() {
        let legacy = r#"{"symbol":"SYN","tf":"M1","bar_ms":60000,"digits":2,"point":0.01,
            "server_time_ms":10861000,"utc_time_ms":86461000,
            "bars":[[10860000,100,101,99,100,1,1]]}"#;
        let mut c: Candles = serde_json::from_str(legacy).unwrap();
        // Previously this Friday quote on Saturday yielded -21h and open=true.
        assert_eq!(c.server_offset_ms(), None);
        assert!(!c.market_open());
        c.quote_observed_utc_ms = Some(61_000);
        c.quote_observation_age_ms = Some(86_400_000);
        assert_eq!(c.fresh_server_time_ms(), None);
        assert!(!c.market_open());
    }

    #[test]
    fn observed_quote_offset_uses_paired_utc_and_cache_age_expires_it() {
        let mut c: Candles = serde_json::from_str(r#"{
            "symbol":"SYN","tf":"M1","bar_ms":60000,"digits":2,"point":0.01,
            "server_time_ms":10861000,"utc_time_ms":62000,
            "quote_observed_utc_ms":61000,"quote_observation_age_ms":1000,
            "bars":[[10860000,100,101,99,100,1,1]]}"#).unwrap();
        assert_eq!(c.server_offset_ms(), Some(10_800_000));
        assert!(c.market_open());
        c.age_clock_by(std::time::Duration::from_secs(29));
        assert_eq!(c.server_offset_ms(), Some(10_800_000));
        c.age_clock_by(std::time::Duration::from_millis(1));
        assert_eq!(c.server_offset_ms(), None);
        assert!(!c.market_open());
        // This applies equally to 10-minute history cache and 800ms current cache.
        assert_eq!(c.server_time_ms, Some(10_861_000), "raw time is never rewritten");
    }

    #[test]
    fn clock_discontinuity_and_future_bar_fail_closed_in_metadata() {
        let mut c: Candles = serde_json::from_str(r#"{
            "symbol":"SYN","tf":"M1","bar_ms":60000,"digits":2,"point":0.01,
            "server_time_ms":10861000,"utc_time_ms":60000,
            "quote_observed_utc_ms":61000,"quote_observation_age_ms":0,
            "bars":[[10860000,100,101,99,100,1,1]]}"#).unwrap();
        assert_eq!(c.server_offset_ms(), None);
        assert!(!c.market_open());
        c.utc_time_ms = 61_000;
        c.bars[0].0 = 10_862_000;
        assert!(!c.market_open(), "bar from future is not a market-open proof");
        c.quote_observation_age_ms = Some(-1);
        assert_eq!(c.server_offset_ms(), None);
    }

    #[test]
    fn ok_false_bez_error_jest_wykrywane() {
        assert_eq!(
            parse_line(r#"{"id":1,"ok":false}"#).unwrap_err(),
            ProtoError::MissingError
        );
    }

    #[test]
    fn stops_level_liczony_z_punktow_i_point() {
        let si = SymbolInfo {
            symbol: "XAUUSD".into(),
            digits: 2,
            point: 0.01,
            stops_level_points: 20.0,
            freeze_level_points: 0.0,
            volume_min: 0.01,
            volume_max: 100.0,
            volume_step: 0.01,
            contract_size: 100.0,
            filling_mask: 2,
            filling_market: 1,
            filling_pending: 2,
            trade_mode: 4,
            visible: true,
            description: String::new(),
            swap_long: -75.82,
            swap_short: 27.41,
            swap_mode: 1,
            swap_rollover3days: 3,
            tick_value: 1.0,
            tick_size: 0.01,
        };
        // 20 punktów × 0.01 = 0.20 $ na złocie — dokładnie to, co zgłasza Vantage
        assert!((si.stops_level_price() - 0.20).abs() < 1e-12);
        assert!((si.round_price(4000.123456) - 4000.12).abs() < 1e-9);
        assert!((si.round_volume(0.0149) - 0.01).abs() < 1e-9);
        assert!(
            (si.round_volume(0.0) - 0.01).abs() < 1e-9,
            "min lot wymuszony"
        );
        assert!(
            (si.round_volume(1000.0) - 100.0).abs() < 1e-9,
            "max lot obcięty"
        );
    }

    #[test]
    fn swap_przelicza_sie_z_punktow_na_dolary() {
        let mut si = SymbolInfo {
            symbol: "XAUUSD".into(),
            digits: 2,
            point: 0.01,
            stops_level_points: 20.0,
            freeze_level_points: 0.0,
            volume_min: 0.01,
            volume_max: 100.0,
            volume_step: 0.01,
            contract_size: 100.0,
            filling_mask: 2,
            filling_market: 1,
            filling_pending: 2,
            trade_mode: 4,
            visible: true,
            description: String::new(),
            swap_long: -75.82,
            swap_short: 27.41,
            swap_mode: 1,
            swap_rollover3days: 3,
            tick_value: 1.0,
            tick_size: 0.01,
        };
        assert!((si.usd_per_point() - 1.0).abs() < 1e-12);
        assert!((si.swap_usd_per_lot_day(true).unwrap() + 75.82).abs() < 1e-9);
        assert!((si.swap_usd_per_lot_day(false).unwrap() - 27.41).abs() < 1e-9);
        // na locie 0,01 to jest dokładnie to, co widać w dealach
        let na_locie = si.swap_usd_per_lot_day(true).unwrap() * 0.01;
        assert!(
            (na_locie + 0.7582).abs() < 1e-9,
            "swap na locie 0,01 = {na_locie}"
        );

        // XAGUSD: tick_value 5,00 przy tick_size = point → 5 $ na punkt
        si.tick_value = 5.0;
        si.tick_size = 0.001;
        si.point = 0.001;
        si.swap_long = -23.65;
        assert!((si.usd_per_point() - 5.0).abs() < 1e-12);
        assert!((si.swap_usd_per_lot_day(true).unwrap() + 118.25).abs() < 1e-9);

        // tryb, którego nie umiemy przeliczyć, MUSI dać None zamiast liczby
        si.swap_mode = 0;
        assert_eq!(
            si.swap_usd_per_lot_day(true),
            None,
            "nieznany tryb nie może zgadywać"
        );
    }

    /// Dwie konwencje dnia tygodnia, obie opisujące środę. Ten test istnieje,
    /// bo pomyłka między nimi jest niewidoczna: swap po prostu potraja się
    /// o dobę za późno i nikt tego nie zauważy w wyniku.
    #[test]
    fn dzien_potrojnego_swapu_przelicza_sie_na_konwencje_silnika() {
        let mut si = SymbolInfo {
            symbol: "XAUUSD".into(),
            digits: 2,
            point: 0.01,
            stops_level_points: 20.0,
            freeze_level_points: 0.0,
            volume_min: 0.01,
            volume_max: 100.0,
            volume_step: 0.01,
            contract_size: 100.0,
            filling_mask: 2,
            filling_market: 1,
            filling_pending: 2,
            trade_mode: 4,
            visible: true,
            description: String::new(),
            swap_long: -75.82,
            swap_short: 27.41,
            swap_mode: 1,
            swap_rollover3days: 3,
            tick_value: 1.0,
            tick_size: 0.01,
        };
        // MT5: 3 = środa (od niedzieli). Silnik: 2 = środa (od poniedziałku).
        assert_eq!(
            si.swap_rollover_weekday_mon0(),
            2,
            "środa w konwencji silnika"
        );
        // pełna tabela — żeby przesunięcie nie „prawie działało"
        for (mt5, silnik) in [(0, 6), (1, 0), (2, 1), (3, 2), (4, 3), (5, 4), (6, 5)] {
            si.swap_rollover3days = mt5;
            assert_eq!(
                si.swap_rollover_weekday_mon0(),
                silnik,
                "MT5 {mt5} (0=niedziela) ma dać {silnik} (0=poniedziałek)"
            );
            assert_eq!(si.swap_rollover_entry_weekday_mon0(), Some((silnik + 1) % 7));
        }
        si.swap_rollover3days = -1;
        assert_eq!(si.swap_rollover_entry_weekday_mon0(), None);
        si.swap_rollover3days = 7;
        assert_eq!(si.swap_rollover_entry_weekday_mon0(), None);
    }

    #[test]
    fn deal_liczy_wynik_netto_a_nie_sam_zysk() {
        let d: Deal = serde_json::from_str(
            r#"[900000001,900000001,900000101,1785339403000,1,1,0.01,4006.0,
                -7.0,-0.35,-0.76,0.0,202406,4,"XAUUSD","koszyk B3"]"#,
        )
        .unwrap();
        assert_eq!(d.ticket(), 900_000_001);
        assert_eq!(d.magic(), 202_406);
        assert_eq!(d.symbol(), "XAUUSD");
        assert!(!d.is_balance());
        // zysk mówi −7,00, ale z konta ubyło −8,11
        assert!((d.profit() + 7.0).abs() < 1e-9);
        assert!((d.net() + 8.11).abs() < 1e-9, "netto = {}", d.net());
    }

    #[test]
    fn lista_symboli_parsuje_sie_z_polami_i_bez() {
        // pełny kształt nowego sidecara — z digits i trade_mode
        let l: SymbolsList = serde_json::from_str(
            r#"{"symbols":[
                {"name":"XAUUSD.s","visible":true,"digits":2,"trade_mode":4},
                {"name":"EURUSD.s","visible":false,"digits":5,"trade_mode":0}
            ],"total":1287}"#,
        )
        .unwrap();
        assert_eq!(l.symbols.len(), 2);
        assert_eq!(l.total, 1287);
        assert_eq!(l.symbols[0].name, "XAUUSD.s");
        assert!(l.symbols[0].visible);
        assert_eq!(l.symbols[0].digits, 2);
        assert_eq!(l.symbols[1].trade_mode, 0);

        // starszy sidecar wysyłał tylko name+visible i bez `total` —
        // most musi to przyjąć z zerami, a nie paść na kształcie
        let s: SymbolsList =
            serde_json::from_str(r#"{"symbols":[{"name":"XAUUSD","visible":true}]}"#).unwrap();
        assert_eq!(s.total, 0);
        assert_eq!(s.symbols[0].digits, 0);
        assert_eq!(s.symbols[0].trade_mode, 0);
    }

    #[test]
    fn wplata_na_konto_nie_jest_transakcja() {
        let d: Deal = serde_json::from_str(
            r#"[900000002,0,0,1785000000000,2,0,0.0,0.0,1000.0,0.0,0.0,0.0,0,0,"",""]"#,
        )
        .unwrap();
        assert!(
            d.is_balance(),
            "wpłata 1000 $ nie może wejść do statystyk handlu"
        );
    }
}
