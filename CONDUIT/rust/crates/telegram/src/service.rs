//! Usługa logowania — spina klienta MTProto z kontraktem serwera.
//!
//! To jest miejsce, w którym atrapa kodu QR zamienia się w prawdziwe logowanie.
//! Serwer definiuje automat stanu ([`conduit_server::auth::TelegramAuth`]),
//! `crates/telegram` dostarcza działającą implementację, a interfejs nie musi
//! wiedzieć, że coś się zmieniło.
//!
//! # Kolejność, która ma znaczenie
//!
//! ```text
//!   1. api_id + api_hash   ← użytkownik, z https://my.telegram.org
//!   2. połączenie MTProto  ← dopiero teraz da się cokolwiek zapytać
//!   3. auth.exportLoginToken → token → PRAWDZIWY kod QR
//!   4. skan telefonem (+ ewentualne hasło 2FA)
//!   5. zapis do secrets.json: api_id, api_hash, łańcuch sesji
//!   ────────────────────────────────────────────────────────────
//!   następne uruchomienia: 5 → 2 → zalogowany, BEZ kodu QR
//! ```
//!
//! Punkt 1 nie jest formalnością. Token logowania wydaje serwer Telegrama
//! w odpowiedzi na `auth.exportLoginToken`, a to wywołanie wymaga poświadczeń
//! aplikacji. Kod QR narysowany przed ich podaniem może zawierać wyłącznie
//! wymyślony adres — i telefon go zeskanuje, i nic się nie stanie.
//!
//! # Dlaczego osobne zadanie w tle
//!
//! Metody cechy są synchroniczne i wołane z uchwytów HTTP. Logowanie trwa
//! sekundy (połączenie, wymiana kluczy, oczekiwanie na skan), a token QR
//! wygasa co ~30 s i trzeba go odświeżać BEZ udziału interfejsu. Dlatego całą
//! pracę wykonuje jedno zadanie tokio, a metody cechy tylko wysyłają do niego
//! polecenia i czekają na odpowiedź z górnym ograniczeniem czasu.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use conduit_server::auth::{AuthStage, AuthState, ChannelInfo, ChannelTopic, TelegramAuth};
use conduit_server::store::Workspace;
use grammers_client::session::types::PeerRef;
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use crate::client::{ClientConfig, Kasowanie, PolitykaKasowania, TelegramClient, Zdarzenie};
use crate::dialogs::DialogEntry;
use crate::history_fetch::{HistoryFetchRequest, HistorySnapshot, TelegramHistorySource};
use crate::login::{LoggedIn, LoginStage, QrLogin};
use crate::photos::PhotoCache;
use crate::session::FileSession;

/// Ile modułów kodu QR na piksel w SVG dla okna.
const QR_MODUL_PX: usize = 6;

const LIMIT_ODPOWIEDZI: Duration = Duration::from_secs(60);

/// Nazwa pliku sesji obok programu.
pub const PLIK_SESJI: &str = "telegram.session";

// ============================================================
//  POLECENIA DO ZADANIA W TLE
// ============================================================

type Odpowiedz = oneshot::Sender<Result<AuthState, String>>;

enum Cmd {
    /// Read-only evidence; never sent to MessageSink. Runs outside update loop.
    HistoryPreview {
        request: HistoryFetchRequest,
        deadline: tokio::time::Instant,
        permit: tokio::sync::OwnedSemaphorePermit,
        reply: oneshot::Sender<Result<HistorySnapshot, String>>,
    },
    SetCredentials {
        api_id: i32,
        api_hash: String,
        reply: Odpowiedz,
    },
    StartQr {
        reply: Odpowiedz,
    },
    Password {
        password: String,
        reply: Odpowiedz,
    },
    Logout {
        forget: bool,
        reply: Odpowiedz,
    },
    /// lista czatów konta do ekranu wyboru kanałów
    Dialogs {
        reply: oneshot::Sender<Result<Vec<ChannelInfo>, String>>,
    },
    /// miniatura zdjęcia profilowego czatu (ścieżka pliku na dysku)
    Photo {
        chat_id: i64,
        reply: oneshot::Sender<Result<Option<PathBuf>, String>>,
    },
    /// powiadomienie na czat — BEZ odpowiedzi, „wyślij i zapomnij"
    Powiadom {
        chat_id: i64,
        topic_id: Option<i64>,
        text: String,
    },
    /// oś B11 przestawiona w panelu — dogonić nią podniesionego klienta
    PolitykaKasowania(PolitykaKasowania),
}

/// Co trzeba wiedzieć o czacie, żeby pobrać jego zdjęcie: dokąd zapytać
/// (`PeerRef` niesie `access_hash`) i o którą wersję obrazka prosić.
#[derive(Debug, Clone)]
struct PeerFoto {
    peer: PeerRef,
    photo_id: Option<i64>,
    is_forum: bool,
}

/// Mapa czat → dane do pobrania zdjęcia, wspólna dla zadania w tle
/// i dla zadań pobierających miniatury.
type MapaPeerow = Arc<Mutex<HashMap<i64, PeerFoto>>>;

// ============================================================
//  USŁUGA
// ============================================================

/// Odbiorca wiadomości z obserwowanych kanałów.
///
/// Zwykły kanał `std`, a nie `tokio`: po drugiej stronie stoi pętla handlowa
/// na WŁASNYM wątku systemowym (cecha `Broker` jest synchroniczna), więc kanał
/// asynchroniczny wymuszałby tam blokowanie na runtime, którego ta pętla
/// świadomie nie dotyka.
pub type MessageSink = std::sync::mpsc::Sender<conduit_core::engine::IncomingMessage>;

/// Dokąd oddawać SKASOWANIA wiadomości (zadanie B11).
///
/// Osobna prenumerata, a nie kolejny wariant w [`MessageSink`]: odbiorcy,
/// którzy o skasowaniach nie chcą wiedzieć, nie muszą wtedy zmieniać ani
/// jednej linii — a przy domyślnej polityce nic tędy i tak nie przechodzi.
pub type KasowanieSink = std::sync::mpsc::Sender<crate::client::Kasowanie>;

/// CO ILE PINGUJEMY TELEGRAMA.
///
/// `bot.py` robił to co 240 s i jego docstring nazywa problem po imieniu:
/// „Telegram potrafi PO CICHU przestac dostarczac update'y bezczynnym
/// klientom (najczestsza przyczyna »bot dziala, ale przestal reagowac na
/// sygnaly«)". Ping jest jedynym sposobem, żeby odróżnić ciszę na kanale
/// od martwego gniazda: martwe gniazdo NIE DAJE BŁĘDU samo z siebie.
const PING_CO: Duration = Duration::from_secs(240);

/// Twardy limit na odpowiedź. Bez niego zawieszone wywołanie MTProto
/// zamieniłoby keepalive w kolejne miejsce, które cicho wisi.
const PING_LIMIT: Duration = Duration::from_secs(30);

/// Ile nieudanych pingów z rzędu wymusza pełne ponowne połączenie.
/// Jeden może być czkawką sieci; trzy to martwe gniazdo.
const PING_BLEDOW_DO_RESTARTU: u32 = 3;

const STRUMIEN_BLEDOW_DO_ODBUDOWY: u32 = 3;

/// Podstawa przerwy po błędzie strumienia. Bez żadnej przerwy zerwane
/// połączenie zamieniłoby pętlę w zajętą karuzelę zżerającą rdzeń.
const STRUMIEN_PRZERWA: Duration = Duration::from_secs(2);

/// Sufit przerwy między próbami. Wyżej nie ma sensu: jeśli Telegram wróci,
/// chcemy to zauważyć w ciągu minuty, a nie po kwadransie.
const STRUMIEN_PRZERWA_MAX: Duration = Duration::from_secs(60);

/// Ile odczekać przed KOLEJNĄ odbudową, gdy poprzednia nie pomogła.
///
/// Bez tego przy trwałej awarii (np. Telegram niedostępny) bot logowałby się
/// od nowa co ~14 s bez końca — a to jest już dobijanie się do serwera, które
/// potrafi skończyć się ograniczeniem po ich stronie. Rośnie liniowo z liczbą
/// nieudanych odbudów pod rząd, do sufitu.
const ODBUDOWA_PRZERWA: Duration = Duration::from_secs(30);
const ODBUDOWA_PRZERWA_MAX: Duration = Duration::from_secs(300);

/// Przerwa po `n`-tym błędzie strumienia z rzędu: 2 s, 4 s, 8 s… do sufitu.
///
/// Wykładniczo, bo dwie sekundy przy trwałej awarii to 1 800 wpisów w dzienniku
/// na godzinę — dziennik przestaje wtedy być czytelny dokładnie wtedy, gdy jest
/// najbardziej potrzebny.
fn przerwa_po_bledzie(n: u32) -> Duration {
    let mnoznik = 1u32
        .checked_shl(n.saturating_sub(1).min(16))
        .unwrap_or(u32::MAX);
    STRUMIEN_PRZERWA
        .saturating_mul(mnoznik)
        .min(STRUMIEN_PRZERWA_MAX)
}

/// Przerwa przed `n`-tą odbudową z rzędu. Pierwsza idzie natychmiast.
fn przerwa_przed_odbudowa(nieudanych_pod_rzad: u32) -> Duration {
    ODBUDOWA_PRZERWA
        .saturating_mul(nieudanych_pod_rzad)
        .min(ODBUDOWA_PRZERWA_MAX)
}

fn klucz_zduplikowany(e: &str) -> bool {
    e.contains("AUTH_KEY_DUPLICATED")
}

