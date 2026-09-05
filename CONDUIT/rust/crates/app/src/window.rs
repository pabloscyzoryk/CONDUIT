//! Okno natywne Windows — Tauri v2 na WebView2.
//!
//! **Dlaczego WebView2, a nie natywny widżet.** Wymaganie brzmi: to samo UI
//! w oknie i w przeglądarce, jednocześnie, ostre w 4K. WebView2 ładuje
//! DOKŁADNIE ten sam adres, który obsługuje przeglądarka
//! (`http://127.0.0.1:8787`), więc:
//!
//!  * nie ma drugiej implementacji interfejsu, która mogłaby się rozjechać,
//!  * okno nie ma własnego stanu — jest kolejnym klientem WebSocketa,
//!  * skalowanie DPI (a więc ostrość na 4K) obsługuje sam WebView2; Tauri
//!    ustawia świadomość DPI w manifeście aplikacji, my nie musimy nic liczyć.
//!    Interfejs jest zbudowany na jednostkach względnych, więc dostajemy
//!    ostrość za darmo.
//!
//! Okno dostaje sufiks `?shell=native` — po nim React wie, że ma pokazać
//! przycisk „Otwórz w przeglądarce". To jedyna różnica między powłokami.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

/// Zapamiętana geometria okna (`window.json` w katalogu konfiguracji).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Geometria {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    maximized: bool,
}

impl Default for Geometria {
    fn default() -> Self {
        // 1560×980 mieści cały pulpit (2 kolumny: 1fr + 372 px) bez zwijania
        Geometria {
            x: 80.0,
            y: 60.0,
            w: 1560.0,
            h: 980.0,
            maximized: false,
        }
    }
}

fn plik_geometrii(dir: &Path) -> PathBuf {
    dir.join("window.json")
}

fn wczytaj_geometrie(dir: &Path) -> Geometria {
    std::fs::read_to_string(plik_geometrii(dir))
        .ok()
        .and_then(|r| serde_json::from_str(&r).ok())
        .map(sanity)
        .unwrap_or_default()
}

/// Chroni przed wskrzeszeniem okna poza ekranem (odłączony drugi monitor)
/// i przed absurdalnie małym rozmiarem po awarii zapisu.
///
/// Próg pozycji jest CELOWO ciasny (−3000). Windows melduje zminimalizowane
/// okno na pozycji −32000, a przy skalowaniu 200 % zapisywało się to jako
/// logiczne −16000 — i przy następnym starcie okno wstawało poza wszystkimi
/// monitorami. Użytkownik widział „serwer wstaje, okna nie ma".
fn sanity(mut g: Geometria) -> Geometria {
    if !g.w.is_finite() || g.w < 900.0 {
        g.w = 1560.0;
    }
    if !g.h.is_finite() || g.h < 600.0 {
        g.h = 980.0;
    }
    if !g.x.is_finite()
        || !g.y.is_finite()
        || g.x < -3000.0
        || g.y < -3000.0
        || g.x > 20000.0
        || g.y > 20000.0
    {
        g.x = 80.0;
        g.y = 60.0;
    }
    g
}

/// Czy tę geometrię wolno w ogóle ZAPISAĆ.
///
/// To jest właściwe miejsce naprawy: nie wolno utrwalać stanu okna
/// zminimalizowanego ani zerowego rozmiaru. Sam `sanity()` przy odczycie nie
/// wystarczał — plik i tak zapełniał się śmieciem po każdej minimalizacji,
/// a wystarczyła jedna wersja bez sanity, żeby okno zniknęło na dobre.
fn wolno_zapisac(g: &Geometria) -> bool {
    g.w.is_finite()
        && g.h.is_finite()
        && g.x.is_finite()
        && g.y.is_finite()
        && g.w >= 200.0
        && g.h >= 200.0
        && g.x > -3000.0
        && g.y > -3000.0
}

fn zapisz_geometrie(dir: &Path, g: &Geometria) {
    if let Err(e) = conduit_server::store::write_json_atomic(&plik_geometrii(dir), g) {
        tracing::warn!(blad = %e, "nie udało się zapisać geometrii okna");
    }
}

