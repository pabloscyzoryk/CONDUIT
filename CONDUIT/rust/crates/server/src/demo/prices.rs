
use conduit_core::types::{Px, Quote, Ts};

/// Źródło kwotowań dla pętli odtwarzania.
pub trait Feed: Send {
    /// Kolejne kwotowanie w kolejności czasu. `None` = koniec danych.
    fn next(&mut self) -> Option<Quote>;
    /// Znacznik następnego kwotowania — pętla potrzebuje go, żeby wiedzieć,
    /// jak długo spać, ZANIM je pobierze.
    fn peek_ts(&self) -> Option<Ts>;
    fn total(&self) -> u64;
    fn done(&self) -> u64;
    fn first_ts(&self) -> Ts;
    fn last_ts(&self) -> Ts;
}

// ============================================================
//  PLIK TICKÓW
// ============================================================

pub struct FileFeed {
    data: conduit_backtest::TickData,
    i: usize,
    i0: usize,
    i1: usize,
}

impl FileFeed {
    /// Otwiera plik i zawęża go do okna `[from, to)` w zegarze ticków.
    /// `to = 0` oznacza „do końca pliku".
    pub fn open(path: &std::path::Path, from: Ts, to: Ts) -> anyhow::Result<Self> {
        let data = conduit_backtest::TickData::open(path)?;
        if data.is_empty() {
            anyhow::bail!("plik ticków jest pusty");
        }
        let i0 = if from > 0 { data.index_at(from) } else { 0 };
        let i1 = if to > 0 {
            data.index_at(to).min(data.len())
        } else {
            data.len()
        };
        if i1 <= i0 {
            anyhow::bail!(
                "wybrany zakres dat nie zawiera ani jednego ticka (plik obejmuje {} … {})",
                crate::lab::dzien(data.first_ts()),
                crate::lab::dzien(data.last_ts())
            );
        }
        Ok(FileFeed {
            data,
            i: i0,
            i0,
            i1,
        })
    }
}

impl Feed for FileFeed {
    fn next(&mut self) -> Option<Quote> {
        if self.i >= self.i1 {
            return None;
        }
        let q = self.data.quote(self.i);
        self.i += 1;
        Some(q)
    }
    fn peek_ts(&self) -> Option<Ts> {
        if self.i >= self.i1 {
            None
        } else {
            Some(self.data.ts(self.i))
        }
    }
    fn total(&self) -> u64 {
        (self.i1 - self.i0) as u64
    }
    fn done(&self) -> u64 {
        (self.i - self.i0) as u64
    }
    fn first_ts(&self) -> Ts {
        self.data.ts(self.i0)
    }
    fn last_ts(&self) -> Ts {
        self.data.ts(self.i1 - 1)
    }
}

// ============================================================
//  GENERATOR
// ============================================================

/// Parametry błądzenia losowego.
#[derive(Debug, Clone, Copy)]
pub struct SynthCfg {
    pub seed: u64,
    pub start_price: Px,
    /// odchylenie standardowe w dolarach na PIERWIASTEK SEKUNDY.
    /// 0,15 $/√s odpowiada dobowej zmienności ok. 44 $ przy złocie ~4100 $,
    /// czyli mniej więcej temu, co widać w archiwum.
    pub vol: f64,
    /// stały spread w dolarach (mediana 102 mln ticków XAUUSD: 0,24 $)
    pub spread: f64,
    /// odstęp między kwotowaniami w ms
    pub interval_ms: i64,
    pub start_ts: Ts,
    /// koniec zakresu; 0 = generuj bez końca
    pub end_ts: Ts,
}

impl Default for SynthCfg {
    fn default() -> Self {
        SynthCfg {
            seed: 1,
            start_price: 4118.0,
            vol: 0.15,
            spread: 0.24,
            interval_ms: 250,
            start_ts: 0,
            end_ts: 0,
        }
    }
}

pub struct SynthFeed {
    cfg: SynthCfg,
    stan: u64,
    cena: Px,
    ts: Ts,
    n: u64,
}

impl SynthFeed {
    pub fn new(cfg: SynthCfg) -> Self {
        SynthFeed {
            stan: cfg
                .seed
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .wrapping_add(0x1234_5678_9ABC_DEF),
            cena: cfg.start_price,
            ts: cfg.start_ts,
            n: 0,
            cfg,
        }
    }

