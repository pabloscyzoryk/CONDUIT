//! Generated examples of the Synergy grammar, independent of private exports.
use super::*;

fn parse(text: &str) -> Vec<Signal> {
    parse_z_opcjami(text, OpcjeParsera {
        rf_wymaga_wykonania: true, partials_jako_komenda: true,
        luz_interpunkcyjny: true, recap_guard: true, ..OpcjeParsera::default()
    })
}

fn entry(text: &str) -> EntrySignal {
    parse(text).into_iter().find_map(|s| match s {
        Signal::Entry(e) => Some(e), _ => None,
    }).unwrap_or_else(|| panic!("missing synthetic entry: {text}"))
}

#[test]
fn entry_separator_and_direction_matrix() {
    for symbol in ["GOLD", "XAUUSD", "XAU"] {
        for separator in ["/", "_", "-", "–", " TO "] {
            for side in [Side::Buy, Side::Sell] {
                let (direction, tp, sl) = if side == Side::Buy {
                    ("BUY", 2120, 2080)
                } else { ("SELL", 2080, 2120) };
                let text = format!("{direction} LIMITS {symbol} @ 2105{separator}2100\nTP1 {tp}\nSL {sl}");
                let e = entry(&text);
                assert_eq!((e.side, e.lo, e.hi, e.sl, e.tps),
                    (side, 2100.0, 2105.0, Some(sl as f64), vec![tp as f64]));
                assert!(e.is_limit);
            }
        }
    }
}

#[test]
fn decimal_comma_and_point_produce_identical_geometry() {
    let a = entry("BUY LIMITS GOLD @ 2100.5/2098.5\nTP1 2110.5\nSL 2090.5");
    let b = entry("BUY LIMITS GOLD @ 2100,5/2098,5\nTP1 2110,5\nSL 2090,5");
    assert_eq!(a, b);
}

#[test]
fn cosmetics_do_not_change_entry_geometry() {
    let raw = "BUY LIMITS GOLD @ 2100/2095\nTP1 2110\nTP2 2120\nSL 2090";
    for decorated in [raw.to_lowercase(), format!("🟢 {raw}\nGood luck!"), raw.replace('\n', "\r\n")] {
        assert_eq!(entry(raw), entry(&decorated));
    }
}

#[test]
fn targets_are_unique_and_directionally_ordered() {
    assert_eq!(entry("BUY GOLD @ 2100/2095\nTP3 2130\nTP1 2110\nTP2 2110\nSL 2090").tps,
        vec![2110.0, 2130.0]);
    assert_eq!(entry("SELL GOLD @ 2105/2100\nTP3 2070\nTP1 2090\nTP2 2090\nSL 2110").tps,
        vec![2090.0, 2070.0]);
}

#[test]
fn buy_and_sell_now_have_no_fabricated_geometry() {
    assert_eq!(parse("BUY NOW"), vec![Signal::MarketOpen { side: Side::Buy }]);
    assert_eq!(parse("SELL NOW"), vec![Signal::MarketOpen { side: Side::Sell }]);
}

#[test]
fn price_bearing_entry_does_not_also_emit_market_now() {
    let parsed = parse("BUY NOW\nBUY GOLD @ 2100/2095\nTP1 2110\nSL 2090");
    assert!(parsed.iter().any(|s| matches!(s, Signal::Entry(_))));
    assert!(!parsed.iter().any(|s| matches!(s, Signal::MarketOpen { .. })));
}

#[test]
fn edited_hit_summary_does_not_open_the_old_entry_again() {
    for suffix in ["TP1 HIT", "SL HIT"] {
        let parsed = parse(&format!("BUY LIMITS GOLD @ 2100/2095\nTP1 2110\nSL 2090\n{suffix}"));
        assert!(!parsed.iter().any(|s| matches!(s, Signal::Entry(_))));
    }
}

#[test]
fn combined_hits_are_two_ordered_commands() {
    assert_eq!(parse("TP1 AND 2 HIT"), vec![Signal::TpHit { index: Some(1) }, Signal::TpHit { index: Some(2) }]);
}

#[test]
fn at_tp_telemetry_preserves_explicit_stop_management() {
    let text = "AT TP1\nMOVE SL TO 2101";
    let mut parsed = parse(text);
    assert_eq!(suppress_at_tp_hits(&mut parsed, text), 1);
    assert_eq!(parsed, vec![Signal::SetSl { value: 2101.0 }]);
}

#[test]
fn running_pips_is_not_confirmed_tp() {
    assert!(parse("+45 PIPS RUNNING").iter().all(|s| !matches!(s, Signal::TpHit { .. })));
    assert_eq!(profit_update_kind("+45 PIPS RUNNING"), ProfitUpdateKind::RunningPips);
    assert!(unindexed_pips_hit("+45 PIPS HIT"));
}

#[test]
fn risk_free_intention_is_distinct_from_execution() {
    for text in ["WILL MAKE THIS RISK FREE", "LOOK TO GO RISK FREE", "TRY TO MAKE IT RISK FREE"] {
        assert!(!parse(text).iter().any(|s| matches!(s, Signal::RiskFree { .. })), "{text}");
    }
    assert!(parse("RISK FREE NOW").iter().any(|s| matches!(s, Signal::RiskFree { .. })));
}

#[test]
fn correction_and_stop_edit_values_are_kept() {
    for text in ["USE 2112 AS TP2", "TP2 CHANGED TO 2112", "TP2 SHOULD BE 2112"] {
        assert!(parse(text).contains(&Signal::TpCorrection { index: 2, value: 2112.0 }), "{text}");
    }
    assert!(parse("SET SL TO 2101").contains(&Signal::SetSl { value: 2101.0 }));
}

#[test]
fn optional_partial_is_not_an_unconditional_exit() {
    assert!(!parse("YOU CAN CLOSE THE WORST LAYER").iter()
        .any(|s| matches!(s, Signal::TakePartials | Signal::CloseAll)));
    assert!(parse("CLOSE ALL").contains(&Signal::CloseAll));
}

#[test]
fn blank_and_service_text_never_create_risk() {
    for text in ["", "   ", "\n", "Good morning", "Thanks", "Preparing a setup"] {
        assert!(!parse(text).iter().any(|s| matches!(s, Signal::Entry(_) | Signal::MarketOpen { .. })));
    }
}
