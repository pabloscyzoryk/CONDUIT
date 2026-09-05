
use conduit_backtest::data::load_messages;
use conduit_backtest::sim::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::{Engine, IncomingMessage};
use conduit_core::parser::{self, OpcjeParsera, Signal};
use conduit_core::settings::*;
use conduit_core::types::*;
use std::path::PathBuf;

const T0: Ts = 1_800_000_000_000;
const ENTRY: &str = "BUY GOLD @ 4000/3995\nTP 4010\nTP 4020\nTP 4030\nSL 3990";
const LIMIT: &str = "BUY LIMITS GOLD @ 4000/3995\nTP 4010\nTP 4020\nTP 4030\nSL 3990";
const AREA_BEFORE_AT: &str = "BUY LIMITS GOLD AREA @ 4000/3995\nTP 4010\nTP 4020\nTP 4030\nSL 3990";
const OFFSET_ENTRY: &str = "SELL LIMITS GOLD @ 2138/2143 AREA\n\n\
TP 2136\nTP 2133\nTP 2129\nTP OPEN\nSL 2144\n\n\
HIGH RISK TRADE\n\n\
SUBTRACTING 3 PIPS FROM EACH LIMIT ORDER\n\n\
SYNTHETIC FIXTURE";
const PARTIAL_PLAN: &str = "AT TP3 +100 PIPS\n\n\
SECURING PARTIAL PROFITS. SL IS SET TO BE AT 2139 AND I WILL TARGET;\n\n\
2120\n2110\n2100\n2090\n2080";
const PARTIAL_FINAL: &str = "TP3 HIT +100 PIPS\n\n\
SECURING PARTIAL PROFITS. SL IS SET TO BE AT 2139 AND I WILL TARGET;\n\n\
2120\n2110\n2100\n2090\n2080";

fn source() -> SourceKey {
    SourceKey::new(-1_000_000_000_401, None)
}

fn message(ts: Ts, id: i64, text: &str) -> IncomingMessage {
    IncomingMessage {
        ts,
        source: source(),
        source_name: "SyntheticFormat-Adversarial".into(),
        msg_id: id,
        reply_to: None,
        edit_of: None,
        text: text.into(),
    }
}

fn reply(ts: Ts, id: i64, parent: i64, text: &str) -> IncomingMessage {
    let mut m = message(ts, id, text);
    m.reply_to = Some(parent);
    m
}

fn edit(ts: Ts, id: i64, parent: Option<i64>, text: &str) -> IncomingMessage {
    let mut m = message(ts, id, text);
    m.reply_to = parent;
    m.edit_of = Some(id);
    m
}

fn quote(ts: Ts, bid: Px) -> Quote {
    Quote {
        ts,
        bid,
        ask: bid + 0.20,
    }
}

fn safe_settings() -> Settings {
    let mut c = Settings::default();
    c.entry_units = 4;
    c.auto_limit = false;
    c.lot_mode_percent = false;
    c.lot_fixed = 0.01;
    c.lot_min = 0.01;
    c.risk_per_basket_pct = 0.0;
    c.max_portfolio_risk_pct = 0.0;
    c.max_open_positions = 0;
    c.max_open_baskets = 0;
    c.assign_tp_per_position = false;
    c.pending_lifetime = PendingLifetime::Never;
    c.pending_drop_on_target = false;
    c.ignore_old_after_min = 0.0;

    c.reply_veto = true;
    c.hint_veto = true;
    c.reply_graph_transitive = true;
    c.dedup_edited_signals = true;
    c.dedup_pelny_status = true;
    c.edycja_wykonuje_reszte_akcji = true;
    c.dedup_klucz_z_wartoscia = true;
    c.edycja_sieroty_nie_otwiera = true;
    c.entry_idempotencja = true;
    c.dedup_management_po_restarcie = true;
    c.recap_guard = true;
    c.parser_luz_interpunkcyjny = true;
    c.tp_hit_match_level = true;
    c.tp_unindexed_pips_require_price = true;
    c.rf_level_sanity_max_usd = 20.0;
    c
}

fn station(bid: Px) -> (Engine, SimBroker) {
    let cfg = safe_settings();
    let mut b = SimBroker::z_ustawien(1_000.0, &cfg);
    b.on_quote(quote(T0, bid));
    (Engine::new(cfg, 1_000.0), b)
}

