
use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use conduit_server::kronika as kr;
use parking_lot::Mutex;
use rust_embed::RustEmbed;
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Zbudowany interfejs (`npm run build:kronika` wsypuje tu `index.html` + `assets/`).
#[derive(RustEmbed)]
#[folder = "web/"]
struct Zasoby;

const WERSJA: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Deserialize)]
struct Sekrety {
    api_id: i32,
    api_hash: String,
}

// ============================================================
//  ARGUMENTY
// ============================================================

struct Argumenty {
    sekrety: PathBuf,
    sesja: PathBuf,
    /// katalog roboczy — tu leży `kronika.json` i (domyślnie) plik zapisu
    katalog: PathBuf,
    /// wymuszona ścieżka pliku; nadpisuje zapisane ustawienia
    plik: Option<String>,
    port: u16,
    bez_interfejsu: bool,
    bez_przegladarki: bool,
}

const POMOC: &str = "\
kronika.exe — rejestrator strumienia Telegrama (CONDUIT)

  --sekrety <plik>     JSON z api_id i api_hash (domyślnie secrets_kronika.json)
  --sesja <plik>       WŁASNY plik sesji (domyślnie kronika.session)
  --katalog <kat>      katalog roboczy: kronika.json i plik zapisu (domyślnie .)
  --plik <nazwa>       JEDEN ciągły plik zapisu (domyślnie kronika.jsonl)
  --port <numer>       wymuś port; domyślnie system przydziela wolny
  --bez-interfejsu     tylko konsola, bez serwera i przeglądarki (VPS)
  --bez-przegladarki   podnieś serwer, ale nie otwieraj przeglądarki
  --eksport <plik|kat> zamień kronikę na zbiór backtestowy i zakończ
  --do <plik>          cel eksportu (domyślnie signals_kronika.json)
  --pomoc, -h          ten opis

UWAGA: nie kopiuj tu telegram.session bota. Ten sam plik sesji w dwóch
procesach powoduje AUTH_KEY_DUPLICATED i Telegram wyłącza OBIE strony.
";

enum Zamiar {
    Nagrywaj(Box<Argumenty>),
    Eksportuj { zrodlo: PathBuf, cel: PathBuf },
    Nic,
}

fn czytaj_argumenty() -> Result<Zamiar> {
    let mut a = Argumenty {
        sekrety: PathBuf::from("secrets_kronika.json"),
        sesja: PathBuf::from("kronika.session"),
        katalog: PathBuf::from("."),
        plik: None,
        port: 0,
        bez_interfejsu: false,
        bez_przegladarki: false,
    };
    let mut it = std::env::args().skip(1).peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--pomoc" | "-h" | "--help" | "/?" => {
                println!("{POMOC}");
                return Ok(Zamiar::Nic);
            }
            "--sekrety" => a.sekrety = it.next().context("--sekrety wymaga ścieżki")?.into(),
            "--sesja" => a.sesja = it.next().context("--sesja wymaga ścieżki")?.into(),
            // `--wyjscie` to nazwa z pierwszej wersji, gdy zapis szedł do katalogu
            // z plikami na dobę. Przyjmujemy ją dalej jako katalog roboczy, żeby
            // nie zepsuć nikomu skrótu ani zadania w harmonogramie.
            "--katalog" | "--wyjscie" => {
                a.katalog = it.next().context("--katalog wymaga ścieżki")?.into()
            }
            "--plik" => a.plik = Some(it.next().context("--plik wymaga nazwy")?),
            "--port" => {
                a.port = it
                    .next()
                    .context("--port wymaga numeru")?
                    .parse()
                    .context("--port musi być liczbą 0-65535")?
            }
            "--bez-interfejsu" | "--konsola" => a.bez_interfejsu = true,
            "--bez-przegladarki" | "--no-browser" => a.bez_przegladarki = true,
            "--eksport" => {
                let zrodlo = PathBuf::from(it.next().context("--eksport wymaga ścieżki")?);
                let mut cel = PathBuf::from("signals_kronika.json");
                if it.peek().map(|x| x == "--do").unwrap_or(false) {
                    it.next();
                    if let Some(c) = it.next() {
                        cel = PathBuf::from(c);
                    }
                }
                return Ok(Zamiar::Eksportuj { zrodlo, cel });
            }
            inne => anyhow::bail!("nieznany argument „{inne}” — użyj --pomoc"),
        }
    }
    Ok(Zamiar::Nagrywaj(Box::new(a)))
}

