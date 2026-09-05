//! PRZEBIEG TICKOWY OBSERWATORA — jedyne miejsce, w którym rama styka się
//! z ceną. Liczy trzy rzeczy naraz, jednym przelotem po tickach:
//!
//!  1. **OKNO WAŻNOŚCI POMYSŁU** — kiedy cena unieważniła pomysł (SL kanału)
//!     albo wykonała go do końca (ostatni cel). Liczone od CHWILI SYGNAŁU
//!     i **niezależnie od tego, czy mieliśmy pozycję** — to jest cała różnica
//!     wobec `sl_touch_ts` silnika, który przestaje mierzyć, gdy koszyk umrze.
//!  2. **MFE/MAE RAMY** — szczyt i dno ŁĄCZNEGO wyniku ramy (zrealizowane +
//!     otwarte), tick po ticku. Suma szczytów pojedynczych pozycji NIE jest
//!     tą wielkością: szczyty padają w różnych chwilach, więc suma jest górnym
//!     oszacowaniem. Oba liczymy i oba pokazujemy.
//!  3. **DRUGIE DOTKNIĘCIE CELU PO STOPIE** — czy rynek doszedł do TP1 już
//!     PO tym, jak stop zabrał nam pozycję. To jest świadek „cena" dla liczby,
//!     której architektura żąda od dwóch niezależnych świadków (E6).
//!
//! Wydajność: zamiast sprawdzać każdy pomysł na każdym ticku (setki żywych
//! ram × dziesiątki milionów ticków) trzymamy cztery kopce progów. Pomysł
//! wypływa dopiero wtedy, gdy cena naprawdę sięgnęła jego poziomu.

use crate::rama::{PowodKonca, Strona, Ts, XAU_CONTRACT};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Kwantyzacja ceny do klucza CAŁKOWITEGO. Kolejność w kopcu nie może zależeć
/// od zaokrąglenia `f64` — to ta sama zasada, co `Zamiar::klucz` bez `f64`.
#[inline]
fn q(px: f64) -> i64 {
    (px * 10_000.0).round() as i64
}

#[derive(Debug, Clone, Copy)]
pub struct PozycjaRamy {
    pub rama: usize,
    pub strona: Strona,
    pub open_ts: Ts,
    pub close_ts: Ts,
    pub open_px: f64,
    pub wolumen: f64,
    pub netto: f64,
}

#[derive(Debug, Clone)]
pub struct PomyslDoObserwacji {
    pub strona: Strona,
    pub ts: Ts,
    pub sl: Option<f64>,
    pub cele: Vec<f64>,
    /// chwila, w której STOP zabrał nam ekspozycję (0 = nie zabrał)
    pub ts_stopu: Ts,
}

#[derive(Debug, Clone, Default)]
pub struct WynikRamy {
    pub mfe_usd: f64,
    pub mae_usd: f64,
    pub ts_mfe: Ts,
    pub ts_mae: Ts,
    /// pierwsze dotknięcie SL POMYSŁU przez cenę (0 = nigdy)
    pub sl_ts: Ts,
    /// pierwsze dotknięcie każdego celu (0 = nigdy)
    pub cel_ts: Vec<Ts>,
    /// pierwsze dotknięcie TP1 PO chwili stopu (0 = nie było)
    pub tp1_po_stopie_ts: Ts,
    /// pierwsze dotknięcie SL POMYSŁU PO chwili stopu
    pub sl_po_stopie_ts: Ts,
    pub tickow_z_ekspozycja: u64,
}

impl WynikRamy {
    /// Koniec okna ważności pomysłu: pierwsze z (SL ceny, ostatni cel,
    /// horyzont). Komenda kanału dochodzi warstwę wyżej, w agregatorze.
    pub fn koniec_okna(
        &self,
        ts_zawiazania: Ts,
        horyzont_ms: i64,
        n_celow: usize,
    ) -> (Ts, PowodKonca) {
        let mut kandydaci: Vec<(Ts, PowodKonca)> = Vec::new();
        if self.sl_ts > 0 {
            kandydaci.push((self.sl_ts, PowodKonca::StopCeny));
        }
        if n_celow > 0 {
            if let Some(t) = self.cel_ts.get(n_celow - 1) {
                if *t > 0 {
                    kandydaci.push((*t, PowodKonca::OstatniCel));
                }
            }
        }
        kandydaci.push((ts_zawiazania + horyzont_ms, PowodKonca::Horyzont));
        kandydaci.sort_by_key(|(t, _)| *t);
        kandydaci[0]
    }
}

