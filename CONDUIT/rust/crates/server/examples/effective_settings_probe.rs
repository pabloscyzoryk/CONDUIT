//! Offline deployment comparison using the production mapper and routing merge.
//! JSON enters through stdin; no workspace bootstrap, authentication or RPC.
use conduit_core::{
    formaty::{Lancuch, PulapyGlobalne},
    routing::Silniki,
    wielosilnik, Engine, Settings,
};
use conduit_server::{settings_map, store::SettingsDoc};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{self, Read},
};

fn typed(v: &Value) -> Result<Settings, &'static str> {
    serde_json::from_value(v.get("settings").unwrap_or(v).clone()).map_err(|_| "invalid_preset")
}

fn differences(expected: &Value, actual: &Value) -> Vec<Value> {
    expected.as_object().into_iter().flatten().filter_map(|(key, value)| {
        let found = actual.get(key)?;
        if value == found { return None; }
        // Terminal paths can contain a person's name. Never disclose them.
        let redact = key == "mt5_terminal_path";
        Some(json!({"field": key, "expected": if redact { json!("[redacted]") } else { value.clone() },
            "actual": if redact { json!("[redacted]") } else { found.clone() },
            "layer": if wielosilnik::POLA_RACHUNKU.contains(&key.as_str()) { "account" } else { "strategy" }}))
    }).collect()
}

fn audit(request: &Value) -> Result<Value, &'static str> {
    let doc: SettingsDoc = serde_json::from_value(
        request
            .get("settings_doc")
            .ok_or("settings_doc_required")?
            .clone(),
    )
    .map_err(|_| "invalid_settings_doc")?;
    let preset = typed(request.get("preset").ok_or("preset_required")?)?;
    let expected = typed(
        request
            .get("expected")
            .unwrap_or(request.get("preset").unwrap()),
    )?;
    let caps: PulapyGlobalne = serde_json::from_value(
        request
            .get("chain_caps")
            .ok_or("chain_caps_required")?
            .clone(),
    )
    .map_err(|_| "invalid_chain_caps")?;
    let expected_caps: PulapyGlobalne = serde_json::from_value(
        request
            .get("expected_chain_caps")
            .cloned()
            .unwrap_or_else(|| json!({})),
    )
    .map_err(|_| "invalid_expected_caps")?;
    // bootstrap gives embedded canonical lot fields precedence over outer LotConfig.
    let lot = settings_map::preset_lot(&doc.settings).unwrap_or(doc.lot);
    let mut account = settings_map::core_from_ui(&doc.settings);
    let blocked_net = doc
        .settings
        .get("closed_profit_net_costs")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // live_core_from_ui: NET live mode is explicitly blocked before connecting.
    account.closed_profit_net_costs = false;
    settings_map::apply_lot(&mut account, &lot);
    if let Some(stops) = request.get("broker_stops_level") {
        let stops = stops
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.)
            .ok_or("invalid_broker_stops_level")?;
        account.stops_level = stops;
    }
    let merged = wielosilnik::ustawienia_formatu(&preset, &account);
    let mut single = Engine::new(merged, 600.);
    single.pulapy = caps.clone();
    let mut blockers = Vec::new();
    if blocked_net {
        blockers.push("LIVE_NET_COST_HOLD");
    }
    if single.cfg.sr_warmup_exact_ticks
        && single.cfg.trail_sr_enabled
        && (single.cfg.trail_sr_min_prominence_atr > 0.
            || single.cfg.trail_sr_offset_atr_mult > 0.
            || single.cfg.trail_sr_offset_spread_mult > 0.)
    {
        blockers.push("LIVE_SR_V2_HOLD");
    }
    let chain = Lancuch {
        nazwa: "offline-audit".into(),
        presety: BTreeMap::from([("Synergy".into(), "reviewed".into())]),
        pulapy: caps.clone(),
        ..Default::default()
    };
    let (multiple, missing) = Silniki::zbuduj(
        &chain,
        &BTreeMap::from([("reviewed".into(), preset.clone())]),
        &account,
        600.,
    );
    if !missing.is_empty() || multiple.lista.len() != 1 {
        return Err("routing_construction_failed");
    }
    let effective = serde_json::to_value(&single.cfg).map_err(|_| "serialization_failed")?;
    let multiple_effective =
        serde_json::to_value(&multiple.lista[0].engine.cfg).map_err(|_| "serialization_failed")?;
    if effective != multiple_effective || single.pulapy != multiple.lista[0].engine.pulapy {
        return Err("routing_branch_mismatch");
    }
    let expected = serde_json::to_value(expected).map_err(|_| "serialization_failed")?;
    let core_differences = differences(&expected, &effective);
    let mut cap_differences = differences(
        &serde_json::to_value(&expected_caps).unwrap(),
        &serde_json::to_value(&caps).unwrap(),
    );
    for difference in &mut cap_differences { difference["layer"] = json!("chain"); }
    let preset_value = serde_json::to_value(preset).unwrap();
    let overlay_differences = differences(&preset_value, &effective);
    Ok(
        json!({"schema":"conduit.effective-settings.v1", "matches": core_differences.is_empty() && cap_differences.is_empty() && blockers.is_empty(),
        "core_fields_compared": effective.as_object().unwrap().len(), "chain_fields_compared":16,
        "core_differences":core_differences, "chain_differences":cap_differences,
        "preset_to_live_overrides":overlay_differences, "single_and_multiple_routing_equal":true,
        "live_start_blockers":blockers,
        "broker_stops_applied":request.get("broker_stops_level").is_some(),
        "lot_panel":lot, "account_fields":wielosilnik::POLA_RACHUNKU,
        "scope":"Configuration only; market facts, imported history, broker acknowledgements and economics require independent replay."}),
    )
}

