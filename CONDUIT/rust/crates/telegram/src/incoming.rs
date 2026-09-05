//! Zamiana aktualizacji Telegrama na `IncomingMessage` z rdzenia.
//!
//! To jest najbardziej podstępny plik w całym crate. Trzy rzeczy, na których
//! poprzedni bot się przewracał:
//!
//! ### 1. Edycje
//! W kanale ATFX co trzeci sygnał bywa poprawiany PO wysłaniu. `UpdateEditMessage`
//! niesie tę samą wiadomość z tym samym `id` — jeśli potraktować ją jak nową,
//! bot otwiera drugi koszyk na ten sam sygnał. Dlatego `edit_of` niesie
//! identyfikator edytowanej wiadomości, a silnik poprawia istniejący koszyk.
//!
//! ### 2. Odpowiedzi
//! Komunikaty zarządzające („zamknij połowę", „SL na BE") to zwykle ODPOWIEDZI
//! na wiadomość z sygnałem. Bez `reply_to` nie da się ich przypisać do koszyka.
//!
//! ### 3. Tematy forum — i pułapka, w którą wpada się zawsze
//! W grupie z tematami pole `reply_to_msg_id` ma **dwa różne znaczenia**:
//!
//! * gdy `reply_to_top_id` jest ustawione → wiadomość jest odpowiedzią wewnątrz
//!   tematu: temat = `reply_to_top_id`, odpowiedź = `reply_to_msg_id`;
//! * gdy `reply_to_top_id` jest puste, a `forum_topic` ustawione → wiadomość
//!   jest wysłana WPROST do tematu: temat = `reply_to_msg_id`,
//!   a odpowiedzi **nie ma żadnej**.
//!
//! Zignorowanie tego drugiego przypadku daje bota, który każdą wiadomość
//! w temacie uważa za odpowiedź na wiadomość o numerze tematu — i przypina
//! sygnały do przypadkowych koszyków.

use conduit_core::engine::IncomingMessage;
use conduit_core::types::{SourceKey, Ts};
use grammers_client::session::types::PeerId;
use grammers_client::tl;

/// Identyfikator tematu „Ogólny" w grupie z forum. Telegram nie oznacza go
/// jawnie — wiadomości w nim nie mają nagłówka odpowiedzi.
pub const GENERAL_TOPIC: i64 = 1;

/// Wyciąg z aktualizacji, jeszcze bez nazwy źródła (tę zna dopiero klient).
#[derive(Debug, Clone, PartialEq)]
pub struct Extracted {
    /// identyfikator czatu w konwencji Bot API (kanał = `-100…`)
    pub chat_id: i64,
    pub topic_id: Option<i64>,
    pub msg_id: i64,
    pub reply_to: Option<i64>,
    pub edit_of: Option<i64>,
    pub text: String,
    pub ts: Ts,
    /// wiadomość wysłana przez nas — bot nie powinien handlować własnym echem
    pub outgoing: bool,
    /// nagłówek odpowiedzi jawnie mówił o temacie forum
    pub forum: bool,
}

impl Extracted {
    pub fn source(&self) -> SourceKey {
        SourceKey::new(self.chat_id, self.topic_id)
    }

    /// Domyka temat „Ogólny": wiadomość bez nagłówka odpowiedzi w grupie,
    /// o której WIEMY, że jest forum, należy do tematu 1.
    ///
    /// Sama aktualizacja tego nie zdradza — trzeba wiedzieć z listy dialogów,
    /// że kanał ma włączone tematy.
    pub fn with_forum_default(mut self, chat_is_forum: bool) -> Self {
        if chat_is_forum && self.topic_id.is_none() {
            self.topic_id = Some(GENERAL_TOPIC);
        }
        self
    }

    pub fn into_incoming(self, source_name: impl Into<String>) -> IncomingMessage {
        IncomingMessage {
            ts: self.ts,
            source: SourceKey::new(self.chat_id, self.topic_id),
            source_name: source_name.into(),
            msg_id: self.msg_id,
            reply_to: self.reply_to,
            edit_of: self.edit_of,
            text: self.text,
        }
    }
}