// ============================================================
//  STAN PROCESU
// ============================================================

/// Wszystko, co widzi zarówno wątek odbioru, jak i serwer HTTP.
struct Stan {
    kronika: Mutex<kr::Kronika>,
    katalog: PathBuf,
    /// gdzie leży `kronika.json`
    konfig: PathBuf,
    /// lista dialogów z Telegrama — odświeżana przy starcie i na żądanie
    kanaly: Mutex<Vec<kr::KanalInfo>>,
    zywe: std::sync::atomic::AtomicBool,
    opis_zrodla: Mutex<String>,
}

type Uchwyt = Arc<Stan>;

fn wczytaj_ustawienia(p: &Path) -> kr::Ustawienia {
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn zapisz_ustawienia(p: &Path, u: &kr::Ustawienia) -> Result<()> {
    if let Some(d) = p.parent() {
        if !d.as_os_str().is_empty() {
            std::fs::create_dir_all(d)?;
        }
    }
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(u)?)?;
    std::fs::rename(&tmp, p)?;
    Ok(())
}

// ============================================================
//  TRASY — kontrakt wspólny z Conduitem (patrz `kr::api`)
// ============================================================

fn blad(kod: StatusCode, tresc: impl Into<String>) -> Response {
    (
        kod,
        Json(serde_json::json!({ "ok": false, "error": tresc.into() })),
    )
        .into_response()
}

async fn zdrowie() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "app": "conduit", "tool": "kronika", "version": WERSJA }))
}

async fn stan(State(st): State<Uchwyt>) -> Json<kr::Stan> {
    let k = st.kronika.lock();
    let sciezka = k.sciezka().to_path_buf();
    Json(kr::Stan {
        ok: true,
        wersja: WERSJA.to_string(),
        tryb: kr::Tryb::Samodzielna,
        plik: sciezka.display().to_string(),
        // Samodzielna kronika ma WŁASNY katalog i własną domyślną nazwę —
        // nie dziedziczy pulpitu po Conduicie, bo to osobny produkt
        // uruchamiany tam, gdzie użytkownik go położył.
        domyslny_plik: st.katalog.join(kr::PLIK_DOMYSLNY).display().to_string(),
        istnieje: sciezka.is_file(),
        rozpoznanie: Some(k.rozpoznanie().clone()),
        kopia: k.ostatnia_kopia().map(|p| p.display().to_string()),
        bajtow: k.rozmiar(),
        plikow: kr::pliki_kroniki(&sciezka).len(),
        ustawienia: k.ustawienia().clone(),
        liczniki: k.liczniki.clone(),
        ostatnie: k.ostatnie(60),
        zrodlo_zywe: st.zywe.load(std::sync::atomic::Ordering::Relaxed),
        zrodlo_opis: st.opis_zrodla.lock().clone(),
    })
}

async fn statystyki(State(st): State<Uchwyt>) -> Json<kr::Statystyki> {
    // Odczyt pliku poza wątkiem wykonawczym: kronika tygodniowa to kilkanaście
    // megabajtów, a `axum` nie ma prawa na tym stanąć.
    let sciezka = st.kronika.lock().sciezka().to_path_buf();
    let s = tokio::task::spawn_blocking(move || kr::statystyki(&sciezka))
        .await
        .unwrap_or_default();
    Json(s)
}

