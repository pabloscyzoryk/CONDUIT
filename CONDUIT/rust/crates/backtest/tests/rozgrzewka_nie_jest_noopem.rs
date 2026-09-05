//! ROZGRZEWKA MUSI DAĆ TYLE PUNKTÓW, ILE FILTR WYMAGA — inaczej jest no-opem.
//!
//! # Skąd ten test
//!
//! `--rozgrzewka-h` w `lotto.exe` dawał wyniki **identyczne co do centa**
//! z przebiegiem zimnym, dla czterech presetów naraz. Zgłosił to zespół Fable
//! i mieli rację, choć przyczyna nie była tam, gdzie jej szukano: wywołanie
//! `set_market_history` działało, bufor był wypełniany — ale ZA KRÓTKO.
//!
//! `price_hist` dostaje jeden punkt na godzinę **z ticków**, a złoto nie ma
//! ticków w weekend ani w przerwie dobowej. Okno liczone kalendarzowo
//! (`od − godzin·3 600 000`) przy starcie w poniedziałek obejmowało
//! piątek→poniedziałek, czyli **~24 punkty zamiast 72**. A `regime_ok` przy
//! `price_hist.len() < n` **przepuszcza każdy sygnał**:
//!
//! ```text
//! if self.price_hist.len() < n.max(2) { return true; }
//! ```
//!
//! Filtr milczał tak samo jak przy zimnym starcie, więc liczby wychodziły
//! identyczne — i wyglądało to na „oś, która nic nie zmienia".
//!
//! # Czego ten test pilnuje
//!
//! Że dla **każdego dnia tygodnia** (w tym poniedziałku po weekendzie)
//! rozgrzewka na 72 h oddaje **co najmniej 72 punkty**. To jest dokładnie
//! warunek, od którego zależy, czy filtr reżimu w ogóle się odezwie.
//!
//! Nasza reguła: *dwa wyniki identyczne co do centa to podejrzenie martwej
//! osi, nie potwierdzenie.*

use conduit_backtest::data::TickData;
use conduit_backtest::sim::SimBroker;
use conduit_core::engine::Engine;

/// Ticki leza w `rust/data/`, a testy startuja z katalogu SKRZYNKI
/// (`crates/backtest`). Sciezka wzgledna cicho nie trafiala w plik, test
/// wychodzil przez `else { return }` i **przechodzil, nie sprawdzajac niczego**
/// — czyli byl dokladnie ta klasa atrapy, ktora ma tu lapac.
fn sciezka_tickow() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/ticks.bin")
}
use conduit_core::settings::Settings;

/// 72 h — `regime_ma_hours` HYPER-2, HYPER-X1, KOAN-2 i FRESHKING-1.
const GODZIN: usize = 72;

fn ticki() -> Option<TickData> {
    TickData::open(sciezka_tickow()).ok()
}

#[test]
fn rozgrzewka_daje_dosc_punktow_na_kazdy_dzien_tygodnia() {
    let Some(t) = ticki() else {
        eprintln!("brak data/ticks.bin — pomijam");
        return;
    };
    let s = Settings::default();
    const DZIEN: i64 = 86_400_000;

    let pierwszy = t.first_ts();
    let ostatni = t.last_ts();
    // Startujemy dopiero, gdy przed dniem startu jest DOŚĆ danych na rozgrzewkę
    // (okno cofa się o `godzin*2 + 120` h), żeby nie mierzyć krawędzi pliku.
    let od_dnia = (pierwszy + (GODZIN as i64 * 2 + 120) * 3_600_000) / DZIEN + 1;
    let do_dnia = ostatni / DZIEN;

    let mut sprawdzonych = 0usize;
    let mut za_krotkie: Vec<(i64, usize)> = Vec::new();

    for d in od_dnia..=do_dnia {
        let od = d * DZIEN;
        // dzień bez ticków = święto/weekend; nie ma czego rozgrzewać
        if t.index_at(od + DZIEN) <= t.index_at(od) {
            continue;
        }
        let (price, _vol) = conduit_backtest::runner::historia_z_tickow(&t, od, GODZIN, &s);
        sprawdzonych += 1;
        if price.len() < GODZIN {
            za_krotkie.push((d, price.len()));
        }
    }

    assert!(
        sprawdzonych > 20,
        "za mało dni do sprawdzenia ({sprawdzonych})"
    );
    assert!(
        za_krotkie.is_empty(),
        "ROZGRZEWKA JEST NO-OPEM w {} z {} dni handlowych — filtr reżimu \
         przepuści wtedy KAŻDY sygnał, a wynik wyjdzie co do centa taki sam \
         jak przy zimnym starcie.\nDni (numer doby, punktów zamiast {}): {:?}",
        za_krotkie.len(),
        sprawdzonych,
        GODZIN,
        &za_krotkie[..za_krotkie.len().min(10)]
    );
}

