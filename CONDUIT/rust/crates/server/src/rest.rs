//! REST dla rzeczy, które nie płyną w czasie rzeczywistym.
//!
//! Zasada podziału: **WebSocket wozi stan, REST wozi dokumenty.**
//! Lista presetów nie zmienia się 10 razy na sekundę i nie musi obciążać
//! każdego snapshotu; historia i logi bywają duże i chce się je stronicować.
//! Wpychanie tego w WS zamieniłoby protokół stanu w RPC.

use crate::auth::AuthState;
use crate::state::StateHandle;
use crate::{settings_map, ui};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

#[path = "alllogs_download.rs"]
mod alllogs_download;

pub fn router() -> Router<StateHandle> {
    Router::new()
        .route("/health", get(health))
        .route("/state", get(state))
        .route("/diag", get(diag))
        .route("/settings", get(get_settings).patch(patch_settings))
        .route("/settings/unmapped", get(unmapped))
        .route("/parse", post(parse_preview))
        .route("/presets", get(list_presets))
        .route("/drabinki", get(drabinki_gotowe))
        .route("/presets/{name}", get(get_preset))
        .route("/presets/{name}/ui", get(get_preset_ui))
        .route("/presets/{name}/settings", post(patch_preset_settings))
        .route("/history", get(history))
        .route("/history/pendings", get(pending_history))
        .route("/history-import/preview", post(crate::history_import::preview_endpoint))
        .route("/logs", get(logs))
        .route("/logs/merge", post(scal_logi))
        .route("/logs/merge/cancel", post(scal_anuluj))
        .route("/fs/dirs", get(fs_dirs))
        .route("/symbols", get(symbols_brokera))
        .route("/backtests", get(backtests))
        .route("/backtests/{id}", get(backtest))
        .route("/models", get(models))
        .route("/models/{id}", get(model))
        .route("/channels", get(channels))
        .route("/channels/{id}/photo", get(channel_photo))
        .route("/auth/state", get(auth_state))
        .route(
            "/auth/credentials",
            post(auth_credentials).delete(auth_forget),
        )
        .route("/auth/qr/start", post(auth_start))
        .route("/auth/qr.svg", get(auth_qr_svg))
        .route("/auth/2fa", post(auth_2fa))
        .route("/auth/logout", post(auth_logout))
        .route("/secrets", get(secrets_summary))
        .route("/email/test", post(email_test))
        .route("/email/subject", get(email_subject))
        .route("/shell/open-browser", post(open_browser))
        .route("/shell/reveal", post(shell_reveal))
        .route("/fs/read", post(fs_read))
        .route("/fs/download", post(alllogs_download::download))
        .route("/fs/copy", post(fs_copy))
        // świece i parametry instrumentów prosto z terminala MT5 (zespół ŚWIECE);
        // logika mieszka w `market.rs`, tu jest tylko wpięcie
        .merge(crate::market::router())
        // eksporty: historia, log panelu, dziennik zdarzeń, paczka diagnostyczna
        .nest("/export", crate::export::router())
        // laboratorium ma własny plik tras — backtesty i trening to osobny
        // świat pojęć niż presety i historia handlu
        .nest("/lab", crate::lab::rest::router())
        // tryb demo: wyszukiwanie danych, start/stop odtwarzania, ręczny sygnał
        .nest("/demo", crate::demo::rest::router())
        .nest("/kronika", crate::kronika_rest::router())
}

/// „Otwórz w przeglądarce" — wołane przyciskiem w oknie natywnym.
///
/// Dlaczego przez serwer, a nie przez API Tauri: w WebView2 `window.open()`
/// otworzyłoby kolejny webview, nie przeglądarkę systemową. Endpoint nie
/// przyjmuje ŻADNEGO parametru — otwiera wyłącznie własny adres serwera,
/// więc nie da się go użyć do uruchomienia czegokolwiek innego.
async fn open_browser(State(st): State<StateHandle>) -> Response {
    let url = st.public_url.read().clone();
    if url.is_empty() {
        return blad(StatusCode::SERVICE_UNAVAILABLE, "adres serwera nieznany");
    }
    match open::that_detached(url.as_str()) {
        Ok(()) => Json(serde_json::json!({ "ok": true, "url": url })).into_response(),
        Err(e) => blad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ============================================================
//  DIAGNOSTYKA
// ============================================================

async fn health(State(st): State<StateHandle>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "ok": true,
        "app": "conduit",
        "version": env!("CARGO_PKG_VERSION"),
        "serverTime": crate::now_ms(),
        "startedAt": st.started_at,
        "clients": st.clients(),
        "rev": st.rev(),
    }))
}

async fn state(State(st): State<StateHandle>) -> Json<ui::UiSnapshot> {
    Json(st.snapshot())
}

/// Co realnie jest podłączone. Ekran, na którym widać prawdę, a nie życzenia.
async fn diag(State(st): State<StateHandle>) -> Json<serde_json::Value> {
    let rt = st.runtime.read().name();
    let unmapped = st.read(|s| settings_map::unmapped_keys(&s.settings));
    Json(serde_json::json!({
        "runtime": rt,
        "clients": st.clients(),
        "workspace": st.workspace.root.display().to_string(),
        "backupPending": st.backup_needed(),
        "unmappedSettings": unmapped,
        "presets": st.workspace.load_presets().len(),
    }))
}

// ============================================================
//  KONFIGURACJA
// ============================================================

async fn get_settings(State(st): State<StateHandle>) -> Json<serde_json::Value> {
    Json(st.read(|s| {
        serde_json::json!({
            "settings": s.settings,
            "mode": s.mode,
            "lot": s.lot,
            "presetId": s.preset_id,
            "favorites": s.favorites,
        })
    }))
}

