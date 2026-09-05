//! Offline JSON-in/JSON-out probe of the actual preset mapper. No workspace bootstrap or RPC.
use std::io::{self, Read};

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let request: serde_json::Value = serde_json::from_str(&input).unwrap();
    let supplied = request.get("preset").expect("preset is required");
    let mut raw = supplied.get("settings").unwrap_or(supplied).clone();
    if let Some(overrides) = request.get("overrides") {
        conduit_server::settings_map::merge_patch(&mut raw, overrides);
    }
    let typed: conduit_core::Settings = serde_json::from_value(raw).unwrap();
    let full = serde_json::to_value(typed).unwrap();
    let mut ui = conduit_server::settings_map::preset_to_ui(&full);
    let before = serde_json::to_value(conduit_server::settings_map::core_from_ui(&ui)).unwrap();
    if let Some(patch) = request.get("patch") {
        conduit_server::settings_map::merge_patch(&mut ui, patch);
    }
    let after = serde_json::to_value(conduit_server::settings_map::core_from_ui(&ui)).unwrap();
    let changed: Vec<_> = before.as_object().unwrap().iter()
        .filter(|(key, value)| after.get(*key) != Some(*value))
        .map(|(key, value)| serde_json::json!({"key":key,"before":value,"after":after[key]}))
        .collect();
    println!("{}", serde_json::json!({"ui":ui,"before":before,"after":after,"changed":changed}));
}
