//! Nadzór nad terminalem MetaTrader 5 — wpięcie w `conduit.exe`.
//!
//! Logika (harmonogram prób, restart aplikacji, wykrywanie ścieżki) mieszka
//! w `conduit_mt5::watchdog` i jest przetestowana na atrapie. Tutaj jest tylko
//! spięcie: odczyt ustawień, własny wątek i przekazywanie zdarzeń do dziennika
//! oraz do powiadomień e-mail.
//!
//! # Dlaczego osobny wątek, a nie zadanie tokio
//!
//! Nadzorca woła `tasklist`, `taskkill` i `Command::spawn`, a potem czeka
//! sekundy albo godziny. To są operacje BLOKUJĄCE. Zadanie tokio, które śpi
//! osiem godzin, zajmuje wątek wykonawczy i odbiera go obsłudze HTTP.
//!
//! # Co dokładnie jest sprawdzane
//!
//! Obecność procesu `terminal64.exe`. To jest test, który odpowiada na pytanie
//! z zadania: „czy MT5 został wyłączony w trakcie pracy". Gdy most do MT5
//! zostanie podłączony, wystarczy podmienić funkcję `probe` na wywołanie
//! `Mt5Bridge::ping()` — reszta nadzoru zostaje bez zmian.

use conduit_mt5::watchdog::{
    Recovery, TerminalControl, Watchdog, WatchdogConfig, WatchdogEvent, WindowsTerminal,
};
use conduit_server::mailer::MailCategory;
use conduit_server::StateHandle;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Uchwyt do zatrzymania nadzoru przy zamykaniu programu.
pub struct Guard {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Guard {
    /// Zatrzymuje nadzór. NIE czeka na wątek w nieskończoność — nadzorca może
    /// akurat spać osiem godzin między seriami prób, a program ma się zamknąć.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // wątek jest `detached` w praktyce: flaga wystarcza, żeby nie robił
        // niczego więcej, a proces i tak zaraz zniknie
        drop(self.handle.take());
    }
}

/// Czyta ustawienia nadzoru z dokumentu panelu.
///
/// Klucze celowo te same, co w `settings_map.rs` — jedno źródło nazw.
pub fn config_from_state(st: &StateHandle) -> (WatchdogConfig, bool) {
    let doc = st.read(|s| s.settings.clone());
    let f = |k: &str, d: f64| doc.get(k).and_then(|v| v.as_f64()).unwrap_or(d);
    let b = |k: &str, d: bool| doc.get(k).and_then(|v| v.as_bool()).unwrap_or(d);
    let s = |k: &str| {
        doc.get(k)
            .and_then(|v| v.as_str())
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
    };

    let follow = b("mt5_follow_terminal_account", false);
    let cfg = WatchdogConfig {
        terminal_path: s("mt5_terminal_path").map(PathBuf::from),
        manage_process: !follow && b("mt5_watchdog", true),
        start_on_launch: !follow && b("mt5_autostart", true),
        attempts_per_cycle: f("mt5_retry_attempts", 10.0).max(1.0) as u32,
        attempt_delay: Duration::from_secs_f64(f("mt5_retry_delay_s", 5.0).max(0.5)),
        restart_after: f("mt5_restart_after", 1.0).max(0.0) as u32,
        health_interval: Duration::from_secs_f64(f("mt5_health_interval_s", 5.0).max(1.0)),
        ..Default::default()
    }
    .sanitized();
    let wlaczony = !follow && b("mt5_watchdog", true);
    (cfg, wlaczony)
}

