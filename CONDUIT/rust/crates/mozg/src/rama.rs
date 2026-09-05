
use serde::{Deserialize, Serialize};

pub type Ts = i64;
pub type Px = f64;

/// Wielkość kontraktu XAUUSD — 100 uncji. Powtórzona tutaj świadomie:
/// crate nie zna `conduit_core` i nie ma go poznać (granica 1).
pub const XAU_CONTRACT: f64 = 100.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Strona {
    Buy,
    Sell,
}

impl Strona {
    #[inline]
    pub fn znak(self) -> f64 {
        match self {
            Strona::Buy => 1.0,
            Strona::Sell => -1.0,
        }
    }
    /// Cena WYJŚCIA z pozycji tej strony (BUY wychodzi po bid).
    #[inline]
    pub fn wyjscie(self, bid: Px, ask: Px) -> Px {
        match self {
            Strona::Buy => bid,
            Strona::Sell => ask,
        }
    }
    pub fn z_napisu(s: &str) -> Option<Strona> {
        match s.trim().to_ascii_uppercase().as_str() {
            "BUY" | "LONG" => Some(Strona::Buy),
            "SELL" | "SHORT" => Some(Strona::Sell),
            _ => None,
        }
    }
}

/// Porządek wariantów == porządek życia. `Ord` jest tu treścią, nie ozdobą:
/// awans robi `max`, więc etap nie umie się cofnąć (N13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EtapRamy {
    /// pomysł istnieje, budżet policzony, u brokera nic
    Zawiazana,
    /// co najmniej jedna próba uzbrojona (zlecenia leżą)
    Czuwa,
    /// co najmniej jedna próba MIAŁA pozycję — ZATRZASK
    Zaangazowana,
    /// ZATRZASK
    Zabezpieczona,
    /// pomysł umarł, pieniądze mogą jeszcze pracować: ZERO nowych prób
    Wygaszana,
    Zamknieta,
}

impl EtapRamy {
    pub const PIERWSZY_ZATRZASK: EtapRamy = EtapRamy::Zaangazowana;
    #[inline]
    pub fn wolno_otwierac(self) -> bool {
        matches!(self, Self::Zawiazana | Self::Czuwa | Self::Zaangazowana)
    }
    pub fn nazwa(self) -> &'static str {
        match self {
            EtapRamy::Zawiazana => "Zawiazana",
            EtapRamy::Czuwa => "Czuwa",
            EtapRamy::Zaangazowana => "Zaangazowana",
            EtapRamy::Zabezpieczona => "Zabezpieczona",
            EtapRamy::Wygaszana => "Wygaszana",
            EtapRamy::Zamknieta => "Zamknieta",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RolaProby {
    Baza,
    Powrot,
    Kontra,
    Runner,
}

/// DLACZEGO pomysł się skończył. Zamknięta lista — „zdanie po polsku się nie
/// policzy" (`journal::RejectCode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PowodKonca {
    /// cena dotknęła SL pomysłu
    StopCeny,
    /// cena osiągnęła OSTATNI zadeklarowany cel
    OstatniCel,
    /// kanał kazał zamknąć / anulować
    KomendaKanalu,
    /// horyzont obserwacji (cenzura), nie rozstrzygnięcie
    Horyzont,
    /// koniec okna danych
    KoniecDanych,
}

impl PowodKonca {
    pub fn nazwa(self) -> &'static str {
        match self {
            PowodKonca::StopCeny => "StopCeny",
            PowodKonca::OstatniCel => "OstatniCel",
            PowodKonca::KomendaKanalu => "KomendaKanalu",
            PowodKonca::Horyzont => "Horyzont",
            PowodKonca::KoniecDanych => "KoniecDanych",
        }
    }
}

