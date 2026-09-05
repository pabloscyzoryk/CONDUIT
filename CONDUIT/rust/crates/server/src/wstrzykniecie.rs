
use crate::proto::Command;
use conduit_core::engine::IncomingMessage;
use conduit_core::types::SourceKey;
use std::sync::atomic::{AtomicI64, Ordering};

/// Nazwa źródła pokazywana przy wiadomości wstrzykniętej z panelu.
pub const ZRODLO_PANELU: &str = "PANEL";

pub const KANAL_PANELU: i64 = 0;

/// Górna granica zasiewu licznika. Numer startowy leży w `[-ZASIEW, -1]`, żeby
/// dwa uruchomienia programu tego samego dnia nie zaczynały od tej samej
/// liczby (a więc żeby edycja z poprzedniej sesji nie trafiła w nowy koszyk).
const ZASIEW: i64 = 1_000_000_000;

/// `0` znaczy „jeszcze nie zasiane" — samo zero nigdy nie jest wydawane.
static NASTEPNY: AtomicI64 = AtomicI64::new(0);

/// Nadaje numer wiadomości wstrzykniętej. ZAWSZE ujemny, ZAWSZE inny.
///
/// `ts` służy wyłącznie do zasiania licznika przy pierwszym wywołaniu.
pub fn nadaj_numer(ts: i64) -> i64 {
    let zasiew = -(1 + ts.rem_euclid(ZASIEW));
    let poprzedni = NASTEPNY
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |x| {
            Some(if x == 0 { zasiew } else { x - 1 })
        })
        .unwrap_or(0);
    if poprzedni == 0 {
        zasiew
    } else {
        poprzedni - 1
    }
}

/// Czy ten numer pochodzi z panelu, a nie z Telegrama?
///
/// Reguła w jednym miejscu, żeby dało się ją sprawdzić testem, a nie czytaniem
/// komentarzy. Numery nadane ręcznie przez operatora są tu nierozróżnialne od
/// telegramowych — bo mają nimi BYĆ.
pub fn numer_wstrzykniety(msg_id: i64) -> bool {
    msg_id < 0
}

pub fn wiadomosc(cmd: &Command, ts: i64, nazwa_zrodla: &str) -> Option<IncomingMessage> {
    let Command::SimulateMessage {
        text,
        channel_id,
        topic_id,
        msg_id,
        reply_to,
        edit_of,
    } = cmd
    else {
        return None;
    };
    Some(IncomingMessage {
        ts,
        source: SourceKey::new(channel_id.unwrap_or(KANAL_PANELU), *topic_id),
        source_name: nazwa_zrodla.to_string(),
        msg_id: msg_id.unwrap_or_else(|| nadaj_numer(ts)),
        reply_to: *reply_to,
        edit_of: *edit_of,
        text: text.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn polecenie(tekst: &str) -> Command {
        Command::SimulateMessage {
            text: tekst.into(),
            channel_id: None,
            topic_id: None,
            msg_id: None,
            reply_to: None,
            edit_of: None,
        }
    }

    /// KONTRAKT ZERA: samo `text` daje dokładnie to, co dawało przed F5.
    #[test]
    fn samo_tekst_daje_dzisiejsze_zachowanie() {
        let im = wiadomosc(
            &polecenie("BUY LIMITS GOLD @ 4005/4000 AREA"),
            1_700_000_000_000,
            ZRODLO_PANELU,
        )
        .expect("to jest simulateMessage");
        assert_eq!(
            im.source,
            SourceKey::new(0, None),
            "domyślny adres to pseudokanał panelu"
        );
        assert_eq!(im.source_name, "PANEL");
        assert!(im.reply_to.is_none());
        assert!(im.edit_of.is_none());
        assert!(
            im.msg_id < 0,
            "numer nadany automatycznie musi być ujemny, jest {}",
            im.msg_id
        );
        assert_eq!(im.text, "BUY LIMITS GOLD @ 4005/4000 AREA");
    }

    /// REGRESJA: stara reguła `-(ts % 1e9)` dawała dwa razy ten sam numer
    /// w tej samej milisekundzie, a `entry_idempotencja` zjadała drugi sygnał.
    #[test]
    fn dwa_wstrzykniecia_w_tej_samej_milisekundzie_maja_rozne_numery() {
        let ts = 1_700_000_000_000;
        let a = wiadomosc(&polecenie("A"), ts, ZRODLO_PANELU)
            .unwrap()
            .msg_id;
        let b = wiadomosc(&polecenie("B"), ts, ZRODLO_PANELU)
            .unwrap()
            .msg_id;
        assert_ne!(
            a, b,
            "ten sam numer = drugi sygnał ginie w idempotencji wejścia"
        );
        assert!(a < 0 && b < 0);
    }

    #[test]
    fn nadane_numery_zostaja_w_zakresie_panelu() {
        for _ in 0..1_000 {
            let n = nadaj_numer(1_700_000_000_000);
            assert!(
                numer_wstrzykniety(n),
                "numer {n} wyszedł poza zakres panelu"
            );
        }
    }

    /// Podany `msg_id` jest brany DOSŁOWNIE — inaczej nie dałoby się
    /// zaadresować wiadomości, która już istnieje.
    #[test]
    fn podany_numer_jest_brany_doslownie() {
        let cmd = Command::SimulateMessage {
            text: "✅ TP1 HIT".into(),
            channel_id: Some(-1_000_000_000_301),
            topic_id: Some(77),
            msg_id: Some(4321),
            reply_to: Some(1000),
            edit_of: Some(4321),
        };
        let im = wiadomosc(&cmd, 1_700_000_000_000, "PANEL").unwrap();
        assert_eq!(im.msg_id, 4321);
        assert_eq!(im.reply_to, Some(1000));
        assert_eq!(im.edit_of, Some(4321));
        assert_eq!(im.source, SourceKey::new(-1_000_000_000_301, Some(77)));
    }

    #[test]
    fn inne_polecenie_nie_jest_wstrzyknieciem() {
        assert!(wiadomosc(&Command::ResumeTrading, 0, "PANEL").is_none());
    }
}
