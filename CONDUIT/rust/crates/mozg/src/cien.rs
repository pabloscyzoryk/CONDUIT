
use crate::polityka::{Domeny, Polityka, Powod, Zamiar};
use crate::wejscie::Wejscie;

/// Licznik jednego rodzaju zamiaru.
#[derive(Debug, Clone, Copy, Default)]
pub struct Licznik {
    pub ile: u64,
    /// suma wyniku koszyków, w których ten zamiar padł — do oceny wagi
    pub wynik_koszykow_usd: f64,
}

impl Licznik {
    #[inline]
    fn dolóż(&mut self, wynik: f64) {
        self.ile += 1;
        self.wynik_koszykow_usd += wynik;
    }
}

/// Zbiór obserwacji z całego przebiegu.
#[derive(Debug, Clone, Default)]
pub struct Obserwacje {
    pub pulsow: u64,
    pub pulsow_z_koszykiem: u64,
    pub trzymaj: u64,
    pub zamknij: Licznik,
    pub zamknij_czesc: Licznik,
    pub przesun_stop: Licznik,
    pub przesun_cel: Licznik,
    pub anuluj: Licznik,
    /// rozbicie po powodzie — bez tego wiadomo ILE, ale nie WIDAĆ DLACZEGO
    pub powody: [u64; 6],
    /// największa liczba zamiarów, jaka padła w jednym pulsie
    pub max_zamiarow_na_puls: usize,
    /// pulsy, w których mózg chciał czegokolwiek poza „trzymaj"
    pub pulsow_z_dzialaniem: u64,
}

#[inline]
fn indeks(p: Powod) -> usize {
    match p {
        Powod::Domyslny => 0,
        Powod::InkasoRiskFree => 1,
        Powod::ZabezpieczOcalalego => 2,
        Powod::WyczerpanyRuch => 3,
        Powod::ObronaMarginesu => 4,
        Powod::RyzykoLuki => 5,
    }
}

pub const NAZWY_POWODOW: [&str; 6] = [
    "domyślny",
    "inkaso RISK FREE",
    "zabezpiecz ocalałego",
    "wyczerpany ruch",
    "obrona marginesu",
    "ryzyko luki",
];

impl Obserwacje {
    /// Jeden puls: policz, co polityka chciałaby zrobić. Nic nie wykonuje.
    pub fn puls<P: Polityka>(&mut self, p: &P, we: &Wejscie, dom: &Domeny) {
        self.pulsow += 1;
        let wynik = match we.biezacy() {
            Some(k) => {
                self.pulsow_z_koszykiem += 1;
                k.wynik_usd()
            }
            None => 0.0,
        };
        let z = p.decyduj(we, dom);
        if z.len() > self.max_zamiarow_na_puls {
            self.max_zamiarow_na_puls = z.len();
        }
        let mut dzialal = false;
        for x in &z {
            match x {
                Zamiar::Trzymaj => self.trzymaj += 1,
                Zamiar::Zamknij { powod, .. } => {
                    self.zamknij.dolóż(wynik);
                    self.powody[indeks(*powod)] += 1;
                    dzialal = true;
                }
                Zamiar::ZamknijCzesc { powod, .. } => {
                    self.zamknij_czesc.dolóż(wynik);
                    self.powody[indeks(*powod)] += 1;
                    dzialal = true;
                }
                Zamiar::PrzesunStop { powod, .. } => {
                    self.przesun_stop.dolóż(wynik);
                    self.powody[indeks(*powod)] += 1;
                    dzialal = true;
                }
                Zamiar::PrzesunCel { powod, .. } => {
                    self.przesun_cel.dolóż(wynik);
                    self.powody[indeks(*powod)] += 1;
                    dzialal = true;
                }
                Zamiar::AnulujOczekujace { powod, .. } => {
                    self.anuluj.dolóż(wynik);
                    self.powody[indeks(*powod)] += 1;
                    dzialal = true;
                }
            }
        }
        if dzialal {
            self.pulsow_z_dzialaniem += 1;
        }
    }

    pub fn raport(&self, polityka: &str) -> String {
        let mut s = String::new();
        s.push_str(&format!("# CIEŃ MÓZGU — polityka: {polityka}\n\n"));
        s.push_str(&format!("pulsów                : {}\n", self.pulsow));
        s.push_str(&format!(
            "  w tym z koszykiem   : {}\n",
            self.pulsow_z_koszykiem
        ));
        s.push_str(&format!(
            "  z DZIAŁANIEM        : {}\n",
            self.pulsow_z_dzialaniem
        ));
        s.push_str(&format!("  samo trzymaj        : {}\n", self.trzymaj));
        s.push_str(&format!(
            "max zamiarów na puls  : {}\n\n",
            self.max_zamiarow_na_puls
        ));
        s.push_str("ZAMIARY\n");
        for (n, l) in [
            ("zamknij", self.zamknij),
            ("zamknij część", self.zamknij_czesc),
            ("przesuń stop", self.przesun_stop),
            ("przesuń cel", self.przesun_cel),
            ("anuluj oczekujące", self.anuluj),
        ] {
            if l.ile > 0 {
                s.push_str(&format!(
                    "  {n:<20} {:>8}   średni wynik koszyka {:>10.2} $\n",
                    l.ile,
                    l.wynik_koszykow_usd / l.ile as f64
                ));
            }
        }
        s.push_str("\nPOWODY\n");
        for (i, n) in NAZWY_POWODOW.iter().enumerate() {
            if self.powody[i] > 0 {
                s.push_str(&format!("  {n:<24} {:>8}\n", self.powody[i]));
            }
        }
        s
    }
}