/// FAKTY O ZDROWIU POŁĄCZENIA — bez ocen i bez decyzji.
///
/// Świadomie tylko liczby: `crates/telegram` nie zna poczty ani dziennika
/// i nie powinien. Alarmy składa warstwa aplikacji (`conduit-app`), która ma
/// mailer i wie, co użytkownik ustawił. Tutaj wyłącznie „co się stało i kiedy".
#[derive(Debug, Default)]
pub struct Zdrowie {
    /// kiedy ostatnio przyszła JAKAKOLWIEK wiadomość z obserwowanych źródeł
    ostatnia_wiadomosc_ms: AtomicI64,
    /// kiedy ostatni ping wrócił poprawnie
    ostatni_ping_ok_ms: AtomicI64,
    /// nieudane pingi Z RZĘDU (zerowane przy pierwszym udanym)
    bledy_pingu: AtomicU32,
    /// ile razy usługa musiała się przełączyć na nowo od startu procesu
    wznowienia: AtomicU32,
    /// błędy strumienia aktualizacji Z RZĘDU (zerowane przy pierwszej
    /// wiadomości, która przyszła po nich) — zadanie B12
    bledy_strumienia: AtomicU32,
    /// ile razy strumień trzeba było ODBUDOWAĆ od startu procesu
    odbudowy_strumienia: AtomicU32,
    /// łączna liczba aktualizacji, które nie doszły (patrz [`crate::luki`])
    zgubione_aktualizacje: AtomicU64,
}

/// Migawka [`Zdrowie`] do odczytu z zewnątrz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZdrowieMigawka {
    pub ostatnia_wiadomosc_ms: i64,
    pub ostatni_ping_ok_ms: i64,
    pub bledy_pingu: u32,
    pub wznowienia: u32,
    /// błędy strumienia POD RZĄD — odróżniają „cisza na kanale" od „strumień
    /// się wywraca i wciąż go podnosimy"
    pub bledy_strumienia: u32,
    /// ile razy strumień faktycznie ODBUDOWANO (nie: ile razy się wywrócił)
    pub odbudowy_strumienia: u32,
    pub zgubione_aktualizacje: u64,
}

impl Zdrowie {
    fn teraz_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
    fn wiadomosc(&self) {
        self.ostatnia_wiadomosc_ms
            .store(Self::teraz_ms(), Ordering::Relaxed);
        // Wiadomość, która doszła, jest DOWODEM, że strumień działa — mocniejszym
        // niż jakikolwiek ping. Seria błędów zostaje więc zamknięta tutaj.
        self.bledy_strumienia.store(0, Ordering::Relaxed);
    }
    /// Błąd `next_zdarzenie()`. Wraca, ile ich było POD RZĄD.
    fn blad_strumienia(&self) -> u32 {
        self.bledy_strumienia.fetch_add(1, Ordering::Relaxed) + 1
    }
    fn odbudowa_strumienia(&self) {
        self.odbudowy_strumienia.fetch_add(1, Ordering::Relaxed);
        self.bledy_strumienia.store(0, Ordering::Relaxed);
    }
    /// Podnosi łączny licznik zgubionych aktualizacji o `ile`.
    fn zgubione(&self, ile: u64) {
        self.zgubione_aktualizacje.fetch_add(ile, Ordering::Relaxed);
    }
    fn ping_ok(&self) {
        self.ostatni_ping_ok_ms
            .store(Self::teraz_ms(), Ordering::Relaxed);
        self.bledy_pingu.store(0, Ordering::Relaxed);
    }
    fn ping_zle(&self) -> u32 {
        self.bledy_pingu.fetch_add(1, Ordering::Relaxed) + 1
    }
    fn wznowienie(&self) {
        self.wznowienia.fetch_add(1, Ordering::Relaxed);
    }
    pub fn migawka(&self) -> ZdrowieMigawka {
        ZdrowieMigawka {
            ostatnia_wiadomosc_ms: self.ostatnia_wiadomosc_ms.load(Ordering::Relaxed),
            ostatni_ping_ok_ms: self.ostatni_ping_ok_ms.load(Ordering::Relaxed),
            bledy_pingu: self.bledy_pingu.load(Ordering::Relaxed),
            wznowienia: self.wznowienia.load(Ordering::Relaxed),
            bledy_strumienia: self.bledy_strumienia.load(Ordering::Relaxed),
            odbudowy_strumienia: self.odbudowy_strumienia.load(Ordering::Relaxed),
            zgubione_aktualizacje: self.zgubione_aktualizacje.load(Ordering::Relaxed),
        }
    }
}

pub struct TelegramService {
    history_slots: Arc<tokio::sync::Semaphore>,
    history_preview_slots: Arc<tokio::sync::Semaphore>,
    state: Arc<Mutex<AuthState>>,
    tx: mpsc::UnboundedSender<Cmd>,
    /// dokąd oddawać odebrane wiadomości; `None` = nikt nie słucha
    sink: Arc<Mutex<Option<MessageSink>>>,
    /// miniatury zdjęć profilowych — trzymane też tutaj, żeby dało się je
    /// oddać z dysku BEZ ruszania zadania w tle (a więc i bez czekania na nie)
    photos: Arc<PhotoCache>,
    peers: MapaPeerow,
    zdrowie: Arc<Zdrowie>,
    /// dokąd oddawać skasowania; `None` = nikt nie słucha
    kasowania: Arc<Mutex<Option<KasowanieSink>>>,
    /// oś B11 — trzymana TUTAJ, a nie w kliencie, bo klient ginie przy każdym
    /// ponownym logowaniu, a ustawienie ma przeżyć
    polityka_kasowania: Arc<Mutex<PolitykaKasowania>>,
}

impl std::fmt::Debug for TelegramService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramService")
            .field("etap", &self.state.lock().stage)
            .finish()
    }
}

impl TelegramService {
    /// Uruchamia usługę i zadanie w tle.
    ///
    /// Wraca NATYCHMIAST — automatyczne logowanie z zapisanej sesji dzieje się
    /// w tle, więc serwer HTTP wstaje, nawet gdy Telegram akurat nie odpowiada.
    /// Ekran logowania pokazuje wtedy „łączę", a nie zawieszoną stronę.
    pub fn start(ws: Workspace) -> Arc<Self> {
        let sekrety = ws.load_secrets().telegram;
        let poczatkowy = if sekrety.has_credentials() {
            AuthState {
                stage: AuthStage::Connecting,
                ..Default::default()
            }
        } else {
            AuthState::need_credentials()
        }
        .with_credentials(
            sekrety.api_id,
            sekrety.api_hash.as_str(),
            sekrety.has_session(),
        );

        let state = Arc::new(Mutex::new(poczatkowy));
        let (tx, rx) = mpsc::unbounded_channel();
        let sink: Arc<Mutex<Option<MessageSink>>> = Arc::new(Mutex::new(None));
        let photos = Arc::new(PhotoCache::new(&ws.root));
        let peers: MapaPeerow = Arc::new(Mutex::new(HashMap::new()));
        let zdrowie = Arc::new(Zdrowie::default());
        let kasowania: Arc<Mutex<Option<KasowanieSink>>> = Arc::new(Mutex::new(None));
        let polityka_kasowania = Arc::new(Mutex::new(PolitykaKasowania::default()));

        let worker = Worker {
            ws,
            state: Arc::clone(&state),
            client: None,
            login: None,
            qr_aktywny: false,
            sink: Arc::clone(&sink),
            photos: Arc::clone(&photos),
            peers: Arc::clone(&peers),
            zdrowie: Arc::clone(&zdrowie),
            kasowania: Arc::clone(&kasowania),
            polityka_kasowania: Arc::clone(&polityka_kasowania),
            odbudowy_pod_rzad: 0,
            zgubione_przepisane: 0,
        };
        tokio::spawn(worker.run(rx));

        Arc::new(TelegramService {
            history_slots: Arc::new(tokio::sync::Semaphore::new(1)),
            history_preview_slots: Arc::new(tokio::sync::Semaphore::new(1)),
            state,
            tx,
            sink,
            photos,
            peers,
            zdrowie,
            kasowania,
            polityka_kasowania,
        })
    }

    /// Fakty o zdrowiu połączenia — do wykrycia „bot pokazuje zielono,
    /// a sygnały nie przychodzą". Patrz [`Zdrowie`].
    pub fn zdrowie(&self) -> ZdrowieMigawka {
        self.zdrowie.migawka()
    }

