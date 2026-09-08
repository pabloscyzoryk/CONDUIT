
mod args;
mod demo;
mod instancja;
mod live;
/// Świece i parametry instrumentów dla panelu (zespół ŚWIECE).
mod market;
mod mt5_guard;
/// Rozdzielanie sygnałów na wiele silników (jeden na format) i widok brokera,
/// dzięki któremu silnik zarządza wyłącznie własnymi pozycjami.
mod routing;
mod runtime_python;
mod receipt_status;
mod quote_silence;
mod strategy_realized_memory;
#[cfg(any(feature = "window", test))]
mod shell_lifecycle;
#[cfg(feature = "window")]
mod window;
mod wznowienie;

use anyhow::Result;
use args::Parsed;
use conduit_server::{ServerConfig, Workspace};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Przekładka między stanem serwera a usługą Telegrama.
///
/// Serwer zna tylko wąski `PowiadamiaczTg` i nie zależy od klienta MTProto —
/// tę zależność domyka dopiero warstwa aplikacji, czyli to miejsce.
struct MostTg(Arc<conduit_telegram::TelegramService>);

impl conduit_server::state::PowiadamiaczTg for MostTg {
    fn wyslij(&self, chat_id: i64, topic_id: Option<i64>, text: &str) {
        self.0.powiadom(chat_id, topic_id, text.to_string());
    }

    fn czy_osiagalny(&self, chat_id: i64) -> bool {
        self.0.czy_znany_czat(chat_id)
    }
}