/// OKNO WAŻNOŚCI POMYSŁU — liczone z CENY i z KANAŁU, nigdy z naszego
/// wykonania. To jest cała różnica między „ile żył nasz koszyk" a „ile żył
/// pomysł traderów"; mieszanie tych dwóch wielkości było powodem, dla którego
/// `handle_sl_hit` kończy jedno razem z drugim.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OknoWaznosci {
    pub od: Ts,
    /// `None` = pomysł nadal ważny na koniec obserwacji
    pub do_ts: Option<Ts>,
    pub powod: Option<PowodKonca>,
}

impl OknoWaznosci {
    pub fn nowe(od: Ts) -> Self {
        OknoWaznosci {
            od,
            do_ts: None,
            powod: None,
        }
    }
    /// Zamknięcie okna jest JEDNORAZOWE — pierwszy powód wygrywa. Bez tego
    /// „ważność" byłaby funkcją kolejności iteracji po zdarzeniach.
    pub fn zamknij(&mut self, ts: Ts, powod: PowodKonca) {
        if self.do_ts.is_none() {
            self.do_ts = Some(ts);
            self.powod = Some(powod);
        }
    }
    pub fn zycie_ms(&self, koniec_obserwacji: Ts) -> i64 {
        self.do_ts.unwrap_or(koniec_obserwacji) - self.od
    }
    pub fn wazna_o(&self, ts: Ts) -> bool {
        ts >= self.od && self.do_ts.map(|k| ts < k).unwrap_or(true)
    }
}

/// GEOMETRIA POMYSŁU — dokładnie to, co podał kanał.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeometriaPomyslu {
    pub strona: Strona,
    /// krawędź BLIŻSZA rynkowi w chwili sygnału (BUY: górna)
    pub krawedz_blizsza: Px,
    /// krawędź DALSZA (BUY: dolna)
    pub krawedz_dalsza: Px,
    pub sl: Option<Px>,
    pub cele: Vec<Px>,
}

impl GeometriaPomyslu {
    pub fn nowa(strona: Strona, lo: Px, hi: Px, sl: Option<Px>, cele: Vec<Px>) -> Self {
        let (blizsza, dalsza) = match strona {
            Strona::Buy => (hi, lo),
            Strona::Sell => (lo, hi),
        };
        GeometriaPomyslu {
            strona,
            krawedz_blizsza: blizsza,
            krawedz_dalsza: dalsza,
            sl,
            cele,
        }
    }
    #[inline]
    pub fn szerokosc(&self) -> f64 {
        (self.krawedz_blizsza - self.krawedz_dalsza).abs()
    }
    /// GŁĘBOKOŚĆ CENY W SZEROKOŚCIACH STREFY. 0 = krawędź bliższa,
    /// 1 = krawędź dalsza, >1 = ZA dalszą krawędzią.
    ///
    /// To jest ta sama jednostka, w której liczy `ai/policy.rs:241-247` —
    /// i jedyna, w której da się porównać sygnały o różnej szerokości strefy.
    #[inline]
    pub fn glebokosc(&self, px: Px) -> f64 {
        let w = self.szerokosc();
        if w <= 0.0 {
            return 0.0;
        }
        (self.krawedz_blizsza - px) * self.strona.znak() / w
    }
    /// Ryzyko JEDNEJ nogi wobec stopu POMYSŁU, w dolarach.
    #[inline]
    pub fn ryzyko_nogi(&self, cena: Px, wolumen: f64) -> Option<f64> {
        let sl = self.sl?;
        Some(((cena - sl) * self.strona.znak()).max(0.0) * XAU_CONTRACT * wolumen)
    }
    #[inline]
    pub fn cel(&self, i: usize) -> Option<Px> {
        self.cele.get(i).copied()
    }
    /// Czy cena dotknęła celu `i` (bid dla BUY, ask dla SELL).
    #[inline]
    pub fn cel_dotkniety(&self, i: usize, bid: Px, ask: Px) -> bool {
        match self.cel(i) {
            Some(c) => match self.strona {
                Strona::Buy => bid >= c,
                Strona::Sell => ask <= c,
            },
            None => false,
        }
    }
    #[inline]
    pub fn sl_dotkniety(&self, bid: Px, ask: Px) -> bool {
        match self.sl {
            Some(s) => match self.strona {
                Strona::Buy => bid <= s,
                Strona::Sell => ask >= s,
            },
            None => false,
        }
    }
}

