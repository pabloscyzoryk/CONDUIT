//! CONDUIT — serwer lokalny.
//!
//! Jeden proces, jeden stan, dowolna liczba powłok interfejsu.
//!
//! ```text
//!   http://127.0.0.1:8787/          zbudowana aplikacja React (SPA)
//!   http://127.0.0.1:8787/api/…     REST — dokumenty (presety, historia, logi)
//!   ws://127.0.0.1:8787/ws          strumień stanu (snapshot → delta → event)
//! ```
//!
//! Serwer **nie jest właścicielem** stanu bota — jest jego subskrybentem,
//! tak samo jak okno natywne i przeglądarka. Stan żyje w [`state::Shared`],
//! a wypełnia go środowisko uruchomieniowe ([`state::Runtime`]): most do MT5
//! i klient Telegrama. Dopóki nic nie jest podłączone, serwer działa i mówi
//! wprost, że brokera nie ma — zamiast udawać handel.

pub mod alllogs;
pub mod archive;
pub mod auth;
pub mod history_import;
/// BRAMKA SPÓJNOŚCI USTAWIEŃ — konfiguracje wewnętrznie sprzeczne wykryte
/// ZANIM bot ruszy. Ostrzega, nigdy nie blokuje.
pub mod bramka_spojnosci;
pub mod coalesce;
pub mod commands;
pub mod demo;
pub mod export;
pub mod hub;
pub mod journal;
/// KRONIKA — rejestrator strumienia Telegrama do JEDNEGO ciągłego pliku.
/// Wspólny rdzeń zakładki „Kronika" i samodzielnej `kronika.exe`.
pub mod kronika;
/// Zakładka „Kronika": REST dla wbudowanego rejestratora. Sam zapis mieszka
/// w [`kronika`] i jest wspólny z samodzielną `kronika.exe`.
pub mod kronika_rest;
pub mod lab;
pub mod mailer;
/// Świece i parametry instrumentów z MT5 (zespół ŚWIECE).
pub mod market;
pub mod notify;
pub mod proto;
pub mod rest;
pub mod replay_capture;
pub mod secrets;
pub mod settings_map;
pub mod state;
pub mod store;
/// Tryb handlu PER INSTANCJA symulacji — rozstrzyganie pola `mode` rekordu
/// (dziedziczenie po bocie głównym) i flaga `tryb_auto_ea` na jej silnikach.
pub mod symulacje;
pub mod ui;
pub mod web;
/// Wstrzykiwanie wiadomości z panelu (F5): reguła numeracji i jedyne miejsce,
/// w którym polecenie panelu staje się wiadomością dla silnika.
pub mod wstrzykniecie;

use anyhow::{Context, Result};
use axum::routing::get;
use axum::Router;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub use state::{Runtime, StateHandle};
pub use store::Workspace;
pub use web::WebSource;

/// Czas ścienny w milisekundach epoki.
///
/// Uwaga: rdzeń (`conduit_core`) NIE MA prawa wołać zegara — determinizm
/// backtestu stoi na tym, że czas przychodzi w zdarzeniu. Serwer jest warstwą
/// wejścia/wyjścia, więc tutaj zegar jest na miejscu; to on stempluje zdarzenia
/// wchodzące do silnika.
pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    /// katalog z konfiguracją i `backup_memory/`
    pub workspace: PathBuf,
    /// jawnie wskazany katalog z interfejsem (nadpisuje zasoby wbudowane)
    pub web_dir: Option<PathBuf>,
    /// co ile zapisywać `backup_memory/`
    pub backup_every: Duration,
    /// saldo startowe, gdy nie ma czego wczytać z pamięci
    pub start_balance: f64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            bind: ([127, 0, 0, 1], 8787).into(),
            workspace: PathBuf::from("."),
            web_dir: None,
            backup_every: Duration::from_secs(15),
            start_balance: 0.0,
        }
    }
}

/// Uruchomiony serwer: uchwyt stanu + adres, na którym naprawdę słucha.
pub struct Running {
    pub state: StateHandle,
    pub addr: SocketAddr,
    handle: tokio::task::JoinHandle<()>,
}

impl Running {
    /// Adres do wpisania w przeglądarce i do załadowania w oknie natywnym.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
    pub async fn wait(self) {
        let _ = self.handle.await;
    }
    pub fn abort(&self) {
        self.handle.abort();
    }
}

