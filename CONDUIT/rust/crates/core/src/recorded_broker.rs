//! Offline broker interaction tape. Recording never retries or substitutes an outcome.
pub mod exact;
pub mod revisions;
use crate::{broker::*, types::*};
use exact::{decode, encode, Exact};
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Call {
    pub method: String,
    pub args: usize,
    pub result: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trace {
    pub schema: u32,
    pub calls: Vec<Call>,
    pub values: Vec<Exact>,
    pub incomplete: bool,
    #[serde(skip)]
    estimated_bytes: usize,
}
impl Default for Trace {
    fn default() -> Self {
        Self {
            schema: 2,
            calls: vec![],
            values: vec![],
            incomplete: false,
            estimated_bytes: 0,
        }
    }
}
impl Trace {
    fn intern(&mut self, value: Exact) -> usize {
        if let Some(n) = self.values.iter().position(|v| v == &value) {
            n
        } else {
            self.estimated_bytes = self.estimated_bytes.saturating_add(value.estimated_bytes());
            if self.estimated_bytes > 16 * 1024 * 1024 {
                self.incomplete = true;
                return 0;
            }
            let n = self.values.len();
            self.values.push(value);
            n
        }
    }
}
const MAX_CALLS: usize = 10_000;

// The exhaustive pattern is intentional: adding a Position/PendingOrder field
// causes a compile error until its comparison contract is reviewed. Every real
// broker read is still performed; only encoding of bit-identical data is reused.
macro_rules! snapshot_equal {($ty:ident,$a:ident,$b:ident,plain[$($p:ident),*],bits[$($f:ident),*],optional[$($o:ident),*])=>{{
    let $ty{$($p:_,)*$($f:_,)*$($o:_,)*}=$a;
    true $(&& $a.$p==$b.$p)* $(&& $a.$f.to_bits()==$b.$f.to_bits())* $(&& $a.$o.map(f64::to_bits)==$b.$o.map(f64::to_bits))*
}};}
fn same_position(a: &Position, b: &Position) -> bool {
    snapshot_equal!(Position,a,b,plain[ticket,side,open_ts,basket,level,frozen,last_peak_ts,is_runner,is_toucher,comment],bits[volume,open_price,peak_pts],optional[sl,tp,vsl])
}
fn same_pending(a: &PendingOrder, b: &PendingOrder) -> bool {
    snapshot_equal!(PendingOrder,a,b,plain[ticket,kind,placed_ts,basket,level,frozen,is_toucher,is_topup,comment],bits[volume,price],optional[sl,tp])
}
pub struct Recorder<'a, B: Broker> {
    inner: &'a mut B,
    trace: RefCell<Trace>,
    mutable: Cell<Option<bool>>,
    position_cache: RefCell<std::collections::HashMap<&'static str, (Vec<Position>, usize)>>,
    pending_cache: RefCell<std::collections::HashMap<&'static str, (Vec<PendingOrder>, usize)>>,
}
impl<'a, B: Broker> Recorder<'a, B> {
    pub fn new(inner: &'a mut B) -> Self {
        Self {
            inner,
            trace: RefCell::new(Trace::default()),
            mutable: Cell::new(None),
            position_cache: RefCell::new(Default::default()),
            pending_cache: RefCell::new(Default::default()),
        }
    }
    fn push<A: Serialize + ?Sized, R: Serialize + ?Sized>(
        &self,
        method: &str,
        args: &A,
        result: &R,
    ) {
        let mut t = self.trace.borrow_mut();
        if t.incomplete || t.calls.len() >= MAX_CALLS {
            t.incomplete = true;
            return;
        }
        match (encode(args), encode(result)) {
            (Ok(args), Ok(result)) => {
                let args = t.intern(args);
                let result = t.intern(result);
                t.calls.push(Call {
                    method: method.into(),
                    args,
                    result,
                });
            }
            _ => t.incomplete = true,
        }
    }
    fn push_positions(&self, method: &'static str, actual: &[Position]) {
        let mut cache = self.position_cache.borrow_mut();
        let mut tape = self.trace.borrow_mut();
        if tape.incomplete || tape.calls.len() >= MAX_CALLS {
            tape.incomplete = true;
            return;
        }
        let known = cache
            .get(method)
            .filter(|(old, _)| {
                old.len() == actual.len()
                    && old.iter().zip(actual).all(|(a, b)| same_position(a, b))
            })
            .map(|(_, id)| *id);
        let result = match known {
            Some(id) => id,
            None => {
                let value = match encode(actual) {
                    Ok(v) => v,
                    Err(_) => {
                        tape.incomplete = true;
                        return;
                    }
                };
                let id = tape.intern(value);
                cache.insert(method, (actual.to_vec(), id));
                id
            }
        };
        let args = tape.intern(Exact::Unit);
        tape.calls.push(Call {
            method: method.into(),
            args,
            result,
        });
    }
    fn push_pendings(&self, method: &'static str, actual: &[PendingOrder]) {
        let mut cache = self.pending_cache.borrow_mut();
        let mut tape = self.trace.borrow_mut();
        if tape.incomplete || tape.calls.len() >= MAX_CALLS {
            tape.incomplete = true;
            return;
        }
        let known = cache
            .get(method)
            .filter(|(old, _)| {
                old.len() == actual.len() && old.iter().zip(actual).all(|(a, b)| same_pending(a, b))
            })
            .map(|(_, id)| *id);
        let result = match known {
            Some(id) => id,
            None => {
                let value = match encode(actual) {
                    Ok(v) => v,
                    Err(_) => {
                        tape.incomplete = true;
                        return;
                    }
                };
                let id = tape.intern(value);
                cache.insert(method, (actual.to_vec(), id));
                id
            }
        };
        let args = tape.intern(Exact::Unit);
        tape.calls.push(Call {
            method: method.into(),
            args,
            result,
        });
    }
    fn before(&self) {
        // Mutable cache edits are recorded at the next boundary. These are
        // cached reads of the same broker view, never additional MT5 RPCs.
        if let Some(positions) = self.mutable.take() {
            if positions {
                self.push_positions("positions_commit", self.inner.positions())
            } else {
                self.push_pendings("pendings_commit", self.inner.pendings())
            }
        }
    }
    pub fn finish(&mut self) -> Trace {
        self.before();
        self.position_cache.borrow_mut().clear();
        self.pending_cache.borrow_mut().clear();
        std::mem::take(&mut *self.trace.borrow_mut())
    }
}
macro_rules! read_scalar {
    ($name:ident,$ret:ty) => {
        fn $name(&self) -> $ret {
            self.before();
            let r = self.inner.$name();
            self.push(stringify!($name), &(), &r);
            r
        }
    };
}
impl<B: Broker> Broker for Recorder<'_, B> {
    fn complete_m1_bars(&self, after_ts: Option<Ts>) -> Option<&[crate::t100::Bar]> {
        self.before();
        let result = self.inner.complete_m1_bars(after_ts);
        self.push("complete_m1_bars", &after_ts, &result);
        result
    }
    read_scalar!(quote, Quote);
    read_scalar!(t100_contract_supported, bool);
    read_scalar!(account, Account);
    read_scalar!(stops_level, f64);
    read_scalar!(volume_min, f64);
    read_scalar!(volume_step, f64);
    read_scalar!(volume_max, f64);
    read_scalar!(close_receipt_reconciliation_active, bool);
    read_scalar!(close_receipts_pending, bool);
    read_scalar!(receipt_barrier, ReceiptBarrier);
    read_scalar!(execution_session, Option<ExecutionSession>);
    read_scalar!(unconfirmed_open, Option<UnconfirmedOpen>);
    fn confirmed_open(&self, intent: &UnconfirmedOpen) -> Option<Ticket> {
        self.before();
        let r = self.inner.confirmed_open(intent);
        self.push("confirmed_open", intent, &r);
        r
    }
    read_scalar!(pending_cancel_snapshot_authoritative, bool);
    read_scalar!(cost_net_supported, bool);
    fn normalize_order_price(&self, p: f64) -> f64 {
        self.before();
        let r = self.inner.normalize_order_price(p);
        self.push("normalize_order_price", &p, &r);
        r
    }
    fn position_identifier(&self, t: Ticket) -> Option<u64> {
        self.before();
        let r = self.inner.position_identifier(t);
        self.push("position_identifier", &t, &r);
        r
    }
    fn positions(&self) -> &[Position] {
        self.before();
        let r = self.inner.positions();
        self.push_positions("positions", r);
        r
    }
    fn find_position(&self, t: Ticket) -> Option<&Position> {
        self.before();
        let r = self.inner.find_position(t);
        self.push("find_position", &t, &r);
        r
    }
    fn pendings(&self) -> &[PendingOrder] {
        self.before();
        let r = self.inner.pendings();
        self.push_pendings("pendings", r);
        r
    }
    fn ukryte_pozycje(&self) -> &[Position] {
        self.before();
        let r = self.inner.ukryte_pozycje();
        self.push_positions("ukryte_pozycje", r);
        r
    }
    fn ukryte_zlecenia(&self) -> &[PendingOrder] {
        self.before();
        let r = self.inner.ukryte_zlecenia();
        self.push_pendings("ukryte_zlecenia", r);
        r
    }
    fn positions_mut(&mut self) -> &mut Vec<Position> {
        self.before();
        self.push_positions("positions_mut", self.inner.positions());
        self.mutable.set(Some(true));
        self.inner.positions_mut()
    }
    fn pendings_mut(&mut self) -> &mut Vec<PendingOrder> {
        self.before();
        self.push_pendings("pendings_mut", self.inner.pendings());
        self.mutable.set(Some(false));
        self.inner.pendings_mut()
    }
    fn open_market(&mut self, r: OrderReq) -> BResult<Ticket> {
        self.before();
        let args = r.clone();
        let out = self.inner.open_market(r);
        self.push("open_market", &args, &out);
        out
    }
    fn place_pending(&mut self, r: PendingReq) -> BResult<Ticket> {
        self.before();
        let args = r.clone();
        let out = self.inner.place_pending(r);
        self.push("place_pending", &args, &out);
        out
    }
    fn modify_position(&mut self, t: Ticket, sl: Option<Px>, tp: Option<Px>) -> BResult<()> {
        self.before();
        let r = self.inner.modify_position(t, sl, tp);
        self.push("modify_position", &(t, sl, tp), &r);
        r
    }
    fn modify_pending(&mut self, t: Ticket, p: Px, sl: Option<Px>, tp: Option<Px>) -> BResult<()> {
        self.before();
        let r = self.inner.modify_pending(t, p, sl, tp);
        self.push("modify_pending", &(t, p, sl, tp), &r);
        r
    }
    fn close_position(&mut self, t: Ticket, why: CloseReason) -> BResult<f64> {
        self.before();
        let r = self.inner.close_position(t, why);
        self.push("close_position", &(t, why), &r);
        r
    }
    fn close_partial(&mut self, t: Ticket, v: f64, why: CloseReason) -> BResult<f64> {
        self.before();
        let r = self.inner.close_partial(t, v, why);
        self.push("close_partial", &(t, v, why), &r);
        r
    }
    fn cancel_pending(&mut self, t: Ticket) -> BResult<()> {
        self.before();
        let r = self.inner.cancel_pending(t);
        self.push("cancel_pending", &t, &r);
        r
    }
    fn drain_closed(&mut self) -> Vec<ClosedTrade> {
        self.before();
        let r = self.inner.drain_closed();
        self.push("drain_closed", &(), &r);
        r
    }
    fn report_cost_consumer_fault(&mut self, why: &str) {
        self.before();
        self.inner.report_cost_consumer_fault(why);
        self.push("report_cost_consumer_fault", why, &());
    }
}