fn tick(e: &mut Engine, b: &mut SimBroker, ts: Ts, bid: Px) {
    let q = quote(ts, bid);
    b.on_quote(q);
    e.on_tick(b, &q);
}

#[test]
fn edit_before_new_fail_closed_then_new_and_redelivery_are_idempotent() {
    let (mut e, mut b) = station(4008.0);

    // Edycja-sierota ma pełną geometrię, ale nie zna oryginału.
    e.on_message(&mut b, &edit(T0 + 1_000, 100, None, AREA_BEFORE_AT));
    assert!(e.baskets.is_empty());
    assert!(b.positions().is_empty() && b.pendings().is_empty());
    assert_eq!(e.odrzuty.get("EditOrphan"), Some(&1));

    // Późniejszy prawdziwy NEW przechodzi; identyczna re-delivery nie tworzy
    // drugiego koszyka. Oś parsera czyta dokładny szyk `GOLD AREA @`.
    e.on_message(&mut b, &message(T0 + 2_000, 100, AREA_BEFORE_AT));
    assert_eq!(e.baskets.len(), 1);
    assert!(!b.pendings().is_empty());
    e.on_message(&mut b, &message(T0 + 3_000, 100, AREA_BEFORE_AT));
    assert_eq!(
        e.baskets.len(),
        1,
        "re-delivery entry nie może podwoić ekspozycji"
    );

    // Edycja tego samego wejścia zmienia plan, ale nadal nie tworzy koszyka.
    let changed = AREA_BEFORE_AT.replace("SL 3990", "SL 3991");
    e.on_message(&mut b, &edit(T0 + 4_000, 100, None, &changed));
    assert_eq!(e.baskets.len(), 1);
    assert_eq!(e.baskets[0].sl, Some(3991.0));
}

#[test]
fn reply_chain_cycle_wrong_parent_hint_veto_and_restart_are_deterministic() {
    let (mut e, mut b) = station(3998.0);
    e.on_message(&mut b, &message(T0, 100, ENTRY));
    assert_eq!(e.baskets.len(), 1);

    e.on_message(&mut b, &reply(T0 + 1_000, 101, 100, "MOVE SL TO 3992"));
    e.on_message(&mut b, &reply(T0 + 2_000, 102, 101, "MOVE SL TO 3993"));
    assert_eq!(e.baskets[0].sl, Some(3993.0), "łańcuch 102 -> 101 -> entry");

    // Dwie edycje tworzą cykl reply 101 <-> 102. Silnik nie wspina się po
    // grafie w pętli: oba id są już trwałymi aliasami jednego koszyka.
    let mut a = edit(T0 + 3_000, 101, Some(102), "MOVE SL TO 3994");
    a.reply_to = Some(102);
    e.on_message(&mut b, &a);
    let mut c = edit(T0 + 4_000, 102, Some(101), "MOVE SL TO 3995");
    c.reply_to = Some(101);
    e.on_message(&mut b, &c);
    assert_eq!(e.baskets[0].sl, Some(3995.0));

    // Jawnie błędny parent nie może spaść na „najnowszy koszyk".
    e.on_message(&mut b, &reply(T0 + 5_000, 200, 999_999, "MOVE SL TO 3996"));
    assert_eq!(e.baskets[0].sl, Some(3995.0));
    // Tak samo wskazówka cenowa niepasująca do koszyka.
    e.on_message(&mut b, &message(T0 + 6_000, 201, "MOVE SL TO 4500"));
    assert_eq!(e.baskets[0].sl, Some(3995.0));

    // Alias zarządzania przeżywa rekonstrukcję z koszyka.
    let snapshot = e.baskets[0].clone();
    let mut after = Engine::new(safe_settings(), 1_000.0);
    after.adopt_baskets(vec![snapshot]);
    after.on_message(&mut b, &reply(T0 + 7_000, 202, 102, "MOVE SL TO 3996"));
    assert_eq!(after.baskets[0].sl, Some(3996.0));
}

