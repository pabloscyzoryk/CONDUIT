
use conduit_backtest::data::{ReplayMessage, TickData};
use conduit_backtest::runner::{run, FormatCfg, RunConfig, SzczebelCfg};
use conduit_core::formaty::PulapyGlobalne;
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

/// Ustawienia bez filtrów — testujemy DRABINKĘ, nie bramki wejścia.
fn ustawienia() -> Settings {
    let mut s = Settings::default();
    s.session_filter = false;
    s.exec_latency_ms = 0;
    s.entry_units = 1;
    s.skip_if_sl_breached = false;
    s.journal_enabled = false;
    s.max_open_baskets = 0;
    s.max_open_positions = 0;
    s.streak_pause_n = 0;
    s.oae_timeout_min = 0.0;
    s.regime_filter = conduit_core::settings::RegimeFilter::Off;
    s
}

const T0: Ts = 1_775_000_000_000 - 1_775_000_000_000 % 86_400_000;

/// Ścieżka ceny zadana odcinkami: `(minuta_od, cena)` — od tej minuty cena
/// stoi na tym poziomie. Spread stały 0,20.
fn dane(dir: &str, minut: usize, odcinki: &[(usize, f32)]) -> TickData {
    let d = std::env::temp_dir().join(dir);
    std::fs::create_dir_all(&d).unwrap();
    let plik = d.join("ticks.bin");
    let mut cena = odcinki.first().map(|(_, c)| *c).unwrap_or(4000.0);
    let ticki: Vec<(Ts, f32, f32)> = (0..minut)
        .map(|i| {
            if let Some((_, c)) = odcinki.iter().rev().find(|(m, _)| *m <= i) {
                cena = *c;
            }
            (T0 + i as i64 * 60_000, cena, cena + 0.20)
        })
        .collect();
    zapisz_ticki(&plik, &ticki);
    TickData::open(&plik).unwrap()
}

/// Sygnał BUY w strefie wokół 4000 z zadanym TP i SL.
fn sygnal(nr: i64, minuta: i64, kanal: &str, tp: f64, sl: f64) -> ReplayMessage {
    ReplayMessage {
        ts: T0 + minuta * 60_000 - 3 * 3_600_000,
        telegram_published_ts: None,
        msg_id: nr,
        reply_to: None,
        edit_of: None,
        text: format!("BUY GOLD @ 4001.00/3999.00\nTP {tp:.2}\nSL {sl:.2}"),
        kanal: kanal.to_string(),
    }
}

/// Sygnał SELL w strefie wokół 4000 z zadanym TP i SL.
///
/// Potrzebny, bo domyślne ustawienia PRZYCINAJĄ stop do ~5–6 $ od wejścia —
/// „daleki" SL 3800 przy BUY naprawdę stoi na ~3994 (złapane debugiem:
/// koszyk z SL 3800 domknął się `reason=Sl` przy 3994). Test zejścia
/// potrzebuje pozycji, której NIE ruszy zanurzenie w dół — czyli SELL-a.
fn sygnal_sell(nr: i64, minuta: i64, kanal: &str, tp: f64, sl: f64) -> ReplayMessage {
    ReplayMessage {
        ts: T0 + minuta * 60_000 - 3 * 3_600_000,
        telegram_published_ts: None,
        msg_id: nr,
        reply_to: None,
        edit_of: None,
        text: format!("SELL GOLD @ 3999.00/4001.00\nTP {tp:.2}\nSL {sl:.2}"),
        kanal: kanal.to_string(),
    }
}

fn noga(format: &str, preset: &str) -> FormatCfg {
    FormatCfg {
        format: format.into(),
        preset: preset.into(),
        settings: ustawienia(),
    }
}

fn szczebel(prog: f64, nazwa: &str, nogi: Vec<FormatCfg>) -> SzczebelCfg {
    SzczebelCfg {
        prog,
        nazwa: nazwa.into(),
        formaty: nogi,
        pulapy: PulapyGlobalne::default(),
    }
}

// ============================================================
//  1. BRAMKA NIERUSZANIA
// ============================================================