/// Ikona zasobnika rysowana w kodzie — 32×32 RGBA.
///
/// Świadomie bez pliku `.ico`: binarka ma być jednym plikiem, a znak jest
/// prosty (kwadrat akcentu z łamaną linią wykresu — jak logo w interfejsie).
fn ikona() -> tauri::image::Image<'static> {
    const N: usize = 32;
    let mut px = vec![0u8; N * N * 4];
    // indygo #4f5ce8 — ten sam akcent co w palecie domyślnej
    let (r, g, b) = (0x4f, 0x5c, 0xe8);
    for y in 0..N {
        for x in 0..N {
            let i = (y * N + x) * 4;
            // zaokrąglone rogi: odcinamy narożniki promieniem 7 px
            let rog = 7.0;
            let dx = (x as f64 + 0.5 - N as f64 / 2.0).abs() - (N as f64 / 2.0 - rog);
            let dy = (y as f64 + 0.5 - N as f64 / 2.0).abs() - (N as f64 / 2.0 - rog);
            let poza = dx > 0.0 && dy > 0.0 && (dx * dx + dy * dy).sqrt() > rog;
            if poza {
                continue;
            }
            px[i] = r;
            px[i + 1] = g;
            px[i + 2] = b;
            px[i + 3] = 0xff;
        }
    }
    // biała łamana: w dół i ostro w górę — ruch ceny trafiający w punkt
    let punkty: [(i32, i32); 4] = [(7, 21), (13, 13), (17, 18), (24, 8)];
    for para in punkty.windows(2) {
        let ((x0, y0), (x1, y1)) = (para[0], para[1]);
        let kroki = ((x1 - x0).abs().max((y1 - y0).abs())) * 4;
        for k in 0..=kroki {
            let t = k as f64 / kroki as f64;
            let x = (x0 as f64 + (x1 - x0) as f64 * t).round() as i32;
            let y = (y0 as f64 + (y1 - y0) as f64 * t).round() as i32;
            for oy in -1..=1 {
                for ox in -1..=1 {
                    let (px_, py_) = (x + ox, y + oy);
                    if (0..N as i32).contains(&px_) && (0..N as i32).contains(&py_) {
                        let i = (py_ as usize * N + px_ as usize) * 4;
                        px[i] = 0xff;
                        px[i + 1] = 0xff;
                        px[i + 2] = 0xff;
                        px[i + 3] = 0xff;
                    }
                }
            }
        }
    }
    tauri::image::Image::new_owned(px, N as u32, N as u32)
}