#[test]
fn duplicate_edit_same_value_is_noop_but_changed_value_executes_once() {
    let (mut e, mut b) = station(3998.0);
    e.on_message(&mut b, &message(T0, 100, ENTRY));
    e.on_message(&mut b, &reply(T0 + 1_000, 200, 100, "MOVE SL TO 3992"));
    assert_eq!(e.baskets[0].sl, Some(3992.0));

    e.on_message(&mut b, &edit(T0 + 2_000, 200, Some(100), "MOVE SL TO 3992"));
    assert_eq!(
        e.baskets[0].sl,
        Some(3992.0),
        "identyczna akcja ma być dedupowana"
    );
    e.on_message(&mut b, &edit(T0 + 3_000, 200, Some(100), "MOVE SL TO 3993"));
    assert_eq!(
        e.baskets[0].sl,
        Some(3993.0),
        "nowa wartość nie może zginąć w dedupie"
    );
    e.on_message(&mut b, &edit(T0 + 4_000, 200, Some(100), "MOVE SL TO 3993"));
    assert_eq!(e.baskets[0].sl, Some(3993.0));
}

#[test]
fn recap_rf_typo_and_tp_forms_are_fail_closed_until_hard_evidence() {
    let options = OpcjeParsera {
        luz_interpunkcyjny: true,
        recap_guard: true,
        ..Default::default()
    };
    for recap in [
        "DAILY RECAP: synthetic BUY 2090-2094, TP1 HIT; session summary, no fresh entry.",
        "THATS 62 TPS HIT ALREADY TODAY TRADERS",
    ] {
        assert!(
            parser::parse_z_opcjami(recap, options)
                .iter()
                .all(|s| matches!(s, Signal::Info)),
            "recap nie może zarządzać koszykiem: {recap}"
        );
    }

    let (mut e, mut b) = station(3998.0);
    e.on_message(&mut b, &message(T0, 100, ENTRY));
    assert!(!b.positions().is_empty());

    e.on_message(&mut b, &reply(T0 + 1_000, 101, 100, "RISK FREE 4967"));
    assert!(
        !e.baskets[0].secured,
        "literówka RF daleko od rynku ma być odrzucona"
    );

    e.on_message(&mut b, &reply(T0 + 2_000, 102, 100, "+50 PIPS HIT"));
    assert_eq!(e.baskets[0].tp_stage, 0, "pips bez ceny nie awansuje celu");

    // Numerowany TP jest idempotentny i nie wymaga zgadywania kolejnego etapu.
    e.on_message(&mut b, &reply(T0 + 3_000, 103, 100, "TP1 HIT"));
    e.on_message(&mut b, &edit(T0 + 4_000, 103, Some(100), "TP1 HIT"));
    assert_eq!(e.baskets[0].tp_stage, 1);

    // Po twardym kwotowaniu unindexed może potwierdzić NASTĘPNY etap.
    b.on_quote(quote(T0 + 5_000, 4021.0));
    e.on_message(&mut b, &reply(T0 + 5_000, 104, 100, "+100 PIPS HIT"));
    assert_eq!(e.baskets[0].tp_stage, 2);
}

/// GOD-X5: `AT TP` jest tylko proximity, a jedynym arbitrem wykonania celu
/// jest biezacy Bid/Ask. Numerowane `TPn HIT` pozostaje informacja, lecz bez
/// ceny nie teleportuje etapu. Sam tick wykonuje TP bez czekania na Telegram.
#[test]
fn at_tp_is_telemetry_and_mt5_price_is_tp_authority() {
    let mut cfg = safe_settings();
    cfg.profit_update_telemetry_only = true;
    cfg.tp_source = TpSource::SignalConfirmedByPrice;
    cfg.tp_unindexed_pips_require_price = true;
    let mut b = SimBroker::z_ustawien(1_000.0, &cfg);
    b.on_quote(quote(T0, 3998.0));
    let mut e = Engine::new(cfg, 1_000.0);

    e.on_message(&mut b, &message(T0, 100, ENTRY));
    assert_eq!(e.baskets[0].tp_stage, 0);

    // Rzeczywisty ksztalt Synergy: pierwsza wersja AT, potem edit do HIT.
    e.on_message(&mut b, &reply(T0 + 1_000, 101, 100, "AT TP1 +20 PIPS 🔥"));
    assert_eq!(e.baskets[0].tp_stage, 0, "AT TP nie jest trafieniem");
    e.on_message(
        &mut b,
        &edit(T0 + 2_000, 101, Some(100), "TP1 HIT +20 PIPS 🔥"),
    );
    assert_eq!(
        e.baskets[0].tp_stage, 0,
        "Telegram TP1 HIT bez brokerowego dotkniecia nie moze teleportowac etapu"
    );
    // Bare/unindexed +PIPS tak samo wymaga biezacej ceny.
    e.on_message(&mut b, &reply(T0 + 3_000, 102, 100, "+50 PIPS HIT 🔥"));
    assert_eq!(e.baskets[0].tp_stage, 0);

    // Cena dotyka celu bez zadnej kolejnej wiadomosci Telegram — rdzen sam
    // bankuje TP1 na fakcie z MT5.
    tick(&mut e, &mut b, T0 + 4_000, 4010.0);
    assert_eq!(e.baskets[0].tp_stage, 1, "tick MT5 musi sam wykonac TP1");
}

