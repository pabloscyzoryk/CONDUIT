//! Shared S/R close mathematics. Both continuous ticks and V2 state-only
//! warmup use these exact operations, in the same order.
use super::StanSr;
use crate::{Settings, types::Ts};

pub(super) struct SrStateMath<'a> {
    pub cfg: &'a Settings,
    pub sr: &'a mut StanSr,
}
impl SrStateMath<'_> {
    fn sr_dynamic_active(&self) -> bool {
        self.cfg.trail_sr_min_prominence_atr > 0.0
            || self.cfg.trail_sr_offset_atr_mult > 0.0
            || self.cfg.trail_sr_offset_spread_mult > 0.0
    }
    fn sr_zamknij_dynamiczne(&mut self) {
        let n = self.cfg.trail_sr_atr_period.max(1) as usize;
        let zakres = self.sr.high - self.sr.low;
        let tr = match self.sr.prev_close {
            Some(c) => zakres
                .max((self.sr.high - c).abs())
                .max((self.sr.low - c).abs()),
            None => zakres,
        };
        self.sr.prev_close = Some(self.sr.close);
        self.sr.atr_true_ranges.push_back(tr.max(0.0));
        self.sr
            .spread_closed
            .push_back(self.sr.spread_close.max(0.0));
        while self.sr.atr_true_ranges.len() > n {
            self.sr.atr_true_ranges.pop_front();
        }
        while self.sr.spread_closed.len() > n {
            self.sr.spread_closed.pop_front();
        }
        self.sr.atr = (self.sr.atr_true_ranges.len() == n)
            .then(|| self.sr.atr_true_ranges.iter().copied().sum::<f64>() / n as f64);
        self.sr.spread_ref = (self.sr.spread_closed.len() == n).then(|| {
            let mut v: Vec<f64> = self.sr.spread_closed.iter().copied().collect();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            // nearest-rank p90: ceil(0.9*n)-1, bez interpolacji i bez przyszłości
            let i = (9 * n).div_ceil(10).saturating_sub(1).min(n - 1);
            v[i]
        });
    }

    pub(super) fn sr_zamknij_swiece(&mut self, ts: Ts) {
        let dynamiczny = self.sr_dynamic_active();
        if dynamiczny {
            self.sr_zamknij_dynamiczne();
        }
        self.sr.zamkniete.push_back((self.sr.high, self.sr.low));
        let fn_ = self.cfg.trail_sr_fractal_n.max(1) as usize;
        let okno = 2 * fn_ + 1;
        if self.sr.zamkniete.len() > okno {
            self.sr.zamkniete.pop_front();
        }
        if self.sr.zamkniete.len() == okno {
            let (sh, sl) = self.sr.zamkniete[fn_];
            let mut low_ok = true;
            let mut high_ok = true;
            for (i, &(h, l)) in self.sr.zamkniete.iter().enumerate() {
                if i == fn_ {
                    continue;
                }
                if l <= sl {
                    low_ok = false;
                }
                if h >= sh {
                    high_ok = false;
                }
            }
            if low_ok {
                self.sr.swingi_low.push_back((sl, ts));
                if dynamiczny {
                    if let Some(atr) = self.sr.atr.filter(|x| *x > 0.0) {
                        let lewy = self
                            .sr
                            .zamkniete
                            .iter()
                            .take(fn_)
                            .map(|&(h, _)| h)
                            .fold(f64::NEG_INFINITY, f64::max);
                        let prawy = self
                            .sr
                            .zamkniete
                            .iter()
                            .skip(fn_ + 1)
                            .map(|&(h, _)| h)
                            .fold(f64::NEG_INFINITY, f64::max);
                        let prom = (lewy.min(prawy) - sl).max(0.0) / atr;
                        self.sr.prominence_low.push_back((sl, ts, prom));
                    }
                }
            }
            if high_ok {
                self.sr.swingi_high.push_back((sh, ts));
                if dynamiczny {
                    if let Some(atr) = self.sr.atr.filter(|x| *x > 0.0) {
                        let lewy = self
                            .sr
                            .zamkniete
                            .iter()
                            .take(fn_)
                            .map(|&(_, l)| l)
                            .fold(f64::INFINITY, f64::min);
                        let prawy = self
                            .sr
                            .zamkniete
                            .iter()
                            .skip(fn_ + 1)
                            .map(|&(_, l)| l)
                            .fold(f64::INFINITY, f64::min);
                        let prom = (sh - lewy.max(prawy)).max(0.0) / atr;
                        self.sr.prominence_high.push_back((sh, ts, prom));
                    }
                }
            }
        }
        // struktura starsza niż okno — poza pamięcią (przycinamy po t_potw)
        let horyzont = ts - 3_600_000 * self.cfg.trail_sr_struct_window_h.max(1) as i64;
        while self
            .sr
            .swingi_low
            .front()
            .map(|&(_, t)| t < horyzont)
            .unwrap_or(false)
        {
            self.sr.swingi_low.pop_front();
        }
        while self
            .sr
            .swingi_high
            .front()
            .map(|&(_, t)| t < horyzont)
            .unwrap_or(false)
        {
            self.sr.swingi_high.pop_front();
        }
        while self
            .sr
            .prominence_low
            .front()
            .map(|&(_, t, _)| t < horyzont)
            .unwrap_or(false)
        {
            self.sr.prominence_low.pop_front();
        }
        while self
            .sr
            .prominence_high
            .front()
            .map(|&(_, t, _)| t < horyzont)
            .unwrap_or(false)
        {
            self.sr.prominence_high.pop_front();
        }
        self.sr.nowa_swieca = true;
    }
}

