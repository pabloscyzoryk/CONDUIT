//! Lista kanałów i tematów — do wyboru źródeł sygnału w interfejsie.
//!
//! Poza wygodą użytkownika ta lista pełni dwie techniczne role:
//!
//! 1. **Zasila cache peerów w sesji.** `grammers` zapisuje napotkane peery
//!    (`auto_cache_peers`), a bez nich strumień aktualizacji nie potrafi
//!    nadrobić zaległości po rozłączeniu — biblioteka mówi o tym wprost
//!    w dokumentacji `stream_updates`. Przejście po dialogach po zalogowaniu
//!    nie jest więc kosmetyką.
//! 2. **Mówi, które czaty są forum.** Aktualizacja z tematu „Ogólny" nie ma
//!    nagłówka odpowiedzi, więc bez tej wiedzy nie da się odróżnić jej od
//!    zwykłej wiadomości w grupie bez tematów.

use std::collections::HashSet;

use grammers_client::session::types::{PeerId, PeerRef};
use grammers_client::session::Session;
use grammers_client::{tl, Client, InvocationError};
use tracing::warn;

use crate::session::FileSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    User,
    Group,
    Channel,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DialogEntry {
    /// identyfikator w konwencji Bot API — ten sam, który trafia do `SourceKey`
    pub chat_id: i64,
    pub name: String,
    pub username: Option<String>,
    pub kind: DialogKind,
    /// czy grupa ma włączone tematy
    pub is_forum: bool,
    /// Identyfikator zdjęcia profilowego. `None` = czat go nie ma.
    ///
    /// To jest ZNACZNIK WERSJI zdjęcia: Telegram nadaje nowe `photo_id` przy
    /// każdej podmianie obrazka, więc porównanie z zapisanym w pamięci
    /// podręcznej wystarcza, żeby wykryć zmianę bez pobierania pliku.
    pub photo_id: Option<i64>,
    /// referencja do wywołań API (niesie access_hash)
    pub peer: PeerRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicEntry {
    pub chat_id: i64,
    pub topic_id: i64,
    pub title: String,
    pub closed: bool,
}

impl TopicEntry {
    /// Klucz źródła, pod którym ten temat występuje w silniku.
    pub fn source_key(&self) -> conduit_core::types::SourceKey {
        conduit_core::types::SourceKey::new(self.chat_id, Some(self.topic_id))
    }
}

/// Pobiera listę dialogów.
///
/// `limit == 0` znaczy „wszystkie". Konto z tysiącami rozmów potrafi się
/// pobierać długo, a do wyboru kanału sygnałowego pierwsze kilkaset wystarcza.
pub async fn list_dialogs(
    client: &Client,
    limit: usize,
) -> Result<Vec<DialogEntry>, InvocationError> {
    use grammers_client::peer::Peer;

    let mut out = Vec::new();
    let mut it = client.iter_dialogs();
    while let Some(d) = it.next().await? {
        let peer = d.peer();
        let (kind, is_forum) = match peer {
            Peer::User(_) => (DialogKind::User, false),
            Peer::Group(g) => {
                // mała grupa nie ma tematów; forum żyje tylko na supergrupach,
                // a te są kanałami typu megagroup
                let forum = match &g.raw {
                    tl::enums::Chat::Channel(c) => c.forum,
                    _ => false,
                };
                (DialogKind::Group, forum)
            }
            Peer::Channel(c) => (DialogKind::Channel, c.raw.forum),
        };
        // Sam identyfikator zdjęcia — BEZ pobierania pliku. Pobranie miniatury
        // kosztuje osobne wywołanie `upload.getFile` na każdy czat, a lista ma
        // 200+ pozycji; tutaj wystarczy wiedzieć, czy zdjęcie w ogóle jest.
        let photo_id = match peer {
            Peer::User(u) => u.photo().map(|p| p.photo_id),
            Peer::Group(g) => g.photo().map(|p| p.photo_id),
            Peer::Channel(c) => c.photo().map(|p| p.photo_id),
        };
        out.push(DialogEntry {
            chat_id: peer.id().bot_api_dialog_id_unchecked(),
            name: peer.name().unwrap_or_default().to_string(),
            username: peer.username().map(|s| s.to_string()),
            kind,
            is_forum,
            photo_id,
            peer: d.peer_ref(),
        });
        if limit > 0 && out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

/// Zbiór czatów z włączonymi tematami — do domknięcia tematu „Ogólny".
pub fn forum_chats(dialogs: &[DialogEntry]) -> HashSet<i64> {
    dialogs
        .iter()
        .filter(|d| d.is_forum)
        .map(|d| d.chat_id)
        .collect()
}

/// Pobiera tematy grupy forum.
///
/// Zwraca pustą listę, gdy czat nie jest forum — wołający nie musi tego
/// sprawdzać osobno.
pub async fn list_topics(
    client: &Client,
    dialog: &DialogEntry,
    limit: i32,
) -> Result<Vec<TopicEntry>, InvocationError> {
    if !dialog.is_forum {
        return Ok(Vec::new());
    }
    let res = client
        .invoke(&tl::functions::messages::GetForumTopics {
            peer: dialog.peer.into(),
            q: None,
            offset_date: 0,
            offset_id: 0,
            offset_topic: 0,
            limit: if limit > 0 { limit } else { 100 },
        })
        .await?;

    let tl::enums::messages::ForumTopics::Topics(t) = res;
    let mut out = Vec::with_capacity(t.topics.len());
    for topic in t.topics {
        match topic {
            tl::enums::ForumTopic::Topic(x) => out.push(TopicEntry {
                chat_id: dialog.chat_id,
                topic_id: x.id as i64,
                title: x.title,
                closed: x.closed,
            }),
            // temat usunięty — zostaje po nim tylko identyfikator
            tl::enums::ForumTopic::Deleted(x) => {
                warn!(topic = x.id, "temat usunięty — pomijam");
            }
        }
    }
    Ok(out)
}

/// Buduje `PeerRef` z identyfikatora Bot API przy pomocy cache'u sesji.
///
/// Zwraca `None`, gdy peer nie jest znany — bez `access_hash` Telegram nie
/// przyjmie żadnego wywołania dotyczącego tego czatu. Lekarstwem jest
/// wcześniejsze [`list_dialogs`], które zapełnia cache.
///
/// Sesję trzeba podać osobno: `Client` nie udostępnia swojej publicznie.
pub async fn peer_from_chat_id(session: &FileSession, chat_id: i64) -> Option<PeerRef> {
    let id = PeerId::from_bot_api_dialog_id(chat_id)?;
    session.peer_ref(id).await.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use grammers_client::session::types::PeerAuth;

    fn wpis(chat_id: i64, forum: bool) -> DialogEntry {
        DialogEntry {
            chat_id,
            name: format!("czat {chat_id}"),
            username: None,
            kind: DialogKind::Channel,
            is_forum: forum,
            photo_id: None,
            peer: PeerRef {
                id: PeerId::channel(1).unwrap(),
                auth: PeerAuth::from_hash(0),
            },
        }
    }

    #[test]
    fn wybierane_sa_tylko_czaty_z_tematami() {
        let d = vec![wpis(-100_1, true), wpis(-100_2, false), wpis(-100_3, true)];
        let f = forum_chats(&d);
        assert_eq!(f.len(), 2);
        assert!(f.contains(&-100_1));
        assert!(!f.contains(&-100_2));
    }

    #[test]
    fn temat_ma_wlasny_klucz_zrodla() {
        let t = TopicEntry {
            chat_id: -1_000_000_000_777,
            topic_id: 42,
            title: "XAUUSD".into(),
            closed: false,
        };
        let k = t.source_key();
        assert_eq!(k.chat_id, -1_000_000_000_777);
        assert_eq!(k.topic_id, Some(42));
        // dwa tematy tego samego kanału to DWA różne źródła
        let t2 = TopicEntry {
            topic_id: 43,
            ..t.clone()
        };
        assert_ne!(t.source_key(), t2.source_key());
    }

    #[test]
    fn identyfikator_bot_api_wraca_do_peer_id() {
        // ta zamiana musi działać w obie strony, bo `SourceKey` trzyma
        // postać Bot API, a wywołania API potrzebują postaci wewnętrznej
        let internal = PeerId::channel(1234).unwrap();
        let bot_api = internal.bot_api_dialog_id_unchecked();
        assert_eq!(bot_api, -1_000_000_001_234);
        assert_eq!(PeerId::from_bot_api_dialog_id(bot_api), Some(internal));

        let chat = PeerId::chat(5555).unwrap();
        assert_eq!(chat.bot_api_dialog_id_unchecked(), -5555);
        assert_eq!(PeerId::from_bot_api_dialog_id(-5555), Some(chat));
    }
}