/// Autonomiczny front-run nie potrzebuje Telegrama ani nawet cenowego
/// `tp_source`. Dokładne zero zachowuje pełne dotknięcie, a wartość dodatnia
/// wykonuje ten sam idempotentny etap wcześniej na Bid/Ask brokera.
#[test]
fn autonomous_mt5_tp_front_run_is_optional_price_only_and_idempotent() {
    // Kontrakt zera w normalnym trybie cenowym: 0.19 USD przed celem to nadal
    // etap 0; dopiero pełne dotknięcie wykonuje TP1.
    let mut legacy = safe_settings();
    legacy.tp_source = TpSource::PriceOnly;
    legacy.tp_price_front_run_usd = 0.0;
    let mut b0 = SimBroker::z_ustawien(1_000.0, &legacy);
    b0.on_quote(quote(T0, 3998.0));
    let mut e0 = Engine::new(legacy, 1_000.0);
    e0.on_message(&mut b0, &message(T0, 100, ENTRY));
    tick(&mut e0, &mut b0, T0 + 1_000, 4009.81);
    assert_eq!(e0.baskets[0].tp_stage, 0);
    tick(&mut e0, &mut b0, T0 + 2_000, 4010.0);
    assert_eq!(e0.baskets[0].tp_stage, 1);

    // Oś dodatnia jest autonomiczna: działa nawet przy SignalOnly, bez
    // jakiejkolwiek wiadomości zarządzającej.
    let mut cfg = safe_settings();
    cfg.tp_source = TpSource::SignalOnly;
    cfg.tp_price_front_run_usd = 0.20;
    let mut b = SimBroker::z_ustawien(1_000.0, &cfg);
    b.on_quote(quote(T0, 3998.0));
    let mut e = Engine::new(cfg, 1_000.0);
    e.on_message(&mut b, &message(T0, 100, ENTRY));
    tick(&mut e, &mut b, T0 + 1_000, 4009.81);
    assert_eq!(e.baskets[0].tp_stage, 1);
    let realized = e.baskets[0].realized;

    // Powtórzenie ticku nie może drugi raz pobrać tej samej transzy.
    tick(&mut e, &mut b, T0 + 2_000, 4009.81);
    assert_eq!(e.baskets[0].tp_stage, 1);
    assert_eq!(e.baskets[0].realized, realized);
}

/// Filtr AT TP usuwa tylko TpHit. Jawna akcja zarzadzania dopisana do tej
/// samej NEW/EDIT nadal przechodzi normalnym routingiem.
#[test]
fn at_tp_multi_intent_keeps_explicit_management_clause() {
    let mut cfg = safe_settings();
    cfg.profit_update_telemetry_only = true;
    cfg.tp_source = TpSource::SignalConfirmedByPrice;
    let mut b = SimBroker::z_ustawien(1_000.0, &cfg);
    b.on_quote(quote(T0, 3998.0));
    let mut e = Engine::new(cfg, 1_000.0);

    e.on_message(&mut b, &message(T0, 100, ENTRY));
    e.on_message(
        &mut b,
        &reply(T0 + 1_000, 101, 100, "AT TP1 +20 PIPS 🔥\nMOVE SL TO 3992"),
    );
    assert_eq!(e.baskets[0].tp_stage, 0, "AT TP ma zostac telemetry");
    assert_eq!(
        e.baskets[0].sl,
        Some(3992.0),
        "jawny MOVE SL z tej samej wiadomosci ma przejsc"
    );
}

