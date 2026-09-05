//! Test integracyjny protokołu WebSocket na PRAWDZIWYM gnieździe.
//!
//! Sprawdza to, czego testy jednostkowe sprawdzić nie mogą, a co jest twardym
//! wymaganiem: **okno natywne i przeglądarka są równorzędnymi klientami tego
//! samego stanu**. Dwa niezależne połączenia, komenda z jednego, zmiana
//! widoczna w obu.

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

type Klient =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn tmp_dir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "conduit-e2e-{tag}-{}-{}",
        std::process::id(),
        conduit_server::now_ms()
    ));
    p
}

async fn wstan(tag: &str) -> (conduit_server::Running, PathBuf) {
    let dir = tmp_dir(tag);
    let cfg = conduit_server::ServerConfig {
        // port 0 = system przydziela wolny; testy nie kolidują ze sobą
        bind: ([127, 0, 0, 1], 0).into(),
        workspace: dir.clone(),
        start_balance: 2000.0,
        backup_every: Duration::from_secs(3600),
        ..Default::default()
    };
    let run = conduit_server::serve(cfg, conduit_server::default_auth())
        .await
        .unwrap();
    (run, dir)
}

async fn polacz(addr: std::net::SocketAddr) -> Klient {
    let (s, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();
    s
}

/// Czeka na komunikat danego typu, pomijając inne (np. `event` z logu startowego).
async fn czekaj_na(k: &mut Klient, typ: &str) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let msg = tokio::time::timeout_at(deadline, k.next())
            .await
            .unwrap_or_else(|_| panic!("nie doczekano się komunikatu „{typ}”"))
            .unwrap()
            .unwrap();
        if let Message::Text(t) = msg {
            let v: Value = serde_json::from_str(&t).unwrap();
            if v["type"] == typ {
                return v;
            }
        }
    }
}

async fn wyslij(k: &mut Klient, v: Value) {
    k.send(Message::Text(v.to_string().into())).await.unwrap();
}

