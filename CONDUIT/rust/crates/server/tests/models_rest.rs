//! Test integracyjny endpointów `/api/models` na PRAWDZIWYM gnieździe.
//!
//! Sprawdza jedną rzecz, której test jednostkowy sprawdzić nie może: że
//! przeglądarka dostaje listę modeli **bez wag** (bo lista z wagami to
//! kilka megabajtów na każde otwarcie widoku), a pełny dokument z wagami
//! dopiero pod adresem konkretnego modelu.
//!
//! Klient HTTP piszemy na surowym gnieździe — dokładanie `reqwest` do
//! zależności deweloperskich dla trzech żądań GET byłoby przesadą.

use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn tmp_dir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "conduit-models-{tag}-{}-{}",
        std::process::id(),
        conduit_server::now_ms()
    ));
    p
}

/// Model o prawdziwym kształcie (`policy.rs`), tylko malutki: 3 wejścia,
/// 2 neurony ukryte, 2 wyjścia. Liczba parametrów jest policzalna w pamięci:
/// sieć pozycji 3×2 + 2 + 2×2 + 2 = 14, sieć koszyka 2×2 + 2 = 6, razem 20.
fn maly_model() -> serde_json::Value {
    serde_json::json!({
        "format": 1,
        "name": "test",
        "created": "2026-07-27T12:00:00+00:00",
        "n_global": 1,
        "n_basket": 1,
        "n_position": 1,
        "feature_names": ["a", "b", "c"],
        "policy": {
            "pos": {
                "dims": [3, 2, 2],
                "w": [[0.1, 0.2, 0.3, 0.4, 0.5, 0.6], [0.7, 0.8, 0.9, 1.0]],
                "b": [[0.0, 0.0], [1.0, 0.0]]
            },
            "bsk": {
                "dims": [2, 2],
                "w": [[0.1, 0.2, 0.3, 0.4]],
                "b": [[0.0, 0.0]]
            }
        },
        "safety": { "max_positions": 40 },
        "score": { "pnl": -10.93, "note": "walidacja" }
    })
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
async fn lista_modeli_jest_bez_wag_a_pojedynczy_model_z_wagami() {
    let dir = tmp_dir("rest");
    std::fs::create_dir_all(dir.join("models")).unwrap();
    std::fs::write(
        dir.join("models").join("atfx_manager_v1.json"),
        serde_json::to_vec_pretty(&maly_model()).unwrap(),
    )
    .unwrap();
    // uszkodzony plik nie może wywrócić listy
    std::fs::write(
        dir.join("models").join("zepsuty.json"),
        b"{ to nie jest json",
    )
    .unwrap();

    let cfg = conduit_server::ServerConfig {
        bind: ([127, 0, 0, 1], 0).into(),
        workspace: dir.clone(),
        backup_every: Duration::from_secs(3600),
        ..Default::default()
    };
    let run = conduit_server::serve(cfg, conduit_server::default_auth())
        .await
        .unwrap();

    // ---------- lista ----------
    let (kod, body) = get(run.addr, "/api/models").await;
    assert_eq!(kod, 200, "lista modeli: {body}");
    let lista: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        lista.as_array().unwrap().len(),
        1,
        "zepsuty plik miał zostać pominięty"
    );
    let m = &lista[0];
    assert_eq!(m["id"], "atfx_manager_v1");
    assert_eq!(m["params"], 20);
    assert_eq!(m["policy"]["pos"]["dims"], serde_json::json!([3, 2, 2]));
    assert_eq!(m["policy"]["pos"]["params"], 14);
    assert_eq!(m["policy"]["bsk"]["params"], 6);
    assert!(
        m["policy"]["pos"]["w"].is_null(),
        "lista NIE może wieźć wag"
    );
    assert!(m["bytes"].as_u64().unwrap() > 0);
    // metadane przechodzą bez zmian — UI ma z czego zbudować kartę modelu
    assert_eq!(m["feature_names"][0], "a");
    assert_eq!(m["score"]["pnl"], -10.93);

    // ---------- pojedynczy model ----------
    let (kod, body) = get(run.addr, "/api/models/atfx_manager_v1").await;
    assert_eq!(kod, 200, "model: {body}");
    let pelny: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(pelny["policy"]["pos"]["w"][0][0], 0.1);
    assert_eq!(pelny["policy"]["bsk"]["dims"], serde_json::json!([2, 2]));
    assert_eq!(pelny["id"], "atfx_manager_v1");

    // ---------- braki i próba wyjścia z katalogu ----------
    let (kod, _) = get(run.addr, "/api/models/nie_ma_takiego").await;
    assert_eq!(kod, 404);
    let (kod, _) = get(run.addr, "/api/models/..%2Fsettings").await;
    assert_eq!(kod, 400, "identyfikator z „..” musi zostać odrzucony");

    run.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn brak_katalogu_models_daje_pusta_liste_a_nie_blad() {
    let dir = tmp_dir("puste");
    let cfg = conduit_server::ServerConfig {
        bind: ([127, 0, 0, 1], 0).into(),
        workspace: dir.clone(),
        backup_every: Duration::from_secs(3600),
        ..Default::default()
    };
    let run = conduit_server::serve(cfg, conduit_server::default_auth())
        .await
        .unwrap();

    let (kod, body) = get(run.addr, "/api/models").await;
    assert_eq!(kod, 200);
    assert_eq!(body.trim(), "[]");

    run.abort();
    let _ = std::fs::remove_dir_all(&dir);
}
