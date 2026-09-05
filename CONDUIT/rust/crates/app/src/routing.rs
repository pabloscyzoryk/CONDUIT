
pub use conduit_core::routing::*;

#[cfg(test)]
mod testy {
    use super::*;
    use conduit_backtest::sim::SimBroker;
    use conduit_core::broker::{Broker, OrderReq};
    use conduit_core::engine::Engine;
    use conduit_core::formaty::{Lancuch, PulapyGlobalne};
    use conduit_core::types::{CloseReason, Quote, Side, SourceKey};
    use conduit_core::wielosilnik;
    use conduit_core::Settings;
    use std::collections::BTreeMap;

    fn broker() -> SimBroker {
        let mut b = SimBroker::new(10_000.0, 0.2, 0.0);
        b.on_quote(Quote {
            ts: 1_700_000_000_000,
            bid: 4000.0,
            ask: 4000.3,
        });
        b
    }

    fn lancuch_dwa(pulapy: PulapyGlobalne) -> Lancuch {
        let mut l = Lancuch {
            nazwa: "TEST".into(),
            ..Default::default()
        };
        l.presety.insert("ATFX".into(), "P-ATFX".into());
        l.presety.insert("Synergy".into(), "P-SYN".into());
        l.pulapy = pulapy;
        l
    }

    fn presety(max_poz: u32, max_kosz: u32) -> BTreeMap<String, Settings> {
        let mut s = Settings::default();
        s.max_open_positions = max_poz;
        s.max_open_baskets = max_kosz;
        s.session_filter = false;
        let mut m = BTreeMap::new();
        m.insert("P-ATFX".to_string(), s.clone());
        m.insert("P-SYN".to_string(), s);
        m
    }

    fn zbuduj(pulapy: PulapyGlobalne, poz: u32, kosz: u32) -> Silniki {
        Silniki::zbuduj(
            &lancuch_dwa(pulapy),
            &presety(poz, kosz),
            &Settings::default(),
            1000.0,
        )
        .0
    }

    fn otworz<B: Broker>(b: &mut B, basket: u32) {
        let _ = b.open_market(OrderReq {
            side: Side::Buy,
            volume: 0.01,
            sl: None,
            tp: None,
            basket: Some(basket),
            level: 0,
            is_toucher: false,
            comment: String::new(),
        });
    }

    fn reczny<B: Broker>(b: &mut B) {
        let _ = b.open_market(OrderReq {
            side: Side::Buy,
            volume: 0.01,
            sl: None,
            tp: None,
            basket: None,
            level: 0,
            is_toucher: false,
            comment: "panel".into(),
        });
    }

    // ============================================================
    //  SLOTY I NUMERACJA
    // ============================================================

    #[test]
    fn dwa_formaty_dostaja_rozlaczne_numery_koszykow() {
        let s = zbuduj(PulapyGlobalne::default(), 9, 3);
        assert_eq!(s.lista.len(), 2);
        assert_ne!(
            s.lista[0].slot, s.lista[1].slot,
            "dwa formaty nie moga dzielic slotu"
        );
        assert_ne!(
            s.lista[0].engine.next_basket_id(),
            s.lista[1].engine.next_basket_id()
        );
        for x in &s.lista {
            assert_eq!(
                wielosilnik::slot_koszyka(x.engine.next_basket_id()),
                x.slot,
                "pierwszy numer koszyka musi juz niesc slot"
            );
        }
    }

    #[test]
    fn zapasowy_jest_dokladnie_jeden_i_to_atfx() {
        let s = zbuduj(PulapyGlobalne::default(), 9, 3);
        let zapasowe: Vec<&str> = s
            .lista
            .iter()
            .filter(|x| x.zapasowy)
            .map(|x| x.format.as_str())
            .collect();
        assert_eq!(
            zapasowe,
            vec!["ATFX"],
            "reczny bilet ma miec DOKLADNIE JEDNEGO opiekuna"
        );
    }

    #[test]
    fn stary_numer_koszyka_trafia_do_zapasowego() {
        let s = zbuduj(PulapyGlobalne::default(), 9, 3);
        let i = s.indeks_koszyka(1).expect("koszyk B1 musi miec opiekuna");
        assert!(s.lista[i].zapasowy);
        assert_eq!(s.lista[i].format, "ATFX");
    }

    // ============================================================
    //  WIDOK BROKERA
    // ============================================================

