
use crate::kronika::{Rodzaj, Wpis};
use anyhow::{Context, Result};
use std::path::Path;

/// Co wyszło z eksportu. `z_edycji` jest tu najważniejsze — to jest dokładnie
/// ta część zbioru, której eksport z aplikacji Telegram NIE MA.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PodsumowanieEksportu {
    pub plik: String,
    pub plikow_zrodlowych: usize,
    pub wpisow: u64,
    pub uszkodzonych: u64,
    pub sygnalow: u32,
    pub zdarzen: u32,
    pub z_edycji: u32,
    pub bajtow: u64,
}

/// Filtr źródeł dla eksportu: `None` = wszystko, co jest w pliku.
///
/// Świadomie osobno od filtra ZAPISU: przy zapisie selekcja jest
/// nieodwracalna, przy eksporcie da się ją cofnąć w sekundę.
pub type FiltrZrodel = Option<crate::kronika::Zrodla>;

/// Czyta kronikę i zapisuje zbiór sygnałów pod `cel`.
pub fn eksportuj(zrodlo: &Path, cel: &Path, filtr: FiltrZrodel) -> Result<PodsumowanieEksportu> {
    let odczyt = crate::kronika::odczyt::czytaj(zrodlo);
    anyhow::ensure!(
        !odczyt.wpisy.is_empty(),
        "kronika jest pusta ({}) — zapis rusza z pierwszą wiadomością z Telegrama",
        zrodlo.display()
    );

    let wybrane: Vec<&Wpis> = odczyt
        .wpisy
        .iter()
        .filter(|w| w.rodzaj.to_wiadomosc())
        .filter(|w| {
            filtr
                .as_ref()
                .map(|f| f.pasuje(w.chat_id, w.temat))
                .unwrap_or(true)
        })
        .collect();

    let (dokument, mut p) = zbuduj(&wybrane);
    let dane = serde_json::to_vec(&dokument)?;
    if let Some(dir) = cel.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    std::fs::write(cel, &dane).with_context(|| format!("zapis {}", cel.display()))?;

    p.plik = cel.display().to_string();
    p.plikow_zrodlowych = odczyt.plikow;
    p.wpisow = odczyt.wpisy.len() as u64;
    p.uszkodzonych = odczyt.uszkodzonych;
    p.bajtow = dane.len() as u64;
    Ok(p)
}

/// Buduje kanoniczny, PŁASKI strumień `{ "messages": [...] }`.
///
/// Eksporter celowo NIE uruchamia parsera jako filtra, NIE grupuje wiadomości
/// w sygnały i NIE przypisuje zarządzania do „ostatniego koszyka". Parser oraz
/// routing są częścią systemu podlegającego testowi; przygotowanie danych nie
/// może wykonać ich pracy z góry ani ukryć wiadomości `Info`.
///
/// `receive_seq` jest globalnym numerem w tym konkretnym eksporcie. Surowe
/// `Wpis::seq` resetuje się po restarcie Kroniki, więc samo nie rozstrzyga
/// remisów pomiędzy sesjami. Oryginalny numer zachowujemy w provenance.
pub fn zbuduj(wpisy: &[&Wpis]) -> (serde_json::Value, PodsumowanieEksportu) {
    let mut p = PodsumowanieEksportu::default();
    let mut messages = Vec::with_capacity(wpisy.len());

    for (ordinal, w) in wpisy.iter().enumerate() {
        let edit_of = w
            .edit_of
            .or_else(|| (w.rodzaj == Rodzaj::Edycja).then_some(w.msg_id));
        let event = match w.rodzaj {
            Rodzaj::Nowa => "new",
            Rodzaj::Edycja => "edit",
            Rodzaj::Skasowana => "delete",
            Rodzaj::Start | Rodzaj::Stop => continue,
        };
        let parsed = conduit_core::parser::parse(&w.text);
        if w.rodzaj == Rodzaj::Nowa
            && parsed
                .iter()
                .any(|s| matches!(s, conduit_core::parser::Signal::Entry(_)))
        {
            p.sygnalow += 1;
        }
        if w.rodzaj == Rodzaj::Edycja {
            p.z_edycji += 1;
        }
        p.zdarzen += 1;
        let latency =
            (w.ts_telegram_ms > 0).then_some(w.odebrano_ms.saturating_sub(w.ts_telegram_ms));
        messages.push(serde_json::json!({
            "ts": w.odebrano_ms,
            "received_at_ms": w.odebrano_ms,
            "receive_seq": ordinal as u64,
            "event": event,
            "msg_id": w.msg_id,
            "reply_to": w.reply_to,
            "edit_of": edit_of,
            "text": w.text,
            "kanal": w.chat,
            "latency_ms": latency,
            "telegram_event_at_ms": w.ts_telegram_ms,
            "chat_id": w.chat_id,
            "topic_id": w.temat,
            "provenance": {
                "capture_seq": w.seq,
                "capture_schema": w.v,
                "monitored": w.nasluchiwany,
                "parser_observed": w.rozpoznane,
                "format": w.format,
                "note": w.uwaga,
            },
            "fidelity": "observed_live"
        }));
    }

    (
        serde_json::json!({
            "schema": "conduit.raw-message-stream.v1",
            "clock": "received_at_ms",
            "messages": messages
        }),
        p,
    )
}