async fn patch_settings(
    State(st): State<StateHandle>,
    Json(patch): Json<serde_json::Value>,
) -> Response {
    match crate::commands::apply_settings_patch(&st, &patch) {
        Ok(()) => Json(serde_json::json!({ "ok": true, "rev": st.rev() })).into_response(),
        Err(e) => blad(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

async fn scal_logi(State(st): State<StateHandle>) -> Response {
    match crate::alllogs::uruchom(&st) {
        // Wraca NATYCHMIAST — postęp leci w sekcji `scalanie` migawki stanu,
        // tak samo jak postęp laboratorium. Wcześniej to żądanie wisiało do
        // końca scalania, więc pasek postępu nie miał czego pokazywać.
        Ok(()) => Json(serde_json::json!({ "ok": true, "started": true })).into_response(),
        Err(e) => blad(StatusCode::CONFLICT, e.to_string()),
    }
}

/// Anuluje trwające scalanie. Flaga jest sprawdzana w pętli po plikach
/// dziennika (tam schodzi >90 % czasu), więc reakcja jest w skali sekundy.
/// Anulowanie zostawia system czysty: zapis idzie przez `.tmp` + `rename`
/// na samym końcu, więc częściowy plik nie powstaje nigdy.
async fn scal_anuluj(State(st): State<StateHandle>) -> Response {
    if !st.read(|s| s.scalanie.aktywne) {
        return blad(StatusCode::CONFLICT, "żadne scalanie nie trwa".to_string());
    }
    crate::alllogs::ANULUJ.store(true, std::sync::atomic::Ordering::Relaxed);
    Json(serde_json::json!({ "ok": true })).into_response()
}

async fn symbols_brokera(
    State(st): State<StateHandle>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let Some(m) = st.market() else {
        return blad(
            StatusCode::SERVICE_UNAVAILABLE,
            "brak mostu do MetaTradera 5 — listy symboli brokera nie ma skąd wziąć",
        );
    };
    if !m.is_connected() {
        return blad(
            StatusCode::SERVICE_UNAVAILABLE,
            "MetaTrader 5 nie jest podłączony — listy symboli brokera nie ma skąd wziąć",
        );
    }
    let filtr = q.get("q").map(|x| x.trim().to_string()).unwrap_or_default();
    let wynik =
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<crate::market::SymbolRow>> {
            let lista = m.symbols(if filtr.is_empty() { None } else { Some(&filtr) })?;
            // Wpis symbolu bota: z listy, jeśli filtr go złapał; inaczej
            // z `symbol_info` (bufor TTL 60 s). Gdy i to zawiedzie, jedzie sam
            // wpis z digits = 0 („nieznane") — brak liczby jest lepszy niż brak
            // instrumentu bota w jego własnej wyszukiwarce. `visible: true`,
            // bo sidecar robi `symbol_select` przy starcie.
            let bot = m.default_symbol();
            let wpis_bota = lista
                .iter()
                .find(|s| s.name == bot)
                .cloned()
                .or_else(|| {
                    m.symbol(&bot).ok().map(|si| crate::market::SymbolRow {
                        name: si.symbol,
                        visible: si.visible,
                        digits: si.digits,
                        trade_mode: si.trade_mode,
                    })
                })
                .unwrap_or(crate::market::SymbolRow {
                    name: bot,
                    visible: true,
                    digits: 0,
                    trade_mode: 0,
                });
            Ok(crate::market::scal_liste_symboli(
                lista,
                wpis_bota,
                crate::market::MAX_SYMBOLS,
            ))
        })
        .await;
    match wynik {
        Ok(Ok(lista)) => Json(lista).into_response(),
        Ok(Err(e)) => blad(StatusCode::SERVICE_UNAVAILABLE, e.to_string()),
        Err(e) => blad(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("lista symboli padła: {e}"),
        ),
    }
}

async fn fs_dirs(
    State(st): State<StateHandle>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let path = q
        .get("path")
        .map(|x| x.trim().to_string())
        .unwrap_or_default();

    // Lista dysków — wejście do nawigacji „w górę" ponad korzeń napędu.
    if path == "NAPĘDY" || path == "NAPEDY" {
        let mut dyski = Vec::new();
        for litera in b'A'..=b'Z' {
            let d = format!("{}:\\", litera as char);
            if std::path::Path::new(&d).is_dir() {
                dyski.push(serde_json::json!({ "name": d.clone(), "path": d }));
            }
        }
        return Json(serde_json::json!({ "path": "NAPĘDY", "parent": null, "dirs": dyski }))
            .into_response();
    }

    let dir = if path.is_empty() {
        st.workspace.logs_dir()
    } else {
        std::path::PathBuf::from(&path)
    };
    if !dir.is_dir() {
        return blad(
            StatusCode::NOT_FOUND,
            format!("katalog nie istnieje: {}", dir.display()),
        );
    }
    let mut dirs: Vec<serde_json::Value> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| {
            serde_json::json!({
                "name": e.file_name().to_string_lossy(),
                "path": e.path().display().to_string(),
            })
        })
        .collect();
    dirs.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));

    let mut pliki: Vec<serde_json::Value> = Vec::new();
    if let Some(rozsz) = q
        .get("pliki")
        .map(|x| x.trim().to_lowercase())
        .filter(|x| !x.is_empty())
    {
        let dozwolone: Vec<&str> = rozsz
            .split(',')
            .map(|x| x.trim())
            .filter(|x| !x.is_empty())
            .collect();
        pliki = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .filter(|e| {
                dozwolone.iter().any(|x| *x == "*")
                    || e.path()
                        .extension()
                        .and_then(|x| x.to_str())
                        .map(|x| dozwolone.contains(&x.to_lowercase().as_str()))
                        .unwrap_or(false)
            })
            .map(|e| {
                let bajtow = e.metadata().map(|m| m.len()).unwrap_or(0);
                serde_json::json!({
                    "name": e.file_name().to_string_lossy(),
                    "path": e.path().display().to_string(),
                    "bytes": bajtow,
                })
            })
            .collect();
        pliki.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    }

    let parent = dir.parent().map(|p| p.display().to_string());
    Json(serde_json::json!({
        "path": dir.display().to_string(),
        // korzeń napędu nie ma rodzica w systemie plików — nawigacja „w górę"
        // prowadzi wtedy do listy dysków
        "parent": parent.filter(|p| !p.is_empty()),
        "dirs": dirs,
        "pliki": pliki,
    }))
    .into_response()
}

