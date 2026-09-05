//! REGRESJA: tryb „każdy dzień osobno" resetuje KONTO, nie WIEDZĘ O RYNKU.
//!
//! Tryb `--daily-reset` tworzy co dobę świeży silnik. Dopóki razem z saldem
//! kasował bufory historii ceny, filtr reżimu z oknem 72 h nie miał szans
//! zebrać próbek (jeden punkt na godzinę, doba ma 24) i **przepuszczał
//! wszystko** — cicho, bez jednej linii w logu.
//!
//! Skutek jest testowany na danych syntetycznych: po przeniesieniu historii
//! przez granicę doby filtr podejmuje te same decyzje co odpowiadający mu
//! przebieg ciągły, więc oba tryby mierzą tę samą strategię.
//!
//! Test nie przypina kwot (te zależą od danych), tylko sam fakt: filtr reżimu
//! MUSI zmieniać wynik w trybie dziennego resetu.

use conduit_backtest::data::{ReplayMessage, TickData};
use conduit_backtest::runner::{run, RunConfig};
use conduit_core::settings::{RegimeFilter, Settings};
use conduit_core::types::Ts;

fn zapisz_ticki(sciezka: &std::path::Path, ticki: &[(Ts, f32, f32)]) {
    let mut buf = vec![0u8; 64];
    buf[0..4].copy_from_slice(&0x4B54_4443u32.to_le_bytes());
    buf[8..16].copy_from_slice(&(ticki.len() as u64).to_le_bytes());
    for (ts, bid, ask) in ticki {
        buf.extend_from_slice(&ts.to_le_bytes());
        buf.extend_from_slice(&bid.to_le_bytes());
        buf.extend_from_slice(&ask.to_le_bytes());
    }
    std::fs::write(sciezka, buf).unwrap();
}

fn ustawienia(filtr: RegimeFilter) -> Settings {
    let mut s = Settings::default();
    s.regime_filter = filtr;
    s.regime_ma_hours = 72.0;
    s.session_filter = false;
    s.exec_latency_ms = 0;
    s.entry_units = 1;
    s.skip_if_sl_breached = false;
    s.journal_enabled = false;
    s.max_open_baskets = 0;
    s.max_open_positions = 0;
    s.streak_pause_n = 0;
    s.oae_timeout_min = 0.0;
    s
}

#[test]
fn filtr_rezimu_dziala_takze_przy_dziennym_resecie() {
    let dir = std::env::temp_dir().join("conduit_test_reset_rynku");
    std::fs::create_dir_all(&dir).unwrap();
    let plik = dir.join("ticks.bin");

    // 8 dni ticków co minutę, cena rośnie liniowo 4000 → 4160.
    // Po pierwszych 72 h średnia z okna leży PONIŻEJ ceny, więc `CounterMa`
    // odrzuca każde kupno, a `Off` je przyjmuje.
    let t0: Ts = 1_775_000_000_000 - 1_775_000_000_000 % 86_400_000;
    let n = 8 * 24 * 60;
    let ticki: Vec<(Ts, f32, f32)> = (0..n)
        .map(|i| {
            let ts = t0 + i as i64 * 60_000;
            let px = 4000.0 + i as f32 * 160.0 / n as f32;
            (ts, px, px + 0.20)
        })
        .collect();
    zapisz_ticki(&plik, &ticki);
    let dane = TickData::open(&plik).unwrap();

    // Jedno kupno dziennie o 12:00 czasu ticków, strefa wokół ceny bieżącej.
    let mut msgs: Vec<ReplayMessage> = Vec::new();
    for d in 0..8i64 {
        let ts_tick = t0 + d * 86_400_000 + 12 * 3_600_000;
        let idx = (ts_tick - t0) / 60_000;
        let px = 4000.0 + idx as f64 * 160.0 / n as f64;
        msgs.push(ReplayMessage {
            kanal: String::new(),
            // znacznik wiadomości jest w czasie Telegrama (UTC), a runner
            // dodaje do niego strefę serwera
            ts: ts_tick - 3 * 3_600_000,
            telegram_published_ts: None,
            msg_id: 1000 + d,
            reply_to: None,
            edit_of: None,
            text: format!(
                "BUY GOLD @ {:.2}/{:.2}\nTP {:.2}\nSL {:.2}",
                px + 0.5,
                px - 0.5,
                px + 6.0,
                px - 6.0
            ),
        });
    }

    let przebieg = |filtr: RegimeFilter, daily: bool| {
        let cfg = RunConfig {
            from: t0,
            to: t0 + 8 * 86_400_000,
            start_balance: 200.0,
            settings: ustawienia(filtr),
            daily_reset: daily,
            ..Default::default()
        };
        let r = run(&dane, &msgs, &cfg);
        r.metrics.trades
    };

    let bez_filtra_c = przebieg(RegimeFilter::Off, false);
    let z_filtrem_c = przebieg(RegimeFilter::CounterMa, false);
    let bez_filtra_d = przebieg(RegimeFilter::Off, true);
    let z_filtrem_d = przebieg(RegimeFilter::CounterMa, true);

    println!(
        "compounding: Off {bez_filtra_c} / CounterMa {z_filtrem_c}   \
         daily-reset: Off {bez_filtra_d} / CounterMa {z_filtrem_d}"
    );

    assert!(
        bez_filtra_c > z_filtrem_c,
        "warunek testu: w compoundingu filtr reżimu MUSI coś odciąć \
         (Off {bez_filtra_c}, CounterMa {z_filtrem_c})"
    );
    assert!(
        bez_filtra_d > z_filtrem_d,
        "filtr reżimu jest MARTWY w trybie dziennego resetu: Off {bez_filtra_d} \
         transakcji, CounterMa {z_filtrem_d} — historia ceny została skasowana \
         razem z saldem, więc średnia 72 h nigdy nie zebrała próbek"
    );

    let _ = std::fs::remove_file(&plik);
}