async fn kanaly(State(st): State<Uchwyt>) -> Json<serde_json::Value> {
    let sciezka = st.kronika.lock().sciezka().to_path_buf();
    let zrodla = st.kronika.lock().ustawienia().zrodla.clone();
    let wpisy = tokio::task::spawn_blocking(move || kr::czytaj(&sciezka).wpisy)
        .await
        .unwrap_or_default();
    let ile = kr::api::wpisow_wg_zrodla(&wpisy);

    let mut lista = st.kanaly.lock().clone();
    // Kanał, z którego COŚ już zebraliśmy, a którego nie ma na liście dialogów
    // (opuszczony, archiwalny, prywatny), musi być widoczny — inaczej nie da
    // się go odznaczyć ani zrozumieć, skąd biorą się wiersze w pliku.
    let znane: std::collections::HashSet<i64> = lista.iter().map(|k| k.chat_id).collect();
    for ((chat_id, temat), n) in &ile {
        if *temat.as_ref().unwrap_or(&0) == 0 && !znane.contains(chat_id) {
            let nazwa = wpisy
                .iter()
                .find(|w| w.chat_id == *chat_id && !w.chat.is_empty())
                .map(|w| w.chat.clone())
                .unwrap_or_else(|| chat_id.to_string());
            lista.push(kr::KanalInfo {
                chat_id: *chat_id,
                nazwa,
                handle: None,
                forum: false,
                tematy: Vec::new(),
                nasluchiwany: false,
                nagrywany: false,
                wpisow: *n,
            });
        }
    }
    for k in lista.iter_mut() {
        k.nagrywany = zrodla.pasuje(k.chat_id, None);
        k.wpisow = ile.get(&(k.chat_id, None)).copied().unwrap_or(0);
        for t in k.tematy.iter_mut() {
            t.nagrywany = zrodla.pasuje(k.chat_id, Some(t.id));
            t.wpisow = ile.get(&(k.chat_id, Some(t.id))).copied().unwrap_or(0);
            k.wpisow += t.wpisow;
        }
    }
    lista.sort_by(|a, b| b.wpisow.cmp(&a.wpisow).then_with(|| a.nazwa.cmp(&b.nazwa)));
    Json(serde_json::json!({ "ok": true, "kanaly": lista, "blad": serde_json::Value::Null }))
}

async fn ustaw(State(st): State<Uchwyt>, Json(nowe): Json<kr::Ustawienia>) -> Response {
    if let Err(e) = nowe.sprawdz() {
        return blad(StatusCode::BAD_REQUEST, e);
    }
    let teraz = kr::teraz_ms();
    if let Err(e) = st.kronika.lock().przestaw(nowe.clone(), teraz) {
        return blad(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"));
    }
    if let Err(e) = zapisz_ustawienia(&st.konfig, &nowe) {
        return blad(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("zapis kronika.json: {e:#}"),
        );
    }
    Json(serde_json::json!({ "ok": true, "ustawienia": nowe })).into_response()
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ZadanieEksportu {
    /// nazwa pliku wynikowego; domyślnie `signals_kronika.json` obok kroniki
    plik: Option<String>,
    /// eksportuj wyłącznie nagrywane źródła (domyślnie: wszystko z pliku)
    tylko_zaznaczone: bool,
}

async fn eksport(State(st): State<Uchwyt>, Json(z): Json<ZadanieEksportu>) -> Response {
    let sciezka = st.kronika.lock().sciezka().to_path_buf();
    let zrodla = st.kronika.lock().ustawienia().zrodla.clone();
    let nazwa = z.plik.unwrap_or_else(|| "signals_kronika.json".into());
    if nazwa.contains("..") || nazwa.contains('/') || nazwa.contains('\\') {
        return blad(
            StatusCode::BAD_REQUEST,
            "nazwa pliku nie może zawierać ścieżki",
        );
    }
    let cel = st.katalog.join(nazwa);
    let filtr = if z.tylko_zaznaczone {
        Some(zrodla)
    } else {
        None
    };
    match tokio::task::spawn_blocking(move || kr::eksportuj(&sciezka, &cel, filtr)).await {
        Ok(Ok(p)) => Json(serde_json::json!({ "ok": true, "wynik": p })).into_response(),
        Ok(Err(e)) => blad(StatusCode::CONFLICT, format!("{e:#}")),
        Err(e) => blad(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("eksport padł: {e}"),
        ),
    }
}

async fn pobierz(State(st): State<Uchwyt>) -> Response {
    let sciezka = st.kronika.lock().sciezka().to_path_buf();
    match std::fs::read(&sciezka) {
        Ok(d) => {
            let nazwa = sciezka
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("kronika.jsonl")
                .to_string();
            (
                [
                    (header::CONTENT_TYPE, "application/x-ndjson".to_string()),
                    (
                        header::CONTENT_DISPOSITION,
                        format!("attachment; filename=\"{nazwa}\""),
                    ),
                ],
                d,
            )
                .into_response()
        }
        Err(_) => blad(StatusCode::NOT_FOUND, "pliku kroniki jeszcze nie ma"),
    }
}

// ============================================================
//  STATYKI
// ============================================================

/// Blokuje wyjście poza zasoby (`..`, ścieżki bezwzględne, dyski).
fn bezpieczna(rel: &str) -> Option<String> {
    let mut czesci = Vec::new();
    for part in rel.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') || part.contains('\\') {
            return None;
        }
        czesci.push(part);
    }
    Some(czesci.join("/"))
}

