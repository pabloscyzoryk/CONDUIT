
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Nazwa procesu terminala. Ta sama od MT5 build 1000 do dziś.
pub const PROCES: &str = "terminal64.exe";

/// Odstępy między kolejnymi SERIAMI prób (sekundy).
///
/// Wartości przepisane z `bot.py` (`RECONNECT_WAITS`, linia 99): 1 min, 3 min,
/// 15 min, 30 min, 1 h, 2 h, 4 h, 8 h — potem co 8 h już bez końca. Sens jest
/// taki: awaria trwająca sekundy to zwykle restart terminala, a awaria trwająca
/// godziny to zwykle weekend albo problem u brokera; w obu przypadkach bot ma
/// próbować dalej, a nie wyłączyć się.
pub const ODSTEPY_S: &[u64] = &[60, 180, 900, 1800, 3600, 7200, 14400, 28800];

#[derive(Debug, Clone)]
pub struct WatchdogConfig {
    /// Ścieżka do `terminal64.exe`. `None` = wykryj automatycznie.
    pub terminal_path: Option<PathBuf>,
    /// Czy w ogóle wolno uruchamiać i zabijać terminal.
    ///
    /// `false` przydaje się, gdy MT5 jest już prowadzony przez coś innego
    /// (usługa, harmonogram zadań) — wtedy nadzorca tylko obserwuje.
    pub manage_process: bool,
    /// Czy podnieść terminal od razu przy starcie bota, jeśli nie działa.
    pub start_on_launch: bool,
    /// Ile prób w jednej serii, zanim przejdziemy do długiego czekania.
    pub attempts_per_cycle: u32,
    /// Odstęp między próbami w serii.
    pub attempt_delay: Duration,
    /// Po ilu nieudanych próbach W SERII restartować aplikację MT5.
    ///
    /// 1 = restart przed drugą próbą (pierwsza jest zawsze „na sucho").
    /// 0 = nigdy nie restartuj, tylko próbuj się łączyć.
    pub restart_after: u32,
    /// Harmonogram przerw między seriami (sekundy). Ostatnia wartość powtarza się.
    pub cycle_waits_s: Vec<u64>,
    /// Ile czekamy na łagodne zamknięcie, zanim wymusimy `taskkill /F`.
    pub graceful_wait: Duration,
    /// Ile czekamy na wstanie terminala po uruchomieniu.
    pub start_grace: Duration,
    /// Co ile sprawdzamy, czy połączenie nadal żyje.
    pub health_interval: Duration,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        WatchdogConfig {
            terminal_path: None,
            manage_process: true,
            start_on_launch: true,
            attempts_per_cycle: 10,
            attempt_delay: Duration::from_secs(5),
            // Pierwsza próba BEZ restartu — patrz komentarz na górze pliku.
            restart_after: 1,
            cycle_waits_s: ODSTEPY_S.to_vec(),
            graceful_wait: Duration::from_secs(20),
            start_grace: Duration::from_secs(15),
            health_interval: Duration::from_secs(5),
        }
    }
}

impl WatchdogConfig {
    /// Sprawdza, czy konfiguracja nie zapętli nadzorcy.
    ///
    /// Zero prób w serii albo zerowy odstęp to nie „agresywne ustawienie",
    /// tylko pętla, która zje rdzeń procesora i zaleje dziennik.
    pub fn sanitized(mut self) -> Self {
        self.attempts_per_cycle = self.attempts_per_cycle.max(1);
        if self.attempt_delay < Duration::from_millis(500) {
            self.attempt_delay = Duration::from_millis(500);
        }
        if self.health_interval < Duration::from_millis(500) {
            self.health_interval = Duration::from_millis(500);
        }
        if self.cycle_waits_s.is_empty() {
            self.cycle_waits_s = ODSTEPY_S.to_vec();
        }
        self
    }
}

// ============================================================
//  HARMONOGRAM (czysta logika, bez systemu i bez zegara)
// ============================================================

/// Co zrobić przy próbie o danym numerze.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attempt {
    /// numer próby w serii, liczony od 1
    pub number: u32,
    /// czy przed tą próbą restartować aplikację MT5
    pub restart_app: bool,
    /// ile odczekać PRZED tą próbą
    pub delay: Duration,
}

/// Harmonogram prób. Świadomie bez stanu systemu — daje się przejść w pętli
/// w teście i sprawdzić co do sekundy.
#[derive(Debug, Clone)]
pub struct RetryPlan {
    cfg: WatchdogConfig,
}

