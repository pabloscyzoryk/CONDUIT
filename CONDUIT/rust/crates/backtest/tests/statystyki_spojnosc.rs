
use conduit_backtest::data::{ReplayMessage, TickData};
use conduit_backtest::runner::{run, RunConfig};
use conduit_core::settings::Settings;
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

const T0: Ts = 1_775_000_000_000 - 1_775_000_000_000 % 86_400_000;
const DNI: i64 = 12;

/// Cena faluje sinusoidą, więc część koszyków kończy się celem, a część
/// stopem. Dwa sygnały dziennie: jeden w sesji, drugi w nocy — ten drugi
/// odpada na `session_filter` i karmi rejestr odrzutów, na którym stoi lejek.
fn dane(dir: &std::path::Path) -> (TickData, Vec<ReplayMessage>) {
    let plik = dir.join("ticks_e.bin");
    let n = DNI * 24 * 60;
    let ticki: Vec<(Ts, f32, f32)> = (0..n)
        .map(|i| {
            let ts = T0 + i * 60_000;
            let f = i as f32 / 240.0;
            let px = 4000.0 + 8.0 * f.sin() + (i as f32) * 0.002;
            (ts, px, px + 0.20)
        })
        .collect();
    zapisz_ticki(&plik, &ticki);
    let dane = TickData::open(&plik).unwrap();

    let cena = |ts_tick: Ts| {
        let i = (ts_tick - T0) / 60_000;
        let f = i as f64 / 240.0;
        4000.0 + 8.0 * f.sin() + (i as f64) * 0.002
    };
    let mut msgs: Vec<ReplayMessage> = Vec::new();
    for d in 0..DNI {
        for (nr, godz) in [(0i64, 10i64), (1, 2)] {
            let ts_tick = T0 + d * 86_400_000 + godz * 3_600_000;
            let px = cena(ts_tick);
            msgs.push(ReplayMessage {
                kanal: String::new(),
                // znacznik wiadomości jest w czasie Telegrama (UTC) — runner
                // dokłada do niego strefę serwera
                ts: ts_tick - 3 * 3_600_000,
                telegram_published_ts: None,
                msg_id: 1000 + d * 10 + nr,
                reply_to: None,
                edit_of: None,
                text: format!(
                    "BUY GOLD @ {:.2}/{:.2}\nTP {:.2}\nSL {:.2}",
                    px + 0.6,
                    px - 0.6,
                    px + 3.0,
                    px - 3.0
                ),
            });
        }
    }
    (dane, msgs)
}

fn ustawienia(prog_be: f64) -> Settings {
    let mut s = Settings::default();
    // sesja WŁĄCZONA — ona produkuje odrzuty, na których stoi lejek
    s.session_filter = true;
    s.session_hours = "8-16".into();
    s.exec_latency_ms = 0;
    s.entry_units = 2;
    s.auto_limit = false;
    s.skip_if_sl_breached = false;
    s.journal_enabled = false;
    s.max_open_baskets = 0;
    s.max_open_positions = 0;
    s.streak_pause_n = 0;
    s.oae_timeout_min = 0.0;
    s.regime_filter = conduit_core::settings::RegimeFilter::Off;
    s.stat_be_prog_usd = prog_be;
    s
}

fn przebieg(dir: &std::path::Path, prog_be: f64) -> conduit_backtest::runner::RunResult {
    let (ticks, msgs) = dane(dir);
    let cfg = RunConfig {
        from: T0,
        to: T0 + DNI * 86_400_000,
        start_balance: 2000.0,
        settings: ustawienia(prog_be),
        ..Default::default()
    };
    run(&ticks, &msgs, &cfg)
}