fn main() -> Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    let a = match args::parse(argv) {
        Ok(Parsed::Help) => {
            print!("{}", args::HELP);
            return Ok(());
        }
        Ok(Parsed::Version) => {
            println!("CONDUIT {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Ok(Parsed::Run(a)) => *a,
        Err(e) => {
            eprintln!("Błąd: {e}\n");
            print!("{}", args::HELP);
            std::process::exit(2);
        }
    };

    init_logi();

    let data_dir = match &a.data_dir {
        Some(p) => p.clone(),
        None => katalog_obok_exe()?,
    };

    // JEDNA INSTANCJA NA PORT.
    //
    // Zanim ruszymy cokolwiek ciężkiego (runtime, Telegram, MT5), pytamy port,
    // czy ktoś go już nie trzyma — i KTO. Gdy trzyma go nasz własny, działający
    // CONDUIT, drugie uruchomienie nie ma czego uruchamiać: pokazuje okno na tę
    // instancję i kończy się po cichu. Wcześniej wywalało się na `bind`
    // z komunikatem „os error 10048", po którym użytkownik uznawał program
    // za zepsuty.
    let adres: std::net::SocketAddr = (a.host, a.port).into();
    match instancja::sprawdz(adres) {
        instancja::Zastane::Wolny => {}
        instancja::Zastane::Nasz { url, wersja } => {
            println!("CONDUIT już działa (wersja {wersja}) — otwieram istniejące okno.");
            println!("Adres: {url}");
            println!("Serwer, Telegram i połączenie z MT5 zostają nietknięte.");
            pokaz_dzialajaca(&url, &a, &data_dir);
            return Ok(());
        }
        instancja::Zastane::Obcy { powod } => {
            eprintln!("Port {} jest zajęty przez INNY program ({powod}).", a.port);
            match instancja::wolny_port_obok(adres) {
                Some(p) => eprintln!(
                    "To nie jest druga instancja CONDUIT-a. Uruchom na innym porcie, np.:\n\
                     \n    conduit.exe --port {p}\n"
                ),
                None => eprintln!(
                    "To nie jest druga instancja CONDUIT-a. Wskaż wolny port opcją --port <numer>."
                ),
            }
            std::process::exit(1);
        }
    }

    let cfg = ServerConfig {
        bind: (a.host, a.port).into(),
        workspace: data_dir.clone(),
        web_dir: a.web_dir.clone(),
        backup_every: Duration::from_secs(15),
        start_balance: a.start_balance,
    };

    // Jedno środowisko asynchroniczne na cały proces. Okno natywne wymaga
    // GŁÓWNEGO wątku (pętla komunikatów Windows), więc runtime dostaje własne
    // wątki, a `main` zostaje wolny dla okna.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let demo_wl = a.demo;
    let ws_auth = Workspace::new(&data_dir);
    let (running, tg) = rt.block_on(async move {
        // PRAWDZIWY klient MTProto. Usługa wstaje natychmiast, a logowanie
        // z zapisanej sesji dzieje się w tle — dzięki temu serwer HTTP jest
        // dostępny nawet wtedy, gdy Telegram akurat nie odpowiada.
        let auth = conduit_telegram::TelegramService::start(ws_auth);
        let tg = auth.clone();
        let r = conduit_server::serve(cfg, auth).await?;
        if demo_wl {
            // `demo::run` tylko SKŁADA konfigurację i oddaje robotę serwerowi;
            // sam przebieg chodzi we własnym wątku systemowym (patrz
            // `conduit_server::demo::start`), więc nic tu nie blokuje runtime'u.
            demo::run(r.state.clone());
        }
        anyhow::Ok((r, tg))
    })?;

    running
        .state
        .tg
        .write()
        .replace(Arc::new(MostTg(tg.clone())));

    // Nadzór nad MT5. W trybie demo świadomie NIE startuje: symulacja gra na
    // wirtualnym brokerze, więc uruchamianie (a tym bardziej zabijanie)
    // prawdziwego terminala byłoby ostatnią rzeczą, jakiej ktoś oczekuje.
    //
    // Idzie PRZED pętlą handlową: nadzorca podnosi terminal, a most tylko się
    // do gotowego podłącza. Odwrotna kolejność dawała dwie próby uruchomienia
    // terminala naraz — nadzorcy i sidecara — na tym samym pliku.
    let terminal_runtime_allowed = !a.demo && running.state.mt5_runtime_start_allowed();
    if !a.demo && !terminal_runtime_allowed {
        running.state.log("mt5", "error", "Połączenie MT5 zablokowane przy starcie",
            "Pełna konfiguracja nie przeszła kontroli startowej. Terminal, watchdog i most NIE zostaną uruchomione. Napraw konfigurację i uruchom Conduit ponownie; Resume trading nie omija tej blokady.");
    }
    let mut straz = if !terminal_runtime_allowed {
        None
    } else {
        mt5_guard::start(running.state.clone())
    };

    let mut zywy = if !terminal_runtime_allowed {
        None
    } else {
        live::start(running.state.clone(), Some(tg))
    };

    if !a.demo {
        let st = running.state.clone();
        std::thread::Builder::new()
            .name("puls".into())
            .spawn(move || {
                // Pierwszy puls dopiero PO pełnym okresie — start i tak
                // ogłasza mail „CONDUIT wystartował".
                let mut ostatni = std::time::Instant::now();
                loop {
                    std::thread::sleep(Duration::from_secs(60));
                    let godzin = st
                        .read(|s| s.settings.get("puls_h").and_then(|v| v.as_f64()))
                        .filter(|x| x.is_finite())
                        .unwrap_or(6.0);
                    if godzin <= 0.0 {
                        continue; // wyłączony; pole czytamy co obrót, więc
                                  // włączenie w panelu działa bez restartu
                    }
                    if ostatni.elapsed().as_secs_f64() < godzin * 3600.0 {
                        continue;
                    }
                    if !live::rynek_powinien_dzialac_z(
                        live::przerwa_dobowa(&st),
                        live::offset_serwera_h(&st),
                    ) {
                        continue; // rynek śpi — puls poczeka na otwarcie
                    }
                    let (konto, mt5, saldo, equity, sygnaly, tryb) = st.read(|s| {
                        (
                            s.connection.account.clone(),
                            s.connection.mt5.clone(),
                            s.stats.balance,
                            s.stats.equity,
                            s.stats.signals,
                            format!("{:?}", s.mode),
                        )
                    });
                    st.notify(
                        conduit_server::mailer::MailCategory::Lifecycle,
                        "PULS — bot żyje",
                        &format!(
                            "Rachunek {} · {} · {}\n\
                             saldo {saldo:.2} · equity {equity:.2} · most MT5: {} · \
                             sygnałów od startu: {sygnaly} · tryb {tryb}\n\n\
                             To jest raport życia co {godzin:.0} h (Ustawienia → \
                             MetaTrader 5 → „puls co N godzin”; 0 wyłącza). \
                             Brak PULSU o czasie znaczy, że bot NIE ŻYJE albo \
                             nie ma poczty.",
                            konto.login,
                            konto.server,
                            konto.broker,
                            if mt5 == "connected" { "OK" } else { "BRAK" },
                        ),
                    );
                    ostatni = std::time::Instant::now();
                }
            })
            .expect("wątek pulsu musi wstać");
    }

    let url = running.url();
    println!("CONDUIT działa: {url}");
    println!("Katalog konfiguracji: {}", data_dir.display());
    wypisz_stan_konfiguracji(&data_dir);

    // `--lab` zmienia tylko WIDOK STARTOWY powłoki — serwer, stan i dane są
    // te same. To jest ta sama aplikacja otwarta na innej zakładce, a nie
    // drugi tryb pracy programu.
    let widok = if a.lab {
        "?view=lab"
    } else if a.demo {
        "?view=demo"
    } else {
        ""
    };

    if a.open {
        let adres = format!("{url}/{widok}");
        if let Err(e) = open::that_detached(adres.as_str()) {
            eprintln!("Nie udało się otworzyć przeglądarki: {e}");
        }
    }

    if a.headless {
        println!("Tryb headless — okno natywne wyłączone. Zatrzymanie: Ctrl+C.");
        rt.block_on(czekaj_na_przerwanie());
        zamknij(&running, &mut straz, &mut zywy);
        return Ok(());
    }

    #[cfg(feature = "window")]
    {
        shell_lifecycle::finish_native_session(
            || window::run(&url, a.lab, &data_dir),
            |error| {
                let adres = format!("{url}/{widok}");
                let text = format!("Native window unavailable ({error}). CONDUIT keeps running at {adres}; opening the browser.");
                eprintln!("{text}");
                running.state.log("system", "warn", "CONDUIT", &text);
                if let Err(error) = open::that_detached(adres.as_str()) {
                    let text = format!("Browser unavailable ({error}). CONDUIT keeps running; open {adres} manually.");
                    eprintln!("{text}");
                    running.state.log("system", "error", "CONDUIT", &text);
                    return Err(error.into());
                }
                Ok(())
            },
            || rt.block_on(czekaj_na_przerwanie()),
            || zamknij(&running, &mut straz, &mut zywy),
        );
        Ok(())
    }

    #[cfg(not(feature = "window"))]
    {
        println!(
            "Binarka zbudowana bez okna natywnego (brak cechy `window`) — \
             działam jak w trybie headless. Otwórz {url} w przeglądarce."
        );
        rt.block_on(czekaj_na_przerwanie());
        zamknij(&running, &mut straz, &mut zywy);
        Ok(())
    }
}

/// Pokazuje INTERFEJS DZIAŁAJĄCEJ INSTANCJI i wraca.
///
/// Okno natywne jest tu równorzędnym klientem cudzego serwera — dokładnie tak
/// samo jak karta przeglądarki. Nie ma własnego stanu, więc nie ma czego
/// synchronizować: to jest ta sama aplikacja, ta sama sesja Telegrama i to samo
/// połączenie z MT5, tylko druga szyba. Zamknięcie tego okna kończy WYŁĄCZNIE
/// ten proces; bot działa dalej.
fn pokaz_dzialajaca(url: &str, a: &args::Args, data_dir: &std::path::Path) {
    let widok = if a.lab { "?view=lab" } else { "" };

    #[cfg(feature = "window")]
    if !a.headless {
        match window::run(url, a.lab, data_dir) {
            Ok(()) => return,
            Err(e) => eprintln!("Nie udało się otworzyć okna ({e}) — otwieram przeglądarkę."),
        }
    }

    #[cfg(not(feature = "window"))]
    let _ = data_dir;

    if a.headless {
        return;
    }
    let adres = format!("{url}/{widok}");
    if let Err(e) = open::that_detached(adres.as_str()) {
        eprintln!("Nie udało się otworzyć przeglądarki: {e}");
        eprintln!("Otwórz ręcznie: {adres}");
    }
}

/// Ostatni zapis stanu przed wyjściem. Bez tego restart gubiłby do 15 sekund
/// pracy — dokładnie tyle, ile trwa okno cyklicznego zapisu.
fn zamknij(
    running: &conduit_server::Running,
    straz: &mut Option<mt5_guard::Guard>,
    zywy: &mut Option<live::Handle>,
) {
    // Najpierw nadzór: inaczej wątek pilnujący MT5 mógłby w tej chwili
    // uruchamiać terminal, którego już nikt nie będzie obsługiwał.
    if let Some(g) = straz.as_mut() {
        g.stop();
    }
    // Pętla handlowa musi stanąć PRZED zapisem stanu: inaczej zdążyłaby
    // jeszcze dopisać pozycję do migawki, którą właśnie utrwalamy.
    if let Some(z) = zywy.as_mut() {
        z.stop();
    }
    running.state.notify(
        conduit_server::mailer::MailCategory::Lifecycle,
        "CONDUIT zatrzymany",
        "Program zamknięty przez użytkownika. Otwarte pozycje zostają u brokera \
         i nie są od tej chwili zarządzane przez bota.",
    );
    match running.state.save_backup() {
        Ok(true) => println!("Zapisano backup_memory."),
        Ok(false) => {}
        Err(e) => eprintln!("Nie udało się zapisać backup_memory: {e}"),
    }
    running.abort();
}

async fn czekaj_na_przerwanie() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        eprintln!("Nie udało się nasłuchiwać Ctrl+C: {e}");
        // bez obsługi sygnału lepiej czekać w nieskończoność niż wyjść po cichu
        std::future::pending::<()>().await;
    }
}

