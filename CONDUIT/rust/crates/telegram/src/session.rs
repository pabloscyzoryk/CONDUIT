//! Sesja zapisywana na dysk — plus przenośny „session string".
//!
//! `grammers-session` 0.10 daje dokładnie dwa magazyny: `MemorySession`
//! (znika przy zamknięciu) i `SqliteSession` (ciągnie `libsql`, czyli
//! kilkadziesiąt sekund kompilacji i natywną bibliotekę w bagażu).
//! Do bota potrzebny jest trzeci wariant: jeden plik, zero zależności.
//!
//! **Dlaczego to jest ważniejsze, niż wygląda.** Logowanie do Telegrama jest
//! drogie: kilka nieudanych prób pod rząd kończy się blokadą na godziny
//! (flood wait). Sesja, która nie przeżywa restartu, to bot, który przy każdym
//! wznowieniu prosi o skanowanie QR — i po kilku restartach nie może się
//! zalogować w ogóle.
//!
//! # Co zawiera plik
//!
//! Klucz autoryzacyjny do datacentrum (256 bajtów), listę datacentrów, cache
//! peerów i stan aktualizacji. **Klucz autoryzacyjny jest równoważny
//! zalogowanemu urządzeniu** — kto ma ten plik, ma dostęp do konta. Plik
//! zapisujemy najpierw obok, a potem podmieniamy (`rename`), żeby przerwanie
//! zapisu nie zostawiło sesji uciętej w połowie.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use grammers_session::types::{
    ChannelState, DcOption, PeerId, PeerInfo, UpdateState, UpdatesState,
};
use grammers_session::{BoxFuture, Session, SessionData};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// Wersja formatu pliku. Zmiana = plik z poprzedniej wersji jest odrzucany
/// (zamiast być czytany na pół i dawać sesję-widmo).
const FORMAT: u32 = 1;