/// Drabinka o JEDNYM szczeblu z progiem 0 to dokładnie `--preset-format`:
/// ten sam silnik, ten sam routing, zero przełączeń. Każda różnica choćby
/// o cent znaczy, że mechanizm drabinki dodaje dryf do zwykłego przebiegu —
/// czyli że wszystkie tabele SENTINEL-0 przestały być porównywalne.
#[test]
fn drabinka_jednoszczeblowa_rowna_preset_format_co_do_centa() {
    let t = dane(
        "conduit_test_drab_parytet",
        3 * 24 * 60,
        &[(0, 4000.0), (100, 4012.0)],
    );
    let msgs = vec![
        sygnal(1, 10, "ZEN", 4010.0, 3800.0),
        sygnal(2, 300, "ZEN", 4200.0, 3800.0),
    ];

    let mut bez = RunConfig {
        from: T0,
        to: T0 + 3 * 86_400_000,
        start_balance: 1000.0,
        settings: ustawienia(),
        formaty: vec![noga("ZEN", "P")],
        ..Default::default()
    };
    let r1 = run(&t, &msgs, &bez);

    bez.formaty = Vec::new();
    bez.drabinka = vec![szczebel(0.0, "JEDYNY", vec![noga("ZEN", "P")])];
    let r2 = run(&t, &msgs, &bez);

    assert_eq!(
        r1.metrics.total_profit, r2.metrics.total_profit,
        "drabinka o jednym szczeblu musi dać CO DO CENTA wynik --preset-format"
    );
    assert_eq!(r1.metrics.trades, r2.metrics.trades);
    assert_eq!(r1.metrics.min_equity, r2.metrics.min_equity);
    assert!(
        r2.przelaczenia.is_empty(),
        "jeden szczebel = zero przełączeń"
    );
    assert_eq!(r2.szczeble.len(), 1);
    assert_eq!(r2.szczeble[0].wejscia, 1, "start liczy się jako wejście");
}

// ============================================================
//  2. ADOPCJA W GÓRĘ
// ============================================================