/// JEDNA PRÓBA — koszyk pracujący na rzecz ramy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proba {
    pub nr: u32,
    pub koszyk: u32,
    pub rola: RolaProby,
    pub strona: Strona,
    /// ryzyko zarezerwowane w chwili rozstawienia planu ($)
    pub ryzyko_rezerwacji: f64,
    pub ts_otwarcia: Ts,
    pub ts_zamkniecia: Ts,
    pub wynik_usd: f64,
    pub miala_pozycje: bool,
    pub stop_zabral: bool,
}

/// POMYSŁ Z KANAŁU. Żyje dłużej niż jego ekspozycja — i o tę różnicę chodzi.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rama {
    pub id: u32,
    /// odcisk setupu (FNV-1a po stronie i geometrii kwantyzowanej do kroku)
    pub odcisk: u64,
    pub kanal: String,
    pub msg_id: i64,
    pub geometria: GeometriaPomyslu,
    pub zalozyciel: u32,
    pub proby: Vec<Proba>,
    pub okno: OknoWaznosci,
    pub etap: EtapRamy,
    pub ts_zawiazania: Ts,
    pub ts_etapu: Ts,
    /// kronika komend kanału: (ts, rodzaj)
    pub komendy: Vec<(Ts, String)>,
}

impl Rama {
    pub fn zawiaz(
        id: u32,
        kanal: String,
        msg_id: i64,
        geometria: GeometriaPomyslu,
        zalozyciel: u32,
        ts: Ts,
        krok_odcisku: f64,
    ) -> Rama {
        let odcisk = odcisk_setupu(&geometria, krok_odcisku);
        Rama {
            id,
            odcisk,
            kanal,
            msg_id,
            geometria,
            zalozyciel,
            proby: Vec::new(),
            okno: OknoWaznosci::nowe(ts),
            etap: EtapRamy::Zawiazana,
            ts_zawiazania: ts,
            ts_etapu: ts,
            komendy: Vec::new(),
        }
    }

    /// ETAP JEST MONOTONICZNY (N13). Awans przez `max`, zejście niemożliwe.
    pub fn awansuj(&mut self, e: EtapRamy, ts: Ts) {
        if e > self.etap {
            self.etap = e;
            self.ts_etapu = ts;
        }
    }

    #[inline]
    pub fn wynik_usd(&self) -> f64 {
        self.proby.iter().map(|p| p.wynik_usd).sum()
    }
    #[inline]
    pub fn miala_pozycje(&self) -> bool {
        self.proby.iter().any(|p| p.miala_pozycje)
    }
    #[inline]
    pub fn stop_zabral(&self) -> bool {
        self.proby.iter().any(|p| p.stop_zabral)
    }
    /// Kiedy skończyła się NASZA EKSPOZYCJA (0 = nigdy jej nie było).
    pub fn ts_konca_ekspozycji(&self) -> Ts {
        self.proby
            .iter()
            .filter(|p| p.miala_pozycje)
            .map(|p| p.ts_zamkniecia)
            .max()
            .unwrap_or(0)
    }
}

