//! Testy RDZENIA DECYZYJNEGO.
//!
//! Najważniejszy z nich (`kanon_odtwarza_komunikat_risk_free`) przepisuje
//! prawdziwy komunikat kanału na asercję. Dopóki przechodzi, „nasz kanon" i
//! „kanon kanału" to jedno i to samo, a nie dwie opowieści o tym samym.

use conduit_mozg::oczy::Oczy;
use conduit_mozg::polityka::{Domeny, Polityka, PolitykaKanon, PolitykaZero, Powod, Zamiar};
use conduit_mozg::rama::{GeometriaPomyslu, Strona};
use conduit_mozg::wejscie::{Koszyk, Rachunek, Szczebel, Wejscie};

fn rachunek() -> Rachunek {
    Rachunek {
        saldo: 1000.0,
        equity: 1000.0,
        margines_uzyty: 0.0,
        margines_wolny: 1000.0,
        poziom_marginesu: None,
        dzwignia: 500.0,
        wynik_dnia_usd: 0.0,
        seria_stopow: 0,
    }
}

fn geometria(sl: f64, cele: Vec<f64>) -> GeometriaPomyslu {
    GeometriaPomyslu {
        strona: Strona::Buy,
        krawedz_blizsza: 4004.0,
        krawedz_dalsza: 4000.0,
        sl: Some(sl),
        cele,
    }
}

/// Szczebel BUY o zadanej cenie wejścia; wynik liczony z ceny bieżącej.
/// 0,01 lota XAUUSD to JEDNA uncja (kontrakt 100), więc ruch o 1 $ na uncji
/// daje 1 $ wyniku — ta arytmetyka była raz pomylona sto razy w drugą stronę,
/// więc jest tu wypisana wprost.
fn szczebel(ticket: u64, wejscie: f64, cena: f64, glebokosc: u16, sl: Option<f64>) -> Szczebel {
    let wynik = (cena - wejscie) * 100.0 * 0.01;
    Szczebel {
        ticket,
        strona: Strona::Buy,
        cena_wejscia: wejscie,
        wolumen: 0.01,
        sl,
        tp: None,
        glebokosc,
        ts_otwarcia: 0,
        wynik_usd: wynik,
        szczyt_usd: wynik.max(0.0),
        dno_usd: 0.0,
    }
}

/// W pełni syntetyczny koszyk RISK FREE: wejścia 4000–4004, cena 4009.
/// Głębokość rośnie w dół ceny: 4004 jest najpłytszy, a 4000 najgłębszy.
fn koszyk_risk_free(cena: f64) -> Koszyk {
    Koszyk {
        id: 1,
        rama_id: 1,
        geometria: geometria(3995.0, vec![4009.0, 4014.0, 4019.0]),
        ts_zawiazania: 0,
        szczeble: vec![
            szczebel(104, 4004.0, cena, 0, Some(3995.0)),
            szczebel(103, 4003.0, cena, 1, Some(3995.0)),
            szczebel(102, 4002.0, cena, 2, Some(3995.0)),
            szczebel(101, 4001.0, cena, 3, Some(3995.0)),
            szczebel(100, 4000.0, cena, 4, Some(3995.0)),
        ],
        oczekujacych: 0,
        etap_celu: 1,
        rf_ogloszony: true,
        budzet_wydany_usd: 0.0,
    }
}

fn domeny_otwarte() -> Domeny {
    Domeny {
        inkaso: Some((0.0, 1.0)),
        stop_tylko_ciasniej: true,
        zamykanie: true,
        prog_modyfikacji_usd: 0.0,
        sufit_interwencji: 24,
    }
}

// ---------------------------------------------------------------------------

/// **KONTRAKT ZERA JAKO TYP.** Polityka zerowa nie potrafi wyprodukować
/// niczego poza „trzymaj" — niezależnie od tego, co widzi.
#[test]
fn polityka_zero_nigdy_nic_nie_robi() {
    let oczy = Oczy::nowe();
    let ks = vec![koszyk_risk_free(4009.0)];
    let we = Wejscie {
        ts: 0,
        oczy: &oczy,
        rachunek: rachunek(),
        koszyki: &ks,
        koszyk: Some(0),
    };
    for dom in [Domeny::cien(), domeny_otwarte()] {
        let z = PolitykaZero.decyduj(&we, &dom);
        assert_eq!(
            z,
            vec![Zamiar::Trzymaj],
            "polityka zerowa wyprodukowała działanie"
        );
    }
}

