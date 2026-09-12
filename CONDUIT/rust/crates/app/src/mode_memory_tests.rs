// Included in live::tests, using its real production helpers and synthetic broker.
fn mode_message(id:i64,reply:Option<i64>,edit:bool,ts:i64,text:&str)->IncomingMessage {
    IncomingMessage {ts,source:SourceKey::new(1,None),source_name:"Synergy".into(),
        msg_id:id,reply_to:reply,edit_of:edit.then_some(id),text:text.into()}
}
fn mode_t100()->Engine {
    let mut cfg=Settings::default();cfg.t100.enabled=true;
    let mut e=Engine::new(cfg,1000.0);e.tryb_auto_ea=true;e
}
fn mode_restore(memory:&mut Trwale,ea:bool,broker:&SimBroker)->routing::Silniki {
    mode_memory::select_mode(memory,ea);
    let mut team=jeden(if ea {mode_t100()}else{Engine::new(Settings::default(),1000.0)});
    przenies_pamiec(&mut team,memory,1000.0,0.0);
    restore_strategy_memory(&mut team,memory,broker,
        if memory.mode_fresh {ContinuationOrigin::Fresh}else{ContinuationOrigin::Memory});
    mode_memory::restore_context(&mut team,memory);
    mode_memory::restore_allocator(&mut team,&memory.modes);team
}

#[test]
fn t100_mode_memory_roundtrip_keeps_learning_context_and_strategy_statistics_separate() {
    let mut broker=broker_z_cena();let mut ea=jeden(mode_t100());
    ea.glowny_mut().engine.on_message(&mut broker,&mode_message(1,None,false,10,
        "BUY LIMIT GOLD @ 4000/3995\nSL 3990\nTP 4020"));
    let plan=conduit_core::t100::EntryPlan {decision_id:1,ts:10,side:Side::Buy,expert:0,volume:0.01,
        entry_reference:4000.0,sl:3990.0,tp:4020.0,approved_budget_usd:12.0,risk_usd:10.0,score:0.8,atr:2.0,context_key:None};
    ea.glowny_mut().engine.t100.on_open_result(&plan,conduit_core::t100::ExecutionOutcome::Confirmed,Some(17));
    let closed:conduit_core::types::ClosedTrade=serde_json::from_value(serde_json::json!({
        "ticket":17,"basket":null,"side":"Buy","volume":0.01,"open_price":4000.0,"close_price":4002.0,
        "open_ts":10,"close_ts":20,"profit":2.0,"commission":0.0,"swap":0.0,"reason":"Sl"})).unwrap();
    ea.glowny_mut().engine.t100.on_closed(&closed,2.0);
    ea.glowny_mut().engine.stats.realized_today=2.0;
    ea.glowny_mut().engine.closed_today=vec![2.0];
    ea.glowny_mut().engine.ensure_next_basket_id_floor(9).unwrap();
    let runtime=conduit_core::recorded_broker::exact::encode(&ea.glowny().engine.t100).unwrap();
    let mut memory=Trwale::default();mode_memory::select_mode(&mut memory,true);
    zapamietaj_silniki(&mut memory,&mut ea,1000.0,"");
    let mut auto=mode_restore(&mut memory,false,&broker);
    assert!(auto.glowny().engine.t100_checkpoint().is_none());
    assert_eq!(auto.glowny().engine.stats.realized_today,0.0);
    assert!(auto.glowny().engine.closed_today.is_empty());assert!(auto.koszyki().is_empty());
    assert_eq!(auto.glowny().engine.next_basket_id(),9);
    auto.glowny_mut().engine.stats.realized_today=-17.0;
    auto.glowny_mut().engine.ensure_next_basket_id_floor(14).unwrap();
    zapamietaj_silniki(&mut memory,&mut auto,1000.0,"");
    let restored=mode_restore(&mut memory,true,&broker);
    assert_eq!(conduit_core::recorded_broker::exact::encode(&restored.glowny().engine.t100).unwrap(),runtime);
    assert_eq!(restored.glowny().engine.stats.realized_today,2.0);
    assert_eq!(restored.glowny().engine.closed_today,vec![2.0]);
    assert_eq!(restored.glowny().engine.next_basket_id(),14);
    assert!(restored.glowny().engine.t100_entry_hold_reason().is_none());
}