    /// Fetch through the existing authenticated connection only. Does not log in,
    /// refresh credentials, publish messages, mark them read, or reserve IDs.
    pub async fn fetch_history_snapshot(
        &self,
        request: HistoryFetchRequest,
    ) -> Result<HistorySnapshot, String> {
        request.validate()?;
        let permit = self
            .history_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| "history preview already running".to_string())?;
        let deadline =
            tokio::time::Instant::now() + Duration::from_millis(request.total_timeout_ms);
        let (reply, receiver) = oneshot::channel();
        self.tx
            .send(Cmd::HistoryPreview {
                request,
                deadline,
                permit,
                reply,
            })
            .map_err(|_| "Telegram worker unavailable".to_string())?;
        // Fetch itself returns partial data by deadline; extra grace permits delivery.
        tokio::time::timeout_at(deadline + Duration::from_secs(2), receiver)
            .await
            .map_err(|_| "Telegram history request timed out in worker queue".to_string())?
            .map_err(|_| "Telegram history worker ended".to_string())?
    }

    /// Podłącza odbiorcę wiadomości z kanałów.
    ///
    /// Do wywołania RAZ, przy starcie pętli handlowej. Dopóki nikt nie
    /// zaprenumeruje, klient nie czyta strumienia aktualizacji — i słusznie:
    /// bez odbiorcy wiadomości i tak nie byłoby dokąd oddać.
    pub fn subscribe(&self, tx: MessageSink) {
        *self.sink.lock() = Some(tx);
    }

    /// Podłącza odbiorcę SKASOWANYCH wiadomości (zadanie B11).
    ///
    /// Bez prenumeraty i przy domyślnej polityce skasowania nie opuszczają
    /// nawet strumienia — dlatego samo istnienie tej metody niczego nie zmienia.
    pub fn subscribe_kasowania(&self, tx: KasowanieSink) {
        *self.kasowania.lock() = Some(tx);
    }

    /// Ustawia oś B11. Zmiana działa OD RAZU, także na już podniesionym kliencie.
    pub fn set_polityka_kasowania(&self, p: PolitykaKasowania) {
        *self.polityka_kasowania.lock() = p;
        // Klient trzyma własną kopię, żeby nie sięgać po zamek na każdej
        // aktualizacji; przy zmianie trzeba go dogonić.
        if self.tx.send(Cmd::PolitykaKasowania(p)).is_err() {
            warn!("Telegram: zadanie w tle nie działa — polityka kasowania nie została przekazana");
        }
    }

    pub fn polityka_kasowania(&self) -> PolitykaKasowania {
        *self.polityka_kasowania.lock()
    }

    /// Wysyła polecenie i czeka na wynik.
    ///
    /// BLOKUJE wątek wołającego — dlatego uchwyty HTTP wołają metody tej cechy
    /// wewnątrz `spawn_blocking`. Alternatywa (odpowiedź „przyjęto", wynik
    /// dopiero w kolejnym odpytaniu) sprawia, że formularz hasła nie umie
    /// powiedzieć „hasło nieprawidłowe", tylko miga stanem.
    fn zawolaj(&self, buduj: impl FnOnce(Odpowiedz) -> Cmd) -> anyhow::Result<AuthState> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(buduj(tx))
            .map_err(|_| anyhow::anyhow!("zadanie logowania do Telegrama nie działa"))?;
        match rx.blocking_recv() {
            Ok(Ok(s)) => Ok(s),
            Ok(Err(e)) => Err(anyhow::anyhow!(e)),
            Err(_) => Err(anyhow::anyhow!(
                "zadanie logowania przerwane bez odpowiedzi"
            )),
        }
    }

    /// Wysyła powiadomienie na czat. WRACA NATYCHMIAST.
    ///
    /// Wołane z pętli silnika przy każdym zdarzeniu bota, więc nie wolno tu
    /// niczego czekać — polecenie ląduje w skrzynce zadania w tle, a błąd
    /// wysyłki idzie do dziennika. Powiadomienie nie ma prawa opóźnić handlu
    /// ani go zatrzymać.
    pub fn powiadom(&self, chat_id: i64, topic_id: Option<i64>, text: String) {
        if self
            .tx
            .send(Cmd::Powiadom {
                chat_id,
                topic_id,
                text,
            })
            .is_err()
        {
            tracing::error!(
                chat_id,
                "zadanie Telegrama nie działa — powiadomienie przepadło"
            );
        }
    }

    /// Czy da się DZIŚ wysłać cokolwiek na ten czat.
    ///
    /// Wysyłka jest z natury „wyślij i zapomnij" — `Cmd::Powiadom` nie ma jak
    /// odesłać wyniku, bo nie wolno mu opóźniać odbioru sygnałów. Skutkiem
    /// ubocznym było ciche gubienie: gdy czatu nie ma w mapie `peers`,
    /// powiadomienie znika z samym wpisem w logu technicznym, a przycisk
    /// „Wyślij test" i tak melduje sukces.
    ///
    /// Mapa `peers` zapełnia się dopiero po pobraniu listy dialogów, więc
    /// ZARAZ PO STARCIE procesu nie ma w niej nic. To najczęstszy moment,
    /// w którym użytkownik naciska „Wyślij test" — i dostaje zieloną chmurkę
    /// za wiadomość, która nigdy nie wyszła.
    ///
    /// Ta metoda nie wysyła niczego i nie czeka: czyta tę samą mapę, z której
    /// korzysta wysyłka, więc odpowiada dokładnie na pytanie „czy to przejdzie".
    pub fn czy_znany_czat(&self, chat_id: i64) -> bool {
        self.peers.lock().contains_key(&chat_id)
    }
}

impl TelegramAuth for TelegramService {
    fn history_import_preview(
        &self,
        request: HistoryFetchRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<conduit_server::history_import::HistoryPreviewResponse, String>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            let permit = self
                .history_preview_slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| "history preview validation already running".to_string())?;
            let snapshot = self.fetch_history_snapshot(request).await?;
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                crate::history_adapter::preview(snapshot)
            })
            .await
            .map_err(|_| "history parser preview task ended".to_string())
        })
    }
    fn state(&self) -> AuthState {
        self.state.lock().clone()
    }

    fn has_credentials(&self) -> bool {
        let s = self.state.lock();
        s.api_hash_set && s.api_id.is_some()
    }

    fn set_credentials(&self, api_id: i32, api_hash: &str) -> anyhow::Result<AuthState> {
        self.zawolaj(|reply| Cmd::SetCredentials {
            api_id,
            api_hash: api_hash.trim().to_string(),
            reply,
        })
    }

    fn start_qr(&self) -> anyhow::Result<AuthState> {
        self.zawolaj(|reply| Cmd::StartQr { reply })
    }

    fn submit_password(&self, password: &str) -> anyhow::Result<AuthState> {
        self.zawolaj(|reply| Cmd::Password {
            password: password.to_string(),
            reply,
        })
    }

    fn logout(&self) -> AuthState {
        self.zawolaj(|reply| Cmd::Logout {
            forget: false,
            reply,
        })
        .unwrap_or_else(|e| AuthState::error(e.to_string()))
    }

    fn forget_credentials(&self) -> AuthState {
        self.zawolaj(|reply| Cmd::Logout {
            forget: true,
            reply,
        })
        .unwrap_or_else(|e| AuthState::error(e.to_string()))
    }

    /// Lista czatów konta. BLOKUJE tak samo jak reszta metod tej cechy —
    /// uchwyt HTTP woła ją w `spawn_blocking`.
    fn list_channels(&self) -> anyhow::Result<Vec<ChannelInfo>> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Cmd::Dialogs { reply: tx })
            .map_err(|_| anyhow::anyhow!("zadanie logowania do Telegrama nie działa"))?;
        match rx.blocking_recv() {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(anyhow::anyhow!(e)),
            Err(_) => Err(anyhow::anyhow!(
                "pobranie listy czatów przerwane bez odpowiedzi"
            )),
        }
    }

    /// Miniatura zdjęcia profilowego czatu.
    ///
    /// DWIE DROGI, i to celowo. Gdy plik już leży w pamięci podręcznej
    /// w aktualnej wersji, oddajemy go OD RAZU z tego wątku — bez wysyłania
    /// polecenia do zadania w tle. Dzięki temu 200 kafelków listy odświeżonej
    /// po raz drugi nie generuje ani jednego przełączenia kontekstu w pętli
    /// odbioru wiadomości, a tym bardziej ani jednego wywołania MTProto.
    /// Dopiero brak pliku (albo zmiana zdjęcia) schodzi do Telegrama.
    fn channel_photo(&self, chat_id: i64) -> anyhow::Result<Option<PathBuf>> {
        let znany = self.peers.lock().get(&chat_id).cloned();
        if let Some(p) = &znany {
            // Czat bez zdjęcia — odpowiedź bez sieci i bez dysku.
            if p.photo_id.is_none() {
                return Ok(None);
            }
            let sciezka = self.photos.path(chat_id);
            if self.photos.aktualny(chat_id, p.photo_id) {
                return Ok(Some(sciezka));
            }
        }

        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Cmd::Photo { chat_id, reply: tx })
            .map_err(|_| anyhow::anyhow!("zadanie logowania do Telegrama nie działa"))?;
        match rx.blocking_recv() {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(anyhow::anyhow!(e)),
            Err(_) => Err(anyhow::anyhow!(
                "pobranie miniatury przerwane bez odpowiedzi"
            )),
        }
    }
}

// ============================================================
//  ZADANIE W TLE
// ============================================================

struct Worker {
    ws: Workspace,
    state: Arc<Mutex<AuthState>>,
    client: Option<TelegramClient>,
    login: Option<QrLogin>,
    qr_aktywny: bool,
    sink: Arc<Mutex<Option<MessageSink>>>,
    photos: Arc<PhotoCache>,
    peers: MapaPeerow,
    zdrowie: Arc<Zdrowie>,
    kasowania: Arc<Mutex<Option<KasowanieSink>>>,
    polityka_kasowania: Arc<Mutex<PolitykaKasowania>>,
    /// ile odbudów strumienia z rzędu NIE POMOGŁO — steruje przerwą przed
    /// kolejną, żeby trwała awaria nie zamieniła się w dobijanie się do serwera
    odbudowy_pod_rzad: u32,
    /// ile zgubionych aktualizacji BIEŻĄCEGO klienta już przepisaliśmy do
    /// zdrowia; zerowane razem z klientem, bo jego licznik startuje od nowa
    zgubione_przepisane: u64,
}

impl Worker {
    fn sciezka_sesji(&self) -> PathBuf {
        self.ws.root.join(PLIK_SESJI)
    }

    /// Zapamiętuje, jak dosięgnąć zdjęcia każdego czatu.
    ///
    /// Wołane po KAŻDYM pobraniu listy dialogów — także tym po zalogowaniu,
    /// które użytkownika nie interesuje. Bez tego pierwsze wejście na ekran
    /// kanałów nie wiedziałoby, o które zdjęcia prosić.
    fn zapamietaj_peery(&self, dialogi: &[DialogEntry]) {
        let mut m = self.peers.lock();
        for d in dialogi {
            m.insert(
                d.chat_id,
                PeerFoto {
                    peer: d.peer,
                    photo_id: d.photo_id,
                    is_forum: d.is_forum,
                },
            );
        }
    }

    /// Nakłada opis poświadczeń na stan i publikuje go.
    fn ustaw(&self, mut s: AuthState) -> AuthState {
        let sek = self.ws.load_secrets().telegram;
        s = s.with_credentials(sek.api_id, sek.api_hash.as_str(), sek.has_session());
        *self.state.lock() = s.clone();
        s
    }