pub fn odcisk_setupu(g: &GeometriaPomyslu, krok: f64) -> u64 {
    let krok = if krok > 0.0 { krok } else { 1.0 };
    let k = |x: f64| (x / krok).round() as i64;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let strona = match g.strona {
        Strona::Buy => 1i64,
        Strona::Sell => -1i64,
    };
    for v in [
        strona,
        k(g.krawedz_dalsza.min(g.krawedz_blizsza)),
        k(g.krawedz_dalsza.max(g.krawedz_blizsza)),
        g.sl.map(k).unwrap_or(i64::MIN),
    ] {
        h ^= v as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod testy {
    use super::*;

    fn geo() -> GeometriaPomyslu {
        GeometriaPomyslu::nowa(
            Strona::Buy,
            4100.0,
            4106.0,
            Some(4094.0),
            vec![4110.0, 4120.0],
        )
    }

    #[test]
    fn etap_jest_monotoniczny() {
        let mut r = Rama::zawiaz(1, "Synergy".into(), 7, geo(), 1, 1000, 1.0);
        r.awansuj(EtapRamy::Zaangazowana, 2000);
        r.awansuj(EtapRamy::Czuwa, 3000); // próba cofnięcia
        assert_eq!(r.etap, EtapRamy::Zaangazowana);
        assert_eq!(r.ts_etapu, 2000);
        r.awansuj(EtapRamy::Zamknieta, 4000);
        assert_eq!(r.etap, EtapRamy::Zamknieta);
    }

    #[test]
    fn okno_zamyka_sie_raz_i_pierwszym_powodem() {
        let mut o = OknoWaznosci::nowe(100);
        o.zamknij(500, PowodKonca::StopCeny);
        o.zamknij(900, PowodKonca::OstatniCel);
        assert_eq!(o.do_ts, Some(500));
        assert_eq!(o.powod, Some(PowodKonca::StopCeny));
        assert_eq!(o.zycie_ms(10_000), 400);
        assert!(o.wazna_o(499));
        assert!(!o.wazna_o(500));
    }

    #[test]
    fn glebokosc_liczy_sie_w_szerokosciach_strefy_dla_obu_stron() {
        let b = geo(); // BUY 4100..4106, bliższa 4106
        assert!((b.glebokosc(4106.0) - 0.0).abs() < 1e-12);
        assert!((b.glebokosc(4100.0) - 1.0).abs() < 1e-12);
        assert!((b.glebokosc(4097.0) - 1.5).abs() < 1e-12); // za dalszą krawędzią
        let s = GeometriaPomyslu::nowa(Strona::Sell, 4100.0, 4106.0, Some(4112.0), vec![4090.0]);
        assert!((s.glebokosc(4100.0) - 0.0).abs() < 1e-12);
        assert!((s.glebokosc(4106.0) - 1.0).abs() < 1e-12);
        assert!((s.glebokosc(4109.0) - 1.5).abs() < 1e-12);
    }

    #[test]
    fn ten_sam_pomysl_daje_ten_sam_odcisk_rozny_inny() {
        let a = odcisk_setupu(&geo(), 1.0);
        let b = odcisk_setupu(
            &GeometriaPomyslu::nowa(Strona::Buy, 4100.2, 4105.8, Some(4094.1), vec![4111.0]),
            1.0,
        );
        assert_eq!(a, b, "różnica groszowa to ten sam setup");
        let c = odcisk_setupu(
            &GeometriaPomyslu::nowa(Strona::Buy, 4090.0, 4096.0, Some(4084.0), vec![]),
            1.0,
        );
        assert_ne!(a, c);
    }

    #[test]
    fn ryzyko_nogi_liczy_od_stopu_pomyslu() {
        let g = geo();
        // 0,02 lota z 4104 do SL 4094 = 10 $ ruchu × 100 × 0,02 = 20 $
        assert!((g.ryzyko_nogi(4104.0, 0.02).unwrap() - 20.0).abs() < 1e-9);
        // noga otwarta PONIŻEJ stopu nie ma ryzyka ujemnego
        assert!((g.ryzyko_nogi(4090.0, 0.02).unwrap() - 0.0).abs() < 1e-9);
    }

    #[test]
    fn dotkniecia_liczone_po_stronie_wyjscia() {
        let g = geo();
        assert!(g.cel_dotkniety(0, 4110.0, 4110.2));
        assert!(!g.cel_dotkniety(0, 4109.9, 4110.1));
        assert!(g.sl_dotkniety(4094.0, 4094.2));
        assert!(!g.sl_dotkniety(4094.1, 4094.3));
    }
}