/// Uruchamia okno. Blokuje wątek główny aż do zamknięcia aplikacji.
///
/// Okno NIE dostaje uchwytu do stanu i to jest celowe: cały stan bierze
/// z `url` przez WebSocket, tak samo jak przeglądarka. Dzięki temu tą samą
/// funkcją otwieramy okno na WŁASNY serwer i na serwer instancji, która już
/// działała, gdy ktoś kliknął `conduit.exe` po raz drugi.
pub fn run(url: &str, lab: bool, data_dir: &Path) -> Result<()> {
    // `?view=lab` wybiera tylko widok startowy — reszta powłoki jest ta sama
    let url_okna = format!("{url}/?shell=native{}", if lab { "&view=lab" } else { "" });
    let url_przegladarki = url.to_string();
    let dir = data_dir.to_path_buf();
    let dir_do_zdarzen = dir.clone();

    let app = tauri::Builder::default()
        .setup(move |app| {
            let g = wczytaj_geometrie(&dir);

            let okno = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External(url_okna.parse().context("zły adres okna")?),
            )
            .title("CONDUIT — Telegram → MetaTrader 5")
            .inner_size(g.w, g.h)
            .position(g.x, g.y)
            // poniżej tej szerokości pulpit i tak składa kolumny w jedną,
            // a tabele zaczynają się przewijać poziomo we własnych kontenerach
            .min_inner_size(1024.0, 640.0)
            .resizable(true)
            .build()?;

            // ---------- okno MA być widoczne ----------
            // Zapisana pozycja mogła pochodzić z monitora, którego już nie ma.
            // Sprawdzamy to u systemu, a nie na podstawie samego pliku: jeśli
            // okno nie leży na ŻADNYM podłączonym monitorze — na środek.
            let na_ekranie = match (okno.outer_position(), okno.available_monitors()) {
                (Ok(poz), Ok(mons)) if !mons.is_empty() => mons.iter().any(|m| {
                    let p = m.position();
                    let s = m.size();
                    poz.x >= p.x - 64
                        && poz.y >= p.y - 64
                        && poz.x < p.x + s.width as i32
                        && poz.y < p.y + s.height as i32
                }),
                // Nie wiemy — nie ruszamy. Lepiej zostawić, niż przestawiać
                // okno użytkownikowi przy każdym starcie.
                _ => true,
            };
            if !na_ekranie {
                tracing::warn!("okno wypadło poza monitory — ustawiam na środku");
                let _ = okno.center();
            }

            if g.maximized {
                let _ = okno.maximize();
            }

            // Bez tego okno potrafiło zostać zminimalizowane albo schowane
            // z poprzedniej sesji — proces działał, a użytkownik go nie widział.
            let _ = okno.unminimize();
            let _ = okno.show();
            let _ = okno.set_focus();

            // ---------- zasobnik systemowy ----------
            let poz_pokaz = MenuItem::with_id(app, "pokaz", "Pokaż okno", true, None::<&str>)?;
            let poz_przegladarka = MenuItem::with_id(
                app,
                "przegladarka",
                "Otwórz w przeglądarce",
                true,
                None::<&str>,
            )?;
            let poz_wyjdz = MenuItem::with_id(app, "wyjdz", "Zakończ", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&poz_pokaz, &poz_przegladarka, &poz_wyjdz])?;

            let url_dla_menu = url_przegladarki.clone();
            TrayIconBuilder::with_id("conduit")
                .icon(ikona())
                .tooltip("CONDUIT — serwer działa")
                .menu(&menu)
                .on_menu_event(move |app, ev| match ev.id().as_ref() {
                    "pokaz" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                    }
                    "przegladarka" => {
                        if let Err(e) = open::that_detached(url_dla_menu.as_str()) {
                            tracing::error!(blad = %e, "nie udało się otworzyć przeglądarki");
                        }
                    }
                    "wyjdz" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(move |okno, event| {
            use tauri::WindowEvent;
            // Geometrię zapisujemy przy każdym ruchu i zmianie rozmiaru.
            // Zapis jest atomowy i tani (jeden mały plik), a dzięki temu
            // przeżywa również twarde ubicie procesu — nie tylko czyste
            // zamknięcie okna.
            if matches!(
                event,
                WindowEvent::Moved(_)
                    | WindowEvent::Resized(_)
                    | WindowEvent::CloseRequested { .. }
            ) {
                if let (Ok(poz), Ok(rozm), Ok(maxi), Ok(skala)) = (
                    okno.outer_position(),
                    okno.inner_size(),
                    okno.is_maximized(),
                    okno.scale_factor(),
                ) {
                    // Rozmiary z API są w pikselach FIZYCZNYCH; zapisujemy
                    // logiczne, żeby przeniesienie okna między monitorem 4K
                    // a zwykłym nie zmieniało jego rozmiaru na ekranie.
                    let g = Geometria {
                        x: poz.x as f64 / skala,
                        y: poz.y as f64 / skala,
                        w: rozm.width as f64 / skala,
                        h: rozm.height as f64 / skala,
                        maximized: maxi,
                    };
                    // Zminimalizowane okno Windows melduje na −32000/0×0.
                    // Zapisanie tego = okno, którego przy następnym starcie
                    // nie widać. Sprawdzamy jedno i drugie.
                    let zminimalizowane = okno.is_minimized().unwrap_or(false);
                    if !zminimalizowane
                        && wolno_zapisac(&g)
                        && (!maxi || matches!(event, WindowEvent::CloseRequested { .. }))
                    {
                        zapisz_geometrie(&dir_do_zdarzen, &g);
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .context("nie udało się zbudować aplikacji Tauri (czy WebView2 jest zainstalowany?)")?;

    app.run(|_app, _event| {});
    Ok(())
}
