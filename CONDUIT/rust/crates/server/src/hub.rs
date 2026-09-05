//! Obsługa połączeń WebSocket.
//!
//! Każde połączenie jest RÓWNORZĘDNYM subskrybentem tego samego stanu —
//! okno natywne, karta przeglądarki i (docelowo) widget zasobnika niczym się
//! nie różnią. Nie ma „klienta głównego", więc nie ma stanu, który mógłby się
//! rozjechać między powłokami.
//!
//! Kolejność po podłączeniu:
//! ```text
//!   klient łączy się z /ws
//!   serwer: snapshot (pełny stan)
//!   serwer: delta … delta … event …      (do rozłączenia)
//!   klient: subscribe / command / settingsPatch / ping
//! ```

use crate::coalesce::Sections;
use crate::commands;
use crate::proto::{ClientMsg, DeltaPatch, ServerMsg};
use crate::state::{Notice, StateHandle};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast::error::RecvError;

pub async fn ws_upgrade(ws: WebSocketUpgrade, State(st): State<StateHandle>) -> Response {
    ws.on_upgrade(move |socket| connection(socket, st))
}

async fn connection(socket: WebSocket, st: StateHandle) {
    let (mut out, mut inp) = socket.split();
    let mut sub = st.subscribe();
    // domyślnie klient dostaje wszystko; `subscribe` może to zawęzić
    let mut interest = Sections::all();

    tracing::info!(klientow = st.clients(), "nowe połączenie WS");

    if send(
        &mut out,
        &ServerMsg::Snapshot {
            state: Box::new(st.snapshot()),
        },
    )
    .await
    .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            notice = sub.recv() => match notice {
                Ok(Notice::Delta { rev, sections }) => {
                    let chce = sections.intersect(interest);
                    if chce.is_empty() {
                        continue;
                    }
                    // treść czytamy ze wspólnego stanu DOPIERO TERAZ — dzięki temu
                    // nie kopiujemy pozycji osobno dla każdego klienta, a dane są
                    // co najmniej tak świeże jak w momencie powiadomienia
                    let patch = st.read(|s| DeltaPatch::build(s, chce));
                    if patch.is_empty() {
                        continue;
                    }
                    if send(&mut out, &ServerMsg::Delta { rev, patch: Box::new(patch) }).await.is_err() {
                        break;
                    }
                }
                Ok(Notice::Event(ev)) => {
                    if send(&mut out, &ServerMsg::Event { event: Box::new((*ev).clone()) }).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(ile)) => {
                    // Klient nie nadążył (zminimalizowane okno, uśpiony laptop).
                    // Nie próbujemy odtwarzać zaległości — wysyłamy pełny stan.
                    // To jedyna odpowiedź, która NIE MOŻE zostawić klienta
                    // z niespójnym obrazem.
                    tracing::warn!(zgubione = ile, "klient nie nadążył — wysyłam pełny stan");
                    if send(&mut out, &ServerMsg::Snapshot { state: Box::new(st.snapshot()) }).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Closed) => break,
            },

            msg = inp.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    if let Some(resp) = handle_client_msg(&st, t.as_str(), &mut interest) {
                        if send(&mut out, &resp).await.is_err() {
                            break;
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    tracing::debug!(blad = %e, "połączenie WS przerwane");
                    break;
                }
            },
        }
    }

    tracing::info!(
        klientow = st.clients().saturating_sub(1),
        "zamknięto połączenie WS"
    );
}

/// Zwraca komunikat do odesłania (ack/pong) albo `None`.
fn handle_client_msg(st: &StateHandle, raw: &str, interest: &mut Sections) -> Option<ServerMsg> {
    let msg: ClientMsg = match serde_json::from_str(raw) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(blad = %e, "nieczytelny komunikat od klienta");
            return Some(ServerMsg::Ack {
                req_id: 0,
                ok: false,
                error: Some(format!("zły format: {e}")),
            });
        }
    };

    match msg {
        ClientMsg::Subscribe { sections } => {
            *interest = if sections.is_empty() {
                Sections::all()
            } else {
                Sections::from_list(&sections)
            };
            None
        }
        ClientMsg::Ping { ts } => Some(ServerMsg::Pong {
            ts,
            server_time: crate::now_ms(),
        }),
        ClientMsg::Command { req_id, cmd, account_session } => {
            let r = commands::apply_scoped(st, &cmd, account_session.as_deref());
            Some(ack(req_id, r))
        }
        ClientMsg::SettingsPatch { req_id, patch } => {
            let r = commands::apply_settings_patch(st, &patch);
            Some(ack(req_id, r))
        }
    }
}