#[tokio::test]
async fn dwa_klienty_widza_ten_sam_stan() {
    let (run, dir) = wstan("dwa").await;
    let addr = run.addr;

    // okno natywne…
    let mut okno = polacz(addr).await;
    // …i przeglądarka
    let mut przegladarka = polacz(addr).await;

    let s1 = czekaj_na(&mut okno, "snapshot").await;
    let s2 = czekaj_na(&mut przegladarka, "snapshot").await;
    assert_eq!(s1["state"]["balance"], 2000.0);
    assert_eq!(s2["state"]["balance"], 2000.0);
    assert_eq!(s1["state"]["mode"], "AUTO");

    // komenda z OKNA
    wyslij(
        &mut okno,
        serde_json::json!({"type":"command","reqId":42,"cmd":"setMode","mode":"AI"}),
    )
    .await;

    let ack = czekaj_na(&mut okno, "ack").await;
    assert_eq!(ack["reqId"], 42);
    assert_eq!(
        ack["ok"], true,
        "komenda konfiguracyjna musi się udać bez brokera: {ack}"
    );

    // …widoczna w OBU powłokach
    let d1 = czekaj_na(&mut okno, "delta").await;
    assert_eq!(d1["patch"]["mode"], "AI");
    let d2 = czekaj_na(&mut przegladarka, "delta").await;
    assert_eq!(
        d2["patch"]["mode"], "AI",
        "przeglądarka nie zobaczyła zmiany z okna"
    );

    // i utrwalona na dysku — po restarcie tryb zostaje
    let doc = conduit_server::Workspace::new(&dir).load_settings();
    assert_eq!(doc.mode, conduit_server::ui::TradingMode::Ai);

    run.abort();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn tryb_auto_ea_przechodzi_cala_droge() {
    let (run, dir) = wstan("autoea").await;
    let mut k = polacz(run.addr).await;
    czekaj_na(&mut k, "snapshot").await;

    wyslij(
        &mut k,
        serde_json::json!({"type":"command","reqId":24,"cmd":"setMode","mode":"AUTO-EA"}),
    )
    .await;
    let ack = czekaj_na(&mut k, "ack").await;
    assert_eq!(ack["reqId"], 24);
    assert_eq!(
        ack["ok"], true,
        "zmiana trybu to komenda konfiguracyjna: {ack}"
    );

    let d = czekaj_na(&mut k, "delta").await;
    assert_eq!(d["patch"]["mode"], "AUTO-EA");

    let doc = conduit_server::Workspace::new(&dir).load_settings();
    assert_eq!(doc.mode, conduit_server::ui::TradingMode::AutoEa);

    run.abort();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn komenda_handlowa_bez_brokera_konczy_sie_jawnym_bledem() {
    let (run, dir) = wstan("broker").await;
    let mut k = polacz(run.addr).await;
    czekaj_na(&mut k, "snapshot").await;

    wyslij(
        &mut k,
        serde_json::json!({"type":"command","reqId":7,"cmd":"closePosition","ticket":123}),
    )
    .await;

    let ack = czekaj_na(&mut k, "ack").await;
    assert_eq!(ack["reqId"], 7);
    assert_eq!(
        ack["ok"], false,
        "bez brokera komenda NIE MOŻE zgłosić powodzenia"
    );
    assert!(
        ack["error"].as_str().unwrap().contains("Broker"),
        "komunikat musi mówić, czego brakuje: {ack}"
    );

    run.abort();
    let _ = std::fs::remove_dir_all(dir);
}

/// REGRESJA: komenda z własnym polem `id` (preset) musi przejść przez
/// prawdziwe gniazdo. Przy poprzedniej nazwie identyfikatora korelacji
/// (`id`) taki komunikat był NIEMOŻLIWY do zbudowania.
#[tokio::test]
async fn preset_z_katalogu_ui_przechodzi_cala_droge() {
    let (run, dir) = wstan("preset").await;
    let mut k = polacz(run.addr).await;
    czekaj_na(&mut k, "snapshot").await;

    wyslij(
        &mut k,
        serde_json::json!({
            "type": "command", "reqId": 11, "cmd": "applyPreset",
            "id": "MONTE-CARLO",
            "values": { "max_dd_pct": 60, "trail_mode": "tiered" }
        }),
    )
    .await;

    let ack = czekaj_na(&mut k, "ack").await;
    assert_eq!(ack["reqId"], 11);
    assert_eq!(ack["ok"], true, "preset odrzucony: {ack}");

    let d = czekaj_na(&mut k, "delta").await;
    assert_eq!(d["patch"]["presetId"], "MONTE-CARLO");
    assert_eq!(d["patch"]["settings"]["max_dd_pct"], 60);

    // i przeżywa restart procesu
    let doc = conduit_server::Workspace::new(&dir).load_settings();
    assert_eq!(doc.preset_id, "MONTE-CARLO");
    assert_eq!(doc.settings["trail_mode"], "tiered");

    run.abort();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn subskrypcja_zawezajaca_odsiewa_sekcje() {
    let (run, dir) = wstan("subskrypcja").await;
    let mut k = polacz(run.addr).await;
    czekaj_na(&mut k, "snapshot").await;

    // klient chce WYŁĄCZNIE statystyk
    wyslij(
        &mut k,
        serde_json::json!({"type":"subscribe","sections":["stats"]}),
    )
    .await;
    // Zamiast czekać „chwilę": serwer przetwarza komunikaty z jednego
    // połączenia po kolei, więc odebrany `pong` DOWODZI, że subskrypcja
    // została już zastosowana. Bez tego test bywał zawodny pod obciążeniem.
    wyslij(&mut k, serde_json::json!({"type":"ping","ts":0})).await;
    czekaj_na(&mut k, "pong").await;

    // zmiana w sekcji, której NIE subskrybuje
    run.state.update(
        conduit_server::coalesce::Sections::one(conduit_server::coalesce::Section::Mode),
        |s| s.mode = conduit_server::ui::TradingMode::Manual,
    );
    // …i w tej, którą subskrybuje
    run.state.update(
        conduit_server::coalesce::Sections::one(conduit_server::coalesce::Section::Stats),
        |s| s.stats.equity = 1234.5,
    );

    let d = czekaj_na(&mut k, "delta").await;
    assert!(
        d["patch"]["stats"].is_object(),
        "brak subskrybowanej sekcji: {d}"
    );
    assert!(
        d["patch"].get("mode").is_none(),
        "przyszła niesubskrybowana sekcja: {d}"
    );

    run.abort();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn setki_zmian_nie_daja_setek_ramek() {
    let (run, dir) = wstan("koalescencja").await;
    let mut k = polacz(run.addr).await;
    czekaj_na(&mut k, "snapshot").await;

    // Trzy serie po 100 natychmiastowych zmian, rozdzielone pauzą dłuższą
    // niż okno koalescencji. Świadomie NIE mierzymy „ile ramek na 300 zmian" —
    // liczba ramek zależy od czasu trwania testu, a nie od liczby zmian.
    // Sprawdzamy właściwość, która ma naprawdę obowiązywać: CZĘSTOTLIWOŚĆ.
    let start = std::time::Instant::now();
    let mut licznik = 0.0;
    for _ in 0..3 {
        for _ in 0..100 {
            licznik += 1.0;
            run.state.update(
                conduit_server::coalesce::Sections::one(conduit_server::coalesce::Section::Stats),
                |s| s.stats.equity = 2000.0 + licznik,
            );
        }
        tokio::time::sleep(Duration::from_millis(140)).await;
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
    let trwalo_ms = start.elapsed().as_millis() as f64;

    let mut delty = 0;
    let mut ostatnia_equity = 0.0;
    while let Ok(Some(Ok(Message::Text(t)))) =
        tokio::time::timeout(Duration::from_millis(80), k.next()).await
    {
        let v: Value = serde_json::from_str(&t).unwrap();
        if v["type"] == "delta" {
            delty += 1;
            if let Some(e) = v["patch"]["stats"]["equity"].as_f64() {
                ostatnia_equity = e;
            }
        }
    }

    assert!(delty > 0, "nie przyszła żadna delta");
    // sufit 10 Hz + 2 ramki zapasu na granice okien
    let sufit = trwalo_ms / 100.0 + 2.0;
    assert!(
        (delty as f64) <= sufit,
        "koalescencja nie trzyma 10 Hz — {delty} ramek w {trwalo_ms:.0} ms (sufit {sufit:.0})"
    );
    assert!(delty < 30, "podejrzanie dużo ramek: {delty}");
    assert_eq!(
        ostatnia_equity, 2300.0,
        "ostatnia ramka musi nieść AKTUALNY stan"
    );

    run.abort();
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn rest_odpowiada_i_qr_jest_prawdziwy() {
    let (run, dir) = wstan("rest").await;
    let base = run.url();

    let zdrowie: Value = reqwest_get(&format!("{base}/api/health"))
        .await
        .expect("health musi odpowiedzieć");
    assert_eq!(zdrowie["ok"], true);
    assert_eq!(zdrowie["app"], "conduit");

    let stan: Value = reqwest_get(&format!("{base}/api/state")).await.unwrap();
    assert_eq!(stan["balance"], 2000.0);

    // BEZ klienta MTProto (a taki jest w testach serwera) logowanie zatrzymuje
    // się na `needCredentials` i NIE POKAZUJE kodu QR.
    //
    // To jest zmiana wobec poprzedniej wersji, która rysowała prawdziwy obrazek
    // QR z zastępczym adresem — i to był błąd: token logowania wydaje serwer
    // Telegrama w odpowiedzi na `auth.exportLoginToken`, więc taki kod nie mógł
    // zalogować nikogo. Użytkownik skanował telefonem obrazek, który wyglądał
    // na działający. Puste miejsce z wyjaśnieniem jest uczciwsze.
    let stan_auth: Value = reqwest_get(&format!("{base}/api/auth/state"))
        .await
        .unwrap();
    assert_eq!(stan_auth["stage"], "needCredentials");
    assert!(
        stan_auth.get("qrSvg").is_none(),
        "zaślepka nie może rysować atrapy kodu QR"
    );
    assert_eq!(stan_auth["apiHashSet"], false);

    // a próba wymuszenia kodu kończy się jawnym błędem, nie atrapą
    let qr: Value = reqwest_post(&format!("{base}/api/auth/qr/start"), "{}")
        .await
        .unwrap();
    assert_eq!(qr["ok"], false);
    assert!(
        qr["error"].as_str().unwrap().contains("MTProto"),
        "komunikat ma mówić, CZEGO brakuje: {qr}"
    );

    // podsumowanie poświadczeń mówi „nie ma", nie podając żadnej wartości
    let sek: Value = reqwest_get(&format!("{base}/api/secrets")).await.unwrap();
    assert_eq!(sek["telegram"]["apiHashSet"], false);
    assert_eq!(sek["telegram"]["sessionSet"], false);
    assert_eq!(sek["smtp"]["passwordSet"], false);

    run.abort();
    let _ = std::fs::remove_dir_all(dir);
}

// --- minimalny klient HTTP, żeby nie ciągnąć `reqwest` do testów ---

async fn reqwest_get(url: &str) -> Option<Value> {
    surowe_zapytanie(url, "GET", None).await
}

async fn reqwest_post(url: &str, body: &str) -> Option<Value> {
    surowe_zapytanie(url, "POST", Some(body)).await
}

async fn surowe_zapytanie(url: &str, metoda: &str, body: Option<&str>) -> Option<Value> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let bez = url.strip_prefix("http://")?;
    let (host, sciezka) = bez
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((bez, "/".into()));
    let mut s = tokio::net::TcpStream::connect(host).await.ok()?;

    let mut req = format!("{metoda} {sciezka} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    if let Some(b) = body {
        req.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    req.push_str("\r\n");
    if let Some(b) = body {
        req.push_str(b);
    }
    s.write_all(req.as_bytes()).await.ok()?;

    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.ok()?;
    let txt = String::from_utf8_lossy(&buf);
    let ciało = txt.split("\r\n\r\n").nth(1)?;
    serde_json::from_str(ciało.trim()).ok()
}
