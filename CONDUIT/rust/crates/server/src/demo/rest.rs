//! REST trybu demo.
//!
//! Ten sam podział, co w reszcie serwera: **WebSocket wozi stan, REST wozi
//! dokumenty i polecenia**. Postęp przebiegu płynie sekcją `demo` snapshotu,
//! więc okno natywne i karta przeglądarki widzą go równocześnie; tutaj są
//! tylko rzeczy, które się nie strumieniują — wyszukiwanie plików, rozpoznanie
//! wskazanej ścieżki, start, stop i zmiana tempa.
//!
//! **O ścieżkach.** W odróżnieniu od laboratorium (gdzie klient podaje NAZWĘ
//! z listy, którą sam dostał) tryb demo przyjmuje ŚCIEŻKĘ — bo wymaganie brzmi
//! „przeciągnij plik albo wpisz ścieżkę ręcznie". Serwer nasłuchuje wyłącznie
//! na pętli zwrotnej, a te trasy tylko CZYTAJĄ nagłówek pliku i nigdy niczego
//! nie zapisują ani nie uruchamiają.

use super::{sniff, DemoConfig, DemoState};
use crate::state::StateHandle;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

pub fn router() -> Router<StateHandle> {
    Router::new()
        .route("/state", get(state))
        .route("/config", get(config).post(save_config))
        .route("/scan", post(scan))
        .route("/inspect", post(inspect))
        .route("/locate", post(locate))
        .route("/start", post(start))
        .route("/stop", post(stop))
        .route("/speed", post(speed))
        .route("/signal", post(signal))
}

async fn state(State(st): State<StateHandle>) -> Json<DemoState> {
    Json(st.read(|s| s.demo.clone()))
}

async fn config(State(st): State<StateHandle>) -> Json<DemoConfig> {
    Json(st.read(|s| s.demo.config.clone()))
}

