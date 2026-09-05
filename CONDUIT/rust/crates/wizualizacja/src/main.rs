//! # `wizualizacja.exe` — samodzielna przeglądarka modeli AI
//!
//! Pokazuje architekturę wytrenowanych sieci (diagram wag), ważność cech
//! wejściowych i symulator decyzji — bez uruchamiania `conduit.exe`, bez MT5,
//! bez Telegrama i bez ani jednego pliku konfiguracyjnego.
//!
//! ## Dlaczego mikroserwer, a nie jeden plik HTML
//!
//! Rozważaliśmy wariant „exe generuje samowystarczalny `.html` z wklejonymi
//! modelami i otwiera go z dysku". Odpadł z trzech powodów:
//!
//! 1. **Interfejs już umie rozmawiać z API.** `AiModelsView` pobiera listę
//!    przez `GET /api/models`, a wagi przez `GET /api/models/{id}` — dokładnie
//!    ten kontrakt, który wystawia `conduit.exe`. Serwer oznacza ZERO zmian
//!    w widoku; wariant z plikiem wymagałby podmiany `fetch` atrapą, czyli
//!    drugiej ścieżki kodu, która cicho rozjeżdża się z pierwszą.
//! 2. **Wagi ładują się leniwie.** Katalog `models/` to kilka megabajtów JSON-a;
//!    serwer wydaje wagi jednego modelu na żądanie, plik HTML musiałby wkleić
//!    wszystkie naraz — kilkanaście megabajtów, które przeglądarka parsuje
//!    przed pierwszym malowaniem.
//! 3. **Odświeżenie po treningu.** Przycisk „Odśwież" ma pokazać model zapisany
//!    minutę temu. Serwer czyta katalog przy każdym żądaniu; plik HTML byłby
//!    migawką nieaktualną w chwili zapisu.
//!
//! Koszt serwera jest znikomy: nasłuch na `127.0.0.1` (bez pytania zapory
//! o dostęp do sieci) na **porcie przydzielonym przez system**, więc
//! `conduit.exe` na 8787 i dowolna liczba kopii tego narzędzia mogą chodzić
//! równocześnie.
//!
//! ## Cykl życia
//!
//! Program jest konsolowy i działa, dopóki żyje jego okno konsoli: zamknięcie
//! okna albo `Ctrl+C` kończy pracę. Wybrane świadomie zamiast ikony w zasobniku —
//! ikona wymaga pętli komunikatów okna i menu kontekstowego (kolejne ~200 linii
//! i zależność od WebView2/Tauri), a narzędzie do jednorazowego zerknięcia w
//! model ma się dać zamknąć krzyżykiem, nie szukaniem ikonki przy zegarze.

use anyhow::{Context, Result};
use axum::extract::{Path as SciezkaUrl, State};
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rust_embed::RustEmbed;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Zbudowany interfejs (`npm run build:wiz` wsypuje tu `index.html` + `assets/`).
#[derive(RustEmbed)]
#[folder = "web/"]
struct Zasoby;

const WERSJA: &str = env!("CARGO_PKG_VERSION");

// ============================================================
//  ARGUMENTY
// ============================================================

struct Argumenty {
    modele: Option<PathBuf>,
    port: u16,
    bez_przegladarki: bool,
}

const POMOC: &str = "\
wizualizacja.exe — podgląd sieci neuronowych modeli AI (CONDUIT)

  --modele <katalog>   katalog z plikami *.json modeli
                       (domyślnie: `models` obok programu)
  --port <numer>       wymuś port; domyślnie system przydziela wolny,
                       dzięki czemu nie ma kolizji z conduit.exe
  --bez-przegladarki   nie otwieraj przeglądarki, tylko wypisz adres
  --pomoc, -h          ten opis

Program działa, dopóki otwarte jest okno konsoli (Ctrl+C kończy).
";