/// Koszyk otwarty na szczeblu 0 (TP daleko), drugi koszyk domyka się z zyskiem
/// i przełącza drabinkę w górę. Pierwszy koszyk MUSI domknąć się później pod
/// nowym silnikiem (zero sierot), transakcje nie mogą się podwoić, a koszyk
/// otwarty PO przełączeniu musi dostać numer wyższy niż adoptowane.
#[test]
fn przelaczenie_w_gore_adoptuje_otwarte_koszyki() {
    // cena: 4000 → (min 100) 4012 domyka TP=4010 sygnału 2
    //            → (min 200) 4032 domyka TP=4030 sygnałów 1 i 3
    let t = dane(
        "conduit_test_drab_gora",
        24 * 60,
        &[(0, 4000.0), (100, 4012.0), (150, 4000.0), (200, 4032.0)],
    );
    let msgs = vec![
        sygnal(1, 10, "ZEN", 4030.0, 3800.0), // zostaje otwarty przez przełączenie
        sygnal(2, 20, "ZEN", 4010.0, 3800.0), // domyka się i podnosi saldo
        sygnal(3, 160, "ZEN", 4030.0, 3800.0), // otwarty już POD szczeblem 1
    ];
    let cfg = RunConfig {
        from: T0,
        to: T0 + 86_400_000,
        start_balance: 1000.0,
        settings: ustawienia(),
        // próg 1000,01: KAŻDE dodatnie domknięcie przekracza go, niezależnie
        // od tego, jak silnik policzy lot — test nie zgaduje wolumenu
        drabinka: vec![
            szczebel(0.0, "DOL", vec![noga("ZEN", "P-DOL")]),
            szczebel(1000.01, "GORA", vec![noga("ZEN", "P-GORA")]),
        ],
        ..Default::default()
    };
    let r = run(&t, &msgs, &cfg);

    assert_eq!(r.przelaczenia.len(), 1, "jedno przejście w górę");
    assert_eq!(r.przelaczenia[0].na, "GORA");
    assert_eq!(
        r.metrics.trades, 3,
        "trzy koszyki = trzy domknięcia, bez podwójnych"
    );
    assert_eq!(
        r.metrics.baskets, 3,
        "koszyk sprzed przełączenia NIE ginie i NIE dubluje się"
    );

    // sierota = pozycja, której nikt nie domknął; przy tej ścieżce ceny
    // wszystko ma prawo się domknąć, więc na koniec equity == balance
    assert!(
        (r.metrics.end_equity - r.metrics.end_balance).abs() < 0.01,
        "po końcu przebiegu nie wisi żadna niedomknięta pozycja (sierota)"
    );

    // atrybucja: domknięcie sygnału 2 należy do DOL (aktywny przy zamknięciu),
    // sygnały 1 i 3 domykają się pod GORA
    assert_eq!(r.szczeble[0].trejdy, 1, "DOL rozliczył jedno domknięcie");
    assert_eq!(
        r.szczeble[1].trejdy, 2,
        "GORA rozliczyła dwa domknięcia (adoptowany + własny)"
    );
    let suma: f64 = r.szczeble.iter().map(|s| s.zysk).sum();
    assert!(
        (suma - r.metrics.total_profit).abs() < 0.01,
        "suma szczebli musi domykać się do wyniku całości ({suma} vs {})",
        r.metrics.total_profit
    );

    // numeracja: trzy RÓŻNE numery koszyków — adopcja podnosi licznik,
    // więc koszyk otwarty po przełączeniu nie wchodzi w numer adoptowanego
    let mut ids: Vec<u32> = r.baskets_dump.iter().map(|b| b.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(
        ids.len(),
        3,
        "numery koszyków nie mogą się powtarzać po adopcji"
    );
}

// ============================================================
//  3. ZEJŚCIE = ZARZĄDZAJ-NIE-OTWIERAJ
// ============================================================

/// Start na szczeblu 1 (ZEN+SYN). Strata ZEN strąca na szczebel 0 (sam ZEN).
/// Otwarty koszyk SYN musi zostać ZARZĄDZONY do naturalnego końca (TP),
/// a NOWY sygnał SYN po zejściu ma być zablokowany i policzony.
#[test]
fn zejscie_zarzadza_starym_koszykiem_i_blokuje_nowe_sygnaly() {
    // cena: 4000 → (min 60) 3994 wybija SL=3995 ZEN-a (BUY); SYN gra SELL,
    // więc zanurzenie w dół to jego zysk pływający, nie stop
    //            → (min 90) 4000 → (min 150) 3968 domyka TP=3970 SYN-a
    let t = dane(
        "conduit_test_drab_dol",
        24 * 60,
        &[(0, 4000.0), (60, 3994.0), (90, 4000.0), (150, 3968.0)],
    );
    let msgs = vec![
        sygnal_sell(1, 10, "SYN", 3970.0, 4200.0), // otwarty PRZED zejściem, domyka się PO
        sygnal(2, 20, "ZEN", 4200.0, 3995.0),      // strata strąca szczebel
        sygnal_sell(3, 100, "SYN", 3970.0, 4200.0), // po zejściu: BLOKADA
        // ZEN ma nogę, wchodzi normalnie; SELL, żeby w spadającym rynku
        // rozliczenie szczebla dolnego miało jednoznaczny znak (TP też jest
        // przycinany przez silnik, więc BUY straciłby więcej, niż SYN zarabia)
        sygnal_sell(4, 110, "ZEN", 3970.0, 4200.0),
    ];
    let cfg = RunConfig {
        from: T0,
        to: T0 + 86_400_000,
        start_balance: 1000.0,
        settings: ustawienia(),
        // próg 999,99: start (1000) siedzi na szczeblu 1, KAŻDA strata strąca
        drabinka: vec![
            szczebel(0.0, "SAM-ZEN", vec![noga("ZEN", "P-ZEN")]),
            szczebel(
                999.99,
                "OBA",
                vec![noga("ZEN", "P-ZEN"), noga("SYN", "P-SYN")],
            ),
        ],
        ..Default::default()
    };
    let r = run(&t, &msgs, &cfg);

    // Zejście po stracie ORAZ powrót w górę, gdy zyski odrobią saldo — drugi
    // ruch jest tak samo częścią kontraktu jak pierwszy (histereza 0).
    assert_eq!(
        r.przelaczenia.len(),
        2,
        "zejście po stracie + powrót po odrobieniu"
    );
    assert_eq!(r.przelaczenia[0].na, "SAM-ZEN");
    assert_eq!(
        r.przelaczenia[1].na, "OBA",
        "zyski odrabiają saldo nad próg — drabinka wraca"
    );

    let syn = r
        .formaty
        .iter()
        .find(|f| f.format == "SYN")
        .expect("silnik SYN istnieje dalej");
    assert_eq!(
        syn.koszyki, 1,
        "NOWY sygnał SYN po zejściu nie ma prawa otworzyć koszyka"
    );
    assert_eq!(
        syn.trejdy, 1,
        "stary koszyk SYN został ZARZĄDZONY do TP — zero sierot"
    );
    assert!(syn.zysk > 0.0, "SELL z 4000 domknięty na TP 3970 to zysk");
    // domknięcie SYN-a nastąpiło PO zejściu, więc rozlicza je szczebel dolny —
    // to jest wprost kontrakt „zysk należy do tego, kto zarządzał przy zamknięciu"
    assert!(
        r.szczeble[0].zysk > 0.0,
        "zysk adoptowanego koszyka SYN księguje szczebel aktywny przy domknięciu"
    );

    assert_eq!(
        r.metrics
            .odrzuty
            .get("SzczebelBezNogi:SYN")
            .copied()
            .unwrap_or(0),
        1,
        "zablokowany sygnał ma być POLICZONY, nie przemilczany"
    );
    // ZEN handluje dalej: strata + wejście nr 4 (zostaje otwarte do końca)
    let zen = r.formaty.iter().find(|f| f.format == "ZEN").unwrap();
    assert_eq!(
        zen.koszyki, 2,
        "ZEN ma nogę na obu szczeblach — sygnał 4 wchodzi normalnie"
    );
}

// ============================================================
//  4. HISTEREZA
// ============================================================

/// Przy histerezie 50 % strata, która zeszłaby ze szczebla przy h=0,
/// NIE strąca drabinki: saldo musiałoby spaść pod połowę progu.
#[test]
fn histereza_trzyma_szczebel_przy_malym_cofnieciu() {
    let t = dane(
        "conduit_test_drab_hist",
        24 * 60,
        &[(0, 4000.0), (60, 3994.0)],
    );
    let msgs = vec![sygnal(1, 20, "ZEN", 4200.0, 3995.0)];
    let mut cfg = RunConfig {
        from: T0,
        to: T0 + 86_400_000,
        start_balance: 1000.0,
        settings: ustawienia(),
        drabinka: vec![
            szczebel(0.0, "DOL", vec![noga("ZEN", "P")]),
            szczebel(999.99, "GORA", vec![noga("ZEN", "P")]),
        ],
        drabinka_histereza_pct: 50.0,
        ..Default::default()
    };
    let r = run(&t, &msgs, &cfg);
    assert!(
        r.przelaczenia.is_empty(),
        "histereza 50 %: zejście dopiero pod {:.2}, mała strata nie strąca",
        999.99 * 0.5
    );

    cfg.drabinka_histereza_pct = 0.0;
    let r0 = run(&t, &msgs, &cfg);
    assert_eq!(
        r0.przelaczenia.len(),
        1,
        "bez histerezy ta sama strata MUSI zejść"
    );
}

// ============================================================
//  5. RESET DOBOWY WRACA NA SZCZEBEL BAZY
// ============================================================

#[test]
fn reset_dobowy_wraca_na_szczebel_kwoty_startowej() {
    let t = dane(
        "conduit_test_drab_dzienny",
        2 * 24 * 60,
        &[(0, 4000.0), (100, 4012.0), (150, 4000.0)],
    );
    let msgs = vec![sygnal(1, 20, "ZEN", 4010.0, 3800.0)];
    let cfg = RunConfig {
        from: T0,
        to: T0 + 2 * 86_400_000,
        start_balance: 1000.0,
        settings: ustawienia(),
        daily_reset: true,
        drabinka: vec![
            szczebel(0.0, "DOL", vec![noga("ZEN", "P")]),
            szczebel(1000.01, "GORA", vec![noga("ZEN", "P")]),
        ],
        ..Default::default()
    };
    let r = run(&t, &msgs, &cfg);

    assert_eq!(
        r.przelaczenia.len(),
        2,
        "w górę wewnątrz dnia 1, w dół o północy"
    );
    assert_eq!(r.przelaczenia[0].na, "GORA");
    assert_eq!(r.przelaczenia[1].na, "DOL");
    assert_eq!(
        r.przelaczenia[1].balance, 1000.0,
        "zejście o północy wynika z powrotu do kwoty startowej, nie ze straty"
    );
    assert_eq!(r.szczeble[0].wejscia, 2, "DOL: start + powrót o północy");
    assert_eq!(r.szczeble[1].wejscia, 1);
}

#[test]
fn szczebel_wolno_podac_jako_preset_a_nazwa_lancucha_znaczy_to_samo_co_dotad() {
    use conduit_backtest::runner::zbuduj_drabinke;

    let dir = std::env::temp_dir().join("conduit_test_drab_preset");
    std::fs::create_dir_all(&dir).unwrap();
    let mut p = conduit_core::settings::Preset {
        name: "SZCZEBEL-A".into(),
        description: String::new(),
        format: "Synergy".into(),
        settings: Settings::default(),
        ea: None,
    };
    p.settings.max_open_baskets = 7;
    std::fs::write(
        dir.join("SZCZEBEL-A.json"),
        serde_json::to_string(&p).unwrap(),
    )
    .unwrap();
    let luzny = dir.join("gdzie_indziej.json");
    p.name = "SZCZEBEL-B".into();
    p.settings.max_open_baskets = 12;
    std::fs::write(&luzny, serde_json::to_string(&p).unwrap()).unwrap();

    let spec = format!("0=preset:SZCZEBEL-A,1000=plik:{}", luzny.display());
    let d = zbuduj_drabinke(&spec, &dir).unwrap();
    assert_eq!(d.len(), 2);
    assert_eq!(d[0].nazwa, "SZCZEBEL-A");
    assert_eq!(d[0].formaty.len(), 1, "goły preset = DOKŁADNIE jedna noga");
    assert_eq!(
        d[0].formaty[0].format, "Synergy",
        "format nogi z pola `format` presetu"
    );
    assert_eq!(d[0].formaty[0].settings.max_open_baskets, 7);
    assert_eq!(
        d[0].pulapy,
        PulapyGlobalne::default(),
        "goły preset nie ma nad sobą łańcucha, więc pułap globalny NIE MOŻE nic przyciąć"
    );
    assert_eq!(d[1].prog, 1000.0);
    assert_eq!(d[1].nazwa, "SZCZEBEL-B");
    assert_eq!(d[1].formaty[0].settings.max_open_baskets, 12);

    // …i postać pierwotna nietknięta: gołe nazwy to nadal łańcuchy wbudowane
    // z ICH pułapami, a nie pliki o tej nazwie.
    let stara = zbuduj_drabinke(
        "0=ZENONLY5,1000=SENTINEL-0",
        std::path::Path::new("../PACKAGE/presets"),
    );
    match stara {
        Ok(d) => {
            assert_eq!(d[1].nazwa, "SENTINEL-0");
            assert_eq!(d[1].pulapy.max_koszykow, 4, "łańcuch wnosi SWÓJ pułap");
        }
        // Presety nóg leżą poza repo testowym — brak pliku to poprawna,
        // NIEZMIENIONA ścieżka błędu, byle nie „łańcuch nie istnieje”.
        Err(e) => assert!(
            !e.to_string().contains("nie istnieje wśród wbudowanych"),
            "goła nazwa przestała trafiać w łańcuch wbudowany: {e}"
        ),
    }
}