/// Bufor zmienności też ma być wypełniony — `vol_factor` chce ≥5 próbek
/// w oknie, inaczej zwraca 1.0 i mnożnik jednostek milczy.
#[test]
fn rozgrzewka_wypelnia_takze_bufor_zmiennosci() {
    let Some(t) = ticki() else { return };
    let mut s = Settings::default();
    s.vol_window_min = 60.0;
    const DZIEN: i64 = 86_400_000;

    let od = (t.first_ts() + (GODZIN as i64 * 2 + 120) * 3_600_000) / DZIEN * DZIEN + 5 * DZIEN;
    let (_price, vol) = conduit_backtest::runner::historia_z_tickow(&t, od, GODZIN, &s);
    assert!(
        vol.len() >= 5,
        "bufor zmienności ma {} próbek — `vol_factor` zwróci 1.0 i reguła \
         zmienności będzie milczeć mimo rozgrzewki",
        vol.len()
    );
}

/// Zimny start (`0`) ma dalej znaczyć ZIMNY — na tym stoi odtwarzalność
/// wcześniejszych liczb LOTTO i bramka parytetu.
#[test]
fn zero_godzin_dalej_znaczy_zimny_start() {
    let Some(t) = ticki() else { return };
    let s = Settings::default();
    let od = t.first_ts() + 20 * 86_400_000;
    let (price, vol) = conduit_backtest::runner::historia_z_tickow(&t, od, 0, &s);
    assert!(
        price.is_empty() && vol.is_empty(),
        "przy 0 h nie wolno wczytać ani jednego punktu"
    );
}

/// Mały, samowystarczalny plik CDTK. Dzięki niemu test semantyki M1 nie
/// zależy od wielkiego eksportu ticków ani nie może przejść „na zielono" po
/// cichym `return`, gdy pliku zabraknie na CI.
fn syntetyczne_ticki(minut: usize) -> (std::path::PathBuf, TickData, i64) {
    const HEADER: usize = 64;
    const MAGIC: u32 = 0x4B54_4443;
    const M1: i64 = 60_000;
    let base = 1_700_000_000_000i64.div_euclid(M1) * M1;
    let mut rekordy: Vec<(i64, f32, f32)> = Vec::with_capacity(minut * 3);
    for m in 0..minut {
        // Pierwszy tick dokładnie na granicy minuty: stan warmup i stan
        // przeżyty tick-po-ticku mają wtedy także identyczne stemple
        // potwierdzeń swingów, nie tylko identyczne ceny.
        for (j, dt) in [0i64, 20_000, 59_000].into_iter().enumerate() {
            let fala = ((m * 17 + j * 11) % 31) as f64 * 0.07;
            let mid = 2_000.0 + (m as f64 * 0.013) + if j == 1 { 0.9 } else { fala };
            let spread = 0.10 + ((m + j) % 7) as f64 * 0.01;
            rekordy.push((
                base + m as i64 * M1 + dt,
                (mid - spread * 0.5) as f32,
                (mid + spread * 0.5) as f32,
            ));
        }
    }

    let mut bytes = vec![0u8; HEADER];
    bytes[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    bytes[8..16].copy_from_slice(&(rekordy.len() as u64).to_le_bytes());
    for (ts, bid, ask) in rekordy {
        bytes.extend_from_slice(&ts.to_le_bytes());
        bytes.extend_from_slice(&bid.to_le_bytes());
        bytes.extend_from_slice(&ask.to_le_bytes());
    }
    let unikat = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "conduit_sr_m1_{}_{}.bin",
        std::process::id(),
        unikat
    ));
    std::fs::write(&path, bytes).expect("zapis syntetycznych tickow");
    let ticks = TickData::open(&path).expect("otwarcie syntetycznych tickow");
    (path, ticks, base)
}