impl RetryPlan {
    pub fn new(cfg: WatchdogConfig) -> Self {
        RetryPlan {
            cfg: cfg.sanitized(),
        }
    }

    /// Opis próby numer `n` (od 1) w serii.
    pub fn attempt(&self, n: u32) -> Attempt {
        let n = n.max(1);
        Attempt {
            number: n,
            restart_app: self.cfg.manage_process
                && self.cfg.restart_after > 0
                && n > self.cfg.restart_after,
            // pierwsza próba jest natychmiastowa — awaria właśnie się wydarzyła
            delay: if n == 1 {
                Duration::ZERO
            } else {
                self.cfg.attempt_delay
            },
        }
    }

    pub fn attempts_per_cycle(&self) -> u32 {
        self.cfg.attempts_per_cycle
    }

    /// Przerwa po nieudanej serii numer `cycle` (od 0).
    pub fn cycle_wait(&self, cycle: u32) -> Duration {
        let w = &self.cfg.cycle_waits_s;
        let i = (cycle as usize).min(w.len() - 1);
        Duration::from_secs(w[i])
    }

    /// Ile łącznie czasu minie od utraty połączenia do końca serii `cycle`.
    /// Do opisu w powiadomieniu — użytkownik ma wiedzieć, kiedy bot odpuści.
    pub fn elapsed_after_cycles(&self, cycles: u32) -> Duration {
        let seria = self.cfg.attempt_delay * (self.cfg.attempts_per_cycle - 1);
        let mut razem = Duration::ZERO;
        for c in 0..cycles {
            razem += seria + self.cycle_wait(c);
        }
        razem
    }
}

// ============================================================
//  ZDARZENIA
// ============================================================

/// Co nadzorca właśnie zrobił. Idzie do dziennika i do powiadomień.
#[derive(Debug, Clone, PartialEq)]
pub enum WatchdogEvent {
    /// terminal nie działał i został uruchomiony przy starcie bota
    Launched { path: PathBuf },
    /// nie udało się znaleźć `terminal64.exe`
    NotFound { szukano: Vec<PathBuf> },
    /// połączenie żyje (pierwsze zestawienie)
    Connected,
    /// połączenie utracone
    Lost { detail: String },
    /// pojedyncza próba nieudana
    AttemptFailed {
        attempt: u32,
        of: u32,
        cycle: u32,
        restarted: bool,
        detail: String,
    },
    /// cała seria nieudana — czekamy do następnej
    CycleFailed { cycle: u32, wait: Duration },
    /// aplikacja MT5 właśnie restartowana
    Restarting { forced: bool },
    /// połączenie wróciło po awarii
    Reconnected { after: Duration, attempts: u32 },
}

impl WatchdogEvent {
    /// Krótki tytuł do dziennika i tematu maila.
    pub fn title(&self) -> String {
        match self {
            WatchdogEvent::Launched { .. } => "Uruchomiono MetaTrader 5".into(),
            WatchdogEvent::NotFound { .. } => "Nie znaleziono terminala MT5".into(),
            WatchdogEvent::Connected => "MT5 podłączony".into(),
            WatchdogEvent::Lost { .. } => "UTRACONO połączenie z MT5".into(),
            WatchdogEvent::AttemptFailed {
                attempt, of, cycle, ..
            } => {
                format!("MT5: próba {attempt}/{of} nieudana (seria {})", cycle + 1)
            }
            WatchdogEvent::CycleFailed { cycle, wait } => format!(
                "MT5 nadal niedostępny — seria {} nieudana, kolejna za {}",
                cycle + 1,
                opis_czasu(*wait)
            ),
            WatchdogEvent::Restarting { forced } => {
                if *forced {
                    "Wymuszam zamknięcie MT5 (taskkill /F)".into()
                } else {
                    "Restartuję aplikację MT5".into()
                }
            }
            WatchdogEvent::Reconnected { after, attempts } => {
                format!(
                    "MT5 połączony ponownie po {} ({attempts} prób)",
                    opis_czasu(*after)
                )
            }
        }
    }

    /// Czy to zdarzenie zasługuje na mail?
    pub fn worth_mailing(&self) -> bool {
        !matches!(
            self,
            WatchdogEvent::AttemptFailed { .. } | WatchdogEvent::Restarting { .. }
        )
    }

