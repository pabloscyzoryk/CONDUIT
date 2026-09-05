//! Klient — spina sesję, logowanie, odbiór wiadomości i powiadomienia.
//!
//! Rola tego pliku jest celowo skromna: nie ma tu żadnej logiki handlowej.
//! Zamienia zdarzenia Telegrama na `IncomingMessage` i podaje je dalej.
//! Wszystko, co decyduje o transakcjach, dzieje się w `conduit-core`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use conduit_core::engine::IncomingMessage;
use conduit_core::types::SourceKey;
use grammers_client::client::{UpdateStream, UpdatesConfiguration};
use grammers_client::session::types::PeerRef;
use grammers_client::{Client, InvocationError, SenderPool};
use parking_lot::Mutex;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::dialogs::{self, DialogEntry};
use crate::incoming;
use crate::login::QrLogin;
use crate::luki::{LicznikLuk, LukiMigawka};
use crate::session::FileSession;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PolitykaKasowania {
    /// Skasowań nie ma. DOKŁADNIE dzisiejsze zachowanie.
    #[default]
    Ignoruj,
    /// Odbiorca dostaje zdarzenie, ale ma na nim tylko zapisać ostrzeżenie.
    Ostrzez,
    /// Odbiorca ma skasować NIEWYPEŁNIONE limity koszyka i zapisać ostrzeżenie.
    /// Otwartych pozycji NIE RUSZA — patrz uzasadnienie wyżej.
    KasujLimity,
}

/// Skasowanie wiadomości przez autora, gotowe do oddania odbiorcy.
///
/// Niesie politykę razem ze zdarzeniem, a nie obok niego: odbiorca, który
/// musiałby dopytać o ustawienie osobnym wywołaniem, prędzej czy później
/// zapytałby o nie w złej chwili (np. po zmianie w panelu) i zrobiłby coś
/// innego, niż wynikało z ustawienia w momencie zdarzenia.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kasowanie {
    /// chwila ODEBRANIA — Telegram nie podaje czasu skasowania
    pub ts: conduit_core::types::Ts,
    pub chat_id: i64,
    pub source_name: String,
    /// numery skasowanych wiadomości; jednoznaczne w obrębie czatu
    pub msg_ids: Vec<i64>,
    pub polityka: PolitykaKasowania,
}

/// Co przyszło ze strumienia.
///
/// Osobny typ zamiast rozszerzania `IncomingMessage`, bo skasowanie NIE MA
/// treści: wepchnięte w wiadomość z pustym tekstem byłoby dla silnika
/// nieodróżnialne od wiadomości bez sygnału.
#[derive(Debug, Clone)]
pub enum Zdarzenie {
    Wiadomosc(IncomingMessage),
    Kasowanie(Kasowanie),
}

/// Zegar ścienny w ms epoki — dla zdarzeń, którym Telegram nie daje daty.
fn teraz_ms() -> conduit_core::types::Ts {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// z <https://my.telegram.org> → API development tools
    pub api_id: i32,
    pub api_hash: String,
    /// plik sesji; kto go ma, ma dostęp do konta
    pub session_path: PathBuf,
    /// czy nadrabiać wiadomości z czasu, gdy bot był wyłączony
    pub catch_up: bool,
    /// pomijać wiadomości wysłane z tego konta (własne echo)
    pub ignore_outgoing: bool,
    /// ile aktualizacji wolno buforować, zanim zaczniemy je gubić
    pub queue_limit: usize,
}

pub const BUFOR_AKTUALIZACJI: usize = 10_000;

impl Default for ClientConfig {
    fn default() -> Self {
        ClientConfig {
            api_id: 0,
            api_hash: String::new(),
            session_path: PathBuf::from("conduit.session"),
            // Sygnał sprzed godzin jest nie tylko bezwartościowy — jest
            // GROŹNY: bot otworzyłby koszyk na cenę, której już dawno nie ma.
            catch_up: false,
            ignore_outgoing: true,
            queue_limit: BUFOR_AKTUALIZACJI,
        }
    }
}

pub struct TelegramClient {
    client: Client,
    session: Arc<FileSession>,
    updates: UpdateStream,
    pool: JoinHandle<()>,
    cfg: ClientConfig,