#[test]
fn sr_m1_jest_deterministyczne_i_nie_czyta_formujacej_sie_minuty() {
    const M1: i64 = 60_000;
    let (path, ticks, base) = syntetyczne_ticki(130);
    let od = base + 90 * M1 + 30_000;
    let a = conduit_backtest::runner::swiece_m1_sr_z_tickow(&ticks, od, 1);
    let b = conduit_backtest::runner::swiece_m1_sr_z_tickow(&ticks, od, 1);
    assert_eq!(a, b, "ta sama historia musi dać bitowo ten sam warmup M1");
    assert_eq!(a.len(), 90);
    assert_eq!(a.first().unwrap().ts, base);
    assert_eq!(a.last().unwrap().ts, base + 89 * M1);
    assert!(a.iter().all(|bar| bar.ts + M1 <= od.div_euclid(M1) * M1));

    // Close i spread to dokładnie ostatni tick domkniętej minuty.
    let q = ticks.quote(90 * 3 - 1);
    assert_eq!(a.last().unwrap().close, q.mid());
    assert_eq!(a.last().unwrap().spread, q.spread());

    drop(ticks);
    let _ = std::fs::remove_file(path);
}

#[test]
fn sr_m1_zero_godzin_to_pusty_kosztowo_legacy_warmup() {
    let (path, ticks, base) = syntetyczne_ticki(3);
    assert!(
        conduit_backtest::runner::swiece_m1_sr_z_tickow(&ticks, base + 60_000, 0).is_empty(),
        "0 h nie może zbudować ani jednej świecy S/R"
    );
    drop(ticks);
    let _ = std::fs::remove_file(path);
}

#[test]
fn sr_m1_cropped_plus_warmup_odtwarza_stan_naturalnego_okna() {
    const M1: i64 = 60_000;
    let (path, ticks, base) = syntetyczne_ticki(210);
    let od = base + 180 * M1;
    let mut s = Settings::default();
    s.trail_sr_enabled = true;
    s.trail_sr_tf_min = 5;
    s.trail_sr_fractal_n = 2;
    s.trail_sr_struct_window_h = 1;
    s.trail_sr_atr_period = 5;
    s.trail_sr_min_prominence_atr = 0.20;
    s.trail_sr_offset_atr_mult = 0.75;
    s.trail_sr_offset_spread_mult = 1.50;

    // Wariant naturalny: silnik naprawdę przeżywa wszystkie ticki sprzed `od`.
    let mut naturalny = Engine::new(s.clone(), 600.0);
    let mut broker = SimBroker::z_ustawien(600.0, &s);
    for i in 0..ticks.index_at(od) {
        let q = ticks.quote(i);
        let _ = broker.on_quote(q);
        naturalny.on_tick(&mut broker, &q);
    }

    // Wariant cropped: startuje dopiero w `od`, lecz dostaje dokładnie to,
    // co ścieżka produkcyjna odbudowuje z domkniętych świec M1.
    let bars = conduit_backtest::runner::swiece_m1_sr_z_tickow(&ticks, od, 1);
    let mut cropped = Engine::new(s, 600.0);
    assert!(
        cropped.rozgrzej_sr_z_m1(&bars),
        "dynamiczne S/R powinno być gotowe"
    );
    assert_eq!(
        cropped.stan_sr(),
        naturalny.stan_sr(),
        "cropped+warmup musi odtworzyć dokładnie ten sam StanSr co pełny strumień"
    );

    drop(ticks);
    let _ = std::fs::remove_file(path);
}
