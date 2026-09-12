//! Pure bounded cache: live never falls back to sampled quote OHLC.
use crate::proto::{AccountIdent, RawM1Bars};
use conduit_core::{t100::Bar, types::Ts};

const LIMIT: usize = 512;

#[derive(Default)]
pub(crate) struct Cache {
    bars: Vec<Bar>,
    available: Vec<Ts>,
    pub issue: Option<String>,
}
impl Cache {
    pub fn clear(&mut self, reason: &str) {
        self.bars.clear(); self.available.clear(); self.issue = Some(reason.into());
    }
    pub fn accept(&mut self, packet: Result<RawM1Bars, String>, account: &AccountIdent, symbol: &str, observed_quote: Ts) {
        let p = match packet {Ok(p) => p, Err(e) => {self.clear(&e); return;}};
        if p.schema != 1 || p.symbol != symbol || p.account.login == 0
            || p.account.login != account.login || p.account.server != account.server
            || p.account.trade_mode != account.trade_mode {
            self.clear("M1 schema or account/symbol mismatch"); return;
        }
        if !p.complete || p.error.is_some() {
            self.clear(p.error.as_deref().unwrap_or("M1 incomplete")); return;
        }
        if p.bars.len() > LIMIT || p.observed_utc_ms <= 0 || p.available_at_ms <= 0
            || p.bars.windows(2).any(|w| w[0].ts >= w[1].ts)
            || p.bars.iter().any(|b| b.ts < 0 || b.ts % 60_000 != 0
                || b.ts.checked_add(60_000).is_none_or(|end|end > p.available_at_ms)
                || ![b.open,b.high,b.low,b.close].iter().all(|v|v.is_finite() && *v > 0.0)
                || b.high < b.open.max(b.close) || b.low > b.open.min(b.close) || b.high < b.low
                || b.max_spread != 0.0 || b.observations != 0) {
            self.clear("M1 malformed or not closed"); return;
        }
        self.issue = p.catchup_truncated.then(|| "M1 bounded catchup: older unavailable candles were omitted".into());
        // A delivery observed while replaying a drained quote batch must not
        // become available to an earlier historical quote from that batch.
        let available = p.available_at_ms.max(observed_quote).max(self.available.last().copied().unwrap_or(0));
        for bar in p.bars {
            if self.bars.last().is_some_and(|b|bar.ts <= b.ts) {continue;}
            self.bars.push(bar); self.available.push(available);
        }
        if self.bars.len() > LIMIT {
            let excess = self.bars.len() - LIMIT;
            self.bars.drain(..excess); self.available.drain(..excess);
        }
    }
    pub fn after(&self, after_ts: Option<Ts>, quote_ts: Ts) -> &[Bar] {
        let start = self.bars.partition_point(|b|after_ts.is_some_and(|after|b.ts <= after));
        let end = self.available.partition_point(|available|*available <= quote_ts);
        if start >= end {&[]} else {&self.bars[start..end]}
    }
}

#[cfg(test)] mod tests {
    use super::*;
    use crate::proto::M1Account;
    fn account() -> AccountIdent {AccountIdent {login:7,server:"fixture".into(),..Default::default()}}
    fn bar(ts:Ts)->Bar {Bar {ts,open:100.0,high:102.0,low:99.0,close:101.0,max_spread:0.0,observations:0}}
    fn packet()->RawM1Bars {RawM1Bars {schema:1,symbol:"XAUUSD".into(),account:M1Account {login:7,server:"fixture".into(),trade_mode:0},observed_utc_ms:1,available_at_ms:180_000,complete:true,error:None,catchup_truncated:false,bars:vec![bar(60_000),bar(120_000)]}}
    #[test] fn complete_m1_suffix_is_non_consuming_and_cannot_backdate_delivery() {
        let mut c=Cache::default();c.accept(Ok(packet()),&account(),"XAUUSD",180_050);
        assert!(c.after(None,180_000).is_empty());assert_eq!(c.after(None,180_050).len(),2);
        assert_eq!(c.after(Some(60_000),180_050),&[bar(120_000)]);
        assert_eq!(c.after(Some(60_000),180_050),&[bar(120_000)]);
        assert!(c.after(Some(120_000),180_050).is_empty());
    }
    #[test] fn complete_m1_invalid_or_changed_scope_clears_previous_data() {
        for case in 0..6 {let mut c=Cache::default();c.accept(Ok(packet()),&account(),"XAUUSD",180_000);
            let mut p=packet(); match case {0=>p.account.login+=1,1=>p.symbol="other".into(),2=>p.bars[1].ts=180_000,3=>p.complete=false,4=>p.bars[0].high=90.0,_=>p.bars.reverse()};
            c.accept(Ok(p),&account(),"XAUUSD",180_000);assert!(c.after(None,180_000).is_empty());assert!(c.issue.is_some());}
    }
    #[test] fn complete_m1_gap_keeps_only_real_bars_and_deduplicates_corrections() {
        let mut c=Cache::default();c.accept(Ok(packet()),&account(),"XAUUSD",180_000);
        let mut p=packet();p.available_at_ms=600_000;p.bars=vec![bar(120_000),bar(540_000)];
        p.bars[0].close=100.5;c.accept(Ok(p),&account(),"XAUUSD",600_000);
        assert_eq!(c.after(None,600_000),&[bar(60_000),bar(120_000),bar(540_000)]);
        c.clear("reconnect");assert!(c.after(None,600_000).is_empty());
    }
    #[test] fn complete_m1_empty_success_has_no_sampled_fallback() {
        let mut c=Cache::default();let mut p=packet();p.bars.clear();c.accept(Ok(p),&account(),"XAUUSD",180_000);
        assert!(c.after(None,180_000).is_empty());assert!(c.issue.is_none());
    }
}