/// Wpis kopca. `klucz` jest całkowity, `wersja` służy leniwemu kasowaniu.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Wpis {
    klucz: i64,
    idx: usize,
    /// numer celu (dla kopców celów) albo `usize::MAX` dla SL
    cel: usize,
}

pub struct Obserwator<'a> {
    pomysly: &'a [PomyslDoObserwacji],
    horyzont_ms: i64,
    // kopce progów: BUY dotyka po bid, SELL po ask
    buy_sl: BinaryHeap<Wpis>,          // pop największe sl (bid <= sl)
    buy_tp: BinaryHeap<Reverse<Wpis>>, // pop najmniejsze tp (bid >= tp)
    sell_sl: BinaryHeap<Reverse<Wpis>>,
    sell_tp: BinaryHeap<Wpis>,
    // te same cztery, ale uzbrajane dopiero W CHWILI STOPU
    buy_tp_po: BinaryHeap<Reverse<Wpis>>,
    sell_tp_po: BinaryHeap<Wpis>,
    buy_sl_po: BinaryHeap<Wpis>,
    sell_sl_po: BinaryHeap<Reverse<Wpis>>,
    pub wyniki: Vec<WynikRamy>,
}

impl<'a> Obserwator<'a> {
    pub fn nowy(pomysly: &'a [PomyslDoObserwacji], horyzont_ms: i64) -> Self {
        let wyniki = pomysly
            .iter()
            .map(|p| WynikRamy {
                cel_ts: vec![0; p.cele.len()],
                ..Default::default()
            })
            .collect();
        Obserwator {
            pomysly,
            horyzont_ms,
            buy_sl: BinaryHeap::new(),
            buy_tp: BinaryHeap::new(),
            sell_sl: BinaryHeap::new(),
            sell_tp: BinaryHeap::new(),
            buy_tp_po: BinaryHeap::new(),
            sell_tp_po: BinaryHeap::new(),
            buy_sl_po: BinaryHeap::new(),
            sell_sl_po: BinaryHeap::new(),
            wyniki,
        }
    }

    fn uzbroj(&mut self, i: usize) {
        let p = &self.pomysly[i];
        match p.strona {
            Strona::Buy => {
                if let Some(s) = p.sl {
                    self.buy_sl.push(Wpis {
                        klucz: q(s),
                        idx: i,
                        cel: usize::MAX,
                    });
                }
                if let Some(c) = p.cele.first() {
                    self.buy_tp.push(Reverse(Wpis {
                        klucz: q(*c),
                        idx: i,
                        cel: 0,
                    }));
                }
            }
            Strona::Sell => {
                if let Some(s) = p.sl {
                    self.sell_sl.push(Reverse(Wpis {
                        klucz: q(s),
                        idx: i,
                        cel: usize::MAX,
                    }));
                }
                if let Some(c) = p.cele.first() {
                    self.sell_tp.push(Wpis {
                        klucz: q(*c),
                        idx: i,
                        cel: 0,
                    });
                }
            }
        }
    }

    fn uzbroj_po_stopie(&mut self, i: usize) {
        let p = &self.pomysly[i];
        match p.strona {
            Strona::Buy => {
                if let Some(c) = p.cele.first() {
                    self.buy_tp_po.push(Reverse(Wpis {
                        klucz: q(*c),
                        idx: i,
                        cel: 0,
                    }));
                }
                if let Some(s) = p.sl {
                    self.buy_sl_po.push(Wpis {
                        klucz: q(s),
                        idx: i,
                        cel: usize::MAX,
                    });
                }
            }
            Strona::Sell => {
                if let Some(c) = p.cele.first() {
                    self.sell_tp_po.push(Wpis {
                        klucz: q(*c),
                        idx: i,
                        cel: 0,
                    });
                }
                if let Some(s) = p.sl {
                    self.sell_sl_po.push(Reverse(Wpis {
                        klucz: q(s),
                        idx: i,
                        cel: usize::MAX,
                    }));
                }
            }
        }
    }