#[test]
fn t100_mode_memory_passive_parser_matches_active_context_without_orders_or_counters() {
    let mut broker=broker_z_cena();let mut active=mode_t100();let mut passive=mode_t100();
    for message in [
        mode_message(1,None,false,10,"BUY LIMIT GOLD @ 4000/3995\nSL 3990\nTP 4020"),
        mode_message(1,None,true,20,"BUY LIMIT GOLD @ 4000/3995\nSL 3988\nTP 4022"),
        mode_message(2,None,false,30,"SELL LIMIT GOLD @ 4010/4015\nSL 4020\nTP 3990"),
        mode_message(3,Some(1),false,40,"CANCEL"),
    ] {
        active.on_message(&mut broker,&message);
        mode_memory::apply_context(&mut passive,&mode_memory::PassiveEvent::Message{message,received_utc:1});
    }
    assert_eq!(active.t100.context,passive.t100.context);
    assert_eq!(active.t100.context.contexts.len(),2);
    assert_eq!(active.t100.context.active_count(),1);
    assert_eq!(passive.created_baskets_count(),0);assert!(passive.baskets.is_empty());
    assert!(broker.positions().is_empty());assert!(broker.pendings().is_empty());
    assert_eq!(conduit_core::recorded_broker::exact::encode(&passive.stats).unwrap(),
        conduit_core::recorded_broker::exact::encode(&mode_t100().stats).unwrap());
}

#[test]
fn t100_mode_memory_gap_orders_saved_messages_before_quarantine_and_new_versions_after() {
    let broker=broker_z_cena();let mut memory=Trwale::default();mode_memory::select_mode(&mut memory,true);
    let mut team=jeden(mode_t100());zapamietaj_silniki(&mut memory,&mut team,1000.0,"");
    mode_memory::select_mode(&mut memory,false);
    memory.modes.queue(mode_memory::PassiveEvent::Message{message:mode_message(1,None,false,10,
        "BUY LIMIT GOLD @ 4000/3995\nSL 3990\nTP 4020"),received_utc:10});
    memory.modes.queue(mode_memory::PassiveEvent::Gap{reason:"synthetic reconnect".into(),observed_utc:15});
    memory.modes.queue(mode_memory::PassiveEvent::Message{message:mode_message(2,None,false,20,
        "BUY LIMIT GOLD @ 4000/3995\nSL 3990\nTP 4020"),received_utc:20});
    // Dispatch can lag reception: this old queued edit must not renew key 1
    // or quarantine the independent key 2 received after the gap.
    memory.modes.queue(mode_memory::PassiveEvent::Message{message:mode_message(1,None,true,30,
        "BUY LIMIT GOLD @ 4000/3995\nSL 3990\nTP 4020"),received_utc:12});
    let mut restored=mode_restore(&mut memory,true,&broker);
    assert_eq!(restored.glowny().engine.t100.context.contexts.len(),2);
    assert_eq!(restored.glowny().engine.t100.context.quarantined_count(),1);
    assert!(restored.glowny().engine.t100.context.best(&restored.glowny().engine.cfg.t100,40,4000.0,2.0,Side::Buy).unwrap().0.key.ends_with(":2"));
    assert!(restored.glowny().engine.t100_entry_hold_reason().is_none());
    let before=conduit_core::recorded_broker::exact::encode(&restored.glowny().engine.t100).unwrap();
    mode_memory::restore_context(&mut restored,&mut memory);
    assert_eq!(conduit_core::recorded_broker::exact::encode(&restored.glowny().engine.t100).unwrap(),before);
}

#[test]
fn t100_mode_memory_overflow_is_explicit_and_never_revalidates_lost_context() {
    let broker=broker_z_cena();let mut team=jeden(mode_t100());
    mode_memory::apply_context(&mut team.glowny_mut().engine,&mode_memory::PassiveEvent::Message{
        message:mode_message(1,None,false,1,"BUY LIMIT GOLD @ 4000/3995\nSL 3990\nTP 4020"),received_utc:1});
    let mut memory=Trwale::default();mode_memory::select_mode(&mut memory,true);
    zapamietaj_silniki(&mut memory,&mut team,1000.0,"");mode_memory::select_mode(&mut memory,false);
    memory.modes.queue(mode_memory::PassiveEvent::Message{message:mode_message(9,None,false,2,&"x".repeat(8*1024*1024)),received_utc:2});
    assert_eq!(memory.modes.auto_ea.as_ref().unwrap().context_overflow,1);
    assert_eq!(memory.modes.auto_ea.as_ref().unwrap().context_events.len(),1);
    let restored=mode_restore(&mut memory,true,&broker);
    assert_eq!(restored.glowny().engine.t100.context.quarantined_count(),1);
    assert_eq!(restored.glowny().engine.t100.context.active_count(),1);
}

