
use crate::state::StateHandle;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

/// Sufit świec na jedno żądanie — ten sam, co w sidecarze i w `conduit_mt5`.
/// Panel doładowuje historię porcjami, więc nie potrzebuje więcej naraz.
pub const MAX_BARS: usize = 5000;
const DOMYSLNIE_BARS: usize = 500;

/// Sufit transakcji na jedno żądanie — ten sam, co w sidecarze i w `conduit_mt5`.
pub const MAX_DEALS: usize = 5000;

/// Sufit pozycji w odpowiedzi `/api/symbols` — ten sam próg, co `MAX_SYMBOLI`
/// w sidecarze (jego obrona działa PRZED wysyłką, ta działa niezależnie od
/// wersji sidecara po drugiej stronie mostu).
pub const MAX_SYMBOLS: usize = 5000;

// ============================================================
//  KSZTAŁT DANYCH DLA PANELU
// ============================================================

/// Jedna świeca. Pola 1:1 z `interface Candle` w `src/types/index.ts`,
/// żeby podmiana źródła nie wymagała ruszania typu po stronie React.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Candle {
    /// początek świecy — epoka w ms, CZAS SERWERA BROKERA
    pub t: i64,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
    /// `tick_volume` — liczba zmian ceny. Realnego wolumenu broker nie podaje.
    pub v: i64,
    /// spread w punktach w chwili domknięcia świecy (pole dodatkowe)
    pub s: i64,
}

/// Odpowiedź `/api/candles`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandlesDoc {
    pub symbol: String,
    /// kanoniczna nazwa interwału (`M5`, `H1`, …), także gdy pytano `5m`
    pub tf: String,
    /// Skąd są te świece. Wyłącznie `"MT5"` — inne źródło nie istnieje,
    /// bo brak danych kończy się błędem, a nie podstawieniem. Pole jest po to,
    /// żeby panel mógł zapalać chorągiewkę „nie z brokera" po FAKCIE, a nie
    /// zgadywać go z rozjazdu ceny.
    pub source: String,
    /// długość świecy w ms — panel wie, kiedy przewinąć świecę bieżącą
    pub bar_ms: i64,
    pub digits: u32,
    pub point: f64,
    /// czas serwera brokera minus UTC, w ms
    pub server_offset_ms: Option<i64>,
    /// czas serwera brokera w chwili odpowiedzi
    pub server_time: Option<i64>,
    /// `false` = ostatnia świeca jest starsza niż trzy interwały; rynek stoi
    pub market_open: bool,
    /// czy OSTATNIA świeca jest domknięta. `false` = jeszcze się formuje
    /// i wolno ją dolepiać z ticków.
    pub complete: bool,
    /// czas najstarszej zwróconej świecy — kursor do doładowania historii
    pub oldest: Option<i64>,
    pub newest: Option<i64>,
    pub count: usize,
    pub candles: Vec<Candle>,
}

/// Odpowiedź `/api/symbol` — parametry instrumentu prosto z serwera brokera.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolDoc {
    pub symbol: String,
    pub description: String,
    pub digits: u32,
    pub point: f64,
    /// `SYMBOL_TRADE_STOPS_LEVEL` w punktach — minimalny dystans SL/TP od ceny
    pub stops_level_points: f64,
    /// to samo w JEDNOSTKACH CENY; podajemy policzone, żeby panel nie mnożył
    pub stops_level_price: f64,
    pub freeze_level_points: f64,
    pub freeze_level_price: f64,
    pub volume_min: f64,
    pub volume_max: f64,
    pub volume_step: f64,
    pub contract_size: f64,
    /// `SYMBOL_TRADE_MODE`: 0 = handel wyłączony, 4 = pełny
    pub trade_mode: i32,
    /// czy instrument jest w Podglądzie rynku terminala
    pub visible: bool,
}

// ============================================================
//  LISTA SYMBOLI BROKERA
// ============================================================

/// Jedna pozycja odpowiedzi `/api/symbols` — do wyszukiwarki instrumentów.
///
/// `tradeMode` w camelCase, jak wszystko, co czyta panel. 0 = handel
/// wyłączony, 4 = pełny (`SYMBOL_TRADE_MODE_*`). `digits` = 0 znaczy
/// „nieznane" (starszy sidecar tej liczby nie wysyła).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolRow {
    pub name: String,
    /// czy instrument jest w Podglądzie rynku terminala
    pub visible: bool,
    pub digits: u32,
    pub trade_mode: i32,
}