async fn statyki(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let rel = match bezpieczna(if path.is_empty() { "index.html" } else { path }) {
        Some(r) if !r.is_empty() => r,
        _ => "index.html".to_string(),
    };
    if let Some(f) = Zasoby::get(&rel) {
        return z_naglowkami(f.data.to_vec(), &rel);
    }
    // trasowanie po stronie klienta: ścieżka bez rozszerzenia dostaje
    // `index.html`, ale brakujący `main.js` ma zwrócić 404, a nie stronę HTML
    let wyglada_na_plik = rel
        .rsplit('/')
        .next()
        .map(|s| s.contains('.'))
        .unwrap_or(false);
    if !wyglada_na_plik {
        if let Some(f) = Zasoby::get("index.html") {
            return z_naglowkami(f.data.to_vec(), "index.html");
        }
    }
    (StatusCode::NOT_FOUND, "nie znaleziono").into_response()
}

fn z_naglowkami(dane: Vec<u8>, rel: &str) -> Response {
    let mime = mime_guess::from_path(rel).first_or_octet_stream();
    let cache = if rel.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mut res = Response::new(axum::body::Body::from(dane));
    let h = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(mime.essence_str()) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    res
}

fn interfejs_wbudowany() -> bool {
    Zasoby::get("index.html")
        .map(|f| !f.data.starts_with(b"<!-- ZASLEPKA"))
        .unwrap_or(false)
}

fn router(st: Uchwyt) -> Router {
    let api = Router::new()
        .route("/health", get(zdrowie))
        .route("/kronika/stan", get(stan))
        .route("/kronika/statystyki", get(statystyki))
        .route("/kronika/kanaly", get(kanaly))
        .route("/kronika/ustawienia", put(ustaw))
        .route("/kronika/eksport", post(eksport))
        .route("/kronika/plik", get(pobierz));
    Router::new()
        .nest("/api", api)
        .fallback(statyki)
        .with_state(st)
}

// ============================================================
//  START
// ============================================================

fn czas_txt(ms: i64) -> String {
    kr::czas_iso(ms, 0)
}

/// Podnosi mikroserwer i (opcjonalnie) otwiera przeglądarkę. Zwraca adres
/// albo pusty napis, gdy interfejs jest wyłączony lub niewbudowany.
///
/// Port 0 = system przydziela wolny. To jest CAŁA odpowiedź na pytanie
/// o kolizję z `conduit.exe` (8787) i z drugą kopią tego narzędzia.
async fn uruchom_interfejs(args: &Argumenty, st: Uchwyt) -> Result<String> {
    if args.bez_interfejsu {
        return Ok(String::new());
    }
    if !interfejs_wbudowany() {
        eprintln!(
            "⚠ binarka nie zawiera interfejsu — zbuduj go: `npm run build:kronika`, \
             potem `cargo build --release -p conduit-kronika`. Nagrywam dalej, bez strony."
        );
        return Ok(String::new());
    }
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], args.port)))
        .await
        .with_context(|| format!("nie mogę zająć portu {}", args.port))?;
    let port = listener.local_addr()?.port();
    let adres = format!("http://127.0.0.1:{port}/");
    let app = router(st);
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("serwer interfejsu padł: {e}");
        }
    });
    if !args.bez_przegladarki {
        if let Err(e) = open::that_detached(adres.as_str()) {
            println!("Nie udało się otworzyć przeglądarki ({e}). Wklej adres: {adres}");
        }
    }
    Ok(adres)
}

