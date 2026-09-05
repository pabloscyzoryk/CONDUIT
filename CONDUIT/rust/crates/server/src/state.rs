//! Jedno źródło prawdy.
//!
//! Cały stan bota żyje w JEDNEJ strukturze pod `RwLock`. Serwer nie jest
//! właścicielem tego stanu — jest jego subskrybentem, dokładnie tak samo jak
//! okno natywne i przeglądarka. To jest warunek wymagania „okno i przeglądarka
//! działają RÓWNOLEGLE na tym samym stanie": skoro nikt nie trzyma prywatnej
//! kopii, nie ma czego synchronizować i nie ma jak się rozjechać.
//!
//! Zapis odbywa się wyłącznie przez [`StateHandle::update`], które przy okazji
//! podbija `rev` i oznacza brudne sekcje. Nie da się zmienić stanu i zapomnieć
//! powiadomić klientów — bo to jedno wywołanie.

use crate::auth::SharedAuth;
use crate::coalesce::{Coalescer, Section, Sections};
use crate::proto::Command;
use crate::store::{BackupMemory, Workspace};
use crate::ui;
use parking_lot::{Mutex, RwLock};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Powiadomienie rozsyłane do wszystkich podłączonych klientów.
///
/// Delta niesie tylko NUMER rewizji i maskę sekcji — właściwą treść każde
/// połączenie odczytuje samo ze wspólnego stanu. Dzięki temu nie kopiujemy
/// pozycji N razy dla N klientów, a każdy dostaje dane co najmniej tak świeże,
/// jak w momencie powiadomienia.
#[derive(Debug, Clone)]
pub enum Notice {
    Delta { rev: u64, sections: Sections },
    Event(Arc<ui::UiEvent>),
}

/// Punkt wpięcia środowiska uruchomieniowego (broker MT5 + klient Telegrama).
///
/// Serwer sam obsługuje komendy konfiguracyjne, ale nie ma prawa wysłać
/// zlecenia — od tego jest `crates/mt5`. Gdy nic nie jest podłączone,
/// komenda handlowa kończy się jawnym błędem w `ack`, a nie ciszą.
pub trait Runtime: Send + Sync {
    fn command(&self, cmd: &Command, state: &StateHandle) -> anyhow::Result<()>;
    /// The original UI intent scope must survive server dispatch and queuing.
    /// Live overrides this and rejects unscoped/stale commands in follow mode.
    fn command_scoped(&self, cmd: &Command, state: &StateHandle, _account_session: Option<&str>) -> anyhow::Result<()> {
        self.command(cmd, state)
    }
    /// Krótki opis do panelu diagnostycznego.
    fn name(&self) -> &'static str {
        "brak"
    }
}

/// Domyślne „nic nie podłączone".
pub struct NoRuntime;

impl Runtime for NoRuntime {
    fn command(&self, _cmd: &Command, _state: &StateHandle) -> anyhow::Result<()> {
        anyhow::bail!("Broker nie jest podłączony — komenda handlowa odrzucona")
    }
}

pub struct Shared {
    snap: RwLock<ui::UiSnapshot>,
    rev: AtomicU64,
    coalescer: Mutex<Coalescer>,
    tx: broadcast::Sender<Notice>,
    pub auth: SharedAuth,
    pub runtime: RwLock<Arc<dyn Runtime>>,
    /// Skąd brać świece i parametry instrumentów.
    ///
    /// Osobno od `runtime`, bo to inny rodzaj dostępu: `runtime` WYSYŁA
    /// polecenia handlowe i idzie przez pętlę silnika, a to tylko CZYTA
    /// i wolno je wołać z wątku HTTP. `None` = Conduit działa bez mostu do
    /// MT5; wtedy `/api/candles` odpowiada błędem, a nie wymyślonymi świecami.
    pub market: RwLock<Option<Arc<dyn crate::market::MarketSource>>>,
    pub workspace: Workspace,
    pub started_at: i64,
    /// adres, pod którym serwer jest osiągalny — potrzebny przyciskowi
    /// „Otwórz w przeglądarce" w oknie natywnym
    pub public_url: RwLock<String>,
    /// Laboratorium: pilnuje, żeby liczyło się JEDNO zadanie naraz, i trzyma
    /// token przerwania dla przycisku „PRZERWIJ".
    pub lab: crate::lab::LabControl,
    /// Tryb demo: jeden przebieg naraz, token zatrzymania i skrzynka na
    /// polecenia z panelu (ręczne sygnały, zamykanie pozycji).
    pub demo: crate::demo::DemoControl,
    /// Powiadomienia e-mail. `None`, dopóki nie wystartuje pętla poczty —
    /// wtedy „wyślij mail testowy" kończy się jawnym błędem zamiast cichym
    /// powodzeniem.
    pub notifier: RwLock<Option<Arc<crate::notify::Notifier>>>,
    pub tg: RwLock<Option<Arc<dyn PowiadamiaczTg>>>,
    /// KRONIKA — rejestrator strumienia z Telegrama do jednego ciągłego pliku.
    ///
    /// Mieszka w stanie, bo mają do niego dostęp DWIE strony: wątek
    /// `conduit-tg-filtr` z `crates/app` (pisze) i uchwyty REST-a (czytają
    /// liczniki, przestawiają opcje). `None` znaczy „nie udało się otworzyć
    /// pliku" — i wtedy panel mówi o tym wprost, zamiast pokazywać zera.
    pub kronika: Mutex<Option<crate::kronika::Kronika>>,
    log_seq: AtomicU64,
    /// czy stan zmienił się od ostatniego zapisu do `backup_memory`
    backup_dirty: std::sync::atomic::AtomicBool,
    /// Bootstrap-only safety latch. UI Resume/settings cannot turn a failed
    /// startup into terminal initialization with fallback credentials.
    mt5_startup_verified: std::sync::atomic::AtomicBool,
}