/// Przyczynowe A/B dla `AT TP`: legacy-as-hit moze byc swiadomym soft TP,
/// jezeli `SignalConfirmedByPrice` ogranicza je BIEZACYM kwotowaniem i jawna
/// tolerancja. To proximity, nie dowod pelnego touch — dlatego osobny wariant
/// telemetry-only musi zostac w sweepie jako baseline.
#[test]
fn at_tp_soft_hit_is_causal_and_controlled_by_broker_tolerance() {
    for (telemetry_only, tolerance, expected_stage) in [
        (false, 0.10, 0usize), // 0.19 przed TP1: poza pasmem
        (false, 0.20, 1usize),
        (false, 0.30, 1usize), // szerszy causal soft-hit
        (true, 0.30, 0usize),  // baseline telemetry: zawsze bez wykonania
    ] {
        let mut cfg = safe_settings();
        cfg.profit_update_telemetry_only = telemetry_only;
        cfg.tp_source = TpSource::SignalConfirmedByPrice;
        cfg.tp_price_tolerance = tolerance;
        let mut b = SimBroker::z_ustawien(1_000.0, &cfg);
        b.on_quote(quote(T0, 3998.0));
        let mut e = Engine::new(cfg, 1_000.0);
        e.on_message(&mut b, &message(T0, 100, ENTRY));

        tick(&mut e, &mut b, T0 + 1_000, 4009.81);
        assert_eq!(e.baskets[0].tp_stage, 0);
        e.on_message(&mut b, &reply(T0 + 2_000, 101, 100, "AT TP1 +20 PIPS 🔥"));
        assert_eq!(
            e.baskets[0].tp_stage, expected_stage,
            "telemetry_only={telemetry_only}, tolerance={tolerance}"
        );

        if expected_stage == 1 {
            let positions_after_first = b.positions().len();
            let realized_after_first = e.baskets[0].realized;
            e.on_message(
                &mut b,
                &reply(
                    T0 + 3_000,
                    102,
                    100,
                    "AT TP1 +30 PIPS AGAIN AFTER PULLING BACK 🔥",
                ),
            );
            assert_eq!(e.baskets[0].tp_stage, 1);
            assert_eq!(b.positions().len(), positions_after_first);
            assert_eq!(e.baskets[0].realized, realized_after_first);
        }
    }
}

/// Aproksymacja oficjalnej metody z onboarding: AT TP przy bliskiej cenie
/// inkasuje 25%, a pozniejsze TP1 HIT tego samego stage nie inkasuje drugi
/// raz. To nadal legacy `HandleTpHit` (ma tez side-effects etapu), dlatego
/// wariant wolno porownywac tylko z OfficialPct i bez `bank_all_at_stage`.
#[test]
fn at_tp_soft_hit_official_pct_is_idempotent_with_later_tp_hit() {
    let mut cfg = safe_settings();
    cfg.profit_update_telemetry_only = false;
    cfg.tp_source = TpSource::SignalConfirmedByPrice;
    cfg.tp_price_tolerance = 0.20;
    cfg.tp_schedule = TpSchedule::OfficialPct;
    cfg.official_pct = [25.0, 25.0, 25.0, 25.0];
    cfg.bank_all_at_stage = 0;
    cfg.bank_close_last = false;
    let mut b = SimBroker::z_ustawien(1_000.0, &cfg);
    b.on_quote(quote(T0, 3998.0));
    let mut e = Engine::new(cfg, 1_000.0);
    e.on_message(&mut b, &message(T0, 100, ENTRY));
    let before = b.positions().len();
    assert!(before >= 4, "test wymaga czterech warstw do czytelnego 25%");

    tick(&mut e, &mut b, T0 + 1_000, 4009.81);
    e.on_message(&mut b, &reply(T0 + 2_000, 101, 100, "AT TP1 +20 PIPS 🔥"));
    assert_eq!(e.baskets[0].tp_stage, 1);
    let after_at = b.positions().len();
    assert_eq!(
        before - after_at,
        1,
        "AT TP1 ma zbankowac jedna z czterech pozycji"
    );
    let realized_after_at = e.baskets[0].realized;

    e.on_message(&mut b, &reply(T0 + 3_000, 102, 100, "TP1 HIT +20 PIPS 🔥"));
    assert_eq!(e.baskets[0].tp_stage, 1);
    assert_eq!(
        b.positions().len(),
        after_at,
        "TP1 HIT nie moze pobrac tej samej transzy drugi raz"
    );
    assert_eq!(e.baskets[0].realized, realized_after_at);
}