#[test]
fn t100_mode_memory_failed_atomic_save_does_not_commit_mode_or_consume_memory() {
    let st=stan("mode-memory-failed-write");st.update(Sections::all(),|s|s.mode=ui::TradingMode::Auto);
    let team=jeden(Engine::new(Settings::default(),1000.0));let mut memory=Trwale::default();
    assert!(apply_mode_with_memory(&st,&team,&mut memory,true,ui::TradingMode::Auto,ui::TradingMode::AutoEa,"",
        |_|anyhow::bail!("synthetic write failure")).is_err());
    assert_eq!(st.read(|s|s.mode),ui::TradingMode::Auto);assert!(memory.modes.auto_ea.is_none());
    assert!(memory.modes.auto.silniki.is_empty());assert_eq!(memory.modes.revision,0);
    sprzataj(&st);
}

#[test]
fn t100_mode_memory_disk_namespaces_and_legacy_auto_migration_are_account_bound() {
    let st=stan("mode-memory-durable");let a=conduit_mt5::proto::AccountIdent{
        login:42,server:"SYNTHETIC".into(),trade_mode:0,..Default::default()};
    let mut auto=jeden(Engine::new(Settings::default(),1000.0));auto.glowny_mut().engine.stats.realized_today=-3.0;
    let mut modes=mode_memory::ModeMemory::default();modes.capture(&auto,"");
    modes.active_auto_ea=true;modes.auto_ea=Some(Default::default());
    let mut ea=jeden(mode_t100());ea.glowny_mut().engine.stats.realized_today=5.0;
    st.update(Sections::all(),|s|s.halt.ustaw(ui::KlasaHaltu::Ryzyko,"shared account risk"));
    save_follow_memory_modes(&st,&ea,&a,77,"XAUUSD",1100.0,"",&modes).unwrap();
    let saved=read_follow_memory(&st,&a,77,"XAUUSD").unwrap().unwrap();
    assert!(saved.silniki.values().all(|s|s.t100.is_none()));
    assert_eq!(saved.silniki["ATFX"].stats.as_ref().unwrap().realized_today,-3.0);
    assert!(saved.auto_ea.as_ref().unwrap().silniki["ATFX"].t100.is_some());
    let mut memory=Trwale::default();apply_follow_memory(&st,&mut memory,saved);
    assert!(memory.modes.active_auto_ea);assert_eq!(memory.silniki["ATFX"].stats.as_ref().unwrap().realized_today,5.0);
    assert!(matches!(memory.modes.auto_ea.as_ref().unwrap().context_events.last(),Some(mode_memory::PassiveEvent::Gap{..})));
    let b=conduit_mt5::proto::AccountIdent{server:"OTHER".into(),..a.clone()};
    bind_account_risk(&st,&mut memory,&b,77,"XAUUSD").unwrap();assert!(memory.modes.auto_ea.is_none());
    assert!(memory.modes.auto.silniki.is_empty());
    // Existing v1 AUTO maps remain valid when all new optional fields are absent.
    let old=read_follow_memory(&st,&a,77,"XAUUSD").unwrap().unwrap();let mut json=serde_json::to_value(old).unwrap();
    for key in ["auto_ea","auto_baskets","active_auto_ea","basket_allocator"] {json.as_object_mut().unwrap().remove(key);}
    let old:FollowAccountMemory=serde_json::from_value(json).unwrap();let mut migrated=Trwale::default();apply_follow_memory(&st,&mut migrated,old);
    assert!(!migrated.modes.active_auto_ea);assert!(migrated.modes.auto.initialized);assert!(!migrated.mode_baskets_known);
    assert_eq!(migrated.silniki["ATFX"].stats.as_ref().unwrap().realized_today,-3.0);
    sprzataj(&st);
}

#[test]
fn t100_mode_memory_allocator_floor_is_monotonic_and_slot_scoped() {
    let mut e=mode_t100();let before=e.created_baskets_count();
    e.ensure_next_basket_id_floor(9).unwrap();e.ensure_next_basket_id_floor(2).unwrap();
    assert_eq!(e.next_basket_id(),9);assert_eq!(e.created_baskets_count(),before);assert!(e.baskets.is_empty());
    assert!(e.ensure_next_basket_id_floor(conduit_core::wielosilnik::pierwszy_numer(1)).is_err());
    assert_eq!(e.next_basket_id(),9);
}

