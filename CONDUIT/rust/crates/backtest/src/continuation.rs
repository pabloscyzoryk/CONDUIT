//! Fresh static simulation only. No MT5 identity, restart or durable checkpoint proof.
use crate::{RunConfig,SimBroker};
use conduit_core::Broker;
use conduit_core::engine::ContinuationOrigin;
use conduit_core::routing::Silniki;

pub(crate) fn static_run_supported(cfg:&RunConfig)->Result<(),String> {
    // Account ownership is authoritative: an ON value shadowed in a strategy
    // preset must not silently activate a different account contract.
    if !cfg.settings.restore_strategy_continuation {return Ok(());}
    if cfg.daily_reset || cfg.flat_na_dobie || !cfg.drabinka.is_empty() {
        return Err("continuation A supports a static fresh simulation only; reset/chain replacement has no continuation proof".into());
    }
    Ok(())
}

pub(crate) fn initialize_fresh(team:&mut Silniki,broker:&mut SimBroker)->Result<Option<String>,String> {
    if !team.lista.iter().any(|s|s.engine.cfg.restore_strategy_continuation) {return Ok(None);}
    // A newly constructed broker has no prior files/history by construction.
    // This proves ONLY model freshness, never freshness of a real account.
    let session=broker.bind_synthetic_continuation_scope()?;
    for i in 0..team.lista.len() {
        if !team.lista[i].engine.cfg.restore_strategy_continuation {continue;}
        let owner=if team.lista[i].format.is_empty(){"SYNTHETIC_SINGLE".to_string()}else{team.lista[i].format.clone()};
        let report=team.z_widokiem(i,broker,|e,b|e.import_strategy_continuation(b,&owner,None,ContinuationOrigin::Fresh));
        if let Some(review)=report.review {return Err(format!("{owner}: {}",review.reason));}
    }
    if broker.execution_session()!=Some(session.clone()) {return Err("synthetic session changed during Fresh initialization".into());}
    Ok(Some(session.scope))
}

pub(crate) fn review_reason(team:&Silniki)->Option<String> {
    team.lista.iter().find_map(|s| {
        if !s.engine.cfg.restore_strategy_continuation {return None;}
        s.engine.continuation_review().map(|r|format!("{}: {}",s.format,r.reason))
            .or_else(||s.engine.continuation_entry_blocked().then(||format!("{}: continuation is uninitialized",s.format)))
    })
}