    #[inline]
    fn zywy(&self, i: usize, ts: Ts) -> bool {
        let p = &self.pomysly[i];
        ts <= p.ts + self.horyzont_ms && self.wyniki[i].sl_ts == 0
    }

    /// Jeden tick. `bid`/`ask` jak w kwotowaniu.
    pub fn tick(&mut self, ts: Ts, bid: f64, ask: f64) {
        let qb = q(bid);
        let qa = q(ask);

        // --- SL pomysłu ---
        while let Some(w) = self.buy_sl.peek() {
            if w.klucz < qb {
                break;
            }
            let w = self.buy_sl.pop().unwrap();
            if self.zywy(w.idx, ts) && ts >= self.pomysly[w.idx].ts {
                self.wyniki[w.idx].sl_ts = ts;
            }
        }
        while let Some(Reverse(w)) = self.sell_sl.peek() {
            if w.klucz > qa {
                break;
            }
            let Reverse(w) = self.sell_sl.pop().unwrap();
            if self.zywy(w.idx, ts) && ts >= self.pomysly[w.idx].ts {
                self.wyniki[w.idx].sl_ts = ts;
            }
        }

        // --- cele pomysłu (po dotknięciu uzbrajamy następny) ---
        while let Some(Reverse(w)) = self.buy_tp.peek() {
            if w.klucz > qb {
                break;
            }
            let Reverse(w) = self.buy_tp.pop().unwrap();
            if self.zywy(w.idx, ts) && ts >= self.pomysly[w.idx].ts {
                if self.wyniki[w.idx].cel_ts[w.cel] == 0 {
                    self.wyniki[w.idx].cel_ts[w.cel] = ts;
                }
                if let Some(c) = self.pomysly[w.idx].cele.get(w.cel + 1) {
                    self.buy_tp.push(Reverse(Wpis {
                        klucz: q(*c),
                        idx: w.idx,
                        cel: w.cel + 1,
                    }));
                }
            }
        }
        while let Some(w) = self.sell_tp.peek() {
            if w.klucz < qa {
                break;
            }
            let w = self.sell_tp.pop().unwrap();
            if self.zywy(w.idx, ts) && ts >= self.pomysly[w.idx].ts {
                if self.wyniki[w.idx].cel_ts[w.cel] == 0 {
                    self.wyniki[w.idx].cel_ts[w.cel] = ts;
                }
                if let Some(c) = self.pomysly[w.idx].cele.get(w.cel + 1) {
                    self.sell_tp.push(Wpis {
                        klucz: q(*c),
                        idx: w.idx,
                        cel: w.cel + 1,
                    });
                }
            }
        }

        // --- TP1 i SL PO STOPIE (uzbrojone zdarzeniem, nie sygnałem) ---
        while let Some(Reverse(w)) = self.buy_tp_po.peek() {
            if w.klucz > qb {
                break;
            }
            let Reverse(w) = self.buy_tp_po.pop().unwrap();
            if self.wyniki[w.idx].tp1_po_stopie_ts == 0 {
                self.wyniki[w.idx].tp1_po_stopie_ts = ts;
            }
        }
        while let Some(w) = self.sell_tp_po.peek() {
            if w.klucz < qa {
                break;
            }
            let w = self.sell_tp_po.pop().unwrap();
            if self.wyniki[w.idx].tp1_po_stopie_ts == 0 {
                self.wyniki[w.idx].tp1_po_stopie_ts = ts;
            }
        }
        while let Some(w) = self.buy_sl_po.peek() {
            if w.klucz < qb {
                break;
            }
            let w = self.buy_sl_po.pop().unwrap();
            if self.wyniki[w.idx].sl_po_stopie_ts == 0 {
                self.wyniki[w.idx].sl_po_stopie_ts = ts;
            }
        }
        while let Some(Reverse(w)) = self.sell_sl_po.peek() {
            if w.klucz > qa {
                break;
            }
            let Reverse(w) = self.sell_sl_po.pop().unwrap();
            if self.wyniki[w.idx].sl_po_stopie_ts == 0 {
                self.wyniki[w.idx].sl_po_stopie_ts = ts;
            }
        }
    }
}