/// Rozkłada nagłówek odpowiedzi na `(temat, odpowiedź, czy_forum)`.
///
/// Wydzielone osobno, bo to jest cała trudność tego modułu — i jedyne miejsce,
/// które naprawdę warto obłożyć testami.
pub fn split_reply(h: Option<&tl::enums::MessageReplyHeader>) -> (Option<i64>, Option<i64>, bool) {
    match h {
        Some(tl::enums::MessageReplyHeader::Header(r)) => {
            if r.forum_topic {
                match r.reply_to_top_id {
                    // odpowiedź WEWNĄTRZ tematu
                    Some(top) => (Some(top as i64), r.reply_to_msg_id.map(|x| x as i64), true),
                    // wiadomość wysłana wprost do tematu — to NIE jest odpowiedź
                    None => (r.reply_to_msg_id.map(|x| x as i64), None, true),
                }
            } else {
                // zwykły kanał albo grupa bez tematów
                (None, r.reply_to_msg_id.map(|x| x as i64), false)
            }
        }
        // odpowiedź na relację (story) — dla bota bez znaczenia
        Some(tl::enums::MessageReplyHeader::MessageReplyStoryHeader(_)) => (None, None, false),
        None => (None, None, false),
    }
}

/// Wyciąga treść z surowej wiadomości.
///
/// `is_edit` pochodzi z rodzaju aktualizacji, nie z samej wiadomości: pole
/// `edit_date` bywa ustawione również na wiadomościach przysłanych jako nowe
/// (np. po dołożeniu podglądu odnośnika), więc opieranie się na nim dawałoby
/// fałszywe edycje.
pub fn from_message(m: &tl::enums::Message, is_edit: bool) -> Option<Extracted> {
    let tl::enums::Message::Message(msg) = m else {
        // MessageEmpty i MessageService nie niosą treści sygnału
        return None;
    };
    let (topic_id, reply_to, forum) = split_reply(msg.reply_to.as_ref());
    let chat_id = PeerId::from(msg.peer_id.clone()).bot_api_dialog_id_unchecked();
    let id = msg.id as i64;

    // czas zdarzenia: dla edycji liczy się chwila POPRAWKI, nie pierwotnej wysyłki
    let secs = if is_edit {
        msg.edit_date.unwrap_or(msg.date)
    } else {
        msg.date
    };

    Some(Extracted {
        chat_id,
        topic_id,
        msg_id: id,
        reply_to,
        edit_of: if is_edit { Some(id) } else { None },
        text: msg.message.clone(),
        ts: secs as Ts * 1000,
        outgoing: msg.out,
        forum,
    })
}

