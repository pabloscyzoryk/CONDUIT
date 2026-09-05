
use std::collections::HashMap;

use grammers_client::tl;

/// Skrzynka numeracji `pts`. Telegram prowadzi osobny licznik dla każdego
/// kanału i JEDEN wspólny dla wszystkiego pozostałego (rozmowy prywatne,
/// zwykłe grupy). Klucze są dokładnie te, których używa `grammers`
/// (`message_box::defs::Key`) — inaczej liczylibyśmy luki w innym miejscu,
/// niż powstają.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Skrzynka {
    Wspolna,
    /// SUROWY `channel_id`, nie identyfikator w konwencji Bot API — tak jak
    /// numeruje Telegram.
    Kanal(i64),
}

/// Pojedyncza wykryta dziura w numeracji.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Luka {
    pub skrzynka: Skrzynka,
    /// `pts`, którego się spodziewaliśmy po poprzedniej aktualizacji
    pub oczekiwane_pts: i32,
    /// `pts`, który faktycznie przyszedł
    pub otrzymane_pts: i32,
    /// ile zdarzeń przepadło
    pub ile: u32,
}

/// Migawka do odczytu z zewnątrz — same fakty, bez ocen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LukiMigawka {
    /// łączna liczba zdarzeń, które nie doszły
    pub zgubione: u64,
    /// ile razy wykryliśmy dziurę (jedna dziura bywa wielozdarzeniowa)
    pub luk: u32,
    /// kiedy ostatnia dziura, ms epoki; 0 = nigdy
    pub ostatnia_luka_ms: i64,
}

#[derive(Debug, Default)]
pub struct LicznikLuk {
    ostatni_pts: HashMap<Skrzynka, i32>,
    zgubione: u64,
    luk: u32,
    ostatnia_luka_ms: i64,
}

impl LicznikLuk {
    pub fn new() -> Self {
        Self::default()
    }

    fn teraz_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    /// Bierze aktualizację pod rachunek. Wraca `Some(luka)` DOKŁADNIE wtedy,
    /// gdy między nią a poprzednią przepadły zdarzenia.
    ///
    /// Wywoływać na KAŻDEJ aktualizacji ze strumienia, także na tych, których
    /// bot nie używa (odczytania, przypięcia). One też zużywają `pts` —
    /// pominięcie ich zamieniłoby ich normalny przebieg w fałszywą lukę.
    pub fn zanotuj(&mut self, u: &tl::enums::Update) -> Option<Luka> {
        let (skrzynka, pts, count) = pts_z_update(u)?;

        // Numeracja liczy się od 1; zero to znacznik „ta aktualizacja powstała
        // z getDifference i jest poza numeracją" (grammers: NO_PTS). Kasujemy
        // wtedy punkt odniesienia tej skrzynki — biblioteka przestawiła już
        // swój `pts` na stan PO nadrobieniu, więc nasz jest nieaktualny
        // i pierwsza zwykła aktualizacja po nadrobieniu zgłosiłaby lukę
        // wielkości całego nadrobienia. Szczegóły w nagłówku modułu.
        if pts <= 0 {
            self.ostatni_pts.remove(&skrzynka);
            return None;
        }

        let poprzedni = match self.ostatni_pts.insert(skrzynka, pts) {
            // Pierwsza aktualizacja z tej skrzynki wyznacza punkt odniesienia.
            // Nie da się orzec, ile przepadło PRZED podłączeniem się — i dobrze,
            // bo zgadywanie robiłoby z licznika generator fałszywych alarmów.
            None => return None,
            Some(p) => p,
        };

        // Powtórka albo aktualizacja spoza kolejności: `pts` nie idzie w przód.
        // Nic nie zginęło, a odejmowanie dałoby ujemną „lukę". Cofamy zapis,
        // żeby punktem odniesienia został ten wyższy, już widziany numer.
        if pts <= poprzedni {
            self.ostatni_pts.insert(skrzynka, poprzedni);
            return None;
        }

        let oczekiwane = poprzedni.saturating_add(count);
        if pts <= oczekiwane {
            return None;
        }

        let ile = (pts - oczekiwane) as u32;
        self.zgubione += ile as u64;
        self.luk += 1;
        self.ostatnia_luka_ms = Self::teraz_ms();
        Some(Luka {
            skrzynka,
            oczekiwane_pts: oczekiwane,
            otrzymane_pts: pts,
            ile,
        })
    }

    pub fn migawka(&self) -> LukiMigawka {
        LukiMigawka {
            zgubione: self.zgubione,
            luk: self.luk,
            ostatnia_luka_ms: self.ostatnia_luka_ms,
        }
    }
}

