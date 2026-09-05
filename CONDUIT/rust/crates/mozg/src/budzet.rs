
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

/// PO CO zarezerwowano ryzyko. Zamknięta lista.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CelRezerwacji {
    SiatkaPoczatkowa,
    DokladkaRynkowa,
    PrzezbrojenieSiatki,
    PowrotPoStopie,
    PowrotPoCelu,
    Kontra,
    PodniesienieLota,
    /// ekspozycja, której na E0 nie umiemy przypisać do żadnej ze ścieżek
    /// (nie pasuje do żadnego szczebla planu) — liczona osobno, nigdy zerem
    NieprzypisanaEkspozycja,
}

impl CelRezerwacji {
    pub fn nazwa(self) -> &'static str {
        match self {
            CelRezerwacji::SiatkaPoczatkowa => "SiatkaPoczatkowa",
            CelRezerwacji::DokladkaRynkowa => "DokladkaRynkowa",
            CelRezerwacji::PrzezbrojenieSiatki => "PrzezbrojenieSiatki",
            CelRezerwacji::PowrotPoStopie => "PowrotPoStopie",
            CelRezerwacji::PowrotPoCelu => "PowrotPoCelu",
            CelRezerwacji::Kontra => "Kontra",
            CelRezerwacji::PodniesienieLota => "PodniesienieLota",
            CelRezerwacji::NieprzypisanaEkspozycja => "NieprzypisanaEkspozycja",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OdmowaBudzetu {
    Wyczerpany,
    PonizejMinimalnegoLota,
}

/// BILET NA EKSPOZYCJĘ. Brak `Clone`/`Copy` jest CZĘŚCIĄ SPECYFIKACJI.
#[derive(Debug)]
pub struct Kwit {
    rama: u32,
    proba: u32,
    ryzyko_usd: f64,
    margines_usd: f64,
    cel: CelRezerwacji,
    _niekopiowalny: PhantomData<*const ()>,
}

impl Kwit {
    pub fn rama(&self) -> u32 {
        self.rama
    }
    pub fn proba(&self) -> u32 {
        self.proba
    }
    pub fn ryzyko_usd(&self) -> f64 {
        self.ryzyko_usd
    }
    pub fn margines_usd(&self) -> f64 {
        self.margines_usd
    }
    pub fn cel(&self) -> CelRezerwacji {
        self.cel
    }
}

/// BUDŻET RYZYKA NA CAŁE ŻYCIE RAMY.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudzetRamy {
    rama: u32,
    /// ile DZIŚ przyznałby silnik na JEDEN plan (mianownik `wykorzystany_pct`)
    przyznany_usd: f64,
    /// sufit egzekwowany — na E0 zawsze `f64::INFINITY`
    sufit_usd: f64,
    zarezerwowany_usd: f64,
    /// **NIGDY NIE MALEJE**
    wydany_usd: f64,
    zwrocony_usd: f64,
    zrealizowany_usd: f64,
    /// szczyt marginesu zaangażowanego przez ramę (obserwacja, N-MARGINES)
    margines_szczyt_usd: f64,
    kwitow_wystawionych: u32,
    kwitow_zwroconych: u32,
    kwitow_zuzytych: u32,
    /// ile ekspozycji poszło z kwitem NIEPRZYPISANYM do żadnego szczebla
    wydany_wg_celu: Vec<(String, f64)>,
}

impl BudzetRamy {
    /// E0: sufit nieskończony. To NIE jest wartość domyślna do zmiany
    /// w presecie — na tym etapie księga ma nie mieć wpływu na nic.
    pub fn ksiega_bez_sufitu(rama: u32, przyznany_usd: f64) -> Self {
        BudzetRamy {
            rama,
            przyznany_usd,
            sufit_usd: f64::INFINITY,
            zarezerwowany_usd: 0.0,
            wydany_usd: 0.0,
            zwrocony_usd: 0.0,
            zrealizowany_usd: 0.0,
            margines_szczyt_usd: 0.0,
            kwitow_wystawionych: 0,
            kwitow_zwroconych: 0,
            kwitow_zuzytych: 0,
            wydany_wg_celu: Vec::new(),
        }
    }

    #[inline]
    pub fn przyznany(&self) -> f64 {
        self.przyznany_usd
    }
    #[inline]
    pub fn wydany(&self) -> f64 {
        self.wydany_usd
    }
    #[inline]
    pub fn zrealizowany(&self) -> f64 {
        self.zrealizowany_usd
    }
    #[inline]
    pub fn margines_szczyt(&self) -> f64 {
        self.margines_szczyt_usd
    }
    #[inline]
    pub fn kwitow(&self) -> (u32, u32, u32) {
        (
            self.kwitow_wystawionych,
            self.kwitow_zwroconych,
            self.kwitow_zuzytych,
        )
    }

    #[inline]
    pub fn dostepny(&self) -> f64 {
        (self.sufit_usd - self.zarezerwowany_usd - (self.wydany_usd - self.zwrocony_usd)).max(0.0)
    }

    #[inline]
    pub fn wykorzystany_pct(&self) -> Option<f64> {
        if self.przyznany_usd > 0.0 {
            Some(self.wydany_usd / self.przyznany_usd * 100.0)
        } else {
            None
        }
    }