/// ZDARZENIE OSI CZASU — kolejność jest treścią, więc sortujemy po (ts, rodzaj).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rodzaj {
    Start,
    Otwarcie,
    Zamkniecie,
    Stop,
}

/// PEŁNY PRZELOT. `ticki` dostarcza `(ts, bid, ask)` w kolejności rosnącej.
pub fn przelot<T: FnMut(usize) -> (Ts, f64, f64)>(
    pomysly: &[PomyslDoObserwacji],
    pozycje: &[PozycjaRamy],
    n_tickow: usize,
    mut tick: T,
    horyzont_ms: i64,
) -> Vec<WynikRamy> {
    let mut obs = Obserwator::nowy(pomysly, horyzont_ms);

    let mut zdarzenia: Vec<(Ts, Rodzaj, usize)> =
        Vec::with_capacity(pomysly.len() + pozycje.len() * 2);
    for (i, p) in pomysly.iter().enumerate() {
        zdarzenia.push((p.ts, Rodzaj::Start, i));
        if p.ts_stopu > 0 {
            zdarzenia.push((p.ts_stopu, Rodzaj::Stop, i));
        }
    }
    for (j, p) in pozycje.iter().enumerate() {
        zdarzenia.push((p.open_ts, Rodzaj::Otwarcie, j));
        zdarzenia.push((p.close_ts, Rodzaj::Zamkniecie, j));
    }
    zdarzenia.sort();

    // stan ram z ekspozycją
    let mut zrealizowane: Vec<f64> = vec![0.0; pomysly.len()];
    let mut otwarte: Vec<Vec<usize>> = vec![Vec::new(); pomysly.len()];
    let mut z_ekspozycja: Vec<usize> = Vec::new();

    let mut e = 0usize;
    for i in 0..n_tickow {
        let (ts, bid, ask) = tick(i);

        while e < zdarzenia.len() && zdarzenia[e].0 <= ts {
            let (_, rodzaj, idx) = zdarzenia[e];
            match rodzaj {
                Rodzaj::Start => obs.uzbroj(idx),
                Rodzaj::Stop => obs.uzbroj_po_stopie(idx),
                Rodzaj::Otwarcie => {
                    let r = pozycje[idx].rama;
                    if otwarte[r].is_empty() {
                        z_ekspozycja.push(r);
                    }
                    otwarte[r].push(idx);
                }
                Rodzaj::Zamkniecie => {
                    let r = pozycje[idx].rama;
                    otwarte[r].retain(|x| *x != idx);
                    zrealizowane[r] += pozycje[idx].netto;
                    if otwarte[r].is_empty() {
                        z_ekspozycja.retain(|x| *x != r);
                        let pl = zrealizowane[r];
                        let w = &mut obs.wyniki[r];
                        if pl > w.mfe_usd {
                            w.mfe_usd = pl;
                            w.ts_mfe = ts;
                        }
                        if pl < w.mae_usd {
                            w.mae_usd = pl;
                            w.ts_mae = ts;
                        }
                    }
                }
            }
            e += 1;
        }

        obs.tick(ts, bid, ask);

        for &r in z_ekspozycja.iter() {
            let mut pl = zrealizowane[r];
            for &j in otwarte[r].iter() {
                let p = &pozycje[j];
                let px = p.strona.wyjscie(bid, ask);
                pl += (px - p.open_px) * p.strona.znak() * XAU_CONTRACT * p.wolumen;
            }
            let w = &mut obs.wyniki[r];
            w.tickow_z_ekspozycja += 1;
            if pl > w.mfe_usd {
                w.mfe_usd = pl;
                w.ts_mfe = ts;
            }
            if pl < w.mae_usd {
                w.mae_usd = pl;
                w.ts_mae = ts;
            }
        }
    }
    obs.wyniki
}

#[cfg(test)]
mod testy {
    use super::*;

    fn pomysl(strona: Strona, ts: Ts, sl: f64, cele: Vec<f64>, stop: Ts) -> PomyslDoObserwacji {
        PomyslDoObserwacji {
            strona,
            ts,
            sl: Some(sl),
            cele,
            ts_stopu: stop,
        }
    }