    /// SEDNO ROZDZIELENIA: silnik nie widzi cudzych pozycji, a po wyjsciu
    /// z widoku wszystkie wracaja na miejsce.
    #[test]
    fn silnik_widzi_wylacznie_swoje_pozycje() {
        let mut s = zbuduj(PulapyGlobalne::default(), 9, 3);
        let mut b = broker();
        let (s0, s1) = (s.lista[0].slot, s.lista[1].slot);
        otworz(&mut b, wielosilnik::baza_slotu(s0) + 1);
        otworz(&mut b, wielosilnik::baza_slotu(s1) + 1);
        otworz(&mut b, wielosilnik::baza_slotu(s1) + 2);
        assert_eq!(b.positions().len(), 3);

        let widziane0 = s.z_widokiem(0, &mut b, |_e, w| w.positions().len());
        let widziane1 = s.z_widokiem(1, &mut b, |_e, w| w.positions().len());
        assert_eq!(widziane0, 1, "pierwszy format widzi tylko swoja pozycje");
        assert_eq!(widziane1, 2);
        assert_eq!(
            b.positions().len(),
            3,
            "po wyjsciu z widoku wracaja WSZYSTKIE"
        );
    }

    /// Straznik jednego formatu nie ma prawa zamknac pozycji drugiego.
    /// To jest ta awaria, dla ktorej widok w ogole powstal: `close_everything`
    /// zamyka wszystko, co widzi.
    #[test]
    fn zamkniecie_wszystkiego_dotyka_tylko_wlasnego_formatu() {
        let mut s = zbuduj(PulapyGlobalne::default(), 9, 3);
        let mut b = broker();
        let (s0, s1) = (s.lista[0].slot, s.lista[1].slot);
        otworz(&mut b, wielosilnik::baza_slotu(s0) + 1);
        otworz(&mut b, wielosilnik::baza_slotu(s1) + 1);

        s.z_widokiem(0, &mut b, |e, w| {
            e.close_everything(w, 1_700_000_001_000, CloseReason::MaxDd)
        });
        let zostale: Vec<u32> = b.positions().iter().filter_map(|p| p.basket).collect();
        assert_eq!(
            zostale,
            vec![wielosilnik::baza_slotu(s1) + 1],
            "cudza pozycja musi przezyc"
        );
    }

    /// Reczny bilet z panelu (`basket: None`) widzi WYLACZNIE silnik zapasowy.
    /// Gdyby widzieli go obaj, zamkneliby go dwa razy.
    #[test]
    fn reczny_bilet_widzi_tylko_zapasowy() {
        let mut s = zbuduj(PulapyGlobalne::default(), 9, 3);
        let mut b = broker();
        reczny(&mut b);
        let mut widzialo = 0;
        for i in 0..s.lista.len() {
            if s.z_widokiem(i, &mut b, |_e, w| w.positions().len()) > 0 {
                widzialo += 1;
            }
        }
        assert_eq!(widzialo, 1, "reczny bilet ma DOKLADNIE jednego opiekuna");
    }

    // ============================================================
    //  PULAPY GLOBALNE
    // ============================================================

    #[test]
    fn dwa_presety_po_9_nie_przekrocza_pulapu_14() {
        let mut s = zbuduj(
            PulapyGlobalne {
                max_pozycji: 14,
                ..Default::default()
            },
            9,
            3,
        );
        let mut b = broker();
        let (s0, s1) = (s.lista[0].slot, s.lista[1].slot);
        for k in 0..9 {
            otworz(&mut b, wielosilnik::baza_slotu(s0) + 1 + k);
        }
        for k in 0..5 {
            otworz(&mut b, wielosilnik::baza_slotu(s1) + 1 + k);
        }
        assert_eq!(b.positions().len(), 14);

        s.przelicz_obce(&b, None);
        let wolno = s.z_widokiem(1, &mut b, |e, w| {
            e.entry_gate(w, 1_700_000_001_000).blocked().is_none()
        });
        assert!(!wolno, "pietnasta pozycja na rachunku nie ma prawa powstac");

        // ...a bez pulapu ten sam stan przepuszcza - czyli blokada bierze sie
        // z warstwy globalnej, a nie z limitu presetu.
        let mut bez = zbuduj(PulapyGlobalne::default(), 9, 3);
        bez.przelicz_obce(&b, None);
        let wolno2 = bez.z_widokiem(1, &mut b, |e, w| {
            e.entry_gate(w, 1_700_000_001_000).blocked().is_none()
        });
        assert!(
            wolno2,
            "bez pulapu drugi format ma jeszcze 4 miejsca wlasnego limitu"
        );
    }