pub type StateHandle = Arc<Shared>;

impl Shared {
    pub fn new(workspace: Workspace, auth: SharedAuth, snap: ui::UiSnapshot) -> StateHandle {
        let (tx, _) = broadcast::channel(512);
        Arc::new(Shared {
            snap: RwLock::new(snap),
            rev: AtomicU64::new(0),
            coalescer: Mutex::new(Coalescer::default_rate()),
            tx,
            auth,
            runtime: RwLock::new(Arc::new(NoRuntime)),
            market: RwLock::new(None),
            workspace,
            started_at: crate::now_ms(),
            public_url: RwLock::new(String::new()),
            lab: crate::lab::LabControl::default(),
            demo: crate::demo::DemoControl::default(),
            notifier: RwLock::new(None),
            tg: RwLock::new(None),
            kronika: Mutex::new(None),
            log_seq: AtomicU64::new(1),
            backup_dirty: std::sync::atomic::AtomicBool::new(false),
            mt5_startup_verified: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub(crate) fn set_mt5_startup_verified(&self, verified: bool) {
        self.mt5_startup_verified.store(verified, Ordering::Release);
    }

    pub fn mt5_runtime_start_allowed(&self) -> bool {
        self.mt5_startup_verified.load(Ordering::Acquire)
    }

    /// Odczyt bez kopiowania całości.
    pub fn read<R>(&self, f: impl FnOnce(&ui::UiSnapshot) -> R) -> R {
        let g = self.snap.read();
        f(&g)
    }

    /// Pełna kopia stanu — do `snapshot` po podłączeniu i do REST-a.
    ///
    /// Ostatnia bramka przed siecią: hasło SMTP jest tu wycierane, nawet gdyby
    /// jakimś sposobem trafiło do stanu. Normalnie go tam nie ma (mieszka
    /// w `secrets.json`), ale ta struktura leci do KAŻDEGO klienta i do REST-a,
    /// więc lepiej mieć dwa zamki niż jeden.
    pub fn snapshot(&self) -> ui::UiSnapshot {
        let mut s = self.snap.read().clone();
        s.rev = self.rev.load(Ordering::Relaxed);
        s.server_time = crate::now_ms();
        s.email = s.email.redacted();
        s
    }

    #[inline]
    pub fn rev(&self) -> u64 {
        self.rev.load(Ordering::Relaxed)
    }

    /// JEDYNA droga do zmiany stanu.
    pub fn update<R>(&self, sections: Sections, f: impl FnOnce(&mut ui::UiSnapshot) -> R) -> R {
        let out = {
            let mut g = self.snap.write();
            f(&mut g)
        };
        let rev = self.rev.fetch_add(1, Ordering::Relaxed) + 1;
        self.backup_dirty.store(true, Ordering::Relaxed);
        self.coalescer.lock().mark(sections);
        // powiadamiamy o istnieniu zmiany; ramkę złoży pętla koalescencji
        let _ = rev;
        out
    }

    /// Zmiana, która NIE jest stanem bota — postęp długiego zadania.
    ///
    /// Różnica wobec [`Shared::update`] jest jedna: nie brudzi
    /// `backup_memory`. Pasek postępu backtestu zmienia się kilka razy na
    /// sekundę, a nie ma go po co odtwarzać po restarcie — bez tego rozróżnienia
    /// każdy przemiał presetów kazałby zapisywać migawkę stanu konta co 15
    /// sekund bez powodu.
    pub fn update_transient<R>(
        &self,
        sections: Sections,
        f: impl FnOnce(&mut ui::UiSnapshot) -> R,
    ) -> R {
        let out = {
            let mut g = self.snap.write();
            f(&mut g)
        };
        self.rev.fetch_add(1, Ordering::Relaxed);
        self.coalescer.lock().mark(sections);
        out
    }

    /// Zdarzenie — omija koalescencję, bo „koszyk B7 utworzony" spóźnione
    /// o 100 ms jest bez wartości, a zgubione jest błędem.
    pub fn emit(&self, ev: ui::UiEvent) {
        let _ = self.tx.send(Notice::Event(Arc::new(ev)));
    }

    /// Wpis do dziennika: ląduje w stanie (sekcja `logs`) i leci jako zdarzenie.
    pub fn log(
        &self,
        category: &str,
        level: &str,
        title: impl Into<String>,
        content: impl Into<String>,
    ) {
        let entry = ui::LogEntry {
            id: self.log_seq.fetch_add(1, Ordering::Relaxed),
            t: crate::now_ms(),
            category: category.to_string(),
            title: title.into(),
            content: content.into(),
            level: level.to_string(),
        };
        self.update(Sections::one(Section::Logs), |s| {
            s.logs.insert(0, entry.clone());
            s.logs.truncate(600);
        });
        self.emit(ui::UiEvent::Log {
            entry: Box::new(entry),
        });
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Notice> {
        self.tx.subscribe()
    }

    /// Liczba podłączonych klientów (okno + karty przeglądarki).
    pub fn clients(&self) -> usize {
        self.tx.receiver_count()
    }

    /// Wywoływane przez pętlę koalescencji.
    pub fn take_dirty(&self, now_ms: i64) -> Option<Sections> {
        self.coalescer.lock().take(now_ms)
    }

    pub fn broadcast_delta(&self, sections: Sections) {
        let _ = self.tx.send(Notice::Delta {
            rev: self.rev(),
            sections,
        });
    }

    // ---------- pamięć stanu ----------

    pub fn backup_needed(&self) -> bool {
        self.backup_dirty.load(Ordering::Relaxed)
    }

    /// Zapisuje `backup_memory/`. Zwraca `false`, gdy nie było czego zapisywać.
    pub fn save_backup(&self) -> anyhow::Result<bool> {
        if !self.backup_dirty.swap(false, Ordering::Relaxed) {
            return Ok(false);
        }
        let snap = self.snapshot();
        let mem = BackupMemory::from_snapshot(&snap, crate::now_ms());
        self.workspace.save_backup(&mem)?;
        Ok(true)
    }

    pub fn set_runtime(&self, r: Arc<dyn Runtime>) {
        *self.runtime.write() = r;
    }

    // ---------- kronika ----------

    /// Podpina rejestrator. Woła to `bootstrap` po wczytaniu `kronika.json`.
    pub fn set_kronika(&self, k: crate::kronika::Kronika) {
        *self.kronika.lock() = Some(k);
    }

    /// Dopisuje wiadomość do kroniki.
    ///
    /// ⚠ **Nie zwraca błędu i nie może go zwracać.** Woła to wątek, przez
    /// który przechodzą wiadomości handlowe: awaria zapisu archiwum nie ma
    /// prawa zatrzymać ani opóźnić sygnału. Błąd ląduje w licznikach (widać go
    /// w panelu) i w logu technicznym — nigdy nie znika po cichu, ale też
    /// nigdy nie blokuje handlu.
    pub fn kronika_zapisz(&self, p: crate::kronika::Przychodzace<'_>, rozpoznane: bool) {
        let mut g = self.kronika.lock();
        let Some(k) = g.as_mut() else { return };
        if let Err(e) = k.zapisz(p, rozpoznane) {
            tracing::warn!(blad = %e, "nie udało się dopisać wiadomości do kroniki");
        }
    }

    // ---------- dane rynkowe ----------

    /// Podpina źródło świec. Woła to warstwa aplikacji, gdy most do MT5 stoi.
    pub fn set_market(&self, m: Arc<dyn crate::market::MarketSource>) {
        *self.market.write() = Some(m);
    }

    /// Odpina źródło — po zatrzymaniu mostu. Od tej chwili `/api/candles`
    /// odpowiada błędem zamiast oddawać świece sprzed awarii jako bieżące.
    pub fn clear_market(&self) {
        *self.market.write() = None;
    }

    pub fn market(&self) -> Option<Arc<dyn crate::market::MarketSource>> {
        self.market.read().clone()
    }

    // ---------- powiadomienia ----------

    pub fn set_notifier(&self, n: Arc<crate::notify::Notifier>) {
        *self.notifier.write() = Some(n);
    }

    /// Zgłasza zdarzenie: PEŁNA treść do dziennika + (jeśli kategoria włączona)
    /// mail. To jest jedyna droga, którą reszta programu powiadamia o czymkolwiek.
    ///
    /// Gdy poczty nie ma (binarka bez pętli poczty, testy), zdarzenie i tak
    /// ląduje w dzienniku — nigdy nie znika po cichu.
    pub fn notify(&self, cat: crate::mailer::MailCategory, subject: &str, body: &str) {
        // Uchwyt wyjmujemy w OSOBNYM wyrażeniu, żeby blokada odczytu została
        // zwolniona, zanim ruszy właściwa praca. W `match` tymczasowy strażnik
        // żyłby przez całe ramię — a `notify` sięga potem po zapis stanu
        // i dziennik, więc trzymanie tu czegokolwiek jest proszeniem się
        // o zakleszczenie przy pierwszej zmianie w sąsiednim module.
        self.notify_mail(cat, subject, body);
        self.notify_tg(subject, body);
    }

    pub fn notify_mail(&self, cat: crate::mailer::MailCategory, subject: &str, body: &str) {
        // Uchwyt wyjmujemy w OSOBNYM wyrażeniu, żeby blokada odczytu została
        // zwolniona, zanim ruszy właściwa praca. W `match` tymczasowy strażnik
        // żyłby przez całe ramię — a `notify` sięga potem po zapis stanu
        // i dziennik, więc trzymanie tu czegokolwiek jest proszeniem się
        // o zakleszczenie przy pierwszej zmianie w sąsiednim module.
        let n = self.notifier.read().clone();
        match n {
            Some(n) => n.notify(self, cat, subject, body),
            None => self.log("email", "info", subject.to_string(), body.to_string()),
        }
    }

    /// To samo powiadomienie na wybrane czaty Telegrama.
    ///
    /// Odbiorcy pochodzą z DWÓCH miejsc panelu, bo panel ma dwa zaznaczenia
    /// znaczące to samo: listę na karcie „Powiadomienia Telegram" i pole
    /// „wysyłaj tu raporty bota" przy kafelku kanału. Bierzemy sumę obu,
    /// żeby żadne z nich nie było martwe.
    ///
    /// Awaria wysyłki NIE może dotknąć handlu: `wyslij` wraca natychmiast,
    /// a błąd ląduje w dzienniku po stronie usługi Telegrama.
    pub fn notify_tg(&self, subject: &str, body: &str) {
        let Some(tg) = self.tg.read().clone() else {
            return;
        };
        let odbiorcy: Vec<i64> = self.read(|s| {
            let mut v = s.notify.channels.clone();
            v.extend(
                s.bindings
                    .values()
                    .filter(|b| b.notify)
                    .map(|b| b.channel_id),
            );
            v.sort_unstable();
            v.dedup();
            v
        });
        if odbiorcy.is_empty() {
            return;
        }
        let tekst = if body.is_empty() {
            subject.to_string()
        } else {
            format!("{subject}\n\n{body}")
        };
        let ile = odbiorcy.len();
        for id in &odbiorcy {
            tg.wyslij(*id, None, &tekst);
        }
        // Ślad w dzienniku PANELU, nie tylko w logu technicznym. Wysyłka jest
        // z natury „wyślij i zapomnij", więc bez tego wpisu użytkownik nie ma
        // jak odróżnić „poszło" od „nie ma odbiorców" ani od „czat nieznany
        // sesji Telegrama" — a to ostatnie zdarza się, dopóki nikt nie otworzy
        // listy kanałów po starcie.
        self.log(
            "telegram",
            "info",
            format!("Powiadomienie na Telegram → {ile} kanał(ów)"),
            format!(
                "{subject}\nodbiorcy: {}",
                odbiorcy
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
}

/// Wysyłka powiadomienia na czat Telegrama.
///
/// Wąski interfejs celowo: serwer nie ma zależeć od klienta Telegrama, a
/// implementację wstawia warstwa aplikacji przy starcie usługi.
pub trait PowiadamiaczTg: Send + Sync {
    /// MUSI wracać natychmiast — wołane z pętli silnika, która obsługuje
    /// ticki. Właściwa wysyłka idzie w tło.
    fn wyslij(&self, chat_id: i64, topic_id: Option<i64>, text: &str);

    /// Czy wysyłka na ten czat MA SZANSĘ dojść.
    ///
    /// `wyslij` jest z natury „wyślij i zapomnij" i nie ma jak odesłać wyniku.
    /// Gdy czatu nie ma w mapie znanych rozmów, powiadomienie znika z samym
    /// wpisem w logu technicznym — a przycisk „Wyślij test" i tak melduje
    /// sukces. Mapa zapełnia się dopiero po pobraniu listy dialogów, więc
    /// zaraz po starcie procesu jest pusta; to dokładnie ten moment, w którym
    /// użytkownik naciska „Wyślij test".
    ///
    /// Domyślnie `true`, żeby implementacje, które nie mają jak tego sprawdzić,
    /// nie blokowały wysyłki — brak wiedzy nie może udawać wiedzy negatywnej.
    fn czy_osiagalny(&self, _chat_id: i64) -> bool {
        true
    }
}