#[test]
fn t100_mode_memory_midday_activation_and_mode_return_keep_account_loss_limit() {
    let mut broker=SimBroker::new(930.0,0.0,0.0);
    let q=Quote{ts:1_700_000_000_000,bid:4000.0,ask:4000.2};broker.on_quote(q.clone());
    let day=q.ts.div_euclid(86_400_000);
    let mut stats=ui::Stats::new(1000.0,q.ts);stats.day_key=day;stats.day_start_equity=1000.0;stats.peak_equity_today=1000.0;
    let mut first=jeden(mode_t100());
    assert_eq!(first.glowny().engine.cfg.t100.daily_loss_pct,6.0);
    mode_memory::preserve_account_risk(&mut first,&stats,day,930.0);
    first.glowny_mut().engine.on_tick(&mut broker,&q);
    assert_eq!(first.glowny().engine.t100.diagnostics.last_reason,"daily_risk_stop");
    assert!(first.glowny().engine.halted.is_none(),"shared policy lock is not a permanent generic halt");
    let mut memory=Trwale::default();mode_memory::select_mode(&mut memory,true);
    zapamietaj_silniki(&mut memory,&mut first,1000.0,"");
    let mut auto=mode_restore(&mut memory,false,&broker);
    mode_memory::preserve_account_halt(&mut auto,"existing account risk");
    assert_eq!(auto.glowny().engine.halted.as_deref(),Some("existing account risk"));
    zapamietaj_silniki(&mut memory,&mut auto,1000.0,"");
    let mut ea=mode_restore(&mut memory,true,&broker);
    mode_memory::preserve_account_risk(&mut ea,&stats,day,990.0);
    ea.glowny_mut().engine.on_tick(&mut broker,&q);
    assert_eq!(ea.glowny().engine.t100.diagnostics.last_reason,"daily_risk_stop");
    assert_eq!(ea.glowny().engine.stats.realized_today,0.0);
    assert_eq!(stats.day_start_equity,1000.0);
    // A different target configuration evaluates the same actual account path.
    let mut relaxed=jeden(mode_t100());relaxed.glowny_mut().engine.cfg.t100.daily_loss_pct=10.0;
    relaxed.glowny_mut().engine=Engine::new(relaxed.glowny().engine.cfg.clone(),930.0);
    relaxed.glowny_mut().engine.tryb_auto_ea=true;
    mode_memory::preserve_account_risk(&mut relaxed,&stats,day,930.0);
    relaxed.glowny_mut().engine.on_tick(&mut broker,&q);
    assert_ne!(relaxed.glowny().engine.t100.diagnostics.last_reason,"daily_risk_stop");
}

#[test]
fn t100_mode_memory_enabling_ordinary_ea_preset_imports_day_before_feed_and_next_tick() {
    let st=stan("mode-memory-enable-preset");let q=Quote{ts:1_700_000_000_000,bid:4000.0,ask:4000.2};
    let day=q.ts.div_euclid(86_400_000);
    st.update(Sections::all(),|s|{s.mode=ui::TradingMode::AutoEa;s.stats.day_key=day;
        s.stats.day_start_equity=1000.0;s.stats.peak_equity_today=1000.0;});
    let mut core=st.read(|s|live_core_from_ui(&s.settings));
    let mut team=jeden(Engine::new(core.clone(),930.0));team.glowny_mut().engine.tryb_auto_ea=true;
    team.lista[0].preset="ORDINARY_EA".into();
    assert!(team.glowny().engine.t100_checkpoint().is_none());
    assert!(!t100_live_feed_required(true,&team));
    let mut next=core.clone();next.t100.enabled=true;
    st.workspace.save_preset(&conduit_core::Preset{name:"ORDINARY_EA".into(),format:"ATFX".into(),
        description:String::new(),settings:next,ea:None}).unwrap();
    let mut mt=std::collections::HashMap::from([("ORDINARY_EA".into(),std::time::UNIX_EPOCH)]);
    przeladuj_ustawienia_z_brokerem(&st,&mut team,&mut core,0.0,&mut mt,true,Some((day,930.0)));
    assert!(team.glowny().engine.cfg.t100.enabled);
    assert!(team.glowny().engine.t100_checkpoint().is_some());
    assert!(team.glowny().engine.t100_entry_hold_reason().is_none());
    assert!(t100_live_feed_required(true,&team));
    assert!(!t100_live_feed_required(false,&team),"AUTO never subscribes the autonomous feed");
    let mut broker=SimBroker::new(930.0,0.0,0.0);broker.on_quote(q.clone());
    team.glowny_mut().engine.on_tick(&mut broker,&q);
    assert_eq!(team.glowny().engine.t100.diagnostics.last_reason,"daily_risk_stop");
    assert!(broker.positions().is_empty());assert!(broker.pendings().is_empty());
    sprzataj(&st);
}