/// Startuje nadzór. Zwraca `None`, gdy jest wyłączony w ustawieniach.
pub fn start(st: StateHandle) -> Option<Guard> {
    if !st.mt5_runtime_start_allowed() {
        tracing::warn!("nadzór MT5 nie startuje: niepotwierdzona konfiguracja startowa");
        return None;
    }
    let (cfg, wlaczony) = config_from_state(&st);
    if !wlaczony && !cfg.start_on_launch {
        tracing::info!("nadzór nad MT5 wyłączony w ustawieniach");
        return None;
    }

    let terminal = WindowsTerminal::new(&cfg, || {
        // Probe: czy proces terminala żyje. Gdy most do MT5 będzie podłączony,
        // to jest jedyne miejsce do zmiany — reszta nadzoru zostaje.
        if conduit_mt5::watchdog::WindowsTerminal::new(&WatchdogConfig::default(), || Ok(()))
            .is_running()
        {
            Ok(())
        } else {
            Err(format!(
                "proces {} nie działa",
                conduit_mt5::watchdog::PROCES
            ))
        }
    });

    if let Some(p) = terminal.path() {
        tracing::info!(sciezka = %p.display(), "MT5: znaleziono terminal");
    }

    let st_ev = st.clone();
    let health = cfg.health_interval;
    let start_on_launch = cfg.start_on_launch;
    let manage = cfg.manage_process;

    let w = Watchdog::new(cfg, Box::new(terminal), move |e| zglos(&st_ev, e));
    let stop = w.stop_flag();
    let stop2 = Arc::clone(&stop);

    let handle = std::thread::Builder::new()
        .name("mt5-watchdog".into())
        .spawn(move || {
            if st.read(|s| s.settings.get("mt5_follow_terminal_account").and_then(|v| v.as_bool()).unwrap_or(false)) {
                return;
            }
            if start_on_launch {
                w.ensure_started();
            }
            if !manage {
                // Nadzór wyłączony: podnieśliśmy terminal przy starcie i tyle.
                return;
            }
            let ctrl_ok = || {
                conduit_mt5::watchdog::WindowsTerminal::new(&WatchdogConfig::default(), || Ok(()))
                    .is_running()
            };

            let mut bylo_ok = ctrl_ok();
            if bylo_ok {
                zglos(&st, WatchdogEvent::Connected);
                oznacz_polaczenie(&st, true);
            }

            loop {
                if stop2.load(Ordering::Relaxed) {
                    return;
                }

                if st.read(|s| s.settings.get("mt5_follow_terminal_account").and_then(|v| v.as_bool()).unwrap_or(false)) {
                    // A runtime switch to follow mode permanently retires this process manager.
                    return;
                }
                std::thread::sleep(health);
                if stop2.load(Ordering::Relaxed) {
                    return;
                }

                if st.read(|s| s.settings.get("mt5_follow_terminal_account").and_then(|v| v.as_bool()).unwrap_or(false)) {
                    return;
                }
                let teraz_ok = ctrl_ok();
                if teraz_ok {
                    if !bylo_ok {
                        bylo_ok = true;
                        oznacz_polaczenie(&st, true);
                    }
                    continue;
                }

                // TERMINAL ZNIKNĄŁ — pełny cykl odzyskiwania.
                oznacz_polaczenie(&st, false);
                match w.recover("proces terminal64.exe zniknął") {
                    Recovery::Recovered { .. } => {
                        bylo_ok = true;
                        oznacz_polaczenie(&st, true);
                    }
                    Recovery::Aborted => return,
                }
            }
        })
        .ok()?;

    Some(Guard {
        stop,
        handle: Some(handle),
    })
}

/// Zdarzenie nadzorcy → dziennik + (jeśli warte) mail.
///
/// Kategoria maila zależy od zdarzenia, żeby wyciszenie „nieudanego wznowienia"
/// nie wyciszyło samej informacji o utracie połączenia.
fn zglos(st: &StateHandle, e: WatchdogEvent) {
    let tytul = e.title();
    let poziom = e.level();
    let tresc = opis(&e);

    if e.worth_mailing() {
        let kat = match e {
            WatchdogEvent::CycleFailed { .. } | WatchdogEvent::NotFound { .. } => {
                MailCategory::Mt5RecoveryFailed
            }
            _ => MailCategory::Mt5Connection,
        };
        st.notify(kat, &tytul, &tresc);
    } else {
        // Pojedyncza nieudana próba nie idzie mailem (byłoby ich dziesięć
        // w pięćdziesiąt sekund), ale w dzienniku MUSI być — to jest ślad,
        // po którym diagnozuje się, dlaczego bot stał.
        st.log("mt5", poziom, tytul, tresc);
    }
}

