//! EKSPORTY — jeden przycisk zamiast kopiowania plików z dysku.
//!
//! Zasada, która rządzi tym plikiem: **eksport ma odpowiadać na pytanie
//! „dlaczego", a nie tylko „co".** Zestawienie transakcji mówi, że bot kupił
//! 0,01 lota po 4046,54. To jest „co". Dopiero dziennik mówi, że kupił, bo
//! wiadomość z kanału została rozpoznana jako `ENTRY`, przeszła przez bramkę
//! ekspozycji, a broker przyjął zlecenie przy spreadzie 0,24 i wolnym
//! marginesie 518,61 — i że pięć sekund później odmówił drugiego zlecenia
//! z kodem `invalid_price`, bo poziom siatki wypadł nad rynkiem.
//!
//! Stąd podział na trzy poziomy szczegółowości:
//!
//! ```text
//!   historia.csv     wynik — co się zamknęło i za ile
//!   log-panelu.csv   przebieg — co program mówił człowiekowi
//!   dziennik.jsonl   DOWÓD — każda decyzja z powodem i migawką rynku
//!   paczka.json      wszystko naraz, do wysłania w zgłoszeniu
//! ```
//!
//! Wszystko idzie przez HTTP z nagłówkiem `Content-Disposition`, więc panel
//! nie musi umieć budować plików — wystarczy, że otworzy adres.

use crate::state::StateHandle;
use crate::{settings_map, ui};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use std::path::PathBuf;

pub fn router() -> Router<StateHandle> {
    Router::new()
        .route("/index", get(indeks))
        .route("/history.csv", get(historia_csv))
        .route("/account-history.csv", get(historia_konta_csv))
        .route("/account-history.json", get(historia_konta_json))
        .route("/history.json", get(historia_json))
        .route("/pendings.csv", get(oczekujace_csv))
        .route("/pendings.json", get(oczekujace_json))
        .route("/logs.csv", get(logi_csv))
        .route("/logs.json", get(logi_json))
        .route("/messages.csv", get(wiadomosci_csv))
        .route("/journal.jsonl", get(dziennik_jsonl))
        .route("/journal.csv", get(dziennik_csv))
        .route("/archive.jsonl", get(archiwum_jsonl))
        .route("/archive.csv", get(archiwum_csv))
        .route("/bundle.json", get(paczka))
}

// ============================================================
//  PARAMETRY
// ============================================================

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filtr {
    /// dolna granica czasu (ms epoki)
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// `all` (domyślnie) / `bot` / `foreign`
    pub scope: Option<String>,
    /// powód zamknięcia, np. `TP`
    pub reason: Option<String>,
    pub limit: Option<usize>,
    /// kategoria wpisu w logu panelu
    pub category: Option<String>,
    pub level: Option<String>,
    pub search: Option<String>,
    /// doba dziennika `YYYY-MM-DD`; bez niej idą wszystkie
    pub day: Option<String>,
    pub day_from: Option<String>,
    pub day_to: Option<String>,
    /// separator kolumn CSV. Excel w polskiej lokalizacji dzieli po średniku,
    /// `pandas` i R po przecinku — więc nie zgadujemy, tylko dajemy wybór.
    pub sep: Option<String>,
    /// tylko ten `magic` (770077 = CONDUIT, 202406 = stary bot.py, 0 = ręczne)
    pub magic: Option<i64>,
    /// instrument
    pub symbol: Option<String>,
    /// same ZAMKNIĘCIA (domyślnie tak — inaczej każda pozycja jest w pliku dwa razy)
    pub out_only: Option<bool>,
}

impl Filtr {
    fn separator(&self) -> char {
        match self.sep.as_deref() {
            Some(";") => ';',
            Some("\t") | Some("tab") => '\t',
            Some("|") => '|',
            _ => ',',
        }
    }
    fn limit(&self) -> usize {
        self.limit.unwrap_or(usize::MAX)
    }
    /// Czy pozycja o tym pochodzeniu mieści się w wybranym zakresie.
    fn pasuje_zrodlo(&self, src: ui::Origin) -> bool {
        match self.scope.as_deref() {
            Some("bot") => src == ui::Origin::Bot,
            Some("foreign") => src != ui::Origin::Bot,
            _ => true,
        }
    }
    fn pasuje_czas(&self, t: i64) -> bool {
        self.from.map(|f| t >= f).unwrap_or(true) && self.to.map(|x| t <= x).unwrap_or(true)
    }
}

// ============================================================
//  CSV
// ============================================================

/// Escapowanie wg RFC 4180 + ochrona przed wykonaniem formuły w arkuszu.
///
/// Drugie jest ważniejsze, niż wygląda: komentarz pozycji przychodzi z
/// Telegrama, a komórka zaczynająca się od `=` albo `+` jest w Excelu
/// FORMUŁĄ. Eksport, który wykonuje cudzy tekst po otwarciu, to nie jest
/// eksport, tylko dziura.
fn pole(s: &str, sep: char) -> String {
    // WYJĄTEK, bez którego lekarstwo jest gorsze od choroby: `-0.57` zaczyna
    // się od minusa, ale jest LICZBĄ. Zabezpieczony apostrofem wpadłby do
    // arkusza jako tekst i kolumna „zysk" przestałaby się sumować — czyli
    // eksport historii straciłby jedyną rzecz, po którą się go robi.
    let liczba = s.parse::<f64>().is_ok();
    let niebezpieczny_poczatek = !liczba && matches!(s.chars().next(), Some('=' | '+' | '-' | '@'));
    let wymaga_cudzyslowu = niebezpieczny_poczatek
        || s.contains(sep)
        || s.contains('"')
        || s.contains('\n')
        || s.contains('\r');
    if !wymaga_cudzyslowu {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 4);
    out.push('"');
    if niebezpieczny_poczatek {
        // apostrof przed treścią — arkusz traktuje komórkę jak tekst,
        // a człowiek czytający plik w edytorze i tak widzi oryginał
        out.push('\'');
    }
    for c in s.chars() {
        if c == '"' {
            out.push('"');
        }
        out.push(c);
    }
    out.push('"');
    out
}

struct Csv {
    sep: char,
    buf: String,
}