/// Buduje stan z plików konfiguracyjnych i pamięci stanu.
pub fn bootstrap(cfg: &ServerConfig, auth: auth::SharedAuth) -> Result<StateHandle> {
    let ws = Workspace::new(&cfg.workspace);
    ws.ensure_dirs()
        .context("nie udało się przygotować katalogu roboczego")?;

    let now = now_ms();
    let mut snap = ui::UiSnapshot::empty(now);
    snap.stats = ui::Stats::new(cfg.start_balance, now);
    snap.balance = cfg.start_balance;

    let (s, mut blad_ustawien) = ws.load_settings_checked();
    snap.mode = s.mode;
    snap.preset_id = s.preset_id;
    snap.lot = s.lot;
    snap.favorites = s.favorites;
    // Nieznana wartość spada na "en" — panel nie może wstać w języku,
    // którego nie ma w słownikach (fallback i tak pokazałby EN, ale stan
    // ma mówić prawdę o tym, co się wyświetla).
    snap.language = match s.language.as_str() {
        "pl" => "pl".into(),
        _ => "en".into(),
    };
    snap.settings = s.settings;

    let ile = snap.settings.as_object().map(|o| o.len()).unwrap_or(0);
    if blad_ustawien.is_none() && ile > 0 && ile < 50 {
        blad_ustawien = Some(format!(
            "wczytano zaledwie {ile} ustawień (kompletna konfiguracja ma ponad 250) — \
             plik prawdopodobnie został utracony i silnik gra DOMYŚLNYMI"
        ));
    }
    if let Some(l) = settings_map::preset_lot(&snap.settings) {
        snap.lot = l;
    }

    let m = ws.load_smtp();
    snap.email = m.email;
    snap.notify = m.notify;
    // Hasło SMTP mieszka w `secrets.json`, nie w stanie. Gdyby stary
    // `smtp.json` je jeszcze niósł, przenosimy je teraz i czyścimy źródło —
    // inaczej hasło zostałoby w pliku, który użytkownik uznaje za niewrażliwy.
    if !snap.email.pass.is_empty() {
        let mut sek = ws.load_secrets();
        sek.smtp.password = std::mem::take(&mut snap.email.pass).into();
        if let Err(e) = ws.save_secrets(&sek) {
            tracing::warn!(blad = %e, "nie udało się przenieść hasła SMTP do secrets.json");
        } else {
            let doc = store::SmtpDoc {
                email: snap.email.redacted(),
                notify: snap.notify.clone(),
            };
            let _ = ws.save_smtp(&doc);
            tracing::info!("hasło SMTP przeniesione z smtp.json do secrets.json");
        }
    }

    // Konfiguracja trybu demo przeżywa restart, ale przebieg NIE wznawia się
    // sam: symulacja, która ruszyła bez świadomej decyzji, byłaby ostatnią
    // rzeczą, jakiej ktoś oczekuje po ponownym uruchomieniu bota.
    snap.demo = demo::DemoState::idle(ws.load_demo());

    let (kanaly, ostrzezenia_kanalow) = ws.load_channels_z_ostrzezeniami();
    snap.bindings = kanaly.bindings;
    // ŁAŃCUCHY: mapa `format → preset` plus pułapy ponad presetami. Brak pliku
    // = łańcuchy wbudowane, bo bot bez ani jednego łańcucha nie ma jak
    // zdecydować, czym handlować.
    snap.lancuchy = ws.load_lancuchy();
    // WSKAŹNIK ŁAŃCUCHA DLA TRYBU AUTO-EA (projekt EA-2). Brak pola w pliku
    // = pusty łańcuch znaków = AUTO-EA gra tym samym co reszta trybów.
    snap.aktywny_ea = ws.load_aktywny_ea();
    snap.formaty = conduit_core::formaty::formaty_wbudowane();
    // Deklaracja wysyłki. Czytana TU, sprawdzana NIŻEJ — dopiero po wznowieniu
    // stanu z pamięci, bo to ono ma ostatnie słowo w sprawie drabinki.
    let paczka = ws.load_paczka();

    // Wznowienie po restarcie. Pozycje z pamięci są tylko punktem odniesienia
    // do rekoncyliacji — prawdą po starcie jest zawsze stan u brokera.
    let wznowione = match ws.load_backup() {
        Some(m) => {
            m.apply_to(&mut snap);
            Some(m.saved_at)
        }
        None => None,
    };
    let wygasla_diagnoza = snap.halt.wyprowadz_od_nowa();

    // IZOLACJA DRABINEK (projekt EA-2c) — dokładnie jedna z nich jest
    // skuteczna: ta od trybu z `settings.json`. Wołane także przy BRAKU
    // pamięci, żeby świeża instalacja miała jawny wyłącznik od pierwszej
    // sekundy, a nie dopiero po pierwszym zapisie.
    ui::przelicz_izolacje_drabinek(&mut snap);

    let mut rozjazd: Vec<String> = Vec::new();
    snap.pieczec_lancuch = paczka
        .as_ref()
        .map(|p| p.aktywny_lancuch.clone())
        .unwrap_or_default();
    if let Some(p) = &paczka {
        let drabinka = snap.drabinka_biezaca().clone();
        let drabinka_rzadzi = drabinka.enabled && !drabinka.szczeble.is_empty();
        // Łańcuch, wobec którego sprawdzamy nogi: przy rządzącej drabince to
        // szczebel zgodny z paczką (jeśli jest), inaczej aktywny z dysku.
        let sprawdzany = if drabinka_rzadzi
            && drabinka
                .szczeble
                .iter()
                .any(|s| s.lancuch == p.aktywny_lancuch)
        {
            p.aktywny_lancuch.clone()
        } else {
            // ŁAŃCUCH WŁAŚCIWY DLA TRYBU (projekt EA-2): w AUTO-EA pieczęć ma
            // pytać o skład WARSTWY EA, bo to nim bot za chwilę zagra.
            snap.aktywny_lancuch_nazwa().to_string()
        };
        if !p.aktywny_lancuch.is_empty() && !drabinka_rzadzi && sprawdzany != p.aktywny_lancuch {
            rozjazd.push(format!(
                "aktywny łańcuch na dysku: {} — a paczka {} wiozła: {}",
                sprawdzany, p.nazwa, p.aktywny_lancuch
            ));
        }
        match snap.lancuchy.lista.iter().find(|l| l.nazwa == sprawdzany) {
            None if !p.aktywny_lancuch.is_empty() => rozjazd.push(format!(
                "łańcucha {sprawdzany} NIE MA w lancuchy.json — bot nie ma czym handlować"
            )),
            Some(l) => {
                for (format, preset) in &p.presety_nog {
                    match l.presety.get(format) {
                        Some(m) if m == preset => {}
                        Some(m) => rozjazd.push(format!(
                            "noga {format} gra presetem {m} — a paczka wiozła {preset}"
                        )),
                        None => rozjazd.push(format!(
                            "noga {format} w ogóle nie ma presetu — miał być {preset}"
                        )),
                    }
                }
            }
            None => {}
        }
        for preset in p.presety_nog.values().filter(|x| !x.is_empty()) {
            if !ws.presets_dir().join(format!("{preset}.json")).is_file() {
                rozjazd.push(format!("brakuje pliku presets/{preset}.json"));
            }
        }
        if !p.aktywny_lancuch.is_empty() && drabinka_rzadzi {
            let szczeble: Vec<&str> = drabinka
                .szczeble
                .iter()
                .map(|s| s.lancuch.as_str())
                .collect();
            if !szczeble.iter().any(|x| *x == p.aktywny_lancuch) {
                rozjazd.push(format!(
                    "drabinka jest WŁĄCZONA i prowadzi wyłącznie do {} — a paczka wiozła {}, \
                     którego nie ma na żadnym szczeblu",
                    szczeble.join(", "),
                    p.aktywny_lancuch
                ));
            }
        }
    }
    let rozjazd_tresc = (!rozjazd.is_empty()).then(|| {
        let rozmiar = |p: std::path::PathBuf| {
            std::fs::metadata(&p)
                .map(|m| format!("{} B", m.len()))
                .unwrap_or("BRAK PLIKU".into())
        };
        format!(
            "KONFIGURACJA OBOK BINARKI NIE JEST TĄ, KTÓRĄ PACZKA MIAŁA PRZYWIEŹĆ.\n\n{}\n\n\
             Katalog: {}\n  settings.json: {}\n  lancuchy.json: {}\n  channels.json: {}\n\n\
             Najczęstsza przyczyna: aktualizacja przez skopiowanie folderu, przy której \
             Windows POMINĄŁ pliki już istniejące — podmienił conduit.exe, a konfigurację \
             zostawił starą. Druga: stare backup_memory na serwerze, które przy starcie \
             nadpisuje drabinkę. HANDEL JEST ZATRZYMANY: bot z cudzym składem jest gorszy \
             niż bot stojący. Skopiuj CAŁĄ paczkę z nadpisaniem, skasuj backup_memory \
             i uruchom ponownie.",
            rozjazd
                .iter()
                .map(|x| format!("  · {x}"))
                .collect::<Vec<_>>()
                .join("\n"),
            ws.root.display(),
            rozmiar(ws.settings_path()),
            rozmiar(ws.lancuchy_path()),
            rozmiar(ws.channels_path()),
        )
    });
    if !rozjazd.is_empty() {
        tracing::error!(
            "PACZKA NIE ZGADZA SIĘ Z KONFIGURACJĄ: {}",
            rozjazd.join("; ")
        );
    }

    snap.stats.session_start_equity = 0.0;
    snap.stats.session_start = now_ms();
    snap.stats.pnl_session = 0.0;

    snap.auth = auth.state();
    if snap.auth.is_logged_in() {
        snap.connection.telegram = "connected".into();
    }

    let st = state::Shared::new(ws, auth, snap);
    st.set_mt5_startup_verified(ile >= 50 && blad_ustawien.is_none() && rozjazd_tresc.is_none());

    if !ostrzezenia_kanalow.is_empty() {
        st.log(
            "channels",
            "warn",
            format!(
                "Powiązania kanałów: {} zmian przy wczytaniu",
                ostrzezenia_kanalow.len()
            ),
            format!(
                "Kanał (albo temat forum) ma DOKŁADNIE JEDEN format. 
                 Dwa formaty na jednym kanale znaczyłyby, że ta sama wiadomość rodzi 
                 dwa koszyki z dwóch parserów na jednym rachunku — podwójna ekspozycja 
                 z jednego sygnału.

{}

                 Sprawdź te kanały w panelu i wybierz format świadomie.",
                ostrzezenia_kanalow.join(
                    "
"
                )
            ),
        );
    }

    if let Some(powod) = blad_ustawien {
        let tresc = format!(
            "Konfiguracja z settings.json NIE ZOSTAŁA WCZYTANA, więc silnik ma ustawienia              DOMYŚLNE — preset, progi ryzyka i konfiguracja wolumenu NIE obowiązują.              HANDEL JEST ZATRZYMANY: granie wartościami, których nikt nie wybrał, jest              gorsze niż niehandlowanie. Wczytaj preset z panelu i zdejmij zatrzymanie.

             Powód: {powod}

             POŁĄCZENIE Z TERMINALEM TEŻ JEST ZABLOKOWANE. Po naprawie pełnej konfiguracji uruchom Conduit ponownie. Samo Resume trading nie uruchomi terminala ani mostu w tej sesji.

             Najczęstsza przyczyna na Windows to znacznik BOM dokładany przez Notatnik              i przez `Set-Content -Encoding UTF8` w PowerShellu — od tej wersji BOM jest              pomijany, więc jeśli błąd mówi o czymś innym, plik ma prawdziwą usterkę              składni. Uszkodzony plik został zachowany obok; napraw go albo wczytaj              preset z panelu, ZANIM cokolwiek zmienisz w ustawieniach — pierwszy zapis              nadpisze settings.json wartościami domyślnymi."
        );
        st.log(
            "settings",
            "error",
            "USTAWIENIA NIE ZOSTAŁY WCZYTANE",
            tresc.clone(),
        );
        st.update(coalesce::Sections::one(coalesce::Section::Halt), |s| {
            s.halt.dolacz(
                ui::KlasaHaltu::Diagnoza,
                "konfiguracja nie została wczytana",
            );
        });
        st.notify(
            mailer::MailCategory::Lifecycle,
            "CONDUIT: utracono konfigurację",
            &tresc,
        );
    }

    // PIECZĘĆ PACZKI — rozjazd wykryty przy wczytaniu (patrz wyżej). Zgłaszamy
    // go TU, bo dopiero tutaj istnieje dziennik panelu i poczta. Zatrzymanie
    // jest osobne od tego z ustawień: konfiguracja może wczytać się bez błędu,
    // a mimo to być o trzy wersje starsza od binarki.
    if let Some(tresc) = rozjazd_tresc {
        st.log(
            "settings",
            "error",
            "PACZKA NIE ZGADZA SIĘ Z KONFIGURACJĄ",
            tresc.clone(),
        );
        st.update(coalesce::Sections::one(coalesce::Section::Halt), |s| {
            s.halt.dolacz(
                ui::KlasaHaltu::Diagnoza,
                "paczka nie zgadza się z konfiguracją na dysku",
            );
        });
        st.notify(
            mailer::MailCategory::Lifecycle,
            "CONDUIT: stara konfiguracja na dysku",
            &tresc,
        );
    }

    // ══ CO SIĘ STAŁO Z DIAGNOZĄ Z POPRZEDNIEGO URUCHOMIENIA ══
    //
    // Tu, a nie wyżej: dopiero w tym miejscu istnieje dziennik panelu i poczta,
    // ORAZ dopiero teraz wiadomo, czy tegoroczne sprawdzenia postawiły diagnozę
    // z powrotem. Cisza jest w obu przypadkach zakazana — „bot znowu handluje"
    // jest tak samo ważną wiadomością jak „bot dalej stoi".
    if let Some(stara) = wygasla_diagnoza {
        let dalej = st.read(|s| s.halt.powod(ui::KlasaHaltu::Diagnoza).to_string());
        if dalej.is_empty() {
            let tresc = format!(
                "Poprzednie uruchomienie zostawiło ZATRZYMANIE HANDLU z powodem:\n\
                 \x20 · {stara}\n\n\
                 To jest zatrzymanie klasy DIAGNOZA — zdanie o stanie świata, które \
                 sprawdza się przy każdym starcie od nowa. Sprawdziłem je teraz \
                 i PRZYCZYNA USTĄPIŁA, więc zatrzymanie WYGASŁO: bot handluje.\n\n\
                 Straż ryzyka (`max_dd_pct`, `max_dd_usd`, pułapy łańcucha) NIE została \
                 przy tym niczym ruszona — diagnoza i ryzyko to dwie osobne klasy \
                 zatrzymania i wygaśnięcie jednej nie rozbraja drugiej."
            );
            st.log(
                "settings",
                "success",
                "ZATRZYMANIE Z POPRZEDNIEGO URUCHOMIENIA WYGASŁO — bot handluje",
                tresc.clone(),
            );
            st.notify(
                mailer::MailCategory::Lifecycle,
                "CONDUIT: zatrzymanie wygasło, bot handluje",
                &tresc,
            );
        } else {
            st.log(
                "settings",
                "warn",
                "Zatrzymanie z poprzedniego uruchomienia POTWIERDZONE",
                format!(
                    "Poprzedni start zostawił zatrzymanie: {stara}\n\
                     Sprawdziłem przyczynę od nowa i NADAL ISTNIEJE, więc handel zostaje \
                     zatrzymany z powodem: {dalej}"
                ),
            );
        }
    }

    let ust_kroniki = st.workspace.load_kronika();
    if ust_kroniki.wlaczona {
        match kronika::Kronika::otworz(ust_kroniki.clone(), &st.workspace.root, now) {
            Ok(k) => {
                // MELDUNEK O CIĄGŁOŚCI — to jest cały sens pliku żyjącego poza
                // katalogiem bota. „Kronika ruszyła" nie odróżnia sytuacji
                // „dopisuję do pliku z 40 tysiącami wpisów" od „właśnie
                // założyłem pusty", a to jest różnica między działającą
                // aktualizacją a cicho utraconym archiwum.
                let zdanie = k.rozpoznanie().zdanie(k.sciezka());
                tracing::info!(plik = %k.sciezka().display(), "{zdanie}");
                let r = k.rozpoznanie().clone();
                let kopia = k.ostatnia_kopia().map(|p| p.display().to_string());
                st.log(
                    "kronika",
                    if r.obcy_format { "warn" } else { "info" },
                    if r.istnial && r.bajtow > 0 {
                        format!("Kronika: kontynuuję istniejący plik ({} wpisów)", r.wierszy)
                    } else {
                        "Kronika: nowy plik".to_string()
                    },
                    format!(
                        "{zdanie}{}",
                        match kopia {
                            Some(c) => format!("\nkopia zapasowa przed dopisaniem: {c}"),
                            None => String::new(),
                        }
                    ),
                );
                st.set_kronika(k);
            }
            // Awaria otwarcia pliku NIE MOŻE zatrzymać bota — handel nie zależy
            // od tego, czy da się prowadzić dziennik. Ale musi być głośna, bo
            // cicha kronika wygląda dokładnie tak samo jak cichy kanał.
            Err(e) => st.log(
                "kronika",
                "error",
                "Nie udało się otworzyć pliku kroniki",
                format!(
                    "{e:#}\n\nBot pracuje normalnie, ale STRUMIEŃ Z TELEGRAMA NIE JEST \
                     ZAPISYWANY. Sprawdź ścieżkę w kronika.json (teraz: {}) i uprawnienia \
                     do katalogu.",
                    ust_kroniki.plik
                ),
            ),
        }
    }

    match wznowione {
        Some(ts) => st.log(
            "backup_memory",
            "success",
            "Wznowiono stan z backup_memory",
            format!("zapis z {}", czas_lokalny(ts)),
        ),
        None => st.log(
            "backup_memory",
            "info",
            "Start bez zapisanego stanu",
            "pierwsze uruchomienie albo pusty katalog",
        ),
    }
    Ok(st)
}