    /// nazwy czatów do `IncomingMessage::source_name`
    names: Mutex<HashMap<i64, String>>,
    /// czaty z włączonymi tematami — bez tego temat „Ogólny" jest nierozpoznawalny
    forums: Mutex<HashSet<i64>>,
    /// gdy `Some`, przepuszczamy tylko te źródła
    allow: Mutex<Option<HashSet<SourceKey>>>,
    /// co robić ze skasowaniami; domyślnie NIC — patrz [`PolitykaKasowania`]
    kasowanie: Mutex<PolitykaKasowania>,
    /// rachunek ciągłości numeracji `pts` — jedyny sposób, żeby zobaczyć
    /// aktualizacje ucięte przez `update_queue_limit`
    luki: Mutex<LicznikLuk>,
}

impl std::fmt::Debug for TelegramClient {
    /// `api_hash` i sesja celowo nie są wypisywane — to poświadczenia.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramClient")
            .field("api_id", &self.cfg.api_id)
            .field("sesja", &self.cfg.session_path)
            .field("zrodel", &self.names.lock().len())
            .field("forow", &self.forums.lock().len())
            .finish()
    }
}

impl TelegramClient {
    /// Podnosi połączenie z Telegramem. NIE loguje — o to trzeba poprosić
    /// osobno przez [`TelegramClient::qr_login`].
    pub async fn connect(cfg: ClientConfig) -> anyhow::Result<Self> {
        anyhow::ensure!(
            cfg.api_id != 0,
            "brak api_id — pobierz go z https://my.telegram.org"
        );
        anyhow::ensure!(!cfg.api_hash.is_empty(), "brak api_hash");

        let session = Arc::new(FileSession::load(&cfg.session_path)?);
        let SenderPool {
            runner,
            updates,
            handle,
        } = SenderPool::new(Arc::clone(&session), cfg.api_id);
        let client = Client::new(handle);
        let pool = tokio::spawn(async move {
            runner.run().await;
        });

        let stream = client
            .stream_updates(
                updates,
                UpdatesConfiguration {
                    catch_up: cfg.catch_up,
                    update_queue_limit: Some(cfg.queue_limit),
                },
            )
            .await
            .map_err(|e| anyhow::anyhow!("strumień aktualizacji: {e}"))?;

        Ok(TelegramClient {
            client,
            session,
            updates: stream,
            pool,
            cfg,
            names: Mutex::new(HashMap::new()),
            forums: Mutex::new(HashSet::new()),
            allow: Mutex::new(None),
            kasowanie: Mutex::new(PolitykaKasowania::default()),
            luki: Mutex::new(LicznikLuk::new()),
        })
    }

    pub fn raw(&self) -> &Client {
        &self.client
    }

    pub fn session(&self) -> &Arc<FileSession> {
        &self.session
    }

    /// Czy sesja z pliku niesie zalogowane konto?
    pub async fn is_authorized(&self) -> Result<bool, InvocationError> {
        self.client.is_authorized().await
    }

    /// Tworzy sterownik logowania kodem QR.
    pub fn qr_login(&self) -> QrLogin {
        QrLogin::new(
            self.client.clone(),
            Arc::clone(&self.session),
            self.cfg.api_id,
            self.cfg.api_hash.clone(),
        )
    }

    /// Strumień aktualizacji — potrzebny [`QrLogin::step`].
    pub fn updates_mut(&mut self) -> &mut UpdateStream {
        &mut self.updates
    }

    /// Pobiera dialogi i zapamiętuje nazwy oraz to, które czaty są forum.
    ///
    /// Warto wywołać zaraz po zalogowaniu: napełnia też cache peerów w sesji,
    /// bez którego nadrabianie zaległych aktualizacji nie działa.
    pub async fn refresh_dialogs(&self, limit: usize) -> Result<Vec<DialogEntry>, InvocationError> {
        let list = dialogs::list_dialogs(&self.client, limit).await?;
        {
            let mut n = self.names.lock();
            let mut f = self.forums.lock();
            for d in &list {
                n.insert(d.chat_id, d.name.clone());
                if d.is_forum {
                    f.insert(d.chat_id);
                }
            }
        }
        info!(
            dialogow = list.len(),
            forow = self.forums.lock().len(),
            "Telegram: lista dialogów odświeżona"
        );
        Ok(list)
    }

