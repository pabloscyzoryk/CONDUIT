//! REST laboratorium — wszystko, co nie jest strumieniem postępu.
//!
//! Podział jest ten sam, co w reszcie serwera: **WebSocket wozi stan
//! (postęp), REST wozi dokumenty** (zakres danych, listy presetów, pliki
//! wyników) i przyjmuje polecenia startu/przerwania. Zlecenie startu wraca
//! natychmiast z identyfikatorem zadania — całe liczenie widać potem
//! w sekcji `lab` snapshotu.

use super::{backtests, training};
use crate::state::StateHandle;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

pub fn router() -> Router<StateHandle> {
    Router::new()
        .route("/info", get(info))
        .route("/state", get(state))
        .route("/backtest", post(start_backtest))
        .route("/train", post(start_train))
        .route("/cancel", post(cancel))
        .route("/open-dir", post(open_dir))
        .route("/{job}/files", get(files))
        .route("/{job}/file/{name}", get(file))
}

/// Co laboratorium znalazło na dysku: ticki, sygnały, katalogi presetów,
/// modele i ewentualny punkt kontrolny do wznowienia.
async fn info(State(st): State<StateHandle>) -> Json<super::LabData> {
    // czytanie nagłówka pliku ticków i skanowanie katalogów to wejście-wyjście,
    // więc nie blokujemy nim wątku wykonawczego serwera
    let ws = st.workspace.clone();
    let d = tokio::task::spawn_blocking(move || super::discover(&ws))
        .await
        .unwrap_or_default();
    Json(d)
}

async fn state(State(st): State<StateHandle>) -> Json<super::LabState> {
    Json(st.read(|s| s.lab.clone()))
}

async fn start_backtest(
    State(st): State<StateHandle>,
    Json(req): Json<backtests::BacktestReq>,
) -> Response {
    match backtests::start(&st, req) {
        Ok(id) => Json(serde_json::json!({ "ok": true, "id": id })).into_response(),
        Err(e) => blad(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

async fn start_train(
    State(st): State<StateHandle>,
    Json(req): Json<training::TrainReq>,
) -> Response {
    match training::start(&st, req) {
        Ok(id) => Json(serde_json::json!({ "ok": true, "id": id })).into_response(),
        Err(e) => blad(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

/// „PRZERWIJ": prosi zadanie o zatrzymanie się. Odpowiedź wraca OD RAZU —
/// samo zatrzymanie trwa tyle, ile domknięcie bieżącego kroku (jedno
/// pokolenie treningu albo kawałek przebiegu backtestu), a potem zadanie
/// zapisuje wyniki i melduje fazę `cancelled`.
async fn cancel(State(st): State<StateHandle>) -> Response {
    if st.lab.request_cancel() {
        st.log(
            "backtests",
            "warn",
            "Zatrzymywanie zadania…",
            "postęp zostanie zapisany",
        );
        Json(serde_json::json!({ "ok": true })).into_response()
    } else {
        blad(StatusCode::CONFLICT, "nic się nie liczy")
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobRef {
    job: String,
}

/// Otwiera katalog wyników w eksploratorze.
///
/// Klient podaje IDENTYFIKATOR ZADANIA, nigdy ścieżkę — serwer sam składa
/// ją z własnego katalogu roboczego i sprawdza, że katalog istnieje. Dzięki
/// temu endpoint nie da się namówić na otwarcie dowolnego miejsca na dysku.
async fn open_dir(State(st): State<StateHandle>, Json(r): Json<JobRef>) -> Response {
    match super::katalog_zadania(&st, &r.job) {
        Ok(d) => match open::that_detached(&d) {
            Ok(()) => Json(serde_json::json!({ "ok": true, "path": d.display().to_string() }))
                .into_response(),
            Err(e) => blad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        },
        Err(e) => blad(StatusCode::NOT_FOUND, format!("{e:#}")),
    }
}

async fn files(State(st): State<StateHandle>, Path(job): Path<String>) -> Response {
    let dir = match super::katalog_zadania(&st, &job) {
        Ok(d) => d,
        Err(e) => return blad(StatusCode::NOT_FOUND, format!("{e:#}")),
    };
    let mut out: Vec<serde_json::Value> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let mut v: Vec<_> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        v.sort();
        for p in v {
            let nazwa = p
                .file_name()
                .map(|x| x.to_string_lossy().to_string())
                .unwrap_or_default();
            let bajty = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            out.push(serde_json::json!({ "name": nazwa, "bytes": bajty }));
        }
    }
    Json(out).into_response()
}

/// Podgląd pliku wyniku. SVG idzie z typem obrazka (żeby dało się go wstawić
/// wprost w `<img>`), reszta jako zwykły tekst.
async fn file(
    State(st): State<StateHandle>,
    Path((job, name)): Path<(String, String)>,
) -> Response {
    let p = match super::plik_zadania(&st, &job, &name) {
        Ok(p) => p,
        Err(e) => return blad(StatusCode::NOT_FOUND, format!("{e:#}")),
    };
    let tresc = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) => return blad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let typ = if name.ends_with(".svg") {
        "image/svg+xml"
    } else if name.ends_with(".json") {
        "application/json"
    } else {
        "text/plain; charset=utf-8"
    };
    (
        [
            (axum::http::header::CONTENT_TYPE, typ),
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        tresc,
    )
        .into_response()
}

fn blad(code: StatusCode, msg: impl Into<String>) -> Response {
    (
        code,
        Json(serde_json::json!({ "ok": false, "error": msg.into() })),
    )
        .into_response()
}
