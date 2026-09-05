
use crate::kronika as kr;
use crate::state::StateHandle;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;

pub fn router() -> Router<StateHandle> {
    Router::new()
        .route("/stan", get(stan))
        .route("/statystyki", get(statystyki))
        .route("/kanaly", get(kanaly))
        .route("/ustawienia", put(ustaw))
        .route("/eksport", post(eksport))
        .route("/plik", get(pobierz))
}

fn blad(code: StatusCode, msg: impl Into<String>) -> Response {
    (
        code,
        Json(serde_json::json!({ "ok": false, "error": msg.into() })),
    )
        .into_response()
}

// ============================================================
//  STAN
// ============================================================

async fn stan(State(st): State<StateHandle>) -> Json<kr::Stan> {
    let ust = st.workspace.load_kronika();
    let sciezka = ust.sciezka(&st.workspace.root);
    let (liczniki, ostatnie, bajtow, rozpoznanie, kopia) = {
        let g = st.kronika.lock();
        match g.as_ref() {
            Some(k) => (
                k.liczniki.clone(),
                k.ostatnie(60),
                k.rozmiar(),
                Some(k.rozpoznanie().clone()),
                k.ostatnia_kopia().map(|p| p.display().to_string()),
            ),
            None => (
                kr::Liczniki::default(),
                Vec::new(),
                std::fs::metadata(&sciezka).map(|m| m.len()).unwrap_or(0),
                None,
                None,
            ),
        }
    };

    // Skąd biorą się wiadomości i czy w ogóle jest z czego nagrywać. Panel ma
    // powiedzieć DLACZEGO nic nie wpada — „0 zdarzeń" bez przyczyny wygląda
    // tak samo przy spokojnej nocy i przy martwej sesji MTProto.
    let (tg_ok, bledy_pingu) = st.read(|s| {
        (
            s.connection.telegram == "connected",
            s.connection.telegram_ping_failures,
        )
    });
    let opis = if !ust.wlaczona {
        "Zapis wyłączony w opcjach — bot pracuje, ale kronika nic nie zapisuje.".to_string()
    } else if !tg_ok {
        "Telegram rozłączony — dopóki nie wróci, nie ma czego zapisywać.".to_string()
    } else if bledy_pingu > 0 {
        format!(
            "Telegram zgłasza {bledy_pingu} niepotwierdzonych keepalive pod rząd — \
             sesja może być martwa mimo statusu „połączony”."
        )
    } else {
        "Podpięta pod strumień bota — nagrywa wszystko, co odbiera Conduit, \
         PRZED filtrem nasłuchiwanych kanałów."
            .to_string()
    };

    Json(kr::Stan {
        ok: true,
        wersja: env!("CARGO_PKG_VERSION").to_string(),
        tryb: kr::Tryb::Wbudowana,
        plik: sciezka.display().to_string(),
        domyslny_plik: st.workspace.domyslny_plik_kroniki(),
        istnieje: sciezka.is_file(),
        rozpoznanie,
        kopia,
        bajtow,
        plikow: kr::pliki_kroniki(&sciezka).len(),
        ustawienia: ust,
        liczniki,
        ostatnie,
        zrodlo_zywe: tg_ok && bledy_pingu == 0,
        zrodlo_opis: opis,
    })
}

async fn statystyki(State(st): State<StateHandle>) -> Json<kr::Statystyki> {
    let zrodla = vec![
        st.workspace.load_kronika().sciezka(&st.workspace.root),
        st.workspace.archive_dir(),
    ];
    let s = tokio::task::spawn_blocking(move || kr::statystyki_z(&zrodla))
        .await
        .unwrap_or_default();
    Json(s)
}

// ============================================================
//  KANAŁY
// ============================================================

