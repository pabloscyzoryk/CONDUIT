
use conduit_backtest::data::TickData;
use conduit_backtest::runner::{run, RunConfig};

const DZIEN: i64 = 86_400_000;

/// Ticki leżą w `rust/data/`, a testy startują z katalogu SKRZYNKI
/// (`crates/backtest`) — patrz `rozgrzewka_nie_jest_noopem::sciezka_tickow`.
fn sciezka(nazwa: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .join(nazwa)
}

fn ticki() -> Option<TickData> {
    TickData::open(sciezka("ticks.bin")).ok()
}

/// Północ pierwszej doby, która NAPRAWDĘ ma ticki (weekend i święto jej nie mają,
/// a test o „dzień ma ticki, zero-dniowe okno nie ma" na pustej dobie nic nie mówi).
fn doba_z_tickami(t: &TickData) -> Option<i64> {
    (t.first_ts() / DZIEN..=t.last_ts() / DZIEN)
        .map(|d| d * DZIEN)
        .find(|&od| t.index_at(od + DZIEN) > t.index_at(od))
}

/// `--to` jest granicą WYŁĄCZNĄ: `[X, X)` to okno puste, `[X, X+doba)` to jeden dzień.
#[test]
fn to_jest_granica_wylaczna() {
    let Some(t) = ticki() else {
        eprintln!("brak data/ticks.bin — pomijam");
        return;
    };
    let od = doba_z_tickami(&t).expect("plik ticków nie ma ani jednej doby z notowaniami");

    let zerowe = run(
        &t,
        &[],
        &RunConfig {
            from: od,
            to: od,
            ..Default::default()
        },
    );
    assert_eq!(
        zerowe.ticks_processed, 0,
        "okno [X, X) przemieliło {} ticków — `--to` przestało być granicą WYŁĄCZNĄ. \
         Jeśli to zmiana zamierzona, przelicz na nowo KAŻDY wynik archiwalny \
         (PARYTET.md, RAPORT.md): każde okno przesunęło się o dobę.",
        zerowe.ticks_processed
    );

    let dzien = run(
        &t,
        &[],
        &RunConfig {
            from: od,
            to: od + DZIEN,
            ..Default::default()
        },
    );
    assert!(
        dzien.ticks_processed > 0,
        "okno [X, X+doba) nie przemieliło ANI JEDNEGO ticka, choć doba {od} ma notowania — \
         to znaczy, że zepsuło się wycinanie okna, a nie sama granica"
    );
}

/// Puste okno kończy CLI błędem, nie tabelą zer.
#[test]
fn cli_odmawia_liczenia_pustego_okna() {
    if ticki().is_none() {
        eprintln!("brak data/ticks.bin — pomijam");
        return;
    }
    let sygnaly = sciezka("signals.json");
    if !sygnaly.exists() {
        eprintln!("brak data/signals.json — pomijam");
        return;
    }
    let out = std::env::temp_dir().join("conduit_test_puste_okno");

    let wy = std::process::Command::new(env!("CARGO_BIN_EXE_btp"))
        .args([
            "--ticks".as_ref(),
            sciezka("ticks.bin").as_os_str(),
            "--signals".as_ref(),
            sygnaly.as_os_str(),
            "--from".as_ref(),
            "2026-08-06".as_ref(),
            "--to".as_ref(),
            "2026-08-06".as_ref(),
            "--balance".as_ref(),
            "400".as_ref(),
            "--out".as_ref(),
            out.as_os_str(),
        ])
        .output()
        .expect("nie udało się uruchomić btp");

    let err = String::from_utf8_lossy(&wy.stderr);
    let std_out = String::from_utf8_lossy(&wy.stdout);

    assert!(
        !wy.status.success(),
        "puste okno zakończyło się SUKCESEM (kod 0) — skrypt przemiatający weźmie \
         te zera za wynik.\nstdout:\n{std_out}\nstderr:\n{err}"
    );
    assert!(
        err.contains("PUSTE OKNO"),
        "błąd nie mówi, że okno jest puste:\n{err}"
    );
    assert!(
        err.contains("--from 2026-08-06 --to 2026-08-07"),
        "błąd nie podaje gotowej poprawki (okno jednodniowe):\n{err}"
    );
    // Sedno zgłoszenia: nie chodzi o to, żeby DOPISAĆ ostrzeżenie do tabeli zer,
    // tylko żeby tabela zer w ogóle nie powstała.
    assert!(
        !std_out.contains("equity końcowe"),
        "puste okno mimo błędu WYDRUKOWAŁO podsumowanie wyników:\n{std_out}"
    );
}