fn katalog_obok_exe() -> Result<PathBuf> {
    Ok(Workspace::next_to_exe()?.root)
}

/// Wypisuje, co program ZNALAZŁ, a nie co powinno być. Ten wydruk kilka razy
/// oszczędzi pytania „dlaczego preset się nie wczytał".
fn wypisz_stan_konfiguracji(dir: &std::path::Path) {
    let ws = Workspace::new(dir);
    let plik = |p: PathBuf| if p.is_file() { "jest" } else { "BRAK" };
    println!(
        "  settings.json: {} · smtp.json: {} · channels.json: {} · presety: {} · backup: {}",
        plik(ws.settings_path()),
        plik(ws.smtp_path()),
        plik(ws.channels_path()),
        ws.load_presets().len(),
        if ws.backup_latest().is_file() {
            "jest"
        } else {
            "brak"
        },
    );
    // Poświadczenia: wypisujemy WYŁĄCZNIE „jest / nie ma". Ani hasza, ani
    // łańcucha sesji — to jest wydruk, który ludzie wklejają do zgłoszeń.
    let sek = ws.load_secrets().telegram;
    println!(
        "  Telegram: api_id {} · api_hash {} · zapisana sesja {} → logowanie {}",
        if sek.api_id != 0 {
            sek.api_id.to_string()
        } else {
            "BRAK".into()
        },
        if sek.api_hash.is_set() {
            "jest"
        } else {
            "BRAK"
        },
        if sek.has_session() { "jest" } else { "brak" },
        if sek.has_session() {
            "automatyczne"
        } else {
            "kodem QR"
        },
    );
}

fn init_logi() {
    use tracing_subscriber::EnvFilter;
    let filtr = EnvFilter::try_from_env("CONDUIT_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,conduit_server=info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filtr)
        .with_target(false)
        .try_init();
}