/// Lista źródeł do zaznaczenia.
///
/// Łączy trzy rzeczy, których żadna z osobna nie wystarcza:
///  * czaty konta z Telegrama — żeby dało się zaznaczyć kanał, z którego
///    jeszcze nic nie przyszło,
///  * powiązania bota (`monitored`) — żeby było widać, co bot HANDLUJE,
///    a co tylko nagrywamy; to są dwie niezależne decyzje,
///  * liczbę wierszy w pliku — żeby zaznaczanie odbywało się na podstawie
///    tego, ile z danego źródła już zebrano.
async fn kanaly(State(st): State<StateHandle>) -> Json<serde_json::Value> {
    let a = st.auth.clone();
    let (lista, blad_listy) = match tokio::task::spawn_blocking(move || a.list_channels()).await {
        Ok(Ok(v)) => (v, None),
        Ok(Err(e)) => (Vec::new(), Some(e.to_string())),
        Err(e) => (
            Vec::new(),
            Some(format!("pobranie listy czatów nie doszło do skutku: {e}")),
        ),
    };

    let ust = st.workspace.load_kronika();
    let sciezka = ust.sciezka(&st.workspace.root);
    let wpisy = tokio::task::spawn_blocking(move || kr::czytaj(&sciezka).wpisy)
        .await
        .unwrap_or_default();
    let ile = kr::api::wpisow_wg_zrodla(&wpisy);
    let bindings = st.read(|s| s.bindings.clone());

    let mut out: Vec<kr::KanalInfo> = lista
        .into_iter()
        .map(|c| {
            let b = bindings.get(&c.id.to_string());
            let tematy: Vec<kr::TematInfo> = c
                .topics
                .iter()
                .map(|t| kr::TematInfo {
                    id: t.id,
                    nazwa: t.title.clone(),
                    nagrywany: ust.zrodla.pasuje(c.id, Some(t.id)),
                    wpisow: ile.get(&(c.id, Some(t.id))).copied().unwrap_or(0),
                })
                .collect();
            let wlasne = ile.get(&(c.id, None)).copied().unwrap_or(0);
            let w_tematach: u64 = tematy.iter().map(|t| t.wpisow).sum();
            kr::KanalInfo {
                chat_id: c.id,
                nazwa: c.name,
                handle: c.handle,
                forum: c.is_forum,
                tematy,
                nasluchiwany: b.map(|x| x.monitored).unwrap_or(false),
                nagrywany: ust.zrodla.pasuje(c.id, None),
                wpisow: wlasne + w_tematach,
            }
        })
        .collect();

    // Źródło, z którego COŚ już zebraliśmy, a którego nie ma na liście czatów
    // (kanał opuszczony, sesja niezalogowana, lista jeszcze nie pobrana) musi
    // być widoczne — inaczej nie da się go odznaczyć ani zrozumieć, skąd biorą
    // się wiersze w pliku.
    let znane: std::collections::HashSet<i64> = out.iter().map(|k| k.chat_id).collect();
    let mut osierocone: std::collections::BTreeMap<i64, u64> = std::collections::BTreeMap::new();
    for ((chat_id, _), n) in &ile {
        if !znane.contains(chat_id) {
            *osierocone.entry(*chat_id).or_insert(0) += n;
        }
    }
    for (chat_id, n) in osierocone {
        let nazwa = wpisy
            .iter()
            .find(|w| w.chat_id == chat_id && !w.chat.is_empty())
            .map(|w| w.chat.clone())
            .unwrap_or_else(|| chat_id.to_string());
        out.push(kr::KanalInfo {
            chat_id,
            nazwa,
            handle: None,
            forum: false,
            tematy: Vec::new(),
            nasluchiwany: false,
            nagrywany: ust.zrodla.pasuje(chat_id, None),
            wpisow: n,
        });
    }

    out.sort_by(|a, b| b.wpisow.cmp(&a.wpisow).then_with(|| a.nazwa.cmp(&b.nazwa)));
    Json(serde_json::json!({ "ok": true, "kanaly": out, "blad": blad_listy }))
}

// ============================================================
//  OPCJE ZAPISU
// ============================================================