/// Domyka KONTRAKT `/api/symbols` po stronie Rusta, niezależnie od wersji
/// sidecara po drugiej stronie mostu:
///
/// 1. skonfigurowany symbol bota jest w wynikach ZAWSZE — nawet gdy filtr
///    go nie łapie (wyszukiwarka nie ma prawa zgubić własnego instrumentu);
/// 2. porządek: widoczne w Podglądzie rynku najpierw, potem alfabetycznie;
/// 3. `max` to twardy sufit odpowiedzi — a symbol bota przeżywa także jego.
pub fn scal_liste_symboli(mut lista: Vec<SymbolRow>, bot: SymbolRow, max: usize) -> Vec<SymbolRow> {
    if !lista.iter().any(|s| s.name == bot.name) {
        lista.push(bot.clone());
    }
    lista.sort_by(|a, b| b.visible.cmp(&a.visible).then_with(|| a.name.cmp(&b.name)));
    lista.truncate(max.max(1));
    // sufit nie może wyciąć symbolu bota — gdy wypadł, wraca kosztem ostatniego
    if !lista.iter().any(|s| s.name == bot.name) {
        lista.pop();
        lista.push(bot);
    }
    lista
}

// ============================================================
//  HISTORIA RACHUNKU
// ============================================================

/// Jedna transakcja z historii rachunku.
///
/// Tu, inaczej niż przy świecach, pola są NAZWANE mimo objętości. Powód:
/// to jest droga eksportu — plik ogląda człowiek i porównuje z wyciągiem
/// z terminala. Tablica liczb oszczędziłaby 60 % bajtów i kosztowała
/// godzinę przy pierwszej reklamacji „nie zgadza mi się kolumna".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DealDoc {
    pub ticket: i64,
    pub order: i64,
    pub position: i64,
    /// czas w ms epoki, CZAS SERWERA BROKERA
    pub time: i64,
    /// `BUY` / `SELL` / `BALANCE` (wpłata, wypłata, korekta)
    #[serde(rename = "type")]
    pub kind: String,
    /// `IN` / `OUT` / `INOUT` / `OUT_BY`
    pub entry: String,
    pub volume: f64,
    pub price: f64,
    pub profit: f64,
    pub commission: f64,
    pub swap: f64,
    pub fee: f64,
    /// zysk + prowizja + swap + opłata — to, co NAPRAWDĘ ubyło z konta.
    /// Liczone tutaj, żeby nikt nie musiał tego składać u siebie i pomylić.
    pub net: f64,
    pub magic: i64,
    /// `DEAL_REASON_*`: 0 klient, 3 ekspert, 4 SL, 5 TP, 6 stop out
    pub reason: i32,
    pub symbol: String,
    pub comment: String,
}

/// Odpowiedź `/api/deals`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DealsDoc {
    /// Ile rekordów spełnia warunki ŁĄCZNIE — nie tylko w tym oknie.
    ///
    /// To jest pole, którego brak kosztował najwięcej: eksport oddawał
    /// garść rekordów z bieżącej sesji i **nie mówił o własnej
    /// niekompletności ani słowem**. Teraz `total` kontra `count` mówi
    /// wprost, czy widać całość.
    pub total: usize,
    pub offset: usize,
    pub count: usize,
    /// czy trzeba dopytać o kolejne okno
    pub more: bool,
    pub source: String,
    pub deals: Vec<DealDoc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlipDoc {
    pub n: usize,
    /// w jednostkach ceny; dodatni = wypełnienie na naszą niekorzyść
    pub mean: f64,
    pub sd: f64,
    /// połowa przedziału ufności 95 %
    pub ci95: f64,
    /// ile wypełnień było dokładnie bez poślizgu
    pub exact: usize,
    pub max_abs: f64,
    /// `history` albo `live` — patrz opis w `/api/costs`
    pub source: String,
}

