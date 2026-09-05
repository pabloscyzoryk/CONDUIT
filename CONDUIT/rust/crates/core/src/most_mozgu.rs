
use std::sync::OnceLock;

use conduit_mozg::oczy::Oczy;
use conduit_mozg::polityka::{Domeny, Zamiar};
use conduit_mozg::rama::{GeometriaPomyslu, Strona};
use conduit_mozg::wejscie::{Koszyk, Rachunek, Szczebel, Wejscie};

use crate::broker::Broker;
use crate::types::{Basket, Position, Side, Ts};

/// Tryb pracy mózgu, czytany raz ze zmiennej `MOZG`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrybMozgu {
    /// mózg w ogóle nie istnieje — ścieżka parytetu
    Wylaczony,
    /// liczy decyzje, NIE wykonuje ich
    Cien,
    /// wykonuje w granicach otwartych domen
    Wykonuj,
}

static TRYB: OnceLock<TrybMozgu> = OnceLock::new();

/// Jeden odczyt na proces. Wzorzec identyczny jak `KRAWEDZ_DIAG` i `MOZG_CIEN`
/// — bo trzy różne sposoby włączania diagnostyki to trzy sposoby pomylenia się.
#[inline]
pub fn tryb() -> TrybMozgu {
    *TRYB.get_or_init(|| match std::env::var("MOZG").ok().as_deref() {
        Some("cien") => TrybMozgu::Cien,
        Some("wykonuj") => TrybMozgu::Wykonuj,
        _ => TrybMozgu::Wylaczony,
    })
}

#[inline]
fn strona(s: Side) -> Strona {
    match s {
        Side::Buy => Strona::Buy,
        Side::Sell => Strona::Sell,
    }
}

/// GŁĘBOKOŚĆ szczebla: 0 = najpłytszy, czyli o NAJGORSZEJ cenie wejścia.
///
/// Dla BUY najgorsza cena to najwyższa, dla SELL najniższa. To rozróżnienie
/// jest tu, a nie w mózgu, bo wymaga wiedzy o stronie rynku — a mózg dostaje
/// głębokość już policzoną, żeby żadna reguła nie musiała jej wyprowadzać
/// z ceny po swojemu.
fn glebokosci(poz: &[&Position], side: Side) -> Vec<u16> {
    let mut idx: Vec<usize> = (0..poz.len()).collect();
    idx.sort_by(|&a, &b| {
        let (x, y) = (poz[a].open_price, poz[b].open_price);
        match side {
            Side::Buy => y.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal),
            Side::Sell => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        }
    });
    let mut g = vec![0u16; poz.len()];
    for (r, &i) in idx.iter().enumerate() {
        g[i] = r as u16;
    }
    g
}

/// Buduje koszyk mózgu z koszyka silnika i pozycji brokera.
fn koszyk_z_silnika<B: Broker>(bk: &Basket, b: &B) -> Koszyk {
    let q = b.quote();
    let poz: Vec<&Position> = b
        .positions()
        .iter()
        .filter(|p| p.basket == Some(bk.id) && !p.frozen)
        .collect();
    let g = glebokosci(&poz, bk.side);
    let szczeble = poz
        .iter()
        .enumerate()
        .map(|(i, p)| {
            // Wynik liczony TĄ SAMĄ arytmetyką co silnik: ruch ceny razy
            // wielkość kontraktu razy loty. 0,01 lota XAUUSD to JEDNA uncja.
            let ruch = (q.exit(p.side) - p.open_price) * p.side.sign();
            let wynik = ruch * crate::types::XAU_CONTRACT * p.volume;
            let szczyt = p.peak_pts * crate::types::XAU_CONTRACT * p.volume;
            Szczebel {
                ticket: p.ticket as u64,
                strona: strona(p.side),
                cena_wejscia: p.open_price,
                wolumen: p.volume,
                sl: p.sl,
                tp: p.tp,
                glebokosc: g[i],
                ts_otwarcia: p.open_ts,
                wynik_usd: wynik,
                szczyt_usd: szczyt.max(0.0),
                dno_usd: f64::NAN,
            }
        })
        .collect::<Vec<_>>();
    let oczekujacych = b
        .pendings()
        .iter()
        .filter(|p| p.basket == Some(bk.id))
        .count() as u16;
    Koszyk {
        id: bk.id,
        rama_id: bk.id,
        geometria: GeometriaPomyslu {
            strona: strona(bk.side),
            krawedz_blizsza: bk.entry_hi,
            krawedz_dalsza: bk.entry_lo,
            sl: bk.sl,
            cele: bk.tps.clone(),
        },
        ts_zawiazania: bk.created_ts,
        szczeble,
        oczekujacych,
        etap_celu: bk.tp_stage.min(u8::MAX as usize) as u8,
        rf_ogloszony: bk.secured,
        budzet_wydany_usd: 0.0,
    }
}