    /// Tematy grupy forum. Dla zwykłego czatu wraca pusta lista.
    ///
    /// Osobna metoda, a nie pole w `DialogEntry`: pobranie tematów to własne
    /// wywołanie API na KAŻDY czat, więc lista dialogów kosztowałaby wtedy
    /// tyle, ile jest forów na koncie — także wtedy, gdy nikt tematów nie ogląda.
    pub async fn topics_of(
        &self,
        dialog: &DialogEntry,
    ) -> Result<Vec<dialogs::TopicEntry>, InvocationError> {
        dialogs::list_topics(&self.client, dialog, 100).await
    }

    /// Ogranicza odbiór do wskazanych źródeł. `None` = wszystko.
    pub fn set_sources(&self, sources: Option<HashSet<SourceKey>>) {
        *self.allow.lock() = sources;
    }

    /// Ręczne dopisanie nazwy źródła (gdy czat nie wyszedł z listy dialogów).
    pub fn set_source_name(&self, chat_id: i64, name: impl Into<String>) {
        self.names.lock().insert(chat_id, name.into());
    }

    /// Oznacza czat jako forum (przydatne, gdy nie było go w dialogach).
    pub fn mark_forum(&self, chat_id: i64) {
        self.forums.lock().insert(chat_id);
    }

    /// Ustawia politykę reakcji na skasowanie wiadomości przez autora.
    ///
    /// Domyślnie [`PolitykaKasowania::Ignoruj`], czyli zachowanie sprzed
    /// zadania B11 co do bajtu. Ustawiać PO KAŻDYM podniesieniu klienta —
    /// przy ponownym logowaniu powstaje nowy `TelegramClient`, więc ustawienie
    /// z poprzedniego nie ma jak przetrwać.
    pub fn set_polityka_kasowania(&self, p: PolitykaKasowania) {
        *self.kasowanie.lock() = p;
    }

    pub fn polityka_kasowania(&self) -> PolitykaKasowania {
        *self.kasowanie.lock()
    }

    /// Ile aktualizacji przepadło od podniesienia klienta. Patrz [`crate::luki`].
    pub fn luki(&self) -> LukiMigawka {
        self.luki.lock().migawka()
    }

    /// Nazwa źródła do pokazania; gdy czatu nie było w dialogach, zostaje numer.
    fn nazwa_zrodla(&self, chat_id: i64) -> String {
        self.names
            .lock()
            .get(&chat_id)
            .cloned()
            .unwrap_or_else(|| chat_id.to_string())
    }