/// Odpowiedź `/api/costs` — koszty, których symulator NIE ma zaszywać.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostsDoc {
    pub symbol: String,
    pub window_days: f64,
    pub pending: SlipDoc,
    pub market: SlipDoc,
    // --- koszt przetrzymania, prosto z serwera brokera ---
    /// swap pozycji długiej w jednostce z `swapMode`
    pub swap_long_raw: f64,
    pub swap_short_raw: f64,
    /// `SYMBOL_SWAP_MODE`; 1 = punkty
    pub swap_mode: i32,
    /// Dzień potrójnego swapu **w konwencji MT5: 0 = niedziela**, więc 3 = środa.
    pub swap_rollover3days: i32,
    /// Dzień UTRZYMANIA w numeracji 0 = poniedziałek; 2 = środa.
    /// Zachowane dla kompatybilności prezentacji. Nie przenosić do
    /// Settings.swap_rollover_weekday, które oznacza dobę WEJŚCIA.
    pub swap_rollover_weekday_mon0: u32,
    /// Doba WEJŚCIA po rolowaniu, właściwa dla Settings/SimBroker.
    /// Dla MT5 Wednesday=3 obciążenie następuje na wejściu w Thursday=3.
    #[serde(default)]
    pub swap_rollover_entry_weekday_mon0: Option<u32>,
    /// swap przeliczony na WALUTĘ RACHUNKU, za jednego lota za dobę.
    /// `null`, gdy tryb swapu jest taki, że przeliczyć się nie da —
    /// zgadywana liczba w tym miejscu jest gorsza niż jej brak.
    pub swap_long_usd_per_lot_day: Option<f64>,
    pub swap_short_usd_per_lot_day: Option<f64>,
    /// ile waluty rachunku daje jeden punkt na jednym locie
    pub usd_per_point: f64,
}

// ============================================================
//  ŹRÓDŁO
// ============================================================

/// Skąd serwer bierze dane rynkowe.
///
/// Cecha, a nie konkretny typ, z tego samego powodu, dla którego `Runtime`
/// jest cechą: `conduit-server` nie zależy od `conduit-mt5` i zależeć nie
/// powinien. Implementację wstawia warstwa aplikacji, gdy most już stoi.
///
/// Metody są SYNCHRONICZNE i wolno im blokować — uchwyty REST wołają je przez
/// `spawn_blocking`, żeby nie zająć wątku wykonawczego.
pub trait MarketSource: Send + Sync {
    fn candles(
        &self,
        symbol: &str,
        tf: &str,
        count: usize,
        to: Option<i64>,
    ) -> anyhow::Result<CandlesDoc>;

    fn symbol(&self, symbol: &str) -> anyhow::Result<SymbolDoc>;

    fn symbols(&self, _q: Option<&str>) -> anyhow::Result<Vec<SymbolRow>> {
        anyhow::bail!("to źródło danych nie zna listy symboli brokera")
    }

    /// PEŁNA historia transakcji z rachunku. Zakres w ms epoki (czas serwera).
    #[allow(clippy::too_many_arguments)]
    fn deals(
        &self,
        from: Option<i64>,
        to: Option<i64>,
        symbol: Option<&str>,
        magic: Option<i64>,
        out_only: bool,
        offset: usize,
        limit: usize,
    ) -> anyhow::Result<DealsDoc>;

    fn costs(&self, symbol: &str, days: f64) -> anyhow::Result<CostsDoc>;

    /// Dedicated, read-only broker-history job. Only start/status/page/release;
    /// called by the background allLogs worker, never by the trading engine.
    fn broker_history(&self, _request: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        anyhow::bail!("broker history export is unavailable for this source")
    }

    /// Instrument silnika — używany, gdy pytający nie poda żadnego.
    fn default_symbol(&self) -> String;

    /// Czy pod spodem w ogóle jest połączenie. `false` = odpowiemy 503
    /// bez zawracania głowy terminalowi.
    fn is_connected(&self) -> bool {
        true
    }
}

// ============================================================
//  TRASY
// ============================================================

pub fn router() -> Router<StateHandle> {
    Router::new()
        .route("/candles", get(candles))
        .route("/symbol", get(symbol))
        .route("/deals", get(deals))
        .route("/costs", get(costs))
}