async fn ustaw(State(st): State<StateHandle>, Json(nowe): Json<kr::Ustawienia>) -> Response {
    if let Err(e) = nowe.sprawdz() {
        return blad(StatusCode::BAD_REQUEST, e);
    }
    let teraz = crate::now_ms();
    let stare = st.workspace.load_kronika();

    // Kolejność: NAJPIERW plik konfiguracji, potem rejestrator. Odwrotna
    // zostawiłaby po nieudanym zapisie ustawień rejestrator pracujący wg opcji,
    // których nigdzie nie ma — po restarcie wróciłby do starych bez słowa.
    if let Err(e) = st.workspace.save_kronika(&nowe) {
        return blad(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("zapis kronika.json: {e:#}"),
        );
    }

    let wynik = {
        let mut g = st.kronika.lock();
        match g.as_mut() {
            Some(k) => k.przestaw(nowe.clone(), teraz),
            None => {
                // Rejestratora jeszcze nie ma (np. włączono go właśnie teraz) —
                // otwieramy od zera.
                match kr::Kronika::otworz(nowe.clone(), &st.workspace.root, teraz) {
                    Ok(k) => {
                        *g = Some(k);
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
        }
    };
    if let Err(e) = wynik {
        // Konfiguracja już zapisana, więc mówimy wprost, co obowiązuje:
        // po restarcie zadziała nowa, teraz pracuje stara.
        return blad(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "opcje zapisane, ale rejestrator ich nie przyjął: {e:#}. \
                 Do restartu pracuje poprzedni plik ({})",
                stare.plik
            ),
        );
    }
    st.log(
        "kronika",
        "info",
        "Zmieniono opcje zapisu kroniki",
        format!(
            "plik: {} · źródła: {} · nierozpoznane: {} · fsync: {:?}",
            nowe.plik,
            match nowe.zrodla.ile() {
                None => "wszystkie".to_string(),
                Some(n) => format!("{n} zaznaczonych"),
            },
            if nowe.nierozpoznane {
                "zapisuję"
            } else {
                "POMIJAM"
            },
            nowe.fsync
        ),
    );
    Json(serde_json::json!({ "ok": true, "ustawienia": nowe })).into_response()
}

// ============================================================
//  EKSPORT
// ============================================================

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ZadanieEksportu {
    plik: Option<String>,
    tylko_zaznaczone: bool,
}

/// Zbiór backtestowy z kroniki.
///
/// To jest WYGODA, nie warunek użyteczności: plik `.jsonl` jest kompletny sam
/// z siebie. Przycisk istnieje po to, żeby nie trzeba było pisać skryptu za
/// każdym razem, gdy chce się puścić backtest na świeżo zebranych danych.
async fn eksport(State(st): State<StateHandle>, Json(z): Json<ZadanieEksportu>) -> Response {
    let ust = st.workspace.load_kronika();
    let sciezka = ust.sciezka(&st.workspace.root);
    let nazwa = z.plik.unwrap_or_else(|| "signals_kronika.json".into());
    if nazwa.contains("..") || nazwa.contains('/') || nazwa.contains('\\') || nazwa.contains(':') {
        return blad(
            StatusCode::BAD_REQUEST,
            "nazwa pliku nie może zawierać ścieżki",
        );
    }
    let cel = st.workspace.root.join(&nazwa);
    let filtr = if z.tylko_zaznaczone {
        Some(ust.zrodla.clone())
    } else {
        None
    };
    match tokio::task::spawn_blocking(move || kr::eksportuj(&sciezka, &cel, filtr)).await {
        Ok(Ok(p)) => {
            st.log(
                "kronika",
                "success",
                format!("Wyeksportowano {} sygnałów z kroniki", p.sygnalow),
                format!(
                    "{}\nzdarzeń: {} (w tym z EDYCJI: {} — tego eksport z aplikacji Telegram NIE MA)",
                    p.plik, p.zdarzen, p.z_edycji
                ),
            );
            Json(serde_json::json!({ "ok": true, "wynik": p })).into_response()
        }
        Ok(Err(e)) => blad(StatusCode::CONFLICT, format!("{e:#}")),
        Err(e) => blad(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("eksport padł: {e}"),
        ),
    }
}

async fn pobierz(State(st): State<StateHandle>) -> Response {
    let sciezka = st.workspace.load_kronika().sciezka(&st.workspace.root);
    match std::fs::read(&sciezka) {
        Ok(d) => {
            let nazwa = sciezka
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("kronika.jsonl")
                .to_string();
            (
                [
                    (header::CONTENT_TYPE, "application/x-ndjson".to_string()),
                    (
                        header::CONTENT_DISPOSITION,
                        format!("attachment; filename=\"{nazwa}\""),
                    ),
                ],
                d,
            )
                .into_response()
        }
        Err(_) => blad(
            StatusCode::NOT_FOUND,
            "pliku kroniki jeszcze nie ma — zapis rusza z pierwszą wiadomością z Telegrama",
        ),
    }
}

#[cfg(test)]
mod testy {
    use super::*;

    fn stan_testowy(tag: &str) -> (StateHandle, std::path::PathBuf) {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "conduit-kron-{tag}-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let cfg = crate::ServerConfig {
            workspace: dir.clone(),
            ..Default::default()
        };
        (crate::bootstrap(&cfg, crate::default_auth()).unwrap(), dir)
    }

    #[tokio::test]
    async fn stan_dziala_na_swiezym_katalogu() {
        let (st, dir) = stan_testowy("stan");
        let s = stan(State(st)).await.0;
        assert!(s.ok);
        assert!(matches!(s.tryb, kr::Tryb::Wbudowana));
        assert!(s.plik.ends_with("kronika.jsonl"));
        // rejestrator ruszył przy starcie, więc plik ISTNIEJE i ma znacznik `start`
        assert!(
            s.istnieje,
            "kronika ma nagrywać od pierwszej sekundy pracy bota"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn opcje_zapisu_przezywaja_zapis_i_wracaja_w_stanie() {
        let (st, dir) = stan_testowy("opcje");
        let mut u = st.workspace.load_kronika();
        u.zrodla = kr::Zrodla::Wybrane {
            lista: vec![kr::Zrodlo {
                chat_id: -100,
                temat: None,
            }],
        };
        u.nierozpoznane = false;
        let r = ustaw(State(st.clone()), Json(u.clone())).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(
            st.workspace.load_kronika(),
            u,
            "opcje muszą przeżyć restart"
        );
        let s = stan(State(st)).await.0;
        assert!(!s.ustawienia.nierozpoznane);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn pusty_wybor_kanalow_jest_odrzucany_a_nie_zapisywany() {
        let (st, dir) = stan_testowy("pusty");
        let mut u = st.workspace.load_kronika();
        u.zrodla = kr::Zrodla::Wybrane { lista: vec![] };
        let r = ustaw(State(st.clone()), Json(u)).await;
        assert_eq!(
            r.status(),
            StatusCode::BAD_REQUEST,
            "zapis do nikąd musi być odrzucony"
        );
        assert!(
            matches!(st.workspace.load_kronika().zrodla, kr::Zrodla::Wszystkie),
            "odrzucone opcje nie mogą wylądować w pliku"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn eksport_nie_wypuszcza_sciezki_poza_katalog() {
        let (st, dir) = stan_testowy("sciezka");
        for zla in ["../poza.json", "a/b.json", "C:/tmp/x.json"] {
            let r = eksport(
                State(st.clone()),
                Json(ZadanieEksportu {
                    plik: Some(zla.into()),
                    tylko_zaznaczone: false,
                }),
            )
            .await;
            assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{zla}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn statystyka_z_pustego_pliku_nie_wybucha() {
        let (st, dir) = stan_testowy("statystyka");
        let s = statystyki(State(st)).await.0;
        assert_eq!(s.wiadomosci, 0);
        assert_eq!(s.procent_edytowanych, 0.0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn zapis_przez_stan_laduje_w_pliku_i_w_statystyce() {
        let (st, dir) = stan_testowy("zapis");
        let t = crate::now_ms();
        st.kronika_zapisz(
            kr::Przychodzace {
                odebrano_ms: t,
                rodzaj: kr::Rodzaj::Nowa,
                chat_id: -100,
                chat: "ATFX",
                temat: None,
                msg_id: 1,
                reply_to: None,
                edit_of: None,
                ts_telegram_ms: t,
                text: "RISK FREE 4057",
                nasluchiwany: true,
                format: Some("ATFX"),
            },
            true,
        );
        st.kronika_zapisz(
            kr::Przychodzace {
                odebrano_ms: t + 30_000,
                rodzaj: kr::Rodzaj::Edycja,
                chat_id: -100,
                chat: "ATFX",
                temat: None,
                msg_id: 1,
                reply_to: None,
                edit_of: Some(1),
                ts_telegram_ms: t,
                text: "RISK FREE 4057\nTP1 4060",
                nasluchiwany: true,
                format: Some("ATFX"),
            },
            true,
        );
        let s = statystyki(State(st)).await.0;
        assert_eq!(s.wiadomosci, 1);
        assert_eq!(s.edytowanych, 1);
        assert_eq!(s.procent_edytowanych, 100.0);
        assert_eq!(s.do_pierwszej.p50_s, 30.0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