fn czas_lokalny(ms: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(ms).single() {
        Some(t) => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => ms.to_string(),
    }
}

/// Składa router: statyki + REST + WebSocket.
pub fn router(st: StateHandle, source: WebSource) -> Router {
    use tower_http::cors::{AllowOrigin, CorsLayer};

    // Podczas pracy nad UI Vite serwuje stronę z :5180, a API jest na :8787 —
    // bez CORS przeglądarka zablokowałaby `fetch`. Wpuszczamy WYŁĄCZNIE
    // pochodzenie lokalne; serwer i tak nasłuchuje na pętli zwrotnej.
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _| {
            origin
                .to_str()
                .map(|o| {
                    o.contains("//localhost")
                        || o.contains("//127.0.0.1")
                        || o.starts_with("tauri://")
                })
                .unwrap_or(false)
        }))
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any);

    let statyki = {
        let src = source.clone();
        get(move |uri| web::serve(src.clone(), uri))
    };

    Router::new()
        .route("/ws", get(hub::ws_upgrade))
        .nest("/api", rest::router())
        .fallback(statyki)
        .layer(cors)
        .with_state(st)
}

/// Startuje serwer i pętle w tle. Wraca, gdy gniazdo już nasłuchuje.
pub async fn serve(cfg: ServerConfig, auth: auth::SharedAuth) -> Result<Running> {
    let st = bootstrap(&cfg, auth)?;

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|x| x.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let source = WebSource::resolve(cfg.web_dir.clone(), &exe_dir);
    if !source.has_app() {
        tracing::warn!(
            "interfejs nie jest zbudowany ({}). API działa; uruchom `npm run build` (skrypt sam kopiuje panel do paczek Rust).",
            source.describe()
        );
    }
    tracing::info!(zrodlo = %source.describe(), "źródło interfejsu");

    let app = router(st.clone(), source);

    let listener = tokio::net::TcpListener::bind(cfg.bind)
        .await
        .with_context(|| format!("nie udało się zająć adresu {}", cfg.bind))?;
    let addr = listener.local_addr()?;
    *st.public_url.write() = format!("http://{addr}");

    tokio::spawn(hub::delta_loop(st.clone()));
    tokio::spawn(hub::backup_loop(st.clone(), cfg.backup_every));
    // Logowanie z zapisanej sesji kończy się JUŻ PO tym miejscu i nie przechodzi
    // przez żaden endpoint — bez tego dozoru `connection.telegram` zostawał na
    // „disconnected" mimo zalogowanego konta.
    tokio::spawn(hub::auth_loop(st.clone()));

    // Poczta: kolejka leży OBOK programu, więc niewysłany alert przeżywa
    // restart procesu — a to jest dokładnie ta chwila, w której powiadomienie
    // jest najbardziej potrzebne.
    let notifier = Arc::new(notify::Notifier::new(
        notify::setup_from_state(&st),
        Arc::new(notify::SmtpMailer),
        Some(st.workspace.root.join("mail_queue.json")),
    ));
    st.set_notifier(Arc::clone(&notifier));
    tokio::spawn(notify::mail_loop(st.clone(), notifier));
    st.notify(
        mailer::MailCategory::Lifecycle,
        "CONDUIT wystartował",
        &format!(
            "Serwer nasłuchuje na http://{addr}\nKatalog roboczy: {}",
            st.workspace.root.display()
        ),
    );

    // `into_make_service_with_connect_info` — żeby handlery mogły zapytać, KTO
    // dzwoni. Potrzebuje tego dokładnie jedno miejsce: `POST /api/shell/reveal`
    // otwiera okno Eksploratora NA MASZYNIE SERWERA, więc dla panelu otwartego
    // z innego komputera jest bezużyteczne i musi zostać odrzucone (panel
    // pokazuje wtedy ścieżkę do skopiowania). Reszta tras tego nie czyta.
    let handle = tokio::spawn(async move {
        let app = app.into_make_service_with_connect_info::<std::net::SocketAddr>();
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(blad = %e, "serwer HTTP zakończył się błędem");
        }
    });

    tracing::info!(%addr, "CONDUIT nasłuchuje");
    Ok(Running {
        state: st,
        addr,
        handle,
    })
}