async fn fs_copy(
    State(st): State<StateHandle>,
    axum::extract::ConnectInfo(kto): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if !kto.ip().is_loopback() {
        return blad(
            StatusCode::CONFLICT,
            "Panel jest otwarty z innej maszyny niz bot — plik trafilby na schowek SERWERA.              Uzyj pobierania pliku.",
        );
    }
    let Some(zadana) = body.get("path").and_then(|x| x.as_str()).map(|x| x.trim()) else {
        return blad(StatusCode::BAD_REQUEST, "brak pola `path`");
    };
    let Ok(pelna) = std::path::PathBuf::from(zadana).canonicalize() else {
        return blad(
            StatusCode::NOT_FOUND,
            format!("plik nie istnieje: {zadana}"),
        );
    };
    if !pelna.is_file() {
        return blad(StatusCode::NOT_FOUND, format!("to nie jest plik: {zadana}"));
    }
    let mut obszary: Vec<std::path::PathBuf> = vec![st.workspace.root.clone()];
    if let Some(d) = crate::alllogs::katalog_docelowy(&st) {
        obszary.push(d);
    }
    let wolno = obszary
        .iter()
        .filter_map(|d| d.canonicalize().ok())
        .any(|d| pelna.starts_with(&d));
    if !wolno {
        return blad(
            StatusCode::FORBIDDEN,
            "sciezka lezy poza katalogiem bota i poza katalogiem docelowym scalania",
        );
    }
    #[cfg(windows)]
    {
        // `canonicalize` na Windows daje przedrostek `\?\`, ktorego
        // PowerShell nie przyjmuje — scinamy go PO walidacji.
        let sc = pelna.display().to_string();
        let czysta = sc.strip_prefix(r"\?\").unwrap_or(&sc).to_string();
        let wynik = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "Set-Clipboard", "-Path"])
            .arg(&czysta)
            .output();
        return match wynik {
            Ok(o) if o.status.success() => Json(serde_json::json!({
                "ok": true, "path": czysta
            }))
            .into_response(),
            Ok(o) => blad(
                StatusCode::INTERNAL_SERVER_ERROR,
                String::from_utf8_lossy(&o.stderr).to_string(),
            ),
            Err(e) => blad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
    }
    #[cfg(not(windows))]
    blad(
        StatusCode::NOT_IMPLEMENTED,
        "kopiowanie pliku na schowek dziala tylko na Windows",
    )
}

async fn fs_read(State(st): State<StateHandle>, Json(body): Json<serde_json::Value>) -> Response {
    const SUFIT: u64 = 32 * 1024 * 1024;

    let Some(zadana) = body.get("path").and_then(|x| x.as_str()).map(|x| x.trim()) else {
        return blad(StatusCode::BAD_REQUEST, "brak pola `path`");
    };
    if zadana.is_empty() {
        return blad(StatusCode::BAD_REQUEST, "pusta ścieżka");
    }
    let Ok(pelna) = std::path::PathBuf::from(zadana).canonicalize() else {
        return blad(
            StatusCode::NOT_FOUND,
            format!("plik nie istnieje: {zadana}"),
        );
    };
    if !pelna.is_file() {
        return blad(StatusCode::NOT_FOUND, format!("to nie jest plik: {zadana}"));
    }
    let mut obszary: Vec<std::path::PathBuf> = vec![st.workspace.root.clone()];
    if let Some(d) = crate::alllogs::katalog_docelowy(&st) {
        obszary.push(d);
    }
    let wolno = obszary
        .iter()
        .filter_map(|d| d.canonicalize().ok())
        .any(|d| pelna.starts_with(&d));
    if !wolno {
        return blad(
            StatusCode::FORBIDDEN,
            "ścieżka leży poza katalogiem bota i poza katalogiem docelowym scalania",
        );
    }
    match std::fs::metadata(&pelna) {
        Ok(m) if m.len() > SUFIT => {
            return blad(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!(
                    "plik ma {:.1} MB — za dużo na schowek (sufit {} MB)",
                    m.len() as f64 / 1_048_576.0,
                    SUFIT / 1_048_576
                ),
            )
        }
        Err(e) => return blad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        _ => {}
    }
    // `from_utf8_lossy`, bo dziennik może nieść bajty spoza UTF-8 (fragmenty
    // cudzych plików, znaki z konsoli). Odmowa z tego powodu byłaby gorsza niż
    // podmiana kilku znaków na znak zastępczy — użytkownik chce treść, nie czystość.
    match std::fs::read(&pelna) {
        Ok(bajty) => {
            let tekst = String::from_utf8_lossy(&bajty).into_owned();
            Json(serde_json::json!({
                "ok": true,
                "path": pelna.display().to_string(),
                "bytes": bajty.len(),
                "text": tekst,
            }))
            .into_response()
        }
        Err(e) => blad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn shell_reveal(
    State(st): State<StateHandle>,
    axum::extract::ConnectInfo(kto): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if !kto.ip().is_loopback() {
        return blad(
            StatusCode::CONFLICT,
            "Panel jest otwarty z innej maszyny niż bot — Eksplorator otworzyłby się na serwerze. \
             Skopiuj ścieżkę i otwórz ją u siebie.",
        );
    }
    let Some(zadana) = body.get("path").and_then(|x| x.as_str()).map(|x| x.trim()) else {
        return blad(StatusCode::BAD_REQUEST, "brak pola `path`");
    };
    if zadana.is_empty() {
        return blad(StatusCode::BAD_REQUEST, "pusta ścieżka");
    }
    let sciezka = std::path::PathBuf::from(zadana);
    let Ok(pelna) = sciezka.canonicalize() else {
        return blad(
            StatusCode::NOT_FOUND,
            format!("plik nie istnieje: {zadana}"),
        );
    };
    if !pelna.is_file() {
        return blad(StatusCode::NOT_FOUND, format!("to nie jest plik: {zadana}"));
    }

    // Dozwolone obszary: katalog bota i katalog docelowy scalania (ten drugi
    // bywa gdzie indziej — użytkownik wskazuje go w panelu).
    let mut obszary: Vec<std::path::PathBuf> = vec![st.workspace.root.clone()];
    if let Some(d) = crate::alllogs::katalog_docelowy(&st) {
        obszary.push(d);
    }
    let wolno = obszary
        .iter()
        .filter_map(|d| d.canonicalize().ok())
        .any(|d| pelna.starts_with(&d));
    if !wolno {
        return blad(
            StatusCode::FORBIDDEN,
            "ścieżka leży poza katalogiem bota i poza katalogiem docelowym scalania",
        );
    }

    match odslon_w_eksploratorze(&pelna) {
        Ok(()) => Json(serde_json::json!({ "ok": true, "path": pelna.display().to_string() }))
            .into_response(),
        Err(e) => blad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[cfg(windows)]
fn odslon_w_eksploratorze(p: &std::path::Path) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    // `canonicalize` na Windows daje przedrostek `\\?\`, którego Eksplorator
    // NIE ROZUMIE — okno otwiera się wtedy na „Dokumentach" bez słowa
    // wyjaśnienia. Ścinamy go tuż przed wywołaniem, już PO walidacji ścieżki.
    let s = p.display().to_string();
    let czysta = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{czysta}\""))
        .spawn()
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("nie udało się uruchomić Eksploratora: {e}"))
}

#[cfg(not(windows))]
fn odslon_w_eksploratorze(p: &std::path::Path) -> anyhow::Result<()> {
    // Poza Windows nie ma jednego „zaznacz plik" — otwieramy katalog.
    let dir = p.parent().unwrap_or(p);
    open::that_detached(dir).map_err(|e| anyhow::anyhow!("nie udało się otworzyć katalogu: {e}"))
}