/// Zapis konfiguracji BEZ uruchamiania — żeby ustawienia przetrwały restart
/// i żeby dało się je przygotować, zanim dane będą pod ręką.
async fn save_config(State(st): State<StateHandle>, Json(cfg): Json<DemoConfig>) -> Response {
    if let Err(e) = cfg.validate() {
        return blad(StatusCode::BAD_REQUEST, format!("{e:#}"));
    }
    if let Err(e) = st.workspace.save_demo(&cfg) {
        return blad(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"));
    }
    st.update(
        crate::coalesce::Sections::one(crate::coalesce::Section::Demo),
        |s| {
            s.demo.config = cfg.clone();
            if !s.demo.running {
                s.demo.speed = cfg.speed;
            }
        },
    );
    Json(serde_json::json!({ "ok": true })).into_response()
}

// ============================================================
//  SZUKANIE DANYCH
// ============================================================

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScanReq {
    /// głębokość schodzenia w podkatalogi (domyślnie 3)
    depth: Option<usize>,
    /// twardy limit czasu w ms (domyślnie 4000)
    timeout_ms: Option<u64>,
    /// dodatkowy katalog wskazany ręcznie
    root: Option<String>,
}

/// Przeszukuje pobliskie katalogi i zwraca to, co ROZPOZNAŁ PO ZAWARTOŚCI.
async fn scan(State(st): State<StateHandle>, Json(r): Json<Option<ScanReq>>) -> Response {
    let r = r.unwrap_or_default();
    let opts = sniff::ScanOpts {
        depth: r.depth.unwrap_or(3).min(8),
        budget: Duration::from_millis(r.timeout_ms.unwrap_or(4000).clamp(50, 30_000)),
        extra_root: r.root.filter(|x| !x.trim().is_empty()).map(PathBuf::from),
        ..Default::default()
    };
    let root = st.workspace.root.clone();
    // Skanowanie to czyste wejście-wyjście po katalogach — nie na wątku
    // wykonawczym serwera, bo zablokowałoby obsługę WebSocketów.
    let wynik = tokio::task::spawn_blocking(move || sniff::scan(&root, &opts)).await;
    match wynik {
        Ok(w) => Json(w).into_response(),
        Err(e) => blad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PathReq {
    path: String,
}

/// Rozpoznaje POJEDYNCZY plik wskazany ręcznie albo upuszczony w oknie.
async fn inspect(Json(r): Json<PathReq>) -> Response {
    let p = PathBuf::from(r.path.trim().trim_matches('"'));
    if !p.exists() {
        return blad(
            StatusCode::NOT_FOUND,
            format!("nie ma pliku: {}", p.display()),
        );
    }
    let sciezka = p.clone();
    match tokio::task::spawn_blocking(move || sniff::sniff(&sciezka)).await {
        Ok(Some(c)) => Json(c).into_response(),
        Ok(None) => blad(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "„{}” nie wygląda ani na ticki, ani na sygnały — sprawdziłem nagłówek CDTK, \
                 klucze JSON-a i pierwsze wiersze tekstu",
                p.display()
            ),
        ),
        Err(e) => blad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocateReq {
    name: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    depth: Option<usize>,
}

/// Szuka pliku po NAZWIE I ROZMIARZE wśród pobliskich katalogów.
///
/// Istnieje dla przeciągania plików: przeglądarka podaje nazwę i rozmiar
/// upuszczonego pliku, ale — z powodów bezpieczeństwa — nie podaje ścieżki.
/// Dopasowanie po rozmiarze co do bajta jest wystarczająco jednoznaczne,
/// żeby po upuszczeniu `ticks.bin` trafić w ten właściwy, a nie w kopię.
async fn locate(State(st): State<StateHandle>, Json(r): Json<LocateReq>) -> Response {
    let nazwa = r.name.trim().to_string();
    if nazwa.is_empty() {
        return blad(StatusCode::BAD_REQUEST, "pusta nazwa pliku");
    }
    let opts = sniff::ScanOpts {
        depth: r.depth.unwrap_or(4).min(8),
        budget: Duration::from_millis(6000),
        ..Default::default()
    };
    let root = st.workspace.root.clone();
    let wynik = match tokio::task::spawn_blocking(move || sniff::scan(&root, &opts)).await {
        Ok(w) => w,
        Err(e) => return blad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let mut trafienia: Vec<sniff::Candidate> = wynik
        .candidates
        .into_iter()
        .filter(|c| c.name == nazwa)
        .collect();
    if let Some(sz) = r.size {
        // rozmiar zgodny co do bajta rozstrzyga między kopiami tej samej nazwy
        let dokladne: Vec<sniff::Candidate> = trafienia
            .iter()
            .filter(|c| c.bytes == sz)
            .cloned()
            .collect();
        if !dokladne.is_empty() {
            trafienia = dokladne;
        }
    }
    Json(serde_json::json!({
        "ok": !trafienia.is_empty(),
        "matches": trafienia,
        "scannedFiles": wynik.files_seen,
        "truncated": wynik.truncated,
    }))
    .into_response()
}

// ============================================================
//  STEROWANIE PRZEBIEGIEM
// ============================================================

async fn start(State(st): State<StateHandle>, Json(cfg): Json<DemoConfig>) -> Response {
    match super::start(&st, cfg) {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => blad(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

async fn stop(State(st): State<StateHandle>) -> Response {
    match super::stop(&st) {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => blad(StatusCode::CONFLICT, format!("{e:#}")),
    }
}

#[derive(Debug, Deserialize)]
struct SpeedReq {
    /// 0 = maksymalne tempo
    speed: f64,
}

/// Zmiana tempa W TRAKCIE przebiegu — bez restartu, bez utraty pozycji.
async fn speed(State(st): State<StateHandle>, Json(r): Json<SpeedReq>) -> Response {
    if !r.speed.is_finite() || r.speed < 0.0 {
        return blad(
            StatusCode::BAD_REQUEST,
            "tempo musi być liczbą ≥ 0 (0 = maksymalne)",
        );
    }
    st.demo.set_speed(r.speed);
    st.update(
        crate::coalesce::Sections::one(crate::coalesce::Section::Demo),
        |s| {
            s.demo.speed = r.speed;
            s.demo.config.speed = r.speed;
        },
    );
    Json(serde_json::json!({ "ok": true, "speed": r.speed })).into_response()
}

/// Wstrzyknięcie wiadomości przez REST — te same pola, co w komendzie
/// WebSocket `simulateMessage` (patrz [`crate::wstrzykniecie`]).
///
/// Ta droga istnieje po to, żeby dało się sterować botem ze skryptu (chaos,
/// I3). Gdyby została przy samym `text`, skrypt nie mógłby wysłać edycji —
/// czyli dokładnie tego, do czego F5 jest potrzebne.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignalReq {
    text: String,
    #[serde(default)]
    account_session: Option<String>,
    #[serde(default)]
    channel_id: Option<i64>,
    #[serde(default)]
    topic_id: Option<i64>,
    #[serde(default)]
    msg_id: Option<i64>,
    #[serde(default)]
    reply_to: Option<i64>,
    #[serde(default)]
    edit_of: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SignalResp {
    ok: bool,
    /// dokąd trafiła wiadomość: `demo` albo `engine`
    target: String,
    /// NUMER, pod jakim wiadomość poszła do silnika.
    ///
    /// Bez tego pola wstrzyknięcie jest jednorazowe: skrypt nie wie, do czego
    /// odnieść późniejszą edycję czy odpowiedź. Zwracany jest zawsze — także
    /// wtedy, gdy numer został nadany automatycznie.
    msg_id: i64,
    parsed: Vec<crate::ui::ParsedSignal>,
}

/// Ręczne wysłanie wiadomości. **Działa także poza trybem demo** — wtedy leci
/// do prawdziwego środowiska uruchomieniowego.
async fn signal(State(st): State<StateHandle>, Json(r): Json<SignalReq>) -> Response {
    let parsed = super::parsuj(&r.text);
    let cel = if st.demo.running() { "demo" } else { "engine" };
    // Rozpakowanie WYCZERPUJĄCE (bez `..`): nowe pole w `SignalReq` przestanie
    // się kompilować tutaj, zamiast po cichu nie dojechać do silnika.
    let SignalReq {
        text,
        account_session,
        channel_id,
        topic_id,
        msg_id,
        reply_to,
        edit_of,
    } = r;
    let cmd = crate::proto::Command::SimulateMessage {
        text,
        channel_id,
        topic_id,
        msg_id,
        reply_to,
        edit_of,
    };
    match super::manual_signal_scoped(&st, &cmd, account_session.as_deref()) {
        Ok(numer) => Json(SignalResp {
            ok: true,
            target: cel.into(),
            msg_id: numer,
            parsed,
        })
        .into_response(),
        Err(e) => blad(StatusCode::SERVICE_UNAVAILABLE, format!("{e:#}")),
    }
}

fn blad(code: StatusCode, msg: impl Into<String>) -> Response {
    (
        code,
        Json(serde_json::json!({ "ok": false, "error": msg.into() })),
    )
        .into_response()
}
