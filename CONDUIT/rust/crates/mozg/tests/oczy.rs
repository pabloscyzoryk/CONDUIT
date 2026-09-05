
use conduit_mozg::oczy::{Oczy, Okres, PAMIEC};

/// Strumień tików o stałym kroku, cena piłokształtna — deterministyczny
/// i bez żadnej zależności od zegara systemowego.
fn strumien(od_ms: i64, krok_ms: i64, ile: usize) -> Vec<(i64, f64, f64)> {
    (0..ile)
        .map(|i| {
            let ts = od_ms + krok_ms * i as i64;
            // trójkąt o okresie 97 tików — liczba pierwsza, żeby wzór nie
            // zgrał się przypadkiem z granicą żadnej świecy
            let f = (i % 97) as f64;
            let mid = 4600.0 + if f < 48.5 { f } else { 97.0 - f } * 0.1;
            (ts, mid - 0.11, mid + 0.11)
        })
        .collect()
}

fn nakarm(o: &mut Oczy, s: &[(i64, f64, f64)]) {
    for (ts, b, a) in s {
        o.tik(*ts, *b, *a);
    }
}

// ---------------------------------------------------------------------------

/// **ZERO PRZYSZŁOŚCI.** Świeca bieżąca nigdy nie jest podpisana jako
/// domknięta, a lista domkniętych nigdy jej nie zawiera.
///
/// To jest ta jedna własność, dla której cały moduł rozróżnia `biezaca`
/// i `hist`. Gdyby świeca w trakcie trafiała do historii, polityka czytałaby
/// jej zamknięcie — czyli cenę, której w chwili decyzji jeszcze nie ma.
#[test]
fn swieca_biezaca_nigdy_nie_jest_domknieta() {
    let mut o = Oczy::nowe();
    let s = strumien(1_700_000_000_000, 137, 5_000);
    for (i, (ts, b, a)) in s.iter().enumerate() {
        o.tik(*ts, *b, *a);
        for okno in o.okna.iter() {
            if let Some(bz) = &okno.biezaca {
                assert!(
                    !bz.domknieta,
                    "tik {i}: świeca bieżąca {} podpisana jako domknięta",
                    okno.okres.nazwa()
                );
            }
            for (j, d) in okno.domkniete().iter().enumerate() {
                assert!(
                    d.domknieta,
                    "tik {i}: {} hist[{j}] w historii, ale NIE domknięta",
                    okno.okres.nazwa()
                );
                if let Some(bz) = &okno.biezaca {
                    assert!(
                        d.start_ms < bz.start_ms,
                        "tik {i}: {} hist[{j}] nie jest STARSZA od bieżącej",
                        okno.okres.nazwa()
                    );
                }
            }
        }
    }
}

/// **DETERMINIZM.** Ta sama sekwencja tików daje bit w bit ten sam obraz.
///
/// Bez tego backtest i Tester MT5 liczą dwie różne rzeczy, a bramka parytetu
/// przestaje cokolwiek znaczyć.
#[test]
fn ten_sam_strumien_daje_ten_sam_obraz() {
    let s = strumien(1_700_000_000_000, 211, 3_000);
    let (mut a, mut b) = (Oczy::nowe(), Oczy::nowe());
    nakarm(&mut a, &s);
    nakarm(&mut b, &s);
    assert_eq!(a.tikow, b.tikow);
    assert_eq!(a.max_przerwa_ms, b.max_przerwa_ms);
    for (x, y) in a.okna.iter().zip(b.okna.iter()) {
        assert_eq!(
            x.domkniete().len(),
            y.domkniete().len(),
            "{}",
            x.okres.nazwa()
        );
        for (p, q) in x.domkniete().iter().zip(y.domkniete().iter()) {
            assert_eq!(p.start_ms, q.start_ms);
            assert!((p.o - q.o).abs() < f64::EPSILON);
            assert!((p.h - q.h).abs() < f64::EPSILON);
            assert!((p.l - q.l).abs() < f64::EPSILON);
            assert!((p.c - q.c).abs() < f64::EPSILON);
            assert_eq!(p.tikow, q.tikow);
        }
    }
}

/// **KOTWICA NIEZALEŻNA OD STARTU.** Granice świec siedzą na wielokrotnościach
/// długości okresu, więc dwa boty uruchomione o różnych porach widzą TE SAME
/// świece. Gdyby kotwicą był pierwszy tik, każdy restart przesuwałby cały
/// obraz — a wtedy „ta sama świeca 1h" znaczyłaby co innego po każdym
/// wznowieniu.
#[test]
fn granice_swiec_sa_wielokrotnoscia_okresu() {
    for start in [0_i64, 1, 59_999, 1_700_000_123_456] {
        let mut o = Oczy::nowe();
        nakarm(&mut o, &strumien(start, 997, 4_000));
        for okno in o.okna.iter() {
            let dl = okno.okres.ms();
            for d in okno.domkniete() {
                assert_eq!(
                    d.start_ms.rem_euclid(dl),
                    0,
                    "{} start {} nie leży na wielokrotności {}",
                    okno.okres.nazwa(),
                    d.start_ms,
                    dl
                );
            }
            if let Some(bz) = &okno.biezaca {
                assert_eq!(
                    bz.start_ms.rem_euclid(dl),
                    0,
                    "{} bieżąca",
                    okno.okres.nazwa()
                );
            }
        }
    }
}

/// Pierścień pamięta najwyżej [`PAMIEC`] świec i trzyma je od NAJŚWIEŻSZEJ.
///
/// Kolejność jest częścią kontraktu: polityka pytająca o `domkniete()[0]`
/// ma dostać ostatnią zamkniętą świecę, a nie najstarszą pamiętaną.
#[test]
fn historia_jest_od_najswiezszej_i_ograniczona() {
    let mut o = Oczy::nowe();
    // 1200 minut = 1200 świec 1m, czyli znacznie więcej niż PAMIEC
    nakarm(&mut o, &strumien(1_700_000_000_000, 1_000, 72_000));
    let m1 = o.okno(Okres::M1);
    assert_eq!(
        m1.domkniete().len(),
        PAMIEC,
        "pierścień ma trzymać dokładnie PAMIEC"
    );
    let h = m1.domkniete();
    for i in 1..h.len() {
        assert!(
            h[i - 1].start_ms > h[i].start_ms,
            "hist[{}] nie jest świeższa od hist[{}]",
            i - 1,
            i
        );
    }
}

#[test]
fn przerwa_w_strumieniu_jest_widoczna() {
    let mut o = Oczy::nowe();
    nakarm(&mut o, &strumien(1_700_000_000_000, 100, 500));
    // Ostatni tik strumienia leży na "od + krok*(ile-1)", nie "od + krok*ile"
    // — stąd jawne wyliczenie zamiast liczby wpisanej z pamięci.
    let ostatni = 1_700_000_000_000 + 100 * (500 - 1);
    let cisza = 53_000_000; // ~14,7 h
    o.tik(ostatni + cisza, 4600.0, 4600.2);
    assert!(o.max_przerwa_ms >= cisza, "przerwa nie została zauważona");
    assert_eq!(o.odstep_ms(), cisza, "odstęp ostatniego tiku");
}

#[test]
fn brak_pomiaru_to_nan_a_nie_zero() {
    let o = Oczy::nowe();
    let m1 = o.okno(Okres::M1);
    assert!(m1.sredni_zakres(5).is_nan());
    assert!(m1.polozenie_w_zakresie(5, 4600.0).is_nan());
    assert!(m1.zmiana(5).is_nan());
    assert!(o.mid().is_nan());
}