/// Klucze panelu, których silnik nie czyta — jawnie, zamiast po cichu.
async fn unmapped(State(st): State<StateHandle>) -> Json<Vec<String>> {
    Json(st.read(|s| settings_map::unmapped_keys(&s.settings)))
}

#[derive(Debug, Deserialize)]
pub struct ParseReq {
    pub text: String,
}

async fn parse_preview(Json(body): Json<ParseReq>) -> Response {
    let tekst = body.text.trim();
    if tekst.is_empty() {
        return blad(StatusCode::BAD_REQUEST, "pusta wiadomość");
    }
    let rozbior = crate::demo::parsuj(tekst);
    let rodzaje: Vec<&str> = rozbior.iter().map(|p| p.kind.as_str()).collect();
    let rozpoznane = rozbior.iter().any(|p| p.kind != "INFO");
    Json(serde_json::json!({
        "ok": true,
        "recognized": rozpoznane,
        "types": rodzaje,
        "parsed": rozbior,
        // Dosłownie to, co zobaczył parser — żeby dało się odróżnić „parser
        // nie umie tego formatu" od „wkleiło się co innego, niż się wydaje"
        // (niewidoczne znaki, twarde spacje, dwie wiadomości sklejone w jedną).
        "input": {
            "text": tekst,
            "chars": tekst.chars().count(),
            "lines": tekst.lines().count(),
        }
    }))
    .into_response()
}

async fn list_presets(State(st): State<StateHandle>) -> Json<Vec<serde_json::Value>> {
    let p = st.workspace.load_presets();
    Json(
        p.into_iter()
            .map(|x| {
                serde_json::json!({
                    "name": x.name,
                    "description": x.description,
                    // FORMAT, POD KTÓRY PRESET POWSTAŁ.
                    //
                    // Bez tego pola panel nie ma jak odfiltrować listy per
                    // format i musi zgadywać — a wymaganie jest twarde:
                    // w liście formatu ATFX nie może pojawić się preset
                    // strojony pod Synergy. Domyślna wartość (`"ATFX"`)
                    // pochodzi z `Preset::format`, więc starsze pliki
                    // presetów działają bez zmian.
                    "format": x.format,
                    "settings": x.settings,
                })
            })
            .collect(),
    )
}

async fn drabinki_gotowe() -> Json<Vec<crate::ui::NazwanaDrabinka>> {
    Json(crate::ui::drabinki_wbudowane())
}

async fn get_preset(State(st): State<StateHandle>, Path(name): Path<String>) -> Response {
    match st
        .workspace
        .load_presets()
        .into_iter()
        .find(|p| p.name.eq_ignore_ascii_case(&name))
    {
        Some(p) => Json(p).into_response(),
        None => blad(StatusCode::NOT_FOUND, format!("nie ma presetu „{name}”")),
    }
}

/// Ustawienia presetu W KSZTAŁCIE PANELU — do edycji per preset.
///
/// Plik presetu trzyma pola w nazewnictwie SILNIKA; kontrolki panelu mówią
/// nazewnictwem UI. Tłumaczenie robi ta sama para funkcji, którą przechodzi
/// każde wczytanie presetu (`preset_to_ui`), więc nie powstaje trzecia
/// konwencja nazw. Bez tego endpointu edycja per preset musiałaby zgadywać
/// kształt — a klasa błędów „preset↔panel gubi pola" jest u nas notowana.
async fn get_preset_ui(State(st): State<StateHandle>, Path(name): Path<String>) -> Response {
    match st
        .workspace
        .load_presets()
        .into_iter()
        .find(|p| p.name.eq_ignore_ascii_case(&name))
    {
        Some(p) => match serde_json::to_value(&p.settings) {
            Ok(v) => Json(serde_json::json!({
                "name": p.name,
                "format": p.format,
                "settings": crate::settings_map::preset_to_ui(&v),
            }))
            .into_response(),
            Err(e) => blad(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("serializacja: {e}"),
            ),
        },
        None => blad(StatusCode::NOT_FOUND, format!("nie ma presetu „{name}”")),
    }
}

async fn patch_preset_settings(
    State(st): State<StateHandle>,
    Path(name): Path<String>,
    Json(patch): Json<serde_json::Value>,
) -> Response {
    if !patch.is_object() {
        return blad(
            StatusCode::BAD_REQUEST,
            "łatka musi być obiektem JSON".to_string(),
        );
    }
    let Some(mut preset) = st
        .workspace
        .load_presets()
        .into_iter()
        .find(|p| p.name.eq_ignore_ascii_case(&name))
    else {
        return blad(StatusCode::NOT_FOUND, format!("nie ma presetu „{name}”"));
    };
    let stare = match serde_json::to_value(&preset.settings) {
        Ok(v) => v,
        Err(e) => {
            return blad(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("serializacja: {e}"),
            )
        }
    };
    let mut doc = crate::settings_map::preset_to_ui(&stare);
    if let Err(error) = crate::settings_map::validate_t100_patch(&doc, &patch) {
        return blad(StatusCode::BAD_REQUEST, error);
    }
    crate::settings_map::merge_patch(&mut doc, &patch);
    preset.settings = crate::settings_map::core_from_ui(&doc);
    if let Err(e) = st.workspace.save_preset(&preset) {
        return blad(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("zapis presetu: {e}"),
        );
    }
    let klucze: Vec<String> = patch
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    st.log(
        "settings",
        "info",
        format!("Preset {} zmieniony z panelu", preset.name),
        format!(
            "Pola: {}. Zapisane do pliku presetu — silnik formatu przeładuje je \
             w ciągu 2 s (na żywo), a zmiana przeżywa restart.",
            klucze.join(", ")
        ),
    );
    Json(serde_json::json!({ "ok": true })).into_response()
}

/// Czaty konta + zapisane powiązania.
///
/// `channels` to PRAWDZIWA lista z Telegrama. Gdy klient nie jest zalogowany
/// (albo binarkę zbudowano bez MTProto), lista jest pusta, a `channelsError`
/// mówi dlaczego — panel ma wtedy napisać „zaloguj się", zamiast podstawiać
/// wymyślone kanały. Zaznaczenie fikcyjnego identyfikatora znaczy „nie
/// handluj nigdy": bot porównuje `chat_id` wiadomości z powiązaniem.
async fn channels(State(st): State<StateHandle>) -> Json<serde_json::Value> {
    let a = st.auth.clone();
    let (lista, blad) = match tokio::task::spawn_blocking(move || a.list_channels()).await {
        Ok(Ok(v)) => (v, None),
        Ok(Err(e)) => (Vec::new(), Some(e.to_string())),
        Err(e) => (
            Vec::new(),
            Some(format!("pobranie listy czatów nie doszło do skutku: {e}")),
        ),
    };
    Json(st.read(|s| {
        serde_json::json!({
            "bindings": s.bindings,
            "channels": lista,
            "channelsError": blad,
        })
    }))
}

