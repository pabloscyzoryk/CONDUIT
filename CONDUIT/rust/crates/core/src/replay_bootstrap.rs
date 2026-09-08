//! Exact decision-state checkpoint for the rules engine. AI/EA require their
//! own state contracts and are explicitly outside this version's qualification.
use super::*;
use crate::recorded_broker::exact::{decode, encode, Exact};
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayBootstrap {
    pub schema: u32,
    pub fields: BTreeMap<String, Exact>,
}

// Every Engine field except the two explicitly unsupported subsystems. This
// list is shared by capture, restore and patch; no defaults are inferred.
macro_rules! decision_fields {
    ($action:ident) => {
        $action!(
            cfg,
            rearm_reconcile,
            pending_sources,
            entry_sources,
            entry_source_aliases,
            entry_source_revision,
            pending_source_revision,
            regime_hist,
            baskets,
            basket_slots,
            basket_slots_len,
            stats,
            logs,
            halted,
            cost_reconciliation_required,
            cost_quarantine,
            risk_override,
            halt_ms,
            halt_prev_ts,
            halt_min_zapisane,
            next_basket_id,
            created_baskets_count,
            entry_source_observations,
            fast_addon_invalid_tp_note,
            order_submission_sequence,
            loss_streak,
            paused_until,
            day_stop,
            kredyt_rozjazd_dzien,
            last_vsl_eval,
            last_tp_hit_ts,
            spread_med,
            spread_buf,
            zmiennosc,
            sr,
            price_hist,
            rezim_miekki,
            slhit_dnia,
            slhit_pauza_do,
            slhit_miekki,
            vol_hist,
            desired,
            last_resize,
            last_relot,
            last_expo,
            limit_cancelled,
            done_actions,
            deferred_entries,
            msg_to_basket,
            opened_today,
            budget_day,
            queued_exits,
            continuation,
            closed_today,
            journal,
            odrzuty,
            odrzucone_wejscia,
            wejscie_w_obrobce,
            zignorowane,
            wygaszanie,
            tryb_auto_ea,
            ea_a,
            slot,
            pulapy,
            obce
        )
    };
}
impl Engine {
    pub fn export_replay_bootstrap(&self) -> Result<ReplayBootstrap, String> {
        if self.cfg.ea_enabled || self.cfg.ai_enabled {
            return Err("unsupported AI/EA decision-state contract".into());
        }
        let mut fields = BTreeMap::new();
        macro_rules! capture {($($name:ident),*)=>{$(fields.insert(stringify!($name).into(),encode(&self.$name).map_err(|e|e.to_string())?);)*};}
        decision_fields!(capture);
        Ok(ReplayBootstrap { schema: 1, fields })
    }
    pub fn from_replay_bootstrap(seed: &ReplayBootstrap) -> Result<Self, String> {
        if seed.schema != 1 {
            return Err("unsupported bootstrap schema".into());
        }
        let cfg: Settings = decode(seed.fields.get("cfg").ok_or("bootstrap settings missing")?)
            .map_err(|e| e.to_string())?;
        let mut engine = Engine::new(cfg, 0.0);
        engine.apply_replay_patch(&seed.fields, true)?;
        if engine.cfg.ea_enabled || engine.cfg.ai_enabled {
            return Err("unsupported AI/EA decision-state contract".into());
        }
        Ok(engine)
    }
    /// Only the offline recorded driver calls this. The patch describes a
    /// witnessed application mutation between engine calls, not a strategy
    /// decision invented by the verifier.
    pub fn apply_replay_patch(
        &mut self,
        patch: &BTreeMap<String, Exact>,
        complete: bool,
    ) -> Result<(), String> {
        let mut used = 0usize;
        macro_rules! restore {($($name:ident),*)=>{$(
            if let Some(value)=patch.get(stringify!($name)) {self.$name=decode(value).map_err(|e|format!("{}: {e}",stringify!($name)))?;used+=1;}
            else if complete {return Err(format!("bootstrap field missing: {}",stringify!($name)));}
        )*};}
        decision_fields!(restore);
        if used != patch.len() {
            return Err("unknown bootstrap/patch field".into());
        }
        if self.cfg.ea_enabled || self.cfg.ai_enabled {
            return Err("unsupported AI/EA decision-state contract".into());
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_checkpoint_preserves_private_memory_and_exact_bits() {
        let mut engine = Engine::new(Settings::default(), 600.0);
        engine.cfg.ea_enabled = false;
        engine.cfg.ai_enabled = false;
        engine.last_vsl_eval = 123;
        engine.paused_until = 567;
        engine.day_stop = 42;
        engine.spread_buf = vec![f64::from_bits(0x3fd3333333333334)];
        engine.desired.insert(
            123,
            DesiredStops {
                sl: Some(3991.0),
                tp: None,
                last_try: 111,
            },
        );
        engine.queued_exits.insert(
            456,
            QueuedExit {
                target: 4001.0,
                deadline: 555,
                reason: CloseReason::Tp,
                market_at_decision: 4000.0,
            },
        );
        let saved = engine.export_replay_bootstrap().unwrap();
        let disk = serde_json::to_vec(&saved).unwrap();
        let restored =
            Engine::from_replay_bootstrap(&serde_json::from_slice(&disk).unwrap()).unwrap();
        assert_eq!(restored.export_replay_bootstrap().unwrap(), saved);
    }
    #[test]
    fn omitted_unknown_or_unsupported_bootstrap_never_defaults_to_success() {
        let mut engine = Engine::new(Settings::default(), 600.0);
        engine.cfg.ea_enabled = false;
        engine.cfg.ai_enabled = false;
        let mut saved = engine.export_replay_bootstrap().unwrap();
        saved.fields.remove("day_stop");
        assert!(Engine::from_replay_bootstrap(&saved).is_err());
        let mut saved = engine.export_replay_bootstrap().unwrap();
        saved.fields.insert("future_state".into(), Exact::Unit);
        assert!(Engine::from_replay_bootstrap(&saved).is_err());
        engine.cfg.ai_enabled = true;
        assert!(engine.export_replay_bootstrap().is_err());
    }
    #[test]
    fn field_inventory_cannot_silently_omit_future_engine_state() {
        let source = include_str!("engine.rs");
        let block = source
            .split("pub struct Engine {")
            .nth(1)
            .unwrap()
            .split("\n}")
            .next()
            .unwrap();
        let actual: std::collections::BTreeSet<_> = block
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("//"))
            .filter_map(|l| {
                l.split_once(':')
                    .map(|(k, _)| k.trim_start_matches("pub ").trim().to_string())
            })
            .collect();
        let mut expected = std::collections::BTreeSet::from(["obs".to_string(), "ea".to_string()]);
        macro_rules! names {($($name:ident),*)=>{$(expected.insert(stringify!($name).to_string());)*};}
        decision_fields!(names);
        assert_eq!(
            actual, expected,
            "new Engine state needs an explicit capture contract"
        );
    }
}