#[cfg(test)]
mod testy {
    use super::*;
    use crate::kronika::SCHEMAT;

    fn wpis(ms: i64, rodzaj: Rodzaj, chat_id: i64, msg_id: i64, text: &str) -> Wpis {
        Wpis {
            v: SCHEMAT,
            seq: msg_id as u64,
            rodzaj,
            odebrano_ms: ms,
            odebrano: String::new(),
            ts_telegram_ms: ms,
            chat_id,
            chat: format!("K{chat_id}"),
            temat: None,
            msg_id,
            reply_to: None,
            edit_of: if rodzaj == Rodzaj::Edycja {
                Some(msg_id)
            } else {
                None
            },
            text: text.into(),
            znakow: text.chars().count(),
            nasluchiwany: true,
            format: None,
            rozpoznane: true,
            uwaga: None,
            ts_telegram: None,
        }
    }

    #[test]
    fn edycja_wchodzi_jako_zdarzenie_z_wlasnym_czasem() {
        // Chwila odebrania edycji, a nie publikacji sygnału — o to w tym
        // wszystkim chodzi.
        let t = 1_785_000_000_000;
        let a = wpis(
            t,
            Rodzaj::Nowa,
            -100,
            1,
            "BUY GOLD 4020-4025 SL 4010 TP 4030",
        );
        let b = wpis(
            t + 141_000,
            Rodzaj::Edycja,
            -100,
            1,
            "BUY GOLD 4020-4025 SL 4010 TP 4030 TP 4040",
        );
        let (doc, p) = zbuduj(&[&a, &b]);
        assert_eq!(p.sygnalow, 1);
        let m = doc["messages"].as_array().unwrap();
        assert_eq!(m.len(), 2, "obie odebrane wersje muszą wejść do replay");
        assert_eq!(m[0]["event"], "new");
        assert_eq!(m[1]["event"], "edit");
        assert_eq!(m[1]["edit_of"], 1);
        assert_eq!(
            m[1]["ts"].as_i64().unwrap(),
            t + 141_000,
            "bez utraty milisekund"
        );
        assert_eq!(p.z_edycji, 1);
    }

    #[test]
    fn eksport_nie_przypisuje_komendy_do_koszyka() {
        // Przygotowanie danych nie może zgadywać adresata. Surowe reply_to
        // zostaje takie, jakie przyszło; routing oceni dopiero silnik.
        let t = 1_785_000_000_000;
        let a = wpis(
            t,
            Rodzaj::Nowa,
            -100,
            1,
            "BUY GOLD 4020-4025 SL 4010 TP 4030",
        );
        let b = wpis(
            t + 1000,
            Rodzaj::Nowa,
            -200,
            1,
            "SELL GOLD 4100-4105 SL 4115 TP 4090",
        );
        let c = wpis(t + 2000, Rodzaj::Nowa, -100, 2, "✅ TP1 HIT +48 PIPS");
        let (doc, p) = zbuduj(&[&a, &b, &c]);
        assert_eq!(p.sygnalow, 2);
        let messages = doc["messages"].as_array().unwrap();
        assert_eq!(
            messages.len(),
            3,
            "Info/management nie mogą wypaść z eksportu"
        );
        assert_eq!(messages[2]["msg_id"], 2);
        assert!(
            messages[2]["reply_to"].is_null(),
            "eksporter nie dopisuje fikcyjnego reply"
        );
        assert_eq!(messages[2]["kanal"], "K-100");
    }

    #[test]
    fn znaczniki_sesji_nie_trafiaja_do_zbioru() {
        let t = 1_785_000_000_000;
        let mut s = wpis(t, Rodzaj::Nowa, 0, 0, "");
        s.rodzaj = Rodzaj::Start;
        let a = wpis(
            t + 1,
            Rodzaj::Nowa,
            -100,
            1,
            "BUY GOLD 4020-4025 SL 4010 TP 4030",
        );
        // `zbuduj` dostaje już przefiltrowane wiersze — sprawdzamy pełną drogę
        let wybrane: Vec<&Wpis> = [&s, &a]
            .into_iter()
            .filter(|w| w.rodzaj.to_wiadomosc())
            .collect();
        let (_, p) = zbuduj(&wybrane);
        assert_eq!(p.sygnalow, 1);
    }

    #[test]
    fn pusta_kronika_konczy_sie_czytelnym_bledem() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "kronika-eksport-{}-{}",
            std::process::id(),
            crate::kronika::teraz_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let e = eksportuj(&dir, &dir.join("out.json"), None).unwrap_err();
        assert!(e.to_string().contains("pusta"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