/// MINIATURA ZDJĘCIA PROFILOWEGO KANAŁU.
///
/// Plik przychodzi z pamięci podręcznej klienta Telegrama (`cache/kanaly/`);
/// jeśli go tam jeszcze nie ma, implementacja cechy pobierze go raz i zapisze.
///
/// Brak zdjęcia to **404 z czytelnym komunikatem**, nie pusty obrazek: pusty
/// PNG wygląda w interfejsie jak dziura i nie da się go odróżnić od awarii,
/// a 404 pozwala przeglądarce od razu odpalić `onError` i zostawić literkę.
async fn channel_photo(
    State(st): State<StateHandle>,
    Path(id): Path<i64>,
    headers: axum::http::HeaderMap,
) -> Response {
    let a = st.auth.clone();
    let sciezka = match tokio::task::spawn_blocking(move || a.channel_photo(id)).await {
        Ok(Ok(Some(p))) => p,
        Ok(Ok(None)) => return blad(StatusCode::NOT_FOUND, "ten czat nie ma zdjęcia profilowego"),
        Ok(Err(e)) => return blad(StatusCode::BAD_GATEWAY, e.to_string()),
        Err(e) => return blad(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let Ok(dane) = std::fs::read(&sciezka) else {
        return blad(
            StatusCode::NOT_FOUND,
            "miniatura zniknęła z pamięci podręcznej",
        );
    };

    // Znacznik treści liczony z rozmiaru i czasu zapisu pliku. Wystarczy:
    // plik podmienia się tylko wtedy, gdy Telegram wyda nowe `photo_id`.
    let etag = std::fs::metadata(&sciezka)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| format!("\"{}-{}\"", dane.len(), d.as_secs()))
        .unwrap_or_else(|| format!("\"{}\"", dane.len()));

    if headers
        .get(axum::http::header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        == Some(etag.as_str())
    {
        let mut res = Response::new(axum::body::Body::empty());
        *res.status_mut() = StatusCode::NOT_MODIFIED;
        naglowki_obrazka(res.headers_mut(), &etag);
        return res;
    }

    let mut res = Response::new(axum::body::Body::from(dane));
    naglowki_obrazka(res.headers_mut(), &etag);
    res
}

/// Nagłówki miniatury: typ MIME + zgoda na zapamiętanie przez przeglądarkę.
///
/// `private`, bo to zawartość konta użytkownika — nie ma prawa wylądować
/// w pamięci podręcznej pośrednika. Doba wystarcza: przy podmianie zdjęcia
/// zmienia się `?v=` w adresie, więc przeglądarka i tak zapyta o nowy zasób.
fn naglowki_obrazka(h: &mut axum::http::HeaderMap, etag: &str) {
    use axum::http::{header, HeaderValue};
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=86400"),
    );
    if let Ok(v) = HeaderValue::from_str(etag) {
        h.insert(header::ETAG, v);
    }
}

// ============================================================
//  HISTORIA I LOGI
// ============================================================

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    /// dolna granica czasu zamknięcia (ms epoki)
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// filtr powodu zamknięcia, np. `TP`
    pub reason: Option<String>,
    pub limit: Option<usize>,
}

async fn history(
    State(st): State<StateHandle>,
    Query(q): Query<HistoryQuery>,
) -> Json<Vec<ui::ClosedPosition>> {
    let limit = q.limit.unwrap_or(500).min(5000);
    Json(st.read(|s| {
        s.closed
            .iter()
            .filter(|c| q.from.map(|f| c.close_time >= f).unwrap_or(true))
            .filter(|c| q.to.map(|t| c.close_time <= t).unwrap_or(true))
            .filter(|c| match &q.reason {
                Some(r) => {
                    serde_json::to_value(c.reason)
                        .ok()
                        .and_then(|v| v.as_str().map(|s| s.to_string()))
                        == Some(r.clone())
                }
                None => true,
            })
            .take(limit)
            .cloned()
            .collect()
    }))
}

async fn pending_history(
    State(st): State<StateHandle>,
    Query(q): Query<HistoryQuery>,
) -> Json<Vec<ui::PendingHistoryItem>> {
    let limit = q.limit.unwrap_or(500).min(5000);
    Json(st.read(|s| s.pending_history.iter().take(limit).cloned().collect()))
}

#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    pub category: Option<String>,
    pub level: Option<String>,
    pub search: Option<String>,
    pub limit: Option<usize>,
}

async fn logs(
    State(st): State<StateHandle>,
    Query(q): Query<LogsQuery>,
) -> Json<Vec<ui::LogEntry>> {
    let limit = q.limit.unwrap_or(300).min(5000);
    let szukaj = q.search.map(|s| s.to_lowercase());
    Json(st.read(|s| {
        s.logs
            .iter()
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
            .take(limit)
            .cloned()
            .collect()
    }))
}

/// Wyniki backtestów leżą jako pliki JSON w `backtests/` katalogu roboczego —
/// wypuszcza je `conduit-backtest`, serwer tylko je podaje.
async fn backtests(State(st): State<StateHandle>) -> Json<Vec<serde_json::Value>> {
    let dir = st.workspace.root.join("backtests");
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let mut files: Vec<_> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
            .collect();
        files.sort();
        for f in files {
            let id = f
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let rozmiar = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
            out.push(serde_json::json!({ "id": id, "bytes": rozmiar }));
        }
    }
    Json(out)
}

async fn backtest(State(st): State<StateHandle>, Path(id): Path<String>) -> Response {
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return blad(StatusCode::BAD_REQUEST, "niedozwolony identyfikator");
    }
    let f = st
        .workspace
        .root
        .join("backtests")
        .join(format!("{id}.json"));
    match std::fs::read_to_string(&f) {
        Ok(raw) => match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(v) => Json(v).into_response(),
            Err(e) => blad(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("uszkodzony wynik: {e}"),
            ),
        },
        Err(_) => blad(StatusCode::NOT_FOUND, format!("nie ma wyniku „{id}”")),
    }
}

// ============================================================
//  MODELE AI
// ============================================================