/// Kanał, do którego należy wiadomość — `None` dla rozmów i zwykłych grup.
fn kanal_wiadomosci(m: &tl::enums::Message) -> Option<i64> {
    let peer = match m {
        tl::enums::Message::Message(x) => Some(&x.peer_id),
        tl::enums::Message::Service(x) => Some(&x.peer_id),
        tl::enums::Message::Empty(x) => x.peer_id.as_ref(),
    }?;
    match peer {
        tl::enums::Peer::Channel(c) => Some(c.channel_id),
        _ => None,
    }
}

/// Mapowanie aktualizacji na `(skrzynka, pts, pts_count)`.
///
/// Idzie za `grammers_session::message_box::PtsInfo::from_update` wariant po
/// wariancie, a nie „na oko". Gdyby nasza lista zgubiła wariant niosący
/// `pts_count > 0`, jego normalny przebieg wyglądałby jak luka, której nie ma.
///
/// # Czego tu ŚWIADOMIE NIE MA
///
/// * **Skrzynka `Secondary` (`qts`)**: `updateNewEncryptedMessage`,
///   `updateChatParticipant`, `updateChannelParticipant`, `updateBotStopped`,
///   `updateBotChatInviteRequester`. Numeracja `qts` jest osobna od `pts`
///   i nie przechodzi tędy ani jeden sygnał handlowy.
/// * **Warianty z `count = 0`**: `updateChannelTooLong` i
///   `updateReadChannelInbox`. One nie ZUŻYWAJĄ numeracji — niosą tylko
///   bieżący `pts` skrzynki. Policzenie ich dałoby luki tam, gdzie sama
///   biblioteka widzi dziurę, idzie po `getDifference` i wszystko odzyskuje
///   (a odzyskane wiadomości i tak przychodzą jako NO_PTS).
///
/// Oba pominięcia mogą co najwyżej ZANIŻYĆ licznik, nigdy go nie zawyżą —
/// i to jest właściwa strona błędu dla liczby, która ma alarmować.
fn pts_z_update(u: &tl::enums::Update) -> Option<(Skrzynka, i32, i32)> {
    use tl::enums::Update as U;
    match u {
        // ---------- skrzynka wspólna ----------
        U::NewMessage(x) => Some((Skrzynka::Wspolna, x.pts, x.pts_count)),
        U::EditMessage(x) => Some((Skrzynka::Wspolna, x.pts, x.pts_count)),
        U::DeleteMessages(x) => Some((Skrzynka::Wspolna, x.pts, x.pts_count)),
        U::ReadHistoryInbox(x) => Some((Skrzynka::Wspolna, x.pts, x.pts_count)),
        U::ReadHistoryOutbox(x) => Some((Skrzynka::Wspolna, x.pts, x.pts_count)),
        U::ReadMessagesContents(x) => Some((Skrzynka::Wspolna, x.pts, x.pts_count)),
        U::WebPage(x) => Some((Skrzynka::Wspolna, x.pts, x.pts_count)),
        U::FolderPeers(x) => Some((Skrzynka::Wspolna, x.pts, x.pts_count)),
        U::PinnedMessages(x) => Some((Skrzynka::Wspolna, x.pts, x.pts_count)),

        // ---------- skrzynki kanałów ----------
        // Kanał bierzemy Z WIADOMOŚCI, nie z osobnego pola — tak samo jak
        // grammers. Wiadomość bez kanału (rozmowa) nie należy do tej skrzynki.
        U::NewChannelMessage(x) => {
            kanal_wiadomosci(&x.message).map(|k| (Skrzynka::Kanal(k), x.pts, x.pts_count))
        }
        U::EditChannelMessage(x) => {
            kanal_wiadomosci(&x.message).map(|k| (Skrzynka::Kanal(k), x.pts, x.pts_count))
        }
        U::DeleteChannelMessages(x) => Some((Skrzynka::Kanal(x.channel_id), x.pts, x.pts_count)),
        U::ChannelWebPage(x) => Some((Skrzynka::Kanal(x.channel_id), x.pts, x.pts_count)),
        U::PinnedChannelMessages(x) => Some((Skrzynka::Kanal(x.channel_id), x.pts, x.pts_count)),

        // Wszystko pozostałe (pisanie, reakcje, statusy) nie zużywa `pts`.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kanal(id: i64) -> tl::enums::Peer {
        tl::enums::Peer::Channel(tl::types::PeerChannel { channel_id: id })
    }

    /// Nowa wiadomość na kanale o zadanym `pts`. `MessageEmpty` wystarcza:
    /// licznik luk patrzy WYŁĄCZNIE na numerację, nigdy na treść.
    fn nowa(kanal_id: i64, pts: i32, count: i32) -> tl::enums::Update {
        tl::enums::Update::NewChannelMessage(tl::types::UpdateNewChannelMessage {
            message: tl::enums::Message::Empty(tl::types::MessageEmpty {
                id: 1,
                peer_id: Some(kanal(kanal_id)),
            }),
            pts,
            pts_count: count,
        })
    }

    fn skasowane(kanal_id: i64, ile: usize, pts: i32) -> tl::enums::Update {
        tl::enums::Update::DeleteChannelMessages(tl::types::UpdateDeleteChannelMessages {
            channel_id: kanal_id,
            messages: (0..ile as i32).collect(),
            pts,
            pts_count: ile as i32,
        })
    }

    #[test]
    fn pierwsza_aktualizacja_tylko_ustawia_punkt_odniesienia() {
        let mut l = LicznikLuk::new();
        // Ile przepadło ZANIM się podłączyliśmy, nie da się orzec — i nie
        // wolno zgadywać, bo każdy start bota byłby wtedy „awarią".
        assert_eq!(l.zanotuj(&nowa(7, 5_000, 1)), None);
        assert_eq!(l.migawka(), LukiMigawka::default());
    }

    #[test]
    fn ciagly_strumien_nie_zglasza_niczego() {
        let mut l = LicznikLuk::new();
        l.zanotuj(&nowa(7, 100, 1));
        for pts in 101..=110 {
            assert_eq!(l.zanotuj(&nowa(7, pts, 1)), None, "pts {pts}");
        }
        assert_eq!(l.migawka().zgubione, 0);
        assert_eq!(l.migawka().luk, 0);
    }

    #[test]
    fn dziura_w_numeracji_jest_policzona_co_do_sztuki() {
        let mut l = LicznikLuk::new();
        l.zanotuj(&nowa(7, 100, 1));
        // Kolejna aktualizacja wnosi 1 zdarzenie, ale numeracja skoczyła o 5 —
        // czyli 4 zdarzenia przepadły po drodze.
        let luka = l
            .zanotuj(&nowa(7, 105, 1))
            .expect("dziura musi być widoczna");
        assert_eq!(luka.ile, 4);
        assert_eq!(luka.oczekiwane_pts, 101);
        assert_eq!(luka.otrzymane_pts, 105);
        assert_eq!(luka.skrzynka, Skrzynka::Kanal(7));

        let m = l.migawka();
        assert_eq!(m.zgubione, 4);
        assert_eq!(m.luk, 1);
        assert!(m.ostatnia_luka_ms > 0, "moment dziury ma trafić do migawki");
    }

    #[test]
    fn paczka_wielozdarzeniowa_nie_jest_dziura() {
        let mut l = LicznikLuk::new();
        l.zanotuj(&nowa(7, 100, 1));
        // Skasowanie trzech wiadomości naraz przesuwa pts o 3 — legalnie.
        assert_eq!(l.zanotuj(&skasowane(7, 3, 103)), None);
        assert_eq!(l.zanotuj(&nowa(7, 104, 1)), None);
        assert_eq!(l.migawka().zgubione, 0);
    }

    #[test]
    fn kanaly_licza_sie_calkiem_osobno() {
        let mut l = LicznikLuk::new();
        // Dwa kanały o zupełnie różnych numeracjach, przeplatane. Wspólny
        // licznik zgłosiłby tu lawinę dziur, których nie ma.
        l.zanotuj(&nowa(1, 100, 1));
        l.zanotuj(&nowa(2, 900_000, 1));
        assert_eq!(l.zanotuj(&nowa(1, 101, 1)), None);
        assert_eq!(l.zanotuj(&nowa(2, 900_001, 1)), None);
        assert_eq!(l.zanotuj(&nowa(1, 102, 1)), None);
        assert_eq!(l.migawka().zgubione, 0);

        // ...a dziura w JEDNYM z nich dalej jest widoczna
        assert!(l.zanotuj(&nowa(2, 900_010, 1)).is_some());
        assert_eq!(l.migawka().zgubione, 8);
    }

    #[test]
    fn skrzynka_wspolna_jest_niezalezna_od_kanalow() {
        let mut l = LicznikLuk::new();
        l.zanotuj(&nowa(1, 100, 1));
        let wspolna = tl::enums::Update::DeleteMessages(tl::types::UpdateDeleteMessages {
            messages: vec![1],
            pts: 50,
            pts_count: 1,
        });
        assert_eq!(l.zanotuj(&wspolna), None, "inna skrzynka, inna numeracja");
        assert_eq!(l.migawka().zgubione, 0);
    }

    #[test]
    fn aktualizacja_z_getdifference_nie_klamie() {
        // grammers składa aktualizacje odtworzone z `getDifference` sam,
        // wstawiając pts = 0 (NO_PTS). Liczenie ich razem z resztą zamieniłoby
        // każde nadrobienie zaległości w „luka na 100 tysięcy zdarzeń".
        let mut l = LicznikLuk::new();
        l.zanotuj(&nowa(7, 100, 1));
        assert_eq!(l.zanotuj(&nowa(7, 0, 0)), None);
        assert_eq!(l.migawka().zgubione, 0);
        assert_eq!(
            l.zanotuj(&nowa(7, 101, 1)),
            None,
            "zero nie mogło zgłosić luki"
        );
    }

    #[test]
    fn nadrobienie_zaleglosci_nie_jest_luka() {
        let mut l = LicznikLuk::new();
        l.zanotuj(&nowa(7, 100, 1));
        for _ in 0..50 {
            assert_eq!(l.zanotuj(&nowa(7, 0, 0)), None);
        }
        assert_eq!(
            l.zanotuj(&nowa(7, 151, 1)),
            None,
            "nadrobione zaległości NIE SĄ zgubionymi aktualizacjami"
        );
        assert_eq!(l.migawka().zgubione, 0);
        assert_eq!(l.migawka().luk, 0);

        // ...a licznik dalej działa: od nowego punktu odniesienia (151)
        // prawdziwa dziura ma być widoczna co do sztuki.
        let luka = l
            .zanotuj(&nowa(7, 155, 1))
            .expect("po zakotwiczeniu dziury dalej widać");
        assert_eq!(luka.ile, 3);
        assert_eq!(l.migawka().zgubione, 3);
    }

    /// Nadrobienie na JEDNYM kanale nie ma prawa rozstroić licznika drugiego —
    /// inaczej jedno nadrobienie kasowałoby nadzór nad całą resztą źródeł.
    #[test]
    fn nadrobienie_kasuje_punkt_odniesienia_tylko_swojej_skrzynki() {
        let mut l = LicznikLuk::new();
        l.zanotuj(&nowa(1, 100, 1));
        l.zanotuj(&nowa(2, 200, 1));
        assert_eq!(l.zanotuj(&nowa(1, 0, 0)), None);
        // kanał 2 nie brał udziału w nadrabianiu — jego dziura ma być policzona
        let luka = l
            .zanotuj(&nowa(2, 205, 1))
            .expect("kanał 2 mierzony niezależnie");
        assert_eq!(luka.ile, 4);
        assert_eq!(luka.skrzynka, Skrzynka::Kanal(2));
    }

    #[test]
    fn powtorka_nie_odejmuje_i_nie_gubi_punktu_odniesienia() {
        let mut l = LicznikLuk::new();
        l.zanotuj(&nowa(7, 100, 1));
        l.zanotuj(&nowa(7, 101, 1));
        // ta sama aktualizacja jeszcze raz (re-delivery)
        assert_eq!(l.zanotuj(&nowa(7, 101, 1)), None);
        assert_eq!(l.zanotuj(&nowa(7, 100, 1)), None, "cofnięcie to nie luka");
        assert_eq!(l.migawka().zgubione, 0);
        // po powtórce dalej mierzymy od NAJWYŻSZEGO widzianego numeru
        assert_eq!(l.zanotuj(&nowa(7, 102, 1)), None);
        assert_eq!(l.migawka().zgubione, 0);
    }

    #[test]
    fn aktualizacje_bez_pts_sa_niewidzialne() {
        let mut l = LicznikLuk::new();
        l.zanotuj(&nowa(7, 100, 1));
        assert_eq!(l.zanotuj(&tl::enums::Update::LoginToken), None);
        assert_eq!(l.zanotuj(&nowa(7, 101, 1)), None, "nic się nie zużyło");
        assert_eq!(l.migawka().zgubione, 0);
    }

    #[test]
    fn dziury_sumuja_sie_przez_caly_czas_pracy() {
        let mut l = LicznikLuk::new();
        l.zanotuj(&nowa(7, 100, 1));
        l.zanotuj(&nowa(7, 103, 1)); // -2
        l.zanotuj(&nowa(7, 110, 1)); // -6
        let m = l.migawka();
        assert_eq!(m.zgubione, 8);
        assert_eq!(m.luk, 2);
    }
}