    /// Czeka na następne zdarzenie z obserwowanych źródeł.
    ///
    /// Pomija wszystko, co nie jest ani wiadomością, ani skasowaniem: statusy
    /// pisania, reakcje, odczytania. Edycje **przepuszcza** — mają ustawione
    /// `edit_of` i silnik poprawia dzięki nim istniejący koszyk zamiast
    /// otwierać nowy.
    ///
    /// Skasowania wychodzą stąd WYŁĄCZNIE wtedy, gdy polityka na to pozwala
    /// (domyślnie nie wychodzą wcale) — patrz [`PolitykaKasowania`].
    pub async fn next_zdarzenie(&mut self) -> Result<Zdarzenie, InvocationError> {
        loop {
            let u = self.updates.next().await?;
            let raw = u.raw();

            // NAJPIERW rachunek ciągłości, PRZED jakimkolwiek odsiewem.
            // Aktualizacje, których bot nie używa (odczytania, przypięcia),
            // też zużywają `pts` — pominięcie ich zamieniłoby ich normalny
            // przebieg w fałszywą lukę.
            // Zamek zdejmujemy OD RAZU, w osobnym wyrażeniu: w `if let`
            // tymczasowy strażnik żyje do końca bloku, a wpis do dziennika
            // trzymałby go wtedy dłużej, niż trzeba.
            let luka = self.luki.lock().zanotuj(raw);
            if let Some(l) = luka {
                warn!(
                    ile = l.ile,
                    oczekiwane = l.oczekiwane_pts,
                    otrzymane = l.otrzymane_pts,
                    "Telegram: LUKA W STRUMIENIU — tyle aktualizacji nie doszło \
                     (najczęstsza przyczyna: przepełniony bufor aktualizacji)"
                );
            }

            let polityka = *self.kasowanie.lock();
            if polityka != PolitykaKasowania::Ignoruj {
                if let Some(k) = incoming::kasowanie_z_update(raw) {
                    // Bez peera nie ma do czego przypiąć skasowania; zgadywanie
                    // czatu spadłoby na cudzy koszyk (patrz `Skasowane`).
                    let Some(chat_id) = k.chat_id else { continue };
                    if let Some(allow) = self.allow.lock().as_ref() {
                        // Temat forum nie jest znany — skasowanie nigdy go nie
                        // niesie. Przepuszczamy czat, jeśli obserwujemy Z NIEGO
                        // cokolwiek; dopasowanie do koszyka i tak idzie po
                        // `msg_id`, który jest unikalny w obrębie czatu.
                        if !allow.iter().any(|s| s.chat_id == chat_id) {
                            continue;
                        }
                    }
                    if k.msg_ids.is_empty() {
                        continue;
                    }
                    return Ok(Zdarzenie::Kasowanie(Kasowanie {
                        ts: teraz_ms(),
                        chat_id,
                        source_name: self.nazwa_zrodla(chat_id),
                        msg_ids: k.msg_ids,
                        polityka,
                    }));
                }
            }

            let Some(e) = incoming::from_update(raw) else {
                continue;
            };
            if e.outgoing && self.cfg.ignore_outgoing {
                continue;
            }
            let is_forum = self.forums.lock().contains(&e.chat_id);
            let e = e.with_forum_default(is_forum);

            if let Some(allow) = self.allow.lock().as_ref() {
                if !allow.contains(&e.source()) {
                    continue;
                }
            }
            let name = self.nazwa_zrodla(e.chat_id);
            return Ok(Zdarzenie::Wiadomosc(e.into_incoming(name)));
        }
    }

    /// Czeka na następną WIADOMOŚĆ, pomijając skasowania.
    ///
    /// Zostaje w tym kształcie dla odbiorców, którzy o skasowaniach nie chcą
    /// wiedzieć (kronika, eksporter). Przy domyślnej polityce jest to dokładnie
    /// ta sama pętla, co przed zadaniem B11.
    pub async fn next_message(&mut self) -> Result<IncomingMessage, InvocationError> {
        loop {
            match self.next_zdarzenie().await? {
                Zdarzenie::Wiadomosc(m) => return Ok(m),
                Zdarzenie::Kasowanie(_) => continue,
            }
        }
    }

    /// Pętla odbioru — podaje wiadomości do kanału.
    ///
    /// Kończy się, gdy odbiorca zniknie albo połączenie padnie.
    pub async fn pump(&mut self, tx: tokio::sync::mpsc::Sender<IncomingMessage>) {
        loop {
            match self.next_message().await {
                Ok(m) => {
                    if tx.send(m).await.is_err() {
                        info!("Telegram: odbiorca wiadomości zniknął — kończę pętlę");
                        return;
                    }
                }
                Err(e) => {
                    warn!(%e, "Telegram: pętla odbioru przerwana");
                    return;
                }
            }
        }
    }