/// Lista modeli z katalogu `models/` — BEZ wag.
///
/// Wagi jednego modelu to ~165 kB JSON-a; lista dziesięciu modeli z wagami
/// byłaby kilkumegabajtową odpowiedzią, której przeglądarka i tak nie potrzebuje,
/// żeby wypełnić select. Zamiast wag idą kształty warstw i liczba parametrów,
/// czyli dokładnie to, co pokazuje karta modelu. Pełny dokument (z wagami)
/// wydaje `/api/models/{id}`.
///
/// Reszta pól (`feature_names`, `safety`, `train`, `score`) przechodzi bez
/// zmian — dzięki temu dołożenie pola do modelu w `crates/ai` nie wymaga
/// ruszania tego endpointu.
async fn models(State(st): State<StateHandle>) -> Json<Vec<serde_json::Value>> {
    let mut out = Vec::new();
    for (id, doc) in st.workspace.load_models() {
        let bajty = st
            .workspace
            .model_path(&id)
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .unwrap_or(0);
        out.push(podsumowanie_modelu(&id, doc, bajty));
    }
    Json(out)
}

async fn model(State(st): State<StateHandle>, Path(id): Path<String>) -> Response {
    let sciezka = match st.workspace.model_path(&id) {
        Some(p) => p,
        None => return blad(StatusCode::BAD_REQUEST, "niedozwolony identyfikator"),
    };
    match std::fs::read_to_string(&sciezka) {
        Ok(raw) => match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(mut v) => {
                if let Some(o) = v.as_object_mut() {
                    o.insert("id".into(), id.as_str().into());
                }
                Json(v).into_response()
            }
            Err(e) => blad(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("uszkodzony model: {e}"),
            ),
        },
        Err(_) => blad(StatusCode::NOT_FOUND, format!("nie ma modelu „{id}”")),
    }
}

/// Zamienia sieć `{dims, w, b}` na `{dims, params}` — kształt zostaje,
/// kilka tysięcy liczb znika.
fn opis_sieci(net: &serde_json::Value) -> serde_json::Value {
    let dims = net.get("dims").cloned().unwrap_or(serde_json::Value::Null);
    let sumuj = |klucz: &str| -> u64 {
        net.get(klucz)
            .and_then(|x| x.as_array())
            .map(|warstwy| {
                warstwy
                    .iter()
                    .map(|r| r.as_array().map(|x| x.len() as u64).unwrap_or(0))
                    .sum()
            })
            .unwrap_or(0)
    };
    serde_json::json!({ "dims": dims, "params": sumuj("w") + sumuj("b") })
}

fn podsumowanie_modelu(id: &str, mut doc: serde_json::Value, bajty: u64) -> serde_json::Value {
    let mut params = 0u64;
    if let Some(policy) = doc.get_mut("policy").and_then(|x| x.as_object_mut()) {
        for klucz in ["pos", "bsk"] {
            let opis = match policy.get(klucz) {
                Some(net) => opis_sieci(net),
                None => continue,
            };
            params += opis.get("params").and_then(|x| x.as_u64()).unwrap_or(0);
            policy.insert(klucz.to_string(), opis);
        }
    }
    if let Some(o) = doc.as_object_mut() {
        o.insert("id".into(), id.into());
        o.insert("bytes".into(), bajty.into());
        o.insert("params".into(), params.into());
    }
    doc
}

// ============================================================
//  LOGOWANIE DO TELEGRAMA
// ============================================================

async fn auth_state(State(st): State<StateHandle>) -> Json<AuthState> {
    let s = st.auth.state();
    zapisz_auth(&st, &s);
    Json(s)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    /// przychodzi jako liczba albo jako tekst — my.telegram.org pokazuje go
    /// obok hasza i użytkownik zwykle kopiuje oba pola jako tekst
    pub api_id: serde_json::Value,
    pub api_hash: String,
}

/// Zapisuje `api_id`/`api_hash` i podnosi klienta MTProto.
///
/// To jest KROK ZERO logowania: bez tych dwóch wartości Telegram nie wyda
/// tokenu, a więc nie ma z czego zrobić kodu QR.
async fn auth_credentials(
    State(st): State<StateHandle>,
    Json(body): Json<Credentials>,
) -> Response {
    let api_id = match &body.api_id {
        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) as i32,
        serde_json::Value::String(s) => s.trim().parse::<i32>().unwrap_or(0),
        _ => 0,
    };
    let hash = body.api_hash.trim();
    if api_id <= 0 {
        return blad(
            StatusCode::BAD_REQUEST,
            "api_id musi być liczbą dodatnią z my.telegram.org",
        );
    }
    // Hasz z my.telegram.org ma 32 znaki szesnastkowe. Sprawdzamy to TUTAJ,
    // bo inaczej błąd wyjdzie dopiero jako odmowa serwera Telegrama po
    // kilkunastu sekundach i bez wskazówki, co jest nie tak.
    if hash.len() != 32 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return blad(
            StatusCode::BAD_REQUEST,
            "api_hash ma mieć dokładnie 32 znaki szesnastkowe (skopiuj go z my.telegram.org)",
        );
    }
    let a = st.auth.clone();
    let h = hash.to_string();
    let maska = crate::secrets::mask(hash);
    match w_tle(move || a.set_credentials(api_id, &h)).await {
        Ok(s) => {
            // W logu ląduje SAM api_id — hasz nigdy, nawet we fragmencie.
            st.log(
                "session_string",
                "info",
                "Zapisano poświadczenia Telegrama",
                format!("api_id={api_id}, api_hash={maska}"),
            );
            zapisz_auth(&st, &s);
            Json(s).into_response()
        }
        Err((c, e)) => blad(c, e),
    }
}

/// Kasuje poświadczenia I sesję — „zacznij od zera".
async fn auth_forget(State(st): State<StateHandle>) -> Json<AuthState> {
    let a = st.auth.clone();
    let s = tokio::task::spawn_blocking(move || a.forget_credentials())
        .await
        .unwrap_or_else(|e| AuthState::error(format!("czyszczenie nie doszło do skutku: {e}")));
    zapisz_auth(&st, &s);
    st.log(
        "session_string",
        "warn",
        "Skasowano poświadczenia Telegrama",
        "api_id, api_hash i łańcuch sesji usunięte z secrets.json",
    );
    Json(s)
}

/// Co jest zapisane w `secrets.json` — BEZ WARTOŚCI, wyłącznie „jest/nie ma".
async fn secrets_summary(State(st): State<StateHandle>) -> Json<serde_json::Value> {
    Json(st.workspace.load_secrets().public_summary())
}

