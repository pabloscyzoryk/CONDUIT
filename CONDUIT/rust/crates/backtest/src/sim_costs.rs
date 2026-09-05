//! In-memory allocation of costs already charged by SimBroker. No cash writes.
//! No durable replay, no real-broker tariff assertion, no automatic fault clear.
use std::collections::HashMap;
use conduit_core::cost_receipt::*;
use conduit_core::types::{ClosedTrade, Ticket};

/// Produced once by the explicit native-swap broker profile. It is not an
/// instruction to round generic broker receipts or entry-cost pools.
#[derive(Clone, Copy)]
pub(crate) struct NativeSwapAllocation {
    pub realized: f64,
    pub remaining: f64,
}

#[derive(Default)]
pub(crate) struct SimCostLedger {
    pools: HashMap<Ticket, ResidualCostPool>,
    pub fault: Option<String>,
    pub quarantined: Vec<ClosedTrade>,
    next_deal: u64,
}

pub(crate) fn volume_units(volume: f64, step: f64) -> Result<u64, CostError> {
    if !volume.is_finite() || volume<=0.0 { return Err(CostError::InvalidVolume); }
    if !step.is_finite() || step<=0.0 { return Err(CostError::InvalidVolumeStep); }
    let n=volume/step;
    if !n.is_finite() || n.round()>MAX_EXACT_VOLUME_UNITS as f64 {return Err(CostError::VolumeUnitOverflow);}
    if n.round()<1.0 || (n-n.round()).abs()>8.0*f64::EPSILON*n.abs().max(1.0) {
        return Err(CostError::InvalidVolumeUnits);
    }
    Ok(n.round() as u64)
}

impl SimCostLedger {
    pub fn latch(&mut self, reason: impl Into<String>) {
        if self.fault.is_none() { self.fault=Some(reason.into()); }
    }
    pub fn entry_pool(volume:f64,step:f64,commission:f64)->Result<ResidualCostPool,CostError> {
        let mut p=ResidualCostPool::new(step)?;
        p.add_entry(volume_units(volume,step)?,commission,0.0)?;
        Ok(p)
    }
    pub fn insert(&mut self,t:Ticket,pool:ResidualCostPool) {
        if self.pools.insert(t,pool).is_some() {self.latch("duplicate simulated entry identity");}
    }
    pub fn swap(&mut self,t:Ticket,amount:f64) {
        let result=self.pools.get_mut(&t).ok_or(CostError::MissingReceipt).and_then(|p|p.accrue_swap(amount));
        if let Err(e)=result {self.latch(format!("swap pool #{t}: {e}"));}
    }
    pub fn swap_native(&mut self,t:Ticket,before:f64,after:f64) {
        let result=(||{
            let pool=self.pools.get_mut(&t).ok_or(CostError::MissingReceipt)?;
            let mut snapshot=pool.snapshot();
            if snapshot.swap != before || !after.is_finite() {return Err(CostError::ReceiptMismatch);}
            snapshot.swap=after;
            *pool=ResidualCostPool::try_from(snapshot)?;
            Ok(())
        })();
        if let Err(e)=result {self.latch(format!("native swap pool #{t}: {e}"));}
    }
    pub fn project(&mut self,tr:ClosedTrade,gross:f64,full:bool,spec:String,run_id:&str,native_swap:Option<NativeSwapAllocation>)
        -> Option<ClosedTrade> {
        if native_swap.is_some() && self.fault.is_some() {
            // A prior unknown accrual cannot become a fabricated complete zero.
            self.quarantined.push(tr); return None;
        }
        let result=(||{
            let mut pool=self.pools.get(&tr.ticket).cloned().ok_or(CostError::MissingReceipt)?;
            let before=pool.snapshot();
            let units=volume_units(tr.volume,before.volume_step)?;
            if full && units!=before.remaining_units {return Err(CostError::ReceiptMismatch);}
            let mut costs=pool.allocate_exit(units)?;
            if let Some(native)=native_swap {
                if !native.realized.is_finite() || !native.remaining.is_finite()
                    || (before.swap-native.realized-native.remaining).abs()>16.0*f64::EPSILON*before.swap.abs().max(1.0)
                    || (full && native.remaining!=0.0) {return Err(CostError::ReceiptMismatch);}
                costs.swap=native.realized;
                let mut after=pool.snapshot();
                after.swap=native.remaining;
                pool=ResidualCostPool::try_from(after)?;
            }
            let deal=self.next_deal.checked_add(1).ok_or(CostError::InvalidDealId)?;
            let receipt=CostReceipt {
                schema:CostSchema::V1,
                key:CostReceiptKey{scope_id:run_id.to_owned(),deal_id:deal},
                position_identifier:tr.ticket, volume:tr.volume, currency:"USD".into(),
                source:CostSource::SimulatorLedger{run_id:run_id.to_owned(),cost_spec_hash:spec},
                gross_profit:Some(gross),entry_commission_alloc:Some(costs.entry_commission),
                exit_commission:Some(0.0),entry_fee_alloc:Some(costs.entry_fee),exit_fee:Some(0.0),
                swap:Some(costs.swap),completeness:CostCompleteness::Complete,entry_allocation:None,
            };
            let projected=tr.clone().with_cost_receipt(receipt)?;
            self.next_deal=deal;
            if full {self.pools.remove(&tr.ticket);} else {self.pools.insert(tr.ticket,pool);}
            Ok(projected)
        })();
        match result {
            Ok(projected)=>Some(projected),
            Err(e)=>{self.latch(format!("closed transche #{}: {e}",tr.ticket));self.quarantined.push(tr);None}
        }
    }
}