/// **KANON KANAŁU, CO DO SZCZEBLA.** Polityka ma zamknąć cztery płytsze
/// wejścia i zabezpieczyć najlepsze — dokładnie jak komunikat.
#[test]
fn kanon_odtwarza_komunikat_risk_free() {
    let oczy = Oczy::nowe();
    let ks = vec![koszyk_risk_free(4009.0)];
    let we = Wejscie {
        ts: 0,
        oczy: &oczy,
        rachunek: rachunek(),
        koszyki: &ks,
        koszyk: Some(0),
    };
    let z = PolitykaKanon::default().decyduj(&we, &domeny_otwarte());

    let zamkniete: Vec<u64> = z
        .iter()
        .filter_map(|x| match x {
            Zamiar::Zamknij {
                ticket,
                powod: Powod::InkasoRiskFree,
            } => Some(*ticket),
            _ => None,
        })
        .collect();
    assert_eq!(
        zamkniete,
        vec![104, 103, 102, 101],
        "polityka zamyka cztery płytsze szczeble i zostawia najgłębszy"
    );

    let stopy: Vec<(u64, f64)> = z
        .iter()
        .filter_map(|x| match x {
            Zamiar::PrzesunStop {
                ticket,
                na,
                powod: Powod::ZabezpieczOcalalego,
            } => Some((*ticket, *na)),
            _ => None,
        })
        .collect();
    assert_eq!(stopy.len(), 1, "dokładnie jeden stop do przesunięcia");
    assert_eq!(stopy[0].0, 100, "chroniony ma być najlepszy szczebel");
    assert!(
        (stopy[0].1 - 4000.0).abs() < 1e-9,
        "stop ma stanąć na syntetycznym wejściu 4000"
    );
}

/// Koszyk pod wodą NIE jest inkasowany. Zamknięcie płytkich nóg w stracie
/// zostawiłoby samą stratną głębię — a kanał ogłasza RISK FREE dopiero wtedy,
/// gdy pomysł już zarabia.
#[test]
fn kanon_nie_inkasuje_kiedy_koszyk_pod_woda() {
    let oczy = Oczy::nowe();
    let ks = vec![koszyk_risk_free(3996.0)]; // cena poniżej wszystkich wejść
    let we = Wejscie {
        ts: 0,
        oczy: &oczy,
        rachunek: rachunek(),
        koszyki: &ks,
        koszyk: Some(0),
    };
    let z = PolitykaKanon::default().decyduj(&we, &domeny_otwarte());
    assert_eq!(z, vec![Zamiar::Trzymaj], "inkaso na stratnym koszyku");
}

/// **DOMENA ZAMKNIĘTA ZNACZY BEZCZYNNOŚĆ.** To jest bezpiecznik: nowa
/// zdolność wymaga JAWNEGO otwarcia domeny, a nie tylko napisania reguły.
#[test]
fn zamknieta_domena_blokuje_kazde_dzialanie() {
    let oczy = Oczy::nowe();
    let ks = vec![koszyk_risk_free(4009.0)];
    let we = Wejscie {
        ts: 0,
        oczy: &oczy,
        rachunek: rachunek(),
        koszyki: &ks,
        koszyk: Some(0),
    };
    let z = PolitykaKanon::default().decyduj(&we, &Domeny::cien());
    assert_eq!(
        z,
        vec![Zamiar::Trzymaj],
        "przy zamkniętej domenie mózg działał"
    );
}

#[test]
fn stop_nie_jest_ruszany_ponizej_progu_brokera() {
    let oczy = Oczy::nowe();
    let ks = vec![koszyk_risk_free(4009.0)];
    let we = Wejscie {
        ts: 0,
        oczy: &oczy,
        rachunek: rachunek(),
        koszyki: &ks,
        koszyk: Some(0),
    };
    let mut dom = domeny_otwarte();
    dom.prog_modyfikacji_usd = 100.0; // próg wyższy niż jakikolwiek zysk nogi
    let z = PolitykaKanon::default().decyduj(&we, &dom);
    assert!(
        !z.iter().any(|x| matches!(x, Zamiar::PrzesunStop { .. })),
        "polityka wyprodukowała stop, którego broker by odrzucił"
    );
    // Inkaso ma się odbyć mimo to — to inny aktuator i inny próg.
    assert!(z.iter().any(|x| matches!(x, Zamiar::Zamknij { .. })));
}

/// **DETERMINIZM.** To samo wejście, ten sam wynik — bit w bit.
#[test]
fn ta_sama_chwila_daje_te_same_zamiary() {
    let oczy = Oczy::nowe();
    let ks = vec![koszyk_risk_free(4009.0)];
    let we = Wejscie {
        ts: 0,
        oczy: &oczy,
        rachunek: rachunek(),
        koszyki: &ks,
        koszyk: Some(0),
    };
    let p = PolitykaKanon::default();
    let a = p.decyduj(&we, &domeny_otwarte());
    let b = p.decyduj(&we, &domeny_otwarte());
    assert_eq!(a, b);
}

#[test]
fn zabezpieczony_szczebel_nie_jest_ruszany_ponownie() {
    let oczy = Oczy::nowe();
    let mut k = koszyk_risk_free(4009.0);
    // najlepszy syntetyczny szczebel ma już stop na wejściu
    if let Some(p) = k.szczeble.iter_mut().find(|s| s.ticket == 100) {
        p.sl = Some(4000.0);
    }
    let ks = vec![k];
    let we = Wejscie {
        ts: 0,
        oczy: &oczy,
        rachunek: rachunek(),
        koszyki: &ks,
        koszyk: Some(0),
    };
    let z = PolitykaKanon::default().decyduj(&we, &domeny_otwarte());
    assert!(
        !z.iter().any(|x| matches!(x, Zamiar::PrzesunStop { .. })),
        "stop przesunięty po raz drugi"
    );
}