/// Przycisk „wyślij mail testowy". Odpowiada wynikiem PRAWDZIWEJ próby.
async fn email_test(State(st): State<StateHandle>) -> Response {
    let Some(n) = st.notifier.read().clone() else {
        return blad(
            StatusCode::SERVICE_UNAVAILABLE,
            "powiadomienia e-mail nie są uruchomione",
        );
    };
    n.reconfigure(crate::notify::setup_from_state(&st));
    // SMTP blokuje do 30 s — nie wolno mu zająć wątku wykonawczego axuma
    let st2 = st.clone();
    let wynik = tokio::task::spawn_blocking(move || n.send_test(&st2)).await;
    match wynik {
        Ok(Ok(opis)) => Json(serde_json::json!({ "ok": true, "detail": opis })).into_response(),
        Ok(Err(e)) => blad(StatusCode::BAD_GATEWAY, e.to_string()),
        Err(e) => blad(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("zadanie wysyłki padło: {e}"),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct SubjectQuery {
    /// Szablon do podejrzenia. Brak parametru = ten zapisany w ustawieniach,
    /// dzięki czemu `GET /api/email/subject` bez niczego odpowiada „jaki temat
    /// wyjdzie w najbliższym mailu".
    pub tpl: Option<String>,
}

/// PODGLĄD TEMATU MAILA + katalog zmiennych z wartościami „na teraz".
///
/// Dlaczego liczy to SERWER, skoro panel ma migawkę stanu i mógłby podstawiać
/// sam: bo wtedy istniałyby dwa silniki szablonów. Pierwsza rozbieżność
/// (inne zaokrąglenie, inny znak waluty, inna definicja doby handlowej)
/// zamieniłaby podgląd w obietnicę bez pokrycia — a podgląd ma jeden cel:
/// pokazać DOKŁADNIE to, co przyjdzie na skrzynkę.
///
/// Kategoria i treść zdarzenia są w podglądzie przykładowe, bo w chwili
/// oglądania ustawień żaden mail nie powstaje. Wszystkie pozostałe zmienne
/// są prawdziwe, prosto z migawki.
async fn email_subject(
    State(st): State<StateHandle>,
    Query(q): Query<SubjectQuery>,
) -> Json<serde_json::Value> {
    use crate::mailer::{i18n as mail_i18n, MailCategory};

    const PRZYKLAD_KAT: MailCategory = MailCategory::Summary;
    const PRZYKLAD_ZDARZENIE: &str = "Podsumowanie dnia";

    let now = crate::now_ms();
    let (vars, zapisany, systemowy, podglad) = st.read(|s| {
        (
            mail_i18n::variables(s, PRZYKLAD_KAT, PRZYKLAD_ZDARZENIE, now),
            s.email.subject.clone(),
            mail_i18n::system_subject(mail_i18n::Language::from_app(&s.language), PRZYKLAD_KAT, PRZYKLAD_ZDARZENIE),
            mail_i18n::subject(s, PRZYKLAD_KAT, PRZYKLAD_ZDARZENIE, q.tpl.as_deref().unwrap_or(&s.email.subject), now),
        )
    });

    let tpl = q.tpl.unwrap_or_else(|| zapisany.clone());
    let pusty = tpl.trim().is_empty();


    Json(serde_json::json!({
        "ok": true,
        "template": tpl,
        "saved": zapisany,
        "preview": podglad,
        "systemowy": systemowy,
        "pusty": pusty,
        "vars": vars.items,
    }))
}

/// Wykonuje operację logowania POZA wątkiem wykonawczym.
///
/// Prawdziwy klient MTProto rozmawia z zadaniem w tle i czeka na wynik
/// blokująco (żeby formularz hasła umiał powiedzieć „hasło nieprawidłowe",
/// a nie tylko migać stanem). Wołanie tego wprost z uchwytu axuma zablokowałoby
/// wątek wykonawczy — a przy jednowątkowym środowisku wręcz zakleszczyło
/// serwer, bo zadanie w tle nie miałoby na czym się wykonać.
async fn w_tle<F>(f: F) -> Result<AuthState, (StatusCode, String)>
where
    F: FnOnce() -> anyhow::Result<AuthState> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(s)) => Ok(s),
        Ok(Err(e)) => Err((StatusCode::SERVICE_UNAVAILABLE, e.to_string())),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("zadanie logowania padło: {e}"),
        )),
    }
}

async fn auth_start(State(st): State<StateHandle>) -> Response {
    let a = st.auth.clone();
    match w_tle(move || a.start_qr()).await {
        Ok(s) => {
            zapisz_auth(&st, &s);
            Json(s).into_response()
        }
        Err((c, e)) => blad(c, e),
    }
}

/// Sam obrazek — dla `<img src="/api/auth/qr.svg">`, gdyby ktoś wolał tak
/// niż wstawiać SVG z JSON-a.
async fn auth_qr_svg(State(st): State<StateHandle>) -> Response {
    match st.auth.state().qr_svg {
        Some(svg) => (
            [
                (axum::http::header::CONTENT_TYPE, "image/svg+xml"),
                (axum::http::header::CACHE_CONTROL, "no-store"),
            ],
            svg,
        )
            .into_response(),
        None => blad(StatusCode::NOT_FOUND, "brak aktywnego kodu QR"),
    }
}

#[derive(Debug, Deserialize)]
pub struct TwoFa {
    pub password: String,
}

async fn auth_2fa(State(st): State<StateHandle>, Json(body): Json<TwoFa>) -> Response {
    if body.password.trim().is_empty() {
        return blad(StatusCode::BAD_REQUEST, "hasło nie może być puste");
    }
    let a = st.auth.clone();
    let haslo = body.password.clone();
    match w_tle(move || a.submit_password(&haslo)).await {
        Ok(s) => {
            zapisz_auth(&st, &s);
            Json(s).into_response()
        }
        // 401, bo to najczęściej złe hasło — UI rozróżnia „błąd sieci"
        // od „hasło nieprawidłowe" po kodzie, nie po treści komunikatu
        Err((_, e)) => blad(StatusCode::UNAUTHORIZED, e),
    }
}

async fn auth_logout(State(st): State<StateHandle>) -> Json<AuthState> {
    let a = st.auth.clone();
    let s = tokio::task::spawn_blocking(move || a.logout())
        .await
        .unwrap_or_else(|e| AuthState::error(format!("wylogowanie nie doszło do skutku: {e}")));
    zapisz_auth(&st, &s);
    st.log("session_string", "warn", "Wylogowano z Telegrama", "");
    Json(s)
}

/// Stan logowania trafia też do wspólnego stanu, żeby WSZYSTKIE podłączone
/// powłoki zobaczyły zalogowanie — także ta, która go nie zainicjowała.
///
/// Zapis jest JEDEN dla całego programu (`hub::zapisz_auth`) i pilnuje go
/// dodatkowo `hub::auth_loop`. Dwie kopie tej logiki znaczyłyby dokładnie to,
/// co znaczyły: ścieżkę logowania, na której nikt migawki nie odświeża.
fn zapisz_auth(st: &StateHandle, s: &AuthState) {
    crate::hub::zapisz_auth(st, s);
}