    /// Pulap koszykow liczy koszyki WSZYSTKICH formatow razem.
    ///
    /// Silnik ma ZERO wlasnych koszykow, a mimo to bramka musi go zatrzymac,
    /// bo pulap lancucha jest wyczerpany przez drugi format. Bez tego dwa
    /// presety po 3 koszyki daly by 6 na rachunku, na ktorym margines jest
    /// jeden. `obce` ustawiamy wprost - dokladnie to samo robi
    /// `przelicz_obce` raz na obrot petli.
    #[test]
    fn pulap_koszykow_liczy_wszystkie_formaty() {
        let mut s = zbuduj(
            PulapyGlobalne {
                max_koszykow: 4,
                ..Default::default()
            },
            0,
            3,
        );
        let mut b = broker();
        s.lista[0].engine.obce.koszyki = 4;
        let wolno = s.z_widokiem(0, &mut b, |e, w| {
            e.entry_gate(w, 1_700_000_001_000).blocked().is_none()
        });
        assert!(
            !wolno,
            "czwarty koszyk na rachunku wyczerpuje pulap lancucha"
        );

        // Ten sam stan BEZ pulapu przechodzi: limit presetu liczy wylacznie
        // koszyki wlasne, a tych jest zero.
        let mut bez = zbuduj(PulapyGlobalne::default(), 0, 3);
        bez.lista[0].engine.obce.koszyki = 4;
        let wolno2 = bez.z_widokiem(0, &mut b, |e, w| {
            e.entry_gate(w, 1_700_000_001_000).blocked().is_none()
        });
        assert!(wolno2, "limit presetu nie ma prawa liczyc cudzych koszykow");
    }

    /// `przelicz_obce` nie moze policzyc silnika jako obcego dla samego siebie.
    #[test]
    fn nikt_nie_liczy_siebie_jako_obcego() {
        let mut s = zbuduj(PulapyGlobalne::default(), 9, 3);
        let mut b = broker();
        let s0 = s.lista[0].slot;
        otworz(&mut b, wielosilnik::baza_slotu(s0) + 1);
        s.przelicz_obce(&b, None);
        assert_eq!(
            s.lista[0].engine.obce.loty_buy, 0.0,
            "wlasny lot nie jest obcy"
        );
        assert!(
            s.lista[1].engine.obce.loty_buy > 0.0,
            "cudzy lot musi byc widoczny"
        );
        assert_eq!(s.lista[0].engine.obce.koszyki, 0);
    }

    /// Blokada przeciwnych kierunkow patrzy na CUDZE pozycje, nie na wlasne.
    #[test]
    fn przeciwny_kierunek_liczy_sie_tylko_miedzy_formatami() {
        let mut s = zbuduj(PulapyGlobalne::default(), 9, 3);
        let mut b = broker();
        let s0 = s.lista[0].slot;
        otworz(&mut b, wielosilnik::baza_slotu(s0) + 1);

        s.przelicz_obce(&b, Some(Side::Sell));
        assert!(
            s.lista[1].engine.obce.przeciwny_kierunek,
            "drugi format ma zobaczyc cudzy BUY jako konflikt dla swojego SELL"
        );
        assert!(
            !s.lista[0].engine.obce.przeciwny_kierunek,
            "wlasna pozycja NIE jest konfliktem miedzy formatami"
        );
    }

    // ============================================================
    //  ROUTING WIADOMOSCI
    // ============================================================

    #[test]
    fn kanal_bez_formatu_daje_jawny_powod() {
        let s = zbuduj(PulapyGlobalne::default(), 9, 3);
        let src = SourceKey::new(-100, None);
        let e = s.trasa(&src, None).unwrap_err();
        assert_eq!(e.kod(), "KanalBezFormatu");
        assert!(
            !e.opis().is_empty(),
            "powod musi dac sie przeczytac czlowiekowi"
        );
        assert_eq!(
            s.trasa(&src, Some(String::new())).unwrap_err().kod(),
            "KanalBezFormatu"
        );
    }