    /// Szczyt RAMY to nie suma szczytów pozycji: dwie nogi mogą mieć szczyty
    /// w różnych chwilach, a rama ma jeden.
    #[test]
    fn mfe_ramy_liczone_na_sumie_a_nie_po_nogach() {
        let pomysly = vec![pomysl(Strona::Buy, 0, 3990.0, vec![4100.0], 0)];
        let poz = vec![
            PozycjaRamy {
                rama: 0,
                strona: Strona::Buy,
                open_ts: 0,
                close_ts: 40,
                open_px: 4000.0,
                wolumen: 0.01,
                netto: 0.0,
            },
            PozycjaRamy {
                rama: 0,
                strona: Strona::Buy,
                open_ts: 0,
                close_ts: 40,
                open_px: 4000.0,
                wolumen: 0.01,
                netto: 0.0,
            },
        ];
        // ceny: 4000 → 4010 → 4000 → 4005
        let seria = [(0i64, 4000.0), (10, 4010.0), (20, 4000.0), (30, 4005.0)];
        let w = przelot(
            &pomysly,
            &poz,
            seria.len(),
            |i| (seria[i].0, seria[i].1, seria[i].1 + 0.2),
            86_400_000,
        );
        // szczyt = 10 $ ruchu × 100 × 0,02 lota = 20 $
        assert!((w[0].mfe_usd - 20.0).abs() < 1e-9, "mfe = {}", w[0].mfe_usd);
        assert!(w[0].mae_usd.abs() < 1e-9);
    }

    /// Cena dotyka celu PO stopie — to jest świadek „cena" dla ram żywych
    /// po stopie. Pierwsze dotknięcie SPRZED stopu nie liczy się do tej liczby.
    #[test]
    fn cel_po_stopie_liczy_sie_osobno_od_pierwszego_dotkniecia() {
        let pomysly = vec![pomysl(Strona::Buy, 0, 3990.0, vec![4010.0], 25)];
        let seria = [
            (0i64, 4000.0),
            (10, 4010.0), // pierwsze dotknięcie TP1 — PRZED stopem
            (20, 3995.0),
            (30, 4009.0),
            (40, 4011.0), // dotknięcie PO stopie
        ];
        let w = przelot(
            &pomysly,
            &[],
            seria.len(),
            |i| (seria[i].0, seria[i].1, seria[i].1 + 0.2),
            86_400_000,
        );
        assert_eq!(w[0].cel_ts[0], 10);
        assert_eq!(w[0].tp1_po_stopie_ts, 40);
        assert_eq!(w[0].sl_ts, 0, "SL pomysłu nie został dotknięty");
    }

    /// Pomysł unieważniony ceną ma zamknięte okno — i od tej chwili nie
    /// zbiera już dotknięć celów.
    #[test]
    fn sl_pomyslu_zamyka_okno_i_wycisza_cele() {
        let pomysly = vec![pomysl(Strona::Sell, 0, 4010.0, vec![3990.0], 0)];
        let seria = [(0i64, 4000.0), (10, 4012.0), (20, 3985.0)];
        let w = przelot(
            &pomysly,
            &[],
            seria.len(),
            |i| (seria[i].0, seria[i].1 - 0.2, seria[i].1),
            86_400_000,
        );
        assert_eq!(w[0].sl_ts, 10);
        assert_eq!(
            w[0].cel_ts[0], 0,
            "po SL pomysł jest martwy, cele go nie wskrzeszają"
        );
        let (koniec, powod) = w[0].koniec_okna(0, 86_400_000, 1);
        assert_eq!((koniec, powod), (10, PowodKonca::StopCeny));
    }

    /// Horyzont jest CENZURĄ, nie rozstrzygnięciem — i musi być widoczny.
    #[test]
    fn horyzont_konczy_okno_gdy_nic_sie_nie_stalo() {
        let pomysly = vec![pomysl(Strona::Buy, 0, 3000.0, vec![9000.0], 0)];
        let seria = [(0i64, 4000.0), (100, 4001.0)];
        let w = przelot(
            &pomysly,
            &[],
            seria.len(),
            |i| (seria[i].0, seria[i].1, seria[i].1 + 0.2),
            500,
        );
        let (koniec, powod) = w[0].koniec_okna(0, 500, 1);
        assert_eq!((koniec, powod), (500, PowodKonca::Horyzont));
    }
}