/// This adapter is only used by the offline verifier. A disagreement aborts
/// the current replay immediately; it can never reach a real broker.
#[derive(Clone, Debug, Serialize)]
pub struct Mismatch {
    pub index: usize,
    pub expected: String,
    pub actual: String,
}
struct Decoded {
    call: Call,
    positions: Option<std::sync::Arc<Vec<Position>>>,
    pendings: Option<std::sync::Arc<Vec<PendingOrder>>>,
    found: Option<Position>,
    bars: Option<std::sync::Arc<Vec<crate::t100::Bar>>>,
}
pub struct ReplayBroker {
    values: Vec<Exact>,
    calls: Vec<Decoded>,
    cursor: Cell<usize>,
    mutable: Cell<Option<bool>>,
    positions: Vec<Position>,
    pendings: Vec<PendingOrder>,
    on_mismatch: Option<Box<dyn Fn(&Mismatch)>>,
}
impl ReplayBroker {
    pub fn new(trace: Trace) -> Result<Self, String> {
        if trace.incomplete || trace.schema != 2 || trace.calls.len() > MAX_CALLS {
            return Err("broker transcript incomplete or unsupported".into());
        }
        let mut calls = Vec::with_capacity(trace.calls.len());
        let mut position_cache =
            std::collections::HashMap::<usize, std::sync::Arc<Vec<Position>>>::new();
        let mut pending_cache =
            std::collections::HashMap::<usize, std::sync::Arc<Vec<PendingOrder>>>::new();
        let mut bar_cache = std::collections::HashMap::<usize, Option<std::sync::Arc<Vec<crate::t100::Bar>>>>::new();
        for call in trace.calls {
            if trace.values.get(call.args).is_none() {
                return Err("broker arguments reference missing".into());
            }
            let value = trace
                .values
                .get(call.result)
                .ok_or("broker result reference missing")?;
            let positions = if matches!(call.method.as_str(), "positions" | "ukryte_pozycje") {
                if !position_cache.contains_key(&call.result) {
                    position_cache.insert(
                        call.result,
                        std::sync::Arc::new(decode(value).map_err(|e| e.to_string())?),
                    );
                }
                position_cache.get(&call.result).cloned()
            } else {
                None
            };
            let pendings = if matches!(call.method.as_str(), "pendings" | "ukryte_zlecenia") {
                if !pending_cache.contains_key(&call.result) {
                    pending_cache.insert(
                        call.result,
                        std::sync::Arc::new(decode(value).map_err(|e| e.to_string())?),
                    );
                }
                pending_cache.get(&call.result).cloned()
            } else {
                None
            };
            let found = if call.method == "find_position" {
                decode(value).map_err(|e| e.to_string())?
            } else {
                None
            };
            let bars = if call.method == "complete_m1_bars" {
                if !bar_cache.contains_key(&call.result) {
                    let bars: Option<Vec<crate::t100::Bar>> = decode(value).map_err(|e| e.to_string())?;
                    bar_cache.insert(call.result, bars.map(std::sync::Arc::new));
                }
                bar_cache.get(&call.result).cloned().flatten()
            } else { None };
            calls.push(Decoded {
                call,
                positions,
                pendings,
                found,
                bars,
            });
        }
        Ok(Self {
            values: trace.values,
            calls,
            cursor: Cell::new(0),
            mutable: Cell::new(None),
            positions: Vec::new(),
            pendings: Vec::new(),
            on_mismatch: None,
        })
    }
    pub fn on_mismatch(&mut self, f: impl Fn(&Mismatch) + 'static) {
        self.on_mismatch = Some(Box::new(f));
    }
    fn fail(&self, expected: &str, actual: &str) -> ! {
        let error = Mismatch {
            index: self.cursor.get(),
            expected: expected.into(),
            actual: actual.into(),
        };
        if let Some(report) = &self.on_mismatch {
            report(&error);
        }
        eprintln!(
            "CONDUIT_REPLAY_MISMATCH {}",
            serde_json::to_string(&error).unwrap_or_default()
        );
        // The shipped profile aborts on panic. Emit/report first and return a
        // deliberate nonzero exit instead; debug tests can catch the typed panic.
        #[cfg(panic = "abort")]
        std::process::exit(3);
        #[cfg(not(panic = "abort"))]
        std::panic::panic_any(error)
    }
    fn consume<A: Serialize + ?Sized>(&self, method: &str, args: &A) -> usize {
        let index = self.cursor.get();
        let Some(c) = self.calls.get(index) else {
            self.fail("end of transcript", method)
        };
        if c.call.method != method {
            self.fail(&c.call.method, method)
        }
        if encode(args).ok().as_ref() != Some(&self.values[c.call.args]) {
            self.fail("recorded arguments", method)
        }
        self.cursor.set(index + 1);
        index
    }
    fn before(&self) {
        if let Some(p) = self.mutable.take() {
            let method = if p {
                "positions_commit"
            } else {
                "pendings_commit"
            };
            let i = self.consume(method, &());
            let actual = if p {
                encode(&self.positions)
            } else {
                encode(&self.pendings)
            };
            if actual.ok().as_ref() != Some(&self.values[self.calls[i].call.result]) {
                self.fail("recorded mutable cache", method)
            }
        }
    }
    fn answer<A: Serialize + ?Sized, R: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        args: &A,
    ) -> R {
        self.before();
        let i = self.consume(method, args);
        decode(&self.values[self.calls[i].call.result])
            .unwrap_or_else(|_| self.fail("well-typed result", method))
    }
    pub fn finish(&self) -> Result<(), String> {
        self.before();
        if self.cursor.get() != self.calls.len() {
            return Err(format!(
                "{} unconsumed broker calls",
                self.calls.len() - self.cursor.get()
            ));
        }
        Ok(())
    }
}
macro_rules! replay_scalar {
    ($name:ident,$ret:ty) => {
        fn $name(&self) -> $ret {
            self.answer(stringify!($name), &())
        }
    };
}
impl Broker for ReplayBroker {
    fn complete_m1_bars(&self, after_ts: Option<Ts>) -> Option<&[crate::t100::Bar]> {
        self.before();
        let i = self.consume("complete_m1_bars", &after_ts);
        self.calls[i].bars.as_deref().map(Vec::as_slice)
    }
    replay_scalar!(quote, Quote);
    replay_scalar!(t100_contract_supported, bool);
    replay_scalar!(account, Account);
    replay_scalar!(stops_level, f64);
    replay_scalar!(volume_min, f64);
    replay_scalar!(volume_step, f64);
    replay_scalar!(volume_max, f64);
    replay_scalar!(close_receipt_reconciliation_active, bool);
    replay_scalar!(close_receipts_pending, bool);
    replay_scalar!(receipt_barrier, ReceiptBarrier);
    replay_scalar!(execution_session, Option<ExecutionSession>);
    replay_scalar!(unconfirmed_open, Option<UnconfirmedOpen>);
    fn confirmed_open(&self, intent: &UnconfirmedOpen) -> Option<Ticket> {
        self.answer("confirmed_open", intent)
    }
    replay_scalar!(pending_cancel_snapshot_authoritative, bool);
    replay_scalar!(cost_net_supported, bool);
    fn normalize_order_price(&self, p: f64) -> f64 {
        self.answer("normalize_order_price", &p)
    }
    fn position_identifier(&self, t: Ticket) -> Option<u64> {
        self.answer("position_identifier", &t)
    }
    fn positions(&self) -> &[Position] {
        self.before();
        let i = self.consume("positions", &());
        self.calls[i].positions.as_deref().unwrap()
    }
    fn find_position(&self, t: Ticket) -> Option<&Position> {
        self.before();
        let i = self.consume("find_position", &t);
        self.calls[i].found.as_ref()
    }
    fn pendings(&self) -> &[PendingOrder] {
        self.before();
        let i = self.consume("pendings", &());
        self.calls[i].pendings.as_deref().unwrap()
    }
    fn ukryte_pozycje(&self) -> &[Position] {
        self.before();
        let i = self.consume("ukryte_pozycje", &());
        self.calls[i].positions.as_deref().unwrap()
    }
    fn ukryte_zlecenia(&self) -> &[PendingOrder] {
        self.before();
        let i = self.consume("ukryte_zlecenia", &());
        self.calls[i].pendings.as_deref().unwrap()
    }
    fn positions_mut(&mut self) -> &mut Vec<Position> {
        self.positions = self.answer("positions_mut", &());
        self.mutable.set(Some(true));
        &mut self.positions
    }
    fn pendings_mut(&mut self) -> &mut Vec<PendingOrder> {
        self.pendings = self.answer("pendings_mut", &());
        self.mutable.set(Some(false));
        &mut self.pendings
    }
    fn open_market(&mut self, r: OrderReq) -> BResult<Ticket> {
        self.answer("open_market", &r)
    }
    fn place_pending(&mut self, r: PendingReq) -> BResult<Ticket> {
        self.answer("place_pending", &r)
    }
    fn modify_position(&mut self, t: Ticket, sl: Option<Px>, tp: Option<Px>) -> BResult<()> {
        self.answer("modify_position", &(t, sl, tp))
    }
    fn modify_pending(&mut self, t: Ticket, p: Px, sl: Option<Px>, tp: Option<Px>) -> BResult<()> {
        self.answer("modify_pending", &(t, p, sl, tp))
    }
    fn close_position(&mut self, t: Ticket, why: CloseReason) -> BResult<f64> {
        self.answer("close_position", &(t, why))
    }
    fn close_partial(&mut self, t: Ticket, v: f64, why: CloseReason) -> BResult<f64> {
        self.answer("close_partial", &(t, v, why))
    }
    fn cancel_pending(&mut self, t: Ticket) -> BResult<()> {
        self.answer("cancel_pending", &t)
    }
    fn drain_closed(&mut self) -> Vec<ClosedTrade> {
        self.answer("drain_closed", &())
    }
    fn report_cost_consumer_fault(&mut self, why: &str) {
        self.answer("report_cost_consumer_fault", why)
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    #[test]
    fn cached_snapshot_is_bit_exact_and_detects_real_mutations() {
        let mut a = Position {
            ticket: 1,
            side: Side::Buy,
            volume: 0.01,
            open_price: 4000.0,
            open_ts: 1,
            sl: Some(3990.0),
            tp: Some(4010.0),
            vsl: None,
            basket: Some(1),
            level: 0,
            frozen: false,
            peak_pts: f64::from_bits(0x7ff8000000000042),
            last_peak_ts: 1,
            is_runner: false,
            is_toucher: false,
            comment: "synthetic".into(),
        };
        let mut b = a.clone();
        assert!(same_position(&a, &b));
        assert_eq!(encode(&a).unwrap(), encode(&b).unwrap());
        a.vsl = Some(-0.0);
        b.vsl = Some(0.0);
        assert!(!same_position(&a, &b));
        b = a.clone();
        b.sl = Some(3991.0);
        assert!(!same_position(&a, &b));
        b = a.clone();
        b.comment.push('x');
        assert!(!same_position(&a, &b));
        b = a.clone();
        b.volume = f64::from_bits(a.volume.to_bits() + 1);
        assert!(!same_position(&a, &b));
        let mut p = PendingOrder {
            ticket: 2,
            kind: PendingKind::BuyLimit,
            volume: 0.01,
            price: 3999.0,
            sl: None,
            tp: None,
            placed_ts: 1,
            basket: Some(1),
            level: 0,
            frozen: false,
            is_toucher: false,
            is_topup: false,
            comment: String::new(),
        };
        let mut q = p.clone();
        assert!(same_pending(&p, &q));
        q.price = 4000.0;
        assert!(!same_pending(&p, &q));
        p.tp = Some(-0.0);
        q = p.clone();
        q.tp = Some(0.0);
        assert!(!same_pending(&p, &q));
    }
}
