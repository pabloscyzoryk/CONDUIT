//! Test integracyjny `GET /api/email/subject` na PRAWDZIWYM gnieździe.
//!
//! Sprawdza jedną rzecz, której test jednostkowy sprawdzić nie może: że
//! **panel i mailer widzą TEN SAM temat**. Podgląd w ustawieniach istnieje
//! wyłącznie po to, żeby użytkownik nie musiał wysyłać maila, aby zobaczyć,
//! co wyjdzie — więc rozjazd między tym endpointem a `Notifier::temat`
//! byłby gorszy niż brak podglądu, bo kłamałby z przekonaniem.
//!
//! Klient HTTP na surowym gnieździe, jak w `models_rest.rs` — dokładanie
//! `reqwest` do zależności deweloperskich dla dwóch żądań GET to przesada.

use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn tmp_dir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "conduit-temat-{tag}-{}-{}",
        std::process::id(),
        conduit_server::now_ms()
    ));
    p
}

async fn get(addr: std::net::SocketAddr, path: &str) -> (u16, String) {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut buf))
        .await
        .unwrap()
        .unwrap();
    let txt = String::from_utf8_lossy(&buf).to_string();
    let kod: u16 = txt
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let body = txt.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (kod, body)
}

#[tokio::test]
async fn podglad_tematu_zwraca_katalog_zmiennych_i_gotowy_temat() {
    let dir = tmp_dir("podglad");
    let cfg = conduit_server::ServerConfig {
        bind: ([127, 0, 0, 1], 0).into(),
        workspace: dir.clone(),
        backup_every: Duration::from_secs(3600),
        ..Default::default()
    };
    let run = conduit_server::serve(cfg, conduit_server::default_auth())
        .await
        .unwrap();

    run.state.update(
        conduit_server::coalesce::Sections::one(conduit_server::coalesce::Section::Settings),
        |s| {
            s.language = "pl".into();
            s.stats.balance = 237.65;
            s.stats.equity = 241.02;
            s.stats.free_margin = 228.62;
            s.connection.user.name = "Demo User".into();
            s.connection.account.broker = "Vantage Global Prime LLP".into();
            s.connection.account.login = 10_000_001;
            s.preset_id = "ULTRA-X3".into();
        },
    );

    // ---------- 1. własny szablon, zmienna powtórzona ----------
    let tpl = urlencoding("Aktualny balans: ${balance} / ${equity} · ${balance} · ${user}");
    let (kod, body) = get(run.addr, &format!("/api/email/subject?tpl={tpl}")).await;
    assert_eq!(kod, 200, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v["preview"],
        "Aktualny balans: $237.65 / $241.02 · $237.65 · Demo User"
    );
    assert_eq!(v["pusty"], false);

    // ---------- 2. katalog zmiennych ma opisy i wartości ----------
    let zmienne = v["vars"].as_array().unwrap();
    assert!(
        zmienne.len() >= 20,
        "katalog ma mieć kilkanaście pozycji, ma {}",
        zmienne.len()
    );
    for z in zmienne {
        assert!(!z["name"].as_str().unwrap().is_empty());
        // opis w selectcie to JEDYNE objaśnienie, jakie widzi użytkownik
        assert!(!z["label"].as_str().unwrap().trim().is_empty(), "{z}");
        assert!(!z["value"].as_str().unwrap().is_empty(), "{z}");
    }
    let nazwy: Vec<&str> = zmienne
        .iter()
        .map(|z| z["name"].as_str().unwrap())
        .collect();
    for wymagana in [
        "balance",
        "equity",
        "user",
        "wolny_margines",
        "preset",
        "pozycje",
        "symbol",
    ] {
        assert!(
            nazwy.contains(&wymagana),
            "brak ${{{wymagana}}} w katalogu: {nazwy:?}"
        );
    }
    let saldo = zmienne.iter().find(|z| z["name"] == "balance").unwrap();
    assert_eq!(saldo["value"], "$237.65");
    assert_eq!(saldo["label"], "Balans konta");

    // ---------- 3. literówka zostaje w tekście ----------
    let (_, body) = get(
        run.addr,
        &format!("/api/email/subject?tpl={}", urlencoding("Saldo ${blans}")),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["preview"], "Saldo ${blans}");

    // ---------- 4. pusty szablon = temat systemowy ----------
    let (_, body) = get(run.addr, "/api/email/subject?tpl=").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["pusty"], true);
    assert_eq!(v["preview"], v["systemowy"]);
    assert_eq!(v["preview"], "[CONDUIT] podsumowanie — Podsumowanie dnia");

    run.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn bez_parametru_podglad_dotyczy_szablonu_zapisanego_w_ustawieniach() {
    // Karta ustawień otwarta po restarcie musi od razu pokazać temat, którym
    // bot NAPRAWDĘ się posługuje — a nie pustkę do czasu pierwszego klawisza.
    let dir = tmp_dir("zapisany");
    let cfg = conduit_server::ServerConfig {
        bind: ([127, 0, 0, 1], 0).into(),
        workspace: dir.clone(),
        backup_every: Duration::from_secs(3600),
        ..Default::default()
    };
    let run = conduit_server::serve(cfg, conduit_server::default_auth())
        .await
        .unwrap();
    run.state.update(
        conduit_server::coalesce::Sections::one(conduit_server::coalesce::Section::Settings),
        |s| s.language = "pl".into(),
    );

    conduit_server::commands::apply(
        &run.state,
        &conduit_server::proto::Command::SetEmail {
            email: conduit_server::ui::EmailConfig {
                host: "smtp.example.com".into(),
                subject: "CONDUIT ${preset}: ${balance}".into(),
                ..Default::default()
            },
        },
    )
    .unwrap();

    let (kod, body) = get(run.addr, "/api/email/subject").await;
    assert_eq!(kod, 200, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["saved"], "CONDUIT ${preset}: ${balance}");
    assert_eq!(v["template"], v["saved"]);
    // saldo zerowe (broker milczy), ale etykieta presetu też jeszcze pusta —
    // i to jest właśnie informacja, którą podgląd ma pokazać bez ściemy
    assert_eq!(v["preview"], "CONDUIT (ustawienia własne): $0.00");

    run.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Minimalne kodowanie parametru zapytania. Interesują nas tylko te znaki,
/// które realnie występują w szablonach tematu: `$ { } / spacja ·`.
fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        let c = *b as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