impl Csv {
    fn new(sep: char, naglowki: &[&str]) -> Self {
        // BOM: bez niego Excel czyta UTF-8 jako stronę kodową Windows
        // i „zamknięcie koszyka" robi się „zamkniÄ™cie".
        let mut buf = String::from("\u{feff}");
        let mut c = Csv {
            sep,
            buf: String::new(),
        };
        c.wiersz(naglowki);
        buf.push_str(&c.buf);
        Csv { sep, buf }
    }
    fn wiersz<S: AsRef<str>>(&mut self, kol: &[S]) {
        for (i, k) in kol.iter().enumerate() {
            if i > 0 {
                self.buf.push(self.sep);
            }
            self.buf.push_str(&pole(k.as_ref(), self.sep));
        }
        self.buf.push_str("\r\n");
    }
}

fn plik_csv(nazwa: &str, tresc: String) -> Response {
    odpowiedz(nazwa, "text/csv; charset=utf-8", tresc.into_bytes())
}

fn odpowiedz(nazwa: &str, typ: &str, dane: Vec<u8>) -> Response {
    let mut h = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(typ) {
        h.insert(header::CONTENT_TYPE, v);
    }
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{nazwa}\"")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    (h, dane).into_response()
}

fn plik_json(nazwa: &str, v: &serde_json::Value) -> Response {
    match serde_json::to_vec_pretty(v) {
        Ok(b) => odpowiedz(nazwa, "application/json; charset=utf-8", b),
        Err(e) => blad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

fn blad(code: StatusCode, msg: impl Into<String>) -> Response {
    (
        code,
        Json(serde_json::json!({ "ok": false, "error": msg.into() })),
    )
        .into_response()
}

/// Czas ścienny w postaci, którą da się posortować tekstowo i wkleić do arkusza.
fn czas(ms: i64) -> String {
    use chrono::{Local, TimeZone};
    if ms <= 0 {
        return String::new();
    }
    match Local.timestamp_millis_opt(ms).single() {
        Some(t) => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => ms.to_string(),
    }
}

fn licz(v: f64) -> String {
    format!("{v:.2}")
}

fn stempel() -> String {
    use chrono::Local;
    Local::now().format("%Y%m%d-%H%M").to_string()
}

fn tekst_enum<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|x| x.as_str().map(|s| s.to_string()))
        .unwrap_or_default()
}

// ============================================================
//  HISTORIA TRANSAKCJI
// ============================================================

fn wybierz_historie(st: &StateHandle, q: &Filtr) -> Vec<ui::ClosedPosition> {
    st.read(|s| {
        s.closed
            .iter()
            .filter(|c| q.pasuje_czas(c.close_time))
            .filter(|c| q.pasuje_zrodlo(c.source))
            .filter(|c| match &q.reason {
                Some(r) => tekst_enum(&c.reason).eq_ignore_ascii_case(r),
                None => true,
            })
            .take(q.limit())
            .cloned()
            .collect()
    })
}

fn history_net_total(dane: &[ui::ClosedPosition]) -> Option<f64> {
    dane.iter().try_fold(0.0, |sum, p| {
        let total = sum + p.net_result()?;
        total.is_finite().then_some(total)
    })
}

fn tabela_historii(dane: &[ui::ClosedPosition], separator: char) -> String {
    let mut c = Csv::new(
        separator,
        &[
            "ticket",
            "zrodlo",
            "magic",
            "symbol",
            "kierunek",
            "wolumen",
            "cena_otwarcia",
            "cena_zamkniecia",
            "czas_otwarcia",
            "czas_zamkniecia",
            "czas_trwania_min",
            "powod",
            "zysk",
            "swap",
            "prowizja",
            "wynik_netto",
            "koszyk",
            "komentarz",
            "profit_basis",
            "net_status",
        ],
    );
    for p in dane {
        let trwanie = if p.close_time > p.open_time && p.open_time > 0 {
            format!("{:.1}", (p.close_time - p.open_time) as f64 / 60_000.0)
        } else {
            String::new()
        };
        c.wiersz(&[
            p.ticket.to_string(),
            tekst_enum(&p.source),
            // pusta komórka zamiast zera: „nie wiemy" to nie to samo co
            // „magic 0", czyli „otwarte ręcznie z terminala"
            p.magic.map(|m| m.to_string()).unwrap_or_default(),
            p.symbol.clone(),
            tekst_enum(&p.direction),
            format!("{:.2}", p.volume),
            format!("{:.5}", p.open_price),
            format!("{:.5}", p.close_price),
            czas(p.open_time),
            czas(p.close_time),
            trwanie,
            tekst_enum(&p.reason),
            licz(p.profit),
            licz(p.swap),
            licz(p.commission),
            p.net_result().map(licz).unwrap_or_default(),
            p.basket_id.map(|b| format!("B{b}")).unwrap_or_default(),
            p.comment.clone(),
            tekst_enum(&p.profit_basis),
            if p.net_result().is_some() { "known" } else { "unknown" }.into(),
        ]);
    }
    c.buf
}

async fn historia_csv(State(st): State<StateHandle>, Query(q): Query<Filtr>) -> Response {
    let dane = wybierz_historie(&st, &q);
    // ZAKRES DANYCH IDZIE DO SAMEGO PLIKU, nie tylko do JSON-a. CSV bywa
    // jedyną rzeczą, którą ktoś ogląda po tygodniu — i musi sam powiedzieć,
    // czego w nim nie ma. Wiersz zaczyna się od `#`, więc arkusz pokaże go
    // jako tekst, a `grep` znajdzie bez otwierania pliku.
    let naglowek = format!(
        "\u{feff}# CONDUIT \u{2014} historia transakcji, wyeksportowano {}\r\n\
         # ZAKRES: od pod\u{142}\u{105}czenia mostu {}. Zamkni\u{119}cia sprzed tej chwili NIE S\u{104} obj\u{119}te \u{2014}\r\n\
         # most nie pobiera historii rachunku na \u{17c}\u{105}danie, a po restarcie zakres zaczyna si\u{119} od nowa.\r\n",
        czas(crate::now_ms()),
        czas(st.started_at),
    );
    // BOM jest już w nagłówku komentarza, więc z tabeli go zdejmujemy —
    // dwa znaczniki kolejności bajtów w jednym pliku psują pierwszą komórkę.
    let tabela = tabela_historii(&dane, q.separator()).trim_start_matches('\u{feff}').to_string();
    plik_csv(
        &format!("conduit-historia-{}.csv", stempel()),
        naglowek + &tabela,
    )
}