    /// Nieudane polecenie logowania musi **przestawić stan widziany przez
    /// interfejs**, a nie tylko wrócić błędem do jednego żądania HTTP.
    ///
    /// Bez tego było tak: `api_id` z literówką → `polacz()` przechodzi (błąd
    /// `API_ID_INVALID` pada dopiero przy `auth.exportLoginToken`), więc stan
    /// zostaje na `connecting`, a ekran logowania kręci kółkiem BEZ KOŃCA.
    /// Treść błędu widział wyłącznie ten, kto patrzył na odpowiedź POST-a.
    fn oglos(&self, r: Result<AuthState, String>) -> Result<AuthState, String> {
        if let Err(e) = &r {
            self.ustaw(AuthState::error(e.clone()));
        }
        r
    }

    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Cmd>) {
        // Automatyczne logowanie z zapisanej sesji — cel całego zadania 1.
        self.wznow_sesje().await;

        let mut nastepny_ping = tokio::time::Instant::now() + PING_CO;

        loop {
            // Gdy kod QR jest aktywny, musimy JEDNOCZEŚNIE czekać na skan
            // i na polecenia z interfejsu. Bez tego „Wygeneruj nowy kod"
            // działałoby dopiero po wygaśnięciu bieżącego tokenu.
            let cmd = if self.qr_aktywny && self.client.is_some() && self.login.is_some() {
                let wynik = {
                    let client = self.client.as_mut().expect("sprawdzone wyżej");
                    let login = self.login.as_mut().expect("sprawdzone wyżej");
                    tokio::select! {
                        etap = login.step(client.updates_mut()) => Krok::Etap(etap),
                        c = rx.recv() => Krok::Cmd(c),
                    }
                };
                match wynik {
                    Krok::Etap(e) => {
                        self.po_etapie_wynik(e).await;
                        continue;
                    }
                    Krok::Cmd(c) => c,
                }
            } else if self.mozna_odbierac() {
                // ZALOGOWANI I JEST KOMU ODDAĆ — czytamy strumień wiadomości
                // równolegle z poleceniami z interfejsu. To jest pętla, przez
                // którą sygnał z kanału trafia do silnika.
                //
                // TRZECIA GAŁĄŹ TO KEEPALIVE i to jest tu rzecz najważniejsza.
                // `next_message()` na martwym gnieździe NIE ZWRACA BŁĘDU — po
                // prostu nigdy nic nie oddaje. Bez własnego budzika pętla
                // czekałaby w nieskończoność, panel pokazywałby „połączony",
                // a sygnały nie przychodziłyby do rana. Budzik daje nam moment,
                // w którym możemy zapytać Telegrama, czy jeszcze z nami rozmawia.
                let wynik = {
                    let client = self.client.as_mut().expect("sprawdzone w mozna_odbierac");
                    tokio::select! {
                        m = client.next_zdarzenie() => Odbior::Zdarzenie(m),
                        c = rx.recv() => Odbior::Cmd(c),
                        _ = tokio::time::sleep_until(nastepny_ping) => Odbior::Budzik,
                    }
                };
                match wynik {
                    Odbior::Zdarzenie(Ok(Zdarzenie::Wiadomosc(m))) => {
                        self.zdrowie.wiadomosc();
                        // Wiadomość, która doszła, jest dowodem, że odbudowa
                        // pomogła — dopiero teraz wolno wrócić do krótkich
                        // przerw przy następnej awarii.
                        self.odbudowy_pod_rzad = 0;
                        self.przepisz_luki();
                        self.oddaj(m);
                        continue;
                    }
                    Odbior::Zdarzenie(Ok(Zdarzenie::Kasowanie(k))) => {
                        self.zdrowie.wiadomosc();
                        self.odbudowy_pod_rzad = 0;
                        self.przepisz_luki();
                        self.oddaj_kasowanie(k);
                        continue;
                    }
                    Odbior::Zdarzenie(Err(e)) => {
                        self.po_bledzie_strumienia(e.to_string()).await;
                        continue;
                    }
                    Odbior::Budzik => {
                        // Także tutaj, nie tylko przy wiadomości: luka mogła
                        // zjeść WSZYSTKIE aktualizacje z obserwowanych źródeł,
                        // a wtedy nic by tego licznika nie przepisało i panel
                        // pokazywałby zero przy realnie zgubionych sygnałach.
                        self.przepisz_luki();
                        self.ping().await;
                        nastepny_ping = tokio::time::Instant::now() + PING_CO;
                        continue;
                    }
                    Odbior::Cmd(c) => c,
                }
            } else {
                rx.recv().await
            };

            let Some(cmd) = cmd else {
                info!("Telegram: zadanie logowania kończy pracę");
                return;
            };
            self.obsluz(cmd).await;
        }
    }

    /// KEEPALIVE — jedyny sposób, żeby odróżnić ciszę na kanale od martwego gniazda.
    ///
    /// Problem, który to zamyka, `bot.py` nazywa po imieniu w docstringu
    /// `tg_keepalive_loop`: *„Telegram potrafi PO CICHU przestac dostarczac
    /// update'y bezczynnym klientom (najczestsza przyczyna »bot dziala, ale
    /// przestal reagowac na sygnaly«)"*. Gniazdo jest otwarte, biblioteka nie
    /// zgłasza błędu, `next_message()` po prostu nigdy nic nie oddaje.
    ///
    /// `is_authorized()` to prawdziwe wywołanie MTProto, więc przechodzi całą
    /// drogę do serwera i z powrotem — a to jest dokładnie to, co chcemy
    /// sprawdzić. Odpowiedź „nie" jest równie użyteczna jak błąd: obie znaczą,
    /// że tą sesją nie odbierzemy już żadnego sygnału.
    ///
    /// Po `PING_BLEDOW_DO_RESTARTU` nieudanych pingach z rzędu zamykamy
    /// klienta i logujemy się od nowa z zapisanej sesji. Jedna czkawka sieci
    /// nie jest tego warta, trzy z rzędu już tak.
    async fn ping(&mut self) {
        let Some(client) = self.client.as_ref() else {
            return;
        };
        let wynik = tokio::time::timeout(PING_LIMIT, client.is_authorized()).await;

        let blad = match wynik {
            Ok(Ok(true)) => {
                self.zdrowie.ping_ok();
                info!("Telegram: keepalive OK — sesja odpowiada");
                return;
            }
            Ok(Ok(false)) => "sesja przestała być autoryzowana".to_string(),
            Ok(Err(e)) => format!("{e}"),
            Err(_) => format!("brak odpowiedzi w {} s", PING_LIMIT.as_secs()),
        };

        let ile = self.zdrowie.ping_zle();
        warn!(blad = %blad, proba = ile, "Telegram: keepalive NIEUDANY");

        // ZDUPLIKOWANY KLUCZ: ponowne logowanie NIE POMOŻE i tylko zaciemnia
        // dziennik (patrz `klucz_zduplikowany`). Zatrzymujemy się od razu przy
        // PIERWSZYM takim błędzie i mówimy wprost, co użytkownik ma zrobić.
        if klucz_zduplikowany(&blad) {
            warn!(
                "Telegram: TA SAMA SESJA DZIAŁA W DWÓCH MIEJSCACH — serwer unieważnił klucz. \
                 Ponowne logowanie tego nie naprawi."
            );
            self.zamknij_klienta().await;
            self.ustaw(AuthState {
                stage: AuthStage::LoggedOut,
                error: Some(
                    "Ta sama sesja Telegrama działa na innej maszynie (AUTH_KEY_DUPLICATED). \
                     Najczęstsza przyczyna: katalog PACKAGE skopiowany na VPS RAZEM z plikiem \
                     telegram.session, przy włączonej drugiej instancji bota. \
                     Serwer unieważnił klucz OBU stronom. \
                     Napraw tak: wyłącz jedną instancję, skasuj tam telegram.session, \
                     a na maszynie docelowej zaloguj się kodem QR od nowa. \
                     Nigdy nie uruchamiaj dwóch botów na jednej sesji."
                        .into(),
                ),
                ..Default::default()
            });
            return;
        }

        if ile < PING_BLEDOW_DO_RESTARTU {
            return;
        }
        warn!("Telegram: {ile} nieudanych pingów z rzędu — zamykam sesję i loguję się od nowa");
        self.zdrowie.wznowienie();
        self.zamknij_klienta().await;
        self.wznow_sesje().await;
    }

    /// Każde polecenie ma TWARDY limit czasu.
    ///
    /// Bez niego zawieszone wywołanie MTProto (a takie się zdarzają, gdy
    /// datacentrum nie odpowiada) trzymałoby wątek uchwytu HTTP w nieskończoność
    /// i cały ekran logowania wyglądałby na zepsuty. Z limitem użytkownik
    /// dostaje komunikat i przycisk „Wygeneruj nowy kod".
    async fn obsluz(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::HistoryPreview {
                request,
                deadline,
                permit,
                mut reply,
            } => {
                if reply.is_closed() {
                    return;
                }
                if self.state.lock().stage != AuthStage::LoggedIn {
                    let _ = reply.send(Err(
                        "Telegram is not logged in; preview does not create a session".into(),
                    ));
                    return;
                }
                let peer = self.peers.lock().get(&request.chat_id).cloned();
                let client = self.client.as_ref().map(|c| c.raw().clone());
                let (Some(peer), Some(client)) = (peer, client) else {
                    let _ = reply.send(Err(
                        "Telegram peer is not cached in the existing session".into()
                    ));
                    return;
                };
                if request.topic_id.is_some() && !peer.is_forum {
                    let _ = reply.send(Err("topic requested for a non-forum chat".into()));
                    return;
                }
                tokio::spawn(async move {
                    let _permit = permit;
                    let mut source = TelegramHistorySource {
                        client,
                        peer: peer.peer,
                        chat_is_forum: peer.is_forum,
                    };
                    tokio::select! {
                        _ = reply.closed() => {},
                        result = crate::history_fetch::fetch_with_source(&mut source, request, deadline) => {
                            let _ = reply.send(Ok(result));
                        }
                    }
                });
            }
            Cmd::SetCredentials {
                api_id,
                api_hash,
                reply,
            } => {
                let wynik = tokio::time::timeout(
                    LIMIT_ODPOWIEDZI,
                    self.zapisz_poswiadczenia(api_id, &api_hash),
                )
                .await;
                let _ = reply.send(self.oglos(splasz(wynik, "zapis poświadczeń")));
            }
            Cmd::StartQr { reply } => {
                let wynik = tokio::time::timeout(LIMIT_ODPOWIEDZI, self.start_qr()).await;
                let _ = reply.send(self.oglos(splasz(wynik, "generowanie kodu QR")));
            }
            Cmd::Password { password, reply } => {
                let wynik = tokio::time::timeout(LIMIT_ODPOWIEDZI, self.haslo(&password)).await;
                let _ = reply.send(self.oglos(splasz(wynik, "weryfikacja hasła")));
            }
            Cmd::Logout { forget, reply } => {
                let wynik = tokio::time::timeout(LIMIT_ODPOWIEDZI, self.wyloguj(forget)).await;
                let _ = reply.send(match wynik {
                    Ok(s) => Ok(s),
                    Err(_) => Ok(AuthState::logged_out()),
                });
            }
            Cmd::Dialogs { reply } => {
                // Konto z tysiącami rozmów pobiera się długo, a tematy forum
                // to osobne wywołanie na każdy czat — stąd własny, szerszy
                // limit czasu niż przy logowaniu.
                let wynik =
                    tokio::time::timeout(Duration::from_secs(90), self.lista_czatow()).await;
                let _ = reply.send(match wynik {
                    Ok(Ok(v)) => Ok(v),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err("Telegram nie oddał listy czatów w ciągu 90 s".to_string()),
                });
            }
            Cmd::Photo { chat_id, reply } => {
                // ŚWIADOMIE bez `await` w pętli: pobranie miniatury to osobne
                // zadanie. Gdyby szło tędy, jedna wolna odpowiedź Telegrama
                // zatrzymałaby ODBIÓR SYGNAŁÓW — a to jest jedyna rzecz, która
                // w tym programie naprawdę nie może czekać na obrazek.
                let znany = self.peers.lock().get(&chat_id).cloned();
                let Some(dane) = znany else {
                    let _ = reply.send(Ok(None));
                    return;
                };
                let Some(client) = self.client.as_ref().map(|c| c.raw().clone()) else {
                    let _ = reply.send(Ok(None));
                    return;
                };
                let cache = Arc::clone(&self.photos);
                tokio::spawn(async move {
                    let wynik = tokio::time::timeout(
                        Duration::from_secs(30),
                        cache.ensure(&client, chat_id, dane.peer, dane.photo_id),
                    )
                    .await;
                    let _ = reply.send(match wynik {
                        Ok(Ok(v)) => Ok(v),
                        Ok(Err(e)) => Err(e.to_string()),
                        Err(_) => Err("Telegram nie oddał miniatury w ciągu 30 s".to_string()),
                    });
                });
            }
            Cmd::PolitykaKasowania(p) => {
                *self.polityka_kasowania.lock() = p;
                if let Some(c) = self.client.as_ref() {
                    c.set_polityka_kasowania(p);
                }
                info!(polityka = ?p, "Telegram: zmieniono politykę reakcji na kasowanie wiadomości");
            }
            Cmd::Powiadom {
                chat_id,
                topic_id,
                text,
            } => {
                // Ta sama zasada co przy miniaturze: ŻADNEGO `await` w pętli.
                // Powiadomienie jest najmniej pilną rzeczą w tym programie
                // i nie ma prawa opóźnić odbioru sygnału.
                let znany = self.peers.lock().get(&chat_id).cloned();
                let klient = self.client.as_ref().map(|c| c.raw().clone());
                let (Some(dane), Some(client)) = (znany, klient) else {
                    tracing::warn!(
                        chat_id,
                        "powiadomienie pominięte: czat nieznany sesji albo brak klienta"
                    );
                    return;
                };
                tokio::spawn(async move {
                    let msg = grammers_client::message::InputMessage::new()
                        .text(text)
                        // przynależność do tematu wyraża się tym samym polem
                        // co odpowiedź — bez tego trafi do „Ogólnego"
                        .reply_to(topic_id.map(|t| t as i32));
                    match tokio::time::timeout(
                        Duration::from_secs(30),
                        client.send_message(dane.peer, msg),
                    )
                    .await
                    {
                        Ok(Ok(_)) => {}
                        Ok(Err(e)) => tracing::error!(
                            blad = %e, chat_id,
                            "nie udało się wysłać powiadomienia na Telegram"
                        ),
                        Err(_) => tracing::error!(
                            chat_id,
                            "Telegram nie przyjął powiadomienia w ciągu 30 s"
                        ),
                    }
                });
            }
        }
    }

    /// Pobiera czaty konta wraz z tematami forum.
    ///
    /// Świadomie NIE cache'ujemy wyniku: użytkownik otwiera ten ekran wtedy,
    /// gdy właśnie dołączył do nowego kanału, a lista sprzed godziny by go
    /// nie zawierała. Wywołanie i tak odświeża cache peerów, którego
    /// potrzebuje strumień aktualizacji.
    async fn lista_czatow(&mut self) -> anyhow::Result<Vec<ChannelInfo>> {
        self.polacz().await?;
        let Some(c) = self.client.as_ref() else {
            anyhow::bail!("klient Telegrama nie działa");
        };
        anyhow::ensure!(
            self.state.lock().stage == AuthStage::LoggedIn,
            "nie jesteś zalogowany do Telegrama — zeskanuj kod QR"
        );

        let dialogi = c.refresh_dialogs(500).await?;
        self.zapamietaj_peery(&dialogi);
        let mut out = Vec::with_capacity(dialogi.len());
        for d in &dialogi {
            let tematy = if d.is_forum {
                match c.topics_of(d).await {
                    Ok(t) => t
                        .into_iter()
                        .map(|x| ChannelTopic {
                            id: x.topic_id,
                            title: x.title,
                            closed: x.closed,
                        })
                        .collect(),
                    Err(e) => {
                        // Brak tematów nie może wyrzucić z listy całego czatu —
                        // bez niego użytkownik nie zaznaczy nawet samej grupy.
                        warn!(czat = d.chat_id, %e, "Telegram: nie udało się pobrać tematów");
                        Vec::new()
                    }
                }
            } else {
                Vec::new()
            };
            out.push(ChannelInfo {
                id: d.chat_id,
                name: d.name.clone(),
                handle: d.username.clone(),
                kind: match d.kind {
                    crate::dialogs::DialogKind::User => "user",
                    crate::dialogs::DialogKind::Group => "group",
                    crate::dialogs::DialogKind::Channel => "channel",
                }
                .to_string(),
                is_forum: d.is_forum,
                // Sama INFORMACJA, czy zdjęcie istnieje — plik pobierze się
                // dopiero wtedy, gdy przeglądarka o niego poprosi.
                has_photo: d.photo_id.is_some(),
                photo_version: d.photo_id.map(|p| p.to_string()),
                topics: tematy,
            });
        }
        Ok(out)
    }

    // ---------- podłączenie ----------

    /// Podnosi klienta na zapisanych poświadczeniach.
    async fn polacz(&mut self) -> anyhow::Result<()> {
        if self.client.is_some() {
            return Ok(());
        }
        let sek = self.ws.load_secrets().telegram;
        anyhow::ensure!(
            sek.has_credentials(),
            "brak api_id/api_hash — pobierz je z https://my.telegram.org"
        );

        let sciezka = self.sciezka_sesji();
        if !sciezka.exists() && sek.has_session() {
            match FileSession::restore_string_session(&sciezka, sek.session_string.as_str()) {
                Ok(()) => info!("Telegram: sesja odtworzona z secrets.json"),
                Err(e) => warn!(%e, "Telegram: zapisany łańcuch sesji jest nieczytelny"),
            }
        }

        let cfg = ClientConfig {
            api_id: sek.api_id,
            api_hash: sek.api_hash.as_str().to_string(),
            session_path: sciezka,
            ..Default::default()
        };
        let c = TelegramClient::connect(cfg).await?;
        // Oś B11 mieszka w usłudze, bo klient ginie przy każdym ponownym
        // logowaniu. Bez tej linii ustawienie z panelu przeżyłoby dokładnie do
        // pierwszej odbudowy strumienia i cicho wróciło do domyślnego.
        c.set_polityka_kasowania(*self.polityka_kasowania.lock());
        self.client = Some(c);
        // Nowy klient = nowy licznik luk, więc punkt odniesienia wraca na zero.
        // Suma w `Zdrowie` zostaje — ona liczy OD STARTU PROCESU.
        self.zgubione_przepisane = 0;
        Ok(())
    }

    /// Automatyczne logowanie przy starcie programu.
    async fn wznow_sesje(&mut self) {
        let sek = self.ws.load_secrets().telegram;
        if !sek.has_credentials() {
            self.ustaw(AuthState::need_credentials());
            return;
        }
        self.ustaw(AuthState {
            stage: AuthStage::Connecting,
            ..Default::default()
        });

        if let Err(e) = self.polacz().await {
            warn!(%e, "Telegram: nie udało się podnieść klienta");
            self.ustaw(AuthState::error(format!(
                "Nie udało się połączyć z Telegramem: {e}"
            )));
            return;
        }

        let zalogowany = match self
            .client
            .as_ref()
            .expect("klient jest")
            .is_authorized()
            .await
        {
            Ok(v) => v,
            Err(e) => {
                // Ten sam przypadek co przy keepalive: kolejne próby nic nie dadzą,
                // a użytkownik dostawał mylące „zeskanuj kod QR jeszcze raz”,
                // choć kod QR problemu nie rozwiązuje.
                if klucz_zduplikowany(&e.to_string()) {
                    warn!("Telegram: AUTH_KEY_DUPLICATED przy sprawdzaniu autoryzacji");
                    self.zamknij_klienta().await;
                    self.ustaw(AuthState {
                        stage: AuthStage::LoggedOut,
                        error: Some(
                            "Ta sama sesja Telegrama działa na innej maszynie \
                             (AUTH_KEY_DUPLICATED). Wyłącz drugą instancję bota, skasuj tam \
                             telegram.session, a tutaj zaloguj się kodem QR od nowa. \
                             Kopiowanie PACKAGE między maszynami MUSI pomijać telegram.session."
                                .into(),
                        ),
                        ..Default::default()
                    });
                    return;
                }
                warn!(%e, "Telegram: nie udało się sprawdzić autoryzacji");
                false
            }
        };

        if zalogowany {
            let kto = if sek.user_name.is_empty() {
                "konto Telegram".to_string()
            } else {
                sek.user_name.clone()
            };
            info!(uzytkownik = %kto, "Telegram: zalogowano z zapisanej sesji — bez kodu QR");
            self.po_zalogowaniu(&kto, None).await;
        } else if sek.has_session() {
            // Sesja była, ale serwer jej nie uznaje: użytkownik zakończył ją
            // z telefonu („Urządzenia → Zakończ sesję"). Mówimy to wprost,
            // zamiast pokazywać kod QR bez wyjaśnienia.
            info!("Telegram: zapisana sesja już nie obowiązuje — potrzebne ponowne logowanie");
            self.ustaw(AuthState {
                stage: AuthStage::LoggedOut,
                error: Some(
                    "Zapisana sesja wygasła albo została zakończona z telefonu. \
                     Zeskanuj kod QR jeszcze raz."
                        .into(),
                ),
                ..Default::default()
            });
        } else {
            self.ustaw(AuthState::logged_out());
        }
    }

    // ---------- logowanie kodem QR ----------

    async fn zapisz_poswiadczenia(
        &mut self,
        api_id: i32,
        api_hash: &str,
    ) -> anyhow::Result<AuthState> {
        anyhow::ensure!(api_id > 0, "api_id musi być liczbą dodatnią");
        anyhow::ensure!(!api_hash.is_empty(), "api_hash nie może być pusty");

        // Zmiana poświadczeń unieważnia poprzednią sesję: klucz autoryzacyjny
        // jest związany z aplikacją, więc sesja spod innego api_id jest
        // bezużyteczna i tylko myliłaby diagnostykę.
        let stare = self.ws.load_secrets().telegram;
        let inne = stare.api_id != api_id || stare.api_hash.as_str() != api_hash;

        let mut sek = self.ws.load_secrets();
        sek.telegram.api_id = api_id;
        sek.telegram.api_hash = api_hash.into();
        if inne {
            sek.telegram.session_string = Default::default();
            sek.telegram.user_id = 0;
            sek.telegram.user_name.clear();
            sek.telegram.handle.clear();
        }
        self.ws.save_secrets(&sek)?;

        if inne {
            self.zamknij_klienta().await;
            let _ = std::fs::remove_file(self.sciezka_sesji());
        }

        self.ustaw(AuthState {
            stage: AuthStage::Connecting,
            ..Default::default()
        });
        self.polacz().await.map_err(|e| {
            // Najczęstsza przyczyna: przestawione api_id i api_hash albo
            // literówka w haszu. Mówimy to, zamiast oddawać surowy błąd MTProto.
            self.ustaw(AuthState::error(format!("{e}")));
            anyhow::anyhow!(
                "Telegram odrzucił poświadczenia ({e}). Sprawdź, czy api_id i api_hash \
                 pochodzą z tej samej aplikacji na my.telegram.org."
            )
        })?;

        // Poświadczenia przyjęte — od razu generujemy pierwszy kod.
        self.start_qr().await
    }

    async fn start_qr(&mut self) -> anyhow::Result<AuthState> {
        self.polacz().await?;
        let client = self.client.as_ref().expect("klient jest po polacz()");

        if client.is_authorized().await.unwrap_or(false) {
            let kto = self.ws.load_secrets().telegram.user_name;
            return Ok(self.po_zalogowaniu(&kto, None).await);
        }

        let mut login = client.qr_login();
        let etap = login.start().await?;
        self.login = Some(login);
        self.qr_aktywny = true;
        Ok(self.po_etapie(etap).await)
    }

    async fn haslo(&mut self, password: &str) -> anyhow::Result<AuthState> {
        let Some(login) = self.login.as_mut() else {
            anyhow::bail!("nie ma aktywnego logowania — wygeneruj kod QR jeszcze raz");
        };
        let etap = login.submit_password(password).await?;
        Ok(self.po_etapie(etap).await)
    }

    /// Zamienia etap z klienta MTProto na stan widziany przez interfejs.
    async fn po_etapie(&mut self, etap: LoginStage) -> AuthState {
        match etap {
            LoginStage::Qr(p) => {
                let s = AuthState {
                    stage: AuthStage::WaitingScan,
                    qr_svg: Some(p.qr.to_svg(QR_MODUL_PX)),
                    token_url: Some(p.url),
                    expires_at: Some(conduit_server::now_ms() + p.expires_in.as_millis() as i64),
                    ..Default::default()
                };
                self.ustaw(s)
            }
            LoginStage::PasswordRequired { hint } => {
                self.qr_aktywny = false;
                self.ustaw(AuthState {
                    stage: AuthStage::WaitingPassword,
                    password_hint: hint,
                    ..Default::default()
                })
            }
            LoginStage::PasswordWrong { hint } => {
                self.qr_aktywny = false;
                self.ustaw(AuthState {
                    stage: AuthStage::WaitingPassword,
                    password_hint: hint,
                    error: Some("Hasło nieprawidłowe — spróbuj jeszcze raz.".into()),
                    ..Default::default()
                })
            }
            LoginStage::Done(li) => {
                self.qr_aktywny = false;
                self.login = None;
                let nazwa = if li.name.is_empty() {
                    "konto Telegram".to_string()
                } else {
                    li.name.clone()
                };
                self.po_zalogowaniu(&nazwa, Some(li)).await
            }
        }
    }

    /// Domknięcie: zapis sesji do `secrets.json` i odświeżenie listy kanałów.
    ///
    /// TO JEST MIEJSCE, w którym „logowanie działa raz" zamienia się
    /// w „logowanie działa zawsze": bez tego zapisu każdy restart bota
    /// wymagałby telefonu pod ręką.
    async fn po_zalogowaniu(&mut self, kto: &str, li: Option<LoggedIn>) -> AuthState {
        if let Some(c) = self.client.as_ref() {
            match c.session().to_string_session() {
                Ok(lancuch) => {
                    let mut sek = self.ws.load_secrets();
                    sek.telegram.session_string = lancuch.into();
                    sek.telegram.saved_at = conduit_server::now_ms();
                    if let Some(li) = &li {
                        sek.telegram.user_id = li.user_id;
                        sek.telegram.user_name = li.name.clone();
                        sek.telegram.handle = li.username.clone().unwrap_or_default();
                    }
                    if let Err(e) = self.ws.save_secrets(&sek) {
                        // Logowanie się udało, więc bot DZIAŁA — ale następny
                        // start znowu poprosi o telefon. To trzeba powiedzieć
                        // głośno, a nie schować w debug-logu.
                        warn!(%e, "Telegram: NIE UDAŁO SIĘ zapisać sesji — po restarcie trzeba będzie zeskanować kod ponownie");
                    } else {
                        info!("Telegram: sesja zapisana — kolejne uruchomienia zalogują się same");
                    }
                }
                Err(e) => warn!(%e, "Telegram: nie udało się zserializować sesji"),
            }

            // Napełnia cache peerów — bez tego nadrabianie zaległych
            // aktualizacji nie działa, a nazwy kanałów są puste.
            match c.refresh_dialogs(200).await {
                Ok(d) => self.zapamietaj_peery(&d),
                Err(e) => warn!(%e, "Telegram: nie udało się pobrać listy dialogów"),
            }
        }
        self.ustaw(AuthState::logged_in(kto))
    }

    // ---------- wylogowanie ----------

    async fn zamknij_klienta(&mut self) {
        self.login = None;
        self.qr_aktywny = false;
        if let Some(c) = self.client.take() {
            c.shutdown().await;
        }
    }

    // ---------- odbiór wiadomości ----------

    /// Czy wolno czytać strumień: jest klient, jesteśmy zalogowani i ktoś
    /// czeka na wiadomości.
    fn mozna_odbierac(&self) -> bool {
        self.client.is_some()
            && !self.qr_aktywny
            && self.state.lock().stage == AuthStage::LoggedIn
            && self.sink.lock().is_some()
    }

    /// Oddaje wiadomość pętli handlowej.
    ///
    /// Gdy odbiorca zniknął (pętla padła), kasujemy prenumeratę — inaczej
    /// każda kolejna wiadomość próbowałaby wysyłki na martwy kanał.
    fn oddaj(&mut self, m: conduit_core::engine::IncomingMessage) {
        let mut s = self.sink.lock();
        let Some(tx) = s.as_ref() else { return };
        if tx.send(m).is_err() {
            warn!("Telegram: odbiorca wiadomości zniknął — wstrzymuję odbiór");
            *s = None;
        }
    }

    /// Oddaje SKASOWANIE. Zawsze zostawia ślad w dzienniku, także wtedy, gdy
    /// nikt nie prenumeruje — bo „autor skasował sygnał" jest faktem wartym
    /// zapisania niezależnie od tego, czy ktoś zamierza coś z tym zrobić.
    fn oddaj_kasowanie(&mut self, k: Kasowanie) {
        warn!(
            czat = k.chat_id,
            zrodlo = %k.source_name,
            ile = k.msg_ids.len(),
            wiadomosci = ?k.msg_ids,
            polityka = ?k.polityka,
            "Telegram: AUTOR SKASOWAŁ WIADOMOŚCI z obserwowanego źródła"
        );
        let mut s = self.kasowania.lock();
        let Some(tx) = s.as_ref() else { return };
        if tx.send(k).is_err() {
            warn!("Telegram: odbiorca skasowań zniknął — wstrzymuję ich przekazywanie");
            *s = None;
        }
    }

    /// Przepisuje licznik luk z klienta do zdrowia usługi.
    ///
    /// Klient ginie przy każdej odbudowie, a panel ma widzieć sumę OD STARTU
    /// PROCESU — inaczej ponowne logowanie kasowałoby dowód, że coś przepadło,
    /// czyli gubiłoby informację dokładnie w chwili, w której jest najcenniejsza.
    fn przepisz_luki(&mut self) {
        let Some(c) = self.client.as_ref() else {
            return;
        };
        let teraz = c.luki().zgubione;
        // `zgubione_od_klienta` liczy tylko PRZYROST bieżącego klienta:
        // po odbudowie licznik klienta startuje od zera, więc różnicę bierzemy
        // względem tego, co z niego już przepisaliśmy.
        if teraz > self.zgubione_przepisane {
            self.zdrowie.zgubione(teraz - self.zgubione_przepisane);
            self.zgubione_przepisane = teraz;
        }
    }

    async fn po_bledzie_strumienia(&mut self, blad: String) {
        let ile = self.zdrowie.blad_strumienia();
        warn!(blad = %blad, proba = ile, "Telegram: strumień aktualizacji przerwany");

        // Zduplikowany klucz — ta sama zasada co przy pingu: ponowne logowanie
        // NIE POMOŻE, a próbowanie w kółko tylko zaciemnia dziennik.
        if klucz_zduplikowany(&blad) {
            warn!(
                "Telegram: TA SAMA SESJA DZIAŁA W DWÓCH MIEJSCACH — serwer unieważnił klucz. \
                 Odbudowa strumienia tego nie naprawi."
            );
            self.zamknij_klienta().await;
            self.ustaw(AuthState {
                stage: AuthStage::LoggedOut,
                error: Some(
                    "Ta sama sesja Telegrama działa na innej maszynie (AUTH_KEY_DUPLICATED). \
                     Wyłącz drugą instancję bota, skasuj tam telegram.session, a tutaj \
                     zaloguj się kodem QR od nowa."
                        .into(),
                ),
                ..Default::default()
            });
            return;
        }

        if ile < STRUMIEN_BLEDOW_DO_ODBUDOWY {
            tokio::time::sleep(przerwa_po_bledzie(ile)).await;
            return;
        }

        // Kolejna odbudowa po nieudanej poprzedniej czeka dłużej — inaczej
        // przy trwałej awarii logowalibyśmy się od nowa co kilkanaście sekund.
        let karencja = przerwa_przed_odbudowa(self.odbudowy_pod_rzad);
        if !karencja.is_zero() {
            warn!(
                sekund = karencja.as_secs(),
                nieudanych = self.odbudowy_pod_rzad,
                "Telegram: poprzednia odbudowa nie pomogła — czekam przed kolejną"
            );
            tokio::time::sleep(karencja).await;
        }

        warn!(
            proba = ile,
            odbudowa = self.odbudowy_pod_rzad + 1,
            "Telegram: {STRUMIEN_BLEDOW_DO_ODBUDOWY} błędy strumienia z rzędu — \
             ODBUDOWUJĘ połączenie z zapisanej sesji"
        );
        self.odbudowy_pod_rzad += 1;
        self.zdrowie.odbudowa_strumienia();
        self.zdrowie.wznowienie();
        // Ostatni odczyt licznika luk PRZED zamknięciem klienta — razem z nim
        // znika jego licznik, a to, co zgubił, ma zostać policzone.
        self.przepisz_luki();
        self.zamknij_klienta().await;
        // Licznik luk odchodzi razem z klientem; suma w zdrowiu ma zostać,
        // więc punkt odniesienia wraca na zero.
        self.zgubione_przepisane = 0;
        self.wznow_sesje().await;
    }

    async fn wyloguj(&mut self, zapomnij: bool) -> AuthState {
        self.zamknij_klienta().await;
        let _ = std::fs::remove_file(self.sciezka_sesji());

        let mut sek = self.ws.load_secrets();
        sek.telegram.session_string = Default::default();
        sek.telegram.user_id = 0;
        sek.telegram.user_name.clear();
        sek.telegram.handle.clear();
        if zapomnij {
            sek.telegram.api_id = 0;
            sek.telegram.api_hash = Default::default();
        }
        if let Err(e) = self.ws.save_secrets(&sek) {
            warn!(%e, "Telegram: nie udało się wyczyścić secrets.json");
        }

        if zapomnij {
            self.ustaw(AuthState::need_credentials())
        } else {
            self.ustaw(AuthState::logged_out())
        }
    }
}