#[derive(Debug)]
pub enum SessionError {
    Io(String),
    Format(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::Io(e) => write!(f, "błąd wejścia/wyjścia sesji: {e}"),
            SessionError::Format(e) => write!(f, "zły format sesji: {e}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// Reprezentacja pliku sesji. `SessionData` z biblioteki nie ma `Serialize`,
/// a mapowanie po `PeerId` jako kluczu JSON-a bywa kruche — dlatego wektory.
#[derive(Serialize, Deserialize)]
struct Persisted {
    version: u32,
    home_dc: i32,
    dc_options: Vec<DcOption>,
    peers: Vec<PeerInfo>,
    updates: UpdatesState,
}

/// Sesja trzymana w pamięci, zrzucana do jednego pliku JSON.
pub struct FileSession {
    path: Option<PathBuf>,
    data: Mutex<SessionData>,
    /// czy od ostatniego zapisu coś się zmieniło
    dirty: Mutex<bool>,
}

impl fmt::Debug for FileSession {
    /// Wypisuje wyłącznie metadane. Klucz autoryzacyjny NIE MOŻE trafić do
    /// logu — jest równoważny zalogowanemu urządzeniu.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileSession")
            .field("path", &self.path)
            .field("ma_klucz", &self.has_auth_key())
            .field("ma_uzytkownika", &self.has_user())
            .field("do_zapisu", &self.is_dirty())
            .finish()
    }
}

impl Default for FileSession {
    fn default() -> Self {
        FileSession {
            path: None,
            data: Mutex::new(SessionData::default()),
            dirty: Mutex::new(false),
        }
    }
}

impl FileSession {
    /// Wczytuje sesję z pliku; brak pliku to nie błąd, tylko „jeszcze nie
    /// zalogowany". Plik uszkodzony to JUŻ błąd — cicha podmiana na pustą
    /// sesję kasowałaby zalogowanie i wywoływała kolejne logowanie.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, SessionError> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Ok(FileSession {
                path: Some(path),
                data: Mutex::new(SessionData::default()),
                dirty: Mutex::new(false),
            });
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| SessionError::Io(e.to_string()))?;
        let data = Self::decode(&raw)?;
        Ok(FileSession {
            path: Some(path),
            data: Mutex::new(data),
            dirty: Mutex::new(false),
        })
    }

    /// Sesja bez pliku — do testów i do logowania „na próbę".
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Sesja odtworzona z przenośnego łańcucha (patrz [`FileSession::to_string_session`]).
    pub fn from_string_session(s: &str) -> Result<Self, SessionError> {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(s.trim())
            .map_err(|e| SessionError::Format(e.to_string()))?;
        let txt = String::from_utf8(raw).map_err(|e| SessionError::Format(e.to_string()))?;
        Ok(FileSession {
            path: None,
            data: Mutex::new(Self::decode(&txt)?),
            dirty: Mutex::new(false),
        })
    }

    fn decode(raw: &str) -> Result<SessionData, SessionError> {
        let p: Persisted =
            serde_json::from_str(raw).map_err(|e| SessionError::Format(e.to_string()))?;
        if p.version != FORMAT {
            return Err(SessionError::Format(format!(
                "wersja pliku {} — obsługiwana {FORMAT}",
                p.version
            )));
        }
        let mut dc_options = HashMap::new();
        for d in p.dc_options {
            dc_options.insert(d.id, d);
        }
        let mut peer_infos = HashMap::new();
        for peer in p.peers {
            peer_infos.insert(peer.id(), peer);
        }
        Ok(SessionData {
            home_dc: p.home_dc,
            dc_options,
            peer_infos,
            updates_state: p.updates,
        })
    }

    fn encode(&self) -> Result<String, SessionError> {
        let d = self.data.lock();
        let p = Persisted {
            version: FORMAT,
            home_dc: d.home_dc,
            dc_options: d.dc_options.values().cloned().collect(),
            peers: d.peer_infos.values().cloned().collect(),
            updates: d.updates_state.clone(),
        };
        serde_json::to_string(&p).map_err(|e| SessionError::Format(e.to_string()))
    }

    /// Przenośny łańcuch sesji — to samo, co plik, tylko w base64.
    ///
    /// Traktować jak hasło do konta.
    pub fn to_string_session(&self) -> Result<String, SessionError> {
        Ok(base64::engine::general_purpose::STANDARD.encode(self.encode()?))
    }

    /// Odtwarza PLIK sesji z przenośnego łańcucha.
    ///
    /// Po co, skoro jest [`FileSession::from_string_session`]: tamta tworzy
    /// sesję bez pliku, więc następny zapis nie miałby dokąd trafić. Tutaj
    /// chodzi o odbudowanie magazynu na dysku — sytuacja „mam kopię sesji
    /// w `secrets.json`, ale plik sesji zniknął" po przeniesieniu bota na inną
    /// maszynę albo po sprzątaniu katalogu.
    ///
    /// Łańcuch jest WERYFIKOWANY przed zapisem: śmieć nie może podmienić
    /// działającej sesji na plik, którego potem nie da się wczytać.
    pub fn restore_string_session(path: impl AsRef<Path>, s: &str) -> Result<(), SessionError> {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(s.trim())
            .map_err(|e| SessionError::Format(e.to_string()))?;
        let txt = String::from_utf8(raw).map_err(|e| SessionError::Format(e.to_string()))?;
        // sprawdzamy, że to w ogóle jest sesja w naszym formacie
        Self::decode(&txt)?;
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| SessionError::Io(e.to_string()))?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, txt.as_bytes()).map_err(|e| SessionError::Io(e.to_string()))?;
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| SessionError::Io(e.to_string()))?;
        }
        std::fs::rename(&tmp, path).map_err(|e| SessionError::Io(e.to_string()))?;
        Ok(())
    }

    /// Czy od ostatniego zapisu coś się zmieniło?
    pub fn is_dirty(&self) -> bool {
        *self.dirty.lock()
    }

    /// Zapisuje sesję do pliku podanego przy wczytaniu.
    ///
    /// Zapis jest ATOMOWY: najpierw plik tymczasowy, potem podmiana nazwy.
    /// Przerwanie procesu w połowie zapisu nie może zostawić sesji uszkodzonej,
    /// bo to znaczy utratę zalogowania.
    pub fn save(&self) -> Result<(), SessionError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let body = self.encode()?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, body.as_bytes()).map_err(|e| SessionError::Io(e.to_string()))?;
        // Windows nie pozwala nadpisać istniejącego pliku przez rename
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| SessionError::Io(e.to_string()))?;
        }
        std::fs::rename(&tmp, path).map_err(|e| SessionError::Io(e.to_string()))?;
        *self.dirty.lock() = false;
        Ok(())
    }

    /// Zapisuje tylko wtedy, gdy było co zapisywać.
    pub fn save_if_dirty(&self) -> Result<(), SessionError> {
        if self.is_dirty() {
            self.save()
        } else {
            Ok(())
        }
    }

    /// Wpis o zalogowanym użytkowniku, wyszukany po fladze `is_self`.
    fn find_self(&self) -> Option<PeerInfo> {
        self.data
            .lock()
            .peer_infos
            .values()
            .find(|p| {
                matches!(
                    p,
                    PeerInfo::User {
                        is_self: Some(true),
                        ..
                    }
                )
            })
            .cloned()
    }

    /// Czy sesja niesie zalogowanego użytkownika?
    ///
    /// Sam klucz autoryzacyjny nie wystarcza — dopiero wpis o „sobie"
    /// w cache'u peerów znaczy, że logowanie się dokończyło.
    pub fn has_user(&self) -> bool {
        self.find_self().is_some()
    }

    /// Czy mamy klucz autoryzacyjny do domowego datacentrum?
    pub fn has_auth_key(&self) -> bool {
        let d = self.data.lock();
        d.dc_options
            .get(&d.home_dc)
            .map(|o| o.auth_key.is_some())
            .unwrap_or(false)
    }

    fn touch(&self) {
        *self.dirty.lock() = true;
    }
}