fn blad(code: StatusCode, msg: impl Into<String>) -> Response {
    (
        code,
        Json(serde_json::json!({ "ok": false, "error": msg.into() })),
    )
        .into_response()
}

#[cfg(test)]
mod testy {
    use super::*;
    use axum::extract::ConnectInfo;

    fn stan(tag: &str) -> StateHandle {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "conduit-rest-{tag}-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let cfg = crate::ServerConfig {
            workspace: dir,
            ..Default::default()
        };
        crate::bootstrap(&cfg, crate::default_auth()).unwrap()
    }

    #[tokio::test]
    async fn t100_preset_patch_preserves_owner_and_rejects_without_writing() {
        let st = stan("t100-preset");
        let preset = conduit_core::Preset {
            name: "SYNTHETIC-T100".into(), description: String::new(), format: "Synthetic".into(),
            settings: conduit_core::Settings::default(), ea: None,
        };
        st.workspace.save_preset(&preset).unwrap();
        let path = st.workspace.presets_dir().join("SYNTHETIC-T100.json");
        let before = std::fs::read(&path).unwrap();
        let global = st.read(|s| s.settings.clone());
        let bad = patch_preset_settings(State(st.clone()), Path(preset.name.clone()),
            Json(serde_json::json!({"t100":{"risk_pct":false}}))).await;
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let ok = patch_preset_settings(State(st.clone()), Path(preset.name.clone()),
            Json(serde_json::json!({"t100":{"enabled":true,"risk_pct":2.0}}))).await;
        assert_eq!(ok.status(), StatusCode::OK);
        let changed: conduit_core::Preset = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(changed.settings.t100.enabled);
        assert_eq!(changed.settings.t100.risk_pct, 2.0);
        assert_eq!(changed.settings.t100.experts, preset.settings.t100.experts);
        assert_eq!(changed.settings.ea_enabled, preset.settings.ea_enabled);
        assert_eq!(st.read(|s| s.settings.clone()), global);
    }

    fn skad(adres: &str) -> ConnectInfo<std::net::SocketAddr> {
        ConnectInfo(adres.parse().expect("poprawny adres"))
    }

    #[tokio::test]
    async fn pokaz_plik_odrzuca_zadanie_spoza_localhosta() {
        let st = stan("reveal-zdalny");
        let plik = st.workspace.root.join("cokolwiek.txt");
        std::fs::write(&plik, b"x").unwrap();
        let r = shell_reveal(
            State(st),
            skad("192.168.1.50:51000"),
            Json(serde_json::json!({ "path": plik.display().to_string() })),
        )
        .await;
        assert_eq!(
            r.status(),
            StatusCode::CONFLICT,
            "zdalny panel MUSI dostać odmowę"
        );
    }

    /// Ścieżka spoza katalogu bota i spoza katalogu docelowego scalania nie
    /// ma prawa przejść — to jedyny endpoint uruchamiający polecenie systemowe.
    #[tokio::test]
    async fn pokaz_plik_nie_wypuszcza_poza_katalog_bota() {
        let st = stan("reveal-poza");
        let mut obcy = std::env::temp_dir();
        obcy.push(format!("conduit-obcy-{}.txt", std::process::id()));
        std::fs::write(&obcy, b"x").unwrap();
        let r = shell_reveal(
            State(st),
            skad("127.0.0.1:51000"),
            Json(serde_json::json!({ "path": obcy.display().to_string() })),
        )
        .await;
        assert_eq!(
            r.status(),
            StatusCode::FORBIDDEN,
            "ścieżka spoza obszaru MUSI być odrzucona"
        );
        let _ = std::fs::remove_file(obcy);
    }

    /// `..` nie wyprowadza poza obszar — ścieżka jest kanonizowana PRZED
    /// porównaniem, więc sztuczka z wyjściem w górę nie działa.
    #[tokio::test]
    async fn pokaz_plik_nie_daje_sie_oszukac_dwiema_kropkami() {
        let st = stan("reveal-kropki");
        let mut obcy = std::env::temp_dir();
        obcy.push(format!("conduit-obcy2-{}.txt", std::process::id()));
        std::fs::write(&obcy, b"x").unwrap();
        let podstep = st
            .workspace
            .root
            .join("..")
            .join(obcy.file_name().unwrap().to_string_lossy().to_string());
        let r = shell_reveal(
            State(st),
            skad("127.0.0.1:51000"),
            Json(serde_json::json!({ "path": podstep.display().to_string() })),
        )
        .await;
        assert_eq!(
            r.status(),
            StatusCode::FORBIDDEN,
            "`..` nie może wyprowadzić poza obszar"
        );
        let _ = std::fs::remove_file(obcy);
    }

    #[tokio::test]
    async fn pokaz_plik_nieistniejacy_to_404_a_nie_cisza() {
        let st = stan("reveal-brak");
        let r = shell_reveal(
            State(st),
            skad("127.0.0.1:51000"),
            Json(serde_json::json!({ "path": "Q:/nie/ma/takiego.txt" })),
        )
        .await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn fs_dirs_dokłada_pliki_tylko_na_zadanie() {
        let st = stan("fsdirs");
        let dir = st.workspace.logs_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("kronika.jsonl"), b"{}\n").unwrap();
        std::fs::write(dir.join("notatka.txt"), b"nie jsonl").unwrap();

        let bez = fs_dirs(State(st.clone()), axum::extract::Query(Default::default())).await;
        let bez = tresc(bez).await;
        assert_eq!(
            bez["pliki"].as_array().map(|a| a.len()),
            Some(0),
            "domyślnie BEZ plików"
        );

        let mut q = std::collections::HashMap::new();
        q.insert("pliki".to_string(), "jsonl".to_string());
        let z = fs_dirs(State(st), axum::extract::Query(q)).await;
        let z = tresc(z).await;
        let pliki = z["pliki"].as_array().expect("lista plików");
        assert_eq!(
            pliki.len(),
            1,
            "filtr rozszerzenia ma odsiać `notatka.txt`: {pliki:?}"
        );
        assert_eq!(pliki[0]["name"], "kronika.jsonl");
        assert!(
            pliki[0]["bytes"].as_u64().unwrap() > 0,
            "rozmiar pliku ma być podany"
        );
    }

    async fn tresc(r: Response) -> serde_json::Value {
        let b = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
        serde_json::from_slice(&b).unwrap()
    }
}
