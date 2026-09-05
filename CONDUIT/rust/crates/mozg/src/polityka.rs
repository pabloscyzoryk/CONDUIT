
use crate::wejscie::{Koszyk, Szczebel, Wejscie};

/// Co mózg chce zrobić. Jeden zamiar = jedna zmiana w świecie.
#[derive(Debug, Clone, PartialEq)]
pub enum Zamiar {
    /// nic nie rób — jawnie, żeby „brak zamiaru" dało się policzyć
    Trzymaj,
    /// zamknij pozycję w całości
    Zamknij { ticket: u64, powod: Powod },
    /// zainkasuj część pozycji
    ZamknijCzesc {
        ticket: u64,
        wolumen: f64,
        powod: Powod,
    },
    /// przesuń stop
    PrzesunStop { ticket: u64, na: f64, powod: Powod },
    /// przesuń cel
    PrzesunCel {
        ticket: u64,
        na: Option<f64>,
        powod: Powod,
    },
    /// anuluj zlecenia oczekujące koszyka
    AnulujOczekujace { koszyk: u32, powod: Powod },
}

/// Powód decyzji. Bez niego dziennik jest listą zdarzeń, a nie wyjaśnieniem —
/// a po przebiegu musi dać się policzyć, ILE RAZY i DLACZEGO mózg coś zrobił.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Powod {
    /// punkt domyślny domeny — zachowanie dzisiejszego silnika
    Domyslny,
    /// kanon RISK FREE kanału: inkasuj płytkie, zostaw najlepszy
    InkasoRiskFree,
    /// ochrona ocalałego po inkasie
    ZabezpieczOcalalego,
    /// ruch się wyczerpał — oddane zbyt wiele ze szczytu
    WyczerpanyRuch,
    /// margines schodzi do progu
    ObronaMarginesu,
    /// zbliża się zamknięcie sesji, a luka otwarcia nie jest do udźwignięcia
    RyzykoLuki,
}

/// Granice, wewnątrz których polityka wybiera punkt.
///
/// Puste (`None`) znaczy „ten aktuator jest dziś poza zasięgiem mózgu" — a nie
/// „rób co chcesz". Domyślnie wszystko jest poza zasięgiem, więc dołożenie
/// nowej zdolności wymaga JAWNEGO otwarcia domeny.
#[derive(Debug, Clone, Default)]
pub struct Domeny {
    /// wolno inkasować część pozycji (ułamek wolumenu w tych granicach)
    pub inkaso: Option<(f64, f64)>,
    /// wolno przesuwać stop, ale tylko w stronę bezpieczniejszą
    pub stop_tylko_ciasniej: bool,
    /// wolno zamykać pozycje
    pub zamykanie: bool,
    pub prog_modyfikacji_usd: f64,
    pub sufit_interwencji: u16,
}

impl Domeny {
    /// Domeny etapu Cień: wszystko zamknięte. Mózg może liczyć, nie może nic.
    pub fn cien() -> Self {
        Domeny {
            sufit_interwencji: u16::MAX,
            ..Default::default()
        }
    }
}

/// Polityka — jedyne miejsce, w którym zapada decyzja.
pub trait Polityka {
    fn decyduj(&self, we: &Wejscie, dom: &Domeny) -> Vec<Zamiar>;
    fn nazwa(&self) -> &'static str;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PolitykaZero;

impl Polityka for PolitykaZero {
    fn decyduj(&self, _we: &Wejscie, _dom: &Domeny) -> Vec<Zamiar> {
        vec![Zamiar::Trzymaj]
    }
    fn nazwa(&self) -> &'static str {
        "zero"
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PolitykaKanon {
    /// przy jakim zysku koszyka (w dolarach) uruchamia się inkaso
    pub prog_inkasa_usd: f64,
    /// zapas nad wejściem przy zabezpieczaniu ocalałego
    pub zapas_be: f64,
}

impl Default for PolitykaKanon {
    fn default() -> Self {
        PolitykaKanon {
            prog_inkasa_usd: 0.0,
            zapas_be: 0.0,
        }
    }
}

impl PolitykaKanon {
    fn dla_koszyka(&self, k: &Koszyk, dom: &Domeny) -> Vec<Zamiar> {
        let mut z = Vec::new();
        if !dom.zamykanie {
            return vec![Zamiar::Trzymaj];
        }
        // Warunek uruchomienia: koszyk JAKO CAŁOŚĆ jest na plusie powyżej
        // progu. Inkasowanie z koszyka pod wodą byłoby realizacją straty na
        // płytkich nogach i zostawieniem samej stratnej głębi.
        let wynik = k.wynik_usd();
        if wynik <= self.prog_inkasa_usd {
            return vec![Zamiar::Trzymaj];
        }
        let najlepszy: Option<u64> = k.najlepszy().map(|s| s.ticket);
        for s in k.do_inkasa() {
            z.push(Zamiar::Zamknij {
                ticket: s.ticket,
                powod: Powod::InkasoRiskFree,
            });
        }
        // Ocalały dostaje stop na wejściu — ale TYLKO jeśli broker to przyjmie.
        // Bez tego warunku polityka produkowałaby zamiary, które giną w ciszy
        // jako odmowy, a dziennik pokazywałby decyzje, których rynek nie widział.
        if let (Some(t), Some(s)) = (najlepszy, k.najlepszy()) {
            if s.wynik_usd >= dom.prog_modyfikacji_usd && !s.zabezpieczony(self.zapas_be) {
                let na = poziom_be(s, self.zapas_be);
                z.push(Zamiar::PrzesunStop {
                    ticket: t,
                    na,
                    powod: Powod::ZabezpieczOcalalego,
                });
            }
        }
        if z.is_empty() {
            z.push(Zamiar::Trzymaj);
        }
        z
    }
}

/// Poziom stopu „na wejściu z zapasem", po właściwej stronie dla danej strony.
#[inline]
fn poziom_be(s: &Szczebel, zapas: f64) -> f64 {
    s.cena_wejscia + s.strona.znak() * zapas
}

impl Polityka for PolitykaKanon {
    fn decyduj(&self, we: &Wejscie, dom: &Domeny) -> Vec<Zamiar> {
        match we.biezacy() {
            Some(k) => self.dla_koszyka(k, dom),
            None => vec![Zamiar::Trzymaj],
        }
    }
    fn nazwa(&self) -> &'static str {
        "kanon"
    }
}
