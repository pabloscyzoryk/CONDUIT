
use serde::{Deserialize, Serialize};

// ============================================================================
//  STAŁE — NIE POLA PRESETU
// ============================================================================

/// Poniżej tego poziomu marginesu (%) nie wolno **zwiększyć** ekspozycji.
///
/// Stała, nie pole. Konwencja „0 = bez limitu" jest udokumentowanym
/// anty-wzorcem (`MaximumGridLayer=0` rozbrajał strażnika w cudzym EA), a pole
/// w `Settings` prędzej czy później wejdzie do sweepu i zostanie wystrojone
/// do zera na tym reżimie, w którym akurat nie zabiło.
pub const SUFIT_ML_TWARDY_PCT: f64 = 150.0;

/// Poniżej tego udziału equity w kapitale szczytowym idzie likwidacja.
/// Bez bramki zerowej i bez nadpisania — dopóki podłoga jest wyłączalna,
/// „ruina niemożliwa z konstrukcji" nie jest prawdziwym zdaniem.
pub const PODLOGA_EQUITY_PCT: f64 = 30.0;

pub const PROG_ROZJAZDU_MARGINESU: f64 = 5.0;

/// Mnożnik kontraktu XAUUSD — 100 uncji na lot. Nie jest osią strojenia.
pub const MNOZNIK_KONTRAKTU_XAU: f64 = 100.0;

// ============================================================================
//  WEJŚCIE — NOGA
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Strona {
    Kupno,
    Sprzedaz,
}