    #[test]
    fn format_spoza_lancucha_nie_handluje_ale_zostawia_slad() {
        let s = zbuduj(PulapyGlobalne::default(), 9, 3);
        let src = SourceKey::new(-100, None);
        let e = s.trasa(&src, Some("Nieznany".into())).unwrap_err();
        assert_eq!(e.kod(), "FormatNieHandluje");
        assert!(e.opis().contains("Nieznany"));
    }

    #[test]
    fn wiadomosc_trafia_do_silnika_swojego_formatu() {
        let s = zbuduj(PulapyGlobalne::default(), 9, 3);
        let src = SourceKey::new(-100, None);
        let i = s.trasa(&src, Some("Synergy".into())).unwrap();
        assert_eq!(s.lista[i].format, "Synergy");
        assert_eq!(s.lista[i].preset, "P-SYN");
    }

    /// Brakujacy plik presetu nie moze przejsc w milczeniu - to znaczy, ze
    /// cala galaz sygnalow przepadnie.
    #[test]
    fn brak_presetu_na_dysku_jest_zglaszany() {
        let mut puste = presety(9, 3);
        puste.remove("P-SYN");
        let (s, braki) = Silniki::zbuduj(
            &lancuch_dwa(PulapyGlobalne::default()),
            &puste,
            &Settings::default(),
            1000.0,
        );
        assert_eq!(s.lista.len(), 1, "format bez presetu nie dostaje silnika");
        assert_eq!(braki.len(), 1);
        assert_eq!(braki[0].kod(), "PresetNieIstnieje");
    }

    // ============================================================
    //  ZAMKNIETE TRANSAKCJE
    // ============================================================

    /// POCZEKALNIA: pierwszy silnik nie ma prawa zabrac cudzych zamkniec.
    /// Bez tego jego seria strat i wynik dnia liczylyby sie z cudzych pozycji.
    #[test]
    fn zamkniete_transakcje_trafiaja_do_wlasciciela() {
        let mut s = zbuduj(PulapyGlobalne::default(), 9, 3);
        let mut b = broker();
        let (s0, s1) = (s.lista[0].slot, s.lista[1].slot);
        otworz(&mut b, wielosilnik::baza_slotu(s0) + 1);
        otworz(&mut b, wielosilnik::baza_slotu(s1) + 1);
        let tickety: Vec<u64> = b.positions().iter().map(|p| p.ticket).collect();
        for t in tickety {
            let _ = b.close_position(t, CloseReason::Manual);
        }

        let moje0 = s.z_widokiem(0, &mut b, |_e, w| w.drain_closed().len());
        assert_eq!(
            moje0, 1,
            "pierwszy silnik bierze WYLACZNIE swoje zamkniecie"
        );
        assert_eq!(s.poczekalnia_len(), 1, "cudze czeka na wlasciciela");

        let moje1 = s.z_widokiem(1, &mut b, |_e, w| w.drain_closed().len());
        assert_eq!(moje1, 1);
        assert_eq!(
            s.poczekalnia_len(),
            0,
            "nic nie moze zostac w poczekalni na zawsze"
        );
    }

    // ============================================================
    //  POJEDYNCZY SILNIK = STAN SPRZED ZMIANY
    // ============================================================

    /// Gwarancja parytetu w jednym zdaniu: przy jednym formacie slot to 0,
    /// numeracja koszykow rusza od 1, a widok nie ukrywa niczego.
    #[test]
    fn pojedynczy_silnik_zachowuje_sie_jak_przed_zmiana() {
        let e = Engine::new(Settings::default(), 1000.0);
        let mut s = Silniki::pojedynczy(
            e,
            "ATFX".into(),
            "HYPER-X1".into(),
            Lancuch::default(),
            true,
        );
        assert_eq!(s.lista[0].slot, wielosilnik::SLOT_STARY);
        assert_eq!(
            s.lista[0].engine.next_basket_id(),
            1,
            "koszyki dalej numeruja sie od B1"
        );

        let mut b = broker();
        otworz(&mut b, 1);
        otworz(&mut b, 7);
        reczny(&mut b);
        let widziane = s.z_widokiem(0, &mut b, |_e, w| w.positions().len());
        assert_eq!(
            widziane, 3,
            "przy jednym silniku widok przepuszcza WSZYSTKO"
        );
    }


    fn lancuch_trzy(pulapy: PulapyGlobalne) -> Lancuch {
        let mut l = Lancuch {
            nazwa: "TRIO".into(),
            ..Default::default()
        };
        l.presety.insert("Synergy".into(), "P-SYN".into());
        l.presety.insert("ZEN".into(), "P-ZEN".into());
        l.presety.insert("STORM".into(), "P-STORM".into());
        l.pulapy = pulapy;
        l
    }