fn ack(req_id: u64, r: anyhow::Result<()>) -> ServerMsg {
    match r {
        Ok(()) => ServerMsg::Ack {
            req_id,
            ok: true,
            error: None,
        },
        Err(e) => {
            tracing::warn!(req_id, blad = %e, "komenda odrzucona");
            ServerMsg::Ack {
                req_id,
                ok: false,
                error: Some(e.to_string()),
            }
        }
    }
}

async fn send<S>(out: &mut S, msg: &ServerMsg) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let txt = match serde_json::to_string(msg) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(blad = %e, "nie udało się zserializować komunikatu");
            return Ok(());
        }
    };
    out.send(Message::Text(txt.into())).await.map_err(|_| ())
}

/// Pętla koalescencji — jedyne miejsce, z którego wychodzą delty.
///
/// Sprawdzamy częściej (50 ms) niż wysyłamy (100 ms), żeby zmiana, która
/// pojawiła się tuż po wysłanej ramce, nie czekała pełnego okna.
pub async fn delta_loop(st: StateHandle) {
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(50));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if let Some(sections) = st.take_dirty(crate::now_ms()) {
            st.broadcast_delta(sections);
        }
    }
}

/// Przepisuje stan logowania do Telegrama do wspólnej migawki.
///
/// Jedyne miejsce, które ustawia `connection.telegram` — dzięki temu znacznik
/// zawsze wynika z FAKTYCZNEGO stanu usługi, a nie z tego, którą drogą
/// użytkownik się zalogował.
pub fn zapisz_auth(st: &StateHandle, s: &crate::auth::AuthState) {
    if st.read(|x| x.auth == *s) {
        return;
    }
    st.update(
        Sections::one(crate::coalesce::Section::Auth) | crate::coalesce::Section::Connection,
        |x| {
            x.auth = s.clone();
            x.connection.telegram = if s.is_logged_in() {
                "connected".into()
            } else {
                "disconnected".into()
            };
            if let Some(u) = &s.user {
                x.connection.user.name = u.clone();
            }
        },
    );
}

/// Dozór nad stanem logowania do Telegrama.
///
/// REGRESJA, którą to naprawia: `connection.telegram` był ustawiany WYŁĄCZNIE
/// na ścieżce REST-owej (kod QR, 2FA, wylogowanie) i raz w `bootstrap`.
/// Logowanie z ZAPISANEJ SESJI dzieje się jednak w tle, sekundę po starcie
/// serwera i bez udziału jakiegokolwiek endpointu — nikt wtedy nie odświeżał
/// migawki, więc panel do końca życia procesu pokazywał „rozłączony", mając
/// pod spodem zalogowane konto i pełną listę czatów. To już raz zablokowało
/// ekran „Kanały": widok czytał ten znacznik i wypisywał „zaloguj się" przy
/// komplecie 210 dialogów na serwerze.
///
/// Pętla nie wie NIC o drogach logowania — pyta usługę o stan i przepisuje go,
/// gdy się różni. Każda przyszła ścieżka (wylogowanie z telefonu, wygaśnięcie
/// sesji, ponowne logowanie) jest obsłużona z góry, bez dopisywania wywołań.
pub async fn auth_loop(st: StateHandle) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let a = st.auth.clone();
        // `state()` bywa implementowane na blokującym kliencie MTProto —
        // pytamy z puli blokującej, żeby nie zatrzymać obsługi HTTP.
        let Ok(s) = tokio::task::spawn_blocking(move || a.state()).await else {
            continue;
        };
        zapisz_auth(&st, &s);
    }
}

/// Cykliczny zapis `backup_memory/`.
pub async fn backup_loop(st: StateHandle, every: std::time::Duration) {
    let mut tick = tokio::time::interval(every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        match st.save_backup() {
            Ok(true) => tracing::debug!("zapisano backup_memory"),
            Ok(false) => {}
            Err(e) => tracing::error!(blad = %e, "nie udało się zapisać backup_memory"),
        }
    }
}