#[test]
fn cancel_be_sl_and_oae_reach_the_expected_engine_handlers() {
    // CANCEL: limitowa siatka znika.
    let (mut e, mut b) = station(4008.0);
    e.on_message(&mut b, &message(T0, 100, LIMIT));
    assert!(!b.pendings().is_empty());
    e.on_message(&mut b, &reply(T0 + 1_000, 101, 100, "CANCEL THE LIMITS"));
    assert!(b.pendings().is_empty());

    // BE: pozycje zostają, ich SL idzie na własne ceny wejścia.
    let (mut e, mut b) = station(3998.0);
    e.on_message(&mut b, &message(T0, 100, ENTRY));
    tick(&mut e, &mut b, T0 + 1_000, 4005.0);
    e.on_message(&mut b, &reply(T0 + 2_000, 101, 100, "SET SL TO BE"));
    assert!(!b.positions().is_empty());
    for p in b.positions() {
        assert_eq!(p.sl, Some(p.open_price));
    }

    // OAE i SL-HIT w jawnych trybach CloseAll zamykają koszyk.
    let (mut e, mut b) = station(3998.0);
    e.cfg.out_at_entry_mode = OutAtEntryMode::CloseAll;
    e.on_message(&mut b, &message(T0, 100, ENTRY));
    e.on_message(
        &mut b,
        &reply(T0 + 1_000, 101, 100, "OUT AT ENTRY ON THE REST"),
    );
    assert!(b.positions().is_empty());

    let (mut e, mut b) = station(3998.0);
    e.cfg.sl_hit_mode = SlHitMode::CloseAll;
    e.on_message(&mut b, &message(T0, 100, ENTRY));
    e.on_message(&mut b, &reply(T0 + 1_000, 101, 100, "SL HIT"));
    assert!(b.positions().is_empty());
}

#[test]
fn raw_replay_uses_receive_seq_and_does_not_double_count_latency() {
    let mut path: PathBuf = std::env::temp_dir();
    path.push(format!(
        "conduit-telegram-adversarial-{}-{}.json",
        std::process::id(),
        T0
    ));
    std::fs::write(
        &path,
        r#"{"messages":[
          {"ts":500,"msg_id":30,"receive_seq":30,"latency_ms":9000,"text":"THIRD","kanal":"Synergy"},
          {"ts":500,"msg_id":10,"receive_seq":10,"latency_ms":12000,"text":"FIRST","kanal":"Synergy"},
          {"ts":500,"msg_id":20,"receive_seq":20,"latency_ms":1,"text":"SECOND","kanal":"Synergy"}
        ]}"#,
    )
    .unwrap();
    let loaded = load_messages(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        loaded.iter().map(|m| m.msg_id).collect::<Vec<_>>(),
        vec![10, 20, 30]
    );
    assert!(
        loaded.iter().all(|m| m.ts == 500),
        "latency to metadana, nie drugi offset"
    );
}

/// Re-delivery tego samego NIENUMEROWANEGO `+PIPS HIT` po restarcie nie może
/// udawać następnego celu, nawet gdy bieżąca cena potwierdza oba szczeble.
#[test]
fn restart_redelivery_unindexed_management_must_not_advance_twice() {
    let (mut e, mut b) = station(3998.0);
    e.on_message(&mut b, &message(T0, 100, ENTRY));
    b.on_quote(quote(T0 + 1_000, 4035.0));
    e.on_message(&mut b, &reply(T0 + 1_000, 101, 100, "+50 PIPS HIT"));
    assert_eq!(e.baskets[0].tp_stage, 1);

    let snapshot = e.baskets[0].clone();
    let mut after = Engine::new(safe_settings(), 1_000.0);
    after.adopt_baskets(vec![snapshot]);
    after.on_message(&mut b, &reply(T0 + 2_000, 101, 100, "+50 PIPS HIT"));
    assert_eq!(
        after.baskets[0].tp_stage, 1,
        "ta sama wiadomość po restarcie nie może udawać następnego TP"
    );
}