    fn presety_trzy(max_poz: u32, max_lotow_nogi: f64) -> BTreeMap<String, Settings> {
        let mut s = Settings::default();
        s.max_open_positions = max_poz;
        s.max_directional_lots = max_lotow_nogi;
        s.session_filter = false;
        let mut m = BTreeMap::new();
        for n in ["P-SYN", "P-ZEN", "P-STORM"] {
            m.insert(n.to_string(), s.clone());
        }
        m
    }

    fn zbuduj_trzy(pulapy: PulapyGlobalne, max_poz: u32, max_lotow_nogi: f64) -> Silniki {
        Silniki::zbuduj(
            &lancuch_trzy(pulapy),
            &presety_trzy(max_poz, max_lotow_nogi),
            &Settings::default(),
            1000.0,
        )
        .0
    }

    fn wolno(s: &mut Silniki, i: usize, b: &mut SimBroker) -> bool {
        s.z_widokiem(i, b, |e, w| {
            e.entry_gate(w, 1_700_000_001_000).blocked().is_none()
        })
    }

    /// (a) LIMIT SKUTECZNY = MIN(limit nogi, pulap globalny) przy TRZECH nogach.
    ///
    /// Kazda noga ma wlasny limit 4 pozycji (razem 12 na rachunku), pulap
    /// lancucha 6. Po szesciu pozycjach STOJA WSZYSTKIE TRZY, choc zadna nie
    /// zblizyla sie do wlasnego limitu - a przy pulapie WYZSZYM niz suma
    /// nog rzadzi z powrotem limit nogi. To jest dokladnie ta sama semantyka,
    /// ktora backtest dostaje przez `--pulapy`.
    #[test]
    fn trzy_nogi_limit_skuteczny_jest_minimum() {
        let mut s = zbuduj_trzy(
            PulapyGlobalne {
                max_pozycji: 6,
                ..Default::default()
            },
            4,
            0.0,
        );
        assert_eq!(s.lista.len(), 3, "trzy formaty = trzy silniki");
        let mut b = broker();
        // po dwie pozycje na noge = 6 na rachunku; NIKT nie zlamal swojego limitu 4
        let sloty: Vec<u32> = s.lista.iter().map(|x| x.slot).collect();
        for sl in &sloty {
            for k in 0..2 {
                otworz(&mut b, wielosilnik::baza_slotu(*sl) + 1 + k);
            }
        }
        assert_eq!(b.positions().len(), 6);

        s.przelicz_obce(&b, None);
        for i in 0..3 {
            assert!(
                !wolno(&mut s, i, &mut b),
                "noga {i} ma jeszcze 2 miejsca WLASNEGO limitu, ale pulap rachunku jest wyczerpany"
            );
        }

        // PULAP WYZSZY NIZ SUMA NOG (13 > 3x4) nie ma prawa niczego zaciskac:
        // rzadzi wtedy limit nogi, czyli 4. To jest polowa umowy „min(...)",
        // o ktora prosil uzytkownik: 30 NA FORMAT oraz 40 NA RACHUNEK.
        let mut luzny = zbuduj_trzy(
            PulapyGlobalne {
                max_pozycji: 13,
                ..Default::default()
            },
            4,
            0.0,
        );
        luzny.przelicz_obce(&b, None);
        for i in 0..3 {
            assert!(
                wolno(&mut luzny, i, &mut b),
                "noga {i} ma wlasne miejsce, pulap jest luzniejszy"
            );
        }
    }