fn czytaj_argumenty() -> Result<Option<Argumenty>> {
    let mut a = Argumenty {
        modele: None,
        port: 0,
        bez_przegladarki: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--pomoc" | "-h" | "--help" | "/?" => {
                println!("{POMOC}");
                return Ok(None);
            }
            "--modele" | "--models" => {
                let v = it.next().context("--modele wymaga ścieżki do katalogu")?;
                a.modele = Some(PathBuf::from(v));
            }
            "--port" => {
                let v = it.next().context("--port wymaga numeru")?;
                a.port = v.parse().context("--port musi być liczbą 0-65535")?;
            }
            "--bez-przegladarki" | "--no-browser" => a.bez_przegladarki = true,
            inne => anyhow::bail!("nieznany argument „{inne}” — użyj --pomoc"),
        }
    }
    Ok(Some(a))
}

// ============================================================
//  KATALOG MODELI
// ============================================================

/// Wynik szukania katalogu modeli.
///
/// `sprawdzone` niesie WSZYSTKIE rozważane ścieżki, bo przy pustym wyniku
/// jedyne sensowne pytanie brzmi „to gdzie ty właściwie patrzyłeś?" —
/// i interfejs musi umieć na nie odpowiedzieć bez zaglądania w kod.
struct Szukanie {
    wybrany: PathBuf,
    znaleziony: bool,
    sprawdzone: Vec<PathBuf>,
}