    pub fn level(&self) -> &'static str {
        match self {
            WatchdogEvent::Connected | WatchdogEvent::Reconnected { .. } => "success",
            WatchdogEvent::Launched { .. } => "info",
            WatchdogEvent::Lost { .. } | WatchdogEvent::NotFound { .. } => "error",
            WatchdogEvent::CycleFailed { .. } => "error",
            _ => "warn",
        }
    }
}

/// „2 h 15 min" zamiast „8100 s".
pub fn opis_czasu(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        return format!("{s} s");
    }
    if s < 3600 {
        return format!("{} min", s / 60);
    }
    let h = s / 3600;
    let m = (s % 3600) / 60;
    if m == 0 {
        format!("{h} h")
    } else {
        format!("{h} h {m} min")
    }
}

// ============================================================
//  STEROWANIE TERMINALEM
// ============================================================

/// Wszystko, co nadzorca robi z systemem. Wydzielone w cechę, żeby test
/// mógł przejść pełny scenariusz awarii bez żywego MT5 i bez zabijania
/// czegokolwiek na maszynie deweloperskiej.
pub trait TerminalControl: Send + Sync {
    /// Czy proces terminala działa?
    fn is_running(&self) -> bool;
    /// Jedna próba nawiązania połączenia z terminalem.
    fn try_connect(&self) -> Result<(), String>;
    /// Zamknij terminal (łagodnie, potem siłą) i uruchom ponownie.
    fn restart(&self) -> Result<(), String>;
    /// Uruchom terminal, jeśli nie działa.
    fn launch(&self) -> Result<PathBuf, String>;
}

fn typowe_sciezki() -> Vec<PathBuf> {
    let mut v = Vec::new();
    let mut dodaj = |base: Option<String>, sub: &str| {
        if let Some(b) = base {
            if !b.is_empty() {
                v.push(PathBuf::from(b).join(sub).join(PROCES));
            }
        }
    };
    for marka in [
        "MetaTrader 5",
        "MetaTrader 5 Terminal",
        "Vantage MetaTrader 5",
        "ATFX MT5",
    ] {
        dodaj(std::env::var("ProgramFiles").ok(), marka);
        dodaj(std::env::var("ProgramFiles(x86)").ok(), marka);
    }
    // instalacje per-użytkownik: %APPDATA%\MetaQuotes\Terminal\<hash>\terminal64.exe
    if let Ok(appdata) = std::env::var("APPDATA") {
        let root = PathBuf::from(appdata).join("MetaQuotes").join("Terminal");
        if let Ok(rd) = std::fs::read_dir(&root) {
            for e in rd.flatten() {
                let p = e.path().join(PROCES);
                if p.is_file() {
                    v.push(p);
                }
            }
        }
    }
    v
}

/// Szuka `terminal64.exe`. Zwraca pierwszą istniejącą ścieżkę.
///
/// `None` nie jest katastrofą: MT5 może być już uruchomiony, a wtedy most i tak
/// się z nim połączy. Katastrofą byłoby ciche założenie, że terminal leży pod
/// ścieżką z `bot.py` — u połowy brokerów leży gdzie indziej.
pub fn discover_terminal() -> Option<PathBuf> {
    typowe_sciezki().into_iter().find(|p| p.is_file())
}

/// Wszystkie sprawdzone lokalizacje — do komunikatu „nie znaleziono".
pub fn searched_paths() -> Vec<PathBuf> {
    typowe_sciezki()
}

/// Prawdziwe sterowanie terminalem na Windows.
///
/// Zamykanie jest DWUETAPOWE, dokładnie jak w `bot.py`: najpierw `taskkill`
/// bez `/F` (terminal dostaje WM_CLOSE i domyka zapisy na dysk), a `/F` dopiero
/// po upływie karencji. Twarde zabicie w trakcie zapisu potrafi uszkodzić
/// historię konta i profil — a to jest szkoda nieodwracalna, w odróżnieniu od
/// dwudziestu sekund czekania.
pub struct WindowsTerminal {
    path: Option<PathBuf>,
    graceful_wait: Duration,
    start_grace: Duration,
    /// sprawdzenie połączenia — dostarcza je most (`Mt5Bridge`)
    probe: Box<dyn Fn() -> Result<(), String> + Send + Sync>,
}