enum Krok {
    Etap(anyhow::Result<LoginStage>),
    Cmd(Option<Cmd>),
}

enum Odbior {
    Zdarzenie(Result<Zdarzenie, grammers_client::InvocationError>),
    Cmd(Option<Cmd>),
    /// minęło `PING_CO` bez żadnego zdarzenia — czas zapytać, czy gniazdo żyje
    Budzik,
}

/// Spłaszcza „przekroczono czas" i błąd operacji do jednego komunikatu.
fn splasz(
    r: Result<anyhow::Result<AuthState>, tokio::time::error::Elapsed>,
    co: &str,
) -> Result<AuthState, String> {
    match r {
        Ok(Ok(s)) => Ok(s),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(format!(
            "{co}: Telegram nie odpowiedział w ciągu {} s — spróbuj jeszcze raz",
            LIMIT_ODPOWIEDZI.as_secs()
        )),
    }
}

impl Worker {
    /// `step` zwraca `Result`; błąd nie może wywalić zadania, bo wtedy
    /// logowanie zawisłoby na zawsze bez żadnego komunikatu.
    async fn po_etapie_wynik(&mut self, r: anyhow::Result<LoginStage>) -> AuthState {
        match r {
            Ok(e) => self.po_etapie(e).await,
            Err(e) => {
                warn!(%e, "Telegram: krok logowania nieudany");
                self.qr_aktywny = false;
                self.ustaw(AuthState::error(format!("Logowanie przerwane: {e}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn budzik_odpala_mimo_ciaglego_ruchu_w_drugiej_galezi() {
        // Skala zegarowa zmniejszona 1200×: ping co 200 ms zamiast co 240 s,
        // „wiadomość" co 20 ms. Odstępy są celowo WYRAŹNIE większe od
        // rozdzielczości zegara Windows (~15,6 ms) — pierwsza wersja tego testu
        // używała 5 ms i padała, bo system nie umie tak krótko spać.
        let okres = Duration::from_millis(200);
        let mut nastepny = tokio::time::Instant::now() + okres;
        let mut pingi = 0u32;
        let mut wiadomosci = 0u32;

        let koniec = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < koniec {
            tokio::select! {
                // „wiadomość" przychodzi bez przerwy, dużo częściej niż ping
                _ = tokio::time::sleep(Duration::from_millis(20)) => {
                    wiadomosci += 1;
                }
                _ = tokio::time::sleep_until(nastepny) => {
                    pingi += 1;
                    nastepny = tokio::time::Instant::now() + okres;
                }
            }
        }

        // Gęstość mierzymy WZGLĘDNIE, a nie bezwzględną liczbą wywołań: ta
        // druga zależy od rozdzielczości zegara systemu i robi z testu ruletkę.
        // Teza jest o proporcji — ruch ma być wielokrotnie gęstszy od pingu,
        // a ping ma mimo to wystrzelić.
        assert!(
            pingi >= 3,
            "budzik ma odliczać NIEZALEŻNIE od ruchu; wystrzelił {pingi} razy"
        );
        assert!(
            wiadomosci >= 3 * pingi,
            "ruch musi być wyraźnie gęstszy od pingu, inaczej test nic nie dowodzi \
             ({wiadomosci} wiadomości na {pingi} pingów)"
        );
        // Przy `sleep(PING_CO)` zamiast `sleep_until(termin)` `pingi` byłoby
        // ZEREM: każda wiadomość kasowałaby odliczanie. To jest ta regresja.
    }

    /// Kontrola samej stałej: ping rzadszy niż `bot.py` (240 s) nie ma sensu,
    /// a limit odpowiedzi musi być wyraźnie krótszy od okresu, inaczej dwa
    /// pingi zachodziłyby na siebie.
    #[test]
    fn stale_keepalive_sa_spojne() {
        assert!(
            PING_CO <= Duration::from_secs(300),
            "rzadziej niż co 5 min to za rzadko"
        );
        assert!(
            PING_LIMIT < PING_CO,
            "limit odpowiedzi musi być krótszy od okresu"
        );
        assert!(
            PING_BLEDOW_DO_RESTARTU >= 2,
            "jedna czkawka sieci nie może zrywać sesji"
        );
    }

    /// `Zdrowie` ma być zbiorem FAKTÓW: udany ping kasuje licznik błędów,
    /// nieudany go podnosi. Na tym stoi reguła alarmu po stronie aplikacji.
    #[test]
    fn zdrowie_liczy_bledy_pod_rzad() {
        let z = Zdrowie::default();
        assert_eq!(z.migawka().bledy_pingu, 0);
        assert_eq!(z.ping_zle(), 1);
        assert_eq!(z.ping_zle(), 2);
        assert_eq!(z.migawka().bledy_pingu, 2);
        z.ping_ok();
        assert_eq!(z.migawka().bledy_pingu, 0, "udany ping kasuje serię");
        assert!(z.migawka().ostatni_ping_ok_ms > 0);

        assert_eq!(z.migawka().ostatnia_wiadomosc_ms, 0);
        z.wiadomosc();
        assert!(z.migawka().ostatnia_wiadomosc_ms > 0);
    }

    // ========================================================
    //  B12 — ODBUDOWA STRUMIENIA
    // ========================================================

    #[test]
    fn licznik_bledow_strumienia_nie_udaje_odbudowy() {
        let z = Zdrowie::default();
        assert_eq!(z.blad_strumienia(), 1);
        assert_eq!(z.blad_strumienia(), 2);
        let m = z.migawka();
        assert_eq!(m.bledy_strumienia, 2);
        assert_eq!(m.odbudowy_strumienia, 0, "nic jeszcze nie odbudowano");
        assert_eq!(
            m.wznowienia, 0,
            "wznowienie ma znaczyć wznowienie, a nie błąd"
        );

        z.odbudowa_strumienia();
        let m = z.migawka();
        assert_eq!(m.odbudowy_strumienia, 1);
        assert_eq!(m.bledy_strumienia, 0, "odbudowa zamyka serię");
    }

    /// Wiadomość, która doszła, jest MOCNIEJSZYM dowodem żywego strumienia
    /// niż jakikolwiek ping — musi więc kasować serię błędów.
    #[test]
    fn wiadomosc_zamyka_serie_bledow_strumienia() {
        let z = Zdrowie::default();
        z.blad_strumienia();
        z.blad_strumienia();
        assert_eq!(z.migawka().bledy_strumienia, 2);
        z.wiadomosc();
        assert_eq!(z.migawka().bledy_strumienia, 0);
        assert!(z.migawka().ostatnia_wiadomosc_ms > 0);
    }

    /// Przerwa ma rosnąć, bo dwie sekundy przy trwałej awarii to 1 800 wpisów
    /// w dzienniku na godzinę — dziennik przestaje być czytelny dokładnie
    /// wtedy, gdy jest najbardziej potrzebny. I ma mieć sufit, bo po powrocie
    /// Telegrama chcemy wrócić w ciągu minuty, a nie po kwadransie.
    #[test]
    fn przerwa_po_bledzie_rosnie_i_ma_sufit() {
        assert_eq!(przerwa_po_bledzie(1), Duration::from_secs(2));
        assert_eq!(przerwa_po_bledzie(2), Duration::from_secs(4));
        assert_eq!(przerwa_po_bledzie(3), Duration::from_secs(8));
        assert_eq!(przerwa_po_bledzie(4), Duration::from_secs(16));
        assert_eq!(przerwa_po_bledzie(50), STRUMIEN_PRZERWA_MAX, "sufit trzyma");
        // przesunięcie bitowe nie ma prawa się przekręcić na dużej liczbie
        assert_eq!(przerwa_po_bledzie(u32::MAX), STRUMIEN_PRZERWA_MAX);
    }

    /// PIERWSZA odbudowa idzie natychmiast — czekanie przy pierwszym zerwaniu
    /// byłoby czystą stratą sygnałów. Dopiero kolejne, po nieudanych, czekają.
    #[test]
    fn pierwsza_odbudowa_jest_natychmiastowa_a_kolejne_czekaja() {
        assert!(
            przerwa_przed_odbudowa(0).is_zero(),
            "pierwsza próba bez zwłoki"
        );
        assert_eq!(przerwa_przed_odbudowa(1), Duration::from_secs(30));
        assert_eq!(przerwa_przed_odbudowa(3), Duration::from_secs(90));
        assert_eq!(
            przerwa_przed_odbudowa(1_000),
            ODBUDOWA_PRZERWA_MAX,
            "sufit trzyma"
        );
        assert_eq!(przerwa_przed_odbudowa(u32::MAX), ODBUDOWA_PRZERWA_MAX);
    }

    #[test]
    fn stale_odbudowy_sa_spojne() {
        assert!(
            STRUMIEN_BLEDOW_DO_ODBUDOWY >= 2,
            "jedno zerwanie bywa czkawką sieci i strumień sam się z niej podnosi"
        );
        // Trzy błędy z narastającą przerwą (2+4) muszą zmieścić się wyraźnie
        // poniżej okresu keepalive — inaczej ping i odbudowa deptałyby sobie
        // po piętach i nie dałoby się powiedzieć, co bota naprawiło.
        let do_odbudowy: Duration = (1..STRUMIEN_BLEDOW_DO_ODBUDOWY)
            .map(przerwa_po_bledzie)
            .sum();
        assert!(
            do_odbudowy < PING_CO,
            "odbudowa ma zdążyć przed kolejnym pingiem ({do_odbudowy:?} vs {PING_CO:?})"
        );
        assert!(ODBUDOWA_PRZERWA <= ODBUDOWA_PRZERWA_MAX);
    }

    // ========================================================
    //  B14 — LICZNIK ZGUBIONYCH AKTUALIZACJI
    // ========================================================

    /// Suma zgubionych ma być liczona OD STARTU PROCESU, a nie od bieżącego
    /// klienta: ponowne logowanie kasowałoby inaczej dowód, że coś przepadło —
    /// czyli gubiłoby informację dokładnie w chwili, w której jest najcenniejsza.
    #[test]
    fn zgubione_aktualizacje_sumuja_sie_ponad_odbudowami() {
        let z = Zdrowie::default();
        assert_eq!(z.migawka().zgubione_aktualizacje, 0);
        z.zgubione(4);
        z.odbudowa_strumienia();
        z.zgubione(3);
        assert_eq!(
            z.migawka().zgubione_aktualizacje,
            7,
            "odbudowa NIE MOŻE kasować dowodu, że aktualizacje przepadły"
        );
    }

    #[test]
    fn swieze_zdrowie_mowi_zero_a_nie_nic() {
        let m = Zdrowie::default().migawka();
        assert_eq!(m.zgubione_aktualizacje, 0);
        assert_eq!(m.bledy_strumienia, 0);
        assert_eq!(m.odbudowy_strumienia, 0);
    }

    // ========================================================
    //  B11 — KASOWANIE WIADOMOŚCI
    // ========================================================

    /// KONTRAKT ZERA: usługa startuje z polityką, która nie zmienia niczego.
    #[test]
    fn usluga_startuje_z_polityka_ignorujaca_kasowanie() {
        assert_eq!(
            PolitykaKasowania::default(),
            PolitykaKasowania::Ignoruj,
            "dołożenie osi B11 nie ma prawa samo w sobie zmienić zachowania bota"
        );
    }

    fn tmp(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "conduit-tgsvc-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn bez_poswiadczen_prosimy_o_nie_zamiast_rysowac_qr() {
        let dir = tmp("brak");
        let ws = Workspace::new(&dir);
        ws.ensure_dirs().unwrap();

        // stan początkowy liczymy tak samo jak `start()`, ale bez runtime'u
        let sek = ws.load_secrets().telegram;
        assert!(!sek.has_credentials());
        let s = AuthState::need_credentials().with_credentials(
            sek.api_id,
            sek.api_hash.as_str(),
            sek.has_session(),
        );
        assert_eq!(s.stage, AuthStage::NeedCredentials);
        assert!(
            s.qr_svg.is_none(),
            "bez api_id nie da się zrobić PRAWDZIWEGO kodu QR"
        );
        assert!(!s.api_hash_set);
        assert!(!s.session_saved);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zapisane_poswiadczenia_daja_etap_laczenia() {
        let dir = tmp("sa");
        let ws = Workspace::new(&dir);
        ws.ensure_dirs().unwrap();

        let mut sek = ws.load_secrets();
        sek.telegram.api_id = 1234567;
        sek.telegram.api_hash = "0123456789abcdef0123456789abcdef".into();
        sek.telegram.session_string = "cokolwiek".into();
        ws.save_secrets(&sek).unwrap();

        let sek = ws.load_secrets().telegram;
        assert!(sek.has_credentials());
        assert!(sek.has_session());
        let s = AuthState {
            stage: AuthStage::Connecting,
            ..Default::default()
        }
        .with_credentials(sek.api_id, sek.api_hash.as_str(), sek.has_session());
        assert_eq!(s.stage, AuthStage::Connecting);
        assert_eq!(s.api_id, Some(1234567));
        assert!(s.session_saved, "UI ma wiedzieć, że logowanie pójdzie samo");

        // i najważniejsze: hasz nie wychodzi na zewnątrz
        let j = serde_json::to_string(&s).unwrap();
        assert!(!j.contains("0123456789abcdef"), "{j}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn poswiadczenia_przezywaja_restart_programu() {
        // TO JEST SEDNO ZADANIA 1: drugi start nie może pytać o to samo
        let dir = tmp("restart");
        {
            let ws = Workspace::new(&dir);
            ws.ensure_dirs().unwrap();
            let mut sek = ws.load_secrets();
            sek.telegram.api_id = 987654;
            sek.telegram.api_hash = "abcdefabcdefabcdefabcdefabcdefab".into();
            sek.telegram.session_string = "BASE64-SESJI".into();
            sek.telegram.user_name = "Demo User".into();
            ws.save_secrets(&sek).unwrap();
        }
        // nowy proces, ten sam katalog
        let ws2 = Workspace::new(&dir);
        let sek = ws2.load_secrets().telegram;
        assert_eq!(sek.api_id, 987654);
        assert_eq!(sek.api_hash.as_str(), "abcdefabcdefabcdefabcdefabcdefab");
        assert_eq!(sek.session_string.as_str(), "BASE64-SESJI");
        assert_eq!(sek.user_name, "Demo User");
        assert!(sek.has_credentials() && sek.has_session());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plik_sesji_odtwarza_sie_z_lancucha_w_secrets() {
        let dir = tmp("odtworz");
        let sciezka = dir.join(PLIK_SESJI);

        let zrodlo = FileSession::in_memory();
        let lancuch = zrodlo.to_string_session().unwrap();
        assert!(!sciezka.exists());

        FileSession::restore_string_session(&sciezka, &lancuch).unwrap();
        assert!(sciezka.exists(), "plik sesji musi powstać z łańcucha");
        // i musi dać się wczytać jako prawdziwa sesja
        FileSession::load(&sciezka).expect("odtworzona sesja musi być czytelna");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn smiec_nie_podmienia_dzialajacej_sesji() {
        let dir = tmp("smiec");
        let sciezka = dir.join(PLIK_SESJI);
        FileSession::restore_string_session(
            &sciezka,
            &FileSession::in_memory().to_string_session().unwrap(),
        )
        .unwrap();

        assert!(FileSession::restore_string_session(&sciezka, "to-nie-jest-base64!!!").is_err());
        assert!(
            FileSession::restore_string_session(&sciezka, "bm90LWEtc2Vzc2lvbg==").is_err(),
            "poprawny base64 z nie-sesją też musi zostać odrzucony"
        );
        // a plik, który był, nadal działa
        FileSession::load(&sciezka).expect("nieudane odtworzenie nie może uszkodzić sesji");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wylogowanie_kasuje_sesje_ale_zostawia_poswiadczenia() {
        let dir = tmp("wyloguj");
        let ws = Workspace::new(&dir);
        ws.ensure_dirs().unwrap();
        let mut sek = ws.load_secrets();
        sek.telegram.api_id = 111;
        sek.telegram.api_hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
        sek.telegram.session_string = "sesja".into();
        ws.save_secrets(&sek).unwrap();

        // to samo, co robi `wyloguj(false)`
        let mut sek = ws.load_secrets();
        sek.telegram.session_string = Default::default();
        sek.telegram.user_name.clear();
        ws.save_secrets(&sek).unwrap();

        let po = ws.load_secrets().telegram;
        assert!(!po.has_session(), "sesja ma zniknąć");
        assert!(
            po.has_credentials(),
            "ale api_id/api_hash zostają — to nie jest reset"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