/// Gdzie szukać modeli, po kolei:
///  1. `--modele` (jeśli podany — i wtedy TYLKO tam, bez cichego zjeżdżania
///     gdzie indziej: użytkownik wskazał katalog i ma zobaczyć jego zawartość
///     albo komunikat o pustce, a nie przypadkowe modele z innego miejsca),
///  2. `models/` obok programu — pakiet wysyłkowy i `LAB/models/`,
///  3. `../rust/models` względem programu — układ `LAB/` obok `rust/`,
///     czyli dwuklik z folderu narzędzi widzi ŚWIEŻE modele z repozytorium,
///  4. `../../models` względem programu — układ `rust/target/release/`,
///     żeby `cargo run` działał bez argumentów,
///  5. `models/` w katalogu bieżącym — uruchomienie z wiersza poleceń.
///
/// Nie kopiujemy modeli do folderu narzędzi celowo: kopia starzeje się po
/// pierwszym treningu, a wtedy narzędzie pokazuje nieaktualną sieć i nikt
/// tego nie zauważa. Lepiej czytać oryginał i głośno mówić, którego nie ma.
fn znajdz_katalog(wskazany: Option<PathBuf>, katalog_exe: &Path) -> Szukanie {
    if let Some(p) = wskazany {
        let p = bezwzgledna(p);
        return Szukanie {
            znaleziony: p.is_dir(),
            sprawdzone: vec![p.clone()],
            wybrany: p,
        };
    }

    let mut kandydaci = vec![katalog_exe.join("models")];
    if let Some(nad) = katalog_exe.parent() {
        kandydaci.push(nad.join("rust").join("models"));
        if let Some(nad2) = nad.parent() {
            kandydaci.push(nad2.join("models"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let c = cwd.join("models");
        if !kandydaci.contains(&c) {
            kandydaci.push(c);
        }
    }

    for k in &kandydaci {
        if k.is_dir() {
            return Szukanie {
                wybrany: k.clone(),
                znaleziony: true,
                sprawdzone: kandydaci.clone(),
            };
        }
    }
    // nic nie istnieje — interfejs powie o tym wprost i wymieni ścieżki
    Szukanie {
        wybrany: kandydaci[0].clone(),
        znaleziony: false,
        sprawdzone: kandydaci,
    }
}

fn bezwzgledna(p: PathBuf) -> PathBuf {
    if p.is_absolute() {
        return p;
    }
    std::env::current_dir().map(|c| c.join(&p)).unwrap_or(p)
}

// ============================================================
//  ODCZYT MODELI
// ============================================================

/// Czy dokument jest MODELEM, czy migawką treningu.
///
/// W `models/` obok modeli leżą pliki `*.checkpoint.json` — stan optymalizatora
/// (`theta`, `sigma`, momenty Adama). Nie mają pola `policy`, więc widok nie ma
/// czego narysować. Pokazywanie ich na liście kończy się kliknięciem w pozycję,
/// która nie umie się otworzyć; dlatego filtrujemy je TUTAJ i mówimy w
/// `/api/wiz/info`, ile ich pominięto — żeby brakująca pozycja nie wyglądała
/// jak zgubiony plik.
fn to_model(doc: &serde_json::Value) -> bool {
    doc.pointer("/policy/pos/dims")
        .and_then(|d| d.as_array())
        .map(|d| d.len() >= 2)
        .unwrap_or(false)
}

/// Wczytuje `(identyfikator, dokument)` dla wszystkich modeli w katalogu.
/// Identyfikator = nazwa pliku bez `.json` — tak jak w `conduit.exe`.
fn wczytaj_modele(katalog: &Path) -> (Vec<(String, serde_json::Value)>, usize) {
    let mut out = Vec::new();
    let mut pominiete = 0usize;
    let rd = match std::fs::read_dir(katalog) {
        Ok(r) => r,
        Err(_) => return (out, 0),
    };
    let mut pliki: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
        .collect();
    pliki.sort();
    for f in pliki {
        let id = f
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let raw = match std::fs::read_to_string(&f) {
            Ok(r) => r,
            Err(_) => {
                pominiete += 1;
                continue;
            }
        };
        match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(v) if to_model(&v) => out.push((id, v)),
            _ => pominiete += 1,
        }
    }
    (out, pominiete)
}

/// Zamienia sieć `{dims, w, b}` na `{dims, params}` — kształt zostaje,
/// kilka tysięcy liczb znika. Odpowiednik `opis_sieci` z `crates/server`.
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

/// Wpis listy: wszystko co pokazuje karta modelu, BEZ wag.
///
/// Odpowiednik `podsumowanie_modelu` z `crates/server/src/rest.rs` — ten sam
/// kształt odpowiedzi, bo konsumuje go ten sam kod TypeScriptu (`AiModelSummary`).
fn podsumowanie(id: &str, mut doc: serde_json::Value, bajty: u64) -> serde_json::Value {
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
//  STAN I TRASY
// ============================================================

struct Stan {
    katalog: PathBuf,
    /// czy `katalog` w ogóle istnieje
    istnieje: bool,
    /// wszystkie rozważone ścieżki — do pokazania, gdy nie ma czego pokazać
    sprawdzone: Vec<PathBuf>,
}

type Uchwyt = Arc<Stan>;

fn blad(kod: StatusCode, tresc: impl Into<String>) -> Response {
    (kod, Json(serde_json::json!({ "error": tresc.into() }))).into_response()
}

/// Skąd program czyta i ile znalazł — pasek górny pokazuje to bez klikania.
async fn info(State(st): State<Uchwyt>) -> Json<serde_json::Value> {
    let (modele, pominiete) = wczytaj_modele(&st.katalog);
    Json(serde_json::json!({
        "katalog": st.katalog.display().to_string(),
        "istnieje": st.istnieje,
        "szukano": st.sprawdzone.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "liczba": modele.len(),
        "pominiete": pominiete,
        "wersja": WERSJA,
    }))
}

async fn zdrowie() -> Json<serde_json::Value> {
    Json(
        serde_json::json!({ "ok": true, "app": "conduit", "tool": "wizualizacja", "version": WERSJA }),
    )
}

/// `GET /api/models` — lista bez wag.
async fn lista_modeli(State(st): State<Uchwyt>) -> Json<Vec<serde_json::Value>> {
    let (modele, _) = wczytaj_modele(&st.katalog);
    let mut out = Vec::with_capacity(modele.len());
    for (id, doc) in modele {
        let bajty = std::fs::metadata(st.katalog.join(format!("{id}.json")))
            .map(|m| m.len())
            .unwrap_or(0);
        out.push(podsumowanie(&id, doc, bajty));
    }
    Json(out)
}

/// `GET /api/models/{id}` — pełny dokument z wagami.
async fn jeden_model(State(st): State<Uchwyt>, SciezkaUrl(id): SciezkaUrl<String>) -> Response {
    // jedyna bariera między adresem z sieci a systemem plików
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || id.contains(':')
    {
        return blad(StatusCode::BAD_REQUEST, "niedozwolony identyfikator");
    }
    let sciezka = st.katalog.join(format!("{id}.json"));
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
    let rel = if path.is_empty() {
        "index.html".to_string()
    } else {
        path.to_string()
    };
    let rel = match bezpieczna(&rel) {
        Some(r) if !r.is_empty() => r,
        _ => "index.html".to_string(),
    };

    if let Some(f) = Zasoby::get(&rel) {
        return z_naglowkami(f.data.to_vec(), &rel);
    }
    // trasowanie po stronie klienta: ścieżka bez rozszerzenia dostaje `index.html`,
    // ale brakujący `main.js` ma zwrócić 404, a nie stronę HTML
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

/// Czy w binarce siedzi zbudowany interfejs, czy tylko zaślepka?
fn interfejs_wbudowany() -> bool {
    Zasoby::get("index.html")
        .map(|f| !f.data.starts_with(b"<!-- ZASLEPKA"))
        .unwrap_or(false)
}

// ============================================================
//  START
// ============================================================

fn main() -> Result<()> {
    let args = match czytaj_argumenty()? {
        Some(a) => a,
        None => return Ok(()), // --pomoc
    };

    let exe = std::env::current_exe().context("nie znam własnej ścieżki")?;
    let katalog_exe = exe.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
    let szukanie = znajdz_katalog(args.modele, &katalog_exe);
    let katalog = szukanie.wybrany.clone();

    if !interfejs_wbudowany() {
        anyhow::bail!(
            "binarka nie zawiera interfejsu — zbuduj go najpierw:\n  npm run build:wiz\n  cargo build --release -p conduit-wizualizacja"
        );
    }

    // Dwa wątki wystarczą: to jest serwer plików dla JEDNEJ karty przeglądarki.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("nie udało się uruchomić środowiska asynchronicznego")?;

    rt.block_on(async move {
        let (modele, pominiete) = wczytaj_modele(&katalog);
        let stan: Uchwyt = Arc::new(Stan {
            katalog: katalog.clone(),
            istnieje: szukanie.znaleziony,
            sprawdzone: szukanie.sprawdzone.clone(),
        });

        let api = Router::new()
            .route("/health", get(zdrowie))
            .route("/wiz/info", get(info))
            .route("/models", get(lista_modeli))
            .route("/models/{id}", get(jeden_model));

        let app = Router::new()
            .nest("/api", api)
            .fallback(statyki)
            .with_state(stan);

        // Port 0 = system przydziela wolny. To jest CAŁA odpowiedź na pytanie
        // o kolizję z `conduit.exe` (8787) i z drugą kopią tego narzędzia.
        let adres = SocketAddr::from(([127, 0, 0, 1], args.port));
        let listener = tokio::net::TcpListener::bind(adres)
            .await
            .with_context(|| format!("nie mogę zająć {adres}"))?;
        let lokalny = listener.local_addr().context("nie znam własnego portu")?;
        let url = format!("http://127.0.0.1:{}/?view=aimodels", lokalny.port());

        println!("┌──────────────────────────────────────────────");
        println!("│ CONDUIT · wizualizacja modeli AI  v{WERSJA}");
        println!("│ katalog modeli : {}", katalog.display());
        if modele.is_empty() {
            println!("│ znaleziono     : 0 modeli — interfejs POWIE o tym wprost");
            println!("│ szukałem w     :");
            for k in &szukanie.sprawdzone {
                let stan = if k.is_dir() {
                    "jest, ale bez modeli"
                } else {
                    "nie ma takiego katalogu"
                };
                println!("│                  · {} — {stan}", k.display());
            }
            println!("│                  (wskaż katalog: --modele <ścieżka>)");
        } else {
            println!("│ znaleziono     : {} modeli", modele.len());
            for (id, doc) in &modele {
                let dims = doc
                    .pointer("/policy/pos/dims")
                    .and_then(|d| d.as_array())
                    .map(|d| {
                        d.iter()
                            .map(|x| x.to_string())
                            .collect::<Vec<_>>()
                            .join("→")
                    })
                    .unwrap_or_else(|| "?".into());
                println!("│                  · {id}  [{dims}]");
            }
        }
        if pominiete > 0 {
            println!("│ pominięto      : {pominiete} plików bez sieci (np. *.checkpoint.json)");
        }
        println!("│ adres          : {url}");
        println!("│ zamknięcie     : Ctrl+C albo zamknij to okno");
        println!("└──────────────────────────────────────────────");

        if !args.bez_przegladarki {
            if let Err(e) = open::that_detached(url.as_str()) {
                println!("Nie udało się otworzyć przeglądarki ({e}). Wklej adres ręcznie: {url}");
            }
        }

        let serwer = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                eprintln!("serwer padł: {e}");
            }
        });

        // Zamknięcie okna konsoli zabija proces; Ctrl+C kończy uprzejmie.
        tokio::select! {
            _ = tokio::signal::ctrl_c() => println!("Kończę."),
            _ = serwer => {}
        }
        Ok::<(), anyhow::Error>(())
    })
}

// ============================================================
//  TESTY
// ============================================================

#[cfg(test)]
mod testy {
    use super::*;

    #[test]
    fn checkpoint_nie_jest_modelem() {
        let model = serde_json::json!({ "policy": { "pos": { "dims": [60, 48, 11] } } });
        let checkpoint = serde_json::json!({ "gen": 12, "theta": [0.1, 0.2] });
        assert!(to_model(&model));
        assert!(!to_model(&checkpoint));
    }

    #[test]
    fn podsumowanie_zdejmuje_wagi_i_liczy_parametry() {
        let doc = serde_json::json!({
            "name": "test",
            "policy": {
                "pos": { "dims": [2, 3], "w": [[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]], "b": [[0.1, 0.2, 0.3]] }
            }
        });
        let s = podsumowanie("test", doc, 123);
        assert_eq!(s["params"], 9);
        assert_eq!(s["bytes"], 123);
        assert_eq!(s["id"], "test");
        assert!(
            s["policy"]["pos"].get("w").is_none(),
            "wagi nie mogą wyjść w liście"
        );
        assert_eq!(s["policy"]["pos"]["dims"][1], 3);
    }

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
    fn wskazany_katalog_wygrywa_i_nie_ma_zapasowych() {
        let s = znajdz_katalog(Some(PathBuf::from("C:/gdzies/modele")), Path::new("C:/app"));
        assert_eq!(s.wybrany, PathBuf::from("C:/gdzies/modele"));
        // wskazanie jawne = jedyny kandydat; żadnego cichego zjeżdżania gdzie indziej
        assert_eq!(s.sprawdzone.len(), 1);
    }

    #[test]
    fn kolejnosc_szukania_obejmuje_uklad_lab() {
        // `LAB/wizualizacja.exe` obok `rust/models` — dwuklik ma trafić w modele
        let s = znajdz_katalog(None, Path::new("C:/projekt/LAB"));
        assert_eq!(s.sprawdzone[0], PathBuf::from("C:/projekt/LAB/models"));
        assert_eq!(s.sprawdzone[1], PathBuf::from("C:/projekt/rust/models"));
        assert_eq!(s.sprawdzone[2], PathBuf::from("C:/models"));
        // nic z tego nie istnieje na maszynie testowej → wybór pada na pierwszy,
        // a `znaleziony` mówi prawdę
        assert!(!s.znaleziony);
        assert_eq!(s.wybrany, PathBuf::from("C:/projekt/LAB/models"));
    }
}