/// Wyciąga treść z aktualizacji. `None` dla wszystkiego, co nie jest
/// wiadomością tekstową (statusy pisania, reakcje, usunięcia…).
pub fn from_update(u: &tl::enums::Update) -> Option<Extracted> {
    use tl::enums::Update as U;
    match u {
        U::NewMessage(x) => from_message(&x.message, false),
        U::NewChannelMessage(x) => from_message(&x.message, false),
        U::EditMessage(x) => from_message(&x.message, true),
        U::EditChannelMessage(x) => from_message(&x.message, true),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skasowane {
    /// czat w konwencji Bot API; `None` = Telegram nie podał peera
    pub chat_id: Option<i64>,
    /// numery skasowanych wiadomości
    pub msg_ids: Vec<i64>,
}

/// Wyciąga skasowanie z aktualizacji. `None` dla wszystkiego innego.
///
/// Świadomie osobna funkcja, a nie kolejna gałąź [`from_update`]: skasowanie
/// NIE JEST wiadomością i nie ma treści, którą dałoby się sparsować. Wepchnięte
/// w `IncomingMessage` z pustym tekstem wyglądałoby dla silnika jak wiadomość
/// bez sygnału — czyli jak nic.
pub fn kasowanie_z_update(u: &tl::enums::Update) -> Option<Skasowane> {
    use tl::enums::Update as U;
    match u {
        U::DeleteChannelMessages(x) => Some(Skasowane {
            // ta sama konwersja co dla wiadomości, żeby identyfikator czatu
            // zgadzał się co do znaku z tym w `SourceKey`
            chat_id: Some(
                PeerId::from(tl::types::PeerChannel {
                    channel_id: x.channel_id,
                })
                .bot_api_dialog_id_unchecked(),
            ),
            msg_ids: x.messages.iter().map(|m| *m as i64).collect(),
        }),
        U::DeleteMessages(x) => Some(Skasowane {
            chat_id: None,
            msg_ids: x.messages.iter().map(|m| *m as i64).collect(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `tl::types::Message` ma pięćdziesiąt pól i nie implementuje `Default`.
    /// Ten pomocnik pozwala testom mówić tylko o tym, co istotne.
    fn msg(
        id: i32,
        peer: tl::enums::Peer,
        text: &str,
        reply: Option<tl::enums::MessageReplyHeader>,
    ) -> tl::enums::Message {
        tl::enums::Message::Message(tl::types::Message {
            out: false,
            mentioned: false,
            media_unread: false,
            silent: false,
            post: false,
            from_scheduled: false,
            legacy: false,
            edit_hide: false,
            pinned: false,
            noforwards: false,
            invert_media: false,
            offline: false,
            video_processing_pending: false,
            paid_suggested_post_stars: false,
            paid_suggested_post_ton: false,
            id,
            from_id: None,
            from_boosts_applied: None,
            from_rank: None,
            peer_id: peer,
            saved_peer_id: None,
            fwd_from: None,
            via_bot_id: None,
            via_business_bot_id: None,
            guestchat_via_from: None,
            reply_to: reply,
            date: 1_700_000_000,
            message: text.to_string(),
            media: None,
            reply_markup: None,
            entities: None,
            views: None,
            forwards: None,
            replies: None,
            edit_date: None,
            post_author: None,
            grouped_id: None,
            reactions: None,
            restriction_reason: None,
            ttl_period: None,
            quick_reply_shortcut_id: None,
            effect: None,
            factcheck: None,
            report_delivery_until_date: None,
            paid_message_stars: None,
            suggested_post: None,
            schedule_repeat_period: None,
            summary_from_language: None,
            rich_message: None,
        })
    }

    fn kanal(id: i64) -> tl::enums::Peer {
        tl::enums::Peer::Channel(tl::types::PeerChannel { channel_id: id })
    }

    /// Nagłówek odpowiedzi z jawnie ustawionymi trzema polami, które decydują.
    fn naglowek(
        forum_topic: bool,
        reply_to_msg_id: Option<i32>,
        reply_to_top_id: Option<i32>,
    ) -> tl::enums::MessageReplyHeader {
        tl::enums::MessageReplyHeader::Header(tl::types::MessageReplyHeader {
            reply_to_scheduled: false,
            forum_topic,
            quote: false,
            reply_to_ephemeral: false,
            reply_to_msg_id,
            reply_to_peer_id: None,
            reply_from: None,
            reply_media: None,
            reply_to_top_id,
            quote_text: None,
            quote_entities: None,
            quote_offset: None,
            todo_item_id: None,
            poll_option: None,
        })
    }

    // ---------- nagłówek odpowiedzi ----------

    #[test]
    fn zwykly_kanal_nie_ma_tematu() {
        let (t, r, f) = split_reply(Some(&naglowek(false, Some(55), None)));
        assert_eq!(t, None);
        assert_eq!(r, Some(55));
        assert!(!f);
    }

    #[test]
    fn wiadomosc_wprost_do_tematu_to_NIE_jest_odpowiedz() {
        // To jest ta pułapka: reply_to_msg_id niesie NUMER TEMATU, a nie
        // wiadomość, na którą ktoś odpowiada.
        let (t, r, f) = split_reply(Some(&naglowek(true, Some(42), None)));
        assert_eq!(t, Some(42), "temat");
        assert_eq!(r, None, "to nie jest odpowiedź na wiadomość 42");
        assert!(f);
    }

    #[test]
    fn odpowiedz_wewnatrz_tematu_ma_oba_pola() {
        let (t, r, f) = split_reply(Some(&naglowek(true, Some(99), Some(42))));
        assert_eq!(t, Some(42), "temat z reply_to_top_id");
        assert_eq!(r, Some(99), "odpowiedź na wiadomość 99");
        assert!(f);
    }

    #[test]
    fn brak_naglowka_to_brak_wszystkiego() {
        assert_eq!(split_reply(None), (None, None, false));
    }

    #[test]
    fn odpowiedz_na_relacje_jest_pomijana() {
        let h = tl::enums::MessageReplyHeader::MessageReplyStoryHeader(
            tl::types::MessageReplyStoryHeader {
                peer: kanal(1),
                story_id: 7,
            },
        );
        assert_eq!(split_reply(Some(&h)), (None, None, false));
    }

    // ---------- pełna ścieżka aktualizacji ----------

    #[test]
    fn nowa_wiadomosc_na_kanale() {
        let u = tl::enums::Update::NewChannelMessage(tl::types::UpdateNewChannelMessage {
            message: msg(10, kanal(1234), "BUY GOLD 4000-4005 SL 3990", None),
            pts: 0,
            pts_count: 0,
        });
        let e = from_update(&u).unwrap();
        assert_eq!(e.msg_id, 10);
        assert_eq!(e.edit_of, None, "nowa wiadomość nie jest edycją");
        assert_eq!(e.reply_to, None);
        assert_eq!(e.topic_id, None);
        assert_eq!(e.text, "BUY GOLD 4000-4005 SL 3990");
        assert_eq!(e.ts, 1_700_000_000_000, "czas w milisekundach");
        // kanał w konwencji Bot API dostaje przedrostek -100
        assert_eq!(e.chat_id, -1_000_000_001_234);
    }

    #[test]
    fn edycja_niesie_edit_of_o_tym_samym_id() {
        // Bez tego bot otworzyłby DRUGI koszyk na poprawiony sygnał.
        let u = tl::enums::Update::EditChannelMessage(tl::types::UpdateEditChannelMessage {
            message: msg(10, kanal(1234), "BUY GOLD 4001-4006 SL 3991", None),
            pts: 0,
            pts_count: 0,
        });
        let e = from_update(&u).unwrap();
        assert_eq!(e.msg_id, 10);
        assert_eq!(e.edit_of, Some(10));
    }

    #[test]
    fn edycja_uzywa_czasu_poprawki() {
        let mut m = msg(10, kanal(1), "poprawka", None);
        if let tl::enums::Message::Message(ref mut x) = m {
            x.edit_date = Some(1_700_000_600);
        }
        let u = tl::enums::Update::EditChannelMessage(tl::types::UpdateEditChannelMessage {
            message: m,
            pts: 0,
            pts_count: 0,
        });
        let e = from_update(&u).unwrap();
        assert_eq!(e.ts, 1_700_000_600_000);
    }

    #[test]
    fn nowa_wiadomosc_ignoruje_edit_date() {
        // Telegram ustawia edit_date także po doklejeniu podglądu odnośnika —
        // opieranie się na nim dawałoby fałszywe edycje.
        let mut m = msg(10, kanal(1), "sygnał", None);
        if let tl::enums::Message::Message(ref mut x) = m {
            x.edit_date = Some(1_700_000_600);
        }
        let u = tl::enums::Update::NewChannelMessage(tl::types::UpdateNewChannelMessage {
            message: m,
            pts: 0,
            pts_count: 0,
        });
        let e = from_update(&u).unwrap();
        assert_eq!(e.edit_of, None);
        assert_eq!(e.ts, 1_700_000_000_000, "czas pierwotnej wysyłki");
    }

    #[test]
    fn komunikat_zarzadzajacy_jest_odpowiedzia() {
        let u = tl::enums::Update::NewChannelMessage(tl::types::UpdateNewChannelMessage {
            message: msg(
                11,
                kanal(1),
                "close half",
                Some(naglowek(false, Some(10), None)),
            ),
            pts: 0,
            pts_count: 0,
        });
        let e = from_update(&u).unwrap();
        assert_eq!(e.reply_to, Some(10));
        assert_eq!(e.edit_of, None);
    }

    #[test]
    fn temat_trafia_do_klucza_zrodla() {
        let u = tl::enums::Update::NewChannelMessage(tl::types::UpdateNewChannelMessage {
            message: msg(
                50,
                kanal(777),
                "sygnał w temacie",
                Some(naglowek(true, Some(42), None)),
            ),
            pts: 0,
            pts_count: 0,
        });
        let e = from_update(&u).unwrap();
        let s = e.source();
        assert_eq!(s.chat_id, -1_000_000_000_777);
        assert_eq!(s.topic_id, Some(42));
        // temat jest NIEZALEŻNYM źródłem — własne koszyki, własny preset
        assert_eq!(s.as_string(), "-1000000000777:42");
    }

    #[test]
    fn temat_ogolny_domykany_gdy_wiemy_ze_to_forum() {
        let u = tl::enums::Update::NewChannelMessage(tl::types::UpdateNewChannelMessage {
            message: msg(5, kanal(9), "w Ogólnym", None),
            pts: 0,
            pts_count: 0,
        });
        let e = from_update(&u).unwrap();
        assert_eq!(e.topic_id, None, "sama aktualizacja tego nie zdradza");
        assert_eq!(
            e.clone().with_forum_default(true).topic_id,
            Some(GENERAL_TOPIC)
        );
        assert_eq!(e.with_forum_default(false).topic_id, None);
    }

    #[test]
    fn wiadomosc_z_tematem_nie_jest_domykana() {
        let e = Extracted {
            chat_id: -100,
            topic_id: Some(42),
            msg_id: 1,
            reply_to: None,
            edit_of: None,
            text: String::new(),
            ts: 0,
            outgoing: false,
            forum: true,
        };
        assert_eq!(e.with_forum_default(true).topic_id, Some(42));
    }

    #[test]
    fn wiadomosci_bez_tresci_sa_pomijane() {
        let empty = tl::enums::Message::Empty(tl::types::MessageEmpty {
            id: 1,
            peer_id: None,
        });
        assert!(from_message(&empty, false).is_none());
    }

    #[test]
    fn aktualizacje_nie_o_wiadomosciach_sa_pomijane() {
        let u = tl::enums::Update::LoginToken;
        assert!(from_update(&u).is_none());
    }

    #[test]
    fn wlasne_echo_jest_oznaczone() {
        let mut m = msg(1, kanal(1), "moja wiadomość", None);
        if let tl::enums::Message::Message(ref mut x) = m {
            x.out = true;
        }
        let e = from_message(&m, false).unwrap();
        assert!(
            e.outgoing,
            "wiadomość wysłana przez nas musi być rozpoznana"
        );
    }

    #[test]
    fn konwersja_do_incoming_message_zachowuje_wszystko() {
        let u = tl::enums::Update::NewChannelMessage(tl::types::UpdateNewChannelMessage {
            message: msg(
                50,
                kanal(777),
                "SELL 4100",
                Some(naglowek(true, Some(9), Some(42))),
            ),
            pts: 0,
            pts_count: 0,
        });
        let im = from_update(&u).unwrap().into_incoming("ATFX GOLD");
        assert_eq!(im.msg_id, 50);
        assert_eq!(im.reply_to, Some(9));
        assert_eq!(im.edit_of, None);
        assert_eq!(im.source.topic_id, Some(42));
        assert_eq!(im.source_name, "ATFX GOLD");
        assert_eq!(im.text, "SELL 4100");
        assert_eq!(im.ts, 1_700_000_000_000);
    }

    // ---------- kasowanie wiadomości (B11) ----------

    #[test]
    fn kasowanie_na_kanale_niesie_czat_w_tej_samej_konwencji_co_wiadomosc() {
        // Klucz rzeczy: identyfikator czatu MUSI zgadzać się co do znaku
        // z tym, który niesie wiadomość — inaczej odbiorca nie dopasuje
        // skasowania do koszyka otwartego na tym samym kanale.
        let wiad = from_update(&tl::enums::Update::NewChannelMessage(
            tl::types::UpdateNewChannelMessage {
                message: msg(10, kanal(1234), "BUY", None),
                pts: 0,
                pts_count: 0,
            },
        ))
        .unwrap();

        let u = tl::enums::Update::DeleteChannelMessages(tl::types::UpdateDeleteChannelMessages {
            channel_id: 1234,
            messages: vec![10, 11],
            pts: 0,
            pts_count: 2,
        });
        let k = kasowanie_z_update(&u).unwrap();
        assert_eq!(k.chat_id, Some(wiad.chat_id));
        assert_eq!(k.chat_id, Some(-1_000_000_001_234));
        assert_eq!(k.msg_ids, vec![10, 11]);
    }

    #[test]
    fn kasowanie_poza_kanalem_nie_zna_czatu() {
        // `UpdateDeleteMessages` NIE NIESIE PEERA — Telegram go nie podaje.
        // Udawanie, że wiemy, z którego czatu to skasowanie, byłoby zgadywaniem
        // na cudzym koszyku.
        let u = tl::enums::Update::DeleteMessages(tl::types::UpdateDeleteMessages {
            messages: vec![7],
            pts: 0,
            pts_count: 1,
        });
        let k = kasowanie_z_update(&u).unwrap();
        assert_eq!(k.chat_id, None);
        assert_eq!(k.msg_ids, vec![7]);
    }

    #[test]
    fn kasowanie_nie_jest_wiadomoscia_i_odwrotnie() {
        // Dwie ścieżki nie mogą się nachodzić: gdyby skasowanie wychodziło
        // z `from_update`, silnik dostałby wiadomość z pustą treścią.
        let kas =
            tl::enums::Update::DeleteChannelMessages(tl::types::UpdateDeleteChannelMessages {
                channel_id: 1,
                messages: vec![1],
                pts: 0,
                pts_count: 1,
            });
        assert!(
            from_update(&kas).is_none(),
            "skasowanie NIE jest wiadomością"
        );

        let wiad = tl::enums::Update::NewChannelMessage(tl::types::UpdateNewChannelMessage {
            message: msg(1, kanal(1), "BUY", None),
            pts: 0,
            pts_count: 1,
        });
        assert!(
            kasowanie_z_update(&wiad).is_none(),
            "wiadomość NIE jest skasowaniem"
        );
    }

    #[test]
    fn kasowanie_pustej_listy_jest_zauwazane_ale_puste() {
        // Telegram potrafi przysłać pustą listę (np. po wyczyszczeniu historii
        // przez kogoś, kto nie miał do czego). Odbiorca ma nie mieć nic do roboty.
        let u = tl::enums::Update::DeleteChannelMessages(tl::types::UpdateDeleteChannelMessages {
            channel_id: 5,
            messages: vec![],
            pts: 0,
            pts_count: 0,
        });
        let k = kasowanie_z_update(&u).unwrap();
        assert!(k.msg_ids.is_empty());
    }

    #[test]
    fn grupa_zwykla_ma_ujemny_identyfikator_bez_setki() {
        let u = tl::enums::Update::NewMessage(tl::types::UpdateNewMessage {
            message: msg(
                3,
                tl::enums::Peer::Chat(tl::types::PeerChat { chat_id: 5555 }),
                "hej",
                None,
            ),
            pts: 0,
            pts_count: 0,
        });
        let e = from_update(&u).unwrap();
        assert_eq!(e.chat_id, -5555);
    }
}