fn opis(e: &WatchdogEvent) -> String {
    match e {
        WatchdogEvent::Launched { path } => format!("Uruchomiono: {}", path.display()),
        WatchdogEvent::NotFound { szukano } => {
            let lista: Vec<String> = szukano
                .iter()
                .take(12)
                .map(|p| format!("  • {}", p.display()))
                .collect();
            format!(
                "Nie znaleziono {}. Sprawdzone lokalizacje:\n{}\n\nWpisz pełną ścieżkę \
                 w Ustawienia → MetaTrader 5 → „ścieżka do terminala”.",
                conduit_mt5::watchdog::PROCES,
                lista.join("\n")
            )
        }
        WatchdogEvent::Connected => "Terminal MetaTrader 5 działa.".to_string(),
        WatchdogEvent::Lost { detail } => format!(
            "Utracono kontakt z terminalem MT5.\nPowód: {detail}\n\n\
             Bot rozpoczyna serię prób wznowienia. Otwarte pozycje pozostają \
             u brokera — bot nie może nimi zarządzać, dopóki nie odzyska połączenia."
        ),
        WatchdogEvent::AttemptFailed {
            attempt,
            of,
            cycle,
            restarted,
            detail,
        } => format!(
            "Próba {attempt}/{of} w serii {} nieudana{}.\nSzczegóły: {detail}",
            cycle + 1,
            if *restarted {
                " (po restarcie aplikacji MT5)"
            } else {
                ""
            }
        ),
        WatchdogEvent::CycleFailed { cycle, wait } => format!(
            "Cała seria {} prób nieudana. Kolejna seria za {}.\n\n\
             Bot NIE kończy pracy — będzie próbował dalej, z coraz rzadszymi \
             podejściami, aż do skutku.",
            cycle + 1,
            conduit_mt5::watchdog::opis_czasu(*wait)
        ),
        WatchdogEvent::Restarting { forced } => {
            if *forced {
                "Terminal nie zamknął się łagodnie — wymuszam zamknięcie.".to_string()
            } else {
                "Zamykam i uruchamiam ponownie aplikację MetaTrader 5.".to_string()
            }
        }
        WatchdogEvent::Reconnected { after, attempts } => format!(
            "Połączenie z MT5 przywrócone po {} i {attempts} próbach.",
            conduit_mt5::watchdog::opis_czasu(*after)
        ),
    }
}