impl WindowsTerminal {
    pub fn new(
        cfg: &WatchdogConfig,
        probe: impl Fn() -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        WindowsTerminal {
            path: cfg.terminal_path.clone().or_else(discover_terminal),
            graceful_wait: cfg.graceful_wait,
            start_grace: cfg.start_grace,
            probe: Box::new(probe),
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn taskkill(force: bool) {
        let mut c = std::process::Command::new("taskkill");
        if force {
            c.arg("/F");
        }
        let _ = c
            .args(["/IM", PROCES])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

impl TerminalControl for WindowsTerminal {
    fn is_running(&self) -> bool {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {PROCES}")])
            .output();
        match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout).contains(PROCES),
            // Brak `tasklist` (albo odmowa dostępu) nie może być odczytany jako
            // „terminal nie działa" — to wywołałoby restart działającego MT5.
            Err(_) => true,
        }
    }

    fn try_connect(&self) -> Result<(), String> {
        (self.probe)()
    }

    fn restart(&self) -> Result<(), String> {
        Self::taskkill(false);
        let deadline = std::time::Instant::now() + self.graceful_wait;
        while std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_secs(1));
            if !self.is_running() {
                break;
            }
        }
        if self.is_running() {
            tracing::warn!("MT5 nie zamknął się łagodnie — wymuszam taskkill /F");
            Self::taskkill(true);
            std::thread::sleep(Duration::from_secs(3));
        }
        self.launch().map(|_| ())
    }

    fn launch(&self) -> Result<PathBuf, String> {
        let Some(p) = &self.path else {
            return Err(format!(
                "nie znaleziono {PROCES} — wskaż ścieżkę w ustawieniach (sprawdzono {} lokalizacji)",
                searched_paths().len()
            ));
        };
        if !p.is_file() {
            return Err(format!(
                "ścieżka do terminala nie istnieje: {}",
                p.display()
            ));
        }
        std::process::Command::new(p)
            .spawn()
            .map_err(|e| format!("nie udało się uruchomić {}: {e}", p.display()))?;
        std::thread::sleep(self.start_grace);
        Ok(p.clone())
    }
}

// ============================================================
//  NADZORCA
// ============================================================

/// Wynik jednej pełnej próby odzyskania połączenia.
#[derive(Debug, Clone, PartialEq)]
pub enum Recovery {
    /// połączenie wróciło
    Recovered { attempts: u32, cycles: u32 },
    /// przerwano z zewnątrz (bot się zamyka)
    Aborted,
}

/// Nadzorca. Trzyma stan (połączony / nie) i prowadzi cykl odzyskiwania.
///
/// Celowo NIE jest asynchroniczny: cała jego praca to czekanie i wołanie
/// blokujących poleceń systemu, więc mieszka we własnym wątku, a nie w puli
/// zadań tokio, której nie wolno blokować.
pub struct Watchdog {
    plan: RetryPlan,
    ctrl: Box<dyn TerminalControl>,
    on_event: Box<dyn Fn(WatchdogEvent) + Send + Sync>,
    /// przerwanie z zewnątrz (zamykanie bota)
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// uśpienie wstrzykiwane z zewnątrz — testy podmieniają je na „nic"
    sleep: Box<dyn Fn(Duration) + Send + Sync>,
}

impl Watchdog {
    pub fn new(
        cfg: WatchdogConfig,
        ctrl: Box<dyn TerminalControl>,
        on_event: impl Fn(WatchdogEvent) + Send + Sync + 'static,
    ) -> Self {
        Watchdog {
            plan: RetryPlan::new(cfg),
            ctrl,
            on_event: Box::new(on_event),
            stop: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            sleep: Box::new(|d| std::thread::sleep(d)),
        }
    }

    /// Podmienia sposób czekania. Test przekazuje tu funkcję pustą i przechodzi
    /// pełny scenariusz ośmiu serii w mikrosekundy zamiast w piętnaście godzin.
    pub fn with_sleep(mut self, f: impl Fn(Duration) + Send + Sync + 'static) -> Self {
        self.sleep = Box::new(f);
        self
    }