async fn historia_json(State(st): State<StateHandle>, Query(q): Query<Filtr>) -> Response {
    let dane = wybierz_historie(&st, &q);
    let suma = history_net_total(&dane);
    let bot = dane.iter().filter(|p| p.source == ui::Origin::Bot).count();
    let v = serde_json::json!({
        "meta": meta(&st, "historia transakcji"),
        "zakresDanych": zakres_danych(&st),
        "podsumowanie": {
            "pozycji": dane.len(),
            "w_tym_bota": bot,
            "spoza_bota": dane.len() - bot,
            "wynik_netto": suma.map(|n| (n * 100.0).round() / 100.0),
            "net_status": if suma.is_some() { "known" } else { "unknown" },
        },
        "pozycje": dane,
    });
    plik_json(&format!("conduit-historia-{}.json", stempel()), &v)
}

// ============================================================
//  PEŁNA HISTORIA RACHUNKU (prosto z terminala)
// ============================================================

/// Ile dealów bierzemy w jednym zapytaniu do mostu.
const DEALS_STRONA: usize = 2000;

fn pobierz_deale(
    st: &StateHandle,
    q: &Filtr,
) -> Result<(Vec<crate::market::DealDoc>, usize), String> {
    let Some(m) = st.market() else {
        return Err("brak połączenia z MT5 — historii rachunku nie ma skąd wziąć".into());
    };
    let magic = q.magic;
    let tylko_zamkniecia = q.out_only.unwrap_or(true);

    let mut out: Vec<crate::market::DealDoc> = Vec::new();
    let mut offset = 0usize;
    let mut total = 0usize;
    loop {
        let d = m
            .deals(
                q.from,
                q.to,
                q.symbol.as_deref(),
                magic,
                tylko_zamkniecia,
                offset,
                DEALS_STRONA,
            )
            .map_err(|e| e.to_string())?;
        total = d.total;
        let ile = d.deals.len();
        out.extend(d.deals);
        // Warunek stopu bierze i `more`, i pustą stronę: gdyby most kiedyś
        // zwrócił `more=true` przy zerowej liczbie rekordów, pętla bez tego
        // drugiego warunku kręciłaby się w nieskończoność.
        if !d.more || ile == 0 {
            break;
        }
        offset += ile;
    }
    Ok((out, total))
}

/// Czy deal jest OPERACJĄ KASOWĄ, a nie transakcją.
///
/// Wpłata 1000 $ ma `type = BALANCE` i `profit = 1000`. Wrzucona do statystyk
/// handlu wygląda jak najlepszy trade w historii konta — i zawyża każdą
/// miarę, której się dotknie.
fn operacja_kasowa(d: &crate::market::DealDoc) -> bool {
    d.kind == "BALANCE"
}

