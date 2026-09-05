//! Session-scoped evidence of position-volume reductions. No price/PnL guess,
//! no synthetic close, no timeout expiry and no durable/restart ACK claim.
use conduit_core::{broker::ExecutionSession, types::Ticket};
use std::{collections::HashMap, time::Instant};

#[derive(Clone, Debug)]
struct VolumeEvidence {
    ticket: Ticket,
    opened: f64,
    snapshot_volume: f64,
    snapshot_reductions: f64,
    receipted: f64,
    gap_since: Option<Instant>,
    last_snapshot_seq: u64,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ReceiptVolumes {
    session: Option<ExecutionSession>,
    states: HashMap<u64, VolumeEvidence>,
    snapshot_seq: u64,
}

fn eps(volume:f64)->f64 {volume.abs().max(1.0)*f64::EPSILON*64.0}
fn differs(a:f64,b:f64)->bool {(a-b).abs()>eps(a.max(b))}
impl ReceiptVolumes {
    fn bind(&mut self, session:&ExecutionSession)->Result<(),String> {
        if session.scope.is_empty() || session.generation==0 {return Err("unverified volume-evidence session".into());}
        match &self.session {
            Some(old) if old!=session => Err("position-volume evidence belongs to another account/runtime generation; reconciliation required".into()),
            Some(_)=>Ok(()),None=>{self.session=Some(session.clone());Ok(())}
        }
    }
    pub fn scope_mismatch(&self, current:Option<&ExecutionSession>)->bool {
        self.session.as_ref().is_some_and(|bound|Some(bound)!=current)
    }
    /// Registration comes from observed owned metadata or a positive OPEN ACK,
    /// not from a closing deal or guessed pending-order correlation.
    pub fn remember(&mut self,session:&ExecutionSession,id:u64,ticket:Ticket,volume:f64)->Result<(),String>{
        self.bind(session)?;
        if id==0 || ticket==0 || !volume.is_finite() || volume<=0.0 {return Err("invalid initial position-volume evidence".into());}
        self.states.entry(id).or_insert(VolumeEvidence{ticket,opened:volume,snapshot_volume:volume,
            snapshot_reductions:0.0,receipted:0.0,gap_since:None,last_snapshot_seq:self.snapshot_seq});
        Ok(())
    }
    /// Full owned-position snapshot. A row missing from this complete list has
    /// observed volume zero, but is NOT assumed closed economically or assigned
    /// a reason. Validate and apply the entire observation atomically.
    pub fn observe(&mut self,session:&ExecutionSession,rows:&[(u64,Ticket,f64)])->Result<(),String>{
        self.bind(session)?;
        let mut values=HashMap::new();
        for &(id,ticket,volume) in rows {
            if id==0 || ticket==0 || !volume.is_finite() || volume<=0.0 {
                return Err("invalid position snapshot identifier/volume".into());
            }
            if values.insert(id,(ticket,volume)).is_some(){return Err(format!("duplicate position identifier={id} in a single snapshot"));}
        }
        let sequence=self.snapshot_seq.checked_add(1).ok_or("snapshot sequence overflow")?;
        let mut next=self.states.clone();
        for (id,e) in &mut next {
            let (ticket,volume)=values.get(id).copied().unwrap_or((e.ticket,0.0));
            if volume>e.snapshot_volume+eps(e.opened) {
                return Err(format!("position identifier={id} increased {} -> {volume}; additional fill or stale snapshot needs entry-deal proof",e.snapshot_volume));
            }
            if volume<e.snapshot_volume-eps(e.opened) {
                e.snapshot_reductions+=e.snapshot_volume-volume;
                if !e.snapshot_reductions.is_finite(){return Err("position reduction overflow".into());}
            }
            e.ticket=ticket;e.snapshot_volume=volume;e.last_snapshot_seq=sequence;
            if differs(e.snapshot_reductions,e.receipted) {
                if e.gap_since.is_none(){e.gap_since=Some(Instant::now());}
            }else{e.gap_since=None;}
        }
        self.states=next;self.snapshot_seq=sequence;Ok(())
    }
    /// Caller has already validated ownership, economics and unique deal ID.
    /// The closed buffer/owner waiting queue remains a separate entry barrier
    /// until this receipt is actually handed to the corresponding Engine.
    pub fn accept_close(&mut self,session:&ExecutionSession,id:u64,volume:f64)->Result<(),String>{
        self.bind(session)?;
        let e=self.states.get_mut(&id).ok_or_else(||format!("no observed/open-ACK volume for identifier={id}"))?;
        let total=e.receipted+volume;
        if !volume.is_finite()||volume<=0.0||!total.is_finite()||total>e.opened+eps(e.opened){
            return Err(format!("closed volume exceeds proven opening for identifier={id}: {total} > {}",e.opened));
        }
        e.receipted=total;
        if differs(e.snapshot_reductions,e.receipted){if e.gap_since.is_none(){e.gap_since=Some(Instant::now());}}
        else{e.gap_since=None;}
        Ok(())
    }
    pub fn pending(&self)->bool {self.states.values().any(|e|differs(e.snapshot_reductions,e.receipted))}
    pub fn pending_details(&self)->Vec<serde_json::Value>{
        let mut rows:Vec<_>=self.states.iter().filter(|(_,e)|differs(e.snapshot_reductions,e.receipted)).map(|(id,e)|{
            serde_json::json!({"identifier":id,"ticket":e.ticket,"opened_volume":e.opened,
                "snapshot_volume":e.snapshot_volume,"observed_reduction":e.snapshot_reductions,
                "receipted_volume":e.receipted,"last_snapshot_seq":e.last_snapshot_seq,
                "pending_for_ms":e.gap_since.map(|t|t.elapsed().as_millis() as u64),
                "account_scope":self.session.as_ref().map(|s|&s.scope),
                "runtime_generation":self.session.as_ref().map(|s|s.generation)})
        }).collect();
        rows.sort_by_key(|r|r["identifier"].as_u64());rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session()->ExecutionSession{ExecutionSession{scope:"[42,demo,0,777,XAUUSD]".into(),generation:1}}
    #[test]
    fn volume_expectations_are_session_scoped_and_cannot_be_cleared_by_rollback(){
        let s=session();let mut v=ReceiptVolumes::default();v.remember(&s,1,11,0.08).unwrap();
        v.observe(&s,&[(1,11,0.06)]).unwrap();assert!(v.pending());
        assert!(v.observe(&s,&[(1,11,0.08)]).is_err());assert!(v.pending());
        let mut other=s.clone();other.generation=2;assert!(v.observe(&other,&[]).is_err());
        other=s.clone();other.scope="other-account".into();assert!(v.accept_close(&other,1,0.02).is_err());
        assert!(v.pending());v.accept_close(&s,1,0.02).unwrap();assert!(!v.pending());
        assert!(v.scope_mismatch(Some(&other)));assert!(!v.scope_mismatch(Some(&s)));
    }
    #[test]
    fn event_first_and_snapshot_first_have_same_settled_evidence(){
        for event_first in [false,true]{let s=session();let mut v=ReceiptVolumes::default();v.remember(&s,1,11,0.08).unwrap();
            if event_first {v.accept_close(&s,1,0.02).unwrap();assert!(v.pending());}
            v.observe(&s,&[(1,12,0.06)]).unwrap(); // physical-ticket alias does not change ownership
            if !event_first {assert!(v.pending());v.accept_close(&s,1,0.02).unwrap();}
            assert!(!v.pending());v.observe(&s,&[]).unwrap();assert!(v.pending());
            v.accept_close(&s,1,0.06).unwrap();assert!(!v.pending());
            assert!(v.accept_close(&s,1,0.01).is_err());
        }
    }
    #[test]
    fn malformed_complete_snapshot_does_not_partly_erase_volume_expectations(){
        let s=session();let mut v=ReceiptVolumes::default();v.remember(&s,1,11,0.08).unwrap();v.remember(&s,2,22,0.01).unwrap();
        assert!(v.observe(&s,&[(1,11,0.06),(2,22,f64::NAN)]).is_err());assert!(!v.pending());
        assert!(v.observe(&s,&[(1,11,0.06),(1,12,0.06)]).is_err());assert!(!v.pending());
        v.observe(&s,&[(1,11,0.06),(2,22,0.01)]).unwrap();assert!(v.pending());assert_eq!(v.pending_details().len(),1);
    }
}