/// Rozkłady muszą sumować się do liczb, które raport pokazywał dotąd.
#[test]
fn rozklady_domykaja_sie_na_przebiegu() {
    let dir = std::env::temp_dir().join("conduit_stat_e_1");
    std::fs::create_dir_all(&dir).unwrap();
    let r = przebieg(&dir, 0.0);
    let s = &r.metrics.stat_sygnalow;

    assert!(r.metrics.trades > 0, "test bez handlu niczego nie mierzy");
    assert!(
        s.koszyki.z_pozycjami > 0,
        "test bez koszyka niczego nie mierzy"
    );

    // E2: kubełki wyniku wyczerpują koszyki z handlem
    assert_eq!(
        s.koszyki.win + s.koszyki.loss + s.koszyki.be,
        s.koszyki.z_pozycjami,
        "W/L/BE musi wyczerpywać koszyki z handlem"
    );
    assert_eq!(s.koszyki.z_pozycjami + s.koszyki.bez_fillu, s.koszyki.total);
    assert_eq!(
        s.koszyki.total, r.metrics.baskets,
        "liczba koszyków ta sama co w metrykach"
    );

    // powody śmierci: sztuki i dolary
    let n_powodow: u32 = s.koszyki.powody.values().map(|k| k.n).sum();
    let usd_powodow: f64 = s.koszyki.powody.values().map(|k| k.usd).sum();
    assert_eq!(
        n_powodow, s.koszyki.z_pozycjami,
        "suma rozkładu = liczba koszyków"
    );
    assert!(
        (usd_powodow - s.koszyki.suma_usd).abs() < 1e-6,
        "dolary rozkładu {usd_powodow} ≠ suma koszyków {}",
        s.koszyki.suma_usd
    );

    // histogram wypełnień obejmuje KAŻDY koszyk z planem siatki
    assert_eq!(
        s.koszyki.fill_hist.iter().sum::<u32>(),
        s.koszyki.total,
        "histogram wypełnień musi liczyć także koszyki, w których nic nie weszło"
    );

    // E3: lejek się domyka
    assert_eq!(
        s.lejek.koszyki + s.lejek.odrzucone_razem,
        s.lejek.sygnaly_widziane,
        "koszyki + odrzucone = widziane"
    );
    assert!(
        s.lejek.odrzucone_razem > 0,
        "nocne sygnały mają odpaść na sesji"
    );
    assert_eq!(
        s.lejek.odrzucone.values().sum::<u32>(),
        s.lejek.odrzucone_razem,
        "rozbicie odrzutów po kodach = suma"
    );
    assert!(
        s.lejek.koszt_filtrow.contains_key("SessionClosed"),
        "filtr sesji ma być wyceniony, a nie tylko policzony"
    );
    assert!(
        s.lejek.wykonanych_pct > 0.0 && s.lejek.wykonanych_pct <= 100.0,
        "%wykonanych poza zakresem: {}",
        s.lejek.wykonanych_pct
    );

    // E4: strony rynku wyczerpują transakcje
    assert_eq!(
        s.transakcje.buy.n + s.transakcje.sell.n,
        r.metrics.trades,
        "BUY + SELL = wszystkie transakcje"
    );
    assert!(s.transakcje.spread_usd > 0.0, "spread musi być policzony");
    assert_eq!(
        s.transakcje.sl_tp_same_tick, 0,
        "na danych tickowych jeden tick nie może przebić SL i TP naraz"
    );
}

/// PARYTET E1: próg remisu przesuwa WYŁĄCZNIE kubełek remisów.
///
/// Gdyby ruszył `wins`/`losses`, zmieniłby razem z nimi `profit_factor`,
/// `avg_loss` i `max_consecutive_losses` — czyli liczby, po których porównuje
/// się całe archiwum `wyniki_*.json`.
#[test]
fn prog_be_nie_rusza_zadnej_starej_liczby() {
    let dir = std::env::temp_dir().join("conduit_stat_e_2");
    std::fs::create_dir_all(&dir).unwrap();
    let zero = przebieg(&dir, 0.0);
    let szeroki = przebieg(&dir, 1.0);

    let (a, b) = (&zero.metrics, &szeroki.metrics);
    assert_eq!(a.trades, b.trades);
    assert_eq!((a.wins, a.losses), (b.wins, b.losses));
    assert_eq!(a.baskets, b.baskets);
    assert_eq!(a.max_consecutive_losses, b.max_consecutive_losses);
    assert!(
        (a.total_profit - b.total_profit).abs() < 1e-9,
        "zysk musi być identyczny"
    );
    assert!(
        (a.win_rate - b.win_rate).abs() < 1e-12,
        "dawna definicja win_rate nietknięta"
    );
    assert!((a.profit_factor - b.profit_factor).abs() < 1e-12);
    // ...a jedyna różnica siedzi w nowym kubełku
    assert!(b.bes >= a.bes, "szerszy próg nie może złapać MNIEJ remisów");
}