#[test]
fn t100_mode_memory_disable_reenable_quarantines_only_past_context_and_keeps_learning() {
    let st=stan("mode-memory-disable-reenable");let mut e=mode_t100();let mut broker=broker_z_cena();
    for id in [1,2] {e.on_message(&mut broker,&mode_message(id,None,false,10,
        "BUY LIMIT GOLD @ 4000/3995\nSL 3990\nTP 4020"));}
    let plan=conduit_core::t100::EntryPlan{decision_id:1,ts:10,side:Side::Buy,expert:0,volume:0.01,
        entry_reference:4000.0,sl:3990.0,tp:4020.0,approved_budget_usd:12.0,risk_usd:10.0,score:0.8,atr:2.0,context_key:None};
    e.t100.on_open_result(&plan,conduit_core::t100::ExecutionOutcome::Confirmed,Some(17));
    let closed:conduit_core::types::ClosedTrade=serde_json::from_value(serde_json::json!({
        "ticket":17,"basket":null,"side":"Buy","volume":0.01,"open_price":4000.0,"close_price":4002.0,
        "open_ts":10,"close_ts":20,"profit":2.0,"commission":0.0,"swap":0.0,"reason":"Sl"})).unwrap();
    e.t100.on_closed(&closed,2.0);assert_eq!(e.t100.diagnostics.expert_closed[0],1);
    let before=serde_json::to_value(&e.t100).unwrap();let contexts=e.t100.context.contexts.clone();
    let stats=conduit_core::recorded_broker::exact::encode(&e.stats).unwrap();
    let mut disabled=e.cfg.clone();disabled.t100.enabled=false;
    e.cfg=apply_live_t100_change(&st,&mut e,disabled.clone(),false);
    assert!(e.cfg.t100.enabled);assert_eq!(serde_json::to_value(&e.t100).unwrap(),before);
    e.cfg=apply_live_t100_change(&st,&mut e,disabled,true);
    assert!(!e.cfg.t100.enabled);assert_eq!(e.t100.context.quarantined_count(),2);
    assert_eq!(e.t100.context.contexts,contexts,"quarantine never cancels or deletes pending history");
    let mut actual=serde_json::to_value(&e.t100).unwrap();let mut expected=before;
    actual.as_object_mut().unwrap().remove("context");expected.as_object_mut().unwrap().remove("context");
    assert_eq!(actual,expected,"learning, market state and all runtime counters survive disabling");
    let checkpoint=e.t100_checkpoint().unwrap();let mut restored=Engine::new(e.cfg.clone(),1000.0);restored.tryb_auto_ea=true;
    restored.restore_t100_checkpoint(Some(&checkpoint)).unwrap();
    let mut enabled=restored.cfg.clone();enabled.t100.enabled=true;
    restored.cfg=apply_live_t100_change(&st,&mut restored,enabled,true);
    assert_eq!(restored.t100.context.quarantined_count(),2);
    assert!(restored.t100_entry_hold_reason().is_none(),"old context does not impose a global trading HOLD");
    restored.on_message(&mut broker,&mode_message(3,Some(1),false,30,"SL 3988"));
    assert_eq!(restored.t100.context.quarantined_count(),2,"partial correction does not restore full validity");
    restored.on_message(&mut broker,&mode_message(1,None,true,40,
        "BUY LIMIT GOLD @ 4000/3995\nSL 3990\nTP 4020"));
    assert_eq!(restored.t100.context.quarantined_count(),1);
    assert!(restored.t100.context.best(&restored.cfg.t100,40,4000.0,2.0,Side::Buy).is_some());
    assert_eq!(restored.t100.diagnostics.expert_closed[0],1);
    assert_eq!(conduit_core::recorded_broker::exact::encode(&e.stats).unwrap(),stats);
    assert!(broker.positions().is_empty());assert!(broker.pendings().is_empty());sprzataj(&st);
}