impl Strona {
    #[inline]
    pub fn znak(self) -> f64 {
        match self {
            Strona::Kupno => 1.0,
            Strona::Sprzedaz => -1.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Noga {
    pub wolumen: f64,
    pub cena_odniesienia: f64,
    pub strona: Strona,
}

impl Noga {
    #[inline]
    pub fn nowa(wolumen: f64, cena_odniesienia: f64, strona: Strona) -> Self {
        Noga {
            wolumen,
            cena_odniesienia,
            strona,
        }
    }
}

#[inline]
pub fn margines_nogi(wolumen: f64, cena: f64, dzwignia: f64, mnoznik: f64) -> f64 {
    if dzwignia <= 0.0 {
        return 0.0;
    }
    wolumen * mnoznik * cena / dzwignia
}

/// Suma marginesu po nogach — BEZ kompensacji hedgingu (model konserwatywny).
#[inline]
pub fn margines_brutto(nogi: &[Noga], dzwignia: f64, mnoznik: f64) -> f64 {
    nogi.iter()
        .map(|n| margines_nogi(n.wolumen, n.cena_odniesienia, dzwignia, mnoznik))
        .sum()
}

/// WOLUMEN NIEPOKRYTY — `|Σ kupno − Σ sprzedaż|`.
///
/// Jedyna część księgi, za którą broker na koncie hedgingowym pobiera
/// margines **na pewno**, niezależnie od tego, jak ustawiony jest
/// `SYMBOL_MARGIN_HEDGED` i `SYMBOL_MARGIN_HEDGED_USE_LEG`.
#[inline]
pub fn wolumen_niepokryty(nogi: &[Noga]) -> f64 {
    nogi.iter()
        .map(|n| n.wolumen * n.strona.znak())
        .sum::<f64>()
        .abs()
}

/// WOLUMEN POKRYTY — `min(Σ kupno, Σ sprzedaż)`, czyli ta część księgi,
/// na której zniżka hedgingowa w ogóle może wystąpić.
#[inline]
pub fn wolumen_pokryty(nogi: &[Noga]) -> f64 {
    let mut k = 0.0;
    let mut s = 0.0;
    for n in nogi {
        match n.strona {
            Strona::Kupno => k += n.wolumen,
            Strona::Sprzedaz => s += n.wolumen,
        }
    }
    k.min(s)
}

// ============================================================================
//  MARGINES — DWA MODELE
// ============================================================================

/// KIERUNEK ROZJAZDU. Nie sama wielkość — ZNAK, bo tylko on mówi, czy błąd
/// oddaje nam zdolność, czy ją zabiera.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum KierunekRozjazdu {
    /// Modele zgodne w granicach [`PROG_ROZJAZDU_MARGINESU`].
    Zgodne,
    /// `model_brokera < model_nogi`. Broker widzi MNIEJ zajętego depozytu, więc
    /// `free_margin` z terminala jest ZAWYŻONY — i to jest kierunek, który
    /// dokłada ekspozycję. Przyczyny: kompensacja hedgingu albo nieświeży
    /// odczyt konta po wypełnieniu.
    RachunekZanizaZajety,
    /// `model_brokera > model_nogi`. Broker trzyma depozyt za coś, czego
    /// w naszej księdze już nie ma (nieświeży odczyt po zamknięciu) albo za
    /// nogę spoza naszego magicu. Kierunek konserwatywny dla ekspozycji,
    /// ale ZAWYŻA ryzyko stop-outu — i to jest ten model, po którym broker
    /// naprawdę liczy likwidację.
    RachunekZawyzaZajety,
    /// Brak liczby brokera — nie ma czego porównać.
    BrakLiczbyRachunku,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct IncydentMarginesu {
    pub kierunek: KierunekRozjazdu,
    pub model_nogi_usd: f64,
    pub model_brokera_usd: f64,
    pub rozjazd_pct: f64,
    /// O ile dolarów `acc.free_margin` kłamie wobec modelu wiążącego.
    /// Dodatnio = terminal pokazuje WIĘCEJ wolnego, niż wynika z księgi.
    pub blad_wolnego_usd: f64,
}

/// DWA MODELE MARGINESU Z JEDNEJ CHWILI.
///
/// Wszystkie pola są migawką JEDNEGO pulsu. Wołający ma obowiązek zebrać
/// nogi i `acc.margin` z tego samego odczytu — inaczej rozjazd, który ten typ
/// mierzy, jest jego własnym artefaktem.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Margines {
    /// Model nóg po OTWARTYCH POZYCJACH, wzorem `Engine::poziom_marginesu`,
    /// honorującym `konto_dzwignia`, BEZ kompensacji hedgingu.
    pub model_nogi_usd: f64,
    /// To samo PO WYPEŁNIENIU wszystkiego, co wisi (pozycje + zlecenia
    /// oczekujące). Odpowiednik wariantu `docelowy` z `poziom_marginesu`,
    /// czyli najgorszego scenariusza, który rachunek sam sobie przygotował.
    pub model_nogi_docelowy_usd: f64,
    pub model_brokera_usd: Option<f64>,
    pub equity_usd: f64,
    pub wolumen_pokryty: f64,
}

impl Margines {
    /// Zbiera migawkę z nóg i liczby brokera.
    #[allow(clippy::too_many_arguments)]
    pub fn zbierz(
        pozycje: &[Noga],
        wiszace: &[Noga],
        dzwignia: f64,
        mnoznik: f64,
        equity_usd: f64,
        model_brokera_usd: Option<f64>,
    ) -> Self {
        let m_poz = margines_brutto(pozycje, dzwignia, mnoznik);
        let m_wis = margines_brutto(wiszace, dzwignia, mnoznik);
        Margines {
            model_nogi_usd: m_poz,
            model_nogi_docelowy_usd: m_poz + m_wis,
            model_brokera_usd,
            equity_usd,
            wolumen_pokryty: wolumen_pokryty(pozycje),
        }
    }

    /// **WIĄŻE GORSZY Z DWÓCH.** Nie ma trybu, w którym korzystniejszy model
    /// wygrywa — i nie ma pola, którym dałoby się taki tryb włączyć.
    #[inline]
    pub fn uzyty_wiazacy(&self) -> f64 {
        self.model_nogi_usd
            .max(self.model_brokera_usd.unwrap_or(0.0))
    }

    /// To samo, ale z doliczeniem wiszących szczebli. Używane wyłącznie tam,
    /// gdzie pytanie brzmi „czy wolno DOŁOŻYĆ" — bo wtedy scenariuszem
    /// odniesienia jest księga po wypełnieniu, a nie księga teraz.
    #[inline]
    pub fn uzyty_wiazacy_docelowy(&self) -> f64 {
        self.model_nogi_docelowy_usd
            .max(self.model_brokera_usd.unwrap_or(0.0) + self.nadwyzka_wiszacych())
    }

    #[inline]
    fn nadwyzka_wiszacych(&self) -> f64 {
        (self.model_nogi_docelowy_usd - self.model_nogi_usd).max(0.0)
    }

    /// WOLNY DEPOZYT WEDŁUG MODELU WIĄŻĄCEGO.
    ///
    /// **Jedyna liczba, którą wolno podać bramce zwiększającej ekspozycję.**
    /// `acc.free_margin` prosto z terminala nią NIE jest: przy
    /// `RachunekZanizaZajety` jest zawyżony dokładnie w tej chwili, w której
    /// rodzina A zamienia go na jednostki.
    #[inline]
    pub fn wolny_wiazacy_usd(&self) -> f64 {
        (self.equity_usd - self.uzyty_wiazacy()).max(0.0)
    }

    /// Poziom marginesu (%) liczony modelem wiążącym. `None` = brak
    /// ekspozycji, czyli poziom nieskończony — i tak trzeba to czytać,
    /// bo zero w tym miejscu kazałoby każdej bramce uznać puste konto
    /// za katastrofę.
    #[inline]
    pub fn poziom_wiazacy_pct(&self) -> Option<f64> {
        let m = self.uzyty_wiazacy();
        if m > 0.0 {
            Some(self.equity_usd / m * 100.0)
        } else {
            None
        }
    }

    /// Poziom PO WYPEŁNIENIU wszystkiego, co wisi.
    #[inline]
    pub fn poziom_docelowy_wiazacy_pct(&self) -> Option<f64> {
        let m = self.uzyty_wiazacy_docelowy();
        if m > 0.0 {
            Some(self.equity_usd / m * 100.0)
        } else {
            None
        }
    }

    /// Rozjazd modeli w procentach większego z nich.
    ///
    /// Mianownikiem jest MAKSIMUM, nie model nóg: przy `model_nogi = 0`
    /// i niezerowym brokerze dzielenie przez model nóg dałoby nieskończoność
    /// zamiast liczby, a to jest realny stan (nasza księga pusta, na koncie
    /// wisi cudza noga).
    #[inline]
    pub fn rozjazd_pct(&self) -> Option<f64> {
        let b = self.model_brokera_usd?;
        let a = self.model_nogi_usd;
        let odn = a.max(b);
        if odn <= 0.0 {
            return Some(0.0);
        }
        Some((a - b).abs() / odn * 100.0)
    }

    #[inline]
    pub fn kierunek(&self) -> KierunekRozjazdu {
        let Some(b) = self.model_brokera_usd else {
            return KierunekRozjazdu::BrakLiczbyRachunku;
        };
        match self.rozjazd_pct() {
            Some(r) if r > PROG_ROZJAZDU_MARGINESU => {
                if b < self.model_nogi_usd {
                    KierunekRozjazdu::RachunekZanizaZajety
                } else {
                    KierunekRozjazdu::RachunekZawyzaZajety
                }
            }
            _ => KierunekRozjazdu::Zgodne,
        }
    }

    /// `Some` wyłącznie wtedy, gdy rozjazd przekroczył próg. Zdarzenie do
    /// dziennika (`MarginDivergence`), nie kolumna w tabeli.
    pub fn incydent(&self) -> Option<IncydentMarginesu> {
        let b = self.model_brokera_usd?;
        let r = self.rozjazd_pct()?;
        if r <= PROG_ROZJAZDU_MARGINESU {
            return None;
        }
        Some(IncydentMarginesu {
            kierunek: self.kierunek(),
            model_nogi_usd: self.model_nogi_usd,
            model_brokera_usd: b,
            rozjazd_pct: r,
            blad_wolnego_usd: (self.equity_usd - b) - self.wolny_wiazacy_usd(),
        })
    }
}

// ============================================================================
//  BRAMKA — JEDYNE WEJŚCIE DLA DECYZJI ZWIĘKSZAJĄCEJ EKSPOZYCJĘ
// ============================================================================

/// PYTANIE O ZWIĘKSZENIE EKSPOZYCJI.
///
/// `margines_nowej_ekspozycji_usd` liczy się BRUTTO, po stawce
/// NIEKOMPENSOWANEJ — także dla kontry. To jest punkt, którego nie miał żaden
/// z trzech projektów: kontra ogranicza ryzyko NETTO, ale margines płaci się
/// od GROSS, a zniżka hedgingowa jest własnością serwera brokera, nie naszą.
/// Preliczanie kontry po zniżce znaczyłoby oddanie sterowania ryzykiem
/// ustawieniu symbolu, którego nawet nie czytamy.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ZapytanieMarginesowe {
    pub margines_nowej_ekspozycji_usd: f64,
    /// Czy pytanie dotyczy szczebli, które dopiero wiszą (wtedy odniesieniem
    /// jest [`Margines::uzyty_wiazacy_docelowy`]).
    pub licz_wiszace: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KodOdmowyMarginesu {
    /// Poniżej [`SUFIT_ML_TWARDY_PCT`] po dołożeniu.
    PoziomPonizejSufitu,
    /// Equity poniżej [`PODLOGA_EQUITY_PCT`] szczytu — żadnej nowej ekspozycji.
    PodlogaEquity,
    /// Wolny depozyt modelu wiążącego nie pokrywa żądania.
    BrakWolnegoDepozytu,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum WerdyktMarginesu {
    Wolno {
        ml_po_pct: Option<f64>,
    },
    Odmowa {
        kod: KodOdmowyMarginesu,
        ml_po_pct: Option<f64>,
        sufit_pct: f64,
    },
}

impl WerdyktMarginesu {
    #[inline]
    pub fn wolno(&self) -> bool {
        matches!(self, WerdyktMarginesu::Wolno { .. })
    }
}

/// **N-MARGINES — JEDYNA BRAMKA.**
///
/// Cały niezmiennik mieści się w trzech zdaniach:
/// 1. odniesieniem jest [`Margines::uzyty_wiazacy`], czyli GORSZY z modeli;
/// 2. nowa ekspozycja liczy się BRUTTO, bez zniżki hedgingowej;
/// 3. sufit jest stałą — nie da się go wyłączyć ani wystroić.
///
/// `equity_szczyt_usd` służy wyłącznie podłodze equity; przy zerze podłoga
/// milczy (brak historii szczytu przy starcie, nie „wyłączona").
pub fn wolno_zwiekszyc(
    m: &Margines,
    z: &ZapytanieMarginesowe,
    equity_szczyt_usd: f64,
) -> WerdyktMarginesu {
    let uzyty = if z.licz_wiszace {
        m.uzyty_wiazacy_docelowy()
    } else {
        m.uzyty_wiazacy()
    };
    let po = uzyty + z.margines_nowej_ekspozycji_usd.max(0.0);
    let ml_po = if po > 0.0 {
        Some(m.equity_usd / po * 100.0)
    } else {
        None
    };

    if equity_szczyt_usd > 0.0 && m.equity_usd < equity_szczyt_usd * PODLOGA_EQUITY_PCT / 100.0 {
        return WerdyktMarginesu::Odmowa {
            kod: KodOdmowyMarginesu::PodlogaEquity,
            ml_po_pct: ml_po,
            sufit_pct: SUFIT_ML_TWARDY_PCT,
        };
    }
    if m.equity_usd - uzyty < z.margines_nowej_ekspozycji_usd {
        return WerdyktMarginesu::Odmowa {
            kod: KodOdmowyMarginesu::BrakWolnegoDepozytu,
            ml_po_pct: ml_po,
            sufit_pct: SUFIT_ML_TWARDY_PCT,
        };
    }
    match ml_po {
        Some(x) if x < SUFIT_ML_TWARDY_PCT => WerdyktMarginesu::Odmowa {
            kod: KodOdmowyMarginesu::PoziomPonizejSufitu,
            ml_po_pct: ml_po,
            sufit_pct: SUFIT_ML_TWARDY_PCT,
        },
        _ => WerdyktMarginesu::Wolno { ml_po_pct: ml_po },
    }
}

// ============================================================================
//  TESTY
// ============================================================================

#[cfg(test)]
mod testy {
    use super::*;

    const LEV: f64 = 500.0;
    const K: f64 = MNOZNIK_KONTRAKTU_XAU;

    #[test]
    fn wzor_zgadza_sie_z_order_calc_margin() {
        let m = margines_nogi(1.0, 4318.68, LEV, K);
        assert!((m - 863.736).abs() < 1e-3, "{m}");
    }

    #[test]
    fn wzor_zgadza_sie_z_zywym_rachunkiem() {
        assert!((margines_nogi(0.01, 4640.0, LEV, K) - 9.28).abs() < 5e-3);
        assert!((margines_nogi(0.15, 4388.333, LEV, K) - 131.65).abs() < 1e-2);
    }

    /// Księga jednostronna: oba modele MUSZĄ dać tę samą liczbę, a wolumen
    /// pokryty jest zerem. Ten test jest kontraktem zera dla całego modułu.
    #[test]
    fn ksiega_jednostronna_zero_rozjazdu() {
        let poz = vec![
            Noga::nowa(0.01, 4630.0, Strona::Kupno),
            Noga::nowa(0.01, 4635.0, Strona::Kupno),
            Noga::nowa(0.01, 4640.0, Strona::Kupno),
        ];
        let brutto = margines_brutto(&poz, LEV, K);
        let m = Margines::zbierz(&poz, &[], LEV, K, 293.34, Some(brutto));
        assert_eq!(m.wolumen_pokryty, 0.0);
        assert_eq!(m.rozjazd_pct(), Some(0.0));
        assert_eq!(m.kierunek(), KierunekRozjazdu::Zgodne);
        assert!(m.incydent().is_none());
        assert!((m.uzyty_wiazacy() - brutto).abs() < 1e-12);
    }

    #[test]
    fn nieswiezy_odczyt_konta_jest_incydentem_i_nie_daje_zdolnosci() {
        let poz = vec![
            Noga::nowa(0.01, 4632.0, Strona::Kupno),
            Noga::nowa(0.01, 4635.0, Strona::Kupno),
            Noga::nowa(0.01, 4638.0, Strona::Kupno),
        ];
        let m = Margines::zbierz(&poz, &[], LEV, K, 293.34, Some(9.28));
        let inc = m.incydent().expect("rozjazd 3x musi być incydentem");
        assert_eq!(inc.kierunek, KierunekRozjazdu::RachunekZanizaZajety);
        assert!(inc.rozjazd_pct > 60.0, "{}", inc.rozjazd_pct);
        // free_margin z terminala kłamie o mniej więcej dwie nogi
        assert!(inc.blad_wolnego_usd > 18.0, "{}", inc.blad_wolnego_usd);
        // model wiążący bierze WIĘKSZY zajęty, więc wolnego jest MNIEJ
        assert!(m.wolny_wiazacy_usd() < 293.34 - 9.28);
    }

    #[test]
    fn nieswiezy_odczyt_po_zamknieciu_tez_zabiera_zdolnosc() {
        let poz: Vec<Noga> = (0..6)
            .map(|i| Noga::nowa(0.01, 4630.0 + i as f64, Strona::Kupno))
            .collect();
        let m = Margines::zbierz(&poz, &[], LEV, K, 285.23, Some(64.86));
        let inc = m.incydent().expect("rozjazd 14 % musi być incydentem");
        assert_eq!(inc.kierunek, KierunekRozjazdu::RachunekZawyzaZajety);
        assert!(
            (m.uzyty_wiazacy() - 64.86).abs() < 1e-9,
            "wiąże liczbę BROKERA"
        );
    }

    /// KOMPENSACJA HEDGINGU. Broker liczy sam wolumen niepokryty (0), model
    /// nóg sumuje brutto. Wiążący ma wziąć model nóg — czyli NIE skorzystać
    /// ze zniżki, której nie kontrolujemy.
    #[test]
    fn kompensacja_hedgingu_nie_daje_zdolnosci() {
        let poz = vec![
            Noga::nowa(0.05, 4600.0, Strona::Kupno),
            Noga::nowa(0.05, 4610.0, Strona::Sprzedaz),
        ];
        let brutto = margines_brutto(&poz, LEV, K);
        assert!((wolumen_niepokryty(&poz)).abs() < 1e-12);
        assert!((wolumen_pokryty(&poz) - 0.05).abs() < 1e-12);
        // wariant SYMBOL_MARGIN_HEDGED = 0 → broker liczy 0
        let m0 = Margines::zbierz(&poz, &[], LEV, K, 238.88, Some(0.0));
        assert!((m0.uzyty_wiazacy() - brutto).abs() < 1e-9);
        assert_eq!(m0.kierunek(), KierunekRozjazdu::RachunekZanizaZajety);
        // wariant USE_LEG = true → broker liczy większą nogę, czyli połowę
        let m1 = Margines::zbierz(&poz, &[], LEV, K, 238.88, Some(brutto / 2.0));
        assert!((m1.uzyty_wiazacy() - brutto).abs() < 1e-9);
        // w OBU wariantach wiążący jest ten sam — model nóg
        assert_eq!(m0.uzyty_wiazacy().to_bits(), m1.uzyty_wiazacy().to_bits());
    }

    /// Wiszące szczeble wchodzą wyłącznie do wariantu docelowego.
    #[test]
    fn wiszace_licza_sie_tylko_do_docelowego() {
        let poz = vec![Noga::nowa(0.01, 4600.0, Strona::Kupno)];
        let wis = vec![
            Noga::nowa(0.01, 4580.0, Strona::Kupno),
            Noga::nowa(0.01, 4560.0, Strona::Kupno),
        ];
        let m = Margines::zbierz(
            &poz,
            &wis,
            LEV,
            K,
            238.88,
            Some(margines_brutto(&poz, LEV, K)),
        );
        assert!(
            (m.model_nogi_usd - 9.2).abs() < 1e-9,
            "{}",
            m.model_nogi_usd
        );
        // 9,20 + 9,16 + 9,12
        assert!(
            (m.model_nogi_docelowy_usd - 27.48).abs() < 1e-9,
            "{}",
            m.model_nogi_docelowy_usd
        );
        assert!(m.poziom_docelowy_wiazacy_pct().unwrap() < m.poziom_wiazacy_pct().unwrap());
    }

    /// Pusta księga to poziom NIESKOŃCZONY, nie zero.
    #[test]
    fn pusta_ksiega_nie_jest_katastrofa() {
        let m = Margines::zbierz(&[], &[], LEV, K, 238.88, Some(0.0));
        assert_eq!(m.poziom_wiazacy_pct(), None);
        assert!(wolno_zwiekszyc(
            &m,
            &ZapytanieMarginesowe {
                margines_nowej_ekspozycji_usd: 0.0,
                licz_wiszace: false
            },
            238.88
        )
        .wolno());
    }

    /// Bramka odmawia poniżej sufitu — i sufit NIE jest polem.
    #[test]
    fn bramka_odmawia_ponizej_sufitu() {
        // 20 nóg × 0,01 przy ~4640 = ~185,6 $ zajętego przy equity 238,88 $,
        // czyli poziom ~129 % — dokładnie ten stan, w którym dokładanie
        // jeszcze jednej nogi jest ostatnią rzeczą, jaką wolno zrobić.
        let poz: Vec<Noga> = (0..20)
            .map(|i| Noga::nowa(0.01, 4630.0 + i as f64, Strona::Kupno))
            .collect();
        let brutto = margines_brutto(&poz, LEV, K);
        let m = Margines::zbierz(&poz, &[], LEV, K, 238.88, Some(brutto));
        let z = ZapytanieMarginesowe {
            margines_nowej_ekspozycji_usd: margines_nogi(0.01, 4640.0, LEV, K),
            licz_wiszace: false,
        };
        match wolno_zwiekszyc(&m, &z, 300.0) {
            WerdyktMarginesu::Odmowa { kod, .. } => {
                assert_eq!(kod, KodOdmowyMarginesu::PoziomPonizejSufitu)
            }
            w => panic!("miała być odmowa, jest {w:?}"),
        }
    }

    /// Podłoga equity nie ma bramki zerowej ani nadpisania.
    #[test]
    fn podloga_equity_odmawia_przed_wszystkim() {
        let poz = vec![Noga::nowa(0.01, 4630.0, Strona::Kupno)];
        let m = Margines::zbierz(&poz, &[], LEV, K, 60.0, Some(9.26));
        let z = ZapytanieMarginesowe {
            margines_nowej_ekspozycji_usd: 0.01,
            licz_wiszace: false,
        };
        match wolno_zwiekszyc(&m, &z, 300.0) {
            WerdyktMarginesu::Odmowa { kod, .. } => {
                assert_eq!(kod, KodOdmowyMarginesu::PodlogaEquity)
            }
            w => panic!("miała być odmowa podłogi, jest {w:?}"),
        }
    }

    /// N19 — migawka przeżywa serializację co do bitu.
    #[test]
    fn margines_przezywa_serializacje() {
        let poz = vec![
            Noga::nowa(0.03, 4632.17, Strona::Kupno),
            Noga::nowa(0.02, 4611.09, Strona::Sprzedaz),
        ];
        let m = Margines::zbierz(&poz, &[], LEV, K, 293.34, Some(41.2));
        let j = serde_json::to_string(&m).unwrap();
        let m2: Margines = serde_json::from_str(&j).unwrap();
        assert_eq!(m.uzyty_wiazacy().to_bits(), m2.uzyty_wiazacy().to_bits());
        assert_eq!(m, m2);
    }
}