/// Stan rachunku widziany przez mózg.
fn rachunek_z_brokera<B: Broker>(b: &B, poziom: Option<f64>, seria: u16, dzien: f64) -> Rachunek {
    let a = b.account();
    Rachunek {
        saldo: a.balance,
        equity: a.equity,
        margines_uzyty: a.margin,
        margines_wolny: a.free_margin,
        poziom_marginesu: poziom,
        dzwignia: a.leverage as f64,
        wynik_dnia_usd: dzien,
        seria_stopow: seria,
    }
}

/// Komplet koszyków mózgu — buduje się RAZ na puls, nie raz na regułę.
pub fn koszyki_z_silnika<B: Broker>(baskets: &[Basket], b: &B) -> Vec<Koszyk> {
    baskets.iter().map(|bk| koszyk_z_silnika(bk, b)).collect()
}

/// Wejście dla decyzji o JEDNYM koszyku.
pub fn wejscie<'a, B: Broker>(
    ts: Ts,
    oczy: &'a Oczy,
    b: &B,
    koszyki: &'a [Koszyk],
    ktory: Option<usize>,
    poziom_marginesu: Option<f64>,
    seria_stopow: u16,
    wynik_dnia: f64,
) -> Wejscie<'a> {
    Wejscie {
        ts,
        oczy,
        rachunek: rachunek_z_brokera(b, poziom_marginesu, seria_stopow, wynik_dnia),
        koszyki,
        koszyk: ktory,
    }
}

pub fn domeny<B: Broker>(b: &B, wolumen_odniesienia: f64) -> Domeny {
    // `stops_level` jest już w JEDNOSTKACH CENY — silnik używa go wprost
    // (`sl_is_valid`: `sl <= q.bid - stops_level`). Przeliczanie go przez
    // rozmiar punktu dałoby próg sto razy za mały i polityka produkowałaby
    // zamiary, które broker cicho odrzuca.
    let prog_usd = b.stops_level() * crate::types::XAU_CONTRACT * wolumen_odniesienia.max(0.01);
    match tryb() {
        TrybMozgu::Wykonuj => Domeny {
            inkaso: Some((0.0, 1.0)),
            stop_tylko_ciasniej: true,
            zamykanie: true,
            prog_modyfikacji_usd: prog_usd,
            sufit_interwencji: 24,
        },
        // W cieniu domeny są OTWARTE, żeby policzyć, co mózg BY zrobił.
        // Kontrakt zera trzyma się i tak, bo w tym trybie ani jeden zamiar
        // nie idzie do egzekutora — z konstrukcji, nie z wartości pól.
        TrybMozgu::Cien => Domeny {
            inkaso: Some((0.0, 1.0)),
            stop_tylko_ciasniej: true,
            zamykanie: true,
            prog_modyfikacji_usd: prog_usd,
            sufit_interwencji: u16::MAX,
        },
        TrybMozgu::Wylaczony => Domeny::default(),
    }
}

pub fn wykonaj<B: Broker>(z: &Zamiar, b: &mut B) -> bool {
    use crate::types::CloseReason;
    match z {
        Zamiar::Trzymaj => true,
        Zamiar::Zamknij { ticket, .. } => b
            .close_position(*ticket as crate::types::Ticket, CloseReason::Manual)
            .is_ok(),
        Zamiar::ZamknijCzesc {
            ticket, wolumen, ..
        } => b
            .close_partial(
                *ticket as crate::types::Ticket,
                *wolumen,
                CloseReason::Manual,
            )
            .is_ok(),
        Zamiar::PrzesunStop { ticket, na, .. } => {
            let t = *ticket as crate::types::Ticket;
            let tp = b.find_position(t).and_then(|p| p.tp);
            b.modify_position(t, Some(*na), tp).is_ok()
        }
        Zamiar::PrzesunCel { ticket, na, .. } => {
            let t = *ticket as crate::types::Ticket;
            let sl = b.find_position(t).and_then(|p| p.sl);
            b.modify_position(t, sl, *na).is_ok()
        }
        Zamiar::AnulujOczekujace { koszyk, .. } => {
            let tickety: Vec<_> = b
                .pendings()
                .iter()
                .filter(|p| p.basket == Some(*koszyk))
                .map(|p| p.ticket)
                .collect();
            tickety.into_iter().all(|t| b.cancel_pending(t).is_ok())
        }
    }
}
