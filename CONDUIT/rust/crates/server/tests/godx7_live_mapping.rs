use conduit_server::settings_map::{core_from_ui, preset_to_ui, unmapped_keys};
use serde_json::json;

#[test]
fn god_x7_fast_addon_survives_the_live_preset_mapping() {
    // These are the engine-native settings that distinguish GOD-X7 from
    // GOD-X6.  The live application loads presets through `preset_to_ui` and
    // then `core_from_ui`; a silent drop at either boundary would make live
    // behavior differ from the Rust backtest despite an apparently correct
    // preset file.
    let engine_preset = json!({
        "lot_mode_percent": true,
        "lot_max": 10.0,
        "fast_addon_move_usd": 8.0,
        "fast_addon_window_s": 60.0,
        "fast_addon_max": 1,
        "fast_addon_lot_mult": 1.0,
        "fast_addon_min_stage": 0,
        "fast_addon_cooldown_s": 60.0
    });

    let live_document = preset_to_ui(&engine_preset);
    let unmapped = unmapped_keys(&live_document);
    for key in [
        "lot_mode_percent",
        "lot_max",
        "fast_addon_move_usd",
        "fast_addon_window_s",
        "fast_addon_max",
        "fast_addon_lot_mult",
        "fast_addon_min_stage",
        "fast_addon_cooldown_s",
    ] {
        assert!(
            !unmapped.iter().any(|candidate| candidate == key),
            "GOD-X7 live setting is unmapped: {key}; unmapped={unmapped:?}"
        );
    }

    let core = core_from_ui(&live_document);
    assert!(core.lot_mode_percent);
    assert_eq!(core.lot_max, 10.0);
    assert_eq!(core.fast_addon_move_usd, 8.0);
    assert_eq!(core.fast_addon_window_s, 60.0);
    assert_eq!(core.fast_addon_max, 1);
    assert_eq!(core.fast_addon_lot_mult, 1.0);
    assert_eq!(core.fast_addon_min_stage, 0);
    assert_eq!(core.fast_addon_cooldown_s, 60.0);
}