    /// Na E0 NIGDY nie odmawia — i to jest jej bramka akceptacji.
    pub fn rezerwuj(
        &mut self,
        proba: u32,
        ryzyko_usd: f64,
        margines_usd: f64,
        cel: CelRezerwacji,
    ) -> Result<Kwit, OdmowaBudzetu> {
        if ryzyko_usd > self.dostepny() {
            return Err(OdmowaBudzetu::Wyczerpany);
        }
        self.zarezerwowany_usd += ryzyko_usd;
        self.kwitow_wystawionych += 1;
        Ok(Kwit {
            rama: self.rama,
            proba,
            ryzyko_usd,
            margines_usd,
            cel,
            _niekopiowalny: PhantomData,
        })
    }

    /// Odmowa brokera — rezerwacja wraca, `wydany` nietknięty.
    pub fn zwroc(&mut self, k: Kwit) {
        self.zarezerwowany_usd = (self.zarezerwowany_usd - k.ryzyko_usd).max(0.0);
        self.kwitow_zwroconych += 1;
    }

    /// Ekspozycja powstała. `wydany_usd` rośnie i nie wróci.
    pub fn zuzyj(&mut self, k: Kwit, faktyczne_ryzyko_usd: f64, faktyczny_margines_usd: f64) {
        self.zarezerwowany_usd = (self.zarezerwowany_usd - k.ryzyko_usd).max(0.0);
        self.wydany_usd += faktyczne_ryzyko_usd.max(0.0);
        self.kwitow_zuzytych += 1;
        self.margines_szczyt_usd = self.margines_szczyt_usd.max(faktyczny_margines_usd);
        let nazwa = k.cel.nazwa().to_string();
        match self.wydany_wg_celu.iter_mut().find(|(c, _)| *c == nazwa) {
            Some((_, v)) => *v += faktyczne_ryzyko_usd.max(0.0),
            None => self
                .wydany_wg_celu
                .push((nazwa, faktyczne_ryzyko_usd.max(0.0))),
        }
    }

    pub fn rozlicz_zamkniecie(&mut self, ryzyko_uwolnione_usd: f64, wynik_usd: f64) {
        self.zwrocony_usd += ryzyko_uwolnione_usd.max(0.0);
        self.zrealizowany_usd += wynik_usd;
    }

    /// JEDYNA zmiana sufitu, jaka istnieje. `podnies` NIE ISTNIEJE (N7).
    pub fn obniz(&mut self, do_usd: f64) -> bool {
        if do_usd < self.sufit_usd {
            self.sufit_usd = do_usd;
            true
        } else {
            false
        }
    }

    /// Bilans księgi (N18): każdy wystawiony kwit został zwrócony albo zużyty.
    pub fn domyka_sie(&self) -> bool {
        self.kwitow_wystawionych == self.kwitow_zwroconych + self.kwitow_zuzytych
    }

    pub fn rozbicie_wydatku(&self) -> &[(String, f64)] {
        &self.wydany_wg_celu
    }
}

#[cfg(test)]
mod testy {
    use super::*;

    #[test]
    fn ksiega_bez_sufitu_nigdy_nie_odmawia() {
        let mut b = BudzetRamy::ksiega_bez_sufitu(1, 30.0);
        for _ in 0..1000 {
            let k = b
                .rezerwuj(0, 25.0, 9.26, CelRezerwacji::DokladkaRynkowa)
                .expect("na E0 księga nie ma prawa odmówić");
            b.zuzyj(k, 25.0, 9.26);
        }
        assert!(b.wykorzystany_pct().unwrap() > 10_000.0);
        assert!(b.domyka_sie());
    }

    #[test]
    fn wydany_nigdy_nie_maleje() {
        let mut b = BudzetRamy::ksiega_bez_sufitu(1, 10.0);
        let k = b
            .rezerwuj(0, 4.0, 9.26, CelRezerwacji::SiatkaPoczatkowa)
            .unwrap();
        b.zuzyj(k, 4.0, 9.26);
        let przed = b.wydany();
        b.rozlicz_zamkniecie(4.0, -4.0); // strata, ryzyko uwolnione
        assert_eq!(b.wydany(), przed, "zamknięcie nie zwraca WYDANEGO");
        assert!(b.wykorzystany_pct().unwrap() >= 40.0);
    }

    #[test]
    fn sufit_wolno_tylko_obnizyc() {
        let mut b = BudzetRamy::ksiega_bez_sufitu(1, 10.0);
        assert!(b.obniz(5.0));
        assert!(!b.obniz(50.0), "podniesienie sufitu musi być niemożliwe");
        assert!(b.rezerwuj(0, 6.0, 0.0, CelRezerwacji::Kontra).is_err());
    }

    #[test]
    fn rama_bez_sl_nie_ma_mianownika() {
        let b = BudzetRamy::ksiega_bez_sufitu(1, 0.0);
        assert_eq!(
            b.wykorzystany_pct(),
            None,
            "brak SL to NIE jest zero ryzyka (N11)"
        );
    }
}