async fn historia_konta_csv(State(st): State<StateHandle>, Query(q): Query<Filtr>) -> Response {
    let (deale, total) = match pobierz_deale(&st, &q) {
        Ok(v) => v,
        Err(e) => return blad(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    let kasowe = deale.iter().filter(|d| operacja_kasowa(d)).count();

    let mut c = Csv::new(
        q.separator(),
        &[
            "ticket",
            "pozycja",
            "zlecenie",
            "czas",
            "typ",
            "wejscie_wyjscie",
            "operacja_kasowa",
            "symbol",
            "wolumen",
            "cena",
            "zysk",
            "prowizja",
            "swap",
            "oplata",
            "netto",
            "magic",
            "zrodlo",
            "powod",
            "komentarz",
        ],
    );
    for d in &deale {
        c.wiersz(&[
            d.ticket.to_string(),
            d.position.to_string(),
            d.order.to_string(),
            czas(d.time),
            d.kind.clone(),
            d.entry.clone(),
            if operacja_kasowa(d) {
                "tak".into()
            } else {
                String::new()
            },
            d.symbol.clone(),
            format!("{:.2}", d.volume),
            format!("{:.5}", d.price),
            licz(d.profit),
            licz(d.commission),
            licz(d.swap),
            licz(d.fee),
            licz(d.net),
            d.magic.to_string(),
            opis_magica(d.magic),
            opis_powodu(d.reason),
            d.comment.clone(),
        ]);
    }

    let naglowek = format!(
        "\u{feff}# CONDUIT \u{2014} PE\u{141}NA historia rachunku z terminala MT5, wyeksportowano {}\r\n\
         # Wierszy w pliku: {} z {} spe\u{142}niaj\u{105}cych warunki. {}\r\n\
         # UWAGA: {} wierszy to OPERACJE KASOWE (wp\u{142}aty/wyp\u{142}aty, typ BALANCE) \u{2014} nie s\u{105} transakcjami\r\n\
         # i nie wolno ich liczy\u{107} do wyniku handlu. Kolumna „operacja_kasowa\u{201d} je oznacza.\r\n",
        czas(crate::now_ms()),
        deale.len(),
        total,
        if deale.len() == total { "To jest ca\u{142}o\u{15b}\u{107}." } else { "UWAGA: to NIE jest ca\u{142}o\u{15b}\u{107}." },
        kasowe,
    );
    let tabela = c.buf.trim_start_matches('\u{feff}').to_string();
    plik_csv(
        &format!("conduit-historia-rachunku-{}.csv", stempel()),
        naglowek + &tabela,
    )
}

async fn historia_konta_json(State(st): State<StateHandle>, Query(q): Query<Filtr>) -> Response {
    let (deale, total) = match pobierz_deale(&st, &q) {
        Ok(v) => v,
        Err(e) => return blad(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    let kasowe: Vec<_> = deale.iter().filter(|d| operacja_kasowa(d)).collect();
    let handel: Vec<_> = deale.iter().filter(|d| !operacja_kasowa(d)).collect();
    let netto: f64 = handel.iter().map(|d| d.net).sum();

    let v = serde_json::json!({
        "meta": meta(&st, "pełna historia rachunku z terminala MT5"),
        "kompletnosc": {
            "wWyniku": deale.len(),
            "spelniajacychWarunki": total,
            "calosc": deale.len() == total,
        },
        "podsumowanie": {
            "transakcji": handel.len(),
            "operacjiKasowych": kasowe.len(),
            "nettoZHandlu": (netto * 100.0).round() / 100.0,
            "uwaga": "Operacje kasowe (typ BALANCE) są WYŁĄCZONE z `nettoZHandlu` — \
                      wpłata 1000 $ w statystykach handlu wygląda jak najlepszy trade \
                      w historii konta.",
        },
        "deale": deale,
    });
    plik_json(&format!("conduit-historia-rachunku-{}.json", stempel()), &v)
}

/// Czyj to `magic` — żeby użytkownik nie musiał pamiętać numerów.
fn opis_magica(m: i64) -> String {
    match m {
        770_077 => "CONDUIT".into(),
        0 => "ręczne / terminal".into(),
        inny => format!("inny automat ({inny})"),
    }
}

/// `DEAL_REASON_*` po ludzku.
fn opis_powodu(r: i32) -> String {
    match r {
        0 => "klient".into(),
        1 => "aplikacja mobilna".into(),
        2 => "przeglądarka".into(),
        3 => "ekspert / bot".into(),
        4 => "stop loss".into(),
        5 => "take profit".into(),
        6 => "stop out".into(),
        7 => "rollover".into(),
        inny => inny.to_string(),
    }
}

// ============================================================
//  ZLECENIA OCZEKUJĄCE (HISTORIA)
// ============================================================

fn wybierz_oczekujace(st: &StateHandle, q: &Filtr) -> Vec<ui::PendingHistoryItem> {
    st.read(|s| {
        s.pending_history
            .iter()
            .filter(|p| q.pasuje_czas(p.end_time))
            .take(q.limit())
            .cloned()
            .collect()
    })
}

async fn oczekujace_csv(State(st): State<StateHandle>, Query(q): Query<Filtr>) -> Response {
    let dane = wybierz_oczekujace(&st, &q);
    let mut c = Csv::new(
        q.separator(),
        &[
            "ticket",
            "symbol",
            "rodzaj",
            "wolumen",
            "cena",
            "sl",
            "tp",
            "czas_zlozenia",
            "czas_konca",
            "status",
            "koszyk",
        ],
    );
    for p in &dane {
        c.wiersz(&[
            p.ticket.to_string(),
            p.symbol.clone(),
            tekst_enum(&p.kind),
            format!("{:.2}", p.volume),
            format!("{:.5}", p.price),
            p.sl.map(|v| format!("{v:.5}")).unwrap_or_default(),
            p.tp.map(|v| format!("{v:.5}")).unwrap_or_default(),
            czas(p.placed_time),
            czas(p.end_time),
            tekst_enum(&p.status),
            p.basket_id.map(|b| format!("B{b}")).unwrap_or_default(),
        ]);
    }
    plik_csv(&format!("conduit-oczekujace-{}.csv", stempel()), c.buf)
}

async fn oczekujace_json(State(st): State<StateHandle>, Query(q): Query<Filtr>) -> Response {
    let dane = wybierz_oczekujace(&st, &q);
    let v = serde_json::json!({
        "meta": meta(&st, "historia zleceń oczekujących"),
        "zlecenia": dane,
    });
    plik_json(&format!("conduit-oczekujace-{}.json", stempel()), &v)
}

// ============================================================
//  LOG PANELU
// ============================================================

fn wybierz_logi(st: &StateHandle, q: &Filtr) -> Vec<ui::LogEntry> {
    let szukaj = q.search.as_ref().map(|s| s.to_lowercase());
    st.read(|s| {
        s.logs
            .iter()
            .filter(|l| q.pasuje_czas(l.t))
            .filter(|l| {
                q.category
                    .as_ref()
                    .map(|c| &l.category == c)
                    .unwrap_or(true)
            })
            .filter(|l| q.level.as_ref().map(|c| &l.level == c).unwrap_or(true))
            .filter(|l| match &szukaj {
                Some(t) => {
                    l.title.to_lowercase().contains(t) || l.content.to_lowercase().contains(t)
                }
                None => true,
            })
            .take(q.limit())
            .cloned()
            .collect()
    })
}

async fn logi_csv(State(st): State<StateHandle>, Query(q): Query<Filtr>) -> Response {
    let dane = wybierz_logi(&st, &q);
    let mut c = Csv::new(
        q.separator(),
        &["id", "czas", "poziom", "kategoria", "tytul", "tresc"],
    );
    for l in &dane {
        c.wiersz(&[
            l.id.to_string(),
            czas(l.t),
            l.level.clone(),
            l.category.clone(),
            l.title.clone(),
            l.content.replace('\n', " ⏎ "),
        ]);
    }
    plik_csv(&format!("conduit-log-{}.csv", stempel()), c.buf)
}

async fn logi_json(State(st): State<StateHandle>, Query(q): Query<Filtr>) -> Response {
    let dane = wybierz_logi(&st, &q);
    let v = serde_json::json!({ "meta": meta(&st, "log panelu"), "wpisy": dane });
    plik_json(&format!("conduit-log-{}.json", stempel()), &v)
}

// ============================================================
//  WIADOMOŚCI Z KANAŁÓW + ICH ROZBIÓR
// ============================================================

/// Wiadomość razem z tym, CO PARSER Z NIEJ WYJĄŁ.
///
/// Bez kolumn `rozbior_*` plik odpowiada tylko na pytanie „co przyszło".
/// Z nimi odpowiada na „jak to zostało zrozumiane" — a to jest jedyna wersja,
/// z której da się później dojść, czemu format kanału nie zadziałał.
async fn wiadomosci_csv(State(st): State<StateHandle>, Query(q): Query<Filtr>) -> Response {
    let dane: Vec<ui::ChatMessage> = st.read(|s| {
        s.messages
            .iter()
            .filter(|m| q.pasuje_czas(m.time))
            .take(q.limit())
            .cloned()
            .collect()
    });
    let mut c = Csv::new(
        q.separator(),
        &[
            "id",
            "czas",
            "kanal_id",
            "kanal",
            "temat",
            "typy",
            "koszyk",
            "edytowana",
            "rozbior_kierunek",
            "rozbior_limit",
            "rozbior_strefa_od",
            "rozbior_strefa_do",
            "rozbior_sl",
            "rozbior_tp",
            "rozbior_indeks_tp",
            "rozbior_poziom",
            "tresc",
        ],
    );
    for m in &dane {
        let p = m.parsed.as_ref().and_then(|v| v.first());
        c.wiersz(&[
            m.id.clone(),
            czas(m.time),
            m.channel_id.to_string(),
            m.channel_name.clone(),
            m.topic_name.clone().unwrap_or_default(),
            m.types.join(" "),
            m.basket_id.map(|b| format!("B{b}")).unwrap_or_default(),
            if m.edited {
                "tak".into()
            } else {
                String::new()
            },
            p.and_then(|x| x.direction.as_ref())
                .map(tekst_enum)
                .unwrap_or_default(),
            p.and_then(|x| x.is_limit)
                .map(|b| {
                    if b {
                        "limit".to_string()
                    } else {
                        "rynek".to_string()
                    }
                })
                .unwrap_or_default(),
            p.and_then(|x| x.entry_low)
                .map(|v| format!("{v:.5}"))
                .unwrap_or_default(),
            p.and_then(|x| x.entry_high)
                .map(|v| format!("{v:.5}"))
                .unwrap_or_default(),
            p.and_then(|x| x.sl)
                .map(|v| format!("{v:.5}"))
                .unwrap_or_default(),
            p.and_then(|x| x.tps.as_ref())
                .map(|v| {
                    v.iter()
                        .map(|t| format!("{t:.5}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default(),
            p.and_then(|x| x.tp_index)
                .map(|v| v.to_string())
                .unwrap_or_default(),
            p.and_then(|x| x.level)
                .map(|v| format!("{v:.5}"))
                .unwrap_or_default(),
            m.text.replace('\n', " ⏎ "),
        ]);
    }
    plik_csv(&format!("conduit-wiadomosci-{}.csv", stempel()), c.buf)
}

// ============================================================
//  DZIENNIK ZDARZEŃ
// ============================================================

/// Jeden plik dziennika: ścieżka, doba, prefiks, rozmiar.
struct PlikDziennika {
    sciezka: PathBuf,
    prefiks: String,
    doba: String,
    bajty: u64,
}

fn pliki_dziennika(st: &StateHandle) -> Vec<PlikDziennika> {
    let dir = st.workspace.journal_dir();
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return out;
    };
    for e in rd.filter_map(|x| x.ok()) {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|x| x.to_str()) else {
            continue;
        };
        // doba to ostatnie 10 znaków nazwy: RRRR-MM-DD
        let (prefiks, doba) = match stem.len().checked_sub(11) {
            Some(i) if stem.as_bytes().get(i) == Some(&b'-') => {
                (stem[..i].to_string(), stem[i + 1..].to_string())
            }
            _ => (stem.to_string(), String::new()),
        };
        let bajty = e.metadata().map(|m| m.len()).unwrap_or(0);
        out.push(PlikDziennika {
            sciezka: p,
            prefiks,
            doba,
            bajty,
        });
    }
    out.sort_by(|a, b| a.doba.cmp(&b.doba).then(a.prefiks.cmp(&b.prefiks)));
    out
}

fn doba_pasuje(q: &Filtr, doba: &str) -> bool {
    if let Some(d) = &q.day {
        return doba == d;
    }
    // Doba ma postać `RRRR-MM-DD`, więc porównanie tekstowe JEST porównaniem
    // chronologicznym — nie ma po co parsować daty.
    if let Some(d) = &q.day_from {
        if doba < d.as_str() {
            return false;
        }
    }
    if let Some(d) = &q.day_to {
        if doba > d.as_str() {
            return false;
        }
    }
    true
}

/// Co da się wyeksportować — panel rysuje z tego listę do kliknięcia.
async fn indeks(State(st): State<StateHandle>) -> Json<serde_json::Value> {
    let pliki: Vec<serde_json::Value> = pliki_dziennika(&st)
        .iter()
        .map(|p| {
            serde_json::json!({
                "day": p.doba,
                "prefix": p.prefiks,
                "bytes": p.bajty,
                "file": p.sciezka.file_name().and_then(|x| x.to_str()).unwrap_or_default(),
            })
        })
        .collect();
    let archiwum: Vec<serde_json::Value> = pliki_archiwum(&st)
        .iter()
        .map(|p| {
            serde_json::json!({
                "day": p.doba,
                "bytes": p.bajty,
                "file": p.sciezka.file_name().and_then(|x| x.to_str()).unwrap_or_default(),
            })
        })
        .collect();
    let (zamkniete, oczekujace, logi, wiadomosci) = st.read(|s| {
        (
            s.closed.len(),
            s.pending_history.len(),
            s.logs.len(),
            s.messages.len(),
        )
    });
    Json(serde_json::json!({
        "journalDir": st.workspace.journal_dir().display().to_string(),
        "journalFiles": pliki,
        "archiveDir": st.workspace.archive_dir().display().to_string(),
        "archiveFiles": archiwum,
        "counts": {
            "closed": zamkniete,
            "pendingHistory": oczekujace,
            "logs": logi,
            "messages": wiadomosci,
        },
    }))
}

/// Surowe linie dziennika — dokładnie to, co czyta `loganaliza.exe`.
///
/// Świadomie BEZ przepakowania: analizator ma dostać bajt w bajt to samo, co
/// leży na dysku. Jedyne, co dokładamy, to sklejenie dób w jeden strumień.
async fn dziennik_jsonl(State(st): State<StateHandle>, Query(q): Query<Filtr>) -> Response {
    let mut out: Vec<u8> = Vec::new();
    let mut plikow = 0;
    for p in pliki_dziennika(&st) {
        if !doba_pasuje(&q, &p.doba) {
            continue;
        }
        let Ok(dane) = std::fs::read(&p.sciezka) else {
            continue;
        };
        out.extend_from_slice(&dane);
        if !out.ends_with(b"\n") {
            out.push(b'\n');
        }
        plikow += 1;
    }
    if plikow == 0 {
        return blad(
            StatusCode::NOT_FOUND,
            "nie ma plików dziennika w tym zakresie dób",
        );
    }
    odpowiedz(
        &format!("conduit-dziennik-{}.jsonl", stempel()),
        "application/x-ndjson; charset=utf-8",
        out,
    )
}

/// Dziennik spłaszczony do arkusza.
///
/// Kolumny dobrane pod JEDNO pytanie: „dlaczego bot zrobił (albo nie zrobił)
/// to, co zrobił". Stąd `powod` i `broker_error` obok `rodzaj`, i stąd cała
/// migawka rynku w tym samym wierszu — żeby nie trzeba było zszywać dwóch
/// plików po znaczniku czasu.
async fn dziennik_csv(State(st): State<StateHandle>, Query(q): Query<Filtr>) -> Response {
    let mut c = Csv::new(
        q.separator(),
        &[
            "event_id",
            "czas_lokalny",
            "czas_brokera",
            "doba",
            "poziom",
            "kategoria",
            "rodzaj",
            "powod",
            "broker_error",
            "operacja",
            "koszyk",
            "ticket",
            "msg_id",
            "zrodlo",
            "tresc",
            "bid",
            "ask",
            "spread",
            "equity",
            "saldo",
            "margines",
            "wolny_margines",
            "pozycje",
            "oczekujace",
            "wolumen",
            "wolumen_netto",
            "plywajacy",
            "dd_usd",
            "dd_pct",
            "zrealizowane_dzis",
            "dane",
        ],
    );
    let mut linii = 0usize;
    for p in pliki_dziennika(&st) {
        if !doba_pasuje(&q, &p.doba) {
            continue;
        }
        let Ok(tresc) = std::fs::read_to_string(&p.sciezka) else {
            continue;
        };
        for linia in tresc.lines() {
            if linia.trim().is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(linia) else {
                continue;
            };
            let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let n = |k: &str| {
                v.get(k)
                    .and_then(|x| x.as_i64())
                    .map(|x| x.to_string())
                    .unwrap_or_default()
            };
            let m = |k: &str| {
                v.get("market")
                    .and_then(|x| x.get(k))
                    .and_then(|x| x.as_f64())
                    .map(|x| format!("{x:.2}"))
                    .unwrap_or_default()
            };
            let d = |k: &str| {
                v.get("data")
                    .and_then(|x| x.get(k))
                    .map(|x| match x.as_str() {
                        Some(t) => t.to_string(),
                        None => x.to_string(),
                    })
                    .unwrap_or_default()
            };
            c.wiersz(&[
                s("event_id"),
                s("ts"),
                s("ts_broker"),
                s("session_day"),
                s("level"),
                s("category"),
                s("kind"),
                s("reason"),
                d("broker_error"),
                d("operation"),
                v.get("basket_id")
                    .and_then(|x| x.as_u64())
                    .map(|b| format!("B{b}"))
                    .unwrap_or_default(),
                n("ticket"),
                n("msg_id"),
                s("source"),
                s("text").replace('\n', " ⏎ "),
                m("bid"),
                m("ask"),
                m("spread"),
                m("equity"),
                m("balance"),
                m("margin_used"),
                m("free_margin"),
                m("open_positions"),
                m("open_pendings"),
                m("open_volume"),
                m("net_volume"),
                m("floating"),
                m("dd_abs"),
                m("dd_pct"),
                m("realized_today"),
                v.get("data").map(|x| x.to_string()).unwrap_or_default(),
            ]);
            linii += 1;
        }
    }
    if linii == 0 {
        return blad(
            StatusCode::NOT_FOUND,
            "nie ma zdarzeń dziennika w tym zakresie dób",
        );
    }
    plik_csv(&format!("conduit-dziennik-{}.csv", stempel()), c.buf)
}

// ============================================================
//  ARCHIWUM WIADOMOŚCI
// ============================================================

/// Pliki archiwum wiadomości, posortowane po dobie.
fn pliki_archiwum(st: &StateHandle) -> Vec<PlikDziennika> {
    let dir = st.workspace.archive_dir();
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return out;
    };
    for e in rd.filter_map(|x| x.ok()) {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|x| x.to_str()) else {
            continue;
        };
        let Some(doba) = stem.strip_prefix("wiadomosci-") else {
            continue;
        };
        let bajty = e.metadata().map(|m| m.len()).unwrap_or(0);
        out.push(PlikDziennika {
            sciezka: p.clone(),
            prefiks: "wiadomosci".into(),
            doba: doba.to_string(),
            bajty,
        });
    }
    out.sort_by(|a, b| a.doba.cmp(&b.doba));
    out
}

/// Surowe archiwum — każda wersja każdej wiadomości, w kolejności odbioru.
async fn archiwum_jsonl(State(st): State<StateHandle>, Query(q): Query<Filtr>) -> Response {
    let mut out: Vec<u8> = Vec::new();
    let mut plikow = 0;
    for p in pliki_archiwum(&st) {
        if !doba_pasuje(&q, &p.doba) {
            continue;
        }
        let Ok(dane) = std::fs::read(&p.sciezka) else {
            continue;
        };
        out.extend_from_slice(&dane);
        if !out.ends_with(b"\n") {
            out.push(b'\n');
        }
        plikow += 1;
    }
    if plikow == 0 {
        return blad(
            StatusCode::NOT_FOUND,
            "archiwum wiadomości jest puste — zapis rusza z pierwszą wiadomością z Telegrama",
        );
    }
    odpowiedz(
        &format!("conduit-wiadomosci-{}.jsonl", stempel()),
        "application/x-ndjson; charset=utf-8",
        out,
    )
}

/// Archiwum spłaszczone do arkusza.
///
/// `wersja` liczy, którym z kolei zapisem tej samej wiadomości jest wiersz —
/// dzięki temu w arkuszu widać od razu, które sygnały były poprawiane, bez
/// szukania po `msg_id`.
async fn archiwum_csv(State(st): State<StateHandle>, Query(q): Query<Filtr>) -> Response {
    use std::collections::HashMap;
    let mut c = Csv::new(
        q.separator(),
        &[
            "seq",
            "zdarzenie",
            "wersja",
            "czas_odbioru",
            "czas_telegrama",
            "chat_id",
            "temat",
            "msg_id",
            "odpowiedz_na",
            "edycja_of",
            "kanal",
            "nasluchiwany",
            "znakow",
            "tresc",
        ],
    );
    let mut wersje: HashMap<(i64, i64), u32> = HashMap::new();
    let mut linii = 0usize;
    for p in pliki_archiwum(&st) {
        if !doba_pasuje(&q, &p.doba) {
            continue;
        }
        let Ok(tresc) = std::fs::read_to_string(&p.sciezka) else {
            continue;
        };
        for linia in tresc.lines() {
            if linia.trim().is_empty() {
                continue;
            }
            let Ok(r) = serde_json::from_str::<crate::archive::MsgRecord>(linia) else {
                continue;
            };
            let w = wersje.entry((r.chat_id, r.msg_id)).or_insert(0);
            *w += 1;
            c.wiersz(&[
                r.seq.to_string(),
                r.event.as_str().to_string(),
                w.to_string(),
                r.received_at.clone(),
                czas(r.msg_ts_ms),
                r.chat_id.to_string(),
                r.topic_id.map(|t| t.to_string()).unwrap_or_default(),
                r.msg_id.to_string(),
                r.reply_to.map(|t| t.to_string()).unwrap_or_default(),
                r.edit_of.map(|t| t.to_string()).unwrap_or_default(),
                r.source_name.clone(),
                if r.monitored {
                    "tak".into()
                } else {
                    String::new()
                },
                r.text.chars().count().to_string(),
                r.text.replace('\n', " ⏎ "),
            ]);
            linii += 1;
        }
    }
    if linii == 0 {
        return blad(
            StatusCode::NOT_FOUND,
            "archiwum wiadomości jest puste w tym zakresie dób",
        );
    }
    plik_csv(&format!("conduit-wiadomosci-{}.csv", stempel()), c.buf)
}

// ============================================================
//  PACZKA — WSZYSTKO W JEDNYM PLIKU
// ============================================================

fn meta(st: &StateHandle, co: &str) -> serde_json::Value {
    let (tryb, preset, saldo, polaczenie) =
        st.read(|s| (s.mode, s.preset_id.clone(), s.balance, s.connection.clone()));
    serde_json::json!({
        "aplikacja": "CONDUIT",
        "wersja": env!("CARGO_PKG_VERSION"),
        "zawartosc": co,
        "wyeksportowano": czas(crate::now_ms()),
        "wyeksportowanoMs": crate::now_ms(),
        "katalogRoboczy": st.workspace.root.display().to_string(),
        "tryb": tryb,
        "preset": preset,
        "saldo": saldo,
        "polaczenie": polaczenie,
        "srodowisko": st.runtime.read().name(),
    })
}

/// ZAKRES DANYCH — zdaniem, nie liczbą.
///
/// Historia w migawce to **wyłącznie to, co most zobaczył od podłączenia**:
/// `opublikuj` podmienia `s.closed` w całości przy każdym obrocie pętli, a
/// sidecar oznacza deale zastane przy starcie jako już widziane i nigdy ich
/// nie wysyła. Eksport po restarcie potrafi więc mieć cztery wiersze na
/// rachunku, na którym zamknięto ich sto.
///
/// **Świadomie bez liczby dealów.** Kusi, żeby dopisać „245 dealów znanych
/// sidecarowi", ale ta liczba mierzy okno ±26 h, a nie historię rachunku, i
/// zawiera zarówno wejścia, jak i wyjścia. W pliku, który użytkownik dostaje
/// do ręki, zostałaby odczytana jako rozmiar historii — czyli dokładnie ten
/// błąd, który popełniono przy jej pierwszym zacytowaniu. Zdanie jest tutaj
/// precyzyjniejsze od liczby.
fn zakres_danych(st: &StateHandle) -> serde_json::Value {
    serde_json::json!({
        "od": czas(st.started_at),
        "odMs": st.started_at,
        "uwaga": "Historia obejmuje WYŁĄCZNIE transakcje, które most zobaczył od \
                  podłączenia do terminala. Zamknięcia sprzed tej chwili nie są \
                  objęte — most nie pobiera historii rachunku na żądanie. \
                  Po restarcie programu zakres zaczyna się od nowa.",
    })
}

/// JEDEN plik, który wystarczy do zgłoszenia błędu.
///
/// Zawiera stan, ustawienia (z jawną listą pól NIEDOCHODZĄCYCH do silnika),
/// historię, log panelu, wiadomości z rozbiorem i CAŁY dziennik zdarzeń.
/// Świadomie BEZ `secrets.json` — hasła, `api_hash` ani łańcucha sesji nie
/// wolno wysłać nikomu, a paczkę robi się właśnie po to, żeby ją wysłać.
async fn paczka(State(st): State<StateHandle>, Query(q): Query<Filtr>) -> Response {
    let snap = st.snapshot();
    let mut dziennik: Vec<serde_json::Value> = Vec::new();
    let mut zrodla: Vec<serde_json::Value> = Vec::new();
    for p in pliki_dziennika(&st) {
        if !doba_pasuje(&q, &p.doba) {
            continue;
        }
        zrodla.push(serde_json::json!({ "doba": p.doba, "prefiks": p.prefiks, "bajty": p.bajty }));
        let Ok(tresc) = std::fs::read_to_string(&p.sciezka) else {
            continue;
        };
        for linia in tresc.lines() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(linia) {
                dziennik.push(v);
            }
        }
    }

    // Archiwum wiadomości: surowe wejście bota, z każdą wersją osobno.
    // Bez niego paczka pokazuje decyzje bez tekstów, na których zapadły.
    let mut wiadomosci_arch: Vec<serde_json::Value> = Vec::new();
    for p in pliki_archiwum(&st) {
        if !doba_pasuje(&q, &p.doba) {
            continue;
        }
        let Ok(tresc) = std::fs::read_to_string(&p.sciezka) else {
            continue;
        };
        for linia in tresc.lines() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(linia) {
                wiadomosci_arch.push(v);
            }
        }
    }

    let niepodpiete = settings_map::unmapped_keys(&snap.settings);
    let v = serde_json::json!({
        "meta": meta(&st, "paczka diagnostyczna"),
        "zakresDanych": zakres_danych(&st),
        "ustawienia": {
            "dokument": snap.settings,
            "lot": snap.lot,
            "preset": snap.preset_id,
            // Jawnie, a nie w komentarzu: pole, które panel pokazuje, a silnik
            // ignoruje, jest najczęstszą przyczyną „ustawiłem, a nie działa".
            "poleNiedochodzaceDoSilnika": niepodpiete,
        },
        "konto": {
            "saldo": snap.balance,
            "statystyki": snap.stats,
            "obce": snap.foreign,
            "wstrzymanie": snap.halt,
        },
        "pozycje": snap.positions,
        "oczekujace": snap.pendings,
        "koszyki": snap.baskets,
        "historia": snap.closed,
        "historiaOczekujacych": snap.pending_history,
        "wiadomosci": snap.messages,
        "logPanelu": snap.logs,
        "powiazaniaKanalow": snap.bindings,
        "dziennik": { "zrodla": zrodla, "zdarzenia": dziennik },
        // Surowe wejście: każda wersja każdej wiadomości, w kolejności odbioru.
        // `wiadomosci` wyżej to migawka panelu (ostatnie 200, po jednej wersji);
        // to jest pełna historia z edycjami.
        "archiwumWiadomosci": wiadomosci_arch,
    });
    plik_json(&format!("conduit-paczka-{}.json", stempel()), &v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pole_escapuje_separator_i_cudzyslow() {
        assert_eq!(pole("abc", ','), "abc");
        assert_eq!(pole("a,b", ','), "\"a,b\"");
        assert_eq!(pole("a,b", ';'), "a,b");
        assert_eq!(pole("on \"rzekl\"", ','), "\"on \"\"rzekl\"\"\"");
        assert_eq!(pole("dwie\nlinie", ','), "\"dwie\nlinie\"");
    }

    #[test]
    fn pole_rozbraja_formule_arkusza() {
        // komentarz pozycji przychodzi z Telegrama — nie może wykonać się
        // w Excelu po dwukliku
        assert_eq!(pole("=1+1", ','), "\"'=1+1\"");
        assert_eq!(pole("@SUM(A1)", ','), "\"'@SUM(A1)\"");
        assert_eq!(pole("-A1", ','), "\"'-A1\"");
        assert_eq!(pole("+42abc", ','), "\"'+42abc\"");
    }

    #[test]
    fn liczby_zostaja_liczbami() {
        // REGRESJA: pierwsza wersja zabezpieczenia przed formułą opatrywała
        // apostrofem KAŻDĄ komórkę zaczynającą się od minusa — czyli każdą
        // stratę. Kolumna „zysk" przestawała się sumować w arkuszu.
        assert_eq!(pole("-0.57", ','), "-0.57");
        assert_eq!(pole("-1234.50", ','), "-1234.50");
        assert_eq!(pole("5", ','), "5");
        assert_eq!(pole("0.00", ','), "0.00");
        assert_eq!(pole("+3.5", ','), "+3.5");
    }

    #[test]
    fn history_export_net_matches_ui_and_preserves_unknown() {
        let make = |basis, profit, swap| {
            crate::ui::closed_from_core(&conduit_core::ClosedTrade { ticket: 1,
                side: conduit_core::Side::Buy, volume: 0.01, open_price: 100.0, close_price: 101.0,
                open_ts: 1, close_ts: 1000, profit, swap, commission: -2.0,
                reason: conduit_core::CloseReason::Partial, basket: Some(1),
                profit_basis: basis, cost_receipt: None }, "TEST")
        };
        use conduit_core::cost_receipt::ProfitBasis;
        let mut rows=vec![make(Some(ProfitBasis::PriceOnlyGross),20.0,-3.0),
            make(Some(ProfitBasis::PricePlusSwap),23.0,3.0)];
        assert_eq!(history_net_total(&rows),Some(36.0));
        let csv=tabela_historii(&rows,',');
        let data:Vec<_>=csv.lines().skip(1).map(|line|line.split(',').collect::<Vec<_>>()).collect();
        assert_eq!(data[0][15],"15.00"); assert_eq!(data[1][15],"21.00");
        assert_eq!(data[0][18],"PriceOnlyGross"); assert_eq!(data[1][18],"PricePlusSwap");
        assert_eq!(data[0][19],"known");
        rows.push(make(None,20.0,0.0));
        assert_eq!(history_net_total(&rows),None);
        let csv=tabela_historii(&rows,',');
        let unknown=csv.lines().last().unwrap().split(',').collect::<Vec<_>>();
        assert_eq!(unknown[15],""); assert_eq!(unknown[19],"unknown");
    }

    #[test]
    fn csv_zaczyna_sie_bomem_i_ma_naglowek() {
        let c = Csv::new(',', &["a", "b"]);
        assert!(
            c.buf.starts_with('\u{feff}'),
            "bez BOM Excel psuje polskie znaki"
        );
        assert!(c.buf.contains("a,b\r\n"));
    }

    #[test]
    fn filtr_zrodla_rozdziela_bota_od_obcych() {
        let f = Filtr {
            scope: Some("bot".into()),
            ..Default::default()
        };
        assert!(f.pasuje_zrodlo(ui::Origin::Bot));
        assert!(!f.pasuje_zrodlo(ui::Origin::External));
        assert!(!f.pasuje_zrodlo(ui::Origin::Manual));

        let f = Filtr {
            scope: Some("foreign".into()),
            ..Default::default()
        };
        assert!(!f.pasuje_zrodlo(ui::Origin::Bot));
        assert!(f.pasuje_zrodlo(ui::Origin::External));
        assert!(f.pasuje_zrodlo(ui::Origin::Manual));

        let f = Filtr::default();
        assert!(f.pasuje_zrodlo(ui::Origin::Bot));
        assert!(f.pasuje_zrodlo(ui::Origin::External));
    }

    #[test]
    fn zakres_dob_obejmuje_granice() {
        let q = Filtr {
            day_from: Some("2026-07-20".into()),
            day_to: Some("2026-07-22".into()),
            ..Default::default()
        };
        assert!(!doba_pasuje(&q, "2026-07-19"));
        assert!(doba_pasuje(&q, "2026-07-20"));
        assert!(doba_pasuje(&q, "2026-07-22"));
        assert!(!doba_pasuje(&q, "2026-07-23"));

        let q = Filtr {
            day: Some("2026-07-21".into()),
            ..Default::default()
        };
        assert!(doba_pasuje(&q, "2026-07-21"));
        assert!(!doba_pasuje(&q, "2026-07-20"));
    }
}