/// Kontrakt zera A7: wyłączenie nowej osi przywraca dokładnie dawną lukę.
/// To nie jest oczekiwane ustawienie live, tylko dowód, że mechanizm jest
/// przełączalny i stary preset bez klucza nie zmienia zachowania po cichu.
#[test]
fn restart_management_dedup_legacy_off_contract() {
    let mut cfg = safe_settings();
    cfg.dedup_management_po_restarcie = false;
    let mut b = SimBroker::z_ustawien(1_000.0, &cfg);
    b.on_quote(quote(T0, 3998.0));
    let mut e = Engine::new(cfg.clone(), 1_000.0);
    e.on_message(&mut b, &message(T0, 100, ENTRY));
    b.on_quote(quote(T0 + 1_000, 4035.0));
    e.on_message(&mut b, &reply(T0 + 1_000, 101, 100, "+50 PIPS HIT"));
    assert_eq!(e.baskets[0].tp_stage, 1);
    assert!(
        e.baskets[0].persisted_done_actions.is_empty(),
        "oś OFF nie może zmieniać zrzutu koszyka"
    );

    let snapshot = e.baskets[0].clone();
    let mut after = Engine::new(cfg, 1_000.0);
    after.adopt_baskets(vec![snapshot]);
    after.on_message(&mut b, &reply(T0 + 2_000, 101, 100, "+50 PIPS HIT"));
    assert_eq!(
        after.baskets[0].tp_stage, 2,
        "OFF musi zachować dawny brak pamięci done_actions po adopt"
    );
}

/// Synthetic edit/restart sequence: `AT TP3`, then an edited SPP/BE plan,
/// followed by `TP3 HIT` with the unchanged plan after restart.
#[test]
fn synthetic_edit_after_restart_does_not_repeat_tp3_or_spp() {
    let mut cfg = safe_settings();
    cfg.tp_source = TpSource::SignalOnly;
    cfg.dedup_klucz_z_wartoscia = true;
    // Testuje routing/dedup, nie polityke bankowania: zachowaj runnera przez
    // oba plany, aby powtorne SPP po restarcie mialo realny skutek do wykrycia.
    cfg.scale_out_pct = 0.0;
    cfg.official_pct = [0.0; 4];
    let mut b = SimBroker::z_ustawien(1_000.0, &cfg);
    b.on_quote(quote(T0, 2137.0));
    let mut e = Engine::new(cfg.clone(), 1_000.0);

    e.on_message(&mut b, &message(T0, 7001, OFFSET_ENTRY));
    tick(&mut e, &mut b, T0 + 100, 2143.0);
    assert!(
        !b.positions().is_empty(),
        "synthetic entry must have live exposure for the SPP test"
    );

    e.on_message(&mut b, &reply(T0 + 300, 7002, 7001, "AT TP3 +100 PIPS"));
    assert_eq!(e.baskets[0].tp_stage, 3);
    e.on_message(
        &mut b,
        &edit(T0 + 308, 7002, Some(7001), "AT TP3 +100 PIPS"),
    );
    assert_eq!(
        e.baskets[0].tp_stage, 3,
        "identyczna pierwsza edycja jest noop"
    );

    e.on_message(&mut b, &edit(T0 + 68_187, 7002, Some(7001), PARTIAL_PLAN));
    assert_eq!(
        e.baskets[0].tp_stage, 1,
        "SPP zaklada nowa drabinke i potwierdza jej pierwszy etap"
    );
    let zapis = e.baskets[0]
        .persisted_done_actions
        .iter()
        .find(|x| x.msg_id == 7002)
        .expect("synthetic actions must be persisted");
    assert!(zapis.actions.iter().any(|x| x == "tp3"));
    assert!(zapis.actions.iter().any(|x| x.starts_with("spp@")));

    let snapshot = e.baskets[0].clone();
    let pozycje_przed = b.positions().len();
    let mut after = Engine::new(cfg, 1_000.0);
    after.adopt_baskets(vec![snapshot]);
    after.on_message(
        &mut b,
        &edit(T0 + 9_751_000, 7002, Some(7001), PARTIAL_FINAL),
    );

    assert_eq!(
        after.baskets[0].tp_stage, 1,
        "finalna edycja po restarcie nie moze udawac TP2 nowej drabinki"
    );
    assert_eq!(
        b.positions().len(),
        pozycje_przed,
        "finalna edycja nie moze drugi raz bankowac runnera"
    );
    assert!(
        after
            .odrzuty
            .get("DuplicateEditedAction")
            .copied()
            .unwrap_or(0)
            >= 2,
        "TP3 i niezmieniony SPP maja byc jawnie odrzucone jako juz wykonane"
    );
}