/// Odbicie stanu w polu `connection.mt5`, które widzi interfejs.
fn oznacz_polaczenie(st: &StateHandle, ok: bool) {
    use conduit_server::coalesce::{Section, Sections};
    st.update(Sections::one(Section::Connection), |s| {
        s.connection.mt5 = if ok {
            "connected".into()
        } else {
            "disconnected".into()
        };
        if !ok {
            s.connection.resolved_symbol.clear();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolved_symbol_is_cleared_by_watchdog_disconnect_not_invented_on_recovery() {
        let st = stan_z(json!({}));
        st.update(conduit_server::coalesce::Sections::all(), |s| s.connection.resolved_symbol = "XAUUSD.s".into());
        oznacz_polaczenie(&st, false);
        assert!(st.read(|s| s.connection.resolved_symbol.is_empty()));
        oznacz_polaczenie(&st, true);
        assert!(st.read(|s| s.connection.resolved_symbol.is_empty()), "only the broker publisher may establish a contract");
    }

    #[test]
    fn follow_terminal_disables_all_process_start_restart_paths() {
        let st = stan_z(json!({"mt5_follow_terminal_account":true,"mt5_watchdog":true,"mt5_autostart":true}));
        let (cfg, enabled)=config_from_state(&st);
        assert!(!enabled && !cfg.manage_process && !cfg.start_on_launch);
        assert!(start(st).is_none());
    }

    fn stan_z(ustawienia: serde_json::Value) -> StateHandle {
        let mut dir = std::env::temp_dir();
        // Nazwa musi być POPRAWNĄ nazwą katalogu: `Debug` dla `Instant` daje
        // „Instant { t: 123.45s }", a nawiasy i dwukropek są na Windows
        // zabronione (os error 267). Stąd liczba nanosekund, nie `{:?}`.
        let uniq = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        dir.push(format!(
            "conduit-guard-{tag}-{}-{uniq}",
            std::process::id(),
            tag = ustawienia.to_string().len()
        ));
        let cfg = conduit_server::ServerConfig {
            workspace: dir,
            ..Default::default()
        };
        let st = conduit_server::bootstrap(&cfg, conduit_server::default_auth()).unwrap();
        st.update(
            conduit_server::coalesce::Sections::one(conduit_server::coalesce::Section::Settings),
            |s| {
                s.settings = ustawienia;
            },
        );
        st
    }

    #[test]
    fn domyslne_ustawienia_daja_harmonogram_z_bot_py() {
        let st = stan_z(json!({}));
        let (cfg, wl) = config_from_state(&st);
        assert!(wl, "nadzór ma być domyślnie włączony");
        assert_eq!(cfg.attempts_per_cycle, 10);
        assert_eq!(cfg.attempt_delay, Duration::from_secs(5));
        assert_eq!(cfg.cycle_waits_s[0], 60);
        assert!(cfg.start_on_launch);
        assert!(
            cfg.terminal_path.is_none(),
            "bez wpisu ścieżkę wykrywamy sami"
        );
    }

    #[test]
    fn ustawienia_z_panelu_docieraja_do_nadzorcy() {
        let st = stan_z(json!({
            "mt5_terminal_path": "D:/MT5/terminal64.exe",
            "mt5_retry_attempts": 3,
            "mt5_retry_delay_s": 12,
            "mt5_restart_after": 2,
            "mt5_health_interval_s": 30,
            "mt5_autostart": false,
        }));
        let (cfg, _) = config_from_state(&st);
        assert_eq!(
            cfg.terminal_path,
            Some(PathBuf::from("D:/MT5/terminal64.exe"))
        );
        assert_eq!(cfg.attempts_per_cycle, 3);
        assert_eq!(cfg.attempt_delay, Duration::from_secs(12));
        assert_eq!(cfg.restart_after, 2);
        assert_eq!(cfg.health_interval, Duration::from_secs(30));
        assert!(!cfg.start_on_launch);
    }

    #[test]
    fn pusta_sciezka_znaczy_wykryj_a_nie_sciezka_pusta() {
        // typowa pułapka: użytkownik czyści pole i zostaje "" — to NIE może
        // znaczyć „szukaj terminala pod ścieżką pustą"
        let st = stan_z(json!({ "mt5_terminal_path": "   " }));
        let (cfg, _) = config_from_state(&st);
        assert!(cfg.terminal_path.is_none());
    }

    #[test]
    fn absurdalne_wartosci_sa_przycinane() {
        let st = stan_z(json!({
            "mt5_retry_attempts": 0,
            "mt5_retry_delay_s": 0,
            "mt5_health_interval_s": 0,
        }));
        let (cfg, _) = config_from_state(&st);
        assert!(cfg.attempts_per_cycle >= 1);
        assert!(cfg.attempt_delay >= Duration::from_millis(500));
        assert!(
            cfg.health_interval >= Duration::from_secs(1),
            "co 0 s = pętla zżerająca rdzeń"
        );
    }

    #[test]
    fn wylaczony_nadzor_nie_zarzadza_procesem() {
        let st = stan_z(json!({ "mt5_watchdog": false }));
        let (cfg, wl) = config_from_state(&st);
        assert!(!wl);
        assert!(
            !cfg.manage_process,
            "bez nadzoru nie wolno zabijać terminala"
        );
    }

    #[test]
    fn opisy_zdarzen_mowia_co_robic() {
        let e = WatchdogEvent::NotFound {
            szukano: vec![PathBuf::from("C:/x/terminal64.exe")],
        };
        let o = opis(&e);
        assert!(o.contains("terminal64.exe"));
        assert!(
            o.contains("Ustawienia"),
            "komunikat ma prowadzić do rozwiązania: {o}"
        );

        let e = WatchdogEvent::CycleFailed {
            cycle: 0,
            wait: Duration::from_secs(60),
        };
        let o = opis(&e);
        assert!(o.contains("NIE kończy pracy"), "{o}");
        assert!(o.contains("1 min"), "{o}");
    }
}
