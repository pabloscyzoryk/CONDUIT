//! Tryb `--demo`: uruchomienie wirtualnego brokera zaraz po starcie.
//!
//! **Co się zmieniło.** Wcześniej ta flaga rysowała wyłącznie syntetyczne
//! kwotowania, żeby dało się obejrzeć ścieżkę `stan → WebSocket → React` bez
//! MT5, i uczciwie mówiła, że bot nie handluje. Teraz tryb demo jest
//! pełnoprawnym trybem pracy (`conduit_server::demo`): ten sam silnik i ten sam
//! symulator brokera, co w backteście, tylko zegar płynie w czasie
//! rzeczywistym. Ten moduł jest już wyłącznie **rozrusznikiem** — decyduje,
//! z jaką konfiguracją wystartować, i oddaje robotę serwerowi.
//!
//! Zasada wyboru źródła ceny: bierzemy to, co zapisano w `demo.json`, ale jeśli
//! wskazanego pliku ticków nie ma na dysku, schodzimy do generatora i mówimy
//! o tym w dzienniku. Uruchomienie z flagą `--demo` ma ZAWSZE dać działający
//! obraz — milczące „nic się nie dzieje, bo brakuje pliku" byłoby najgorszą
//! z możliwych odpowiedzi.

use conduit_server::demo::PriceSource;
use conduit_server::StateHandle;

pub fn run(st: StateHandle) {
    let mut cfg = st.workspace.load_demo();

    if cfg.price_source == PriceSource::File {
        let jest =
            !cfg.ticks_path.trim().is_empty() && std::path::Path::new(&cfg.ticks_path).is_file();
        if !jest {
            // spróbuj tego, czego używa laboratorium — to najczęstszy układ
            let (ticks, signals) = conduit_server::lab::data_paths(&st.workspace);
            if ticks.is_file() {
                cfg.ticks_path = ticks.display().to_string();
                if cfg.signals_path.trim().is_empty() && signals.is_file() {
                    cfg.signals_path = signals.display().to_string();
                }
            } else {
                st.log(
                    "events",
                    "warn",
                    "TRYB DEMO: brak pliku ticków",
                    format!(
                        "nie znalazłem „{}” ani danych laboratorium — przechodzę na generator cen \
                         (błądzenie losowe z ziarnem {})",
                        cfg.ticks_path, cfg.seed
                    ),
                );
                cfg.price_source = PriceSource::Synthetic;
            }
        }
    }

    // Generator bez pliku sygnałów gra wyłącznie na sygnałach z panelu —
    // i tak ma być, bo daty w pliku nie mają nic wspólnego z wymyśloną ceną.
    if cfg.price_source == PriceSource::Synthetic && !cfg.signals_path.trim().is_empty() {
        let ma_okno = !cfg.ticks_from.trim().is_empty();
        if !ma_okno {
            cfg.use_file_signals = false;
        }
    }

    if let Err(e) = conduit_server::demo::start(&st, cfg) {
        st.log("events", "error", "TRYB DEMO nie ruszył", format!("{e:#}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domyslnie_gramy_z_pliku_tickow() {
        let c = conduit_server::demo::DemoConfig::default();
        assert_eq!(c.price_source, PriceSource::File);
        assert!(c.use_file_signals);
        assert_eq!(c.balance, 200.0);
    }
}
