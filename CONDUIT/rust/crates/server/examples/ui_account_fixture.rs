//! Synthetic UI fixture only. Does not bootstrap Conduit or connect to a broker.
fn main() {
    let mut value = serde_json::to_value(conduit_server::ui::UiSnapshot::empty(1_788_159_600_000)).unwrap();
    value["settings"] = serde_json::to_value(conduit_core::Settings::default()).unwrap();
    value["settings"]["mt5_follow_terminal_account"] = serde_json::json!(true);
    value["settings"]["mt5_allow_real_account"] = serde_json::json!(false);
    value["auth"]["stage"] = serde_json::json!("loggedIn");
    value["auth"]["user"] = serde_json::json!("SYNTHETIC OFFLINE TEST");
    value["connection"]["telegram"] = serde_json::json!("connected");
    value["connection"]["mt5"] = serde_json::json!("connected");
    value["connection"]["accountVerified"] = serde_json::json!("ok");
    value["favorites"] = serde_json::json!(["XAUUSD"]);
    value["language"] = serde_json::json!("en");
    let output = std::env::args().nth(1).expect("output fixture path is required");
    std::fs::write(&output, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    println!("Synthetic fixture generated; no broker/network initialized.");
}
