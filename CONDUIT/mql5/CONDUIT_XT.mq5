#property copyright "CONDUIT"
#property version   "2.00"
#property strict

#define XT_WERSJA "XT-2026.09.05-CONTRACT"

//====================================================================
//  WEJŚCIA — mapowanie 1:1 z polami presetu
//====================================================================
input string  In_Plik              = "conduit_most.csv"; // most poleceń
input long    In_Magic             = 770077;
input bool    In_MostRequireSchema2= false;    // harness: odmowa starego mostu
input long    In_DiagDetailFromMs = 0;         // tester-only bounded raw request evidence
input long    In_DiagDetailToMs = 0;

// --- zegar / wykonanie ---
input int     In_ExecLatencyMs     = 250;      // exec_latency_ms
input bool    In_LiveTickOrderStrict = false; // live_tick_order_strict; false = legacy XT
input double  In_StopsLevel        = 0.2;      // 0 = weź z brokera

// --- lot ---
input bool    In_LotModePercent    = false;     // lot_mode_percent
input double  In_LotPercent        = 1.0;      // lot_percent
input double  In_LotFixed          = 0.01;     // lot_fixed
input double  In_LotMin            = 0.01;     // lot_min
input double  In_LotMax            = 100.0;    // lot_max
input double  In_LotScaleStep      = 0.0;      // lot_scale_step
input int     In_LotBase           = 0;        // lot_base: 0=Balance 1=Equity 2=MinOfBoth
input double  In_LotMaxZSalda      = 0.0;      // lot_max_z_salda (podstawa/X = sufit; 0=brak)
input bool    In_OdliczKredyt      = false;    // odlicz_kredyt
input double  In_KredytReczny      = 0.0;      // kredyt_reczny (emulacja CREDIT w testerze)

// --- strefa ---
input int     In_ZoneOffsetMode    = 0;        // 0=None 1=Price 2=Directional
input double  In_EntryHiOffset     = 0.0;
input double  In_EntryLoOffset     = 0.0;
input double  In_EntryDeepOffset   = 4.0;      // entry_deep_offset
input double  In_EntryTolOffset    = 0.3;     // entry_tol_offset
input bool    In_SkipIfSlBreached  = true;     // skip_if_sl_breached
input double  In_MaxChaseBeyond    = 0.0;      // max_chase_beyond_zone
input double  In_EntrySlDistLimit  = 0.0;      // entry_sl_dist_limit
input bool    In_OnlyLimitSignals  = false;    // only_limit_signals

// --- siatka ---
input int     In_EntryUnits        = 1;        // entry_units
input int     In_EntryUnitsLimit   = 0;        // entry_units_limit
input bool    In_GridAnchorAbs     = false;     // grid_anchor_absolute
input bool    In_UnitsPerLevel     = true;      // units_per_level
input bool    In_UnitsPerLevelZone = true;      // units_per_level_zone
input bool    In_SyncOnlyLiveLevels= true;      // sync_only_live_levels
input bool    In_PpmEnabled        = false;    // ppm_enabled
input double  In_Ppm               = 1.0;      // ppm
input bool    In_PpmForLimits      = true;     // ppm_for_limits
input double  In_EntryDepthCurve   = 1.0;      // entry_depth_curve
input double  In_EntryAllowanceUsd = 0.0;      // entry_allowance_usd
input int     In_EntryAllowanceUnits=0;        // entry_allowance_units
input bool    In_WeightsFromRR     = false;     // entry_weights_from_rr
input double  In_RRPower           = 1.0;      // entry_weights_rr_power
input double  In_RRCap             = 4.0;     // entry_weights_rr_cap
input string  In_EntryWeights      = "";       // entry_weights
input string  In_EntryUklad        = "";       // entry_uklad (ile pozycji na szczeblu, OD PLYTKIEJ do glebokiej)
input bool    In_EntryJedenNaGleb  = false;    // entry_jeden_na_glebokiej
input int     In_UkladKotwica      = 1;        // entry_uklad_kotwica: 0=Planowany 1=Ocalaly (DOMYSLNIE)
input int     In_KrzywaKotwica     = 0;        // entry_krzywa_kotwica: 0=Ocalaly 1=Planowany
input int     In_TpDrabKotwica     = 0;        // tp_drabinka_kotwica: 0=Ocalaly 1=Planowany
input double  In_EntryWarstwyOffset= 0.0;      // entry_warstwy_offset
input bool    In_EntryWarstwyTekst = false;    // entry_warstwy_z_tekstu
input double  In_RiskPerBasketPct  = 0.0;     // risk_per_basket_pct
input double  In_EntryRiskBudget   = 0.0;      // entry_risk_budget
input double  In_EntryTp1Budget    = 0.0;      // entry_tp1_budget
input int     In_PendingCrossPol   = 0;        // 0=Market 1=Skip 2=Shift 3=Stop
input bool    In_DropUnplaceable   = false;    // drop_unplaceable_levels
input bool    In_AutoLimit         = true;     // auto_limit
input int     In_MarketEntryMode   = 0;        // market_entry_mode: 0=GridAtOnce 1=Single 2=Laddered
input int     In_MarketEntryUnits  = 0;        // market_entry_units (0 = OFF)
input double  In_MaxPortfolioRisk  = 0.0;      // max_portfolio_risk_pct (pozycje+pendingi)
input double  In_ProfitBudgetArmPct = 0.0;     // profit_budget_arm_pct: 0 preserves legacy
input double  In_ProfitBudgetKeepPct = 50.0;   // profit_budget_keep_pct
input double  In_ProfitBudgetDeployPct = 100.0; // profit_budget_deploy_pct
input double  In_VolWindowMin      = 0.0;      // vol_window_min (0=reguła wyłączona)
input double  In_VolRangeUsd       = 15.0;      // vol_range_usd
input double  In_VolUnitsMult      = 0.7;      // vol_units_mult

// --- dławik obsunięcia budżetu nowych koszyków (dd_soft/hard) ---
input double  In_DdSoftPct         = 0.0;      // dd_soft_pct
input double  In_DdSoftMult        = 0.5;      // dd_soft_mult
input double  In_DdHardPct         = 0.0;      // dd_hard_pct
input double  In_DdHardMult        = 0.25;      // dd_hard_mult
input int     In_DdGuardScope      = 0;        // dd_guard_scope: 0=Daily 1=Lifetime 2=LifetimePeakDailyReset

// --- stop ---
input double  In_SlMinDist         = 0.0;      // sl_min_dist
input double  In_SlMaxDist         = 0.0;      // sl_max_dist
input bool    In_AdaptiveParams    = false;    // adaptive_params
input double  In_SlMinDistZoneMult = 0.0;      // sl_min_dist_zone_mult
input double  In_SlMinDistAtrMult  = 0.0;      // sl_min_dist_atr_mult
input double  In_SlMinDistFloor    = 0.0;      // sl_min_dist_floor
input double  In_SlMinDistCap      = 0.0;      // sl_min_dist_cap
input double  In_AdaptAtrWindowMin = 60.0;      // adaptive_atr_window_min (0=60)
input double  In_EntryDeepZoneMult = 0.0;      // entry_deep_zone_mult
input double  In_EntryUnitsZoneRef = 0.0;      // entry_units_zone_ref

// --- cele ---
input int     In_TpSchedule        = 2;        // 0=AllRunners 1=AllAtTp1 2=Ladder 3=OfficialCounts 4=OfficialPct 5=ScaleOutPct
input string  In_OfficialCounts    = "1,1,1";      // official_counts
input string  In_OfficialPct       = "15,30,30,20"; // official_pct
input bool    In_OfficialSpp       = false;    // official_spp (powtarza ostatnią transzę)
input int     In_BankAllAtStage    = 0;        // bank_all_at_stage (0 = OFF)
input bool    In_PendCancelOnRf    = false;    // pending_cancel_on_riskfree
input bool    In_AssignTpPerPos    = true;     // assign_tp_per_position
input double  In_ScaleOutPct       = 30.0;
input int     In_LastRunner        = 2;        // 0=NoTp 1=NextTp 2=Runner
input double  In_TpOpenOffset      = 5.0;      // tp_open_offset
input bool    In_TpOpenExtra       = false;    // tp_open_extra
input int     In_RunnerCeleN       = 0;        // runner_cele_n
input double  In_RunnerCeleKrok    = 10.0;     // runner_cele_krok
input bool    In_CelePominZaCena   = false;    // cele_pomin_za_cena
input bool    In_TpFreezeAfterLad  = true;     // tp_freeze_after_ladder
input bool    In_CeleNaOstatnim    = false;    // cele_na_ostatnim (TYLER)
input bool    In_RetargetRespectsFinal = false; // retarget_respects_final_target
input int     In_BankRounding      = 1;        // 0=Nearest 1=Up 2=Down
input int     In_BankFrom          = 0;        // 0=Worst 1=Best
input bool    In_BankCloseLast     = false;    // bank_close_last
input bool    In_PartialClose      = false;    // partial_close
input double  In_PartialMinLot     = 0.02;     // partial_min_lot
input bool    In_PartialOdPierw    = false;    // partial_pct_od_pierwotnego (TYLER)
input int     In_TpSource          = 2;        // 0=PriceOnly 1=SignalOnly 2=Either 3=SignalConfirmedByPrice 4=PriceFirstSignalWindow
input double  In_TpPriceTol        = 0.30;     // tp_price_tolerance
input double  In_TpPriceFrontRun   = 0.0;      // tp_price_front_run_usd (0=pelny touch legacy)
input double  In_TpSigMaxLeadS     = 0.0;      // tp_signal_max_lead_s
input double  In_TpSigMaxLagS      = 0.0;      // tp_signal_max_lag_s
input bool    In_TpStageFromFill   = true;     // tp_stage_from_broker_fill
input bool    In_TpHitFillStages   = true;     // tp_hit_fill_stages
input bool    In_TpHitMatchLevel   = false;    // tp_hit_match_level (TPHIT z poziomem→etap)
input bool    In_TpUnidxPipsPrice  = false;    // tp_unindexed_pips_require_price
input int     In_PendingLifetime   = 1;        // 0=Never 1=UntilTp1 2=Tp2 3=Tp3
input bool    In_PendingDropOnTgt  = true;     // pending_drop_on_target
input bool    In_PendingDropArm    = false;    // pending_drop_arm
input bool    In_DropRequireTouch  = false;    // pending_drop_require_zone_touch
input double  In_DropGraceMin      = 0.0;      // pending_drop_grace_min
input double  In_DropGraceMaxDist  = 0.0;      // pending_drop_grace_max_dist
input int     In_DropKeepN         = 0;        // pending_drop_keep_n
input double  In_PendingTtlH       = 0.0;      // pending_ttl_h
input bool    In_PendingTtlOdKosz  = true;     // pending_ttl_from_basket
input int     In_SlPolowaOdKonca   = 0;        // sl_polowa_od_konca (TYLER)
input double  In_SlPolowaUlamek    = 0.5;      // sl_polowa_ulamek
input int     In_LadderFromTp      = 0;        // ladder_from_tp
input int     In_LadderLag         = 0;        // ladder_lag
input double  In_LadderOffset      = 0.0;      // ladder_offset

// --- komunikaty kanału ---
input int     In_RiskFreeMode      = 1;        // 0=Ignore 1=CloseAllKeepNearest 2=MoveSlToBeOnly 3=CloseProfitableOnly 4=CloseEverything 5=CloseAllKeepBest
input int     In_RiskFreeRunners   = 1;        // risk_free_runners
input int     In_RfRunnerTarget    = 1;        // 0=KeepTp 1=LastTp 2=NoTpTrailOnly 3=NextTp
input bool    In_RiskFreeTrail     = false;    // risk_free_trail
input int     In_OutAtEntryMode    = 1;        // 0=Ignore 1=CloseAll 2=CloseLosersOnly 3=CloseFlatOnly 4=MoveSlToBe
input int     In_OaePodWoda        = 0;        // oae_pod_woda: 0=NicNieRob 1=Zamknij 2=DociagnijStop
input double  In_OaeBandPts        = 1.0;      // oae_band_pts
input double  In_OaeTimeoutMin     = 0.0;      // oae_timeout_min
input double  In_OaeProfitMin      = 0.5;      // oae_profit_min
input int     In_SlHitMode         = 1;        // 0=Ignore 1=CancelPendings 2=CloseAll 3=VerifyByPrice
input double  In_SlHitVerifyTol    = 0.0;      // sl_hit_verify_tol
input bool    In_HonorCancel       = true;     // honor_cancel
input bool    In_ExplicitPendingUntilCancel = false; // publisher validity for explicit LIMIT/STOP
input bool    In_HonorCloseAll     = true;     // honor_close_all
input bool    In_HonorMarketOpen   = false;    // honor_market_open
input bool    In_HonorStopOrders   = false;    // honor_stop_orders
input double  In_BasketHintTol     = 0.6;      // basket_hint_tolerance
input bool    In_HintVeto          = false;    // hint_veto (Z-3)
input bool    In_ReplyVeto         = false;    // reply_veto
input bool    In_ReplyGraph        = false;    // reply_graph_transitive
input bool    In_DedupEdited       = true;     // dedup_edited_signals
input bool    In_DedupKeyValue     = false;    // dedup_klucz_z_wartoscia (kontrakt mostu)
input bool    In_DedupPelnyStatus  = false;    // dedup_pelny_status (false=legacy: zapisz ignored)
input bool    In_EditRest          = true;     // edycja_wykonuje_reszte_akcji (true=legacy XT)
input bool    In_EditOrphanNoEntry = false;    // edycja_sieroty_nie_otwiera
input bool    In_EntryIdempotency = true;      // entry_idempotencja
input bool    In_DedupMgmtReplay   = false;    // dedup_management_po_restarcie (re-delivery NEW w sesji)
input double  In_RfLevelSanityUsd  = 0.0;      // rf_level_sanity_max_usd
input double  In_SppMaxAgeH        = 12.0;     // spp_max_age_h
input bool    In_SppKeepTp         = false;    // spp_keep_tp
input bool    In_ResetTpOnTargetEdit = true;  // model EA: nowy plan SPP zeruje postep jak Rust; false=legacy EA
input int     In_SppSlMode         = 0;        // spp_sl_mode: 0=Off 1=Stop 2=OnlyIfBetter 3=RunnersOnly 4=RunnersOnlyIfBetter 5=BankersOnly
input double  In_SppSlPad          = 0.0;      // spp_sl_pad
input bool    In_SppArmsRunnerClk  = false;    // spp_arms_runner_clock (Z-10)
input bool    In_BeAtTp1           = false;    // be_at_tp1
input int     In_BeOdEtapu         = 0;        // be_od_etapu (0 = OFF)
input int     In_BeMinPozycji      = 0;        // be_min_pozycji
input bool    In_BeCoversLateFills = false;    // be_covers_late_fills
input bool    In_BeNeverLoosen    = false;    // be_never_loosen
input bool    In_ConfirmedExitRetry = false; // confirmed_exit_retry: Done only after broker-flat
input int     In_TestExitScenario = 0; // TESTER ONLY: 0=OFF, 1..5=confirmed exits, 6=partial receipts, 7/8=edit refusal/golden
input bool    In_SlPoTp1Krawedz    = false;    // sl_po_tp1_na_krawedz
input int     In_NoTpAfterStage    = 0;        // no_tp_after_stage (0 = OFF)

// --- TRAILING S/R PO STRUKTURZE (engine.rs, rodzina trail_sr_*) ---
input bool    In_TrailSrEnabled    = false;    // trail_sr_enabled
input int     In_TrailSrScope      = 0;        // trail_sr_scope: 0=Runner 1=Tp3Up 2=All
input int     In_TrailSrActiv      = 3;        // trail_sr_activation: 0=Entry 1=Gain 2=Tp1 3=Tp2 4=Tp3
input double  In_TrailSrMinGain    = 3.0;      // trail_sr_min_gain
input double  In_TrailSrMinDistP   = 6.0;      // trail_sr_min_dist_price
input double  In_TrailSrOffset     = 0.5;      // trail_sr_offset
input double  In_TrailSrMinDistTp  = 2.0;      // trail_sr_min_dist_tp
input int     In_TrailSrTfMin      = 1;        // trail_sr_tf_min
input int     In_TrailSrFractalN   = 3;        // trail_sr_fractal_n
input int     In_TrailSrWindowH    = 24;       // trail_sr_struct_window_h
input double  In_BeOffset          = 0.0;      // be_offset
input bool    In_SlEditToPendings  = false;    // sl_edit_reaches_pendings (Z-5)
input bool    In_TpCorrToBroker    = false;    // tp_correction_to_broker (Z-8)
input bool    In_ExitOnOpposite    = false;    // exit_on_opposite_signal

// --- SMART SL (drabinka stopów wg rangi pozycji) ---
input int     In_SmartSlMode       = 0;        // 0=Off 1=BreakevenOnly 2=Ladder 3=LadderWithBe
input int     In_SmartSlDelay      = 0;        // smart_sl_delay
input bool    In_SmartSlOnlyAfterRf= false;    // smart_sl_only_after_rf
input bool    In_SmartSlFloorBeRf  = true;     // smart_sl_floor_be_after_rf

// --- BE-LOCK ---
input double  In_BeLockPts         = 0.0;      // be_lock_pts

// --- TRAILING (engine.rs:6309 trail_candidate, 6393 trail_z_parametrow) ---
input int     In_TrailMode         = 0;        // trail_mode: 0=Off 1=Gap 2=LockPct 3=Tiered 4=Atr 5=Chandelier
input double  In_TrailStart        = 25.0;     // trail_start
input double  In_TrailGap          = 20.0;     // trail_gap
input bool    In_TrailAdaptiveEnabled = false;
input bool    In_TrailAdaptiveRunnersOnly = true;
input double  In_TrailAdaptiveWindowS = 90.0;
input int     In_TrailAdaptiveMinSamples = 8;
input double  In_TrailAdaptiveTrendEr = 0.55;
input double  In_TrailAdaptiveReversalEr = 0.45;
input double  In_TrailAdaptiveTrendGapMult = 1.6;
input double  In_TrailAdaptiveChopGapMult = 0.85;
input double  In_TrailAdaptiveReversalGapMult = 0.45;
input double  In_TrailAdaptiveFastVolS = 20.0;
input double  In_TrailAdaptiveSlowVolS = 120.0;
input double  In_TrailAdaptiveVolRatio = 1.8;
input double  In_TrailAdaptiveVolFavorableMult = 1.25;
input double  In_TrailAdaptiveVolAdverseMult = 0.65;
input double  In_TrailAdaptiveMinPeak = 0.0;
input double  In_TrailAdaptiveMinGap = 0.0;
input double  In_TrailAdaptiveMaxGap = 0.0;
input double  In_TrailLockPct      = 50.0;     // trail_lock_pct
input string  In_TrailTiers        = "5:1,10:5,15:9,20:14,30:23,50:42";       // trail_tiers "prog:blokada,..."
input int     In_TrailRunnerMode   = 3;        // trail_runner_mode (domyślna silnika: Tiered)
input double  In_TrailRunnerStart  = 5.0;      // trail_runner_start
input double  In_TrailRunnerGap    = 8.0;      // trail_runner_gap
input double  In_TrailRunnerLockPct= 50.0;     // trail_runner_lock_pct
input string  In_TrailRunnerTiers  = "5:1,10:4,20:12,35:26,60:50,100:88";       // trail_runner_tiers
input bool    In_TrailSplit        = false;    // trail_split
input int     In_TrailRunnersN     = 1;        // trail_runners_n
input bool    In_TrailRunByDepth   = false;    // trail_runners_by_depth
input double  In_TrailMinDist      = 0.0;      // trail_min_dist
input double  In_TrailAtrMult      = 0.0;      // trail_atr_mult
// FEATURE XT (nie ma odpowiednika w silniku; wyrosl z bledu portu, ktory
// okazal sie sensowna regula): kazda pozycja BEZ celu (runner siatki
// z official_counts/last_runner=NoTp) jest runnerem trailingu takze bez
// przejscia przez RISK FREE. Domyslnie OFF = parytet z silnikiem.
input bool    In_RunnerBezCelu     = false;    // XT-FEATURE: no-TP = runner trailingu

// --- RISK-FREE jako reguła silnika (engine.rs:7226) ---
input bool    In_RfEnabled         = false;    // riskfree_enabled
input double  In_RfTriggerUsd      = 0.0;      // riskfree_trigger_usd
input double  In_RfTriggerR        = 0.0;      // riskfree_trigger_r
input int     In_RfKeepUnits       = 1;        // riskfree_keep_units
input double  In_RfBeOffset        = 0.0;      // riskfree_be_offset
input int     In_RfRunnerStop      = 0;        // riskfree_runner_stop: 0=Be 1=BeOwn 2=TrailGap 3=Off
input int     In_RfRunnerTarget2   = 1;        // riskfree_runner_target (reguły): 0=KeepTp 1=LastTp 2=NoTpTrailOnly 3=NextTp
input double  In_RfRunnerGap       = 15.0;     // riskfree_runner_gap
input double  In_RfRunnerMaxHoldM  = 90.0;     // riskfree_runner_max_hold_min
input bool    In_RfMaxHoldRuleOnly = false;    // runner_max_hold_rule_only (Z-10)

// --- WYJŚCIA UZNANIOWE (manage_positions engine.rs:5977) ---
input double  In_ExitMinHoldMin    = 0.0;      // exit_min_hold_min
input double  In_ExitMinProfit     = 0.0;      // exit_min_profit
input double  In_HoldAfterTpHitMin = 0.0;      // hold_after_tp_hit_min
input double  In_ExitRMultiple     = 0.0;      // exit_r_multiple
input double  In_ExitRoundDist     = 0.0;      // exit_round_dist
input double  In_ExitRoundStep     = 10.0;      // exit_round_step
input double  In_ExitSpreadMult    = 0.0;      // exit_spread_mult
input bool    In_SmartExit         = false;    // smart_exit
input double  In_SmartExitTake     = 0.0;      // smart_exit_take
input double  In_SmartExitGiveback = 0.3;      // smart_exit_giveback (ułamek 0..1)
input double  In_SmartExitMinPeak  = 4.0;      // smart_exit_min_peak
input double  In_SmartExitDropSpd  = 0.0;      // smart_exit_drop_speed ($/min)
input double  In_SmartExitSpdWinS  = 60.0;      // smart_exit_speed_window_s
input double  In_SmartExitHoldPend = 2.0;      // smart_exit_hold_if_pending ($ dystans)
input int     In_SmartExitMinPend  = 1;        // smart_exit_min_pendings
input int     In_SmartExitPendScope= 0;        // smart_exit_pending_scope: 0=SameBasket 1=AnyBasket
input double  In_SmartExitPendDist = 0.3;      // smart_exit_pending_min_dist
input double  In_HarvestRetracePct = 0.0;      // harvest_retrace_pct
input double  In_HarvestStart      = 8.0;      // harvest_start
input double  In_StaleTakeMin      = 0.0;      // stale_take_min
input double  In_StaleTakeProfit   = 15.0;      // stale_take_profit
input double  In_StaleTakeMin2     = 0.0;      // stale_take_min2
input double  In_StaleTakeProfit2  = 35.0;      // stale_take_profit2
input double  In_RevExitRange      = 0.0;      // rev_exit_range
input double  In_RevExitSlope      = 14.0;      // rev_exit_slope
input double  In_RevExitProfit     = 4.0;      // rev_exit_profit
input double  In_RevExitWindowMin  = 60.0;     // rev_exit_window_min
input double  In_BasketTargetUsd   = 0.0;      // basket_target_usd
input bool    In_ExitViaLimit      = false;
input double  In_ExitLimitOffset   = 0.0;      // exit_limit_offset
input double  In_ExitLimitWaitS    = 60.0;      // exit_limit_wait_s
input double  In_ExitLimitMinProfit= 0.0;      // exit_limit_min_profit
input double  In_ZoneExitAdverseS  = 0.0;      // zone_exit_adverse_s (ujemna=lustro)
input bool    In_ZoneExitAdvClose  = false;    // zone_exit_adverse_close

// --- WIRTUALNY SL ---
input bool    In_VirtualSl         = false;    // virtual_sl
input bool    In_VirtualSlAll      = false;    // virtual_sl_all
input bool    In_VslOnlyWhenRej    = true;    // virtual_sl_only_when_rejected
input double  In_VslEvalS          = 0.0;      // vsl_eval_s
input double  In_VslBrokerOffset   = 0.0;      // vsl_broker_offset

// --- filtry / limity ---
input bool    In_SessionFilter     = false;     // session_filter
input string  In_SessionHours      = "7-20";   // session_hours
input int     In_MaxOpenPositions  = 0;        // max_open_positions
input bool    In_EnforcePosLimit   = false;    // enforce_position_limit_on_fill
input int     In_MaxOpenBaskets    = 0;        // max_open_baskets
input bool    In_ExposureCountPend = false;    // exposure_count_pendings
input double  In_ExpoBonusProfitPct= 0.0;      // exposure_bonus_profit_pct
input int     In_ExpoBonusPositions= 0;        // exposure_bonus_positions
input int     In_ExpoBonusBaskets  = 0;        // exposure_bonus_baskets
input double  In_MaxDirectionalLots= 0.0;      // max_directional_lots
input int     In_StreakPauseN      = 0;        // streak_pause_n
input double  In_StreakPauseMin    = 60.0;     // streak_pause_min
// --- HAMULEC SL-HIT (engine.rs:415-419, 1812-1839, 9064-9072) ---
// Liczy KAZDY komunikat SL HIT kanalu w dobie; po n-tym pauzuje nowe wejscia.
input int     In_SlhitPauseN       = 0;        // slhit_pause_n (0 = wylaczony)
input double  In_SlhitPauseMin     = 0.0;      // slhit_pause_min (0 = do konca doby)
input int     In_RegimeFilter      = 0;        // 0=Off 1=TrendMa 2=CounterMa
input double  In_RegimeMaHours     = 72.0;     // regime_ma_hours
// --- WYCISZENIE REZIMU (settings.rs:435-451, 1800-1804, 1823-1886) ---
// Gdy zakres okna glownego przekracza prog, filtr rezimu NIE MA ZDANIA.
input double  In_RegimeZmiennoscMax= 0.0;      // regime_range_mute_usd (0 = wylaczone)
input int     In_RegimeGdyRozerwany= 0;        // regime_range_mute_mode: 0=Milcz/Pass 1=KrotkieOkno 2=Miekko/Soft
input bool    In_RegimeSoft        = false;    // regime_soft (werdykt odmowny -> wejscie miekkie zamiast odrzucenia)
input double  In_RegimeSoftLotMult = 1.0;      // regime_soft_lot_mult
input double  In_RegimeSoftRiskMult= 1.0;      // regime_soft_risk_mult
input double  In_RegimeSoftUnitsMult=1.0;      // regime_soft_units_mult
input int     In_RegimeSoftMaxPos  = 0;        // regime_soft_max_positions (0 = bez zmiany)
input int     In_SideFilter        = 0;        // 0=Both 1=BuyOnly 2=SellOnly
input double  In_MarginCallPct     = 50.0;     // margin_call_level_pct
input double  In_EquityFloorPct    = 0.0;      // equity_floor_pct
input double  In_BasketMaxAgeMin   = 0.0;    // basket_max_age_min
input bool    In_WiekOdWypelnienia = false;    // wiek_od_wypelnienia
input double  In_IgnoreOldAfterMin = 0.0;      // ignore_old_after_min
input int     In_DailySignalBudget = 0;        // daily_signal_budget
input bool    In_MergeSameSide     = false;    // merge_same_side
input double  In_MergeWindowMin    = 20.0;      // merge_window_min
input double  In_MergeMinOverlap   = 0.5;      // merge_min_overlap (0..1)
input double  In_SignalMinRR       = 0.0;      // signal_min_rr
input double  In_SignalMinZoneW    = 0.0;      // signal_min_zone_width
input double  In_SignalMaxZoneW    = 0.0;      // signal_max_zone_width
input bool    In_TrendFilterOn     = false;    // trend_filter_enabled
input double  In_TrendFilterWinH   = 24.0;      // trend_filter_window_h
input double  In_TrendFilterDropPct= 0.0;      // trend_filter_drop_pct
input int     In_TrendFilterMode   = 1;        // trend_filter_mode: 0=Block 1=Shrink
input double  In_TrendFilterShrink = 0.5;      // trend_filter_shrink (mnożnik jednostek)

// --- STRAŻNICY DNIA / KONTA (check_guards engine.rs:5645) ---
input double  In_MaxDdPct          = 0.0;      // max_dd_pct
input double  In_MaxDdUsd          = 0.0;      // max_dd_usd
input bool    In_UsdScaleWithLot   = false;    // usd_scale_with_lot
input double  In_DayTargetUsd      = 0.0;      // day_target_usd
input double  In_DayTargetPct      = 0.0;      // day_target_pct
input double  In_DayGateOdSalda    = 0.0;      // day_gate_od_salda
input double  In_DayGateDoSalda    = 0.0;      // day_gate_do_salda
input bool    In_DayTargetClose    = false;    // day_target_close
input bool    In_DayTargetScaleLot = false;    // day_target_scale_lot
input double  In_DayTrailStopUsd   = 0.0;      // day_trail_stop_usd
input double  In_DayTrailStopPct   = 0.0;      // day_trail_stop_pct
input double  In_DayTrailArmPct    = 0.0;      // day_trail_arm_pct
input int     In_DayTrailBasis     = 0;        // 0=EquityPeak, 1=ProfitPeak
input double  In_EodFlatHour       = 0.0;      // eod_flat_hour
input bool    In_FlatWeekend       = false;    // flat_weekend
input double  In_FlatWeekendHour   = 20.0;      // flat_weekend_hour

// --- BRAMKI MARGINESU (rodzina ml_*, engine.rs:6975-7019) ---
input bool    In_MlLiczWiszace     = false;    // ml_licz_wiszace
input double  In_MlMinWejscie      = 0.0;      // ml_min_wejscie
input double  In_MlMinWarstwa      = 0.0;      // ml_min_warstwa
input double  In_MlMinReentry      = 0.0;      // ml_min_reentry
input double  In_MlMinRearm        = 0.0;      // ml_min_rearm
input double  In_MlMinPiramida     = 0.0;      // ml_min_piramida
input double  In_MlMinFastAddon    = 0.0;      // ml_min_fast_addon
input double  In_MlMinRelotUp      = 0.0;      // ml_min_relot_up
input double  In_MlMinDrabina      = 0.0;      // ml_min_drabina

// --- EXPO CAP (redukuj_ekspozycje engine.rs:7021) ---
input double  In_ExpoCapPct        = 0.0;      // expo_cap_pct
input bool    In_ExpoCapClose      = false;    // expo_cap_close
input double  In_ExpoCapS          = 0.0;      // expo_cap_s
input double  In_ExpoCapMlPct      = 0.0;      // expo_cap_ml_pct
input double  In_KontoDzwignia     = 0.0;      // konto_dzwignia (0=dźwignia konta)

// --- filtr tempa ---
input double  In_FastFillRejectS   = 0.0;    // fast_fill_reject_s
input int     In_FastFillLayers    = 3;        // fast_fill_layers
input double  In_FastFillSoftAgeM  = 0.0;     // fast_fill_soft_age_min

// --- PIRAMIDA (handle_tp_hit engine.rs:3947) ---
input int     In_PyramidAfterStage = 0;        // pyramid_after_stage
input double  In_PyramidLotMult    = 1.0;      // pyramid_lot_mult
input int     In_PyramidRegimeLb   = 0;        // pyramid_regime_lookback
input double  In_PyramidRegMaxFast = 30.0;      // pyramid_regime_max_fast_pct
input double  In_PyramidMinEqMult  = 0.0;      // pyramid_min_equity_mult

// --- DOKŁADKA TEMPOWA (fast_addon_sweep engine.rs:7967) ---
input double  In_FastAddonMoveUsd  = 0.0;      // fast_addon_move_usd
input double  In_FastAddonWindowS  = 60.0;      // fast_addon_window_s
input int     In_FastAddonMax      = 1;        // fast_addon_max
input double  In_FastAddonLotMult  = 1.0;      // fast_addon_lot_mult
input int     In_FastAddonMinStage = 0;        // fast_addon_min_stage
input double  In_FastAddonCooldownS= 60.0;      // fast_addon_cooldown_s

// --- REARM (rearm_pass engine.rs:8221) ---
input bool    In_RearmGridOnReturn = false;    // rearm_grid_on_return
input bool    In_RearmKeepEmpty    = false;    // rearm_keep_empty_alive
input bool    In_RearmBlockSecured = false;    // rearm_block_after_secured
input bool    In_SppBlockRearmFlat = false;    // spp_blocks_rearm_when_flat
input bool    In_RearmBezPozycji   = false;    // rearm_bez_pozycji
input double  In_RearmBezPozMaxH   = 6.0;      // rearm_bez_pozycji_max_h
input double  In_RearmMinBasketPl  = 0.0;      // rearm_min_basket_profit
input int     In_RearmMaxTimes     = 1;        // rearm_max_times (0=bez limitu)
input double  In_RearmMinGapMin    = 15.0;      // rearm_min_gap_min

// --- powtórne wejścia ---
input bool    In_ReenterAfterTp    = false;     // reenter_after_tp
input int     In_ReenterMinTpStage = 1;        // reenter_min_tp_stage
input int     In_ReenterMax        = 0;        // reenter_max (0=BEZ LIMITU!)
input double  In_MarketEntryStep   = 1.0;      // market_entry_step
input bool    In_PpmForMarket      = false;    // ppm_for_market
input double  In_ReenterMinRetS    = 0.0;      // reenter_min_return_s
input bool    In_ReenterRespectCap = false;    // reenter_respect_cap
input bool    In_ReenterStopAfterRf= false;    // reenter_stop_after_riskfree
input double  In_SltpRetryS        = 3.0;      // sltp_retry_s

// --- RELOT ZLECEN OCZEKUJACYCH (engine.rs:6539) ---
input bool    In_RelotOnBalance    = false;    // pending_relot_on_balance
input bool    In_RelotTopup        = false;    // pending_relot_topup
input bool    In_RelotWgPlanu      = true;    // pending_relot_wg_planu
input bool    In_RelotUp           = true;     // pending_relot_up
input bool    In_RelotDown         = true;     // pending_relot_down
input double  In_RelotUpOdSalda    = 0.0;      // pending_relot_up_od_salda
input double  In_PendingResizeS    = 30.0;     // pending_resize_s (kadencja)

// --- BRAMKI KAPITAŁOWE *_small (engine.rs:896-1027; próg vs saldo STARTOWE) ---
input int     In_EntryUnitsSmall   = 1;        // entry_units_small
input double  In_EntryUnitsSmallM  = 0.0;      // entry_units_small_mult
input double  In_RiskPerBSmall     = 0.0;      // risk_per_basket_pct_small
input double  In_RiskPerBSmallM    = 0.0;      // risk_per_basket_pct_small_mult
input int     In_ReenterMaxSmall   = 0;        // reenter_max_small
input double  In_ReenterMaxSmallM  = 0.0;      // reenter_max_small_mult
input int     In_MaxPosSmall       = 0;        // max_open_positions_small
input double  In_MaxPosSmallM      = 0.0;      // max_open_positions_small_mult
input int     In_MaxBaskSmall      = 0;        // max_open_baskets_small
input double  In_MaxBaskSmallM     = 0.0;      // max_open_baskets_small_mult
input double  In_BasketAgeSmall    = 0.0;      // basket_max_age_min_small
input double  In_BasketAgeSmallM   = 0.0;      // basket_max_age_min_small_mult
input double  In_FfSoftAgeSmall    = 0.0;      // fast_fill_soft_age_min_small
input double  In_FfSoftAgeSmallM   = 0.0;      // fast_fill_soft_age_min_small_mult
input double  In_MktStepSmall      = 1.0;      // market_entry_step_small
input double  In_MktStepSmallM     = 0.0;      // market_entry_step_small_mult
input double  In_SlMinDistSmall    = 0.0;      // sl_min_dist_small
input double  In_SlMinDistSmallM   = 0.0;      // sl_min_dist_small_mult
input double  In_LotPercentSmall   = 1.0;      // lot_percent_small
input double  In_LotPercentSmallM  = 0.0;      // lot_percent_small_mult

// --- TRYB DZIENNY (odpowiednik --daily-reset; rozbieg dla filtra reżimu) ---
input string  In_DzienOd           = "";       // RRRR.MM.DD HH:MM (puste = bez ograniczen)
input string  In_DzienDo           = "";       // RRRR.MM.DD HH:MM
input bool    In_Diag              = false;    // zrzut śladu do pliku
input string  In_DiagFile          = "conduit_xt_diag.csv"; // relative Common/Files path; isolate parallel experiments


//====================================================================
//  STAŁE
//====================================================================
#define MAXTP     32
#define MAXLV     40
#define MAXTK     80
#define MAXB      600
#define XAU_CONTRACT 100.0

// stany koszyka — odpowiednik BasketState
#define ST_PENDING  0
#define ST_WORKING  1
#define ST_RISKFREE 2
#define ST_DONE     3

//====================================================================
//  MOST POLECEŃ (wczytany raz w OnInit)
//====================================================================
struct Msg
  {
   long     ts;            // już w zegarze ticków (bez opóźnienia wykonania)
   long     msg_id;
   long     reply_to;
   long     edit_of;
   string   hints;         // wskazówki cenowe, rozdzielone przecinkiem
   string   akcje[12];     // surowe napisy akcji
   int      n;
  };
Msg      g_msg[];
int      g_nmsg = 0;
int      g_mi   = 0;       // indeks pierwszej NIEPRZETWORZONEJ wiadomości
int      g_most_schema = 1;
bool     g_most_dedup_value = false;
bool     g_most_contract_seen = false;
string   g_most_channel = "";

//====================================================================
//  KOSZYK
//====================================================================
// Raw accepted EntrySignal, before target filtering, runner expansion or SL
// management. Mirrors core entry_edit::same_source, including optional fields.
struct NativeEntryPlanSource
  {
   bool known;
   int side;
   bool is_limit,is_stop;
   double lo,hi,sl;
   bool has_sl,tp_open;
   double warstwy_offset;
   bool has_warstwy_offset;
   int ntp;
   double tps[MAXTP];
  };

struct Basket
  {
   int      id;
   long     msg_id;
   int      side;          // 0 = BUY, 1 = SELL
   bool     is_limit;
   bool     is_stop;
   bool     source_explicit;
   bool     source_withdrawn;
   bool     entry_review; // session-retained: unresolved legacy edit may not add risk
   bool     realized_review; // confirmed close geometry missing; block additional risk
   bool     realized_owner_missing;
   NativeEntryPlanSource entry_source;
   double   review_requested_lo, review_requested_hi;
   double   entry_lo, entry_hi;   // strefa Z SYGNAŁU (przed offsetami)
   double   zone_lo,  zone_hi;    // strefa po offsetach
   double   sl;
   bool     has_sl;
   double   tps[MAXTP];
   int      ntp;
   bool     tp_open;
   double   warstwy_offset; // wartość jawnie podana w treści ENTRY2
   bool     has_warstwy_offset;
   long     created_ts;
   int      state;
   int      tp_stage;
   int      plan_observed_stage; // target touched without a filled position; never consumes a future partial
   bool     had_positions;
   bool     zone_touched;
   bool     drop_armed;
   bool     secured;
   bool     rearm_blocked_by_spp;
   long     secured_ts;    // chwila zabezpieczenia (RF/SPP/reguła)
   long     be_ts;         // chwila komendy BE; osłania późniejsze fille
   bool     secured_by_rule;
   long     last_tp_ts;
   int      reentries;
   double   last_entry_px;
   bool     has_last_entry;
   double   age_limit_min;   // skrócone życie z filtru tempa (0 = brak)
   bool     tempo_checked;
   bool     tempo_fast;
   bool     pyramided;       // piramida zrobiona (engine: bk.pyramided)
   int      fast_addons;
   long     last_addon_ts;
   long     adverse_since;   // zone_exit_adverse
   int      rearms;
   long     last_rearm_ts;
   long     drop_po_ts;      // okno łaski kasowania siatki (0 = brak terminu)
   double   realized;        // zysk zrealizowany koszyka (informacyjnie)
   bool     exit_pending;    // durable for this tester session; never re-open this basket
   long     exit_last_attempt;
   string   exit_reason;     // first exit intent wins, including retries
   long     tp_touch_ts[MAXTP]; // pierwsze dotknięcie celu ceną (TpSource okna)
   long     tphit_sig_ts[MAXTP];// czas komunikatu TPHIT o tym celu (SignalConfirmedByPrice)

   // --- plan siatki ---
   double   lv_price[MAXLV];
   double   lv_vol[MAXLV];
   double   lot_planu;       // lot bazowy planu (RELOT wg planu)
   double   lv_tp[MAXLV];
   bool     lv_has_tp[MAXLV];
   int      lv_units[MAXLV];
   long     lv_fill_ts[MAXLV];
   bool     lv_filled[MAXLV];
   bool     lv_cancelled[MAXLV];
   int      nlv;

   // --- bilety ---
   ulong    pend[MAXTK];  int pend_lv[MAXTK];  int npend;
   bool     pend_top[MAXTK];   // dokladka relotu (is_topup)
   ulong    pos[MAXTK];   int pos_lv[MAXTK];   int npos;
  };
Basket   g_b[MAXB];
int      g_nb = 0;
struct NativeBasketResult
  {
   int id;
   long source_id;
   double profit;
   int closes;
  };
int      g_next_id = 1;
// Fault injection is inert by default and rejected outside the MQL tester.
int      g_test_exit_stage = 0, g_test_close_reject = 0, g_test_cancel_reject = 0;
int      g_test_partial_remaining = 0;
bool     g_test_cancel_until_fill = false, g_test_saw_fill = false, g_test_saw_partial = false;
bool     g_test_exit_finished = false;
long     g_test_exit_started = 0, g_test_exit_requested = 0;
ulong    g_test_exit_ticket = 0, g_test_reference_ticket = 0;

//====================================================================
//  STAN GLOBALNY SILNIKA
//====================================================================
double   g_stops = 0.20;
long     g_now = 0;            // znacznik bieżącego ticka (ms)
double   g_bid = 0, g_ask = 0;

// historia rynku (filtr reżimu) — jeden punkt na godzinę, engine.rs:5340
double   g_ph_px[1000];
long     g_ph_ts[1000];
int      g_nph = 0;

// bufor zmienności — jeden punkt na 5 s (engine vol_hist), cięty po czasie
#define MAXVH 20000
double   g_vh_px[MAXVH];
long     g_vh_ts[MAXVH];
int      g_vh_head = 0;   // indeks najstarszego
int      g_vh_n = 0;
long     g_vh_last = 0;

// pauza po serii strat
long     g_dzien_od = 0, g_dzien_do = 0;   // 0 = brak ograniczenia
bool     g_doba_zamknieta = false;
double   g_min_equity = 1e18;
double   g_eq_hi = -1e18, g_eq_lo = 1e18;  // EQ_STAT doby docelowej
long     g_paused_until = 0;
int      g_loss_streak  = 0;

// HAMULEC SL-HIT — engine.rs:415-419. Licznik komunikatow SL HIT kanalu w
// biezacej dobie i termin konca pauzy wejsc. LONG_MIN = brak pauzy,
// LONG_MAX = pauza do granicy doby (zdejmuje ja RolkaDoby).
int      g_slhit_dnia     = 0;
long     g_slhit_pauza_do = LONG_MIN;
long     g_rej_slhit      = 0;      // telemetria odrzucen hamulca
// WYCISZENIE REZIMU — engine.rs:414. Flaga zyje od bramki rezimu do konca
// obslugi wejscia; czytaja ja UnitsBase/RiskPerBasketEff/MaxOpenPositionsEff/LotSize.
bool     g_rezim_miekki   = false;
long     g_wyciszen       = 0;      // telemetria wejsc miekkich

// STATYSTYKI DNIA / KONTA (odpowiednik Stats silnika)
double   g_start_balance = 0;      // saldo startowe przebiegu
double   g_peak_equity   = 0;      // szczyt equity (Lifetime)
double   g_day_start_eq  = 0;      // equity na starcie doby
double   g_day_peak_eq   = 0;      // szczyt equity dnia
long     g_day           = -9999999; // indeks bieżącej doby (day_of)
long     g_day_stop      = -999999999; // Z-2: doba zamknięta dla wejść
string   g_halted        = "";     // powód zatrzymania (max_dd); "" = handluje
int      g_opened_today  = 0;      // daily_signal_budget
long     g_budget_day    = -9999999;
long     g_last_tp_hit_ts = 0;     // hold_after_tp_hit_min
long     g_last_vsl_eval = 0;
long     g_last_expo     = 0;
long     g_last_relot    = 0;

// mediana spreadu (exit_spread_mult) — 512 próbek jak w silniku
double   g_spread_buf[512];
int      g_spread_n = 0;
double   g_spread_med = 0;

// historia reżimu tempa (przeloty) — cap 100, dla bramki piramidy
bool     g_regime_hist[100];
long     g_regime_hist_ts[100];
int      g_nregime = 0;

// pamięć akcji wykonanych przez wiadomość (dedup edycji)
long     g_done_msg[4000];
string   g_done_key[4000];
int      g_ndone = 0;

// mapa msg_id -> basket
long     g_map_msg[];
int      g_map_bid[];
struct NativeEntrySource { long original; int basket; bool cancelled; bool had_edit; };
NativeEntrySource g_sources[];
long g_source_alias_msg[];
int g_source_alias_record[];
int g_nsources=0, g_nsource_alias=0;
long g_source_message=0, g_source_original=0;
bool g_source_edit=false;
int      g_nmap = 0;

// TRWALY rejestr pozycja -> koszyk (wyniki per koszyk po zamknięciu)
#define MAXREJ 20000
ulong    g_rej_tk[MAXREJ];
int      g_rej_bid[MAXREJ];
long     g_rej_msg[MAXREJ];
double   g_rej_booked[MAXREJ]; // confirmed cumulative OUT PnL, including partials
bool     g_rej_realized_review[MAXREJ];
int      g_nrej = 0;

// ---- STAN PER POZYCJA (odpowiednik pól Position silnika) ----
// peak_pts / last_peak_ts / vsl / is_runner / wol_pierwotny.
// MT5 nie trzyma tych pól — rejestr po tickecie, sprzątany przy zamknięciu.
#define MAXPS 3000
ulong    g_ps_tk[MAXPS];
double   g_ps_peak[MAXPS];
long     g_ps_peak_ts[MAXPS];
double   g_ps_vsl[MAXPS];      // 0 = brak
bool     g_ps_isrunner[MAXPS];
double   g_ps_wol0[MAXPS];     // wolumen pierwotny (TYLER); 0 = niezapisany
double   g_ps_last_vol[MAXPS]; // last reconciled broker volume
int      g_nps = 0;

int PsIdx(ulong t)
  {
   for(int i = g_nps - 1; i >= 0; i--) if(g_ps_tk[i] == t) return i;
   return -1;
  }
int PsEnsure(ulong t)
  {
   int i = PsIdx(t);
   if(i >= 0) return i;
   if(g_nps >= MAXPS)
     { // recykling wpisów po martwych pozycjach
      int w = 0;
      for(int j = 0; j < g_nps; j++)
         if(PositionSelectByTicket(g_ps_tk[j]))
           { g_ps_tk[w]=g_ps_tk[j]; g_ps_peak[w]=g_ps_peak[j]; g_ps_peak_ts[w]=g_ps_peak_ts[j];
             g_ps_vsl[w]=g_ps_vsl[j]; g_ps_isrunner[w]=g_ps_isrunner[j]; g_ps_wol0[w]=g_ps_wol0[j];
             g_ps_last_vol[w]=g_ps_last_vol[j]; w++; }
      g_nps = w;
      if(g_nps >= MAXPS) return -1;
     }
   i = g_nps; g_nps++;
   g_ps_tk[i] = t; g_ps_peak[i] = 0.0; g_ps_peak_ts[i] = g_now;
   g_ps_vsl[i] = 0.0; g_ps_isrunner[i] = false; g_ps_wol0[i] = 0.0;
   g_ps_last_vol[i] = PositionSelectByTicket(t) ? PositionGetDouble(POSITION_VOLUME) : 0.0;
   return i;
  }
void PsForget(ulong t)
  {
   int i = PsIdx(t);
   if(i < 0) return;
   for(int j = i; j < g_nps - 1; j++)
     { g_ps_tk[j]=g_ps_tk[j+1]; g_ps_peak[j]=g_ps_peak[j+1]; g_ps_peak_ts[j]=g_ps_peak_ts[j+1];
       g_ps_vsl[j]=g_ps_vsl[j+1]; g_ps_isrunner[j]=g_ps_isrunner[j+1]; g_ps_wol0[j]=g_ps_wol0[j+1];
       g_ps_last_vol[j]=g_ps_last_vol[j+1]; }
   g_nps--;
  }

#define MAXQE 200
ulong    g_qe_tk[MAXQE];
double   g_qe_target[MAXQE];
long     g_qe_deadline[MAXQE];
double   g_qe_mkt[MAXQE];
string   g_qe_reason[MAXQE];
int      g_nqe = 0;
int QeIdx(ulong t) { for(int i = 0; i < g_nqe; i++) if(g_qe_tk[i] == t) return i; return -1; }
void QeForget(int i)
  {
   for(int j = i; j < g_nqe - 1; j++)
     { g_qe_tk[j]=g_qe_tk[j+1]; g_qe_target[j]=g_qe_target[j+1]; g_qe_deadline[j]=g_qe_deadline[j+1];
       g_qe_mkt[j]=g_qe_mkt[j+1]; g_qe_reason[j]=g_qe_reason[j+1]; }
   g_nqe--;
  }

int      g_handle_diag = INVALID_HANDLE;
long     g_cnt_sig = 0, g_cnt_basket = 0, g_cnt_order = 0, g_cnt_reject = 0;
long     g_rej_session = 0, g_rej_regime = 0, g_rej_streak = 0;
long     g_rej_maxpos = 0, g_rej_maxbask = 0, g_rej_slbreach = 0, g_rej_risk = 0;
long     g_rej_broker = 0, g_rej_daystop = 0, g_rej_budget = 0, g_rej_jakosc = 0;
long     g_rej_trend = 0, g_rej_ml = 0, g_rej_margincall = 0, g_rej_floor = 0;
long     g_merges = 0;
// PREMIA WYPEŁNIEŃ (decyzja D3): suma (poziom−fill)·wolumen·100 po limitach
double   g_premia_usd = 0.0;
int      g_premia_n = 0, g_premia_lepiej = 0;
// telemetria nowych reguł
long     g_expo_zdarzen = 0, g_expo_pend_skas = 0, g_expo_poz_domk = 0;
long     g_cnt_trail_mod = 0, g_cnt_belock = 0, g_cnt_smart_sl = 0;
long     g_cnt_harvest = 0, g_cnt_stale = 0, g_cnt_smartexit = 0, g_cnt_vsl = 0;
long     g_cnt_rf_rule = 0, g_cnt_rf_maxhold = 0, g_cnt_piramida = 0;
long     g_cnt_rearm = 0, g_cnt_fastaddon = 0, g_cnt_revexit = 0, g_cnt_oae_timeout = 0;
long     g_cnt_zoneexit = 0, g_cnt_enforce = 0, g_cnt_ttl = 0, g_cnt_grace = 0;
int      g_kod[40]; long g_kod_n[40]; int g_nkod = 0;
long     g_open_request_count = 0, g_open_request_cap_exceeded = 0;
double   g_open_request_max_volume = 0.0, g_open_accepted_max_volume = 0.0;

// Measure the final volume transmitted to the broker, including rejected
// requests. Closed partial volumes cannot establish that an order cap held.
void AuditOpenRequest(const double volume)
  {
   g_open_request_count++;
   g_open_request_max_volume = MathMax(g_open_request_max_volume, volume);
   if(In_LotMax > 0.0 && volume > In_LotMax + 1e-9) g_open_request_cap_exceeded++;
  }
ENUM_ORDER_TYPE_FILLING g_fill_deal    = ORDER_FILLING_FOK;
ENUM_ORDER_TYPE_FILLING g_fill_pending = ORDER_FILLING_RETURN;
long     g_zamk_blad = 0, g_mod_blad = 0;
long     g_rej_place = 0, g_rej_modify = 0, g_retry_fail = 0;

// ZAMIARY SL/TP (kolejka ponowień try_modify/retry_stops — engine.rs:361/384)
#define MAXZAM 400
ulong    g_zam_tk[MAXZAM];
double   g_zam_sl[MAXZAM], g_zam_tp[MAXZAM];
bool     g_zam_hsl[MAXZAM], g_zam_htp[MAXZAM];
long     g_zam_ts[MAXZAM];
int      g_nzam = 0;
void ZliczOdrzucenie(int kod)
  {
   for(int i = 0; i < g_nkod; i++) if(g_kod[i] == kod) { g_kod_n[i]++; return; }
   if(g_nkod < 40) { g_kod[g_nkod] = kod; g_kod_n[g_nkod] = 1; g_nkod++; }
  }

//====================================================================
//  NARZĘDZIA
//====================================================================
double RoundLot(double v) { return MathRound(v * 100.0) / 100.0; }

double VolMin()
  {
   double v = SymbolInfoDouble(_Symbol, SYMBOL_VOLUME_MIN);
   return v > 0.0 ? v : 0.01;
  }

double VolStep()
  {
   double v = SymbolInfoDouble(_Symbol, SYMBOL_VOLUME_STEP);
   return v > 0.0 ? v : 0.01;
  }

int VolDigits(double step)
  {
   double x = step;
   for(int d = 0; d <= 8; d++)
     {
      if(MathAbs(x - MathRound(x)) <= 1e-9) return d;
      x *= 10.0;
     }
   return 8;
  }

double NormVol(double v)
  {
   double step = VolStep();
   double vmin = VolMin();
   double vmax = SymbolInfoDouble(_Symbol, SYMBOL_VOLUME_MAX);
   if(vmax <= 0.0) vmax = 1e100;
   double r = MathRound(v / step) * step;
   if(r < vmin) r = vmin;
   if(r > vmax) r = vmax;
   return NormalizeDouble(r, VolDigits(step));
  }

// Najblizszy wykonalny partial (remis w gore), bez zamkniecia calego ticketu
// i bez resztki ponizej SYMBOL_VOLUME_MIN. 0.01/0.01 idzie doslownie starym
// wzorem BankOnTp, co utrzymuje zgodnosc historycznych przebiegow.
double PartialCloseVolume(double current, double desired)
  {
   if(!MathIsValidNumber(current) || !MathIsValidNumber(desired) ||
      current <= 0.0 || desired <= 0.0) return 0.0;
   double vmin = VolMin(), step = VolStep();
   if(MathAbs(vmin - 0.01) <= 1e-12 && MathAbs(step - 0.01) <= 1e-12)
     {
      double want = MathRound(desired * 100.0) / 100.0;
      double cut = MathMin(MathMax(want, 0.01), MathMax(current - 0.01, 0.0));
      return cut >= 0.01 - 1e-9 ? cut : 0.0;
     }

   double nearest = MathFloor(desired / step + 0.5 + 1e-12) * step;
   double max_cut = MathFloor(MathMax(current - vmin, 0.0) / step + 1e-12) * step;
   double cut = NormalizeDouble(MathMin(MathMax(nearest, vmin), max_cut), VolDigits(step));
   while(cut >= vmin - 1e-9 && current - cut < vmin - 1e-9)
      cut = NormalizeDouble(MathMax(cut - step, 0.0), VolDigits(step));
   if(cut < vmin - 1e-9 || cut >= current - 1e-9 || current - cut < vmin - 1e-9)
      return 0.0;
   return cut;
  }

// Live Rust's SymbolInfo::round_price executes before the Python MT5 request.
// NormalizeDouble has different half-cent behavior and can move a stop one tick.
double NormPx(double p)
  {
   double factor = MathPow(10.0, (int)SymbolInfoInteger(_Symbol, SYMBOL_DIGITS));
   return MathRound(p * factor) / factor;
  }

int    SideSign(int side) { return side == 0 ? 1 : -1; }
double EntryPx(int side)  { return side == 0 ? g_ask : g_bid; }
double ExitPx(int side)   { return side == 0 ? g_bid : g_ask; }
double MidPx()            { return (g_bid + g_ask) * 0.5; }
double BetterEdge(int side, double lo, double hi) { return side == 0 ? lo : hi; }
double WorseEdge (int side, double lo, double hi) { return side == 0 ? hi : lo; }
// Side::better(a,b) = a jest LEPSZĄ ceną niż b (dla BUY niżej, dla SELL wyżej)
bool   SideBetter(int side, double a, double b2) { return side == 0 ? (a < b2) : (a > b2); }

// broker.rs — dokładne odpowiedniki predykatów wykonalności
bool SlIsValid(int side, double sl)
  { return side == 0 ? (sl <= g_bid - g_stops) : (sl >= g_ask + g_stops); }
bool TpIsValid(int side, double tp)
  { return side == 0 ? (tp >= g_bid + g_stops) : (tp <= g_ask - g_stops); }
bool LimitPxIsValid(int side, double p)
  { return side == 0 ? (p <= g_ask - g_stops) : (p >= g_bid + g_stops); }
bool StopPxIsValid(int side, double p)
  { return side == 0 ? (p >= g_ask + g_stops) : (p <= g_bid - g_stops); }
double ClampLimitPx(int side, double p)
  { return side == 0 ? MathMin(p, g_ask - g_stops) : MathMax(p, g_bid + g_stops); }

int HourOf(long ts_ms) { return (int)(((ts_ms % 86400000) + 86400000) % 86400000 / 3600000); }
long DayOf(long ts_ms) { long d = ts_ms / 86400000; if(ts_ms < 0 && ts_ms % 86400000 != 0) d--; return d; }
// weekday: 0=poniedziałek (types.rs — epoka 1970-01-01 to czwartek=3)
int WeekdayOf(long ts_ms) { long d = DayOf(ts_ms); return (int)(((d + 3) % 7 + 7) % 7); }

void Diag(string s)
  {
   if(!In_Diag || g_handle_diag == INVALID_HANDLE) return;
   FileWrite(g_handle_diag, (string)g_now, s);
  }

void DiagKoszyk(int bi, string co)
  {
   if(!In_Diag || g_handle_diag == INVALID_HANDLE) return;
   string lv = "";
   for(int i = 0; i < g_b[bi].nlv; i++)
      lv += StringFormat("%s%.2f@%.2fx%d%s", (i > 0 ? " " : ""),
                         g_b[bi].lv_price[i], g_b[bi].lv_vol[i], g_b[bi].lv_units[i],
                         g_b[bi].lv_has_tp[i] ? StringFormat("/tp%.2f", g_b[bi].lv_tp[i]) : "/-");
   string tp = "";
   for(int i = 0; i < g_b[bi].ntp; i++) tp += StringFormat("%s%.2f", (i > 0 ? " " : ""), g_b[bi].tps[i]);
   FileWrite(g_handle_diag, co, (string)g_now, (string)g_b[bi].id, (string)g_b[bi].msg_id,
             (g_b[bi].side == 0 ? "BUY" : "SELL"),
             StringFormat("%.2f", g_b[bi].zone_lo), StringFormat("%.2f", g_b[bi].zone_hi),
             StringFormat("%.2f", g_b[bi].sl), tp, (string)g_b[bi].tp_stage, lv,
             StringFormat("%.2f", g_bid), StringFormat("%.2f", g_ask));
  }

void ZapiszWlasciciela(ulong t, int bi)
  {
   // Reconciliation may rediscover a known ticket on every tick. Registration
   // must be idempotent or it exhausts the ledger and duplicates result rows.
   for(int i = g_nrej - 1; i >= 0; i--)
      if(g_rej_tk[i] == t && g_rej_bid[i] == g_b[bi].id) return;
   if(g_nrej >= MAXREJ) return;
   g_rej_tk[g_nrej] = t; g_rej_bid[g_nrej] = g_b[bi].id; g_rej_msg[g_nrej] = g_b[bi].msg_id;
   g_rej_booked[g_nrej] = 0.0;
   g_rej_realized_review[g_nrej] = false;
   g_nrej++;
  }

// Book only the new broker-confirmed amount. A losing partial belongs to the
// basket while the residual position is still alive; its later final close
// must not book that partial a second time. This runs at receipt reconciliation
// before management/re-entry, matching the engine's drain_closed boundary.
bool NativeStrategyRealized(int side,double open_price,double close_price,double volume,
                            double swap,double broker_net,bool net_mode,double &value)
  {
   if(net_mode)
     { if(!MathIsValidNumber(broker_net))return false;value=broker_net;return true; }
   if((side!=0 && side!=1) || !MathIsValidNumber(open_price) || open_price<=0.0
      || !MathIsValidNumber(close_price) || close_price<=0.0
      || !MathIsValidNumber(volume) || volume<=0.0 || !MathIsValidNumber(swap))return false;
   // Legacy strategy basis matches SimBroker PricePlusSwap; broker cash and
   // the exported deal ledger remain the actual confirmed MT5 amounts.
   value=(close_price-open_price)*SideSign(side)*XAU_CONTRACT*volume+swap;
   return MathIsValidNumber(value);
  }
bool NativeRealizedBarrier(int bi,ulong ticket,string why)
  {
   bool found=false;
   for(int i=0;i<g_nrej;i++)if(g_rej_tk[i]==ticket && g_rej_bid[i]==g_b[bi].id)
     {g_rej_realized_review[i]=true;found=true;}
   if(!found)g_b[bi].realized_owner_missing=true;
   if(!g_b[bi].realized_review && In_Diag && g_handle_diag!=INVALID_HANDLE)
      FileWrite(g_handle_diag,"STRATEGY_REALIZED_BARRIER",(string)g_now,(string)g_b[bi].id,
                (string)ticket,why);
   g_b[bi].realized_review=true;
   return false;
  }
bool ReconcilePositionRealized(int bi, ulong t, double &last_price, bool &was_tp)
  {
   int ri = -1;
   for(int i = g_nrej - 1; i >= 0; i--)
      if(g_rej_tk[i] == t && g_rej_bid[i] == g_b[bi].id) { ri = i; break; }
   if(ri < 0 || !HistorySelectByPosition(t))return NativeRealizedBarrier(bi,t,"history_or_owner_missing");
   double entry_price=0.0,entry_volume=0.0;
   int entry_side=-1,entries=0;
   for(int d=0;d<HistoryDealsTotal();d++)
     {
      ulong dt=HistoryDealGetTicket(d);long entry,type;
      if(dt==0 || !HistoryDealGetInteger(dt,DEAL_ENTRY,entry))return NativeRealizedBarrier(bi,t,"entry_read");
      if(entry!=DEAL_ENTRY_IN)continue;
      if(++entries!=1 || !HistoryDealGetInteger(dt,DEAL_TYPE,type)
         || !HistoryDealGetDouble(dt,DEAL_PRICE,entry_price)
         || !HistoryDealGetDouble(dt,DEAL_VOLUME,entry_volume)
         || (type!=DEAL_TYPE_BUY && type!=DEAL_TYPE_SELL))return NativeRealizedBarrier(bi,t,"entry_geometry");
      entry_side=type==DEAL_TYPE_BUY ? 0 : 1;
     }
   if(entries!=1 || !MathIsValidNumber(entry_price) || entry_price<=0.0
      || !MathIsValidNumber(entry_volume) || entry_volume<=0.0)
      return NativeRealizedBarrier(bi,t,"entry_geometry_missing");
   double total = 0.0;
   double closed_volume=0.0;
   last_price = 0.0; was_tp = false;
   for(int d = 0; d < HistoryDealsTotal(); d++)
     {
      ulong dt = HistoryDealGetTicket(d);
      long entry = HistoryDealGetInteger(dt, DEAL_ENTRY);
      if(entry != DEAL_ENTRY_OUT && entry != DEAL_ENTRY_OUT_BY) continue;
      double close_price,volume,swap,value;
      if(!HistoryDealGetDouble(dt,DEAL_PRICE,close_price)
         || !HistoryDealGetDouble(dt,DEAL_VOLUME,volume)
         || !HistoryDealGetDouble(dt,DEAL_SWAP,swap)
         || !NativeStrategyRealized(entry_side,entry_price,close_price,volume,swap,0.0,false,value))
         return NativeRealizedBarrier(bi,t,"close_geometry_or_swap_missing");
      closed_volume+=volume;
      if(closed_volume>entry_volume+1e-9)return NativeRealizedBarrier(bi,t,"close_volume_exceeds_entry");
      total += value;
      last_price = close_price;
      if((ENUM_DEAL_REASON)HistoryDealGetInteger(dt, DEAL_REASON) == DEAL_REASON_TP) was_tp = true;
     }
   if(!MathIsValidNumber(total))return NativeRealizedBarrier(bi,t,"strategy_total_overflow");
   double delta = total - g_rej_booked[ri];
   g_b[bi].realized += delta;
   g_rej_booked[ri] = total;
   g_rej_realized_review[ri]=false;
   g_b[bi].realized_review=g_b[bi].realized_owner_missing;
   for(int i=0;i<g_nrej;i++)
      if(g_rej_bid[i]==g_b[bi].id && g_rej_realized_review[i])g_b[bi].realized_review=true;
   if(MathAbs(delta) > 1e-10 && In_Diag && g_handle_diag != INVALID_HANDLE)
      FileWrite(g_handle_diag, "REALIZED_RECEIPT", (string)g_now, (string)g_b[bi].id,
                (string)t, DoubleToString(delta, 8), DoubleToString(g_b[bi].realized, 8));
   return true;
  }

//====================================================================
//  WCZYTANIE MOSTU (rozpoznaje oba formaty: z polem kanal i bez)
//====================================================================
bool WczytajMost()
  {
   int h = FileOpen(In_Plik, FILE_READ | FILE_TXT | FILE_ANSI | FILE_COMMON);
   if(h == INVALID_HANDLE)
      h = FileOpen(In_Plik, FILE_READ | FILE_TXT | FILE_ANSI);
   if(h == INVALID_HANDLE)
     {
      Print("BLAD: nie moge otworzyc mostu ", In_Plik, " err=", GetLastError());
      return false;
     }
   ArrayResize(g_msg, 20000);
   g_nmsg = 0;
   g_most_schema = 1;
   g_most_dedup_value = false;
   g_most_contract_seen = false;
   while(!FileIsEnding(h))
     {
      string line = FileReadString(h);
      if(StringLen(line) < 3) continue;
      if(StringGetCharacter(line, 0) == '#')
        {
         if(StringFind(line, "# CONTRACT ") == 0)
           {
            g_most_contract_seen = true;
            string contract[];
            int fields=StringSplit(line,' ',contract);
            g_most_schema=0;
            for(int ci=0; ci<fields; ci++)
              {
               if(contract[ci]=="schema=1") g_most_schema=1;
               if(contract[ci]=="schema=2") g_most_schema=2;
               if(contract[ci]=="dedup_value=1") g_most_dedup_value=true;
              }
            if(g_most_schema==0)
              {
               Print("BLAD KONTRAKTU MOSTU: unsupported or missing schema version.");
               FileClose(h);
               return false;
              }
           }
         continue;
        }
      string p[];
      int k = StringSplit(line, '|', p);
      if(p[0] != "M") continue;
      if(k < 7)
        {
         Print("BLAD KONTRAKTU MOSTU: incomplete message record.");
         FileClose(h);
         return false;
        }
      if(g_nmsg >= ArraySize(g_msg)) ArrayResize(g_msg, g_nmsg + 5000);
      g_msg[g_nmsg].ts       = (long)StringToInteger(p[1]);
      if(g_msg[g_nmsg].ts<=0 || (g_nmsg>0 && g_msg[g_nmsg].ts<g_msg[g_nmsg-1].ts))
        {
         Print("BLAD KONTRAKTU MOSTU: invalid or non-monotonic message timestamp.");
         FileClose(h);
         return false;
        }
      g_msg[g_nmsg].msg_id   = (long)StringToInteger(p[2]);
      g_msg[g_nmsg].reply_to = (long)StringToInteger(p[3]);
      g_msg[g_nmsg].edit_of  = (long)StringToInteger(p[4]);
      g_msg[g_nmsg].hints    = p[5];
      // pole 6: nazwa kanału (nowy format, bez dwukropka) albo pierwsza akcja
      int start = 6;
      if(k > 6 && StringFind(p[6], ":") < 0)
        {
         start = 7;
         if(g_most_channel=="")g_most_channel=p[6];
         else if(g_most_channel!=p[6])
           {Print("BLAD KONTRAKTU MOSTU: XT requires one source channel per experiment.");FileClose(h);return false;}
        }
      int n = 0;
      for(int i = start; i < k; i++)
        {
         if(StringLen(p[i]) == 0) continue;
         if(n>=12)
           {
            Print("BLAD KONTRAKTU MOSTU: message exceeds supported action capacity; no truncation allowed.");
            FileClose(h);
            return false;
           }
         g_msg[g_nmsg].akcje[n] = p[i];
         n++;
        }
      g_msg[g_nmsg].n = n;
      if(n > 0) g_nmsg++;
     }
   FileClose(h);
   ArrayResize(g_msg, g_nmsg);
   if(In_MostRequireSchema2 && (!g_most_contract_seen || g_most_schema < 2))
     {
      Print("BLAD KONTRAKTU MOSTU: ekspert wymaga schema=2, plik ma schema=", g_most_schema);
      return false;
     }
   if(g_most_contract_seen && g_most_dedup_value != In_DedupKeyValue)
     {
      Print("BLAD KONTRAKTU MOSTU: dedup_value w pliku=", g_most_dedup_value,
            " ale In_DedupKeyValue=", In_DedupKeyValue);
      return false;
     }
   PrintFormat("most: %d wiadomosci z %s", g_nmsg, In_Plik);
   return g_nmsg > 0;
  }

//====================================================================
//  MAPA WIADOMOŚĆ -> KOSZYK
//====================================================================
void MapPut(long msg, int bid)
  {
   for(int i = 0; i < g_nmap; i++)
      if(g_map_msg[i] == msg) { g_map_bid[i] = bid; return; }
   if(g_nmap>=ArraySize(g_map_msg)) {ArrayResize(g_map_msg,g_nmap+512);ArrayResize(g_map_bid,g_nmap+512);}
   g_map_msg[g_nmap] = msg; g_map_bid[g_nmap] = bid; g_nmap++;
  }
int MapGet(long msg)
  {
   for(int i = g_nmap - 1; i >= 0; i--)
      if(g_map_msg[i] == msg) return g_map_bid[i];
   return -1;
  }

int NativeSourceFind(long message)
  {
   for(int i=g_nsource_alias-1;i>=0;i--)
      if(g_source_alias_msg[i]==message)return g_source_alias_record[i];
   return -1;
  }
void NativeSourceAlias(long message,int record)
  {
   if(message==0 || record<0)return;
   for(int i=g_nsource_alias-1;i>=0;i--)
      if(g_source_alias_msg[i]==message){g_source_alias_record[i]=record;return;}
   if(g_nsource_alias>=ArraySize(g_source_alias_msg))
     {ArrayResize(g_source_alias_msg,g_nsource_alias+512);ArrayResize(g_source_alias_record,g_nsource_alias+512);}
   g_source_alias_msg[g_nsource_alias]=message;g_source_alias_record[g_nsource_alias++]=record;
   if(g_sources[record].basket>=0)MapPut(message,g_sources[record].basket);
  }
int NativeSourceEnsure(long original)
  {
   int i=NativeSourceFind(original);if(i>=0)return i;
   if(g_nsources>=ArraySize(g_sources))ArrayResize(g_sources,g_nsources+512);
   i=g_nsources++;g_sources[i].original=original;g_sources[i].basket=MapGet(original);
   g_sources[i].cancelled=false;g_sources[i].had_edit=false;
   NativeSourceAlias(original,i);return i;
  }
void NativeSourceAccept(long message,int basket)
  {
   long original=message==g_source_message ? g_source_original : message;
   int i=NativeSourceEnsure(original);g_sources[i].basket=basket;
   if(message==g_source_message && g_source_edit)g_sources[i].had_edit=true;
   MapPut(original,basket);MapPut(message,basket);NativeSourceAlias(message,i);
   for(int a=0;a<g_nsource_alias;a++)if(g_source_alias_record[a]==i)MapPut(g_source_alias_msg[a],basket);
  }
bool NativeSourceEntryBlocked(long original,bool is_new)
  {
   int i=NativeSourceFind(original);
   return i>=0 && (g_sources[i].cancelled || (is_new && g_sources[i].had_edit));
  }
void NativeObserveSourceReply(int mi)
  {
   if(g_msg[mi].reply_to==0)return;
   bool cancel=false,entry=false;
   for(int a=0;a<g_msg[mi].n;a++)
     {
      string action=g_msg[mi].akcje[a];
      if(StringFind(action,"ENTRY:")==0 || StringFind(action,"ENTRY2:")==0 || StringFind(action,"MKT:")==0)entry=true;
      if(StringFind(action,"CANCEL:")==0)cancel=true;
     }
   if(entry || (!cancel && (In_EditOrphanNoEntry || !In_ReplyGraph)))return;
   int i=NativeSourceEnsure(g_msg[mi].reply_to);
   if(cancel)g_sources[i].cancelled=true;
   NativeSourceAlias(g_msg[mi].msg_id,i);
  }
bool NativeCompleteRecovery(int side,bool is_limit,bool is_stop,double lo,double hi,
                            double sl,bool has_sl,bool tp_open,double offset,bool has_offset,double &tps[],int ntp)
  {
   if(!MathIsValidNumber(lo) || !MathIsValidNumber(hi) || lo<=0 || hi<=0 || lo>hi || (is_limit&&is_stop))return false;
   if(!has_sl || !MathIsValidNumber(sl) || sl<=0 || (side==0 ? sl>=lo : sl<=hi))return false;
   if(ntp==0 && !tp_open)return false;
   double edge=side==0 ? lo : hi;
   for(int i=0;i<ntp;i++)if(!MathIsValidNumber(tps[i]) || tps[i]<=0 || (tps[i]-edge)*SideSign(side)<=0)return false;
   return !has_offset || (MathIsValidNumber(offset) && offset>=0);
  }
int BIdx(int id)
  {
   for(int i = g_nb - 1; i >= 0; i--) if(g_b[i].id == id) return i;
   return -1;
  }
bool Alive(int i) { return g_b[i].state != ST_DONE; }

bool ConfirmedExitPending(int bi)
  {
   return In_ConfirmedExitRetry && bi >= 0 && bi < g_nb && g_b[bi].exit_pending;
  }
bool ExitRiskAllowed(int bi)
  {
   return !ConfirmedExitPending(bi);
  }
bool ExplicitPendingSource(int bi)
  {
   return bi >= 0 && bi < g_nb && In_ExplicitPendingUntilCancel
          && (g_b[bi].source_explicit || g_b[bi].is_limit || g_b[bi].is_stop);
  }
bool SourceWithdrawn(int bi)
  {
   return bi >= 0 && bi < g_nb && g_b[bi].source_withdrawn;
  }
bool EntryReviewBlocked(int bi)
  {
   return bi >= 0 && bi < g_nb && (g_b[bi].entry_review || g_b[bi].realized_review);
  }
bool KeepExplicitPending(int bi)
  {
   return ExplicitPendingSource(bi) && !SourceWithdrawn(bi);
  }

// Exact ownership only. Unknown/contradictory managed objects prevent proof
// of flat; never guess from price or from the newest basket.
int ExitMergeOwner(int owner, int candidate)
  {
   if(candidate < 0) return owner;
   if(owner == -1 || owner == candidate) return candidate;
   return -2;
  }
int ExitOwner(ulong ticket, ulong identifier, string comment)
  {
   int owner = -1;
   for(int bi = 0; bi < g_nb; bi++)
     {
      for(int i = 0; i < g_b[bi].npos; i++)
         if(g_b[bi].pos[i] == ticket || (identifier > 0 && g_b[bi].pos[i] == identifier))
            owner = ExitMergeOwner(owner, bi);
      for(int i = 0; i < g_b[bi].npend; i++)
         if(g_b[bi].pend[i] == ticket || (identifier > 0 && g_b[bi].pend[i] == identifier))
            owner = ExitMergeOwner(owner, bi);
     }
   for(int i = 0; i < g_nrej; i++)
      if(g_rej_tk[i] == ticket || (identifier > 0 && g_rej_tk[i] == identifier))
         for(int bi = 0; bi < g_nb; bi++)
            if(g_b[bi].id == g_rej_bid[i]) owner = ExitMergeOwner(owner, bi);
   // All native opens use B<id>. Accept only a complete numeric id (not B1
   // matching B10); no arbitrary broker text substring inference.
   if(StringLen(comment) > 1 && StringSubstr(comment, 0, 1) == "B")
     {
      bool numeric = true;
      for(int c = 1; c < StringLen(comment); c++)
        {
         ushort ch = StringGetCharacter(comment, c);
         if(ch < 48 || ch > 57) { numeric = false; break; }
        }
      if(numeric)
        {
         int id = (int)StringToInteger(StringSubstr(comment, 1));
         for(int bi = 0; bi < g_nb; bi++)
            if(g_b[bi].id == id) owner = ExitMergeOwner(owner, bi);
        }
     }
   return owner;
  }
bool ExitOwnedSnapshot(int bi, ulong &positions[], ulong &orders[])
  {
   ArrayResize(positions, 0); ArrayResize(orders, 0);
   bool complete = true;
   int np = PositionsTotal(), no = OrdersTotal();
   for(int i = 0; i < np; i++)
     {
      ulong t = PositionGetTicket(i);
      if(t == 0 || !PositionSelectByTicket(t)) { complete = false; continue; }
      if(PositionGetString(POSITION_SYMBOL) != _Symbol || PositionGetInteger(POSITION_MAGIC) != In_Magic) continue;
      int owner = ExitOwner(t, (ulong)PositionGetInteger(POSITION_IDENTIFIER), PositionGetString(POSITION_COMMENT));
      if(owner < 0) { complete = false; continue; }
      if(owner == bi) { int n = ArraySize(positions); ArrayResize(positions, n + 1); positions[n] = t; }
     }
   for(int i = 0; i < no; i++)
     {
      ulong t = OrderGetTicket(i);
      if(t == 0 || !OrderSelect(t)) { complete = false; continue; }
      if(OrderGetString(ORDER_SYMBOL) != _Symbol || OrderGetInteger(ORDER_MAGIC) != In_Magic) continue;
      int owner = ExitOwner(t, 0, OrderGetString(ORDER_COMMENT));
      if(owner < 0) { complete = false; continue; }
      if(owner == bi) { int n = ArraySize(orders); ArrayResize(orders, n + 1); orders[n] = t; }
     }
   if(np != PositionsTotal() || no != OrdersTotal()) complete = false;
   return complete;
  }
bool NativeEnsureBasketCapacity(string entry_kind)
  {
   if(g_nb<MAXB)return true;
   // Resolve ownership before moving any slot. A partially moved table would
   // itself make the old and new indices appear to own the same ticket.
   bool retain[MAXB];
   for(int i=0;i<g_nb;i++)
     {
      retain[i]=Alive(i) || g_b[i].exit_pending || g_b[i].entry_review || g_b[i].realized_review;
      if(!retain[i])
        {
         ulong positions[],orders[];
         retain[i]=!ExitOwnedSnapshot(i,positions,orders)
                   || ArraySize(positions)>0 || ArraySize(orders)>0;
        }
     }
   int write=0;
   for(int i=0;i<g_nb;i++)if(retain[i])
     {if(write!=i)g_b[write]=g_b[i];write++;}
   g_nb=write;
   if(g_nb<MAXB)return true;
   g_cnt_reject++;
   PrintFormat("NATIVE_CAPACITY_REJECT kind=%s retained=%d limit=%d",entry_kind,g_nb,MAXB);
   if(In_Diag && g_handle_diag!=INVALID_HANDLE)
      FileWrite(g_handle_diag,"NATIVE_CAPACITY_REJECT",(string)g_now,entry_kind,(string)g_nb,(string)MAXB);
   return false;
  }
void ExitRememberLivePositions(int bi, ulong &positions[])
  {
   for(int i = 0; i < ArraySize(positions); i++)
     {
      ulong t = positions[i]; bool found = false;
      for(int k = 0; k < g_b[bi].npos; k++) if(g_b[bi].pos[k] == t) { found = true; break; }
      if(found) continue;
      if(g_b[bi].npos < MAXTK)
        {
         int k = g_b[bi].npos++;
         g_b[bi].pos[k] = t; g_b[bi].pos_lv[k] = -1;
         ZapiszWlasciciela(t, bi); g_b[bi].had_positions = true;
        }
      // Do not erase closed cached tickets: OdswiezBilety still owes their
      // existing realized/history reconciliation, including after Done.
     }
  }
bool ExitTicketPending(ulong ticket)
  {
   if(!In_ConfirmedExitRetry || !PositionSelectByTicket(ticket)) return false;
   if(PositionGetString(POSITION_SYMBOL) != _Symbol || PositionGetInteger(POSITION_MAGIC) != In_Magic) return false;
   return ConfirmedExitPending(ExitOwner(ticket, (ulong)PositionGetInteger(POSITION_IDENTIFIER), PositionGetString(POSITION_COMMENT)));
  }
bool ExitCancelOwned(int bi, ulong ticket)
  {
   if(!OrderSelect(ticket)) return true;
   if(OrderGetString(ORDER_SYMBOL) != _Symbol || OrderGetInteger(ORDER_MAGIC) != In_Magic
      || ExitOwner(ticket, 0, OrderGetString(ORDER_COMMENT)) != bi) return false;
   if(In_TestExitScenario > 0 && MQLInfoInteger(MQL_TESTER)
      && (g_test_cancel_reject > 0 || g_test_cancel_until_fill))
     {
      if(g_test_cancel_reject > 0) g_test_cancel_reject--;
      PrintFormat("CEXIT_TEST_EVENT|cancel_rejected_injected|%I64d|%I64u", g_now, ticket);
      return false; // no cancellation RPC: the real tester order stays alive
     }
   MqlTradeRequest r; MqlTradeResult res;
   ZeroMemory(r); ZeroMemory(res);
   r.action = TRADE_ACTION_REMOVE; r.order = ticket;
   bool sent = OrderSend(r, res);
   bool ack = sent && res.retcode == TRADE_RETCODE_DONE;
   if(!ack) { g_rej_broker++; ZliczOdrzucenie((int)res.retcode); }
   // A successful RPC is not proof of cancellation. A racing fill is picked
   // up by the next whole broker snapshot and closed by the same intent.
   bool removed = ack && !OrderSelect(ticket);
   if(removed)
      for(int i = g_b[bi].npend - 1; i >= 0; i--)
         if(g_b[bi].pend[i] == ticket)
           {
            int lv = g_b[bi].pend_lv[i];
            if(lv >= 0 && lv < g_b[bi].nlv && !g_b[bi].lv_filled[lv]) g_b[bi].lv_cancelled[lv] = true;
            for(int j = i; j < g_b[bi].npend - 1; j++)
              { g_b[bi].pend[j] = g_b[bi].pend[j+1]; g_b[bi].pend_lv[j] = g_b[bi].pend_lv[j+1]; g_b[bi].pend_top[j] = g_b[bi].pend_top[j+1]; }
            g_b[bi].npend--;
           }
   return removed;
  }
bool ExitCloseOwned(int bi, ulong ticket, string reason)
  {
   if(!PositionSelectByTicket(ticket)) return true;
   if(PositionGetString(POSITION_SYMBOL) != _Symbol || PositionGetInteger(POSITION_MAGIC) != In_Magic
      || ExitOwner(ticket, (ulong)PositionGetInteger(POSITION_IDENTIFIER), PositionGetString(POSITION_COMMENT)) != bi) return false;
   ZapomnijZamiar(ticket);
   int queued = QeIdx(ticket); if(queued >= 0) QeForget(queued);
   g_powod_zamk = reason;
   bool ack = ZamknijPozycje(ticket); // position=ticket, actual current residual volume
   return ack && !PositionSelectByTicket(ticket);
  }
void AttemptConfirmedExit(int bi, bool first)
  {
   if(!ConfirmedExitPending(bi)) return;
   ulong positions[]; ulong orders[];
   bool complete = ExitOwnedSnapshot(bi, positions, orders);
   if(complete && ArraySize(positions) == 0 && ArraySize(orders) == 0)
     { g_b[bi].exit_pending = false; g_b[bi].state = ST_DONE; return; }
   if(!first && g_now - g_b[bi].exit_last_attempt < 1000) return;
   g_b[bi].state = ST_WORKING;
   g_b[bi].exit_last_attempt = g_now;
   for(int i = 0; i < ArraySize(orders); i++) ExitCancelOwned(bi, orders[i]);
   complete = ExitOwnedSnapshot(bi, positions, orders);
   ExitRememberLivePositions(bi, positions);
   for(int i = 0; i < ArraySize(positions); i++) ExitCloseOwned(bi, positions[i], g_b[bi].exit_reason);
   complete = ExitOwnedSnapshot(bi, positions, orders);
   if(complete && ArraySize(positions) == 0 && ArraySize(orders) == 0)
     { g_b[bi].exit_pending = false; g_b[bi].state = ST_DONE; }
  }
void RequestConfirmedExit(int bi, string reason)
  {
   if(!In_ConfirmedExitRetry || bi < 0 || bi >= g_nb) return;
   bool first = !g_b[bi].exit_pending;
   if(first)
     { g_b[bi].exit_pending = true; g_b[bi].exit_reason = reason; g_b[bi].exit_last_attempt = g_now; }
   AttemptConfirmedExit(bi, first);
  }
void RetryConfirmedExits()
  {
   if(!In_ConfirmedExitRetry) return;
   for(int bi = 0; bi < g_nb; bi++) if(ConfirmedExitPending(bi)) AttemptConfirmedExit(bi, false);
  }

//====================================================================
//  BRAMKI KAPITAŁOWE *_small — engine.rs:896-1027
//  Próg porównuje SALDO BIEŻĄCE z SALDEM STARTOWYM × mult.
//====================================================================
bool KapMale(double mult)
  { return mult > 0.0 && AccountInfoDouble(ACCOUNT_BALANCE) < g_start_balance * mult; }
double KapF(double duze, double male, double mult) { return KapMale(mult) ? male : duze; }
int    KapU(int duze, int male, double mult)       { return KapMale(mult) ? male : duze; }

// engine.rs:1015 — w trybie miekkim wlasny limit pozycji MA PIERWSZENSTWO
// przed bramka kapitalowa *_small.
int  MaxOpenPositionsEff()
  {
   if(g_rezim_miekki && In_RegimeSoftMaxPos > 0) return In_RegimeSoftMaxPos;
   return KapU(In_MaxOpenPositions, In_MaxPosSmall, In_MaxPosSmallM);
  }
int  MaxOpenBasketsEff()   { return KapU(In_MaxOpenBaskets, In_MaxBaskSmall, In_MaxBaskSmallM); }
// engine.rs:1003 — GLOWNE pokretlo trybu miekkiego: caly plan siatki jest
// przeskalowywany do zadanego procentu ryzyka koszyka.
double RiskPerBasketEff()
  {
   double r = KapF(In_RiskPerBasketPct, In_RiskPerBSmall, In_RiskPerBSmallM);
   if(g_rezim_miekki && r > 0.0 && In_RegimeSoftRiskMult != 1.0) return r * In_RegimeSoftRiskMult;
   return r;
  }
double BasketMaxAgeEff()   { return KapF(In_BasketMaxAgeMin, In_BasketAgeSmall, In_BasketAgeSmallM); }
int  UnitsBase(bool is_limit)
  {
   int u = (is_limit && In_EntryUnitsLimit > 0) ? In_EntryUnitsLimit : In_EntryUnits;
   u = KapU(u, In_EntryUnitsSmall, In_EntryUnitsSmallM);
   // engine.rs:981 — podloga 1 szczebel; zero szczebli byloby twarda blokada
   if(g_rezim_miekki && In_RegimeSoftUnitsMult != 1.0)
      return (int)MathMax(MathFloor(u * In_RegimeSoftUnitsMult), 1.0);
   return u;
  }

//====================================================================
//  LOT — engine.rs:1393-1536 (podstawa, kredyt, sufit, compounding)
//====================================================================
double KredytSkuteczny()
  {
   if(!In_OdliczKredyt) return 0.0;
   // Tester nie zna CREDIT — emulacja przez In_KredytReczny (decyzja D5).
   // ACCOUNT_CREDIT czytamy dla porządku (w testerze zwykle 0).
   double c = AccountInfoDouble(ACCOUNT_CREDIT);
   if(In_KredytReczny > 0.0) c = In_KredytReczny;
   return MathMax(c, 0.0);
  }
double PodstawaLota()
  {
   double bal = AccountInfoDouble(ACCOUNT_BALANCE);
   double eq  = AccountInfoDouble(ACCOUNT_EQUITY);
   double baza;
   if(In_LotBase == 1) baza = eq;
   else if(In_LotBase == 2) baza = MathMin(bal, eq);
   else baza = bal;
   return MathMax(baza - KredytSkuteczny(), 0.0);
  }
double SufitLota()
  {
   double sufit = (In_LotMax > 0.0) ? In_LotMax : 1e18;
   if(In_LotMaxZSalda > 0.0)
      sufit = MathMin(sufit, PodstawaLota() / In_LotMaxZSalda);
   return sufit;
  }
// clamp KAŻDEGO zlecenia (wagi R:R mnożą lot bazowy per szczebel) — engine.rs:1481
double WolumenZlecenia(double v)
  {
   double dol = (In_LotMin > 0.0) ? In_LotMin : 0.01;
   double gora = SufitLota();
   if(dol > gora) { double t = dol; dol = gora; gora = t; }
   if(gora < 0.01) gora = 0.01;
   return RoundLot(MathMin(MathMax(v, dol), gora));
  }
double LotSize()
  {
   double podst = PodstawaLota();
   double pct = KapF(In_LotPercent, In_LotPercentSmall, In_LotPercentSmallM);
   double lot = In_LotModePercent ? (podst * pct / 100.0 / 100.0) : In_LotFixed;
   if(In_LotScaleStep > 0.0)
     {
      double steps = MathMax(MathFloor(podst / In_LotScaleStep), 1.0);
      lot = MathMax(lot, 0.01 * steps);
     }
   // engine.rs:1589 — mnoznik miekki PO lot_scale_step i PRZED klamrami
   // lot_min/lot_max, zeby sufit lota nie ustapil.
   if(g_rezim_miekki && In_RegimeSoftLotMult != 1.0) lot *= In_RegimeSoftLotMult;
   return WolumenZlecenia(lot);
  }

//====================================================================
//  HISTORIA RYNKU + BUFOR ZMIENNOŚCI + FILTRY
//====================================================================
void PushHist()
  {
   if(g_nph == 0 || g_now - g_ph_ts[g_nph - 1] >= 3600000)
     {
      if(g_nph >= 720)
        {
         for(int i = 0; i < g_nph - 1; i++) { g_ph_ts[i] = g_ph_ts[i+1]; g_ph_px[i] = g_ph_px[i+1]; }
         g_nph--;
        }
      g_ph_ts[g_nph] = g_now;
      g_ph_px[g_nph] = MidPx();
      g_nph++;
     }
  }
void PushVolHist()
  {
   if(g_vh_n > 0 && g_now - g_vh_last < 5000) return;
   g_vh_last = g_now;
   int idx = (g_vh_head + g_vh_n) % MAXVH;
   if(g_vh_n >= MAXVH) { g_vh_head = (g_vh_head + 1) % MAXVH; g_vh_n--; }
   g_vh_ts[idx] = g_now; g_vh_px[idx] = MidPx();
   g_vh_n++;
   // cięcie po czasie: trzymamy maks. 24 h
   while(g_vh_n > 0 && g_now - g_vh_ts[g_vh_head] > 86400000)
     { g_vh_head = (g_vh_head + 1) % MAXVH; g_vh_n--; }
  }
// zakres H−L z okna [g_now−win_ms, g_now]; n_out = liczba próbek
double VolRange(long win_ms, int &n_out)
  {
   double lo = 1e18, hi = -1e18;
   n_out = 0;
   for(int i = g_vh_n - 1; i >= 0; i--)
     {
      int idx = (g_vh_head + i) % MAXVH;
      if(g_now - g_vh_ts[idx] > win_ms) break;
      if(g_vh_px[idx] < lo) lo = g_vh_px[idx];
      if(g_vh_px[idx] > hi) hi = g_vh_px[idx];
      n_out++;
     }
   if(n_out == 0) return 0.0;
   return hi - lo;
  }
// najstarsza próbka w oknie (fast_addon) — n_out próbek w oknie
double VolOldestInWindow(long win_ms, int &n_out)
  {
   double baza = 0.0;
   n_out = 0;
   for(int i = g_vh_n - 1; i >= 0; i--)
     {
      int idx = (g_vh_head + i) % MAXVH;
      if(g_now - g_vh_ts[idx] > win_ms) break;
      baza = g_vh_px[idx];
      n_out++;
     }
   return baza;
  }
// vol_factor — engine.rs:610: mnożnik jednostek z reżimu zmienności
// (zakres H−L w oknie >= vol_range_usd → vol_units_mult; min 5 próbek)
double VolFactor()
  {
   if(In_VolWindowMin <= 0.0) return 1.0;
   int n = 0;
   double r = VolRange((long)(In_VolWindowMin * 60000.0), n);
   if(n < 5) return 1.0;
   if(r >= In_VolRangeUsd) return MathMax(In_VolUnitsMult, 0.01);
   return 1.0;
  }

// atr_proxy — engine.rs:657 (zakres w oknie adaptive_atr_window_min, min 10 próbek)
bool AtrProxy(double &out)
  {
   double win = (In_AdaptAtrWindowMin > 0.0) ? In_AdaptAtrWindowMin : 60.0;
   int n = 0;
   double r = VolRange((long)(win * 60000.0), n);
   if(n < 10 || r <= 0.0) { out = 0.0; return false; }
   out = r;
   return true;
  }

// zakres H−L PELNEGO okna glownego filtru rezimu — engine.rs:9375
// zakres_okna_glownego. false = za malo probek (zimny start), out bez sensu.
bool ZakresOknaGlownego(double &out)
  {
   int n = (int)MathMax(In_RegimeMaHours, 2.0);
   if(g_nph < n) return false;
   double lo = 1e18, hi = -1e18;
   for(int i = g_nph - n; i < g_nph; i++)
     {
      if(g_ph_px[i] < lo) lo = g_ph_px[i];
      if(g_ph_px[i] > hi) hi = g_ph_px[i];
     }
   out = hi - lo;
   return true;
  }

// czy okno glowne jest ROZERWANE — engine.rs:9364 rezim_wyciszony.
bool RegimeWyciszony()
  {
   if(In_RegimeFilter == 0 || In_RegimeZmiennoscMax <= 0.0) return false;
   double z;
   if(!ZakresOknaGlownego(z)) return false;
   return z > In_RegimeZmiennoscMax;
  }

bool RegimeOk(int side, double price)
  {
   if(In_RegimeFilter == 0) return true;
   int n = (int)In_RegimeMaHours;
   if(g_nph < MathMax(n, 2)) return true;      // za mało próbek = przepuszczamy
   // okno rozerwane MILCZY — engine.rs:9422 zwraca None, a brak zdania nie
   // jest zdaniem przeciwnym (engine.rs:9516 unwrap_or(true)).
   double zakr;
   if(In_RegimeZmiennoscMax > 0.0 && ZakresOknaGlownego(zakr)
      && zakr > In_RegimeZmiennoscMax) return true;
   double s = 0;
   for(int i = g_nph - n; i < g_nph; i++) s += g_ph_px[i];
   double ma = s / n;
   bool with = (side == 0) ? (price > ma) : (price < ma);
   return (In_RegimeFilter == 1) ? with : !with;
  }

// trend_adverse — engine.rs:8186. 0=nie, 1=tak, -1=brak wiedzy
int TrendAdverse(int side)
  {
   if(!In_TrendFilterOn || In_TrendFilterDropPct <= 0.0) return -1;
   long okno = (long)(MathMax(In_TrendFilterWinH, 1.0) * 3600000.0);
   long t0 = g_now - okno;
   if(g_nph < 2) return -1;
   if(g_ph_ts[0] > t0) return -1;               // bufor krótszy niż okno
   double teraz = g_ph_px[g_nph - 1];
   double dawniej = 0.0; bool jest = false;
   for(int i = 0; i < g_nph; i++)
      if(g_ph_ts[i] >= t0) { dawniej = g_ph_px[i]; jest = true; break; }
   if(!jest || dawniej <= 0.0) return -1;
   double zmiana = (teraz - dawniej) / dawniej * 100.0;
   bool adv = (side == 0) ? (zmiana <= -In_TrendFilterDropPct) : (zmiana >= In_TrendFilterDropPct);
   return adv ? 1 : 0;
  }

//====================================================================
//  RODZINA MARGINESU — engine.rs:6975-7019
//  Poziom liczony z ceny OTWARCIA (jak MT5 order_calc_margin dla XAUUSD).
//====================================================================
void PoziomMarginesu(double &teraz, bool &ma_teraz, double &docelowy, bool &ma_doc)
  {
   double eq = AccountInfoDouble(ACCOUNT_EQUITY);
   ma_teraz = false; ma_doc = false; teraz = 0; docelowy = 0;
   if(eq <= 0.0) { ma_teraz = true; ma_doc = true; return; } // zera = bramki szczelne
   // konto_dzwignia nadpisuje dźwignię konta do testów stresu (engine.rs:6984)
   double lev = (In_KontoDzwignia > 0.0) ? In_KontoDzwignia
                : (double)AccountInfoInteger(ACCOUNT_LEVERAGE);
   if(lev < 1.0) lev = 1.0;
   double m_poz = 0.0, m_pend = 0.0;
   for(int i = PositionsTotal() - 1; i >= 0; i--)
     {
      ulong t = PositionGetTicket(i);
      if(t == 0 || PositionGetInteger(POSITION_MAGIC) != In_Magic) continue;
      m_poz += PositionGetDouble(POSITION_VOLUME) * XAU_CONTRACT * PositionGetDouble(POSITION_PRICE_OPEN) / lev;
     }
   for(int i = OrdersTotal() - 1; i >= 0; i--)
     {
      ulong t = OrderGetTicket(i);
      if(t == 0 || OrderGetInteger(ORDER_MAGIC) != In_Magic) continue;
      m_pend += OrderGetDouble(ORDER_VOLUME_CURRENT) * XAU_CONTRACT * OrderGetDouble(ORDER_PRICE_OPEN) / lev;
     }
   if(m_poz > 0.0) { teraz = eq / m_poz * 100.0; ma_teraz = true; }
   double razem = m_poz + m_pend;
   if(razem > 0.0) { docelowy = eq / razem * 100.0; ma_doc = true; }
  }
bool MarginesPozwala(double prog)
  {
   if(prog <= 0.0) return true;
   double teraz, doc; bool mt, md;
   PoziomMarginesu(teraz, mt, doc, md);
   bool ma = In_MlLiczWiszace ? md : mt;
   double ml = In_MlLiczWiszace ? doc : teraz;
   if(!ma) return true;   // brak ekspozycji = poziom nieskończony
   return ml > prog;
  }


//====================================================================
//  BRAMKA WEJŚĆ — engine.rs:8663 bramka_wejscia (pełna)
//  Zwraca 0 = otwarte, inaczej kod powodu (do statystyk odrzuceń).
//====================================================================
int LiczPozycje()
  {
   int n = 0;
   for(int i = PositionsTotal() - 1; i >= 0; i--)
     {
      ulong t = PositionGetTicket(i);
      if(t == 0) continue;
      if(PositionGetInteger(POSITION_MAGIC) == In_Magic) n++;
     }
   return n;
  }
int LiczZlecenia()
  {
   int n = 0;
   for(int i = OrdersTotal() - 1; i >= 0; i--)
     {
      ulong t = OrderGetTicket(i);
      if(t == 0) continue;
      if(OrderGetInteger(ORDER_MAGIC) == In_Magic) n++;
     }
   return n;
  }
int LiczZyweKoszyki()
  {
   int n = 0;
   for(int i = 0; i < g_nb; i++) if(Alive(i)) n++;
   return n;
  }
double PozZysk(ulong t)
  {
   if(!PositionSelectByTicket(t)) return 0.0;
   // Position::profit_usd in core is gross mark-to-market, without swap or
   // broker cash rounding. Receipt/account accounting remains net elsewhere.
   int side=PositionGetInteger(POSITION_TYPE)==POSITION_TYPE_BUY ? 0 : 1;
   return (ExitPx(side)-PositionGetDouble(POSITION_PRICE_OPEN))*SideSign(side)
          *XAU_CONTRACT*PositionGetDouble(POSITION_VOLUME);
  }
// Stable like Rust slice::sort_by: equal strategy keys retain the receipt
// order. Non-adjacent swaps can silently reverse ties after a smaller key.
void NativeStableSortTickets(ulong &tickets[], double &keys[], int count, bool descending)
  {
   for(int i=1;i<count;i++)
     {
      ulong ticket=tickets[i];double key=keys[i];int j=i;
      while(j>0 && (descending ? key>keys[j-1] : key<keys[j-1]))
        {tickets[j]=tickets[j-1];keys[j]=keys[j-1];j--;}
      tickets[j]=ticket;keys[j]=key;
     }
  }
void NativeSortProfit(ulong &tickets[], int count, bool descending)
  {
   double keys[MAXTK];
   for(int i=0;i<count;i++)keys[i]=PozZysk(tickets[i]);
   NativeStableSortTickets(tickets,keys,count,descending);
  }

double PozPunkty(ulong t)
  {
   if(!PositionSelectByTicket(t)) return 0.0;
   long typ = PositionGetInteger(POSITION_TYPE);
   double op = PositionGetDouble(POSITION_PRICE_OPEN);
   return (typ == POSITION_TYPE_BUY) ? (g_bid - op) : (op - g_ask);
  }

bool HoursOk(int hour)
  {
   if(!In_SessionFilter) return true;
   string cz[];
   int k = StringSplit(In_SessionHours, ',', cz);
   for(int i = 0; i < k; i++)
     {
      string ab[];
      int m = StringSplit(cz[i], '-', ab);
      if(m >= 2)
        {
         int a = (int)StringToInteger(ab[0]), b2 = (int)StringToInteger(ab[1]);
         if(hour >= a && hour < b2) return true;
        }
      else if(m == 1)
        {
         if(hour == (int)StringToInteger(ab[0])) return true;
        }
     }
   return false;
  }

// warunkowy bonus ekspozycji — engine.rs:987
void BonusEkspozycji(int &bonus_poz, int &bonus_kosz)
  {
   bonus_poz = 0; bonus_kosz = 0;
   if(In_ExpoBonusProfitPct <= 0.0 || (In_ExpoBonusPositions == 0 && In_ExpoBonusBaskets == 0)) return;
   double prog = AccountInfoDouble(ACCOUNT_BALANCE) * In_ExpoBonusProfitPct / 100.0;
   if(prog <= 0.0) return;
   double plyw = 0.0;
   for(int i = PositionsTotal() - 1; i >= 0; i--)
     {
      ulong t = PositionGetTicket(i);
      if(t == 0 || PositionGetInteger(POSITION_MAGIC) != In_Magic) continue;
      plyw += PozZysk(t);
     }
   if(plyw >= prog) { bonus_poz = In_ExpoBonusPositions; bonus_kosz = In_ExpoBonusBaskets; }
  }

bool DayPctGuardActive()
  {
   double e = g_day_start_eq;
   if(In_DayGateOdSalda > 0.0 && e < In_DayGateOdSalda) return false;
   if(In_DayGateDoSalda > 0.0 && e >= In_DayGateDoSalda) return false;
   return true;
  }

int EntryGate()
  {
   if(g_dzien_od > 0 && g_now < g_dzien_od)   return 7;  // rozbieg — tylko historia
   if(g_dzien_do > 0 && g_now >= g_dzien_do)  return 7;  // po dobie
   if(StringLen(g_halted) > 0)                return 8;  // Halted (max_dd)
   // HAMULEC SL-HIT stoi PO halted, a PRZED pauza po serii — engine.rs:9061-9076
   if(In_SlhitPauseN > 0 && g_now < g_slhit_pauza_do) return 13;
   if(g_now < g_paused_until)                 return 1;  // StreakPause
   // Z-2: doba zamknięta przez strażnika
   if(g_day_stop != -999999999 && DayOf(g_now) == g_day_stop) return 9;
   if(!HoursOk(HourOf(g_now)))                return 2;  // SessionClosed
   double eq = AccountInfoDouble(ACCOUNT_EQUITY);
   // cel dnia (usd/pct) zamyka bramkę wejść — engine.rs:8699-8729
   if(In_DayTargetUsd > 0.0)
     {
      double lot = LotSize();
      double scale = In_DayTargetScaleLot ? MathMax(lot / 0.01, 1.0)
                     : (In_UsdScaleWithLot ? MathMax(lot / 0.01, 1.0) : 1.0);
      if(eq - g_day_start_eq >= In_DayTargetUsd * scale) return 10;
     }
   if(DayPctGuardActive() && In_DayTargetPct > 0.0)
     {
      double prog = MathMax(g_day_start_eq, 1.0) * In_DayTargetPct / 100.0;
      if(eq - g_day_start_eq >= prog) return 10;
     }
   // limity ekspozycji (+ warunkowy bonus) — engine.rs:8766-8805
   int bonus_poz, bonus_kosz;
   BonusEkspozycji(bonus_poz, bonus_kosz);
   int limit_poz = MaxOpenPositionsEff();
   if(limit_poz > 0)
     {
      int n = LiczPozycje() + (In_ExposureCountPend ? LiczZlecenia() : 0);
      if(n >= limit_poz + bonus_poz)           return 3;  // MaxOpenPositions
     }
   int limit_kosz = MaxOpenBasketsEff();
   if(limit_kosz > 0 && LiczZyweKoszyki() >= limit_kosz + bonus_kosz) return 4;
   // limit lotów kierunkowych — engine.rs:8806-8833
   if(In_MaxDirectionalLots > 0.0)
     {
      double buy = 0.0, sell = 0.0;
      for(int i = PositionsTotal() - 1; i >= 0; i--)
        {
         ulong t = PositionGetTicket(i);
         if(t == 0 || PositionGetInteger(POSITION_MAGIC) != In_Magic) continue;
         if(PositionGetInteger(POSITION_TYPE) == POSITION_TYPE_BUY) buy += PositionGetDouble(POSITION_VOLUME);
         else sell += PositionGetDouble(POSITION_VOLUME);
        }
      if(MathMax(buy, sell) >= In_MaxDirectionalLots) return 11;
     }
   if(In_EquityFloorPct > 0.0)
     {
      if(eq <= g_start_balance * In_EquityFloorPct / 100.0) return 5;
     }
   if(In_MarginCallPct > 0.0)
     {
      double mar = AccountInfoDouble(ACCOUNT_MARGIN);
      if(mar > 0.0)
        {
         double lvl = eq / mar * 100.0;
         if(lvl <= In_MarginCallPct) return 6;
        }
     }
   // bramka poziomu marginesu ml_min_wejscie — engine.rs:8922
   if(!MarginesPozwala(In_MlMinWejscie)) return 12;
   return 0;
  }

// pamięć bramki na tick — patrz CONDUIT_X (regresja H2, log 870 MB)
long g_gate_ts[4]  = {-1, -1, -1, -1};
int  g_gate_val[4] = {0, 0, 0, 0};
int  g_bank_klucz  = 0;
int EntryGateNaTick()
  {
   int k = g_bank_klucz;
   if(g_gate_ts[k] != g_now)
     { g_gate_ts[k] = g_now; g_gate_val[k] = EntryGate(); }
   return g_gate_val[k];
  }

//====================================================================
//  WAGI SIATKI — settings.rs:3478/3526
//====================================================================
void RRMultipliers(double &prices[], int n, double sl, bool has_sl,
                   double tp1, bool has_tp1, double &out[])
  {
   for(int i = 0; i < n; i++) out[i] = 1.0;
   if(!has_sl || !has_tp1) return;
   double w[MAXLV];
   for(int i = 0; i < n; i++)
     {
      double ryz = MathAbs(prices[i] - sl);
      double nag = MathAbs(tp1 - prices[i]);
      if(ryz <= 1e-9 || nag <= 1e-9) return;    // brak jakości = równe wagi
      w[i] = nag / ryz;
     }
   double pw = (In_RRPower > 0.0) ? In_RRPower : 1.0;
   for(int i = 0; i < n; i++)
     {
      w[i] = MathPow(w[i], pw);
      if(!MathIsValidNumber(w[i]) || w[i] <= 0.0) return;
     }
   double mn = w[0];
   for(int i = 1; i < n; i++) if(w[i] < mn) mn = w[i];
   if(!(mn > 0.0)) return;
   double cap = (In_RRCap > 0.0) ? In_RRCap : 1e18;
   for(int i = 0; i < n; i++) w[i] = MathMin(w[i] / mn, cap);
   double mean = 0; for(int i = 0; i < n; i++) mean += w[i];
   mean /= n;
   if(!(mean > 0.0)) return;
   for(int i = 0; i < n; i++) out[i] = w[i] / mean;
  }

//  UKLAD DRABINKI WPROST — odwzorowanie `Settings::uklad_drabinki()`
//  (settings.rs). Lista czytana OD KRAWEDZI PLYTKIEJ do glebokiej; wpis
//  spoza zakresu 0..9 jest przycinany, wpis nieliczbowy POMIJANY (Rust:
//  `filter_map(parse::<i64>().ok())`), a sama suma zero = os wylaczona.
int UkladDrabinki(int &out[])
  {
   string p[];
   int k = StringSplit(In_EntryUklad, ',', p);
   int n = 0, suma = 0;
   for(int i = 0; i < k && n < MAXLV; i++)
     {
      string t = p[i];
      StringTrimLeft(t); StringTrimRight(t);
      int L = StringLen(t);
      if(L == 0) continue;
      bool liczba = true;
      for(int c = 0; c < L; c++)
        {
         ushort z = StringGetCharacter(t, c);
         bool cyfra = (z >= '0' && z <= '9');
         bool znak  = ((z == '-' || z == '+') && c == 0 && L > 1);
         if(!cyfra && !znak) { liczba = false; break; }
        }
      if(!liczba) continue;
      int v = (int)StringToInteger(t);
      if(v < 0) v = 0;
      if(v > 9) v = 9;
      out[n] = v; n++; suma += v;
     }
   if(suma == 0) return 0;
   return n;
  }

void DepthMultipliers(int n, double &out[])
  {
   for(int i = 0; i < n; i++) out[i] = 1.0;
   string p[];
   int k = StringSplit(In_EntryWeights, ',', p);
   double w[64]; int nw = 0;
   for(int i = 0; i < k && nw < 64; i++)
     {
      double v = StringToDouble(p[i]);
      if(v > 0.0) { w[nw] = v; nw++; }
     }
   if(nw < 2 || n == 1) return;
   int last = nw - 1;
   double mean = 0;
   for(int i = 0; i < n; i++)
     {
      double depth = (double)(n - 1 - i) / (double)(n - 1);
      int idx = (int)MathRound(depth * last);
      if(idx > last) idx = last;
      out[i] = w[idx];
      mean += out[i];
     }
   mean /= n;
   if(mean <= 0.0) { for(int i = 0; i < n; i++) out[i] = 1.0; return; }
   for(int i = 0; i < n; i++) out[i] /= mean;
  }

//====================================================================
//  CELE — engine.rs:3843 target_for_ex (+ cele_na_ostatnim)
//====================================================================
void ParseCounts(int &c[], int &nc)
  {
   string p[];
   int k = StringSplit(In_OfficialCounts, ',', p);
   nc = 0;
   for(int i = 0; i < k && nc < 16; i++)
     {
      string s = p[i]; StringTrimLeft(s); StringTrimRight(s);
      if(StringLen(s) == 0) continue;
      c[nc] = (int)StringToInteger(s); nc++;
     }
  }
void ParseOfficialPct(double &c[], int &nc)
  {
   string p[];
   int k = StringSplit(In_OfficialPct, ',', p);
   nc = 0;
   for(int i = 0; i < k && nc < 16; i++)
     {
      double v = StringToDouble(p[i]);
      c[nc] = v; nc++;
     }
   if(nc == 0) { c[0]=15; c[1]=30; c[2]=30; c[3]=20; nc=4; }
  }

bool TargetForEx(int bi, int idx, int total, double &out)
  {
   int ntp = g_b[bi].ntp;
   if(ntp <= 0) return false;
   int side = g_b[bi].side;
   bool open_extra = g_b[bi].tp_open && In_TpOpenExtra && In_TpOpenOffset != 0.0;
   double last = g_b[bi].tps[ntp - 1];
   if(open_extra) last = last + SideSign(side) * In_TpOpenOffset;

   if(In_CeleNaOstatnim) { out = last; return true; }               // TYLER
   if(In_TpSchedule == 0) { out = last; return true; }              // AllRunners
   if(In_TpSchedule == 1) { out = g_b[bi].tps[0]; return true; }    // AllAtTp1
   if(In_TpSchedule == 2 || In_TpSchedule == 4 || In_TpSchedule == 5)
     {                                                             // Ladder/OfficialPct/ScaleOutPct
      int tier = (idx * ntp) / MathMax(total, 1);
      if(tier > ntp - 1) tier = ntp - 1;
      out = g_b[bi].tps[tier];
      return true;
     }
   // OfficialCounts
   int c[16], nc; ParseCounts(c, nc);
   int acc = 0;
   for(int i = 0; i < nc; i++)
     {
      acc += c[i];
      if(idx < acc) { int j = MathMin(i, ntp - 1); out = g_b[bi].tps[j]; return true; }
     }
   if(In_LastRunner == 0) return false;                            // NoTp
   if(In_LastRunner == 1) { out = g_b[bi].tps[MathMin(nc, ntp - 1)]; return true; }
   out = In_TpFreezeAfterLad ? last : (last + SideSign(side) * In_TpOpenOffset);
   return true;
  }

//====================================================================
//  PLAN SIATKI — engine.rs:2797 plan_grid + 3015 cap_basket_risk
//====================================================================
double GridStep()
  {
   if(In_PpmEnabled && In_PpmForLimits && In_Ppm > 0.0) return 1.0 / In_Ppm;
   return 0.0;
  }

int UnitsForLevel(double level, int bi, int base_units, int n_levels)
  {
   bool mnoznik_kraty = In_GridAnchorAbs && In_UnitsPerLevel
                        && (g_b[bi].is_limit || In_UnitsPerLevelZone);
   int u = (n_levels > 1 && !mnoznik_kraty) ? 1 : base_units;
   if(In_EntryRiskBudget > 0.0 && g_b[bi].has_sl)
     {
      double d = MathAbs(level - g_b[bi].sl);
      if(d > 0.0) u = MathMin(u, (int)MathMax(MathRound(In_EntryRiskBudget / d), 1.0));
     }
   if(In_EntryTp1Budget > 0.0 && g_b[bi].ntp > 0)
     {
      double d = MathAbs(g_b[bi].tps[0] - level);
      if(d > 0.0) u = MathMin(u, (int)MathMax(MathRound(In_EntryTp1Budget / d), 1.0));
     }
   return MathMax(u, 1);
  }

// dławik dd_soft/hard — engine.rs:3086 dlawik_mult (hard = min z soft, clamp 0..1)
double DlawikMult()
  {
   if(In_DdSoftPct <= 0.0 && In_DdHardPct <= 0.0) return 1.0;
   double eq = AccountInfoDouble(ACCOUNT_EQUITY);
   double base;
   if(In_DdGuardScope == 0) base = g_day_peak_eq;
   else base = g_peak_equity;
   if(base <= 0.0) return 1.0;
   double dd_pct = (base - eq) / MathMax(base, 1.0) * 100.0;
   double m = 1.0;
   if(In_DdSoftPct > 0.0 && dd_pct >= In_DdSoftPct) m = In_DdSoftMult;
   if(In_DdHardPct > 0.0 && dd_pct >= In_DdHardPct) m = MathMin(m, In_DdHardMult);
   return MathMin(MathMax(m, 0.0), 1.0);
  }

double LevelRisk(int bi, int i)
  {
   if(!g_b[bi].has_sl) return 0.0;
   return MathAbs(g_b[bi].lv_price[i] - g_b[bi].sl) * XAU_CONTRACT
          * g_b[bi].lv_vol[i] * MathMax(g_b[bi].lv_units[i], 1);
  }
double PlanRisk(int bi)
  {
   double s = 0;
   for(int i = 0; i < g_b[bi].nlv; i++) s += LevelRisk(bi, i);
   return s;
  }

// ryzyko otwarte + wiszące CAŁEGO rachunku (max_portfolio_risk_pct) —
// engine.rs:3127: |cena−SL| BEZWZGLĘDNIE (SL za wejściem nadal zajmuje budżet;
// znak jest zarezerwowany dla market_risk_cap). SL albo wirtualny SL.
double RyzykoPortfela()
  {
   double s = 0.0;
   for(int i = PositionsTotal() - 1; i >= 0; i--)
     {
      ulong t = PositionGetTicket(i);
      if(t == 0 || PositionGetInteger(POSITION_MAGIC) != In_Magic) continue;
      double psl = PositionGetDouble(POSITION_SL);
      if(psl == 0.0)
        {
         int ip = PsIdx(t);
         if(ip >= 0 && g_ps_vsl[ip] != 0.0) psl = g_ps_vsl[ip];
        }
      if(psl == 0.0) continue;
      double op = PositionGetDouble(POSITION_PRICE_OPEN);
      s += MathAbs(op - psl) * XAU_CONTRACT * PositionGetDouble(POSITION_VOLUME);
     }
   for(int i = OrdersTotal() - 1; i >= 0; i--)
     {
      ulong t = OrderGetTicket(i);
      if(t == 0 || OrderGetInteger(ORDER_MAGIC) != In_Magic) continue;
      double psl = OrderGetDouble(ORDER_SL);
      if(psl == 0.0) continue;
      double op = OrderGetDouble(ORDER_PRICE_OPEN);
      s += MathAbs(op - psl) * XAU_CONTRACT * OrderGetDouble(ORDER_VOLUME_CURRENT);
     }
   return s;
  }

// Opt-in reserve of observed daily equity profit. This only sizes NEW exposure.
// The tester always starts from a fresh account; no cross-run day anchor is reused.
bool PbPositive(double x) { return MathIsValidNumber(x) && x > 0.0; }

int ProfitBudgetAvailable(double &remaining, string &error)
  {
   remaining = 0.0; error = "";
   if(!MathIsValidNumber(In_MaxPortfolioRisk)) {error="InvalidSettings";return -1;}
   bool portfolio_enabled=In_MaxPortfolioRisk>0.0;
   bool reserve_enabled=In_ProfitBudgetArmPct!=0.0;
   if(!portfolio_enabled && !reserve_enabled)return 0;
   if(reserve_enabled && (!PbPositive(In_ProfitBudgetArmPct) || !MathIsValidNumber(In_ProfitBudgetKeepPct)
      || In_ProfitBudgetKeepPct < 0.0 || In_ProfitBudgetKeepPct > 100.0
      || !MathIsValidNumber(In_ProfitBudgetDeployPct)
      || In_ProfitBudgetDeployPct < 0.0 || In_ProfitBudgetDeployPct > 100.0))
     { error = "InvalidSettings"; return -1; }
   double equity = AccountInfoDouble(ACCOUNT_EQUITY);
   if(!MathIsValidNumber(equity)) { error = "InvalidAccount"; return -1; }
   double floor=0.0,reserve_capacity=0.0;
   bool has_reserve=false;
   if(reserve_enabled)
     {
      if(g_day != DayOf(g_now) || !PbPositive(g_day_start_eq) || !PbPositive(g_day_peak_eq))
        { error = "UnknownDayAnchor"; return -1; }
      double peak=MathMax(g_day_peak_eq,equity),profit=MathMax(peak-g_day_start_eq,0.0);
      if(profit>0.0 && profit>=g_day_start_eq*In_ProfitBudgetArmPct/100.0)
        {
         floor=g_day_start_eq+profit*In_ProfitBudgetKeepPct/100.0;
         reserve_capacity=MathMax(equity-floor,0.0)*In_ProfitBudgetDeployPct/100.0;
         has_reserve=true;
        }
     }
   double portfolio_capacity=portfolio_enabled ? MathMax(equity,0.0)*In_MaxPortfolioRisk/100.0 : 0.0;
   // Validate each computed capacity before min: a finite second guard must
   // never hide an overflow in the first guard.
   if((portfolio_enabled && !MathIsValidNumber(portfolio_capacity))
      || (has_reserve && !MathIsValidNumber(reserve_capacity)))
     {error="InvalidAccount";return -1;}
   if(!portfolio_enabled && !has_reserve)return 0;
   double capacity=portfolio_enabled ? portfolio_capacity : reserve_capacity;
   if(portfolio_enabled && has_reserve)capacity=MathMin(capacity,reserve_capacity);
   if(!PbPositive(g_bid) || !PbPositive(g_ask) || g_ask < g_bid)
     { error = "InvalidQuote"; return -1; }
   double used = 0.0;
   for(int i = PositionsTotal() - 1; i >= 0; i--)
     {
      ulong t = PositionGetTicket(i);
      if(t == 0) { error = "InvalidExposure"; return -1; }
      if(PositionGetInteger(POSITION_MAGIC) != In_Magic) continue;
      // This native fixture has one XAU quote stream. Never price another
      // symbol silently using this stream if extra exposure was introduced.
      if(PositionGetString(POSITION_SYMBOL) != _Symbol) { error = "InvalidQuote"; return -1; }
      double sl = PositionGetDouble(POSITION_SL);
      if(sl == 0.0) { int ip = PsIdx(t); if(ip >= 0) sl = g_ps_vsl[ip]; }
      if(!PbPositive(sl)) { error = "MissingStop"; return -1; }
      double volume = PositionGetDouble(POSITION_VOLUME);
      if(!PbPositive(volume)) { error = "InvalidExposure"; return -1; }
      int side = PositionGetInteger(POSITION_TYPE) == POSITION_TYPE_BUY ? 0 : 1;
      used += MathMax((ExitPx(side) - sl) * SideSign(side), 0.0) * XAU_CONTRACT * volume;
     }
   for(int i = OrdersTotal() - 1; i >= 0; i--)
     {
      ulong t = OrderGetTicket(i);
      if(t == 0) { error = "InvalidExposure"; return -1; }
      if(OrderGetInteger(ORDER_MAGIC) != In_Magic) continue;
      if(OrderGetString(ORDER_SYMBOL) != _Symbol) { error = "InvalidQuote"; return -1; }
      double sl = OrderGetDouble(ORDER_SL), entry = OrderGetDouble(ORDER_PRICE_OPEN);
      double volume = OrderGetDouble(ORDER_VOLUME_CURRENT);
      if(!PbPositive(sl)) { error = "MissingStop"; return -1; }
      if(!PbPositive(entry) || !PbPositive(volume)) { error = "InvalidExposure"; return -1; }
      long typ = OrderGetInteger(ORDER_TYPE);
      int side;
      if(typ == ORDER_TYPE_BUY_LIMIT || typ == ORDER_TYPE_BUY_STOP || typ == ORDER_TYPE_BUY_STOP_LIMIT) side = 0;
      else if(typ == ORDER_TYPE_SELL_LIMIT || typ == ORDER_TYPE_SELL_STOP || typ == ORDER_TYPE_SELL_STOP_LIMIT) side = 1;
      else { error = "InvalidExposure"; return -1; }
      used += MathMax((entry - sl) * SideSign(side), 0.0) * XAU_CONTRACT * volume;
     }
   if(!MathIsValidNumber(floor) || !MathIsValidNumber(capacity) || !MathIsValidNumber(used))
     { error = "InvalidExposure"; return -1; }
   remaining = MathMax(capacity - used, 0.0);
   return 1;
  }

double ProfitBudgetUnits(double value, double step, bool up)
  {
   double n = value / step;
   if(!MathIsValidNumber(n) || n > 4503599627370496.0) return -1.0;
   double rounded = MathRound(n);
   double tolerance = 8.0 * 2.2204460492503131e-16 * MathMax(MathAbs(n), 1.0);
   if(MathAbs(n - rounded) <= tolerance) return rounded;
   return up ? MathCeil(n) : MathFloor(n);
  }

bool ProfitBudgetFloorVolume(double requested, double &volume)
  {
   double vmin = SymbolInfoDouble(_Symbol, SYMBOL_VOLUME_MIN);
   double step = SymbolInfoDouble(_Symbol, SYMBOL_VOLUME_STEP);
   double vmax = SymbolInfoDouble(_Symbol, SYMBOL_VOLUME_MAX);
   if(!PbPositive(requested) || !PbPositive(vmin) || !PbPositive(step) || !PbPositive(vmax)
      || vmax < vmin || !PbPositive(In_LotMin)
      || !MathIsValidNumber(In_LotMax) || In_LotMax < 0.0
      || !MathIsValidNumber(In_LotMaxZSalda) || In_LotMaxZSalda < 0.0) return false;
   double wire = step * 1e8;
   if(!MathIsValidNumber(wire) || wire < 1.0
      || MathAbs(wire - MathRound(wire)) > 8.0 * 2.2204460492503131e-16 * MathMax(MathAbs(wire), 1.0)) return false;
   double minimum = MathMax(vmin, In_LotMin), maximum = vmax;
   if(In_LotMax > 0.0) maximum = MathMin(maximum, In_LotMax);
   if(In_LotMaxZSalda > 0.0)
     {
      double capital = PodstawaLota();
      if(!MathIsValidNumber(capital) || capital < 0.0) return false;
      maximum = MathMin(maximum, capital / In_LotMaxZSalda);
     }
   if(minimum > maximum) return false;
   double low = ProfitBudgetUnits(minimum, step, true);
   double high = ProfitBudgetUnits(MathMin(requested, maximum), step, false);
   if(low < 0.0 || high < low || high < 1.0) return false;
   volume = high * step;
   double eps = 16.0 * 2.2204460492503131e-16 * MathMax(MathMax(MathAbs(volume), MathAbs(step)), 1.0);
   return PbPositive(volume) && volume <= requested + eps && volume <= maximum + eps && volume + eps >= minimum;
  }

bool ProfitBudgetLimit(int bi, int side, double entry, double sl, double requested, double &volume)
  {
   if(In_ProfitBudgetArmPct != 0.0 && g_day == DayOf(g_now))
     {
      double equity = AccountInfoDouble(ACCOUNT_EQUITY);
      if(MathIsValidNumber(equity)) g_day_peak_eq = MathMax(g_day_peak_eq, equity);
     }
   double remaining; string error;
   int state = ProfitBudgetAvailable(remaining, error);
   if(state == 0) return true; // retain the already-normalized legacy request
   if(state > 0)
     {
      if(remaining <= 0.0) error = "Exhausted";
      else if(!PbPositive(sl)) error = "MissingStop";
      else if(!PbPositive(entry) || (entry-sl) * SideSign(side) <= 0.0) error = "InvalidNewStop";
      else if(!PbPositive(requested)) error = "InvalidVolume";
      else
        {
         double per_lot = (entry-sl) * SideSign(side) * XAU_CONTRACT;
         if(!ProfitBudgetFloorVolume(MathMin(requested, remaining/per_lot), volume)) error = "Exhausted";
         else if(!MathIsValidNumber(per_lot*volume)
                 || per_lot*volume > remaining + 16.0*2.2204460492503131e-16*MathMax(remaining,1.0)) error = "Exhausted";
        }
     }
   if(error == "") return true;
   g_rej_budget++;
   if(In_Diag && g_handle_diag != INVALID_HANDLE)
      FileWrite(g_handle_diag, "PROFIT_BUDGET_REJECT", (string)g_now,
                bi >= 0 && bi < g_nb ? (string)g_b[bi].id : "0", "ProfitBudget::"+error);
   return false;
  }

void CapBasketRisk(int bi)
  {
   double pct = RiskPerBasketEff();
   double profit_remaining; string profit_error;
   int profit_state = ProfitBudgetAvailable(profit_remaining, profit_error);
   // engine.rs:3022: wczesny powrot TYLKO gdy OBA limity wylaczone —
   // sufit portfelowy dziala takze przy risk_per_basket_pct=0.
   if((pct <= 0.0 && In_MaxPortfolioRisk <= 0.0 && profit_state == 0) || g_b[bi].nlv == 0) return;
   if(!g_b[bi].has_sl) return;
   double cap = (pct > 0.0)
      ? MathMax(AccountInfoDouble(ACCOUNT_EQUITY), 0.0) * pct * DlawikMult() / 100.0
      : 1e18;
   // sufit portfelowy: min z wolnym budżetem portfela — engine.rs:3121
   if(profit_state != 0)
      cap = MathMin(cap, profit_state > 0 ? profit_remaining : 0.0);
   else if(In_MaxPortfolioRisk > 0.0)
     {
      double wolne = MathMax(AccountInfoDouble(ACCOUNT_EQUITY), 0.0) * In_MaxPortfolioRisk / 100.0
                     - RyzykoPortfela();
      cap = MathMin(cap, MathMax(wolne, 0.0));
     }
   if(cap <= 0.0) { g_b[bi].nlv = 0; return; }
   if(cap >= 1e17) return;   // oba limity efektywnie bez ograniczenia
   double now = PlanRisk(bi);
   if(now <= cap) return;
   // 1) proporcjonalne ścięcie wolumenów
   double factor = cap / now;
   for(int i = 0; i < g_b[bi].nlv; i++)
      g_b[bi].lv_vol[i] = WolumenZlecenia(MathMax(g_b[bi].lv_vol[i] * factor, In_LotMin));
   now = PlanRisk(bi);
   // 2) odrzucanie NAJPŁYTSZYCH poziomów (ostatnich w kolejności)
   while(now > cap && g_b[bi].nlv > 1)
     {
      g_b[bi].nlv--;
      now = PlanRisk(bi);
     }
   // 3) nawet jeden poziom przy minimalnym locie się nie mieści
   if(now > cap) g_b[bi].nlv = 0;
  }

void PlanGrid(int bi, int units_mult_num = 1, int units_mult_den = 1)
  {
   if(!ExitRiskAllowed(bi)) return;
   g_b[bi].nlv = 0;
   int side = g_b[bi].side;
   double lo = g_b[bi].zone_lo, hi = g_b[bi].zone_hi;
   int units = MathMax(UnitsBase(g_b[bi].is_limit), 1);
   // adaptive_units — engine.rs:2813 (szerokosc strefy SYGNALU, nie po offsetach)
   units = MathMax(AdaptiveUnits(units, MathAbs(g_b[bi].entry_hi - g_b[bi].entry_lo)), 1);
   // trend_filter tryb Shrink: mnożnik jednostek (engine: przepuszcza mniejszym rozmiarem)
   if(units_mult_den > 0 && (units_mult_num != units_mult_den))
      units = (int)MathMax(MathRound((double)units * units_mult_num / units_mult_den), 1.0);

   double prices[MAXLV]; int np = 0;
   //  UKLAD WPROST: liczby pozycji przypisane szczeblom, indeksowane
   //  RAZEM z `prices` — patrz `Engine::przesiej_rownolegle` (engine.rs).
   int uk[MAXLV];     int nuk = UkladDrabinki(uk);
   int uk_szt[MAXLV]; int n_szt = 0;
   double step = GridStep();

   if(In_GridAnchorAbs && step > 0.0)
     {
      double v = MathCeil(lo / step - 1e-9) * step;
      while(v <= hi + 1e-9 && np < MAXLV)
        { prices[np] = MathRound(v * 100.0) / 100.0; np++; v += step; }
      for(int a = 0; a < np - 1; a++)
         for(int b2 = a + 1; b2 < np; b2++)
           {
            bool sw = (side == 0) ? (prices[b2] < prices[a]) : (prices[b2] > prices[a]);
            if(sw) { double t = prices[a]; prices[a] = prices[b2]; prices[b2] = t; }
           }
      if(np == 0) { prices[0] = BetterEdge(side, lo, hi); np = 1; }
     }
   else if(step > 0.0 && hi - lo > step * 0.5)
     {
      int n = (int)MathMax(MathFloor((hi - lo) / step), 1);
      for(int i = 0; i <= n && np < MAXLV; i++)
        {
         double p = (side == 0) ? (lo + i * step) : (hi - i * step);
         if(p >= lo - 1e-9 && p <= hi + 1e-9) { prices[np] = p; np++; }
        }
     }
   else if(units == 1)
     {
      //  KTORA KRAWEDZ przy jednym szczeblu — engine.rs `entry_jeden_na_glebokiej`.
      prices[0] = In_EntryJedenNaGleb ? BetterEdge(side, lo, hi)
                                      : WorseEdge(side, lo, hi);
      np = 1;
     }
   else if(nuk > 0)
     {
      //  UKLAD WPROST — engine.rs. Lista jest podana od krawedzi PLYTKIEJ,
      //  a wewnetrzna kolejnosc `prices` jest odwrotna (0 = najglebszy),
      //  wiec odwracamy ja TUTAJ raz i nigdzie indziej. Szczeble z zerem
      //  sa pomijane calkiem — tego wlasnie `entry_weights` nie umie.
      for(int i = nuk - 1; i >= 0 && np < MAXLV; i--)
        {
         if(uk[i] == 0) continue;
         double fu = (double)(nuk - 1 - i) / (double)MathMax(nuk - 1, 1);
         prices[np] = (side == 0) ? (lo + fu * (hi - lo)) : (hi - fu * (hi - lo));
         uk_szt[n_szt] = uk[i]; n_szt++;
         np++;
        }
     }
   else
     {
      for(int i = 0; i < units && np < MAXLV; i++)
        {
         double f = (double)i / (double)MathMax(units - 1, 1);
         double k = In_EntryDepthCurve;
         if(k > 0.0 && MathAbs(k - 1.0) > 1e-12) f = MathPow(f, k);
         prices[np] = (side == 0) ? (lo + f * (hi - lo)) : (hi - f * (hi - lo));
         np++;
        }
     }
   if(np == 0) { prices[0] = BetterEdge(side, lo, hi); np = 1; }

   // ENTRY2 zachowuje przesunięcie warstw podane przez kanał. Indeks 0 jest
   // najgłębszy, a „pierwsze wejście" kanału to ostatni (najpłytszy), więc
   // przesuwamy ku niemu wszystkie pozostałe ceny.
   double warstwy = (In_EntryWarstwyTekst && g_b[bi].has_warstwy_offset)
                    ? g_b[bi].warstwy_offset : In_EntryWarstwyOffset;
   if(MathAbs(warstwy) > 1e-12 && np > 1)
     {
      double o = SideSign(side) * warstwy;
      for(int i = 0; i < np - 1; i++) prices[i] += o;
     }

   //  PLAN KONTRA TO, CO ZOSTALO — wspolna maszyneria rodziny `*_kotwica`
   //  (engine.rs). Pamietamy, ile szczebli mial PLAN i ktorym z nich jest
   //  kazdy ocalaly; broker amputuje strefe inaczej przy kazdym sygnale.
   int plan_n = np;
   int idx_plan[MAXLV];
   for(int i = 0; i < np; i++) idx_plan[i] = i;

   // szczeble, których broker nie przyjmie — PRZED wagami i capem
   if(In_DropUnplaceable && g_b[bi].has_sl)
     {
      int w = 0;
      for(int i = 0; i < np; i++)
        {
         bool ok = (side == 0) ? (g_b[bi].sl <= prices[i] - g_stops)
                               : (g_b[bi].sl >= prices[i] + g_stops);
         //  Liczby pozycji ida RAZEM z cenami. Bez tego drabinka zjezdza
         //  o tyle miejsc, ile szczebli odrzucil broker — patrz
         //  `Engine::przesiej_rownolegle` (engine.rs).
         //  KOTWICA UKLADU (`entry_uklad_kotwica`): przy `Planowany`
         //  liczba idzie RAZEM ze swoim szczeblem; przy `Ocalaly` zostaje
         //  na miejscu, wiec caly uklad zjezdza w strone glebi o tyle
         //  miejsc, ile odrzucil broker.
         if(ok)
           {
            prices[w] = prices[i];
            idx_plan[w] = idx_plan[i];
            if(n_szt > 0 && In_UkladKotwica == 0) uk_szt[w] = uk_szt[i];
            w++;
           }
        }
      if(n_szt > 0 && In_UkladKotwica == 0) n_szt = w;
      np = w;
      if(np == 0) { g_b[bi].nlv = 0; return; }
     }

   double lot = LotSize();
   double mult[MAXLV];
   if(In_WeightsFromRR)
      RRMultipliers(prices, np, g_b[bi].sl, g_b[bi].has_sl,
                    g_b[bi].ntp > 0 ? g_b[bi].tps[0] : 0.0, g_b[bi].ntp > 0, mult);
   else
      DepthMultipliers((In_KrzywaKotwica == 1 && plan_n > 0) ? plan_n : np, mult);

   for(int i = 0; i < np; i++)
     {
      g_b[bi].lv_price[i]  = prices[i];
      //  Liczba pozycji podana WPROST — `UnitsForLevel` celowo nie jest
      //  wolane, bo tamto rozdziela pule jednostek, a tu rozdzial jest dany.
      g_b[bi].lv_units[i]  = (n_szt > 0)
                             ? (int)MathMax(uk_szt[i], 1)
                             : UnitsForLevel(prices[i], bi, units, np);
      //  Miejsce w tablicy wag: pierwotne (kotwica `Planowany`) albo
      //  biezace (`Ocalaly`) — patrz `Engine::miejsce` (engine.rs).
      int im = (In_KrzywaKotwica == 1 && !In_WeightsFromRR && plan_n > 0)
               ? (int)MathMin(idx_plan[i], plan_n - 1) : i;
      g_b[bi].lv_vol[i]    = WolumenZlecenia(MathMax(lot * mult[im], In_LotMin));
      g_b[bi].lot_planu    = lot;
      double tp;
      int it = (In_TpDrabKotwica == 1) ? idx_plan[i] : i;
      int nt = (In_TpDrabKotwica == 1) ? (int)MathMax(plan_n, 1) : (int)MathMax(np, 1);
      g_b[bi].lv_has_tp[i] = TargetForEx(bi, it, nt, tp);
      g_b[bi].lv_tp[i]     = g_b[bi].lv_has_tp[i] ? tp : 0.0;
      g_b[bi].lv_fill_ts[i] = 0;
      g_b[bi].lv_filled[i]  = false;
      g_b[bi].lv_cancelled[i] = false;
     }
   g_b[bi].nlv = np;

   // Warstwa allowance po GORSZEJ stronie strefy. Oba pola muszą być
   // aktywne; append stoi przed CapBasketRisk, więc dodatkowa ekspozycja
   // uczestniczy w tym samym limicie co reszta siatki.
   if(In_EntryAllowanceUsd > 0.0 && In_EntryAllowanceUnits > 0
      && g_b[bi].nlv < MAXLV)
     {
      double poza = WorseEdge(side, lo, hi) + SideSign(side) * In_EntryAllowanceUsd;
      bool miesci = !g_b[bi].has_sl
                    || (side == 0 ? g_b[bi].sl <= poza - g_stops
                                  : g_b[bi].sl >= poza + g_stops);
      if(!In_DropUnplaceable || miesci)
        {
         int j = g_b[bi].nlv;
         g_b[bi].lv_price[j] = MathRound(poza * 100.0) / 100.0;
         g_b[bi].lv_units[j] = In_EntryAllowanceUnits;
         g_b[bi].lv_vol[j] = WolumenZlecenia(MathMax(lot, In_LotMin));
         double tp_allow;
         g_b[bi].lv_has_tp[j] = TargetForEx(bi, MathMax(np, 1) - 1, MathMax(np, 1), tp_allow);
         g_b[bi].lv_tp[j] = g_b[bi].lv_has_tp[j] ? tp_allow : 0.0;
         g_b[bi].lv_fill_ts[j] = 0;
         g_b[bi].lv_filled[j] = false;
         g_b[bi].lv_cancelled[j] = false;
         g_b[bi].nlv++;
        }
     }

   if(In_MarketEntryUnits > 0 && !g_b[bi].is_limit)
     {
      int budzet = In_MarketEntryUnits;
      int zostaje = 0;
      for(int i = 0; i < g_b[bi].nlv; i++)
        {
         if(budzet <= 0) break;
         int u = (int)MathMax(g_b[bi].lv_units[i], 1);
         if(u <= budzet) budzet -= u;
         else { g_b[bi].lv_units[i] = budzet; budzet = 0; }
         zostaje++;
        }
      g_b[bi].nlv = zostaje;
     }

   CapBasketRisk(bi);
  }

//====================================================================
//  SKŁADANIE ZLECEŃ
//====================================================================
// SL wysyłany do brokera: przy virtual_sl_all broker dostaje siatkę ratunkową
// odsuniętą o vsl_broker_offset (engine.rs:3777 broker_sl); prawdziwy SL gra
// wirtualnie w ManagePositions.
bool BrokerSl(int side, double sl, bool has_sl, double &out)
  {
   if(!has_sl) { out = 0; return false; }
   if(In_VirtualSlAll && In_VslBrokerOffset > 0.0)
     { out = sl - SideSign(side) * In_VslBrokerOffset; return true; }
   out = sl; return true;
  }

bool WyslijRynek(int bi, int lvl, double vol, double sl, bool has_sl,
                 double tp, bool has_tp, string kom, ulong &ticket)
  {
   if(!ExitRiskAllowed(bi) || SourceWithdrawn(bi)) return false;
   if(EntryReviewBlocked(bi)) return false;
   MqlTradeRequest  r; MqlTradeResult res;
   ZeroMemory(r); ZeroMemory(res);
   r.action       = TRADE_ACTION_DEAL;
   r.symbol       = _Symbol;
   r.volume       = NormVol(vol);
   r.type         = (g_b[bi].side == 0) ? ORDER_TYPE_BUY : ORDER_TYPE_SELL;
   r.price        = (g_b[bi].side == 0) ? g_ask : g_bid;
   r.deviation    = 50;
   r.magic        = In_Magic;
   r.comment      = kom;
   r.type_filling = g_fill_deal;
   double bsl; bool hbsl = BrokerSl(g_b[bi].side, sl, has_sl, bsl);
   // Preserve the requested protection. A currently invalid stop must be
   // rejected by the broker, never converted silently into a naked order.
   if(hbsl) r.sl = NormPx(bsl);
   if(has_tp) r.tp = NormPx(tp);
   if(!ProfitBudgetLimit(bi, g_b[bi].side, r.price, r.sl, vol, r.volume)) return false;
   AuditOpenRequest(r.volume);
   if(!OrderSend(r, res) ||
      (res.retcode != TRADE_RETCODE_DONE && res.retcode != TRADE_RETCODE_PLACED))
     {
      g_rej_broker++; g_rej_place++;
      ZliczOdrzucenie((int)res.retcode);
      if(In_Diag)
         FileWrite(g_handle_diag, "ODRZUC_RYNEK", (string)g_now, (string)g_b[bi].id,
                   (string)res.retcode, StringFormat("%.2f", r.price),
                   StringFormat("%.2f", sl), StringFormat("%.2f", tp),
                   StringFormat("%.2f", vol), StringFormat("%.2f/%.2f", g_bid, g_ask),
                   res.comment);
      return false;
     }
   ticket = res.order;
   g_open_accepted_max_volume = MathMax(g_open_accepted_max_volume, r.volume);
   // wirtualny SL dla wejść rynkowych
   if(has_sl && (In_VirtualSl || In_VirtualSlAll))
     { int ip = PsEnsure(ticket); if(ip >= 0) g_ps_vsl[ip] = sl; }
   return true;
  }

bool WyslijLimit(int bi, double price, double vol, double sl, bool has_sl,
                 double tp, bool has_tp, string kom, int typ, ulong &ticket)
  {
   if(!ExitRiskAllowed(bi) || SourceWithdrawn(bi)) return false;
   if(EntryReviewBlocked(bi)) return false;
   MqlTradeRequest  r; MqlTradeResult res;
   ZeroMemory(r); ZeroMemory(res);
   r.action       = TRADE_ACTION_PENDING;
   r.symbol       = _Symbol;
   r.volume       = NormVol(vol);
   r.type         = (ENUM_ORDER_TYPE)typ;
   r.price        = NormPx(price);
   r.magic        = In_Magic;
   r.comment      = kom;
   r.type_time    = ORDER_TIME_GTC;
   r.type_filling = g_fill_pending;
   double bsl; bool hbsl = BrokerSl(g_b[bi].side, sl, has_sl, bsl);
   if(hbsl) r.sl = NormPx(bsl);
   if(has_tp) r.tp = NormPx(tp);
   if(!ProfitBudgetLimit(bi, g_b[bi].side, r.price, r.sl, vol, r.volume)) return false;
   AuditOpenRequest(r.volume);
   if(!OrderSend(r, res) ||
      (res.retcode != TRADE_RETCODE_DONE && res.retcode != TRADE_RETCODE_PLACED))
     {
      g_rej_broker++; g_rej_place++;
      ZliczOdrzucenie((int)res.retcode);
      if(In_Diag)
         FileWrite(g_handle_diag, "ODRZUC_LIMIT", (string)g_now, (string)g_b[bi].id,
                   (string)res.retcode, StringFormat("%.2f", price),
                   StringFormat("%.2f", sl), StringFormat("%.2f", tp),
                   StringFormat("%.2f", vol), StringFormat("%.2f/%.2f", g_bid, g_ask),
                   res.comment);
      return false;
     }
   ticket = res.order;
   g_open_accepted_max_volume = MathMax(g_open_accepted_max_volume, r.volume);
   if(In_ConfirmedExitRetry) ZapiszWlasciciela(ticket, bi);
   return true;
  }

// budżet ryzyka wejść rynkowych — engine.rs:3165 market_risk_cap
double MarketRiskCap(int bi, bool &jest)
  {
   jest = false;
   double pct = RiskPerBasketEff();
   if(pct <= 0.0) return 0.0;
   double cap = MathMax(AccountInfoDouble(ACCOUNT_EQUITY), 0.0) * pct / 100.0;
   double zajete = 0.0;
   for(int i = 0; i < g_b[bi].npos; i++)
     {
      ulong t = g_b[bi].pos[i];
      if(!PositionSelectByTicket(t)) continue;
      double psl = PositionGetDouble(POSITION_SL);
      if(psl == 0.0) continue;
      double op   = PositionGetDouble(POSITION_PRICE_OPEN);
      int    pside= (PositionGetInteger(POSITION_TYPE) == POSITION_TYPE_BUY) ? 0 : 1;
      double adw  = (op - psl) * SideSign(pside);
      if(adw > 0.0) zajete += adw * XAU_CONTRACT * PositionGetDouble(POSITION_VOLUME);
     }
   jest = true;
   return MathMax(cap - zajete, 0.0);
  }

// engine.rs:3210 market_risk_scale
double MarketRiskScale(double loty, double px, double sl, bool has_sl, double cap, bool has_cap)
  {
   if(!has_cap || loty <= 0.0 || cap <= 0.0 || !has_sl) return 1.0;
   double ryzyko = MathAbs(px - sl) * XAU_CONTRACT * loty;
   if(ryzyko <= cap) return 1.0;
   return cap / ryzyko;
  }

bool NativeLevelKnown(int level)
  {
   // Known special legs: reentry=-2, target pending=-3, fast addon=-4.
   // Only a reconciled but unassigned level (-1) must block every grid level.
   return level >= 0 || level == -2 || level == -3 || level == -4;
  }

bool GridLevelHasLivePosition(int bi, int lv)
  {
   if(!In_ConfirmedExitRetry)
     {
      for(int k = 0; k < g_b[bi].npos; k++)
         if(g_b[bi].pos_lv[k] == lv) return true;
      return false;
     }
   ulong positions[]; ulong orders[];
   if(!ExitOwnedSnapshot(bi, positions, orders)) return true;
   for(int p = 0; p < ArraySize(positions); p++)
     {
      bool known = false;
      for(int k = 0; k < g_b[bi].npos; k++)
         if(g_b[bi].pos[k] == positions[p])
           {
            known = true;
            if(!NativeLevelKnown(g_b[bi].pos_lv[k]) || g_b[bi].pos_lv[k] == lv) return true;
            break;
           }
      if(!known) return true;
     }
   // A racing pending not yet assigned to a cached level cannot be ignored.
   for(int p = 0; p < ArraySize(orders); p++)
     {
      bool known = false;
      for(int k = 0; k < g_b[bi].npend; k++)
         if(g_b[bi].pend[k] == orders[p])
           { known = NativeLevelKnown(g_b[bi].pend_lv[k]); break; }
      if(!known) return true;
     }
   return false;
  }

int PlaceGrid(int bi, bool tylko_brakujace = false, int max_szczebli = 0,
              bool odtwarzaj_wypelnione = false)
  {
   if(!ExitRiskAllowed(bi)) return 0;
   if(EntryReviewBlocked(bi)) return 0;
   int placed = 0;
   int side = g_b[bi].side;
   string kom = "B" + IntegerToString(g_b[bi].id);
   // Zlecenie STOP jest typem własnym koszyka, nie awaryjną polityką
   // przecięcia. Przy wyłączonej osi zachowujemy historyczną ścieżkę.
   bool zlecenie_stop = g_b[bi].is_stop && In_HonorStopOrders;
   bool chce_rynek = (!g_b[bi].is_limit && !In_AutoLimit && !zlecenie_stop);

   // MarketEntryMode=Single: jedna pozycja z łącznym wolumenem (engine plan_grid)
   if(chce_rynek && In_MarketEntryMode == 1 && !tylko_brakujace)
     {
      double suma = 0.0;
      for(int i = 0; i < g_b[bi].nlv; i++) suma += g_b[bi].lv_vol[i] * g_b[bi].lv_units[i];
      if(suma <= 0.0) return 0;
      double px_rynek = EntryPx(side);
      bool ma_cap = false;
      double cap = MarketRiskCap(bi, ma_cap);
      double skala = MarketRiskScale(suma, px_rynek, g_b[bi].sl, g_b[bi].has_sl, cap, ma_cap);
      double vol = WolumenZlecenia(MathMax(suma * skala, In_LotMin));
      // engine.rs:3619: nawet minimalny lot ponad budżet = ODMOWA wejścia
      if(ma_cap && g_b[bi].has_sl)
        {
         double ryzyko = MathAbs(px_rynek - g_b[bi].sl) * XAU_CONTRACT * vol;
         if(ryzyko > cap + 1e-9) return 0;
        }
      double tp; bool htp = TargetForEx(bi, 0, 1, tp);
      ulong tk = 0;
      if(WyslijRynek(bi, 0, vol, g_b[bi].sl, g_b[bi].has_sl, tp, htp, kom, tk))
        {
         if(g_b[bi].npos < MAXTK)
           { g_b[bi].pos[g_b[bi].npos] = tk; g_b[bi].pos_lv[g_b[bi].npos] = 0; g_b[bi].npos++; }
         ZapiszWlasciciela(tk, bi);
         g_b[bi].had_positions = true;
         g_b[bi].state = ST_WORKING;
         g_b[bi].lv_filled[0] = true;
         if(g_b[bi].lv_fill_ts[0] == 0) g_b[bi].lv_fill_ts[0] = g_now;
         g_b[bi].last_entry_px = px_rynek; g_b[bi].has_last_entry = true;
         placed = 1; g_cnt_order++;
        }
      return placed;
     }

   // ---- FAZA A: ile lotow pojdzie PO RYNKU (engine.rs sync_grid) ----
   double px_rynek = EntryPx(side);
   double loty_rynkowe = 0.0;
   if(chce_rynek || In_PendingCrossPol == 0)
      for(int i = 0; i < g_b[bi].nlv; i++)
        {
         if(tylko_brakujace)
           {
            if(!odtwarzaj_wypelnione && g_b[bi].lv_filled[i]) continue;
            if(!odtwarzaj_wypelnione && (In_SyncOnlyLiveLevels || max_szczebli > 0)
               && g_b[bi].lv_cancelled[i]) continue;
            bool live_pend = false;
            for(int k2 = 0; k2 < g_b[bi].npend; k2++)
               if(g_b[bi].pend_lv[k2] == i) { live_pend = true; break; }
            if(live_pend) continue;
            if(odtwarzaj_wypelnione)
              {
               if(GridLevelHasLivePosition(bi, i)) continue;
              }
           }
         bool mozliwe = zlecenie_stop ? StopPxIsValid(side, g_b[bi].lv_price[i])
                                      : LimitPxIsValid(side, g_b[bi].lv_price[i]);
         if(!(chce_rynek || !mozliwe)) continue;
         loty_rynkowe += g_b[bi].lv_vol[i] * g_b[bi].lv_units[i];
        }
   bool   ma_cap = false;
   double cap_rynek = MarketRiskCap(bi, ma_cap);
   double skala_rynek = MarketRiskScale(loty_rynkowe, px_rynek, g_b[bi].sl, g_b[bi].has_sl,
                                        cap_rynek, ma_cap);
   double ryzyko_rynkowe = 0.0;
   bool   budzet_wyczerpany = false;
   bool   ml_stop = false;

   // ---- FAZA B: rozstawianie ----
   for(int i = 0; i < g_b[bi].nlv && !ml_stop; i++)
     {
      if(tylko_brakujace)
        {
         // Przezbrojenie (`only_existing=false` w rdzeniu) odtwarza również
         // szczebel wcześniej wypełniony/skasowany. Tryby synchronizacji
         // istniejącej siatki honorują cancelled tylko za osią Z-7.
         if(!odtwarzaj_wypelnione && g_b[bi].lv_filled[i]) continue;
         if(!odtwarzaj_wypelnione && (In_SyncOnlyLiveLevels || max_szczebli > 0)
            && g_b[bi].lv_cancelled[i]) continue;
         bool ma_zlecenie = false;
         for(int k2 = 0; k2 < g_b[bi].npend; k2++)
            if(g_b[bi].pend_lv[k2] == i) { ma_zlecenie = true; break; }
         if(ma_zlecenie) continue;
         // Szczebel, na ktorym stoi ZYWA POZYCJA, tez nie dostaje drugiego
         // zlecenia — silnik liczy go w `have` i nie dostawia ponad `want`.
         if(odtwarzaj_wypelnione)
           {
            if(GridLevelHasLivePosition(bi, i)) continue;
           }
        }
      // reżim zmienności — engine sync_grid:3400/3475 stosuje RÓWNOLEGLE:
      // (a) budżet poziomów: allowed = round(nlv×f).max(1) tylko przy f<1,
      // (b) scaled_units na KAŻDYM poziomie: want = round(units×f).max(1),
      //     także dla f>1 (więcej jednostek w spokojnym rynku).
      double vf = VolFactor();
      if(vf < 1.0 - 1e-9)
        {
         int allowed = (int)MathMax(MathRound(g_b[bi].nlv * vf), 1.0);
         if(i >= allowed) continue;   // poziom ucięty przez reżim
        }
      int want = g_b[bi].lv_units[i];
      if(MathAbs(vf - 1.0) > 1e-9)
         want = (int)MathMax(MathRound(want * vf), 1.0);
      for(int u = 0; u < want; u++)
        {
         // bramka marginesu PRZED KAŻDĄ jednostką — engine.rs:3641 (jeżeli
         // szósta zabiera margines poniżej progu, siódma nie ma prawa polecieć)
         if(!MarginesPozwala(In_MlMinWarstwa)) { ml_stop = true; break; }
         bool zmiesci = zlecenie_stop ? StopPxIsValid(side, g_b[bi].lv_price[i])
                                      : LimitPxIsValid(side, g_b[bi].lv_price[i]);
         int  zast = zmiesci ? -1 : In_PendingCrossPol;   // -1 = brak zastępczego
         if(zast == 1) continue;                          // Skip
         bool jako_rynek = chce_rynek || (zast == 0);     // Market
         ulong tk = 0;
         bool ok;
         if(jako_rynek)
           {
            double vol_r = WolumenZlecenia(MathMax(g_b[bi].lv_vol[i] * skala_rynek, In_LotMin));
            if(budzet_wyczerpany) break;
            if(ma_cap && g_b[bi].has_sl)
              {
               double rr = MathAbs(px_rynek - g_b[bi].sl) * XAU_CONTRACT * vol_r;
               if(ryzyko_rynkowe + rr > cap_rynek + 1e-9) { budzet_wyczerpany = true; break; }
               ryzyko_rynkowe += rr;
              }
            ok = WyslijRynek(bi, i, vol_r, g_b[bi].sl, g_b[bi].has_sl,
                             g_b[bi].lv_tp[i], g_b[bi].lv_has_tp[i], kom, tk);
            if(ok)
              {
               if(g_b[bi].npos < MAXTK)
                 { g_b[bi].pos[g_b[bi].npos] = tk; g_b[bi].pos_lv[g_b[bi].npos] = i; g_b[bi].npos++; }
               ZapiszWlasciciela(tk, bi);
               g_b[bi].had_positions = true;
               g_b[bi].state = ST_WORKING;
               if(g_b[bi].lv_fill_ts[i] == 0) g_b[bi].lv_fill_ts[i] = g_now;
               g_b[bi].lv_filled[i] = true;
               g_b[bi].last_entry_px = px_rynek; g_b[bi].has_last_entry = true;
              }
           }
         else
           {
            double px = g_b[bi].lv_price[i];
            int typ;
            if(zlecenie_stop && zmiesci)
               typ = (side == 0) ? ORDER_TYPE_BUY_STOP : ORDER_TYPE_SELL_STOP;
            else if(zast == 3 && StopPxIsValid(side, px))
               typ = (side == 0) ? ORDER_TYPE_BUY_STOP : ORDER_TYPE_SELL_STOP;
            else
              {
               if(zast == 3 || zast == 2) px = ClampLimitPx(side, px);
               typ = (side == 0) ? ORDER_TYPE_BUY_LIMIT : ORDER_TYPE_SELL_LIMIT;
              }
            ok = WyslijLimit(bi, px, g_b[bi].lv_vol[i], g_b[bi].sl, g_b[bi].has_sl,
                             g_b[bi].lv_tp[i], g_b[bi].lv_has_tp[i], kom,
                             typ, tk);
            if(ok && g_b[bi].npend < MAXTK)
              { g_b[bi].pend[g_b[bi].npend] = tk; g_b[bi].pend_lv[g_b[bi].npend] = i;
                g_b[bi].pend_top[g_b[bi].npend] = false; g_b[bi].npend++; }
           }
         if(ok) { placed++; g_cnt_order++; }
         else break;   // engine.rs: Err(_) => break — koniec prób na tym szczeblu
        }
      if(tylko_brakujace && max_szczebli > 0 && placed >= max_szczebli) break;
     }
   return placed;
  }

// Wymiana wolumenu pendingu: cancel+replace (MT5 nie zna modyfikacji wolumenu)
bool PodmienWolumen(int bi, int idx, double nowy_vol)
  {
   if(idx < 0 || idx >= g_b[bi].npend) return false;
   ulong t = g_b[bi].pend[idx];
   if(!OrderSelect(t)) return false;
   double px  = OrderGetDouble(ORDER_PRICE_OPEN);
   double sl  = OrderGetDouble(ORDER_SL);
   double tp  = OrderGetDouble(ORDER_TP);
   int    typ = (int)OrderGetInteger(ORDER_TYPE);
   string kom = OrderGetString(ORDER_COMMENT);
   int    lv  = g_b[bi].pend_lv[idx];
   double stary = OrderGetDouble(ORDER_VOLUME_CURRENT);
   if(MathAbs(nowy_vol - stary) < 0.005) return false;

   MqlTradeRequest r; MqlTradeResult res;
   ZeroMemory(r); ZeroMemory(res);
   r.action = TRADE_ACTION_REMOVE;
   r.order  = t;
   if(!OrderSend(r, res) || res.retcode != TRADE_RETCODE_DONE) return false;

   for(int j = idx; j < g_b[bi].npend - 1; j++)
     {
      g_b[bi].pend[j]     = g_b[bi].pend[j+1];
      g_b[bi].pend_lv[j]  = g_b[bi].pend_lv[j+1];
      g_b[bi].pend_top[j] = g_b[bi].pend_top[j+1];
     }
   g_b[bi].npend--;

   ulong tk = 0;
   if(!WyslijLimit(bi, px, nowy_vol, sl, sl > 0.0, tp, tp > 0.0, kom, typ, tk))
      return false;   // szczebel zostaje pusty — tak samo zachowuje sie silnik
   if(g_b[bi].npend < MAXTK)
     {
      g_b[bi].pend[g_b[bi].npend]     = tk;
      g_b[bi].pend_lv[g_b[bi].npend]  = lv;
      g_b[bi].pend_top[g_b[bi].npend] = false;
      g_b[bi].npend++;
     }
   return true;
  }

bool UsunPending(int bi, int idx)
  {
   if(!ExitRiskAllowed(bi)) return false;
   if(idx < 0 || idx >= g_b[bi].npend) return false;
   if(In_ConfirmedExitRetry) return ExitCancelOwned(bi, g_b[bi].pend[idx]);
   MqlTradeRequest r; MqlTradeResult res;
   ZeroMemory(r); ZeroMemory(res);
   r.action = TRADE_ACTION_REMOVE;
   r.order  = g_b[bi].pend[idx];
   if(!OrderSend(r, res) || res.retcode != TRADE_RETCODE_DONE) return false;
   for(int j = idx; j < g_b[bi].npend - 1; j++)
     {
      g_b[bi].pend[j]     = g_b[bi].pend[j+1];
      g_b[bi].pend_lv[j]  = g_b[bi].pend_lv[j+1];
      g_b[bi].pend_top[j] = g_b[bi].pend_top[j+1];
     }
   g_b[bi].npend--;
   return true;
  }


//====================================================================
//  RELOT ZLECEN OCZEKUJACYCH — engine.rs:6539 relot_pendings
//  + kierunki up/down, prog salda, bramka ml_min_relot_up
//====================================================================
int    g_relot_up     = 0;
int    g_relot_down   = 0;
int    g_relot_ok     = 0;
int    g_relot_odmowy = 0;
double g_relot_lotow  = 0.0;

void RelotPendings()
  {
   if(!In_RelotOnBalance) return;
   long gap = (long)(MathMax(In_PendingResizeS, 0.0) * 1000.0);
   if(g_now - g_last_relot < gap) return;
   g_last_relot = g_now;

   // prog salda dla relotu W GORE (pending_relot_up_od_salda)
   bool up_dozwolony = In_RelotUp;
   if(up_dozwolony && In_RelotUpOdSalda > 0.0
      && AccountInfoDouble(ACCOUNT_BALANCE) < In_RelotUpOdSalda)
      up_dozwolony = false;
   if(up_dozwolony && !MarginesPozwala(In_MlMinRelotUp))
      up_dozwolony = false;

   for(int bi = 0; bi < g_nb; bi++)
     {
      double cel = LotSize();       // GOLY lot bazowy przy biezacym saldzie
      if(g_b[bi].state == ST_DONE || !ExitRiskAllowed(bi) || g_b[bi].npend == 0) continue;
      if(EntryReviewBlocked(bi)) continue;

      for(int lv = 0; lv < g_b[bi].nlv; lv++)
        {
         int    sztuki = 0;
         double suma   = 0.0;
         int    i_baza = -1;          // pierwsze zlecenie BAZOWE na tym poziomie
         int    i_top  = -1;          // ostatnia DOKLADKA (zdejmowana w dol)
         for(int k = 0; k < g_b[bi].npend; k++)
           {
            if(g_b[bi].pend_lv[k] != lv) continue;
            if(!OrderSelect(g_b[bi].pend[k])) continue;
            suma += OrderGetDouble(ORDER_VOLUME_CURRENT);
            if(g_b[bi].pend_top[k]) i_top = k;
            else { sztuki++; if(i_baza < 0) i_baza = k; }
           }
         if(suma <= 0.0 || i_baza < 0) continue;
         if(sztuki < 1) sztuki = 1;

         double cel_szczebla;
         if(In_RelotWgPlanu)
           {
            double baza_planu = (g_b[bi].lot_planu > 0.0) ? g_b[bi].lot_planu : cel;
            double waga = g_b[bi].lv_vol[lv] / baza_planu;
            cel_szczebla = MathRound(sztuki * waga * cel * 100.0) / 100.0;
           }
         else
            cel_szczebla = MathRound(sztuki * cel * 100.0) / 100.0;

         suma = MathRound(suma * 100.0) / 100.0;
         double roznica = cel_szczebla - suma;
         if(MathAbs(roznica) < 0.005) continue;

         if(roznica > 0.0)
           {
            if(!up_dozwolony) continue;
            g_relot_up++;
            g_relot_lotow += MathAbs(roznica);
            if(In_RelotTopup)
              {
               if(!OrderSelect(g_b[bi].pend[i_baza])) continue;
               double  px  = OrderGetDouble(ORDER_PRICE_OPEN);
               double  sl  = OrderGetDouble(ORDER_SL);
               double  tp  = OrderGetDouble(ORDER_TP);
               int     typ = (int)OrderGetInteger(ORDER_TYPE);
               string  kom = OrderGetString(ORDER_COMMENT);
               ulong   tk  = 0;
               // engine.rs:6819: dokladka przez wolumen_zlecenia (podloga
               // lot_min, SUFIT lot_max) — bez tego przeciekal ogranicznik
               // („najwieksza pozycja miala 27,10 lota przy lot_max=10")
               if(WyslijLimit(bi, px, WolumenZlecenia(roznica), sl, sl > 0.0,
                              tp, tp > 0.0, kom, typ, tk))
                 {
                  if(g_b[bi].npend < MAXTK)
                    {
                     g_b[bi].pend[g_b[bi].npend]     = tk;
                     g_b[bi].pend_lv[g_b[bi].npend]  = lv;
                     g_b[bi].pend_top[g_b[bi].npend] = true;   // DOKLADKA
                     g_b[bi].npend++;
                    }
                  g_relot_ok++;
                 }
               else g_relot_odmowy++;
              }
            else
              {
               double jednostka = MathMax(cel_szczebla / (double)sztuki, 0.01);
               if(PodmienWolumen(bi, i_baza, WolumenZlecenia(jednostka))) g_relot_ok++;
               else g_relot_odmowy++;
              }
           }
         else
           {
            if(!In_RelotDown) continue;
            g_relot_down++;
            g_relot_lotow += MathAbs(roznica);
            // engine.rs:6745-6806: w JEDNYM przebiegu zdejmij tyle dokladek,
            // ile trzeba, a reszte nadmiaru zdejmij z BAZY (vol - nadmiar).
            double nadmiar = -roznica;
            while(nadmiar >= 0.005)
              {
               int it = -1;
               for(int k = 0; k < g_b[bi].npend; k++)
                  if(g_b[bi].pend_lv[k] == lv && g_b[bi].pend_top[k]) it = k;
               if(it < 0) break;
               double v_top = OrderSelect(g_b[bi].pend[it])
                  ? OrderGetDouble(ORDER_VOLUME_CURRENT) : 0.0;
               if(UsunPending(bi, it)) { g_relot_ok++; nadmiar -= v_top; }
               else { g_relot_odmowy++; break; }
              }
            if(nadmiar >= 0.005)
              {
               // odszukaj biezaca baze (indeksy mogly sie przesunac)
               int ib = -1;
               for(int k = 0; k < g_b[bi].npend; k++)
                  if(g_b[bi].pend_lv[k] == lv && !g_b[bi].pend_top[k]) { ib = k; break; }
               if(ib >= 0 && OrderSelect(g_b[bi].pend[ib]))
                 {
                  double v_bazy = OrderGetDouble(ORDER_VOLUME_CURRENT);
                  double nowy = WolumenZlecenia(MathMax(v_bazy - nadmiar, 0.01));
                  if(PodmienWolumen(bi, ib, nowy)) g_relot_ok++;
                  else g_relot_odmowy++;
                 }
              }
           }
        }
     }
  }

//====================================================================
//  KASOWANIE SIATKI (+keep_n — engine.rs:4883, +znaczniki cancelled)
//====================================================================
int CancelPendings(int bi)
  {
   if(In_ConfirmedExitRetry)
     {
      if(ConfirmedExitPending(bi)) return 0; // one cadence/owner controls full exits
      ulong positions[]; ulong orders[];
      ExitOwnedSnapshot(bi, positions, orders);
      int count = 0;
      for(int i = 0; i < ArraySize(orders); i++)
         if(ExitCancelOwned(bi, orders[i])) count++;
      // Only broker-confirmed cancellations remove local tickets.
      return count;
     }
   int n = 0;
   for(int i = g_b[bi].npend - 1; i >= 0; i--)
     {
      ulong t = g_b[bi].pend[i];
      if(OrderSelect(t))
        {
         MqlTradeRequest r; MqlTradeResult res;
         ZeroMemory(r); ZeroMemory(res);
         r.action = TRADE_ACTION_REMOVE;
         r.order  = t;
         if(OrderSend(r, res)) n++;
        }
      int lv = g_b[bi].pend_lv[i];
      if(lv >= 0 && lv < g_b[bi].nlv && !g_b[bi].lv_filled[lv]) g_b[bi].lv_cancelled[lv] = true;
      for(int j = i; j < g_b[bi].npend - 1; j++)
        { g_b[bi].pend[j] = g_b[bi].pend[j+1]; g_b[bi].pend_lv[j] = g_b[bi].pend_lv[j+1];
          g_b[bi].pend_top[j] = g_b[bi].pend_top[j+1]; }
      g_b[bi].npend--;
     }
   return n;
  }

int WithdrawPendingSource(int bi)
  {
   if(bi < 0 || bi >= g_nb) return 0;
   g_b[bi].source_withdrawn = true;
   g_b[bi].drop_po_ts = 0;
   ulong positions[]; ulong orders[];
   ExitOwnedSnapshot(bi, positions, orders);
   int cancelled = 0;
   for(int i = 0; i < ArraySize(orders); i++)
      if(ExitCancelOwned(bi, orders[i])) cancelled++;
   for(int level = 0; level < g_b[bi].nlv; level++)
      if(!g_b[bi].lv_filled[level]) g_b[bi].lv_cancelled[level] = true;
   ExitOwnedSnapshot(bi, positions, orders);
   if(ArraySize(positions) == 0 && ArraySize(orders) == 0) g_b[bi].state = ST_DONE;
   return cancelled;
  }
void RetrySourceCancellations()
  {
   for(int bi = 0; bi < g_nb; bi++)
      if(SourceWithdrawn(bi)) WithdrawPendingSource(bi);
  }

// Kasuje siatkę, ale ZOSTAWIA n najpłytszych zleceń — engine.rs:4883.
// Najpłytsze = najbliżej ceny (BUY: najwyższa cena zlecenia).
int CancelPendingsKeep(int bi, int keep_n)
  {
   if(!ExitRiskAllowed(bi)) return 0;
   if(keep_n <= 0) return CancelPendings(bi);
   if(g_b[bi].npend <= keep_n) return 0;
   // sortowanie indeksów pend po "płytkości"
   int idx[MAXTK];
   double px[MAXTK];
   int n = g_b[bi].npend;
   for(int i = 0; i < n; i++)
     {
      idx[i] = i;
      px[i] = OrderSelect(g_b[bi].pend[i]) ? OrderGetDouble(ORDER_PRICE_OPEN) : 0.0;
     }
   // Stable insertion order for equal pending prices.
   for(int i=1;i<n;i++)
     {
      int index=idx[i],j=i;double price=px[i];
      while(j>0 && (g_b[bi].side==0 ? price>px[j-1] : price<px[j-1]))
        {idx[j]=idx[j-1];px[j]=px[j-1];j--;}
      idx[j]=index;px[j]=price;
     }
   // kasujemy od najgłębszych (pozycje keep_n..n-1 w rankingu płytkości)
   int do_kasacji[MAXTK]; int nk = 0;
   for(int i = keep_n; i < n; i++) { do_kasacji[nk] = idx[i]; nk++; }
   // sortuj indeksy malejąco, żeby usuwanie nie psuło numeracji
   for(int a = 0; a < nk - 1; a++)
      for(int b2 = a + 1; b2 < nk; b2++)
         if(do_kasacji[b2] > do_kasacji[a])
           { int t = do_kasacji[a]; do_kasacji[a] = do_kasacji[b2]; do_kasacji[b2] = t; }
   int k = 0;
   for(int i = 0; i < nk; i++)
      if(UsunPendingZnacz(bi, do_kasacji[i])) k++;
   return k;
  }
bool UsunPendingZnacz(int bi, int idx)
  {
   if(idx < 0 || idx >= g_b[bi].npend) return false;
   int lv = g_b[bi].pend_lv[idx];
   if(!UsunPending(bi, idx)) return false;
   if(lv >= 0 && lv < g_b[bi].nlv && !g_b[bi].lv_filled[lv]) g_b[bi].lv_cancelled[lv] = true;
   return true;
  }

// blisko_strefy — engine.rs:7525 (straż odległości okna łaski; od bliższej
// krawędzi, ceną STRONY koszyka: BUY=ask, SELL=bid — engine.rs:7534)
bool BliskoStrefy(int bi)
  {
   if(In_DropGraceMaxDist <= 0.0) return true;
   double cena = (g_b[bi].side == 0) ? g_ask : g_bid;
   double d1 = MathAbs(cena - g_b[bi].zone_lo);
   double d2 = MathAbs(cena - g_b[bi].zone_hi);
   double d = MathMin(d1, d2);
   if(cena >= g_b[bi].zone_lo && cena <= g_b[bi].zone_hi) d = 0.0;
   return d <= In_DropGraceMaxDist;
  }

//====================================================================
//  ZAMYKANIE / MODYFIKACJE (+ kolejka zamiarów try_modify/retry_stops)
//====================================================================
string g_powod_zamk = "";

bool ZamknijPozycje(ulong t)
  {
   if(!PositionSelectByTicket(t)) return false;
   if(In_TestExitScenario > 0 && MQLInfoInteger(MQL_TESTER) && g_test_close_reject > 0)
     {
      g_test_close_reject--;
      PrintFormat("CEXIT_TEST_EVENT|close_rejected_injected|%I64d|%I64u", g_now, t);
      return false; // no close RPC: broker truth still contains the position
     }
   if(In_Diag && g_handle_diag != INVALID_HANDLE)
      FileWrite(g_handle_diag, "ZAMKNIJ", (string)g_now, (string)t, g_powod_zamk,
                StringFormat("%.2f/%.2f", g_bid, g_ask));
   MqlTradeRequest r; MqlTradeResult res;
   ZeroMemory(r); ZeroMemory(res);
   long typ = PositionGetInteger(POSITION_TYPE);
   r.action       = TRADE_ACTION_DEAL;
   r.position     = t;
   r.symbol       = _Symbol;
   r.volume       = PositionGetDouble(POSITION_VOLUME);
   r.type         = (typ == POSITION_TYPE_BUY) ? ORDER_TYPE_SELL : ORDER_TYPE_BUY;
   r.price        = (typ == POSITION_TYPE_BUY) ? g_bid : g_ask;
   r.deviation    = 50;
   r.magic        = In_Magic;
   r.type_filling = g_fill_deal;
   if(In_TestExitScenario > 0 && MQLInfoInteger(MQL_TESTER) && g_test_partial_remaining > 0)
     {
      g_test_partial_remaining--;
      r.volume = NormVol(r.volume / 2.0); // an actual partial deal, not a fake result
      PrintFormat("CEXIT_TEST_EVENT|partial_request_injected|%I64d|%I64u|%.4f", g_now, t, r.volume);
     }
   bool ok = OrderSend(r, res) && res.retcode == TRADE_RETCODE_DONE;
   if(In_TestExitScenario > 0 && MQLInfoInteger(MQL_TESTER))
     {
      double residual = PositionSelectByTicket(t) ? PositionGetDouble(POSITION_VOLUME) : 0.0;
      if(ok && residual > 0.0) g_test_saw_partial = true;
      PrintFormat("CEXIT_TEST_EVENT|close_ack|%I64d|%I64u|%u|%.4f|%.4f", g_now, t, res.retcode, r.volume, residual);
     }
   if(!ok)
     {
      g_zamk_blad++;
      ZliczOdrzucenie((int)res.retcode);
      if(In_Diag)
         FileWrite(g_handle_diag, "ZAMK_BLAD", (string)g_now, (string)t,
                   (string)res.retcode, StringFormat("%.2f", r.volume),
                   StringFormat("%.2f/%.2f", g_bid, g_ask), res.comment);
     }
   return ok;
  }

bool ZamknijCzesc(ulong t, double vol)
  {
   if(!PositionSelectByTicket(t)) return false;
   double current = PositionGetDouble(POSITION_VOLUME);
   double cut = NormVol(vol);
   // Ostatnia ochrona parity z Rust/Mt5Bridge: nigdy nie zostawiamy ogarka
   // ponizej minimum brokera. Ta galaz nie powinna zajsc po
   // PartialCloseVolume, ale chroni wywolanie funkcji po zmianie stanu.
   if(cut >= current - 1e-9 || current - cut < VolMin() - 1e-9)
      return ZamknijPozycje(t);
   MqlTradeRequest r; MqlTradeResult res;
   ZeroMemory(r); ZeroMemory(res);
   long typ = PositionGetInteger(POSITION_TYPE);
   r.action       = TRADE_ACTION_DEAL;
   r.position     = t;
   r.symbol       = _Symbol;
   r.volume       = cut;
   r.type         = (typ == POSITION_TYPE_BUY) ? ORDER_TYPE_SELL : ORDER_TYPE_BUY;
   r.price        = (typ == POSITION_TYPE_BUY) ? g_bid : g_ask;
   r.deviation    = 50;
   r.magic        = In_Magic;
   r.type_filling = g_fill_deal;
   bool ok = OrderSend(r, res) && res.retcode == TRADE_RETCODE_DONE;
   if(!ok) { g_zamk_blad++; ZliczOdrzucenie((int)res.retcode); }
   return ok;
  }

void ZapomnijZamiar(ulong t)
  {
   for(int i = 0; i < g_nzam; i++)
      if(g_zam_tk[i] == t)
        {
         for(int j = i; j < g_nzam - 1; j++)
           {
            g_zam_tk[j] = g_zam_tk[j+1]; g_zam_sl[j] = g_zam_sl[j+1]; g_zam_tp[j] = g_zam_tp[j+1];
            g_zam_hsl[j] = g_zam_hsl[j+1]; g_zam_htp[j] = g_zam_htp[j+1]; g_zam_ts[j] = g_zam_ts[j+1];
           }
         g_nzam--;
         return;
        }
  }

void ZapamietajZamiar(ulong t, double sl, bool hsl, double tp, bool htp)
  {
   for(int i = 0; i < g_nzam; i++)
      if(g_zam_tk[i] == t)
        { g_zam_sl[i] = sl; g_zam_hsl[i] = hsl; g_zam_tp[i] = tp; g_zam_htp[i] = htp;
          g_zam_ts[i] = g_now; return; }
   if(g_nzam >= MAXZAM) return;
   g_zam_tk[g_nzam] = t; g_zam_sl[g_nzam] = sl; g_zam_hsl[g_nzam] = hsl;
   g_zam_tp[g_nzam] = tp; g_zam_htp[g_nzam] = htp; g_zam_ts[g_nzam] = g_now;
   g_nzam++;
  }

bool ModyfikujPozycje(ulong t, double sl, bool has_sl, double tp, bool has_tp)
  {
   if(!PositionSelectByTicket(t)) return false;
   if(In_Diag && g_handle_diag != INVALID_HANDLE && In_DiagDetailFromMs > 0
      && g_now >= In_DiagDetailFromMs && g_now <= In_DiagDetailToMs)
      FileWrite(g_handle_diag, "RAW_SLTP", (string)g_now, (string)t,
                DoubleToString(sl,16), DoubleToString(NormPx(sl),16),
                DoubleToString(MathRound(sl*100.0)/100.0,16),
                DoubleToString(tp,16), has_sl, has_tp,
                DoubleToString(g_bid,16), DoubleToString(g_ask,16));
   int bok = (PositionGetInteger(POSITION_TYPE) == POSITION_TYPE_BUY) ? 0 : 1;
   if((has_sl && !SlIsValid(bok, sl)) || (has_tp && !TpIsValid(bok, tp)))
     {
      g_mod_blad++; g_rej_modify++;
      if(In_SltpRetryS > 0.0) ZapamietajZamiar(t, sl, has_sl, tp, has_tp);
      return false;
     }
   MqlTradeRequest r; MqlTradeResult res;
   ZeroMemory(r); ZeroMemory(res);
   r.action   = TRADE_ACTION_SLTP;
   r.position = t;
   r.symbol   = _Symbol;
   r.sl       = has_sl ? NormPx(sl) : 0.0;
   r.tp       = has_tp ? NormPx(tp) : 0.0;
   bool ok = OrderSend(r, res) && res.retcode == TRADE_RETCODE_DONE;
   if(ok) ZapomnijZamiar(t);
   else
     {
      g_mod_blad++; g_rej_modify++;
      if(In_SltpRetryS > 0.0) ZapamietajZamiar(t, sl, has_sl, tp, has_tp);
     }
   return ok;
  }

void PonowStopy()
  {
   if(In_SltpRetryS <= 0.0 || g_nzam == 0) return;
   long gap = (long)(In_SltpRetryS * 1000.0);
   for(int i = g_nzam - 1; i >= 0; i--)
     {
      ulong t = g_zam_tk[i];
      if(!PositionSelectByTicket(t)) { ZapomnijZamiar(t); continue; }
      if(ExitTicketPending(t)) { ZapomnijZamiar(t); continue; }
      if(g_now - g_zam_ts[i] < gap) continue;
      int bok = (PositionGetInteger(POSITION_TYPE) == POSITION_TYPE_BUY) ? 0 : 1;
      if(g_zam_hsl[i] && !SlIsValid(bok, g_zam_sl[i])) continue;
      if(g_zam_htp[i] && !TpIsValid(bok, g_zam_tp[i])) continue;
      MqlTradeRequest r; MqlTradeResult res;
      ZeroMemory(r); ZeroMemory(res);
      r.action   = TRADE_ACTION_SLTP;
      r.position = t;
      r.symbol   = _Symbol;
      r.sl       = g_zam_hsl[i] ? NormPx(g_zam_sl[i]) : 0.0;
      r.tp       = g_zam_htp[i] ? NormPx(g_zam_tp[i]) : 0.0;
      if(OrderSend(r, res) && res.retcode == TRADE_RETCODE_DONE) ZapomnijZamiar(t);
      else { g_zam_ts[i] = g_now; g_retry_fail++; }
     }
  }

void CloseOrQueue(ulong t, string reason)
  {
   if(ExitTicketPending(t)) return;
   if(!In_ExitViaLimit)
     { g_powod_zamk = reason; ZamknijPozycje(t); return; }
   if(QeIdx(t) >= 0) return;
   if(!PositionSelectByTicket(t)) return;
   int side = (PositionGetInteger(POSITION_TYPE) == POSITION_TYPE_BUY) ? 0 : 1;
   double zysk = PozPunkty(t);
   if(zysk < In_ExitLimitMinProfit)
     { g_powod_zamk = reason; ZamknijPozycje(t); return; }
   double spread = MathMax(g_ask - g_bid, 0.0);
   double zapas = spread + MathMax(In_ExitLimitOffset, 0.0);
   if(zapas <= 0.0)
     { g_powod_zamk = reason; ZamknijPozycje(t); return; }
   double rynek = ExitPx(side);
   double target = (side == 0) ? (rynek + zapas) : (rynek - zapas);
   if(g_nqe >= MAXQE) { g_powod_zamk = reason; ZamknijPozycje(t); return; }
   g_qe_tk[g_nqe] = t;
   g_qe_target[g_nqe] = target;
   g_qe_deadline[g_nqe] = g_now + (long)(MathMax(In_ExitLimitWaitS, 0.0) * 1000.0);
   g_qe_mkt[g_nqe] = rynek;
   g_qe_reason[g_nqe] = reason;
   g_nqe++;
  }
void SweepQueuedExits()
  {
   for(int i = g_nqe - 1; i >= 0; i--)
     {
      ulong t = g_qe_tk[i];
      if(!PositionSelectByTicket(t)) { QeForget(i); continue; }
      if(ExitTicketPending(t)) { QeForget(i); continue; }
      int side = (PositionGetInteger(POSITION_TYPE) == POSITION_TYPE_BUY) ? 0 : 1;
      bool osiagniete = (side == 0) ? (g_bid >= g_qe_target[i] - 1e-9)
                                    : (g_ask <= g_qe_target[i] + 1e-9);
      bool spozniony = (g_now >= g_qe_deadline[i]);
      if(!osiagniete && !spozniony) continue;
      g_powod_zamk = g_qe_reason[i] + (osiagniete ? "_LIMIT" : "_TIMEOUT");
      ZamknijPozycje(t);
      QeForget(i);
     }
  }
bool JestWKolejceWyjscia(ulong t) { return QeIdx(t) >= 0; }

// W31a — odpowiednik pętli engine.rs po rekoncyliacji. Obejmuje nie tylko
// pendingi, ale też re-entry i dokładki rynkowe utworzone po komendzie BE.
void CoverLateFills()
  {
   if(!In_BeCoversLateFills) return;
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!ExitRiskAllowed(bi)) continue;
      if(!Alive(bi) || g_b[bi].be_ts <= 0) continue;
      for(int i = 0; i < g_b[bi].npos; i++)
        {
         ulong t = g_b[bi].pos[i];
         if(!PositionSelectByTicket(t)) continue;
         long open_ts = (long)PositionGetInteger(POSITION_TIME_MSC);
         if(open_ts < g_b[bi].be_ts) continue;
         double op = PositionGetDouble(POSITION_PRICE_OPEN);
         double cur_sl = PositionGetDouble(POSITION_SL);
         double cur_tp = PositionGetDouble(POSITION_TP);
         double be = op + SideSign(g_b[bi].side) * In_BeOffset;
         // SideBetter(cur, be): obecny stop leży głębiej i warto go podnieść.
         bool warto = (cur_sl == 0.0) || SideBetter(g_b[bi].side, cur_sl, be);
         if(!warto || !SlIsValid(g_b[bi].side, be)) continue;
         if(ModyfikujPozycje(t, be, true, cur_tp, cur_tp != 0.0)
            && In_Diag && g_handle_diag != INVALID_HANDLE)
            FileWrite(g_handle_diag, "BE_LATE_FILL", (string)g_now,
                      (string)g_b[bi].id, (string)t,
                      StringFormat("%.2f", op), StringFormat("%.2f", be));
        }
     }
  }

//====================================================================
//  ODŚWIEŻENIE REJESTRU BILETÓW (rekoncyliacja — engine.rs:5176)
//  W hedgingu bilet pozycji == bilet zlecenia, które ją otworzyło.
//  + tp_stage_from_broker_fill (engine.rs:5216)
//====================================================================
void OdswiezBilety()
  {
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(g_b[bi].state == ST_DONE && !In_ConfirmedExitRetry) continue;
      // Preserve order submission order for fills observed in one snapshot.
      // Rust appends those receipts in this order; reversing them changes the
      // retained grid level when RiskFree compares identical fill prices.
      for(int i = 0; i < g_b[bi].npend; )
        {
         ulong t = g_b[bi].pend[i];
         if(OrderSelect(t)) { i++; continue; }     // still pending
         int lv = g_b[bi].pend_lv[i];
         bool stal_sie_pozycja = PositionSelectByTicket(t);
         for(int j = i; j < g_b[bi].npend - 1; j++)
           { g_b[bi].pend[j] = g_b[bi].pend[j+1]; g_b[bi].pend_lv[j] = g_b[bi].pend_lv[j+1];
          g_b[bi].pend_top[j] = g_b[bi].pend_top[j+1]; }
         g_b[bi].npend--;
         if(stal_sie_pozycja && g_b[bi].npos < MAXTK)
           {
            bool listed = false;
            if(In_ConfirmedExitRetry)
               for(int k = 0; k < g_b[bi].npos; k++) if(g_b[bi].pos[k] == t) { listed = true; g_b[bi].pos_lv[k] = lv; break; }
            if(!listed) { g_b[bi].pos[g_b[bi].npos] = t; g_b[bi].pos_lv[g_b[bi].npos] = lv; g_b[bi].npos++; }
            ZapiszWlasciciela(t, bi);
            g_b[bi].had_positions = true;
            g_b[bi].state = (g_b[bi].state == ST_RISKFREE) ? ST_RISKFREE : ST_WORKING;
            if(lv >= 0 && lv < g_b[bi].nlv)
              { if(g_b[bi].lv_fill_ts[lv] == 0) g_b[bi].lv_fill_ts[lv] = g_now;
                g_b[bi].lv_filled[lv] = true; }
            // PREMIA WYPEŁNIENIA (D3): fill limitu vs poziom zlecenia
            double fill = PositionGetDouble(POSITION_PRICE_OPEN);
            if(lv >= 0 && lv < g_b[bi].nlv)
              {
               double poziom = g_b[bi].lv_price[lv];
               double prem = (poziom - fill) * SideSign(g_b[bi].side);
               g_premia_n++;
               if(prem > 1e-9)
                 {
                  g_premia_lepiej++;
                  g_premia_usd += prem * PositionGetDouble(POSITION_VOLUME) * XAU_CONTRACT;
                 }
              }
            // wirtualny SL dla wypełnień z limitów (engine.rs:5575-5585)
            if(g_b[bi].has_sl && (In_VirtualSl || In_VirtualSlAll))
              { int ip = PsEnsure(t); if(ip >= 0 && g_ps_vsl[ip] == 0.0) g_ps_vsl[ip] = g_b[bi].sl; }
            PsEnsure(t);
           }
        }
      // pozycje zamknięte przez brokera (TP/SL/stop out)
      for(int i = g_b[bi].npos - 1; i >= 0; i--)
        {
         ulong t = g_b[bi].pos[i];
         if(PositionSelectByTicket(t))
           {
            double volume = PositionGetDouble(POSITION_VOLUME);
            int ip = PsEnsure(t);
            if(ip >= 0 && volume < g_ps_last_vol[ip] - 1e-9)
              {
               double partial_price; bool partial_tp;
               if(!ReconcilePositionRealized(bi, t, partial_price, partial_tp)) continue;
              }
            if(ip >= 0) g_ps_last_vol[ip] = volume;
            continue;
           }
         // KSIEGOWANIE realized KOSZYKA (engine.rs:5148 bk.realized += profit)
         // — czyta je riskfree_trigger i rearm_min_basket_profit.
         double cena_out = 0.0; bool byl_tp = false;
         if(ReconcilePositionRealized(bi, t, cena_out, byl_tp))
           {
            // tp_stage_from_broker_fill (engine.rs:5216-5242): NAJWYZSZY indeks
            // celu osiagniety/miniety cena zamkniecia — kierunkowo, bez tolerancji;
            // dziala niezaleznie od tp_source.
            if(In_TpStageFromFill && byl_tp && g_b[bi].ntp > 0 && cena_out > 0.0)
              {
               int naj = 0;
               for(int q2 = 0; q2 < g_b[bi].ntp; q2++)
                 {
                  bool minieta = (g_b[bi].side == 0)
                     ? (cena_out >= g_b[bi].tps[q2] - 1e-9)
                     : (cena_out <= g_b[bi].tps[q2] + 1e-9);
                  if(minieta) naj = q2 + 1;
                 }
               if(naj > g_b[bi].tp_stage) HandleTpHit(bi, naj);
              }
           }
         else continue; // retain ownership and retry when broker history is available
         PsForget(t);
         for(int j = i; j < g_b[bi].npos - 1; j++)
           { g_b[bi].pos[j] = g_b[bi].pos[j+1]; g_b[bi].pos_lv[j] = g_b[bi].pos_lv[j+1]; }
         g_b[bi].npos--;
        }
     }
  }

//====================================================================
//  SZCZYTY POZYCJI (peak_pts / last_peak_ts) — engine.rs:6006
//====================================================================
void UpdatePeaks()
  {
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!Alive(bi)) continue;
      for(int i = 0; i < g_b[bi].npos; i++)
        {
         ulong t = g_b[bi].pos[i];
         if(!PositionSelectByTicket(t)) continue;
         int ip = PsEnsure(t);
         if(ip < 0) continue;
         int side = (PositionGetInteger(POSITION_TYPE) == POSITION_TYPE_BUY) ? 0 : 1;
         double pts = (ExitPx(side) - PositionGetDouble(POSITION_PRICE_OPEN)) * SideSign(side);
         if(pts > g_ps_peak[ip]) { g_ps_peak[ip] = pts; g_ps_peak_ts[ip] = g_now; }
        }
      // pierwsze dotkniecia celow cena (dla TpSource okien czasowych)
      int side2 = g_b[bi].side;
      for(int q2 = 0; q2 < g_b[bi].ntp; q2++)
        {
         if(g_b[bi].tp_touch_ts[q2] != 0) continue;
         bool touched = (side2 == 0) ? (g_bid >= g_b[bi].tps[q2]) : (g_ask <= g_b[bi].tps[q2]);
         if(touched) g_b[bi].tp_touch_ts[q2] = g_now;
        }
     }
  }


//====================================================================
//  ADAPTACYJNE PARAMETRY WEJŚCIA (engine.rs:1038 adaptive_sl_min_dist)
//====================================================================
double AdaptiveSlMinDist(double sig_width)
  {
   double baza = KapF(In_SlMinDist, In_SlMinDistSmall, In_SlMinDistSmallM);
   if(!In_AdaptiveParams) return baza;
   double v = 0.0; bool zrodlo = false;
   if(In_SlMinDistZoneMult > 0.0 && sig_width > 0.0)
     { v = MathMax(v, In_SlMinDistZoneMult * sig_width); zrodlo = true; }
   if(In_SlMinDistAtrMult > 0.0)
     {
      double atr;
      if(AtrProxy(atr)) { v = MathMax(v, In_SlMinDistAtrMult * atr); zrodlo = true; }
     }
   if(!zrodlo) return baza;
   if(In_SlMinDistFloor > 0.0) v = MathMax(v, In_SlMinDistFloor);
   if(In_SlMinDistCap   > 0.0) v = MathMin(v, In_SlMinDistCap);
   return v;
  }

// adaptive_deep_offset — engine.rs:1077: rozciągnięcie strefy z szerokości
double AdaptiveDeepOffset(double sig_width)
  {
   if(In_AdaptiveParams && In_EntryDeepZoneMult > 0.0 && sig_width > 0.0)
      return In_EntryDeepZoneMult * sig_width;
   return In_EntryDeepOffset;
  }
// adaptive_units — engine.rs:1092: szczeble z szerokości strefy, clamp 1..3×base
// (mnożnik godzinowy units_by_hour poza zakresem v1 — puste = 1.0 jak w silniku)
int AdaptiveUnits(int base, double sig_width)
  {
   if(!In_AdaptiveParams) return base;
   double f = 1.0;
   if(In_EntryUnitsZoneRef > 0.0 && sig_width > 0.0)
      f *= sig_width / In_EntryUnitsZoneRef;
   if(MathAbs(f - 1.0) < 1e-9) return base;
   int n = (int)MathMax(MathRound(base * f), 1.0);
   return (int)MathMin(MathMax(n, 1), MathMax(base * 3, 1));
  }

// pokrycie stref [0..1] — engine zone_overlap (część wspólna / mniejsza strefa)
double ZoneOverlap(double lo1, double hi1, double lo2, double hi2)
  {
   double lo = MathMax(MathMin(lo1, hi1), MathMin(lo2, hi2));
   double hi = MathMin(MathMax(lo1, hi1), MathMax(lo2, hi2));
   double wspolna = hi - lo;
   if(wspolna <= 0.0) return 0.0;
   double w1 = MathAbs(hi1 - lo1), w2 = MathAbs(hi2 - lo2);
   double mn = MathMin(w1, w2);
   if(mn <= 0.0) return 1.0;
   return wspolna / mn;
  }

//====================================================================
//  WEJŚCIE — engine.rs:2269 handle_entry (pełna kolejność bramek)
//====================================================================
void HandleEntry(long msg_id, int side, bool is_limit, bool is_stop, double lo, double hi,
                 double sl, bool has_sl, bool tp_open, double warstwy_offset,
                 bool has_warstwy_offset, double &tps[], int ntp, const NativeEntryPlanSource &source)
  {
   g_cnt_sig++;
   // exit_on_opposite_signal — engine.rs:1671: w dispatch, PRZED wszystkimi
   // bramkami; sygnal przeciwny odrzucony filtrem TEZ zamyka stare koszyki.
   if(In_ExitOnOpposite)
     {
      for(int bi2 = 0; bi2 < g_nb; bi2++)
        {
         if(!Alive(bi2) || g_b[bi2].side == side) continue;
         if(In_ConfirmedExitRetry) { RequestConfirmedExit(bi2, "OPPOSITE"); continue; }
         g_powod_zamk = "OPPOSITE";
         for(int i = g_b[bi2].npos - 1; i >= 0; i--) ZamknijPozycje(g_b[bi2].pos[i]);
         CancelPendings(bi2);
         g_b[bi2].state = ST_DONE;
        }
     }
   if(StringLen(g_halted) > 0) { g_cnt_reject++; return; }
   if(In_OnlyLimitSignals && !is_limit) { g_cnt_reject++; return; }
   // (filtr tagów: most nie niesie tagów — pole NIEODWZOROWANE, patrz D6)

   int gate = EntryGate();
   if(gate != 0)
     {
      g_cnt_reject++;
      if(gate == 1) g_rej_streak++;
      else if(gate == 2) g_rej_session++;
      else if(gate == 3) g_rej_maxpos++;
      else if(gate == 4) g_rej_maxbask++;
      else if(gate == 5) g_rej_floor++;
      else if(gate == 6) g_rej_margincall++;
      else if(gate == 9) g_rej_daystop++;
      else if(gate == 10) g_rej_daystop++;
      else if(gate == 12) g_rej_ml++;
      else if(gate == 13) g_rej_slhit++;
      return;
     }
   if(In_SideFilter == 1 && side != 0) { g_cnt_reject++; return; }
   if(In_SideFilter == 2 && side != 1) { g_cnt_reject++; return; }
   // ---- bramka rezimu z wyciszeniem (engine.rs:2459-2497) ----
   // Kolejnosc jest wiazaca: wyciszenie sprawdzamy PRZED galezia werdyktu
   // odmownego, dokladnie jak w silniku.
   g_rezim_miekki = false;
   bool rezim_przeszedl = RegimeOk(side, MidPx());
   if(In_RegimeGdyRozerwany == 2 && RegimeWyciszony())
     { g_rezim_miekki = true; g_wyciszen++; }
   if(!rezim_przeszedl)
     {
      if(!In_RegimeSoft) { g_cnt_reject++; g_rej_regime++; return; }
      g_rezim_miekki = true;                  // engine.rs:2495
     }
   // filtr trendu wyzszego rzedu — Block odrzuca, Shrink zmniejsza jednostki
   int units_num = 1, units_den = 1;
   int adv = TrendAdverse(side);
   if(adv == 1)
     {
      if(In_TrendFilterMode == 0) { g_cnt_reject++; g_rej_trend++; return; }
      // Shrink: mnożnik jednostek — TYLKO gdy shrink > 0 (engine.rs:2818:
      // przy 0 nie kurczy wcale, pelny rozmiar)
      if(In_TrendFilterShrink > 0.0)
        { units_num = (int)MathRound(In_TrendFilterShrink * 1000.0); units_den = 1000; }
     }

   // ---- strefa (engine.rs:2729 compute_zone; deep adaptacyjny :2733) ----
   double deep = AdaptiveDeepOffset(MathAbs(hi - lo));
   double zlo = lo, zhi = hi;
   if(In_ZoneOffsetMode == 1) { zhi += In_EntryHiOffset; zlo += In_EntryLoOffset; }
   else if(In_ZoneOffsetMode == 2)
     {
      if(side == 0) { zlo -= deep; zhi += In_EntryTolOffset; }
      else          { zhi += deep; zlo -= In_EntryTolOffset; }
     }
   double t1 = MathMin(zlo, zhi), t2 = MathMax(zlo, zhi);
   zlo = t1; zhi = t2;

   // ---- stop (engine.rs:2757 compute_sl; sl_min_dist adaptacyjny) ----
   double sig_w = MathAbs(hi - lo);
   double slv = sl; bool has = has_sl;
   if(has)
     {
      double min_d = AdaptiveSlMinDist(sig_w);
      if(min_d > 0.0)
        {
         double mid = (zlo + zhi) * 0.5;
         double want = (side == 0) ? (mid - min_d) : (mid + min_d);
         slv = (side == 0) ? MathMin(slv, want) : MathMax(slv, want);
        }
      if(In_SlMaxDist > 0.0)
        {
         double mid = (zlo + zhi) * 0.5;
         double capp = (side == 0) ? (mid - In_SlMaxDist) : (mid + In_SlMaxDist);
         slv = (side == 0) ? MathMax(slv, capp) : MathMin(slv, capp);
        }
     }

   // ---- jakość sygnału: R:R przy GORSZEJ krawędzi (engine.rs:2403) ----
   if(In_SignalMinRR > 0.0 && has && ntp > 0)
     {
      double krawedz = WorseEdge(side, zlo, zhi);
      double ryzyko = MathAbs(krawedz - slv);
      double nagroda = MathAbs(tps[0] - krawedz);
      double rr = (ryzyko > 1e-9) ? nagroda / ryzyko : 1e18;
      if(rr < In_SignalMinRR) { g_cnt_reject++; g_rej_jakosc++; return; }
     }
   if((In_SignalMinZoneW > 0.0 && sig_w < In_SignalMinZoneW)
      || (In_SignalMaxZoneW > 0.0 && sig_w > In_SignalMaxZoneW))
     { g_cnt_reject++; g_rej_jakosc++; return; }

   // ---- SL już przebity? (engine.rs:2450 — SL SYGNAŁU, nie po min_dist) ----
   if(In_SkipIfSlBreached && has_sl)
     {
      bool breached = (side == 0) ? (g_ask <= sl) : (g_bid >= sl);
      if(breached) { g_cnt_reject++; g_rej_slbreach++; return; }
     }
   // ---- nie gonimy setupu, który uciekł (TYLKO wejście po rynku) ----
   bool wejdzie_po_rynku = (!is_limit && !In_AutoLimit);
   if(In_MaxChaseBeyond > 0.0 && wejdzie_po_rynku)
     {
      double worst = WorseEdge(side, zlo, zhi);
      double beyond = (side == 0) ? (g_ask - worst) : (worst - g_bid);
      if(beyond > In_MaxChaseBeyond) { g_cnt_reject++; return; }
     }
   if(In_EntrySlDistLimit > 0.0 && has)
     {
      double worst = WorseEdge(side, zlo, zhi);
      if(MathAbs(worst - slv) > In_EntrySlDistLimit) { g_cnt_reject++; return; }
     }

   // ---- merge_same_side (engine.rs:2531) ----
   if(In_MergeSameSide)
     {
      long okno = (long)(MathMax(In_MergeWindowMin, 0.0) * 60000.0);
      for(int bi2 = g_nb - 1; bi2 >= 0; bi2--)
        {
         if(!Alive(bi2) || g_b[bi2].side != side) continue;
         if(g_now < g_b[bi2].created_ts || g_now - g_b[bi2].created_ts > okno) continue;
         if(ZoneOverlap(g_b[bi2].zone_lo, g_b[bi2].zone_hi, zlo, zhi) < In_MergeMinOverlap) continue;
         ApplySourceEntryEdit(bi2, source, tps, ntp);
         MapPut(msg_id, g_b[bi2].id);
         NativeSourceAccept(msg_id,g_b[bi2].id);
         g_merges++;
         return;   // sygnał scalony — budżetu dnia nie zużywa
        }
     }

   // ---- daily_signal_budget (engine.rs:2593) ----
   if(In_DailySignalBudget > 0)
     {
      long dzien = DayOf(g_now);
      if(dzien != g_budget_day) { g_budget_day = dzien; g_opened_today = 0; }
      if(g_opened_today >= In_DailySignalBudget) { g_cnt_reject++; g_rej_budget++; return; }
     }

   // ---- nowy koszyk ----
   if(!NativeEnsureBasketCapacity("Entry"))return;
   int bi = g_nb; g_nb++;
   ZeroMemory(g_b[bi]);
   g_b[bi].id = g_next_id; g_next_id++;
   g_b[bi].msg_id = msg_id;
   g_b[bi].side = side;
   g_b[bi].is_limit = is_limit;
   g_b[bi].is_stop = is_stop;
   g_b[bi].source_explicit = In_ExplicitPendingUntilCancel && (is_limit || is_stop);
   g_b[bi].source_withdrawn = false;
   g_b[bi].entry_source = source;
   g_b[bi].entry_lo = lo; g_b[bi].entry_hi = hi;
   g_b[bi].zone_lo = zlo; g_b[bi].zone_hi = zhi;
   g_b[bi].sl = slv; g_b[bi].has_sl = has;
   g_b[bi].ntp = MathMin(ntp, MAXTP);
   for(int i = 0; i < g_b[bi].ntp; i++) g_b[bi].tps[i] = tps[i];
   g_b[bi].tp_open = tp_open;
   g_b[bi].warstwy_offset = warstwy_offset;
   g_b[bi].has_warstwy_offset = has_warstwy_offset;
   g_b[bi].created_ts = g_now;
   g_b[bi].state = ST_PENDING;
   g_b[bi].tp_stage = 0;
   g_b[bi].plan_observed_stage = 0;
   g_b[bi].had_positions = false;
   g_b[bi].rearm_blocked_by_spp = false;
   g_b[bi].zone_touched = false;
   g_b[bi].drop_armed = false;
   g_b[bi].secured = false;
   g_b[bi].secured_ts = 0;
   g_b[bi].secured_by_rule = false;
   g_b[bi].last_tp_ts = 0;
   g_b[bi].reentries = 0;
   g_b[bi].has_last_entry = false;
   g_b[bi].age_limit_min = 0.0;
   g_b[bi].tempo_checked = false;
   g_b[bi].tempo_fast = false;
   g_b[bi].pyramided = false;
   g_b[bi].fast_addons = 0;
   g_b[bi].last_addon_ts = 0;
   g_b[bi].adverse_since = 0;
   g_b[bi].rearms = 0;
   g_b[bi].last_rearm_ts = 0;
   g_b[bi].drop_po_ts = 0;
   g_b[bi].realized = 0.0;
   g_b[bi].be_ts = 0;
   for(int i = 0; i < MAXTP; i++) { g_b[bi].tp_touch_ts[i] = 0; g_b[bi].tphit_sig_ts[i] = 0; }
   g_b[bi].npend = 0; g_b[bi].npos = 0; g_b[bi].nlv = 0;
   MapPut(msg_id, g_b[bi].id);
   NativeSourceAccept(msg_id,g_b[bi].id);
   g_cnt_basket++;
   g_opened_today++;

   PlanGrid(bi, units_num, units_den);
   if(g_b[bi].nlv == 0)
     {
      g_b[bi].state = ST_DONE;
      g_rej_risk++;
      DiagKoszyk(bi, "ODRZUC_RYZYKO");
      return;
     }
   PlaceGrid(bi);
   DiagKoszyk(bi, "KOSZYK");
  }

//====================================================================
//  MKT (BUY NOW / SELL NOW) — engine.rs:2115 handle_market_open
//  Cele/SL z ostatniego żywego koszyka tego kierunku; strefa punktowa.
//====================================================================
void HandleMkt(long msg_id, int side)
  {
   if(!In_HonorMarketOpen) return;
   // engine.rs:2115-2134: pelna bramka wejscia obowiazuje takze dla NOW
   if(StringLen(g_halted) > 0) return;
   if(EntryGate() != 0) { g_cnt_reject++; return; }
   double tps[MAXTP]; int ntp = 0;
   double sl = 0.0; bool has_sl = false;
   for(int i = g_nb - 1; i >= 0; i--)
     {
      if(!Alive(i) || g_b[i].side != side) continue;
      ntp = g_b[i].ntp;
      for(int k = 0; k < ntp; k++) tps[k] = g_b[i].tps[k];
      sl = g_b[i].sl; has_sl = g_b[i].has_sl;
      break;
     }
   double px = EntryPx(side);
   // strefa punktowa: lo == hi == cena bieżąca; wejście rynkiem
   bool stary_auto = In_AutoLimit;   // MKT wchodzi po rynku niezależnie od auto_limit
   // (silnik: koszyk z punktową strefą + is_limit=false; PlaceGrid z auto_limit
   //  złożyłby limit na cenie — dlatego tu krótka ścieżka rynkowa)
   if(!NativeEnsureBasketCapacity("MarketOpen"))return;
   int bi = g_nb; g_nb++;
   ZeroMemory(g_b[bi]);
   g_b[bi].id = g_next_id; g_next_id++;
   g_b[bi].msg_id = msg_id;
   g_b[bi].side = side;
   g_b[bi].is_limit = false;
   g_b[bi].is_stop = false;
   g_b[bi].source_explicit = false;
   g_b[bi].source_withdrawn = false;
   g_b[bi].entry_lo = px; g_b[bi].entry_hi = px;
   g_b[bi].zone_lo = px; g_b[bi].zone_hi = px;
   g_b[bi].sl = sl; g_b[bi].has_sl = has_sl;
   g_b[bi].ntp = ntp;
   for(int k = 0; k < ntp; k++) g_b[bi].tps[k] = tps[k];
   g_b[bi].tp_open = false;
   g_b[bi].warstwy_offset = 0.0;
   g_b[bi].has_warstwy_offset = false;
   g_b[bi].created_ts = g_now;
   g_b[bi].state = ST_PENDING;
   g_b[bi].tp_stage = 0;
   g_b[bi].plan_observed_stage = 0;
   g_b[bi].had_positions = false;
   g_b[bi].rearm_blocked_by_spp = false;
   g_b[bi].zone_touched = true;
   g_b[bi].drop_armed = false;
   g_b[bi].secured = false; g_b[bi].secured_ts = 0; g_b[bi].secured_by_rule = false;
   g_b[bi].last_tp_ts = 0; g_b[bi].reentries = 0; g_b[bi].has_last_entry = false;
   g_b[bi].age_limit_min = 0.0; g_b[bi].tempo_checked = false; g_b[bi].tempo_fast = false;
   g_b[bi].pyramided = false; g_b[bi].fast_addons = 0; g_b[bi].last_addon_ts = 0;
   g_b[bi].adverse_since = 0; g_b[bi].rearms = 0; g_b[bi].last_rearm_ts = 0;
   g_b[bi].drop_po_ts = 0; g_b[bi].realized = 0.0;
   g_b[bi].be_ts = 0;
   for(int i = 0; i < MAXTP; i++) { g_b[bi].tp_touch_ts[i] = 0; g_b[bi].tphit_sig_ts[i] = 0; }
   g_b[bi].npend = 0; g_b[bi].npos = 0; g_b[bi].nlv = 0;
   MapPut(msg_id, g_b[bi].id);
   NativeSourceAccept(msg_id,g_b[bi].id);
   g_cnt_basket++;
   double lot = LotSize();
   // engine.rs:2157: pozycja NOW dostaje zawsze OSTATNI cel drabinki
   double tp = 0.0; bool htp = false;
   if(ntp > 0) { tp = tps[ntp - 1]; htp = true; }
   g_b[bi].nlv = 1;
   g_b[bi].lv_price[0] = px; g_b[bi].lv_vol[0] = lot; g_b[bi].lv_units[0] = 1;
   g_b[bi].lot_planu = lot;
   g_b[bi].lv_has_tp[0] = htp; g_b[bi].lv_tp[0] = htp ? tp : 0.0;
   g_b[bi].lv_fill_ts[0] = 0; g_b[bi].lv_filled[0] = false; g_b[bi].lv_cancelled[0] = false;
   ulong tk = 0;
   if(WyslijRynek(bi, 0, lot, sl, has_sl, tp, htp, "B" + IntegerToString(g_b[bi].id), tk))
     {
      if(g_b[bi].npos < MAXTK)
        { g_b[bi].pos[g_b[bi].npos] = tk; g_b[bi].pos_lv[g_b[bi].npos] = 0; g_b[bi].npos++; }
      ZapiszWlasciciela(tk, bi);
      g_b[bi].had_positions = true;
      g_b[bi].state = ST_WORKING;
      g_b[bi].lv_filled[0] = true; g_b[bi].lv_fill_ts[0] = g_now;
      g_cnt_order++;
     }
   else g_b[bi].state = ST_DONE;
  }

//====================================================================
//  KOSZYK ADRESAT KOMUNIKATU — engine.rs:2057 target_basket (+hint_veto Z-3)
//====================================================================
int TargetZAliasem(int mi, int bi)
  {
   if(bi >= 0 && !ExitRiskAllowed(bi)) return -1;
   if(bi >= 0 && In_ReplyGraph) MapPut(g_msg[mi].msg_id, g_b[bi].id);
   return bi;
  }

int TargetBasket(int mi, bool register_alias = true)
  {
   if(g_msg[mi].reply_to != 0)
     {
      int id = MapGet(g_msg[mi].reply_to);
      if(id >= 0)
        {
         int bi = BIdx(id);
         // Jawny adres jest rozstrzygający także wtedy, gdy koszyk jest Done.
         // Jeśli mapa wskazuje wpis już usunięty, nie wolno spaść na cudzy.
         if(bi >= 0) return register_alias ? TargetZAliasem(mi, bi) : bi;
         return -1;
        }
      // F1: jawny reply do sygnału, którego bot nie ma, nie może wykonać
      // polecenia na najnowszym koszyku.
      if(In_ReplyVeto) return -1;
     }
   int zywe[MAXB]; int nz = 0;
   for(int i = 0; i < g_nb; i++) if(Alive(i)) { zywe[nz] = i; nz++; }
   if(nz == 0) return -1;

   bool mial_hinty = false;
   if(In_BasketHintTol > 0.0 && StringLen(g_msg[mi].hints) > 0)
     {
      mial_hinty = true;
      string hs[];
      int k = StringSplit(g_msg[mi].hints, ',', hs);
      double tol = In_BasketHintTol;
      for(int j = 0; j < k; j++)
        {
         double h = StringToDouble(hs[j]);
         for(int q = nz - 1; q >= 0; q--)
           {
            int bi = zywe[q];
            bool in_zone = (h >= g_b[bi].zone_lo - tol && h <= g_b[bi].zone_hi + tol);
            bool is_tgt = false;
            for(int t = 0; t < g_b[bi].ntp; t++)
               if(MathAbs(g_b[bi].tps[t] - h) <= tol) { is_tgt = true; break; }
            bool is_sl = g_b[bi].has_sl && MathAbs(g_b[bi].sl - h) <= tol;
            if(in_zone || is_tgt || is_sl) return register_alias ? TargetZAliasem(mi, bi) : bi;
           }
        }
     }
   // Z-3 hint_veto: wskazówki były, żadna nie pasuje → komunikat bez adresata
   if(In_HintVeto && mial_hinty) return -1;
   for(int q = nz - 1; q >= 0; q--)
      if(g_b[zywe[q]].npos > 0) return register_alias ? TargetZAliasem(mi, zywe[q]) : zywe[q];
   return register_alias ? TargetZAliasem(mi, zywe[nz - 1]) : zywe[nz - 1];
  }

//====================================================================
//  BANKOWANIE NA CELU — engine.rs:4276 bank_on_tp (pełne: TYLER)
//====================================================================
int BankCount(int n, double pct)
  {
   if(n == 0 || pct <= 0.0) return 0;
   double raw = n * pct / 100.0;
   double c;
   if(In_BankRounding == 0)      c = MathRound(raw);
   else if(In_BankRounding == 1) c = MathCeil(raw - 1e-9);
   else                          c = MathFloor(raw + 1e-9);
   return (int)MathMin(MathMax(c, 0), n);
  }

void BankOnTp(int bi, int stage)
  {
   if(!ExitRiskAllowed(bi)) return;
   ulong live[MAXTK]; int n = 0;
   for(int i = 0; i < g_b[bi].npos; i++)
      if(PositionSelectByTicket(g_b[bi].pos[i])) { live[n] = g_b[bi].pos[i]; n++; }
   if(n == 0) return;

   // zapis wolumenu pierwotnego (TYLER partial_pct_od_pierwotnego)
   if(In_PartialOdPierw)
      for(int i = 0; i < n; i++)
        {
         int ip = PsEnsure(live[i]);
         if(ip >= 0 && g_ps_wol0[ip] <= 0.0 && PositionSelectByTicket(live[i]))
            g_ps_wol0[ip] = PositionGetDouble(POSITION_VOLUME);
        }

   double pct = 0.0;
   if((In_TpSchedule == 0 || In_TpSchedule == 1 || In_TpSchedule == 2) && In_AssignTpPerPos)
      pct = 0.0;                              // broker sam zamyka po TP pozycji
   else if(In_TpSchedule == 3)                // OfficialCounts
     {
      int c[16], nc; ParseCounts(c, nc);
      int cc = 0;
      if(stage - 1 < nc) cc = c[stage - 1];
      else if(In_OfficialSpp && nc > 0) cc = c[nc - 1];
      pct = (double)cc / (double)n * 100.0;
     }
   else if(In_TpSchedule == 4)                // OfficialPct
     {
      double c[16]; int nc; ParseOfficialPct(c, nc);
      if(stage - 1 < nc) pct = c[stage - 1];
      else if(In_OfficialSpp && nc > 0) pct = c[nc - 1];
      else pct = 0.0;
     }
   else if(In_TpSchedule == 5) pct = In_ScaleOutPct;
   if(pct <= 0.0) return;

   // Stable bank order, including equal gross-profit legs.
   NativeSortProfit(live,n,In_BankFrom!=0);

   // partiale z wolumenu?
   bool part_ok = In_PartialClose;
   if(part_ok)
      for(int i = 0; i < n; i++)
        {
         if(!PositionSelectByTicket(live[i])) { part_ok = false; break; }
         if(PositionGetDouble(POSITION_VOLUME) < In_PartialMinLot - 1e-9) { part_ok = false; break; }
        }
   if(part_ok)
     {
      g_powod_zamk = "BANK_PARTIAL";
      for(int i = 0; i < n; i++)
        {
         if(!PositionSelectByTicket(live[i])) continue;
         double vol = PositionGetDouble(POSITION_VOLUME);
         double baza = vol;
         if(In_PartialOdPierw)
           {
            int ip = PsIdx(live[i]);
            if(ip >= 0 && g_ps_wol0[ip] > 0.0) baza = g_ps_wol0[ip];
           }
         double cut = PartialCloseVolume(vol, baza * pct / 100.0);
         if(cut <= 0.0) continue;
         ZamknijCzesc(live[i], cut);
        }
      return;
     }

   int close_n = BankCount(n, pct);
   if(In_Diag)
      FileWrite(g_handle_diag, "BANK", (string)g_now, (string)g_b[bi].id,
                (string)stage, StringFormat("n=%d pct=%.1f close_n=%d", n, pct, close_n));
   if(close_n == 0) return;
   int limit = In_BankCloseLast ? MathMin(close_n, n) : MathMin(close_n, n - 1);
   if(limit <= 0) return;
   g_powod_zamk = "BANK_TP";
   for(int i = 0; i < limit; i++) ZamknijPozycje(live[i]);
  }

//====================================================================
//  PRZESUNIĘCIE CELÓW — engine.rs:4422 retarget
//====================================================================
// Pure decision helpers used by the actual trade paths and offline regressions.
// Equal stops remain allowed, matching Rust's strict side.better comparison.
bool BeReplacementAllowed(int side, double current, double proposed, bool protect)
  {
   return !protect || current == 0.0 || !SideBetter(side, proposed, current);
  }

double RetargetPrice(int side, bool keep_final, double final_target, double current,
                     bool has_next, double next, double last, int schedule,
                     bool freeze_after_ladder, double open_offset)
  {
   if(keep_final) return final_target;
   double candidate = (schedule == 0) ? last : (has_next ? next : last);
   if(!has_next && !freeze_after_ladder)
      candidate = current + SideSign(side) * open_offset;
   return candidate;
  }

bool TestBeRetargetContract()
  {
   bool ok = !BeReplacementAllowed(0, 4006.0, 4000.0, true)
             && !BeReplacementAllowed(1, 3994.0, 4000.0, true)
             && BeReplacementAllowed(0, 3995.0, 4000.0, true)
             && BeReplacementAllowed(1, 4005.0, 4000.0, true)
             && BeReplacementAllowed(0, 4006.0, 4000.0, false)
             && BeReplacementAllowed(0, 0.0, 4000.0, true)
             && BeReplacementAllowed(0, 4000.0, 4000.0, true);
   ok = ok && RetargetPrice(0, true, 4042.0, 4030.0, true, 4020.0, 4030.0, 4, false, 12.0) == 4042.0
           && RetargetPrice(0, true, 4042.0, 4042.0, false, 0.0, 4030.0, 4, false, 12.0) == 4042.0
           && RetargetPrice(0, false, 0.0, 4030.0, true, 4020.0, 4030.0, 4, false, 12.0) == 4020.0
           && RetargetPrice(0, false, 0.0, 4042.0, false, 0.0, 4030.0, 4, false, 12.0) == 4054.0
           && RetargetPrice(1, true, 3958.0, 3970.0, false, 0.0, 3970.0, 4, false, 12.0) == 3958.0;
   Print("BE_RETARGET_CONTRACT_SELFTEST ", ok ? "PASS" : "FAIL");
   return ok;
  }

void Retarget(int bi)
  {
   if(!ExitRiskAllowed(bi)) return;
   int stage = g_b[bi].tp_stage;
   bool has_next = (stage < g_b[bi].ntp);
   double next = has_next ? g_b[bi].tps[stage] : 0.0;
   bool has_last = (g_b[bi].ntp > 0);
   double last = has_last ? g_b[bi].tps[g_b[bi].ntp - 1] : 0.0;

   for(int i = 0; i < g_b[bi].npos; i++)
     {
      ulong t = g_b[bi].pos[i];
      if(!PositionSelectByTicket(t)) continue;
      double cur_tp = PositionGetDouble(POSITION_TP);
      double cur_sl = PositionGetDouble(POSITION_SL);
      if(cur_tp == 0.0) continue;                 // runner bez celu — nie ruszamy
      bool keep_final = In_RetargetRespectsFinal && In_CeleNaOstatnim;
      double final_target = 0.0;
      if(keep_final)
        {
         // Same initial final-target rule, including an explicit TP OPEN offset.
         // Existing TP=0 runners were skipped above; never restore their TP.
         if(!TargetForEx(bi, 0, 1, final_target)) continue;
        }
      else if(In_TpSchedule == 0) { if(!has_last) continue; }
      else if(!has_next && !has_last) continue;
      double newtp = RetargetPrice(g_b[bi].side, keep_final, final_target, cur_tp,
                                   has_next, next, last, In_TpSchedule,
                                   In_TpFreezeAfterLad, In_TpOpenOffset);
      if(TpIsValid(g_b[bi].side, newtp))
        {
         ModyfikujPozycje(t, cur_sl, cur_sl != 0.0, newtp, true);
         if(In_Diag)
            FileWrite(g_handle_diag, "RETARGET", (string)g_now, (string)g_b[bi].id,
                      (string)t, StringFormat("%.2f->%.2f", cur_tp, newtp),
                      (string)g_b[bi].tp_stage);
        }
     }
  }

void MoveBasketToBe(int bi)
  {
   if(!ExitRiskAllowed(bi)) return;
   // Rust zapisuje chwilę bezwarunkowo, także gdy nie ma jeszcze pozycji.
   // Dzięki temu opcjonalna oś może osłonić pending wypełniony po komendzie.
   g_b[bi].be_ts = g_now;
   for(int i = 0; i < g_b[bi].npos; i++)
     {
      ulong t = g_b[bi].pos[i];
      if(!PositionSelectByTicket(t)) continue;
      double op = PositionGetDouble(POSITION_PRICE_OPEN);
      double tp = PositionGetDouble(POSITION_TP);
      double be = op + SideSign(g_b[bi].side) * In_BeOffset;
      double cur_sl = PositionGetDouble(POSITION_SL);
      if(BeReplacementAllowed(g_b[bi].side, cur_sl, be, In_TrailSrEnabled || In_BeNeverLoosen)
         && SlIsValid(g_b[bi].side, be))
         ModyfikujPozycje(t, be, true, tp, tp != 0.0);
     }
  }

// engine.rs:4756 set_basket_sl (+Z-5 sl_edit_reaches_pendings)
void SetBasketSl(int bi, double sl)
  {
   if(!ExitRiskAllowed(bi)) return;
   g_b[bi].sl = sl; g_b[bi].has_sl = true;
   for(int i = 0; i < g_b[bi].npos; i++)
     {
      ulong t = g_b[bi].pos[i];
      if(!PositionSelectByTicket(t)) continue;
      double tp = PositionGetDouble(POSITION_TP);
      if(SlIsValid(g_b[bi].side, sl)) ModyfikujPozycje(t, sl, true, tp, tp != 0.0);
     }
   if(In_SlEditToPendings)
     {
      double bsl; bool hbsl = BrokerSl(g_b[bi].side, sl, true, bsl);
      for(int i = 0; i < g_b[bi].npend; i++)
        {
         ulong t = g_b[bi].pend[i];
         if(!OrderSelect(t)) continue;
         MqlTradeRequest r; MqlTradeResult res;
         ZeroMemory(r); ZeroMemory(res);
         r.action = TRADE_ACTION_MODIFY;
         r.order  = t;
         r.price  = OrderGetDouble(ORDER_PRICE_OPEN);
         r.sl     = hbsl ? NormPx(bsl) : 0.0;
         r.tp     = OrderGetDouble(ORDER_TP);
         r.type_time = ORDER_TIME_GTC;
         if(!OrderSend(r, res) || res.retcode != TRADE_RETCODE_DONE)
            ZliczOdrzucenie((int)res.retcode);
        }
     }
  }

// engine.rs:4713 sl_polowa_drogi (TYLER) — per pozycja, tylko zapadka
void SlPolowaDrogi(int bi)
  {
   if(!ExitRiskAllowed(bi)) return;
   double ulamek = (In_SlPolowaUlamek > 0.0) ? In_SlPolowaUlamek : 0.5;
   int side = g_b[bi].side;
   for(int i = 0; i < g_b[bi].npos; i++)
     {
      ulong t = g_b[bi].pos[i];
      if(!PositionSelectByTicket(t)) continue;
      double op = PositionGetDouble(POSITION_PRICE_OPEN);
      double tp = PositionGetDouble(POSITION_TP);
      double cur = PositionGetDouble(POSITION_SL);
      double cena = ExitPx(side);
      double ruch = (cena - op) * SideSign(side);
      if(ruch <= 0.0) continue;
      double want = op + SideSign(side) * ruch * ulamek;
      bool lepszy = (cur == 0.0) || ((want - cur) * SideSign(side) > 0.0);
      if(!lepszy) continue;
      if(SlIsValid(side, want)) ModyfikujPozycje(t, want, true, tp, tp != 0.0);
     }
  }

// engine.rs:4176 apply_smart_sl — SL wg RANGI pozycji (0 = najlepsze wejście)
void ApplySmartSl(int bi)
  {
   if(!ExitRiskAllowed(bi)) return;
   if(In_SmartSlMode == 0) return;
   if(In_SmartSlOnlyAfterRf && !g_b[bi].secured) return;
   int side = g_b[bi].side;
   int stage = g_b[bi].tp_stage;
   // żywe pozycje posortowane od najlepszego wejścia
   ulong tk[MAXTK]; double op[MAXTK]; int n = 0;
   for(int i = 0; i < g_b[bi].npos; i++)
     {
      ulong t = g_b[bi].pos[i];
      if(!PositionSelectByTicket(t)) continue;
      tk[n] = t; op[n] = PositionGetDouble(POSITION_PRICE_OPEN); n++;
     }
   if(n == 0) return;
   NativeStableSortTickets(tk,op,n,side!=0);
   bool use_be = (In_SmartSlMode == 1 || In_SmartSlMode == 3);
   bool use_ladder = (In_SmartSlMode == 2 || In_SmartSlMode == 3);
   int moved = 0;
   for(int rank = 0; rank < n; rank++)
     {
      ulong t = tk[rank];
      if(!PositionSelectByTicket(t)) continue;
      double cur_sl = PositionGetDouble(POSITION_SL);
      double tp = PositionGetDouble(POSITION_TP);
      double be_px = op[rank] + SideSign(side) * In_BeOffset;
      int step = stage - rank - In_SmartSlDelay;

      double want = 0.0; bool has_want = false;
      if(step <= 0)
        { if(g_b[bi].has_sl) { want = g_b[bi].sl; has_want = true; } }
      else if(use_be && step == 1)
        { want = be_px; has_want = true; }
      else if(use_ladder)
        {
         int idx = step - 1 - (use_be ? 1 : 0);
         if(idx < 0)
           { if(g_b[bi].has_sl) { want = g_b[bi].sl; has_want = true; } }
         else
           {
            if(idx < g_b[bi].ntp) { want = g_b[bi].tps[idx]; has_want = true; }
            else if(g_b[bi].ntp > 0) { want = g_b[bi].tps[g_b[bi].ntp - 1]; has_want = true; }
           }
        }
      else
        { want = be_px; has_want = true; }

      // podłoga BE po zabezpieczeniu (smart_sl_floor_be_after_rf)
      double floor_v = 0.0; bool has_floor = false;
      if(g_b[bi].secured && In_SmartSlFloorBeRf)
        {
         if(!g_b[bi].has_sl) { floor_v = be_px; has_floor = true; }
         else
           {
            floor_v = (side == 0) ? MathMax(g_b[bi].sl, be_px) : MathMin(g_b[bi].sl, be_px);
            has_floor = true;
           }
        }
      else if(g_b[bi].has_sl) { floor_v = g_b[bi].sl; has_floor = true; }

      double v = 0.0; bool has_v = false;
      if(has_want && has_floor)
        { v = (side == 0) ? MathMax(want, floor_v) : MathMin(want, floor_v); has_v = true; }
      else if(has_want) { v = want; has_v = true; }
      else if(has_floor) { v = floor_v; has_v = true; }
      if(!has_v) continue;

      // tylko zaciskanie: nowy SL musi być taki, że stary jest „lepszą ceną"
      bool tighter = (cur_sl == 0.0) || SideBetter(side, cur_sl, v);
      if(!tighter) continue;
      if(SlIsValid(side, v) && ModyfikujPozycje(t, v, true, tp, tp != 0.0))
        { moved++; g_cnt_smart_sl++; }
     }
  }

//====================================================================
//  TRAFIONY CEL — engine.rs:3907 handle_tp_hit (pełny)
//====================================================================
bool ApplySppTargetPlan(int bi, const double &targets[], int count)
  {
   if(!ExitRiskAllowed(bi)) return false;
   if(count <= 0) return false;
   if(In_ResetTpOnTargetEdit)
      for(int i = 1; i < count; i++)
         if((g_b[bi].side == 0 && targets[i] < targets[i-1])
            || (g_b[bi].side == 1 && targets[i] > targets[i-1])) return false;
   bool changed = (count != g_b[bi].ntp);
   for(int i = 0; i < count && !changed; i++)
      if(targets[i] != g_b[bi].tps[i]) changed = true;
   for(int i = 0; i < count; i++) g_b[bi].tps[i] = targets[i];
   g_b[bi].ntp = count;
   if(In_ResetTpOnTargetEdit && changed)
     {
      g_b[bi].tp_stage = 0;
      g_b[bi].plan_observed_stage = 0;
      g_b[bi].zone_touched = false;
      g_b[bi].drop_armed = false;
      g_b[bi].drop_po_ts = 0;
      g_b[bi].last_tp_ts = 0;
      for(int i = 0; i < MAXTP; i++) g_b[bi].tp_touch_ts[i] = 0;
     }
   return changed;
  }

int NativeReadSppTargets(const string &fields[],int count,double &targets[])
  {
   int written=0;
   for(int i=3;i<count && written<MAXTP;i++)
     {
      // Empty targets serialize as a trailing comma in the bridge. They are
      // an empty Vec, not the numeric target zero returned by StringToDouble("").
      if(StringLen(fields[i])==0)continue;
      targets[written]=StringToDouble(fields[i]);written++;
     }
   return written;
  }

bool NativeCorrectionPendingTarget(int bi,double &target)
  {
   if(g_b[bi].ntp<=0)return false;
   int index=g_b[bi].tp_stage;
   if(index<0 || index>=g_b[bi].ntp)index=g_b[bi].ntp-1;
   target=g_b[bi].tps[index];return true;
  }

// Pure state regression executed before any trading state is loaded.
bool TestSppTargetPlanReset()
  {
   if(!In_ResetTpOnTargetEdit) return true;
   int bi = MAXB - 1;
   Basket saved = g_b[bi];
   g_b[bi].side = 0;
   g_b[bi].ntp = 2;
   g_b[bi].tps[0] = 10.0; g_b[bi].tps[1] = 20.0;
   g_b[bi].tp_stage = 2; g_b[bi].tp_touch_ts[0] = 123;
   g_b[bi].zone_touched = true; g_b[bi].drop_armed = true;
   g_b[bi].drop_po_ts = 456; g_b[bi].last_tp_ts = 789;
   double targets[2]; targets[0] = 30.0; targets[1] = 40.0;
   bool changed = ApplySppTargetPlan(bi, targets, 2);
   bool ok = changed && g_b[bi].tp_stage == 0 && g_b[bi].tp_touch_ts[0] == 0
             && !g_b[bi].zone_touched && !g_b[bi].drop_armed
             && g_b[bi].drop_po_ts == 0 && g_b[bi].last_tp_ts == 0;
   g_b[bi].tp_stage = 1; g_b[bi].tp_touch_ts[0] = 999;
   ok = !ApplySppTargetPlan(bi, targets, 2) && ok
        && g_b[bi].tp_stage == 1 && g_b[bi].tp_touch_ts[0] == 999;
   g_b[bi] = saved;
   Print("SPP_PLAN_RESET_SELFTEST ", ok ? "PASS" : "FAIL");
   return ok;
  }

void HandleTpHit(int bi, int index)
  {
   if(!ExitRiskAllowed(bi) || (In_ConfirmedExitRetry && !Alive(bi))) return;
   if(g_b[bi].npos == 0)
     {
      if(KeepExplicitPending(bi)) return;
      int observed = MathMax(g_b[bi].tp_stage, g_b[bi].plan_observed_stage);
      int stage = MathMax((index > 0) ? index : observed + 1, 1);
      if(stage <= observed) return;
      if(In_PendingDropOnTgt) DropGridOnTarget(bi, stage);
      else g_b[bi].plan_observed_stage = MathMax(g_b[bi].plan_observed_stage, stage);
      return;
     }
   g_last_tp_hit_ts = g_now;
   int stage_now = g_b[bi].tp_stage;
   int target_stage = MathMax((index > 0) ? index : (stage_now + 1), 1);
   if(target_stage <= stage_now) return;
   if(In_Diag && g_handle_diag != INVALID_HANDLE)
      FileWrite(g_handle_diag, "TPHIT", (string)g_now, (string)g_b[bi].id,
                (string)index, (string)stage_now, (string)target_stage);
   g_b[bi].last_tp_ts = g_now;

   if(In_TpHitFillStages)
      for(int st = stage_now + 1; st <= target_stage; st++)
        { g_b[bi].tp_stage = st; g_b[bi].plan_observed_stage = MathMax(g_b[bi].plan_observed_stage, st); BankOnTp(bi, st); }
   else
     { g_b[bi].tp_stage = target_stage; g_b[bi].plan_observed_stage = MathMax(g_b[bi].plan_observed_stage, target_stage); BankOnTp(bi, target_stage); }

   if(In_BankAllAtStage > 0 && target_stage >= In_BankAllAtStage)
     {
      if(In_ConfirmedExitRetry) { RequestConfirmedExit(bi, "BANK_ALL"); return; }
      g_powod_zamk = "BANK_ALL";
      for(int i = g_b[bi].npos - 1; i >= 0; i--)
         if(PositionSelectByTicket(g_b[bi].pos[i]))
            ZamknijPozycje(g_b[bi].pos[i]);
      CancelPendings(bi);
      g_b[bi].state = ST_DONE;
      return;
     }

   // PIRAMIDA — engine.rs:3947 (PRZED kasowaniem siatki!)
   if(In_PyramidAfterStage > 0
      && MarginesPozwala(In_MlMinPiramida)
      && target_stage >= In_PyramidAfterStage)
     {
      bool regime_ok = true;
      if(In_PyramidRegimeLb > 0)
        {
         int n = MathMin(In_PyramidRegimeLb, g_nregime);
         if(n >= MathMin(In_PyramidRegimeLb, 8))
           {
            int przeloty = 0;
            for(int i = g_nregime - n; i < g_nregime; i++)
               if(g_regime_hist[i]) przeloty++;
            if(100.0 * przeloty / n > In_PyramidRegMaxFast) regime_ok = false;
           }
        }
      bool kapital_ok = In_PyramidMinEqMult <= 0.0
         || AccountInfoDouble(ACCOUNT_BALANCE) >= g_start_balance * In_PyramidMinEqMult;
      if(!g_b[bi].pyramided && !g_b[bi].tempo_fast && regime_ok && kapital_ok
         && g_b[bi].ntp > 0)
        {
         double px = g_b[bi].tps[0];
         double vol = WolumenZlecenia(MathMax(LotSize() * MathMax(In_PyramidLotMult, 0.0), In_LotMin));
         int typ = (g_b[bi].side == 0) ? ORDER_TYPE_BUY_LIMIT : ORDER_TYPE_SELL_LIMIT;
         ulong tk = 0;
         double ostatni = g_b[bi].tps[g_b[bi].ntp - 1];
         if(LimitPxIsValid(g_b[bi].side, px)
            && WyslijLimit(bi, px, vol, g_b[bi].sl, g_b[bi].has_sl, ostatni, true,
                           "B" + IntegerToString(g_b[bi].id), typ, tk))
           {
            if(g_b[bi].npend < MAXTK)
              { g_b[bi].pend[g_b[bi].npend] = tk; g_b[bi].pend_lv[g_b[bi].npend] = -3;
                g_b[bi].pend_top[g_b[bi].npend] = false; g_b[bi].npend++; }
            g_cnt_piramida++;
           }
         g_b[bi].pyramided = true;   // także po odmowie — nie dobijamy się co cel
        }
     }

   // kasowanie limitów wg konfiguracji
   int cs = -1;
   if(In_PendingLifetime == 1) cs = 1;
   else if(In_PendingLifetime == 2) cs = 2;
   else if(In_PendingLifetime == 3) cs = 3;
   if(cs > 0 && target_stage >= cs && !KeepExplicitPending(bi)) CancelPendings(bi);

   if(In_NoTpAfterStage > 0 && target_stage >= In_NoTpAfterStage)
     {
      for(int q = 0; q < g_b[bi].npos; q++)
        {
         ulong tq = g_b[bi].pos[q];
         if(!PositionSelectByTicket(tq)) continue;
         double sq = PositionGetDouble(POSITION_SL);
         double tp_q = PositionGetDouble(POSITION_TP);
         bool bez_celu = true;
         if(tp_q != 0.0)
            bez_celu = ModyfikujPozycje(tq, sq, sq != 0.0, 0.0, false);
         if(bez_celu)
           {
            int iq = PsEnsure(tq);
            if(iq >= 0) g_ps_isrunner[iq] = true;
           }
        }
     }

   //  STOP NA WEJSCIE PO ETAPIE — engine.rs `be_od_etapu`. `be_at_tp1` to
   //  szczegolny przypadek (etap 1); pole uogolnia go na dowolny prog.
   //  Kanon Synergy to 3: „TP3 HIT ... SL IS SET TO BE AT 4038".
   int prog_be = (In_BeOdEtapu > 0) ? In_BeOdEtapu : (In_BeAtTp1 ? 1 : 0);
   if(prog_be > 0 && target_stage >= prog_be)
     {
      int zywe_be = 0;
      for(int i = 0; i < g_b[bi].npos; i++)
         if(PositionSelectByTicket(g_b[bi].pos[i])) zywe_be++;
      if(In_BeMinPozycji <= 0 || zywe_be >= In_BeMinPozycji) MoveBasketToBe(bi);
     }

   // Stop na płytkiej (najwcześniej dotykanej) krawędzi STREFY SYGNAŁU.
   if(In_SlPoTp1Krawedz && target_stage >= 1)
     {
      double a = MathMin(g_b[bi].entry_lo, g_b[bi].entry_hi);
      double z = MathMax(g_b[bi].entry_lo, g_b[bi].entry_hi);
      if(z - a > 1e-9) SetBasketSl(bi, WorseEdge(g_b[bi].side, a, z));
     }

   // TYLER: stop w połowie drogi po przedostatnim celu — engine.rs:4063
   if(In_SlPolowaOdKonca > 0)
     {
      int prog = g_b[bi].ntp - In_SlPolowaOdKonca;
      if(prog > 0 && target_stage >= prog) SlPolowaDrogi(bi);
     }

   // drabinka SL: SL = osiągnięty TP[etap − lag] ± oddech — engine.rs:4071
   // (saturating_sub: przy lag >= etap kotwicą jest TP1)
   if(In_LadderFromTp > 0 && target_stage >= In_LadderFromTp)
     {
      int li = (int)MathMax(target_stage - 1 - In_LadderLag, 0);
      if(li < g_b[bi].ntp)
        {
         double v = g_b[bi].tps[li] - SideSign(g_b[bi].side) * In_LadderOffset;
         SetBasketSl(bi, v);
        }
     }

   ApplySmartSl(bi);
   Retarget(bi);
  }

// engine.rs:4103 drop_grid_on_target (+okno łaski, +keep_n)
void DropGridOnTarget(int bi, int stage)
  {
   if(!ExitRiskAllowed(bi)) return;
   if(KeepExplicitPending(bi)) return;
   g_b[bi].plan_observed_stage = MathMax(g_b[bi].plan_observed_stage, stage);
   int cs;
   if(In_PendingLifetime == 1) cs = 1;
   else if(In_PendingLifetime == 2) cs = 2;
   else if(In_PendingLifetime == 3) cs = 3;
   else return;
   if(stage < cs) return;
   long laska = (long)(MathMax(In_DropGraceMin, 0.0) * 60000.0);
   if(laska > 0 && BliskoStrefy(bi))
     {
      if(g_b[bi].drop_po_ts == 0) { g_b[bi].drop_po_ts = g_now + laska; g_cnt_grace++; }
      return;
     }
   int n = CancelPendingsKeep(bi, In_DropKeepN);
   if(n > 0) DiagKoszyk(bi, "DROP_NA_CELU");
  }

// dokoncz_odroczone_kasowanie — engine.rs:7557-7589: kasuje gdy minął TERMIN
// ALBO cena uciekła ponad prog (drugi wyzwalacz jest ważniejszy — powrót
// z >8 $ zdarza się rzadko); wypełnienie nogi NIE odwołuje kasacji.
void DokonczOdroczoneKasowanie()
  {
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!ExitRiskAllowed(bi)) continue;
      if(!Alive(bi) || KeepExplicitPending(bi) || g_b[bi].drop_po_ts == 0) continue;
      if(g_now < g_b[bi].drop_po_ts && BliskoStrefy(bi)) continue;
      g_b[bi].drop_po_ts = 0;
      int n = CancelPendingsKeep(bi, In_DropKeepN);
      if(n > 0) DiagKoszyk(bi, "DROP_PO_LASCE");
     }
  }

//====================================================================
//  RISK FREE (komunikat) — engine.rs:4460
//====================================================================
bool RfLevelPlausible(int bi, double level)
  {
   if(In_RfLevelSanityUsd <= 0.0) return true;
   double best = MathMin(MathAbs(level - g_bid), MathAbs(level - g_ask));
   best = MathMin(best, MathAbs(level - g_b[bi].zone_lo));
   best = MathMin(best, MathAbs(level - g_b[bi].zone_hi));
   if(g_b[bi].has_sl) best = MathMin(best, MathAbs(level - g_b[bi].sl));
   for(int i = 0; i < g_b[bi].ntp; i++)
      best = MathMin(best, MathAbs(level - g_b[bi].tps[i]));
   return best <= In_RfLevelSanityUsd;
  }

void HandleRiskFree(int bi, double level, bool has_level)
  {
   if(!ExitRiskAllowed(bi)) return;
   if(In_RiskFreeMode == 0) return;

   if(In_PendCancelOnRf && !KeepExplicitPending(bi))
      CancelPendings(bi);

   ulong live[MAXTK]; int n = 0;
   for(int i = 0; i < g_b[bi].npos; i++)
      if(PositionSelectByTicket(g_b[bi].pos[i])) { live[n] = g_b[bi].pos[i]; n++; }
   if(n == 0) return;

   int side = g_b[bi].side;
   double reference = has_level ? level : BetterEdge(side, g_b[bi].zone_lo, g_b[bi].zone_hi);
   int keep_n = MathMax(In_RiskFreeRunners, 1);

   ulong order[MAXTK];
   for(int i = 0; i < n; i++) order[i] = live[i];
   if(In_RiskFreeMode == 1)            // CloseAllKeepNearest
     {
      double distance[MAXTK];
      for(int i=0;i<n;i++)
        {distance[i]=1e18;if(PositionSelectByTicket(order[i]))distance[i]=MathAbs(PositionGetDouble(POSITION_PRICE_OPEN)-reference);}
      NativeStableSortTickets(order,distance,n,false);
     }
   else if(In_RiskFreeMode == 5)       // CloseAllKeepBest
      NativeSortProfit(order,n,true);

   ulong keepers[MAXTK]; int nk = 0;
   if(In_RiskFreeMode == 1 || In_RiskFreeMode == 5)
      for(int i = 0; i < MathMin(keep_n, n); i++) { keepers[nk] = order[i]; nk++; }
   else if(In_RiskFreeMode == 2)
      for(int i = 0; i < n; i++) { keepers[nk] = live[i]; nk++; }

   for(int i = 0; i < n; i++)
     {
      bool is_keeper = false;
      for(int j = 0; j < nk; j++) if(keepers[j] == live[i]) { is_keeper = true; break; }
      if(is_keeper) continue;
      bool should = false;
      if(In_RiskFreeMode == 3)      should = (PozZysk(live[i]) > 0.0);
      else if(In_RiskFreeMode == 2) should = false;
      else                          should = true;
      if(should) { g_powod_zamk = "RISKFREE"; ZamknijPozycje(live[i]); }
     }

   // runnery: SL na BE + cel wg konfiguracji
   ulong be_t[MAXTK]; int nbe = 0;
   if(In_RiskFreeMode == 2) { for(int i = 0; i < n; i++) { be_t[nbe] = live[i]; nbe++; } }
   else                     { for(int i = 0; i < nk; i++) { be_t[nbe] = keepers[i]; nbe++; } }

   for(int i = 0; i < nbe; i++)
     {
      ulong t = be_t[i];
      if(!PositionSelectByTicket(t)) continue;
      double op = PositionGetDouble(POSITION_PRICE_OPEN);
      double cur_tp = PositionGetDouble(POSITION_TP);
      double cur_sl = PositionGetDouble(POSITION_SL);
      double be = op + SideSign(side) * In_BeOffset;
      // LastTp/NextTp przy PUSTEJ drabince = brak celu (engine: tps.last()->None
      // ZDEJMUJE cel i flaguje runnera), nie „zostaw stary TP".
      double newtp = cur_tp; bool has_new = (cur_tp != 0.0);
      if(In_RfRunnerTarget == 1)
        {
         if(g_b[bi].ntp > 0) { newtp = g_b[bi].tps[g_b[bi].ntp - 1]; has_new = true; }
         else { has_new = false; newtp = 0.0; }
        }
      else if(In_RfRunnerTarget == 2) { has_new = false; newtp = 0.0; }
      else if(In_RfRunnerTarget == 3)
        {
         int st = g_b[bi].tp_stage;
         if(st < g_b[bi].ntp) { newtp = g_b[bi].tps[st]; has_new = true; }
         else if(g_b[bi].ntp > 0) { newtp = g_b[bi].tps[g_b[bi].ntp - 1]; has_new = true; }
         else { has_new = false; newtp = 0.0; }
        }
      if(BeReplacementAllowed(side, cur_sl, be, In_BeNeverLoosen)
         && SlIsValid(side, be)) ModyfikujPozycje(t, be, true, newtp, has_new);
      else if(newtp != cur_tp) ModyfikujPozycje(t, cur_sl, cur_sl != 0.0, newtp, has_new);
      // is_runner = pozycja bez celu (engine.rs:4565)
      int ip = PsEnsure(t);
      if(ip >= 0) g_ps_isrunner[ip] = !has_new;
     }
   g_b[bi].state = ST_RISKFREE;
   g_b[bi].secured = true;
   g_b[bi].secured_ts = g_now;   // secured_by_rule zostaje false (komunikat)
  }

//====================================================================
//  OUT AT ENTRY — engine.rs:4587
//====================================================================
void HandleOutAtEntry(int bi)
  {
   if(!ExitRiskAllowed(bi)) return;
   if(In_OutAtEntryMode == 0) return;
   if(In_ConfirmedExitRetry && In_OutAtEntryMode == 1) { RequestConfirmedExit(bi, "OAE"); return; }
   for(int i = g_b[bi].npos - 1; i >= 0; i--)
     {
      ulong t = g_b[bi].pos[i];
      if(!PositionSelectByTicket(t)) continue;
      double pnl = PozZysk(t);
      double pts = PozPunkty(t);
      bool act = false;
      if(In_OutAtEntryMode == 1)      act = true;                       // CloseAll
      else if(In_OutAtEntryMode == 2) act = (pnl < 0.0);                // CloseLosersOnly
      else if(In_OutAtEntryMode == 3) act = (MathAbs(pts) <= In_OaeBandPts);
      if(act) { g_powod_zamk = "OAE"; ZamknijPozycje(t); }
      else if(In_OutAtEntryMode == 4)
        {
         double op = PositionGetDouble(POSITION_PRICE_OPEN);
         double tp = PositionGetDouble(POSITION_TP);
         double cur_sl = PositionGetDouble(POSITION_SL);
         double be = op + SideSign(g_b[bi].side) * In_BeOffset;
         if(SlIsValid(g_b[bi].side, be)) ModyfikujPozycje(t, be, true, tp, tp != 0.0);
         else if(In_OaePodWoda == 1)
           {
            g_powod_zamk = "OAE_UNDERWATER";
            ZamknijPozycje(t);
           }
         else if(In_OaePodWoda == 2)
           {
            // Najciaśniejszy stop po właściwej stronie rynku; nigdy nie
            // rozluźnia już istniejącego stopu.
            double kres = (g_b[bi].side == 0) ? (g_bid - g_stops) : (g_ask + g_stops);
            double cel = (g_b[bi].side == 0) ? MathMin(be, kres) : MathMax(be, kres);
            bool ciasniej = (cur_sl == 0.0)
                            || (g_b[bi].side == 0 ? cel > cur_sl : cel < cur_sl);
            if(ciasniej && SlIsValid(g_b[bi].side, cel))
               ModyfikujPozycje(t, cel, true, tp, tp != 0.0);
           }
        }
     }
   CancelPendings(bi);
   if(In_OutAtEntryMode == 1) g_b[bi].state = ST_DONE;
  }

//====================================================================
//  SL HIT — engine.rs:4634
//====================================================================
void HandleSlHit(int bi)
  {
   if(!ExitRiskAllowed(bi)) return;
   if(In_SlHitMode == 0) return;
   if(In_SlHitMode == 1) { CancelPendings(bi); return; }
   if(In_SlHitMode == 2)
     {
      if(In_ConfirmedExitRetry) { RequestConfirmedExit(bi, "SLHIT"); return; }
      g_powod_zamk = "SLHIT";
      for(int i = g_b[bi].npos - 1; i >= 0; i--) ZamknijPozycje(g_b[bi].pos[i]);
      CancelPendings(bi);
      g_b[bi].state = ST_DONE;
      return;
     }
   // VerifyByPrice
   if(g_b[bi].has_sl)
     {
      double dist = (MidPx() - g_b[bi].sl) * SideSign(g_b[bi].side);
      if(dist > In_SlHitVerifyTol) return;
     }
   CancelPendings(bi);
  }

//====================================================================
//  SPP-BE — engine.rs:4821 zastosuj_spp_be (spp_sl_mode + pad)
//====================================================================
void ZastosujSppBe(int bi, double poziom)
  {
   if(!ExitRiskAllowed(bi)) return;
   if(In_SppSlMode == 0) return;
   int side = g_b[bi].side;
   double cel = poziom - SideSign(side) * In_SppSlPad;
   bool tylko_runnery = (In_SppSlMode == 3 || In_SppSlMode == 4);
   bool tylko_bank = (In_SppSlMode == 5);
   bool tylko_lepszy = (In_SppSlMode == 2 || In_SppSlMode == 4);
   int n = 0;
   for(int i = 0; i < g_b[bi].npos; i++)
     {
      ulong t = g_b[bi].pos[i];
      if(!PositionSelectByTicket(t)) continue;
      double tp = PositionGetDouble(POSITION_TP);
      double cur = PositionGetDouble(POSITION_SL);
      int ip = PsIdx(t);
      bool runner = (ip >= 0 && g_ps_isrunner[ip]);   // jak engine.rs:4840 (p.is_runner)
      if((tylko_runnery && !runner) || (tylko_bank && runner)) continue;
      if(tylko_lepszy && cur != 0.0 && !SideBetter(side, cur, cel)) continue;
      if(SlIsValid(side, cel) && ModyfikujPozycje(t, cel, true, tp, tp != 0.0)) n++;
     }
   if(!tylko_runnery && !tylko_bank && n > 0)
     { g_b[bi].sl = cel; g_b[bi].has_sl = true; }
  }

//====================================================================
//  ZAMKNIJ WSZYSTKO — engine.rs:4968 (czyści też kolejkę wyjść)
//====================================================================
void CloseEverything()
  {
   g_nqe = 0;   // kolejka wyjść traci sens
   if(StringLen(g_powod_zamk) == 0) g_powod_zamk = "CLOSEALL";
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!Alive(bi)) continue;
      if(In_ConfirmedExitRetry) { RequestConfirmedExit(bi, g_powod_zamk); continue; }
      for(int i = g_b[bi].npos - 1; i >= 0; i--) ZamknijPozycje(g_b[bi].pos[i]);
      CancelPendings(bi);
      g_b[bi].state = ST_DONE;
     }
  }

//====================================================================
//  DEDUP EDYCJI
//====================================================================
bool JuzWykonane(long msg, string key)
  {
   for(int i = 0; i < g_ndone; i++)
      if(g_done_msg[i] == msg && g_done_key[i] == key) return true;
   return false;
  }
void ZapamietajAkcje(long msg, string key)
  {
   if(g_ndone >= 4000)
     {
      for(int i = 0; i < 2000; i++) { g_done_msg[i] = g_done_msg[i + 2000]; g_done_key[i] = g_done_key[i + 2000]; }
      g_ndone = 2000;
     }
   g_done_msg[g_ndone] = msg; g_done_key[g_ndone] = key; g_ndone++;
  }

// Zapis do pamięci dedupu po dispatchu. Legacy pamięta również akcję
// zignorowaną; pełny status wyłącznie tę, która naprawdę przeszła.
void ZapamietajPoStatusie(long msg, string key, bool wykonana)
  {
   if(!In_DedupPelnyStatus || wykonana) ZapamietajAkcje(msg, key);
  }

// engine.rs `cele_z_runnerem_od`: najpierw usuń cele już zabrane przez
// rynek (z bezpiecznym ostatnim celem), potem dopisz drabinkę runnera.
void PrzygotujCele(int side, double &tps[], int &ntp)
  {
   ntp = MathMin(MathMax(ntp, 0), MAXTP);
   if(In_CelePominZaCena && ntp > 0)
     {
      double stare[MAXTP]; int ns = ntp;
      for(int i = 0; i < ns; i++) stare[i] = tps[i];
      int w = 0; double rynek = MidPx();
      for(int i = 0; i < ns; i++)
        {
         bool przed = (side == 0) ? (stare[i] > rynek) : (stare[i] < rynek);
         if(przed) { tps[w] = stare[i]; w++; }
        }
      if(w == 0) { tps[0] = stare[ns - 1]; w = 1; }
      ntp = w;
     }
   if(In_RunnerCeleN <= 0 || In_RunnerCeleKrok <= 0.0 || ntp <= 0) return;
   double ostatni = tps[ntp - 1];
   for(int i = 1; i <= In_RunnerCeleN && ntp < MAXTP; i++)
     {
      tps[ntp] = ostatni + SideSign(side) * In_RunnerCeleKrok * i;
      ntp++;
     }
  }

// Basket::zeruj_postep: native stores target timestamps, while price/SL touch
// histories are not retained by this adapter. Signal delivery memory is separate.
void NativeResetEntryProgress(int bi)
  {
   g_b[bi].tp_stage = 0;
   g_b[bi].plan_observed_stage = 0;
   g_b[bi].zone_touched = false;
   g_b[bi].drop_armed = false;
   g_b[bi].drop_po_ts = 0;
   g_b[bi].last_tp_ts = 0;
   for(int i = 0; i < MAXTP; i++) g_b[bi].tp_touch_ts[i] = 0;
  }

bool NativeEntryPlanChanged(const Basket &previous, const Basket &current)
  {
   if(previous.zone_lo != current.zone_lo || previous.zone_hi != current.zone_hi
      || previous.has_sl != current.has_sl
      || (current.has_sl && previous.sl != current.sl) || previous.ntp != current.ntp) return true;
   for(int i = 0; i < current.ntp; i++)
      if(previous.tps[i] != current.tps[i]) return true;
   return false;
  }

bool NativeEntryGridUnchanged(const Basket &previous, const Basket &current)
  {
   if(MathAbs(previous.zone_lo-current.zone_lo)>=1e-9 || MathAbs(previous.zone_hi-current.zone_hi)>=1e-9
      || previous.has_sl!=current.has_sl || previous.ntp!=current.ntp) return false;
   if(current.has_sl && (long)(previous.sl*1e9)!=(long)(current.sl*1e9)) return false;
   for(int i=0;i<current.ntp;i++)if(previous.tps[i]!=current.tps[i])return false;
   return true;
  }

bool NativeSameEntrySource(const NativeEntryPlanSource &previous, const NativeEntryPlanSource &current)
  {
   if(!previous.known || !current.known || previous.side!=current.side
      || previous.is_limit!=current.is_limit || previous.is_stop!=current.is_stop
      || previous.lo!=current.lo || previous.hi!=current.hi
      || previous.has_sl!=current.has_sl || (current.has_sl && previous.sl!=current.sl)
      || previous.tp_open!=current.tp_open || previous.has_warstwy_offset!=current.has_warstwy_offset
      || (current.has_warstwy_offset && previous.warstwy_offset!=current.warstwy_offset)
      || previous.ntp!=current.ntp) return false;
   for(int i=0;i<current.ntp;i++)if(previous.tps[i]!=current.tps[i])return false;
   return true;
  }

bool ApplySourceEntryEdit(int bi,const NativeEntryPlanSource &source,double &targets[],int count)
  {
   if(!ExitRiskAllowed(bi) || SourceWithdrawn(bi) || EntryReviewBlocked(bi))return false;
   // The source snapshot, not the current tightened stop/target plan, decides
   // whether a repeated source delivery is an exact no-op. No broker RPC here.
   if(NativeSameEntrySource(g_b[bi].entry_source,source))return true;
   if(source.side!=g_b[bi].side)return false;
   for(int i=1;i<source.ntp;i++)
      if((source.side==0 && source.tps[i]<source.tps[i-1])
         || (source.side==1 && source.tps[i]>source.tps[i-1]))return false;
   ApplyEntryEdit(bi,source.side,source.is_limit,source.is_stop,source.lo,source.hi,
                  source.sl,source.has_sl,source.warstwy_offset,source.has_warstwy_offset,targets,count);
   if(EntryReviewBlocked(bi))return false;
   g_b[bi].entry_source=source;
   return true;
  }

void ApplyEntryEdit(int bi, int side, bool is_limit, bool is_stop,
                    double lo, double hi, double sl, bool has_sl,
                    double warstwy_offset, bool has_warstwy_offset,
                    double &tps[], int ntp)
  {
   if(!ExitRiskAllowed(bi)) return;
   if(SourceWithdrawn(bi) || EntryReviewBlocked(bi)) return;
   Basket committed = g_b[bi];
   bool rebuild = g_b[bi].state == ST_PENDING && g_b[bi].npos == 0;
   ulong before_positions[]; ulong before_orders[];
   bool before_complete = true;
   if(rebuild) before_complete = ExitOwnedSnapshot(bi, before_positions, before_orders);
   if(rebuild && !before_complete)
     {
      g_b[bi].entry_review=true;
      g_b[bi].review_requested_lo=lo;g_b[bi].review_requested_hi=hi;
      if(In_Diag && g_handle_diag!=INVALID_HANDLE)
         FileWrite(g_handle_diag,"ENTRY_REVIEW",(string)g_now,(string)g_b[bi].id,
                   "LegacyEditReceiptBarrier",DoubleToString(lo,8),DoubleToString(hi,8));
      return;
     }
   double deep = AdaptiveDeepOffset(MathAbs(hi - lo));
   double zlo = lo, zhi = hi;
   if(In_ZoneOffsetMode == 1) { zhi += In_EntryHiOffset; zlo += In_EntryLoOffset; }
   else if(In_ZoneOffsetMode == 2)
     {
      if(side == 0) { zlo -= deep; zhi += In_EntryTolOffset; }
      else          { zhi += deep; zlo -= In_EntryTolOffset; }
     }
   double t1 = MathMin(zlo, zhi), t2 = MathMax(zlo, zhi);
   g_b[bi].zone_lo = t1; g_b[bi].zone_hi = t2;
   g_b[bi].entry_lo = lo; g_b[bi].entry_hi = hi;
   if(has_sl)
     {
      double slv = sl;
      double min_d = AdaptiveSlMinDist(MathAbs(hi - lo));
      double mid = (t1 + t2) * 0.5;
      if(min_d > 0.0)
        {
         double want = (side == 0) ? (mid - min_d) : (mid + min_d);
         slv = (side == 0) ? MathMin(slv, want) : MathMax(slv, want);
        }
      if(In_SlMaxDist > 0.0)
        {
         double capp = (side == 0) ? (mid - In_SlMaxDist) : (mid + In_SlMaxDist);
         slv = (side == 0) ? MathMax(slv, capp) : MathMin(slv, capp);
        }
      SetBasketSl(bi, slv);   // dociera do otwartych pozycji (i pendingów Z-5)
     }
   else { g_b[bi].has_sl=false;g_b[bi].sl=0.0; } // compute_sl(None): no broker stop removal
   g_b[bi].ntp = MathMin(ntp, MAXTP);
   for(int i = 0; i < g_b[bi].ntp; i++) g_b[bi].tps[i] = tps[i];
   bool plan_changed = NativeEntryPlanChanged(committed, g_b[bi]);
   // Full ENTRY edits reset effective-plan progress even for Working baskets.
   // This is independent of the separate SPP target-plan compatibility switch.
   if(plan_changed) NativeResetEntryProgress(bi);
   // przestawienie siatki TYLKO w stanie Armed bez pozycji
   if(rebuild && !NativeEntryGridUnchanged(committed,g_b[bi]))
     {
      int removed = 0;
      if(before_complete)
         for(int i = 0; i < ArraySize(before_orders); i++)
            if(ExitCancelOwned(bi, before_orders[i])) removed++;
      ulong after_positions[]; ulong after_orders[];
      bool after_complete = ExitOwnedSnapshot(bi, after_positions, after_orders);
      // This Armed transaction started with no tracked positions. Any actual
      // fill, retained pending or uncertain snapshot invalidates replacement.
      bool clean = before_complete && after_complete
                   && ArraySize(before_positions) == 0 && ArraySize(after_positions) == 0
                   && removed == ArraySize(before_orders) && ArraySize(after_orders) == 0;
      if(!clean)
        {
         g_b[bi] = committed;
         g_b[bi].entry_review = true;
         g_b[bi].review_requested_lo = lo; g_b[bi].review_requested_hi = hi;
         // Restore only committed strategy geometry. Confirmed broker changes
         // are facts; reconcile actual fills/cancels rather than undoing them.
         OdswiezBilety();
         ExitOwnedSnapshot(bi, after_positions, after_orders);
         ExitRememberLivePositions(bi, after_positions);
         if(ArraySize(after_positions) > 0) g_b[bi].state = ST_WORKING;
         if(In_Diag && g_handle_diag != INVALID_HANDLE)
            FileWrite(g_handle_diag, "ENTRY_REVIEW", (string)g_now, (string)g_b[bi].id,
                      "LegacyCancelOrFillUnconfirmed", DoubleToString(lo, 8), DoubleToString(hi, 8));
         return;
        }
      PlanGrid(bi);
      if(g_b[bi].nlv > 0) PlaceGrid(bi);
     }
  }

//====================================================================
//  WYKONANIE JEDNEJ WIADOMOŚCI
//====================================================================
void WykonajWiadomosc(int mi)
  {
   long key_msg = (g_msg[mi].edit_of != 0) ? g_msg[mi].edit_of : g_msg[mi].msg_id;
   g_source_original=key_msg;g_source_message=g_msg[mi].msg_id;g_source_edit=g_msg[mi].edit_of!=0;
   NativeObserveSourceReply(mi);

   for(int a = 0; a < g_msg[mi].n; a++)
     {
      string raw = g_msg[mi].akcje[a];
      int colon = StringFind(raw, ":");
      if(colon < 0) continue;
      string kind = StringSubstr(raw, 0, colon);
      string rest = StringSubstr(raw, colon + 1);
      string f[];
      int nf = StringSplit(rest, ',', f);
      if(nf < 1) continue;
      string akey = f[0];
      bool entry_kind = (kind == "ENTRY" || kind == "ENTRY2");
      if(kind=="INFO")continue;
      if((entry_kind || kind=="MKT") && NativeSourceEntryBlocked(key_msg,g_msg[mi].edit_of==0))
        {ZapamietajPoStatusie(key_msg,akey,false);continue;}
      if((entry_kind || kind == "MKT") && (SourceWithdrawn(BIdx(MapGet(key_msg))) || EntryReviewBlocked(BIdx(MapGet(key_msg)))))
        { ZapamietajPoStatusie(key_msg, akey, false); continue; }

      // Wejście w edycji ma własną ścieżkę: poprawia koszyk zanim dedup
      // zarządzania odsieje stare akcje doklejone do tej samej wiadomości.
      if(In_DedupEdited && g_msg[mi].edit_of != 0 && !entry_kind
         && JuzWykonane(g_msg[mi].edit_of, akey))
         continue;

      // Re-delivery po rekonekcie może przyjść jako NEW z tym samym msg_id.
      // ENTRY ma osobną idempotencję, MKT nie ma koszyka-adresata; dokładnie
      // jak Rust filtrujemy tu wyłącznie akcje zarządzające już wykonane.
      bool management_kind = (!entry_kind && kind != "MKT");
      if(In_DedupMgmtReplay && g_msg[mi].edit_of == 0 && management_kind
         && JuzWykonane(g_msg[mi].msg_id, akey))
         continue;

      if(entry_kind)
        {
         bool v2 = (kind == "ENTRY2");
         if((!v2 && nf < 7) || (v2 && nf < 13))
           { ZapamietajPoStatusie(key_msg, akey, false); continue; }
         int side = (f[1] == "BUY") ? 0 : 1;
         bool is_limit = (StringToInteger(f[2]) != 0);
         bool is_stop = v2 && (StringToInteger(f[3]) != 0);
         int ix = v2 ? 4 : 3;
         double lo = StringToDouble(f[ix]);
         double hi = StringToDouble(f[ix + 1]);
         bool has_sl = (f[ix + 2] != "nan");
         double sl = has_sl ? StringToDouble(f[ix + 2]) : 0.0;
         bool tp_open = (StringToInteger(f[ix + 3]) != 0);
         double warstwy_offset = 0.0; bool has_warstwy_offset = false;
         double tps[MAXTP]; int ntp = 0;
         if(v2)
           {
            has_warstwy_offset = (f[8] != "nan");
            if(has_warstwy_offset) warstwy_offset = StringToDouble(f[8]);
            int wire_ntp = MathMax((int)StringToInteger(f[12]), 0);
            for(int i = 0; i < wire_ntp && 13 + i < nf && ntp < MAXTP; i++)
              { tps[ntp] = StringToDouble(f[13 + i]); ntp++; }
           }
         else
            for(int i = 7; i < nf && ntp < MAXTP; i++)
              { if(StringLen(f[i])==0)continue;tps[ntp] = StringToDouble(f[i]); ntp++; }
         bool complete_recovery=NativeCompleteRecovery(side,is_limit,is_stop,lo,hi,sl,has_sl,tp_open,
                                                       warstwy_offset,has_warstwy_offset,tps,ntp);
         NativeEntryPlanSource source;ZeroMemory(source);source.known=true;
         source.side=side;source.is_limit=is_limit;source.is_stop=is_stop;
         source.lo=lo;source.hi=hi;source.sl=sl;source.has_sl=has_sl;source.tp_open=tp_open;
         source.warstwy_offset=warstwy_offset;source.has_warstwy_offset=has_warstwy_offset;
         source.ntp=ntp;for(int i=0;i<ntp;i++)source.tps[i]=tps[i];
         PrzygotujCele(side, tps, ntp);

         if(g_msg[mi].edit_of==0 && In_EntryIdempotency && MapGet(key_msg)>=0)
           {
            int known=BIdx(MapGet(key_msg));
            bool handled=known>=0 && ApplySourceEntryEdit(known,source,tps,ntp);
            ZapamietajPoStatusie(key_msg,akey,handled);continue;
           }

         if(g_msg[mi].edit_of != 0)
           {
            int id = MapGet(g_msg[mi].edit_of);
            if(id >= 0)
              {
               int bx = BIdx(id);
                 if(bx >= 0)
                 {
                  if(!ExitRiskAllowed(bx) || SourceWithdrawn(bx) || EntryReviewBlocked(bx)) { ZapamietajPoStatusie(key_msg, akey, false); continue; }
                  bool handled=ApplySourceEntryEdit(bx,source,tps,ntp);
                  if(handled) {int source_index=NativeSourceEnsure(key_msg);g_sources[source_index].had_edit=true;}
                  DiagKoszyk(bx, handled ? "EDYCJA" : "EDYCJA_ODRZUC");
                  ZapamietajPoStatusie(key_msg, akey, handled);
                  if(!In_EditRest) return;
                  continue;
                 }
               // A consumed source whose basket was pruned cannot be recreated.
               continue;
              }
            // Edycja-sierota nie może spaść do HandleEntry i utworzyć
            // świeżego koszyka na starych cenach. Usuwamy wyłącznie akcję
            // ENTRY; dalsze akcje zarządzające z wiadomości idą normalnie.
            if(In_EditOrphanNoEntry || !complete_recovery)
              {
               ZapamietajPoStatusie(key_msg, akey, false);
               if(In_Diag && g_handle_diag != INVALID_HANDLE)
                  FileWrite(g_handle_diag, "ODRZUC_EDIT_ORPHAN", (string)g_now,
                            (string)g_msg[mi].msg_id, (string)g_msg[mi].edit_of);
               continue;
              }
           }
         long rej0 = g_cnt_reject, risk0 = g_rej_risk;
         HandleEntry(g_msg[mi].msg_id, side, is_limit, is_stop, lo, hi, sl, has_sl,
                     tp_open, warstwy_offset, has_warstwy_offset, tps, ntp,source);
         // JEDYNE miejsce gaszenia flagi miekkiego rezimu — engine.rs:1755.
         // Gaszenie przy kazdym `return` w HandleEntry zabiloby dzialanie
         // regime_soft_risk_mult (CapBasketRisk/MarketRiskCap czytaja ja pozniej).
         g_rezim_miekki = false;
         int nowy_id = MapGet(g_msg[mi].msg_id);
         bool wykonana = (g_cnt_reject == rej0 && g_rej_risk == risk0
                          && nowy_id >= 0 && BIdx(nowy_id) >= 0);
         ZapamietajPoStatusie(key_msg, akey, wykonana);
         continue;
        }
      if(kind == "MKT")
        {
         if((g_msg[mi].edit_of!=0 && MapGet(key_msg)<0)
            || ((In_EntryIdempotency || g_msg[mi].edit_of!=0) && MapGet(key_msg)>=0))
           {ZapamietajPoStatusie(key_msg,akey,false);continue;}
         if(nf > 1) HandleMkt(g_msg[mi].msg_id, (f[1] == "BUY") ? 0 : 1);
         int mid = MapGet(g_msg[mi].msg_id);
         ZapamietajPoStatusie(key_msg, akey,
                              In_HonorMarketOpen && mid >= 0 && BIdx(mid) >= 0);
         continue;
        }

      // HAMULEC SL-HIT liczy KAZDY komunikat SL HIT kanalu — takze taki, ktory
      // nie trafia w zaden nasz koszyk (engine.rs:1813-1816: informacja o rezimie
      // pochodzi z kanalu, nie z naszych pozycji). Dlatego PRZED TargetBasket.
      if(kind == "SLHIT")
        {
         g_slhit_dnia++;
         if(In_SlhitPauseN > 0 && g_slhit_dnia >= In_SlhitPauseN
            && g_now >= g_slhit_pauza_do)     // engine.rs:1819 — trwajacej pauzy nie przedluza
            g_slhit_pauza_do = (In_SlhitPauseMin > 0.0)
                               ? g_now + (long)(In_SlhitPauseMin * 60000.0)
                               : LONG_MAX;    // do granicy doby — zdejmie RolkaDoby
        }

      // Wyłączona akcja nie może tworzyć aliasu reply_graph ani trafić do
      // pamięci „wykonanych" w trybie pełnego statusu.
      if(kind=="CANCEL" && g_msg[mi].reply_to!=0)
        {
         int source=NativeSourceFind(g_msg[mi].reply_to);
         if(source>=0 && g_sources[source].basket<0)
           {ZapamietajPoStatusie(key_msg,akey,true);continue;}
        }
      if(kind == "CANCEL" && In_ExplicitPendingUntilCancel)
        {
         int target = TargetBasket(mi, false);
         if(ExplicitPendingSource(target))
           {
            bool known_reply = g_msg[mi].reply_to != 0 && MapGet(g_msg[mi].reply_to) >= 0;
            if(!known_reply)
              { ZapamietajPoStatusie(key_msg, akey, false); continue; }
            TargetZAliasem(mi, target);
            WithdrawPendingSource(target);
            ZapamietajPoStatusie(key_msg, akey, true);
            continue;
           }
        }
      if((kind == "CANCEL" && !In_HonorCancel)
         || (kind == "CLOSEALL" && !In_HonorCloseAll))
        { ZapamietajPoStatusie(key_msg, akey, false); continue; }

      // Rust CloseAllScope::Global does not require a target basket. Legacy
      // OFF preserves the historical route (including the target requirement).
      if(In_ConfirmedExitRetry && kind == "CLOSEALL")
        {
         g_powod_zamk = "CLOSEALL"; CloseEverything();
         ZapamietajPoStatusie(key_msg, akey, true); continue;
        }

      int bi = TargetBasket(mi);
      if(bi < 0) { ZapamietajPoStatusie(key_msg, akey, false); continue; }

      bool wykonana = false;
      if(kind == "TPHIT" || kind == "TPHIT2")
        {
         if(In_TpSource == 0)
           { ZapamietajPoStatusie(key_msg, akey, false); continue; }
         int idx = (nf > 1) ? (int)StringToInteger(f[1]) : -1;
         if(idx==0)idx=1; // Some(0).max(1), not None / advance-current-stage
         bool has_hit_level = (kind == "TPHIT2" && nf > 2 && f[2] != "nan");
         double hit_level = has_hit_level ? StringToDouble(f[2]) : 0.0;
         bool unindexed_pips = (kind == "TPHIT2" && nf > 3
                                && StringToInteger(f[3]) != 0);

         // Z-6: poziom bez indeksu dopasowujemy do najwyższego celu w
         // tolerancji. Powtórzenie staje się wtedy idempotentne.
         if(In_TpHitMatchLevel && idx < 0 && has_hit_level)
           {
            double tol2 = MathMax(In_BasketHintTol, 0.5);
            for(int i = 0; i < g_b[bi].ntp; i++)
               if(MathAbs(g_b[bi].tps[i] - hit_level) <= tol2) idx = i + 1;
           }

         // signal_tp_check — engine.rs:1937-2014:
         // SignalConfirmedByPrice: cena TERAZ przy poziomie lub za nim
         //   (BUY: bid >= tp − tp_price_tolerance); spozniony komunikat po
         //   glebokim cofnieciu ODPADA — historia dotkniec nie wystarcza.
         // PriceFirstSignalWindow: po dotknieciu — okno lag (czas od
         //   PIERWSZEGO dotkniecia); przed dotknieciem — lead mierzony
         //   BLISKOSCIA CENY (tolerancja), wykonanie natychmiast albo odmowa.
         bool force_price = In_TpUnidxPipsPrice && idx < 0 && unindexed_pips;
         if(force_price || In_TpSource == 3 || In_TpSource == 4)
           {
            int cel = (idx > 0) ? idx - 1 : g_b[bi].tp_stage;
            if(cel < 0 || cel >= g_b[bi].ntp)
              { ZapamietajPoStatusie(key_msg, akey, false); continue; }
            else
              {
               double tpv = g_b[bi].tps[cel];
               double tol = MathMax(In_TpPriceTol, 0.0);
               bool blisko = (g_b[bi].side == 0) ? (g_bid >= tpv - tol)
                                                 : (g_ask <= tpv + tol);
               bool ok;
               if(force_price || In_TpSource == 3) ok = blisko;
               else
                 {
                  long touch = g_b[bi].tp_touch_ts[cel];
                  if(touch != 0)
                     ok = (In_TpSigMaxLagS <= 0.0
                           || g_now - touch <= (long)(In_TpSigMaxLagS * 1000.0));
                  else
                     ok = (In_TpSigMaxLeadS > 0.0 && blisko);
                 }
               if(!ok) { ZapamietajPoStatusie(key_msg, akey, false); continue; }
              }
           }
         HandleTpHit(bi, idx);
         wykonana = true;
        }
      else if(kind == "SLHIT")   { HandleSlHit(bi); wykonana = true; }
      else if(kind == "RF")
        {
         bool hl = (nf > 1 && f[1] != "nan");
         double poziom = hl ? StringToDouble(f[1]) : 0.0;
         if(hl && !RfLevelPlausible(bi, poziom)) wykonana = false;
         else { HandleRiskFree(bi, poziom, hl); wykonana = true; }
        }
      else if(kind == "OAE")     { HandleOutAtEntry(bi); wykonana = (In_OutAtEntryMode != 0); }
      else if(kind == "CANCEL")  { CancelPendings(bi); wykonana = true; }
      else if(kind == "CLOSEALL"){ g_powod_zamk = "CLOSEALL"; CloseEverything(); wykonana = true; }
      else if(kind == "BE")      { MoveBasketToBe(bi); wykonana = true; }
      else if(kind == "SETSL")
        {
         if(nf > 1) { SetBasketSl(bi, StringToDouble(f[1])); wykonana = true; }
        }
      else if(kind == "TPCORR")
        {
         if(nf > 2)
           {
            int idx = (int)StringToInteger(f[1]);
            double v = StringToDouble(f[2]);
            if(idx >= 1 && idx <= g_b[bi].ntp) g_b[bi].tps[idx - 1] = v;
            else if(idx == g_b[bi].ntp + 1 && g_b[bi].ntp < MAXTP) { g_b[bi].tps[g_b[bi].ntp] = v; g_b[bi].ntp++; }
            // Z-8: korekta celu dochodzi do brokera (retarget + pendingi)
            if(In_TpCorrToBroker)
              {
               Retarget(bi);
               for(int i = 0; i < g_b[bi].npend; i++)
                 {
                  ulong t = g_b[bi].pend[i];
                  if(!OrderSelect(t)) continue;
                  double tp2 = 0.0;
                  if(!NativeCorrectionPendingTarget(bi,tp2))continue;
                  MqlTradeRequest r; MqlTradeResult res;
                  ZeroMemory(r); ZeroMemory(res);
                  r.action = TRADE_ACTION_MODIFY;
                  r.order  = t;
                  r.price  = OrderGetDouble(ORDER_PRICE_OPEN);
                  r.sl     = OrderGetDouble(ORDER_SL);
                  r.tp     = NormPx(tp2);
                  r.type_time = ORDER_TIME_GTC;
                  if(!OrderSend(r, res) || res.retcode != TRADE_RETCODE_DONE)
                     ZliczOdrzucenie((int)res.retcode);
                 }
              }
            wykonana = true;
           }
        }
      else if(kind == "SPP")
        {
         // SPP:klucz,sl,be,t1,t2,...
         if(In_SppMaxAgeH > 0.0 &&
            (g_now - g_b[bi].created_ts) > (long)(In_SppMaxAgeH * 3600000.0))
           { ZapamietajPoStatusie(key_msg, akey, false); continue; }
         if(nf > 3 && !In_SppKeepTp)
           {
            double tt[MAXTP];int ntp=NativeReadSppTargets(f,nf,tt);
            if(ntp > 0) ApplySppTargetPlan(bi, tt, ntp);
           }
         if(nf > 1 && f[1] != "nan") SetBasketSl(bi, StringToDouble(f[1]));
         // poziom BE z komunikatu — spp_sl_mode (engine dispatch:1783)
         if(nf > 2 && f[2] != "nan") ZastosujSppBe(bi, StringToDouble(f[2]));
         bool ma_poz = (g_b[bi].npos > 0);
         if(ma_poz)
           {
            g_b[bi].secured = true;
            if(In_SppBlockRearmFlat)
              {
               // Tryb zgodny z Rust: zegar SPP uzbraja wyłącznie jawna oś.
               if(In_SppArmsRunnerClk && g_b[bi].secured_ts == 0)
                  g_b[bi].secured_ts = g_now;
              }
            else if(g_b[bi].secured_ts == 0 || In_SppArmsRunnerClk)
               g_b[bi].secured_ts = g_now; // historyczne zachowanie EA
           }
         else if(In_SppBlockRearmFlat)
            g_b[bi].rearm_blocked_by_spp = true;
         else
           {
            // Kontrakt legacy EA: przed nową osią SPP oznaczał secured także
            // bez pozycji. Zachowujemy go dokładnie przy OFF.
            g_b[bi].secured = true;
            if(g_b[bi].secured_ts == 0 || In_SppArmsRunnerClk) g_b[bi].secured_ts = g_now;
           }
         HandleTpHit(bi, -1);
         wykonana = true;
        }
      ZapamietajPoStatusie(key_msg, akey, wykonana);
     }
  }


//====================================================================
//  WYKRYWANIE CELÓW Z CENY — engine.rs:5423
//====================================================================
void WykryjCeleZCeny()
  {
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!ExitRiskAllowed(bi)) continue;
      if(!Alive(bi)) continue;
      int side = g_b[bi].side;
      if(!g_b[bi].zone_touched)
        {
         bool w_strefie = (side == 0) ? (g_ask <= g_b[bi].zone_hi + 1e-9)
                                      : (g_bid >= g_b[bi].zone_lo - 1e-9);
         if(w_strefie) g_b[bi].zone_touched = true;
        }
      bool ma_poz = (g_b[bi].npos > 0);
      // Autonomiczny front-run jest wyjątkiem od SignalOnly: dodatnia oś
      // czyta wyłącznie bieżący Bid/Ask MT5. Nie dotyczy pustej siatki,
      // więc nie skraca jej życia ani nie ukrywa reguły wejścia.
      if(In_TpSource == 1 && (!ma_poz || In_TpPriceFrontRun <= 0.0)) continue;
      int stage = ma_poz ? g_b[bi].tp_stage : MathMax(g_b[bi].tp_stage, g_b[bi].plan_observed_stage);
      if(stage >= g_b[bi].ntp) continue;
      double next = g_b[bi].tps[stage];
      double front = ma_poz ? MathMax(In_TpPriceFrontRun, 0.0) : 0.0;
      bool reached = (side == 0) ? (g_bid >= next - front) : (g_ask <= next + front);
      if(!reached) { g_b[bi].drop_armed = true; continue; }
      // engine.rs:5423-5503: SAMO DOTKNIECIE ceny przesuwa etap we WSZYSTKICH
      // trybach cenowych (PriceOnly/Either/SignalConfirmedByPrice/
      // PriceFirstSignalWindow) — rozróżnianie trybów dotyczy wyłącznie
      // obsługi KOMUNIKATU (signal_tp_check w WykonajWiadomosc).
      if(ma_poz)
         HandleTpHit(bi, stage + 1);
      else if(In_PendingDropOnTgt && (g_b[bi].drop_armed || !In_PendingDropArm))
        {
         if(In_DropRequireTouch && !g_b[bi].zone_touched) continue;
         DropGridOnTarget(bi, stage + 1);
        }
     }
  }

//====================================================================
//  TTL PENDINGÓW — engine.rs on_tick krok 17 (pending_ttl_h)
//====================================================================
void PendingTtl()
  {
   if(In_PendingTtlH <= 0.0) return;
   long lim = (long)(In_PendingTtlH * 3600000.0);
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!ExitRiskAllowed(bi)) continue;
      if(!Alive(bi) || KeepExplicitPending(bi) || g_b[bi].npend == 0) continue;
      for(int i = g_b[bi].npend - 1; i >= 0; i--)
        {
         ulong t = g_b[bi].pend[i];
         if(!OrderSelect(t)) continue;
         long od;
         if(In_PendingTtlOdKosz) od = g_b[bi].created_ts;
         else od = (long)OrderGetInteger(ORDER_TIME_SETUP_MSC);
         if(g_now - od <= lim) continue;
         if(UsunPendingZnacz(bi, i)) g_cnt_ttl++;
        }
     }
  }

//====================================================================
//  WYGASANIE — engine.rs:6466 expire_stale + 7592 expire_old
//====================================================================
void ExpireStale()
  {
   if(In_IgnoreOldAfterMin <= 0.0) return;
   long lim = (long)(In_IgnoreOldAfterMin * 60000.0);
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!ExitRiskAllowed(bi)) continue;
      if(!Alive(bi) || KeepExplicitPending(bi) || g_b[bi].had_positions || g_b[bi].npos > 0) continue;
      if(g_now - g_b[bi].created_ts > lim)
        {
         if(In_ConfirmedExitRetry) RequestConfirmedExit(bi, "WYGASL");
         else { CancelPendings(bi); g_b[bi].state = ST_DONE; }
        }
     }
  }

void ExpireOld()
  {
   double limit_glob = BasketMaxAgeEff();
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!ExitRiskAllowed(bi)) continue;
      if(!Alive(bi) || KeepExplicitPending(bi)) continue;
      double lim = 1e18;
      if(limit_glob > 0.0) lim = limit_glob;
      if(g_b[bi].age_limit_min > 0.0) lim = MathMin(lim, g_b[bi].age_limit_min);
      if(lim >= 1e17) continue;
      // wiek_od_wypelnienia: zegar od PIERWSZEGO wypełnienia — engine.rs:7615
      long od = g_b[bi].created_ts;
      if(In_WiekOdWypelnienia)
        {
         long naj = 0;
         for(int i = 0; i < g_b[bi].nlv; i++)
            if(g_b[bi].lv_fill_ts[i] > 0 && (naj == 0 || g_b[bi].lv_fill_ts[i] < naj))
               naj = g_b[bi].lv_fill_ts[i];
         if(naj == 0) continue;   // niewypełniony nie podlega temu limitowi
         od = naj;
        }
      if(g_now - od <= (long)(lim * 60000.0)) continue;
      DiagKoszyk(bi, "WYGASL");
      g_powod_zamk = "WYGASL";
      if(In_ConfirmedExitRetry) { RequestConfirmedExit(bi, "WYGASL"); continue; }
      int bylo = g_b[bi].npos, zamkniete = 0;
      for(int i = g_b[bi].npos - 1; i >= 0; i--)
         if(ZamknijPozycje(g_b[bi].pos[i])) zamkniete++;
      CancelPendings(bi);
      if(zamkniete >= bylo) g_b[bi].state = ST_DONE;
      // inaczej koszyk zostaje żywy i ponowi przy następnym ticku
     }
  }

//====================================================================
//  FILTR TEMPA — engine.rs:7799 (+ historia reżimu dla piramidy)
//====================================================================
void FastFillCheck()
  {
   if(In_FastFillRejectS <= 0.0) return;
   int potrzeba = MathMax(In_FastFillLayers, 2);
   long prog = (long)(In_FastFillRejectS * 1000.0);
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!ExitRiskAllowed(bi)) continue;
      if(!Alive(bi) || g_b[bi].tempo_checked) continue;
      long cz[MAXLV]; int nc = 0;
      for(int i = 0; i < g_b[bi].nlv; i++) if(g_b[bi].lv_fill_ts[i] > 0) { cz[nc] = g_b[bi].lv_fill_ts[i]; nc++; }
      if(nc < potrzeba) continue;
      for(int a = 0; a < nc - 1; a++)
         for(int b2 = a + 1; b2 < nc; b2++)
            if(cz[b2] < cz[a]) { long t = cz[a]; cz[a] = cz[b2]; cz[b2] = t; }
      long rozpietosc = cz[potrzeba - 1] - cz[0];
      g_b[bi].tempo_checked = true;
      bool przelot = (rozpietosc < prog);
      // historia reżimu widzi OBIE odpowiedzi (engine.rs:7830)
      if(g_nregime >= 100)
        { for(int i = 0; i < 99; i++) { g_regime_hist[i] = g_regime_hist[i+1]; g_regime_hist_ts[i] = g_regime_hist_ts[i+1]; } g_nregime = 99; }
      g_regime_hist[g_nregime] = przelot; g_regime_hist_ts[g_nregime] = g_now; g_nregime++;
      if(!przelot) continue;
      g_b[bi].tempo_fast = true;
      double miekki = KapF(In_FastFillSoftAgeM, In_FfSoftAgeSmall, In_FfSoftAgeSmallM);
      if(miekki > 0.0)
        {
         CancelPendings(bi);
         g_b[bi].age_limit_min = miekki;
         continue;
        }
      g_powod_zamk = "FASTFILL";
      if(In_ConfirmedExitRetry) { RequestConfirmedExit(bi, "FASTFILL"); continue; }
      for(int i = g_b[bi].npos - 1; i >= 0; i--) ZamknijPozycje(g_b[bi].pos[i]);
      CancelPendings(bi);
      g_b[bi].state = ST_DONE;
     }
  }

//====================================================================
//  ENFORCE POSITION LIMIT — engine.rs:7763 (limit działa też po fillach)
//====================================================================
void EnforcePositionLimit()
  {
   int limit = MaxOpenPositionsEff();
   if(!In_EnforcePosLimit || limit == 0) return;
   if(LiczPozycje() < limit) return;
   int razem = 0;
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!ExitRiskAllowed(bi)) continue;
      if(!Alive(bi) || g_b[bi].npend == 0) continue;
      razem += CancelPendings(bi);
     }
   if(razem > 0) g_cnt_enforce += razem;
  }

//====================================================================
//  ZONE EXIT ADVERSE — engine.rs:8079 (ujemna wartość = kontrola lustrzana)
//====================================================================
void ZoneExitAdverseSweep()
  {
   if(In_ZoneExitAdverseS == 0.0) return;
   bool lustro = (In_ZoneExitAdverseS < 0.0);
   long prog = (long)(MathAbs(In_ZoneExitAdverseS) * 1000.0);
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!ExitRiskAllowed(bi)) continue;
      if(!Alive(bi) || !g_b[bi].had_positions) continue;
      double lo = MathMin(g_b[bi].zone_lo, g_b[bi].zone_hi);
      double hi = MathMax(g_b[bi].zone_lo, g_b[bi].zone_hi);
      bool przeciw;
      if((g_b[bi].side == 0 && !lustro) || (g_b[bi].side == 1 && lustro))
         przeciw = (g_bid < lo);
      else
         przeciw = (g_ask > hi);
      if(!przeciw) { g_b[bi].adverse_since = 0; continue; }
      if(g_b[bi].adverse_since == 0) { g_b[bi].adverse_since = g_now; continue; }
      if(g_now - g_b[bi].adverse_since < prog) continue;
      int zamkniete = 0;
      if(In_ZoneExitAdvClose)
        {
         g_powod_zamk = "ZONEEXIT";
         if(In_ConfirmedExitRetry) { RequestConfirmedExit(bi, "ZONEEXIT"); continue; }
         for(int i = g_b[bi].npos - 1; i >= 0; i--)
            if(ZamknijPozycje(g_b[bi].pos[i])) zamkniete++;
        }
      int skasowane = CancelPendings(bi);
      g_b[bi].adverse_since = 0;
      if(In_ZoneExitAdvClose) g_b[bi].state = ST_DONE;
      if(zamkniete + skasowane > 0) g_cnt_zoneexit++;
     }
  }

//====================================================================
//  DOKŁADKA TEMPOWA — engine.rs:7967 fast_addon_sweep
//====================================================================
void FastAddonSweep()
  {
   if(In_FastAddonMoveUsd <= 0.0 || In_FastAddonMax == 0 || In_FastAddonWindowS <= 0.0) return;
   int n = 0;
   double baza = VolOldestInWindow((long)(In_FastAddonWindowS * 1000.0), n);
   if(n < 3) return;
   double mid = MidPx();
   long ostyg = (long)(In_FastAddonCooldownS * 1000.0);
   int limit = MaxOpenPositionsEff();
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!ExitRiskAllowed(bi)) continue;
      if(EntryReviewBlocked(bi)) continue;
      if(!Alive(bi) || !g_b[bi].had_positions) continue;
      if(g_b[bi].fast_addons >= In_FastAddonMax || g_b[bi].tp_stage < In_FastAddonMinStage) continue;
      if(ostyg > 0 && g_b[bi].last_addon_ts > 0 && g_now - g_b[bi].last_addon_ts < ostyg) continue;
      double ruch = (mid - baza) * SideSign(g_b[bi].side);
      if(ruch < In_FastAddonMoveUsd) continue;
      // TA ŚCIEŻKA NIE WOŁA entry_gate — tylko licznik pozycji + ml (engine.rs:8030)
      if(limit > 0 && LiczPozycje() >= limit) break;
      if(!MarginesPozwala(In_MlMinFastAddon)) break;
      // Match the Rust caller: cap AFTER the addon multiplier, before broker rounding.
      double vol = MathMax(LotSize() * MathMax(In_FastAddonLotMult, 0.0), In_LotMin);
      if(In_LotMax > 0.0) vol = MathMin(vol, In_LotMax);
      vol = RoundLot(vol);
      double tp_ost = (g_b[bi].ntp > 0) ? g_b[bi].tps[g_b[bi].ntp - 1] : 0.0;
      // A locally invalid target keeps the slot but starts the configured
      // cooldown, matching the source-backed retry policy in Rust.
      if(g_b[bi].ntp>0 && !TpIsValid(g_b[bi].side,tp_ost))
        {g_b[bi].last_addon_ts=g_now;continue;}
      long requests_before=g_open_request_count;
      ulong tk = 0;
      if(WyslijRynek(bi, -4, vol, g_b[bi].sl, g_b[bi].has_sl, tp_ost, g_b[bi].ntp > 0,
                     "B" + IntegerToString(g_b[bi].id), tk))
        {
         if(g_b[bi].npos < MAXTK)
           { g_b[bi].pos[g_b[bi].npos] = tk; g_b[bi].pos_lv[g_b[bi].npos] = -4; g_b[bi].npos++; }
         ZapiszWlasciciela(tk, bi);
         g_b[bi].fast_addons++;
         g_b[bi].last_addon_ts = g_now;
         g_cnt_fastaddon++;
         g_cnt_order++;
        }
      else if(g_open_request_count>requests_before)
        {
         // A transmitted refusal/unknown result consumes the attempt to
         // prevent duplicate risk. Local guards above do not consume it.
         g_b[bi].fast_addons=MathMin(g_b[bi].fast_addons+1,In_FastAddonMax);
         g_b[bi].last_addon_ts = g_now;
        }
     }
  }

//====================================================================
//  REARM — engine.rs:8221 (przezbrojenie siatki po powrocie do strefy)
//====================================================================
void RearmPass()
  {
   if(!In_RearmGridOnReturn) return;
   if(!MarginesPozwala(In_MlMinRearm)) return;
   if(EntryGateNaTick() != 0) return;
   long odstep = (long)(MathMax(In_RearmMinGapMin, 0.0) * 60000.0);
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!ExitRiskAllowed(bi)) continue;
      if(EntryReviewBlocked(bi)) continue;
      if(!Alive(bi) || g_b[bi].nlv == 0) continue;
      if(!g_b[bi].had_positions)
        {
         if(!In_RearmBezPozycji) continue;
         if(In_RearmBezPozMaxH > 0.0
            && g_now - g_b[bi].created_ts > (long)(In_RearmBezPozMaxH * 3600000.0)) continue;
        }
      if(In_RearmBlockSecured && g_b[bi].secured) continue;
      if(In_SppBlockRearmFlat && g_b[bi].rearm_blocked_by_spp) continue;
      if(In_RearmMaxTimes > 0 && g_b[bi].rearms >= In_RearmMaxTimes) continue;
      if(g_b[bi].last_rearm_ts != 0 && g_now - g_b[bi].last_rearm_ts < odstep) continue;
      int side = g_b[bi].side;
      double px = EntryPx(side);
      if(px < g_b[bi].zone_lo - 1e-9 || px > g_b[bi].zone_hi + 1e-9) continue;
      if(g_b[bi].has_sl)
        {
         bool przebity = (side == 0) ? (px <= g_b[bi].sl) : (px >= g_b[bi].sl);
         if(przebity) continue;
        }
      // potwierdzenie: koszyk na plusie (zrealizowane + otwarte)
      double otwarte = 0.0;
      for(int i = 0; i < g_b[bi].npos; i++) otwarte += PozZysk(g_b[bi].pos[i]);
      if(g_b[bi].realized + otwarte < In_RearmMinBasketPl) continue;
      int dostawione = PlaceGrid(bi, true, 0, true);
      if(dostawione == 0) continue;
      g_b[bi].rearms++;
      g_b[bi].last_rearm_ts = g_now;
      g_cnt_rearm++;
     }
  }

//====================================================================
//  DRABINA RYNKOWA — engine.rs:8328 market_ladder_pass (Laddered)
//====================================================================
void MarketLadderPass()
  {
   if(In_MarketEntryMode != 2 || In_AutoLimit) return;
   if(!MarginesPozwala(In_MlMinDrabina)) return;
   if(EntryGateNaTick() != 0) return;
   double step = MarketStep();
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!ExitRiskAllowed(bi)) continue;
      if(EntryReviewBlocked(bi)) continue;
      if(!Alive(bi) || g_b[bi].is_limit || g_b[bi].nlv == 0) continue;
      bool zostal = false;
      for(int i = 0; i < g_b[bi].nlv; i++)
         if(!g_b[bi].lv_filled[i] && !g_b[bi].lv_cancelled[i]) { zostal = true; break; }
      if(!zostal) continue;
      int side = g_b[bi].side;
      double px = EntryPx(side);
      if(g_b[bi].has_sl)
        {
         bool przebity = (side == 0) ? (px <= g_b[bi].sl) : (px >= g_b[bi].sl);
         if(przebity) continue;
        }
      if(g_b[bi].has_last_entry)
        {
         if((g_b[bi].last_entry_px - px) * SideSign(side) < step) continue;
        }
      PlaceGrid(bi, true, 1);   // jeden szczebel na przejście
     }
  }

//====================================================================
//  RE-ENTRY — engine.rs:8393 (+ml_min_reentry, +kap reenter_max_small)
//====================================================================
double MarketStep()
  {
   double base = KapF((In_MarketEntryStep > 0.0) ? In_MarketEntryStep : 1.0,
                      In_MktStepSmall, In_MktStepSmallM);
   if(base <= 0.0) base = 1.0;
   if(In_PpmEnabled && In_PpmForMarket && In_Ppm > 0.0) return base / In_Ppm;
   return base;
  }

void ReentryPass()
  {
   if(!In_ReenterAfterTp) return;
   if(!MarginesPozwala(In_MlMinReentry)) return;
   int reenter_lim = KapU(In_ReenterMax, In_ReenterMaxSmall, In_ReenterMaxSmallM);
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!ExitRiskAllowed(bi)) continue;
      if(EntryReviewBlocked(bi)) continue;
      double step = MarketStep();
      double lot = LotSize();
      if(!Alive(bi)) continue;
      if(g_b[bi].tp_stage < In_ReenterMinTpStage) continue;
      if(In_ReenterStopAfterRf && g_b[bi].secured) continue;
      if(reenter_lim > 0 && g_b[bi].reentries >= reenter_lim) continue;
      if(In_ReenterMinRetS > 0.0 && g_b[bi].last_tp_ts != 0 &&
         g_now - g_b[bi].last_tp_ts < (long)(In_ReenterMinRetS * 1000.0)) continue;
      int side = g_b[bi].side;
      double px = EntryPx(side);
      if(px < g_b[bi].zone_lo - 1e-9 || px > g_b[bi].zone_hi + 1e-9) continue;
      if(g_b[bi].has_sl)
        {
         bool breached = (side == 0) ? (px <= g_b[bi].sl) : (px >= g_b[bi].sl);
         if(breached) continue;
        }
      if(g_b[bi].has_last_entry)
        {
         double moved = (g_b[bi].last_entry_px - px) * SideSign(side);
         if(moved < step) continue;
        }
      if(In_EntrySlDistLimit > 0.0 && g_b[bi].has_sl &&
         MathAbs(px - g_b[bi].sl) > In_EntrySlDistLimit) continue;
      double tp = 0; bool has_tp = false;
      if(In_TpSchedule == 0) { if(g_b[bi].ntp > 0) { tp = g_b[bi].tps[g_b[bi].ntp-1]; has_tp = true; } }
      else
        {
         int st = g_b[bi].tp_stage;
         if(st < g_b[bi].ntp) { tp = g_b[bi].tps[st]; has_tp = true; }
         else if(g_b[bi].ntp > 0) { tp = g_b[bi].tps[g_b[bi].ntp-1]; has_tp = true; }
        }
      bool   ma_cap = false;
      double cap = 0.0;
      if(In_ReenterRespectCap) cap = MarketRiskCap(bi, ma_cap);
      double skala = MarketRiskScale(lot, px, g_b[bi].sl, g_b[bi].has_sl, cap, ma_cap);
      double vol_re = WolumenZlecenia(MathMax(lot * skala, In_LotMin));
      if(ma_cap && g_b[bi].has_sl)
        {
         double ryzyko = MathAbs(px - g_b[bi].sl) * XAU_CONTRACT * vol_re;
         if(ryzyko > cap + 1e-9) continue;   // budzet wyczerpany
        }
      if(EntryGateNaTick() != 0) continue;   // bramka rachunku (pamięć na tick)
      ulong tk = 0;
      if(WyslijRynek(bi, -2, vol_re, g_b[bi].sl, g_b[bi].has_sl, tp, has_tp,
                     "B" + IntegerToString(g_b[bi].id) + "R", tk))
        {
         if(g_b[bi].npos < MAXTK)
           { g_b[bi].pos[g_b[bi].npos] = tk; g_b[bi].pos_lv[g_b[bi].npos] = -2; g_b[bi].npos++; }
         ZapiszWlasciciela(tk, bi);
         g_b[bi].reentries++;
         g_b[bi].last_entry_px = px; g_b[bi].has_last_entry = true;
         g_b[bi].had_positions = true;
         g_b[bi].state = ST_WORKING;   // engine.rs:8544 — bezwarunkowo Working
         g_cnt_order++;
        }
     }
  }

//====================================================================
//  REVERSAL-EXIT — engine.rs:8567 (zakres ORAZ oddanie przewagi)
//====================================================================
void RevExitSweep()
  {
   if(In_RevExitRange <= 0.0) return;
   double win = (In_RevExitWindowMin > 0.0) ? In_RevExitWindowMin : 60.0;
   long t0 = (long)(win * 60000.0);
   double lo = 1e18, hi = -1e18;
   int n = 0;
   for(int i = g_vh_n - 1; i >= 0; i--)
     {
      int idx = (g_vh_head + i) % MAXVH;
      if(g_now - g_vh_ts[idx] > t0) break;
      if(g_vh_px[idx] < lo) lo = g_vh_px[idx];
      if(g_vh_px[idx] > hi) hi = g_vh_px[idx];
      n++;
     }
   if(n < 10) return;
   double range = hi - lo;
   if(range < In_RevExitRange) return;
   double mid = MidPx();
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!ExitRiskAllowed(bi)) continue;
      if(!Alive(bi)) continue;
      for(int i = g_b[bi].npos - 1; i >= 0; i--)
        {
         ulong t = g_b[bi].pos[i];
         if(!PositionSelectByTicket(t)) continue;
         int side = (PositionGetInteger(POSITION_TYPE) == POSITION_TYPE_BUY) ? 0 : 1;
         double against = (side == 0) ? (hi - mid) : (mid - lo);
         if(against < In_RevExitSlope) continue;
         if(PozPunkty(t) < In_RevExitProfit) continue;
         // ŚWIADOMIE po rynku, mimo exit_via_limit (engine.rs:8616)
         g_powod_zamk = "REVEXIT";
         if(ZamknijPozycje(t)) g_cnt_revexit++;
        }
     }
  }

//====================================================================
//  REDUKCJA EKSPOZYCJI — engine.rs:7021 redukuj_ekspozycje
//====================================================================
void RedukujEkspozycje()
  {
   if(In_ExpoCapPct <= 0.0 && In_ExpoCapMlPct <= 0.0) return;
   long gap = (long)(MathMax(In_ExpoCapS, 0.0) * 1000.0);
   if(gap > 0 && g_now - g_last_expo < gap) return;
   g_last_expo = g_now;

   double eq = AccountInfoDouble(ACCOUNT_EQUITY);
   if(eq <= 0.0) return;
   double lev = (double)AccountInfoInteger(ACCOUNT_LEVERAGE);
   if(lev < 1.0) lev = 1.0;

   // ---- (1) WŁASNY STOP-OUT po poziomie marginesu (expo_cap_ml_pct) ----
   if(In_ExpoCapMlPct > 0.0)
     {
      bool zadzialalo = false;
      while(true)
        {
         double m = 0.0;
         for(int i = PositionsTotal() - 1; i >= 0; i--)
           {
            ulong t = PositionGetTicket(i);
            if(t == 0 || PositionGetInteger(POSITION_MAGIC) != In_Magic) continue;
            m += PositionGetDouble(POSITION_VOLUME) * XAU_CONTRACT * PositionGetDouble(POSITION_PRICE_OPEN) / lev;
           }
         if(m <= 0.0) break;
         double e = AccountInfoDouble(ACCOUNT_EQUITY);
         if(e / m * 100.0 >= In_ExpoCapMlPct) break;
         // najbardziej stratna — remis po numerze biletu (powtarzalność)
         ulong naj_t = 0; double naj_p = 1e18;
         for(int i = PositionsTotal() - 1; i >= 0; i--)
           {
            ulong t = PositionGetTicket(i);
            if(t == 0 || PositionGetInteger(POSITION_MAGIC) != In_Magic) continue;
            if(ExitTicketPending(t)) continue;
            double p = PozZysk(t);
            if(p < naj_p || (p == naj_p && t < naj_t)) { naj_p = p; naj_t = t; }
           }
         if(naj_t == 0) break;
         g_powod_zamk = "EXPOCAP_ML";
         if(!ZamknijPozycje(naj_t)) break;
         g_expo_poz_domk++;
         zadzialalo = true;
        }
      if(zadzialalo) g_expo_zdarzen++;
     }

   if(In_ExpoCapPct <= 0.0) return;
   // ---- (2) próg ekspozycji POTENCJALNEJ (pozycje+pendingi) ----
   eq = AccountInfoDouble(ACCOUNT_EQUITY);
   if(eq <= 0.0) return;
   double m_poz = 0.0, m_pend = 0.0;
   for(int i = PositionsTotal() - 1; i >= 0; i--)
     {
      ulong t = PositionGetTicket(i);
      if(t == 0 || PositionGetInteger(POSITION_MAGIC) != In_Magic) continue;
      m_poz += PositionGetDouble(POSITION_VOLUME) * XAU_CONTRACT * PositionGetDouble(POSITION_PRICE_OPEN) / lev;
     }
   for(int i = OrdersTotal() - 1; i >= 0; i--)
     {
      ulong t = OrderGetTicket(i);
      if(t == 0 || OrderGetInteger(ORDER_MAGIC) != In_Magic) continue;
      m_pend += OrderGetDouble(ORDER_VOLUME_CURRENT) * XAU_CONTRACT * OrderGetDouble(ORDER_PRICE_OPEN) / lev;
     }
   double razem = m_poz + m_pend;
   double limit = eq * In_ExpoCapPct / 100.0;
   if(razem <= limit) return;
   g_expo_zdarzen++;
   double nadmiar = razem - limit;
   double mid = MidPx();
   // (a) kasowanie pendingów najdalszych od mid — engine.rs:7129-7160:
   // JEDNO przejście po posortowanej liście ofiar; odmowa brokera = CONTINUE
   // do następnej (nie przerywa całej redukcji).
   ulong of_t[400]; double of_m[400]; int of_bi[400]; int nof = 0;
   for(int bi = 0; bi < g_nb && nof < 400; bi++)
     {
      if(!ExitRiskAllowed(bi)) continue;
      if(!Alive(bi)) continue;
      for(int i = 0; i < g_b[bi].npend && nof < 400; i++)
        {
         ulong t = g_b[bi].pend[i];
         if(!OrderSelect(t)) continue;
         of_t[nof] = t;
         of_m[nof] = OrderGetDouble(ORDER_VOLUME_CURRENT) * XAU_CONTRACT
                     * OrderGetDouble(ORDER_PRICE_OPEN) / lev;
         of_bi[nof] = bi;
         nof++;
        }
     }
   // sortowanie po odległości od mid malejąco (remisy po tickecie rosnąco)
   for(int a = 0; a < nof - 1; a++)
      for(int b2 = a + 1; b2 < nof; b2++)
        {
         double da = OrderSelect(of_t[a]) ? MathAbs(OrderGetDouble(ORDER_PRICE_OPEN) - mid) : -1;
         double db = OrderSelect(of_t[b2]) ? MathAbs(OrderGetDouble(ORDER_PRICE_OPEN) - mid) : -1;
         if(db > da || (db == da && of_t[b2] < of_t[a]))
           { ulong t = of_t[a]; of_t[a] = of_t[b2]; of_t[b2] = t;
             double m = of_m[a]; of_m[a] = of_m[b2]; of_m[b2] = m;
             int bb = of_bi[a]; of_bi[a] = of_bi[b2]; of_bi[b2] = bb; }
        }
   for(int k = 0; k < nof && nadmiar > 0.0; k++)
     {
      int bi = of_bi[k];
      int idx = -1;
      for(int i = 0; i < g_b[bi].npend; i++)
         if(g_b[bi].pend[i] == of_t[k]) { idx = i; break; }
      if(idx < 0) continue;
      if(!UsunPendingZnacz(bi, idx)) continue;   // odmowa → następna ofiara
      nadmiar -= of_m[k];
      g_expo_pend_skas++;
     }
   // (b) przy expo_cap_close: domykanie najbardziej podwodnych pozycji —
   // jedno przejście, odmowa nie odejmuje marginesu i idziemy dalej
   if(In_ExpoCapClose && nadmiar > 0.0)
     {
      ulong pz_t[400]; double pz_m[400]; double pz_p[400]; int npz = 0;
      for(int i = PositionsTotal() - 1; i >= 0 && npz < 400; i--)
        {
         ulong t = PositionGetTicket(i);
         if(t == 0 || PositionGetInteger(POSITION_MAGIC) != In_Magic) continue;
         if(ExitTicketPending(t)) continue;
         pz_t[npz] = t;
         pz_p[npz] = PozZysk(t);
         pz_m[npz] = PositionGetDouble(POSITION_VOLUME) * XAU_CONTRACT
                     * PositionGetDouble(POSITION_PRICE_OPEN) / lev;
         npz++;
        }
      for(int a = 0; a < npz - 1; a++)
         for(int b2 = a + 1; b2 < npz; b2++)
            if(pz_p[b2] < pz_p[a] || (pz_p[b2] == pz_p[a] && pz_t[b2] < pz_t[a]))
              { ulong t = pz_t[a]; pz_t[a] = pz_t[b2]; pz_t[b2] = t;
                double m = pz_m[a]; pz_m[a] = pz_m[b2]; pz_m[b2] = m;
                double p = pz_p[a]; pz_p[a] = pz_p[b2]; pz_p[b2] = p; }
      for(int k = 0; k < npz && nadmiar > 0.0; k++)
        {
         g_powod_zamk = "EXPOCAP";
         if(!ZamknijPozycje(pz_t[k])) continue;
         nadmiar -= pz_m[k];
         g_expo_poz_domk++;
        }
     }
  }

//====================================================================
//  RISK-FREE JAKO REGUŁA — engine.rs:7226 riskfree_pass
//====================================================================
void RiskfreePass()
  {
   if(!In_RfEnabled) return;
   // ---- limit trzymania runnera (zegar od UWOLNIENIA koszyka) ----
   if(In_RfRunnerMaxHoldM > 0.0)
     {
      long max_age = (long)(In_RfRunnerMaxHoldM * 60000.0);
      for(int bi = 0; bi < g_nb; bi++)
        {
         if(!ExitRiskAllowed(bi)) continue;
         if(!Alive(bi) || !g_b[bi].secured || g_b[bi].secured_ts == 0) continue;
         if(In_RfMaxHoldRuleOnly && !g_b[bi].secured_by_rule) continue;   // Z-10
         if(g_now - g_b[bi].secured_ts <= max_age) continue;
         g_powod_zamk = "RF_MAXHOLD";
         for(int i = g_b[bi].npos - 1; i >= 0; i--)
            if(ZamknijPozycje(g_b[bi].pos[i])) g_cnt_rf_maxhold++;
        }
     }

   if(In_RfTriggerUsd <= 0.0 && In_RfTriggerR <= 0.0) return;
   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!ExitRiskAllowed(bi)) continue;
      if(!Alive(bi) || g_b[bi].secured || g_b[bi].npos == 0) continue;
      // 1. czy próg zysku przekroczony?
      double otwarte = 0.0, ryzyko = 0.0;
      ulong zywe[MAXTK]; int n = 0;
      for(int i = 0; i < g_b[bi].npos; i++)
        {
         ulong t = g_b[bi].pos[i];
         if(!PositionSelectByTicket(t)) continue;
         zywe[n] = t; n++;
         otwarte += PozZysk(t);
         double psl = PositionGetDouble(POSITION_SL);
         if(psl != 0.0)
            ryzyko += MathAbs(PositionGetDouble(POSITION_PRICE_OPEN) - psl)
                      * XAU_CONTRACT * PositionGetDouble(POSITION_VOLUME);
        }
      if(n == 0) continue;
      double wynik = g_b[bi].realized + otwarte;
      bool prog_kwota = In_RfTriggerUsd > 0.0 && wynik >= In_RfTriggerUsd;
      bool prog_r = In_RfTriggerR > 0.0 && ryzyko > 0.0 && wynik >= ryzyko * In_RfTriggerR;
      if(!prog_kwota && !prog_r) continue;

      // 2. Stable gross-profit ranking (keep_units).
      NativeSortProfit(zywe,n,true);
      int ile_run = (int)MathMin(MathMax(In_RfKeepUnits, 1), n);

      // 3. uczciwość: zabankowana suma pokrywa straty
      double bank = 0.0;
      for(int i = ile_run; i < n; i++) bank += PozZysk(zywe[i]);
      if(g_b[bi].realized + bank < 0.0) continue;

      // 4. średnia ważona wejść CAŁEGO koszyka
      double suma_wol = 0.0, suma_px = 0.0;
      for(int i = 0; i < n; i++)
        {
         if(!PositionSelectByTicket(zywe[i])) continue;
         double v = PositionGetDouble(POSITION_VOLUME);
         suma_wol += v;
         suma_px += PositionGetDouble(POSITION_PRICE_OPEN) * v;
        }
      if(suma_wol <= 0.0) continue;
      double srednia = suma_px / suma_wol;
      int side = g_b[bi].side;

      // 5. domknięcie części bankującej
      double zabankowane = 0.0;
      for(int i = ile_run; i < n; i++)
        {
         double z = PozZysk(zywe[i]);
         g_powod_zamk = "RF_REGULA";
         if(ZamknijPozycje(zywe[i])) { zabankowane += z; g_cnt_rf_rule++; }
        }
      // basket_realized_broker_only=true: OdswiezBilety books confirmed deals.
      // Booking the command estimate here would count the same exit twice.

      // 6. runner: stop wg riskfree_runner_stop, cel wg riskfree_runner_target
      double be_koszyka = srednia + SideSign(side) * In_RfBeOffset;
      for(int i = 0; i < ile_run; i++)
        {
         ulong t = zywe[i];
         if(!PositionSelectByTicket(t)) continue;
         double wlasne = PositionGetDouble(POSITION_PRICE_OPEN);
         double cur_tp = PositionGetDouble(POSITION_TP);
         double cur_sl = PositionGetDouble(POSITION_SL);
         double be = be_koszyka;
         if(In_RfRunnerStop == 1) be = wlasne + SideSign(side) * In_RfBeOffset; // BeOwn
         double newtp = cur_tp; bool has_new = (cur_tp != 0.0);
         if(In_RfRunnerTarget2 == 1)
           {
            if(g_b[bi].ntp > 0) { newtp = g_b[bi].tps[g_b[bi].ntp - 1]; has_new = true; }
            else { has_new = false; newtp = 0.0; }   // pusta drabinka = bez celu
           }
         else if(In_RfRunnerTarget2 == 2) { has_new = false; newtp = 0.0; }
         else if(In_RfRunnerTarget2 == 3)
           {
            int st = g_b[bi].tp_stage;
            if(st < g_b[bi].ntp) { newtp = g_b[bi].tps[st]; has_new = true; }
            else if(g_b[bi].ntp > 0) { newtp = g_b[bi].tps[g_b[bi].ntp - 1]; has_new = true; }
            else { has_new = false; newtp = 0.0; }
           }
         // engine.rs:7432: stop tylko WPRZOD — obecny SL lepszy (ciasniejszy)
         // niz BE nie jest cofany; `lepszy` = cur_sl jest lepsza CENA niz be.
         bool wolno_be = (cur_sl == 0.0) || SideBetter(side, cur_sl, be);
         if(In_RfRunnerStop == 3)   // Off: runner bez SL (i bez wirtualnego!)
           {
            ModyfikujPozycje(t, 0, false, newtp, has_new);
            int ip0 = PsEnsure(t);
            if(ip0 >= 0) g_ps_vsl[ip0] = 0.0;   // engine.rs:7421 p.vsl = None
           }
         else if(In_RfRunnerStop == 2)   // TrailGap: start BE, potem zapadka
           {
            if(wolno_be && SlIsValid(side, be)) ModyfikujPozycje(t, be, true, newtp, has_new);
            else if(newtp != cur_tp) ModyfikujPozycje(t, cur_sl, cur_sl != 0.0, newtp, has_new);
            // engine.rs:7449: peak_pts = BIEZACY zysk punktowy (nie zero!) —
            // zapadka (a) rusza od razu z kotwica na chwili uwolnienia
            int ip = PsEnsure(t);
            if(ip >= 0)
              {
               double open2 = PositionGetDouble(POSITION_PRICE_OPEN);
               g_ps_peak[ip] = (ExitPx(side) - open2) * SideSign(side);
               g_ps_peak_ts[ip] = g_now;
              }
           }
         else                        // Be / BeOwn
           {
            if(wolno_be && SlIsValid(side, be)) ModyfikujPozycje(t, be, true, newtp, has_new);
            else if(newtp != cur_tp) ModyfikujPozycje(t, cur_sl, cur_sl != 0.0, newtp, has_new);
           }
         int ip2 = PsEnsure(t);
         if(ip2 >= 0) g_ps_isrunner[ip2] = !has_new;
        }
      g_b[bi].state = ST_RISKFREE;
      g_b[bi].secured = true;
      g_b[bi].secured_ts = g_now;
      g_b[bi].secured_by_rule = true;
     }
  }


//====================================================================
//  TRAILING — engine.rs:6309 trail_candidate + 6393 trail_z_parametrow
//====================================================================
struct AdaptiveMovement
  {
   long first_ts, last_ts;
   double first_px, last_px, path;
   int samples;
  };
bool g_adaptive_valid = false, g_adaptive_has_vol = false;
double g_adaptive_er = 0.0, g_adaptive_vol = 0.0;
void AdaptivePush(AdaptiveMovement &a, long ts, double px)
  {
   if(a.samples == 0) { a.first_ts = ts; a.first_px = px; }
   else a.path += MathAbs(px - a.last_px);
   a.last_ts = ts; a.last_px = px; a.samples++;
  }
bool AdaptivePathRate(const AdaptiveMovement &a, double &rate)
  {
   double seconds = (double)(a.last_ts - a.first_ts) / 1000.0;
   if(a.samples < 3 || seconds <= 0.0 || a.path <= 0.0) return false;
   rate = a.path / seconds;
   return true;
  }
void UpdateAdaptiveSnapshot()
  {
   g_adaptive_valid = false; g_adaptive_has_vol = false;
   if(!In_TrailAdaptiveEnabled) return;
   long er_ms = (long)(MathMax(In_TrailAdaptiveWindowS, 0.0) * 1000.0);
   long fast_ms = (long)(MathMax(In_TrailAdaptiveFastVolS, 0.0) * 1000.0);
   long slow_ms = (long)(MathMax(In_TrailAdaptiveSlowVolS, 0.0) * 1000.0);
   if(er_ms <= 0) return;
   long oldest = g_now - MathMax(er_ms, MathMax(fast_ms, slow_ms));
   int lo = 0, hi = g_vh_n;
   while(lo < hi)
     {
      int mid = (lo + hi) / 2;
      if(g_vh_ts[(g_vh_head + mid) % MAXVH] < oldest) lo = mid + 1;
      else hi = mid;
     }
   AdaptiveMovement er, fast, slow;
   ZeroMemory(er); ZeroMemory(fast); ZeroMemory(slow);
   double current = MidPx();
   int last = (g_vh_head + g_vh_n - 1 + MAXVH) % MAXVH;
   bool sampled = g_vh_n > 0 && g_vh_ts[last] == g_now && g_vh_px[last] == current;
   for(int i = lo; i < g_vh_n + (sampled ? 0 : 1); i++)
     {
      long ts = g_now; double px = current;
      if(i < g_vh_n) { int j = (g_vh_head + i) % MAXVH; ts = g_vh_ts[j]; px = g_vh_px[j]; }
      if(ts > g_now) continue;
      if(ts >= g_now - er_ms) AdaptivePush(er, ts, px);
      if(fast_ms > 0 && ts >= g_now - fast_ms) AdaptivePush(fast, ts, px);
      if(slow_ms > 0 && ts >= g_now - slow_ms) AdaptivePush(slow, ts, px);
     }
   if(er.samples < MathMax(In_TrailAdaptiveMinSamples, 2) || er.path <= 1e-12) return;
   g_adaptive_er = MathMax(-1.0, MathMin(1.0, (er.last_px - er.first_px) / er.path));
   double a = 0.0, b = 0.0;
   if(AdaptivePathRate(fast, a) && AdaptivePathRate(slow, b) && b > 1e-12)
     { g_adaptive_has_vol = true; g_adaptive_vol = a / b; }
   g_adaptive_valid = true;
  }
double AdaptiveTrailGap(int side, double peak, bool runner, double base_gap)
  {
   if(!g_adaptive_valid || !In_TrailAdaptiveEnabled
      || (In_TrailAdaptiveRunnersOnly && !runner)
      || peak < MathMax(In_TrailAdaptiveMinPeak, 0.0)) return base_gap;
   double signed_er = g_adaptive_er * SideSign(side);
   double trend = MathMin(1.0, MathAbs(In_TrailAdaptiveTrendEr));
   double reversal = MathMin(1.0, MathAbs(In_TrailAdaptiveReversalEr));
   double mult = signed_er >= trend ? In_TrailAdaptiveTrendGapMult
      : (signed_er <= -reversal ? In_TrailAdaptiveReversalGapMult : In_TrailAdaptiveChopGapMult);
   if(!MathIsValidNumber(mult)) mult = 1.0;
   mult = MathMax(mult, 0.0);
   if(In_TrailAdaptiveVolRatio > 0.0 && g_adaptive_has_vol && g_adaptive_vol >= In_TrailAdaptiveVolRatio)
     {
      double vm = signed_er >= 0.0 ? In_TrailAdaptiveVolFavorableMult : In_TrailAdaptiveVolAdverseMult;
      if(MathIsValidNumber(vm)) mult *= MathMax(vm, 0.0);
     }
   double gap = MathMax(base_gap, 0.0) * mult;
   double lower = MathMax(In_TrailAdaptiveMinGap, 0.0);
   if(lower > 0.0) gap = MathMax(gap, lower);
   if(In_TrailAdaptiveMaxGap > 0.0) gap = MathMin(gap, MathMax(In_TrailAdaptiveMaxGap, lower));
   return gap;
  }
// parse_tiers "prog:blokada,prog:blokada" — najwyższy osiągnięty próg wygrywa
bool TrailTiers(string tiers, double peak, double &keep)
  {
   string p[];
   int k = StringSplit(tiers, ',', p);
   bool jest = false;
   for(int i = 0; i < k; i++)
     {
      string ab[];
      if(StringSplit(p[i], ':', ab) < 2) continue;
      double thr = StringToDouble(ab[0]);
      double kp = StringToDouble(ab[1]);
      if(peak >= thr) { keep = kp; jest = true; }
     }
   return jest;
  }

// trail_z_parametrow: kandydat SL z jawnych parametrów (0=Off..5=Chandelier)
bool TrailZParametrow(int side, double open, double peak, int mode,
                      double start, double gap, double lock, string tiers, bool runner, double &out)
  {
   if(mode == 0 || peak < start) return false;
   int s = SideSign(side);
   if(mode == 1) { out = ExitPx(side) - s * AdaptiveTrailGap(side, peak, runner, gap); return true; }
   if(mode == 2) { out = open + s * peak * lock / 100.0; return true; }        // LockPct
   if(mode == 3)                                                               // Tiered
     {
      double keep;
      if(!TrailTiers(tiers, peak, keep)) return false;
      out = open + s * keep;
      return true;
     }
   if(mode == 4 || mode == 5)                                                  // Atr / Chandelier
     {
      if(In_TrailAtrMult <= 0.0) return false;
      double atr;
      if(!AtrProxy(atr)) return false;
      double luka = AdaptiveTrailGap(side, peak, runner, In_TrailAtrMult * atr);
      if(mode == 4) out = ExitPx(side) - s * luka;             // kotwica: cena bieżąca
      else out = open + s * peak - s * luka;                   // kotwica: ekstremum
      return true;
     }
   return false;
  }

// runnerzy wg głębokości — engine.rs:6283 (N najlepszych wejść koszyka)
bool JestRunneremWgGlebokosci(int bi, ulong t)
  {
   if(!In_TrailSplit || !In_TrailRunByDepth) return false;
   int n = MathMax(In_TrailRunnersN, 1);
   ulong tk[MAXTK]; double op[MAXTK]; int m = 0;
   for(int i = 0; i < g_b[bi].npos; i++)
     {
      ulong x = g_b[bi].pos[i];
      if(!PositionSelectByTicket(x)) continue;
      tk[m] = x; op[m] = PositionGetDouble(POSITION_PRICE_OPEN); m++;
     }
   NativeStableSortTickets(tk,op,m,g_b[bi].side!=0);
   for(int i = 0; i < MathMin(n, m); i++) if(tk[i] == t) return true;
   return false;
  }

//  Czy pozycja jest RUNNEREM — ta sama definicja, ktorej uzywa pass
//  trailingu: przy `trail_runners_by_depth` decyduje glebokosc, inaczej
//  znacznik `is_runner` (pozycja bez celu). Wspoldzielimy ja z blokiem S/R,
//  zeby dwie rodziny nie rozjechaly sie w definicji tego samego slowa.
bool JestRunnerem(int bi, ulong t)
  {
   if(In_TrailRunByDepth) return JestRunneremWgGlebokosci(bi, t);
   int ipr = PsIdx(t);
   return (ipr >= 0) ? g_ps_isrunner[ipr] : false;
  }

//  Czy pozycja nalezy do WARSTWY TP3 LUB WYZSZEJ — engine.rs
//  `sr_warstwa_tp3_up`. Pozycja BEZ celu liczy sie jako nalezaca (runner).
bool SrWarstwaTp3Up(int bi, double cur_tp)
  {
   if(cur_tp == 0.0) return true;
   if(bi < 0) return false;
   for(int i = 0; i < g_b[bi].ntp; i++)
      if(MathAbs(g_b[bi].tps[i] - cur_tp) < 0.01) return (i >= 2);
   return false;
  }

// pełny kandydat trailingu (4 warstwy pierwszeństwa) — engine.rs:6309
bool TrailCandidate(int bi, ulong t, int side, double open, double peak, double &out)
  {
   // (a) runner koszyka ZABEZPIECZONEGO przy riskfree_enabled + TrailGap
   if(In_RfEnabled && In_RfRunnerStop == 2 && g_b[bi].secured)
     {
      double luz = (In_RfRunnerGap > 0.0) ? In_RfRunnerGap : 25.0;
      luz = AdaptiveTrailGap(side, peak, true, luz);
      if(peak <= 0.0) return false;
      out = open + SideSign(side) * (peak - luz);
      return true;
     }
   // (b) risk_free_trail: koszyk zabezpieczony komunikatem prowadzi runnera
   //     zapadką runnerową, gdy zwykły trailing wyłączony (engine.rs:6350)
   if(In_RiskFreeTrail && !In_RfEnabled && g_b[bi].secured
      && In_TrailMode == 0 && In_TrailRunnerMode != 0)
      return TrailZParametrow(side, open, peak, In_TrailRunnerMode,
                              In_TrailRunnerStart, In_TrailRunnerGap,
                              In_TrailRunnerLockPct, In_TrailRunnerTiers, true, out);
   // (c) split runnerowy / (d) tryb bazowy
   // is_runner ustawiaja WYLACZNIE sciezki risk-free (engine.rs:4565/7419/7442)
   // — pozycja bez TP, ktora nie przeszla przez RF, NIE jest runnerem.
   // Wyjatek: FEATURE In_RunnerBezCelu (domyslnie OFF) rozszerza definicje
   // na kazda pozycje bez celu — patrz RAPORTEKSPERT §7.
   bool runner_teraz;
   if(In_TrailRunByDepth) runner_teraz = JestRunneremWgGlebokosci(bi, t);
   else
     {
      int ip = PsIdx(t);
      runner_teraz = (ip >= 0 && g_ps_isrunner[ip]);
      if(!runner_teraz && In_RunnerBezCelu)
        {
         if(PositionSelectByTicket(t) && PositionGetDouble(POSITION_TP) == 0.0)
            runner_teraz = true;
        }
     }
   bool runner = In_TrailSplit && runner_teraz;
   if(runner)
      return TrailZParametrow(side, open, peak, In_TrailRunnerMode,
                              In_TrailRunnerStart, In_TrailRunnerGap,
                              In_TrailRunnerLockPct, In_TrailRunnerTiers, runner_teraz, out);
   return TrailZParametrow(side, open, peak, In_TrailMode,
                           In_TrailStart, In_TrailGap, In_TrailLockPct, In_TrailTiers, runner_teraz, out);
  }

//====================================================================
//  MANAGE POSITIONS — engine.rs:5977 (pełne)
//====================================================================
//====================================================================
//  TRAILING S/R PO STRUKTURZE — port z engine.rs (rodzina `trail_sr_*`)
//
//  Swieca struktury budowana po MID (mierzymy strukture, nie egzekucje).
//  Swing = ekstremum z `n` swiecami OSTRO gorszymi po OBU stronach; staje
//  sie widzialny dopiero przy zamknieciu n-tej swiecy PO szczytowej, wiec
//  zero podgladania przyszlosci jest konstrukcyjne.
//
//  Przy `In_TrailSrEnabled = false` NIKT tego nie dotyka.
//====================================================================
#define SR_MAXW   64
#define SR_MAXSW 4096

long   g_sr_kub  = LONG_MIN;
double g_sr_hi   = 0.0, g_sr_lo = 0.0;
double g_sr_ch[SR_MAXW], g_sr_cl[SR_MAXW];
int    g_sr_n    = 0;
double g_sr_swl[SR_MAXSW];  long g_sr_swl_t[SR_MAXSW];  int g_sr_swl_n = 0;
double g_sr_swh[SR_MAXSW];  long g_sr_swh_t[SR_MAXSW];  int g_sr_swh_n = 0;
bool   g_sr_nowa = false;

void SrDopiszSwing(double &lvl[], long &tt[], int &n, double v, long ts)
  {
   if(n >= SR_MAXSW)
     { for(int i = 1; i < n; i++) { lvl[i-1] = lvl[i]; tt[i-1] = tt[i]; } n--; }
   lvl[n] = v; tt[n] = ts; n++;
  }

void SrPrzytnij(double &lvl[], long &tt[], int &n, long horyzont)
  {
   int w = 0;
   for(int i = 0; i < n; i++)
      if(tt[i] >= horyzont) { lvl[w] = lvl[i]; tt[w] = tt[i]; w++; }
   n = w;
  }

void SrZamknijSwiece(long ts)
  {
   int fn_ = MathMax(In_TrailSrFractalN, 1);
   int okno = 2 * fn_ + 1;
   if(okno > SR_MAXW) okno = SR_MAXW;
   if(g_sr_n >= okno)
     { for(int i = 1; i < g_sr_n; i++) { g_sr_ch[i-1] = g_sr_ch[i]; g_sr_cl[i-1] = g_sr_cl[i]; } g_sr_n--; }
   g_sr_ch[g_sr_n] = g_sr_hi; g_sr_cl[g_sr_n] = g_sr_lo; g_sr_n++;

   if(g_sr_n == okno)
     {
      double sh = g_sr_ch[fn_], sl = g_sr_cl[fn_];
      bool low_ok = true, high_ok = true;
      for(int i = 0; i < g_sr_n; i++)
        {
         if(i == fn_) continue;
         if(g_sr_cl[i] <= sl) low_ok  = false;
         if(g_sr_ch[i] >= sh) high_ok = false;
        }
      if(low_ok)  SrDopiszSwing(g_sr_swl, g_sr_swl_t, g_sr_swl_n, sl, ts);
      if(high_ok) SrDopiszSwing(g_sr_swh, g_sr_swh_t, g_sr_swh_n, sh, ts);
     }
   long horyzont = ts - 3600000 * (long)MathMax(In_TrailSrWindowH, 1);
   SrPrzytnij(g_sr_swl, g_sr_swl_t, g_sr_swl_n, horyzont);
   SrPrzytnij(g_sr_swh, g_sr_swh_t, g_sr_swh_n, horyzont);
   g_sr_nowa = true;
  }

void SrNaTicku(long ts, double mid)
  {
   if(!In_TrailSrEnabled) return;
   long kub = (long)MathFloor((double)ts / (60000.0 * MathMax(In_TrailSrTfMin, 1)));
   if(g_sr_kub == LONG_MIN) { g_sr_kub = kub; g_sr_hi = mid; g_sr_lo = mid; return; }
   if(kub != g_sr_kub)
     { SrZamknijSwiece(ts); g_sr_kub = kub; g_sr_hi = mid; g_sr_lo = mid; }
   else
     { if(mid > g_sr_hi) g_sr_hi = mid; if(mid < g_sr_lo) g_sr_lo = mid; }
  }

//  Najblizszy od ceny poziom po WLASCIWEJ stronie, z oddechem od ceny
//  i od nastepnego nieodhaczonego celu sygnalu. `false` = brak kandydata.
bool SrKandydat(int side, double mid, double next_tp, bool ma_next_tp, long ts, double &out)
  {
   int n = (side == 0) ? g_sr_swl_n : g_sr_swh_n;
   double naj = 0.0; bool jest = false; double najd = 0.0;
   for(int i = 0; i < n; i++)
     {
      double lvl = (side == 0) ? g_sr_swl[i] : g_sr_swh[i];
      long   tt  = (side == 0) ? g_sr_swl_t[i] : g_sr_swh_t[i];
      if(tt > ts) continue;                                  // zero lookahead
      if(side == 0 ? !(lvl < mid) : !(lvl > mid)) continue;   // wlasciwa strona
      if(In_TrailSrMinDistP > 0.0 && MathAbs(mid - lvl) < In_TrailSrMinDistP) continue;
      if(ma_next_tp && MathAbs(lvl - next_tp) < In_TrailSrMinDistTp) continue;
      double d = MathAbs(mid - lvl);
      if(!jest || d < najd) { naj = lvl; najd = d; jest = true; }
     }
   if(jest) out = naj;
   return jest;
  }

void ManagePositions()
  {
   UpdateAdaptiveSnapshot();
   bool sr_swieca = In_TrailSrEnabled && g_sr_nowa;
   g_sr_nowa = false;
   // kolejka wyjść PRZED regułami
   SweepQueuedExits();
   // mediana spreadu (512 próbek) — tylko gdy ktoś jej używa
   if(In_ExitSpreadMult > 0.0)
     {
      g_spread_buf[g_spread_n % 512] = g_ask - g_bid;
      g_spread_n++;
      if(g_spread_n % 512 == 0)
        {
         double v[512];
         ArrayCopy(v, g_spread_buf);
         ArraySort(v);
         g_spread_med = v[256];
        }
     }
   bool vsl_due = (In_VslEvalS <= 0.0) || (g_now - g_last_vsl_eval >= (long)(In_VslEvalS * 1000.0));
   if(vsl_due) g_last_vsl_eval = g_now;

   // ---- CEL NA POZIOMIE KOSZYKA (basket_target_usd) ----
   if(In_BasketTargetUsd > 0.0)
      for(int bi = 0; bi < g_nb; bi++)
        {
         if(!ExitRiskAllowed(bi)) continue;
         if(!Alive(bi) || g_b[bi].npos == 0) continue;
         double zysk = 0.0;
         for(int i = 0; i < g_b[bi].npos; i++) zysk += PozZysk(g_b[bi].pos[i]);
         if(zysk < In_BasketTargetUsd) continue;
         for(int i = g_b[bi].npos - 1; i >= 0; i--)
            CloseOrQueue(g_b[bi].pos[i], "HARVEST_KOSZYK");
        }

   for(int bi = 0; bi < g_nb; bi++)
     {
      if(!ExitRiskAllowed(bi)) continue;
      if(!Alive(bi)) continue;
      int side = g_b[bi].side;
      for(int i = g_b[bi].npos - 1; i >= 0; i--)
        {
         ulong t = g_b[bi].pos[i];
         if(!PositionSelectByTicket(t)) continue;
         int ip = PsEnsure(t);
         double open = PositionGetDouble(POSITION_PRICE_OPEN);
         double cur_sl = PositionGetDouble(POSITION_SL);
         double cur_tp = PositionGetDouble(POSITION_TP);
         double pts = (ExitPx(side) - open) * SideSign(side);
         double peak = (ip >= 0) ? g_ps_peak[ip] : pts;
         long   peak_ts = (ip >= 0) ? g_ps_peak_ts[ip] : g_now;
         long   open_ts = (long)PositionGetInteger(POSITION_TIME_MSC);

         // ---- wirtualny SL ----
         if(In_VirtualSl && vsl_due && ip >= 0 && g_ps_vsl[ip] != 0.0)
           {
            double v = g_ps_vsl[ip];
            bool hit = (side == 0) ? (g_bid <= v) : (g_ask >= v);
            if(hit)
              {
               g_powod_zamk = "VSL";
               if(ZamknijPozycje(t)) { g_cnt_vsl++; continue; }
              }
           }
         // pozycja z zakolejkowanym wyjściem czeka — nie ruszają jej inne reguły
         if(In_ExitViaLimit && JestWKolejceWyjscia(t)) continue;

         // ---- wspólne bramki reguł uznaniowych ----
         double wiek_min = (double)(g_now - open_ts) / 60000.0;
         bool za_swieza = In_ExitMinHoldMin > 0.0 && wiek_min < In_ExitMinHoldMin;
         bool za_maly_zysk = In_ExitMinProfit > 0.0 && pts < In_ExitMinProfit;
         bool po_tp_hit = In_HoldAfterTpHitMin > 0.0 && g_last_tp_hit_ts > 0
            && (double)(g_now - g_last_tp_hit_ts) / 60000.0 < In_HoldAfterTpHitMin;
         bool reguly_wolne = !za_swieza && !za_maly_zysk && !po_tp_hit;

         if(reguly_wolne && pts > 0.0)
           {
            // 1. wielokrotność ryzyka
            if(In_ExitRMultiple > 0.0 && cur_sl != 0.0)
              {
               double ryzyko = MathAbs(open - cur_sl);
               if(ryzyko > 1e-9 && pts >= ryzyko * In_ExitRMultiple)
                 { CloseOrQueue(t, "HARVEST_R"); g_cnt_harvest++; continue; }
              }
            // 2. okrągły poziom PRZED nami
            if(In_ExitRoundDist > 0.0 && In_ExitRoundStep > 0.0)
              {
               double cena = ExitPx(side);
               double najbl = MathRound(cena / In_ExitRoundStep) * In_ExitRoundStep;
               bool przed = (side == 0) ? (najbl >= cena) : (najbl <= cena);
               if(przed && MathAbs(najbl - cena) <= In_ExitRoundDist)
                 { CloseOrQueue(t, "HARVEST_ROUND"); g_cnt_harvest++; continue; }
              }
            // 3. spread się rozjechał
            if(In_ExitSpreadMult > 0.0 && g_spread_med > 0.0)
              {
               if((g_ask - g_bid) >= g_spread_med * In_ExitSpreadMult)
                 { CloseOrQueue(t, "HARVEST_SPREAD"); g_cnt_harvest++; continue; }
              }
           }

         // ---- SMART EXIT ----
         if(In_SmartExit && pts > 0.0 && reguly_wolne)
           {
            bool trzymaj_bo_siatka = false;
            if(In_SmartExitHoldPend > 0.0)
              {
               int ile = 0;
               for(int o = OrdersTotal() - 1; o >= 0; o--)
                 {
                  ulong ot = OrderGetTicket(o);
                  if(ot == 0 || OrderGetInteger(ORDER_MAGIC) != In_Magic) continue;
                  long typ = OrderGetInteger(ORDER_TYPE);
                  int oside = (typ == ORDER_TYPE_BUY_LIMIT || typ == ORDER_TYPE_BUY_STOP) ? 0 : 1;
                  if(oside != side) continue;
                  if(In_SmartExitPendScope == 0)
                    {
                     // SameBasket: sprawdź, czy to zlecenie naszego koszyka
                     bool nasz = false;
                     for(int k2 = 0; k2 < g_b[bi].npend; k2++)
                        if(g_b[bi].pend[k2] == ot) { nasz = true; break; }
                     if(!nasz) continue;
                    }
                  double d = (side == 0) ? (g_bid - OrderGetDouble(ORDER_PRICE_OPEN))
                                         : (OrderGetDouble(ORDER_PRICE_OPEN) - g_ask);
                  if(d > In_SmartExitPendDist && d <= In_SmartExitHoldPend) ile++;
                 }
               trzymaj_bo_siatka = (ile >= MathMax(In_SmartExitMinPend, 1));
              }
            if(!trzymaj_bo_siatka)
              {
               if(In_SmartExitTake > 0.0 && pts >= In_SmartExitTake)
                 { CloseOrQueue(t, "SMARTEXIT_TAKE"); g_cnt_smartexit++; continue; }
               if(In_SmartExitGiveback > 0.0 && peak >= In_SmartExitMinPeak
                  && peak - pts >= peak * In_SmartExitGiveback)
                 { CloseOrQueue(t, "SMARTEXIT_GIVEBACK"); g_cnt_smartexit++; continue; }
               if(In_SmartExitDropSpd > 0.0)
                 {
                  long okno = (long)(In_SmartExitSpdWinS * 1000.0);
                  long wiek = g_now - peak_ts;
                  if(wiek > 0 && wiek <= okno)
                    {
                     double spadek = peak - pts;
                     double na_min = spadek / MathMax((double)wiek / 60000.0, 1e-6);
                     if(na_min >= In_SmartExitDropSpd)
                       { CloseOrQueue(t, "SMARTEXIT_SPEED"); g_cnt_smartexit++; continue; }
                    }
                 }
              }
           }

         // ---- harvest: cofnięcie o % szczytu ----
         if(In_HarvestRetracePct > 0.0 && peak >= In_HarvestStart)
           {
            if(peak - pts >= peak * In_HarvestRetracePct / 100.0)
              { CloseOrQueue(t, "HARVEST"); g_cnt_harvest++; continue; }
           }

         // ---- out-at-entry po czasie ----
         if(In_OaeTimeoutMin > 0.0)
           {
            if(wiek_min >= In_OaeTimeoutMin && pts < In_OaeProfitMin)
              {
               g_powod_zamk = "OAE_TIMEOUT";
               if(ZamknijPozycje(t)) { g_cnt_oae_timeout++; continue; }
              }
           }

         // ---- stagnacja dwuczłonowa ----
         double stag_min = (double)(g_now - peak_ts) / 60000.0;
         bool s1 = In_StaleTakeMin > 0.0 && stag_min >= In_StaleTakeMin && pts >= In_StaleTakeProfit;
         bool s2 = In_StaleTakeMin2 > 0.0 && stag_min >= In_StaleTakeMin2 && pts >= In_StaleTakeProfit2;
         if(s1 || s2) { CloseOrQueue(t, "STALE"); g_cnt_stale++; continue; }

         // ---- BE-lock (Z-4: brak SL = wolno stawiać) ----
         if(In_BeLockPts > 0.0 && pts >= In_BeLockPts)
           {
            double be = open + SideSign(side) * In_BeOffset;
            bool luzuje = (cur_sl != 0.0) && SideBetter(side, be, cur_sl);
            if(!luzuje && SlIsValid(side, be))
              { if(ModyfikujPozycje(t, be, true, cur_tp, cur_tp != 0.0)) g_cnt_belock++; }
           }

         // ---- TRAILING ----
         double cand;
         if(TrailCandidate(bi, t, side, open, peak, cand))
           {
            bool improves = (cur_sl == 0.0) || !SideBetter(side, cand, cur_sl);
            bool above_be = !SideBetter(side, cand, open);
            if(improves && above_be)
              {
               double min_d = MathMax(In_TrailMinDist, g_stops);
               double safe = (side == 0) ? MathMin(cand, g_bid - min_d)
                                         : MathMax(cand, g_ask + min_d);
               bool still = (cur_sl == 0.0) || !SideBetter(side, safe, cur_sl);
               if(still && SlIsValid(side, safe))
                 {
                  if(In_VirtualSl && !In_VslOnlyWhenRej)
                    { if(ip >= 0) g_ps_vsl[ip] = cand; }
                  else
                    { if(ModyfikujPozycje(t, safe, true, cur_tp, cur_tp != 0.0)) g_cnt_trail_mod++; }
                 }
              }
           }

         // ---- TRAILING S/R PO STRUKTURZE (engine.rs, blok osobny) ----
         //
         // OSOBNY blok, nie rozszerzenie TrailCandidate: tamta funkcja niesie
         // semantyke gap/lock_pct/tiers i pulapke In_TrailRunnerMode. Definicje
         // runnera WSPOLDZIELIMY z passem wyzej.
         if(sr_swieca)
           {
            bool w_zakresie = false;
            if(In_TrailSrScope == 2)      w_zakresie = true;              // All
            else if(In_TrailSrScope == 0) w_zakresie = JestRunnerem(bi, t); // Runner
            else                          w_zakresie = SrWarstwaTp3Up(bi, cur_tp);

            int    stage    = (bi >= 0) ? g_b[bi].tp_stage : 0;
            bool   ma_ntp   = (bi >= 0 && stage < g_b[bi].ntp);
            double next_tp  = ma_ntp ? g_b[bi].tps[stage] : 0.0;

            bool aktywna = false;
            if(In_TrailSrActiv == 0)      aktywna = true;
            else if(In_TrailSrActiv == 1) aktywna = (pts >= In_TrailSrMinGain);
            else if(In_TrailSrActiv == 2) aktywna = (stage >= 1);
            else if(In_TrailSrActiv == 3) aktywna = (stage >= 2);
            else if(In_TrailSrActiv == 4) aktywna = (stage >= 3);

            double poziom;
            if(w_zakresie && aktywna
               && SrKandydat(side, (g_bid + g_ask) * 0.5, next_tp, ma_ntp, g_now, poziom))
              {
               double sl_prop = poziom - SideSign(side) * In_TrailSrOffset;
               //  WSPOLNA ZAPADKA — stop czytamy z ZYWEJ pozycji, nie ze
               //  zdjecia sprzed petli. Blok BE-lock i trailing WYZEJ mogly
               //  juz w TYM SAMYM ticku podbic stop; porownanie ze starym
               //  `cur_sl` przepuscilo by propozycje GORSZA od aktualnej.
               //  Silnik robi dokladnie to samo (engine.rs: „SL czytamy
               //  z ZYWEJ pozycji, nie ze snapshotu").
               double zywy_sl = cur_sl, zywy_tp = cur_tp;
               if(PositionSelectByTicket(t))
                 { zywy_sl = PositionGetDouble(POSITION_SL);
                   zywy_tp = PositionGetDouble(POSITION_TP); }
               // ZAPADKA: tylko w strone zysku, ale BEZ wymogu „powyzej BE" —
               // S/R moze podbic stop z -5 $ na -2 $ i to jest cenne.
               bool poprawia = (zywy_sl == 0.0)
                               || (!SideBetter(side, sl_prop, zywy_sl)
                                   && MathAbs(sl_prop - zywy_sl) > 1e-9);
               // Odmowa stops_level = POMIN te swiece, zadnego przycinania:
               // stop ma lezec NA STRUKTURZE, przyciety to inny mechanizm.
               if(poprawia && SlIsValid(side, sl_prop))
                 {
                  if(In_VirtualSl && !In_VslOnlyWhenRej)
                    {
                     //  ZAPADKA takze tutaj — fallback na wirtualny stop BEZ
                     //  zapadki byl bledem (engine.rs, „lekcja audytu C").
                     if(ip >= 0)
                       {
                        double v = g_ps_vsl[ip];
                        bool vpop = (v == 0.0)
                                    || (!SideBetter(side, sl_prop, v)
                                        && MathAbs(sl_prop - v) > 1e-9);
                        if(vpop) g_ps_vsl[ip] = sl_prop;
                       }
                    }
                  else
                    { if(ModyfikujPozycje(t, sl_prop, true, zywy_tp, zywy_tp != 0.0)) g_cnt_trail_mod++; }
                 }
              }
           }
        }
     }
  }

//====================================================================
//  STRAŻNICY — engine.rs:5645 check_guards (+Z-2 zatrzymaj_dobe)
//====================================================================
void ZatrzymajDobe()
  {
   long d = DayOf(g_now);
   if(g_day_stop == d) return;
   g_day_stop = d;
  }

void CheckGuards()
  {
   if(StringLen(g_halted) > 0) return;
   double eq = AccountInfoDouble(ACCOUNT_EQUITY);
   double lot = LotSize();
   double scale = In_UsdScaleWithLot ? MathMax(lot / 0.01, 1.0) : 1.0;
   double base = (In_DdGuardScope == 0) ? g_day_peak_eq : g_peak_equity;
   double dd = base - eq;
   double dd_pct = dd / MathMax(base, 1.0) * 100.0;

   if(In_MaxDdPct > 0.0 && dd_pct >= In_MaxDdPct)
     {
      g_powod_zamk = "MAXDD";
      CloseEverything();
      g_halted = StringFormat("MAX DRAWDOWN %.1f%% >= %.1f%%", dd_pct, In_MaxDdPct);
      return;
     }
   if(In_MaxDdUsd > 0.0 && dd >= In_MaxDdUsd * scale)
     {
      g_powod_zamk = "MAXDD";
      CloseEverything();
      g_halted = StringFormat("MAX DRAWDOWN %.2f$ >= %.2f$", dd, In_MaxDdUsd * scale);
      return;
     }
   if(In_DayTrailStopUsd > 0.0)
     {
      double d = g_day_peak_eq - eq;
      if(d >= In_DayTrailStopUsd * scale && (LiczPozycje() > 0 || LiczZlecenia() > 0))
        { g_powod_zamk = "DAYTRAIL"; CloseEverything(); }
     }
   if(In_DayTargetUsd > 0.0 && In_DayTargetClose)
     {
      double scale2 = In_DayTargetScaleLot ? MathMax(lot / 0.01, 1.0) : scale;
      double today = eq - g_day_start_eq;
      if(today >= In_DayTargetUsd * scale2 && (LiczPozycje() > 0 || LiczZlecenia() > 0))
        { g_powod_zamk = "DAYTARGET"; CloseEverything(); }
     }
   if(DayPctGuardActive() && In_DayTargetPct > 0.0 && In_DayTargetClose)
     {
      double prog = MathMax(g_day_start_eq, 1.0) * In_DayTargetPct / 100.0;
      if(eq - g_day_start_eq >= prog && (LiczPozycje() > 0 || LiczZlecenia() > 0))
        { g_powod_zamk = "DAYTARGET"; CloseEverything(); }
     }
   if(DayPctGuardActive() && In_DayTrailStopPct > 0.0)
     {
      double szczyt = MathMax(g_day_peak_eq, 1.0);
      double zysk_szczytu = g_day_peak_eq - g_day_start_eq;
      bool uzbrojony = In_DayTrailArmPct <= 0.0
         || zysk_szczytu >= MathMax(g_day_start_eq, 1.0) * In_DayTrailArmPct / 100.0;
      if(In_DayTrailBasis == 1)
        {
         szczyt = zysk_szczytu;
         if(zysk_szczytu <= 0.0) uzbrojony = false;
        }
      double oddane = g_day_peak_eq - eq;
      double prog = szczyt * In_DayTrailStopPct / 100.0;
      if(uzbrojony && oddane >= prog)
        {
         ZatrzymajDobe();   // Z-2: dobę zamykamy NIEZALEŻNIE od pozycji
         if(LiczPozycje() > 0 || LiczZlecenia() > 0) { g_powod_zamk = "DAYTRAIL"; CloseEverything(); }
        }
     }
   int hour = HourOf(g_now);
   if(In_EodFlatHour > 0.0 && hour == (int)In_EodFlatHour)
     {
      ZatrzymajDobe();
      if(LiczPozycje() > 0 || LiczZlecenia() > 0) { g_powod_zamk = "EODFLAT"; CloseEverything(); }
     }
   if(In_FlatWeekend)
     {
      int wd = WeekdayOf(g_now);
      if(wd == 4 && hour >= (int)In_FlatWeekendHour)
        {
         ZatrzymajDobe();
         if(LiczPozycje() > 0 || LiczZlecenia() > 0) { g_powod_zamk = "WEEKEND"; CloseEverything(); }
        }
     }
  }

//====================================================================
//  ROLKA DOBY — engine.rs:5298 (reset day_*, zdjęcie blokady poza Lifetime)
//====================================================================
void RolkaDoby()
  {
   long d = DayOf(g_now);
   if(d == g_day) return;
   g_day = d;
   double eq = AccountInfoDouble(ACCOUNT_EQUITY);
   g_day_start_eq = eq;
   g_day_peak_eq = eq;
   // nowa doba zeruje licznik SL HIT i zdejmuje hamulec — engine.rs:5620-5622.
   // Granica doby zgodna z silnikiem: session_offset()=0 (settings.rs:3780).
   g_slhit_dnia = 0;
   g_slhit_pauza_do = LONG_MIN;
   // UWAGA (weryfikacja portu): silnik NIGDY nie resetuje peak_equity na dobie.
   // LifetimePeakDailyReset = baza DD od szczytu WSZECH CZASÓW + codzienne
   // zdjęcie blokady (strażnik zapala się ponownie — „blokada dożywotnia na
   // raty", settings.rs:4197). Reset szczytu degenerowałby scope=2 do Daily.
   // zdjęcie blokady WYŁĄCZNIE rodzaju MAX DRAWDOWN, poza Lifetime (engine.rs:5327)
   if(StringLen(g_halted) > 0 && In_DdGuardScope != 1
      && StringFind(g_halted, "MAX DRAWDOWN") == 0) g_halted = "";
  }

//====================================================================
//  PAUZA PO SERII STRAT — engine.rs:3730 (PACZKA zamknięć per tick)
//====================================================================
void AktualizujSerie()
  {
   static ulong ostatni_deal = 0;
   if(In_StreakPauseN <= 0) return;
   HistorySelect(0, TimeCurrent() + 60);
   int total = HistoryDealsTotal();
   double suma = 0.0;
   int    ile  = 0;
   for(int i = 0; i < total; i++)
     {
      ulong d = HistoryDealGetTicket(i);
      if(d <= ostatni_deal) continue;
      ostatni_deal = d;
      if(HistoryDealGetInteger(d, DEAL_MAGIC) != In_Magic) continue;
      if(HistoryDealGetInteger(d, DEAL_ENTRY) != DEAL_ENTRY_OUT) continue;
      suma += HistoryDealGetDouble(d, DEAL_PROFIT) + HistoryDealGetDouble(d, DEAL_SWAP);
      ile++;
     }
   if(ile == 0) return;
   if(suma < 0.0)
     {
      g_loss_streak++;
      if(g_loss_streak >= In_StreakPauseN)
        { g_paused_until = g_now + (long)(In_StreakPauseMin * 60000.0); g_loss_streak = 0; }
     }
   else g_loss_streak = 0;
  }

//====================================================================
//  CYKL ŻYCIA
//====================================================================
// Native MQL VM regression harness. It creates ONLY tester-owned orders and
// bypasses the signal file. Injection is never reachable outside MQL_TESTER.
void ExitTestFinish(bool pass, string detail)
  {
   if(g_test_exit_finished) return;
   g_test_exit_finished = true;
   PrintFormat("CEXIT_TEST_RESULT|%d|%s|%s|confirmed=%d|ts=%I64d|positions=%d|orders=%d|fill=%d|partial=%d",
               In_TestExitScenario, pass ? "PASS" : "FAIL", detail,
               In_ConfirmedExitRetry, g_now, PositionsTotal(), OrdersTotal(), g_test_saw_fill, g_test_saw_partial);
   g_test_close_reject = 0; g_test_cancel_reject = 0;
   g_test_cancel_until_fill = false; g_test_partial_remaining = 0;
   // Test cleanup is explicit and not confused with the tested exit reason.
   for(int bi = 0; bi < g_nb; bi++)
     {
      ulong positions[]; ulong orders[];
      ExitOwnedSnapshot(bi, positions, orders);
      for(int i = 0; i < ArraySize(orders); i++) ExitCancelOwned(bi, orders[i]);
      ExitOwnedSnapshot(bi, positions, orders);
      for(int i = 0; i < ArraySize(positions); i++) { g_powod_zamk = "TEST_CLEANUP"; ZamknijPozycje(positions[i]); }
     }
   TesterStop();
  }
bool ExitTestRequire(bool ok, string detail)
  {
   if(!ok) ExitTestFinish(false, detail);
   return ok;
  }
void ExitTestBasket(int bi)
  {
   ZeroMemory(g_b[bi]);
   g_b[bi].id = bi + 1; g_b[bi].msg_id = 900001 + bi;
   g_b[bi].side = 0; g_b[bi].state = ST_WORKING; g_b[bi].created_ts = g_now;
   g_b[bi].entry_lo = g_bid - 1.0; g_b[bi].entry_hi = g_ask + 1.0;
   g_b[bi].zone_lo = g_bid - 1.0; g_b[bi].zone_hi = g_ask + 1.0;
   g_b[bi].ntp = 3;
   for(int i = 0; i < 3; i++) g_b[bi].tps[i] = g_ask + 10.0 * (i + 1);
   g_b[bi].nlv = 1; g_b[bi].lv_price[0] = g_bid - 1.0;
   g_b[bi].lv_vol[0] = 0.08; g_b[bi].lv_units[0] = 1;
  }
bool ExitTestOpen(int bi, double volume, ulong &ticket)
  {
   if(!WyslijRynek(bi, 0, volume, 0.0, false, 0.0, false,
                  "B" + IntegerToString(g_b[bi].id), ticket)) return false;
   if(!PositionSelectByTicket(ticket)) return false;
   g_b[bi].pos[0] = ticket; g_b[bi].pos_lv[0] = 0; g_b[bi].npos = 1;
   g_b[bi].had_positions = true; ZapiszWlasciciela(ticket, bi);
   return true;
  }
bool ExitTestNoNewRisk()
  {
   int positions = PositionsTotal(), orders = OrdersTotal();
   double lo = g_b[0].zone_lo, hi = g_b[0].zone_hi;
   ulong ticket = 0;
   if(WyslijRynek(0, 0, 0.01, 0.0, false, 0.0, false, "B1", ticket)) return false;
   if(WyslijLimit(0, g_bid - 5.0, 0.01, 0.0, false, 0.0, false, "B1", ORDER_TYPE_BUY_LIMIT, ticket)) return false;
   if(PlaceGrid(0, true, 0, true) != 0) return false;
   double targets[1]; targets[0] = g_ask + 100.0;
   ApplyEntryEdit(0, 1, false, false, lo - 100.0, hi - 100.0,
                  0.0, false, 0.0, false, targets, 1);
   HandleRiskFree(0, g_bid - 1.0, true);
   HandleTpHit(0, 4);
   RearmPass(); MarketLadderPass(); ReentryPass(); FastAddonSweep(); RelotPendings();
   return positions == PositionsTotal() && orders == OrdersTotal()
          && lo == g_b[0].zone_lo && hi == g_b[0].zone_hi && g_b[0].side == 0;
  }
void EditReviewScenarioTick()
  {
   if(HourOf(g_now) < 2) return;
   if(!ExitTestRequire(PositionsTotal() == 0 && OrdersTotal() == 0, "initial tester account must be empty")) return;
   g_nb = 2; ExitTestBasket(0); ExitTestBasket(1);
   if(!ExitTestRequire(ExitTestOpen(1, 0.02, g_test_reference_ticket), "reference open failed")) return;
   g_b[0].state = ST_PENDING;
   g_b[0].nlv = 2;
   ulong old_orders[2];
   for(int i = 0; i < 2; i++)
     {
      double price = g_bid - 50.0 - i;
      if(!ExitTestRequire(WyslijLimit(0, price, 0.01, 0.0, false, 0.0, false,
                                     "B1", ORDER_TYPE_BUY_LIMIT, old_orders[i]), "fixture pending open failed")) return;
      g_b[0].pend[i] = old_orders[i]; g_b[0].pend_lv[i] = i; g_b[0].npend++;
      g_b[0].lv_price[i] = price; g_b[0].lv_units[i] = 1; g_b[0].lv_vol[i] = 0.01;
     }
   double old_lo = g_b[0].zone_lo, old_hi = g_b[0].zone_hi;
   g_b[0].tp_stage = 2; g_b[0].plan_observed_stage = 3; g_b[0].tp_touch_ts[0] = 123;
   double requested_lo = g_bid - 61.0, requested_hi = g_bid - 60.0;
   double targets[1]; targets[0] = g_ask + 10.0;
   g_test_cancel_reject = In_TestExitScenario == 7 ? 1 : 0;
   ApplyEntryEdit(0, 0, true, false, requested_lo, requested_hi,
                  0.0, false, 0.0, false, targets, 1);
   if(In_TestExitScenario == 7)
     {
      if(!ExitTestRequire(EntryReviewBlocked(0) && g_b[0].zone_lo == old_lo && g_b[0].zone_hi == old_hi,
                         "edit failure committed geometry or lost review")) return;
      if(!ExitTestRequire(g_b[0].tp_stage == 2 && g_b[0].plan_observed_stage == 3
                         && g_b[0].tp_touch_ts[0] == 123,"edit failure lost committed progress")) return;
      if(!ExitTestRequire(OrdersTotal() == 1 && g_b[0].npend == 1,
                         "partial cancellation lost broker truth or created duplicate grid")) return;
      ulong ticket;
      if(!ExitTestRequire(!WyslijRynek(0, 0, 0.01, 0.0, false, 0.0, false, "B1", ticket)
                         && PlaceGrid(0) == 0, "review allowed new risk")) return;
      ApplyEntryEdit(0, 0, true, false, requested_lo - 10.0, requested_hi - 10.0,
                     0.0, false, 0.0, false, targets, 1);
      if(!ExitTestRequire(g_b[0].zone_lo == old_lo && EntryReviewBlocked(0)
                         && ExitRiskAllowed(0), "review did not survive edit or blocked existing management")) return;
     }
   else
     {
      if(!ExitTestRequire(!EntryReviewBlocked(0) && g_b[0].zone_lo == requested_lo
                         && g_b[0].zone_hi == requested_hi && OrdersTotal() > 0,
                         "clean edit did not place replacement")) return;
      if(!ExitTestRequire(!OrderSelect(old_orders[0]) && !OrderSelect(old_orders[1]), "old grid survived successful edit")) return;
     }
   if(!ExitTestRequire(PositionSelectByTicket(g_test_reference_ticket), "different basket was closed")) return;
   ExitTestFinish(true, In_TestExitScenario == 7 ? "EDIT_REFUSAL_RETAINS_COMMITTED_PLAN_AND_REVIEW" : "EDIT_REPLACEMENT_GOLDEN");
  }

void PartialReceiptScenarioTick()
  {
   if(g_test_exit_stage == 0)
     {
      if(HourOf(g_now) < 2) return;
      if(!ExitTestRequire(PositionsTotal() == 0 && OrdersTotal() == 0, "initial tester account must be empty")) return;
      g_nb = 2; ExitTestBasket(0); ExitTestBasket(1);
      if(!ExitTestRequire(ExitTestOpen(1, 0.02, g_test_reference_ticket), "reference open failed")) return;
      if(!ExitTestRequire(ExitTestOpen(0, 0.08, g_test_exit_ticket), "owned open failed")) return;
      PsEnsure(g_test_exit_ticket);
      g_powod_zamk = "TEST_LOSING_PARTIAL";
      if(!ExitTestRequire(ZamknijCzesc(g_test_exit_ticket, 0.04), "partial close failed")) return;
      g_test_exit_stage = 1;
      return;
     }
   OdswiezBilety();
   if(!ExitTestRequire(PositionSelectByTicket(g_test_reference_ticket), "different basket was closed")) return;
   double expected = 0.0;
   if(!ExitTestRequire(HistorySelectByPosition(g_test_exit_ticket), "partial history unavailable")) return;
   for(int i = 0; i < HistoryDealsTotal(); i++)
     {
      ulong d = HistoryDealGetTicket(i);
      if(HistoryDealGetInteger(d, DEAL_ENTRY) == DEAL_ENTRY_OUT)
         expected += HistoryDealGetDouble(d, DEAL_PROFIT) + HistoryDealGetDouble(d, DEAL_SWAP)
                   + HistoryDealGetDouble(d, DEAL_COMMISSION);
     }
   if(!ExitTestRequire(MathAbs(expected - g_b[0].realized) < 1e-6, "partial/final receipt lost or duplicated")) return;
   if(g_test_exit_stage == 1)
     {
      if(!ExitTestRequire(expected < 0.0, "fixture must realize a losing partial")) return;
      if(!ExitTestRequire(PositionSelectByTicket(g_test_exit_ticket)
                         && MathAbs(PositionGetDouble(POSITION_VOLUME) - 0.04) < 1e-9,
                         "partial must leave a live residual")) return;
      OdswiezBilety();
      if(!ExitTestRequire(MathAbs(expected - g_b[0].realized) < 1e-6, "repeated receipt was double booked")) return;
      PrintFormat("CEXIT_TEST_EVENT|losing_partial_booked_while_alive|%.8f|%.8f", expected, g_b[0].realized);
      g_powod_zamk = "TEST_FINAL_RECEIPT";
      if(!ExitTestRequire(ZamknijPozycje(g_test_exit_ticket), "final close failed")) return;
      g_test_exit_stage = 2;
      return;
     }
   if(!ExitTestRequire(!PositionSelectByTicket(g_test_exit_ticket), "final close left residual")) return;
   PrintFormat("CEXIT_TEST_EVENT|final_receipt_once|%.8f|%.8f", expected, g_b[0].realized);
   ExitTestFinish(true, "LOSING_PARTIAL_RECONCILED_BEFORE_FINAL_CLOSE");
  }

void KnownSpecialLevelScenarioTick()
  {
   if(HourOf(g_now) < 2) return;
   if(!ExitTestRequire(PositionsTotal()==0 && OrdersTotal()==0 && In_ProfitBudgetArmPct==0.0, "special-level fixture requires fresh legacy account")) return;
   g_nb=1; ExitTestBasket(0);
   ulong market=0, pending=0;
   if(!ExitTestRequire(ExitTestOpen(0,0.01,market), "special addon open failed")) return;
   g_b[0].pos_lv[0]=-4;
   if(!ExitTestRequire(WyslijLimit(0,g_bid-5.0,0.01,g_bid-15.0,true,0.0,false,"B1",ORDER_TYPE_BUY_LIMIT,pending), "special pending open failed")) return;
   g_b[0].pend[0]=pending;g_b[0].pend_lv[0]=-3;g_b[0].npend=1;
   if(!ExitTestRequire(!GridLevelHasLivePosition(0,0), "known special legs occupied unrelated grid")) return;
   if(!ExitTestRequire(GridLevelHasLivePosition(0,-4), "addon did not occupy its own level")) return;
   g_b[0].pos_lv[0]=-1;
   if(!ExitTestRequire(GridLevelHasLivePosition(0,0), "unknown position level was ignored")) return;
   g_b[0].pos_lv[0]=-4;g_b[0].pend_lv[0]=-1;
   if(!ExitTestRequire(GridLevelHasLivePosition(0,0), "unknown pending level was ignored")) return;
   g_b[0].pend_lv[0]=-3;
   ExitTestFinish(true,"KNOWN_SPECIAL_LEGS_AND_UNKNOWN_OWNERSHIP");
  }

void SourceFixtureMessage(long message,long edit,long reply,string action)
  {
   ArrayResize(g_msg,1);g_msg[0].ts=g_now;g_msg[0].msg_id=message;
   g_msg[0].edit_of=edit;g_msg[0].reply_to=reply;g_msg[0].hints="";
   g_msg[0].n=1;g_msg[0].akcje[0]=action;WykonajWiadomosc(0);
  }
void SourceRecoveryScenarioTick()
  {
   if(HourOf(g_now)<2)return;
   if(!ExitTestRequire(PositionsTotal()==0 && OrdersTotal()==0,"source fixture needs fresh account"))return;
   double hi=MathFloor(g_bid)-10.0,lo=hi-1.0,sl=lo-10.0,tp=hi+5.0;
   string entry=StringFormat("ENTRY2:entry,BUY,1,0,%.5f,%.5f,%.5f,0,nan,0,0,0,1,%.5f",lo,hi,sl,tp);
   SourceFixtureMessage(10,100,0,entry);
   if(In_EditOrphanNoEntry)
     {ExitTestFinish(g_nb==0 && OrdersTotal()==0,"LEGACY_ORPHAN_POLICY_RETAINS_REJECTION");return;}
   if(!ExitTestRequire(g_nb==1 && MapGet(100)==g_b[0].id && MapGet(10)==g_b[0].id
                      && g_b[0].created_ts==g_now && OrdersTotal()>0,"complete edit did not create protected receive-time pending"))return;
   int orders=OrdersTotal(),id=g_b[0].id;
   SourceFixtureMessage(10,100,0,entry);
   SourceFixtureMessage(100,0,0,entry);
   if(!ExitTestRequire(g_nb==1 && g_b[0].id==id && OrdersTotal()==orders,"duplicate edit/late NEW created risk"))return;
   SourceFixtureMessage(20,0,200,"INFO:source_reply_link");
   SourceFixtureMessage(21,0,20,"CANCEL:cancel");
   SourceFixtureMessage(200,200,0,entry);
   if(!ExitTestRequire(g_nb==1 && OrdersTotal()==orders && NativeSourceEntryBlocked(200,false),"unknown reply CANCEL guessed another basket or missed source"))return;
   SourceFixtureMessage(300,300,0,"MKT:market,BUY");
   string missing=StringFormat("ENTRY2:entry,BUY,1,0,%.5f,%.5f,nan,0,nan,0,0,0,1,%.5f",lo,hi,tp);
   SourceFixtureMessage(400,400,0,missing);
   if(!ExitTestRequire(g_nb==1,"incomplete orphan created basket"))return;
   g_halted="source_fixture_risk_gate";SourceFixtureMessage(500,500,0,entry);g_halted="";
   if(!ExitTestRequire(MapGet(500)<0,"risk rejection consumed source"))return;
   SourceFixtureMessage(500,500,0,entry);
   if(!ExitTestRequire(g_nb==2 && MapGet(500)>=0,"risk-rejected revision could not be retried"))return;
   for(int i=0;i<MAXB+20;i++)MapPut(10000+i,10000+i);
   if(!ExitTestRequire(MapGet(100)==id,"source map evicted old identity"))return;
   for(int bi=0;bi<g_nb;bi++)CancelPendings(bi);
   if(!ExitTestRequire(PositionsTotal()==0 && OrdersTotal()==0,"source fixture cleanup unconfirmed"))return;
   g_nb=0;SourceFixtureMessage(10,100,0,entry);
   if(!ExitTestRequire(g_nb==0 && OrdersTotal()==0,"pruned consumed source reopened"))return;
   ExitTestFinish(true,"PROTECTED_RECEIVE_EDIT_LATE_NEW_CANCEL_ALIAS_RISK_RETRY_PRUNE");
  }

void ProfitBudgetScenarioTick()
  {
   if(HourOf(g_now) < 2) return;
   if(!ExitTestRequire(PositionsTotal()==0 && OrdersTotal()==0, "profit budget needs fresh account")) return;
   if(!ExitTestRequire(In_ProfitBudgetArmPct==1.0 && In_ProfitBudgetKeepPct==50.0
                      && In_ProfitBudgetDeployPct==100.0 && In_MaxPortfolioRisk==0.0, "profit budget test inputs")) return;
   g_nb=1;ExitTestBasket(0);g_day=DayOf(g_now);
   double eq=AccountInfoDouble(ACCOUNT_EQUITY), remaining=0;string error;
   g_day_start_eq=eq;g_day_peak_eq=eq;
   if(!ExitTestRequire(ProfitBudgetAvailable(remaining,error)==0,"unarmed must retain legacy")) return;
   // Inject an already-observed peak into this tester-only fixture; production
   // reads only its normal day observation, never this synthetic test anchor.
   g_day_start_eq=eq-100.0;g_day_peak_eq=eq;
   if(!ExitTestRequire(ProfitBudgetAvailable(remaining,error)==1 && MathAbs(remaining-50.0)<1e-8,"observed profit capacity")) return;
   double entry=MathFloor(g_bid)-10.0, stop=entry-10.0, volume=0.10;
   if(!ExitTestRequire(!ProfitBudgetLimit(0,0,entry,0.0,0.1,volume),"missing SL was accepted")) return;
   volume=0.10;
   if(!ExitTestRequire(!ProfitBudgetLimit(0,0,entry,entry+1.0,0.1,volume),"adverse new stop was accepted")) return;
   // Broker-normalized SL must be used: 49.99 cannot buy a 50.00 risk lot.
   g_day_start_eq=eq-99.98;volume=0.05;
   if(!ExitTestRequire(ProfitBudgetLimit(0,0,entry,NormPx(stop+0.004),0.05,volume)
                      && MathAbs(volume-0.04)<1e-9,"normalized stop exceeded reserve")) return;
   g_day_start_eq=eq-100.0;
   ulong pending=0, refused=0;
   if(!ExitTestRequire(WyslijLimit(0,entry,0.10,stop,true,0.0,false,"B1",ORDER_TYPE_BUY_LIMIT,pending),"capped pending was refused")) return;
   g_b[0].pend[0]=pending;g_b[0].pend_lv[0]=0;g_b[0].npend=1;
   if(!ExitTestRequire(OrderSelect(pending) && MathAbs(OrderGetDouble(ORDER_VOLUME_CURRENT)-0.05)<1e-9,"native send did not floor to budget")) return;
   if(!ExitTestRequire(!WyslijLimit(0,entry,0.01,stop,true,0.0,false,"B1",ORDER_TYPE_BUY_LIMIT,refused)
                      && OrdersTotal()==1,"sequential sends overspent reserve")) return;
   if(!ExitTestRequire(ExitCancelOwned(0,pending),"budget fixture cancel failed")) return;
   if(!ExitTestRequire(ProfitBudgetAvailable(remaining,error)==1 && MathAbs(remaining-50.0)<1e-8,"confirmed cancel did not release reserve")) return;
   g_day--;
   if(!ExitTestRequire(ProfitBudgetAvailable(remaining,error)==-1 && error=="UnknownDayAnchor","unknown anchor was guessed")) return;
   g_day=DayOf(g_now);
   PrintFormat("CEXIT_TEST_EVENT|profit_budget|capacity=50|normalized_stop_volume=0.04|pending_volume=0.05|sequential_refused=1");
   ExitTestFinish(true,"PROFIT_BUDGET_NATIVE_FLOOR_AND_ACKNOWLEDGED_EXPOSURE");
  }

void PortfolioBudgetScenarioTick()
  {
   if(HourOf(g_now)<2)return;
   if(!ExitTestRequire(PositionsTotal()==0 && OrdersTotal()==0,"portfolio budget needs fresh account"))return;
   if(!ExitTestRequire(In_MaxPortfolioRisk==10.0 && In_ProfitBudgetArmPct==0.0,"portfolio-only test inputs"))return;
   g_nb=1;ExitTestBasket(0);
   // The standalone portfolio cap cannot depend on a profit-reserve anchor.
   g_day=-1;g_day_start_eq=0.0;g_day_peak_eq=0.0;
   double eq=AccountInfoDouble(ACCOUNT_EQUITY),remaining=0.0;string error;
   if(!ExitTestRequire(ProfitBudgetAvailable(remaining,error)==1 && MathAbs(remaining-eq*0.1)<1e-8,"portfolio-only capacity requires no anchor"))return;
   double entry=MathFloor(g_bid)-10.0,stop=entry-10.0;
   ulong pending=0,refused=0;
   if(!ExitTestRequire(WyslijLimit(0,entry,0.10,stop,true,0.0,false,"B1",ORDER_TYPE_BUY_LIMIT,pending),"portfolio pending refused"))return;
   g_b[0].pend[0]=pending;g_b[0].pend_lv[0]=0;g_b[0].npend=1;
   if(!ExitTestRequire(OrderSelect(pending) && MathAbs(OrderGetDouble(ORDER_VOLUME_CURRENT)-0.03)<1e-9,"portfolio cap did not floor actual pending to .03"))return;
   long requests=g_open_request_count;
   if(!ExitTestRequire(!WyslijLimit(0,entry,0.01,stop,true,0.0,false,"B1",ORDER_TYPE_BUY_LIMIT,refused)
                      && OrdersTotal()==1 && g_open_request_count==requests,"portfolio budget sent excess risk"))return;
   if(!ExitTestRequire(ExitCancelOwned(0,pending),"portfolio cancellation unconfirmed"))return;
   if(!ExitTestRequire(ProfitBudgetAvailable(remaining,error)==1 && MathAbs(remaining-eq*0.1)<1e-8,"cancel ACK did not release portfolio risk"))return;
   if(!ExitTestRequire(!WyslijRynek(0,0,0.01,0.0,false,0.0,false,"B1",refused)
                      && g_open_request_count==requests,"portfolio-only market without SL reached broker"))return;
   PrintFormat("CEXIT_TEST_EVENT|portfolio_budget|capacity=%.8f|pending_volume=0.03|sequential_refused=1|missing_market_stop_refused=1",remaining);
   ExitTestFinish(true,"PORTFOLIO_CAP_BEFORE_PROFIT_ARM_AND_ACKNOWLEDGED_EXPOSURE");
  }

void BasketCapacityScenarioTick()
  {
   if(HourOf(g_now)<2)return;
   if(!ExitTestRequire(PositionsTotal()==0 && OrdersTotal()==0 && In_HonorMarketOpen
                      && In_EntryIdempotency && In_ProfitBudgetArmPct==0.0
                      && In_MaxPortfolioRisk==0.0,"capacity fixture inputs"))return;
   // Bounded actual history: 601 distinct basket identities, each opened and
   // closed before the next. The live exposure count is never 601.
   for(int id=1;id<=MAXB+1;id++)
     {
      g_nb=1;ExitTestBasket(0);g_b[0].id=id;g_b[0].msg_id=100000+id;
      ulong ticket=0;
      if(!ExitTestRequire(ExitTestOpen(0,0.01,ticket),"history fixture open failed"))return;
      g_powod_zamk="TEST_CAPACITY_HISTORY";
      if(!ExitTestRequire(ZamknijPozycje(ticket),"history fixture close failed"))return;
     }
   NativeBasketResult history[];
   if(!ExitTestRequire(CollectNativeBasketResults(history) && ArraySize(history)==MAXB+1,"history results truncated at live capacity"))return;
   double sum=0.0;
   for(int i=0;i<ArraySize(history);i++)
     {if(!ExitTestRequire(history[i].closes==1,"history close duplicated or omitted"))return;sum+=history[i].profit;}
   if(!ExitTestRequire(MathAbs(sum-(AccountInfoDouble(ACCOUNT_BALANCE)-g_start_balance))<1e-6,"history results do not reconcile cash"))return;
   g_nb=MAXB;g_next_id=MAXB+2;
   for(int i=0;i<MAXB;i++){ZeroMemory(g_b[i]);g_b[i].id=10000+i;g_b[i].state=ST_DONE;}
   ExitTestBasket(MAXB-1);g_b[MAXB-1].id=9000;
   ulong kept=0;
   if(!ExitTestRequire(ExitTestOpen(MAXB-1,0.01,kept),"live sentinel open failed"))return;
   MapPut(900000,9000);
   g_b[1].exit_reason="stale";g_b[1].review_requested_lo=123.0;
   long sent=g_open_request_count;
   SourceFixtureMessage(777777,0,0,"MKT:market,BUY");
   if(!ExitTestRequire(g_nb==2 && BIdx(9000)==0 && PositionSelectByTicket(kept)
                      && MapGet(900000)==9000 && PositionsTotal()==2
                      && g_open_request_count==sent+1,"MarketOpen lost live basket or failed compaction"))return;
   if(!ExitTestRequire(StringLen(g_b[1].exit_reason)==0 && g_b[1].review_requested_lo==0.0,"reused market slot retained old metadata"))return;
   SourceFixtureMessage(777777,0,0,"MKT:market,BUY");
   if(!ExitTestRequire(g_nb==2 && PositionsTotal()==2 && g_open_request_count==sent+1,"MarketOpen source duplicated"))return;
   for(int i=2;i<MAXB;i++){ZeroMemory(g_b[i]);g_b[i].id=20000+i;g_b[i].state=ST_DONE;}
   g_nb=MAXB;g_b[2].exit_reason="stale";g_b[2].review_requested_hi=456.0;
   double hi=MathFloor(g_bid)-10.0,lo=hi-1.0,sl=lo-10.0,tp=hi+5.0;
   string entry=StringFormat("ENTRY2:entry,BUY,1,0,%.5f,%.5f,%.5f,0,nan,0,0,0,1,%.5f",lo,hi,sl,tp);
   SourceFixtureMessage(888888,0,0,entry);
   if(!ExitTestRequire(g_nb==3 && PositionsTotal()==2 && OrdersTotal()>0
                      && StringLen(g_b[2].exit_reason)==0 && g_b[2].review_requested_hi==0.0,"Entry compaction lost live exposure or reused stale metadata"))return;
   int orders=OrdersTotal();SourceFixtureMessage(888888,0,0,entry);
   if(!ExitTestRequire(g_nb==3 && PositionsTotal()==2 && OrdersTotal()==orders,"Entry source duplicated"))return;
   for(int i=3;i<MAXB;i++){ZeroMemory(g_b[i]);g_b[i].id=30000+i;g_b[i].state=ST_PENDING;}
   g_nb=MAXB;sent=g_open_request_count;
   if(!ExitTestRequire(!NativeEnsureBasketCapacity("fixture_all_active") && g_nb==MAXB
                      && PositionsTotal()==2 && OrdersTotal()==orders && sent==g_open_request_count,"active capacity changed risk or lost baskets"))return;
   g_nb=3;CancelPendings(2);
   for(int bi=0;bi<2;bi++)for(int i=g_b[bi].npos-1;i>=0;i--)ZamknijPozycje(g_b[bi].pos[i]);
   if(!ExitTestRequire(PositionsTotal()==0 && OrdersTotal()==0,"capacity fixture cleanup incomplete"))return;
   PrintFormat("CEXIT_TEST_EVENT|basket_capacity|history_ids=%d|retained_exposure=1|market_and_entry_compact=1|duplicate_sources=0|active_cap=%d",MAXB+1,MAXB);
   ExitTestFinish(true,"HISTORY_IDS_LIVE_CAPACITY_SOURCE_IDEMPOTENCY_AND_FRESH_SLOTS");
  }

void FillReceiptOrderScenarioTick()
  {
   if(HourOf(g_now)<2)return;
   if(!ExitTestRequire(PositionsTotal()==0 && OrdersTotal()==0
                      && In_RiskFreeMode==1 && In_RiskFreeRunners==1
                      && In_ProfitBudgetArmPct==0.0 && In_MaxPortfolioRisk==0.0,
                      "fill receipt fixture inputs"))return;
   ulong tie_tickets[3];double tie_keys[3];
   tie_tickets[0]=101;tie_tickets[1]=102;tie_tickets[2]=103;
   tie_keys[0]=2.0;tie_keys[1]=2.0;tie_keys[2]=1.0;
   NativeStableSortTickets(tie_tickets,tie_keys,3,false);
   if(!ExitTestRequire(tie_tickets[0]==103 && tie_tickets[1]==101 && tie_tickets[2]==102,"ascending equal-key order changed"))return;
   tie_tickets[0]=101;tie_tickets[1]=102;tie_tickets[2]=103;
   tie_keys[0]=1.0;tie_keys[1]=1.0;tie_keys[2]=2.0;
   NativeStableSortTickets(tie_tickets,tie_keys,3,true);
   if(!ExitTestRequire(tie_tickets[0]==103 && tie_tickets[1]==101 && tie_tickets[2]==102,"descending equal-key order changed"))return;
   g_nb=1;ExitTestBasket(0);g_b[0].nlv=3;
   ulong tickets[3];
   double price=0.0;
   for(int i=0;i<3;i++)
     {
      if(!ExitTestRequire(ExitTestOpen(0,0.01,tickets[i]),"receipt fixture position open failed"))return;
      if(i==0)price=PositionGetDouble(POSITION_PRICE_OPEN);
      if(!ExitTestRequire(PositionGetDouble(POSITION_PRICE_OPEN)==price,"receipt fixture prices differ"))return;
      g_b[0].lv_price[i]=price+i;g_b[0].lv_filled[i]=false;g_b[0].lv_fill_ts[i]=0;
     }
   // Actual tester positions provide a confirmed broker snapshot. Only the
   // pre-reconciliation pending cache is synthetic, ordered by submission.
   g_b[0].npos=0;g_b[0].npend=3;
   for(int i=0;i<3;i++){g_b[0].pend[i]=tickets[i];g_b[0].pend_lv[i]=i;}
   OdswiezBilety();
   if(!ExitTestRequire(g_b[0].npos==3 && g_b[0].npend==0,"receipt count changed"))return;
   for(int i=0;i<3;i++)
      if(!ExitTestRequire(g_b[0].pos[i]==tickets[i] && g_b[0].pos_lv[i]==i,"receipt registration reversed submission order"))return;
   OdswiezBilety();
   if(!ExitTestRequire(g_b[0].npos==3,"receipt replay duplicated exposure"))return;
   HandleRiskFree(0,price,true);
   if(!ExitTestRequire(PositionsTotal()==1 && PositionSelectByTicket(tickets[0]),"equal-price RiskFree retained a different grid level"))return;
   PrintFormat("CEXIT_TEST_EVENT|fill_receipt_order|positions=3|equal_price=1|retained_level=0|duplicate_receipts=0");
   ExitTestFinish(true,"SIMULTANEOUS_RECEIPT_ORDER_AND_RISKFREE_TIE");
  }

void EntryProgressScenarioTick()
  {
   if(HourOf(g_now)<2)return;
   if(!ExitTestRequire(PositionsTotal()==0 && OrdersTotal()==0
                      && In_ZoneOffsetMode==0 && In_SlMaxDist==0.0,
                      "entry progress fixture inputs"))return;
   g_nb=1;ExitTestBasket(0);
   ulong ticket=0;
   if(!ExitTestRequire(ExitTestOpen(0,0.01,ticket),"entry progress position open failed"))return;
   double lo=g_bid-2.0,hi=g_bid-1.0,sl=g_bid-100.0;
   double targets[2];targets[0]=g_ask+100.0;targets[1]=g_ask+200.0;
   ApplyEntryEdit(0,0,false,false,lo,hi,sl,true,0.0,false,targets,2);
   for(int mutation=0;mutation<4;mutation++)
     {
      g_b[0].tp_stage=2;g_b[0].plan_observed_stage=3;
      g_b[0].zone_touched=true;g_b[0].drop_armed=true;
      g_b[0].drop_po_ts=123;g_b[0].last_tp_ts=456;
      for(int i=0;i<MAXTP;i++)g_b[0].tp_touch_ts[i]=789;
      g_b[0].secured=true;g_b[0].fast_addons=1;
      if(mutation==1)targets[0]+=1.0;
      if(mutation==2)lo-=1.0;
      if(mutation==3)sl-=1.0;
      ApplyEntryEdit(0,0,false,false,lo,hi,sl,true,0.0,false,targets,2);
      bool unchanged=mutation==0;
      bool progress_ok=unchanged ? (g_b[0].tp_stage==2 && g_b[0].plan_observed_stage==3
                        && g_b[0].zone_touched && g_b[0].drop_armed && g_b[0].drop_po_ts==123
                        && g_b[0].last_tp_ts==456)
                        : (g_b[0].tp_stage==0 && g_b[0].plan_observed_stage==0
                        && !g_b[0].zone_touched && !g_b[0].drop_armed && g_b[0].drop_po_ts==0
                        && g_b[0].last_tp_ts==0);
      for(int i=0;i<MAXTP;i++)progress_ok=progress_ok && g_b[0].tp_touch_ts[i]==(unchanged?789:0);
      if(!ExitTestRequire(progress_ok && g_b[0].secured && g_b[0].fast_addons==1
                         && PositionsTotal()==1 && PositionSelectByTicket(ticket),"ENTRY reset/no-op touched wrong state"))return;
     }
   g_b[0].tp_stage=2;g_b[0].tp_touch_ts[0]=789;targets[0]+=1.0;
   if(!ExitTestRequire(ApplySppTargetPlan(0,targets,2)
                      && (In_ResetTpOnTargetEdit ? g_b[0].tp_stage==0 : g_b[0].tp_stage==2),
                      "ENTRY reset changed separate SPP switch"))return;
   Print("CEXIT_TEST_EVENT|entry_progress|unchanged_preserved=1|tp_zone_sl_reset=1|working_position_preserved=1|spp_separate=1");
   ExitTestFinish(true,"FULL_ENTRY_EFFECTIVE_PLAN_PROGRESS_AND_NOOP");
  }

void EntrySourceNoopScenarioTick()
  {
   if(HourOf(g_now)<2)return;
   if(!ExitTestRequire(PositionsTotal()==0 && OrdersTotal()==0
                      && In_ZoneOffsetMode==0 && In_SlMaxDist==0.0,
                      "source no-op fixture inputs"))return;
   g_nb=1;ExitTestBasket(0);MapPut(500,1);
   ulong ticket=0;
   if(!ExitTestRequire(ExitTestOpen(0,0.01,ticket),"source no-op position open failed"))return;
   double lo=g_bid-2.0,hi=g_bid-1.0,sl=g_bid-100.0,tp1=g_ask+100.0,tp2=g_ask+200.0;
   string entry=StringFormat("ENTRY2:entry,BUY,0,0,%.8f,%.8f,%.8f,0,nan,0,0,0,2,%.8f,%.8f",lo,hi,sl,tp1,tp2);
   SourceFixtureMessage(501,500,0,entry);
   if(!ExitTestRequire(g_b[0].entry_source.known && g_b[0].entry_source.ntp==2,"raw source snapshot missing"))return;
   // Actual confirmed stop management after the source was accepted.
   double protected_sl=NormPx(g_bid-1.0);SetBasketSl(0,protected_sl);
   if(!ExitTestRequire(PositionSelectByTicket(ticket)
                      && PositionGetDouble(POSITION_SL)==protected_sl,"managed stop unconfirmed"))return;
   g_b[0].tp_stage=2;g_b[0].plan_observed_stage=3;g_b[0].tp_touch_ts[0]=123;
   g_b[0].secured=true;
   SourceFixtureMessage(502,500,0,entry);
   if(!ExitTestRequire(g_b[0].tp_stage==2 && g_b[0].plan_observed_stage==3
                      && g_b[0].tp_touch_ts[0]==123 && g_b[0].sl==protected_sl
                      && PositionSelectByTicket(ticket) && PositionGetDouble(POSITION_SL)==protected_sl,
                      "same raw source overwrote managed stop or reset progress"))return;
   // A genuinely changed raw TP must be accepted and retain its new snapshot.
   string changed=StringFormat("ENTRY2:entry,BUY,0,0,%.8f,%.8f,%.8f,0,nan,0,0,0,2,%.8f,%.8f",lo,hi,sl,tp1+1.0,tp2);
   SourceFixtureMessage(503,500,0,changed);
   if(!ExitTestRequire(g_b[0].tp_stage==0 && g_b[0].plan_observed_stage==0
                      && MathAbs(g_b[0].entry_source.tps[0]-(tp1+1.0))<1e-7,
                      "changed source failed to commit/reset"))return;
   g_b[0].tp_stage=1;SourceFixtureMessage(504,500,0,changed);
   if(!ExitTestRequire(g_b[0].tp_stage==1,"accepted changed source repeated reset"))return;
   Print("CEXIT_TEST_EVENT|entry_source_noop|raw_source_before_runner=1|managed_sl_preserved=1|progress_preserved=1|changed_source_committed=1");
   ExitTestFinish(true,"RAW_SOURCE_NOOP_AFTER_CONFIRMED_STOP_MANAGEMENT");
  }

void EntryReceiptBarrierScenarioTick()
  {
   if(HourOf(g_now)<2)return;
   if(!ExitTestRequire(PositionsTotal()==0 && OrdersTotal()==0,"entry barrier fixture account"))return;
   g_nb=1;ExitTestBasket(0);g_b[0].state=ST_PENDING;
   double old_sl=NormPx(g_bid-100.0);ulong known=0,unknown=0;
   if(!ExitTestRequire(WyslijLimit(0,g_bid-50.0,0.01,old_sl,true,0.0,false,"B1",ORDER_TYPE_BUY_LIMIT,known),"known fixture pending failed"))return;
   g_b[0].pend[0]=known;g_b[0].pend_lv[0]=0;g_b[0].npend=1;
   // Deliberately outside the strategy's ownership registry, but still inside
   // this isolated tester and known to this bounded fixture by its receipt.
   MqlTradeRequest request;MqlTradeResult receipt;ZeroMemory(request);ZeroMemory(receipt);
   request.action=TRADE_ACTION_PENDING;request.symbol=_Symbol;request.magic=In_Magic;
   request.type=ORDER_TYPE_BUY_LIMIT;request.type_time=ORDER_TIME_GTC;request.type_filling=g_fill_pending;
   request.volume=0.01;request.price=NormPx(g_bid-60.0);request.sl=old_sl;request.comment="unresolved_fixture";
   if(!ExitTestRequire(OrderSend(request,receipt) && (receipt.retcode==TRADE_RETCODE_DONE
                      || receipt.retcode==TRADE_RETCODE_PLACED),"unknown fixture pending failed"))return;
   unknown=receipt.order;
   g_b[0].sl=old_sl;g_b[0].has_sl=true;g_b[0].tp_stage=2;
   double original_lo=g_b[0].zone_lo,original_hi=g_b[0].zone_hi;
   double targets[1];targets[0]=g_ask+100.0;
   ApplyEntryEdit(0,0,true,false,g_bid-62.0,g_bid-61.0,g_bid-110.0,true,0.0,false,targets,1);
   if(!ExitTestRequire(EntryReviewBlocked(0) && g_b[0].tp_stage==2
                      && g_b[0].zone_lo==original_lo && g_b[0].zone_hi==original_hi
                      && OrdersTotal()==2 && OrderSelect(known) && OrderGetDouble(ORDER_SL)==old_sl,
                      "uncertain snapshot changed committed progress or broker stop"))return;
   Print("CEXIT_TEST_EVENT|entry_receipt_barrier|unknown_owner=1|known_stop_unchanged=1|orders_preserved=2|review_before_mutation=1");
   ExitTestFinish(true,"UNCERTAIN_ENTRY_SNAPSHOT_BEFORE_BROKER_MUTATION");
  }

void EmptySppScenarioTick()
  {
   if(HourOf(g_now)<2)return;
   if(!ExitTestRequire(PositionsTotal()==0 && OrdersTotal()==0 && In_BankAllAtStage==0
                      && !In_BankCloseLast && In_SmartSlMode==2 && !In_SmartSlFloorBeRf,
                      "empty SPP fixture inputs"))return;
   g_nb=1;ExitTestBasket(0);MapPut(700,1);
   ulong ticket=0;if(!ExitTestRequire(ExitTestOpen(0,0.01,ticket),"empty SPP position open failed"))return;
   g_b[0].has_sl=false;g_b[0].ntp=3;
   for(int i=0;i<3;i++)g_b[0].tps[i]=NormPx(g_bid-3.0+i);
   g_b[0].tp_stage=2;g_b[0].plan_observed_stage=2;
   double expected_sl=g_b[0].tps[2];
   SourceFixtureMessage(701,0,700,"SPP:empty,nan,nan,");
   if(!ExitTestRequire(g_b[0].ntp==3 && g_b[0].tps[2]==expected_sl
                      && g_b[0].tp_stage==3 && g_b[0].plan_observed_stage==3
                      && PositionSelectByTicket(ticket) && PositionGetDouble(POSITION_SL)==expected_sl,
                      "empty SPP became TP0 or lost the stage-dependent broker stop"))return;
   Print("CEXIT_TEST_EVENT|empty_spp|targets_preserved=3|stage_before=2|stage_after=3|actual_ladder_stop=1");
   ExitTestFinish(true,"EMPTY_SPP_TARGETS_RETAIN_PLAN_AND_ADVANCE_EXISTING_STAGE");
  }

void CorrectionWireScenarioTick()
  {
   if(HourOf(g_now)<2)return;
   if(!ExitTestRequire(PositionsTotal()==0 && OrdersTotal()==0 && In_TpCorrToBroker
                      && In_BankAllAtStage==0 && !In_BankCloseLast && In_TpSource==2,
                      "correction fixture inputs"))return;
   g_nb=1;ExitTestBasket(0);MapPut(800,1);
   ulong position=0,pending=0;
   if(!ExitTestRequire(ExitTestOpen(0,0.01,position),"correction position open failed"))return;
   g_b[0].ntp=3;for(int i=0;i<3;i++)g_b[0].tps[i]=NormPx(g_ask+10.0*(i+1));
   g_b[0].tp_stage=1;g_b[0].plan_observed_stage=1;
   double sl=NormPx(g_bid-100.0),price=NormPx(g_bid-50.0);
   if(!ExitTestRequire(WyslijLimit(0,price,0.01,sl,true,g_b[0].tps[2],true,"B1",ORDER_TYPE_BUY_LIMIT,pending),"correction pending open failed"))return;
   g_b[0].pend[0]=pending;g_b[0].pend_lv[0]=-2;g_b[0].npend=1;
   double desired=NormPx(g_ask+21.0);
   SourceFixtureMessage(801,0,800,StringFormat("TPCORR:correct,2,%.8f",desired));
   if(!ExitTestRequire(OrderSelect(pending) && OrderGetDouble(ORDER_TP)==desired
                      && OrderGetDouble(ORDER_PRICE_OPEN)==price && OrderGetDouble(ORDER_SL)==sl,
                      "correction pending used final/per-grid TP or changed other fields"))return;
   g_b[0].tp_stage=2;g_b[0].plan_observed_stage=2;
   SourceFixtureMessage(802,0,800,"TPHIT2:zero,0,nan,0");
   if(!ExitTestRequire(g_b[0].tp_stage==2 && g_b[0].plan_observed_stage==2
                      && PositionSelectByTicket(position),"TP0 advanced an already reached stage"))return;
   Print("CEXIT_TEST_EVENT|correction_wire|pending_current_stage_tp=1|price_sl_preserved=1|explicit_zero_index_not_none=1");
   ExitTestFinish(true,"TP_CORRECTION_PENDING_STAGE_AND_ZERO_INDEX");
  }

void StrategyRealizedScenarioTick()
  {
   if(HourOf(g_now)<2)return;
   double opens[5]={4595.80,4594.43,4593.44,4592.19,4591.13};
   double cash[5]={1.77,0.40,-0.59,-1.84,-2.90};
   double raw=0.0,rounded=0.0,value=0.0;
   for(int i=0;i<5;i++)
     {
      if(!ExitTestRequire(NativeStrategyRealized(1,opens[i],4594.03,0.01,0.0,cash[i],false,value),"confirmed strategy geometry rejected"))return;
      raw+=value;rounded+=cash[i];
     }
   double floating=(4593.57-4596.73)*(-1.0)*XAU_CONTRACT*0.01;
   PrintFormat("CEXIT_TEST_EVENT|strategy_realized_boundary|raw=%.17g|cash=%.17g|floating=%.17g|raw_total=%.17g|cash_total=%.17g",raw,rounded,floating,raw+floating,rounded+floating);
   if(!ExitTestRequire(raw+floating>=0.0 && rounded+floating<0.0,"B1045 confirmed binary threshold not reproduced"))return;
   double swapped=0.0,partial=0.0,net=0.0;
   if(!ExitTestRequire(NativeStrategyRealized(0,4000.0,4001.0,0.04,-0.75,999.0,false,swapped)
                      && NativeStrategyRealized(0,4000.0,4001.0,0.02,-0.25,999.0,false,partial)
                      && swapped==3.25 && partial==1.75,"allocated swap or partial volume changed basis"))return;
   if(!ExitTestRequire(NativeStrategyRealized(-1,0,0,0,0,12.345,true,net) && net==12.345,
                      "canonical net mode was re-derived"))return;
   if(!ExitTestRequire(!NativeStrategyRealized(0,0,4001,0.01,0,0,false,value),"missing entry geometry accepted"))return;
   ExitTestFinish(true,"CONFIRMED_PRICE_PLUS_SWAP_STRATEGY_BASIS_WITH_ACTUAL_CASH_PRESERVED");
  }

void ExitFaultScenarioTick()
  {
   if(!MQLInfoInteger(MQL_TESTER) || In_TestExitScenario == 0 || g_test_exit_finished) return;
   if(In_TestExitScenario == 9) { KnownSpecialLevelScenarioTick(); return; }
   if(In_TestExitScenario == 10) { ProfitBudgetScenarioTick(); return; }
   if(In_TestExitScenario == 13) { PortfolioBudgetScenarioTick(); return; }
   if(In_TestExitScenario == 14) { BasketCapacityScenarioTick(); return; }
   if(In_TestExitScenario == 15) { FillReceiptOrderScenarioTick(); return; }
   if(In_TestExitScenario == 16) { EntryProgressScenarioTick(); return; }
   if(In_TestExitScenario == 17) { EntrySourceNoopScenarioTick(); return; }
   if(In_TestExitScenario == 18) { EntryReceiptBarrierScenarioTick(); return; }
   if(In_TestExitScenario == 19) { EmptySppScenarioTick(); return; }
   if(In_TestExitScenario == 20) { CorrectionWireScenarioTick(); return; }
   if(In_TestExitScenario == 21) { StrategyRealizedScenarioTick(); return; }
   if(In_TestExitScenario == 11 || In_TestExitScenario == 12) { SourceRecoveryScenarioTick(); return; }
   if(In_TestExitScenario == 6) { PartialReceiptScenarioTick(); return; }
   if(In_TestExitScenario == 7 || In_TestExitScenario == 8) { EditReviewScenarioTick(); return; }
   if(g_test_exit_stage == 0)
     {
      // Fixture starts inside the normal session, not on the first quote
      // during the broker's 01:00 daily trading break (native retcode10018).
      if(HourOf(g_now) < 2) return;
      if(!ExitTestRequire(PositionsTotal() == 0 && OrdersTotal() == 0, "initial tester account must be empty")) return;
      if(!ExitTestRequire(In_BankAllAtStage == 3 && In_AssignTpPerPos && In_TpSchedule == 0, "test inputs contract")) return;
      if(!ExitTestRequire((In_TestExitScenario == 4) ? !In_ConfirmedExitRetry
                         : (In_TestExitScenario == 5 || In_ConfirmedExitRetry), "wrong ON/OFF scenario")) return;
      g_test_exit_started = g_now;
      g_nb = 2; ExitTestBasket(0); ExitTestBasket(1);
      if(!ExitTestRequire(ExitTestOpen(1, 0.02, g_test_reference_ticket), "reference open failed")) return;
      if(In_TestExitScenario == 2 || In_TestExitScenario == 3)
        {
         bool fill_case = In_TestExitScenario == 2;
         double price = fill_case ? g_ask + MathMax(g_stops + 0.02, 0.22) : g_bid - 50.0;
         int type = fill_case ? ORDER_TYPE_BUY_STOP : ORDER_TYPE_BUY_LIMIT;
         if(!ExitTestRequire(WyslijLimit(0, price, 0.08, 0.0, false, 0.0, false,
                                        "B1", type, g_test_exit_ticket), "pending open failed")) return;
         g_b[0].pend[0] = g_test_exit_ticket; g_b[0].pend_lv[0] = 0; g_b[0].npend = 1;
         g_test_cancel_reject = fill_case ? 0 : 1;
         g_test_cancel_until_fill = fill_case;
         g_test_partial_remaining = fill_case ? 1 : 0;
         g_test_exit_requested = g_now;
         RequestConfirmedExit(0, "TEST_PENDING");
        }
      else
        {
         if(!ExitTestRequire(ExitTestOpen(0, 0.08, g_test_exit_ticket), "owned open failed")) return;
         g_test_close_reject = In_TestExitScenario == 5 ? 0 : 1;
         g_test_exit_requested = g_now;
         HandleTpHit(0, 3);
        }
      if(In_TestExitScenario == 4)
        {
         bool legacy = g_b[0].state == ST_DONE && PositionSelectByTicket(g_test_exit_ticket);
         ExitTestFinish(legacy, "EXPECTED_LEGACY_DONE_WITH_LIVE_POSITION"); return;
        }
      if(In_TestExitScenario != 5)
        {
         if(!ExitTestRequire(ConfirmedExitPending(0) && g_b[0].state != ST_DONE, "refusal lost exit intent")) return;
         string reason = g_b[0].exit_reason;
         RequestConfirmedExit(0, "MUST_NOT_REPLACE_FIRST_REASON");
         if(!ExitTestRequire(g_b[0].exit_reason == reason, "reason overwritten")) return;
         if(!ExitTestRequire(ExitTestNoNewRisk(), "edit/rearm/open changed closing basket")) return;
        }
      g_halted = "TEST_HALT";
      g_test_exit_stage = 1;
      PrintFormat("CEXIT_TEST_EVENT|intent_held|%I64d|%s|%I64u", g_now, g_b[0].exit_reason, g_test_exit_ticket);
      return;
     }
   OdswiezBilety();
   if(In_TestExitScenario == 2)
     {
      ulong p[]; ulong o[]; ExitOwnedSnapshot(0, p, o);
      if(ArraySize(p) > 0 && !g_test_saw_fill)
        {
         g_test_saw_fill = true;
         PrintFormat("CEXIT_TEST_EVENT|actual_pending_fill|%I64d|%I64u|%I64u", g_now, g_test_exit_ticket, p[0]);
        }
     }
   RetryConfirmedExits(); // must still run with g_halted = TEST_HALT
   if(!ExitTestRequire(PositionSelectByTicket(g_test_reference_ticket), "different basket was closed")) return;
   if(g_b[0].state == ST_DONE)
     {
      ulong p[]; ulong o[]; bool complete = ExitOwnedSnapshot(0, p, o);
      if(!ExitTestRequire(complete && ArraySize(p) == 0 && ArraySize(o) == 0 && !g_b[0].exit_pending, "Done was not broker-flat")) return;
      if(In_TestExitScenario != 5 && !ExitTestRequire(g_now - g_test_exit_requested >= 1000, "retry before cadence")) return;
      if(In_TestExitScenario == 2 && !ExitTestRequire(g_test_saw_fill && g_test_saw_partial, "missing actual fill/partial proof")) return;
      // The native legacy ledger books on disappearing tickets. Confirmed
      // Done must not bypass that reconciliation and silently lose realized.
      OdswiezBilety();
      double expected = 0.0;
      if(HistorySelectByPosition(g_test_exit_ticket))
         for(int i = 0; i < HistoryDealsTotal(); i++)
           {
            ulong d = HistoryDealGetTicket(i);
            if(HistoryDealGetInteger(d, DEAL_ENTRY) == DEAL_ENTRY_OUT)
               expected += HistoryDealGetDouble(d, DEAL_PROFIT) + HistoryDealGetDouble(d, DEAL_SWAP) + HistoryDealGetDouble(d, DEAL_COMMISSION);
           }
      if(In_ConfirmedExitRetry && !ExitTestRequire(MathAbs(expected - g_b[0].realized) < 1e-6, "closed realized was lost or duplicated")) return;
      PrintFormat("CEXIT_TEST_EVENT|realized|%.8f|%.8f", expected, g_b[0].realized);
      ExitTestFinish(true, In_TestExitScenario == 5 ? "NO_FAULT_GOLDEN" : "BROKER_FLAT_CONFIRMED_UNDER_HALT"); return;
     }
   if(g_now - g_test_exit_started > 1800000) ExitTestFinish(false, "30-minute simulated timeout, no assumed fill");
  }

int OnInit()
  {
   if(!MQLInfoInteger(MQL_TESTER))
     {
      Print("CONDUIT_XT: ten ekspert dziala WYLACZNIE w Testerze Strategii.");
      return INIT_FAILED;
     }
   if(!TestSppTargetPlanReset()) return INIT_FAILED;
   if(!TestBeRetargetContract()) return INIT_FAILED;
   if(In_TestExitScenario < 0 || In_TestExitScenario > 21) return INIT_PARAMETERS_INCORRECT;
   if(In_TestExitScenario > 0
      && (ENUM_ACCOUNT_MARGIN_MODE)AccountInfoInteger(ACCOUNT_MARGIN_MODE) != ACCOUNT_MARGIN_MODE_RETAIL_HEDGING)
      return INIT_PARAMETERS_INCORRECT;
   if((ENUM_ACCOUNT_MARGIN_MODE)AccountInfoInteger(ACCOUNT_MARGIN_MODE)
      != ACCOUNT_MARGIN_MODE_RETAIL_HEDGING)
     {
      Print("CONDUIT_XT: HEDGING is required; netting cannot reproduce basket/position semantics.");
      return INIT_PARAMETERS_INCORRECT;
     }

   if(AccountInfoString(ACCOUNT_CURRENCY)!="USD"
      || SymbolInfoString(_Symbol,SYMBOL_CURRENCY_PROFIT)!="USD"
      || SymbolInfoDouble(_Symbol,SYMBOL_TRADE_CONTRACT_SIZE)!=XAU_CONTRACT)
     {
      Print("CONDUIT_XT: strategy realized requires confirmed USD profit and contract 100.");
      return INIT_PARAMETERS_INCORRECT;
     }
   long maska = SymbolInfoInteger(_Symbol, SYMBOL_FILLING_MODE);
   if((maska & SYMBOL_FILLING_FOK) != 0)      g_fill_deal = ORDER_FILLING_FOK;
   else if((maska & SYMBOL_FILLING_IOC) != 0) g_fill_deal = ORDER_FILLING_IOC;
   else                                       g_fill_deal = ORDER_FILLING_RETURN;
   PrintFormat("CONDUIT_XT %s: maska wypelnienia=%d -> deal=%d", XT_WERSJA, maska, (int)g_fill_deal);

   g_stops = In_StopsLevel;
   if(g_stops <= 0.0)
     {
      long lv = SymbolInfoInteger(_Symbol, SYMBOL_TRADE_STOPS_LEVEL);
      g_stops = lv * SymbolInfoDouble(_Symbol, SYMBOL_POINT);
     }
   PrintFormat("CONDUIT_XT: stops_level=%.5f krok_lota=%.2f min_lot=%.2f kredyt=%.2f",
               g_stops, SymbolInfoDouble(_Symbol, SYMBOL_VOLUME_STEP),
               SymbolInfoDouble(_Symbol, SYMBOL_VOLUME_MIN), KredytSkuteczny());
   // Odcisk symbolu do uczciwego porównania z modelem Rust. Te wartości są
   // własnością brokera/Testera, a nie presetem strategii.
   PrintFormat("BROKER_SPEC symbol=%s digits=%d point=%.8f tick_size=%.8f "
               "tick_value=%.8f contract=%.2f stops=%.8f freeze=%.8f "
               "vol_min=%.4f vol_step=%.4f vol_max=%.4f swap_mode=%d "
               "swap_long=%.8f swap_short=%.8f swap3day=%d leverage=%d hedging=%d cash_digits=%d pending_limit=%d",
               _Symbol, (int)SymbolInfoInteger(_Symbol, SYMBOL_DIGITS),
               SymbolInfoDouble(_Symbol, SYMBOL_POINT),
               SymbolInfoDouble(_Symbol, SYMBOL_TRADE_TICK_SIZE),
               SymbolInfoDouble(_Symbol, SYMBOL_TRADE_TICK_VALUE),
               SymbolInfoDouble(_Symbol, SYMBOL_TRADE_CONTRACT_SIZE),
               g_stops,
               SymbolInfoInteger(_Symbol, SYMBOL_TRADE_FREEZE_LEVEL)
                  * SymbolInfoDouble(_Symbol, SYMBOL_POINT),
               SymbolInfoDouble(_Symbol, SYMBOL_VOLUME_MIN),
               SymbolInfoDouble(_Symbol, SYMBOL_VOLUME_STEP),
               SymbolInfoDouble(_Symbol, SYMBOL_VOLUME_MAX),
               (int)SymbolInfoInteger(_Symbol, SYMBOL_SWAP_MODE),
               SymbolInfoDouble(_Symbol, SYMBOL_SWAP_LONG),
               SymbolInfoDouble(_Symbol, SYMBOL_SWAP_SHORT),
               (int)SymbolInfoInteger(_Symbol, SYMBOL_SWAP_ROLLOVER3DAYS),
               (int)AccountInfoInteger(ACCOUNT_LEVERAGE),
               AccountInfoInteger(ACCOUNT_MARGIN_MODE) == ACCOUNT_MARGIN_MODE_RETAIL_HEDGING,
               (int)AccountInfoInteger(ACCOUNT_CURRENCY_DIGITS),
               (int)AccountInfoInteger(ACCOUNT_LIMIT_ORDERS));
   // Official MQL5 contract: seconds from midnight of the broker's weekday.
   // Preserve raw seconds (an end may be 86400); do not convert to UTC or
   // modulo 24 hours. Quote availability does not imply trade availability.
   for(int session_day = 0; session_day < 7; session_day++)
     {
      int trade_sessions = 0, quote_sessions = 0;
      datetime session_from = 0, session_to = 0;
      while(SymbolInfoSessionTrade(_Symbol, (ENUM_DAY_OF_WEEK)session_day,
                                  (uint)trade_sessions, session_from, session_to))
        {
         PrintFormat("BROKER_SESSION kind=trade day_sun0=%d index=%d from_seconds=%I64d to_seconds=%I64d clock=broker",
                     session_day, trade_sessions, (long)session_from, (long)session_to);
         trade_sessions++;
        }
      while(SymbolInfoSessionQuote(_Symbol, (ENUM_DAY_OF_WEEK)session_day,
                                  (uint)quote_sessions, session_from, session_to))
        {
         PrintFormat("BROKER_SESSION kind=quote day_sun0=%d index=%d from_seconds=%I64d to_seconds=%I64d clock=broker",
                     session_day, quote_sessions, (long)session_from, (long)session_to);
         quote_sessions++;
        }
      PrintFormat("BROKER_SESSION_SUMMARY day_sun0=%d trade_count=%d quote_count=%d clock=broker",
                  session_day, trade_sessions, quote_sessions);
     }
   if(StringLen(In_DzienOd) > 0) g_dzien_od = (long)StringToTime(In_DzienOd) * 1000;
   if(StringLen(In_DzienDo) > 0) g_dzien_do = (long)StringToTime(In_DzienDo) * 1000;
   if(g_dzien_od > 0)
      PrintFormat("CONDUIT_XT: tryb dzienny — handel tylko %s .. %s", In_DzienOd, In_DzienDo);
   if(In_TestExitScenario == 0 && !WczytajMost()) return INIT_FAILED;
   if(In_Diag)
      g_handle_diag = In_TestExitScenario > 0
         ? FileOpen("confirmed_exit_test_diag.csv", FILE_WRITE | FILE_CSV)
         : FileOpen(In_DiagFile, FILE_WRITE | FILE_CSV | FILE_COMMON | FILE_SHARE_READ);
   g_nb = 0; g_mi = 0; g_nph = 0; g_ndone = 0; g_nmap = 0;
   g_nsources=0;g_nsource_alias=0;g_source_message=0;g_source_original=0;g_source_edit=false;
   g_slhit_dnia = 0; g_slhit_pauza_do = LONG_MIN; g_rej_slhit = 0;
   g_rezim_miekki = false; g_wyciszen = 0;
   g_start_balance = TesterStatistics(STAT_INITIAL_DEPOSIT);
   g_peak_equity = g_start_balance;
   g_day_start_eq = g_start_balance;
   g_day_peak_eq = g_start_balance;
   return INIT_SUCCEEDED;
  }

int NativeBasketResultIndex(NativeBasketResult &results[],int id,long source_id)
  {
   for(int i=0;i<ArraySize(results);i++)if(results[i].id==id)return i;
   int n=ArraySize(results);ArrayResize(results,n+1);
   results[n].id=id;results[n].source_id=source_id;results[n].profit=0.0;results[n].closes=0;
   return n;
  }
bool CollectNativeBasketResults(NativeBasketResult &results[])
  {
   ArrayResize(results,0);
   if(!HistorySelect(0,TimeCurrent()+86400))return false;
   bool complete=true;
   int total = HistoryDealsTotal();
   // IDs are lifetime identities, not indices into the live MAXB slots.
   // Include registered zero-close baskets just as the previous report did.
   for(int j=0;j<g_nrej;j++)if(g_rej_bid[j]>=0)
      NativeBasketResultIndex(results,g_rej_bid[j],g_rej_msg[j]);
   for(int i = 0; i < total; i++)
     {
      ulong d = HistoryDealGetTicket(i);
      if(HistoryDealGetInteger(d, DEAL_MAGIC) != In_Magic) continue;
      if(HistoryDealGetInteger(d, DEAL_ENTRY) != DEAL_ENTRY_OUT) continue;
      ulong poz = (ulong)HistoryDealGetInteger(d, DEAL_POSITION_ID);
      bool mapped=false;
      for(int j = 0; j < g_nrej; j++)
         if(g_rej_tk[j] == poz)
           {
            if(g_rej_bid[j]>=0)
              {
               int k=NativeBasketResultIndex(results,g_rej_bid[j],g_rej_msg[j]);
               results[k].profit+=HistoryDealGetDouble(d,DEAL_PROFIT)
                                 +HistoryDealGetDouble(d,DEAL_SWAP)
                                 +HistoryDealGetDouble(d,DEAL_COMMISSION);
               results[k].closes++;
               mapped=true;
              }
            break;
           }
      if(!mapped)complete=false;
     }
   return complete;
  }

// Full registered broker history, independent of compaction and lifetime ID.
void ZrzucWyniki()
  {
   if(g_handle_diag==INVALID_HANDLE)return;
   NativeBasketResult results[];
   bool complete=CollectNativeBasketResults(results);
   FileWrite(g_handle_diag,"BASKET_RESULT_COVERAGE",complete?"1":"0",(string)ArraySize(results));
   for(int j=0;j<ArraySize(results);j++)
     {
      FileWrite(g_handle_diag,"WYNIK",(string)results[j].id,(string)results[j].source_id,
                StringFormat("%.5f",results[j].profit),(string)results[j].closes);
     }
  }

// Ledger diagnostyczny z milisekundami: historia BROKERA, nie zaokrąglony
// raport HTML. Wyłącznie odczyt na końcu testu, za istniejącym In_Diag.
void ZrzucLedgerBrokera()
  {
   if(!In_Diag || g_handle_diag == INVALID_HANDLE) return;
   bool cost_history_ok = HistorySelect(0, TimeCurrent() + 86400);
   // Diagnostic schema only: trading logic is unchanged. Missing old fee
   // columns must not be interpreted as broker-confirmed zero cost.
   FileWrite(g_handle_diag, "COST_DIAG_SCHEMA", "3", cost_history_ok ? "1" : "0");
   ResetLastError();
   string cost_currency = AccountInfoString(ACCOUNT_CURRENCY);
   bool cost_currency_ok = (GetLastError() == 0 && StringLen(cost_currency) > 0);
   FileWrite(g_handle_diag, "COST_DIAG_CURRENCY", cost_currency);
   int cost_candidates = 0, cost_exported = 0, cost_read_errors = 0;
   for(int j = 0; j < g_nrej; j++)
      FileWrite(g_handle_diag, "POSITION_MAP", (string)g_rej_tk[j],
                (string)g_rej_bid[j], (string)g_rej_msg[j]);
   for(int i = 0; cost_history_ok && i < HistoryDealsTotal(); i++)
     {
      ulong d = HistoryDealGetTicket(i);
      long magic = 0;
      if(d == 0 || !HistoryDealGetInteger(d, DEAL_MAGIC, magic))
        {
         cost_read_errors++;
         FileWrite(g_handle_diag, "COST_DIAG_READ_ERROR", (string)d, "ticket_or_magic");
         continue;
        }
      if(magic != In_Magic) continue;
      cost_candidates++;
      long ts = 0, pos = 0, entry = 0, side = 0, reason = 0;
      double volume = 0, price = 0, profit = 0, swap = 0, commission = 0, fee = 0;
      string comment = "";
      // bool overloads distinguish confirmed zero from failed property reads.
      // Call every getter even when an earlier one failed; never export defaults.
      bool ok = HistoryDealGetInteger(d, DEAL_TIME_MSC, ts);
      ok = HistoryDealGetInteger(d, DEAL_POSITION_ID, pos) && ok;
      ok = HistoryDealGetInteger(d, DEAL_ENTRY, entry) && ok;
      ok = HistoryDealGetInteger(d, DEAL_TYPE, side) && ok;
      ok = HistoryDealGetInteger(d, DEAL_REASON, reason) && ok;
      ok = HistoryDealGetDouble(d, DEAL_VOLUME, volume) && ok;
      ok = HistoryDealGetDouble(d, DEAL_PRICE, price) && ok;
      ok = HistoryDealGetDouble(d, DEAL_PROFIT, profit) && ok;
      ok = HistoryDealGetDouble(d, DEAL_SWAP, swap) && ok;
      ok = HistoryDealGetDouble(d, DEAL_COMMISSION, commission) && ok;
      ok = HistoryDealGetDouble(d, DEAL_FEE, fee) && ok;
      ok = HistoryDealGetString(d, DEAL_COMMENT, comment) && ok;
      ok = ok && MathIsValidNumber(volume) && MathIsValidNumber(price)
              && MathIsValidNumber(profit) && MathIsValidNumber(swap)
              && MathIsValidNumber(commission) && MathIsValidNumber(fee);
      if(!ok)
        {
         cost_read_errors++;
         FileWrite(g_handle_diag, "COST_DIAG_READ_ERROR", (string)d, "property_read_or_nonfinite");
         continue;
        }
      FileWrite(g_handle_diag, "DEAL", (string)ts, (string)d, (string)pos,
                (string)entry, (string)side, DoubleToString(volume, 8),
                DoubleToString(price, 8), DoubleToString(profit, 8),
                DoubleToString(swap, 8), DoubleToString(commission, 8),
                (string)reason, comment, DoubleToString(fee, 8));
      cost_exported++;
     }
   FileWrite(g_handle_diag, "COST_DIAG_READ_PROOF", "1", (string)cost_candidates,
             (string)cost_exported, (string)cost_read_errors, cost_currency_ok ? "1" : "0");
   for(int i = 0; i < HistoryOrdersTotal(); i++)
     {
      ulong o = HistoryOrderGetTicket(i);
      if(HistoryOrderGetInteger(o, ORDER_MAGIC) != In_Magic) continue;
      FileWrite(g_handle_diag, "ORDER", (string)o,
                (string)HistoryOrderGetInteger(o, ORDER_TIME_SETUP_MSC),
                (string)HistoryOrderGetInteger(o, ORDER_TIME_DONE_MSC),
                (string)HistoryOrderGetInteger(o, ORDER_TYPE),
                (string)HistoryOrderGetInteger(o, ORDER_STATE),
                DoubleToString(HistoryOrderGetDouble(o, ORDER_VOLUME_INITIAL), 8),
                DoubleToString(HistoryOrderGetDouble(o, ORDER_PRICE_OPEN), 8),
                DoubleToString(HistoryOrderGetDouble(o, ORDER_SL), 8),
                DoubleToString(HistoryOrderGetDouble(o, ORDER_TP), 8));
     }
  }

struct FinalPositionSnapshot
  { ulong ticket; long side; double volume; double price; long ts; double sl; double tp; double profit; double swap; };
struct FinalOrderSnapshot
  { ulong ticket; long type; double volume; double price; long ts; double sl; double tp; };
FinalPositionSnapshot g_final_positions[];
FinalOrderSnapshot g_final_orders[];
bool g_final_account_seen = false;
double g_final_balance, g_final_equity, g_final_margin;
long g_final_tick_ts = 0;

void WriteDailyAccountSnapshot()
  {
   if(!g_final_account_seen || g_handle_diag == INVALID_HANDLE) return;
   FileWrite(g_handle_diag, "DAY_END_ACCOUNT", (string)g_final_tick_ts,
             DoubleToString(g_final_balance, 8), DoubleToString(g_final_equity, 8),
             DoubleToString(g_final_margin, 8), (string)ArraySize(g_final_positions),
             (string)ArraySize(g_final_orders));
  }

void CaptureFinalBrokerState()
  {
   if(!In_Diag || g_handle_diag == INVALID_HANDLE) return;
   // The tester liquidates before OnDeinit. Capture the actual end-of-tick
   // portfolio here; querying the account in OnDeinit would silently look flat.
   if(g_final_account_seen && DayOf(g_now) != DayOf(g_final_tick_ts)) WriteDailyAccountSnapshot();
   g_final_account_seen = true;
   g_final_tick_ts = g_now;
   g_final_balance = AccountInfoDouble(ACCOUNT_BALANCE);
   g_final_equity = AccountInfoDouble(ACCOUNT_EQUITY);
   g_final_margin = AccountInfoDouble(ACCOUNT_MARGIN);
   ArrayResize(g_final_positions, PositionsTotal());
   int n = 0;
   for(int i = 0; i < PositionsTotal(); i++)
     {
      ulong t = PositionGetTicket(i);
      if(t == 0 || PositionGetString(POSITION_SYMBOL) != _Symbol
         || PositionGetInteger(POSITION_MAGIC) != In_Magic) continue;
      g_final_positions[n].ticket = t;
      g_final_positions[n].side = PositionGetInteger(POSITION_TYPE);
      g_final_positions[n].volume = PositionGetDouble(POSITION_VOLUME);
      g_final_positions[n].price = PositionGetDouble(POSITION_PRICE_OPEN);
      g_final_positions[n].ts = PositionGetInteger(POSITION_TIME_MSC);
      g_final_positions[n].sl = PositionGetDouble(POSITION_SL);
      g_final_positions[n].tp = PositionGetDouble(POSITION_TP);
      g_final_positions[n].profit = PositionGetDouble(POSITION_PROFIT);
      g_final_positions[n].swap = PositionGetDouble(POSITION_SWAP);
      n++;
     }
   ArrayResize(g_final_positions, n);
   ArrayResize(g_final_orders, OrdersTotal());
   n = 0;
   for(int i = 0; i < OrdersTotal(); i++)
     {
      ulong t = OrderGetTicket(i);
      if(t == 0 || OrderGetString(ORDER_SYMBOL) != _Symbol
         || OrderGetInteger(ORDER_MAGIC) != In_Magic) continue;
      g_final_orders[n].ticket = t;
      g_final_orders[n].type = OrderGetInteger(ORDER_TYPE);
      g_final_orders[n].volume = OrderGetDouble(ORDER_VOLUME_CURRENT);
      g_final_orders[n].price = OrderGetDouble(ORDER_PRICE_OPEN);
      g_final_orders[n].ts = OrderGetInteger(ORDER_TIME_SETUP_MSC);
      g_final_orders[n].sl = OrderGetDouble(ORDER_SL);
      g_final_orders[n].tp = OrderGetDouble(ORDER_TP);
      n++;
     }
   ArrayResize(g_final_orders, n);
  }

void ZrzucStanBrokera()
  {
   if(!In_Diag || g_handle_diag == INVALID_HANDLE || !g_final_account_seen) return;
   WriteDailyAccountSnapshot();
   FileWrite(g_handle_diag, "FINAL_ACCOUNT", (string)g_now,
             DoubleToString(g_bid, 8), DoubleToString(g_ask, 8),
             DoubleToString(g_final_balance, 8), DoubleToString(g_final_equity, 8),
             DoubleToString(g_final_margin, 8));
   for(int i = 0; i < ArraySize(g_final_positions); i++)
      FileWrite(g_handle_diag, "OPEN_POSITION", (string)g_final_positions[i].ticket,
                (string)g_final_positions[i].side, DoubleToString(g_final_positions[i].volume, 8),
                DoubleToString(g_final_positions[i].price, 8), (string)g_final_positions[i].ts,
                DoubleToString(g_final_positions[i].sl, 8), DoubleToString(g_final_positions[i].tp, 8),
                DoubleToString(g_final_positions[i].profit, 8), DoubleToString(g_final_positions[i].swap, 8));
   for(int i = 0; i < ArraySize(g_final_orders); i++)
      FileWrite(g_handle_diag, "OPEN_ORDER", (string)g_final_orders[i].ticket,
                (string)g_final_orders[i].type, DoubleToString(g_final_orders[i].volume, 8),
                DoubleToString(g_final_orders[i].price, 8), (string)g_final_orders[i].ts,
                DoubleToString(g_final_orders[i].sl, 8), DoubleToString(g_final_orders[i].tp, 8));
  }

void OnDeinit(const int reason)
  {
   PrintFormat("OPEN_VOLUME_AUDIT requests=%I64d max_transmitted=%.8f max_accepted_request=%.8f cap=%.8f cap_exceeded=%I64d",
               g_open_request_count, g_open_request_max_volume, g_open_accepted_max_volume,
               In_LotMax, g_open_request_cap_exceeded);
   // Ostatnia migawka planu po wszystkich edycjach/relotach. x_diff porównuje
   // ją z finalnym `koszyki.json`, zamiast mieszać początkowy plan EA z
   // końcowym stanem Rust.
   for(int bi = 0; bi < g_nb; bi++) DiagKoszyk(bi, "FINAL");
   ZrzucStanBrokera();
   ZrzucWyniki();
   ZrzucLedgerBrokera();
   if(g_handle_diag != INVALID_HANDLE) FileClose(g_handle_diag);
   PrintFormat("CONDUIT_XT %s: sygnalow=%d koszykow=%d zlecen=%d odrzucen=%d scalen=%d",
               XT_WERSJA, (int)g_cnt_sig, (int)g_cnt_basket, (int)g_cnt_order,
               (int)g_cnt_reject, (int)g_merges);
   for(int i = 0; i < g_nkod; i++)
      PrintFormat("  ODRZUCENIE BROKERA kod=%d razy=%d", g_kod[i], (int)g_kod_n[i]);
   for(int i = 0; i < g_nkod; i++)
      if(g_kod[i] == TRADE_RETCODE_LIMIT_ORDERS)
         PrintFormat("BROKER_PENDING_LIMIT rejected=%d account_limit=%d", (int)g_kod_n[i], (int)AccountInfoInteger(ACCOUNT_LIMIT_ORDERS));
   PrintFormat("  MIN_EQUITY=%.2f", g_min_equity);
   PrintFormat("EQ_STAT hi=%.2f lo=%.2f bal=%.2f",
               (g_eq_hi < -1e17 ? AccountInfoDouble(ACCOUNT_BALANCE) : g_eq_hi),
               (g_eq_lo >  1e17 ? AccountInfoDouble(ACCOUNT_BALANCE) : g_eq_lo),
               AccountInfoDouble(ACCOUNT_BALANCE));
   // PREMIA WYPEŁNIEŃ (D3) — sędzia dosypuje na limitach; mierzymy per przebieg
   PrintFormat("PREMIA_FILL n=%d lepiej=%d usd=%.2f",
               g_premia_n, g_premia_lepiej, g_premia_usd);
   PrintFormat("  ODRZUCONE ZLECENIA=%d | odrzucone modyfikacje SL/TP=%d | nieudane ponowienia=%d",
               (int)g_rej_place, (int)g_rej_modify, (int)g_retry_fail);
   PrintFormat("  nieudane zamkniecia=%d  nieudane modyfikacje=%d",
               (int)g_zamk_blad, (int)g_mod_blad);
   PrintFormat("  odrzucenia: sesja=%d rezim=%d seria=%d maxpoz=%d maxkoszyk=%d slprzebity=%d ryzyko=%d broker=%d",
               (int)g_rej_session, (int)g_rej_regime, (int)g_rej_streak,
               (int)g_rej_maxpos, (int)g_rej_maxbask, (int)g_rej_slbreach,
               (int)g_rej_risk, (int)g_rej_broker);
   PrintFormat("  odrzucenia2: daystop=%d budzet=%d jakosc=%d trend=%d ml=%d margincall=%d floor=%d slhit=%d",
               (int)g_rej_daystop, (int)g_rej_budget, (int)g_rej_jakosc,
               (int)g_rej_trend, (int)g_rej_ml, (int)g_rej_margincall, (int)g_rej_floor,
               (int)g_rej_slhit);
   // ODBIOR PORTU: zera znacza cichy no-op (najczestsza przyczyna — zbyt krotki
   // rozbieg: ZakresOknaGlownego potrzebuje regime_ma_hours probek godzinowych).
   PrintFormat("  rezim: slhit_odrzucen=%d wyciszen_miekkich=%d slhit_dnia=%d",
               (int)g_rej_slhit, (int)g_wyciszen, g_slhit_dnia);
   PrintFormat("  reguly: trail=%d belock=%d smartsl=%d harvest=%d stale=%d smartexit=%d vsl=%d",
               (int)g_cnt_trail_mod, (int)g_cnt_belock, (int)g_cnt_smart_sl,
               (int)g_cnt_harvest, (int)g_cnt_stale, (int)g_cnt_smartexit, (int)g_cnt_vsl);
   PrintFormat("  reguly2: rf_rule=%d rf_maxhold=%d piramida=%d rearm=%d fastaddon=%d revexit=%d oae_t=%d",
               (int)g_cnt_rf_rule, (int)g_cnt_rf_maxhold, (int)g_cnt_piramida,
               (int)g_cnt_rearm, (int)g_cnt_fastaddon, (int)g_cnt_revexit, (int)g_cnt_oae_timeout);
   PrintFormat("  reguly3: zoneexit=%d enforce=%d ttl=%d grace=%d expo_zd=%d expo_pend=%d expo_poz=%d",
               (int)g_cnt_zoneexit, (int)g_cnt_enforce, (int)g_cnt_ttl, (int)g_cnt_grace,
               (int)g_expo_zdarzen, (int)g_expo_pend_skas, (int)g_expo_poz_domk);
   PrintFormat("  relot: up=%d down=%d ok=%d odmowy=%d lotow=%.2f",
               g_relot_up, g_relot_down, g_relot_ok, g_relot_odmowy, g_relot_lotow);
   if(StringLen(g_halted) > 0) PrintFormat("  HALTED: %s", g_halted);
  }

//====================================================================
//  ONTICK — kolejność wg engine.rs:5038 (34 kroki, jednoformatowo)
//====================================================================
void PrzetworzWiadomosciCzasu()
  {
   while(g_mi < g_nmsg && g_msg[g_mi].ts + In_ExecLatencyMs <= g_now)
     {
      if(In_Diag && g_handle_diag != INVALID_HANDLE)
         FileWrite(g_handle_diag, "MESSAGE", (string)g_now,
                   (string)(g_msg[g_mi].ts + In_ExecLatencyMs),
                   (string)g_msg[g_mi].msg_id, (string)g_msg[g_mi].reply_to,
                   (string)g_msg[g_mi].edit_of, (string)g_msg[g_mi].n,
                   DoubleToString(g_bid, 8), DoubleToString(g_ask, 8));
      WykonajWiadomosc(g_mi);
      g_mi++;
      OdswiezBilety();
     }
  }

void OnTick()
  {
   MqlTick tk;
   if(!SymbolInfoTick(_Symbol, tk)) return;
   g_bid = tk.bid; g_ask = tk.ask;
   g_now = (long)tk.time_msc;
   if(In_TestExitScenario > 0)
     { if(g_now <= 0) g_now = (long)tk.time * 1000; ExitFaultScenarioTick(); CaptureFinalBrokerState(); return; }
   SrNaTicku(g_now, (g_bid + g_ask) * 0.5);
   if(g_now <= 0) g_now = (long)tk.time * 1000;

   double eq = AccountInfoDouble(ACCOUNT_EQUITY);
   if(eq < g_min_equity) g_min_equity = eq;
   if(g_dzien_od <= 0 || (g_now >= g_dzien_od && (g_dzien_do <= 0 || g_now < g_dzien_do)))
     {
      if(eq > g_eq_hi) g_eq_hi = eq;
      if(eq < g_eq_lo) g_eq_lo = eq;
     }
   if(eq > g_peak_equity) g_peak_equity = eq;

   // 4. rekoncyliacja (fill pending->pozycja, tp_stage_from_broker_fill)
   OdswiezBilety();
   RetryConfirmedExits();
   RetrySourceCancellations();
   CoverLateFills();
   // 6. pauza po serii strat
   AktualizujSerie();
   // 7. rolka doby (reset day_*, zdjęcie blokady)
   RolkaDoby();
   if(eq > g_day_peak_eq) g_day_peak_eq = eq;
   // 8. historia rynku + bufor zmienności
   PushHist();
   PushVolHist();
   // 11. szczyty pozycji + dotknięcia celów
   UpdatePeaks();
   // 12. strażnicy
   CheckGuards();
   // 13. ponowienia SL/TP
   PonowStopy();
   // 14. reguła risk-free
   RiskfreePass();
   // 15. zarządzanie pozycjami (vsl, wyjścia, trailing)
   ManagePositions();

   // Legacy XT dzielił zarządzanie tickiem wiadomościami w środku.
   // Zostaje odtwarzalny przy false; nie jest parytetem strict runnera.
   if(!In_LiveTickOrderStrict) PrzetworzWiadomosciCzasu();

   // KONIEC DOBY (tryb dzienny) — koszyk przez północ jest UCINANY
   if(g_dzien_do > 0 && g_now >= g_dzien_do && !g_doba_zamknieta)
     {
      g_doba_zamknieta = true;
      g_powod_zamk = "EOD";
      CloseEverything();
      CaptureFinalBrokerState();
      return;
     }

   // 16. wykrywanie celów z ceny
   WykryjCeleZCeny();
   // 17. TTL pendingów
   PendingTtl();
   // 20. wygaszanie koszyków bez handlu
   ExpireStale();
   // 21. odroczone kasowanie (okno łaski)
   DokonczOdroczoneKasowanie();
   // 22. twardy limit wieku
   ExpireOld();
   // 23. filtr tempa
   FastFillCheck();
   // 24. egzekwowany limit pozycji
   EnforcePositionLimit();
   // 25. trwałe wyjście ze strefy
   ZoneExitAdverseSweep();
   // 26. dokładka tempowa
   FastAddonSweep();
   // 28. relot pendingów
   RelotPendings();
   // 29. redukcja ekspozycji — PO relocie, PRZED rearm/drabiną/reentry!
   RedukujEkspozycje();
   // 30. przezbrojenie siatki
   RearmPass();
   // 31. drabina rynkowa (Laddered)
   MarketLadderPass();
   // 32. re-entry
   ReentryPass();
   // 33. reversal-exit
   RevExitSweep();
   // 34. sprzątanie pustych koszyków. Domyślnie odwzorowuje historyczny Rust
   //  1:1. Jawna oś zachowuje wyłącznie koszyk, który już handlował i czeka
   //  na legalny powrót ceny dla `RearmPass`.
   for(int bi = 0; bi < g_nb; bi++)
     {
      bool czeka_na_rearm = In_RearmGridOnReturn && In_RearmKeepEmpty
                            && g_b[bi].had_positions;
      if(Alive(bi) && ExitRiskAllowed(bi) && !czeka_na_rearm && g_b[bi].npos == 0 && g_b[bi].npend == 0
         && g_now - g_b[bi].created_ts > 60000)
         g_b[bi].state = ST_DONE;
     }
   // Parytet kolejności z Rust/live: broker wykonał istniejące SL/TP i
   // pendingi PRZED OnTick, teraz zakończyło się CAŁE zarządzanie tickiem.
   // Nowa wiadomość nie może dostać ponownego TP/TTL/rearm na tym ticku.
   if(In_LiveTickOrderStrict) PrzetworzWiadomosciCzasu();
   CaptureFinalBrokerState();
  }
//+------------------------------------------------------------------+