#[derive(Debug, Deserialize)]
pub struct DealsQuery {
    /// dolna granica czasu w ms epoki (czas serwera brokera); brak = od początku
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub symbol: Option<String>,
    /// filtr `magic`; np. 770077 = tylko nasz bot, 202406 = stary bot.py
    pub magic: Option<i64>,
    /// tylko zamknięcia (`DEAL_ENTRY_OUT`/`OUT_BY`) — do rozliczenia wyników
    pub out_only: Option<bool>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

async fn deals(State(st): State<StateHandle>, Query(q): Query<DealsQuery>) -> Response {
    let Some(m) = st.market() else {
        return brak_zrodla();
    };
    if !m.is_connected() {
        return blad(
            StatusCode::SERVICE_UNAVAILABLE,
            "MetaTrader 5 nie jest podłączony — historii nie ma skąd wziąć.",
        );
    }
    let symbol = q.symbol;
    let (from, to, magic) = (q.from, q.to, q.magic);
    let out_only = q.out_only.unwrap_or(false);
    let offset = q.offset.unwrap_or(0);
    let limit = q.limit.unwrap_or(MAX_DEALS).clamp(1, MAX_DEALS);

    match tokio::task::spawn_blocking(move || {
        m.deals(from, to, symbol.as_deref(), magic, out_only, offset, limit)
    })
    .await
    {
        Ok(Ok(d)) => Json(d).into_response(),
        Ok(Err(e)) => blad(StatusCode::SERVICE_UNAVAILABLE, e.to_string()),
        Err(e) => blad(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("odczyt historii padł: {e}"),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct CostsQuery {
    pub symbol: Option<String>,
    pub days: Option<f64>,
}

async fn costs(State(st): State<StateHandle>, Query(q): Query<CostsQuery>) -> Response {
    let Some(m) = st.market() else {
        return brak_zrodla();
    };
    if !m.is_connected() {
        return blad(
            StatusCode::SERVICE_UNAVAILABLE,
            "MetaTrader 5 nie jest podłączony — kosztów nie ma skąd zmierzyć.",
        );
    }
    let symbol = q.symbol.unwrap_or_else(|| m.default_symbol());
    let days = q.days.unwrap_or(40.0).clamp(1.0, 3650.0);
    match tokio::task::spawn_blocking(move || m.costs(&symbol, days)).await {
        Ok(Ok(d)) => Json(d).into_response(),
        Ok(Err(e)) => blad(StatusCode::SERVICE_UNAVAILABLE, e.to_string()),
        Err(e) => blad(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("pomiar kosztów padł: {e}"),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct CandlesQuery {
    pub symbol: Option<String>,
    /// `M1 M5 M15 M30 H1 H4 D1 W1 MN1`; przyjmowany też zapis panelu (`5m`, `1h`)
    pub tf: Option<String>,
    pub count: Option<usize>,
    /// Doładowanie historii: zwróć świece STARSZE niż ten czas (granica
    /// wyłączna). Podaje się `t` najstarszej świecy, którą panel już ma —
    /// dzięki temu paczki się nie nakładają i sklejają się bez odsiewania.
    pub to: Option<i64>,
}

async fn candles(State(st): State<StateHandle>, Query(q): Query<CandlesQuery>) -> Response {
    let Some(m) = st.market() else {
        return brak_zrodla();
    };
    if !m.is_connected() {
        return blad(
            StatusCode::SERVICE_UNAVAILABLE,
            "MetaTrader 5 nie jest podłączony — świec nie ma skąd wziąć. \
             Sprawdź, czy terminal działa (Ustawienia → MetaTrader 5).",
        );
    }
    let symbol = q.symbol.unwrap_or_else(|| m.default_symbol());
    let tf = q.tf.unwrap_or_else(|| "M5".to_string());
    let count = q.count.unwrap_or(DOMYSLNIE_BARS).clamp(1, MAX_BARS);
    let to = q.to;

    match tokio::task::spawn_blocking(move || m.candles(&symbol, &tf, count, to)).await {
        Ok(Ok(d)) => Json(d).into_response(),
        Ok(Err(e)) => blad(StatusCode::SERVICE_UNAVAILABLE, e.to_string()),
        Err(e) => blad(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("odczyt świec padł: {e}"),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct SymbolQuery {
    pub symbol: Option<String>,
}

async fn symbol(State(st): State<StateHandle>, Query(q): Query<SymbolQuery>) -> Response {
    let Some(m) = st.market() else {
        return brak_zrodla();
    };
    let symbol = q.symbol.unwrap_or_else(|| m.default_symbol());
    match tokio::task::spawn_blocking(move || m.symbol(&symbol)).await {
        Ok(Ok(d)) => Json(d).into_response(),
        Ok(Err(e)) => blad(StatusCode::SERVICE_UNAVAILABLE, e.to_string()),
        Err(e) => blad(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("odczyt symbolu padł: {e}"),
        ),
    }
}

fn brak_zrodla() -> Response {
    blad(
        StatusCode::SERVICE_UNAVAILABLE,
        "brak źródła danych rynkowych — Conduit działa bez mostu do MetaTradera 5. \
         Świece pokazywane w panelu w tym trybie są POGLĄDOWE i nie pochodzą od brokera.",
    )
}

fn blad(code: StatusCode, msg: impl Into<String>) -> Response {
    (
        code,
        Json(serde_json::json!({ "ok": false, "error": msg.into() })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swieca_serializuje_sie_pod_typ_panelu() {
        let c = Candle {
            t: 1_785_338_100_000,
            o: 4027.01,
            h: 4027.48,
            l: 4025.83,
            c: 4027.12,
            v: 510,
            s: 23,
        };
        let j = serde_json::to_string(&c).unwrap();
        // React czyta dokładnie te nazwy — zmiana którejkolwiek psuje wykres
        assert!(j.contains("\"t\":1785338100000"), "{j}");
        assert!(j.contains("\"o\":4027.01"), "{j}");
        assert!(j.contains("\"c\":4027.12"), "{j}");
        assert!(j.contains("\"v\":510"), "{j}");
    }

    fn row(name: &str, visible: bool) -> SymbolRow {
        SymbolRow {
            name: name.into(),
            visible,
            digits: 2,
            trade_mode: 4,
        }
    }

    #[test]
    fn wiersz_symbolu_serializuje_sie_w_camelcase() {
        let j = serde_json::to_string(&row("XAUUSD.s", true)).unwrap();
        // KONTRAKT dla frontu (agent KREDYT): name, visible, digits, tradeMode
        assert!(j.contains("\"name\":\"XAUUSD.s\""), "{j}");
        assert!(j.contains("\"visible\":true"), "{j}");
        assert!(j.contains("\"digits\":2"), "{j}");
        assert!(j.contains("\"tradeMode\":4"), "{j}");
        assert!(
            !j.contains("trade_mode"),
            "snake_case nie ma prawa wyjść na REST: {j}"
        );
    }

    #[test]
    fn symbole_widoczne_najpierw_potem_alfabet() {
        let lista = vec![
            row("ZZZUSD", false),
            row("XAUUSD.s", true),
            row("AAAUSD", false),
            row("BTCUSD.s", true),
        ];
        let out = scal_liste_symboli(lista, row("XAUUSD.s", true), 100);
        let nazwy: Vec<&str> = out.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(nazwy, ["BTCUSD.s", "XAUUSD.s", "AAAUSD", "ZZZUSD"]);
        assert_eq!(
            out.len(),
            4,
            "symbol bota już był na liście — nie wolno go zdublować"
        );
    }

    #[test]
    fn symbol_bota_jest_dolaczany_gdy_filtr_go_nie_zlapal() {
        // wyniki filtra `q=eur` — złota tam nie ma, a kontrakt mówi: MA BYĆ
        let lista = vec![row("EURUSD.s", false), row("EURGBP.s", false)];
        let out = scal_liste_symboli(lista, row("XAUUSD.s", true), 100);
        assert_eq!(
            out[0].name, "XAUUSD.s",
            "widoczny symbol bota idzie na czoło"
        );
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn sufit_tnie_ogon_ale_nie_symbol_bota() {
        // 5 niewidocznych z początku alfabetu + niewidoczny symbol bota,
        // który po sortowaniu ląduje na samym końcu — sufit 3 by go wyciął
        let lista: Vec<SymbolRow> = ["AAA", "BBB", "CCC", "DDD", "EEE"]
            .iter()
            .map(|n| row(n, false))
            .collect();
        let out = scal_liste_symboli(lista, row("XAUUSD.s", false), 3);
        assert_eq!(out.len(), 3, "sufit musi być twardy");
        assert!(
            out.iter().any(|s| s.name == "XAUUSD.s"),
            "symbol bota nie może wypaść przez sufit: {out:?}"
        );
        // reszta to początek posortowanej listy
        assert_eq!(out[0].name, "AAA");
        assert_eq!(out[1].name, "BBB");
    }

    #[test]
    fn dokument_ma_pole_zrodla_i_nazwy_camelcase() {
        let d = CandlesDoc {
            symbol: "XAUUSD".into(),
            tf: "M5".into(),
            source: "MT5".into(),
            bar_ms: 300_000,
            digits: 2,
            point: 0.01,
            server_offset_ms: Some(10_800_000),
            server_time: Some(1_785_338_162_980),
            market_open: true,
            complete: false,
            oldest: Some(1),
            newest: Some(2),
            count: 0,
            candles: vec![],
        };
        let j = serde_json::to_string(&d).unwrap();
        assert!(j.contains("\"source\":\"MT5\""));
        assert!(j.contains("\"barMs\":300000"));
        assert!(j.contains("\"serverOffsetMs\":10800000"));
        assert!(j.contains("\"marketOpen\":true"));
    }
}
