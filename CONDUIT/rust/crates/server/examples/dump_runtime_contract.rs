//! Offline settings conversion using the same public functions as live.rs.
//! Never starts a server, broker, terminal or Telegram connection.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        bail!("usage: dump_runtime_contract <settings.json snapshot> <new output preset.json>");
    }
    let source = PathBuf::from(&args[1]);
    let target = PathBuf::from(&args[2]);
    let doc: Value = serde_json::from_slice(&std::fs::read(&source)?)?;
    let settings = doc.get("settings").context("missing settings object")?;
    if !settings.is_object() {
        bail!("settings is not an object");
    }
    let lot: conduit_server::ui::LotConfig = serde_json::from_value(
        doc.get("lot").cloned().context("missing lot configuration")?,
    )?;
    let mut core = conduit_server::settings_map::core_from_ui(settings);
    conduit_server::settings_map::apply_lot(&mut core, &lot);
    let core_value = serde_json::to_value(&core)?;
    let mut account_fields = serde_json::Map::new();
    for key in conduit_core::wielosilnik::POLA_RACHUNKU {
        account_fields.insert((*key).into(), core_value.get(*key).cloned().unwrap_or(Value::Null));
    }
    let result = json!({
        "name": "OFFLINE-RUNTIME-ACCOUNT-SNAPSHOT",
        "format": "Synergy",
        "settings": core_value,
        "offline_runtime_contract": {
            "source": source,
            "converter": "core_from_ui + apply_lot",
            "no_connections_or_orders": true,
            "broker_reported_overrides_not_available": ["stops_level", "live account credit/leverage/margin mode", "actual swap/contract execution"],
            "account_fields": account_fields
        }
    });
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&target)
        .with_context(|| format!("refuse overwrite / cannot create {}", target.display()))?;
    serde_json::to_writer_pretty(&mut file, &result)?;
    file.write_all(b"\n")?;
    println!("offline runtime account snapshot written: {}", target.display());
    Ok(())
}