    #[test]
    fn widok_pokazuje_rachunek_calego_konta_a_nie_swojego_slotu() {
        let mut s = zbuduj_trzy(PulapyGlobalne::default(), 0, 0.0);
        let mut b = SimBroker::new(1000.0, 0.2, 0.0);
        b.on_quote(Quote {
            ts: 1_700_000_000_000,
            bid: 4000.0,
            ask: 4000.3,
        });
        let sloty: Vec<u32> = s.lista.iter().map(|x| x.slot).collect();
        // noga 0 ma JEDNA pozycje, nogi 1 i 2 po dziesiec
        otworz(&mut b, wielosilnik::baza_slotu(sloty[0]) + 1);
        for sl in &sloty[1..] {
            for k in 0..10u32 {
                otworz(&mut b, wielosilnik::baza_slotu(*sl) + 1 + k / 5);
            }
        }
        let prawda = b.account();
        assert!(prawda.margin > 0.0);

        let widziany = s.z_widokiem(0, &mut b, |_e, w| w.account());
        assert!(
            (widziany.margin - prawda.margin).abs() < 1e-9,
            "noga z 1 pozycja na 21 ma widziec margines CALEGO konta: {} zamiast {}",
            widziany.margin,
            prawda.margin
        );
        assert!(
            (widziany.equity - prawda.equity).abs() < 1e-9,
            "equity tez opisuje konto, nie slot: {} zamiast {}",
            widziany.equity,
            prawda.equity
        );
        assert!((widziany.free_margin - prawda.free_margin).abs() < 1e-9);

        // POZYCJE dalej sa filtrowane — to jest cala umowa `Widok`:
        // limity PRESETU maja opisywac format, a RACHUNEK jest wspolny.
        let ile = s.z_widokiem(0, &mut b, |_e, w| w.positions().len());
        assert_eq!(ile, 1, "widok pozycji ma zostac odfiltrowany po slocie");
    }

    /// PARYTET TEJ NAPRAWY: przy JEDNYM silniku widok nie chowa niczego,
    /// wiec `account()` musi byc tozsamosciowe CO DO BITU. Gdyby drgnelo,
    /// bramka parytetu (FS-M3-SYN 162615.75) bylaby do wyrzucenia.
    #[test]
    fn przy_jednej_nodze_rachunek_z_widoku_jest_tozsamosciowy() {
        let mut s = Silniki::pojedynczy(
            Engine::new(Settings::default(), 1000.0),
            "Synergy".into(),
            "P-SYN".into(),
            Lancuch::default(),
            true,
        );
        let mut b = broker();
        otworz(&mut b, 1);
        otworz(&mut b, 7);
        reczny(&mut b);
        let prawda = b.account();
        let widziany = s.z_widokiem(0, &mut b, |_e, w| w.account());
        assert_eq!(widziany.equity.to_bits(), prawda.equity.to_bits());
        assert_eq!(widziany.margin.to_bits(), prawda.margin.to_bits());
        assert_eq!(widziany.free_margin.to_bits(), prawda.free_margin.to_bits());
        assert_eq!(widziany.balance.to_bits(), prawda.balance.to_bits());
    }

    /// (b) KONTRAKT ZERA: pulapy nie zmieniaja ukladu JEDNONOZNEGO.
    ///
    /// Bramka parytetu mierzy przebieg jednonogowy. Gdyby warstwa pulapow
    /// dokladala tam cokolwiek — choćby jedno porownanie liczone od innej
    /// liczby — kazda liczba z bramki bylaby do wyrzucenia. Przy jednym
    /// silniku obce obciazenie MUSI byc zerowe, a bramka wejscia musi dac ten
    /// sam werdykt z pulapami i bez nich.
    #[test]
    fn pulapy_nie_zmieniaja_ukladu_jednonozego() {
        let mut cfg = Settings::default();
        cfg.max_open_positions = 4;
        cfg.session_filter = false;
        let mut b = broker();
        for k in 0..3 {
            otworz(&mut b, 1 + k);
        }

        let mut bez = Silniki::pojedynczy(
            Engine::new(cfg.clone(), 1000.0),
            "Synergy".into(),
            "P-SYN".into(),
            Lancuch::default(),
            true,
        );
        let mut lancuch = Lancuch::default();
        lancuch.pulapy = PulapyGlobalne {
            max_pozycji: 6,
            max_koszykow: 3,
            max_lotow: 1.0,
            max_lotow_kierunkowo: 1.0,
            blokuj_przeciwne_kierunki: true,
            ..Default::default()
        };
        let mut e = Engine::new(cfg, 1000.0);
        e.pulapy = lancuch.pulapy.clone();
        let mut z = Silniki::pojedynczy(e, "Synergy".into(), "P-SYN".into(), lancuch, true);

        bez.przelicz_obce(&b, Some(Side::Sell));
        z.przelicz_obce(&b, Some(Side::Sell));
        assert_eq!(
            z.lista[0].engine.obce,
            conduit_core::wielosilnik::ObceObciazenie::default(),
            "przy jednym silniku nie ma zadnego obcego obciazenia"
        );
        assert_eq!(bez.lista[0].engine.obce, z.lista[0].engine.obce);
        assert_eq!(
            wolno(&mut bez, 0, &mut b),
            wolno(&mut z, 0, &mut b),
            "pulapy nie maja prawa zmienic werdyktu przy jednej nodze"
        );
        assert!(
            wolno(&mut z, 0, &mut b),
            "trzy pozycje przy limicie 4 to jeszcze wolne miejsce"
        );
    }

