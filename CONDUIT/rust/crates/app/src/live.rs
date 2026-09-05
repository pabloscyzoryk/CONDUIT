
#[cfg(test)]
#[path = "restart_differential_tests.rs"]
mod restart_differential_tests;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use conduit_core::broker::{Broker, BrokerError, OrderReq, PendingReq};
use conduit_core::engine::{Engine, IncomingMessage, SrWarmupBar, EngineContinuationV1,
    ContinuationOrigin, ContinuationReviewScope, ContinuationImportReport};
use conduit_core::types::*;
use conduit_mt5::{Mt5Bridge, SidecarConfig};
use conduit_server::coalesce::{Section, Sections};
use conduit_server::mailer::MailCategory;
use conduit_server::proto::Command;
use conduit_server::{ui, StateHandle};
use serde_json::Value;

use crate::routing;
use crate::wznowienie;

/// Co ile publikujemy stan do interfejsu. 250 ms to kompromis: oko widzi
/// płynność, a przy 60 kwotowaniach na sekundę nie zalewamy WebSocketa.
const PUBLISH_EVERY_MS: u128 = 250;

/// Ile zamkniętych transakcji trzymamy w panelu.
const CLOSED_KEEP: usize = 500;

/// Ile wiadomości trzymamy w panelu.
const MSG_KEEP: usize = 200;

/// Po ilu minutach ciszy w strumieniu kwotowań mówimy o tym głośno.
///
/// XAUUSD u Vantage kwotuje kilkadziesiąt razy na sekundę w sesji i milczy
/// w weekend oraz w przerwie serwisowej. Pięć minut ciszy w środku tygodnia
/// znaczy „rynek zamknięty albo terminal zamarł" — jedno i drugie użytkownik
/// ma zobaczyć w panelu, a nie domyślić się z tego, że wykres stoi.
const CISZA_KWOTOWAN_MIN: f64 = 5.0;

/// Po ilu minutach ciszy w kwotowaniach ODBUDOWUJEMY most do terminala.
///
/// Świadomie dłużej niż próg powiadomienia (5 min): najpierw człowiek dostaje
/// wiadomość, a dopiero gdy cisza trwa, bot działa sam. Krócej byłoby ryzykowne
/// — zwykła przerwa w notowaniach kasowałaby połączenie bez potrzeby.
const CISZA_ODBUDOWA_MIN: f64 = 12.0;

const SYGNAL_MAX_WIEK_MIN: f64 = 5.0;

/// Czy o tej porze notowania W OGÓLE powinny płynąć.
///
/// Bez tego bot przebudowywałby most co 12 minut przez cały weekend — cisza
/// przy zamkniętym rynku jest normalna i nie ma czego naprawiać. Granice biorą
/// się z danych: w `ticks.bin` przerwa tygodniowa wypada piątek 23:56 →
/// poniedziałek 01:00 czasu serwera (UTC+3).
pub(crate) fn rynek_powinien_dzialac(przerwa: (f64, f64)) -> bool {
    rynek_powinien_dzialac_z(przerwa, 3.0)
}

pub(crate) fn rynek_powinien_dzialac_z(przerwa: (f64, f64), offset_h: f64) -> bool {
    let sek = conduit_server::now_ms() / 1000 + (offset_h * 3600.0) as i64;
    rynek_czynny_o(sek, przerwa)
}

/// Offset serwera brokera w godzinach, z dokumentu panelu.
/// Brak klucza = 3,0 (EET latem) — czyli zachowanie sprzed zmiany.
pub(crate) fn offset_serwera_h(st: &StateHandle) -> f64 {
    st.read(|s| {
        s.settings
            .get("server_tz_offset_h")
            .and_then(|v| v.as_f64())
    })
    .filter(|v| v.is_finite() && v.abs() <= 18.0)
    .unwrap_or(3.0)
}

fn rynek_czynny_o(sek_serwera: i64, przerwa: (f64, f64)) -> bool {
    let dni = sek_serwera.div_euclid(86_400);
    let godz = sek_serwera.rem_euclid(86_400) / 3600;
    // 1970-01-01 był CZWARTKIEM, więc dzień 0 to czwartek (indeks 3 przy
    // poniedziałku = 0). Stąd przesunięcie o 3.
    let dzien = (dni + 3).rem_euclid(7); // 0 = poniedziałek … 6 = niedziela
    match dzien {
        5 | 6 => return false,           // sobota, niedziela
        4 if godz >= 23 => return false, // piątek — notowania gasną ok. 23:56
        0 if godz < 1 => return false,   // poniedziałek — ruszają ok. 01:00
        _ => {}
    }
    // ---- przerwa DOBOWA (minutowa rozdzielczość, bo 01:05 ≠ 01:00) ----
    let (od, do_) = przerwa;
    if (od - do_).abs() < 1e-9 {
        return true; // okno puste = przerwa wyłączona
    }
    let min_dnia = (sek_serwera.rem_euclid(86_400) / 60) as f64 / 60.0; // godzina ułamkowo
    let w_przerwie = if od < do_ {
        min_dnia >= od && min_dnia < do_
    } else {
        // okno przechodzi przez północ (np. 23,5 → 0,5)
        min_dnia >= od || min_dnia < do_
    };
    !w_przerwie
}

const LIVE_NET_COST_HOLD: &str = "Koszty netto są obecnie trybem badawczym: brak certyfikowanej migracji i trwałego ACK. LIVE nie włącza nowego księgowania. Nowe wejścia HOLD; działająca sesja zachowuje poprzednie księgowanie oraz close/SL/TP/cancel. Nie zeruję koszyków ani DD. Przy starcie ON nie łączę MT5; ochronę istniejących pozycji sprawdź w terminalu.";
const LIVE_SR_V2_HOLD: &str = "S/R V2 z dokładnych ticków jest badawcze: adapter historii LIVE nie jest podłączony. Żądana konfiguracja nie zostaje uruchomiona; nowe wejścia HOLD. Działająca noga zachowuje poprzednią konfigurację i ochronne close/SL/TP/cancel. Przy starcie ON nie łączę MT5; ochronę istniejących pozycji sprawdź w terminalu.";

fn live_sr_v2_requested(c: &conduit_core::Settings) -> bool {
    c.sr_warmup_exact_ticks && c.trail_sr_enabled && (c.trail_sr_min_prominence_atr > 0.0
        || c.trail_sr_offset_atr_mult > 0.0 || c.trail_sr_offset_spread_mult > 0.0)
}

fn note_live_sr_hold(st: &StateHandle) {
    st.update(Sections::one(Section::Halt), |s| {
        let mut reasons=s.halt.diagnoza.clone();
        if !reasons.contains(LIVE_SR_V2_HOLD) {
            if !reasons.is_empty() { reasons.push_str(ui::HALT_SEP); }
            reasons.push_str(LIVE_SR_V2_HOLD);
            s.halt.ustaw(ui::KlasaHaltu::Diagnoza, &reasons);
        }
    });
    st.log("mt5", "error", "S/R V2 LIVE — konfiguracja odrzucona, nowe wejścia HOLD", LIVE_SR_V2_HOLD);
}

fn live_sr_start_allowed(st: &StateHandle) -> bool {
    let (core,balance)=st.read(|s| (live_core_from_ui(&s.settings),s.stats.balance));
    // Use actual chain/file ownership, not an unrelated global strategy checkbox.
    let team=zbuduj_silniki(st,&core,balance);
    if !team.lista.iter().any(|s| live_sr_v2_requested(&s.engine.cfg)) { return true; }
    note_live_sr_hold(st); false
}

fn reject_live_sr_transition(st: &StateHandle, next: conduit_core::Settings,
    old: &conduit_core::Settings) -> conduit_core::Settings {
    if live_sr_v2_requested(&next) { note_live_sr_hold(st); old.clone() } else { next }
}

fn live_net_cost_requested(doc: &Value) -> bool {
    doc.get("closed_profit_net_costs").and_then(Value::as_bool).unwrap_or(false)
}

/// LIVE is deliberately still legacy until a durable ledger migration exists.
/// The requested JSON value remains visible; this is not a silent config edit.
fn live_core_from_ui(doc: &Value) -> conduit_core::Settings {
    let mut c = conduit_server::settings_map::core_from_ui(doc);
    c.closed_profit_net_costs = false;
    c
}

fn live_cost_start_allowed(st: &StateHandle) -> bool {
    if !st.read(|s| live_net_cost_requested(&s.settings)) { return true; }
    st.update(Sections::one(Section::Halt), |s| s.halt.ustaw(ui::KlasaHaltu::Diagnoza, LIVE_NET_COST_HOLD));
    st.log("mt5", "error", "LIVE koszty netto — HOLD przed podłączeniem", LIVE_NET_COST_HOLD);
    false
}

fn przeladuj_ustawienia(
    st: &StateHandle,
    silniki: &mut routing::Silniki,
    core: &mut conduit_core::Settings,
    stops_level: f64,
    mtime_presetow: &mut std::collections::HashMap<String, std::time::SystemTime>,
) {
    let mut nowe = st.read(|s| {
        let mut c = live_core_from_ui(&s.settings);
        conduit_server::settings_map::apply_lot(&mut c, &s.lot);
        c
    });
    // `stops_level` wyrównany PRZED porównaniem: dokument niesie wartość
    // wpisaną ręcznie, a `core` trzyma odczyt z serwera. Porównanie tych
    // dwóch pól znaczyło, że pierwsza zmiana stops_level u brokera uzbraja
    // „Ustawienia przeładowane w locie" co 2 s do końca procesu — cała
    // konfiguracja silników podmieniana 43 200 razy na dobę bez powodu.
    nowe.stops_level = stops_level;
    if nowe != *core {
        *core = nowe;
        // DOKUMENT PANELU OPISUJE RACHUNEK, a nie handel: dźwignia, koszty,
        // opóźnienie, dziennik. Cały dokument podmienia konfigurację tylko
        // tam, gdzie dokument JEST konfiguracją — czyli na starej ścieżce
        // ręcznej (silnik bez pliku presetu).
        //
        // Silnik ZAMROŻONY jest tu drugim wyjątkiem i to nie jest szczegół:
        // jego pola handlu to MIGAWKA konfiguracji, którą otwarto jego wciąż
        // żywe koszyki. Podmiana ich dokumentem znaczyłaby, że pozycje
        // dogrywa ustawienie, którego nikt dla nich nie wybrał — a zamrożenie
        // istnieje dokładnie po to, żeby tak się nie stało. Rachunek (koszty,
        // opóźnienie, dziennik) i owszem, ma być świeży.
        let mut sr_transition_rejected=false;
        for s in silniki.lista.iter_mut() {
            let proposed = if s.z_pliku || s.tylko_zarzadzanie {
                conduit_core::wielosilnik::ustawienia_formatu(&s.engine.cfg, core)
            } else {
                core.clone()
            };
            sr_transition_rejected |= live_sr_v2_requested(&proposed);
            s.engine.cfg = reject_live_sr_transition(st, proposed, &s.engine.cfg);
            s.engine.continuation_configuration_changed();
        }
        st.log(
            "settings",
            if sr_transition_rejected { "warn" } else { "info" },
            if sr_transition_rejected { "Zmiana ustawień S/R V2 odrzucona — dotychczasowa ochrona pozostaje" }
                else { "Ustawienia przeładowane w locie" },
            String::new(),
        );
    }
    // ---------- EDYCJA PRESETU NA DYSKU → SILNIK FORMATU ----------
    //
    // Panel edytuje pola handlu per preset (plik), a silnik czyta preset tylko
    // przy budowie — bez tego bloku edycja czekałaby na restart. Stan silnika
    // (koszyki, statystyki, sloty) zostaje nietknięty. Silnik ze starej
    // ścieżki ręcznej odpada na `z_pliku == false`: jego konfiguracją jest
    // dokument i podmiana plikiem byłaby cichą zmianą strategii.
    for s in silniki.lista.iter_mut() {
        if !s.z_pliku {
            continue;
        }
        let Some(mt) = mtime_presetu(st, &s.preset) else {
            continue;
        };
        let znany = mtime_presetow.get(&s.preset).copied();
        if znany.is_none() {
            // pierwszy odczyt = stan zastany, nie zmiana
            mtime_presetow.insert(s.preset.clone(), mt);
            continue;
        }
        if znany == Some(mt) {
            continue;
        }
        mtime_presetow.insert(s.preset.clone(), mt);
        let Some(p) = st
            .workspace
            .load_presets()
            .into_iter()
            .find(|p| p.name.eq_ignore_ascii_case(&s.preset))
        else {
            continue;
        };
        let proposed=conduit_core::wielosilnik::ustawienia_formatu(&p.settings, core);
        let sr_transition_rejected=live_sr_v2_requested(&proposed);
        s.engine.cfg = reject_live_sr_transition(st, proposed, &s.engine.cfg);
        s.engine.continuation_configuration_changed();
        st.log(
            "settings",
            if sr_transition_rejected { "warn" } else { "info" },
            if sr_transition_rejected { format!("Preset {} NIE został przeładowany: S/R V2 LIVE HOLD",s.preset) }
                else { format!("Preset {} przeładowany w locie", s.preset) },
            if sr_transition_rejected { LIVE_SR_V2_HOLD.to_string() } else { format!(
                "Plik presetu zmienił się na dysku (edycja per preset w panelu). \
                 Silnik formatu {} gra od teraz nowymi ustawieniami; koszyki \
                 i statystyki zostały nietknięte.",
                s.format
            ) },
        );
    }
}

fn loty_nog(
    silniki: &routing::Silniki,
    szczeble: &[WierszSzczebla],
    prog_biezacy: f64,
    saldo: f64,
    zamkniete: &[ui::ClosedPosition],
) -> Vec<ui::LotNogi> {
    let mut out: Vec<ui::LotNogi> = silniki
        .lista
        .iter()
        .map(|sl| {
            let podstawa = sl.engine.podstawa_lota();
            let wolumen: f64 = zamkniete
                .iter()
                .filter(|z| {
                    // Transakcja bez koszyka (obca/ręczna) nie jest wolumenem
                    // ŻADNEJ nogi: `unwrap_or(0)` wliczał ją nodze slotu 0,
                    // więc kafel „LOT AUTO" pokazywał cudzy obrót jako własny.
                    z.basket_id
                        .and_then(|id| silniki.indeks_koszyka(id))
                        .map(|i| silniki.lista[i].format == sl.format)
                        .unwrap_or(false)
                })
                .map(|z| z.volume)
                .sum();
            // Silnik bez formatu nie ma jak dostać wiadomości
            // (`routing::trasa` → `KanalBezFormatu`/`FormatNieHandluje`),
            // więc jego lot nie jest lotem, którym cokolwiek zagra.
            let handluje = !sl.tylko_zarzadzanie && !sl.format.is_empty() && sl.powod.is_empty();
            let powod = if handluje {
                String::new()
            } else if !sl.powod.is_empty() {
                sl.powod.clone()
            } else if sl.tylko_zarzadzanie {
                "zamrozona".to_string()
            } else {
                "brakFormatu".to_string()
            };
            ui::LotNogi {
                format: sl.format.clone(),
                preset: sl.preset.clone(),
                lot: sl.engine.lot_size(podstawa),
                wolumen_wykonany: (wolumen * 100.0).round() / 100.0,
                zamrozona: sl.tylko_zarzadzanie,
                handluje,
                z_pliku: sl.z_pliku,
                lot_max: sl.engine.cfg.lot_max,
                lot_koszyka: sl.engine.lot_koszyka_planowany(podstawa),
                poziomy_wejscia: sl.engine.poziomy_wejscia_planowane(),
                pulap_lancucha: silniki.pulapy.max_lotow,
                stan: if handluje {
                    "aktywna".into()
                } else {
                    "nieaktywna".into()
                },
                powod,
                lancuch: silniki.lancuch.clone(),
                prog: prog_biezacy,
            }
        })
        .collect();

    // ---------- SZCZEBLE, NA KTÓRYCH KONTO JESZCZE (ALBO JUŻ) NIE STOI ----------
    //
    // Użytkownik ma prawo wyedytować lot presetu, zanim konto na niego
    // urośnie — inaczej pierwsze przełączenie drabinki wchodzi w życie
    // z ustawieniami, których nikt nie oglądał. Te wiersze NIE MAJĄ silnika
    // na rachunku: to opis szczebla, nie noga, która cokolwiek prowadzi.
    for w in szczeble {
        // Noga aktywnego łańcucha bez obserwowanego źródła niesie własny
        // powód i NIE jest szczeblem drabinki — nie wolno jej podpisać
        // „czeka na próg", bo ona nie czeka na nic, tylko nie ma czym grać.
        let wlasny = !w.powod.is_empty();
        let minieta = !wlasny && w.prog < prog_biezacy;
        out.push(ui::LotNogi {
            format: w.format.clone(),
            preset: w.preset.clone(),
            lot: w.engine.lot_size(w.engine.podstawa_lota()),
            wolumen_wykonany: 0.0,
            zamrozona: false,
            handluje: false,
            z_pliku: w.z_pliku,
            lot_max: w.engine.cfg.lot_max,
            lot_koszyka: w.engine.lot_koszyka_planowany(w.engine.podstawa_lota()),
            poziomy_wejscia: w.engine.poziomy_wejscia_planowane(),
            pulap_lancucha: 0.0,
            stan: if wlasny || minieta {
                "nieaktywna".into()
            } else {
                "kolejka".into()
            },
            powod: if wlasny {
                w.powod.to_string()
            } else if minieta {
                "minieta".into()
            } else {
                "kolejka".into()
            },
            lancuch: w.lancuch.clone(),
            prog: w.prog,
        });
    }
    out
}

/// Pary (format, preset) nóg, które już mają swój silnik.
fn pary_nog(s: &routing::Silniki) -> Vec<(String, String)> {
    s.lista
        .iter()
        .map(|x| (x.format.clone(), x.preset.clone()))
        .collect()
}

/// Jeden wiersz opisujący nogę SZCZEBLA, na którym konto nie stoi.
///
/// Trzyma własny `Engine` wyłącznie po to, żeby policzyć lot tym samym
/// kodem co noga prawdziwa (`lot_size`) — drugi wzór na lot to druga
/// prawda i pierwsza okazja, żeby panel pokazał co innego, niż bot zagra.
pub(crate) struct WierszSzczebla {
    pub prog: f64,
    pub lancuch: String,
    pub format: String,
    pub preset: String,
    pub z_pliku: bool,
    /// Kod powodu, dla którego ta noga nie handluje. Wiersz niesie go sam,
    /// bo powstaje z dwóch różnych źródeł: nogi AKTYWNEGO łańcucha bez
    /// obserwowanego kanału (`brakZrodla`) i szczeble drabinki, na których
    /// konto nie stoi (`kolejka` / `minieta`).
    pub powod: &'static str,
    pub engine: Engine,
}

/// Buduje opis szczebli drabinki INNYCH niż aktywny.
///
/// Woła `load_presets()` (odczyt katalogu z dysku), więc jest wywoływana
/// przy PRZEBUDOWIE łańcucha i przy zmianie plików presetów — nigdy
/// w pętli publikacji, która chodzi 4× na sekundę.
fn zbuduj_szczeble(
    st: &StateHandle,
    core: &conduit_core::Settings,
    saldo: f64,
    aktywny_lancuch: &str,
    aktywne_nogi: &[(String, String)],
) -> Vec<WierszSzczebla> {
    let (drabinka, lancuchy) = st.read(|s| (s.drabinka.clone(), s.lancuchy.clone()));
    let presety: std::collections::BTreeMap<String, conduit_core::Settings> = st
        .workspace
        .load_presets()
        .into_iter()
        .map(|p| (p.name, p.settings))
        .collect();
    let mut out = Vec::new();
    // Nogi, które JUŻ opisują prawdziwe silniki aktywnego łańcucha. Ten sam
    // preset na dwóch szczeblach (FQ-3 stoi i w ZENONLY3, i w SENTINEL-0C)
    // dałby dwa kafelki edytujące jeden plik — a jeden z nich kłamałby,
    // że preset nie gra.
    let mut widziane: Vec<(String, String)> = aktywne_nogi.to_vec();

    if let Some(l) = lancuchy.aktywny() {
        for (format, preset) in &l.presety {
            if preset.is_empty() {
                continue;
            }
            let klucz = (format.clone(), preset.clone());
            if widziane.contains(&klucz) {
                continue;
            }
            widziane.push(klucz);
            let cfg = presety
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(preset))
                .map(|(_, s)| conduit_core::wielosilnik::ustawienia_formatu(s, core));
            let z_pliku = cfg.is_some();
            out.push(WierszSzczebla {
                prog: -1.0,
                lancuch: l.nazwa.clone(),
                format: format.clone(),
                preset: preset.clone(),
                z_pliku,
                powod: "brakZrodla",
                engine: Engine::new(cfg.unwrap_or_else(|| core.clone()), saldo),
            });
        }
    }

    // ---------- SZCZEBLE DRABINKI ----------
    //
    // DRABINKA WYŁĄCZONA = NIE MA KOLEJKI. Bez tego panel obiecywał
    // „w kolejce" przy szczeblach, które nigdy nie wejdą, bo automatyczna
    // zmiana łańcucha jest wyłączona.
    if !drabinka.enabled {
        return out;
    }
    for sz in &drabinka.szczeble {
        if sz.lancuch == aktywny_lancuch {
            continue; // ten szczebel opisują prawdziwe silniki
        }
        let Some(l) = lancuchy.lista.iter().find(|l| l.nazwa == sz.lancuch) else {
            continue;
        };
        for (format, preset) in &l.presety {
            if preset.is_empty() {
                continue;
            }
            // Ten sam preset na dwóch szczeblach opisujemy RAZ — inaczej
            // panel pokazałby dwa kafelki, które edytują ten sam plik.
            let klucz = (format.clone(), preset.clone());
            if widziane.contains(&klucz) {
                continue;
            }
            widziane.push(klucz);
            let cfg = presety
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(preset))
                .map(|(_, s)| conduit_core::wielosilnik::ustawienia_formatu(s, core));
            let z_pliku = cfg.is_some();
            out.push(WierszSzczebla {
                prog: sz.prog_balance,
                lancuch: sz.lancuch.clone(),
                format: format.clone(),
                preset: preset.clone(),
                z_pliku,
                powod: "",
                engine: Engine::new(cfg.unwrap_or_else(|| core.clone()), saldo),
            });
        }
    }
    out
}

fn mtime_presetu(st: &StateHandle, nazwa: &str) -> Option<std::time::SystemTime> {
    if nazwa.is_empty() {
        return None;
    }
    // Ta sama normalizacja nazwy co przy zapisie (`sanitize_file_name` jest
    // prywatne w store) — nazwy presetów w łańcuchach są proste (A-Z, cyfry,
    // myślnik), więc plik nazywa się dokładnie `<nazwa>.json`.
    let sciezka = st.workspace.presets_dir().join(format!("{nazwa}.json"));
    std::fs::metadata(sciezka).and_then(|m| m.modified()).ok()
}

pub(crate) fn przerwa_dobowa(st: &StateHandle) -> (f64, f64) {
    st.read(|s| {
        let f = |k: &str, d: f64| {
            s.settings
                .get(k)
                .and_then(|v| v.as_f64())
                .filter(|x| x.is_finite())
                .unwrap_or(d)
        };
        (
            f("przerwa_dobowa_od_h", 0.0),
            f("przerwa_dobowa_do_h", 1.083),
        )
    })
}

#[cfg(test)]
mod testy_rynku {
    /// PRAWDZIWA funkcja (`rynek_czynny_o`), nie duplikat arytmetyki —
    /// duplikat przeżył jedną zmianę logiki (przerwa dobowa) i od razu
    /// przestał opisywać kod.
    fn czynny(sek_utc: i64) -> bool {
        super::rynek_czynny_o(sek_utc + 3 * 3600, (0.0, 1.083))
    }

    #[test]
    fn weekend_nie_jest_awaria() {
        // Znaczniki policzone, nie zgadnięte (12:00 UTC danego dnia).
        assert!(!czynny(1_785_585_600), "sobota 2026-08-01");
        assert!(!czynny(1_785_672_000), "niedziela 2026-08-02");
        assert!(czynny(1_785_412_800), "czwartek 2026-07-30");
        assert!(
            czynny(1_785_758_400),
            "poniedziałek 2026-08-03 w środku dnia"
        );
    }

    #[test]
    fn granice_tygodnia_licza_sie_w_czasie_SERWERA() {
        // Piątek 23:30 UTC to u brokera (UTC+3) już SOBOTA 02:30 — rynek stoi.
        // Gdyby liczyć w UTC, bot próbowałby odbudowywać most przez pół nocy.
        assert!(
            !czynny(1_785_540_600),
            "piątek 23:30 UTC = sobota u brokera"
        );
        // Poniedziałek 00:30 UTC to u brokera 03:30 — notowania już idą.
        assert!(
            czynny(1_785_717_000),
            "poniedziałek 00:30 UTC = 03:30 u brokera"
        );
    }

    #[test]
    fn przerwa_dobowa_nie_jest_awaria() {
        use super::rynek_czynny_o;
        let wtorek_0030 = 20_669_i64 * 86_400 + 30 * 60;
        assert!(
            !rynek_czynny_o(wtorek_0030, (0.0, 1.083)),
            "00:30 to przerwa dobowa"
        );
        // 01:04 — nadal przerwa (zapas do 01:05, bo notowania wracają nierówno).
        assert!(
            !rynek_czynny_o(wtorek_0030 + 34 * 60, (0.0, 1.083)),
            "01:04 jeszcze przerwa"
        );
        // 01:06 — rynek ma już działać; dalsza cisza JEST podejrzana.
        assert!(
            rynek_czynny_o(wtorek_0030 + 36 * 60, (0.0, 1.083)),
            "01:06 po przerwie"
        );
        // Puste okno (od == do) = przerwa wyłączona: 00:30 uchodzi za czynne.
        assert!(
            rynek_czynny_o(wtorek_0030, (0.0, 0.0)),
            "od==do wyłącza przerwę"
        );
        // Okno przez północ (23,5 → 0,5) też działa — brokerzy różnie kładą przerwę.
        assert!(
            !rynek_czynny_o(wtorek_0030 - 3600, (23.5, 0.5)),
            "23:30 w oknie 23,5→0,5"
        );
    }
}

/// Czy ta wiadomość OTWIERA nowy koszyk.
///
/// Bramka wieku wolno dotknąć **wyłącznie** takich wiadomości. Komunikat
/// zarządzający (`TP HIT`, `RISK FREE`, `CLOSE ALL`, `SL`, korekta celu) dotyczy
/// koszyka, który JUŻ ŻYJE na rachunku, więc odrzucenie go z powodu wieku byłoby
/// gorsze od wykonania: pozycja zostałaby bez opieki, choć autor kazał ją zamknąć.
/// Z tego samego powodu przepuszczamy wszystkie EDYCJE — edycja starej wiadomości
/// jest z natury „stara", a niesie poprawkę do czegoś, co już stoi.
fn otwiera_koszyk(text: &str, edit_of: Option<i64>) -> bool {
    conduit_core::telegram_ingress::opens_basket(text, edit_of)
}

/// Czy ta wiadomość jest NIECZYTELNYM SYGNAŁEM — kształt bloku wejścia
/// (cel + stop), a parser nie wyciąga z niej ani wejścia, ani polecenia?
///
/// Wydzielone z `ostrzez_o_nieczytelnym_sygnale`, żeby dało się to sprawdzić
/// testem bez stanu aplikacji, bez Telegrama i bez mostu do MT5.
fn nieczytelny_sygnal(text: &str) -> bool {
    use conduit_core::parser::Signal;
    if !conduit_core::parser::wyglada_na_wejscie(text) {
        return false;
    }
    // `Info` to jedyny wynik, który znaczy „nic z tego nie zrozumiałem".
    // Wystarczy JEDNO polecenie (choćby `TpHit`), żeby wiadomość była
    // czytelna — kanał melduje trafienie celu i cytuje przy tym poziomy.
    conduit_core::parser::parse(text)
        .iter()
        .all(|s| matches!(s, Signal::Info))
}

fn ostrzez_o_nieczytelnym_sygnale(st: &StateHandle, im: &IncomingMessage) {
    if im.edit_of.is_some() || !nieczytelny_sygnal(&im.text) {
        return;
    }
    let temat = match im.source.topic_id {
        Some(t) => format!(" · temat {t}"),
        None => String::new(),
    };
    let format = format_zrodla(st, &im.source).filter(|f| !f.is_empty());
    let tresc = format!(
        "Wiadomość z „{}\"{temat} ma kształt sygnału wejścia — jest w niej cel \
         i stop — ale parser nie rozpoznał w niej ANI wejścia, ANI polecenia \
         zarządzającego. Bot jej NIE wykonał.\n\n\
         Najczęstsza przyczyna: kanał zmienił sposób zapisywania sygnałów. \
         Jeżeli takich wiadomości jest więcej, bot przestał handlować i trzeba \
         dopisać wzorzec do parsera — sam z siebie się to nie naprawi.\n\n\
         format źródła: {}\n\nTREŚĆ:\n{}",
        im.source_name,
        format
            .as_deref()
            .unwrap_or("(brak — źródło tylko nasłuchiwane)"),
        im.text
    );
    // Poziom zależy od tego, czy źródło HANDLUJE. Kanał podpięty „na próbę"
    // (bez formatu) z definicji sypie nieznanym zapisem — to nie jest awaria,
    // tylko powód, dla którego się go najpierw obserwuje.
    match format {
        Some(_) => {
            st.log(
                "telegram",
                "error",
                "SYGNAŁ NIECZYTELNY — bot go nie wykonał",
                tresc.clone(),
            );
            st.notify(
                MailCategory::SignalUnreadable,
                &format!("Nieczytelny sygnał z „{}\"", im.source_name),
                &tresc,
            );
        }
        None => st.log(
            "telegram",
            "warn",
            "Nierozpoznany zapis sygnału (źródło bez formatu)",
            tresc,
        ),
    }
}

fn wiek_ponad_prog(teraz_ms: i64, wyslano_ms: i64, prog_min: f64) -> Option<f64> {
    conduit_core::telegram_ingress::stale_entry_age_minutes(teraz_ms, wyslano_ms, prog_min)
}

/// Próg wieku sygnału z dokumentu panelu. Brak klucza = [`SYGNAL_MAX_WIEK_MIN`].
fn prog_wieku_sygnalu(st: &StateHandle) -> f64 {
    st.read(|s| {
        s.settings
            .get("signal_max_age_min")
            .and_then(|v| v.as_f64())
    })
    .filter(|v| v.is_finite() && *v >= 0.0)
    .unwrap_or(SYGNAL_MAX_WIEK_MIN)
}

/// Co ile zapisujemy zrzut koszyków, gdy się zmieniły. Zapis jest atomowy
/// i waży kilka kilobajtów, ale nie ma po co robić go 4 razy na sekundę.
const ZRZUT_CO: Duration = Duration::from_secs(5);

/// Timestamp retry nie wymusza zapisu co sekundę; powstanie/zmiana/usunięcie
/// obowiązku wyjścia zapisuje się na końcu bieżącej iteracji pętli.
fn sygnatura_pending_exit<'a>(
    intents: impl IntoIterator<Item = (u32, &'a Option<conduit_core::PendingBasketExit>)>,
) -> Vec<(u32, conduit_core::CloseReason)> {
    let mut signature: Vec<_> = intents.into_iter()
        .filter_map(|(id, intent)| intent.as_ref().map(|exit| (id, exit.reason)))
        .collect();
    signature.sort_by_key(|(id, _)| *id);
    signature
}

const CISZA_TELEGRAM_MIN: f64 = 320.0;

const PINGOW_DO_ALARMU: u32 = 2;

/// Co ile sprawdzamy zdrowie Telegrama w pętli handlowej.
const TELEGRAM_SPRAWDZAJ_CO: Duration = Duration::from_secs(60);

/// Od ilu procent obsunięcia mail, GDY STRAŻNIK RYZYKA JEST WYŁĄCZONY.
///
/// Nie jest to próg handlowy — nic nie zatrzymuje. To jest wyłącznie moment,
/// w którym warto obudzić człowieka. 15 % konta to strata, o której właściciel
/// chce wiedzieć w nocy, a jednocześnie na tyle dużo, że nie wywoła jej zwykły
/// oddech rynku przy otwartej siatce.
const ALARM_DD_PCT_DOMYSLNY: f64 = 15.0;

/// O ile punktów procentowych musi się pogłębić obsunięcie, żeby poszedł
/// kolejny mail. Bez hamulca obsunięcie idzie dziesiątkami procent, więc krok
/// 0,25 pp z gałęzi ze strażnikiem zamieniłby alarm w zalew.
const ALARM_DD_KROK_PP: f64 = 5.0;

// ============================================================
//  POLECENIA
// ============================================================

/// Wszystko, co wchodzi do pętli silnika z zewnątrz.
enum LiveCmd {
    /// Ręczna intencja należy do sesji widocznej w chwili jej podjęcia.
    Scoped { account_session: String, command: Box<LiveCmd> },
    /// komenda handlowa z panelu
    Panel(Command),
    /// wiadomość z kanału — podlega bramce trybu MANUAL/AUTO
    Kanal(IncomingMessage, i64),
    Reczny(IncomingMessage),
    /// „Wykonaj" przy wiadomości czekającej w trybie MANUAL
    Wykonaj(String),
    /// „Odrzuć" przy wiadomości czekającej w trybie MANUAL
    Odrzuc(String),
}

/// Uchwyt wpięty w `StateHandle::set_runtime`.
///
/// Sam nie handluje — tylko przenosi polecenia z wątku HTTP do pętli silnika.
pub struct LiveRuntime {
    tx: Sender<LiveCmd>,
    /// nazwa źródła używana dla sygnałów wpisanych ręcznie w panelu
    connected: Arc<AtomicBool>,
}

impl LiveRuntime {
    fn send(&self, c: LiveCmd) -> anyhow::Result<()> {
        self.tx
            .send(c)
            .map_err(|_| anyhow::anyhow!("pętla handlowa nie działa — komenda odrzucona"))
    }
}

impl conduit_server::Runtime for LiveRuntime {
    fn command(&self, cmd: &Command, state: &StateHandle) -> anyhow::Result<()> {
        anyhow::ensure!(!state.read(|s| s.settings.get("mt5_follow_terminal_account").and_then(Value::as_bool).unwrap_or(false)),
            "Ręczne polecenie wymaga identyfikatora aktualnej sesji rachunku. Odśwież widok i podejmij decyzję ponownie.");
        // Sygnał wpisany ręcznie w panelu idzie tą samą drogą, co wiadomość
        // z Telegrama — inaczej test ręczny sprawdzałby inną ścieżkę kodu niż
        // ta, która handluje naprawdę.
        match cmd {
            Command::SimulateMessage { .. } => {
                // Budowa wiadomości mieszka w `conduit_server::wstrzykniecie`
                // — jednym miejscu, wspólnym dla żywej ścieżki, trybu demo
                // i testów. Tam też opisana jest reguła numeracji (zakres
                // ujemny zarezerwowany dla panelu).
                let im = conduit_server::wstrzykniecie::wiadomosc(
                    cmd,
                    conduit_server::now_ms(),
                    conduit_server::wstrzykniecie::ZRODLO_PANELU,
                )
                .ok_or_else(|| anyhow::anyhow!("polecenie wstrzyknięcia bez treści"))?;
                return self.send(LiveCmd::Reczny(im));
            }
            Command::ExecuteMessage { id } => return self.send(LiveCmd::Wykonaj(id.clone())),
            Command::DismissMessage { id } => return self.send(LiveCmd::Odrzuc(id.clone())),
            _ => {}
        }

        if !self.connected.load(Ordering::Relaxed) {
            anyhow::bail!(
                "brak połączenia z MetaTrader 5 — komenda handlowa odrzucona. \
                 Sprawdź, czy terminal działa i czy sidecar Pythona wstał \
                 (Ustawienia → MetaTrader 5)."
            );
        }
        self.send(LiveCmd::Panel(cmd.clone()))
    }

    fn command_scoped(&self, cmd: &Command, state: &StateHandle, expected_account_session: Option<&str>) -> anyhow::Result<()> {
        let follow = state.read(|s| s.settings.get("mt5_follow_terminal_account").and_then(Value::as_bool).unwrap_or(false));
        if !follow {
            anyhow::ensure!(expected_account_session.unwrap_or("").is_empty(),
                "Polecenie pochodzi ze starej sesji FOLLOW. Odśwież widok po zmianie trybu rachunku.");
            return self.command(cmd, state);
        }
        let valid = state.read(|s| {
            s.connection.mt5 == "connected"
                && s.connection.account_verified == "ok"
                && !s.connection.account_session.is_empty()
                && expected_account_session == Some(s.connection.account_session.as_str())
        });
        anyhow::ensure!(self.connected.load(Ordering::Acquire) && valid,
            "Ręczne polecenie nie należy do aktualnej, potwierdzonej sesji rachunku. Odśwież widok i podejmij decyzję ponownie.");
        let command = match cmd {
            Command::SimulateMessage { .. } => LiveCmd::Reczny(
                conduit_server::wstrzykniecie::wiadomosc(cmd, conduit_server::now_ms(), conduit_server::wstrzykniecie::ZRODLO_PANELU)
                    .ok_or_else(|| anyhow::anyhow!("polecenie wstrzyknięcia bez treści"))?),
            Command::ExecuteMessage { id } => LiveCmd::Wykonaj(id.clone()),
            Command::DismissMessage { id } => LiveCmd::Odrzuc(id.clone()),
            _ => LiveCmd::Panel(cmd.clone()),
        };
        self.send(LiveCmd::Scoped { account_session: expected_account_session.unwrap().to_string(), command: Box::new(command) })
    }

    fn name(&self) -> &'static str {
        "MT5 (na żywo)"
    }
}

/// Druga bramka, tuż przed wykonaniem: komenda mogła czekać podczas zmiany sesji.
/// Wiadomości kanału są osobnym, automatycznym źródłem; nie są kliknięciem UI.
fn command_for_live_session(command: LiveCmd, follow: bool, session: &str) -> Option<LiveCmd> {
    match command {
        LiveCmd::Scoped { account_session, command } => {
            // An already-scoped intent never becomes unscoped if FOLLOW is disabled
            // while it is queued. Legacy commands have no envelope at all.
            if session.is_empty() || account_session != session { return None; }
            match *command {
                LiveCmd::Scoped { .. } | LiveCmd::Kanal(_, _) => None,
                command => Some(command),
            }
        }
        LiveCmd::Kanal(message, received_utc) => Some(LiveCmd::Kanal(message, received_utc)),
        command if !follow => Some(command),
        _ => None,
    }
}

/// Uchwyt do zatrzymania pętli przy zamykaniu programu.
pub struct Handle {
    stop: Arc<AtomicBool>,
}

impl Handle {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

// ============================================================
//  BROKER Z PAMIĘCIĄ — opakowanie mostu MT5
// ============================================================

/// Opakowanie `Mt5Bridge`, które robi dwie rzeczy, których most robić nie musi.
///
/// 1. **Pamięta zamknięte transakcje.** Silnik konsumuje `drain_closed()`
///    (księguje statystyki), więc gdyby nikt ich po drodze nie przechwycił,
///    panel nigdy nie pokazałby historii — most, inaczej niż `SimBroker`,
///    nie prowadzi własnego `history`.
/// 2. **Liczy odmowy brokera.** Każdy błąd zlecenia jest tu przechwytywany
///    z nazwą operacji, żeby kategoria maila `OrderError` miała skąd wziąć
///    treść. Bez tego odmowa jest widoczna dopiero w logu procesu.
struct Recording {
    inner: Mt5Bridge,
    history: Vec<ClosedTrade>,
    /// odmowy od ostatniego odczytu
    errors: Vec<Odmowa>,
    /// łączna liczba odmów od startu — do panelu
    error_count: u64,
}

/// ODMOWA BROKERA Z KONTEKSTEM.
///
/// Sama para „operacja + rodzaj błędu" nie wystarcza, żeby po tygodniu
/// odpowiedzieć na pytanie „dlaczego bota nie było w tym ruchu". Trzeba
/// wiedzieć, CZEGO odmówiono: którego koszyka, którego poziomu siatki, po
/// jakiej cenie i z jakimi stopami. Bez tego wpis w dzienniku mówi tylko,
/// że coś się nie udało — a to jest dokładnie ta klasa zapisu, przez którą
/// „rozstawiono 0 zleceń" zajęło nam wieczór.
#[derive(Debug, Clone)]
struct Odmowa {
    op: &'static str,
    err: BrokerError,
    ticket: Option<Ticket>,
    basket: Option<u32>,
    level: Option<i32>,
    volume: Option<f64>,
    price: Option<f64>,
    sl: Option<f64>,
    tp: Option<f64>,
}

impl Odmowa {
    fn nowa(op: &'static str, err: BrokerError) -> Self {
        Odmowa {
            op,
            err,
            ticket: None,
            basket: None,
            level: None,
            volume: None,
            price: None,
            sl: None,
            tp: None,
        }
    }
}

impl Recording {
    fn new(inner: Mt5Bridge) -> Self {
        Recording {
            inner,
            history: Vec::new(),
            errors: Vec::new(),
            error_count: 0,
        }
    }

    /// Zapisuje odmowę razem z kontekstem żądania.
    fn note_ctx<T>(&mut self, op: &'static str, ctx: Odmowa, r: BResult<T>) -> BResult<T> {
        if let Err(e) = &r {
            let mut o = ctx;
            o.op = op;
            o.err = *e;
            self.errors.push(o);
            self.error_count += 1;
        }
        r
    }

    fn note<T>(&mut self, op: &'static str, r: BResult<T>) -> BResult<T> {
        let ctx = Odmowa::nowa(op, BrokerError::Rejected);
        self.note_ctx(op, ctx, r)
    }

    fn take_errors(&mut self) -> Vec<Odmowa> {
        std::mem::take(&mut self.errors)
    }
}

type BResult<T> = Result<T, BrokerError>;

impl Broker for Recording {
    fn quote(&self) -> Quote {
        self.inner.quote()
    }
    fn account(&self) -> Account {
        self.inner.account()
    }
    fn stops_level(&self) -> f64 {
        self.inner.stops_level()
    }
    fn volume_min(&self) -> f64 {
        self.inner.volume_min()
    }
    fn volume_step(&self) -> f64 {
        self.inner.volume_step()
    }
    fn volume_max(&self) -> f64 {
        self.inner.volume_max()
    }
    fn close_receipt_reconciliation_active(&self) -> bool {
        self.inner.close_receipt_reconciliation_active()
    }
    fn close_receipts_pending(&self) -> bool {
        self.inner.close_receipts_pending()
    }
    fn receipt_barrier(&self) -> conduit_core::broker::ReceiptBarrier { self.inner.receipt_barrier() }
    fn execution_session(&self) -> Option<conduit_core::broker::ExecutionSession> { self.inner.execution_session() }
    fn position_identifier(&self, ticket: Ticket) -> Option<u64> { self.inner.position_identifier(ticket) }
    fn pending_cancel_snapshot_authoritative(&self) -> bool { self.inner.pending_cancel_snapshot_authoritative() }
    fn cost_net_supported(&self) -> bool { self.inner.cost_net_supported() }
    fn report_cost_consumer_fault(&mut self, reason:&str) {self.inner.report_cost_consumer_fault(reason);}
    fn positions(&self) -> &[Position] {
        self.inner.positions()
    }
    fn pendings(&self) -> &[PendingOrder] {
        self.inner.pendings()
    }
    fn positions_mut(&mut self) -> &mut Vec<Position> {
        self.inner.positions_mut()
    }
    fn pendings_mut(&mut self) -> &mut Vec<PendingOrder> {
        self.inner.pendings_mut()
    }
    fn open_market(&mut self, r: OrderReq) -> BResult<Ticket> {
        let mut ctx = Odmowa::nowa("otwarcie rynkowe", BrokerError::Rejected);
        ctx.basket = r.basket;
        ctx.level = Some(r.level);
        ctx.volume = Some(r.volume);
        ctx.sl = r.sl;
        ctx.tp = r.tp;
        let x = self.inner.open_market(r);
        self.note_ctx("otwarcie rynkowe", ctx, x)
    }
    fn place_pending(&mut self, r: PendingReq) -> BResult<Ticket> {
        let mut ctx = Odmowa::nowa("zlecenie oczekujące", BrokerError::Rejected);
        ctx.basket = r.basket;
        ctx.level = Some(r.level);
        ctx.volume = Some(r.volume);
        ctx.price = Some(r.price);
        ctx.sl = r.sl;
        ctx.tp = r.tp;
        let x = self.inner.place_pending(r);
        self.note_ctx("zlecenie oczekujące", ctx, x)
    }
    fn modify_position(&mut self, t: Ticket, sl: Option<Px>, tp: Option<Px>) -> BResult<()> {
        let mut ctx = Odmowa::nowa("modyfikacja pozycji", BrokerError::Rejected);
        ctx.ticket = Some(t);
        ctx.sl = sl;
        ctx.tp = tp;
        let x = self.inner.modify_position(t, sl, tp);
        self.note_ctx("modyfikacja pozycji", ctx, x)
    }
    fn modify_pending(
        &mut self,
        t: Ticket,
        price: Px,
        sl: Option<Px>,
        tp: Option<Px>,
    ) -> BResult<()> {
        let mut ctx = Odmowa::nowa("modyfikacja zlecenia", BrokerError::Rejected);
        ctx.ticket = Some(t);
        ctx.price = Some(price);
        ctx.sl = sl;
        ctx.tp = tp;
        let x = self.inner.modify_pending(t, price, sl, tp);
        self.note_ctx("modyfikacja zlecenia", ctx, x)
    }
    fn close_position(&mut self, t: Ticket, reason: CloseReason) -> BResult<f64> {
        let mut ctx = Odmowa::nowa("zamknięcie pozycji", BrokerError::Rejected);
        ctx.ticket = Some(t);
        let x = self.inner.close_position(t, reason);
        self.note_ctx("zamknięcie pozycji", ctx, x)
    }
    fn close_partial(&mut self, t: Ticket, volume: f64, reason: CloseReason) -> BResult<f64> {
        let mut ctx = Odmowa::nowa("zamknięcie częściowe", BrokerError::Rejected);
        ctx.ticket = Some(t);
        ctx.volume = Some(volume);
        let x = self.inner.close_partial(t, volume, reason);
        self.note_ctx("zamknięcie częściowe", ctx, x)
    }
    fn cancel_pending(&mut self, t: Ticket) -> BResult<()> {
        let mut ctx = Odmowa::nowa("kasowanie zlecenia", BrokerError::Rejected);
        ctx.ticket = Some(t);
        let x = self.inner.cancel_pending(t);
        self.note_ctx("kasowanie zlecenia", ctx, x)
    }
    fn drain_closed(&mut self) -> Vec<ClosedTrade> {
        let v = self.inner.drain_closed();
        self.history.extend(v.iter().cloned());
        if self.history.len() > CLOSED_KEEP * 2 {
            let ile = self.history.len() - CLOSED_KEEP;
            self.history.drain(0..ile);
        }
        v
    }
}

// ============================================================
//  START
// ============================================================

/// Buduje konfigurację sidecara z ustawień panelu.
fn sidecar_config(st: &StateHandle) -> SidecarConfig {
    let doc = st.read(|s| s.settings.clone());
    let s = |k: &str| {
        doc.get(k)
            .and_then(|v| v.as_str())
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
    };
    let f = |k: &str, d: f64| doc.get(k).and_then(|v| v.as_f64()).unwrap_or(d);

    let mut cfg = SidecarConfig {
        follow_terminal_account: doc.get("mt5_follow_terminal_account").and_then(Value::as_bool).unwrap_or(false),
        allow_real_account: doc.get("mt5_allow_real_account").and_then(Value::as_bool).unwrap_or(false),
        close_receipt_reconcile: doc.get("close_receipt_reconcile").and_then(Value::as_bool).unwrap_or(false),
        closed_profit_net_costs: doc.get("closed_profit_net_costs").and_then(Value::as_bool).unwrap_or(false),
        symbol: s("mt5_symbol").unwrap_or_else(|| "XAUUSD".to_string()),
        magic: f("mt5_magic", 770_077.0) as i64,
        deviation_points: f("mt5_deviation_points", 30.0).max(0.0) as u32,
        terminal_path: s("mt5_terminal_path").map(std::path::PathBuf::from),
        login: doc
            .get("mt5_login")
            .and_then(|v| v.as_i64())
            .filter(|x| *x != 0),
        server: s("mt5_server"),
        ..Default::default()
    };
    // Hasło rachunku — z `secrets.json`, nigdy z dokumentu ustawień.
    // Transport poda je sidecarowi zmienną środowiskową (nie argumentem,
    // bo argumenty widać w liście procesów). Puste = terminal loguje się
    // zapamiętanymi poświadczeniami.
    {
        let sek = st.workspace.load_secrets();
        if sek.mt5.password.is_set() && !cfg.follow_terminal_account {
            cfg.password = Some(sek.mt5.password.into_inner());
        }
    }
    if cfg.follow_terminal_account {
        cfg.login = None;
        cfg.server = None;
        cfg.password = None;
    }
    // Minimalny dystans SL/TP, którego spodziewa się SYMULATOR. Most porówna
    // to z wartością z serwera brokera i krzyknie przy rozjeździe. Dotąd nikt
    // tych dwóch liczb nie zestawiał: preset mówił 0,20, broker mówił 0,20
    // i wszystko grało — ale gdyby Vantage zmienił warunki (robi to przed
    // danymi makro), backtest i konto zaczęłyby opisywać inny handel, bez
    // jednego słowa ostrzeżenia. `0` znaczy „nie ustawiono", więc nie sprawdzaj.
    cfg.expected_stops_level_price = doc
        .get("sim_stops_level")
        .and_then(|v| v.as_f64())
        .filter(|x| x.is_finite() && *x > 0.0);

    // Interpreter Pythona: pole z panelu, potem `python` z PATH. Sidecar
    // wymaga pakietu `MetaTrader5`, więc to musi być TEN interpreter, w którym
    // pakiet jest zainstalowany — stąd możliwość wpisania pełnej ścieżki.
    if let Some(p) = s("mt5_python") {
        cfg.python = std::path::PathBuf::from(p);
    }
    cfg.script = sciezka_sidecara();
    cfg
}

/// Znacznik doklejany na POCZĄTKU komentarza każdego zlecenia.
///
/// Panel ma pola „KOMENTARZ POZYCJI (MT5)" i „własny komentarz" od dawna, ale
/// nikt nigdy nie wołał `Mt5Bridge::set_tag` — więc komentarz był ZAWSZE
/// domyślnym `CD`, cokolwiek użytkownik wpisał. To ta sama klasa błędu co
/// ustawienie, które nie dociera do silnika: pole w panelu jest, opis jest,
/// a efektu nie ma.
///
/// Znacznik idzie na początek komentarza, bo część maszynowa (`<koszyk>.<poziom>`)
/// musi się zmieścić w 31 znakach MT5 — dlatego przycinamy go do 24 znaków
/// i zostawiamy tylko drukowalne ASCII (MT5 nie lubi reszty). 24 znaki to
/// tyle, żeby zmieścił się znacznik z datą i godziną (`TEST 2026-07-28 20:55`
/// = 21 znaków) razem z częścią maszynową.
// ============================================================
//  ROZGRZEWKA HISTORII RYNKU
// ============================================================

/// Ile świec H1 poprosić PONAD wymagane okno.
///
/// Zapas jest potrzebny z dwóch powodów: ostatnia świeca zwykle się dopiero
/// formuje (odrzucamy ją), a broker potrafi mieć w serii dziury po przerwach
/// technicznych. Doba zapasu kosztuje nas kilka milisekund i zdejmuje obie
/// te niepewności.
const ZAPAS_H1: usize = 24;

/// Ile świec M1 dolewamy do bufora zmienności ponad wymagane okno.
const ZAPAS_M1: usize = 30;

fn rozgrzej_historie(
    st: &StateHandle,
    broker: &Recording,
    silniki: &mut routing::Silniki,
    symbol: &str,
) {
    // Ile potrzebuje NAJBARDZIEJ WYMAGAJĄCY silnik — historia jest wspólna dla
    // rachunku, więc bierzemy maksimum, nie sumę.
    let godzin = silniki
        .lista
        .iter()
        .filter(|s| s.engine.cfg.regime_filter != conduit_core::settings::RegimeFilter::Off)
        .map(|s| s.engine.cfg.regime_ma_hours as usize)
        .max()
        .unwrap_or(0);
    let minut = silniki
        .lista
        .iter()
        .map(|s| {
            s.engine
                .cfg
                .vol_window_min
                .max(s.engine.cfg.rev_exit_window_min)
        })
        .fold(0.0_f64, f64::max)
        .max(0.0) as usize;
    // Pełna odbudowa struktury wymaga nie tylko ATR, lecz także całego
    // horyzontu potwierdzonych swingów. Dodajemy po obu stronach okno
    // fractala i okres ATR; nadal mieści się to z dużym zapasem w limicie
    // 5000 świec dla typowych 24 h.
    let sr_minut = silniki
        .lista
        .iter()
        .filter(|s| {
            let c = &s.engine.cfg;
            !live_sr_v2_requested(c) && c.trail_sr_enabled
                && (c.trail_sr_min_prominence_atr > 0.0
                    || c.trail_sr_offset_atr_mult > 0.0
                    || c.trail_sr_offset_spread_mult > 0.0)
        })
        .map(|s| {
            let c = &s.engine.cfg;
            let tf = c.trail_sr_tf_min.max(1) as usize;
            c.trail_sr_struct_window_h.max(1) as usize * 60
                + (2 * c.trail_sr_fractal_n.max(1) as usize
                    + c.trail_sr_atr_period.max(1) as usize
                    + 3)
                    * tf
        })
        .max()
        .unwrap_or(0);

    if godzin == 0 && minut == 0 && sr_minut == 0 {
        st.log(
            "mt5",
            "info",
            "Rozgrzewka historii pominięta",
            "Żaden pracujący preset nie używa filtra reżimu, okna zmienności \
             ani dynamicznego S/R, \
             więc nie ma czego wczytywać."
                .to_string(),
        );
        return;
    }

    let md = conduit_mt5::MarketData::new(broker.inner.transport().handle());

    // ---------- H1: filtr reżimu ----------
    let mut price: Vec<(Ts, Px)> = Vec::new();
    let mut blad_h1: Option<String> = None;
    if godzin > 0 {
        let ile = (godzin + ZAPAS_H1).min(conduit_mt5::market::MAX_BARS);
        match md.candles(symbol, "H1", ile, None) {
            Ok(c) => {
                // Świeca, która się DOPIERO FORMUJE, nie jest godziną
                // zamkniętą. Silnik dołoży bieżącą godzinę sam, przy pierwszym
                // ticku — tak samo jak robi to w backteście.
                let bars = if c.last_closed() || c.bars.is_empty() {
                    &c.bars[..]
                } else {
                    &c.bars[..c.bars.len() - 1]
                };
                price = bars.iter().map(|b| (b.t(), b.close())).collect();
            }
            Err(e) => blad_h1 = Some(e.to_string()),
        }
    }

    // ---------- M1: bufor zmienności i reversal-exit ----------
    //
    // Ten bufor odbudowuje się z ticków w ciągu kilku minut (krok 5 s,
    // `vol_factor` chce 5 próbek), więc jego brak NIE jest awarią tej samej
    // klasy co martwy filtr reżimu. Dolewamy go, bo przy oknie 30–60 minut
    // zakres H−L byłby przez pierwsze pół godziny zaniżony — a to zaniża
    // mnożnik jednostek, czyli po cichu zmienia wielkość pozycji.
    let mut vol: Vec<(Ts, Px)> = Vec::new();
    let mut sr_bars: Vec<SrWarmupBar> = Vec::new();
    let mut blad_m1: Option<String> = None;
    let mut blad_sr_spread: Option<String> = None;
    if minut > 0 || sr_minut > 0 {
        let ile_vol = if minut > 0 { minut * 2 + ZAPAS_M1 } else { 0 };
        let ile = ile_vol
            .max(sr_minut.saturating_add(ZAPAS_M1))
            .min(conduit_mt5::market::MAX_BARS);
        match md.candles(symbol, "M1", ile, None) {
            Ok(c) => {
                if minut > 0 {
                    // Zachowujemy dotychczasową semantykę bufora zmienności,
                    // włącznie z ostatnią formującą się świecą.
                    vol = c.bars.iter().map(|b| (b.t(), b.close())).collect();
                }
                if sr_minut > 0 {
                    let zamkniete = if c.last_closed() || c.bars.is_empty() {
                        &c.bars[..]
                    } else {
                        &c.bars[..c.bars.len() - 1]
                    };
                    let spread_wymagany = silniki.lista.iter().any(|s| {
                        s.engine.cfg.trail_sr_enabled
                            && s.engine.cfg.trail_sr_offset_spread_mult > 0.0
                    });
                    let point = match md.symbol_info(symbol) {
                        Ok(si) if si.point > 0.0 => Some(si.point),
                        Ok(_) if spread_wymagany => {
                            blad_sr_spread =
                                Some("MT5 zwrócił point=0; spreadu nie wolno zgadywać".into());
                            None
                        }
                        Err(e) if spread_wymagany => {
                            blad_sr_spread =
                                Some(format!("nie udało się pobrać point instrumentu: {e}"));
                            None
                        }
                        _ => None,
                    };
                    if !spread_wymagany || point.is_some() {
                        let point = point.unwrap_or(0.0);
                        sr_bars = zamkniete
                            .iter()
                            .map(|b| SrWarmupBar {
                                ts: b.t(),
                                high: b.high(),
                                low: b.low(),
                                close: b.close(),
                                spread: b.spread().max(0) as f64 * point,
                            })
                            .collect();
                    }
                }
            }
            Err(e) => blad_m1 = Some(e.to_string()),
        }
    }

    let czynny = godzin == 0 || price.len() >= godzin;
    let mut sr_status: Vec<String> = Vec::new();
    let mut sr_czynny = true;
    for s in silniki.lista.iter_mut() {
        s.engine.set_market_history(price.clone(), vol.clone());
        let c = &s.engine.cfg;
        if live_sr_v2_requested(c) {
            sr_czynny=false;
            sr_status.push(format!("S/R V2 {}/{}: HOLD — LIVE producer unavailable",s.format,s.preset));
            continue;
        }
        let dynamiczny_sr = c.trail_sr_enabled
            && (c.trail_sr_min_prominence_atr > 0.0
                || c.trail_sr_offset_atr_mult > 0.0
                || c.trail_sr_offset_spread_mult > 0.0);
        if dynamiczny_sr {
            let zapas_barow =
                2 * c.trail_sr_fractal_n.max(1) as i64 + c.trail_sr_atr_period.max(1) as i64 + 3;
            let wymagane_ms = c.trail_sr_struct_window_h.max(1) as i64 * 3_600_000
                + zapas_barow * c.trail_sr_tf_min.max(1) as i64 * 60_000;
            let pokrycie_ok = match (sr_bars.first(), sr_bars.last()) {
                (Some(a), Some(z)) => z.ts.saturating_sub(a.ts) >= wymagane_ms,
                _ => false,
            };
            let gotowy = if blad_sr_spread.is_none() && pokrycie_ok {
                s.engine.rozgrzej_sr_z_m1(&sr_bars)
            } else {
                // Jawnie kasujemy ewentualny przeniesiony stan: po błędzie
                // historii nie wolno kontynuować na pozornie „ciepłym" ATR.
                let _ = s.engine.rozgrzej_sr_z_m1(&[]);
                false
            };
            sr_czynny &= gotowy;
            sr_status.push(format!(
                "dynamiczne S/R {}/{}: {} domkniętych M1, pokrycie={} → {}",
                s.format,
                s.preset,
                sr_bars.len(),
                if pokrycie_ok { "pełne" } else { "za krótkie" },
                if gotowy {
                    "CZYNNE"
                } else {
                    "NIECZYNNE (fail-closed)"
                }
            ));
        }
    }

    // ---------- MELDUNEK ----------
    // Bez tego wpisu nie da się z logu odczytać, CZY bot filtrował. Piszemy
    // liczby, nie ocenę: ile godzin wczytano, ile wymagane, werdykt.
    let mut tresc = String::new();
    if godzin > 0 {
        tresc.push_str(&format!(
            "filtr reżimu: historia {} h / wymagane {} h → {}\n",
            price.len(),
            godzin,
            if czynny {
                "CZYNNY"
            } else {
                "NIECZYNNY (przepuszcza wszystko)"
            }
        ));
    } else {
        tresc.push_str("filtr reżimu: wyłączony we wszystkich pracujących presetach\n");
    }
    if minut > 0 {
        tresc.push_str(&format!(
            "okno zmienności: {} próbek M1 / okno {} min\n",
            vol.len(),
            minut
        ));
    }
    for status in &sr_status {
        tresc.push_str(status);
        tresc.push('\n');
    }
    if let Some(e) = &blad_h1 {
        tresc.push_str(&format!("\n⚠ NIE UDAŁO SIĘ POBRAĆ ŚWIEC H1: {e}\n"));
    }
    if let Some(e) = &blad_m1 {
        tresc.push_str(&format!("⚠ NIE UDAŁO SIĘ POBRAĆ ŚWIEC M1: {e}\n"));
    }
    if let Some(e) = &blad_sr_spread {
        tresc.push_str(&format!("⚠ NIE UDAŁO SIĘ ODTWORZYĆ SPREADU S/R: {e}\n"));
    }
    if !czynny {
        tresc.push_str(
            "\nBOT STARTUJE Z MARTWYM FILTREM REŻIMU. Przy niepełnej historii \
             `regime_ok` PRZEPUSZCZA KAŻDY sygnał — bot będzie handlował \
             znacznie więcej, niż wynika z backtestu, który ten preset \
             wyprodukował. Sprawdź, czy terminal oddaje świece H1 dla tego \
             symbolu, i zrestartuj bota.",
        );
    }
    if !sr_czynny {
        tresc.push_str(
            "\nDYNAMICZNE S/R STARTUJE FAIL-CLOSED: do zebrania pełnego okna \
             zamkniętych świec nie podniesie żadnego SL. Nie podstawiamy \
             sztucznego ATR ani spreadu.",
        );
    }

    let historia_ok = czynny && sr_czynny;

    st.log(
        "mt5",
        if historia_ok { "info" } else { "warn" },
        if historia_ok {
            "Historia rynku wczytana"
        } else {
            "⚠ HISTORIA RYNKU NIEPEŁNA"
        },
        tresc,
    );

    // Martwy filtr na rachunku, który ma go używać, to nie jest drobiazg do
    // dziennika — to jest zmiana strategii bez wiedzy właściciela.
    if !czynny {
        st.notify(
            MailCategory::Lifecycle,
            "⚠ Bot wystartował z NIECZYNNYM filtrem reżimu",
            &format!(
                "Wczytano {} godzin historii, preset wymaga {}. Do czasu zebrania \
                 pełnego okna bot bierze sygnały, które preset każe odrzucać.",
                price.len(),
                godzin
            ),
        );
    }
    if !sr_czynny {
        st.notify(
            MailCategory::Lifecycle,
            "⚠ Dynamiczne S/R wystartowało bez pełnej historii",
            "Oś pozostaje fail-closed i nie modyfikuje SL, dopóki z zamkniętych \
             świec nie powstanie pełne okno ATR/spreadu. Sprawdź historię M1 \
             i point instrumentu w terminalu MT5.",
        );
    }
}

fn znacznik_komentarza(st: &StateHandle) -> String {
    let doc = st.read(|s| s.settings.clone());
    let tryb = doc
        .get("comment_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("source");
    if tryb != "custom" {
        return conduit_mt5::comment::DEFAULT_TAG.to_string();
    }
    let wlasny: String = doc
        .get("comment_custom")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(24)
        .collect();
    let wlasny = wlasny.trim().to_string();
    if wlasny.is_empty() {
        conduit_mt5::comment::DEFAULT_TAG.to_string()
    } else {
        wlasny
    }
}

fn parametry_archiwum(st: &StateHandle) -> (bool, u32, i64) {
    st.read(|s| {
        let g = |k: &str, d: f64| s.settings.get(k).and_then(|v| v.as_f64()).unwrap_or(d);
        (
            s.settings
                .get("journal_enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            g("archive_retention_days", 365.0).max(0.0) as u32,
            (g("server_tz_offset_h", 3.0) * 3_600_000.0) as i64,
        )
    })
}

fn archiwum(st: &StateHandle) -> Option<conduit_server::archive::MessageArchive> {
    let (wlaczone, dni, strefa) = parametry_archiwum(st);
    if !wlaczone {
        return None;
    }
    Some(conduit_server::archive::MessageArchive::new(
        st.workspace.archive_dir(),
        strefa,
        dni,
    ))
}

/// FORMAT przypisany kanałowi albo tematowi forum.
///
/// `None` znaczy „to źródło nie handluje" i jest to stan POPRAWNY: kanał można
/// nasłuchiwać i zbierać z niego sygnały, nie ryzykując na nieprzebadanym
/// formacie. Wiadomość z takiego źródła i tak zostawia ślad w dzienniku
/// decyzji — patrz `BrakTrasy`.
///
/// Temat forum jest osobnym źródłem: ma własny format, niezależny od formatu
/// kanału. To nie jest wyjątek, tylko konsekwencja — na forum każdy temat
/// prowadzi kto inny i podaje sygnały po swojemu.
fn format_zrodla(st: &StateHandle, src: &SourceKey) -> Option<String> {
    // Sama reguła mieszka na `ChannelBinding::format_dla` — tam, gdzie dają
    // się do niej dopisać testy bez stawiania całego stanu aplikacji.
    st.read(|s| {
        s.bindings
            .get(&src.chat_id.to_string())?
            .format_dla(src.topic_id)
    })
}

fn odcisk_zrodel(st: &StateHandle) -> Vec<String> {
    st.read(|s| {
        let mut u: Vec<String> = Vec::new();
        for b in s.bindings.values().filter(|b| b.monitored) {
            if !b.format.is_empty() && !u.contains(&b.format) {
                u.push(b.format.clone());
            }
            for f in b.topics.values() {
                if !f.is_empty() && !u.contains(f) {
                    u.push(f.clone());
                }
            }
        }
        u.sort();
        u
    })
}

fn odcisk_konfiguracji(st: &StateHandle) -> String {
    let zrodla = odcisk_zrodel(st).join(",");
    st.read(|s| {
        let l = s.lancuchy.aktywny();
        let nogi = l
            .map(|l| {
                l.presety
                    .iter()
                    .map(|(f, p)| format!("{f}>{p}"))
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        let pulapy = l.map(|l| format!("{:?}", l.pulapy)).unwrap_or_default();
        format!(
            "zrodla[{zrodla}] lancuch[{}] nogi[{nogi}] pulapy[{pulapy}]",
            s.lancuchy.aktywny
        )
    })
}

/// Bezsekretowa migawka TEGO, CZYM FAKTYCZNIE gra żywa pętla.
///
/// Nie serializujemy dokumentu UI ani secrets.json. Każda noga oddaje
/// końcowe Settings już po złożeniu preset + pola rachunku, więc rekord
/// pozwala odtworzyć realną konfigurację, a nie tylko nazwy plików. Osobno
/// zapisujemy routing obserwowanych źródeł oraz identyfikatory buforów
/// journala; te drugie nie wchodzą do config_sha256.
fn migawka_provenance(
    st: &StateHandle,
    silniki: &routing::Silniki,
) -> conduit_server::journal::provenance::ProvenanceSnapshot {
    let mut engines: Vec<serde_json::Value> = silniki
        .lista
        .iter()
        .map(|s| {
            serde_json::json!({
                "format": s.format,
                "preset": s.preset,
                "slot": s.slot,
                "fallback": s.zapasowy,
                "manage_only": s.tylko_zarzadzanie,
                "from_preset_file": s.z_pliku,
                "inactive_reason": s.powod,
                "settings": s.engine.cfg,
            })
        })
        .collect();
    engines.sort_by(|a, b| {
        let key = |v: &serde_json::Value| {
            (
                v.get("slot").and_then(|x| x.as_u64()).unwrap_or(0),
                v.get("format")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
            )
        };
        key(a).cmp(&key(b))
    });

    let mut engine_runs: Vec<serde_json::Value> = silniki
        .lista
        .iter()
        .map(|s| {
            serde_json::json!({
                "format": s.format,
                "slot": s.slot,
                "journal_run_id": s.engine.journal.run_id,
            })
        })
        .collect();
    engine_runs.sort_by_key(|v| v.get("slot").and_then(|x| x.as_u64()).unwrap_or(0));

    let (mode, sources) = st.read(|s| {
        let mut sources: Vec<serde_json::Value> = s
            .bindings
            .values()
            .filter(|binding| binding.monitored)
            .map(|binding| {
                serde_json::json!({
                    "channel_id": binding.channel_id,
                    "format": binding.format,
                    "topics": binding.topics,
                })
            })
            .collect();
        sources.sort_by_key(|v| v.get("channel_id").and_then(|x| x.as_i64()).unwrap_or(0));
        (format!("{:?}", s.mode), sources)
    });

    conduit_server::journal::provenance::ProvenanceSnapshot::new(serde_json::json!({
        "active_chain": silniki.lancuch,
        "mode": mode,
        "chain_limits": silniki.pulapy,
        "sources": sources,
        "engines": engines,
    }))
    .with_runtime(serde_json::json!({
        "engine_runs": engine_runs,
    }))
}

fn zbuduj_silniki(st: &StateHandle, core: &conduit_core::Settings, saldo: f64) -> routing::Silniki {
    // Łańcuch może przypisywać preset formatowi, do którego nie jest podpięty
    // ANI JEDEN obserwowany kanał. Stawianie dla niego silnika byłoby nie
    // tylko bezużyteczne — przy dwóch takich wpisach włączałoby tryb
    // wielosilnikowy (a z nim przenumerowanie koszyków) na rachunku, na
    // którym dalej gra jedno źródło.
    let uzywane = odcisk_zrodel(st);
    let (lancuchy, preset_id) = st.read(|s| (s.lancuchy.clone(), s.preset_id.clone()));
    let lancuch = lancuchy.aktywny().cloned().unwrap_or_default();
    // PORÓWNANIE NAZW FORMATU: bez rozróżniania wielkości liter i bez
    // otaczających spacji. Nazwa formatu jest kluczem w DWÓCH niezależnych
    // dokumentach (`channels.json` i `lancuchy.json`) — jedna spacja albo
    // „Zen" wpisane inaczej niż „ZEN" cicho rozspójniało routing: łańcuch
    // miał nogę, kanał miał format, a bot twierdził, że nie ma czym grać.
    let pasuje = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
    let formaty_lancucha = lancuchy.formaty_handlujace();
    let handlujace: Vec<String> = formaty_lancucha
        .iter()
        .filter(|f| uzywane.iter().any(|u| pasuje(u, f)))
        .cloned()
        .collect();

    if handlujace.len() <= 1 {
        let (format, powod) = match handlujace.first() {
            Some(f) => (f.clone(), String::new()),
            None => match formaty_lancucha.first() {
                Some(f) => (f.clone(), "brakZrodla".to_string()),
                None => (String::new(), String::new()),
            },
        };
        let preset = lancuch
            .preset_dla(&format)
            .map(|s| s.to_string())
            .unwrap_or(preset_id);
        let cfg_nogi: Option<conduit_core::Settings> = if preset.is_empty() {
            None
        } else {
            st.workspace
                .load_presets()
                .into_iter()
                .find(|p| p.name.eq_ignore_ascii_case(&preset))
                .map(|p| conduit_core::wielosilnik::ustawienia_formatu(&p.settings, core))
        };
        if !format.is_empty() && cfg_nogi.is_none() && !preset.is_empty() {
            st.log(
                "settings",
                "warn",
                format!("Noga {format} → {preset}: pliku presetu NIE MA na dysku"),
                format!(
                    "Silnik gra ustawieniami z dokumentu panelu (stara ścieżka). \
                     Jeśli to szczebel drabinki, jego ochrona (lot_max itd.) NIE \
                     obowiązuje — wgraj plik presetu do katalogu presets.{}",
                    wskazowka_o_presecie(st, &preset).unwrap_or_default()
                ),
            );
        }
        let z_pliku = cfg_nogi.is_some();
        let mut engine = Engine::new(cfg_nogi.unwrap_or_else(|| core.clone()), saldo);
        engine.pulapy = lancuch.pulapy.clone();
        if z_pliku {
            st.log(
                "settings",
                "info",
                format!("Silnik {format}: handel z presetu {preset}, rachunek z panelu"),
                "Łańcuch jednonogi ładuje pola HANDLU z pliku presetu nogi \
                 (ta sama zasada co przy wielu silnikach). Pola RACHUNKU — \
                 karta lota, koszty brokera, opóźnienie — nadal z dokumentu."
                    .to_string(),
            );
        }
        if !powod.is_empty() || format.is_empty() {
            st.log(
                "settings",
                "warn",
                "Żaden format nie handluje",
                format!(
                    "Aktywny łańcuch „{}” przypisuje presety formatom {:?}, ale ŻADEN z nich \
                     nie jest podpięty do obserwowanego kanału (obserwowane formaty: {:?}). \
                     Bot będzie dalej zarządzał koszykami, które już są na rachunku, ale \
                     NIE WEŹMIE żadnego nowego sygnału.\n\nNapraw to w jednym z dwoch miejsc: \
                     Kanały → wybierz format dla źródła, albo Łańcuchy → przypisz preset \
                     formatowi, którego już słuchasz.",
                    lancuch.nazwa,
                    lancuchy.formaty_handlujace(),
                    uzywane
                ),
            );
        }
        let mut s = routing::Silniki::pojedynczy(engine, format, preset, lancuch, z_pliku);
        s.lista[0].powod = powod;
        return s;
    }

    // ---------- dwa formaty lub więcej ----------
    //
    // Do budowy silników idzie łańcuch OKROJONY do formatów, których ktoś
    // faktycznie słucha. Bez tego wpis w łańcuchu bez podpiętego kanału
    // stawiałby pusty silnik i — co gorsza — włączałby tryb wielosilnikowy
    // (a z nim przenumerowanie koszyków) na rachunku, na którym dalej gra
    // jedno źródło.
    let mut lancuch_uzywany = lancuch.clone();
    lancuch_uzywany
        .presety
        .retain(|f, _| handlujace.contains(f));

    let presety: std::collections::BTreeMap<String, conduit_core::Settings> = st
        .workspace
        .load_presets()
        .into_iter()
        .map(|p| (p.name, p.settings))
        .collect();
    let (silniki, braki) = routing::Silniki::zbuduj(&lancuch_uzywany, &presety, core, saldo);

    for b in &braki {
        // Cisza jest zakazana: format wpisany do łańcucha, którego presetu nie
        // ma na dysku, znaczy że część sygnałów przepadnie BEZ ŚLADU.
        //
        // EA-21: gdy plik o tej nazwie JEST, a niesie inne `name`, komunikat
        // „wgraj brakujący plik" jest mylący — dopisujemy prawdziwą przyczynę.
        let szczegol = match b {
            routing::BrakTrasy::PresetNieIstnieje { preset, .. } => {
                wskazowka_o_presecie(st, preset).unwrap_or_default()
            }
            _ => String::new(),
        };
        st.log(
            "settings",
            "error",
            format!("Format bez presetu: {}", b.kod()),
            format!("{}{szczegol}", b.opis()),
        );
    }
    if !silniki.sloty_przesuniete.is_empty() {
        st.log(
            "settings",
            "warn",
            "Kolizja slotów numeracji koszyków",
            format!(
                "Formatom {:?} trzeba było przesunąć slot, bo skrót ich nazwy trafił \
                 w zajęty numer. Skutek jest wyłącznie kosmetyczny (inne numery \
                 koszyków), ale zapisuję to, bo numeracja tych formatów zmieni się \
                 ponownie, gdy zmieni się skład łańcucha.",
                silniki.sloty_przesuniete
            ),
        );
    }
    let opis: Vec<String> = silniki
        .lista
        .iter()
        .map(|s| format!("{} → {} (slot {})", s.format, s.preset, s.slot))
        .collect();
    st.log(
        "settings",
        "success",
        format!(
            "Łańcuch „{}”: {} silników",
            lancuch.nazwa,
            silniki.lista.len()
        ),
        format!(
            "{}\n\nKażdy format ma własny silnik i własne ustawienia zarządzania, \
             ale wszystkie stoją na JEDNYM rachunku. Limity ekspozycji i obsunięcia \
             działają dwuwarstwowo: obowiązuje NIŻSZY z limitu presetu i pułapu łańcucha.",
            opis.join("\n")
        ),
    );
    silniki
}

// ============================================================
//  EA-21: ROZJAZD PÓL RACHUNKU MIĘDZY NOGAMI ŁAŃCUCHA
// ============================================================

fn sprawdz_rozjazd_nog(
    st: &StateHandle,
    silniki: &routing::Silniki,
    core: &conduit_core::Settings,
) -> Option<String> {
    let presety: std::collections::BTreeMap<String, conduit_core::Settings> = st
        .workspace
        .load_presets()
        .into_iter()
        .map(|p| (p.name, p.settings))
        .collect();
    // Bierzemy nogi, które FAKTYCZNIE stoją na plikach presetów — i to plik
    // jest tu deklaracją, z którą porównujemy rachunek. Kolejność jest stała
    // (kolejność silników), żeby dwa starty na tej samej konfiguracji dawały
    // identyczny wpis w dzienniku.
    let pary: Vec<(String, conduit_core::Settings)> = silniki
        .lista
        .iter()
        .filter(|s| s.z_pliku && !s.preset.is_empty())
        .filter_map(|s| {
            presety
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(&s.preset))
                .map(|(_, c)| (s.format.clone(), c.clone()))
        })
        .collect();
    let nogi: Vec<(&str, &conduit_core::Settings)> =
        pary.iter().map(|(f, c)| (f.as_str(), c)).collect();
    let lista = conduit_core::formaty::rozjazd_rachunku(&nogi, core);
    if lista.is_empty() {
        return None;
    }

    let tabela: Vec<String> = lista.iter().map(|x| x.to_string()).collect();
    let oslabione: Vec<&conduit_core::formaty::RozjazdRachunku> =
        lista.iter().filter(|x| x.oslabia).collect();

    let parytetowe: Vec<&conduit_core::formaty::RozjazdRachunku> = lista
        .iter()
        .filter(|x| {
            conduit_core::formaty::POLA_PARYTETU_WYKONANIA
                .contains(&x.pole.as_str())
        })
        .collect();

    if oslabione.is_empty() {
        if nogi.len() == 1 && !parytetowe.is_empty() {
            let powod = format!(
                "rachunek zmienia ścieżkę wykonania nogi {} ({})",
                parytetowe[0].format, parytetowe[0].pole
            );
            st.log(
                "settings",
                "error",
                format!(
                    "ROZJAZD PARYTETU LIVE/BACKTEST: {} pól wykonania",
                    parytetowe.len()
                ),
                format!(
                    "Aktywny łańcuch ma jedną nogę, lecz dokument rachunku nadpisuje pola, \
                     które mogą zmienić rozmiar, wystawienie, anulowanie albo zamknięcie \
                     zlecenia. Nawet ostrzejszy bezpiecznik nie jest tu kosmetyką: wynik \
                     live przestaje być wynikiem ukoronowanego presetu.\n\n{}\n\n\
                     HANDEL JEST ZATRZYMANY; zarządzanie otwartymi pozycjami działa dalej. \
                     Zsynchronizuj te wartości w settings.json i presecie albo wykonaj \
                     koronację z dokładnie tym dokumentem rachunku.",
                    parytetowe
                        .iter()
                        .map(|x| x.to_string())
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            );
            return Some(powod);
        }
        st.log(
            "settings",
            "warn",
            format!("Rachunek nadpisuje {} pól nogom łańcucha", lista.len()),
            format!(
                "Rachunek jest JEDEN, więc poniższe wartości z presetów nóg zostały \
                nadpisane wartością z dokumentu panelu. Żadna z nich nie osłabia \
                deklarowanego bezpiecznika ani — dla pojedynczej nogi — nie zmienia \
                ścieżki decyzji zleceń, więc handel idzie dalej. Warunki brokera lub \
                ustawienia runtime nadal różnią się od przenośnego presetu:\n\n{}",
                tabela.join("\n")
            ),
        );
        return None;
    }

    let powod = format!(
        "rachunek zdejmuje bezpiecznik nodze {} ({})",
        oslabione[0].format, oslabione[0].pole
    );
    st.log(
        "settings",
        "error",
        format!(
            "ROZJAZD BEZPIECZNIKÓW: {} nóg gra bez własnej ochrony",
            oslabione.len()
        ),
        format!(
            "Rachunek jest JEDEN i jego pola nadpisują presety nóg. Poniższe \
             nadpisania ZDEJMUJĄ nodze ochronę, którą deklarował jej preset — czyli \
             bot pojechałby na konto z INNYM układem bezpieczników niż ten, który \
             zmierzył backtest.\n\n{}\n\n\
             HANDEL JEST ZATRZYMANY (zarządzanie otwartymi koszykami działa dalej, \
             nowych wejść nie ma). Napraw JEDNO z dwóch i uruchom ponownie:\n\
             \x20 · ustaw to pole w USTAWIENIACH RACHUNKU (dokument panelu) na wartość \
             co najmniej tak ostrą, jak deklaruje preset nogi, albo\n\
             \x20 · zdejmij z łańcucha nogę, której preset obiecuje ochronę, jakiej \
             rachunek nie daje.\n\n\
             Uzasadnienie listy pól: `formaty.rs::BEZPIECZNIKI_RACHUNKU`.",
            tabela.join("\n")
        ),
    );
    Some(powod)
}

fn wskazowka_o_presecie(st: &StateHandle, preset: &str) -> Option<String> {
    if preset.is_empty() {
        return None;
    }
    let plik = st.workspace.presets_dir().join(format!("{preset}.json"));
    let tresc = std::fs::read_to_string(&plik).ok()?;
    let nazwa = serde_json::from_str::<serde_json::Value>(&tresc)
        .ok()
        .and_then(|v| {
            v.get("name")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        });
    Some(match nazwa {
        Some(n) => format!(
            "\n\n⛔ PLIK JEST, TYLKO NAZYWA SIĘ INACZEJ W ŚRODKU.\n\
             \x20 {} istnieje, ale niesie `name`: „{n}”, a łańcuch pyta o „{preset}”.\n\
             \x20 Tożsamością presetu jest POLE `name` w pliku, nie nazwa pliku — \
             więc bot słusznie twierdzi, że tego presetu nie ma.\n\
             \x20 Napraw jedno z dwóch: wpisz „{preset}” do pola `name` w pliku albo \
             wskaż w łańcuchu preset „{n}”.",
            plik.display()
        ),
        None => format!(
            "\n\n⛔ PLIK {} ISTNIEJE, ALE NIE DA SIĘ GO WCZYTAĆ (brak pola `name` \
             albo uszkodzony JSON). Bot pomija uszkodzone presety po cichu — ten \
             wpis jest jedynym śladem.",
            plik.display()
        ),
    })
}

type PamiecTresci = conduit_core::telegram_ingress::ContentMemory;

fn kanal_obserwowany(st: &StateHandle, src: &SourceKey) -> bool {
    // Reguła na `ChannelBinding::obserwuje` — wspólna z testami.
    st.read(|s| {
        s.bindings
            .get(&src.chat_id.to_string())
            .map(|b| b.obserwuje(src.topic_id))
            .unwrap_or(false)
    })
}

fn sciezka_sidecara() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let obok = dir.join("mt5_sidecar.py");
            if obok.is_file() {
                return obok;
            }
        }
    }
    let w_repo = conduit_mt5::default_sidecar_path();
    if w_repo.is_file() {
        return w_repo;
    }
    std::path::PathBuf::from("mt5_sidecar.py")
}

/// Startuje pętlę handlową. Zwraca `None`, gdy nie udało się utworzyć wątku.
///
/// `tg` to usługa Telegrama — pętla sama zaprenumeruje z niej wiadomości.
pub fn start(
    st: StateHandle,
    tg: Option<Arc<conduit_telegram::TelegramService>>,
) -> Option<Handle> {
    if !st.mt5_runtime_start_allowed() {
        st.log("mt5", "error", "Nie uruchamiam MT5: konfiguracja startowa niepotwierdzona",
            "Napraw konfigurację i uruchom Conduit ponownie. Odblokowanie panelu nie zastępuje poprawnego wczytania konfiguracji.");
        return None;
    }
    if !live_cost_start_allowed(&st) { return None; }
    if !live_sr_start_allowed(&st) { return None; }
    let (tx, rx) = std::sync::mpsc::channel::<LiveCmd>();
    let stop = Arc::new(AtomicBool::new(false));
    let connected = Arc::new(AtomicBool::new(false));

    st.set_runtime(Arc::new(LiveRuntime {
        tx: tx.clone(),
        connected: Arc::clone(&connected),
    }));

    // Wiadomości z Telegrama wpadają do tej samej skrzynki, co polecenia
    // z panelu. Jedna kolejka = jedna kolejność zdarzeń = brak wyścigu
    // między „zamknij koszyk" z panelu a „SL HIT" z kanału.
    //
    // Wątek pośredniczący, bo usługa Telegrama nie zna typu `LiveCmd`
    // (i nie powinna — zależność idzie tylko w jedną stronę).
    if let Some(tg) = tg {
        // DOZÓR NAD TELEGRAMEM — własny wątek, bo to jedyna rzecz w programie,
        // która musi działać także wtedy, gdy MT5 leży i pętla handlowa kręci
        // się w ponowieniach. Awaria Telegrama i awaria MT5 są niezależne;
        // wykrywanie jednej nie może zależeć od drugiej.
        dozor_telegrama(st.clone(), Arc::clone(&tg), Arc::clone(&stop));

        let (mtx, mrx) = std::sync::mpsc::channel::<IncomingMessage>();
        tg.subscribe(mtx);
        let tx2 = tx.clone();
        let st_f = st.clone();
        let _ = std::thread::Builder::new().name("conduit-tg-filtr".into()).spawn(move || {
            // ARCHIWUM PRZED FILTREM — i to jest cała jego wartość.
            //
            // Gdyby zapis szedł za bramką `kanal_obserwowany`, archiwum byłoby
            // kompletne tylko tak, jak kompletna jest bieżąca konfiguracja
            // nasłuchu: włączenie kanału jutro nie odtworzy tego, co mówił
            // wczoraj. A eksport z Telegrama tego nie nadrobi, bo zwija
            // edycje do wersji końcowej i gubi wiadomości skasowane.
            let mut arch_param = parametry_archiwum(&st_f);
            let mut arch = archiwum(&st_f);
            let mut tresci = PamiecTresci::new();
            for m in mrx {
                let obserwowany = kanal_obserwowany(&st_f, &m.source);
                // ARCHIWUM ODŚWIEŻANE W LOCIE. Konfiguracja czytana raz przy
                // starcie wątku rozjeżdżała się z dziennikiem silnika
                // (przeładowanie co 2 s) aż do restartu procesu. Parametry
                // mutowalne (`retention_days`, `local_offset_ms`) zmieniamy
                // W MIEJSCU — nowy obiekt zaczynałby `seq` od zera i dublował
                // numery wierszy w pliku dnia.
                let p = parametry_archiwum(&st_f);
                if p != arch_param {
                    arch_param = p;
                    if !p.0 {
                        arch = None;
                    } else if let Some(a) = arch.as_mut() {
                        a.retention_days = p.1;
                        a.local_offset_ms = p.2;
                    } else {
                        arch = archiwum(&st_f);
                    }
                }
                // ODSIEW DOSTAW BEZ ZMIANY TREŚCI — patrz [`PamiecTresci`].
                // Liczony także dla wiadomości nieobserwowanych nie jest:
                // one i tak nie idą do silnika, a pamięć ma zostać mała.
                let bez_zmiany = obserwowany && tresci.duplikat_tresci(&m);
                if bez_zmiany {
                    tracing::debug!(
                        msg_id = m.msg_id,
                        zrodlo = %m.source_name,
                        "powtórna dostawa bez nowej wykonywalnej rewizji — nie idzie do silnika"
                    );
                }

                let received_utc = conduit_server::now_ms();
                if obserwowany && !bez_zmiany && tx2.send(LiveCmd::Kanal(m.clone(), received_utc)).is_err() {
                    return;
                }

                // KRONIKA — TEN SAM STRUMIEŃ, ŻADNEGO DRUGIEGO POŁĄCZENIA.
                //
                // Conduit ma już sesję MTProto. Uruchomienie tu drugiego
                // klienta (choćby „tylko do nagrywania") to AUTH_KEY_DUPLICATED
                // i Telegram wyłącza OBIE strony — zdarzyło się to już w tym
                // projekcie. Wbudowana kronika jest więc DRUGIM ODBIORCĄ tej
                // samej wiadomości, dokładnie tak jak archiwum obok: przed
                // bramką `kanal_obserwowany`, żeby włączenie kanału jutro nie
                // wymagało wczorajszych danych, których nikt nie zapisał.
                //
                // `kronika_zapisz` nie zwraca błędu i nie może: to jest wątek,
                // przez który przechodzą sygnały handlowe.
                st_f.kronika_zapisz(
                    conduit_server::kronika::Przychodzace {
                        odebrano_ms: received_utc,
                        rodzaj: if m.edit_of.is_some() {
                            conduit_server::kronika::Rodzaj::Edycja
                        } else {
                            conduit_server::kronika::Rodzaj::Nowa
                        },
                        chat_id: m.source.chat_id,
                        chat: &m.source_name,
                        temat: m.source.topic_id,
                        msg_id: m.msg_id,
                        reply_to: m.reply_to,
                        edit_of: m.edit_of,
                        ts_telegram_ms: m.ts,
                        text: &m.text,
                        nasluchiwany: obserwowany,
                        // Format przypisany źródłu W TEJ CHWILI — bez niego
                        // z pliku nie da się odtworzyć, KTÓRYM parserem bot
                        // czytał tę wiadomość, a przy dwóch formatach naraz
                        // to jest połowa odpowiedzi na pytanie „dlaczego tak".
                        format: format_zrodla(&st_f, &m.source).as_deref(),
                    },
                    // podpowiedź dla interfejsu i dla opcji „pomijaj
                    // nierozpoznane"; prawdą w pliku pozostaje surowy tekst
                    !conduit_core::parser::parse(&m.text)
                        .iter()
                        .all(|s| matches!(s, conduit_core::parser::Signal::Info)),
                );

                if let Some(a) = arch.as_mut() {
                    let ev = if m.edit_of.is_some() {
                        conduit_server::archive::MsgEvent::Edited
                    } else {
                        conduit_server::archive::MsgEvent::Received
                    };
                    if let Err(e) = a.zapisz(
                        conduit_server::now_ms(),
                        ev,
                        m.source.chat_id,
                        m.source.topic_id,
                        m.msg_id,
                        m.reply_to,
                        m.edit_of,
                        m.ts,
                        &m.source_name,
                        &m.text,
                        obserwowany,
                    ) {
                        // Cisza jest zakazana także tutaj, ale awaria zapisu
                        // archiwum NIE MOŻE zatrzymać handlu — wiadomość leci
                        // dalej niezależnie od tego, czy dało się ją zapisać.
                        tracing::warn!(blad = %e, "nie udało się dopisać wiadomości do archiwum");
                    }
                }
                // Wysyłka do pętli handlowej jest już ZA NAMI — patrz komentarz
                // „HANDEL PIERWSZY, DOWODY DRUGIE" na górze tej pętli.
            }
        });
    }

    let st2 = st.clone();
    let stop2 = Arc::clone(&stop);
    let conn2 = Arc::clone(&connected);
    let handle = std::thread::Builder::new()
        .name("conduit-live".into())
        .spawn(move || petla(st2, rx, stop2, conn2))
        .ok()?;
    drop(handle);
    Some(Handle { stop })
}

// ============================================================
//  PĘTLA
// ============================================================

fn petla(
    st: StateHandle,
    rx: Receiver<LiveCmd>,
    stop: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
) {
    // Wiadomości, które przyszły, ZANIM most wstał. Nie wolno ich zgubić:
    // sygnał z Telegrama w trakcie łączenia z MT5 to najzwyklejsza rzecz
    // pod słońcem, a zgubiony sygnał jest niewidoczny w żadnym logu.
    let mut skrzynka: Vec<LiveCmd> = Vec::new();
    let mut proba = 0u32;
    // PAMIĘĆ MIĘDZY PODEJŚCIAMI — patrz `Trwale`. Musi żyć POZA pętlą,
    // bo to jest jedyna rzecz, która odróżnia „bot wznowił połączenie"
    // od „bot zaczyna dzień od nowa".
    let mut trwale = Trwale::default();

    loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }

        // ---------- podłączenie mostu ----------
        if !live_cost_start_allowed(&st) { return; }
        if !live_sr_start_allowed(&st) { return; }
        let mut cfg = sidecar_config(&st);
        // A concurrent settings update cannot activate an uncertified ledger
        // between the above check and Bridge::connect. The loop will HOLD it.
        cfg.closed_profit_net_costs = false;
        st.log(
            "mt5",
            "info",
            "Łączę z MetaTrader 5",
            format!(
                "symbol {} · magic {} · python {} · sidecar {}",
                cfg.symbol,
                cfg.magic,
                cfg.python.display(),
                cfg.script.display()
            ),
        );
        let configured_symbol = cfg.symbol.clone();
        let most = match Mt5Bridge::connect(cfg) {
            Ok(b) => b,
            Err(e) => {
                proba += 1;
                connected.store(false, Ordering::Relaxed);
                oznacz_mt5(&st, false);
                let tresc = format!(
                    "Próba {proba} nieudana: {e:#}\n\n\
                     Najczęstsze przyczyny:\n\
                     • terminal MetaTrader 5 nie jest uruchomiony albo nie jest zalogowany,\n\
                     • w Pythonie brakuje pakietu MetaTrader5 (pip install MetaTrader5),\n\
                     • pole „interpreter Pythona” wskazuje inny Python niż ten z pakietem,\n\
                     • w terminalu wyłączony jest handel algorytmiczny."
                );
                // Pierwsza porażka idzie mailem, kolejne tylko do dziennika —
                // dławik i tak by je zwinął, ale po co je w ogóle produkować.
                if proba == 1 {
                    st.notify(MailCategory::Mt5Connection, "Brak połączenia z MT5", &tresc);
                } else {
                    st.log("mt5", "error", "Brak połączenia z MT5", tresc);
                }
                // zbieramy to, co przyszło w czasie czekania
                while let Ok(c) = rx.try_recv() {
                    skrzynka.push(c);
                }
                for _ in 0..30 {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                continue;
            }
        };

        // In follow mode the contract is selected from the CURRENT broker, not saved settings.
        let symbol = if most.transport().config().follow_terminal_account {
            most.symbol_info().symbol.clone()
        } else { configured_symbol };

        proba = 0;
        // FOLLOW: a bridge alone does not mean the user's rendered account is bound.
        // Manual commands become available only after the first verified publication.
        connected.store(!most.transport().config().follow_terminal_account, Ordering::Release);

        // ŚWIECE dla wykresu. Uchwyt transportu, a NIE most: most należy od tej
        // chwili na wyłączność do pętli handlowej poniżej, a panel musi umieć
        // zapytać o świece z wątku HTTP, nie czekając na kolejny obrót pętli.
        // Odczyt jest wyłącznie odczytem — nie ma stąd drogi do zlecenia.
        st.set_market(Arc::new(crate::market::Mt5Market::new(
            most.transport().handle(),
        )));

        let kto = most.ident().clone();
        st.notify(
            MailCategory::Mt5Connection,
            "Połączono z MetaTrader 5",
            &format!(
                "Rachunek {} ({}) · {} · {}\n\
                 Symbol {symbol}. Bot zarządza pozycjami od tej chwili.",
                kto.login,
                kto.kind(),
                kto.server,
                kto.company
            ),
        );

        // ---------- pętla handlowa ----------
        let powod = handel(&st, most, &rx, &stop, &symbol, &mut skrzynka, &mut trwale, &connected);
        connected.store(false, Ordering::Relaxed);
        // Źródło świec odpinamy RAZEM z mostem. Inaczej po zerwaniu połączenia
        // wykres dostawałby ostatnią zbuforowaną paczkę jako bieżącą — czyli
        // dokładnie to kłamstwo, które ta zmiana miała usunąć.
        st.clear_market();
        oznacz_mt5(&st, false);

        if stop.load(Ordering::Relaxed) {
            return;
        }
        st.notify(
            MailCategory::Mt5Connection,
            "Utracono połączenie z MT5",
            &format!(
                "{powod}\n\nOtwarte pozycje zostają u brokera i NIE są zarządzane, \
                 dopóki połączenie nie wróci. Bot próbuje dalej."
            ),
        );
        std::thread::sleep(Duration::from_secs(3));
    }
}

#[derive(Default)]
struct Trwale {
    /// pamięć każdego silnika z osobna, kluczowana NAZWĄ FORMATU
    silniki: std::collections::BTreeMap<String, TrwalySilnik>,
    szczyt_equity: f64,
    /// koszyki w PAMIĘCI — świeższe niż zrzut na dysku (ten zapisuje się co 5 s)
    koszyki: Vec<Basket>,
    next_basket_id: u32,
    /// czy to jest pierwsze podejście w tym procesie
    bylo_polaczenie: bool,
    /// Sygnały z kanału czekające na ręczną decyzję (tryb MANUAL).
    ///
    /// Musi przeżyć rekonekt razem z listą wiadomości w panelu — inaczej
    /// przyciski „Wykonaj / Odrzuć" zostają na ekranie, a nie ma już czego
    /// wykonać.
    czekajace: std::collections::HashMap<String, IncomingMessage>,
    /// Czy mail „HANDEL ZATRZYMANY przez strażnika ryzyka" już poszedł.
    ///
    /// W `Trwale`, a nie w `handel()`, bo zatrzymanie przeżywa rekonekt —
    /// gdyby flaga wracała do `false` przy każdym wznowieniu mostu, ten sam
    /// mail szedłby po każdej czkawce terminala.
    zatrzymanie_zgloszone: bool,
    /// Zegary raportu okresowego — patrz [`ZegarRaportu`]. Leżą TU, bo
    /// `handel()` startuje od nowa przy każdym wznowieniu mostu do MT5,
    /// a odstęp między raportami ma być liczony od ostatniego RAPORTU,
    /// nie od ostatniego rekonektu.
    raport: ZegarRaportu,
    konto: String,
    risk_scope_magic: Option<i64>,
    /// A failed account snapshot cannot be escaped by switching to another account.
    follow_persist_failed: bool,
}

#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
struct TrwalySilnik {
    stats: Option<conduit_core::types::Stats>,
    halted: Option<String>,
    risk_override: bool,
    closed_today: Vec<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stopped_trading_day: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pending_sources: Vec<conduit_core::engine::PendingSourceRecord>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    continuation: Option<EngineContinuationV1>,
}

fn follow_account_key(toz: &conduit_mt5::proto::AccountIdent, symbol: &str) -> String {
    format!("{}@{}#{}:{}", toz.login, toz.server, toz.trade_mode, symbol)
}

fn follow_switch_allowed(memory: &Trwale, key: &str) -> bool {
    !memory.follow_persist_failed || memory.konto == key
}

/// The actual publisher's day/session anchors, shared with account-switch regression.
/// Extraction only: arithmetic/order is identical for legacy mode.
fn update_pnl_anchors(stats: &mut ui::Stats, balance: f64, equity: f64, day: i64, ts: i64) {
    if stats.day_key != day && day > 0 {
        stats.day_key = day;
        stats.day_start_equity = equity;
        stats.peak_equity_today = equity;
        stats.max_dd_today = 0.0;
        stats.drawdown_now = 0.0;
        stats.peak_balance_today = balance;
        stats.max_dd_balance_today = 0.0;
        stats.drawdown_balance_now = 0.0;
        stats.equity_curve = vec![ui::CurvePoint { t: ts, v: equity }];
    }
    if stats.session_start_equity <= 0.0 {
        stats.session_start_equity = equity;
    }
    stats.peak_equity_today = stats.peak_equity_today.max(equity);
    stats.drawdown_now = (stats.peak_equity_today - equity).max(0.0);
    stats.max_dd_today = stats.max_dd_today.max(stats.drawdown_now);
    if stats.peak_balance_today <= 0.0 {
        stats.peak_balance_today = balance;
    }
    stats.peak_balance_today = stats.peak_balance_today.max(balance);
    stats.drawdown_balance_now = (stats.peak_balance_today - balance).max(0.0);
    stats.max_dd_balance_today = stats.max_dd_balance_today.max(stats.drawdown_balance_now);
    stats.pnl_today = equity - stats.day_start_equity;
    stats.pnl_session = equity - stats.session_start_equity;
}

#[derive(serde::Serialize, serde::Deserialize)]
struct FollowAccountMemory {
    version: u32,
    login: i64,
    server: String,
    trade_mode: u8,
    magic: i64,
    symbol: String,
    silniki: std::collections::BTreeMap<String, TrwalySilnik>,
    peak_equity: f64,
    ui_stats: ui::Stats,
    risk_override: ui::RiskOverride,
    risk_halt: String,
}

fn follow_risk_path(st: &StateHandle, toz: &conduit_mt5::proto::AccountIdent, magic: i64, symbol: &str) -> std::path::PathBuf {
    wznowienie::sciezka_scoped(&st.workspace,toz.login,&toz.server,toz.trade_mode as i32,magic,symbol)
        .with_file_name("risk_state.json")
}

fn read_follow_memory(st: &StateHandle, toz: &conduit_mt5::proto::AccountIdent, magic: i64, symbol: &str) -> anyhow::Result<Option<FollowAccountMemory>> {
    let path=follow_risk_path(st,toz,magic,symbol);
    let bytes=match std::fs::read(&path) {
        Ok(b)=>b,
        Err(e) if e.kind()==std::io::ErrorKind::NotFound => return Ok(None),
        Err(e)=>return Err(e.into()),
    };
    let m: FollowAccountMemory=serde_json::from_slice(&bytes)?;
    anyhow::ensure!(m.version==1 && m.login==toz.login && m.server==toz.server
        && m.trade_mode==toz.trade_mode && m.magic==magic && m.symbol==symbol
        && m.ui_stats.konto_kotwic==follow_account_key(toz,symbol),
        "niezgodna tożsamość w risk_state.json; nie resetuję ochrony rachunku");
    Ok(Some(m))
}

fn sygnatura_ryzyka(silniki: &routing::Silniki) -> String {
    serde_json::to_string(&silniki.lista.iter().map(|s| (
        &s.format, &s.engine.halted, s.engine.risk_override,
        s.engine.stopped_trading_day(), s.engine.pending_source_memory_revision(),
    )).collect::<Vec<_>>()).expect("risk signature contains only JSON-safe scalars")
}

fn save_follow_memory(st: &StateHandle, silniki: &routing::Silniki, toz: &conduit_mt5::proto::AccountIdent,
    magic: i64, symbol: &str, peak_equity: f64, diagnosis: &str) -> anyhow::Result<()> {
    let memory=silniki.lista.iter().map(|s| {
        let halted=s.engine.halted.as_deref().and_then(|r| {
            let rest=if diagnosis.is_empty() {r} else if r==diagnosis {""} else {
                r.strip_prefix(diagnosis).and_then(|x| x.strip_prefix(ui::HALT_SEP)).unwrap_or(r)
            };
            (!rest.is_empty()).then(|| rest.to_string())
        });
        (s.format.clone(),TrwalySilnik{stats:Some(s.engine.stats.clone()),halted,
            risk_override:s.engine.risk_override,closed_today:s.engine.closed_today.clone(),
            stopped_trading_day:s.engine.stopped_trading_day(),
            pending_sources:s.engine.export_pending_source_memory(),
            continuation:s.engine.export_strategy_continuation()})
    }).collect();
    let (mut ui_stats,risk_override)=st.read(|s|(s.stats.clone(),s.risk_override.clone()));
    ui_stats.konto_kotwic=follow_account_key(toz,symbol);
    let m=FollowAccountMemory{version:1,login:toz.login,server:toz.server.clone(),trade_mode:toz.trade_mode,
        magic,symbol:symbol.to_string(),silniki:memory,peak_equity,ui_stats,risk_override,
        risk_halt:rozbij_klasy_zatrzymania(silniki,diagnosis).1};
    conduit_server::store::write_json_atomic(&follow_risk_path(st,toz,magic,symbol),&m)
}

fn apply_follow_memory(st: &StateHandle, trwale: &mut Trwale, memory: FollowAccountMemory) {
    trwale.silniki=memory.silniki;
    trwale.szczyt_equity=memory.peak_equity;
    st.update(Sections::all(),|s| {
        s.stats=memory.ui_stats;
        s.risk_override=memory.risk_override;
        s.halt.ustaw(ui::KlasaHaltu::Ryzyko,memory.risk_halt);
    });
}

/// Bind durable risk for both fixed-login and terminal-follow connections.
/// Identity validation happens before discarding the previous in-memory account.
fn bind_account_risk(st: &StateHandle, trwale: &mut Trwale,
    toz: &conduit_mt5::proto::AccountIdent, magic: i64, symbol: &str) -> anyhow::Result<bool> {
    let key = follow_account_key(toz, symbol);
    anyhow::ensure!(follow_switch_allowed(trwale, &key),
        "Poprzedni rachunek ma niezapisany stan ochrony; napraw zapis przed zmianą konta.");
    anyhow::ensure!(!trwale.follow_persist_failed || trwale.risk_scope_magic == Some(magic),
        "Poprzedni zakres magic ma niezapisany stan ochrony; napraw zapis przed zmianą zakresu.");
    if trwale.konto == key && trwale.risk_scope_magic == Some(magic) { return Ok(false); }
    let memory = read_follow_memory(st, toz, magic, symbol)?;
    *trwale = Trwale::default();
    trwale.konto = key;
    trwale.risk_scope_magic = Some(magic);
    if let Some(memory) = memory { apply_follow_memory(st, trwale, memory); }
    Ok(true)
}

/// Clear rendered broker state independently of restored account PnL/risk anchors.
/// Returning A -> B -> A must not retain B rows just because A's Stats were restored.
fn clear_follow_transients(st: &StateHandle) {
    st.update(Sections::all(), |s| {
        s.positions.clear();
        s.pendings.clear();
        s.baskets.clear();
        s.closed.clear();
        s.pending_history.clear();
        s.quotes.clear();
        s.foreign = Default::default();
        s.connection.mt5 = "disconnected".into();
        s.connection.account_verified.clear();
        s.connection.account_session.clear();
        s.connection.resolved_symbol.clear();
    });
}

/// Account-scoped state is never inherited from an unbound or different UI backup.
fn reset_follow_ui(st: &StateHandle, key: &str, balance: f64) {
    st.update(Sections::all(), |s| {
        if s.stats.konto_kotwic != key {
            s.stats = ui::Stats::new(balance, conduit_server::now_ms());
            s.stats.konto_kotwic = key.to_string();
            s.risk_override = Default::default();
            s.halt.ustaw(ui::KlasaHaltu::Ryzyko, "");
            s.positions.clear();
            s.pendings.clear();
            s.baskets.clear();
            s.closed.clear();
            s.quotes.clear();
        }
        for m in &mut s.messages {
            if m.pending_action.as_deref() == Some("await") {
                m.pending_action = Some("dismissed".to_string());
            }
        }
    });
}

/// Właściwa pętla: kwotowania, wiadomości, polecenia, publikacja, dziennik.
/// Wraca z powodem zakończenia, gdy most padnie albo program się zamyka.
fn przetworz_ticki_live(
    silniki: &mut routing::Silniki,
    broker: &mut Recording,
    ticki: &[Quote],
    dziennik: &mut Option<conduit_server::journal::JournalWriter>,
    scisla_kolejnosc: bool,
) {
    if !ticki.is_empty() {
        // OBCE OBCIĄŻENIE PRZED KWOTOWANIAMI, nie po.
        silniki.przelicz_obce(broker, None);
    }
    for q in ticki {
        if scisla_kolejnosc {
            broker.inner.set_replay_quote(*q);
            debug_assert_eq!(
                broker.quote(),
                *q,
                "kwota brokera musi odpowiadać odtwarzanemu tickowi"
            );
        }
        let dispatch_utc = conduit_server::now_ms();
        silniki.kazdy(broker, |e, w| e.on_tick_received(w, q, dispatch_utc));
        if let Some(d) = dziennik.as_mut() {
            for s in silniki.lista.iter_mut() {
                if s.engine.journal.len() >= 512 {
                    let mut evs = s.engine.drain_journal();
                    let _ = d.write(&mut evs, conduit_server::now_ms());
                }
            }
        }
    }
}

fn handel(
    st: &StateHandle,
    most: Mt5Bridge,
    rx: &Receiver<LiveCmd>,
    stop: &AtomicBool,
    symbol: &str,
    skrzynka: &mut Vec<LiveCmd>,
    trwale: &mut Trwale,
    connected: &AtomicBool,
) -> String {
    let info = most.symbol_info().clone();
    let mut broker = Recording::new(most);
    let follow = broker.inner.transport().config().follow_terminal_account;
    let account_session = if follow {
        connected.store(false, Ordering::Release);
        clear_follow_transients(st);
        conduit_server::journal::provenance::new_run_id("mt5-account")
    } else { String::new() };
    let was_account_bound = !trwale.konto.is_empty();
    let account_changed = match bind_account_risk(st, trwale, broker.inner.ident(),
        broker.inner.transport().config().magic, symbol) {
        Ok(changed) => changed,
        Err(e) => {
            let reason = format!("Niedostępna pamięć ryzyka rachunku: {e}. Nie resetuję ochrony; napraw risk_state.json przed wznowieniem.");
            st.update(Sections::one(Section::Halt), |s|
                s.halt.ustaw(ui::KlasaHaltu::Diagnoza, reason.clone()));
            return reason;
        }
    };
    if !follow && account_changed {
        if was_account_bound {
            skrzynka.clear();
            while rx.try_recv().is_ok() {}
        }
        reset_follow_ui(st, &trwale.konto, broker.account().balance);
    }
    if follow {
        let key = follow_account_key(broker.inner.ident(), symbol);
        // Messages/commands received before account binding cannot safely name a new account.
        let mut dropped = skrzynka.len() + trwale.czekajace.len();
        skrzynka.clear();
        trwale.czekajace.clear();
        while rx.try_recv().is_ok() { dropped += 1; }
        reset_follow_ui(st, &key, broker.account().balance);
        if dropped > 0 {
            st.log("mt5", "warn", "Odrzucono nieprzypisaną kolejkę po podłączeniu konta",
                format!("{dropped} poleceń/wiadomości z okresu bez potwierdzonej sesji konta; nie przenoszę ich między rachunkami."));
        }
    }
    let mut znacznik = znacznik_komentarza(st);
    broker.inner.set_tag(znacznik.clone());

    // ---------- silnik ----------
    // `stops_level` NIE jest brany z panelu, tylko z serwera brokera. Wpisana
    // ręcznie wartość, która nie zgadza się z prawdą, daje zlecenia odrzucane
    // w nieskończoność — to była jedna z drogich lekcji starego bota.
    //
    // WIELKOŚĆ POZYCJI: `core_from_ui` celowo nie tyka trzech pól lota, bo mają
    // własny sterownik. Tryb demo je stosował, ścieżka żywa — nie, więc bot
    // handlował domyślnym `lot_fixed = 0,01` niezależnie od presetu (patrz
    // `settings_map::apply_lot`). Bez tej linijki konto nigdy się nie składa.
    let mut core = st.read(|s| {
        let mut c = live_core_from_ui(&s.settings);
        conduit_server::settings_map::apply_lot(&mut c, &s.lot);
        c
    });
    core.stops_level = info.stops_level_price();
    let saldo = broker.account().balance;
    let kredyt_brokera = broker.account().credit;
    // JEDEN SILNIK NA FORMAT HANDLUJĄCY — patrz `crate::routing`.
    let mut odcisk_biezacy = odcisk_konfiguracji(st);
    let mut silniki = zbuduj_silniki(st, &core, saldo);
    // Concurrent file change after pre-connect check: no new risk can escape.
    if silniki.lista.iter().any(|s| live_sr_v2_requested(&s.engine.cfg)) {
        note_live_sr_hold(st);
        broker.inner.hold_new_entries(LIVE_SR_V2_HOLD);
    }
    let run_id = conduit_server::journal::provenance::new_run_id("live");
    // Opis szczebli drabinki, na których konto NIE stoi. Budowany tu i przy
    // każdej przebudowie łańcucha — nie w pętli publikacji (4×/s), bo czyta
    // katalog presetów z dysku.
    let mut szczeble = zbuduj_szczeble(st, &core, saldo, &silniki.lancuch, &pary_nog(&silniki));
    for s in silniki.lista.iter_mut() {
        s.engine
            .set_run_id(format!("{run_id}/cfg-0/slot-{}", s.slot));
        // KREDYT od pierwszej chwili, nie od pierwszego ticka. Bez tej linijki
        // istnieje okno między podłączeniem a pierwszym kwotowaniem, w którym
        // `stats.credit == 0` — a w tym oknie potrafi już przyjść sygnał
        // i wtedy koszyk rozstawiłby się lotem liczonym od PEŁNEGO salda
        // z bonusem, czyli dwa razy większym, niż zamierzał właściciel.
        s.engine.stats.credit = kredyt_brokera;
    }

    // Durable risk was bound to this exact account before constructing engines.
    // PRZENIESIENIE PAMIĘCI Z POPRZEDNIEGO PODEJŚCIA — uzasadnienie przy
    // `struct Trwale`. Robimy to PRZED wznowieniem koszyków, żeby licznik
    // koszyków startował z właściwej wartości. TA SAMA funkcja, którą woła
    // przebudowa łańcucha i bateria dowodowa — druga ręczna kopia tej pętli
    // rozjechałaby się z pierwszą przy najbliższej poprawce, bez objawu.
    for (format, r) in przenies_pamiec(&mut silniki, trwale, saldo, kredyt_brokera) {
        st.log(
            "mt5",
            "warn",
            "Blokada strażnika ryzyka PRZENIESIONA przez ponowne podłączenie",
            format!(
                "Format {format}. Powód blokady: {r}\n\nUtrata połączenia z MT5 nie jest \
                 decyzją o wznowieniu handlu. Bot pozostaje zatrzymany, dopóki nie \
                 klikniesz „wznów handel” w panelu."
            ),
        );
    }

    let toz = broker.inner.ident().clone();
    st.log(
        "mt5",
        "success",
        "MetaTrader 5 podłączony",
        format!(
            "Rachunek {} ({}) · {} · {} · dźwignia 1:{}\n\
         {symbol} · saldo {:.2} {} · stops_level {:.2} (z serwera) · \
             wolumen {}–{} krok {}",
            toz.login,
            toz.kind(),
            toz.server,
            toz.company,
            toz.leverage,
            saldo,
            toz.currency,
            info.stops_level_price(),
            info.volume_min,
            info.volume_max,
            info.volume_step
        ),
    );
    // ---------- OD CZEGO LICZY SIĘ LOT ----------
    //
    // Wpis PRZY STARCIE, zawsze, nawet gdy bonusu nie ma. Bez niego jedyną
    // drogą do odpowiedzi na pytanie „czy bot liczy wolumen od moich 300 $,
    // czy od 600 $ z bonusem" jest odgadywanie z wolumenów w historii — a to
    // jest dokładnie ta pomyłka, która na koncie z bonusem 100 % podwaja
    // ekspozycję i zauważa się dopiero po fakcie.
    {
        let kredyt = silniki.glowny().engine.kredyt_skuteczny();
        let podstawa = silniki.glowny().engine.podstawa_lota();
        let zrodlo = if !core.odlicz_kredyt {
            "odliczanie kredytu WYŁĄCZONE"
        } else if core.kredyt_reczny > 0.0 {
            "kwota RĘCZNA z ustawień"
        } else {
            "odczyt z terminala (ACCOUNT_CREDIT)"
        };
        let rozjazd = core.odlicz_kredyt
            && core.kredyt_reczny > 0.0
            && (core.kredyt_reczny - kredyt_brokera).abs() > 0.01;
        st.log(
            "mt5",
            if rozjazd { "warn" } else { "info" },
            "Podstawa wielkości pozycji",
            format!(
                "saldo brokera   {saldo:>10.2} {waluta}\n\
                 kredyt          {kredyt:>10.2} {waluta}   ({zrodlo})\n\
                 podstawa lota   {podstawa:>10.2} {waluta}\n\
                 \n\
                 terminal raportuje ACCOUNT_CREDIT = {kredyt_brokera:.2} {waluta}\n\
                 {}",
                if rozjazd {
                    "⚠ ROZJAZD: kwota ręczna nie zgadza się z terminalem. Wygrywa ręczna.\n\
                     Jeśli broker zdjął bonus, WYZERUJ pole ręczne — zero znaczy AUTOMAT,\n\
                     czyli „bierz z terminala\", a nie „kredytu nie ma\"."
                } else {
                    "Kredyt jest PODUSZKĄ: margines, wolny depozyt i strażnik obsunięcia\n\
                     dalej widzą pełne saldo. Bonus schodzi wyłącznie z podstawy lota."
                },
                waluta = toz.currency,
            ),
        );
    }

    // ---------- ROZGRZEWKA HISTORII RYNKU ----------
    // Musi iść PRZED pierwszym `on_tick`. Uzasadnienie przy `rozgrzej_historie`.
    rozgrzej_historie(st, &broker, &mut silniki, symbol);

    // Rachunek na PRAWDZIWYCH pieniądzach musi być widać z drugiego końca
    // pokoju — i to zanim padnie pierwsze zlecenie, a nie po nim.
    if toz.is_real() {
        st.notify(
            MailCategory::Lifecycle,
            "UWAGA: rachunek RZECZYWISTY",
            &format!(
                "Bot podłączył się do rachunku {} ({} · {}) — to NIE jest konto demo. \
                 Od tej chwili zlecenia idą za prawdziwe pieniądze.",
                toz.login, toz.server, toz.company
            ),
        );
    }

    let mut diagnoza_petli: String;
    {
        let mut powody: Vec<String> = Vec::new();
        if let Some(r) = sprawdz_terminal(st, &toz, &info) {
            powody.push(r);
        }
        if let Some(r) = sprawdz_rozjazd_nog(st, &silniki, &core) {
            powody.push(r);
        }
        let (d_serwera, r_serwera) = st.read(|s| (s.halt.diagnoza.clone(), s.halt.ryzyko.clone()));
        if !d_serwera.is_empty() {
            powody.push(d_serwera);
        }
        diagnoza_petli = powody.join(ui::HALT_SEP);

        // KLASA RYZYKO — najpierw z SILNIKÓW (blokada przeniesiona przez
        // `przenies_pamiec` po czkawce mostu jest świeższa), a gdy tam pusto,
        // ze stanu wznowionego z dysku (restart procesu po zadziałaniu
        // strażnika obsunięcia).
        let (_, ryzyko_silnikow) = rozbij_klasy_zatrzymania(&silniki, "");
        let ryzyko = if ryzyko_silnikow.is_empty() {
            r_serwera
        } else {
            ryzyko_silnikow
        };

        let mut wszystko: Vec<String> = Vec::new();
        if !diagnoza_petli.is_empty() {
            wszystko.push(diagnoza_petli.clone());
        }
        if !ryzyko.is_empty() {
            wszystko.push(ryzyko);
        }
        if !wszystko.is_empty() {
            let blokada = wszystko.join(ui::HALT_SEP);
            for s in silniki.lista.iter_mut() {
                s.engine.halted = Some(blokada.clone());
            }
        }
    }

    // ---------- WZNOWIENIE STANU ----------
    // Musi iść PRZED pierwszym `on_tick`: rekoncyliacja koszyków w silniku
    // dopisuje pozycje do koszyków, które już istnieją, więc koszyk musi tam
    // być, zanim przyjdzie pierwsze kwotowanie.
    let mut zrzut_ostatni = wznow_koszyki(st, &mut silniki, &broker, &toz, symbol, trwale);
    let continuation_origin=live_continuation_origin(st,&broker,&toz,symbol,trwale);
    let continuation_reports=restore_strategy_memory(&mut silniki,trwale,&broker,continuation_origin);
    log_continuation_reports(st,&continuation_reports);
    let mut zrzut_kiedy = Instant::now();
    let mut zrzut_exit_signature = sygnatura_pending_exit(silniki.lista.iter()
        .flat_map(|s| s.engine.baskets.iter().map(|b| (b.id, &b.pending_exit))));

    // ---------- dziennik ----------
    let mut dziennik = if core.journal_enabled {
        Some(conduit_server::journal::JournalWriter::new(
            st.workspace.journal_dir(),
            conduit_server::journal::WriterConfig {
                text_mirror: core.journal_text_mirror,
                retention_days: core.journal_retention_days,
                local_offset_ms: core.server_tz_offset_ms,
                prefix: "live".into(),
            },
        ))
    } else {
        None
    };

    // Jawna tożsamość sesji musi być pierwszym rekordem tej części pliku.
    // Bypasuje filtr min_level świadomie: bez niej późniejszych decyzji nie
    // da się przypisać do binarki, rachunku i konkretnej konfiguracji.
    let mut provenance = conduit_server::journal::provenance::RuntimeProvenance::new(
        conduit_server::journal::provenance::RuntimeContext {
            run_id: run_id.clone(),
            instance_id: conduit_server::journal::provenance::process_instance_id().to_string(),
            account_login: toz.login,
            account_server: toz.server.clone(),
            symbol: symbol.to_string(),
            server_offset_ms: core.server_tz_offset_ms,
            session_offset_ms: core.session_offset(),
        },
    );
    let provenance_wall_ms = conduit_server::now_ms();
    let provenance_broker_ms = match broker.quote().ts {
        ts if ts > 0 => ts,
        _ => provenance_wall_ms + core.server_tz_offset_ms,
    };
    let mut provenance_start =
        provenance.session_start(provenance_broker_ms, migawka_provenance(st, &silniki));
    if let Some(writer) = dziennik.as_mut() {
        let _ = writer.write(
            std::slice::from_mut(&mut provenance_start),
            provenance_wall_ms,
        );
    }

    let mut bufor: Vec<ui::ChatMessage> = st.read(|s| s.messages.clone());
    let mut ostatnia_publikacja = Instant::now() - Duration::from_secs(60);
    let mut ostatnie_ustawienia = Instant::now();
    // mtime plików presetów — do przeładowania edycji per preset w locie
    let mut mtime_presetow: std::collections::HashMap<String, std::time::SystemTime> =
        std::collections::HashMap::new();
    // Szczyt equity też przeżywa ponowne podłączenie — inaczej strażnik
    // obsunięcia po czkawce terminala mierzyłby od NOWEGO, niższego szczytu
    // i przestawał widzieć stratę, która już się wydarzyła.
    let mut szczyt_equity = broker.account().equity.max(trwale.szczyt_equity);
    let mut ostrzezenie_dd = 0.0f64;
    let mut ostatni_ts = broker.quote().ts;
    // Cisza w strumieniu kwotowań — mierzona ZEGAREM MASZYNY, nie znacznikiem
    // z ticka. Zamarły terminal potrafi oddawać w kółko ten sam tick z tą samą
    // godziną i po znaczniku wygląda to jak sprawny rynek.
    let mut ostatni_tick_o = Instant::now();
    let mut cisza_zgloszona = false;
    let mut drabinka_o = Instant::now();
    // Próg wieku sygnału — odświeżany razem z resztą ustawień, żeby zmiana
    // w panelu działała bez restartu.
    let mut prog_wieku = prog_wieku_sygnalu(st);
    // Nowy Engine po przebudowie zaczyna własną sekwencję event_id. Numer
    // generacji w prefiksie zapobiega kolizji z rekordami sprzed zmiany.
    let mut config_generation = 0u64;

    // wiadomości, które czekały na podłączenie mostu
    let zaległe: Vec<LiveCmd> = std::mem::take(skrzynka);
    let mut kolejka: Vec<LiveCmd> = zaległe;
    let mut close_receipt_issue_logged: Option<String> = None;
    let mut net_cost_hold = false;
    let mut sr_warmup_hold = silniki.lista.iter().any(|s| live_sr_v2_requested(&s.engine.cfg))
        || st.read(|s| s.halt.diagnoza.contains(LIVE_SR_V2_HOLD));
    let mut czekajace = std::mem::take(&mut trwale.czekajace);

    let mut risk_state_signature = String::new();
    let mut initial_risk_error = save_follow_memory(st, &silniki, &toz,
        broker.inner.transport().config().magic, symbol, szczyt_equity, &diagnoza_petli)
        .err().map(|e| format!("Nie można zapisać pamięci ryzyka rachunku — handel wstrzymany: {e}"));
    trwale.follow_persist_failed = initial_risk_error.is_some();

    // JEDNO WYJŚCIE Z PĘTLI. Świadomie `break` zamiast `return`: dzięki temu
    // zapamiętanie stanu w `Trwale` i ostatni zrzut na dysk są na ścieżce,
    // której nie da się ominąć nową gałęzią wyjścia dopisaną za pół roku.
    let powod = loop {
        let mut sprawdz_provenance = false;
        if let Some(reason)=initial_risk_error.take() { break reason; }
        if stop.load(Ordering::Relaxed) {
            break "Program zamykany.".to_string();
        }
        let requested_follow = st.read(|s| s.settings.get("mt5_follow_terminal_account").and_then(Value::as_bool).unwrap_or(false));
        let requested_real = st.read(|s| s.settings.get("mt5_allow_real_account").and_then(Value::as_bool).unwrap_or(false));
        let requested_receipts = st.read(|s| s.settings.get("close_receipt_reconcile").and_then(Value::as_bool).unwrap_or(false));
        let requested_net_costs = st.read(|s| live_net_cost_requested(&s.settings));
        if requested_net_costs && !net_cost_hold {
            // Reject the mode transition, not the protective session. Both
            // producer and all refreshed consumers retain legacy accounting.
            broker.inner.hold_new_entries(LIVE_NET_COST_HOLD);
            net_cost_hold = true;
            st.log("mt5", "error", "Odrzucono zmianę księgowania LIVE — nowe wejścia HOLD", LIVE_NET_COST_HOLD);
        }
        if net_cost_hold && !diagnoza_petli.contains(LIVE_NET_COST_HOLD) {
            if !diagnoza_petli.is_empty() { diagnoza_petli.push_str(ui::HALT_SEP); }
            diagnoza_petli.push_str(LIVE_NET_COST_HOLD);
            st.update(Sections::one(Section::Halt), |s| s.halt.ustaw(ui::KlasaHaltu::Diagnoza, &diagnoza_petli));
        }
        sr_warmup_hold |= st.read(|s| s.halt.diagnoza.contains(LIVE_SR_V2_HOLD));
        if sr_warmup_hold {
            broker.inner.hold_new_entries(LIVE_SR_V2_HOLD);
            if !diagnoza_petli.contains(LIVE_SR_V2_HOLD) {
                if !diagnoza_petli.is_empty() { diagnoza_petli.push_str(ui::HALT_SEP); }
                diagnoza_petli.push_str(LIVE_SR_V2_HOLD);
                st.update(Sections::one(Section::Halt), |s| s.halt.ustaw(ui::KlasaHaltu::Diagnoza,&diagnoza_petli));
            }
        }
        if requested_follow != broker.inner.transport().config().follow_terminal_account
            || requested_receipts != broker.inner.transport().config().close_receipt_reconcile
            || (requested_follow && requested_real != broker.inner.transport().config().allow_real_account) {
            break "Zmieniono zasady wyboru konta — nowa sesja MT5.".to_string();
        }
        if broker.inner.transport().config().follow_terminal_account {
            // Check BEFORE replaying ticks or dispatching queued commands. The sidecar
            // repeats the account pin at every actual broker mutation (including retries).
            if let Err(e) = broker.inner.refresh_account() {
                break format!("Follow terminal: tożsamość/połączenie wymaga wznowienia: {e}");
            }
        }

        // ---------- tryb AUTO-EA ----------
        // Flaga runtime dla nadchodzących osi warstwy EA (trailing S/R,
        // cykl harvest). KONTRAKT ZERA: dziś żadna oś jej nie czyta, więc
        // AUTO-EA zachowuje się co do bitu jak AUTO. Odświeżana co obrót
        // pętli, bo tryb przełącza się w panelu bez restartu — a ustawiana
        // TUTAJ, przed poleceniami i tickami, żeby wszystko, co silniki
        // zrobią w tym obrocie, widziało już bieżący tryb.
        let auto_ea = tryb_auto_ea(st);
        for s in silniki.lista.iter_mut() {
            s.engine.tryb_auto_ea = auto_ea;
        }

        // ---------- polecenia i wiadomości ----------
        let mut kolejka_zamknieta = false;
        loop {
            match rx.try_recv() {
                Ok(c) => kolejka.push(c),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    kolejka_zamknieta = true;
                    break;
                }
            }
        }
        if kolejka_zamknieta {
            break "Kolejka poleceń zamknięta.".to_string();
        }

        // ---------- kwotowania ----------
        let scisla_kolejnosc = silniki
            .lista
            .iter()
            .any(|s| s.engine.cfg.live_tick_order_strict);
        let ticki = if scisla_kolejnosc {
            broker.inner.poll_ticks()
        } else {
            broker.inner.poll()
        };
        if let Some(q) = ticki.last() {
            ostatni_ts = q.ts;
            ostatni_tick_o = Instant::now();
            if cisza_zgloszona {
                cisza_zgloszona = false;
                st.log(
                    "mt5",
                    "success",
                    "Kwotowania wróciły",
                    format!("{symbol}: bid {:.2} / ask {:.2}", q.bid, q.ask),
                );
            }
        }
        // Cisza w strumieniu — jedyny objaw, po którym da się odróżnić
        // „rynek zamknięty" i „terminal zamarł, ale połączenie trzyma"
        // od normalnej pracy. Nadzorca procesu tego NIE widzi: proces żyje.
        // Ta sama bramka pory, którą ma odbudowa mostu niżej: przerwa dobowa
        // złota (00:00–01:05 czasu serwera) trwa dłużej niż próg 5 min, więc
        // bez niej mail „Brak kwotowań" wracał ok. 00:05 KAŻDEJ nocy — młyn,
        // przeciw któremu okno przerwy powstało, tyle że na powiadomieniu.
        if !cisza_zgloszona
            && ostatni_tick_o.elapsed() >= Duration::from_secs_f64(CISZA_KWOTOWAN_MIN * 60.0)
            && rynek_powinien_dzialac_z(przerwa_dobowa(st), offset_serwera_h(st))
        {
            cisza_zgloszona = true;
            zglos_cisze(st, symbol, &info, ostatni_ts, ostatni_tick_o.elapsed());
        }
        if ostatni_tick_o.elapsed() >= Duration::from_secs_f64(CISZA_ODBUDOWA_MIN * 60.0)
            && rynek_powinien_dzialac_z(przerwa_dobowa(st), offset_serwera_h(st))
        {
            break format!(
                "Brak kwotowań {symbol} od {} — odbudowuję połączenie z terminalem.",
                conduit_mt5::watchdog::opis_czasu(ostatni_tick_o.elapsed())
            );
        }

        // W ścisłym trybie ticki z paczki muszą przejść PRZED wiadomościami.
        // `poll_ticks` zostawił w moście ostatni kurs paczki, dlatego helper
        // przed każdym `on_tick` cofa go do kursu tego konkretnego ticka.
        // Dopiero po replayu wchłaniamy stan terminala z końca paczki.
        if scisla_kolejnosc {
            przetworz_ticki_live(&mut silniki, &mut broker, &ticki, &mut dziennik, true);
            broker.inner.poll_state();
        }
        let receipt_issue = broker.inner.close_receipt_issue().map(str::to_owned);
        if receipt_issue != close_receipt_issue_logged {
            if let Some(issue) = &receipt_issue {
                st.log("mt5", "error", "Niepełne potwierdzenie zamknięcia — blokada nowych wejść", issue);
            } else if close_receipt_issue_logged.is_some() {
                st.log("mt5", "info", "Potwierdzenia zamknięć uzgodnione", "Odczyt stanu brokera zakończony; tymczasowa bramka wejść zdjęta.");
            }
            close_receipt_issue_logged = receipt_issue;
        }

        // Wiadomości obsługujemy DOPIERO gdy znamy cenę. Sygnał policzony
        // przy kwotowaniu zerowym dałby strefę i SL względem zera.
        if ostatni_ts != 0 {
            for c in kolejka.drain(..) {
                let Some(c) = command_for_live_session(c, follow, &account_session) else {
                    st.log("mt5", "warn", "Odrzucono ręczne polecenie z nieaktualnej sesji",
                        "Polecenie nie zostanie przeniesione na inny rachunek ani nowe połączenie. Odśwież widok i zdecyduj ponownie.");
                    continue;
                };
                match c {
                    LiveCmd::Scoped { .. } => unreachable!("session envelope is removed by the dispatch gate"),
                    // Wiadomość z kanału — BRAMKA TRYBU. W MANUAL bot nic nie
                    // otwiera sam: wiadomość ląduje w panelu z przyciskami
                    // „Wykonaj / Odrzuć". Panel te przyciski rysował od dawna,
                    // ale nie było po stronie bota nikogo, kto by je obsłużył —
                    // a jednocześnie nic nie pilnowało, żeby MANUAL faktycznie
                    // wstrzymywał handel. To jest ta brakująca bramka.
                    LiveCmd::Kanal(mut im, received_utc) => {
                        // BRAMKA WIEKU — patrz `SYGNAL_MAX_WIEK_MIN`.
                        //
                        // MUSI stać PRZED `im.ts = ostatni_ts`, bo ta linijka
                        // kasuje jedyną informację, po której da się poznać, że
                        // sygnał jest sprzed pół godziny. Dotyczy wyłącznie
                        // wiadomości OTWIERAJĄCYCH koszyk — komunikaty
                        // zarządzające i edycje idą dalej niezależnie od wieku.
                        let stary = wiek_ponad_prog(conduit_server::now_ms(), im.ts, prog_wieku)
                            .filter(|_| otwiera_koszyk(&im.text, im.edit_of));
                        if let Some(wiek) = stary {
                            // Ślad w dzienniku decyzji i licznikach lejka —
                            // sam log panelu ginie z restartem, a lejek bez
                            // tego wpisu nie widział całej kolejki z przerwy.
                            dziennikuj_odrzut_wieku(
                                &mut silniki,
                                &broker,
                                &im,
                                format_zrodla(st, &im.source),
                                wiek,
                                prog_wieku,
                            );
                            let mut m = wiadomosc_ui(st, &im);
                            m.pending_action = Some("dismissed".into());
                            st.log(
                                "telegram",
                                "warn",
                                format!("Sygnał PRZETERMINOWANY ({wiek:.0} min) — pominięty"),
                                format!(
                                    "Wiadomość z „{}” została wysłana {wiek:.0} min temu, \
                                     a próg to {prog_wieku:.0} min. Bot jej NIE otworzył: \
                                     strefa i stop policzone dla ceny sprzed {wiek:.0} min \
                                     opisują rynek, którego już nie ma.\n\n\
                                     Tak wygląda kolejka z czasu, gdy most do MT5 leżał. \
                                     Komunikaty zarządzające istniejącymi koszykami \
                                     przechodzą normalnie — pominięte są tylko OTWARCIA.\n\n{}",
                                    im.source_name, im.text
                                ),
                            );
                            dopisz(&mut bufor, m);
                            continue;
                        }
                        im.ts = ostatni_ts;
                        ostrzez_o_nieczytelnym_sygnale(st, &im);
                        let auto = tryb_automatyczny(st);
                        let mut m = wiadomosc_ui(st, &im);
                        if auto {
                            m.pending_action = None;
                            // „Wykonany" TYLKO wtedy, gdy silnik naprawdę go wziął.
                            // Sygnał odrzucony przez bramkę wejścia wyglądał tu
                            // dotąd identycznie jak otwarty koszyk — a to każe
                            // szukać awarii mostu tam, gdzie bot zachował się
                            // dokładnie tak, jak mu kazano.
                            match skieruj(st, &mut silniki, &mut broker, &im, received_utc) {
                                WynikSygnalu::Odroczony(status) => {
                                    m.pending_action = Some("deferred".into());
                                    st.log("telegram", "info", "Sygnał ODROCZONY — nie wykonany", format!("{}: {:?}: {}", status.action_id, status.state, status.reason));
                                }
                                WynikSygnalu::Pominiety(powod) => st.log(
                                    "telegram",
                                    "warn",
                                    format!("Sygnał POMINIĘTY: {}", im.source_name),
                                    format!(
                                        "Bot ŚWIADOMIE go nie wziął — {powod}.\n\n\
                                         To NIE jest awaria mostu do MT5: silnik \
                                         zobaczył sygnał i odrzucił go zgodnie \
                                         z ustawieniami presetu.\n\n{}",
                                        im.text
                                    ),
                                ),
                                WynikSygnalu::Przekazany => st.log(
                                    "telegram",
                                    "info",
                                    format!("Sygnał wykonany: {}", im.source_name),
                                    im.text.clone(),
                                ),
                            }
                        } else {
                            m.pending_action = Some("await".into());
                            czekajace.insert(m.id.clone(), im.clone());
                            st.log(
                                "telegram",
                                "warn",
                                format!("Sygnał CZEKA na decyzję: {}", im.source_name),
                                format!(
                                    "Tryb MANUAL — bot nie otworzył niczego sam. \
                                     Kliknij „Wykonaj” przy wiadomości w panelu.\n\n{}",
                                    im.text
                                ),
                            );
                        }
                        dopisz(&mut bufor, m);
                    }
                    // Sygnał wpisany ręcznie w panelu — użytkownik właśnie
                    // kliknął „wyślij", więc pytanie „czy na pewno" byłoby
                    // pytaniem o to samo drugi raz.
                    LiveCmd::Reczny(mut im) => {
                        im.ts = ostatni_ts;
                        let powod = skieruj(st, &mut silniki, &mut broker, &im, conduit_server::now_ms());
                        let mut m = wiadomosc_ui(st, &im);
                        match powod {
                            WynikSygnalu::Odroczony(status) => {
                                m.pending_action = Some("deferred".into());
                                st.log("telegram", "info", "Sygnał ręczny ODROCZONY", format!("{}: {:?}: {}", status.action_id, status.state, status.reason));
                            }
                            WynikSygnalu::Pominiety(powod) => {
                                m.pending_action = Some("dismissed".into());
                                st.log(
                                    "telegram",
                                    "warn",
                                    "Sygnał ręczny POMINIĘTY",
                                    format!(
                                        "Silnik ŚWIADOMIE go nie wziął — {powod}.\n\n{}",
                                        im.text
                                    ),
                                );
                            }
                            WynikSygnalu::Przekazany => {
                                m.pending_action = Some("executed".into());
                                st.log(
                                    "telegram",
                                    "info",
                                    "Sygnał ręczny → silnik",
                                    im.text.clone(),
                                );
                            }
                        }
                        dopisz(&mut bufor, m);
                    }
                    LiveCmd::Wykonaj(id) => match czekajace.remove(&id) {
                        Some(mut im) => {
                            im.ts = ostatni_ts;
                            // Kliknięcie „Wykonaj" nie unieważnia bramek
                            // silnika — odrzut ma wyglądać jak odrzut, nie
                            // jak wykonanie (ta sama naprawa co w Reczny).
                            match skieruj(st, &mut silniki, &mut broker, &im, conduit_server::now_ms()) {
                                WynikSygnalu::Odroczony(status) => {
                                    oznacz(&mut bufor, &id, "deferred");
                                    st.log("telegram", "info", "Kliknięty sygnał ODROCZONY", format!("{}: {:?}: {}", status.action_id, status.state, status.reason));
                                }
                                WynikSygnalu::Pominiety(powod) => {
                                    oznacz(&mut bufor, &id, "dismissed");
                                    st.log(
                                        "telegram",
                                        "warn",
                                        "Sygnał kliknięty POMINIĘTY",
                                        format!(
                                            "Silnik ŚWIADOMIE go nie wziął — {powod}.\n\n{}",
                                            im.text
                                        ),
                                    );
                                }
                                WynikSygnalu::Przekazany => {
                                    oznacz(&mut bufor, &id, "executed");
                                    st.log(
                                        "telegram",
                                        "success",
                                        "Sygnał wykonany ręcznie",
                                        im.text,
                                    );
                                }
                            }
                        }
                        None => st.log(
                            "telegram",
                            "warn",
                            "Nie ma czego wykonać",
                            format!("wiadomość {id} już nie czeka"),
                        ),
                    },
                    LiveCmd::Odrzuc(id) => {
                        czekajace.remove(&id);
                        oznacz(&mut bufor, &id, "dismissed");
                        st.log("telegram", "info", "Sygnał odrzucony", id);
                    }
                    LiveCmd::Panel(cmd) => {
                        wykonaj_panel(
                            st,
                            &mut silniki,
                            &mut broker,
                            &cmd,
                            ostatni_ts,
                            &mut diagnoza_petli,
                        );
                    }
                }
            }
        }

        if !scisla_kolejnosc {
            przetworz_ticki_live(&mut silniki, &mut broker, &ticki, &mut dziennik, false);
        }

        // ---------- odmowy brokera ----------
        // Kolejność jest istotna: najpierw dziennik (żeby odmowa miała migawkę
        // rynku z TEJ chwili), potem log panelu i mail.
        let bledy = broker.take_errors();
        if !bledy.is_empty() {
            dziennikuj_odmowy(&mut silniki, &broker, &bledy);
            zglos_bledy(st, &bledy);
        }

        // ---------- straż obsunięcia ----------
        let acc = broker.account();
        szczyt_equity = szczyt_equity.max(acc.equity);
        sprawdz_obsuniecie(
            st,
            &silniki,
            acc,
            szczyt_equity,
            &mut ostrzezenie_dd,
            &mut trwale.zatrzymanie_zgloszone,
        );

        // ---------- zmiana ustawień w locie ----------
        // Panel zapisuje ustawienia w dowolnej chwili; silnik musi je zobaczyć
        // bez restartu programu. `stops_level` zawsze zostaje z serwera.
        if ostatnie_ustawienia.elapsed() >= Duration::from_secs(2) {
            ostatnie_ustawienia = Instant::now();
            sprawdz_provenance = true;
            prog_wieku = prog_wieku_sygnalu(st);
            przeladuj_ustawienia(
                st,
                &mut silniki,
                &mut core,
                info.stops_level_price(),
                &mut mtime_presetow,
            );
            if st.read(|s| s.halt.diagnoza.contains(LIVE_SR_V2_HOLD)) {
                sr_warmup_hold=true;
                broker.inner.hold_new_entries(LIVE_SR_V2_HOLD);
            }
            // Presety szczebli, na których konto nie stoi, też wolno edytować
            // z panelu — i zmiana ma być widoczna z tym samym opóźnieniem
            // (2 s) co przy nodze grającej. Inaczej użytkownik poprawia lot
            // presetu z poczekalni i nie ma potwierdzenia, że coś się stało.
            szczeble = zbuduj_szczeble(
                st,
                &core,
                broker.account().balance,
                &silniki.lancuch,
                &pary_nog(&silniki),
            );
            // Znacznik komentarza żyje poza `Settings`, więc ma własne
            // porównanie — inaczej zmiana samego komentarza nie doszłaby
            // do brokera aż do restartu.
            let nowy_znacznik = znacznik_komentarza(st);
            if nowy_znacznik != znacznik {
                znacznik = nowy_znacznik;
                broker.inner.set_tag(znacznik.clone());
                st.log(
                    "settings",
                    "info",
                    "Zmieniono komentarz zleceń",
                    format!("nowe zlecenia dostaną prefiks „{znacznik}”"),
                );
            }
        }

        // ---------- publikacja ----------
        if ostatnia_publikacja.elapsed().as_millis() >= PUBLISH_EVERY_MS {
            if follow {
                if let Err(e) = broker.inner.refresh_account() {
                    break format!("Follow terminal: nie publikuję niepotwierdzonej sesji rachunku: {e}");
                }
            }
            ostatnia_publikacja = Instant::now();
            opublikuj(
                st,
                &broker,
                &silniki,
                &szczeble,
                &bufor,
                symbol,
                &info,
                &diagnoza_petli,
                &account_session,
            );
            if follow { connected.store(true, Ordering::Release); }
        }

        // ---------- zrzut koszyków na dysk ----------
        // Jedyna rzecz, która pozwala odtworzyć DRABINKĘ CELÓW po restarcie:
        // w komentarzu MT5 mieści się numer koszyka i poziom siatki, i nic
        // więcej. Zapisujemy przy każdej ZMIANIE, bo utrata ostatniej zmiany
        // to utrata dokładnie tego koszyka, który właśnie powstał.
        let exit_signature = sygnatura_pending_exit(silniki.lista.iter()
            .flat_map(|s| s.engine.baskets.iter().map(|b| (b.id, &b.pending_exit))));
        let follow=broker.inner.transport().config().follow_terminal_account;
        // Source revisions change on restore, alias or withdrawal. The complete
        // ledger is serialized by zapisz_zrzut only when dirty or on its regular
        // interval; unchanged history must not be copied on every live tick.
        let risk_signature = sygnatura_ryzyka(&silniki);
        if zrzut_kiedy.elapsed() >= ZRZUT_CO || exit_signature != zrzut_exit_signature
            || risk_signature != risk_state_signature {
            zrzut_kiedy = Instant::now();
            if zapisz_zrzut(st, &silniki, &toz, symbol, &mut zrzut_ostatni, follow, broker.inner.transport().config().magic, szczyt_equity, &diagnoza_petli) {
                zrzut_exit_signature = exit_signature;
                risk_state_signature = risk_signature;
                trwale.follow_persist_failed=false;
            } else {
                trwale.follow_persist_failed=true;
                break "Nie zapisano stanu rachunku: zatrzymuję sesję, aby nie utracić ochrony po restarcie lub zmianie konta.".to_string();
            }
        }

        // ---------- podsumowanie okresowe ----------
        podsumowanie(st, &silniki, &broker, symbol, &mut trwale.raport);
        // ---------- DRABINKA FFS-1C + PRZEBUDOWA ŁAŃCUCHA W LOCIE ----------
        // Drabinka co najwyżej ZMIENIA aktywny łańcuch (SetAktywnyLancuch);
        // faktyczną przebudowę robi strażnik niżej — WSPÓLNA ścieżka dla
        // zmiany ręcznej z panelu i drabinkowej, więc adopcja koszyków jest
        // jedna i testowana raz.
        drabinka_tick(st, &broker, &mut drabinka_o);
        {
            let aktywny = st.read(|s| s.lancuchy.aktywny.clone());
            // KAŻDA zmiana konfiguracji, na której stoją silniki, przebudowuje
            // je od nowa — nie tylko zmiana nazwy łańcucha (patrz
            // `odcisk_konfiguracji`).
            let odcisk = odcisk_konfiguracji(st);
            let ten_sam_lancuch = aktywny == silniki.lancuch;
            if odcisk != odcisk_biezacy {
                config_generation += 1;
                let engine_run_prefix = format!("{run_id}/cfg-{config_generation}");
                // dziennik starych silników NAJPIERW — po podmianie nie
                // byłoby skąd go zabrać
                if let Some(d) = dziennik.as_mut() {
                    for s in silniki.lista.iter_mut() {
                        if !s.engine.journal.is_empty() {
                            let mut evs = s.engine.drain_journal();
                            let _ = d.write(&mut evs, conduit_server::now_ms());
                        }
                    }
                }
                zrzut_ostatni = przebuduj_lancuch(
                    st,
                    &mut silniki,
                    &broker,
                    &toz,
                    symbol,
                    &info,
                    trwale,
                    szczyt_equity,
                    &mut diagnoza_petli,
                    &engine_run_prefix,
                    &if ten_sam_lancuch {
                        format!(
                            "Zmieniła się konfiguracja źródeł albo nóg łańcucha.
                             było:  {odcisk_biezacy}
                             jest:  {odcisk}
                             Silniki stawiane od nowa, żeby zobaczyły zmianę bez restartu bota."
                        )
                    } else {
                        format!("Aktywny łańcuch zmienił się na „{aktywny}” (panel albo drabinka).")
                    },
                );
                odcisk_biezacy = odcisk;
                if diagnoza_petli.contains(LIVE_SR_V2_HOLD) {
                    sr_warmup_hold=true;
                    broker.inner.hold_new_entries(LIVE_SR_V2_HOLD);
                }
                sprawdz_provenance = true;
                // Po zmianie łańcucha inne szczeble drabinki są już INNE:
                // ten, z którego właśnie zeszliśmy, staje się „miniętym",
                // a aktywny przestaje być „w kolejce".
                szczeble = zbuduj_szczeble(
                    st,
                    &core,
                    broker.account().balance,
                    &silniki.lancuch,
                    &pary_nog(&silniki),
                );
                zrzut_kiedy = Instant::now();
            }
        }

        if sprawdz_provenance {
            let wall_ms = conduit_server::now_ms();
            let broker_ms = match broker.quote().ts {
                ts if ts > 0 => ts,
                _ => wall_ms + core.server_tz_offset_ms,
            };
            if let Some(mut event) =
                provenance.config_change(broker_ms, migawka_provenance(st, &silniki))
            {
                if let Some(writer) = dziennik.as_mut() {
                    let _ = writer.write(std::slice::from_mut(&mut event), wall_ms);
                }
            }
        }

        if let Some(d) = dziennik.as_mut() {
            for s in silniki.lista.iter_mut() {
                if !s.engine.journal.is_empty() {
                    let mut evs = s.engine.drain_journal();
                    let _ = d.write(&mut evs, conduit_server::now_ms());
                }
            }
        }

        // Bez kwotowań pętla i tak musi oddać procesor. 20 ms to ćwierć
        // typowego odstępu między tickami XAUUSD — nie gubimy rozdzielczości.
        if ticki.is_empty() {
            std::thread::sleep(Duration::from_millis(20));
        }

        if !broker.inner.transport().is_connected() {
            break "Sidecar MT5 rozłączył się.".to_string();
        }
    };

    // ---------- co przeżywa to wyjście ----------
    // OSTATNI ZRZUT. Bez tego utrata sidecara gubiłaby koszyki powstałe od
    // ostatniego zapisu — czyli dokładnie te, których wznowienie najbardziej
    // potrzebuje.
    let saved=zapisz_zrzut(st, &silniki, &toz, symbol, &mut zrzut_ostatni, broker.inner.transport().config().follow_terminal_account, broker.inner.transport().config().magic, szczyt_equity, &diagnoza_petli);
    trwale.follow_persist_failed=!saved;
    if let Some(d) = dziennik.as_mut() {
        for s in silniki.lista.iter_mut() {
            let mut evs = s.engine.drain_journal();
            let _ = d.write(&mut evs, conduit_server::now_ms());
        }
    }
    zapamietaj_silniki(trwale, &mut silniki, szczyt_equity, &diagnoza_petli);
    // Wstrzymane sygnały MANUAL wracają do pamięci przeżywającej rekonekt —
    // patrz komentarz przy `mem::take` na górze tej funkcji.
    trwale.czekajace = czekajace;

    powod
}

fn silniki_zamrozone(
    stare: &[(String, String, u32, conduit_core::Settings)],
    koszyki: &[Basket],
    nowe: &routing::Silniki,
    saldo: f64,
    credit: f64,
    engine_run_prefix: &str,
    pulapy: &conduit_core::formaty::PulapyGlobalne,
) -> (Vec<routing::Silnik>, Vec<(String, u32)>) {
    let sloty_z_koszykami: std::collections::BTreeSet<u32> = koszyki
        .iter()
        .map(|b| conduit_core::wielosilnik::slot_koszyka(b.id))
        .collect();
    let mut zamrozone = Vec::new();
    let mut kolizje = Vec::new();
    for (format, preset, slot, cfg) in stare {
        let ma_koszyki = sloty_z_koszykami.contains(slot);
        let jest_w_nowym = nowe.indeks_formatu(format).is_some();
        if !ma_koszyki || jest_w_nowym {
            continue;
        }
        if nowe.lista.iter().any(|s| s.slot == *slot)
            || zamrozone.iter().any(|s: &routing::Silnik| s.slot == *slot)
        {
            kolizje.push((format.clone(), *slot));
            continue;
        }
        let mut engine = Engine::new(cfg.clone(), saldo);
        engine.przypisz_slot(*slot);
        engine.pulapy = pulapy.clone();
        engine.set_run_id(format!("{engine_run_prefix}/slot-{slot}"));
        engine.stats.credit = credit;
        zamrozone.push(routing::Silnik {
            powod: "zamrozona".into(),
            format: format.clone(),
            preset: format!("{preset} (zamrożony)"),
            slot: *slot,
            zapasowy: false,
            tylko_zarzadzanie: true,
            // zamrożony gra KOPIĄ starej konfiguracji z pamięci, a nie
            // świeżym plikiem — panel ma o tym mówić wprost
            z_pliku: false,
            engine,
        });
    }
    (zamrozone, kolizje)
}

// ============================================================
//  KLASY ZATRZYMANIA W PĘTLI HANDLOWEJ
// ============================================================

fn zloz_per_noga(czesci: &[(String, String)], ile_nog: usize) -> String {
    let Some((_, pierwszy)) = czesci.first() else {
        return String::new();
    };
    let wszystkie_te_same = czesci.iter().all(|(_, r)| r == pierwszy);
    if ile_nog <= 1 || (wszystkie_te_same && czesci.len() == ile_nog) {
        return pierwszy.clone();
    }
    czesci
        .iter()
        .map(|(f, r)| format!("{f}: {r}"))
        .collect::<Vec<_>>()
        .join(ui::HALT_SEP)
}

/// Rozbija zatrzymanie silników na DWIE KLASY — patrz [`ui::KlasaHaltu`].
///
/// `Engine::halted` ma jedno pole na powód i nie wie nic o klasach; wiedzę
/// o tym, co jest diagnozą, ma wyłącznie ta pętla, bo to ona diagnozę nałożyła
/// (bramka startowa wyżej). Stąd reguła: co równa się `diagnoza` albo zaczyna
/// się od niej, jest klasy DIAGNOZA; RESZTA pochodzi od strażnika
/// (`Engine::check_guards`) albo z pamięci przeniesionej przez rekonekt, czyli
/// jest klasy RYZYKO.
///
/// Wołane z `diagnoza = ""` oddaje w drugim polu dokładnie to, co
/// [`routing::Silniki::halted`] — służy wtedy za odczyt blokad z silników.
fn rozbij_klasy_zatrzymania(silniki: &routing::Silniki, diagnoza: &str) -> (String, String) {
    let mut byla_diagnoza = false;
    let mut ryzyka: Vec<(String, String)> = Vec::new();
    for s in silniki.lista.iter() {
        let Some(r) = s.engine.halted.as_deref() else {
            continue;
        };
        let mut reszta = r;
        if !diagnoza.is_empty() {
            if r == diagnoza {
                byla_diagnoza = true;
                reszta = "";
            } else if let Some(x) = r
                .strip_prefix(diagnoza)
                .and_then(|x| x.strip_prefix(ui::HALT_SEP))
            {
                byla_diagnoza = true;
                reszta = x;
            }
        }
        if !reszta.is_empty() {
            ryzyka.push((s.format.clone(), reszta.to_string()));
        }
    }
    let d = if byla_diagnoza {
        diagnoza.to_string()
    } else {
        String::new()
    };
    (d, zloz_per_noga(&ryzyka, silniki.lista.len()))
}

fn zdejmij_diagnoze_z_silnikow(silniki: &mut routing::Silniki, diagnoza: &str) {
    for s in silniki.lista.iter_mut() {
        let Some(r) = s.engine.halted.take() else {
            continue;
        };
        let reszta = if r == diagnoza {
            String::new()
        } else {
            match r
                .strip_prefix(diagnoza)
                .and_then(|x| x.strip_prefix(ui::HALT_SEP))
            {
                Some(x) => x.to_string(),
                None => r,
            }
        };
        if !reszta.is_empty() {
            s.engine.halted = Some(reszta);
        }
    }
}

fn przenies_pamiec(
    nowe: &mut routing::Silniki,
    trwale: &mut Trwale,
    saldo: f64,
    credit: f64,
) -> Vec<(String, String)> {
    let mut blokady = Vec::new();
    for s in nowe.lista.iter_mut() {
        let Some(t) = trwale.silniki.get_mut(&s.format) else {
            continue;
        };
        s.engine.restore_pending_source_memory(&t.pending_sources);
        let Some(stats) = t.stats.take() else {
            continue;
        };
        s.engine.stats = stats;
        s.engine.stats.balance = saldo;
        s.engine.stats.credit = credit;
        s.engine.closed_today = std::mem::take(&mut t.closed_today);
        s.engine.risk_override = t.risk_override;
        s.engine.restore_stopped_trading_day(t.stopped_trading_day);
        if let Some(r) = t.halted.take() {
            blokady.push((s.format.clone(), r.clone()));
            s.engine.halted = Some(match s.engine.halted.take() {
                Some(d) if !d.is_empty() && d != r => format!("{d}{}{r}", ui::HALT_SEP),
                _ => r,
            });
        }
    }
    blokady
}

/// Stage A: always AFTER broker-backed basket adoption. This shared helper is
/// exercised by the actual recovery differential, not a hand-copied model.
fn restore_strategy_memory<B:Broker>(silniki:&mut routing::Silniki,trwale:&Trwale,
    broker:&B,origin:ContinuationOrigin)->Vec<(String,ContinuationImportReport)> {
    let mut reports=Vec::new();
    for slot in silniki.lista.iter_mut().filter(|s|s.engine.cfg.restore_strategy_continuation) {
        let snapshot=trwale.silniki.get(&slot.format).and_then(|m|m.continuation.as_ref());
        let report=slot.engine.import_strategy_continuation(broker,&slot.format,snapshot,origin);
        reports.push((slot.format.clone(),report));
    }
    let lost_owner=trwale.silniki.iter().find(|(owner,m)|m.continuation.is_some()
        && !silniki.lista.iter().any(|s|&s.format==*owner));
    let account_reason=lost_owner.map(|(owner,_)|format!("stored continuation owner {owner} is absent from the restored chain"))
        .or_else(||reports.iter().find_map(|(_,r)|r.review.as_ref()
            .filter(|r|r.scope==ContinuationReviewScope::Account).map(|r|r.reason.clone())));
    if let Some(reason)=account_reason {
        for slot in silniki.lista.iter_mut().filter(|s|s.engine.cfg.restore_strategy_continuation) {
            slot.engine.hold_strategy_continuation(ContinuationReviewScope::Account,reason.clone());
        }
        for (_,r) in &mut reports {r.review=Some(conduit_core::engine::ContinuationReview{
            scope:ContinuationReviewScope::Account,reason:reason.clone()});}
    }
    reports
}

/// None returned by the legacy JSON parser is NOT proof of a fresh account.
fn live_continuation_origin(st:&StateHandle,broker:&Recording,
    toz:&conduit_mt5::proto::AccountIdent,symbol:&str,trwale:&Trwale)->ContinuationOrigin {
    if trwale.bylo_polaczenie {return ContinuationOrigin::Memory;}
    let magic=broker.inner.transport().config().magic;
    let follow=broker.inner.transport().config().follow_terminal_account;
    let baskets=if follow {wznowienie::sciezka_scoped(&st.workspace,toz.login,&toz.server,
        toz.trade_mode as i32,magic,symbol)} else {wznowienie::sciezka(&st.workspace)};
    let risk=if follow {follow_risk_path(st,toz,magic,symbol)} else {baskets.with_file_name("risk_state.json")};
    let no_files=matches!(baskets.try_exists(),Ok(false)) && matches!(risk.try_exists(),Ok(false));
    if no_files && trwale.silniki.is_empty() && trwale.koszyki.is_empty()
        && broker.positions().is_empty() && broker.pendings().is_empty() {
        ContinuationOrigin::Fresh
    } else {ContinuationOrigin::UnverifiedDisk}
}

fn log_continuation_reports(st:&StateHandle,reports:&[(String,ContinuationImportReport)]) {
    for (owner,r) in reports {
        if let Some(review)=&r.review {
            st.log("mt5","error",format!("Kontynuacja strategii {owner}: REVIEW"),
                format!("Zakres {:?}: {}. Nowe ryzyko wstrzymane; ochronne wyjścia i SL pozostają czynne. Etap A nie jest atomowym checkpointem dysku/receiptów.",review.scope,review.reason));
        } else {
            st.log("mt5","info",format!("Kontynuacja strategii {owner}: odtworzono etap A"),
                format!("Zamiary SL/TP: {}, terminy wyjść: {}. To nie certyfikuje pełnej pamięci strategii ani trwałego ACK receiptów.",r.imported_stops,r.imported_exits));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn przebuduj_lancuch(
    st: &StateHandle,
    silniki: &mut routing::Silniki,
    broker: &Recording,
    toz: &conduit_mt5::proto::AccountIdent,
    symbol: &str,
    info: &conduit_mt5::proto::SymbolInfo,
    trwale: &mut Trwale,
    szczyt_equity: f64,
    // Zdanie klasy DIAGNOZA bramki startowej — przebudowa łańcucha potrafi je
    // ZMIENIĆ (nowy skład, nowy rozjazd bezpieczników) i publikacja stanu musi
    // dostać wersję aktualną, inaczej świeża diagnoza zostałaby zaksięgowana
    // jako RYZYKO i — jako jedyna z dwóch klas — przeżyłaby restart.
    diagnoza: &mut String,
    engine_run_prefix: &str,
    powod: &str,
) -> String {
    let stary_lancuch = silniki.lancuch.clone();

    // Validate actual replacement before moving any old basket/risk memory.
    let mut core = st.read(|s| {
        let mut c = live_core_from_ui(&s.settings);
        conduit_server::settings_map::apply_lot(&mut c, &s.lot);
        c
    });
    core.stops_level = info.stops_level_price();
    let acc = broker.account();
    let mut nowe = zbuduj_silniki(st, &core, acc.balance);
    if nowe.lista.iter().any(|s| live_sr_v2_requested(&s.engine.cfg)) {
        note_live_sr_hold(st);
        if !diagnoza.contains(LIVE_SR_V2_HOLD) {
            if !diagnoza.is_empty() { diagnoza.push_str(ui::HALT_SEP); }
            diagnoza.push_str(LIVE_SR_V2_HOLD);
        }
        return String::new(); // next checkpoint writes unchanged state; no adoption/reset
    }

    // ---------- 1. stara konfiguracja per format (do zamrożenia) ----------
    let stare: Vec<(String, String, u32, conduit_core::Settings)> = silniki
        .lista
        .iter()
        .map(|s| {
            (
                s.format.clone(),
                s.preset.clone(),
                s.slot,
                s.engine.cfg.clone(),
            )
        })
        .collect();

    // ---------- 2. pamięć do Trwale (koszyki wyjęte z silników) ----------
    zapamietaj_silniki(trwale, silniki, szczyt_equity, diagnoza);

    // ---------- 3. nowe silniki z NOWEGO aktywnego łańcucha ----------
    for s in nowe.lista.iter_mut() {
        s.engine
            .set_run_id(format!("{engine_run_prefix}/slot-{}", s.slot));
        s.engine.stats.credit = acc.credit;
    }

    // ---------- 3a. KOSZYKI SLOTU 0 WRACAJĄ DO SWOJEGO FORMATU ----------
    //
    // Numer koszyka ze slotu 0 (B1, B2… — ścieżka jednosilnikowa) NIE niesie
    // formatu, a przydział przy adopcji idzie wyłącznie po slocie (musi —
    // widok brokera używa tej samej reguły). Slot 0 przejmuje silnik
    // ZAPASOWY, którego `zbuduj` wybiera deterministycznie (ATFX albo
    // pierwszy z listy) — czyli przy przejściu ZENONLY3 → SENTINEL-0 koszyki
    // ZEN-a trafiłyby pod nogę SYNERGY. My jednak WIEMY, czyj był slot 0:
    // to format jedynego silnika starego zespołu. Jeśli nowy łańcuch ma ten
    // format, ZAPASOWYM zostaje właśnie on — i koszyki wracają do swojego
    // formatu bez ruszania numeracji (id w komentarzach MT5 są nietykalne).
    if let Some((format0, _, _, _)) = stare.iter().find(|(_, _, slot, _)| *slot == 0) {
        if let Some(i) = nowe.indeks_formatu(format0) {
            if !nowe.lista[i].zapasowy {
                for s in nowe.lista.iter_mut() {
                    s.zapasowy = false;
                }
                nowe.lista[i].zapasowy = true;
                st.log(
                    "settings",
                    "info",
                    format!("Silnik zapasowy: {} (właściciel slotu 0)", format0),
                    "Koszyki z numeracją sprzed trybu wielosilnikowego (B1, B2…) \
                     należą do formatu, który je otworzył — zapasowym zostaje jego \
                     silnik, żeby adopcja nie oddała ich cudzej nodze."
                        .to_string(),
                );
            }
        }
    }

    // ---------- 4. silniki ZAMROŻONE dla formatów z żywymi koszykami ----------
    let pulapy_nowe = nowe.pulapy.clone();
    let (zamrozone, kolizje) = silniki_zamrozone(
        &stare,
        &trwale.koszyki,
        &nowe,
        acc.balance,
        acc.credit,
        engine_run_prefix,
        &pulapy_nowe,
    );
    for (format, slot) in &kolizje {
        // Skrajny przypadek: nowy łańcuch przydzielił slot starego formatu
        // INNEMU formatowi (kolizja skrótów nazw). Koszyki pójdą po slocie
        // do nowego silnika — mówimy o tym głośno zamiast dublować slot,
        // bo dwa silniki na jednym slocie to podwójne zarządzanie.
        st.log(
            "settings",
            "warn",
            format!("Slot {slot} formatu {format} zajęty w nowym łańcuchu"),
            "Koszyki tego slotu przejmie silnik nowego łańcucha o tym samym slocie. \
             Sprawdź przydział w dzienniku adopcji poniżej."
                .to_string(),
        );
    }
    for z in zamrozone {
        st.log(
            "settings",
            "info",
            format!("Format {} ZAMROŻONY (tylko zarządzanie)", z.format),
            format!(
                "Nowy łańcuch nie ma nogi dla formatu {}, a jego koszyki wciąż żyją \
                 na rachunku. Prowadzi je dalej STARA konfiguracja ({}); nowych \
                 sygnałów ten format nie bierze. Silnik zniknie przy kolejnej \
                 przebudowie, gdy koszyki się domkną.",
                z.format, z.preset
            ),
        );
        nowe.lista.push(z);
    }

    // ---------- 4a. EA-21: rozjazd bezpieczników w NOWYM składzie ----------
    //
    // Zmiana łańcucha (ręczna albo szczeblem drabinki) potrafi wstawić nogę,
    // której preset deklaruje ochronę, jakiej dokument rachunku nie daje —
    // dokładnie tak samo jak przy starcie. Sprawdzenie musi więc iść też tędy,
    // inaczej bramka startowa broniłaby wyłącznie pierwszej minuty pracy bota.
    // PRZED `przenies_pamiec`, żeby blokada przeniesiona z pamięci (strażnik
    // obsunięcia) miała ostatnie słowo — jest starsza i ważniejsza.
    //
    // KLASA: to jest DIAGNOZA nowego składu i ZASTĘPUJE diagnozę bramki
    // startowej — tamta opisywała skład, którego już nie ma. Bez tej linijki
    // publikacja zaksięgowałaby świeży rozjazd jako RYZYKO, czyli jedyną
    // klasę, która przeżywa restart, i wróciłaby stara pętla w nowym miejscu.
    *diagnoza = String::new();
    if let Some(r) = sprawdz_rozjazd_nog(st, &nowe, &core) {
        for s in nowe.lista.iter_mut() {
            s.engine.halted = Some(r.clone());
        }
        *diagnoza = r;
    }

    // ---------- 5. pamięć per format wraca (stats, blokady, wynik dnia) ----------
    przenies_pamiec(&mut nowe, trwale, acc.balance, acc.credit);

    rozgrzej_historie(st, broker, &mut nowe, symbol);
    let zrzut_json = wznow_koszyki(st, &mut nowe, broker, toz, symbol, trwale);
    let continuation_reports=restore_strategy_memory(&mut nowe,trwale,broker,ContinuationOrigin::Memory);
    log_continuation_reports(st,&continuation_reports);

    let ile_koszykow = nowe.koszyki().len();
    let opis_silnikow: Vec<String> = nowe
        .lista
        .iter()
        .map(|s| {
            format!(
                "{} → {}{}",
                s.format,
                s.preset,
                if s.tylko_zarzadzanie {
                    " [zamrożony]"
                } else {
                    ""
                }
            )
        })
        .collect();
    let tresc = format!(
        "{powod}\n\nŁańcuch {stary_lancuch} → {} · koszyki przejęte: {ile_koszykow}\n{}\n\n\
         Adoptowane koszyki są od tej chwili zarządzane ustawieniami NOWEJ nogi \
         swojego formatu. Kotwice PnL dnia i sesji NIE zostały ruszone — to to \
         samo konto.",
        nowe.lancuch,
        opis_silnikow.join("\n")
    );
    st.log(
        "settings",
        "success",
        format!("ŁAŃCUCH PRZEŁĄCZONY: {}", nowe.lancuch),
        tresc.clone(),
    );
    st.notify(
        MailCategory::Lifecycle,
        &format!("Łańcuch przełączony: {} → {}", stary_lancuch, nowe.lancuch),
        &tresc,
    );

    *silniki = nowe;
    zrzut_json
}

fn drabinka_tick(st: &StateHandle, broker: &Recording, ostatnie: &mut Instant) {
    if ostatnie.elapsed() < Duration::from_secs(60) {
        return;
    }
    *ostatnie = Instant::now();

    // `s.settings` to DOKUMENT PANELU (`serde_json::Value`), a nie
    // `conduit_core::Settings` — pola czyta się po kluczu. Domyślki muszą być
    // te same co w `Settings::default()` (bonus wyłączony, kwota ręczna 0),
    // inaczej brak klucza w dokumencie znaczyłby coś innego niż jego brak
    // w silniku.
    let (drabinka, aktywny, credit_cfg) = st.read(|s| {
        (
            s.drabinka.clone(),
            s.lancuchy.aktywny.clone(),
            conduit_server::settings_map::core_from_ui(&s.settings),
        )
    });
    // The broker's Balance excludes Credit when credit_balance_separate is ON.
    // The ladder uses owned balance, independently of the sizing lot_base selection.
    let konto = broker.account();
    let balance = if credit_cfg.credit_balance_separate {
        credit_cfg.saldo_wlasne(konto.balance, konto.credit)
    } else {
        // Preserve the legacy negative-balance ladder boundary when the new axis is OFF.
        konto.balance - credit_cfg.kredyt_skuteczny_z(konto.credit)
    };
    let Some(szczebel) = drabinka.wybierz(balance) else {
        return;
    };
    let (prog, lancuch) = (szczebel.prog_balance, szczebel.lancuch.clone());

    // Szczebel się nie zmienił → co najwyżej dopisz punkt odniesienia
    // histerezy po restarcie (backup mógł nie zdążyć).
    if lancuch == aktywny {
        if (drabinka.biezacy_prog - prog).abs() > 1e-9 {
            st.update(Sections::one(Section::Settings), |s| {
                s.drabinka.biezacy_prog = prog
            });
        }
        return;
    }

    match conduit_server::commands::apply(
        st,
        &conduit_server::proto::Command::SetAktywnyLancuch {
            nazwa: lancuch.clone(),
        },
    ) {
        Ok(()) => {
            st.update(Sections::one(Section::Settings), |s| {
                s.drabinka.biezacy_prog = prog;
                s.drabinka.ostatnia_zmiana_ts = conduit_server::now_ms();
            });
            let kierunek = if prog >= drabinka.biezacy_prog {
                "≥"
            } else {
                "<"
            };
            let tresc = format!(
                "DRABINKA FFS-1C: środki własne {balance:.2} {kierunek} {prog:.0} → łańcuch {lancuch}. \
                 Przebudowa silników i adopcja koszyków nastąpi w tej samej pętli \
                 (wpis „ŁAŃCUCH PRZEŁĄCZONY” poniżej poda liczbę przejętych)."
            );
            st.log(
                "plan",
                "success",
                format!("DRABINKA: → {lancuch}"),
                tresc.clone(),
            );
            st.notify(
                MailCategory::Lifecycle,
                &format!("DRABINKA: {lancuch}"),
                &tresc,
            );
        }
        Err(e) => {
            st.log(
                "plan",
                "error",
                "DRABINKA: błąd przełączenia",
                format!(
                    "Nie udało się ustawić łańcucha „{lancuch}” przy balance \
                     {balance:.2}: {e}. Bot gra dalej łańcuchem „{aktywny}”."
                ),
            );
        }
    }
}

/// Przenosi CAŁĄ pamięć silników do [`Trwale`] — wspólna dla wyjścia z pętli
/// (rekonekt/zamknięcie) i dla przebudowy łańcucha w locie. Jedna funkcja,
/// bo to jest dokładnie ten zestaw, którego zapomnienie robi sieroty:
/// koszyki, statystyki per format, blokady, wynik dnia, szczyt equity.
fn zapamietaj_silniki(
    trwale: &mut Trwale,
    silniki: &mut routing::Silniki,
    szczyt_equity: f64,
    // Zdanie klasy DIAGNOZA obowiązujące w tej chwili — patrz niżej.
    diagnoza: &str,
) {
    trwale.silniki.clear();
    trwale.koszyki.clear();
    trwale.next_basket_id = silniki.next_basket_id();
    for s in silniki.lista.iter_mut() {
        trwale.koszyki.extend(std::mem::take(&mut s.engine.baskets));
        let halted = s.engine.halted.as_deref().and_then(|r| {
            let reszta = if diagnoza.is_empty() || r == diagnoza {
                if diagnoza.is_empty() {
                    r
                } else {
                    ""
                }
            } else {
                r.strip_prefix(diagnoza)
                    .and_then(|x| x.strip_prefix(ui::HALT_SEP))
                    .unwrap_or(r)
            };
            (!reszta.is_empty()).then(|| reszta.to_string())
        });
        trwale.silniki.insert(
            s.format.clone(),
            TrwalySilnik {
                stats: Some(s.engine.stats.clone()),
                halted,
                risk_override: s.engine.risk_override,
                closed_today: std::mem::take(&mut s.engine.closed_today),
                stopped_trading_day: s.engine.stopped_trading_day(),
                pending_sources: s.engine.export_pending_source_memory(),
                continuation: s.engine.export_strategy_continuation(),
            },
        );
    }
    trwale.koszyki.sort_by_key(|b| b.id);
    trwale.szczyt_equity = szczyt_equity;
    trwale.bylo_polaczenie = true;
}

/// Kieruje wiadomość do silnika przypisanego jej formatowi.
///
/// # Cisza jest tu niedopuszczalna
///
/// Wiadomość, która nie trafiła do żadnego silnika, MUSI zostawić ślad —
/// inaczej użytkownik widzi w panelu sygnał, po którym nic się nie stało,
/// i nie ma jak się dowiedzieć dlaczego. Dlatego każdy powód ma własny kod
/// ([`routing::BrakTrasy::kod`]), własne zdanie wyjaśniające i osobny licznik.
/// Tlumaczy kod odrzucenia silnika na zdanie dla czlowieka.
///
/// Dziennik oglada uzytkownik, nie parser — „SessionClosed" nic mu nie mowi,
/// „poza godzinami handlu" mowi wszystko.
fn powod_po_polsku(kod: &str) -> String {
    match kod {
        "SessionClosed" | "session_closed" => "poza godzinami handlu (filtr sesji)".into(),
        "RegimeFilter" | "regime_filter" => "filtr rezimu nie wpuscil".into(),
        "TrendFilter" | "trend_filter" => "filtr trendu nie wpuscil".into(),
        "SideFilter" | "side_filter" => "kierunek odrzucony przez filtr strony".into(),
        "StreakPause" | "streak_pause" => "pauza po serii strat".into(),
        "SlHitBrake" | "sl_hit_brake" | "SlHitPause" | "slhit" | "slhit_pause" => {
            "hamulec SL-HIT: kanal oglosil w tej dobie swoje stopy".into()
        }
        "MaxOpenPositions" | "max_open_positions" => "pulap otwartych pozycji".into(),
        "MaxOpenBaskets" | "max_open_baskets" => "pulap otwartych koszykow".into(),
        "SlBreached" | "sl_breached" => "stop juz przekroczony w chwili sygnalu".into(),
        "DayGate" | "day_gate" => "bramka dobowa".into(),
        "ExpoCap" | "expo_cap" => "pulap ekspozycji".into(),
        inny => format!("bramka wejscia: {inny}"),
    }
}

/// Kieruje wiadomosc do wlasciwego silnika.
///
/// Zwraca `Some(powod)`, gdy silnik ODRZUCIL sygnal — wolajacy ma wtedy napisac
/// w dzienniku, ze go POMINAL, zamiast twierdzic, ze go wykonal.
enum WynikSygnalu {
    Przekazany,
    Pominiety(String),
    Odroczony(conduit_core::engine::DeferredEntryStatus),
}

fn skieruj(
    st: &StateHandle,
    silniki: &mut routing::Silniki,
    broker: &mut Recording,
    im: &IncomingMessage,
    received_utc: i64,
) -> WynikSygnalu {
    let format = format_zrodla(st, &im.source);
    match silniki.trasa(&im.source, format) {
        Ok(i) => {
            // Kierunek sygnału trzeba znać PRZED bramką, żeby pułap
            // „nie otwieraj przeciwnie do innego formatu" miał czego pilnować.
            // Parsujemy tu drugi raz (silnik zrobi to sam) — to jest kilka
            // mikrosekund na wiadomość i cena za to, żeby pułap nie musiał
            // wchodzić w środek `on_message`.
            let strona = conduit_core::parser::parse(&im.text)
                .into_iter()
                .find_map(|s| match s {
                    conduit_core::parser::Signal::Entry(e) => Some(e.side),
                    conduit_core::parser::Signal::MarketOpen { side } => Some(side),
                    _ => None,
                });
            silniki.przelicz_obce(broker, strona);
            // MIGAWKA LICZNIKOW ODRZUCEN przed i po. Klucz, ktory urosl, jest
            // powodem pominiecia — czytamy to, co silnik i tak liczy, zamiast
            // zakladac cokolwiek o wyniku `on_message`.
            let przed: std::collections::BTreeMap<String, u64> =
                silniki.z_widokiem(i, broker, |e, _| e.odrzuty.clone());
            silniki.z_widokiem(i, broker, |e, w| e.on_message_received(w, im, received_utc));
            let deferred = silniki.z_widokiem(i, broker, |e, _| {
                e.deferred_entry_status(&im.source, im.edit_of.unwrap_or(im.msg_id))
                    .or_else(|| im.reply_to.and_then(|id| e.deferred_entry_status(&im.source, id)))
            });
            if let Some(status) = deferred {
                use conduit_core::engine::DeferredEntryState;
                match status.state {
                    DeferredEntryState::Waiting | DeferredEntryState::NoEntry => return WynikSygnalu::Odroczony(status),
                    DeferredEntryState::Executed => {},
                    _ => return WynikSygnalu::Pominiety(format!("{:?}: {}", status.state, status.reason)),
                }
            }
            let po: std::collections::BTreeMap<String, u64> =
                silniki.z_widokiem(i, broker, |e, _| e.odrzuty.clone());
            return po
                .iter()
                .find(|(k, v)| **v > przed.get(*k).copied().unwrap_or(0))
                .map(|(k, _)| WynikSygnalu::Pominiety(powod_po_polsku(k)))
                .unwrap_or(WynikSygnalu::Przekazany);
        }
        Err(powod) => {
            // Licznik rośnie NIEZALEŻNIE od poziomu dziennika — dziennik
            // można wyciszyć, licznik nie. Bez tego „ile sygnałów przepadło
            // na routingu" byłoby niemierzalne.
            let s = silniki.glowny_mut();
            *s.engine.odrzuty.entry(powod.kod().to_string()).or_insert(0) += 1;
            st.log(
                "telegram",
                "warn",
                format!("Sygnał POMINIĘTY: {}", powod.kod()),
                format!("{}\n\nTreść wiadomości:\n{}", powod.opis(), im.text),
            );
        }
    }
    // Gałąź błędu routingu zameldowała się sama, a brak trasy to nie jest
    // odrzucenie przez bramkę wejścia — wołający nie ma tu nic do dopisania.
    WynikSygnalu::Przekazany
}

// ============================================================
//  DOZÓR NAD TELEGRAMEM
// ============================================================

/// Pilnuje, czy kanał sygnałowy NAPRAWDĘ żyje.
///
/// # Awaria, której nikt nie widzi
///
/// `crates/telegram` pinguje teraz Telegrama co 4 minuty (`service.rs::ping`),
/// bo martwe gniazdo MTProto nie zgłasza błędu — `next_message()` po prostu
/// nigdy nic nie oddaje. Ale sam ping to za mało: ktoś musi z jego wyniku
/// zrobić wniosek i powiedzieć o nim człowiekowi. To jest to miejsce.
///
/// # Dlaczego nie alarmujemy po samej ciszy
///
/// Kanał sygnałowy potrafi milczeć pół nocy i to jest normalne. Alarmowanie
/// po samej ciszy dawałoby maila po każdej spokojnej nocy, a alarm, który
/// przychodzi codziennie, przestaje być alarmem. Dlatego warunek jest
/// KONIUNKCJĄ: cisza NA KANALE **oraz** brak potwierdzenia, że gniazdo żyje.
/// Cisza przy sprawnym keepalive to spokojna noc; cisza przy martwym
/// keepalive to awaria, która kosztuje całą sesję.
/// Czy cisza na kanałach jest AWARIĄ, czy spokojną nocą?
///
/// Wydzielone, żeby dało się to sprawdzić testem bez stawiania Telegrama.
/// Warunek jest KONIUNKCJĄ i to jest w tym cała treść: alarm należy się
/// wyłącznie wtedy, gdy do ciszy dokłada się brak potwierdzenia, że gniazdo
/// żyje. Sama cisza to normalna noc na kanale sygnałowym.
fn alarm_telegram_nalezy_sie(cisza_min: f64, bledy_pingu: u32, rynek_czynny: bool) -> bool {
    bledy_pingu >= PINGOW_DO_ALARMU || (rynek_czynny && cisza_min >= CISZA_TELEGRAM_MIN)
}

fn dozor_telegrama(
    st: StateHandle,
    tg: Arc<conduit_telegram::TelegramService>,
    stop: Arc<AtomicBool>,
) {
    let _ = std::thread::Builder::new().name("conduit-tg-dozor".into()).spawn(move || {
        let start = conduit_server::now_ms();
        let mut zgloszone = false;
        loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(TELEGRAM_SPRAWDZAJ_CO);
            if stop.load(Ordering::Relaxed) {
                return;
            }

            let z = tg.zdrowie();
            let teraz = conduit_server::now_ms();

            // ---------- FAKTY DO PANELU ----------
            // Wpisujemy MOMENTY (ms epoki), nie „minut temu". Wartość względna
            // zapisana w migawce zestarzałaby się w `backup_memory` i po
            // restarcie plakietka pokazywałaby „2 min temu" dla pingu sprzed
            // ośmiu godzin — czyli kłamałaby najbardziej dokładnie tam, gdzie
            // ma być wiarygodna. Panel przelicza na „min temu" przy rysowaniu.
            st.update(Sections::one(Section::Connection), |s| {
                s.connection.telegram_last_message_ms =
                    if z.ostatnia_wiadomosc_ms > 0 { Some(z.ostatnia_wiadomosc_ms) } else { None };
                s.connection.telegram_last_ping_ok_ms =
                    if z.ostatni_ping_ok_ms > 0 { Some(z.ostatni_ping_ok_ms) } else { None };
                s.connection.telegram_ping_failures = z.bledy_pingu;
                s.connection.telegram_reconnects = z.wznowienia;
            });

            // Dopóki nic nie przyszło, punktem odniesienia jest START PROGRAMU.
            // Inaczej świeżo uruchomiony bot z zerowym znacznikiem wyglądałby
            // jak bot milczący od 1970 roku.
            let od_wiadomosci = teraz - if z.ostatnia_wiadomosc_ms > 0 { z.ostatnia_wiadomosc_ms } else { start };
            let cisza_min = od_wiadomosci as f64 / 60_000.0;
            let ping_zyje = z.bledy_pingu == 0;

            let zle = alarm_telegram_nalezy_sie(
                cisza_min,
                z.bledy_pingu,
                rynek_powinien_dzialac_z(przerwa_dobowa(&st), offset_serwera_h(&st)),
            );
            if !zle {
                if zgloszone && ping_zyje {
                    zgloszone = false;
                    st.log(
                        "telegram",
                        "success",
                        "Telegram znowu odpowiada",
                        format!(
                            "Keepalive wrócił poprawnie. Cisza na kanałach trwa {cisza_min:.0} min, \
                             ale połączenie jest potwierdzone."
                        ),
                    );
                }
                continue;
            }
            if zgloszone {
                continue;
            }
            zgloszone = true;
            st.notify(
                conduit_server::mailer::MailCategory::Mt5Connection,
                "TELEGRAM MILCZY, a połączenia nie da się potwierdzić",
                &format!(
                    "Od {cisza_min:.0} min nie przyszła ŻADNA wiadomość z obserwowanych kanałów, \
                     a keepalive nie wraca poprawnie ({} nieudanych prób z rzędu).\n\n\
                     To jest ta awaria, w której panel pokazuje „Telegram: połączony”, a sygnały \
                     nie przychodzą — gniazdo MTProto potrafi umrzeć PO CICHU, bez błędu.\n\n\
                     Bot próbuje sam: po trzech nieudanych pingach zamyka sesję i loguje się \
                     ponownie z zapisanej sesji. Do tej pory usługa wznawiała się {} razy \
                     od uruchomienia programu.\n\n\
                     Otwarte pozycje i zlecenia NIE SĄ tym dotknięte — nimi zarządza część MT5, \
                     która działa niezależnie. Ryzyko polega na tym, że bot nie zobaczy nowych \
                     sygnałów ANI komunikatów zarządzających (TP HIT, RISK FREE, CLOSE) \
                     dla koszyków, które w tej chwili żyją.",
                    z.bledy_pingu, z.wznowienia
                ),
            );
        }
    });
}

// ============================================================
//  WZNOWIENIE STANU PO RESTARCIE
// ============================================================

/// Oddaje silnikowi koszyki sprzed restartu i mówi w panelu, co odzyskał.
///
/// Zwraca serializację zapisanego zrzutu, żeby pętla mogła zapisywać plik
/// tylko wtedy, gdy coś się naprawdę zmieniło.
fn wznow_koszyki(
    st: &StateHandle,
    silniki: &mut routing::Silniki,
    broker: &Recording,
    toz: &conduit_mt5::proto::AccountIdent,
    symbol: &str,
    trwale: &Trwale,
) -> String {
    let magic = st
        .read(|s| s.settings.get("mt5_magic").and_then(|v| v.as_f64()))
        .unwrap_or(770_077.0) as i64;

    // PAMIĘĆ PRZED DYSKIEM. Przy ponownym podłączeniu (czkawka terminala)
    // koszyki w pamięci są z definicji świeższe niż zrzut, który zapisuje się
    // co 5 s. Przy starcie procesu pamięć jest pusta i wtedy liczy się dysk.
    let (zrzut, skad) = if trwale.bylo_polaczenie && !trwale.koszyki.is_empty() {
        (
            Some(wznowienie::Zrzut {
                wersja: wznowienie::WERSJA,
                zapisano: conduit_server::now_ms(),
                login: toz.login,
                magic,
                symbol: symbol.to_string(),
                next_basket_id: trwale.next_basket_id,
                koszyki: trwale.koszyki.clone(),
            }),
            "pamięć procesu (ponowne podłączenie)",
        )
    } else if broker.inner.transport().config().follow_terminal_account {
        (wznowienie::wczytaj_scoped(&st.workspace, toz.login, &toz.server, toz.trade_mode as i32, magic, symbol), "zrzut przypisany do loginu/serwera/typu/symbolu")
    } else {
        (wznowienie::wczytaj(&st.workspace), "plik koszyki.json")
    };
    let wiek = zrzut
        .as_ref()
        .map(|z| conduit_server::now_ms() - z.zapisano);
    let byl_plik = zrzut.is_some();
    let w = wznowienie::odtworz(
        zrzut,
        broker.positions(),
        broker.pendings(),
        toz.login,
        magic,
        symbol,
        &znacznik_komentarza(st),
    );

    let q = broker.quote();
    for p in broker.positions() {
        // Dziennik prowadzi silnik, który tą pozycją zarządza — inaczej
        // po restarcie pozycja formatu Synergy zapisałaby się w dzienniku
        // pod ATFX i każda analiza „ile wejść ma który format" byłaby fałszem.
        let i = silniki.indeks_koszyka(p.basket.unwrap_or(0)).unwrap_or(0);
        silniki.lista[i].engine.journal.note_open(p, &q);
    }

    let nic_do_odzyskania = w.koszyki.is_empty()
        && w.sieroty == 0
        && w.reczne == 0
        && w.wygasle == 0
        && w.zrzut_odrzucony.is_none();
    if nic_do_odzyskania {
        st.log(
            "mt5",
            "info",
            "Start bez otwartych koszyków",
            format!(
                "Na rachunku {} nie ma nic z magic {magic} do przejęcia.",
                toz.login
            ),
        );
        return String::new();
    }

    let ile = w.koszyki.len();
    let numery: Vec<String> = w.koszyki.iter().map(|b| format!("B{}", b.id)).collect();
    // ROZDZIELENIE KOSZYKÓW MIĘDZY SILNIKI. Reguła jest ta sama, którą stosuje
    // widok brokera — inaczej silnik dostałby koszyk, którego pozycji nie widzi,
    // uznał go za pusty i przestał nim zarządzać. Szczegóły: `routing.rs`.
    let przydzial = silniki.rozdaj_koszyki(w.koszyki.clone());

    let tresc = format!(
        "{}\n\nKoszyki przejęte: {}\nPrzydział do silników:\n{}\nŹródło treści: {}\nWiek zrzutu: {}\n\
         Rachunek {} · magic {magic} · {symbol}\n\n\
         Od tej chwili bot znowu nimi zarządza: etapy celów, SL koszyka, RISK FREE \
         i trailing liczą się dalej od miejsca, w którym stanęły.",
        w.opis(),
        if numery.is_empty() { "—".to_string() } else { numery.join(", ") },
        przydzial.opis(),
        if w.zrzut_uzyty {
            skad
        } else if byl_plik {
            "SAME ZLECENIA U BROKERA (zrzut odrzucony)"
        } else {
            "SAME ZLECENIA U BROKERA (brak zrzutu)"
        },
        match wiek {
            Some(ms) if ms >= 0 => conduit_mt5::watchdog::opis_czasu(Duration::from_millis(ms as u64)),
            _ => "brak zrzutu".to_string(),
        },
        toz.login,
    );

    if w.sieroty > 0 || w.zrzut_odrzucony.is_some() {
        st.notify(
            MailCategory::Lifecycle,
            "Wznowienie stanu po restarcie — SĄ POZYCJE BEZ ROZPOZNANIA",
            &format!(
                "{tresc}\n\nPozycji/zleceń z naszym magic, których komentarza NIE DA SIĘ \
                 odczytać: {}. Bot ich nie przypisze do żadnego koszyka, więc nie obejmie \
                 zarządzaniem koszykowym (etapy celów, SL koszyka, RISK FREE).\n\n\
                 Najczęstsza przyczyna: własny komentarz dłuższy niż 29 znaków \
                 (Ustawienia → komentarz pozycji). Sprawdź te pozycje ręcznie w panelu.",
                w.sieroty
            ),
        );
    } else {
        st.log(
            "mt5",
            "success",
            format!("Wznowiono {ile} koszyków po restarcie"),
            tresc,
        );
    }
    // KOSZYK BEZ OPIEKI to awaria warta maila: pozycje zostają na rachunku
    // z żywym ryzykiem i nikt ich nie zamknie.
    if !przydzial.porzucone.is_empty() {
        st.notify(
            MailCategory::Lifecycle,
            "KOSZYKI BEZ OPIEKI po restarcie",
            &format!(
                "Żaden format aktywnego łańcucha nie przyjął tych koszyków: {:?}\n\n\
                 Ich pozycje i zlecenia zostają na rachunku i NIKT nimi nie zarządza — \
                 etapy celów, SL koszyka i RISK FREE nie będą się liczyć. Przypisz \
                 preset do formatu w panelu łańcuchów albo zamknij te koszyki ręcznie.",
                przydzial.porzucone
            ),
        );
    }

    for s in &silniki.lista {
        debug_assert!(
            s.engine.next_basket_id() > s.engine.baskets.iter().map(|b| b.id).max().unwrap_or(0),
            "licznik koszyków musi stać ponad odtworzonymi numerami"
        );
    }

    serde_json::to_string(&silniki.koszyki()).unwrap_or_default()
}

/// Zapisuje zrzut, gdy koszyki się zmieniły. `ostatni` to poprzednia
/// serializacja — porównanie napisów jest tańsze niż zapis pliku.
fn zapisz_zrzut(
    st: &StateHandle,
    silniki: &routing::Silniki,
    toz: &conduit_mt5::proto::AccountIdent,
    symbol: &str,
    ostatni: &mut String,
    follow: bool,
    magic: i64,
    peak_equity: f64,
    diagnosis: &str,
) -> bool {
    if let Err(e)=save_follow_memory(st,silniki,toz,magic,symbol,peak_equity,diagnosis) {
        st.log("mt5","error","Nie zapisano pamięci ryzyka rachunku",format!("{e}; zapis zostanie ponowiony, sesja wstrzymana do odzyskania trwałej ochrony."));
        return false;
    }
    let koszyki = silniki.koszyki();
    let teraz = match serde_json::to_string(&koszyki) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if teraz == *ostatni {
        return true;
    }
    let zapis = if follow {
        wznowienie::zapisz_scoped(&st.workspace, &koszyki, silniki.next_basket_id(), toz.login,
            magic, symbol, conduit_server::now_ms(), &toz.server, toz.trade_mode as i32)
    } else { wznowienie::zapisz(
        &st.workspace,
        &koszyki,
        silniki.next_basket_id(),
        toz.login,
        magic,
        symbol,
        conduit_server::now_ms(),
    ) };
    match zapis {
        Ok(()) => {
            *ostatni = teraz;
            true
        },
        Err(e) => {
            // Cisza zakazana także tutaj: bez tego pliku następny restart
            // odtworzy koszyki bez drabinki celów, a nikt się nie dowie dlaczego.
            tracing::warn!(blad = %e, "nie udało się zapisać zrzutu koszyków");
            st.log(
                "mt5",
                "error",
                "Nie udało się zapisać zrzutu koszyków",
                format!(
                    "{e}\n\nPlik: {}\nSkutek: po restarcie koszyki odtworzą się \
                     wyłącznie z komentarzy zleceń — bez drabinki celów i bez planu siatki.",
                    wznowienie::sciezka(&st.workspace).display()
                ),
            );
            false
        }
    }
}

// ============================================================
//  PRZYPADKI BRZEGOWE TERMINALA
// ============================================================

/// Rzeczy, które terminal potrafi zrobić po cichu.
///
/// Nadzorca (`mt5_guard`) pilnuje, czy PROCES żyje. Most pilnuje, czy da się
/// z nim rozmawiać. Nikt nie pilnował, czy rozmawiamy z WŁAŚCIWYM kontem
/// i czy symbol w ogóle wolno handlować — a to są dwie awarie, które nie dają
/// żadnego błędu, tylko brak wyniku.
fn sprawdz_terminal(
    st: &StateHandle,
    toz: &conduit_mt5::proto::AccountIdent,
    info: &conduit_mt5::proto::SymbolInfo,
) -> Option<String> {
    if st.read(|s| s.settings.get("mt5_follow_terminal_account").and_then(Value::as_bool).unwrap_or(false)) {
        st.log("mt5", "info", "Śledzę konto wybrane w terminalu",
            format!("{} · {} · {}; stare dane logowania nie są wysyłane. Każde polecenie ma blokadę tożsamości rachunku.", toz.login, toz.server, toz.kind()));
        return None;
    }
    let oczekiwany = st
        .read(|s| s.settings.get("mt5_login").and_then(|v| v.as_i64()))
        .filter(|x| *x != 0);
    if let Some(chciany) = oczekiwany {
        if chciany != toz.login {
            st.notify(
                MailCategory::Mt5Connection,
                "🔴 TERMINAL NA INNYM KONCIE — HANDEL ZABLOKOWANY",
                &format!(
                    "W ustawieniach jest rachunek {chciany}, a terminal MetaTrader 5 \
                     jest zalogowany na {} ({} · {} · {}).\n\n\
                     Bot NIE OTWORZY żadnej pozycji, dopóki konta się nie zgodzą. \
                     Pozycjami, które już są na rachunku, dalej zarządza.\n\n\
                     Napraw jedno z dwóch:\n\
                     • przełącz terminal na rachunek {chciany} (pamiętaj: API potrafi \
                       trzymać w tle INNĄ instancję terminala niż ta, którą widzisz — \
                       zamknij WSZYSTKIE terminal64.exe i uruchom bota ponownie), albo\n\
                     • popraw pole „numer rachunku” w Ustawieniach → MetaTrader 5, \
                       jeśli zmiana konta była zamierzona.",
                    toz.login,
                    toz.server,
                    toz.company,
                    toz.kind(),
                ),
            );
            return Some(format!(
                "TERMINAL NA INNYM KONCIE: oczekiwano {chciany}, terminal jest na {} \
                 ({} · {}). Przełącz konto w terminalu albo popraw „numer rachunku” \
                 w Ustawieniach → MetaTrader 5.",
                toz.login, toz.server, toz.company
            ));
        }
    } else {
        st.log(
            "mt5",
            "warn",
            "Nie sprawdzam, na jakim koncie jest terminal",
            format!(
                "Pole „numer rachunku” w Ustawieniach → MetaTrader 5 jest puste, \
                 więc bot przyjmuje KAŻDE konto, na które terminal jest zalogowany \
                 (teraz: {} · {}). Wpisanie numeru zamienia cichą pomyłkę \
                 w czytelny alarm. Panel pokazuje ten stan jako ŻÓŁTE \
                 „konto niezweryfikowane” przy wskaźniku MT5.",
                toz.login, toz.server
            ),
        );
    }

    // ---- 2. symbol wyłączony z handlu ----
    // `SYMBOL_TRADE_MODE`: 0 wyłączony, 1 tylko kupno, 2 tylko sprzedaż,
    // 3 tylko zamykanie, 4 pełny.
    let (poziom, opis) = match info.trade_mode {
        0 => ("error", "handel tym symbolem jest WYŁĄCZONY przez brokera"),
        1 => (
            "warn",
            "broker dopuszcza wyłącznie pozycje KUPNA — sygnały SELL zostaną odrzucone",
        ),
        2 => (
            "warn",
            "broker dopuszcza wyłącznie pozycje SPRZEDAŻY — sygnały BUY zostaną odrzucone",
        ),
        3 => (
            "error",
            "broker dopuszcza wyłącznie ZAMYKANIE pozycji — nowe wejścia będą odrzucane",
        ),
        _ => ("", ""),
    };
    if !opis.is_empty() {
        let tresc = format!(
            "{}: {opis}.\n\nTo NIE jest błąd bota — tak jest ustawiony instrument \
             po stronie brokera. Objawem byłaby seria odmów bez wyjaśnienia.",
            info.symbol
        );
        if poziom == "error" {
            st.notify(
                MailCategory::Mt5Connection,
                "Symbol wyłączony z handlu",
                &tresc,
            );
        } else {
            st.log("mt5", "warn", "Ograniczony handel symbolem", tresc);
        }
    }
    None
}

/// Cisza w strumieniu kwotowań — rynek zamknięty albo zamarły terminal.
fn zglos_cisze(
    st: &StateHandle,
    symbol: &str,
    info: &conduit_mt5::proto::SymbolInfo,
    ostatni_ts: i64,
    ile: Duration,
) {
    let _ = info;
    let tresc = format!(
        "Od {} nie przyszło ANI JEDNO kwotowanie {symbol}.\n\
         Ostatni znacznik z terminala: {}.\n\n\
         Dwie możliwości, obie warte sprawdzenia:\n\
         • rynek jest zamknięty (weekend, przerwa serwisowa brokera) — wtedy jest to normalne \
           i bot ruszy sam, gdy notowania wrócą;\n\
         • terminal MetaTrader 5 zamarł, ale trzyma połączenie — proces żyje, więc nadzorca \
           procesu tego NIE zobaczy. Wtedy pomaga ręczny restart terminala.\n\n\
         Otwarte pozycje i zlecenia zostają u brokera. Bot nie zamyka niczego z tego powodu — \
         zamykanie na podstawie nieaktualnej ceny byłoby gorsze niż czekanie.",
        conduit_mt5::watchdog::opis_czasu(ile),
        if ostatni_ts > 0 {
            conduit_server::store::stamp(ostatni_ts)
        } else {
            "brak".into()
        }
    );
    st.notify(
        MailCategory::Mt5Connection,
        &format!("Brak kwotowań {symbol}"),
        &tresc,
    );
}

fn podsumowanie(
    st: &StateHandle,
    silniki: &routing::Silniki,
    broker: &Recording,
    symbol: &str,
    zegary: &mut ZegarRaportu,
) {
    let engine = &silniki.glowny().engine;
    let (tg_on, tg_min, mail_on, mail_min) = st.read(|s| {
        (
            s.notify.summary_enabled,
            s.notify.summary_interval_min,
            // poczta chce raportu tylko wtedy, gdy jest włączona ORAZ ma
            // zaznaczoną kategorię „Raport okresowy" — inaczej i tak by go
            // odrzucił dławik, a zegar zdążyłby się przewinąć
            s.email.enabled && s.email.categories.summary,
            s.email.interval_min,
        )
    });
    let chce_tg = tg_on
        && tg_min > 0.0
        && zegary.telegram.elapsed() >= Duration::from_secs_f64(tg_min * 60.0);
    let chce_mail = mail_on
        && mail_min > 0.0
        && zegary.mail.elapsed() >= Duration::from_secs_f64(mail_min * 60.0);
    if !chce_tg && !chce_mail {
        return;
    }

    let acc = broker.account();
    let s = &engine.stats;
    let q = broker.quote();
    // Liczby PODSUMOWANIA opisuja rachunek, wiec ida ze WSZYSTKICH silnikow.
    // Przy jednym formacie wychodza dokladnie te same wartosci co przed
    // wprowadzeniem wielu silnikow.
    let dzis = silniki.realized_today();
    let zamkniec = silniki.zamkniec_dzis();
    let zywych = silniki.zywe_koszyki();
    let zatrzymany = silniki.halted();

    let nic_sie_nie_dzieje = dzis.abs() < 0.005
        && broker.positions().is_empty()
        && broker.pendings().is_empty()
        && zywych == 0;
    if nic_sie_nie_dzieje {
        return;
    }
    let teraz = Instant::now();
    if chce_mail {
        zegary.mail = teraz;
    }
    if chce_tg {
        zegary.telegram = teraz;
    }

    let otwarte: f64 = broker.positions().iter().map(|p| p.profit_usd(&q)).sum();
    let temat = format!("Podsumowanie: {dzis:+.2} $ dzisiaj");
    let tresc = format!(
        "Rachunek · saldo {:.2} $ · equity {:.2} $ · wolny margines {:.2} $\n\
         Dzisiaj · zrealizowane {:+.2} $ · obsunięcie dnia {:.2} $ · transakcji {}\n\
         Teraz · pozycji {} (pływające {:+.2} $) · zleceń oczekujących {} · koszyków {}\n\
         {symbol} · bid {:.2} / ask {:.2}\n\
         Skuteczność od startu · {} z {} ({:.0} %) · profit factor {:.2}{}",
        acc.balance,
        acc.equity,
        acc.free_margin,
        dzis,
        s.day_max_dd,
        zamkniec,
        broker.positions().len(),
        otwarte,
        broker.pendings().len(),
        zywych,
        q.bid,
        q.ask,
        s.wins,
        s.trades,
        s.win_rate() * 100.0,
        s.profit_factor(),
        match zatrzymany.as_deref() {
            Some(r) => format!("\n\nUWAGA: HANDEL ZATRZYMANY — {r}"),
            None => String::new(),
        }
    );
    if chce_mail {
        st.notify_mail(MailCategory::Summary, &temat, &tresc);
    }
    if chce_tg {
        st.notify_tg(&temat, &tresc);
    }
}

/// Zegary raportu okresowego — po jednym na kanał dostawy.
///
/// Mieszkają w [`Trwale`], a NIE w `handel()`, i to jest cała poprawka
/// jednej z trzech przyczyn „maile przychodzą rzadko". `handel()` jest
/// wywoływane od nowa przy KAŻDYM wznowieniu mostu do MT5 (zerwanie
/// sidecara, 12 minut ciszy w kwotowaniach), więc zegar zadeklarowany w jego
/// ciele wracał wtedy do zera. Przy terminalu, który czka częściej niż co
/// 20 minut, odstęp 20 minut NIGDY nie mijał — a użytkownik dostawał tylko
/// maile „MT5 podłączony", czyli dowód, że rekonekty faktycznie są.
struct ZegarRaportu {
    mail: Instant,
    telegram: Instant,
}

impl Default for ZegarRaportu {
    fn default() -> Self {
        let t = Instant::now();
        ZegarRaportu {
            mail: t,
            telegram: t,
        }
    }
}

// ============================================================
//  POLECENIA Z PANELU
// ============================================================

/// Puste pole formularza znaczy „BRAK", a nie „cena zero".
///
/// Panel wysyła nieuzupełnione pola SL/TP/cena jako `0`. Bez tej zamiany
/// zlecenie leciało do brokera z TP = 0.00, a to dla pozycji BUY jest cel
/// PO NIEWŁAŚCIWEJ STRONIE ceny — most odrzucał je w `precheck_stops` jako
/// `InvalidStops`. Objaw dla użytkownika: „klikam BUY i nic się nie dzieje",
/// bo pozycja nie powstawała, a odmowa lądowała tylko w dzienniku.
#[inline]
fn poziom(v: Option<f64>) -> Option<Px> {
    v.filter(|x| x.is_finite() && *x > 0.0)
}

/// Zamknięcie zbiorcze z panelu. `which` NIE jest ozdobą: „Zyskowne"
/// i „Tracące" to osobne przyciski i muszą zamykać osobne zbiory.
/// Wcześniej każdy z nich zamykał WSZYSTKO — najgorszy możliwy rodzaj
/// nieporozumienia w terminalu handlowym.
/// Domyka pozycje spoza bota. Osobno od silników, bo silnik ich nie zna —
/// bez tego przycisk „zamknij wszystko" zostawiałby część rachunku otwartą
/// i nie mówił o tym ani słowa.
fn zamknij_obce(broker: &mut Recording, which: &str) -> Option<String> {
    if which != "all" {
        return None;
    }
    let obce: Vec<Ticket> = broker
        .inner
        .foreign_positions()
        .iter()
        .map(|p| p.ticket)
        .collect();
    let ile_obcych = obce.len();
    if ile_obcych == 0 {
        return None;
    }
    let mut n = 0;
    for t in obce {
        let r = broker.inner.foreign_close(t);
        if broker.note("close_position", r).is_ok() {
            n += 1;
        }
    }
    Some(format!("spoza bota {n} z {ile_obcych}"))
}

fn zamknij_zbiorczo_silnika<B: Broker>(
    engine: &mut Engine,
    broker: &mut B,
    which: &str,
    ts: Ts,
) -> Result<String, String> {
    use conduit_core::types::CloseReason;
    if which == "all" {
        engine.close_everything(broker, ts, CloseReason::Manual);
        return Ok("zamknięto wszystko (pozycje i zlecenia oczekujące)".to_string());
    }
    let chce_zysk = which == "profit";
    let q = broker.quote();
    let tickety: Vec<Ticket> = broker
        .positions()
        .iter()
        .filter(|p| {
            let z = p.profit_usd(&q);
            if chce_zysk {
                z > 0.0
            } else {
                z < 0.0
            }
        })
        .map(|p| p.ticket)
        .collect();
    if tickety.is_empty() {
        return Ok(format!(
            "nie ma pozycji {} — nic do zamknięcia",
            if chce_zysk { "na plusie" } else { "na minusie" }
        ));
    }
    let ile = tickety.len();
    let mut zamkniete = 0;
    let mut suma = 0.0;
    let mut ostatni_blad = None;
    for t in tickety {
        match broker.close_position(t, CloseReason::Manual) {
            Ok(p) => {
                zamkniete += 1;
                suma += p;
            }
            Err(e) => ostatni_blad = Some(e),
        }
    }
    let opis = format!(
        "zamknięto {zamkniete} z {ile} pozycji {} · {suma:+.2} $",
        if chce_zysk { "na plusie" } else { "na minusie" }
    );
    match ostatni_blad {
        Some(e) if zamkniete == 0 => Err(format!("{opis} — {}", opis_bledu(e))),
        Some(e) => Ok(format!("{opis} (część odrzucona: {})", opis_bledu(e))),
        None => Ok(opis),
    }
}

/// Zapis koszyka z panelu (SL / strefa / cele).
///
/// Do tej pory ta komenda wpadała w `_ => Ok(String::new())` i kończyła się
/// CISZĄ: panel pokazywał „Koszyk zapisany", a u brokera nie zmieniało się nic.
/// Nowy SL koszyka schodzi na jego niezamrożone pozycje — dokładnie tak, jak
/// robi to silnik przy sygnale „SET SL".
fn zapisz_koszyk<B: Broker>(
    engine: &mut Engine,
    broker: &mut B,
    id: u32,
    patch: &serde_json::Value,
) -> Result<String, String> {
    let o = patch
        .as_object()
        .ok_or_else(|| "łatka koszyka nie jest obiektem".to_string())?;
    let Some(b) = engine.baskets.iter_mut().find(|b| b.id == id) else {
        return Err(format!("nie ma koszyka B{id}"));
    };

    let mut opis: Vec<String> = Vec::new();
    // `null` znaczy „skasuj SL", liczba dodatnia — nowy poziom. Brak klucza
    // znaczy „nie ruszaj" i to jest trzeci, osobny przypadek.
    let nowy_sl = o.get("sl").map(|v| poziom(v.as_f64()));
    if let Some(sl) = nowy_sl {
        b.sl = sl;
        opis.push(match sl {
            Some(x) => format!("SL {x:.2}"),
            None => "SL skasowany".to_string(),
        });
    }
    if let Some(v) = o
        .get("zoneLow")
        .and_then(|v| v.as_f64())
        .filter(|x| *x > 0.0)
    {
        b.zone_lo = v;
        opis.push(format!("dół strefy {v:.2}"));
    }
    if let Some(v) = o
        .get("zoneHigh")
        .and_then(|v| v.as_f64())
        .filter(|x| *x > 0.0)
    {
        b.zone_hi = v;
        opis.push(format!("góra strefy {v:.2}"));
    }
    if let Some(v) = o.get("tps").and_then(|v| v.as_array()) {
        let cele: Vec<Px> = v
            .iter()
            .filter_map(|x| x.as_f64())
            .filter(|x| *x > 0.0)
            .collect();
        if !cele.is_empty() {
            opis.push(format!(
                "cele {}",
                cele.iter()
                    .map(|x| format!("{x:.2}"))
                    .collect::<Vec<_>>()
                    .join(" / ")
            ));
            b.tps = cele;
        }
    }
    let tickety = b.tickets.clone();

    // Nowy SL musi dojść DO BROKERA, nie tylko do modelu koszyka. Pozycje
    // zamrożone (ręcznie edytowane) świadomie omijamy — użytkownik przejął
    // nad nimi kontrolę i nikt nie ma prawa mu jej odbierać.
    if let Some(sl) = nowy_sl {
        let mut zmienione = 0;
        let mut odmowa = None;
        for t in tickety {
            let Some((zamrozona, tp)) = broker
                .positions()
                .iter()
                .find(|p| p.ticket == t)
                .map(|p| (p.frozen, p.tp))
            else {
                continue;
            };
            if zamrozona {
                continue;
            }
            match broker.modify_position(t, sl, tp) {
                Ok(()) => zmienione += 1,
                Err(e) => odmowa = Some(e),
            }
        }
        opis.push(format!("SL ustawiony na {zmienione} pozycjach"));
        if let Some(e) = odmowa {
            if zmienione == 0 {
                return Err(format!(
                    "koszyk B{id}: broker odrzucił zmianę SL — {}",
                    opis_bledu(e)
                ));
            }
        }
    }

    if opis.is_empty() {
        return Ok(format!("koszyk B{id}: nic do zmiany"));
    }
    Ok(format!("koszyk B{id}: {}", opis.join(" · ")))
}

fn wznow_handel(
    st: &StateHandle,
    silniki: &mut routing::Silniki,
    diagnoza: &mut String,
    ts: Ts,
) -> Result<String, String> {
    if silniki.lista.iter().any(|s|s.engine.continuation_entry_blocked()) {
        return Err("CONTINUATION REVIEW: zwykłe Wznów nie uzgadnia brakującej pamięci strategii i nie rozbraja strażnika ryzyka".into());
    }
    if !diagnoza.is_empty() {
        let zostaje = st.read(|s| s.halt.powod(ui::KlasaHaltu::Ryzyko).to_string());
        let zdjeta = std::mem::take(diagnoza);
        zdejmij_diagnoze_z_silnikow(silniki, &zdjeta);
        st.update(Sections::one(Section::Halt), |s| {
            s.halt.zdejmij(ui::KlasaHaltu::Diagnoza);
        });
        st.log(
            "settings",
            "warn",
            "Zdjęto zatrzymanie klasy DIAGNOZA (strażnik ryzyka NIETKNIĘTY)",
            format!(
                "Zdjęty powód: {zdjeta}\n\n\
                 `max_dd_pct`, `max_dd_usd` i pułapy łańcucha działają dalej — \
                 zdjęcie diagnozy NIE jest rozbrojeniem strażnika.\n\n\
                 UWAGA: przyczyna diagnozy nie musiała zniknąć; to Ty właśnie \
                 zdecydowałeś, że mimo niej handlujemy. Przy następnym starcie \
                 zostanie sprawdzona od nowa.{}",
                if zostaje.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\n\nZATRZYMANIE KLASY RYZYKO ZOSTAJE AKTYWNE: {zostaje}\n\
                         Ono mówi o tym, co stało się na RACHUNKU, więc nie gaśnie \
                         razem z diagnozą. Żeby je zdjąć, kliknij „Wznów handel\" \
                         jeszcze raz — dopiero to wyłączy strażnika ryzyka."
                    )
                }
            ),
        );
        return Ok(if zostaje.is_empty() {
            "zdjęto zatrzymanie diagnostyczne (strażnik ryzyka DZIAŁA)".to_string()
        } else {
            format!("zdjęto zatrzymanie diagnostyczne · ZOSTAJE ryzyko: {zostaje}")
        });
    }

    // POWÓD ZATRZYMANIA BIERZEMY PRZED `resume_trading` — ta metoda
    // robi `self.halted.take()`, więc po niej nie ma już czego cytować.
    let powod = silniki.halted().unwrap_or_else(|| "—".into());
    for s in silniki.lista.iter_mut() {
        s.engine.resume_trading(ts);
    }
    // STAN „STRAŻNIK WYŁĄCZONY" MUSI DOTRZEĆ DO PANELU.
    //
    // `ui::State::risk_override` istniało od zawsze, było zapisywane
    // na dysk i wysyłane do przeglądarki — i NIGDY przez nikogo
    // ustawiane. Skutek: po jednym kliknięciu „Wznów handel"
    // `engine.risk_override` zostawał na `true` (przeżywa rekonekt
    // i przebudowę łańcucha), czyli `check_guards` wracał natychmiast
    // i `max_dd_pct`, `max_dd_usd` oraz pułapy łańcucha przestawały
    // istnieć — a panel nie pokazywał NIC. Co gorsza, jedyny przycisk
    // „uzbrój strażnika ponownie" mieszka w banerze rysowanym pod
    // warunkiem `riskOverride.active`, więc był nieosiągalny.
    st.update(Sections::one(Section::Halt), |s| {
        s.risk_override = ui::RiskOverride {
            active: true,
            since: conduit_server::now_ms(),
            reason: powod.clone(),
        };
        s.halt.zdejmij(ui::KlasaHaltu::Ryzyko);
    });
    st.notify(
        MailCategory::Drawdown,
        "STRAŻNIK RYZYKA WYŁĄCZONY (ręczne wznowienie)",
        &format!(
            "Handel wznowiony mimo: {powod}\n\n\
             UWAGA: `max_dd_pct`, `max_dd_usd` i pułapy łańcucha są od tej chwili \
             NIEAKTYWNE — bot nie zatrzyma się sam, choćby konto osuwało się dalej. \
             Stan przeżywa rekonekt do MT5 i przebudowę łańcucha, czyli trwa do \
             końca życia procesu albo do kliknięcia „Uzbrój strażnika\" w panelu."
        ),
    );
    Ok("handel wznowiony (STRAŻNIK WYŁĄCZONY)".to_string())
}

fn wykonaj_panel(
    st: &StateHandle,
    silniki: &mut routing::Silniki,
    broker: &mut Recording,
    cmd: &Command,
    ts: Ts,
    // Zdanie klasy DIAGNOZA nałożone przez bramkę startową. `&mut`, bo
    // „Wznów handel" potrafi je ZDJĄĆ — a wtedy publikacja stanu nie ma prawa
    // dalej go raportować (patrz `Command::ResumeTrading` niżej).
    diagnoza: &mut String,
) {
    use conduit_core::types::CloseReason;
    // KTORY SILNIK OBSLUGUJE KTORE POLECENIE
    //
    // * polecenia dotyczace KONKRETNEGO KOSZYKA ida do silnika, ktory nim
    //   zarzadza — inaczej „zapisz koszyk" trafialoby w pustke, bo drugi
    //   silnik tego koszyka nie ma na liscie;
    // * polecenia dotyczace CALEGO RACHUNKU (zamknij wszystko, wznow handel,
    //   uzbroj straznika) ida do WSZYSTKICH — uzytkownik kliknal jeden
    //   przycisk i oczekuje jednego skutku na calym koncie;
    // * reszta nie dotyka silnika wcale i idzie prosto do brokera.
    let dla_koszyka = |s: &routing::Silniki, id: u32| s.indeks_koszyka(id);
    let r: Result<String, String> = match cmd {
        // Pozycja spoza bota idzie inną ścieżką: nie ma jej w księdze silnika,
        // więc `close_position` odbiłby ją jako nieznany numer zlecenia.
        // Automat jej nie prowadzi — ale ręczne kliknięcie użytkownika ma
        // działać, bo to jego rachunek.
        Command::ClosePosition { ticket } if broker.inner.is_foreign(*ticket) => {
            let r = broker.inner.foreign_close(*ticket);
            broker
                .note("close_position", r)
                .map(|p| format!("zamknięto #{ticket} (spoza bota) · {p:+.2} $"))
                .map_err(|e| format!("#{ticket}: {}", opis_bledu(e)))
        }
        Command::ClosePosition { ticket } => broker
            .close_position(*ticket, CloseReason::Manual)
            .map(|p| format!("zamknięto #{ticket} · {p:+.2} $"))
            .map_err(|e| format!("#{ticket}: {}", opis_bledu(e))),
        // „Zamknij wszystko" ma zamknac WSZYSTKO — kazdy silnik domyka swoje,
        // a pozycje spoza bota domyka osobna sciezka w `zamknij_zbiorczo`.
        Command::CloseBulk { which } => {
            let mut opisy: Vec<String> = Vec::new();
            let mut blad: Option<String> = None;
            for i in 0..silniki.lista.len() {
                match silniki
                    .z_widokiem(i, broker, |e, w| zamknij_zbiorczo_silnika(e, w, which, ts))
                {
                    Ok(o) => opisy.push(o),
                    Err(e) => blad = Some(e),
                }
            }
            match zamknij_obce(broker, which) {
                Some(o) => opisy.push(o),
                None => {}
            }
            match blad {
                Some(e) if opisy.is_empty() => Err(e),
                Some(e) => Ok(format!("{} (czesc odrzucona: {e})", opisy.join(" · "))),
                None => Ok(opisy.join(" · ")),
            }
        }
        Command::CloseBasket { id } => {
            let tickety: Vec<Ticket> = broker
                .positions()
                .iter()
                .filter(|p| p.basket == Some(*id))
                .map(|p| p.ticket)
                .collect();
            let ile = tickety.len();
            let mut n = 0;
            for t in tickety {
                if broker.close_position(t, CloseReason::Manual).is_ok() {
                    n += 1;
                }
            }
            let oczekujace: Vec<Ticket> = broker
                .pendings()
                .iter()
                .filter(|p| p.basket == Some(*id))
                .map(|p| p.ticket)
                .collect();
            let ile_z = oczekujace.len();
            let mut z = 0;
            for t in oczekujace {
                if broker.cancel_pending(t).is_ok() {
                    z += 1;
                }
            }
            if n == 0 && z == 0 && (ile > 0 || ile_z > 0) {
                return zglos(
                    st,
                    Err(format!(
                        "koszyk B{id}: broker NIE przyjął ani jednego zamknięcia \
                         ({ile} pozycji, {ile_z} zleceń zostaje). Koszyk NIE został \
                         oznaczony jako zakończony — dalej nim zarządzam."
                    )),
                );
            }
            // Koszyk musi być ZAKOŃCZONY także w silniku. Bez tego zostawał
            // `Armed` z pustą siatką: panel pokazywał żywy koszyk, którego
            // u brokera już nie ma, a wygaszanie po czasie próbowało kasować
            // zlecenia dawno skasowane.
            //
            // Ale tylko wtedy, gdy NAPRAWDĘ nic nie zostało. Koszyk z połową
            // pozycji dalej otwartych ma zostać pod zarządem silnika.
            let zostalo = broker
                .positions()
                .iter()
                .filter(|p| p.basket == Some(*id))
                .count()
                + broker
                    .pendings()
                    .iter()
                    .filter(|p| p.basket == Some(*id))
                    .count();
            if zostalo == 0 {
                if let Some(i) = dla_koszyka(silniki, *id) {
                    if let Some(bk) = silniki.lista[i]
                        .engine
                        .baskets
                        .iter_mut()
                        .find(|b| b.id == *id)
                    {
                        bk.state = conduit_core::types::BasketState::Done;
                    }
                }
            }
            let ogon = if zostalo > 0 {
                format!(" — UWAGA: {zostalo} pozycji/zleceń zostało, koszyk dalej pod zarządem")
            } else {
                String::new()
            };
            Ok(format!(
                "koszyk B{id}: zamknięto {n} z {ile} pozycji, skasowano {z} z {ile_z} zleceń{ogon}"
            ))
        }
        Command::DeletePending { ticket } if broker.inner.is_foreign_order(*ticket) => {
            let r = broker.inner.foreign_cancel_pending(*ticket);
            broker
                .note("cancel_pending", r)
                .map(|_| format!("skasowano zlecenie #{ticket} (spoza bota)"))
                .map_err(|e| format!("#{ticket}: {}", opis_bledu(e)))
        }
        Command::DeletePending { ticket } => broker
            .cancel_pending(*ticket)
            .map(|_| format!("skasowano zlecenie #{ticket}"))
            .map_err(|e| format!("#{ticket}: {}", opis_bledu(e))),
        // „Usuń wszystkie" znaczy WSZYSTKIE, także cudze. Wcześniej przycisk
        // widział tylko zlecenia bota i przy 48 obcych limitach kasował zero —
        // wyglądało to jak zepsuty przycisk, a było ciche pominięcie.
        Command::DeleteAllPendings => {
            let nasze: Vec<Ticket> = broker.pendings().iter().map(|p| p.ticket).collect();
            let obce = broker.inner.foreign_order_tickets();
            let (ile_n, ile_o) = (nasze.len(), obce.len());
            let mut n = 0;
            for t in nasze {
                if broker.cancel_pending(t).is_ok() {
                    n += 1;
                }
            }
            let mut o = 0;
            for t in obce {
                let r = broker.inner.foreign_cancel_pending(t);
                if broker.note("cancel_pending", r).is_ok() {
                    o += 1;
                }
            }
            Ok(format!(
                "skasowano {} z {} zleceń oczekujących (bota {n}/{ile_n}, spoza bota {o}/{ile_o})",
                n + o,
                ile_n + ile_o
            ))
        }
        Command::ModifyPosition { ticket, sl, tp } if broker.inner.is_foreign(*ticket) => {
            let r = broker
                .inner
                .foreign_modify(*ticket, poziom(*sl), poziom(*tp));
            broker
                .note("modify_position", r)
                .map(|_| format!("zmieniono SL/TP #{ticket} (spoza bota)"))
                .map_err(|e| format!("#{ticket}: {}", opis_bledu(e)))
        }
        Command::ModifyPosition { ticket, sl, tp } => broker
            .modify_position(*ticket, poziom(*sl), poziom(*tp))
            .map(|_| format!("zmieniono SL/TP #{ticket}"))
            .map_err(|e| format!("#{ticket}: {}", opis_bledu(e))),

        // Zamknięcie CZĘŚCI pozycji. Działa tak samo dla pozycji bota i dla
        // cudzej — różni się tylko tym, przez którą warstwę idzie.
        Command::ClosePartial { ticket, volume } => {
            if broker.inner.is_foreign(*ticket) {
                let r = broker.inner.foreign_close_partial(*ticket, *volume);
                broker
                    .note("close_partial", r)
                    .map(|p| {
                        format!("zamknięto {volume} lota z #{ticket} (spoza bota) · {p:+.2} $")
                    })
                    .map_err(|e| format!("#{ticket}: {}", opis_bledu(e)))
            } else {
                broker
                    .close_partial(*ticket, *volume, CloseReason::Manual)
                    .map(|p| format!("zamknięto {volume} lota z #{ticket} · {p:+.2} $"))
                    .map_err(|e| format!("#{ticket}: {}", opis_bledu(e)))
            }
        }
        Command::ModifyPending {
            ticket,
            price,
            sl,
            tp,
        } if broker.inner.is_foreign_order(*ticket) => {
            let r = broker
                .inner
                .foreign_modify_pending(*ticket, *price, poziom(*sl), poziom(*tp));
            broker
                .note("modify_pending", r)
                .map(|_| format!("zmieniono zlecenie #{ticket} (spoza bota)"))
                .map_err(|e| format!("#{ticket}: {}", opis_bledu(e)))
        }
        Command::ModifyPending {
            ticket,
            price,
            sl,
            tp,
        } => broker
            .modify_pending(*ticket, *price, poziom(*sl), poziom(*tp))
            .map(|_| format!("zmieniono zlecenie #{ticket}"))
            .map_err(|e| format!("#{ticket}: {}", opis_bledu(e))),

        // RĘCZNE ZLECENIE Z PANELU.
        //
        // `kind` był tu wcześniej ignorowany: każde zlecenie — także „Limit"
        // i „Stop" — wychodziło jako otwarcie PO RYNKU. Panel ma trzy zakładki
        // rodzaju zlecenia i pole „cena aktywacji"; wybór użytkownika musi
        // znaczyć dokładnie to, co jest na przycisku.
        Command::OpenOrder {
            kind,
            direction,
            volume,
            price,
            sl,
            tp,
        } => {
            let side = if matches!(direction, ui::Direction::Buy) {
                Side::Buy
            } else {
                Side::Sell
            };
            let (sl, tp, cena) = (poziom(*sl), poziom(*tp), poziom(*price));
            match kind.as_str() {
                "limit" | "stop" => {
                    let Some(p) = cena else {
                        return zglos(
                            st,
                            Err(format!(
                                "zlecenie {kind} wymaga ceny aktywacji — pole „cena” jest puste"
                            )),
                        );
                    };
                    let rodzaj = if kind == "limit" {
                        PendingKind::limit(side)
                    } else {
                        PendingKind::stop(side)
                    };
                    broker
                        .place_pending(PendingReq {
                            kind: rodzaj,
                            volume: *volume,
                            price: p,
                            sl,
                            tp,
                            basket: None,
                            level: 0,
                            is_toucher: false,
                            comment: "panel".into(),
                            is_topup: false,
                        })
                        .map(|t| format!("wystawiono {rodzaj:?} #{t} · {volume} lot @ {p:.2}"))
                        .map_err(|e| format!("{rodzaj:?} @ {p:.2}: {}", opis_bledu(e)))
                }
                _ => broker
                    .open_market(OrderReq {
                        side,
                        volume: *volume,
                        sl,
                        tp,
                        basket: None,
                        level: 0,
                        is_toucher: false,
                        comment: "panel".into(),
                    })
                    .map(|t| format!("otwarto #{t} · {side:?} {volume} lot po rynku"))
                    .map_err(|e| format!("{side:?} {volume} lot: {}", opis_bledu(e))),
            }
        }

        Command::UpdateBasket { id, patch } => match dla_koszyka(silniki, *id) {
            Some(i) => silniki.z_widokiem(i, broker, |e, w| zapisz_koszyk(e, w, *id, patch)),
            None => Err(format!("nie ma koszyka B{id}")),
        },
        // WZNOWIENIE I UZBROJENIE DOTYCZA CALEGO RACHUNKU.
        //
        // Panel ma jeden przycisk „wznow handel". Gdyby wznawial tylko jeden
        // silnik, uzytkownik widzialby zielona kontrolke przy koncie, ktore
        // w polowie dalej stoi — i nie mial jak sie dowiedziec, ktora polowa.
        Command::ResumeTrading => wznow_handel(st, silniki, diagnoza, ts),
        Command::RearmGuard => {
            for s in silniki.lista.iter_mut() {
                s.engine.rearm_guard(ts);
            }
            st.update(Sections::one(Section::Halt), |s| {
                s.risk_override = ui::RiskOverride::default();
            });
            Ok("strażnik uzbrojony ponownie".to_string())
        }
        // Cisza jest zakazana także tutaj: komenda, której pętla nie umie
        // wykonać, musi zostawić ślad, a nie zniknąć.
        inne => Err(format!("pętla handlowa nie obsługuje polecenia {inne:?}")),
    };

    zglos(st, r);
}

/// Wynik polecenia z panelu → dziennik + TOAST + (przy odmowie) mail.
///
/// Toast jest tu kluczowy. `ack` na WebSockecie potwierdza tylko PRZYJĘCIE
/// komendy do kolejki — samo wykonanie dzieje się w wątku handlowym sekundę
/// później. Bez tego zdarzenia odmowa brokera nie docierała do użytkownika
/// wcale: panel meldował sukces, pozycji nie było, a powód leżał w dzienniku,
/// do którego nikt nie zaglądał.
fn zglos(st: &StateHandle, r: Result<String, String>) {
    match r {
        Ok(s) if s.is_empty() => {}
        Ok(s) => {
            st.log("trade", "success", "Polecenie z panelu", s.clone());
            st.emit(ui::UiEvent::Toast {
                level: "success".into(),
                title: "Polecenie wykonane".into(),
                text: s,
            });
        }
        Err(e) => {
            st.log("trade", "error", "Polecenie z panelu odrzucone", e.clone());
            st.emit(ui::UiEvent::Toast {
                level: "error".into(),
                title: "Polecenie odrzucone".into(),
                text: e.clone(),
            });
            st.notify(MailCategory::OrderError, "Polecenie z panelu odrzucone", &e);
        }
    }
}

// ============================================================
//  POWIADOMIENIA
// ============================================================

fn dziennikuj_odmowy<B: Broker>(silniki: &mut routing::Silniki, broker: &B, bledy: &[Odmowa]) {
    use conduit_core::journal::{
        Ev, EventCategory, EventKind, EventLevel, MarketSnapshot, RejectCode,
    };

    if bledy.is_empty() || !silniki.glowny().engine.journal.enabled() {
        return;
    }
    let q = broker.quote();
    let migawka = Some(MarketSnapshot::build(
        &q,
        &broker.account(),
        broker.positions(),
        broker.pendings().len(),
        silniki.glowny().engine.stats.peak_equity,
        silniki.glowny().engine.stats.realized_today,
    ));

    for o in bledy {
        // ODMOWA IDZIE DO DZIENNIKA SILNIKA, KTORY JA SPOWODOWAL.
        //
        // Numer koszyka w odmowie wskazuje slot, a slot wskazuje silnik.
        // Wrzucanie wszystkich odmow do jednego dziennika znaczyloby, ze
        // „ile zlecen odrzucil broker formatowi Synergy" jest niepoliczalne —
        // a to jest pierwsza liczba, ktorej sie szuka przy nowym formacie.
        let i = o
            .basket
            .and_then(|b| silniki.indeks_koszyka(b))
            .unwrap_or_else(|| silniki.lista.iter().position(|s| s.zapasowy).unwrap_or(0));
        let engine = &mut silniki.lista[i].engine;
        let mut ev = Ev::new(
            q.ts,
            EventLevel::Error,
            EventCategory::Order,
            EventKind::OrderRejected,
        )
        .text(format!("{}: {}", o.op, opis_bledu(o.err)))
        .reason(RejectCode::from(o.err))
        .market(migawka.clone())
        .basket_opt(o.basket)
        .put("operation", o.op)
        // `RejectCode` skleja `InvalidStops` z `WrongSide`, a `InvalidPrice`
        // wrzuca do worka `broker_rejected`. Dokładny wariant zostaje tutaj,
        // bo różnica „zły poziom siatki" ↔ „zły stop-loss" jest różnicą
        // między dwiema zupełnie innymi naprawami.
        .put("broker_error", format!("{:?}", o.err))
        .put("hint", opis_bledu(o.err))
        .put_f("stops_level", broker.stops_level());
        if let Some(t) = o.ticket {
            ev = ev.ticket(t);
        }
        if let Some(v) = o.level {
            ev = ev.put("level", v as i64);
        }
        if let Some(v) = o.volume {
            ev = ev.put_f("volume", v);
        }
        if let Some(v) = o.price {
            ev = ev.put_f("price", v);
        }
        if let Some(v) = o.sl {
            ev = ev.put_f("sl", v);
        }
        if let Some(v) = o.tp {
            ev = ev.put_f("tp", v);
        }
        engine.journal.push(ev.build());
    }
}

fn dziennikuj_odrzut_wieku<B: Broker>(
    silniki: &mut routing::Silniki,
    broker: &B,
    im: &IncomingMessage,
    format: Option<String>,
    wiek_min: f64,
    prog_min: f64,
) {
    use conduit_core::journal::{
        signal_id, Ev, EventCategory, EventKind, EventLevel, MarketSnapshot, RejectCode,
    };
    let i = silniki
        .trasa(&im.source, format)
        .ok()
        .unwrap_or_else(|| silniki.lista.iter().position(|s| s.zapasowy).unwrap_or(0));
    let engine = &mut silniki.lista[i].engine;
    // Licznik lejka rośnie NIEZALEŻNIE od poziomu dziennika — kod `StaleSignal`
    // ten sam, którym silnik stempluje w DZIENNIKU własne zamknięcia z wieku
    // (twardy limit wieku koszyka, engine.rs), więc forensyka grupuje po jednym
    // kodzie. Kubełek `odrzuty` dla tego kodu zasila wyłącznie ta bramka:
    // `expire_stale_baskets` (ignore_old_after_min) pisze notki koszyka, nie
    // liczniki.
    *engine
        .odrzuty
        .entry(format!("{:?}", RejectCode::StaleSignal))
        .or_insert(0) += 1;
    if !engine.journal.wants(EventLevel::Warn) {
        return;
    }
    let q = broker.quote();
    let snap = Some(MarketSnapshot::build(
        &q,
        &broker.account(),
        broker.positions(),
        broker.pendings().len(),
        engine.stats.peak_equity,
        engine.stats.realized_today,
    ));
    engine.journal.push(
        Ev::new(
            q.ts,
            EventLevel::Warn,
            EventCategory::Decision,
            EventKind::SignalRejected,
        )
        .text(format!(
            "sygnał przeterminowany: wysłany {wiek_min:.0} min temu, próg {prog_min:.0} min"
        ))
        .msg(im.msg_id)
        .signal(signal_id(im.msg_id, "entry"))
        .source(im.source_name.clone())
        .reason(RejectCode::StaleSignal)
        .market(snap)
        .build(),
    );
}

/// Odmowy brokera → log panelu + mail (kategoria `OrderError`).
///
/// To jest brakujące miejsce wywołania, o którym mówił audyt: kategoria
/// istniała w konfiguracji i w dławiku, ale nie było kodu, który by ją
/// kiedykolwiek wywołał. Zapis do dziennika robi `dziennikuj_odmowy`.
fn zglos_bledy(st: &StateHandle, bledy: &[Odmowa]) {
    let mut opis = String::new();
    for o in bledy {
        opis.push_str(&format!("• {}: {}", o.op, opis_bledu(o.err)));
        if let Some(b) = o.basket {
            opis.push_str(&format!(" (koszyk B{b}"));
            if let Some(l) = o.level {
                opis.push_str(&format!(", poziom {l}"));
            }
            if let Some(p) = o.price {
                opis.push_str(&format!(", cena {p:.2}"));
            }
            opis.push(')');
        } else if let Some(t) = o.ticket {
            opis.push_str(&format!(" (#{t})"));
        }
        opis.push('\n');
    }
    st.log(
        "trade",
        "error",
        format!("Broker odrzucił {} operacji", bledy.len()),
        opis.clone(),
    );
    st.notify(
        MailCategory::OrderError,
        &format!("Broker odrzucił zlecenie ({})", bledy.len()),
        &format!(
            "{opis}\nJeśli powtarza się „SL/TP za blisko ceny”, sprawdź stops_level \
             u brokera. Jeśli „cena zlecenia oczekującego niedopuszczalna” — poziom siatki \
             wypadł po złej stronie rynku albo w pasie stops_level; decyduje o tym \
             ustawienie „pending_cross_policy”. Jeśli „brak marginesu” — zmniejsz lot \
             albo liczbę jednostek."
        ),
    );
}

fn opis_bledu(e: BrokerError) -> &'static str {
    match e {
        BrokerError::InvalidStops => "SL/TP bliżej ceny niż wymaga broker (stops level)",
        BrokerError::InvalidPrice => {
            "cena zlecenia oczekującego niedopuszczalna — poziom po złej stronie rynku \
             albo bliżej ceny niż stops level (to NIE jest problem z SL/TP)"
        }
        BrokerError::WrongSide => "SL/TP po niewłaściwej stronie ceny",
        BrokerError::NoSuchTicket => "zlecenie/pozycja już nie istnieje",
        BrokerError::NotEnoughMargin => "brak wolnego marginesu",
        BrokerError::InvalidVolume => {
            "wolumen poza dopuszczalnym zakresem albo niezgodny z krokiem"
        }
        BrokerError::MarketClosed => "rynek zamknięty",
        BrokerError::Rejected => "broker odrzucił zlecenie",
    }
}

/// Straż obsunięcia → mail (kategoria `Drawdown`).
///
/// Drugie brakujące miejsce wywołania z audytu. Próg bierzemy z tych samych
/// ustawień, których używa silnik (`max_dd_pct` / `max_dd_usd`), żeby mail
/// i zatrzymanie handlu mówiły o tej samej granicy.
fn sprawdz_obsuniecie(
    st: &StateHandle,
    silniki: &routing::Silniki,
    acc: Account,
    szczyt: f64,
    ostatnie: &mut f64,
    zgloszone_zatrzymanie: &mut bool,
) {
    // Prog strażnika bierzemy z silnika GLOWNEGO, bo obsuniecie mierzymy tu na
    // equity CALEGO rachunku — a to jest jedna liczba. Zatrzymanie sprawdzamy
    // natomiast u wszystkich: stojacy jeden format to juz powod do maila.
    let engine = &silniki.glowny().engine;
    let zatrzymany = silniki.halted();
    let dd = (szczyt - acc.equity).max(0.0);
    if dd <= 0.0 || szczyt <= 0.0 {
        return;
    }
    let pct = dd / szczyt * 100.0;

    if let Some(powod) = zatrzymany.as_ref() {
        if !*zgloszone_zatrzymanie {
            *zgloszone_zatrzymanie = true;
            *ostatnie = pct;
            st.notify(
                MailCategory::Drawdown,
                "HANDEL ZATRZYMANY przez strażnika ryzyka",
                &format!(
                    "Powód: {powod}\n\nEquity {:.2} $, szczyt {:.2} $, obsunięcie {:.2} $ \
                     ({pct:.1} %).\n\nNowe sygnały NIE będą realizowane, dopóki nie \
                     wznowisz handlu w panelu.",
                    acc.equity, szczyt, dd
                ),
            );
        }
        return;
    }
    // Handel znów idzie — kolejne zatrzymanie ma prawo do własnego maila.
    *zgloszone_zatrzymanie = false;

    let prog_pct = engine.cfg.max_dd_pct;
    let prog_usd = engine.cfg.max_dd_usd;

    if prog_pct <= 0.0 && prog_usd <= 0.0 {
        let prog_alarm = st
            .read(|s| s.settings.get("alert_dd_pct").and_then(|v| v.as_f64()))
            .unwrap_or(ALARM_DD_PCT_DOMYSLNY);
        if prog_alarm <= 0.0 || pct < prog_alarm {
            return;
        }
        // Pierwszy mail na progu, kolejne co 5 punktów procentowych głębiej.
        // Krok 0,25 pp z gałęzi ze strażnikiem byłby tu zalewem: bez hamulca
        // obsunięcie potrafi iść dziesiątkami procent i nikt tego nie ucina.
        if *ostatnie > 0.0 && pct < *ostatnie + ALARM_DD_KROK_PP {
            return;
        }
        *ostatnie = pct;
        st.notify(
            MailCategory::Drawdown,
            &format!("UWAGA: obsunięcie {pct:.1} % — STRAŻNIK JEST WYŁĄCZONY"),
            &format!(
                "Equity {:.2} $, szczyt {:.2} $, obsunięcie {:.2} $ ({pct:.1} %).\n\n\
                 W ustawieniach `max_dd_pct` i `max_dd_usd` są ZEROWE, czyli strażnik \
                 ryzyka jest świadomie wyłączony. BOT NIE ZATRZYMA HANDLU SAM — to jest \
                 wyłącznie ostrzeżenie.\n\n\
                 Jedyną ochroną konta są w tej chwili stop-lossy wystawione u brokera. \
                 Jeśli chcesz przerwać handel, zrób to w panelu (zatrzymanie bota albo \
                 tryb MANUAL).\n\n\
                 Próg tego ostrzeżenia: {prog_alarm:.1} % (ustawienie `alert_dd_pct`, \
                 0 = bez ostrzeżeń). Kolejny mail po pogłębieniu o {ALARM_DD_KROK_PP:.0} pp.",
                acc.equity, szczyt, dd
            ),
        );
        return;
    }

    // ---------- STRAŻNIK WŁĄCZONY ----------
    // Ostrzegamy od 60 % ustawionego limitu — mail po fakcie zatrzymania jest
    // informacją, mail przed zatrzymaniem jest szansą na reakcję.
    let bliski =
        (prog_pct > 0.0 && pct >= prog_pct * 0.6) || (prog_usd > 0.0 && dd >= prog_usd * 0.6);
    if !bliski {
        return;
    }
    // Kolejny mail dopiero po pogłębieniu o 1/4 punktu procentowego —
    // dławik w `mailer` i tak zwija powtórki, ale nie ma po co ich robić.
    if pct < *ostatnie + 0.25 {
        return;
    }
    *ostatnie = pct;
    st.notify(
        MailCategory::Drawdown,
        &format!("Obsunięcie {pct:.1} % zbliża się do limitu"),
        &format!(
            "Equity {:.2} $, szczyt {:.2} $, obsunięcie {:.2} $ ({pct:.1} %).\n\
             Limit z ustawień: {} / {}.\n\nPo przekroczeniu strażnik zatrzyma handel.",
            acc.equity,
            szczyt,
            dd,
            if prog_pct > 0.0 {
                format!("{prog_pct:.1} %")
            } else {
                "—".into()
            },
            if prog_usd > 0.0 {
                format!("{prog_usd:.2} $")
            } else {
                "—".into()
            },
        ),
    );
}

/// Czy przy wyłączonym strażniku mail o obsunięciu ma prawo wyjść?
///
/// Wydzielone z `sprawdz_obsuniecie`, żeby dało się to sprawdzić testem bez
/// stawiania całej pętli handlowej i serwera poczty.
#[cfg_attr(not(test), allow(dead_code))]
fn alarm_dd_nalezy_sie(pct: f64, prog_alarm: f64, ostatni_mail_pct: f64) -> bool {
    if prog_alarm <= 0.0 || pct < prog_alarm {
        return false;
    }
    ostatni_mail_pct <= 0.0 || pct >= ostatni_mail_pct + ALARM_DD_KROK_PP
}

// ============================================================
//  PUBLIKACJA STANU
// ============================================================

// ---------- tłumaczenie „obcych" faktów z MT5 na model panelu ----------
//
// Te funkcje siedzą TU, a nie w `conduit_server::ui`, bo warstwa serwera
// świadomie nie zależy od `conduit_mt5`. Aplikacja zna obie strony i to ona
// je zszywa.

/// Pozycja spoza bota → wiersz panelu. Zysk bierzemy PROSTO OD BROKERA:
/// dla obcego symbolu nie mamy własnego kwotowania, a zgadywanie byłoby
/// gorsze niż fakt.
fn poz_obca(p: &conduit_mt5::ForeignPosition) -> ui::Position {
    ui::Position {
        ticket: p.ticket,
        symbol: p.symbol.clone(),
        direction: p.side.into(),
        volume: p.volume,
        open_price: p.open_price,
        open_time: p.open_ts,
        sl: p.sl,
        tp: p.tp,
        vsl: None,
        profit: p.profit,
        swap: 0.0,
        commission: 0.0,
        comment: p.comment.clone(),
        magic: Some(p.magic),
        basket_id: None,
        level: 0,
        frozen: false,
        peak_pts: 0.0,
        runner: false,
        toucher: false,
        last_peak_time: p.open_ts,
        source: ui::Origin::from_name(p.origin.as_str()),
    }
}

fn zlec_obce(o: &conduit_mt5::ForeignOrder) -> ui::PendingOrder {
    ui::PendingOrder {
        ticket: o.ticket,
        symbol: o.symbol.clone(),
        kind: o.kind.into(),
        volume: o.volume,
        price: o.price,
        sl: o.sl,
        tp: o.tp,
        placed_time: o.placed_ts,
        comment: o.comment.clone(),
        basket_id: None,
        level: 0,
        frozen: false,
        source: ui::Origin::from_name(o.origin.as_str()),
        magic: Some(o.magic),
    }
}

fn zamk_obce(c: &conduit_mt5::ForeignClosed) -> ui::ClosedPosition {
    ui::ClosedPosition {
        ticket: c.ticket,
        symbol: c.symbol.clone(),
        direction: c.side.into(),
        volume: c.volume,
        open_price: c.open_price,
        close_price: c.close_price,
        open_time: c.open_ts,
        close_time: c.close_ts,
        profit: c.profit,
        swap: c.swap,
        commission: c.commission,
        // Powodu zamknięcia cudzej transakcji NIE znamy — nie zgadujemy go.
        reason: ui::CloseReason::Manual,
        comment: c.comment.clone(),
        basket_id: None,
        source: ui::Origin::from_name(c.origin.as_str()),
        magic: Some(c.magic),
    }
}

/// Podsumowanie tego, czego bot nie prowadzi — do paska informacyjnego.
fn podsumuj_obce(
    pos: &[conduit_mt5::ForeignPosition],
    ord: &[conduit_mt5::ForeignOrder],
) -> ui::ForeignSummary {
    let mut s = ui::ForeignSummary {
        positions: pos.len(),
        pendings: ord.len(),
        ..Default::default()
    };
    for p in pos {
        s.volume += p.volume;
        s.profit += p.profit;
        if !s.magics.contains(&p.magic) {
            s.magics.push(p.magic);
        }
        if !s.symbols.contains(&p.symbol) {
            s.symbols.push(p.symbol.clone());
        }
        let c = p.comment.trim();
        if !c.is_empty() && !s.comments.iter().any(|x| x == c) && s.comments.len() < 3 {
            s.comments.push(c.to_string());
        }
    }
    for o in ord {
        if !s.magics.contains(&o.magic) {
            s.magics.push(o.magic);
        }
        if !s.symbols.contains(&o.symbol) {
            s.symbols.push(o.symbol.clone());
        }
    }
    s.magics.sort_unstable();
    s.symbols.sort();
    s
}

fn sekcje() -> Sections {
    let mut s = Sections::one(Section::Quotes);
    for x in [
        Section::Positions,
        Section::Pendings,
        Section::Baskets,
        Section::Closed,
        Section::Stats,
        Section::Messages,
        Section::Connection,
        Section::Halt,
    ] {
        s.insert(x);
    }
    s
}

/// `Quote.ts` jest już zegarem serwera. Offset UTC→serwer służy wiadomościom,
/// nie ponownemu przesuwaniu granicy doby w panelu.
fn doba_kwotowania_brokera(ts: Ts, cfg: &conduit_core::Settings) -> i64 {
    (ts + cfg.session_offset()).div_euclid(86_400_000)
}

/// Called only inside the publisher's atomic update, together with quotes and
/// account identity. A preset never supplies this symbol; it comes from Bridge.
fn publish_mt5_binding(connection: &mut ui::ConnectionState, symbol: &str, account_session: &str) {
    connection.mt5 = "connected".into();
    connection.account_session = account_session.to_string();
    connection.resolved_symbol = symbol.to_string();
}

fn opublikuj(
    st: &StateHandle,
    broker: &Recording,
    silniki: &routing::Silniki,
    szczeble: &[WierszSzczebla],
    msgs: &[ui::ChatMessage],
    symbol: &str,
    info: &conduit_mt5::SymbolInfo,
    // Zdanie klasy DIAGNOZA nałożone przez bramkę startową tej pętli — po nim
    // rozpoznajemy, która część `Engine::halted` jest diagnozą, a która
    // ryzykiem (patrz `rozbij_klasy_zatrzymania`).
    diagnoza: &str,
    account_session: &str,
) {
    // Strefa czasowa serwera jest polem RACHUNKU (te sama dla wszystkich
    // formatow — patrz `wielosilnik::POLA_RACHUNKU`), wiec wolno ja wziac
    // z dowolnego silnika.
    let engine = &silniki.glowny().engine;
    let q = broker.quote();
    let acc = broker.account();
    let toz = broker.inner.ident().clone();
    // Ta sama doba serwera co w core. `q.ts` jest już w tym zegarze;
    // ponowne +server_tz_offset_ms resetowałoby panel trzy godziny za wcześnie.
    let doba = doba_kwotowania_brokera(q.ts, &engine.cfg);

    // Pozycje BOTA — te, którymi silnik zarządza.
    let mut pozycje: Vec<ui::Position> = broker
        .positions()
        .iter()
        .map(|x| ui::position_from_core(x, symbol, x.profit_usd(&q)))
        .collect();
    let mut oczekujace: Vec<ui::PendingOrder> = broker
        .pendings()
        .iter()
        .map(|x| ui::pending_from_core(x, symbol))
        .collect();

    // …i WSZYSTKO POZOSTAŁE, co jest na rachunku. CONDUIT jest podglądem całego
    // konta, nie tylko własnego handlu: te pozycje realnie zużywają margines
    // i wchodzą w equity, więc ich pominięcie tworzyło sprzeczność, w której
    // equity nie zgadzało się z listą. Bot ich NIE prowadzi — mówi o tym pole
    // `source`, które panel pokazuje jako plakietkę przy wierszu.
    let obce_poz = broker.inner.foreign_positions();
    let obce_zlec = broker.inner.foreign_orders();
    let podsumowanie = podsumuj_obce(obce_poz, obce_zlec);
    pozycje.extend(obce_poz.iter().map(poz_obca));
    oczekujace.extend(obce_zlec.iter().map(zlec_obce));

    // Panel pokazuje KOSZYKI CALEGO RACHUNKU, ze wszystkich formatow razem —
    // uzytkownik patrzy na jedno konto, a nie na dwa silniki.
    let koszyki: Vec<ui::Basket> = silniki
        .koszyki()
        .iter()
        .map(|b| ui::basket_from_core(b, symbol))
        .collect();
    let (ile_msg, ile_sig) = silniki.wiadomosci_i_sygnaly();
    let (halt_diagnoza, halt_ryzyko) = rozbij_klasy_zatrzymania(silniki, diagnoza);
    // Historia też jest historią CAŁEGO rachunku: transakcje bota i cudze,
    // scalone i posortowane po czasie zamknięcia, każda z etykietą źródła.
    let mut zamkniete: Vec<ui::ClosedPosition> = broker
        .history
        .iter()
        .rev()
        .take(CLOSED_KEEP)
        .map(|c| ui::closed_from_core(c, symbol))
        .collect();
    zamkniete.extend(broker.inner.foreign_closed().iter().map(zamk_obce));
    zamkniete.sort_by_key(|c| std::cmp::Reverse(c.close_time));
    zamkniete.truncate(CLOSED_KEEP);

    debug_assert_eq!(symbol, info.symbol, "publisher must use the bound broker contract");

    let mut kotwice_wyzerowane = false;
    st.update(sekcje(), |s| {
        let stare = s.quotes.get(symbol).cloned();
        let mut k = ui::Quote {
            symbol: symbol.into(),
            bid: q.bid,
            ask: q.ask,
            spread: q.spread(),
            time: q.ts,
            change: 0.0,
            change_pct: 0.0,
            day_high: q.bid,
            day_low: q.bid,
        };
        if let Some(o) = stare {
            // „Dzienne" ekstremum jest dzienne tylko wtedy, gdy stary zapis
            // pochodzi z TEJ SAMEJ doby serwera — bez tego warunku high/low
            // rosły od startu procesu (po tygodniu pracy pokazywały zakres
            // tygodnia, nie dnia). Ta sama doba brokera, którą niżej
            // przestawiają liczniki dobowe (`day_key`).
            let doba_starego = doba_kwotowania_brokera(o.time, &engine.cfg);
            if doba_starego == doba {
                k.day_high = o.day_high.max(q.bid);
                k.day_low = if o.day_low > 0.0 {
                    o.day_low.min(q.bid)
                } else {
                    q.bid
                };
            }
        }
        s.quotes.insert(symbol.into(), k);
        s.positions = pozycje.clone();
        s.pendings = oczekujace.clone();
        s.baskets = koszyki.clone();
        s.closed = zamkniete.clone();
        s.foreign = podsumowanie.clone();
        s.messages = msgs.to_vec();
        s.balance = acc.balance;
        s.stats.balance = acc.balance;
        s.stats.equity = acc.equity;
        s.stats.margin = acc.margin;
        s.stats.free_margin = acc.free_margin;
        s.stats.margin_level = if acc.margin > 0.0 {
            acc.equity / acc.margin * 100.0
        } else {
            0.0
        };

        // ---------- KREDYT BONUSOWY: trzy liczby i skąd pochodzą ----------
        //
        // Panel dostaje SALDO BROKERA, KREDYT i PODSTAWĘ LOTA osobno, razem
        // ze źródłem. Bez tego rozbicia użytkownik nie ma jak sprawdzić, od
        // czego bot liczy wolumen — a na koncie 300 $ + 100 % bonusu to jest
        // różnica dwukrotna.
        //
        // Liczby biorą się z SILNIKA (`kredyt_skuteczny`/`podstawa_lota`),
        // a nie z powtórzonego tu wzoru. Drugi wzór to drugie źródło prawdy
        // i pierwsza okazja, żeby panel pokazał co innego, niż bot gra.
        s.stats.credit = acc.credit;
        s.stats.credit_applied = engine.kredyt_odliczony_od_podstawy();
        s.stats.lot_base = engine.podstawa_lota();
        s.stats.lot_nogi = loty_nog(
            silniki,
            szczeble,
            s.drabinka.biezacy_prog,
            acc.balance,
            &zamkniete,
        );
        s.stats.credit_source = if !engine.cfg.odlicz_kredyt {
            "off".into()
        } else if engine.cfg.kredyt_reczny > 0.0 {
            "reczny".into()
        } else {
            "terminal".into()
        };
        // Rozjazd tylko wtedy, gdy w ogóle odliczamy I ktoś wpisał kwotę
        // ręcznie. Przy automacie nie ma czego porównywać — jest jedno źródło.
        s.stats.credit_mismatch = engine.cfg.odlicz_kredyt
            && engine.cfg.kredyt_reczny > 0.0
            && (engine.cfg.kredyt_reczny - acc.credit).abs() > 0.01;

        if toz.login != 0 {
            let klucz = if s.settings.get("mt5_follow_terminal_account").and_then(Value::as_bool).unwrap_or(false) {
                follow_account_key(&toz, symbol)
            } else { format!("{}@{}", toz.login, toz.server) };
            if s.stats.przelacz_konto(&klucz, acc.equity, doba, q.ts) {
                kotwice_wyzerowane = true;
            }
        }

        update_pnl_anchors(&mut s.stats, acc.balance, acc.equity, doba, q.ts);
        s.stats.messages = ile_msg;
        s.stats.signals = ile_sig;
        if s.stats
            .equity_curve
            .last()
            .map(|c| q.ts - c.t > 30_000)
            .unwrap_or(true)
        {
            s.stats.equity_curve.push(ui::CurvePoint {
                t: q.ts,
                v: acc.equity,
            });
            if s.stats.equity_curve.len() > 600 {
                s.stats.equity_curve.remove(0);
            }
        }
        publish_mt5_binding(&mut s.connection, &info.symbol, account_session);
        // Panel ma pokazywać, NA KTÓRYM rachunku bot handluje. Do tej pory
        // pola konta zostawały puste (login 0, serwer „", dźwignia 0) i jedyną
        // widoczną różnicą między demem a rachunkiem realnym była wysokość
        // salda — czyli żadna, gdy oba mają podobne saldo.
        if toz.login != 0 {
            s.connection.account = ui::AccountInfo {
                login: toz.login,
                server: toz.server.clone(),
                broker: toz.company.clone(),
                currency: toz.currency.clone(),
                leverage: toz.leverage,
                kind: toz.kind().to_string(),
            };
            // WERYFIKACJA KONTA — patrz `ui::ConnectionState::account_verified`.
            // Liczona przy KAŻDEJ publikacji, nie raz przy starcie: użytkownik
            // może wpisać numer rachunku w trakcie pracy i wskaźnik ma się
            // przestawić bez restartu.
            s.connection.account_verified = if s.settings.get("mt5_follow_terminal_account").and_then(Value::as_bool).unwrap_or(false) {
                "ok".into()
            } else { match s
                .settings
                .get("mt5_login")
                .and_then(|v| v.as_i64())
                .filter(|x| *x != 0)
            {
                None => "brak".into(),
                Some(chciany) if chciany == toz.login => "ok".into(),
                Some(_) => "rozjazd".into(),
            } };
        }
        s.halt
            .ustaw(ui::KlasaHaltu::Diagnoza, halt_diagnoza.clone());
        s.halt.ustaw(ui::KlasaHaltu::Ryzyko, halt_ryzyko.clone());
        // SIEĆ BEZPIECZEŃSTWA DLA `risk_override`.
        //
        // Właściwym miejscem zapisu jest obsługa `ResumeTrading`/`RearmGuard`,
        // ale flaga silnika przeżywa rekonekt (`Trwale`) i przebudowę
        // łańcucha, a stan panelu wraca też z `backup_memory`. Bez tej
        // synchronizacji dałoby się dostać stan „silnik bez strażnika, panel
        // czysty" — czyli dokładnie ten, który był tu bugiem: konto bez
        // hamulca i bez śladu o tym w interfejsie.
        let nadpisanie = silniki.lista.iter().any(|x| x.engine.risk_override);
        if nadpisanie != s.risk_override.active {
            s.risk_override.active = nadpisanie;
            if nadpisanie {
                if s.risk_override.since == 0 {
                    s.risk_override.since = conduit_server::now_ms();
                }
                if s.risk_override.reason.is_empty() {
                    s.risk_override.reason = "ręczne wznowienie handlu".into();
                }
            } else {
                s.risk_override.since = 0;
                s.risk_override.reason.clear();
            }
        }
    });
    if kotwice_wyzerowane {
        st.log(
            "mt5",
            "warn",
            "Zmiana rachunku — kotwice PnL wyzerowane",
            format!(
                "Terminal jest teraz na {} · {} ({}). PnL dnia i sesji liczą się                  od bieżącego equity — kotwice poprzedniego konta nie mogą być                  używane po zmianie rachunku.",
                toz.login, toz.server, toz.company
            ),
        );
    }
}

fn oznacz_mt5(st: &StateHandle, ok: bool) {
    st.update(Sections::one(Section::Connection), |s| {
        s.connection.mt5 = if ok {
            "connected".into()
        } else {
            "disconnected".into()
        };
        if !ok {
            // Bez mostu nie ma czego weryfikować — puste, nie stare „ok",
            // które przy martwym połączeniu byłoby kłamstwem.
            s.connection.account_verified = String::new();
            s.connection.account_session.clear();
            s.connection.resolved_symbol.clear();
        }
    });
}

/// Czy bot ma wykonywać sygnały z kanału SAM.
///
/// `MANUAL` = nie; wiadomość czeka na kliknięcie w panelu.
/// `AUTO`, `AUTO-EA` i `AI` = tak.
fn tryb_automatyczny(st: &StateHandle) -> bool {
    st.read(|s| !matches!(s.mode, ui::TradingMode::Manual))
}

/// Czy panel stoi w trybie AUTO-EA („potwór": zarządzanie klasy EA
/// + kierunek z sygnałów traderów).
///
/// KONTRAKT ZERA: wynik ląduje wyłącznie we fladze
/// [`conduit_core::Engine::tryb_auto_ea`], której dziś żadna oś nie czyta —
/// AUTO-EA zachowuje się co do bitu jak AUTO. Flagę będą konsultować dopiero
/// nadchodzące osie warstwy EA (trailing S/R, cykl harvest).
fn tryb_auto_ea(st: &StateHandle) -> bool {
    st.read(|s| matches!(s.mode, ui::TradingMode::AutoEa))
}

fn dopisz(bufor: &mut Vec<ui::ChatMessage>, m: ui::ChatMessage) {
    bufor.push(m);
    if bufor.len() > MSG_KEEP {
        let ile = bufor.len() - MSG_KEEP;
        bufor.drain(0..ile);
    }
}

fn oznacz(bufor: &mut [ui::ChatMessage], id: &str, stan: &str) {
    if let Some(m) = bufor.iter_mut().find(|m| m.id == id) {
        m.pending_action = Some(stan.to_string());
    }
}

fn wiadomosc_ui(st: &StateHandle, im: &IncomingMessage) -> ui::ChatMessage {
    let parsed = conduit_server::demo::parsuj(&im.text);
    let types: Vec<String> = parsed.iter().map(|p| p.kind.clone()).collect();
    // Format przypisany do ŹRÓDŁA (kanał + temat), nie do samego czatu.
    let format = format_zrodla(st, &im.source).filter(|f| !f.is_empty());
    // Etykieta tematu: najpierw nazwa formatu, który użytkownik do niego
    // przypiął („ZEN"), bo to jedyna nazwa tematu, jaką bot w ogóle zna —
    // Telegram oddaje nazwy per dialog, nie per temat. Dopiero potem goły
    // numer, żeby dało się rozróżnić dwa nieprzypisane tematy.
    let topic_name = im.source.topic_id.map(|t| match format.as_deref() {
        Some(f) => format!("{f} · temat {t}"),
        None => format!("temat {t}"),
    });
    ui::ChatMessage {
        // ŹRÓDŁO W IDENTYFIKATORZE, nie sam numer. `msg_id` jest unikalny
        // wyłącznie w obrębie czatu, więc przy kilku kanałach naraz „live-1"
        // znaczyło jednocześnie pierwszą wiadomość Synergy i pierwszą ZEN —
        // a na tym identyfikatorze stoją „Wykonaj"/„Odrzuć" w trybie MANUAL.
        // Ten sam kontrakt, co klucz dedupu w silniku (Pakiet A).
        id: format!("live-{}-{}", im.source.as_string(), im.msg_id),
        time: im.ts,
        channel_id: im.source.chat_id,
        channel_name: im.source_name.clone(),
        topic_id: im.source.topic_id,
        topic_name,
        format,
        text: im.text.clone(),
        types,
        basket_id: None,
        edited: im.edit_of.is_some(),
        pending_action: None,
        parsed: Some(parsed),
    }
}

// ============================================================
//  TESTY
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use conduit_backtest::sim::SimBroker;
    use conduit_core::journal::{EventKind, RejectCode};
    use conduit_core::settings::Settings;

    fn sr_research_config() -> Settings {
        let mut c=Settings::default();c.sr_warmup_exact_ticks=true;
        c.trail_sr_enabled=true;c.trail_sr_offset_atr_mult=0.25;c
    }
    fn sr_manual_state(tag:&str)->StateHandle {
        let st=stan(tag);
        st.update(Sections::all(),|s|{s.lancuchy.aktywny="NO_ACTIVE_CHAIN".into();s.preset_id.clear();});st
    }
    #[test]
    fn live_sr_v2_cold_start_checks_actual_file_owner_and_preserves_risk() {
        let st=sr_manual_state("sr-v2-cold");
        st.update(Sections::all(),|s|{
            s.settings=serde_json::to_value(sr_research_config()).unwrap();
            s.stats.balance=159.8;s.halt.ustaw(ui::KlasaHaltu::Ryzyko,"existing DD");
        });
        assert!(!live_sr_start_allowed(&st));
        st.read(|s|{assert_eq!(s.stats.balance,159.8);assert!(s.halt.ryzyko.contains("existing DD"));
            assert!(s.halt.diagnoza.contains(LIVE_SR_V2_HOLD));assert_eq!(s.settings["sr_warmup_exact_ticks"],true);});
        // A file-owned OFF strategy is not activated by a stale global child.
        st.workspace.save_preset(&conduit_core::Preset{name:"SR-OWNER".into(),description:String::new(),
            format:"Synergy".into(),settings:Settings::default(),ea:None}).unwrap();
        st.update(Sections::all(),|s|s.preset_id="SR-OWNER".into());
        assert!(live_sr_start_allowed(&st));
        // Conversely global OFF cannot hide ON in the selected file.
        st.update(Sections::all(),|s|s.settings=serde_json::to_value(Settings::default()).unwrap());
        st.workspace.save_preset(&conduit_core::Preset{name:"SR-OWNER".into(),description:String::new(),
            format:"Synergy".into(),settings:sr_research_config(),ea:None}).unwrap();
        assert!(!live_sr_start_allowed(&st));
    }

    #[test]
    fn live_sr_v2_reload_rejects_doc_and_file_changes_without_resetting_protection() {
        for file_owned in [false,true] {
            let st=sr_manual_state(if file_owned {"sr-v2-file-reload"}else{"sr-v2-doc-reload"});
            let save=|c:Settings|st.workspace.save_preset(&conduit_core::Preset{name:"SR-RELOAD".into(),
                description:String::new(),format:"Synergy".into(),settings:c,ea:None}).unwrap();
            if file_owned {save(Settings::default());st.update(Sections::all(),|s|s.preset_id="SR-RELOAD".into());}
            let mut core=live_core_from_ui(&st.read(|s|s.settings.clone()));
            let mut team=zbuduj_silniki(&st,&core,600.0);
            team.glowny_mut().engine.stats.realized_today=123.45;
            team.glowny_mut().engine.halted=Some("existing DD".into());
            let old=team.glowny().engine.cfg.clone();let before=serde_json::to_value(&team.glowny().engine.stats).unwrap();
            let mut mt=std::collections::HashMap::new();
            if file_owned {save(sr_research_config());mt.insert("SR-RELOAD".into(),std::time::UNIX_EPOCH);}
            else {st.update(Sections::all(),|s|s.settings=serde_json::to_value(sr_research_config()).unwrap());}
            st.update(Sections::one(Section::Halt),|s|s.halt.ustaw(ui::KlasaHaltu::Diagnoza,LIVE_NET_COST_HOLD));
            przeladuj_ustawienia(&st,&mut team,&mut core,old.stops_level,&mut mt);
            let actual=serde_json::to_value(&team.glowny().engine.cfg).unwrap();
            let previous=serde_json::to_value(&old).unwrap();
            let changed:Vec<_>=actual.as_object().unwrap().iter().filter(|(k,v)|previous.get(*k)!=Some(*v)).collect();
            assert!(changed.is_empty(),"old protective strategy file={file_owned}; changed={changed:?}");
            assert_eq!(serde_json::to_value(&team.glowny().engine.stats).unwrap(),before);
            assert_eq!(team.glowny().engine.halted.as_deref(),Some("existing DD"));
            st.read(|s|{assert!(s.halt.diagnoza.contains(LIVE_SR_V2_HOLD));assert!(s.halt.diagnoza.contains(LIVE_NET_COST_HOLD));});
        }
    }

    #[test]
    fn live_sr_v2_parent_off_and_static_children_do_not_hold() {
        for mode in 0..3 {let st=sr_manual_state(&format!("sr-v2-noop-{mode}"));let mut c=sr_research_config();
            match mode {0=>c.sr_warmup_exact_ticks=false,1=>c.trail_sr_enabled=false,_=>c.trail_sr_offset_atr_mult=0.0}
            st.update(Sections::all(),|s|s.settings=serde_json::to_value(&c).unwrap());
            assert!(live_sr_start_allowed(&st));
            assert_eq!(reject_live_sr_transition(&st,c.clone(),&Settings::default()),c);
        }
    }

    #[test]
    fn live_cost_mode_cold_start_holds_without_touching_saved_risk_or_configuration() {
        let st=stan("live-cost-cold-hold");
        st.update(Sections::all(),|s| {
            s.settings["closed_profit_net_costs"]=serde_json::json!(true);
            s.stats.balance=159.8;
            s.halt.ustaw(ui::KlasaHaltu::Ryzyko,"existing DD");
        });
        assert!(!live_cost_start_allowed(&st));
        st.read(|s| {
            assert!(live_net_cost_requested(&s.settings));
            assert_eq!(s.stats.balance,159.8);
            assert!(s.halt.ryzyko.contains("existing DD"));
            assert!(s.halt.diagnoza.contains("brak certyfikowanej migracji"));
        });
    }

    #[test]
    fn live_cost_mode_reload_keeps_existing_legacy_basis_without_resetting_ledger() {
        let st=stan("live-cost-reload-hold");
        let mut core=live_core_from_ui(&st.read(|s|s.settings.clone()));
        let mut silniki=zbuduj_silniki(&st,&core,600.);
        silniki.glowny_mut().engine.stats.realized_today=123.45;
        silniki.glowny_mut().engine.halted=Some("existing DD".into());
        let before=serde_json::to_value(&silniki.glowny().engine.stats).unwrap();
        st.update(Sections::one(Section::Settings),|s|s.settings["closed_profit_net_costs"]=serde_json::json!(true));
        przeladuj_ustawienia(&st,&mut silniki,&mut core,0.,&mut Default::default());
        assert!(!core.closed_profit_net_costs);
        for leg in &silniki.lista { assert!(!leg.engine.cfg.closed_profit_net_costs); }
        assert_eq!(serde_json::to_value(&silniki.glowny().engine.stats).unwrap(),before);
        assert_eq!(silniki.glowny().engine.halted.as_deref(),Some("existing DD"));
        assert!(st.read(|s|live_net_cost_requested(&s.settings)),"requested configuration remains visible");
        let off=serde_json::json!({"closed_profit_net_costs":false,"lot_fixed":0.07});
        assert_eq!(live_core_from_ui(&off),conduit_server::settings_map::core_from_ui(&off));
    }

    #[test]
    fn follow_manual_commands_require_published_session_and_are_rechecked_on_dequeue() {
        use conduit_server::Runtime;
        let st = stan("follow-manual-session");
        let (tx, rx) = std::sync::mpsc::channel();
        let connected = Arc::new(AtomicBool::new(false));
        let runtime = LiveRuntime { tx, connected: connected.clone() };
        let cmd = Command::ClosePosition { ticket: 7 };
        st.update(Sections::all(), |s| {
            s.settings["mt5_follow_terminal_account"] = serde_json::json!(true);
            s.connection.mt5 = "connected".into();
            s.connection.account_verified = "ok".into();
            s.connection.account_session = "A-generation-1".into();
        });
        assert!(runtime.command_scoped(&cmd, &st, Some("A-generation-1")).is_err(), "no commands before publisher activates bridge");
        connected.store(true, Ordering::Release);
        assert!(runtime.command(&cmd, &st).is_err(), "legacy API cannot silently bind a follow intent");
        assert!(runtime.command_scoped(&cmd, &st, None).is_err());
        assert!(runtime.command_scoped(&cmd, &st, Some("B-generation-1")).is_err());
        assert!(rx.try_recv().is_err());
        runtime.command_scoped(&cmd, &st, Some("A-generation-1")).unwrap();
        assert!(command_for_live_session(rx.try_recv().unwrap(), true, "A-generation-2").is_none(), "even same-account reconnect invalidates old intents");
        runtime.command_scoped(&cmd, &st, Some("A-generation-1")).unwrap();
        assert!(matches!(command_for_live_session(rx.try_recv().unwrap(), true, "A-generation-1"), Some(LiveCmd::Panel(Command::ClosePosition { ticket: 7 }))));
        assert!(command_for_live_session(LiveCmd::Panel(cmd.clone()), true, "A-generation-1").is_none());
        assert!(command_for_live_session(LiveCmd::Wykonaj("old-message".into()), true, "A-generation-1").is_none());
        assert!(command_for_live_session(LiveCmd::Scoped { account_session: "A-generation-1".into(), command: Box::new(LiveCmd::Panel(cmd.clone())) }, false, "").is_none(), "turning FOLLOW off must not strip an existing intent binding");
        oznacz_mt5(&st, false);
        assert!(st.read(|s| s.connection.account_session.is_empty()));
        assert!(runtime.command_scoped(&cmd, &st, Some("A-generation-1")).is_err());
        st.update(Sections::all(), |s| s.settings["mt5_follow_terminal_account"] = serde_json::json!(false));
        assert!(runtime.command_scoped(&cmd, &st, Some("A-generation-1")).is_err());
        runtime.command(&cmd, &st).unwrap();
        assert!(matches!(command_for_live_session(rx.try_recv().unwrap(), false, ""), Some(LiveCmd::Panel(Command::ClosePosition { ticket: 7 }))));
        sprzataj(&st);
    }

    #[test]
    fn follow_stale_close_does_not_touch_same_ticket_on_another_session() {
        let mut broker = broker_z_cena();
        let ticket = broker.open_market(OrderReq { side: Side::Buy, volume: 0.01, sl: None, tp: None,
            basket: Some(7), level: 0, is_toucher: false, comment: "account A".into() }).unwrap();
        let stale = LiveCmd::Scoped { account_session: "B-generation-1".into(),
            command: Box::new(LiveCmd::Panel(Command::ClosePosition { ticket })) };
        let mut executions = 0;
        if let Some(LiveCmd::Panel(Command::ClosePosition { ticket })) = command_for_live_session(stale, true, "A-generation-2") {
            executions += 1;
            broker.close_position(ticket, CloseReason::Manual).unwrap();
        }
        assert_eq!(executions, 0);
        assert_eq!(broker.positions().len(), 1);
        assert_eq!(broker.positions()[0].ticket, ticket);
        assert!(broker.drain_closed().is_empty());
        let fresh = LiveCmd::Scoped { account_session: "A-generation-2".into(),
            command: Box::new(LiveCmd::Panel(Command::ClosePosition { ticket })) };
        if let Some(LiveCmd::Panel(Command::ClosePosition { ticket })) = command_for_live_session(fresh, true, "A-generation-2") {
            broker.close_position(ticket, CloseReason::Manual).unwrap();
        }
        assert!(broker.positions().is_empty());
        assert_eq!(broker.drain_closed().len(), 1);
    }

    #[test]
    fn resolved_symbol_follows_pu_vantage_pu_binding_without_mutating_auto_settings() {
        let st = stan("resolved-symbol-switch");
        st.update(Sections::all(), |s| {
            s.settings["mt5_follow_terminal_account"] = serde_json::json!(true);
            s.settings["mt5_symbol"] = serde_json::json!("");
        });
        let settings = st.read(|s| s.settings.clone());
        for (symbol, session, server) in [
            ("XAUUSD.s", "PU-1", "PUPrime-PUBLIC-DEMO"),
            ("XAUUSD", "Vantage-2", "Vantage-PUBLIC-DEMO"),
            ("XAUUSD.s", "PU-3", "PUPrime-PUBLIC-DEMO"),
        ] {
            clear_follow_transients(&st);
            st.read(|s| {
                assert!(s.connection.resolved_symbol.is_empty());
                assert!(s.connection.account_session.is_empty());
                assert!(s.quotes.is_empty());
            });
            // The same binding helper used inside opublikuj's single update.
            st.update(Sections::all(), |s| {
                s.connection.account.server = server.into();
                publish_mt5_binding(&mut s.connection, symbol, session);
            });
            st.read(|s| {
                assert_eq!(s.connection.resolved_symbol, symbol);
                assert_eq!(s.connection.account_session, session);
                assert_eq!(s.connection.account.server, server);
                assert_eq!(s.connection.mt5, "connected");
                assert_eq!(s.settings, settings, "auto is configuration, never the resolved broker symbol");
            });
            oznacz_mt5(&st, false);
            st.read(|s| {
                assert!(s.connection.resolved_symbol.is_empty(), "failed reconnect cannot advertise old contract");
                assert!(s.connection.account_session.is_empty());
                assert_eq!(s.connection.mt5, "disconnected");
            });
        }
        sprzataj(&st);
    }

    #[test]
    fn resolved_symbol_serde_defaults_and_runtime_only_backup() {
        let mut connection = ui::ConnectionState::default();
        let mut old = serde_json::to_value(&connection).unwrap();
        assert_eq!(old["resolvedSymbol"], "");
        old.as_object_mut().unwrap().remove("resolvedSymbol");
        let legacy: ui::ConnectionState = serde_json::from_value(old).unwrap();
        assert!(legacy.resolved_symbol.is_empty());
        publish_mt5_binding(&mut connection, "XAUUSD.s", "PU-session");
        let wire = serde_json::to_value(&connection).unwrap();
        assert_eq!(wire["resolvedSymbol"], "XAUUSD.s");
        assert!(wire.get("resolved_symbol").is_none());
        let st = stan("resolved-symbol-backup");
        st.update(Sections::all(), |s| s.connection = connection);
        let backup = st.read(|s| conduit_server::store::BackupMemory::from_snapshot(s, 1));
        let persisted = serde_json::to_value(&backup).unwrap();
        assert!(persisted.get("connection").is_none());
        assert!(persisted.get("resolvedSymbol").is_none());
        sprzataj(&st);
    }

    #[test]
    fn follow_clear_transients_does_not_depend_on_restored_stats_key() {
        let st = stan("follow-clear-transients");
        let (engines, mut broker) = super::testy_adopcji_lancucha::warsztat();
        let extra = broker.open_market(OrderReq { side: Side::Buy, volume: 0.01, sl: None, tp: None,
            basket: Some(7), level: 0, is_toucher: false, comment: "history fixture".into() }).unwrap();
        broker.close_position(extra, CloseReason::Manual).unwrap();
        let closed = broker.drain_closed();
        st.update(Sections::all(), |s| {
            s.stats.konto_kotwic = "A-already-restored".into();
            s.stats.pnl_today = -37.0;
            s.halt.ustaw(ui::KlasaHaltu::Ryzyko, "A own DD");
            s.positions = broker.positions().iter().map(|p| ui::position_from_core(p,"XAUUSD.s",0.0)).collect();
            s.pendings = broker.pendings().iter().map(|p| ui::pending_from_core(p,"XAUUSD.s")).collect();
            s.baskets = engines.koszyki().iter().map(|b| ui::basket_from_core(b,"XAUUSD.s")).collect();
            s.closed = closed.iter().map(|c| ui::closed_from_core(c,"XAUUSD.s")).collect();
            s.quotes.insert("XAUUSD.s".into(), ui::Quote { symbol: "XAUUSD.s".into(), bid: 4000.0,
                ask: 4000.2, spread: 0.2, time: 1_700_000_000_000, change_pct: 0.0, change: 0.0, day_high: 5000.0, day_low: 3000.0 });
            s.foreign.positions = 9;
            s.connection.account_session = "B-generation".into();
            s.connection.resolved_symbol = "XAUUSD.s".into();
            s.connection.account_verified = "ok".into();
            s.connection.mt5 = "connected".into();
        });
        st.read(|s| assert!(!s.positions.is_empty() && !s.pendings.is_empty() && !s.baskets.is_empty() && !s.closed.is_empty() && !s.quotes.is_empty()));
        clear_follow_transients(&st);
        st.read(|s| {
            assert!(s.positions.is_empty() && s.pendings.is_empty() && s.baskets.is_empty() && s.closed.is_empty() && s.quotes.is_empty() && s.pending_history.is_empty());
            assert_eq!(s.foreign.positions, 0);
            assert_eq!(s.stats.konto_kotwic, "A-already-restored");
            assert_eq!(s.stats.pnl_today, -37.0);
            assert_eq!(s.halt.ryzyko, "A own DD");
            assert_eq!(s.connection.mt5, "disconnected");
            assert!(s.connection.account_session.is_empty() && s.connection.account_verified.is_empty());
            assert!(s.connection.resolved_symbol.is_empty());
        });
        sprzataj(&st);
    }

    #[test]
    fn follow_config_strips_fixed_account_and_real_requires_explicit_flag() {
        let st = stan("follow-config");
        st.update(Sections::one(Section::Settings), |s| {
            s.settings["mt5_follow_terminal_account"] = serde_json::json!(true);
            s.settings["mt5_allow_real_account"] = serde_json::json!(false);
            s.settings["mt5_login"] = serde_json::json!(999);
            s.settings["mt5_server"] = serde_json::json!("old-real");
            s.settings["mt5_symbol"] = serde_json::json!("");
        });
        let cfg = sidecar_config(&st);
        assert!(cfg.follow_terminal_account);
        assert!(!cfg.allow_real_account);
        assert!(cfg.login.is_none() && cfg.server.is_none() && cfg.password.is_none());
        st.update(Sections::one(Section::Settings), |s| s.settings["mt5_follow_terminal_account"] = serde_json::json!(false));
        let legacy = sidecar_config(&st);
        assert_eq!(legacy.login, Some(999));
        assert_eq!(legacy.server.as_deref(), Some("old-real"));
        sprzataj(&st);
    }

    #[test]
    fn follow_ui_scope_resets_foreign_risk_but_preserves_configuration_diagnosis() {
        let st = stan("follow-ui");
        let a = conduit_mt5::proto::AccountIdent { login:42, server:"A".into(), trade_mode:0, ..Default::default() };
        let b = conduit_mt5::proto::AccountIdent { server:"B".into(), ..a.clone() };
        let ka = follow_account_key(&a, "XAUUSD");
        let kb = follow_account_key(&b, "XAUUSD");
        assert_ne!(ka,kb);
        assert_ne!(ka,follow_account_key(&conduit_mt5::proto::AccountIdent { trade_mode:2, ..a.clone() }, "XAUUSD"));
        assert_ne!(ka,follow_account_key(&a, "XAUUSD.s"));
        st.update(Sections::all(), |s| {
            s.stats.konto_kotwic=ka;
            s.stats.pnl_today=123.0;
            s.stats.max_dd_today=88.0;
            s.risk_override.active=true;
            s.halt.ustaw(ui::KlasaHaltu::Ryzyko,"old account risk");
            s.halt.ustaw(ui::KlasaHaltu::Diagnoza,"config must remain blocked");
        });
        reset_follow_ui(&st,&kb,600.0);
        st.read(|s| {
            assert_eq!(s.stats.pnl_today,0.0);
            assert_eq!(s.stats.max_dd_today,0.0);
            assert!(!s.risk_override.active);
            assert!(s.halt.ryzyko.is_empty());
            assert_eq!(s.halt.diagnoza,"config must remain blocked");
        });
        st.update(Sections::all(),|s| s.stats.pnl_today=77.0);
        reset_follow_ui(&st,&kb,600.0);
        assert_eq!(st.read(|s| s.stats.pnl_today),77.0,"same account reconnect preserves own day state");
        sprzataj(&st);
    }

    #[test]
    fn follow_account_a_b_a_restores_own_dd_guard_and_stats_without_commands() {
        let st=stan("follow-risk-return");
        let a=conduit_mt5::proto::AccountIdent{login:42,server:"A".into(),trade_mode:0,..Default::default()};
        let b=conduit_mt5::proto::AccountIdent{server:"B".into(),..a.clone()};
        let mut ea=jeden(silnik());
        ea.glowny_mut().engine.halted=Some("config diagnosis · DD reached".into());
        ea.glowny_mut().engine.stats.realized_today=-100.0;
        ea.glowny_mut().engine.closed_today=vec![-50.0,-50.0];
        ea.glowny_mut().engine.restore_stopped_trading_day(Some(12345));
        st.update(Sections::all(),|s|s.stats.pnl_today=-100.0);
        save_follow_memory(&st,&ea,&a,77,"XAUUSD",900.0,"config diagnosis").unwrap();
        let mut eb=jeden(silnik());
        eb.glowny_mut().engine.stats.realized_today=22.0;
        save_follow_memory(&st,&eb,&b,77,"XAUUSD",700.0,"").unwrap();
        let restored=read_follow_memory(&st,&a,77,"XAUUSD").unwrap().unwrap();
        let mut memory=Trwale::default();
        apply_follow_memory(&st,&mut memory,restored);
        let mut new_a=jeden(silnik());
        przenies_pamiec(&mut new_a,&mut memory,500.0,0.0);
        assert_eq!(new_a.glowny().engine.halted.as_deref(),Some("DD reached"));
        assert_eq!(new_a.glowny().engine.stats.realized_today,-100.0);
        assert_eq!(new_a.glowny().engine.closed_today,vec![-50.0,-50.0]);
        assert_eq!(new_a.glowny().engine.stopped_trading_day(),Some(12345));
        assert_eq!(memory.szczyt_equity,900.0);
        assert!(memory.czekajace.is_empty());
        assert_eq!(st.read(|s|s.stats.pnl_today),-100.0);
        let saved_b=read_follow_memory(&st,&b,77,"XAUUSD").unwrap().unwrap();
        assert!(saved_b.risk_halt.is_empty());
        assert!(saved_b.silniki.values().all(|s|s.stopped_trading_day.is_none()),
            "account A's stop must never appear in account B's persisted memory");
        let mut memory_b=Trwale::default();
        apply_follow_memory(&st,&mut memory_b,saved_b);
        let mut new_b=jeden(silnik());
        przenies_pamiec(&mut new_b,&mut memory_b,500.0,0.0);
        assert_eq!(new_b.glowny().engine.stopped_trading_day(),None);
        sprzataj(&st);
    }

    #[test]
    fn fixed_login_snapshot_binds_account_risk_after_process_restart() {
        let st = stan("fixed-risk-restart");
        let a = conduit_mt5::proto::AccountIdent {
            login:42, server:"fixed-A".into(), trade_mode:0, ..Default::default()
        };
        let b = conduit_mt5::proto::AccountIdent { server:"fixed-B".into(), ..a.clone() };
        let mut live = jeden(silnik());
        live.glowny_mut().engine.restore_stopped_trading_day(Some(12345));
        live.glowny_mut().engine.stats.realized_today = -12.0;
        live.glowny_mut().engine.restore_pending_source_memory(&[
            conduit_core::engine::PendingSourceRecord {
                source: SourceKey::new(-990077, None), msg_id: 77, basket_id: 99,
                aliases: vec![88], cancelled_ts: Some(12345),
            }
        ]);
        st.update(Sections::all(), |s| s.settings["mt5_magic"] = serde_json::json!(88));
        // This is the production fixed-login (follow=false) snapshot path.
        assert!(zapisz_zrzut(&st, &live, &a, "XAUUSD", &mut String::new(),
            false, 77, 600.0, ""));
        let mut memory = Trwale::default(); // fresh process, no RAM carry-over
        assert!(bind_account_risk(&st, &mut memory, &a, 77, "XAUUSD").unwrap());
        let mut restarted = jeden(silnik());
        przenies_pamiec(&mut restarted, &mut memory, 400.0, 0.0);
        assert_eq!(restarted.glowny().engine.stopped_trading_day(), Some(12345));
        assert_eq!(restarted.glowny().engine.stats.realized_today, -12.0);
        assert_eq!(restarted.glowny().engine.export_pending_source_memory().len(), 1);
        assert!(!bind_account_risk(&st, &mut memory, &a, 77, "XAUUSD").unwrap());
        assert!(bind_account_risk(&st, &mut memory, &a, 88, "XAUUSD").unwrap());
        assert!(memory.silniki.is_empty(), "a different magic scope cannot inherit day stop");
        assert!(bind_account_risk(&st, &mut memory, &a, 77, "XAUUSD").unwrap());
        assert!(memory.silniki.values().any(|s| s.stopped_trading_day == Some(12345)));
        assert!(bind_account_risk(&st, &mut memory, &b, 77, "XAUUSD").unwrap());
        let mut other = jeden(silnik());
        przenies_pamiec(&mut other, &mut memory, 400.0, 0.0);
        assert_eq!(other.glowny().engine.stopped_trading_day(), None);
        assert!(other.glowny().engine.export_pending_source_memory().is_empty(),
            "source withdrawal from account A cannot poison account B");
        assert!(bind_account_risk(&st, &mut memory, &a, 77, "XAUUSD").unwrap());
        let mut returned = jeden(silnik());
        przenies_pamiec(&mut returned, &mut memory, 400.0, 0.0);
        assert_eq!(returned.glowny().engine.stopped_trading_day(), Some(12345));
        let sources = returned.glowny().engine.export_pending_source_memory();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].cancelled_ts, Some(12345));
        assert_eq!(sources[0].aliases, vec![88]);
        sprzataj(&st);
    }

    #[test]
    fn follow_corrupt_or_wrong_scope_risk_file_is_error_not_new_account() {
        let st=stan("follow-risk-corrupt");
        let a=conduit_mt5::proto::AccountIdent{login:42,server:"A".into(),trade_mode:0,..Default::default()};
        assert!(read_follow_memory(&st,&a,77,"XAUUSD").unwrap().is_none());
        save_follow_memory(&st,&jeden(silnik()),&a,77,"XAUUSD",600.0,"").unwrap();
        let path=follow_risk_path(&st,&a,77,"XAUUSD");
        let mut forged: Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        forged["server"]=serde_json::json!("foreign");
        conduit_server::store::write_json_atomic(&path,&forged).unwrap();
        assert!(read_follow_memory(&st,&a,77,"XAUUSD").is_err());
        std::fs::write(&path,b"broken-json").unwrap();
        assert!(read_follow_memory(&st,&a,77,"XAUUSD").is_err());
        sprzataj(&st);
    }

    #[test]
    fn follow_failed_snapshot_cannot_be_escaped_by_account_switch() {
        let mut memory=Trwale::default();
        memory.konto="A".into();
        memory.follow_persist_failed=true;
        assert!(!follow_switch_allowed(&memory,"B"));
        assert!(follow_switch_allowed(&memory,"A"),"same account may retry persisting its own protection");
        memory.follow_persist_failed=false;
        assert!(follow_switch_allowed(&memory,"B"));
    }

    #[test]
    fn follow_publisher_pnl_is_own_account_delta_not_balance_difference_or_credit() {
        let st=stan("follow-pnl-own-account");
        let a=conduit_mt5::proto::AccountIdent{login:42,server:"PUPrime".into(),trade_mode:2,..Default::default()};
        let b=conduit_mt5::proto::AccountIdent{login:88,server:"Vantage".into(),trade_mode:0,..Default::default()};
        let publish=|balance:f64,equity:f64,credit:f64,ts:i64| {
            st.update(Sections::all(),|s| {
                s.stats.balance=balance;s.stats.equity=equity;s.stats.credit=credit;
                update_pnl_anchors(&mut s.stats,balance,equity,20696,ts);
            });
        };
        reset_follow_ui(&st,&follow_account_key(&a,"XAUUSD.s"),159.8);
        publish(159.8,459.8,300.0,1000);
        st.read(|s|{assert_eq!(s.stats.pnl_today,0.0);assert_eq!(s.stats.pnl_session,0.0);assert_eq!(s.stats.max_dd_today,0.0);});
        save_follow_memory(&st,&jeden(silnik()),&a,77,"XAUUSD.s",459.8,"").unwrap();
        reset_follow_ui(&st,&follow_account_key(&b,"XAUUSD"),1000.0);
        publish(1000.0,1000.0,0.0,2000);
        st.read(|s|{assert_eq!(s.stats.pnl_today,0.0);assert_eq!(s.stats.pnl_session,0.0);assert_eq!(s.stats.max_dd_today,0.0);});
        publish(1010.0,1010.0,0.0,3000);
        st.read(|s|{assert_eq!(s.stats.pnl_today,10.0);assert_eq!(s.stats.pnl_session,10.0);});
        save_follow_memory(&st,&jeden(silnik()),&b,77,"XAUUSD",1010.0,"").unwrap();
        let mut memory=Trwale::default();
        apply_follow_memory(&st,&mut memory,read_follow_memory(&st,&a,77,"XAUUSD.s").unwrap().unwrap());
        reset_follow_ui(&st,&follow_account_key(&a,"XAUUSD.s"),164.8);
        publish(164.8,464.8,300.0,4000);
        st.read(|s|{
            assert!((s.stats.pnl_today-5.0).abs()<1e-9);
            assert!((s.stats.pnl_session-5.0).abs()<1e-9);
            assert_eq!(s.stats.max_dd_today,0.0);
            assert_eq!(s.stats.max_dd_balance_today,0.0);
            assert_eq!(s.stats.day_start_equity,459.8);
            assert_eq!(s.stats.session_start_equity,459.8);
        });
        sprzataj(&st);
    }

    #[test]
    fn doba_panelu_nie_dodaje_offsetu_do_zegara_brokera_drugi_raz() {
        let cfg = Settings::default();
        assert_eq!(cfg.server_tz_offset_ms, 3 * 3_600_000);
        let dzien = 20_693_i64;
        let polnoc = dzien * 86_400_000;
        assert_eq!(doba_kwotowania_brokera(polnoc, &cfg), dzien);
        assert_eq!(
            doba_kwotowania_brokera(polnoc + 22 * 3_600_000, &cfg),
            dzien
        );
        assert_eq!(
            doba_kwotowania_brokera(polnoc + 86_400_000 - 1, &cfg),
            dzien
        );
        assert_eq!(
            doba_kwotowania_brokera(polnoc + 86_400_000, &cfg),
            dzien + 1
        );
    }

    /// Silnik z włączonym dziennikiem i migawkami rynku.
    fn silnik() -> Engine {
        let mut cfg = Settings::default();
        cfg.journal_enabled = true;
        cfg.journal_snapshots = true;
        Engine::new(cfg, 1000.0)
    }

    fn broker_z_cena() -> SimBroker {
        let mut b = SimBroker::new(1000.0, 0.2, 0.0);
        b.on_quote(Quote {
            ts: 1_700_000_000_000,
            bid: 4000.0,
            ask: 4000.3,
        });
        b
    }

    /// Pojedynczy silnik opakowany tak, jak robi to warstwa żywa przy JEDNYM
    /// formacie handlującym: slot 0, silnik zapasowy, zerowe pułapy.
    fn jeden(engine: Engine) -> routing::Silniki {
        routing::Silniki::pojedynczy(
            engine,
            "ATFX".into(),
            "TEST".into(),
            conduit_core::formaty::Lancuch::default(),
            true,
        )
    }

    #[test]
    fn provenance_bierze_rzeczywista_konfiguracje_nogi_i_routing_bez_dokumentu_sekretow() {
        let st = stan("provenance-snapshot");
        st.update(Sections::one(Section::Settings), |s| {
            s.settings["password"] = serde_json::json!("NIE_WOLNO_ZAPISAC");
            s.bindings.insert(
                "-123".into(),
                ui::ChannelBinding {
                    channel_id: -123,
                    monitored: true,
                    notify: false,
                    format: "ZEN".into(),
                    topics: [("7".into(), "Synergy".into())].into_iter().collect(),
                },
            );
        });

        let mut cfg = Settings::default();
        cfg.lot_fixed = 0.07;
        cfg.session_hours = "9-15".into();
        let mut silniki = jeden(Engine::new(cfg, 1000.0));
        silniki.lancuch = "TEST-CHAIN".into();
        silniki.lista[0].format = "ZEN".into();
        silniki.lista[0].preset = "TEST-PRESET".into();
        silniki.lista[0].engine.set_run_id("live-test/cfg-0/slot-0");

        let snapshot = migawka_provenance(&st, &silniki);
        assert_eq!(snapshot.config["active_chain"], "TEST-CHAIN");
        assert_eq!(snapshot.config["engines"][0]["format"], "ZEN");
        assert_eq!(snapshot.config["engines"][0]["preset"], "TEST-PRESET");
        assert_eq!(snapshot.config["engines"][0]["settings"]["lot_fixed"], 0.07);
        assert_eq!(
            snapshot.config["engines"][0]["settings"]["session_hours"],
            "9-15"
        );
        assert_eq!(snapshot.config["sources"][0]["channel_id"], -123);
        assert_eq!(snapshot.config["sources"][0]["topics"]["7"], "Synergy");
        assert_eq!(
            snapshot.runtime["engine_runs"][0]["journal_run_id"],
            "live-test/cfg-0/slot-0"
        );
        let serialized = serde_json::to_string(&snapshot.config).unwrap();
        assert!(!serialized.contains("NIE_WOLNO_ZAPISAC"));
        sprzataj(&st);
    }

    #[test]
    fn odmowa_brokera_trafia_do_dziennika_z_powodem_i_migawka() {
        let e = silnik();
        let b = broker_z_cena();

        let odmowa = Odmowa {
            op: "zlecenie oczekujące",
            err: BrokerError::InvalidPrice,
            ticket: None,
            basket: Some(7),
            level: Some(2),
            volume: Some(0.03),
            price: Some(4001.0),
            sl: Some(3990.0),
            tp: Some(4010.0),
        };
        let mut sil = jeden(e);
        dziennikuj_odmowy(&mut sil, &b, &[odmowa]);

        let ev = sil.glowny().engine.journal.peek();
        assert_eq!(ev.len(), 1, "odmowa musi dać dokładnie jedno zdarzenie");
        let x = &ev[0];

        assert_eq!(x.kind, EventKind::OrderRejected);
        assert_eq!(x.basket_id, Some(7));
        // POWÓD z zamkniętej listy — bez niego wpisu nie da się policzyć w `jq`
        assert!(x.reason.is_some(), "odmowa bez powodu jest bezużyteczna");

        // MIGAWKA RYNKU: warunki, w których zapadła decyzja
        let m = x.market.as_ref().expect("migawka rynku jest obowiązkowa");
        assert_eq!(m.bid, 4000.0);
        assert_eq!(m.ask, 4000.3);

        // KONTEKST ŻĄDANIA: czego dokładnie odmówiono
        assert_eq!(
            x.data.get("operation").and_then(|v| v.as_str()),
            Some("zlecenie oczekujące")
        );
        assert_eq!(x.data.get("level").and_then(|v| v.as_i64()), Some(2));
        assert_eq!(x.data.get("price").and_then(|v| v.as_f64()), Some(4001.0));
        assert_eq!(x.data.get("sl").and_then(|v| v.as_f64()), Some(3990.0));
        assert_eq!(x.data.get("tp").and_then(|v| v.as_f64()), Some(4010.0));
        assert_eq!(x.data.get("volume").and_then(|v| v.as_f64()), Some(0.03));

        // `RejectCode` skleja warianty; dokładny błąd musi zostać rozróżnialny,
        // bo „zły poziom siatki" i „zły stop-loss" to dwie inne naprawy
        assert_eq!(
            x.data.get("broker_error").and_then(|v| v.as_str()),
            Some("InvalidPrice")
        );
    }

    /// Odmowa dotycząca POZYCJI niesie numer zlecenia, nie koszyk.
    #[test]
    fn odmowa_modyfikacji_niesie_ticket() {
        let e = silnik();
        let b = broker_z_cena();
        let mut o = Odmowa::nowa("modyfikacja pozycji", BrokerError::InvalidStops);
        o.ticket = Some(12345);
        o.sl = Some(4000.1);
        let mut sil = jeden(e);
        dziennikuj_odmowy(&mut sil, &b, &[o]);

        let x = &sil.glowny().engine.journal.peek()[0];
        assert_eq!(x.ticket, Some(12345));
        assert_eq!(x.reason, Some(RejectCode::InvalidStops));
        assert_eq!(
            x.data.get("broker_error").and_then(|v| v.as_str()),
            Some("InvalidStops")
        );
    }


    /// Świeży `StateHandle` na katalogu tymczasowym.
    fn stan(tag: &str) -> StateHandle {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "conduit-halt-{tag}-{}-{}",
            std::process::id(),
            conduit_server::now_ms()
        ));
        let cfg = conduit_server::ServerConfig {
            workspace: p,
            ..Default::default()
        };
        conduit_server::bootstrap(&cfg, conduit_server::default_auth()).unwrap()
    }

    fn sprzataj(st: &StateHandle) {
        let _ = std::fs::remove_dir_all(st.workspace.root.clone());
    }

    /// Zespół jednonogi z nałożoną blokadą i stanem panelu opisującym ją
    /// tak, jak zrobiłaby to publikacja z pętli handlowej.
    fn zatrzymany_zespol(
        st: &StateHandle,
        halted: &str,
        diagnoza: &str,
        ryzyko: &str,
    ) -> routing::Silniki {
        let mut sil = jeden(silnik());
        sil.glowny_mut().engine.halted = Some(halted.to_string());
        st.update(Sections::one(Section::Halt), |s| {
            s.halt.ustaw(ui::KlasaHaltu::Diagnoza, diagnoza);
            s.halt.ustaw(ui::KlasaHaltu::Ryzyko, ryzyko);
        });
        sil
    }

    /// TEST 3. ZDJĘCIE ZATRZYMANIA KLASY DIAGNOZA NIE RUSZA `risk_override`.
    #[test]
    fn zdjecie_diagnozy_nie_rozbraja_straznika() {
        let st = stan("diag");
        const POWOD: &str = "konfiguracja nie została wczytana";
        let mut sil = zatrzymany_zespol(&st, POWOD, POWOD, "");
        let mut diagnoza = POWOD.to_string();

        wznow_handel(&st, &mut sil, &mut diagnoza, 1_700_000_000_000).unwrap();

        assert!(
            !sil.glowny().engine.risk_override,
            "zdjęcie DIAGNOZY nie ma prawa wyłączyć max_dd_pct / max_dd_usd / pułapów łańcucha"
        );
        assert!(
            !st.read(|s| s.risk_override.active),
            "panel też nie ma pokazywać rozbrojenia"
        );
        assert!(
            sil.glowny().engine.halted.is_none(),
            "silnik ma znów handlować"
        );
        assert!(!st.read(|s| s.halt.active), "baner zatrzymania ma zgasnąć");
        assert!(
            diagnoza.is_empty(),
            "publikacja nie może dalej raportować zdjętej diagnozy"
        );
        sprzataj(&st);
    }

    /// TEST 4. ZDJĘCIE ZATRZYMANIA KLASY RYZYKO DALEJ WYŁĄCZA STRAŻNIKA.
    ///
    /// To zachowanie jest SŁUSZNE i ma zostać: wznowienie handlu mimo
    /// przekroczonego `max_dd_pct` jest świadomą decyzją o graniu bez hamulca
    /// i musi być widoczne w panelu oraz w poczcie.
    #[test]
    fn zdjecie_ryzyka_dalej_wylacza_straznika() {
        let st = stan("ryz");
        const POWOD: &str = "MAX DRAWDOWN 41.2% ≥ 40.0%";
        let mut sil = zatrzymany_zespol(&st, POWOD, "", POWOD);
        let mut diagnoza = String::new();

        wznow_handel(&st, &mut sil, &mut diagnoza, 1_700_000_000_000).unwrap();

        assert!(
            sil.glowny().engine.risk_override,
            "to jest dotychczasowa, słuszna ścieżka"
        );
        assert!(
            st.read(|s| s.risk_override.active),
            "panel MUSI krzyczeć o rozbrojeniu"
        );
        assert_eq!(st.read(|s| s.risk_override.reason.clone()), POWOD);
        assert!(sil.glowny().engine.halted.is_none());
        sprzataj(&st);
    }

    /// TEST 5. OBA NARAZ — zdejmujemy TO, O CO UŻYTKOWNIK PROSIŁ.
    ///
    /// Konto stoi po obsunięciu (RYZYKO, przeżyło restart) i dodatkowo dziś
    /// rano nie wczytała się konfiguracja (DIAGNOZA). Pierwsze kliknięcie
    /// zdejmuje TYLKO diagnozę: strażnik zostaje uzbrojony, zatrzymanie od
    /// obsunięcia dalej blokuje nowe wejścia, a dziennik mówi o tym wprost.
    #[test]
    fn oba_naraz_zdjecie_diagnozy_zostawia_ryzyko() {
        let st = stan("oba");
        const D: &str = "konfiguracja nie została wczytana";
        const R: &str = "MAX DRAWDOWN 41.2% ≥ 40.0%";
        let mut sil = zatrzymany_zespol(&st, &format!("{D}{}{R}", ui::HALT_SEP), D, R);
        let mut diagnoza = D.to_string();

        wznow_handel(&st, &mut sil, &mut diagnoza, 1_700_000_000_000).unwrap();

        assert!(
            !sil.glowny().engine.risk_override,
            "strażnik ma zostać uzbrojony"
        );
        assert!(!st.read(|s| s.risk_override.active));
        assert_eq!(
            sil.glowny().engine.halted.as_deref(),
            Some(R),
            "zatrzymanie od obsunięcia ZOSTAJE — restart ani klik w diagnozę go nie zdejmują"
        );
        assert!(st.read(|s| s.halt.active), "baner ma dalej stać");
        assert_eq!(
            st.read(|s| s.halt.powod(ui::KlasaHaltu::Ryzyko).to_string()),
            R
        );
        assert!(!st.read(|s| s.halt.ma(ui::KlasaHaltu::Diagnoza)));
        assert!(
            st.read(|s| s
                .logs
                .iter()
                .any(|l| l.content.contains("KLASY RYZYKO ZOSTAJE"))),
            "cisza zakazana: użytkownik musi wiedzieć, że drugie zatrzymanie zostaje"
        );
        sprzataj(&st);
    }

    /// Rozbicie na klasy: co jest diagnozą tej pętli, a co powodem od strażnika.
    #[test]
    fn rozbicie_na_klasy_odroznia_diagnoze_od_ryzyka() {
        const D: &str = "rachunek zdejmuje bezpiecznik nodze STORM (expo_cap_pct)";
        let mut sil = jeden(silnik());

        sil.glowny_mut().engine.halted = Some(D.into());
        assert_eq!(
            rozbij_klasy_zatrzymania(&sil, D),
            (D.to_string(), String::new())
        );

        sil.glowny_mut().engine.halted = Some(format!("{D}{}MAX DD 40%", ui::HALT_SEP));
        assert_eq!(
            rozbij_klasy_zatrzymania(&sil, D),
            (D.to_string(), "MAX DD 40%".to_string()),
            "reszta po odjęciu diagnozy pochodzi od strażnika"
        );

        sil.glowny_mut().engine.halted = Some("MAX DD 40%".into());
        assert_eq!(
            rozbij_klasy_zatrzymania(&sil, D),
            (String::new(), "MAX DD 40%".to_string()),
            "diagnoza, której w silniku NIE MA, nie ma prawa się pojawić w stanie"
        );
    }

    #[test]
    fn pamiec_rekonektu_gubi_diagnoze_a_trzyma_ryzyko() {
        const D: &str = "TERMINAL NA INNYM KONCIE: oczekiwano 111, terminal jest na 222";
        const R: &str = "MAX DRAWDOWN 41.2% ≥ 40.0%";
        let mut stare = jeden(silnik());
        stare.glowny_mut().engine.halted = Some(format!("{D}{}{R}", ui::HALT_SEP));

        let mut trwale = Trwale::default();
        zapamietaj_silniki(&mut trwale, &mut stare, 1000.0, D);
        assert_eq!(
            trwale.silniki.get("ATFX").and_then(|t| t.halted.clone()),
            Some(R.to_string()),
            "pamięć ma nieść wyłącznie zdanie o RACHUNKU"
        );

        // nowe podejście do mostu: świeża diagnoza NOWEGO składu
        const D2: &str = "rachunek zdejmuje bezpiecznik nodze STORM (expo_cap_pct)";
        let mut nowe = jeden(silnik());
        nowe.glowny_mut().engine.halted = Some(D2.into());
        przenies_pamiec(&mut nowe, &mut trwale, 1000.0, 0.0);
        assert_eq!(
            nowe.glowny().engine.halted.as_deref(),
            Some(format!("{D2}{}{R}", ui::HALT_SEP).as_str()),
            "obie prawdy naraz: świeża diagnoza I powód od strażnika"
        );
        let (d, r) = rozbij_klasy_zatrzymania(&nowe, D2);
        assert_eq!((d.as_str(), r.as_str()), (D2, R));
    }

    #[test]
    fn przy_wylaczonym_strazniku_alarm_i_tak_wychodzi() {
        // próg 15 %, nic jeszcze nie wysłano
        assert!(
            !alarm_dd_nalezy_sie(14.9, 15.0, 0.0),
            "poniżej progu — cisza"
        );
        assert!(alarm_dd_nalezy_sie(15.0, 15.0, 0.0), "na progu — mail");
        assert!(
            alarm_dd_nalezy_sie(41.0, 15.0, 0.0),
            "głęboko poniżej — mail"
        );
    }

    /// Bez hamulca obsunięcie idzie dziesiątkami procent. Kolejny mail dopiero
    /// po pogłębieniu o 5 pp, inaczej alarm zamienia się w zalew i przestaje
    /// cokolwiek znaczyć.
    #[test]
    fn kolejny_alarm_dopiero_po_pogłebieniu() {
        assert!(!alarm_dd_nalezy_sie(17.0, 15.0, 15.0), "+2 pp to za mało");
        assert!(alarm_dd_nalezy_sie(20.0, 15.0, 15.0), "+5 pp — mail");
        assert!(!alarm_dd_nalezy_sie(24.9, 15.0, 20.0));
        assert!(alarm_dd_nalezy_sie(25.0, 15.0, 20.0));
    }

    /// SPOKOJNA NOC TO NIE AWARIA.
    ///
    /// Kanał sygnałowy potrafi milczeć pół nocy. Alarm po samej ciszy
    /// przychodziłby codziennie i przestałby cokolwiek znaczyć. Awarią jest
    /// dopiero cisza, przy której keepalive NIE POTWIERDZA, że gniazdo żyje —
    /// bo martwe gniazdo MTProto nie zgłasza błędu samo z siebie.
    #[test]
    fn cisza_przy_zywym_pingu_to_nie_awaria() {
        // Noc i weekend: cisza dowolnej długości NIE jest awarią.
        assert!(
            !alarm_telegram_nalezy_sie(600.0, 0, false),
            "dziesięć godzin ciszy przy zamkniętym rynku to spokojna noc"
        );
        assert!(
            !alarm_telegram_nalezy_sie(300.0, 0, true),
            "pięć godzin ciszy mieści się w zmierzonej normie kanału"
        );
        assert!(
            alarm_telegram_nalezy_sie(330.0, 0, true),
            "cisza ponad zmierzone maksimum"
        );
        assert!(
            alarm_telegram_nalezy_sie(5.0, PINGOW_DO_ALARMU, false),
            "martwe gniazdo jest awarią nawet w nocy i nawet tuż po wiadomości"
        );
        assert!(alarm_telegram_nalezy_sie(120.0, 5, true));
        assert!(
            !alarm_telegram_nalezy_sie(5.0, 1, false),
            "jeden ping to czkawka sieci"
        );
    }

    /// Próg jest w minutach i ma być na tyle wysoki, żeby nie łapać przerwy
    /// w sesji, i na tyle niski, żeby nie przespać całej nocy.
    #[test]
    fn prog_ciszy_telegrama_jest_rozsadny() {
        assert!(
            CISZA_TELEGRAM_MIN > 310.0,
            "próg poniżej zmierzonego maksimum dałby fałszywe alarmy w środku dnia"
        );
        assert!(
            CISZA_TELEGRAM_MIN <= 480.0,
            "powyżej ośmiu godzin alarm budzi za późno"
        );
        // Poniżej progu i przy zdrowym pingu — cisza nie alarmuje.
        assert!(!alarm_telegram_nalezy_sie(
            CISZA_TELEGRAM_MIN - 0.1,
            0,
            true
        ));
    }

    // ========================================================
    //  BRAMKA WIEKU SYGNAŁU
    // ========================================================

    /// Sedno bramki: sygnał, który przeleżał przerwę w moście, NIE MOŻE zostać
    /// otwarty po nowej cenie. Odbudowa mostu trwa co najmniej
    /// `CISZA_ODBUDOWA_MIN` minut, więc wszystko, co przez nią przeszło,
    /// musi wpaść w bramkę.
    #[test]
    fn sygnal_z_czasu_przerwy_w_moscie_nie_przechodzi() {
        let teraz = 1_785_759_000_000i64;
        let min = |m: i64| teraz - m * 60_000;

        assert!(
            wiek_ponad_prog(teraz, min(0), SYGNAL_MAX_WIEK_MIN).is_none(),
            "świeży sygnał musi przechodzić"
        );
        assert!(
            wiek_ponad_prog(teraz, min(4), SYGNAL_MAX_WIEK_MIN).is_none(),
            "cztery minuty to jeszcze normalna praca"
        );
        // Dokładnie na progu przepuszczamy — bramka ma się odzywać dopiero
        // POWYŻEJ, tak jak wszystkie pozostałe progi w tym pliku.
        assert!(wiek_ponad_prog(teraz, min(5), SYGNAL_MAX_WIEK_MIN).is_none());

        let w = wiek_ponad_prog(teraz, min(13), SYGNAL_MAX_WIEK_MIN)
            .expect("sygnał sprzed 13 minut musi zostać zatrzymany");
        assert!(
            (w - 13.0).abs() < 1e-9,
            "wiek ma być podany w minutach: {w}"
        );

        // To jest ten przypadek, dla którego bramka powstała: pełna odbudowa
        // mostu po ciszy w kwotowaniach nigdy nie trwa krócej niż 12 minut.
        assert!(
            wiek_ponad_prog(teraz, min(CISZA_ODBUDOWA_MIN as i64), SYGNAL_MAX_WIEK_MIN).is_some(),
            "próg wieku MUSI być krótszy od czasu odbudowy mostu, \
             inaczej cała kolejka z przerwy wchodzi do handlu"
        );
        assert!(
            SYGNAL_MAX_WIEK_MIN < CISZA_ODBUDOWA_MIN,
            "gdyby próg był dłuższy od odbudowy mostu, bramka nie robiłaby nic"
        );
    }

    /// `0` znaczy „nie chcę bramki", a nie „odrzucaj wszystko".
    /// To ta sama pułapka co `reenter_max` i pułapy globalne.
    #[test]
    fn zerowy_prog_wieku_wylacza_bramke() {
        let teraz = 1_785_759_000_000i64;
        assert!(wiek_ponad_prog(teraz, teraz - 10 * 3_600_000, 0.0).is_none());
        assert!(wiek_ponad_prog(teraz, teraz - 10 * 3_600_000, -1.0).is_none());
        // Brak znacznika czasu (stary zapis, ręczny bilet) też nie może
        // wyglądać jak wiadomość z 1970 roku.
        assert!(wiek_ponad_prog(teraz, 0, SYGNAL_MAX_WIEK_MIN).is_none());
    }

    #[test]
    fn czujnik_lapie_ksztalt_sygnalu_bez_odczytu() {
        // czytelne — parser wyciąga wejście, więc żadnego alarmu
        assert!(!nieczytelny_sygnal(
            "XAUUSD Buy 4100\n\n🥇TP1 4103\n🥈TP2 4106\n🥉TP3 4110\n🏅TP4 Open\n\n🚫SL 4094"
        ));
        assert!(!nieczytelny_sygnal(
            "BUY LIMITS GOLD @ 4100/4095 AREA\n\nTP 4103\nTP OPEN\nSL 4094"
        ));
        // proza i komunikaty zarządzające NIE mają kształtu wejścia
        assert!(!nieczytelny_sygnal("TP1 HIT 🔥"));
        assert!(!nieczytelny_sygnal("GOOD MORNING TRADERS"));
        assert!(!nieczytelny_sygnal(
            "We are looking for a buy around the 4250 mark"
        ));
        // kształt jest (cel + stop), a treści nie da się rozebrać —
        // dokładnie ten stan ma krzyczeć
        assert!(nieczytelny_sygnal(
            "ZLOTO KUPUJEMY\n\nCEL: TP 4283\nSTOP: SL 4274"
        ));
    }

    #[test]
    fn znacznik_z_przyszlosci_nie_jest_przeterminowany() {
        let teraz = 1_785_759_000_000i64;
        assert!(wiek_ponad_prog(teraz, teraz + 90_000, SYGNAL_MAX_WIEK_MIN).is_none());
    }

    /// BRAMKA DOTYCZY WYŁĄCZNIE OTWARĆ.
    ///
    /// Odrzucenie starego „CLOSE ALL" byłoby gorsze od wykonania go: pozycja
    /// zostałaby na rachunku bez opieki, choć autor kazał ją zamknąć.
    #[test]
    fn bramka_wieku_nie_dotyka_komunikatow_zarzadzajacych() {
        let wejscie = "BUY GOLD @ 4100/4105\nSL 4099\nTP 4108\nTP 4112\nTP 4117";
        assert!(otwiera_koszyk(wejscie, None), "to jest otwarcie koszyka");

        for zarzadzajacy in [
            "TP1 HIT",
            "+30 PIPS HIT\n\nRISK FREE 4103",
            "CLOSE ALL",
            "OUT AT ENTRY",
            "CANCEL THE LIMITS",
        ] {
            assert!(
                !otwiera_koszyk(zarzadzajacy, None),
                "komunikat zarządzający NIE MOŻE wpaść w bramkę wieku: {zarzadzajacy}"
            );
        }

        // Edycja starej wiadomości jest z natury „stara", a niesie poprawkę
        // do koszyka, który już stoi.
        assert!(
            !otwiera_koszyk(wejscie, Some(1234)),
            "edycja przechodzi niezależnie od wieku"
        );
    }

    /// `alert_dd_pct = 0` musi znaczyć „nie chcę tych ostrzeżeń", a nie
    /// „ostrzegaj od zera procent".
    #[test]
    fn zerowy_prog_alarmu_wylacza_ostrzezenia() {
        assert!(!alarm_dd_nalezy_sie(50.0, 0.0, 0.0));
        assert!(!alarm_dd_nalezy_sie(99.0, -1.0, 0.0));
    }

    /// Wyłączony dziennik nie może kosztować ani jednej alokacji zdarzenia.
    #[test]
    fn wylaczony_dziennik_nie_zapisuje_nic() {
        let mut cfg = Settings::default();
        cfg.journal_enabled = false;
        let e = Engine::new(cfg, 1000.0);
        let b = broker_z_cena();
        let mut sil = jeden(e);
        dziennikuj_odmowy(
            &mut sil,
            &b,
            &[Odmowa::nowa("otwarcie rynkowe", BrokerError::Rejected)],
        );
        assert!(sil.glowny().engine.journal.is_empty());
    }

    /// Każda odmowa z partii to osobny wiersz — sklejenie ich w jeden wpis
    /// ukryłoby, ILE poziomów siatki broker odrzucił.
    #[test]
    fn kazda_odmowa_to_osobny_wpis() {
        let e = silnik();
        let b = broker_z_cena();
        let bledy: Vec<Odmowa> = (0..4)
            .map(|i| {
                let mut o = Odmowa::nowa("zlecenie oczekujące", BrokerError::InvalidPrice);
                o.basket = Some(3);
                o.level = Some(i);
                o
            })
            .collect();
        let mut sil = jeden(e);
        dziennikuj_odmowy(&mut sil, &b, &bledy);
        assert_eq!(sil.glowny().engine.journal.peek().len(), 4);
    }

    fn im(msg_id: i64, edit_of: Option<i64>, tekst: &str) -> IncomingMessage {
        IncomingMessage {
            ts: 1_700_000_000_000,
            source: SourceKey::new(-100, None),
            source_name: "TEST".into(),
            msg_id,
            reply_to: None,
            edit_of,
            text: tekst.into(),
        }
    }

    #[test]
    fn edycja_bez_zmiany_tresci_jest_odsiewana() {
        let mut p = PamiecTresci::new();
        let nowa = im(6846, None, "PREPARE FOR BUY LIMITS");
        assert!(
            !p.duplikat_tresci(&nowa),
            "pierwsza dostawa zawsze przechodzi"
        );

        let noop = im(6846, Some(6846), "PREPARE FOR BUY LIMITS");
        assert!(
            p.duplikat_tresci(&noop),
            "edycja bez zmiany treści to duplikat"
        );

        // realna edycja (autor dopisał poziomy) MUSI przejść…
        let realna = im(6846, Some(6846), "PREPARE FOR BUY LIMITS\nTP 4650");
        assert!(
            !p.duplikat_tresci(&realna),
            "zmiana treści nie jest duplikatem"
        );
        // …a jej powtórka już nie.
        assert!(p.duplikat_tresci(&realna));

        // edycja NIEZNANEGO msg_id (np. po restarcie procesu) przechodzi —
        // nie da się udowodnić, że niczego nie zmienia
        let sierota = im(7000, Some(7000), "TP1 HIT");
        assert!(!p.duplikat_tresci(&sierota));
    }

    /// Ta sama treść pod innym `msg_id` albo z innego źródła to NIE duplikat —
    /// kanały potrafią wysłać identyczny komunikat („TP1 HIT") dwa razy
    /// naprawdę i oba są osobnymi zdarzeniami.
    #[test]
    fn ta_sama_tresc_innego_msg_id_nie_jest_duplikatem() {
        let mut p = PamiecTresci::new();
        assert!(!p.duplikat_tresci(&im(1, None, "TP1 HIT")));
        assert!(!p.duplikat_tresci(&im(2, None, "TP1 HIT")));
        let z_forum = IncomingMessage {
            source: SourceKey::new(-100, Some(7)),
            ..im(1, None, "TP1 HIT")
        };
        assert!(!p.duplikat_tresci(&z_forum), "temat forum to osobne źródło");
    }

    #[test]
    fn odrzut_bramki_wieku_zostawia_slad_w_dzienniku_i_lejku() {
        let e = silnik();
        let b = broker_z_cena();
        let mut sil = jeden(e);
        let m = im(6900, None, "BUY GOLD @ 4001/3999\nTP 4010\nSL 3980");

        dziennikuj_odrzut_wieku(&mut sil, &b, &m, None, 13.0, 5.0);

        let engine = &sil.glowny().engine;
        assert_eq!(
            engine.odrzuty.get("StaleSignal").copied(),
            Some(1),
            "licznik lejka musi urosnąć — ten sam kubełek co bramka silnika"
        );
        let ev = engine.journal.peek();
        assert_eq!(ev.len(), 1, "odrzut musi dać dokładnie jedno zdarzenie");
        assert_eq!(ev[0].kind, EventKind::SignalRejected);
        assert_eq!(ev[0].msg_id, Some(6900));
        assert!(
            ev[0].reason.is_some(),
            "odrzut bez powodu jest bezużyteczny"
        );
        assert!(ev[0].market.is_some(), "migawka rynku jest obowiązkowa");
    }

    /// Licznik rośnie także przy WYCISZONYM dzienniku — dziennik można
    /// wyłączyć, ewidencji lejka nie (ta sama zasada co licznik `BrakTrasy`).
    #[test]
    fn odrzut_wieku_liczy_sie_takze_bez_dziennika() {
        let mut cfg = Settings::default();
        cfg.journal_enabled = false;
        let e = Engine::new(cfg, 1000.0);
        let b = broker_z_cena();
        let mut sil = jeden(e);

        dziennikuj_odrzut_wieku(&mut sil, &b, &im(1, None, "x"), None, 30.0, 5.0);

        let engine = &sil.glowny().engine;
        assert_eq!(engine.odrzuty.get("StaleSignal").copied(), Some(1));
        assert!(
            engine.journal.is_empty(),
            "wyciszony dziennik zostaje pusty"
        );
    }
}

// ============================================================
//  BATERIA DOWODOWA DRABINKI — adopcja koszyków przy zmianie łańcucha
// ============================================================
//
// Testy wołają DOKŁADNIE te funkcje, którymi jedzie żywa przebudowa
// (`zapamietaj_silniki`, `silniki_zamrozone`, `przenies_pamiec`,
// `wznowienie::odtworz`, `Silniki::rozdaj_koszyki`) — bez `StateHandle`,
// który jest wyłącznie klejem logującym. Kopia logiki w teście byłaby
// testem kopii, nie kodu.
#[cfg(test)]
mod testy_adopcji_lancucha {
    use super::*;
    use conduit_backtest::sim::SimBroker;
    use conduit_core::broker::Broker;
    use conduit_core::formaty::{Lancuch, PulapyGlobalne};
    use conduit_core::settings::Settings;
    use std::collections::BTreeMap;

    const MAGIC: i64 = 770_077;
    const LOGIN: i64 = 10_000_001;
    const T0: Ts = 1_700_000_000_000;

    fn kwot(ts: Ts, bid: f64) -> Quote {
        Quote {
            ts,
            bid,
            ask: bid + 0.20,
        }
    }

    fn ustawienia() -> Settings {
        let mut s = Settings::default();
        s.session_filter = false;
        s.regime_filter = conduit_core::settings::RegimeFilter::Off;
        s.max_open_positions = 20;
        s.max_open_baskets = 10;
        // dwa poziomy siatki: dotknięcie górnego zostawia dolny jako ŻYWY
        // pending — dokładnie stan „koszyk częściowo wypełniony" z baterii
        s.entry_units = 2;
        s
    }

    fn lancuch(nazwa: &str, formaty: &[&str]) -> Lancuch {
        let mut l = Lancuch {
            nazwa: nazwa.into(),
            ..Default::default()
        };
        for f in formaty {
            l.presety.insert((*f).to_string(), format!("P-{f}"));
        }
        l
    }

    fn presety(formaty: &[&str]) -> BTreeMap<String, Settings> {
        formaty
            .iter()
            .map(|f| {
                let mut u = ustawienia();
                // ZEN wchodzi po rynku (pozycje otwarte w warsztacie),
                // Synergy siatką limitów — obie klasy stanu w jednej adopcji.
                if *f == "ZEN" {
                    u.auto_limit = false;
                }
                (format!("P-{f}"), u)
            })
            .collect()
    }

    fn zespol(nazwa: &str, formaty: &[&str], saldo: f64) -> routing::Silniki {
        let (s, braki) = routing::Silniki::zbuduj(
            &lancuch(nazwa, formaty),
            &presety(formaty),
            &ustawienia(),
            saldo,
        );
        assert!(braki.is_empty());
        s
    }

    fn wiadomosc(ts: Ts, id: i64, tekst: &str) -> IncomingMessage {
        IncomingMessage {
            ts,
            source: SourceKey::new(-1000 - id, None),
            source_name: "TEST".into(),
            msg_id: id,
            reply_to: None,
            edit_of: None,
            text: tekst.into(),
        }
    }

    /// Sygnał LIMIT — siatka zleceń oczekujących (koszyk Armed).
    const SYGNAL_LIMIT: &str = "BUY LIMIT GOLD @ 3995/3990\nTP 4010\nTP 4020\nSL 3980";
    /// Sygnał rynkowy — pozycja otwarta od razu.
    const SYGNAL_RYNKOWY: &str = "BUY GOLD @ 4001/3999\nTP 4010\nTP 4020\nSL 3980";

    /// Pełna adopcja tą samą ścieżką co żywa przebudowa. Zwraca nowy zespół.
    fn przebuduj_testowo(
        stary: &mut routing::Silniki,
        broker: &SimBroker,
        nowy_lancuch: &str,
        nowe_formaty: &[&str],
        trwale: &mut Trwale,
    ) -> (
        routing::Silniki,
        wznowienie::Wynik,
        routing::RaportPrzydzialu,
    ) {
        let stare: Vec<(String, String, u32, conduit_core::Settings)> = stary
            .lista
            .iter()
            .map(|s| {
                (
                    s.format.clone(),
                    s.preset.clone(),
                    s.slot,
                    s.engine.cfg.clone(),
                )
            })
            .collect();
        zapamietaj_silniki(trwale, stary, broker.equity(), "");

        let saldo = broker.account().balance;
        let mut nowe = zespol(nowy_lancuch, nowe_formaty, saldo);

        // slot 0 wraca do swojego formatu (ta sama reguła co w przebudowie)
        if let Some((format0, _, _, _)) = stare.iter().find(|(_, _, slot, _)| *slot == 0) {
            if let Some(i) = nowe.indeks_formatu(format0) {
                for s in nowe.lista.iter_mut() {
                    s.zapasowy = false;
                }
                nowe.lista[i].zapasowy = true;
            }
        }

        let pulapy = nowe.pulapy.clone();
        let (zamrozone, kolizje) = silniki_zamrozone(
            &stare,
            &trwale.koszyki,
            &nowe,
            saldo,
            broker.account().credit,
            "test",
            &pulapy,
        );
        assert!(kolizje.is_empty(), "kolizje slotów w teście: {kolizje:?}");
        for z in zamrozone {
            nowe.lista.push(z);
        }
        przenies_pamiec(&mut nowe, trwale, saldo, broker.account().credit);

        let zrzut = wznowienie::Zrzut {
            wersja: wznowienie::WERSJA,
            zapisano: conduit_server::now_ms(),
            login: LOGIN,
            magic: MAGIC,
            symbol: "XAUUSD".into(),
            next_basket_id: trwale.next_basket_id,
            koszyki: trwale.koszyki.clone(),
        };
        let w = wznowienie::odtworz(
            Some(zrzut),
            broker.positions(),
            broker.pendings(),
            LOGIN,
            MAGIC,
            "XAUUSD",
            "CD",
        );
        let raport = nowe.rozdaj_koszyki(w.koszyki.clone());
        (nowe, w, raport)
    }

    /// Warsztat: dwa formaty (Synergy+ZEN), w ZEN pozycja otwarta,
    /// w Synergy żywa siatka pendingów, w ZEN drugi koszyk częściowo
    /// wypełniony (pozycja + pendingi).
    pub(super) fn warsztat() -> (routing::Silniki, SimBroker) {
        let mut z = zespol("SENTINEL-0", &["Synergy", "ZEN"], 1000.0);
        let mut b = SimBroker::new(1000.0, 0.2, 0.0);
        b.on_quote(kwot(T0, 4000.0));

        let i_zen = z.indeks_formatu("ZEN").unwrap();
        let i_syn = z.indeks_formatu("Synergy").unwrap();

        // ZEN: koszyk 1 — pozycja rynkowa
        z.lista[i_zen]
            .engine
            .on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_RYNKOWY));
        // Synergy: koszyk — siatka limitów (Armed)
        z.lista[i_syn]
            .engine
            .on_message(&mut b, &wiadomosc(T0 + 1_000, 2, SYGNAL_LIMIT));
        // ZEN: koszyk 2 — siatka, potem dotknięcie jednego poziomu (częściowy fill)
        z.lista[i_zen]
            .engine
            .on_message(&mut b, &wiadomosc(T0 + 2_000, 3, SYGNAL_LIMIT));
        let q = kwot(T0 + 3_000, 3994.8); // dotyka górnego poziomu strefy
        b.on_quote(q);
        for s in z.lista.iter_mut() {
            s.engine.on_tick(&mut b, &q);
        }
        (z, b)
    }

    /// KONTRAKT A+B: przejście „w dół" (SENTINEL-0 → ZENONLY3): noga ZEN
    /// przejmuje swoje koszyki, Synergy zostaje ZAMROŻONE ze starą
    /// konfiguracją; zero sierot, zero podwójnych, zero porzuconych.
    #[test]
    fn zejscie_w_dol_adoptuje_wszystko_i_zamraza_forme_bez_nogi() {
        let (mut a, b) = warsztat();
        let przed: std::collections::BTreeSet<u32> = a.koszyki().iter().map(|k| k.id).collect();
        assert!(
            przed.len() >= 3,
            "warsztat ma dać ≥3 koszyki, jest {}",
            przed.len()
        );
        assert!(!b.positions().is_empty() && !b.pendings().is_empty());

        // znacznik do sprawdzenia transferu pamięci
        let i_zen = a.indeks_formatu("ZEN").unwrap();
        a.lista[i_zen].engine.stats.realized_today = 123.45;

        let mut trwale = Trwale::default();
        let (nowe, w, raport) = przebuduj_testowo(&mut a, &b, "ZENONLY3", &["ZEN"], &mut trwale);

        // zero sierot i porzuconych, komplet adoptowany dokładnie raz
        assert_eq!(w.sieroty, 0, "sieroty na brokerze");
        assert!(
            raport.porzucone.is_empty(),
            "koszyki bez opiekuna: {:?}",
            raport.porzucone
        );
        let po: Vec<u32> = nowe.koszyki().iter().map(|k| k.id).collect();
        let po_zbior: std::collections::BTreeSet<u32> = po.iter().copied().collect();
        assert_eq!(po.len(), po_zbior.len(), "koszyk zaadoptowany PODWÓJNIE");
        assert_eq!(po_zbior, przed, "zbiór koszyków po adopcji ≠ przed");

        // Synergy: silnik zamrożony, ze SWOIM koszykiem, bez nowych sygnałów
        let i_syn = nowe
            .indeks_formatu("Synergy")
            .expect("zamrożony silnik Synergy");
        assert!(nowe.lista[i_syn].tylko_zarzadzanie);
        assert!(
            !nowe.lista[i_syn].engine.baskets.is_empty(),
            "zamrożony trzyma swój koszyk"
        );
        assert!(
            matches!(
                nowe.trasa(&SourceKey::new(-1002, None), Some("Synergy".into())),
                Err(conduit_core::routing::BrakTrasy::FormatNieHandluje { .. })
            ),
            "zamrożony format NIE MA prawa brać nowych sygnałów"
        );
        // ZEN handluje dalej
        assert!(nowe
            .trasa(&SourceKey::new(-1001, None), Some("ZEN".into()))
            .is_ok());

        // pamięć formatu przeniesiona (wynik dnia ZEN przetrwał przebudowę)
        let i_zen2 = nowe.indeks_formatu("ZEN").unwrap();
        assert_eq!(nowe.lista[i_zen2].engine.stats.realized_today, 123.45);

        // brak kolizji numeracji: świeży koszyk dostaje numer spoza starych
        for s in &nowe.lista {
            assert!(
                s.engine.next_basket_id()
                    > s.engine.baskets.iter().map(|k| k.id).max().unwrap_or(0),
                "licznik koszyków poniżej odtworzonych numerów (format {})",
                s.format
            );
        }
    }

    /// KONTRAKT E + ciągłość zarządzania: po adopcji z OTWARTĄ pozycją
    /// silnik dalej prowadzi koszyk (tick nie zabija, koszyk żyje,
    /// pozycja ma opiekuna).
    #[test]
    fn po_adopcji_zarzadzanie_tyka_dalej() {
        let (mut a, mut b) = warsztat();
        let mut trwale = Trwale::default();
        let (mut nowe, _w, _r) = przebuduj_testowo(&mut a, &b, "ZENONLY3", &["ZEN"], &mut trwale);

        let zywe_przed = nowe.koszyki().iter().filter(|k| k.alive()).count();
        assert!(zywe_przed > 0);
        // kilka ticków po adopcji — nic nie ma prawa osieroceć ani zniknąć
        for i in 0..5 {
            let q = kwot(T0 + 10_000 + i * 1_000, 4000.0 + i as f64 * 0.1);
            b.on_quote(q);
            for s in nowe.lista.iter_mut() {
                s.engine.on_tick(&mut b, &q);
            }
        }
        // pozycja z brokera nadal przypisana do KONKRETNEGO silnika
        for p in b.positions() {
            let i = nowe.indeks_koszyka(p.basket.unwrap_or(0));
            assert!(
                i.is_some(),
                "pozycja koszyka {:?} bez opiekuna po tickach",
                p.basket
            );
        }
        assert!(
            nowe.koszyki().iter().filter(|k| k.alive()).count() > 0,
            "adopcja + ticki nie mają prawa zabić żywych koszyków"
        );
    }

    /// Przejście „w górę" slot0 → sloty (ZENONLY3 → SENTINEL-0): koszyki B1…
    /// (numeracja jednosilnikowa) wracają do silnika SWOJEGO formatu, bo
    /// zapasowym zostaje właściciel slotu 0 — a nie przypadkowy pierwszy.
    #[test]
    fn wejscie_w_gore_slot0_wraca_do_swojego_formatu() {
        // stary zespół: pojedynczy ZEN na slocie 0 (dokładnie jak ścieżka
        // jednosilnikowa `zbuduj_silniki`)
        let mut e = Engine::new(ustawienia(), 1000.0);
        e.pulapy = PulapyGlobalne::default();
        let mut a = routing::Silniki::pojedynczy(
            e,
            "ZEN".into(),
            "P-ZEN".into(),
            lancuch("ZENONLY3", &["ZEN"]),
            true,
        );
        let mut b = SimBroker::new(1000.0, 0.2, 0.0);
        b.on_quote(kwot(T0, 4000.0));
        a.lista[0]
            .engine
            .on_message(&mut b, &wiadomosc(T0, 7, SYGNAL_RYNKOWY));
        let przed: std::collections::BTreeSet<u32> = a.koszyki().iter().map(|k| k.id).collect();
        assert!(!przed.is_empty());
        assert!(przed
            .iter()
            .all(|id| conduit_core::wielosilnik::slot_koszyka(*id) == 0));

        let mut trwale = Trwale::default();
        let (nowe, w, raport) =
            przebuduj_testowo(&mut a, &b, "SENTINEL-0", &["Synergy", "ZEN"], &mut trwale);

        assert_eq!(w.sieroty, 0);
        assert!(raport.porzucone.is_empty());
        let i_zen = nowe.indeks_formatu("ZEN").unwrap();
        assert!(
            nowe.lista[i_zen].zapasowy,
            "zapasowym MUSI być właściciel slotu 0 (ZEN), nie pierwszy z listy"
        );
        assert_eq!(
            nowe.lista[i_zen]
                .engine
                .baskets
                .iter()
                .map(|k| k.id)
                .collect::<std::collections::BTreeSet<u32>>(),
            przed,
            "koszyki B1… mają wrócić do formatu, który je otworzył"
        );
        // a Synergy handluje normalnie (nie jest zamrożony)
        assert!(nowe
            .trasa(&SourceKey::new(-1002, None), Some("Synergy".into()))
            .is_ok());
    }

    /// Powrót „w dół potem w górę": zamrożony format odzyskuje nogę
    /// w kolejnym łańcuchu i wraca do normalnego handlu z tymi samymi
    /// koszykami — pełny cykl SENTINEL-0 → ZENONLY3 → SENTINEL-0.
    #[test]
    fn pelny_cykl_w_dol_i_w_gore_bez_strat() {
        let (mut a, b) = warsztat();
        let przed: std::collections::BTreeSet<u32> = a.koszyki().iter().map(|k| k.id).collect();

        let mut trwale = Trwale::default();
        let (mut posrednie, _w1, _r1) =
            przebuduj_testowo(&mut a, &b, "ZENONLY3", &["ZEN"], &mut trwale);

        let mut trwale2 = Trwale::default();
        let (koncowe, w2, r2) = przebuduj_testowo(
            &mut posrednie,
            &b,
            "SENTINEL-0",
            &["Synergy", "ZEN"],
            &mut trwale2,
        );

        assert_eq!(w2.sieroty, 0);
        assert!(r2.porzucone.is_empty());
        let po: std::collections::BTreeSet<u32> = koncowe.koszyki().iter().map(|k| k.id).collect();
        assert_eq!(po, przed, "po pełnym cyklu zbiór koszyków identyczny");
        // Synergy znów handluje (już nie zamrożony)
        let i_syn = koncowe.indeks_formatu("Synergy").unwrap();
        assert!(!koncowe.lista[i_syn].tylko_zarzadzanie);
        assert!(koncowe
            .trasa(&SourceKey::new(-1002, None), Some("Synergy".into()))
            .is_ok());
    }

    /// KONTRAKT D: zmiana łańcucha NIE dotyka kotwic PnL — klucz kotwic
    /// patrzy na (login, serwer), a te się nie zmieniają.
    #[test]
    fn kotwice_pnl_nietkniete_przy_zmianie_lancucha() {
        let mut stats = conduit_server::ui::Stats::new(1000.0, T0);
        assert!(!stats.przelacz_konto("10000002@PUPrime-PUBLIC-DEMO", 1000.0, 100, T0));
        stats.session_start_equity = 1000.0;
        stats.day_start_equity = 980.0;
        // zmiana łańcucha = ŻADNEGO wywołania przelacz_konto z innym kluczem;
        // ten sam klucz jest no-opem niezależnie od liczby wywołań
        for _ in 0..5 {
            assert!(!stats.przelacz_konto("10000002@PUPrime-PUBLIC-DEMO", 1234.0, 101, T0 + 1));
        }
        assert_eq!(stats.session_start_equity, 1000.0);
        assert_eq!(stats.day_start_equity, 980.0);
    }
}

// ============================================================
//  WERDYKT 0,01/0,02 — łańcuch JEDNONOGI musi grać presetem nogi
// ============================================================
#[cfg(test)]
mod testy_jednonogiego_lancucha {
    use super::*;
    use conduit_core::settings::Settings;

    fn stan(tag: &str) -> StateHandle {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "conduit-noga-{tag}-{}-{}",
            std::process::id(),
            conduit_server::now_ms()
        ));
        let cfg = conduit_server::ServerConfig {
            workspace: dir,
            ..Default::default()
        };
        conduit_server::bootstrap(&cfg, conduit_server::default_auth()).unwrap()
    }

    #[test]
    fn zenonly5_z_dokumentem_hyper2_gra_lotem_presetu_nogi() {
        let st = stan("fq5");
        // FRESHQUEEN-5 na dysku — sufit lota 0,01, wlasna sesja 9-15
        let mut fq5 = Settings::default();
        fq5.lot_max = 0.01;
        fq5.session_hours = "9-15".into();
        st.workspace
            .save_preset(&conduit_core::Preset {
                name: "FRESHQUEEN-5".into(),
                description: String::new(),
                format: "ZEN".into(),
                settings: fq5,
                ea: None,
            })
            .unwrap();
        // kanał ZEN podpięty i obserwowany; aktywny łańcuch ZENONLY5
        st.update(Sections::one(Section::Settings), |s| {
            s.lancuchy.aktywny = "ZENONLY5".into();
            s.bindings.insert(
                "-100200300".into(),
                ui::ChannelBinding {
                    channel_id: -100_200_300,
                    monitored: true,
                    notify: false,
                    format: "ZEN".into(),
                    topics: Default::default(),
                },
            );
        });
        let mut doc = Settings::default();
        doc.lot_mode_percent = true;
        doc.lot_percent = 0.5;
        doc.lot_max = 100.0;
        doc.session_hours = "8-16".into();

        let silniki = zbuduj_silniki(&st, &doc, 300.0);

        assert_eq!(silniki.lista.len(), 1);
        let s0 = &silniki.lista[0];
        assert_eq!(
            s0.preset, "FRESHQUEEN-5",
            "etykieta ma mówić prawdę o nodze"
        );
        assert_eq!(
            s0.engine.cfg.lot_max, 0.01,
            "HANDEL z presetu nogi: sufit lota FQ-5, nie 100 z dokumentu"
        );
        assert_eq!(
            s0.engine.cfg.session_hours, "9-15",
            "pole handlowe nogi (sesja) też z presetu, nie z dokumentu"
        );
        assert!(
            !s0.engine.cfg.lot_mode_percent,
            "tryb lota z presetu FQ-5, nie z dokumentu"
        );
        assert_eq!(
            s0.engine.cfg.lot_percent, 1.0,
            "procent lota z presetu FQ-5"
        );
        assert!(
            s0.z_pliku,
            "noga zbudowana z PLIKU presetu — panel ma to pokazać"
        );
        // WYNIKOWY lot clampuje sufit NOGI: cokolwiek policzy wzór, 0,01
        assert_eq!(
            s0.engine.lot_size(300.0),
            0.01,
            "wynikowy lot musi respektować sufit nogi"
        );
    }

    #[test]
    fn zen_jako_temat_forum_daje_noge_freshqueen5() {
        let st = stan("forum");
        let mut fq5 = Settings::default();
        fq5.lot_max = 0.01;
        st.workspace
            .save_preset(&conduit_core::Preset {
                name: "FRESHQUEEN-5".into(),
                description: String::new(),
                format: "ZEN".into(),
                settings: fq5,
                ea: None,
            })
            .unwrap();
        st.update(Sections::one(Section::Settings), |s| {
            s.lancuchy.aktywny = "ZENONLY5".into();
            s.preset_id = "HYPER-2".into();
            // JEDNA grupa forum, DWA tematy (zanonimizowany identyfikator testowy)
            s.bindings.insert(
                "-1000000000042".into(),
                ui::ChannelBinding {
                    channel_id: -1_000_000_000_101,
                    monitored: true,
                    notify: false,
                    format: String::new(), // grupa forum nie ma formatu własnego
                    topics: [
                        ("31".to_string(), "ZEN".to_string()),
                        ("44".to_string(), "NOVA".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                },
            );
        });
        let silniki = zbuduj_silniki(&st, &Settings::default(), 300.0);
        assert_eq!(silniki.lista.len(), 1);
        assert_eq!(
            silniki.lista[0].format, "ZEN",
            "format tematu forum MUSI liczyć się jako obsługiwany"
        );
        assert_eq!(
            silniki.lista[0].preset, "FRESHQUEEN-5",
            "noga bierze preset z ŁAŃCUCHA, nigdy z preset_id panelu"
        );
    }

    /// Ta sama topologia, ale grupa NIE jest obserwowana. Noga nadal musi być
    /// opisana prawdziwie (ZEN→FRESHQUEEN-5, nie handluje) — panel nie ma
    /// prawa wymyślić nogi „bez formatu · HYPER-2", której w łańcuchu nie ma.
    #[test]
    fn nieobserwowana_grupa_nie_zmyslas_nogi_hyper2() {
        let st = stan("nieobs");
        let mut fq5 = Settings::default();
        fq5.lot_max = 0.01;
        st.workspace
            .save_preset(&conduit_core::Preset {
                name: "FRESHQUEEN-5".into(),
                description: String::new(),
                format: "ZEN".into(),
                settings: fq5,
                ea: None,
            })
            .unwrap();
        st.update(Sections::one(Section::Settings), |s| {
            s.lancuchy.aktywny = "ZENONLY5".into();
            s.preset_id = "HYPER-2".into();
            s.bindings.insert(
                "-1000000000042".into(),
                ui::ChannelBinding {
                    channel_id: -1_000_000_000_101,
                    monitored: false,
                    notify: false,
                    format: String::new(),
                    topics: [("31".to_string(), "ZEN".to_string())]
                        .into_iter()
                        .collect(),
                },
            );
        });
        let silniki = zbuduj_silniki(&st, &Settings::default(), 300.0);
        assert!(
            silniki.lista.iter().all(|s| s.preset != "HYPER-2"),
            "HYPER-2 nie jest w łańcuchu ZENONLY5 — nie wolno go pokazać jako nogi"
        );
        assert_eq!(silniki.lista[0].format, "ZEN");
        assert_eq!(silniki.lista[0].preset, "FRESHQUEEN-5");
        assert_eq!(
            silniki.lista[0].powod, "brakZrodla",
            "panel ma napisać, CZEGO brakuje — inaczej użytkownik nie wie,              czy poprawiać Kanały, czy Łańcuchy"
        );
        let nogi = loty_nog(&silniki, &[], 0.0, 300.0, &[]);
        assert!(
            !nogi[0].handluje,
            "bez obserwowanego źródła noga nie bierze sygnałów"
        );
        assert_eq!(nogi[0].stan, "nieaktywna");
        assert_eq!(nogi[0].preset, "FRESHQUEEN-5");
    }

    #[test]
    fn niezmienniki_listy_nog_na_calej_macierzy() {
        use conduit_core::formaty::lancuchy_wbudowane;

        let wszystkie = lancuchy_wbudowane();
        let st = stan("macierz");
        for l in &wszystkie {
            for preset in l.presety.values().filter(|p| !p.is_empty()) {
                let _ = st.workspace.save_preset(&conduit_core::Preset {
                    name: preset.clone(),
                    description: String::new(),
                    format: String::new(),
                    settings: Settings::default(),
                    ea: None,
                });
            }
        }

        // sześć układów źródeł: nic, ZEN, Synergy, oba, ZEN jako TEMAT forum
        // oraz ZEN wpisany inną wielkością liter (routing ma to znosić)
        let uklady: Vec<(&str, Vec<(&str, bool)>)> = vec![
            ("brak zrodel", vec![]),
            ("ZEN", vec![("ZEN", false)]),
            ("Synergy", vec![("Synergy", false)]),
            ("ZEN+Synergy", vec![("ZEN", false), ("Synergy", false)]),
            ("ZEN jako temat forum", vec![("ZEN", true)]),
            ("zen inna wielkoscia liter", vec![("zEn", false)]),
        ];
        let drabinki: [(&str, bool, f64); 4] = [
            ("wylaczona", false, -1.0),
            ("wlaczona szczebel 0", true, 0.0),
            ("wlaczona szczebel 500", true, 500.0),
            ("wlaczona szczebel 2000", true, 2000.0),
        ];

        let core = Settings::default();
        let mut sprawdzonych = 0usize;
        for lanc in &wszystkie {
            for (opis_zrodel, zrodla) in &uklady {
                for (opis_drab, wlaczona, prog) in drabinki {
                    st.update(Sections::one(Section::Settings), |s| {
                        s.lancuchy.aktywny = lanc.nazwa.clone();
                        // preset_id celowo spoza łańcucha: jeżeli wycieknie
                        // na listę nóg, test to złapie
                        s.preset_id = "KOAN-2".into();
                        s.drabinka.enabled = wlaczona;
                        s.drabinka.biezacy_prog = prog;
                        s.bindings.clear();
                        for (i, (fmt, temat)) in zrodla.iter().enumerate() {
                            let id = -(1000 + i as i64);
                            s.bindings.insert(
                                id.to_string(),
                                ui::ChannelBinding {
                                    channel_id: id,
                                    monitored: true,
                                    notify: false,
                                    format: if *temat { String::new() } else { (*fmt).into() },
                                    topics: if *temat {
                                        [("7".to_string(), (*fmt).to_string())]
                                            .into_iter()
                                            .collect()
                                    } else {
                                        Default::default()
                                    },
                                },
                            );
                        }
                    });

                    let silniki = zbuduj_silniki(&st, &core, 300.0);
                    let szczeble =
                        zbuduj_szczeble(&st, &core, 300.0, &silniki.lancuch, &pary_nog(&silniki));
                    let nogi = loty_nog(&silniki, &szczeble, prog, 300.0, &[]);
                    let gdzie = format!(
                        "lancuch {} | zrodla: {} | drabinka {}",
                        lanc.nazwa, opis_zrodel, opis_drab
                    );
                    sprawdzonych += 1;

                    // I1 — KAŻDA noga aktywnego łańcucha jest na liście, raz
                    for (fmt, preset) in lanc.presety.iter().filter(|(_, p)| !p.is_empty()) {
                        let ile = nogi
                            .iter()
                            .filter(|n| &n.format == fmt && &n.preset == preset)
                            .count();
                        assert_eq!(
                            ile, 1,
                            "{gdzie}: noga {fmt}->{preset} wystepuje {ile}x (ma raz)"
                        );
                    }

                    // I2 — preset_id nie ma prawa nazwać nogi, gdy łańcuch ma nogi
                    if !lanc.presety.values().all(|p| p.is_empty()) {
                        assert!(
                            !nogi.iter().any(|n| n.preset == "KOAN-2"),
                            "{gdzie}: preset_id wyciekl na liste nog"
                        );
                    }

                    // I3 — bez duplikatów (format, preset)
                    let mut pary: Vec<(String, String)> = nogi
                        .iter()
                        .map(|n| (n.format.clone(), n.preset.clone()))
                        .collect();
                    let przed = pary.len();
                    pary.sort();
                    pary.dedup();
                    assert_eq!(przed, pary.len(), "{gdzie}: zdublowane nogi");

                    // I4 — handluje wtedy i tylko wtedy, gdy format ma źródło
                    let karmione: Vec<String> =
                        zrodla.iter().map(|(f, _)| f.to_lowercase()).collect();
                    for n in &nogi {
                        if n.handluje {
                            assert!(
                                karmione.contains(&n.format.to_lowercase()),
                                "{gdzie}: {} handluje BEZ zrodla",
                                n.format
                            );
                            assert_eq!(n.stan, "aktywna", "{gdzie}: grajaca noga ma zly stan");
                            assert!(n.powod.is_empty(), "{gdzie}: grajaca noga ma powod");
                        }
                    }

                    // I5 — stan i powód zawsze ze znanego zbioru
                    for n in &nogi {
                        assert!(
                            ["aktywna", "kolejka", "nieaktywna"].contains(&n.stan.as_str()),
                            "{gdzie}: nieznany stan {:?}",
                            n.stan
                        );
                        assert!(
                            n.powod.is_empty()
                                || [
                                    "brakZrodla",
                                    "zamrozona",
                                    "brakFormatu",
                                    "kolejka",
                                    "minieta"
                                ]
                                .contains(&n.powod.as_str()),
                            "{gdzie}: nieznany powod {:?}",
                            n.powod
                        );
                    }

                    // I6 — drabinka wyłączona = ani jednego wiersza „w kolejce"
                    if !wlaczona {
                        assert!(
                            !nogi.iter().any(|n| n.stan == "kolejka"),
                            "{gdzie}: kolejka przy WYLACZONEJ drabince"
                        );
                    }

                    // I7 — suma kafla liczy się wyłącznie z nóg handlujących
                    let suma: f64 = nogi.iter().filter(|n| n.handluje).map(|n| n.lot).sum();
                    assert!(
                        suma.is_finite() && suma >= 0.0,
                        "{gdzie}: suma lota bez sensu"
                    );
                }
            }
        }
        assert!(
            sprawdzonych >= 100,
            "macierz za mala: {sprawdzonych} kombinacji"
        );
        println!("sprawdzonych kombinacji: {sprawdzonych}");
    }

    #[test]
    fn podpiecie_kanalu_w_trakcie_pracy_zmienia_odcisk() {
        let st = stan("odcisk-kanal");
        st.update(Sections::one(Section::Settings), |s| {
            s.lancuchy.aktywny = "ZENONLY5".into();
        });
        let przed = odcisk_konfiguracji(&st);
        assert!(
            !przed.contains("ZEN,") && !przed.contains("zrodla[ZEN]"),
            "start bez źródeł"
        );

        // użytkownik zaznacza TEMAT forum i nadaje mu format ZEN
        st.update(Sections::one(Section::Settings), |s| {
            s.bindings.insert(
                "-1000000000042".into(),
                ui::ChannelBinding {
                    channel_id: -1_000_000_000_101,
                    monitored: true,
                    notify: false,
                    format: String::new(),
                    topics: [("31".to_string(), "ZEN".to_string())]
                        .into_iter()
                        .collect(),
                },
            );
        });
        let po = odcisk_konfiguracji(&st);
        assert_ne!(
            przed, po,
            "podpięcie kanału MUSI zmienić odcisk — inaczej brak przebudowy"
        );
        assert!(po.contains("ZEN"), "odcisk niesie nowy format: {po}");

        // i od tej chwili noga naprawdę handluje
        let silniki = zbuduj_silniki(&st, &Settings::default(), 300.0);
        let nogi = loty_nog(&silniki, &[], -1.0, 300.0, &[]);
        let zen = nogi.iter().find(|n| n.format == "ZEN").expect("noga ZEN");
        assert!(
            zen.handluje,
            "po podpięciu kanału noga ZEN musi brać sygnały"
        );
        assert_eq!(zen.powod, "", "żadnego powodu bezczynności");
    }

    /// PODMIANA PRESETU WEWNĄTRZ AKTYWNEGO ŁAŃCUCHA — nazwa się nie zmienia,
    /// więc stary warunek („inna nazwa łańcucha") tego nie łapał i bot grał
    /// dalej poprzednim presetem.
    #[test]
    fn podmiana_presetu_w_lancuchu_zmienia_odcisk() {
        let st = stan("odcisk-noga");
        st.update(Sections::one(Section::Settings), |s| {
            s.lancuchy.aktywny = "ZENONLY5".into();
        });
        let przed = odcisk_konfiguracji(&st);
        st.update(Sections::one(Section::Settings), |s| {
            if let Some(l) = s.lancuchy.lista.iter_mut().find(|l| l.nazwa == "ZENONLY5") {
                l.presety.insert("ZEN".into(), "FRESHQUEEN-3".into());
            }
        });
        let po = odcisk_konfiguracji(&st);
        assert_ne!(
            przed, po,
            "zmiana nogi wewnątrz łańcucha MUSI zmienić odcisk"
        );
        assert!(
            po.contains("ZEN>FRESHQUEEN-3"),
            "odcisk niesie nową nogę: {po}"
        );
    }

    #[test]
    fn bez_zrodel_widac_wszystkie_nogi_lancucha_nie_tylko_pierwsza() {
        let st = stan("komplet");
        for n in ["HYPER-2", "FRESHQUEEN-3"] {
            st.workspace
                .save_preset(&conduit_core::Preset {
                    name: n.into(),
                    description: String::new(),
                    format: String::new(),
                    settings: Settings::default(),
                    ea: None,
                })
                .unwrap();
        }
        st.update(Sections::one(Section::Settings), |s| {
            s.lancuchy.aktywny = "SENTINEL-0".into();
            s.preset_id = "HYPER-2".into();
            // ani jednego obserwowanego kanału — dokładnie stan po starcie
        });
        let core = Settings::default();
        let silniki = zbuduj_silniki(&st, &core, 300.0);
        let szczeble = zbuduj_szczeble(&st, &core, 300.0, &silniki.lancuch, &pary_nog(&silniki));
        let nogi = loty_nog(&silniki, &szczeble, -1.0, 300.0, &[]);

        let pary: Vec<String> = nogi
            .iter()
            .map(|n| format!("{}→{}", n.format, n.preset))
            .collect();
        assert!(
            pary.contains(&"Synergy→HYPER-2".to_string()),
            "brak nogi Synergy: {pary:?}"
        );
        assert!(
            pary.contains(&"ZEN→FRESHQUEEN-3".to_string()),
            "DRUGA noga łańcucha zniknęła — to jest ten błąd: {pary:?}"
        );
        assert!(
            nogi.iter().all(|n| !n.handluje),
            "bez źródeł nic nie handluje"
        );
        assert!(
            nogi.iter().all(|n| n.powod == "brakZrodla"),
            "powód ma wskazywać brak ŹRÓDŁA, nie próg drabinki"
        );
        assert!(
            nogi.iter().all(|n| n.stan == "nieaktywna"),
            "noga bez źródła nie jest „w kolejce” — ona nie czeka na próg"
        );
    }

    /// SZCZEBLE DRABINKI, NA KTÓRYCH KONTO NIE STOI, TEŻ MUSZĄ BYĆ WIDOCZNE.
    /// Użytkownik ma prawo wyedytować lot presetu, zanim konto na niego
    /// urośnie — i zobaczyć, że preset ze szczebla już miniętego przestał grać.
    #[test]
    fn drabinka_pokazuje_kolejke_i_szczeble_miniete() {
        let st = stan("szczeble");
        for n in ["FRESHQUEEN-5", "FRESHQUEEN-3", "HYPER-2C"] {
            let mut c = Settings::default();
            c.lot_max = 0.05;
            st.workspace
                .save_preset(&conduit_core::Preset {
                    name: n.into(),
                    description: String::new(),
                    format: String::new(),
                    settings: c,
                    ea: None,
                })
                .unwrap();
        }
        st.update(Sections::one(Section::Settings), |s| {
            s.lancuchy.aktywny = "ZENONLY5".into();
            s.drabinka.enabled = true;
            s.drabinka.biezacy_prog = 0.0;
            s.drabinka.szczeble = vec![
                ui::SzczebelDrabinki {
                    prog_balance: 0.0,
                    lancuch: "ZENONLY5".into(),
                },
                ui::SzczebelDrabinki {
                    prog_balance: 500.0,
                    lancuch: "ZENONLY3".into(),
                },
                ui::SzczebelDrabinki {
                    prog_balance: 2000.0,
                    lancuch: "SENTINEL-0C".into(),
                },
            ];
            s.bindings.insert(
                "-1".into(),
                ui::ChannelBinding {
                    channel_id: -1,
                    monitored: true,
                    notify: false,
                    format: "ZEN".into(),
                    topics: Default::default(),
                },
            );
        });
        let core = Settings::default();
        let silniki = zbuduj_silniki(&st, &core, 300.0);
        let szczeble = zbuduj_szczeble(&st, &core, 300.0, &silniki.lancuch, &pary_nog(&silniki));
        let nogi = loty_nog(&silniki, &szczeble, 0.0, 300.0, &[]);

        let akt: Vec<_> = nogi.iter().filter(|n| n.stan == "aktywna").collect();
        assert_eq!(akt.len(), 1, "na szczeblu 0 gra dokładnie jedna noga");
        assert_eq!(akt[0].preset, "FRESHQUEEN-5");

        let kolejka: Vec<&str> = nogi
            .iter()
            .filter(|n| n.stan == "kolejka")
            .map(|n| n.preset.as_str())
            .collect();
        assert!(
            kolejka.contains(&"FRESHQUEEN-3"),
            "szczebel 500 czeka w kolejce: {kolejka:?}"
        );
        assert!(
            kolejka.contains(&"HYPER-2C"),
            "szczebel 2000 czeka w kolejce: {kolejka:?}"
        );
        assert!(
            !nogi.iter().any(|n| n.preset == "HYPER-2"),
            "HYPER-2 nie jest w tej drabince i nie wolno go pokazać"
        );

        // Konto urosło ponad 500: szczebel bazowy staje się MINIĘTY.
        st.update(Sections::one(Section::Settings), |s| {
            s.lancuchy.aktywny = "ZENONLY3".into();
            s.drabinka.biezacy_prog = 500.0;
        });
        let silniki = zbuduj_silniki(&st, &core, 600.0);
        let szczeble = zbuduj_szczeble(&st, &core, 600.0, &silniki.lancuch, &pary_nog(&silniki));
        let nogi = loty_nog(&silniki, &szczeble, 500.0, 600.0, &[]);
        let minieta = nogi
            .iter()
            .find(|n| n.preset == "FRESHQUEEN-5")
            .expect("FQ-5 na liście");
        assert_eq!(minieta.stan, "nieaktywna");
        assert_eq!(minieta.powod, "minieta");
        assert_eq!(
            nogi.iter()
                .find(|n| n.preset == "HYPER-2C")
                .map(|n| n.stan.as_str()),
            Some("kolejka"),
            "szczebel 2000 nadal czeka"
        );
    }

    #[test]
    fn karta_lota_panelu_nie_rusza_lota_nogi_na_calej_drodze_produkcyjnej() {
        let st = stan("droga-produkcyjna");
        // FRESHQUEEN-5 na dysku: sufit 0,01
        let mut fq5 = Settings::default();
        fq5.lot_mode_percent = true;
        fq5.lot_percent = 0.5;
        fq5.lot_max = 0.01;
        st.workspace
            .save_preset(&conduit_core::Preset {
                name: "FRESHQUEEN-5".into(),
                description: String::new(),
                format: "ZEN".into(),
                settings: fq5,
                ea: None,
            })
            .unwrap();

        // STAN PANELU dokładnie jak na rachunku: dokument z polami HYPER-2
        // i karta lota ustawiona na procent 0,5 (to jest `settings.json`).
        st.update(Sections::one(Section::Settings), |s| {
            s.lancuchy.aktywny = "ZENONLY5".into();
            s.preset_id = "HYPER-2".into();
            s.lot = ui::LotConfig {
                mode: "percent".into(),
                fixed: 0.01,
                percent: 0.5,
            };
            s.settings = serde_json::json!({
                "lot_mode_percent": true,
                "lot_percent": 0.5,
                "lot_fixed": 0.01,
                "lot_max": 100.0,
                "lot_min": 0.01,
                "session_hours": "8-16",
            });
            s.bindings.insert(
                "-100200300".into(),
                ui::ChannelBinding {
                    channel_id: -100_200_300,
                    monitored: true,
                    notify: false,
                    format: "ZEN".into(),
                    topics: Default::default(),
                },
            );
        });

        // ---- 1. START PĘTLI: te trzy linijki to kopia `handel()` ----
        let mut core = st.read(|s| {
            let mut c = conduit_server::settings_map::core_from_ui(&s.settings);
            conduit_server::settings_map::apply_lot(&mut c, &s.lot);
            c
        });
        assert_eq!(
            core.lot_max, 100.0,
            "dokument panelu ma sufit HYPER-2 — to jest wejście testu"
        );
        let mut silniki = zbuduj_silniki(&st, &core, 447.19);
        assert_eq!(silniki.lista.len(), 1);
        assert_eq!(silniki.lista[0].preset, "FRESHQUEEN-5");
        assert!(silniki.lista[0].z_pliku);
        assert_eq!(
            silniki.lista[0].engine.lot_size(447.19),
            0.01,
            "447,19 · 0,5 % = 0,022 → z dokumentu wyszłoby 0,02; sufit nogi daje 0,01"
        );

        // ---- 2. PRZEŁADOWANIE CO 2 s (dwa obroty) ----
        let mut mtime: std::collections::HashMap<String, std::time::SystemTime> =
            Default::default();
        przeladuj_ustawienia(&st, &mut silniki, &mut core, 0.2, &mut mtime);
        assert_eq!(
            silniki.lista[0].engine.lot_size(447.19),
            0.01,
            "pierwsze przeładowanie"
        );

        st.update(Sections::one(Section::Settings), |s| {
            s.lot = ui::LotConfig {
                mode: "fixed".into(),
                fixed: 0.50,
                percent: 5.0,
            };
        });
        przeladuj_ustawienia(&st, &mut silniki, &mut core, 0.2, &mut mtime);
        assert_eq!(
            silniki.lista[0].engine.lot_size(447.19),
            0.01,
            "karta lota panelu NIE JEST lotem automatu — po jej zmianie noga gra dalej 0,01"
        );
        assert_eq!(
            silniki.lista[0].engine.cfg.lot_max, 0.01,
            "sufit nogi przeżył przeładowanie"
        );

        // ---- 3. TO, CO WIDZI PANEL (`/api/state`) ----
        let nogi = loty_nog(&silniki, &[], 0.0, 300.0, &[]);
        assert_eq!(nogi.len(), 1);
        assert_eq!(nogi[0].lot, 0.01, "kafel LOT AUTO ma pokazać 0,01");
        assert!(nogi[0].handluje, "noga ZEN bierze sygnały");
        assert!(
            nogi[0].z_pliku,
            "panel ma widzieć, że handel idzie z pliku presetu"
        );
        assert_eq!(nogi[0].lot_max, 0.01);
    }

    /// SILNIK ZAMROŻONY NIE DAJE SIĘ PRZESTAWIĆ DOKUMENTEM.
    ///
    /// Zamrożona noga prowadzi koszyki, które otwarła STARA konfiguracja —
    /// i to jest jedyny powód, dla którego w ogóle istnieje. Gdyby
    /// przeładowanie ustawień podmieniło jej pola handlu dokumentem panelu,
    /// otwarte pozycje dogrywałoby ustawienie, którego nikt dla nich nie
    /// wybrał. Rachunek (koszty, opóźnienie) ma się odświeżać normalnie.
    #[test]
    fn zamrozona_noga_nie_bierze_pol_handlu_z_dokumentu() {
        let st = stan("zamrozona");
        let mut core = Settings::default();

        // Silnik zamrożony budujemy tak, jak robi to `przebuduj_lancuch`:
        // stara konfiguracja + `tylko_zarzadzanie`.
        let mut stara = Settings::default();
        stara.session_hours = "9-15".into();
        stara.lot_max = 0.01;
        let mut engine = Engine::new(stara, 447.19);
        engine.przypisz_slot(3);
        let mut silniki = routing::Silniki::pojedynczy(
            Engine::new(core.clone(), 447.19),
            "ATFX".into(),
            String::new(),
            conduit_core::formaty::Lancuch::default(),
            false,
        );
        silniki.lista.push(routing::Silnik {
            powod: String::new(),
            format: "ZEN".into(),
            preset: "FRESHQUEEN-5 (zamrożony)".into(),
            slot: 3,
            zapasowy: false,
            tylko_zarzadzanie: true,
            z_pliku: false,
            engine,
        });

        // Dokument panelu zmienia się (inna sesja, inny sufit lota).
        st.update(Sections::one(Section::Settings), |s| {
            s.settings = serde_json::json!({ "session_hours": "0-24", "lot_max": 50.0 });
        });
        let mut mtime: std::collections::HashMap<String, std::time::SystemTime> =
            Default::default();
        przeladuj_ustawienia(&st, &mut silniki, &mut core, 0.2, &mut mtime);

        let z = &silniki.lista[1];
        assert!(z.tylko_zarzadzanie);
        assert_eq!(
            z.engine.cfg.session_hours, "9-15",
            "zamrożona zostaje przy swojej sesji"
        );
        assert_eq!(
            z.engine.cfg.lot_max, 0.01,
            "zamrożona zostaje przy swoim sufcie lota"
        );
        // ...a silnik ze starej ścieżki ręcznej BIERZE cały dokument
        assert_eq!(silniki.lista[0].engine.cfg.session_hours, "0-24");
        assert_eq!(silniki.lista[0].engine.cfg.lot_max, 50.0);
    }

    #[test]
    fn edycja_pliku_presetu_zmienia_lot_nogi_w_locie() {
        let st = stan("edycja-presetu");
        let zapisz = |lot_max: f64| {
            let mut p = Settings::default();
            p.lot_mode_percent = true;
            p.lot_percent = 0.5;
            p.lot_max = lot_max;
            st.workspace
                .save_preset(&conduit_core::Preset {
                    name: "FRESHQUEEN-5".into(),
                    description: String::new(),
                    format: "ZEN".into(),
                    settings: p,
                    ea: None,
                })
                .unwrap();
        };
        zapisz(0.01);
        st.update(Sections::one(Section::Settings), |s| {
            s.lancuchy.aktywny = "ZENONLY5".into();
            s.bindings.insert(
                "-100200300".into(),
                ui::ChannelBinding {
                    channel_id: -100_200_300,
                    monitored: true,
                    notify: false,
                    format: "ZEN".into(),
                    topics: Default::default(),
                },
            );
        });
        let mut core = Settings::default();
        let mut silniki = zbuduj_silniki(&st, &core, 447.19);
        let mut mtime: std::collections::HashMap<String, std::time::SystemTime> =
            Default::default();
        // pierwszy obrót zapamiętuje stan zastany
        przeladuj_ustawienia(&st, &mut silniki, &mut core, 0.2, &mut mtime);
        assert_eq!(silniki.lista[0].engine.lot_size(447.19), 0.01);

        // panel zapisuje NOWY sufit do pliku presetu (POST /presets/../settings)
        std::thread::sleep(std::time::Duration::from_millis(1100));
        zapisz(0.05);
        przeladuj_ustawienia(&st, &mut silniki, &mut core, 0.2, &mut mtime);
        assert_eq!(
            silniki.lista[0].engine.lot_size(447.19),
            0.02,
            "447,19 · 0,5 % = 0,022 → 0,02 pod nowym sufitem 0,05"
        );
    }

    #[test]
    fn brak_formatu_handlujacego_nie_wchodzi_do_sumy_kafla() {
        let st = stan("bez-formatu");
        st.update(Sections::one(Section::Settings), |s| {
            s.lancuchy.aktywny = "ZENONLY5".into();
            s.preset_id = "HYPER-2".into();
            // ani jednego obserwowanego kanału z formatem ZEN
        });
        let core = Settings::default();
        let silniki = zbuduj_silniki(&st, &core, 447.19);
        let nogi = loty_nog(&silniki, &[], 0.0, 300.0, &[]);
        assert_eq!(
            nogi.len(),
            1,
            "silnik istnieje — koszyki muszą mieć opiekuna"
        );
        assert_eq!(nogi[0].format, "ZEN", "tożsamość nogi pochodzi z łańcucha");
        assert_eq!(nogi[0].preset, "FRESHQUEEN-5");
        assert_ne!(
            nogi[0].preset, "HYPER-2",
            "preset_id nie ma prawa nazwać nogi łańcucha"
        );
        assert!(
            !nogi[0].handluje,
            "bez obserwowanego źródła noga nie dostanie wiadomości — kafel jej nie sumuje"
        );
        assert_eq!(nogi[0].powod, "brakZrodla");
    }

    /// Fallback ŚWIADOMY: noga wskazuje preset, którego NIE MA na dysku →
    /// stara ścieżka (dokument), plus warn w logu. Ręczna konfiguracja
    /// panelu bez plików presetów ma dalej działać.
    #[test]
    fn noga_bez_pliku_presetu_spada_na_dokument_z_ostrzezeniem() {
        let st = stan("brak-pliku");
        st.update(Sections::one(Section::Settings), |s| {
            s.lancuchy.aktywny = "ZENONLY5".into();
            s.bindings.insert(
                "-100200300".into(),
                ui::ChannelBinding {
                    channel_id: -100_200_300,
                    monitored: true,
                    notify: false,
                    format: "ZEN".into(),
                    topics: Default::default(),
                },
            );
        });
        let mut doc = Settings::default();
        doc.lot_max = 77.0;
        let silniki = zbuduj_silniki(&st, &doc, 300.0);
        assert_eq!(
            silniki.lista[0].engine.cfg.lot_max, 77.0,
            "fallback = dokument"
        );
        let warn = st.read(|s| {
            s.logs
                .iter()
                .any(|l| l.title.contains("pliku presetu NIE MA"))
        });
        assert!(warn, "brak pliku nogi MUSI krzyczeć w logu");
    }

    #[test]
    fn dwie_nogi_licza_loty_niezaleznie_ze_wspolnego_balance() {
        use conduit_core::formaty::Lancuch;
        use std::collections::BTreeMap;

        let mut rachunek = Settings::default();
        // DOKUMENT PANELU: celowo sprzeczny z obiema nogami. Gdyby
        // którekolwiek z tych pól wróciło do POLA_RACHUNKU, wynik poniżej
        // przestałby się zgadzać.
        rachunek.lot_mode_percent = false;
        rachunek.lot_fixed = 0.99;
        rachunek.lot_percent = 9.0;
        rachunek.lot_max = 100.0;

        let mut fq5 = Settings::default();
        fq5.lot_mode_percent = true;
        fq5.lot_percent = 0.5;
        fq5.lot_max = 0.01; // sufit NOGI
        let mut fq4 = Settings::default();
        fq4.lot_mode_percent = true;
        fq4.lot_percent = 0.5;
        fq4.lot_max = 0.0; // bez sufitu — skaluje się z saldem

        let mut l = Lancuch {
            nazwa: "TEST-2NOGI".into(),
            ..Default::default()
        };
        l.presety.insert("ZEN".into(), "P-FQ5".into());
        l.presety.insert("Synergy".into(), "P-FQ4".into());
        let mut presety = BTreeMap::new();
        presety.insert("P-FQ5".to_string(), fq5);
        presety.insert("P-FQ4".to_string(), fq4);

        let (silniki, braki) = routing::Silniki::zbuduj(&l, &presety, &rachunek, 3000.0);
        assert!(braki.is_empty());
        let zen = &silniki.lista[silniki.indeks_formatu("ZEN").unwrap()].engine;
        let syn = &silniki.lista[silniki.indeks_formatu("Synergy").unwrap()].engine;

        // wspólny balance 3000: FQ5 = 0,01 (sufit), FQ4 = 3000·0,5 % = 0,15
        assert_eq!(zen.lot_size(3000.0), 0.01);
        assert_eq!(syn.lot_size(3000.0), 0.15);
        // balance rośnie ×2: noga skalowana rośnie, noga z sufitem NIE
        assert_eq!(
            zen.lot_size(6000.0),
            0.01,
            "sufit nogi trzyma niezależnie od salda"
        );
        assert_eq!(
            syn.lot_size(6000.0),
            0.30,
            "noga bez sufitu skaluje się z saldem"
        );
    }

    // ============================================================
    //  EA-21: ROZJAZD PÓL RACHUNKU — GŁOŚNOŚĆ I ZATRZYMANIE
    // ============================================================

    /// Kanał ZEN podpięty, aktywny łańcuch ZENONLY5 (ZEN → FRESHQUEEN-5).
    /// Zwraca stan gotowy do zbudowania nogi z pliku presetu.
    fn stan_z_noga(tag: &str, preset: Settings) -> StateHandle {
        let st = stan(tag);
        st.workspace
            .save_preset(&conduit_core::Preset {
                name: "FRESHQUEEN-5".into(),
                description: String::new(),
                format: "ZEN".into(),
                settings: preset,
                ea: None,
            })
            .unwrap();
        st.update(Sections::one(Section::Settings), |s| {
            s.lancuchy.aktywny = "ZENONLY5".into();
            s.bindings.insert(
                "-100200300".into(),
                ui::ChannelBinding {
                    channel_id: -100_200_300,
                    monitored: true,
                    notify: false,
                    format: "ZEN".into(),
                    topics: Default::default(),
                },
            );
        });
        st
    }

    #[test]
    fn basket_realized_broker_only_noga_i_reload_nie_gubia_osi() {
        for enabled in [false, true] {
            let mut preset = Settings::default();
            preset.basket_realized_broker_only = enabled;
            let st = stan_z_noga(if enabled { "ledger-on" } else { "ledger-off" }, preset);
            st.update(Sections::one(Section::Settings), |s| {
                s.settings["basket_realized_broker_only"] = serde_json::json!(!enabled);
            });
            let mut core = st.read(|s| conduit_server::settings_map::core_from_ui(&s.settings));
            let mut silniki = zbuduj_silniki(&st, &core, 600.0);
            assert!(silniki.lista[0].z_pliku);
            assert_eq!(
                silniki.lista[0].engine.cfg.basket_realized_broker_only,
                enabled
            );
            let mut mtime = std::collections::HashMap::new();
            let stops_level = core.stops_level;
            przeladuj_ustawienia(&st, &mut silniki, &mut core, stops_level, &mut mtime);
            assert_eq!(
                silniki.lista[0].engine.cfg.basket_realized_broker_only,
                enabled
            );
        }
    }

    #[test]
    fn confirmed_exit_retry_noga_i_reload_nie_gubia_osi() {
        for enabled in [false, true] {
            let mut preset = Settings::default();
            preset.confirmed_exit_retry = enabled;
            let st = stan_z_noga(if enabled { "exit-on" } else { "exit-off" }, preset);
            st.update(Sections::one(Section::Settings), |s| {
                s.settings["confirmed_exit_retry"] = serde_json::json!(!enabled);
            });
            let mut core = st.read(|s| conduit_server::settings_map::core_from_ui(&s.settings));
            let mut silniki = zbuduj_silniki(&st, &core, 600.0);
            assert!(silniki.lista[0].z_pliku);
            assert_eq!(silniki.lista[0].engine.cfg.confirmed_exit_retry, enabled);
            let mut mtime = std::collections::HashMap::new();
            let stops_level = core.stops_level;
            przeladuj_ustawienia(&st, &mut silniki, &mut core, stops_level, &mut mtime);
            assert_eq!(silniki.lista[0].engine.cfg.confirmed_exit_retry, enabled);
        }
    }

    #[test]
    fn confirmed_exit_retry_sygnatura_wymusza_zapis_intencji_nie_kazdego_retry() {
        let no_exit = None;
        let first = Some(conduit_core::PendingBasketExit {
            reason: conduit_core::CloseReason::Tp,
            last_attempt_ts: 1000,
        });
        let retry = Some(conduit_core::PendingBasketExit {
            reason: conduit_core::CloseReason::Tp,
            last_attempt_ts: 2000,
        });
        let changed = Some(conduit_core::PendingBasketExit {
            reason: conduit_core::CloseReason::Manual,
            last_attempt_ts: 2000,
        });
        let empty = sygnatura_pending_exit([(1, &no_exit)]);
        let active = sygnatura_pending_exit([(1, &first)]);
        assert_ne!(empty, active);
        assert_eq!(active, sygnatura_pending_exit([(1, &retry)]));
        assert_ne!(active, sygnatura_pending_exit([(1, &changed)]));
        assert_eq!(active, sygnatura_pending_exit([(2, &no_exit), (1, &retry)]));
    }

    #[test]
    fn zdjety_bezpiecznik_nogi_zatrzymuje_handel_i_krzyczy() {
        let mut fq5 = Settings::default();
        fq5.expo_cap_pct = 80.0;
        let st = stan_z_noga("expo", fq5);

        let doc = Settings::default(); // expo_cap_pct = 0 → ZDEJMUJE straż
        let silniki = zbuduj_silniki(&st, &doc, 300.0);
        assert!(
            silniki.lista[0].z_pliku,
            "kontrola testu: noga stoi na pliku presetu"
        );
        assert_eq!(
            silniki.lista[0].engine.cfg.expo_cap_pct, 0.0,
            "kontrola testu: rachunek FAKTYCZNIE nadpisał pole nogi"
        );

        let powod = sprawdz_rozjazd_nog(&st, &silniki, &doc);
        assert!(
            powod.is_some(),
            "zdjęcie bezpiecznika MUSI zatrzymać handel"
        );
        let p = powod.unwrap();
        assert!(
            p.contains("ZEN") && p.contains("expo_cap_pct"),
            "powód ma nazwać nogę i pole: {p}"
        );

        let (jest_error, tresc) = st.read(|s| {
            let l = s
                .logs
                .iter()
                .find(|l| l.title.contains("ROZJAZD BEZPIECZNIKÓW"));
            (
                l.map(|x| x.level == "error").unwrap_or(false),
                l.map(|x| x.content.clone()).unwrap_or_default(),
            )
        });
        assert!(
            jest_error,
            "wpis musi być poziomu ERROR, nie ciszą i nie info"
        );
        assert!(
            tresc.contains("expo_cap_pct"),
            "wpis ma wymieniać pole: {tresc}"
        );
        assert!(
            tresc.contains("80"),
            "wpis ma podać liczbę deklarowaną przez preset: {tresc}"
        );
    }

    /// DRUGA STRONA: nawet ostrzejszy rachunek nie jest tym samym przebiegiem.
    /// Bezpieczeństwo może być większe, ale live nie odtwarza wtedy koronacji,
    /// dlatego pojedyncza noga musi stanąć przed NOWYM wejściem.
    #[test]
    fn ostrzejszy_rachunek_ktory_zmienia_wykonanie_zatrzymuje_handel() {
        let mut fq5 = Settings::default();
        fq5.expo_cap_pct = 80.0;
        let st = stan_z_noga("zgodny", fq5);

        let mut doc = Settings::default();
        doc.expo_cap_pct = 60.0; // OSTRZEJ, ale INACZEJ niż koronowany preset
        let silniki = zbuduj_silniki(&st, &doc, 300.0);
        assert!(
            sprawdz_rozjazd_nog(&st, &silniki, &doc).is_some(),
            "każda zmiana ścieżki wykonania pojedynczej nogi musi zatrzymać nowe wejścia"
        );
        let blad = st.read(|s| {
            s.logs
                .iter()
                .any(|l| l.level == "error" && l.title.contains("ROZJAZD PARYTETU"))
        });
        assert!(
            blad,
            "panel ma jednoznacznie nazwać rozjazd live/backtest"
        );
    }

    /// Wlasnosci brokera i uruchomienia maja byc jawne, ale nie zmieniaja
    /// polityki wejsc/wyjsc presetu. To jest dokladnie dopuszczony rozjazd
    /// wdrożenia wielobrokerowego: swap rachunku i ręczne prowadzenie terminala.
    #[test]
    fn roznice_brokera_i_runtime_ostrzegaja_ale_nie_zatrzymuja() {
        let mut preset = Settings::default();
        // GOD-X7 deliberately carries zero simulated long swap, whereas the
        // The account document may record a broker-specific long swap.
        // Make that broker-only difference explicit in this synthetic fixture.
        preset.swap_long_points = 0.0;
        let st = stan_z_noga("broker", preset);
        let mut doc = Settings::default();
        doc.swap_long_points = -75.82;
        doc.swap_rollover_weekday = 2;
        doc.mt5_autostart = false;
        doc.mt5_watchdog = false;
        let silniki = zbuduj_silniki(&st, &doc, 300.0);

        assert!(
            sprawdz_rozjazd_nog(&st, &silniki, &doc).is_none(),
            "broker/runtime nie moze blokowac zgodnego algorytmu"
        );
        let (warn, error) = st.read(|s| {
            (
                s.logs.iter().any(|l| {
                    l.level == "warn" && l.title.contains("Rachunek nadpisuje 4")
                }),
                s.logs.iter().any(|l| l.level == "error"),
            )
        });
        assert!(warn, "cztery dopuszczone roznice musza byc jawne");
        assert!(!error, "dopuszczone roznice nie sa bledem parytetu");
    }

    /// (druga połowa W19) PLIK JEST, TYLKO `name` W ŚRODKU MÓWI CO INNEGO.
    ///
    /// Tożsamością presetu jest pole `name` w pliku, nie nazwa pliku. Operator
    /// widzi `presets/STORM-1.json` w katalogu i komunikat „wgraj brakujący
    /// plik" — czyli dokładnie ten stan, którego nie da się rozwiązać z panelu.
    #[test]
    fn preset_o_innej_nazwie_w_srodku_daje_wskazowke() {
        let st = stan("w19");
        std::fs::create_dir_all(st.workspace.presets_dir()).unwrap();
        std::fs::write(
            st.workspace.presets_dir().join("STORM-1.json"),
            serde_json::to_string(&conduit_core::Preset {
                name: "STORM-1a".into(),
                description: String::new(),
                format: "STORM".into(),
                settings: Settings::default(),
                ea: None,
            })
            .unwrap(),
        )
        .unwrap();

        let w = wskazowka_o_presecie(&st, "STORM-1").expect("plik jest — musi być wskazówka");
        assert!(
            w.contains("STORM-1a"),
            "wskazówka ma podać PRAWDZIWĄ nazwę z pliku: {w}"
        );
        assert!(w.contains("STORM-1"), "i nazwę, o którą pyta łańcuch");

        // preset, którego naprawdę nie ma na dysku, nie dostaje wskazówki —
        // istniejący komunikat („wgraj brakujący plik") jest wtedy prawdziwy
        assert!(wskazowka_o_presecie(&st, "NIE-MA-TAKIEGO").is_none());
    }
}