/// Wygodny konstruktor dla przypadku „nic jeszcze nie podłączone".
pub fn default_auth() -> auth::SharedAuth {
    Arc::new(auth::UnconfiguredAuth::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coalesce::{Section, Sections};

    fn tmp_workspace(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "conduit-srv-{tag}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        p
    }

    #[test]
    fn bootstrap_dziala_na_pustym_katalogu() {
        let dir = tmp_workspace("boot");
        let cfg = ServerConfig {
            workspace: dir.clone(),
            start_balance: 2000.0,
            ..Default::default()
        };
        let st = bootstrap(&cfg, default_auth()).unwrap();
        assert_eq!(st.read(|s| s.balance), 2000.0);
        assert!(dir.join("presets").is_dir());
        assert!(dir.join("backup_memory").is_dir());
        assert!(!st.mt5_runtime_start_allowed(), "empty first-run config cannot attach MT5");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stan_wraca_po_restarcie() {
        let dir = tmp_workspace("restart");
        let cfg = ServerConfig {
            workspace: dir.clone(),
            start_balance: 2000.0,
            ..Default::default()
        };

        {
            let st = bootstrap(&cfg, default_auth()).unwrap();
            st.update(Sections::one(Section::Positions), |s| {
                s.balance = 2456.78;
                s.halt = ui::HaltState::nowy(ui::KlasaHaltu::Ryzyko, "MAX DD 40%");
            });
            assert!(st.save_backup().unwrap());
        }

        // nowy proces czyta ten sam katalog
        let st2 = bootstrap(&cfg, default_auth()).unwrap();
        assert_eq!(st2.read(|s| s.balance), 2456.78);
        assert!(st2.read(|s| s.halt.active));
        assert_eq!(st2.read(|s| s.halt.reason.clone()), "MAX DD 40%");

        let _ = std::fs::remove_dir_all(dir);
    }


    /// Kompletny, POPRAWNY `settings.json` — tyle kluczy, żeby przeszedł
    /// bramkę „ubogi dokument ustawień to awaria" (próg 50).
    fn zapisz_poprawne_ustawienia(dir: &std::path::Path) {
        let mut o = serde_json::Map::new();
        for i in 0..80 {
            o.insert(format!("pole_{i}"), serde_json::json!(i));
        }
        let d = store::SettingsDoc {
            settings: serde_json::Value::Object(o),
            ..Default::default()
        };
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("settings.json"),
            serde_json::to_vec_pretty(&d).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn mt5_startup_latch_cannot_be_bypassed_by_resume_after_config_failure() {
        let dir = tmp_workspace("mt5-startup-latch");
        let cfg = ServerConfig { workspace: dir.clone(), ..Default::default() };
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("settings.json"), b"{ broken").unwrap();
        let failed = bootstrap(&cfg, default_auth()).unwrap();
        assert!(!failed.mt5_runtime_start_allowed());
        failed.update(Sections::one(Section::Halt), |s| s.halt = Default::default());
        zapisz_poprawne_ustawienia(&dir);
        assert!(!failed.mt5_runtime_start_allowed(), "even disk repair requires a fresh bootstrap");
        let repaired = bootstrap(&cfg, default_auth()).unwrap();
        assert!(repaired.mt5_runtime_start_allowed());
        repaired.update(Sections::one(Section::Halt), |s| {
            s.halt = ui::HaltState::nowy(ui::KlasaHaltu::Ryzyko, "MAX DD");
        });
        assert!(repaired.mt5_runtime_start_allowed(), "risk halt must not prevent managing existing positions");
    }

    #[test]
    fn diagnoza_gasnie_gdy_przyczyna_ustapila_i_mowi_o_tym_glosno() {
        let dir = tmp_workspace("diagnoza-gasnie");
        let cfg = ServerConfig {
            workspace: dir.clone(),
            start_balance: 1000.0,
            ..Default::default()
        };
        std::fs::create_dir_all(&dir).unwrap();

        // ---- start 1: konfiguracja NIE DO ODCZYTANIA ----
        std::fs::write(dir.join("settings.json"), b"{ to nie jest JSON").unwrap();
        {
            let st = bootstrap(&cfg, default_auth()).unwrap();
            assert!(
                st.read(|s| s.halt.active),
                "uszkodzony settings.json MUSI zatrzymać handel — to jest słuszna diagnoza"
            );
            assert!(
                st.read(|s| s.halt.ma(ui::KlasaHaltu::Diagnoza)),
                "zdanie o nieodczytanej konfiguracji jest klasy DIAGNOZA"
            );
            assert!(st.save_backup().unwrap());
        }

        // ---- start 2: konfiguracja POPRAWNA ----
        zapisz_poprawne_ustawienia(&dir);
        let st2 = bootstrap(&cfg, default_auth()).unwrap();
        assert!(
            !st2.read(|s| s.halt.active),
            "przyczyna ustąpiła — zatrzymanie z POPRZEDNIEGO uruchomienia nie ma prawa wrócić"
        );
        assert!(
            st2.read(|s| s.halt.reason.is_empty()),
            "powód też ma zniknąć, nie tylko flaga"
        );
        let logi = st2.read(|s| s.logs.clone());
        assert!(
            logi.iter().any(|l| l.title.contains("WYGASŁO")),
            "cisza jest zakazana: użytkownik musi zobaczyć, że bot znowu handluje"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ryzyko_przezywa_restart() {
        let dir = tmp_workspace("ryzyko-restart");
        let cfg = ServerConfig {
            workspace: dir.clone(),
            ..Default::default()
        };
        zapisz_poprawne_ustawienia(&dir);
        {
            let st = bootstrap(&cfg, default_auth()).unwrap();
            st.update(Sections::one(Section::Halt), |s| {
                s.halt
                    .ustaw(ui::KlasaHaltu::Ryzyko, "MAX DRAWDOWN 41.2% ≥ 40.0%");
            });
            assert!(st.save_backup().unwrap());
        }
        let st2 = bootstrap(&cfg, default_auth()).unwrap();
        assert!(
            st2.read(|s| s.halt.active),
            "strażnik obsunięcia MUSI przeżyć restart"
        );
        assert_eq!(
            st2.read(|s| s.halt.powod(ui::KlasaHaltu::Ryzyko).to_string()),
            "MAX DRAWDOWN 41.2% ≥ 40.0%"
        );
        assert!(
            !st2.read(|s| s.halt.ma(ui::KlasaHaltu::Diagnoza)),
            "przy poprawnej konfiguracji nie ma się skąd wziąć diagnoza"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// TEST 5a. OBA NARAZ — WARSTWA STARTU.
    ///
    /// Ryzyko z dysku (wczorajszy strażnik) plus świeża diagnoza (dziś rano
    /// przyjechała paczka z uszkodzoną konfiguracją). Do naprawy `bootstrap`
    /// PODMIENIAŁ całą strukturę `halt`, więc diagnoza po cichu kasowała powód
    /// zatrzymania od strażnika: użytkownik czytał „konfiguracja nie została
    /// wczytana" i nie miał jak się dowiedzieć, że konto stoi TAKŻE po
    /// obsunięciu.
    #[test]
    fn diagnoza_nie_kasuje_ryzyka_przy_starcie() {
        let dir = tmp_workspace("oba-start");
        let cfg = ServerConfig {
            workspace: dir.clone(),
            ..Default::default()
        };
        zapisz_poprawne_ustawienia(&dir);
        {
            let st = bootstrap(&cfg, default_auth()).unwrap();
            st.update(Sections::one(Section::Halt), |s| {
                s.halt.ustaw(ui::KlasaHaltu::Ryzyko, "MAX DD 40%");
            });
            assert!(st.save_backup().unwrap());
        }
        // konfiguracja psuje się MIĘDZY uruchomieniami
        std::fs::write(dir.join("settings.json"), b"{ znowu nie JSON").unwrap();

        let st2 = bootstrap(&cfg, default_auth()).unwrap();
        assert_eq!(
            st2.read(|s| s.halt.powod(ui::KlasaHaltu::Ryzyko).to_string()),
            "MAX DD 40%",
            "diagnoza nie ma prawa skasować zatrzymania od strażnika"
        );
        assert!(
            st2.read(|s| s.halt.ma(ui::KlasaHaltu::Diagnoza)),
            "świeża diagnoza ma stanąć obok, a nie zamiast"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stary_zapis_bez_klasy_migruje_po_tresci() {
        for (powod, ma_zostac) in [
            ("konfiguracja nie została wczytana", false),
            (
                "STORM: konfiguracja nie została wczytana · ZEN: konfiguracja nie została wczytana",
                false,
            ),
            ("MAX DRAWDOWN 41.2% ≥ 40.0%", true),
            (
                "STORM: MAX DD 40% · ZEN: konfiguracja nie została wczytana",
                true,
            ),
        ] {
            let dir = tmp_workspace("stary-zapis");
            let cfg = ServerConfig {
                workspace: dir.clone(),
                ..Default::default()
            };
            zapisz_poprawne_ustawienia(&dir);
            {
                let st = bootstrap(&cfg, default_auth()).unwrap();
                assert!(st.save_backup().unwrap());
            }
            // podmieniamy `halt` w gotowym pliku na kształt SPRZED zmiany
            let p = dir.join("backup_memory").join("latest.json");
            let mut v: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            v["halt"] = serde_json::json!({ "active": true, "reason": powod });
            std::fs::write(&p, serde_json::to_vec_pretty(&v).unwrap()).unwrap();

            let st2 = bootstrap(&cfg, default_auth()).unwrap();
            assert_eq!(
                st2.read(|s| s.halt.active),
                ma_zostac,
                "stary zapis [{powod}] został zaklasyfikowany odwrotnie, niż powinien"
            );
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn update_podbija_rewizje_i_znaczy_sekcje() {
        let dir = tmp_workspace("rev");
        let cfg = ServerConfig {
            workspace: dir.clone(),
            ..Default::default()
        };
        let st = bootstrap(&cfg, default_auth()).unwrap();

        let r0 = st.rev();
        st.update(Sections::one(Section::Stats), |s| s.stats.equity = 1.0);
        assert_eq!(st.rev(), r0 + 1);

        let brudne = st
            .take_dirty(now_ms() + 10_000)
            .expect("sekcja musi być brudna");
        assert!(brudne.contains(Section::Stats));

        let _ = std::fs::remove_dir_all(dir);
    }
}


#[cfg(test)]
mod testy_pieczeci_i_drabinki {
    use super::*;
    use crate::coalesce::{Section, Sections};

    fn tmp(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "conduit-vps1-{tag}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        p
    }

    /// Zakłada katalog roboczy z pieczęcią i drabinką w `backup_memory`,
    /// po czym startuje bota. Zwraca `true`, gdy pieczęć ZATRZYMAŁA handel.
    fn start_z_pieczecia(
        tag: &str,
        lancuch_paczki: &str,
        szczeble: Vec<&str>,
        wlaczona: bool,
    ) -> (bool, StateHandle) {
        let dir = tmp(tag);
        let cfg = ServerConfig {
            workspace: dir.clone(),
            ..Default::default()
        };

        // pierwszy start: zakładamy katalog i zapisujemy drabinkę do pamięci
        {
            let st = bootstrap(&cfg, default_auth()).unwrap();
            // `aktywny` CELOWO inny niż łańcuch paczki — bez rozjazdu ten test
            // przechodziłby, niczego nie sprawdzając.
            let inny = st.read(|s| {
                s.lancuchy
                    .lista
                    .iter()
                    .map(|l| l.nazwa.clone())
                    .find(|n| n != lancuch_paczki)
                    .expect("lista musi mieć co najmniej dwa łańcuchy")
            });
            st.update(Sections::one(Section::Settings), |s| {
                s.lancuchy.aktywny = inny;
                s.drabinka = ui::DrabinkaLancuchow {
                    enabled: wlaczona,
                    wlacznik: Some(wlaczona),
                    szczeble: szczeble
                        .iter()
                        .enumerate()
                        .map(|(i, l)| ui::SzczebelDrabinki {
                            prog_balance: (i as f64) * 500.0,
                            lancuch: (*l).to_string(),
                        })
                        .collect(),
                    histereza_pct: 2.0,
                    biezacy_prog: -1.0,
                    ostatnia_zmiana_ts: 0,
                };
            });
            // Drabinka wraca z `backup_memory`, ale `aktywny` już nie — musi
            // trafić NA DYSK, inaczej drugi start czyta domyślny i rozjazdu,
            // o który w teście chodzi, w ogóle by nie było.
            let z = st.read(|s| s.lancuchy.clone());
            st.workspace.save_lancuchy(&z, "").unwrap();
            assert!(st.save_backup().unwrap());
        }

        // pieczęć wjeżdża DOPIERO TERAZ, żeby pierwszy start jej nie widział
        let p = store::Paczka {
            nazwa: "VPSREADY".into(),
            zbudowano: "test".into(),
            aktywny_lancuch: lancuch_paczki.to_string(),
            presety_nog: Default::default(),
        };
        std::fs::write(
            dir.join("PACZKA.json"),
            serde_json::to_vec_pretty(&p).unwrap(),
        )
        .unwrap();

        let st2 = bootstrap(&cfg, default_auth()).unwrap();
        let halt = st2.read(|s| s.halt.active);
        (halt, st2)
    }

    /// Nazwy bierzemy Z LISTY WBUDOWANEJ, nie z literału — korona zmienia się
    /// co tydzień i test wbity w nazwę umiera na zdrowym kodzie.
    fn nazwy() -> Vec<String> {
        conduit_core::formaty::lancuchy_wbudowane()
            .into_iter()
            .map(|l| l.nazwa)
            .collect()
    }

    #[test]
    fn wylaczona_drabinka_nie_wywraca_pieczeci() {
        let n = nazwy();
        let paczka = n[0].clone();
        let obce: Vec<&str> = vec![&n[1], &n[2]];
        let (halt, st) = start_z_pieczecia("off", &paczka, obce, false);
        // Rozjazd na `lancuchy.aktywny` ZOSTAJE (tak ma być — to nie drabinka
        // rządzi), więc sprawdzamy TREŚĆ: o szczeblach nie ma być ani słowa.
        let logi = st.read(|s| s.logs.clone());
        let o_drabince = logi.iter().any(|l| l.content.contains("drabinka"));
        assert!(
            !o_drabince,
            "przy wyłączonej drabince pieczęć nie ma prawa o niej mówić"
        );
        assert!(
            halt,
            "rozjazd na samym `aktywny` dalej zatrzymuje handel — to nie jest VPS-1"
        );
        let _ = std::fs::remove_dir_all(st.workspace.root.clone());
    }

    /// DRABINKA WŁĄCZONA I WIELOSZCZEBLOWA, zawierająca łańcuch z paczki —
    /// druga połowa pułapki. Konto wejdzie na ten łańcuch przy swoim progu,
    /// więc pieczęć musi przejść, mimo że pozostałe szczeble są inne.
    #[test]
    fn wlaczona_drabinka_ze_szczeblem_z_paczki_przechodzi() {
        let n = nazwy();
        let paczka = n[0].clone();
        let szczeble: Vec<&str> = vec![&n[1], &n[0], &n[2]];
        let (halt, st) = start_z_pieczecia("on-ok", &paczka, szczeble, true);
        assert!(
            !halt,
            "wieloszczeblowa drabinka Z łańcuchem paczki to konfiguracja POPRAWNA — \
             pieczęć nie ma prawa zatrzymać handlu"
        );
        let _ = std::fs::remove_dir_all(st.workspace.root.clone());
    }

    /// DRABINKA WŁĄCZONA, ale łańcucha z paczki nie ma na żadnym szczeblu —
    /// to jest PRAWDZIWY rozjazd: konto nie dojdzie do składu z wysyłki nigdy.
    #[test]
    fn wlaczona_drabinka_bez_lancucha_paczki_zatrzymuje() {
        let n = nazwy();
        let paczka = n[0].clone();
        let szczeble: Vec<&str> = vec![&n[1], &n[2]];
        let (halt, st) = start_z_pieczecia("on-zle", &paczka, szczeble, true);
        assert!(
            halt,
            "drabinka, która NIGDY nie doprowadzi do składu z paczki, to rozjazd"
        );
        let logi = st.read(|s| s.logs.clone());
        assert!(
            logi.iter()
                .any(|l| l.content.contains("drabinka jest WŁĄCZONA")),
            "komunikat ma nazwać przyczynę wprost"
        );
        let _ = std::fs::remove_dir_all(st.workspace.root.clone());
    }

    /// Nazwa z pieczęci trafia do migawki — panel bez niej nie mógłby ostrzec
    /// PRZED włączeniem drabinki, a tylko wtedy ostrzeżenie ma jeszcze sens.
    #[test]
    fn nazwa_z_pieczeci_jest_w_migawce() {
        let n = nazwy();
        let (_, st) = start_z_pieczecia("snap", &n[0], vec![&n[0]], true);
        assert_eq!(st.read(|s| s.pieczec_lancuch.clone()), n[0]);
        let _ = std::fs::remove_dir_all(st.workspace.root.clone());
    }
}