    #[test]
    fn trzy_nogi_nie_utopia_wspolnego_marginesu() {
        // Saldo 1000, dzwignia 500, zloto po 4000 => 0,01 lota ≈ 8 $ marginesu.
        let mut b = SimBroker::new(1000.0, 0.2, 0.0);
        b.on_quote(Quote {
            ts: 1_700_000_000_000,
            bid: 4000.0,
            ask: 4000.3,
        });

        // ---------- BEZ PULAPU: kazda noga siedzi w swoim limicie ----------
        let mut bez = zbuduj_trzy(PulapyGlobalne::default(), 0, 0.50);
        let sloty: Vec<u32> = bez.lista.iter().map(|x| x.slot).collect();
        for sl in &sloty {
            for k in 0..30u32 {
                otworz(&mut b, wielosilnik::baza_slotu(*sl) + 1 + k / 10);
            }
        }
        assert!(
            (b.positions().iter().map(|p| p.volume).sum::<f64>() - 0.90).abs() < 1e-9,
            "kontrola testu: broker mial przyjac 0,90 lota"
        );
        bez.przelicz_obce(&b, None);
        let bez_pulapu = b.margin_level_pct();
        assert!(
            bez_pulapu < 200.0,
            "kontrola testu: bez pulapu trzy nogi maja zjechac marginesem nisko (jest {bez_pulapu:.1} %)"
        );
        for i in 0..3 {
            // Kazda noga ma 0,30 lota przy WLASNYM limicie 0,50 — czyli
            // zadna nie widzi powodu, zeby przestac, choc rachunek juz jedzie
            // po marginesie. To jest DOKLADNIE ta luka: straze per noga,
            // ryzyko wspolne.
            assert!(
                wolno(&mut bez, i, &mut b),
                "noga {i} pilnuje wylacznie swojego wolumenu i przepuszcza dalej"
            );
        }

        // ---------- Z PULAPEM: stoja wszystkie, margines zdrowy ----------
        let mut b2 = SimBroker::new(1000.0, 0.2, 0.0);
        b2.on_quote(Quote {
            ts: 1_700_000_000_000,
            bid: 4000.0,
            ask: 4000.3,
        });
        let mut z = zbuduj_trzy(
            PulapyGlobalne {
                max_lotow: 0.24,
                ..Default::default()
            },
            0,
            0.50,
        );
        let sloty: Vec<u32> = z.lista.iter().map(|x| x.slot).collect();
        // Dokladamy po jednej pozycji na noge az DOWOLNA noga stanie —
        // czyli symulujemy trzy nogi dokladajace rownolegle na jednym koncie.
        let mut nr = 0u32;
        loop {
            z.przelicz_obce(&b2, None);
            let ktos_moze = (0..3).any(|i| wolno(&mut z, i, &mut b2));
            if !ktos_moze {
                break;
            }
            for (i, sl) in sloty.iter().enumerate() {
                z.przelicz_obce(&b2, None);
                if wolno(&mut z, i, &mut b2) {
                    otworz(&mut b2, wielosilnik::baza_slotu(*sl) + 1 + nr / 10);
                }
            }
            nr += 1;
            assert!(nr < 200, "petla testu nie moze biec w nieskonczonosc");
        }
        let lot = b2.positions().iter().map(|p| p.volume).sum::<f64>();
        assert!(
            lot <= 0.24 + 1e-9,
            "pulap lancucha przepuscil {lot:.2} lota przy sufycie 0,24"
        );
        let z_pulapem = b2.margin_level_pct();
        assert!(
            z_pulapem > 2.0 * bez_pulapu,
            "pulap ma zostawic margines w bezpiecznym zakresie (jest {z_pulapem:.1} %, bez pulapu {bez_pulapu:.1} %)"
        );
        assert!(
            z_pulapem > 400.0,
            "po zadzialaniu pulapu poziom marginesu nie ma prawa siedziec nisko ({z_pulapem:.1} %)"
        );
    }
}