    /// splitmix64 — deterministyczny, bez zależności, dobrze wymieszany.
    #[inline]
    fn u64(&mut self) -> u64 {
        self.stan = self.stan.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.stan;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Rozkład jednostajny (0, 1).
    #[inline]
    fn unit(&mut self) -> f64 {
        // zakres otwarty: `ln(0)` w Box-Mullerze dałoby nieskończoność
        ((self.u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    /// Rozkład normalny (Box-Muller).
    #[inline]
    fn normal(&mut self) -> f64 {
        let u1 = self.unit();
        let u2 = self.unit();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

impl Feed for SynthFeed {
    fn next(&mut self) -> Option<Quote> {
        if self.cfg.end_ts > 0 && self.ts >= self.cfg.end_ts {
            return None;
        }
        let dt_s = self.cfg.interval_ms as f64 / 1000.0;
        let krok = self.cfg.vol * dt_s.sqrt() * self.normal();
        self.cena += krok;
        // Twarde widełki wokół ceny startowej. Bez nich kilkudniowy przebieg
        // potrafi zawędrować do zera albo do kilkunastu tysięcy dolarów, a bot
        // przestaje wtedy odpowiadać na cokolwiek sensownego.
        let lo = self.cfg.start_price * 0.5;
        let hi = self.cfg.start_price * 2.0;
        if self.cena < lo || self.cena > hi {
            self.cena = self.cena.clamp(lo, hi);
        }
        // Zaokrąglamy ŚRODEK, a spread dokładamy symetrycznie — dzięki temu
        // spread jest dokładnie taki, jak w konfiguracji, a nie „pływa"
        // o pół centa przy każdym ticku od dwóch niezależnych zaokrągleń.
        let mid = (self.cena * 100.0).round() / 100.0;
        let q = Quote {
            ts: self.ts,
            bid: mid - self.cfg.spread / 2.0,
            ask: mid + self.cfg.spread / 2.0,
        };
        self.ts += self.cfg.interval_ms;
        self.n += 1;
        Some(q)
    }

    fn peek_ts(&self) -> Option<Ts> {
        if self.cfg.end_ts > 0 && self.ts >= self.cfg.end_ts {
            None
        } else {
            Some(self.ts)
        }
    }

    fn total(&self) -> u64 {
        if self.cfg.end_ts > self.cfg.start_ts && self.cfg.interval_ms > 0 {
            ((self.cfg.end_ts - self.cfg.start_ts) / self.cfg.interval_ms) as u64
        } else {
            0
        }
    }
    fn done(&self) -> u64 {
        self.n
    }
    fn first_ts(&self) -> Ts {
        self.cfg.start_ts
    }
    fn last_ts(&self) -> Ts {
        self.cfg.end_ts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zbierz(cfg: SynthCfg, ile: usize) -> Vec<Quote> {
        let mut f = SynthFeed::new(cfg);
        (0..ile).filter_map(|_| f.next()).collect()
    }

    #[test]
    fn to_samo_ziarno_daje_ten_sam_przebieg_co_do_centa() {
        let cfg = SynthCfg {
            seed: 42,
            ..Default::default()
        };
        let a = zbierz(cfg, 500);
        let b = zbierz(cfg, 500);
        assert_eq!(a.len(), 500);
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.bid, y.bid);
            assert_eq!(x.ask, y.ask);
            assert_eq!(x.ts, y.ts);
        }
    }

    #[test]
    fn inne_ziarno_daje_inny_przebieg() {
        let a = zbierz(
            SynthCfg {
                seed: 1,
                ..Default::default()
            },
            200,
        );
        let b = zbierz(
            SynthCfg {
                seed: 2,
                ..Default::default()
            },
            200,
        );
        assert!(
            a.iter().zip(b.iter()).any(|(x, y)| x.bid != y.bid),
            "ziarno nie ma wpływu"
        );
    }

    #[test]
    fn spread_jest_staly_a_ask_nigdy_nizej_niz_bid() {
        let q = zbierz(
            SynthCfg {
                seed: 7,
                spread: 0.24,
                ..Default::default()
            },
            300,
        );
        for x in &q {
            assert!(x.ask > x.bid, "ask {} <= bid {}", x.ask, x.bid);
            assert!((x.spread() - 0.24).abs() < 0.011, "spread {}", x.spread());
        }
    }

    #[test]
    fn znaczniki_rosna_o_zadany_odstep() {
        let q = zbierz(
            SynthCfg {
                seed: 3,
                interval_ms: 250,
                start_ts: 1_000_000,
                ..Default::default()
            },
            10,
        );
        assert_eq!(q[0].ts, 1_000_000);
        for w in q.windows(2) {
            assert_eq!(w[1].ts - w[0].ts, 250);
        }
    }

    #[test]
    fn zmiennosc_ma_rzad_wielkosci_zloty_a_nie_kosmosu() {
        // dobowa zmienność przy 0,15 $/√s to ok. 44 $ — sprawdzamy, że przez
        // godzinę cena nie ucieka o setki dolarów ani nie stoi w miejscu
        let cfg = SynthCfg {
            seed: 11,
            vol: 0.15,
            interval_ms: 250,
            ..Default::default()
        };
        let q = zbierz(cfg, 4 * 3600); // godzina po 250 ms
        let start = q[0].mid();
        let odchyl = q
            .iter()
            .map(|x| (x.mid() - start).abs())
            .fold(0.0f64, f64::max);
        assert!(odchyl > 0.5, "cena stoi w miejscu: {odchyl}");
        assert!(odchyl < 200.0, "cena ucieka: {odchyl}");
    }

    #[test]
    fn zakres_dat_konczy_generator() {
        let cfg = SynthCfg {
            seed: 5,
            start_ts: 0,
            end_ts: 10_000,
            interval_ms: 1000,
            ..Default::default()
        };
        let mut f = SynthFeed::new(cfg);
        let mut n = 0;
        while f.next().is_some() {
            n += 1;
            assert!(n < 1000, "generator nie zatrzymał się na końcu zakresu");
        }
        assert_eq!(n, 10);
        assert_eq!(f.total(), 10);
        assert_eq!(f.done(), 10);
    }
}
