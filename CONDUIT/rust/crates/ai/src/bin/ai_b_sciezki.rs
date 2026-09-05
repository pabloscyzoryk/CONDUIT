//! AI-B, etap 1 — **szkielet ceny**: bezstratna (co do $0,05) kompresja ticków.
//!
//! Po co: eksperyment AI-B ocenia wiele wariantów zarządzania na dużym
//! strumieniu ticków. Kompresja ogranicza liczbę kroków z jawną granicą błędu,
//! zamiast dawać jedynie przybliżoną obietnicę szybkości.
//!
//! GWARANCJA (to jest sedno, nie optymalizacja):
//! zachowujemy tick, gdy jego bid odbiega od ostatniego zachowanego o ≥ `EPS`,
//! gdy spread zmienił się o ≥ `SPREAD_EPS`, albo gdy minęło ≥ `MAX_DT_MS`.
//! Wynika stąd, że **cała pominięta ścieżka leży w paśmie ±EPS wokół ostatniego
//! zachowanego punktu** — bo pierwszy tick, który z tego pasma wychodzi, jest
//! zachowywany. Zatem:
//!   * poziom oddalony od szkieletu o > EPS NIE ZOSTAŁ dotknięty,
//!   * poziom dotknięty jest odwzorowany z błędem ≤ EPS,
//!   * dwa poziomy odległe o > 2·EPS nie mogą zamienić się kolejnością.
//! Wszystkie dystanse zarządzania w AI-B są ≥ 0,20 $ (minimum brokera), czyli
//! ≥ 4·EPS. Kolejność SL/TP jest więc odtworzona wiernie.
//!
//! Format wyjścia = ten sam CDTK, co wejście — czyta go `TickData` bez zmian.

use anyhow::{bail, Result};
use conduit_backtest::data::TickData;
use std::io::Write;

/// Maksymalny błąd odwzorowania poziomu ceny, w dolarach.
const EPS: f64 = 0.05;
/// Wymuszony punkt co tę liczbę milisekund (żeby oś czasu nie miała dziur).
const MAX_DT_MS: i64 = 60_000;
/// Zmiana spreadu, przy której zawsze zapisujemy punkt.
const SPREAD_EPS: f64 = 0.05;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let wej = a
        .get(1)
        .cloned()
        .unwrap_or_else(|| "data/ticks.bin".to_string());
    let wyj = a
        .get(2)
        .cloned()
        .unwrap_or_else(|| "ai_b/szkielet.bin".to_string());

    let t = TickData::open(&wej)?;
    let n = t.len();
    if n == 0 {
        bail!("pusty plik ticków");
    }
    eprintln!("wejście {wej}: {n} ticków");

    // nagłówek kopiujemy z oryginału i podmieniamy licznik
    let naglowek = {
        let mut b = std::fs::read(&wej)?;
        b.truncate(64);
        b
    };

    let mut buf: Vec<u8> = Vec::with_capacity(64 + n / 8 * 16);
    buf.extend_from_slice(&naglowek);

    let mut zapisz = |buf: &mut Vec<u8>, ts: i64, bid: f64, ask: f64| {
        buf.extend_from_slice(&ts.to_le_bytes());
        buf.extend_from_slice(&(bid as f32).to_le_bytes());
        buf.extend_from_slice(&(ask as f32).to_le_bytes());
    };

    let (mut ost_ts, mut ost_bid, mut ost_spr) = (t.ts(0), t.bid(0), t.ask(0) - t.bid(0));
    zapisz(&mut buf, t.ts(0), t.bid(0), t.ask(0));
    let mut ile = 1usize;
    // KONTROLA GWARANCJI, liczona w tym samym przebiegu: maksymalne odchylenie
    // POMINIĘTEGO ticka od ostatniego zachowanego punktu. Osobny przebieg
    // porównujący po znacznikach czasu jest do tego bezużyteczny, bo w danych
    // są ticki o TYM SAMYM znaczniku milisekundowym i wyszukiwanie „ostatni
    // punkt o ts ≤ ts_i" trafia wtedy w tick PÓŹNIEJSZY w kolejności.
    let mut maks_pominiete = 0.0f64;

    for i in 1..n {
        let ts = t.ts(i);
        let bid = t.bid(i);
        let ask = t.ask(i);
        let spr = ask - bid;
        let trzeba = (bid - ost_bid).abs() >= EPS
            || (spr - ost_spr).abs() >= SPREAD_EPS
            || ts - ost_ts >= MAX_DT_MS
            || i == n - 1;
        if trzeba {
            zapisz(&mut buf, ts, bid, ask);
            ile += 1;
            ost_ts = ts;
            ost_bid = bid;
            ost_spr = spr;
        } else {
            let d = (bid - ost_bid).abs();
            if d > maks_pominiete {
                maks_pominiete = d;
            }
        }
    }

    buf[8..16].copy_from_slice(&(ile as u64).to_le_bytes());
    if let Some(k) = std::path::Path::new(&wyj).parent() {
        std::fs::create_dir_all(k)?;
    }
    let mut f = std::fs::File::create(&wyj)?;
    f.write_all(&buf)?;
    f.flush()?;

    eprintln!(
        "szkielet {wyj}: {ile} punktów ({:.2} % oryginału), {:.1} MB",
        100.0 * ile as f64 / n as f64,
        buf.len() as f64 / 1e6
    );

    eprintln!(
        "kontrola gwarancji: maks odchylenie POMINIĘTEGO ticka = {maks_pominiete:.4} $ (próg {EPS})"
    );
    if maks_pominiete >= EPS {
        bail!("gwarancja EPS złamana: {maks_pominiete}");
    }
    let s = TickData::open(&wyj)?;
    eprintln!(
        "odczyt zwrotny: {} punktów, {} … {}",
        s.len(),
        s.first_ts(),
        s.last_ts()
    );
    Ok(())
}