fn main() {
    let mut input = String::new();
    let result = io::stdin()
        .read_to_string(&mut input)
        .map_err(|_| "input_unreadable")
        .and_then(|_| serde_json::from_str::<Value>(&input).map_err(|_| "invalid_json"))
        .and_then(|request| audit(&request));
    match result {
        Ok(report) => println!("{report}"),
        Err(code) => {
            println!("{}", json!({"error":code}));
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Value {
        let cfg = Settings {
            lot_mode_percent: true,
            lot_percent: 0.8,
            lot_fixed: 0.12,
            lot_max: 5.,
            day_trail_basis: conduit_core::settings::DayTrailBasis::ProfitPeak,
            profit_budget_arm_pct: 7.,
            profit_budget_keep_pct: 60.,
            profit_budget_deploy_pct: 80.,
            edycja_sieroty_nie_otwiera: false,
            explicit_pending_until_cancel: false,
            ..Default::default()
        };
        let full = serde_json::to_value(&cfg).unwrap();
        let mut settings = settings_map::preset_to_ui(&full);
        // The preset must own strategy sizing even if the panel holds stale values.
        settings_map::merge_patch(
            &mut settings,
            &json!({"lot_mode_percent":false,"lot_fixed":9.,"lot_percent":22.,"lot_max":99.}),
        );
        json!({"settings_doc":{"settings":settings,"lot":{"mode":"fixed","fixed":8.,"percent":11.}},"preset":full,"chain_caps":{}})
    }
    #[test]
    fn preset_strategy_survives_stale_panel_lot_and_new_axes() {
        let r = audit(&request()).unwrap();
        assert_eq!(r["matches"], true, "{r}");
        assert!(r["core_fields_compared"].as_u64().unwrap() > 500);
        assert_eq!(r["lot_panel"]["fixed"], 9.);
    }
    #[test]
    fn account_overlay_and_chain_limits_are_not_silent() {
        let mut v = request();
        settings_map::merge_patch(
            &mut v["settings_doc"]["settings"],
            &json!({"expo_cap_pct":80.,"lot_base":"equity"}),
        );
        v["chain_caps"] = json!({"maxPozycji":20,"maxKoszykow":6,"blokujPrzeciwneKierunki":true});
        let r = audit(&v).unwrap();
        assert_eq!(r["matches"], false);
        assert_eq!(r["chain_differences"].as_array().unwrap().len(), 3);
        assert!(r["core_differences"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["field"] == "expo_cap_pct"));
    }
    #[test]
    fn broker_stops_and_live_net_hold_are_explicit() {
        let mut v = request();
        v["broker_stops_level"] = json!(0.5);
        settings_map::merge_patch(
            &mut v["settings_doc"]["settings"],
            &json!({"closed_profit_net_costs":true}),
        );
        let r = audit(&v).unwrap();
        assert_eq!(r["matches"], false);
        assert_eq!(r["live_start_blockers"], json!(["LIVE_NET_COST_HOLD"]));
        assert!(r["core_differences"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["field"] == "stops_level"));
    }
    #[test]
    fn exact_tick_sr_blocker_and_private_terminal_path_are_explicitly_safe() {
        let mut v = request();
        v["preset"]["sr_warmup_exact_ticks"] = json!(true);
        v["preset"]["trail_sr_enabled"] = json!(true);
        v["preset"]["trail_sr_min_prominence_atr"] = json!(1.);
        v["preset"]["mt5_terminal_path"] = json!("C:/SYNTHETIC-PRIVATE-A/terminal.exe");
        v["settings_doc"]["settings"]["mt5_terminal_path"] =
            json!("C:/SYNTHETIC-PRIVATE-B/terminal.exe");
        let r = audit(&v).unwrap();
        assert_eq!(r["matches"], false);
        assert_eq!(r["live_start_blockers"], json!(["LIVE_SR_V2_HOLD"]));
        assert!(!r.to_string().contains("SYNTHETIC-PRIVATE"));
    }
}