    pub fn stop_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        std::sync::Arc::clone(&self.stop)
    }

    fn przerwane(&self) -> bool {
        self.stop.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn zdarzenie(&self, e: WatchdogEvent) {
        (self.on_event)(e);
    }

    /// Podnosi terminal przy starcie bota, jeśli nie działa.
    ///
    /// Zwraca `true`, gdy po tej operacji terminal działa (albo działał już
    /// wcześniej). Brak terminala NIE jest błędem krytycznym — most spróbuje
    /// się połączyć i tak, a użytkownik dostanie jasny komunikat.
    pub fn ensure_started(&self) -> bool {
        if self.ctrl.is_running() {
            return true;
        }
        match self.ctrl.launch() {
            Ok(p) => {
                self.zdarzenie(WatchdogEvent::Launched { path: p });
                true
            }
            Err(e) => {
                self.zdarzenie(WatchdogEvent::NotFound {
                    szukano: searched_paths(),
                });
                tracing::warn!(blad = %e, "nie udało się uruchomić terminala MT5");
                false
            }
        }
    }

    /// Pełny cykl odzyskiwania połączenia. Wraca dopiero, gdy się uda
    /// (albo gdy ktoś ustawi flagę zatrzymania).
    ///
    /// Program NIGDY nie kończy się z powodu niedostępnego MT5 — po
    /// wyczerpaniu harmonogramu ostatni odstęp powtarza się w nieskończoność.
    /// Bot, który sam się poddał, to bot, który zostawił otwarte pozycje bez
    /// nadzoru.
    pub fn recover(&self, powod: &str) -> Recovery {
        self.zdarzenie(WatchdogEvent::Lost {
            detail: powod.to_string(),
        });

        let mut cycle = 0u32;
        let mut razem = Duration::ZERO;
        let mut prob = 0u32;

        loop {
            for n in 1..=self.plan.attempts_per_cycle() {
                if self.przerwane() {
                    return Recovery::Aborted;
                }
                let a = self.plan.attempt(n);
                if !a.delay.is_zero() {
                    (self.sleep)(a.delay);
                    razem += a.delay;
                }
                if a.restart_app {
                    self.zdarzenie(WatchdogEvent::Restarting { forced: false });
                    if let Err(e) = self.ctrl.restart() {
                        tracing::warn!(blad = %e, "restart MT5 nieudany");
                    }
                }
                prob += 1;
                match self.ctrl.try_connect() {
                    Ok(()) => {
                        self.zdarzenie(WatchdogEvent::Reconnected {
                            after: razem,
                            attempts: prob,
                        });
                        return Recovery::Recovered {
                            attempts: prob,
                            cycles: cycle,
                        };
                    }
                    Err(e) => self.zdarzenie(WatchdogEvent::AttemptFailed {
                        attempt: n,
                        of: self.plan.attempts_per_cycle(),
                        cycle,
                        restarted: a.restart_app,
                        detail: e,
                    }),
                }
            }

            let wait = self.plan.cycle_wait(cycle);
            self.zdarzenie(WatchdogEvent::CycleFailed { cycle, wait });
            if self.przerwane() {
                return Recovery::Aborted;
            }
            (self.sleep)(wait);
            razem += wait;
            cycle = cycle.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};

    /// Atrapa terminala: łączy się dopiero po `padnie_do` nieudanych próbach.
    struct Atrapa {
        proby: AtomicU32,
        restarty: AtomicU32,
        uruchomienia: AtomicU32,
        /// ile pierwszych prób ma się nie udać
        padnie_do: u32,
        dziala: AtomicBool,
        /// czy `launch` ma się udać
        znaleziony: bool,
    }

    impl Atrapa {
        fn nowa(padnie_do: u32) -> Arc<Atrapa> {
            Arc::new(Atrapa {
                proby: AtomicU32::new(0),
                restarty: AtomicU32::new(0),
                uruchomienia: AtomicU32::new(0),
                padnie_do,
                dziala: AtomicBool::new(true),
                znaleziony: true,
            })
        }
    }

    /// Pośrednik, żeby test mógł trzymać `Arc` i jednocześnie oddać `Box` nadzorcy.
    struct Uchwyt(Arc<Atrapa>);

    impl TerminalControl for Uchwyt {
        fn is_running(&self) -> bool {
            self.0.dziala.load(Ordering::Relaxed)
        }
        fn try_connect(&self) -> Result<(), String> {
            let n = self.0.proby.fetch_add(1, Ordering::Relaxed) + 1;
            if n <= self.0.padnie_do {
                Err(format!("initialize() nieudane (próba {n})"))
            } else {
                Ok(())
            }
        }
        fn restart(&self) -> Result<(), String> {
            self.0.restarty.fetch_add(1, Ordering::Relaxed);
            self.0.dziala.store(true, Ordering::Relaxed);
            Ok(())
        }
        fn launch(&self) -> Result<PathBuf, String> {
            if !self.0.znaleziony {
                return Err("brak terminal64.exe".into());
            }
            self.0.uruchomienia.fetch_add(1, Ordering::Relaxed);
            self.0.dziala.store(true, Ordering::Relaxed);
            Ok(PathBuf::from("C:/MT5/terminal64.exe"))
        }
    }

    fn nadzorca(
        cfg: WatchdogConfig,
        a: &Arc<Atrapa>,
    ) -> (
        Watchdog,
        Arc<Mutex<Vec<WatchdogEvent>>>,
        Arc<Mutex<Vec<Duration>>>,
    ) {
        let log: Arc<Mutex<Vec<WatchdogEvent>>> = Arc::default();
        let spanie: Arc<Mutex<Vec<Duration>>> = Arc::default();
        let l = Arc::clone(&log);
        let s = Arc::clone(&spanie);
        let w = Watchdog::new(cfg, Box::new(Uchwyt(Arc::clone(a))), move |e| {
            l.lock().unwrap().push(e)
        })
        // testy nie czekają NAPRAWDĘ — zapisują, ile nadzorca chciał czekać
        .with_sleep(move |d| s.lock().unwrap().push(d));
        (w, log, spanie)
    }

    // ---------------- harmonogram ----------------

    #[test]
    fn pierwsza_proba_jest_natychmiastowa_i_bez_restartu() {
        // REGRESJA wobec bot.py: tam KAŻDA próba zaczynała się od restartu
        // aplikacji, więc chwilowa zadyszka terminala kosztowała pełny cykl
        // zamknij-zabij-uruchom, zamiast jednego ponowienia.
        let p = RetryPlan::new(WatchdogConfig::default());
        let a1 = p.attempt(1);
        assert_eq!(
            a1.delay,
            Duration::ZERO,
            "awaria właśnie się wydarzyła — próbujemy od razu"
        );
        assert!(
            !a1.restart_app,
            "pierwsza próba nie może zabijać działającego terminala"
        );

        let a2 = p.attempt(2);
        assert_eq!(a2.delay, Duration::from_secs(5));
        assert!(
            a2.restart_app,
            "gdy samo ponowienie nie pomogło, restartujemy aplikację"
        );
    }

    #[test]
    fn restart_da_sie_calkowicie_wylaczyc() {
        let cfg = WatchdogConfig {
            restart_after: 0,
            ..Default::default()
        };
        let p = RetryPlan::new(cfg);
        for n in 1..=10 {
            assert!(
                !p.attempt(n).restart_app,
                "próba {n} nie powinna restartować"
            );
        }
        // i to samo, gdy nadzorca nie ma prawa ruszać procesu
        let p2 = RetryPlan::new(WatchdogConfig {
            manage_process: false,
            ..Default::default()
        });
        assert!(!p2.attempt(9).restart_app);
    }

    #[test]
    fn odstepy_miedzy_seriami_rosna_i_zatrzymuja_sie_na_ostatnim() {
        let p = RetryPlan::new(WatchdogConfig::default());
        assert_eq!(p.cycle_wait(0), Duration::from_secs(60));
        assert_eq!(p.cycle_wait(1), Duration::from_secs(180));
        assert_eq!(p.cycle_wait(7), Duration::from_secs(28800));
        // po wyczerpaniu listy ostatnia wartość powtarza się BEZ KOŃCA —
        // bot nie ma prawa się poddać, bo zostawiłby pozycje bez nadzoru
        assert_eq!(p.cycle_wait(50), Duration::from_secs(28800));
        assert_eq!(p.cycle_wait(u32::MAX), Duration::from_secs(28800));
    }

    #[test]
    fn zla_konfiguracja_nie_zapetla_nadzorcy() {
        let cfg = WatchdogConfig {
            attempts_per_cycle: 0,
            attempt_delay: Duration::ZERO,
            health_interval: Duration::ZERO,
            cycle_waits_s: Vec::new(),
            ..Default::default()
        }
        .sanitized();
        assert_eq!(cfg.attempts_per_cycle, 1);
        assert!(cfg.attempt_delay >= Duration::from_millis(500));
        assert!(cfg.health_interval >= Duration::from_millis(500));
        assert!(!cfg.cycle_waits_s.is_empty());
    }

    #[test]
    fn laczny_czas_walki_jest_policzalny() {
        let p = RetryPlan::new(WatchdogConfig::default());
        // seria: 9 odstępów po 5 s = 45 s; + przerwa 60 s
        assert_eq!(p.elapsed_after_cycles(1), Duration::from_secs(45 + 60));
        assert_eq!(
            p.elapsed_after_cycles(2),
            Duration::from_secs(45 + 60 + 45 + 180)
        );
    }

    // ---------------- scenariusze odzyskiwania ----------------

    #[test]
    fn odzyskanie_po_pierwszej_probie_nie_dotyka_procesu() {
        let a = Atrapa::nowa(0);
        let (w, log, spanie) = nadzorca(WatchdogConfig::default(), &a);
        let r = w.recover("terminal_info() = None");
        assert_eq!(
            r,
            Recovery::Recovered {
                attempts: 1,
                cycles: 0
            }
        );
        assert_eq!(
            a.restarty.load(Ordering::Relaxed),
            0,
            "nie wolno restartować działającego MT5"
        );
        assert!(
            spanie.lock().unwrap().is_empty(),
            "pierwsza próba jest natychmiastowa"
        );

        let l = log.lock().unwrap();
        assert!(matches!(l[0], WatchdogEvent::Lost { .. }));
        assert!(matches!(
            l[1],
            WatchdogEvent::Reconnected { attempts: 1, .. }
        ));
    }

    #[test]
    fn odzyskanie_w_drugiej_probie_restartuje_aplikacje_raz() {
        let a = Atrapa::nowa(1);
        let (w, log, _) = nadzorca(WatchdogConfig::default(), &a);
        assert_eq!(
            w.recover("padło"),
            Recovery::Recovered {
                attempts: 2,
                cycles: 0
            }
        );
        assert_eq!(a.restarty.load(Ordering::Relaxed), 1);

        let l = log.lock().unwrap();
        assert!(l
            .iter()
            .any(|e| matches!(e, WatchdogEvent::AttemptFailed { attempt: 1, .. })));
        assert!(l
            .iter()
            .any(|e| matches!(e, WatchdogEvent::Restarting { .. })));
    }

    #[test]
    fn dluga_awaria_przechodzi_przez_kolejne_serie_z_dluzszymi_przerwami() {
        // 25 nieudanych prób przy 10 na serię = trzecia seria; sprawdzamy,
        // że przerwy między seriami idą wg harmonogramu
        let a = Atrapa::nowa(25);
        let (w, log, spanie) = nadzorca(WatchdogConfig::default(), &a);
        let r = w.recover("broker zamknął sesję");
        assert_eq!(
            r,
            Recovery::Recovered {
                attempts: 26,
                cycles: 2
            }
        );

        let s = spanie.lock().unwrap();
        // przerwy MIĘDZY seriami — reszta to odstępy 5 s wewnątrz serii
        let dlugie: Vec<u64> = s.iter().map(|d| d.as_secs()).filter(|x| *x >= 60).collect();
        assert_eq!(dlugie, vec![60, 180], "harmonogram przerw musi narastać");

        let l = log.lock().unwrap();
        let serie: Vec<u32> = l
            .iter()
            .filter_map(|e| match e {
                WatchdogEvent::CycleFailed { cycle, .. } => Some(*cycle),
                _ => None,
            })
            .collect();
        assert_eq!(serie, vec![0, 1]);
    }

    #[test]
    fn zatrzymanie_przerywa_walke_natychmiast() {
        // bot się zamyka w trakcie awarii MT5 — nadzorca nie może trzymać
        // procesu przy życiu przez osiem godzin czekania
        let a = Atrapa::nowa(u32::MAX);
        let (w, _log, _s) = nadzorca(WatchdogConfig::default(), &a);
        let flaga = w.stop_flag();
        flaga.store(true, Ordering::Relaxed);
        assert_eq!(w.recover("cokolwiek"), Recovery::Aborted);
        assert_eq!(
            a.proby.load(Ordering::Relaxed),
            0,
            "po zatrzymaniu nie próbujemy w ogóle"
        );
    }

    #[test]
    fn nieskonczona_awaria_nie_konczy_programu() {
        // 200 prób = 20 serii; harmonogram ma 8 pozycji, więc dalej powtarza
        // ostatnią. Program nigdy nie wychodzi — po prostu czeka rzadziej.
        let a = Atrapa::nowa(200);
        let (w, _log, spanie) = nadzorca(WatchdogConfig::default(), &a);
        let r = w.recover("długa awaria");
        assert!(matches!(r, Recovery::Recovered { cycles: 20, .. }));
        let s = spanie.lock().unwrap();
        let osemki = s.iter().filter(|d| d.as_secs() == 28800).count();
        assert!(
            osemki >= 10,
            "po wyczerpaniu listy odstęp 8 h ma się powtarzać, było {osemki}"
        );
    }

    // ---------------- start terminala ----------------

    #[test]
    fn dzialajacy_terminal_nie_jest_uruchamiany_drugi_raz() {
        let a = Atrapa::nowa(0);
        let (w, log, _) = nadzorca(WatchdogConfig::default(), &a);
        assert!(w.ensure_started());
        assert_eq!(a.uruchomienia.load(Ordering::Relaxed), 0);
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn wylaczony_terminal_jest_podnoszony_przy_starcie() {
        let a = Atrapa::nowa(0);
        a.dziala.store(false, Ordering::Relaxed);
        let (w, log, _) = nadzorca(WatchdogConfig::default(), &a);
        assert!(w.ensure_started());
        assert_eq!(a.uruchomienia.load(Ordering::Relaxed), 1);
        assert!(matches!(
            log.lock().unwrap()[0],
            WatchdogEvent::Launched { .. }
        ));
    }

    #[test]
    fn brak_pliku_terminala_daje_jasny_komunikat_a_nie_ciszy() {
        let a = Arc::new(Atrapa {
            proby: AtomicU32::new(0),
            restarty: AtomicU32::new(0),
            uruchomienia: AtomicU32::new(0),
            padnie_do: 0,
            dziala: AtomicBool::new(false),
            znaleziony: false,
        });
        let (w, log, _) = nadzorca(WatchdogConfig::default(), &a);
        assert!(!w.ensure_started());
        let l = log.lock().unwrap();
        assert!(matches!(l[0], WatchdogEvent::NotFound { .. }), "{:?}", l[0]);
        assert!(l[0].title().contains("Nie znaleziono"));
    }

    // ---------------- opisy ----------------

    #[test]
    fn zdarzenia_maja_czytelne_tytuly_i_poziomy() {
        let e = WatchdogEvent::CycleFailed {
            cycle: 1,
            wait: Duration::from_secs(180),
        };
        assert_eq!(
            e.title(),
            "MT5 nadal niedostępny — seria 2 nieudana, kolejna za 3 min"
        );
        assert_eq!(e.level(), "error");
        assert!(e.worth_mailing());

        // pojedyncza nieudana próba NIE zasługuje na mail — inaczej jedna
        // awaria dałaby dziesięć wiadomości w pięćdziesiąt sekund
        let a = WatchdogEvent::AttemptFailed {
            attempt: 3,
            of: 10,
            cycle: 0,
            restarted: true,
            detail: "x".into(),
        };
        assert!(!a.worth_mailing());
        assert_eq!(a.title(), "MT5: próba 3/10 nieudana (seria 1)");

        assert!(WatchdogEvent::Lost {
            detail: String::new()
        }
        .worth_mailing());
        assert_eq!(
            WatchdogEvent::Reconnected {
                after: Duration::from_secs(3720),
                attempts: 12
            }
            .title(),
            "MT5 połączony ponownie po 1 h 2 min (12 prób)"
        );
    }

    #[test]
    fn czas_opisuje_sie_po_ludzku() {
        assert_eq!(opis_czasu(Duration::from_secs(45)), "45 s");
        assert_eq!(opis_czasu(Duration::from_secs(180)), "3 min");
        assert_eq!(opis_czasu(Duration::from_secs(3600)), "1 h");
        assert_eq!(opis_czasu(Duration::from_secs(28800)), "8 h");
        assert_eq!(opis_czasu(Duration::from_secs(5400)), "1 h 30 min");
    }

    #[test]
    fn wykrywanie_terminala_nie_wywraca_sie_bez_mt5() {
        // na maszynie deweloperskiej MT5 zwykle nie ma — funkcja ma zwrócić
        // None, a nie panikować przy czytaniu nieistniejących katalogów
        let _ = discover_terminal();
        assert!(!searched_paths().is_empty(), "musimy gdziekolwiek szukać");
        assert!(searched_paths().iter().all(|p| p.ends_with(PROCES)));
    }
}