    /// Wysyła powiadomienie na czat (opcjonalnie do konkretnego tematu).
    ///
    /// W grupie z tematami wiadomość bez `topic_id` wyląduje w „Ogólnym",
    /// a nie tam, gdzie leci sygnał.
    pub async fn notify(
        &self,
        chat_id: i64,
        topic_id: Option<i64>,
        text: &str,
    ) -> anyhow::Result<()> {
        let peer: PeerRef = dialogs::peer_from_chat_id(&self.session, chat_id)
            .await
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "czat {chat_id} nieznany sesji — wywołaj najpierw refresh_dialogs()"
                )
            })?;
        let msg = grammers_client::message::InputMessage::new()
            .text(text)
            // przynależność do tematu wyraża się tym samym polem, co odpowiedź
            .reply_to(topic_id.map(|t| t as i32));
        self.client.send_message(peer, msg).await?;
        Ok(())
    }

    /// Zapisuje sesję (klucz autoryzacyjny + stan aktualizacji).
    pub async fn save(&self) -> anyhow::Result<()> {
        // stan aktualizacji jest w strumieniu, nie w sesji — bez tego kroku
        // nadrabianie zaległości po restarcie zaczyna od zera
        if let Err(e) = self.updates.sync_update_state().await {
            warn!(%e, "Telegram: nie udało się zgrać stanu aktualizacji");
        }
        self.session.save()?;
        Ok(())
    }

    /// Zamyka połączenie i czeka na wygaszenie puli.
    pub async fn shutdown(self) {
        if let Err(e) = self.save().await {
            warn!(%e, "Telegram: zapis sesji przy zamykaniu nieudany");
        }
        self.client.disconnect();
        let _ = self.pool.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domyslna_konfiguracja_nie_nadrabia_zaleglosci() {
        let c = ClientConfig::default();
        assert!(
            !c.catch_up,
            "sygnał sprzed godzin otworzyłby koszyk na nieaktualnej cenie"
        );
        assert!(
            c.ignore_outgoing,
            "własne echo nie może wyzwalać transakcji"
        );
    }

    /// B14: 500 było za mało, a obcięcie bufora grammers zgłasza wyłącznie
    /// przez `log::warn!`, którego ten projekt nie słucha. Limit ma być
    /// wyraźnie wyższy — i taki sam jak w kronice, bo problem jest ten sam.
    #[test]
    fn bufor_aktualizacji_jest_wyrazie_wiekszy_niz_dawne_500() {
        assert!(
            ClientConfig::default().queue_limit >= 10_000,
            "przy zapchanym buforze grammers ucina OGON paczki i mówi o tym tylko przez log::warn!"
        );
        assert_eq!(ClientConfig::default().queue_limit, BUFOR_AKTUALIZACJI);
    }

    #[test]
    fn domyslna_polityka_kasowania_nic_nie_zmienia() {
        assert_eq!(PolitykaKasowania::default(), PolitykaKasowania::Ignoruj);
    }

    /// Najostrzejszy dostępny wariant sięga do LIMITÓW i nie dalej.
    ///
    /// Ten test jest tu po to, żeby zamknięcie pozycji na podstawie skasowanej
    /// wiadomości wymagało świadomego dopisania wariantu — a nie było czymś,
    /// co ktoś kiedyś doda „przy okazji". Kasowanie wiadomości to jedno
    /// kliknięcie autora; to za mało, żeby ruszać otwartą pozycją.
    ///
    /// STRAŻNIKIEM JEST `match` BEZ `_`, a nie liczba elementów tablicy.
    /// Pierwsza wersja tego testu sprawdzała `[Ignoruj, Ostrzez, KasujLimity]
    /// .len() == 3` — czyli długość tablicy, którą sama zbudowała. Przechodziła
    /// tak samo po dołożeniu wariantu `ZamknijPozycje`, więc nie pilnowała
    /// niczego. Wyczerpujące dopasowanie ROZSYPIE KOMPILACJĘ przy każdym nowym
    /// wariancie i zmusi autora do przeczytania uzasadnienia wyżej.
    #[test]
    fn polityka_kasowania_nie_ma_wariantu_zamykajacego_pozycje() {
        fn jak_daleko_siega(p: PolitykaKasowania) -> &'static str {
            match p {
                PolitykaKasowania::Ignoruj => "nigdzie",
                PolitykaKasowania::Ostrzez => "dziennik",
                PolitykaKasowania::KasujLimity => "limity",
            }
        }
        for p in [
            PolitykaKasowania::Ignoruj,
            PolitykaKasowania::Ostrzez,
            PolitykaKasowania::KasujLimity,
        ] {
            assert_ne!(
                jak_daleko_siega(p),
                "pozycje",
                "skasowana wiadomość NIE MOŻE ruszać otwartej pozycji"
            );
        }
    }

    #[tokio::test]
    async fn brak_poswiadczen_jest_wykrywany_od_razu() {
        let e = TelegramClient::connect(ClientConfig::default())
            .await
            .unwrap_err();
        assert!(e.to_string().contains("api_id"), "{e}");

        let e = TelegramClient::connect(ClientConfig {
            api_id: 1234,
            ..Default::default()
        })
        .await
        .unwrap_err();
        assert!(e.to_string().contains("api_hash"), "{e}");
    }
}