impl Session for FileSession {
    type Error = SessionError;

    fn home_dc_id(&self) -> Result<i32, Self::Error> {
        Ok(self.data.lock().home_dc)
    }

    fn set_home_dc_id(&self, dc_id: i32) -> BoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            self.data.lock().home_dc = dc_id;
            self.touch();
            Ok(())
        })
    }

    fn dc_option(&self, dc_id: i32) -> Result<Option<DcOption>, Self::Error> {
        Ok(self.data.lock().dc_options.get(&dc_id).cloned())
    }

    fn set_dc_option(&self, dc_option: &DcOption) -> BoxFuture<'_, Result<(), Self::Error>> {
        let dc_option = dc_option.clone();
        Box::pin(async move {
            self.data.lock().dc_options.insert(dc_option.id, dc_option);
            self.touch();
            Ok(())
        })
    }

    fn peer(&self, peer: PeerId) -> BoxFuture<'_, Result<Option<PeerInfo>, Self::Error>> {
        Box::pin(async move {
            // `PeerId::self_user()` to WARTOWNIK, a nie prawdziwy identyfikator:
            // w mapie leży pod swoim zwykłym numerem, z flagą `is_self`.
            // Zwykłe `get()` zwróciłoby `None`, a wtedy `stream_updates` uznaje,
            // że nie jesteśmy zalogowani, i nigdy nie nadrabia zaległości.
            // (`MemorySession` z biblioteki ma dokładnie tę wadę; magazyn
            // SQLite szuka po fladze — i my robimy tak samo.)
            if peer.bot_api_dialog_id().is_none() {
                return Ok(self.find_self());
            }
            Ok(self.data.lock().peer_infos.get(&peer).cloned())
        })
    }

    fn cache_peer(&self, peer: &PeerInfo) -> BoxFuture<'_, Result<(), Self::Error>> {
        let peer = peer.clone();
        Box::pin(async move {
            // `extend_info` dokłada brakujące pola zamiast nadpisywać całość —
            // peer „min" (bez access_hash) nie może skasować pełnego wpisu
            self.data
                .lock()
                .peer_infos
                .entry(peer.id())
                .or_insert_with(|| peer.clone())
                .extend_info(&peer);
            self.touch();
            Ok(())
        })
    }

    fn updates_state(&self) -> BoxFuture<'_, Result<UpdatesState, Self::Error>> {
        Box::pin(async move { Ok(self.data.lock().updates_state.clone()) })
    }

    fn set_update_state(&self, update: UpdateState) -> BoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            let mut d = self.data.lock();
            match update {
                UpdateState::All(s) => d.updates_state = s,
                UpdateState::Primary { pts, date, seq } => {
                    d.updates_state.pts = pts;
                    d.updates_state.date = date;
                    d.updates_state.seq = seq;
                }
                UpdateState::Secondary { qts } => d.updates_state.qts = qts,
                UpdateState::Channel { id, pts } => {
                    d.updates_state.channels.retain(|c| c.id != id);
                    d.updates_state.channels.push(ChannelState { id, pts });
                }
            }
            drop(d);
            self.touch();
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "conduit_tg_test_{}_{}.json",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[tokio::test]
    async fn brak_pliku_to_pusta_sesja_a_nie_blad() {
        let p = tmp("brak");
        let s = FileSession::load(&p).unwrap();
        assert!(!s.has_user());
        assert!(!s.has_auth_key());
        assert!(!s.is_dirty());
    }

    #[tokio::test]
    async fn sesja_przezywa_zapis_i_odczyt() {
        let p = tmp("runda");
        {
            let s = FileSession::load(&p).unwrap();
            s.set_home_dc_id(4).await.unwrap();
            s.cache_peer(&PeerInfo::User {
                id: 777_000,
                auth: Some(grammers_session::types::PeerAuth::from_hash(12345)),
                bot: Some(false),
                is_self: Some(true),
            })
            .await
            .unwrap();
            s.set_update_state(UpdateState::Primary {
                pts: 10,
                date: 20,
                seq: 30,
            })
            .await
            .unwrap();
            assert!(s.is_dirty(), "zmiany muszą być oznaczone do zapisu");
            s.save().unwrap();
            assert!(!s.is_dirty(), "po zapisie nie ma czego zapisywać");
        }
        let s = FileSession::load(&p).unwrap();
        assert_eq!(s.home_dc_id().unwrap(), 4);
        let st = s.updates_state().await.unwrap();
        assert_eq!((st.pts, st.date, st.seq), (10, 20, 30));
        assert!(s.has_user(), "zalogowany użytkownik musi przetrwać restart");
        let _ = std::fs::remove_file(&p);
    }

    #[tokio::test]
    async fn zapis_nadpisuje_istniejacy_plik() {
        // Windows nie pozwala na rename na istniejący plik — gdyby o tym
        // zapomnieć, drugi zapis sesji przestałby działać po cichu
        let p = tmp("nadpis");
        let s = FileSession::load(&p).unwrap();
        s.set_home_dc_id(2).await.unwrap();
        s.save().unwrap();
        s.set_home_dc_id(5).await.unwrap();
        s.save().unwrap();
        let s2 = FileSession::load(&p).unwrap();
        assert_eq!(s2.home_dc_id().unwrap(), 5);
        let _ = std::fs::remove_file(&p);
    }

    #[tokio::test]
    async fn string_session_przenosi_calosc() {
        let a = FileSession::in_memory();
        a.set_home_dc_id(3).await.unwrap();
        a.cache_peer(&PeerInfo::Channel {
            id: 1_234_567,
            auth: Some(grammers_session::types::PeerAuth::from_hash(-99)),
            kind: Some(grammers_session::types::ChannelKind::Broadcast),
        })
        .await
        .unwrap();

        let s = a.to_string_session().unwrap();
        let b = FileSession::from_string_session(&s).unwrap();
        assert_eq!(b.home_dc_id().unwrap(), 3);
        let peer = b
            .peer(PeerId::channel(1_234_567).unwrap())
            .await
            .unwrap()
            .expect("kanał musi przetrwać przeniesienie");
        assert_eq!(peer.auth().unwrap().hash(), -99);
    }

    #[test]
    fn uszkodzony_plik_jest_bledem_a_nie_cichym_wylogowaniem() {
        let p = tmp("uszkodzony");
        std::fs::write(&p, b"{ to nie jest json").unwrap();
        let e = FileSession::load(&p).unwrap_err();
        assert!(matches!(e, SessionError::Format(_)), "{e}");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn plik_z_innej_wersji_formatu_jest_odrzucany() {
        let p = tmp("wersja");
        std::fs::write(
            &p,
            br#"{"version":99,"home_dc":2,"dc_options":[],"peers":[],"updates":{"pts":0,"qts":0,"date":0,"seq":0,"channels":[]}}"#,
        )
        .unwrap();
        let e = FileSession::load(&p).unwrap_err();
        assert!(matches!(e, SessionError::Format(_)), "{e}");
        let _ = std::fs::remove_file(&p);
    }

    #[tokio::test]
    async fn zapytanie_o_siebie_dziala_mimo_ze_wartownik_nie_jest_kluczem() {
        // To jest pułapka, na której `MemorySession` z biblioteki się wykłada:
        // `PeerId::self_user()` NIE jest kluczem w mapie — wpis leży pod
        // prawdziwym numerem użytkownika, z flagą `is_self`. Gdyby `peer()`
        // robiło zwykłe `get()`, `stream_updates` uznałoby, że nie jesteśmy
        // zalogowani, i nigdy nie nadrobiłoby zaległych wiadomości.
        let s = FileSession::in_memory();
        s.cache_peer(&PeerInfo::User {
            id: 424_242,
            auth: Some(grammers_session::types::PeerAuth::from_hash(7)),
            bot: Some(false),
            is_self: Some(true),
        })
        .await
        .unwrap();
        // cudzy użytkownik NIE MOŻE zostać wzięty za nas
        s.cache_peer(&PeerInfo::User {
            id: 999,
            auth: Some(grammers_session::types::PeerAuth::from_hash(1)),
            bot: Some(false),
            is_self: Some(false),
        })
        .await
        .unwrap();

        assert!(
            PeerId::self_user().bot_api_dialog_id().is_none(),
            "wartownik nie ma numeru"
        );
        let me = s
            .peer(PeerId::self_user())
            .await
            .unwrap()
            .expect("wpis o sobie");
        match me {
            PeerInfo::User { id, is_self, .. } => {
                assert_eq!(id, 424_242);
                assert_eq!(is_self, Some(true));
            }
            other => panic!("oczekiwano użytkownika, jest {other:?}"),
        }
        // zwykłe wyszukanie po numerze nadal działa
        let inny = s.peer(PeerId::user(999).unwrap()).await.unwrap();
        assert!(inny.is_some());
    }

    #[tokio::test]
    async fn brak_zalogowania_zwraca_none_a_nie_przypadkowego_peera() {
        let s = FileSession::in_memory();
        s.cache_peer(&PeerInfo::Channel {
            id: 5,
            auth: Some(grammers_session::types::PeerAuth::from_hash(1)),
            kind: None,
        })
        .await
        .unwrap();
        assert!(s.peer(PeerId::self_user()).await.unwrap().is_none());
        assert!(!s.has_user());
    }

    #[tokio::test]
    async fn stan_kanalu_nie_duplikuje_sie() {
        let s = FileSession::in_memory();
        s.set_update_state(UpdateState::Channel { id: 5, pts: 1 })
            .await
            .unwrap();
        s.set_update_state(UpdateState::Channel { id: 5, pts: 2 })
            .await
            .unwrap();
        s.set_update_state(UpdateState::Channel { id: 6, pts: 9 })
            .await
            .unwrap();
        let st = s.updates_state().await.unwrap();
        assert_eq!(st.channels.len(), 2);
        assert_eq!(st.channels.iter().find(|c| c.id == 5).unwrap().pts, 2);
    }
}