fn wypisz_naglowek(
    args: &Argumenty,
    sciezka: &Path,
    konfig: &Path,
    ust: &kr::Ustawienia,
    adres: &str,
) {
    println!("┌──────────────────────────────────────────────");
    println!("│ KRONIKA v{WERSJA} — rejestrator Telegrama");
    println!("│ plik zapisu : {}", sciezka.display());
    println!("│ konfiguracja: {}", konfig.display());
    println!("│ sesja       : {}", args.sesja.display());
    println!(
        "│ nagrywam    : {}",
        match ust.zrodla.ile() {
            None => "wszystkie kanały".to_string(),
            Some(n) => format!("{n} zaznaczonych źródeł"),
        }
    );
    if adres.is_empty() {
        println!("│ interfejs   : wyłączony (--bez-interfejsu)");
    } else {
        println!("│ interfejs   : {adres}");
    }
    println!("│ zatrzymanie : Ctrl+C — plik jest dopisywany, restart niczego nie kasuje");
    println!("└──────────────────────────────────────────────");
}

async fn polacz(args: &Argumenty, st: &Uchwyt) -> Option<conduit_telegram::TelegramClient> {
    let mut powiedz = |t: String| {
        eprintln!("⚠ {t}");
        *st.opis_zrodla.lock() = t;
    };

    let surowe = match std::fs::read_to_string(&args.sekrety) {
        Ok(s) => s,
        Err(e) => {
            powiedz(format!(
                "Nie mogę wczytać {} ({e}). Bez api_id i api_hash z my.telegram.org \
                 nie ma jak się połączyć. Statystyka i eksport z już zebranego pliku działają.",
                args.sekrety.display()
            ));
            return None;
        }
    };
    let s: Sekrety = match serde_json::from_str(&surowe) {
        Ok(s) => s,
        Err(e) => {
            powiedz(format!("{} jest uszkodzony: {e}", args.sekrety.display()));
            return None;
        }
    };

    let cfg = conduit_telegram::ClientConfig {
        api_id: s.api_id,
        api_hash: s.api_hash,
        session_path: args.sesja.clone(),
        catch_up: true,
        ignore_outgoing: false,
        queue_limit: 10_000,
    };

    let klient = match conduit_telegram::TelegramClient::connect(cfg).await {
        Ok(k) => k,
        Err(e) => {
            powiedz(format!("Nie mogę połączyć się z Telegramem: {e:#}"));
            return None;
        }
    };
    if !klient.is_authorized().await.unwrap_or(false) {
        powiedz(format!(
            "Kronika nie jest zalogowana. Zaloguj ją OSOBNO, na własny plik sesji ({}) — \
             NIE kopiuj tu sesji bota: ten sam plik w dwóch procesach to AUTH_KEY_DUPLICATED \
             i Telegram wyłącza obie strony.",
            args.sesja.display()
        ));
        return None;
    }

    // Bez filtra źródeł NA POZIOMIE KLIENTA — rejestrator nasłuchuje wszystkiego,
    // a selekcję robi zapis. To nie jest to samo: filtr klienta ucina strumień
    // na wejściu, więc lista kanałów w interfejsie przestałaby się zapełniać
    // i nie dałoby się zaznaczyć kanału, którego jeszcze nie nagrywamy.
    klient.set_sources(None);
    Some(klient)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = match czytaj_argumenty()? {
        Zamiar::Nic => return Ok(()),
        Zamiar::Eksportuj { zrodlo, cel } => {
            let p = kr::eksportuj(&zrodlo, &cel, None)?;
            println!("zapisano {}", p.plik);
            println!("  plików źródłowych : {}", p.plikow_zrodlowych);
            println!("  wpisów odczytanych: {}", p.wpisow);
            if p.uszkodzonych > 0 {
                println!("  linii uszkodzonych: {}", p.uszkodzonych);
            }
            println!("  sygnałów wejścia  : {}", p.sygnalow);
            println!("  zdarzeń           : {}", p.zdarzen);
            println!(
                "  w tym z EDYCJI    : {}   <- tego eksport z aplikacji NIE MA",
                p.z_edycji
            );
            return Ok(());
        }
        Zamiar::Nagrywaj(a) => a,
    };

    if args.sesja.file_name().and_then(|s| s.to_str()) == Some(conduit_telegram::PLIK_SESJI) {
        anyhow::bail!(
            "sesja wskazuje na {} — to plik BOTA. Ten sam plik sesji w dwóch \
             procesach zabija Telegram po obu stronach (AUTH_KEY_DUPLICATED). \
             Zaloguj Kronikę osobno, na własny plik.",
            conduit_telegram::PLIK_SESJI
        );
    }

    let katalog = args.katalog.clone();
    std::fs::create_dir_all(&katalog)
        .with_context(|| format!("nie mogę utworzyć {}", katalog.display()))?;
    let konfig = katalog.join("kronika.json");
    let mut ust = wczytaj_ustawienia(&konfig);
    if let Some(p) = &args.plik {
        ust.plik = p.clone();
    }
    ust.sprawdz()
        .map_err(|e| anyhow::anyhow!("kronika.json: {e}"))?;

    let teraz = kr::teraz_ms();
    let kronika = kr::Kronika::otworz(ust.clone(), &katalog, teraz)?;
    let sciezka = kronika.sciezka().to_path_buf();
    let st: Uchwyt = Arc::new(Stan {
        kronika: Mutex::new(kronika),
        katalog: katalog.clone(),
        konfig: konfig.clone(),
        kanaly: Mutex::new(Vec::new()),
        zywe: std::sync::atomic::AtomicBool::new(false),
        opis_zrodla: Mutex::new("łączę się z Telegramem…".into()),
    });
    // Zapisujemy ustawienia od razu, żeby plik `kronika.json` istniał i dało się
    // go otworzyć w edytorze — pusta konfiguracja domyślna jest niewidoczna,
    // a niewidoczna konfiguracja wygląda jak brak konfiguracji.
    let _ = zapisz_ustawienia(&konfig, &ust);

    // ---------- interfejs PRZED Telegramem ----------
    //
    // Kolejność jest tu decyzją, nie przypadkiem. Wcześniej program najpierw
    // logował się do Telegrama i przy `bail!` kończył pracę — czyli przy braku
    // sesji użytkownik nie dostawał NICZEGO: ani interfejsu, ani powodu, ani
    // dostępu do już zebranych danych. A kronika bez sesji dalej ma sens:
    // pokazuje statystykę edycji z pliku, pozwala go pobrać i wyeksportować.
    //
    // Teraz strona wstaje pierwsza, a stan połączenia jest jej ZAWARTOŚCIĄ
    // (`zrodlo_zywe` + `zrodlo_opis`), a nie warunkiem uruchomienia.
    let adres = uruchom_interfejs(&args, Arc::clone(&st)).await?;
    wypisz_naglowek(&args, &sciezka, &konfig, &ust, &adres);

    let mut klient = match polacz(&args, &st).await {
        Some(k) => k,
        // Brak sesji nie kończy programu: serwer stoi, plik jest dostępny,
        // a interfejs mówi wprost, czego brakuje.
        None => {
            println!("⚠ Nie nagrywam — brak połączenia z Telegramem. Interfejs działa dalej.");
            println!("   Powód widać w oknie przeglądarki. Ctrl+C kończy.");
            tokio::signal::ctrl_c().await.ok();
            st.kronika
                .lock()
                .zakoncz(kr::teraz_ms(), "Ctrl+C (bez sesji)");
            return Ok(());
        }
    };

    match klient.refresh_dialogs(0).await {
        Ok(lista) => {
            let mut out = Vec::with_capacity(lista.len());
            for d in &lista {
                let tematy = if d.is_forum {
                    klient
                        .topics_of(d)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .map(|t| kr::TematInfo {
                            id: t.topic_id,
                            nazwa: t.title,
                            nagrywany: false,
                            wpisow: 0,
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                out.push(kr::KanalInfo {
                    chat_id: d.chat_id,
                    nazwa: d.name.clone(),
                    handle: d.username.clone(),
                    forum: d.is_forum,
                    tematy,
                    nasluchiwany: false,
                    nagrywany: false,
                    wpisow: 0,
                });
            }
            println!("Pobrano {} rozmów z konta.", out.len());
            *st.kanaly.lock() = out;
        }
        Err(e) => eprintln!("nie udało się pobrać listy rozmów: {e}"),
    }
    st.zywe.store(true, std::sync::atomic::Ordering::Relaxed);
    *st.opis_zrodla.lock() = format!("własna sesja {}", args.sesja.display());

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                let n = st.kronika.lock().liczniki.zapisanych;
                st.kronika.lock().zakoncz(kr::teraz_ms(), "Ctrl+C");
                println!("\nzatrzymano. zapisanych zdarzeń: {n}");
                return Ok(());
            }
            wiad = klient.next_message() => {
                let m = match wiad {
                    Ok(m) => m,
                    Err(e) => {
                        // Błąd sieci nie może zabić rejestratora — ma chodzić
                        // tygodniami. Zapisujemy, czekamy, próbujemy dalej.
                        eprintln!("[{}] błąd odbioru: {e}", czas_txt(kr::teraz_ms()));
                        *st.opis_zrodla.lock() = format!("błąd odbioru: {e}");
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        continue;
                    }
                };
                let ms = kr::teraz_ms();
                let rozpoznane = conduit_core::parser::parse(&m.text)
                    .iter()
                    .any(|s| !matches!(s, conduit_core::parser::Signal::Info));
                let p = kr::Przychodzace {
                    odebrano_ms: ms,
                    rodzaj: if m.edit_of.is_some() { kr::Rodzaj::Edycja } else { kr::Rodzaj::Nowa },
                    chat_id: m.source.chat_id,
                    chat: &m.source_name,
                    temat: m.source.topic_id,
                    msg_id: m.msg_id,
                    reply_to: m.reply_to,
                    edit_of: m.edit_of,
                    ts_telegram_ms: m.ts,
                    text: &m.text,
                    // samodzielna kronika nie zna konfiguracji bota i nie ma
                    // prawa jej zgadywać — pole zostaje puste, zamiast kłamać
                    nasluchiwany: false,
                    format: None,
                };
                let wynik = st.kronika.lock().zapisz(p, rozpoznane);
                match wynik {
                    Ok(kr::Decyzja::Zapisano) => {
                        let n = st.kronika.lock().liczniki.zapisanych;
                        if n % 50 == 0 {
                            println!("[{}] zapisanych: {n}", czas_txt(ms));
                        }
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("[{}] BŁĄD ZAPISU: {e:#}", czas_txt(ms)),
                }
            }
        }
    }
}

#[cfg(test)]
mod testy {
    use super::*;

    #[test]
    fn nie_da_sie_wyjsc_poza_zasoby() {
        assert!(bezpieczna("../../secret.txt").is_none());
        assert!(bezpieczna("..\\secret.txt").is_none());
        assert!(bezpieczna("C:/windows/win.ini").is_none());
        assert_eq!(
            bezpieczna("assets/index.js").as_deref(),
            Some("assets/index.js")
        );
    }

    #[test]
    fn ustawienia_przezywaja_zapis_i_odczyt() {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "kronika-cfg-{}-{}.json",
            std::process::id(),
            kr::teraz_ms()
        ));
        let mut u = kr::Ustawienia::default();
        u.zrodla = kr::Zrodla::Wybrane {
            lista: vec![kr::Zrodlo {
                chat_id: -100,
                temat: Some(7),
            }],
        };
        u.fsync = kr::Fsync::Co { n: 20 };
        zapisz_ustawienia(&p, &u).unwrap();
        assert_eq!(wczytaj_ustawienia(&p), u);
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn brak_pliku_konfiguracji_daje_bezpieczne_domyslne() {
        let u = wczytaj_ustawienia(Path::new("C:/nie/ma/takiego/kronika.json"));
        assert!(u.nierozpoznane, "domyślnie zapisujemy wszystko");
        assert_eq!(u.fsync, kr::Fsync::Kazda);
    }
}
