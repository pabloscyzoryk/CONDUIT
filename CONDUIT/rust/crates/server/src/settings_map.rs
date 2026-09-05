//! Most między dokumentem ustawień z panelu a konfiguracją silnika.
//!
//! W React żyje 149 kluczy odwzorowanych z `bot.py`. `conduit_core::Settings`
//! ma inny, mniejszy i uporządkowany zestaw — bo część kluczy z `bot.py`
//! dotyczyła wyłącznie panelu (kolory, interwał odpytywania, scalanie logów),
//! a część została w rdzeniu połączona w jedno pole (np. trzy flagi trailingu
//! → jeden `TrailMode`).
//!
//! **Uczciwe zastrzeżenie:** to mapowanie jest CZĘŚCIOWE i jawnie wypisane
//! poniżej. Klucze, których tu nie ma, nie docierają do silnika — panel je
//! zapamięta, ale nie zmienią zachowania bota. Lista braków jest zwracana
//! przez [`unmapped_keys`], żeby dało się ją pokazać w UI zamiast udawać,
//! że wszystko działa.

use conduit_core::settings::*;
use serde_json::Value;

fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k)?.as_f64()
}
fn b(v: &Value, k: &str) -> Option<bool> {
    v.get(k)?.as_bool()
}
fn s(v: &Value, k: &str) -> Option<String> {
    Some(v.get(k)?.as_str()?.to_string())
}
fn u(v: &Value, k: &str) -> Option<u32> {
    Some(f(v, k)? as u32)
}

/// Klucze dokumentu UI, które NIE trafiają do `conduit_core::Settings`.
///
/// **To NIE znaczy „martwe".** Lista miesza trzy różne rzeczy i trzeba je
/// rozróżniać, bo inaczej audyt ustawień wyciąga fałszywe wnioski:
///
///  1. **Wygląd i zachowanie panelu** — czyta je React (`display_currency`,
///     `one_click`, `poll_ms`, `merge_*`, `ui_*`). Działają, tylko nie w silniku.
///  2. **Czytane POZA rdzeniem** — `comment_mode` i `comment_custom` bierze
///     `live.rs` (`znacznik_komentarza`) i wysyła do mostu MT5. Też działają.
///  3. **Naprawdę martwe** — nikt ich nie czyta. Te NIE MAJĄ prawa być
///     widoczne w `settingsSchema.ts`: przełącznik, który nic nie robi, jest
///     gorszy niż jego brak, bo użytkownik myśli, że coś ustawił.
///     Stan na 29.07: `allow_mt5_modify`, `spp_refresh_after_modify`,
///     `sim_clock_strict`, `grid_fallback_best_edge`, `comment_include_topic`,
///     `day_flat_broker_clock` — usunięte z panelu, zostają w dokumencie,
///     żeby stare presety i `settings.json` wczytywały się bez ostrzeżeń.
pub const UI_ONLY_KEYS: &[&str] = &[
    // --- 1. panel ---
    "one_click",
    "display_currency",
    "poll_ms",
    "show_positions_on_chart",
    "show_potential_tpsl",
    "exclude_pending_potential",
    // --- 2. czytane przez `live.rs`, nie przez rdzeń ---
    "comment_mode",
    "comment_custom",
    "price_log",
    "price_log_interval_s",
    "merge_config",
    "merge_chronological",
    // Wygląd panelu. Trzymamy go w `settings.json`, a nie tylko w magazynie
    // przeglądarki, żeby wybór motywu przeżył czyszczenie danych karty
    // i pojechał razem z folderem na inny VPS.
    "ui_theme",
    "ui_palette",
    // --- 3. martwe: nikt ich nie czyta; USUNIĘTE Z PANELU 29.07 ---
    // Zostają na liście wyłącznie po to, żeby stare presety i `settings.json`
    // wczytywały się bez zgłaszania „nieobsługiwane ustawienie".
    //
    //  * `allow_mt5_modify` — wymagałoby wykrywania cudzej modyfikacji SL/TP
    //    w moście (`conduit_mt5`); dopóki tego nie ma, przełącznik kłamał,
    //  * `spp_refresh_after_modify` — miała ją czytać pętla SPP w rdzeniu,
    //  * `comment_include_topic` — `znacznik_komentarza` w `live.rs` nigdy
    //    nie sięga po nazwę tematu.
    "allow_mt5_modify",
    "spp_refresh_after_modify",
    "comment_include_topic",
    // Poniższe dwa były w `bot.py` ŁATKAMI NA BŁĘDY, których w tym silniku
    // nie da się popełnić, więc nie mają czego przełączać:
    //  * `grid_fallback_best_edge` — siatka nie potrafi już wpaść na złą
    //    krawędź, bo krawędzie wybiera `Side::better_edge`/`worse_edge`,
    //  * `day_flat_broker_clock` — godzina ZAWSZE pochodzi z czasu ticka,
    //    bo rdzeń w ogóle nie ma dostępu do zegara ściennego.
    "grid_fallback_best_edge",
    "day_flat_broker_clock",
    // `sim_clock_strict` dotyczyło symulatora, a nie silnika — backtest ma
    // dziś jeden, ścisły zegar i nie ma czego przełączać.
    "sim_clock_strict",
];

/// Buduje konfigurację silnika z dokumentu panelu.
///
/// Wartości nierozpoznane zostawiają wartość domyślną rdzenia — nigdy nie
/// zgadujemy. Lepiej, żeby ustawienie „nie działało" i było to widać w
/// [`unmapped_keys`], niż żeby działało inaczej, niż mówi etykieta w panelu.
pub fn core_from_ui(doc: &Value) -> Settings {
    let mut c = Settings::default();
    if !doc.is_object() {
        return c;
    }

    // Dokument, który NADAL niesie klucze w nazewnictwie SILNIKA, nie przeszedł
    // przez [`preset_to_ui`] — bo ta funkcja je z dokumentu usuwa. Taki plik
    // powstaje, gdy preset trafi do `settings.json` z pominięciem tłumaczenia,
    // i jest cichy w najgorszy możliwy sposób: klucz JEST w pliku, ma poprawną
    // wartość, panel go pokazuje — a silnik czyta pod inną nazwą, więc dostaje
    // wartość DOMYŚLNĄ.
    //
    // Zaobserwowane 30.07.2026 na `PACKAGE/settings.json`: `tp_schedule` stało
    // w pliku jako `OfficialCounts`, a silnik grał `Ladder`; `zone_offset_mode`
    // stało jako `Directional`, a silnik grał `None`. Cztery takie pola razem
    // to różnica 1936 $ → −133 $ na oknie czerwiec–lipiec.
    //
    // Tłumaczymy więc taki dokument w locie. To NIE tworzy drugiego źródła
    // prawdy: po tłumaczeniu klucze rdzenia z dokumentu znikają, a dla pliku
    // zapisanego normalnie przez panel warunek niżej jest fałszywy i nie dzieje
    // się nic. Wartość z presetu ma tu pierwszeństwo przed kluczem panelu
    // celowo — obok niej stoi zwykle nieodświeżona pozostałość po poprzedniej
    // konfiguracji, a nie świadomy wybór użytkownika.
    if PRZETLUMACZONE.iter().any(|k| doc.get(*k).is_some()) {
        return core_from_ui(&preset_to_ui(doc));
    }

    // ---------- wielkość pozycji ----------
    if let Some(v) = f(doc, "lot_scale_step") {
        c.lot_scale_step = v;
    }
    // TRYB I WIELKOŚĆ LOTA — mapowane TĘDY od 04.08.2026.
    //
    // Do tej pory te trzy pola świadomie tu NIE przechodziły („panel ma dla
    // nich osobny sterownik `Command::SetLot`"). Kosztowało to dwie usterki
    // naraz, obie na żywym rachunku:
    //
    // 1. **Preset tracił lot przy każdej edycji z panelu.** Droga łatki
    //    (`rest.rs::patch_preset_settings`) kończy się na `core_from_ui`, więc
    //    pole nieprzepisane wracało do wartości domyślnej — zapis JEDNEGO pola
    //    presetu po cichu przestawiał mu lot na stały 0,01.
    // 2. **Karta panelu rządziła lotem automatu.** Jedynym nośnikiem tych pól
    //    był `ui::LotConfig`, wspólny dla całego rachunku — a lot jest
    //    własnością PRESETU nogi (patrz `wielosilnik::POLA_RACHUNKU`).
    //
    // Teraz pola jadą normalną drogą dokument→silnik, jak cała reszta.
    // `apply_lot` nadal istnieje i nadal nadpisuje TE trzy pola kartą panelu,
    // ale wołamy go WYŁĄCZNIE na dokumencie rachunku (`live.rs`), gdzie
    // opisuje konfigurację ręczną bez pliku presetu.
    if let Some(v) = b(doc, "lot_mode_percent") {
        c.lot_mode_percent = v;
    }
    if let Some(v) = f(doc, "lot_fixed") {
        c.lot_fixed = v;
    }
    if let Some(v) = f(doc, "lot_percent") {
        c.lot_percent = v;
    }
    if let Some(v) = f(doc, "lot_min") {
        c.lot_min = v;
    }
    if let Some(v) = f(doc, "lot_max") {
        c.lot_max = v;
    }
    // KREDYT BONUSOWY — dwa pola karty lota. Tędy, a nie przez `Command::SetLot`:
    // to nie jest wielkość pozycji, tylko PODSTAWA, od której się ją liczy,
    // i zapisuje się razem z resztą ustawień rachunku.
    if let Some(v) = b(doc, "odlicz_kredyt") {
        c.odlicz_kredyt = v;
    }
    if let Some(v) = b(doc, "credit_balance_separate") {
        c.credit_balance_separate = v;
    }
    if let Some(v) = b(doc, "close_receipt_reconcile") {
        c.close_receipt_reconcile = v;
    }
    if let Some(v) = b(doc, "closed_profit_net_costs") {
        c.closed_profit_net_costs = v;
    }
    if let Some(v) = b(doc, "restore_strategy_continuation") {
        c.restore_strategy_continuation = v;
    }
    if let Some(v) = b(doc, "order_volume_contract_v2") {
        c.order_volume_contract_v2 = v;
    }
    if let Some(v) = b(doc, "be_never_loosen") {
        c.be_never_loosen = v;
    }
    if let Some(v) = b(doc, "entry_edit_geometry_v2") {
        c.entry_edit_geometry_v2 = v;
    }
    if let Some(v) = b(doc, "sr_warmup_exact_ticks") {
        c.sr_warmup_exact_ticks = v;
    }
    if let Some(v) = b(doc, "retarget_respects_final_target") {
        c.retarget_respects_final_target = v;
    }
    if let Some(v) = f(doc, "kredyt_reczny") {
        // Ujemna kwota nie znaczy nic sensownego, a przepuszczona podniosłaby
        // podstawę lota POWYŻEJ salda. Panel ma `min=0`, ale plik ustawień
        // można wpisać ręcznie.
        c.kredyt_reczny = if v.is_finite() && v > 0.0 { v } else { 0.0 };
    }

    // ---------- strefa wejścia ----------
    if b(doc, "entry_offset_dir").unwrap_or(false) {
        c.zone_offset_mode = ZoneOffsetMode::Directional;
    } else if b(doc, "custom_entry").unwrap_or(false) {
        c.zone_offset_mode = ZoneOffsetMode::Price;
    }
    if let Some(v) = f(doc, "entry_high_offset") {
        c.entry_hi_offset = v;
    }
    if let Some(v) = f(doc, "entry_low_offset") {
        c.entry_lo_offset = v;
    }
    if let Some(v) = f(doc, "entry_deep_offset") {
        c.entry_deep_offset = v;
    }
    // M3: głębokość siatki jako ułamek dystansu strefa→SL (0 = stała kwota).
    if let Some(v) = f(doc, "entry_deep_frac_to_sl") {
        c.entry_deep_frac_to_sl = v;
    }
    if let Some(v) = f(doc, "entry_tol_offset") {
        c.entry_tol_offset = v;
    }
    if let Some(v) = b(doc, "only_limit_signals") {
        c.only_limit_signals = v;
    }
    if let Some(v) = b(doc, "auto_limit") {
        c.auto_limit = v;
    }
    if let Some(v) = f(doc, "ignore_old_after_min") {
        c.ignore_old_after_min = v;
    }
    if let Some(v) = b(doc, "skip_if_sl_breached") {
        c.skip_if_sl_breached = v;
    }
    if let Some(v) = f(doc, "max_chase_beyond_zone") {
        c.max_chase_beyond_zone = v;
    }
    if let Some(v) = f(doc, "sl_max_dist") {
        c.sl_max_dist = v;
    }

    // ---------- siatka ----------
    if let Some(v) = u(doc, "entry_units") {
        c.entry_units = v.max(1);
    }
    if let Some(v) = u(doc, "entry_units_limit") {
        c.entry_units_limit = v;
    }
    if let Some(v) = f(doc, "ppm") {
        c.ppm = v;
    }
    if let Some(v) = b(doc, "ppm_enabled") {
        c.ppm_enabled = v;
    }
    if let Some(v) = f(doc, "entry_risk_budget") {
        c.entry_risk_budget = v;
    }
    if let Some(v) = f(doc, "entry_tp1_budget") {
        c.entry_tp1_budget = v;
    }
    if let Some(v) = s(doc, "entry_weights") {
        c.entry_weights = v;
    }
    if let Some(v) = f(doc, "risk_per_basket_pct") {
        c.risk_per_basket_pct = v;
    }
    if let Some(v) = u(doc, "entry_touch_units") {
        c.toucher_units = v;
    }
    if let Some(v) = f(doc, "entry_touch_tp") {
        c.toucher_tp_index = (v as usize).saturating_sub(1);
    }
    if let Some(v) = s(doc, "entry_touch_levels") {
        c.toucher_bands = v;
    }
    // PPM ma dwa niezależne zastosowania — gęstość siatki limitów i odstęp
    // kolejnych wejść rynkowych. Zlanie ich w jedno pokrętło zmieniało
    // zachowanie presetów, które włączały tylko jedno z nich.
    if let Some(v) = b(doc, "ppm_for_limits") {
        c.ppm_for_limits = v;
    }
    if let Some(v) = b(doc, "ppm_immediate") {
        c.ppm_for_market = v;
    }
    if let Some(v) = f(doc, "market_entry_step") {
        c.market_entry_step = v;
    }
    // Brak klucza = `GridAtOnce`, czyli zachowanie sprzed rozdzielenia tej
    // decyzji od `auto_limit`. Preset sprzed tej zmiany dostaje więc dokładnie
    // to, co mierzył backtest. Panel bywa pisany małą literą — rozumiemy obie
    // konwencje, bo dokładnie na tym rozjeździe zginęło pięć pól 30.07.
    if let Some(v) = s(doc, "market_entry_mode") {
        c.market_entry_mode = match v.as_str() {
            "Single" | "single" => MarketEntryMode::Single,
            "Laddered" | "laddered" => MarketEntryMode::Laddered,
            _ => MarketEntryMode::GridAtOnce,
        };
    }
    if let Some(v) = f(doc, "pending_ttl_h") {
        c.pending_ttl_h = v;
    }
    if let Some(v) = b(doc, "explicit_pending_until_cancel") {
        c.explicit_pending_until_cancel = v;
    }
    if let Some(v) = b(doc, "pending_ttl_from_basket") {
        c.pending_ttl_from_basket = v;
    }
    // REGUŁA HANDLOWA, nie ozdobnik: kasowanie niewypełnionej siatki, gdy cena
    // sięgnęła celu bez nas. Bez niej presety zerują konto (patrz historia
    // strojenia), więc musi dać się ją przenieść w presecie, a nie tylko
    // odziedziczyć z domyślnych.
    if let Some(v) = b(doc, "pending_drop_on_target") {
        c.pending_drop_on_target = v;
    }

    // ---------- reżim zmienności ----------
    if let Some(v) = f(doc, "vol_window_min") {
        c.vol_window_min = v;
    }
    if let Some(v) = f(doc, "vol_range_usd") {
        c.vol_range_usd = v;
    }
    if let Some(v) = f(doc, "vol_units_mult") {
        c.vol_units_mult = v;
    }
    if let Some(v) = b(doc, "pending_resize_on_vol") {
        c.pending_resize_on_vol = v;
    }
    if let Some(v) = f(doc, "pending_resize_sec") {
        c.pending_resize_s = v;
    }
    c.pending_lifetime = if b(doc, "pending_never_cancel").unwrap_or(false) {
        PendingLifetime::Never
    } else if b(doc, "valid_till_tp2").unwrap_or(false) {
        PendingLifetime::UntilTp2
    } else {
        PendingLifetime::UntilTp1
    };

    // ---------- stop loss ----------
    if let Some(v) = f(doc, "sl_min_dist") {
        c.sl_min_dist = v;
    }
    if b(doc, "sl_dist_limit").unwrap_or(false) {
        if let Some(v) = f(doc, "sl_dist_max") {
            c.entry_sl_dist_limit = v;
        }
    }
    if let Some(v) = b(doc, "virtual_sl") {
        c.virtual_sl = v;
    }
    if let Some(v) = b(doc, "virtual_sl_only_when_rejected") {
        c.virtual_sl_only_when_rejected = v;
    }
    if let Some(v) = f(doc, "vsl_eval_s") {
        c.vsl_eval_s = v;
    }
    if let Some(v) = b(doc, "virtual_sl_all") {
        c.virtual_sl_all = v;
    }
    if let Some(v) = f(doc, "vsl_net_off") {
        c.vsl_broker_offset = v;
    }

    // ---------- cele ----------
    c.tp_schedule = if b(doc, "all_runners").unwrap_or(false) {
        TpSchedule::AllRunners
    } else if b(doc, "official_mode").unwrap_or(false) {
        if b(doc, "official_use_counts").unwrap_or(false) {
            TpSchedule::OfficialCounts
        } else {
            TpSchedule::OfficialPct
        }
    } else if b(doc, "scale_out").unwrap_or(false) {
        TpSchedule::ScaleOutPct
    } else {
        TpSchedule::Ladder
    };
    if let Some(v) = f(doc, "scale_out_pct") {
        c.scale_out_pct = v;
    }
    c.official_pct = [
        f(doc, "official_pct_tp1").unwrap_or(c.official_pct[0]),
        f(doc, "official_pct_tp2").unwrap_or(c.official_pct[1]),
        f(doc, "official_pct_tp3").unwrap_or(c.official_pct[2]),
        f(doc, "official_pct_spp").unwrap_or(c.official_pct[3]),
    ];
    if let Some(v) = s(doc, "official_counts") {
        c.official_counts = v;
    }
    if let Some(v) = b(doc, "official_spp") {
        c.official_spp = v;
    }
    if let Some(v) = b(doc, "official_assign_tps") {
        c.assign_tp_per_position = v;
    }
    if let Some(v) = f(doc, "tp_open_offset") {
        c.tp_open_offset = v;
    }
    if let Some(v) = b(doc, "tp_freeze_after_ladder") {
        c.tp_freeze_after_ladder = v;
    }
    // Trzy źródła wiedzy o trafieniu celu (cena / broker / kanał) mają teraz
    // jeden przełącznik priorytetu zamiast dwóch niezależnych booli.
    //
    // Stare presety niosą jeszcze parę `tp_detect_price` / `tp_detect_signal`,
    // więc najpierw wyprowadzamy z niej tryb — a dopiero potem pozwalamy
    // wygrać jawnemu `tp_source`. Dzięki temu zapisany preset zachowuje się
    // tak, jak zachowywał się w poprzednim bocie.
    match (b(doc, "tp_detect_price"), b(doc, "tp_detect_signal")) {
        (Some(true), Some(false)) => c.tp_source = TpSource::PriceOnly,
        (Some(false), Some(true)) => c.tp_source = TpSource::SignalOnly,
        (Some(true), Some(true)) => c.tp_source = TpSource::Either,
        // oba wyłączone znaczyło w `bot.py` „niech działa cena" — nigdy
        // „nie wykrywaj celów wcale"
        (Some(false), Some(false)) => c.tp_source = TpSource::PriceOnly,
        _ => {}
    }
    if let Some(v) = s(doc, "tp_source") {
        use conduit_core::settings::TpSource::*;
        c.tp_source = match v.as_str() {
            "PriceOnly" => PriceOnly,
            "SignalOnly" => SignalOnly,
            "SignalConfirmedByPrice" => SignalConfirmedByPrice,
            "PriceFirstSignalWindow" => PriceFirstSignalWindow,
            _ => Either,
        };
    }
    if let Some(v) = f(doc, "tp_price_tolerance") {
        c.tp_price_tolerance = v;
    }
    if let Some(v) = f(doc, "tp_price_front_run_usd") {
        c.tp_price_front_run_usd = v;
    }
    if let Some(v) = f(doc, "tp_signal_max_lead_s") {
        c.tp_signal_max_lead_s = v;
    }
    if let Some(v) = f(doc, "tp_signal_max_lag_s") {
        c.tp_signal_max_lag_s = v;
    }
    if let Some(v) = b(doc, "tp_stage_from_broker_fill") {
        c.tp_stage_from_broker_fill = v;
    }

    if let Some(v) = b(doc, "tp_hit_fill_stages") {
        c.tp_hit_fill_stages = v;
    }

    // Zaokrąglanie transzy. Panel ma dwa pola (dla harmonogramu oficjalnego
    // i dla scale-outu), bo `bot.py` też je miał; w rdzeniu jest jedno pole,
    // więc bierzemy to, które odpowiada włączonemu trybowi.
    let rounding_key = if b(doc, "official_mode").unwrap_or(false) {
        "official_round"
    } else {
        "scale_out_round"
    };
    if let Some(v) = s(doc, rounding_key) {
        c.bank_rounding = match v.as_str() {
            "nearest" => BankRounding::Nearest,
            "down" => BankRounding::Down,
            _ => BankRounding::Up,
        };
    }
    if let Some(v) = s(doc, "scale_out_from") {
        c.bank_from = if v == "best" {
            BankFrom::Best
        } else {
            BankFrom::Worst
        };
    }
    if let Some(v) = s(doc, "scale_out_last_runner") {
        c.last_runner = match v.as_str() {
            "next_tp" => LastRunner::NextTp,
            "no_tp" => LastRunner::NoTp,
            _ => LastRunner::Runner,
        };
    }
    if let Some(v) = b(doc, "official_close_last") {
        c.bank_close_last = v;
    }
    if let Some(v) = b(doc, "partial_close") {
        c.partial_close = v;
    }
    if let Some(v) = f(doc, "partial_min_lot") {
        c.partial_min_lot = v;
    }
    // ---- RODZINA TYLER (10.08.2026) ----
    if let Some(v) = b(doc, "partial_pct_od_pierwotnego") {
        c.partial_pct_od_pierwotnego = v;
    }
    if let Some(v) = b(doc, "cele_na_ostatnim") {
        c.cele_na_ostatnim = v;
    }
    if let Some(v) = f(doc, "sl_polowa_od_konca") {
        c.sl_polowa_od_konca = v.max(0.0) as usize;
    }
    if let Some(v) = f(doc, "sl_polowa_ulamek") {
        c.sl_polowa_ulamek = v;
    }
    if let Some(v) = f(doc, "spp_max_age_h") {
        c.spp_max_age_h = v;
    }
    if let Some(v) = b(doc, "spp_keep_tp") {
        c.spp_keep_tp = v;
    }
    // JAWNY POZIOM STOPU Z KOMUNIKATU SPP. Obie konwencje nazw wariantu, jak
    // przy `regime_filter`; nierozpoznana wartość NIE MOŻE po cichu zmienić
    // zachowania handlowego, więc spada na `Off` (= dzisiejsze zachowanie).
    if let Some(v) = s(doc, "spp_sl_mode") {
        c.spp_sl_mode = match v.as_str() {
            "Stop" | "stop" | "set_sl" | "SetSl" => SppSlMode::Stop,
            "OnlyIfBetter" | "only_if_better" | "tylko_gdy_lepszy" => SppSlMode::OnlyIfBetter,
            "RunnersOnly" | "runners_only" | "runner" => SppSlMode::RunnersOnly,
            "RunnersOnlyIfBetter" | "runners_only_if_better" | "runner_better" => {
                SppSlMode::RunnersOnlyIfBetter
            }
            "BankersOnly" | "bankers_only" | "bankujace" => SppSlMode::BankersOnly,
            _ => SppSlMode::Off,
        };
    }
    if let Some(v) = f(doc, "spp_sl_pad") {
        c.spp_sl_pad = v;
    }

    // ---------- reakcje na komunikaty ----------
    // Wartość przyjmujemy w OBU konwencjach: panel pisze `"all_runners"` /
    // `"scale_out"`, preset z pliku niesie nazwę wariantu rdzenia. Bez nazw
    // rdzenia `"MoveSlToBeOnly"` nie pasowało do niczego i wpadało w gałąź
    // `else`, czyli po cichu ustawiało `CloseAllKeepNearest` — zmierzone
    // −976 $ na oknie czerwiec–lipiec (ULTRA-X3).
    if b(doc, "ignore_risk_free").unwrap_or(false) {
        c.risk_free_mode = RiskFreeMode::Ignore;
    } else {
        c.risk_free_mode = match s(doc, "risk_free_mode").as_deref() {
            Some("all_runners") | Some("MoveSlToBeOnly") => RiskFreeMode::MoveSlToBeOnly,
            Some("Ignore") => RiskFreeMode::Ignore,
            _ => RiskFreeMode::CloseAllKeepNearest,
        };
    }
    if let Some(v) = u(doc, "risk_free_runners") {
        c.risk_free_runners = v.max(1);
    }
    // M4: próg zysku dla ruchu stopu na BE po komunikacie RISK FREE.
    if let Some(v) = f(doc, "risk_free_be_min_profit") {
        c.risk_free_be_min_profit = v;
    }
    if let Some(v) = s(doc, "risk_free_runner_target") {
        // obie konwencje (patrz `regime_filter`); `riskfree_runner_target`
        // niżej robił to od początku, ten jeden został w tyle
        c.risk_free_runner_target = match v.as_str() {
            "keep" | "KeepTp" => RiskFreeRunnerTarget::KeepTp,
            "next" | "NextTp" => RiskFreeRunnerTarget::NextTp,
            "none" | "NoTpTrailOnly" => RiskFreeRunnerTarget::NoTpTrailOnly,
            _ => RiskFreeRunnerTarget::LastTp,
        };
    }
    if let Some(v) = b(doc, "risk_free_trail") {
        c.risk_free_trail = v;
    }
    c.out_at_entry_mode = if b(doc, "ignore_out_at_entry").unwrap_or(false) {
        OutAtEntryMode::Ignore
    } else {
        // obie konwencje: panel (`"losers"`) i preset z pliku
        // (`"CloseLosersOnly"`); sama konwencja panelu dawała cichy powrót
        // do `CloseAll` — zmierzone −370 $ na oknie czerwiec–lipiec
        match s(doc, "out_at_entry_mode").as_deref() {
            Some("losers") | Some("CloseLosersOnly") => OutAtEntryMode::CloseLosersOnly,
            Some("flat") | Some("CloseFlatOnly") => OutAtEntryMode::CloseFlatOnly,
            Some("be") | Some("MoveSlToBe") => OutAtEntryMode::MoveSlToBe,
            Some("Ignore") => OutAtEntryMode::Ignore,
            _ => OutAtEntryMode::CloseAll,
        }
    };
    if let Some(v) = f(doc, "oae_band_pts") {
        c.oae_band_pts = v;
    }
    if let Some(v) = s(doc, "oae_pod_woda") {
        c.oae_pod_woda = match v.as_str() {
            "zamknij" | "close" | "Zamknij" => OaePodWoda::Zamknij,
            "dociagnij_stop" | "tighten_stop" | "DociagnijStop" => OaePodWoda::DociagnijStop,
            _ => OaePodWoda::NicNieRob,
        };
    }
    if let Some(v) = f(doc, "sl_hit_verify_tol") {
        c.sl_hit_verify_tol = v;
        if v > 0.0 {
            c.sl_hit_mode = SlHitMode::VerifyByPrice;
        }
    }
    // jawny wybór trybu wygrywa z domysłem opartym na tolerancji
    if let Some(v) = s(doc, "sl_hit_mode") {
        // obie konwencje: panel (`"verify"`) i preset z pliku (`"VerifyByPrice"`)
        c.sl_hit_mode = match v.as_str() {
            "close_all" | "CloseAll" => SlHitMode::CloseAll,
            "verify" | "VerifyByPrice" => SlHitMode::VerifyByPrice,
            "ignore" | "Ignore" => SlHitMode::Ignore,
            _ => SlHitMode::CancelPendings,
        };
    }
    if let Some(v) = b(doc, "honor_cancel") {
        c.honor_cancel = v;
    }
    if let Some(v) = b(doc, "honor_close_all") {
        c.honor_close_all = v;
    }
    // W33: zasięg „CLOSE ALL". Brak klucza zostawia domyślne `Global`,
    // czyli zachowanie sprzed 24.08.2026 co do centa.
    if let Some(v) = s(doc, "close_all_scope") {
        c.close_all_scope = match v.as_str() {
            "Basket" => CloseAllScope::Basket,
            _ => CloseAllScope::Global,
        };
    }
    // W31b: wykonywanie komendy „Take partials" i wielkość transzy.
    if let Some(v) = b(doc, "partials_wykonuj") {
        c.partials_wykonuj = v;
    }
    if let Some(v) = f(doc, "partials_pct") {
        c.partials_pct = v;
    }
    // 25.08: poboczne zapisy poleceń („Set SL to BE", „Out. At BE", „RISK FREEE",
    // „ALL TP'S HIT", „zone is no longer valid", „TP3 should be 4043").
    if let Some(v) = b(doc, "parser_luz_interpunkcyjny") {
        c.parser_luz_interpunkcyjny = v;
    }
    if let Some(v) = b(doc, "recap_guard") {
        c.recap_guard = v;
    }
    if let Some(v) = b(doc, "profit_update_telemetry_only") {
        c.profit_update_telemetry_only = v;
    }
    if let Some(v) = b(doc, "honor_market_open") {
        c.honor_market_open = v;
    }
    if let Some(v) = b(doc, "dedup_edited_signals") {
        c.dedup_edited_signals = v;
    }
    // ---- Pakiet A: osie dedupu i edycji (nazwy wspólne z rdzeniem) ----
    if let Some(v) = b(doc, "dedup_pelny_status") {
        c.dedup_pelny_status = v;
    }
    if let Some(v) = b(doc, "edycja_wykonuje_reszte_akcji") {
        c.edycja_wykonuje_reszte_akcji = v;
    }
    if let Some(v) = b(doc, "dedup_klucz_z_wartoscia") {
        c.dedup_klucz_z_wartoscia = v;
    }
    if let Some(v) = b(doc, "edycja_sieroty_nie_otwiera") {
        c.edycja_sieroty_nie_otwiera = v;
    }
    if let Some(v) = b(doc, "entry_idempotencja") {
        c.entry_idempotencja = v;
    }
    if let Some(v) = b(doc, "dedup_management_po_restarcie") {
        c.dedup_management_po_restarcie = v;
    }
    // ---- Pakiet B: osie z audytu TYLER (nazwy wspólne z rdzeniem) ----
    if let Some(v) = b(doc, "rf_wymaga_wykonania") {
        c.rf_wymaga_wykonania = v;
    }
    if let Some(v) = u(doc, "market_entry_units") {
        c.market_entry_units = v;
    }
    if let Some(v) = u(doc, "market_hybrid_now_units") {
        c.market_hybrid_now_units = v;
    }
    if let Some(v) = u(doc, "market_hybrid_pending_units") {
        c.market_hybrid_pending_units = v;
    }
    if let Some(v) = f(doc, "market_hybrid_lot_mult") {
        c.market_hybrid_lot_mult = v;
    }
    if let Some(v) = f(doc, "market_hybrid_max_chase_usd") {
        c.market_hybrid_max_chase_usd = v;
    }
    if let Some(v) = u(doc, "market_hybrid_tp_stage") {
        c.market_hybrid_tp_stage = v.min(u8::MAX as u32) as u8;
    }
    if let Some(v) = u(doc, "market_unfilled_cancel_stage") {
        c.market_unfilled_cancel_stage = v.min(u8::MAX as u32) as u8;
    }
    if let Some(v) = b(doc, "pending_cancel_on_riskfree") {
        c.pending_cancel_on_riskfree = v;
    }
    if let Some(v) = u(doc, "bank_all_at_stage") {
        c.bank_all_at_stage = v.min(u8::MAX as u32) as u8;
    }
    // Pakiet E1: próg remisu w raportach — oś POMIARU, nie handlu.
    if let Some(v) = f(doc, "stat_be_prog_usd") {
        c.stat_be_prog_usd = v;
    }
    // „TOLERANCJA CEN" z panelu — w rdzeniu służy do przypisania komunikatu do
    // właściwego koszyka po podanym poziomie
    if let Some(v) = f(doc, "price_tol") {
        c.basket_hint_tolerance = v;
    }
    if let Some(v) = f(doc, "oae_timeout_min") {
        c.oae_timeout_min = v;
    }
    if let Some(v) = f(doc, "oae_profit_min") {
        c.oae_profit_min = v;
    }

    // ---------- breakeven / trailing ----------
    if b(doc, "be_lock").unwrap_or(false) {
        if let Some(v) = f(doc, "be_lock_points") {
            c.be_lock_pts = v;
        }
    }
    if let Some(v) = b(doc, "be_at_tp1") {
        c.be_at_tp1 = v;
    }
    // Osie prowadzenia pozycji używane przez GOD-X4/GOD-X5. Backtest czyta
    // je wprost z JSON, a żywy bot przechodzi przez ten most — pominięcie
    // oznacza dwie strategie pod tą samą nazwą presetu.
    if let Some(v) = u(doc, "be_od_etapu") {
        c.be_od_etapu = v.min(u8::MAX as u32) as u8;
    }
    if let Some(v) = u(doc, "be_min_pozycji") {
        c.be_min_pozycji = v;
    }
    if let Some(v) = b(doc, "cele_pomin_za_cena") {
        c.cele_pomin_za_cena = v;
    }
    if let Some(v) = b(doc, "entry_jeden_na_glebokiej") {
        c.entry_jeden_na_glebokiej = v;
    }
    if let Some(v) = b(doc, "sl_po_tp1_na_krawedz") {
        c.sl_po_tp1_na_krawedz = v;
    }
    if let Some(v) = f(doc, "sl_wlasny_na_pozycje") {
        c.sl_wlasny_na_pozycje = v;
    }
    // Nazwy trybu przyjmujemy w OBU konwencjach — panel pisze `"gap"`, preset
    // z pliku niesie `"Gap"`. Rozpoznawanie samej konwencji panelu oznaczało,
    // że wartość z presetu „nie pasuje do żadnej gałęzi" i po cichu wygrywa
    // wartość domyślna rdzenia.
    //
    // `"off"` MUSI być osobną gałęzią, a nie brakiem dopasowania. `None` znaczy
    // tu „klucza nie ma / nie rozumiem", więc wywołujący zostawia domyślną —
    // a domyślną `trail_runner_mode` jest `Tiered`. Preset z jawnym
    // `trail_runner_mode = "Off"` WŁĄCZAŁ więc rodzinę runnerową zamiast ją
    // wyłączyć: zmierzone −923 $ na oknie czerwiec–lipiec (ULTRA-X3).
    let mode_of = |key: &str| -> Option<TrailMode> {
        match s(doc, key).as_deref() {
            Some("gap") | Some("Gap") => Some(TrailMode::Gap),
            Some("lock_pct") | Some("LockPct") => Some(TrailMode::LockPct),
            Some("tiered") | Some("Tiered") => Some(TrailMode::Tiered),
            Some("atr") | Some("Atr") => Some(TrailMode::Atr),
            Some("chandelier") | Some("Chandelier") => Some(TrailMode::Chandelier),
            Some("off") | Some("Off") => Some(TrailMode::Off),
            _ => None,
        }
    };
    // TRAILING i SMART SL to dwie różne rzeczy i `bot.py` też je rozdzielał:
    // „runner trail" podąża za CENĄ, „smart SL" wspina się po drabince CELÓW,
    // różnicując stopy wewnątrz koszyka wg jakości wejścia.
    if b(doc, "runner_trail").unwrap_or(false) {
        c.trail_mode = mode_of("trail_mode").unwrap_or(TrailMode::Gap);
    }
    let be_layer = b(doc, "breakeven_protection").unwrap_or(false);
    // `risk_free_smart_sl` w poprzednim bocie włączał drabinkę stopów DLA
    // KOSZYKA PO RISK FREE. Odwzorowujemy to jako drabinkę ograniczoną do
    // koszyków zabezpieczonych, zamiast po cichu włączać ją wszędzie.
    let rf_smart = b(doc, "risk_free_smart_sl").unwrap_or(false);
    let ladder = b(doc, "smart_sl").unwrap_or(false) || rf_smart;
    c.smart_sl_only_after_rf = rf_smart && !b(doc, "smart_sl").unwrap_or(false);
    c.smart_sl_mode = match (ladder, be_layer) {
        (true, true) => SmartSlMode::LadderWithBe,
        (true, false) => SmartSlMode::Ladder,
        (false, true) => SmartSlMode::BreakevenOnly,
        (false, false) => SmartSlMode::Off,
    };
    // „TRAIL ONLY AFTER TP2" = opóźnienie całej drabinki o jeden szczebel.
    // Przełącznik jest dwustanowy, więc sam w sobie nie umie oddać wartości
    // większej niż 1 — dlatego obok jedzie klucz liczbowy `smart_sl_delay_n`
    // i ma pierwszeństwo. Bez tego `smart_sl_delay = 2` (NEWALPHA-2, OMEGA-1)
    // spadało po pierwszej edycji z panelu do 1, cicho i nieodwracalnie.
    c.smart_sl_delay = if b(doc, "trail_after_tp2").unwrap_or(false) {
        1
    } else {
        0
    };
    if let Some(v) = f(doc, "smart_sl_delay_n") {
        c.smart_sl_delay = v.max(0.0) as usize;
    }
    if let Some(v) = b(doc, "smart_sl_floor_be_after_rf") {
        c.smart_sl_floor_be_after_rf = v;
    }
    if let Some(v) = f(doc, "be_offset") {
        c.be_offset = v;
    }
    // W31a: czy komenda „set BE" kryje pozycje wypełnione PO niej.
    if let Some(v) = b(doc, "be_covers_late_fills") {
        c.be_covers_late_fills = v;
    }
    if let Some(v) = f(doc, "sltp_retry_s") {
        c.sltp_retry_s = v;
    }
    if let Some(v) = f(doc, "runner_trail_start") {
        c.trail_start = v;
    }
    if let Some(v) = f(doc, "runner_trail_gap") {
        c.trail_gap = v;
    }
    if let Some(v) = f(doc, "trail_lock_pct") {
        c.trail_lock_pct = v;
    }
    if let Some(v) = s(doc, "trail_tiers") {
        c.trail_tiers = v;
    }
    if let Some(v) = b(doc, "trail_split") {
        c.trail_split = v;
    }
    if let Some(v) = u(doc, "trail_runners_n") {
        c.trail_runners_n = v;
    }
    if let Some(m) = mode_of("trail_runner_mode") {
        c.trail_runner_mode = m;
    }
    if let Some(v) = f(doc, "trail_runner_start") {
        c.trail_runner_start = v;
    }
    if let Some(v) = f(doc, "trail_runner_gap") {
        c.trail_runner_gap = v;
    }
    if let Some(v) = f(doc, "trail_runner_lock_pct") {
        c.trail_runner_lock_pct = v;
    }
    if let Some(v) = s(doc, "trail_runner_tiers") {
        c.trail_runner_tiers = v;
    }
    if let Some(v) = f(doc, "trail_min_dist") {
        c.trail_min_dist = v;
    }
    // ---------- trailing S/R po strukturze 1M (OS_SR_SPEC.md) ----------
    // Brak klucza zostawia domyślne (rodzina WYŁĄCZONA) — kontrakt zera.
    // Enumy w OBU konwencjach jak `mode_of` wyżej: panel niesie wartości
    // rdzenia ("Runner", "Tp2"), ale małe litery też rozumiemy, żeby ręcznie
    // pisany preset nie przegrywał po cichu z wartością domyślną.
    if let Some(v) = b(doc, "trail_sr_enabled") {
        c.trail_sr_enabled = v;
    }
    if let Some(v) = s(doc, "trail_sr_scope") {
        c.trail_sr_scope = match v.as_str() {
            "runner" | "Runner" => TrailSrScope::Runner,
            "tp3_up" | "Tp3Up" => TrailSrScope::Tp3Up,
            "all" | "All" => TrailSrScope::All,
            _ => c.trail_sr_scope,
        };
    }
    if let Some(v) = s(doc, "trail_sr_activation") {
        c.trail_sr_activation = match v.as_str() {
            "entry" | "Entry" => TrailSrActivation::Entry,
            "gain" | "Gain" => TrailSrActivation::Gain,
            "tp1" | "Tp1" => TrailSrActivation::Tp1,
            "tp2" | "Tp2" => TrailSrActivation::Tp2,
            "tp3" | "Tp3" => TrailSrActivation::Tp3,
            _ => c.trail_sr_activation,
        };
    }
    if let Some(v) = f(doc, "trail_sr_min_gain") {
        c.trail_sr_min_gain = v;
    }
    if let Some(v) = f(doc, "trail_sr_min_dist_price") {
        c.trail_sr_min_dist_price = v;
    }
    // Parametry S/R były w Settings i presetach, lecz ginęły wyłącznie na
    // drodze panel→live. Wartości domyślne także mapujemy jawnie, by test
    // round-trip wykrywał przyszłe rozjazdy zamiast ufać przypadkowej równości.
    if let Some(v) = u(doc, "trail_sr_tf_min") {
        c.trail_sr_tf_min = v;
    }
    if let Some(v) = u(doc, "trail_sr_fractal_n") {
        c.trail_sr_fractal_n = v;
    }
    if let Some(v) = f(doc, "trail_sr_offset") {
        c.trail_sr_offset = v;
    }
    if let Some(v) = f(doc, "trail_sr_min_dist_tp") {
        c.trail_sr_min_dist_tp = v;
    }
    if let Some(v) = u(doc, "trail_sr_struct_window_h") {
        c.trail_sr_struct_window_h = v;
    }
    if let Some(v) = f(doc, "trail_sr_min_prominence_atr") {
        c.trail_sr_min_prominence_atr = v;
    }
    if let Some(v) = f(doc, "trail_sr_offset_atr_mult") {
        c.trail_sr_offset_atr_mult = v;
    }
    if let Some(v) = f(doc, "trail_sr_offset_spread_mult") {
        c.trail_sr_offset_spread_mult = v;
    }
    if let Some(v) = u(doc, "trail_sr_atr_period") {
        c.trail_sr_atr_period = v;
    }
    // ---------- EA-CORE — szkielet warstwy EA (FALA 0, 24.08.2026) ----------
    //
    // Brak klucza zostawia domyślne, czyli WARSTWĘ WYŁĄCZONĄ — kontrakt zera
    // idzie przez panel tak samo jak przez plik presetu. Enumy w OBU
    // konwencjach (wartość rdzenia i małe litery), jak `trail_sr_*` wyżej:
    // ręcznie pisany preset nie ma przegrywać po cichu z domyślną.
    if let Some(v) = b(doc, "ea_enabled") {
        c.ea_enabled = v;
    }
    if let Some(v) = f(doc, "ea_tick_s") {
        c.ea_tick_s = v;
    }
    if let Some(v) = s(doc, "ea_state_src") {
        c.ea_state_src = match v.as_str() {
            "float_r" | "FloatR" => EaStateSrc::FloatR,
            "float_pct_equity" | "FloatPctEquity" => EaStateSrc::FloatPctEquity,
            _ => c.ea_state_src,
        };
    }
    if let Some(v) = f(doc, "ea_defense_enter") {
        c.ea_defense_enter = v;
    }
    if let Some(v) = f(doc, "ea_defense_exit") {
        c.ea_defense_exit = v;
    }
    if let Some(v) = f(doc, "ea_offense_enter") {
        c.ea_offense_enter = v;
    }
    if let Some(v) = f(doc, "ea_offense_exit") {
        c.ea_offense_exit = v;
    }
    if let Some(v) = f(doc, "ea_state_dwell_s") {
        c.ea_state_dwell_s = v;
    }
    if let Some(v) = s(doc, "ea_state_ratchet") {
        c.ea_state_ratchet = match v.as_str() {
            "nie_luzuj_w_koszyku" | "NieLuzujWKoszyku" => EaRatchet::NieLuzujWKoszyku,
            "swobodny" | "Swobodny" => EaRatchet::Swobodny,
            _ => c.ea_state_ratchet,
        };
    }
    if let Some(v) = b(doc, "ea_state_journal") {
        c.ea_state_journal = v;
    }
    if let Some(v) = b(doc, "ea_dozor_sl") {
        c.ea_dozor_sl = v;
    }

    // ---------- RODZINA A: EKSPOZYCJA WOBEC STANU RACHUNKU ----------
    //
    // OSIEM PÓL, KTÓRE DZIAŁAŁY W BACKTEŚCIE I GINĘŁY NA ŻYWO (25.08.2026).
    //
    // Osie A1–A4 są wpięte w silnik (`Engine::ea_sufit_jednostek`,
    // `place_grid`), a `bt.exe` wczytuje preset wprost do `Settings` przez
    // serde — więc backtest je honoruje. Bot na żywo idzie DRUGĄ drogą:
    // preset ląduje w `settings.json`, a silnik dostaje wynik `core_from_ui`,
    // który startuje od `Settings::default()` i przypisuje WYŁĄCZNIE pola
    // wymienione w tym pliku. Pola nietłumaczone wracają więc do zera —
    // po cichu, bez ostrzeżenia, przy poprawnej wartości w pliku.
    //
    // To jest DOKŁADNIE ta klasa błędu, którą dokumentuje komentarz na górze
    // tej funkcji (30.07.2026: `tp_schedule` i `zone_offset_mode` stały
    // w pliku, a silnik grał domyślnymi — cztery pola razem dały różnicę
    // 1936 $ → −133 $ na oknie czerwiec–lipiec). Bez tego wpisu preset EA
    // zmierzony jako zyskowny handlowałby na żywo BEZ warstwy, która dała
    // mu ten wynik — a różnicy nie zobaczyłby nikt.
    //
    // `unmapped_keys` tego nie łapie z konstrukcji: sprawdza klucze OBECNE
    // W DOKUMENCIE, a dopóki żaden preset nie ma pól rodziny A, dokument ich
    // nie niesie i alarm milczy. Wykrywalne jest to dopiero od strony
    // `Settings` — patrz test `rodzina_a_przezywa_obieg_panelu` niżej.
    if let Some(v) = f(doc, "ea_lot_z_wolnego_marginesu") {
        c.ea_lot_z_wolnego_marginesu = v;
    }
    if let Some(v) = f(doc, "ea_stop_dokladek_przy_stracie") {
        c.ea_stop_dokladek_przy_stracie = v;
    }
    if let Some(v) = f(doc, "ea_stop_dokladek_powrot") {
        c.ea_stop_dokladek_powrot = v;
    }
    if let Some(v) = f(doc, "ea_redukcja_przy_zageszczeniu") {
        c.ea_redukcja_przy_zageszczeniu = v;
    }
    if let Some(v) = f(doc, "ea_zageszczenie_podloga") {
        c.ea_zageszczenie_podloga = v;
    }
    // Wariant zapisujemy OBIEMA konwencjami — panel pisze `snake_case`,
    // a preset zapisany przez serde niesie nazwę wariantu Rust. Ta sama
    // dwoistość co przy `ea_state_ratchet` kilka linijek wyżej.
    if let Some(v) = s(doc, "ea_stan_dnia") {
        c.ea_stan_dnia = match v.as_str() {
            "off" | "Off" => EaStanDnia::Off,
            "tylko_inkaso" | "TylkoInkaso" => EaStanDnia::TylkoInkaso,
            _ => c.ea_stan_dnia,
        };
    }
    if let Some(v) = f(doc, "ea_stan_dnia_prog_sl") {
        c.ea_stan_dnia_prog_sl = v as u32;
    }
    if let Some(v) = f(doc, "ea_stan_dnia_jednostki_mult") {
        c.ea_stan_dnia_jednostki_mult = v;
    }
    if let Some(v) = f(doc, "ladder_from_tp") {
        c.ladder_from_tp = v as usize;
    }
    if let Some(v) = f(doc, "ladder_lag") {
        c.ladder_lag = v as usize;
    }
    if let Some(v) = f(doc, "ladder_offset") {
        c.ladder_offset = v;
    }

    // ---------- wyjścia ----------
    if b(doc, "harvest").unwrap_or(false) {
        if let Some(v) = f(doc, "harvest_retrace_pct") {
            c.harvest_retrace_pct = v;
        }
        if let Some(v) = f(doc, "harvest_start") {
            c.harvest_start = v;
        }
    }
    if let Some(v) = f(doc, "stale_take_min") {
        c.stale_take_min = v;
    }
    if let Some(v) = f(doc, "stale_take_profit") {
        c.stale_take_profit = v;
    }
    if let Some(v) = f(doc, "stale_take_min2") {
        c.stale_take_min2 = v;
    }
    if let Some(v) = f(doc, "stale_take_profit2") {
        c.stale_take_profit2 = v;
    }
    if let Some(v) = f(doc, "rev_exit_range") {
        c.rev_exit_range = v;
    }
    if let Some(v) = f(doc, "rev_exit_slope") {
        c.rev_exit_slope = v;
    }
    if let Some(v) = f(doc, "rev_exit_profit") {
        c.rev_exit_profit = v;
    }
    if let Some(v) = f(doc, "rev_exit_window_min") {
        c.rev_exit_window_min = v;
    }
    // ---------- reguły doświadczonego tradera ----------
    // Nazwy wspólne z rdzeniem, więc przepisujemy wprost. Wszystkie domyślnie
    // wyłączone (0 / false), więc brak klucza w dokumencie NIE zmienia
    // zachowania żadnego istniejącego presetu.
    if let Some(v) = f(doc, "exit_min_hold_min") {
        c.exit_min_hold_min = v;
    }
    if let Some(v) = f(doc, "exit_min_profit") {
        c.exit_min_profit = v;
    }
    if let Some(v) = f(doc, "exit_r_multiple") {
        c.exit_r_multiple = v;
    }
    if let Some(v) = f(doc, "basket_target_usd") {
        c.basket_target_usd = v;
    }
    if let Some(v) = f(doc, "exit_round_dist") {
        c.exit_round_dist = v;
    }
    if let Some(v) = f(doc, "exit_round_step") {
        c.exit_round_step = v;
    }
    if let Some(v) = f(doc, "exit_spread_mult") {
        c.exit_spread_mult = v;
    }
    if let Some(v) = b(doc, "exit_on_opposite_signal") {
        c.exit_on_opposite_signal = v;
    }
    if let Some(v) = f(doc, "hold_after_tp_hit_min") {
        c.hold_after_tp_hit_min = v;
    }
    if let Some(v) = b(doc, "toucher_tp_one_based") {
        c.toucher_tp_one_based = v;
    }
    if let Some(v) = b(doc, "pending_drop_arm") {
        c.pending_drop_arm = v;
    }
    if let Some(v) = b(doc, "ml_licz_wiszace") {
        c.ml_licz_wiszace = v;
    }
    if let Some(v) = f(doc, "ml_min_wejscie") {
        c.ml_min_wejscie = v;
    }
    if let Some(v) = f(doc, "ml_min_warstwa") {
        c.ml_min_warstwa = v;
    }
    if let Some(v) = f(doc, "ml_min_reentry") {
        c.ml_min_reentry = v;
    }
    if let Some(v) = f(doc, "ml_min_rearm") {
        c.ml_min_rearm = v;
    }
    if let Some(v) = f(doc, "ml_min_piramida") {
        c.ml_min_piramida = v;
    }
    if let Some(v) = f(doc, "ml_min_fast_addon") {
        c.ml_min_fast_addon = v;
    }
    if let Some(v) = f(doc, "ml_min_relot_up") {
        c.ml_min_relot_up = v;
    }
    if let Some(v) = f(doc, "ml_min_drabina") {
        c.ml_min_drabina = v;
    }
    if let Some(v) = f(doc, "konto_dzwignia") {
        c.konto_dzwignia = v;
    }
    if let Some(v) = b(doc, "wiek_od_wypelnienia") {
        c.wiek_od_wypelnienia = v;
    }
    if let Some(v) = f(doc, "pending_drop_grace_min") {
        c.pending_drop_grace_min = v;
    }
    if let Some(v) = f(doc, "pending_drop_grace_max_dist") {
        c.pending_drop_grace_max_dist = v;
    }
    if let Some(v) = u(doc, "pending_drop_keep_n") {
        c.pending_drop_keep_n = v;
    }
    if let Some(v) = b(doc, "grid_anchor_absolute") {
        c.grid_anchor_absolute = v;
    }
    // G1: druga połowa rozdzielonej kraty — mnożnik zleceń na poziomie.
    // Brak klucza zostawia domyślne `true`, czyli stare zachowanie.
    if let Some(v) = b(doc, "units_per_level") {
        c.units_per_level = v;
    }
    // U-BUG38R: czy mnożnik kraty łapie też sygnał strefowy (rodzaj pusty
    // + `auto_limit`). Brak klucza zostawia domyślne `true` — stare
    // zachowanie i parytet co do bitu.
    if let Some(v) = b(doc, "units_per_level_zone") {
        c.units_per_level_zone = v;
    }
    // Brak klucza = `Market`, czyli zachowanie symulatora. Preset sprzed tej
    // zmiany dostaje więc dokładnie to, co mierzył backtest.
    if let Some(v) = s(doc, "pending_cross_policy") {
        c.pending_cross_policy = match v.as_str() {
            "Stop" => PendingCrossPolicy::Stop,
            "Shift" => PendingCrossPolicy::Shift,
            "Skip" => PendingCrossPolicy::Skip,
            _ => PendingCrossPolicy::Market,
        };
    }
    if let Some(v) = b(doc, "tp_open_extra") {
        c.tp_open_extra = v;
    }

    // ---------- mądre wyjście ----------
    // Nazwy pól są w panelu i w rdzeniu IDENTYCZNE, więc przepisujemy je
    // wprost. Ten blok istnieje dlatego, że bez niego cała rodzina
    // `smart_exit_*` przechodziła przez `settings.json` nietknięta, ale
    // `core_from_ui` jej nie czytało — backtest honorował ustawienie,
    // a bot na żywo grał bez niego pod tą samą nazwą presetu.
    if let Some(v) = b(doc, "smart_exit") {
        c.smart_exit = v;
    }
    if let Some(v) = f(doc, "smart_exit_take") {
        c.smart_exit_take = v;
    }
    if let Some(v) = f(doc, "smart_exit_giveback") {
        c.smart_exit_giveback = v;
    }
    if let Some(v) = f(doc, "smart_exit_min_peak") {
        c.smart_exit_min_peak = v;
    }
    if let Some(v) = f(doc, "smart_exit_drop_speed") {
        c.smart_exit_drop_speed = v;
    }
    if let Some(v) = f(doc, "smart_exit_speed_window_s") {
        c.smart_exit_speed_window_s = v;
    }
    if let Some(v) = f(doc, "smart_exit_hold_if_pending") {
        c.smart_exit_hold_if_pending = v;
    }
    if let Some(v) = u(doc, "smart_exit_min_pendings") {
        c.smart_exit_min_pendings = v;
    }
    if let Some(v) = s(doc, "smart_exit_pending_scope") {
        c.smart_exit_pending_scope = match v.as_str() {
            "AnyBasket" | "anyBasket" | "any" => PendingScope::AnyBasket,
            _ => PendingScope::SameBasket,
        };
    }
    if let Some(v) = f(doc, "smart_exit_pending_min_dist") {
        c.smart_exit_pending_min_dist = v;
    }

    // ---------- RISK FREE JAKO REGUŁA (nie reakcja na komunikat) ----------
    // Nazwy w rdzeniu i w panelu są IDENTYCZNE, więc przepisujemy wprost.
    // Wszystko domyślnie neutralne (false / 0), więc brak klucza w dokumencie
    // NIE zmienia zachowania żadnego istniejącego presetu.
    if let Some(v) = b(doc, "riskfree_enabled") {
        c.riskfree_enabled = v;
    }
    if let Some(v) = f(doc, "riskfree_trigger_usd") {
        c.riskfree_trigger_usd = v;
    }
    if let Some(v) = f(doc, "riskfree_trigger_r") {
        c.riskfree_trigger_r = v;
    }
    if let Some(v) = u(doc, "riskfree_keep_units") {
        c.riskfree_keep_units = v;
    }
    if let Some(v) = f(doc, "riskfree_be_offset") {
        c.riskfree_be_offset = v;
    }
    if let Some(v) = s(doc, "riskfree_runner_target") {
        c.riskfree_runner_target = match v.as_str() {
            "KeepTp" | "keep" => RiskFreeRunnerTarget::KeepTp,
            "NoTpTrailOnly" | "none" => RiskFreeRunnerTarget::NoTpTrailOnly,
            "NextTp" | "next" => RiskFreeRunnerTarget::NextTp,
            _ => RiskFreeRunnerTarget::LastTp,
        };
    }
    if let Some(v) = s(doc, "riskfree_runner_stop") {
        c.riskfree_runner_stop = match v.as_str() {
            "BeOwn" => RiskFreeRunnerStop::BeOwn,
            "TrailGap" => RiskFreeRunnerStop::TrailGap,
            "Off" => RiskFreeRunnerStop::Off,
            _ => RiskFreeRunnerStop::Be,
        };
    }
    if let Some(v) = f(doc, "riskfree_runner_gap") {
        c.riskfree_runner_gap = v;
    }
    if let Some(v) = f(doc, "riskfree_runner_max_hold_min") {
        c.riskfree_runner_max_hold_min = v;
    }

    // ---------- AI: równolegle czy zamiast reguł ----------
    // Nazwa `ai_enabled` nie mówiła najważniejszego: model włączony ZAMIAST
    // zarządzania zdejmuje wszystkie zabezpieczenia. To jest osobny
    // przełącznik właśnie po to, żeby ta różnica była widoczna.
    if let Some(v) = b(doc, "ai_replaces_management") {
        c.ai_replaces_management = v;
    }

    // ---------- MODEL KOSZTÓW BROKERA (Priorytet 0, NAUKOWIEC.md §8) ----------
    //
    // Te pola NIE są strategią — są modelem rzeczywistości. Wyłączony swap nie
    // jest ustawieniem neutralnym, tylko błędem modelu, dlatego jako jedyne
    // w tym pliku przychodzą domyślnie WŁĄCZONE. Przy 72 h trzymania sam swap
    // to −3 621 $ na próbce, zweryfikowane dealami z konta.
    if let Some(v) = b(doc, "swap_enabled") {
        c.swap_enabled = v;
    }
    if let Some(v) = f(doc, "swap_long_points") {
        c.swap_long_points = v;
    }
    if let Some(v) = f(doc, "swap_short_points") {
        c.swap_short_points = v;
    }
    if let Some(v) = f(doc, "swap_point_value") {
        c.swap_point_value = v;
    }
    if let Some(v) = u(doc, "swap_rollover_weekday") {
        c.swap_rollover_weekday = v;
    }
    if let Some(v) = f(doc, "swap_rollover_mult") {
        c.swap_rollover_mult = v;
    }
    // Pakiet D1/D1b: noce weekendowe i doba rolowania wyliczona z serwera.
    if let Some(v) = b(doc, "swap_pomijaj_weekend") {
        c.swap_pomijaj_weekend = v;
    }
    if let Some(v) = b(doc, "swap_rollover_z_serwera") {
        c.swap_rollover_z_serwera = v;
    }
    if let Some(v) = u(doc, "swap_rollover3days_mt5") {
        c.swap_rollover3days_mt5 = v;
    }
    // Pakiet D5/D6/D4 i D3: wierność samej pętli backtestu.
    if let Some(v) = b(doc, "runner_ksiegowanie_v2") {
        c.runner_ksiegowanie_v2 = v;
    }
    if let Some(v) = b(doc, "msg_kurs_sprzed_luki") {
        c.msg_kurs_sprzed_luki = v;
    }
    if let Some(v) = b(doc, "live_tick_order_strict") {
        c.live_tick_order_strict = v;
    }
    if let Some(v) = f(doc, "slippage_pending_pts") {
        c.slippage_pending_pts = v;
    }
    if let Some(v) = f(doc, "stop_out_level_pct") {
        c.stop_out_level_pct = v;
    }
    if let Some(v) = f(doc, "margin_call_level_pct") {
        c.margin_call_level_pct = v;
    }
    if let Some(v) = f(doc, "entry_depth_curve") {
        c.entry_depth_curve = v;
    }
    // Jawne układy/krzywe wejścia i kotwice TP są tekstowymi DSL-ami.
    // Nie interpretujemy ich w moście — mają dojść co do znaku do rdzenia.
    if let Some(v) = s(doc, "entry_uklad") {
        c.entry_uklad = v;
    }
    if let Some(v) = s(doc, "entry_uklad_kotwica") {
        c.entry_uklad_kotwica = v;
    }
    if let Some(v) = s(doc, "entry_krzywa_kotwica") {
        c.entry_krzywa_kotwica = v;
    }
    if let Some(v) = s(doc, "tp_drabinka_kotwica") {
        c.tp_drabinka_kotwica = v;
    }
    if let Some(v) = f(doc, "entry_warstwy_offset") {
        c.entry_warstwy_offset = v;
    }
    if let Some(v) = b(doc, "entry_warstwy_z_tekstu") {
        c.entry_warstwy_z_tekstu = v;
    }
    if let Some(v) = u(doc, "runner_cele_n") {
        c.runner_cele_n = v;
    }
    if let Some(v) = f(doc, "runner_cele_krok") {
        c.runner_cele_krok = v;
    }
    if let Some(v) = f(doc, "runner_partial_pct") {
        c.runner_partial_pct = v;
    }
    // W30: warstwa allowance przed strefą (kanon Tylera). Dwa pola, bo kwota
    // mówi GDZIE, a jednostki ILE — brak któregokolwiek zostawia zero, czyli
    // plan siatki bajt w bajt jak przed 24.08.2026.
    if let Some(v) = f(doc, "entry_allowance_usd") {
        c.entry_allowance_usd = v;
    }
    if let Some(v) = u(doc, "entry_allowance_units") {
        c.entry_allowance_units = v;
    }

    // ---------- twardy czas życia koszyka ----------
    if let Some(v) = f(doc, "basket_max_age_min") {
        c.basket_max_age_min = v;
    }

    // ---------- filtr trendu wyższego rzędu ----------
    if let Some(v) = b(doc, "trend_filter_enabled") {
        c.trend_filter_enabled = v;
    }
    if let Some(v) = f(doc, "trend_filter_window_h") {
        c.trend_filter_window_h = v;
    }
    if let Some(v) = f(doc, "trend_filter_drop_pct") {
        c.trend_filter_drop_pct = v;
    }
    if let Some(v) = s(doc, "trend_filter_mode") {
        c.trend_filter_mode = match v.as_str() {
            "Block" | "block" => TrendFilterMode::Block,
            _ => TrendFilterMode::Shrink,
        };
    }
    if let Some(v) = f(doc, "trend_filter_shrink") {
        c.trend_filter_shrink = v;
    }

    // ---------- poprawki istniejących reguł (pod przełącznikiem) ----------
    if let Some(v) = b(doc, "pending_drop_require_zone_touch") {
        c.pending_drop_require_zone_touch = v;
    }
    if let Some(v) = b(doc, "trail_runners_by_depth") {
        c.trail_runners_by_depth = v;
    }

    // ---------- wielkość pozycji wg jakości szczebla ----------
    if let Some(v) = b(doc, "entry_weights_from_rr") {
        c.entry_weights_from_rr = v;
    }
    if let Some(v) = f(doc, "entry_weights_rr_power") {
        c.entry_weights_rr_power = v;
    }
    if let Some(v) = f(doc, "entry_weights_rr_cap") {
        c.entry_weights_rr_cap = v;
    }
    // JAWNE ODRZUCANIE SZCZEBLA, KTÓREGO BROKER I TAK NIE PRZYJMIE.
    //
    // Pole dołożone przy pracy nad „szczeblem widmem": przy geometrii ULTRA-X3
    // najgłębszy szczebel siatki leży DOKŁADNIE na stopie w 1828 z 1865
    // sygnałów (98,0 %), więc broker odrzuca go z kodem 10016 — a jego ryzyko
    // i tak wchodzi do wyliczenia wolumenu pozostałych szczebli.
    //
    // Bez tej linijki pole było MARTWE po drodze z panelu: silnik je czyta,
    // preset je niesie, ale `core_from_ui` gubiło je po cichu. To ta sama
    // usterka, przez którą bot handlował lotem 0,01 zamiast 0,5 % kapitału.
    if let Some(v) = b(doc, "drop_unplaceable_levels") {
        c.drop_unplaceable_levels = v;
    }

    // NIEZMIENNIK KRAWĘDZI STREFY. Bez tej linijki pole byłoby martwe po
    // drodze z panelu — dokładnie tak przepadło `drop_unplaceable_levels`
    // dwa akapity wyżej.
    if let Some(v) = b(doc, "zakaz_ponizej_krawedzi") {
        c.zakaz_ponizej_krawedzi = v;
    }

    // ---------- parametry liczone z SYGNAŁU ----------
    if let Some(v) = b(doc, "adaptive_params") {
        c.adaptive_params = v;
    }
    if let Some(v) = f(doc, "sl_min_dist_zone_mult") {
        c.sl_min_dist_zone_mult = v;
    }
    if let Some(v) = f(doc, "sl_min_dist_atr_mult") {
        c.sl_min_dist_atr_mult = v;
    }
    if let Some(v) = f(doc, "sl_min_dist_floor") {
        c.sl_min_dist_floor = v;
    }
    if let Some(v) = f(doc, "sl_min_dist_cap") {
        c.sl_min_dist_cap = v;
    }
    if let Some(v) = f(doc, "entry_deep_zone_mult") {
        c.entry_deep_zone_mult = v;
    }
    if let Some(v) = f(doc, "entry_units_zone_ref") {
        c.entry_units_zone_ref = v;
    }
    if let Some(v) = f(doc, "adaptive_atr_window_min") {
        c.adaptive_atr_window_min = v;
    }
    if let Some(v) = s(doc, "units_by_hour") {
        c.units_by_hour = v;
    }

    // ---------- skalowanie po zdarzeniu ----------
    if let Some(v) = b(doc, "basket_realized_broker_only") {
        c.basket_realized_broker_only = v;
    }
    if let Some(v) = b(doc, "confirmed_exit_retry") {
        c.confirmed_exit_retry = v;
    }
    if let Some(v) = b(doc, "defer_entry_until_receipts") {
        c.defer_entry_until_receipts = v;
    }
    if let Some(v) = f(doc, "deferred_entry_max_age_s") {
        c.deferred_entry_max_age_s = v;
    }
    if let Some(v) = b(doc, "rearm_grid_on_return") {
        c.rearm_grid_on_return = v;
    }
    if let Some(v) = b(doc, "rearm_keep_empty_alive") {
        c.rearm_keep_empty_alive = v;
    }
    if let Some(v) = b(doc, "rearm_bez_pozycji") {
        c.rearm_bez_pozycji = v;
    }
    if let Some(v) = f(doc, "rearm_bez_pozycji_max_h") {
        c.rearm_bez_pozycji_max_h = v;
    }
    if let Some(v) = b(doc, "rearm_block_after_secured") {
        c.rearm_block_after_secured = v;
    }
    if let Some(v) = b(doc, "spp_blocks_rearm_when_flat") {
        c.spp_blocks_rearm_when_flat = v;
    }
    if let Some(v) = f(doc, "rearm_min_basket_profit") {
        c.rearm_min_basket_profit = v;
    }
    if let Some(v) = u(doc, "rearm_max_times") {
        c.rearm_max_times = v;
    }
    if let Some(v) = f(doc, "rearm_min_gap_min") {
        c.rearm_min_gap_min = v;
    }

    // ---------- konto: cele i stopy dnia w procentach ----------
    if let Some(v) = f(doc, "day_target_pct") {
        c.day_target_pct = v;
    }
    if let Some(v) = f(doc, "profit_budget_arm_pct") { c.profit_budget_arm_pct = v; }
    if let Some(v) = f(doc, "profit_budget_keep_pct") { c.profit_budget_keep_pct = v; }
    if let Some(v) = f(doc, "profit_budget_deploy_pct") { c.profit_budget_deploy_pct = v; }
    if let Some(v) = f(doc, "day_trail_stop_pct") {
        c.day_trail_stop_pct = v;
    }
    if let Some(v) = f(doc, "day_trail_arm_pct") {
        c.day_trail_arm_pct = v;
    }
    if let Some(v) = s(doc, "day_trail_basis") {
        c.day_trail_basis = match v.as_str() {
            "profit_peak" | "ProfitPeak" => DayTrailBasis::ProfitPeak,
            _ => DayTrailBasis::EquityPeak,
        };
    }

    // ---------- budżet transakcji i jakość sygnału ----------
    if let Some(v) = u(doc, "daily_signal_budget") {
        c.daily_signal_budget = v;
    }
    if let Some(v) = f(doc, "signal_min_rr") {
        c.signal_min_rr = v;
    }
    if let Some(v) = f(doc, "signal_min_zone_width") {
        c.signal_min_zone_width = v;
    }
    if let Some(v) = f(doc, "signal_max_zone_width") {
        c.signal_max_zone_width = v;
    }

    // ---------- łączenie koszyków ----------
    if let Some(v) = b(doc, "merge_same_side") {
        c.merge_same_side = v;
    }
    if let Some(v) = f(doc, "merge_window_min") {
        c.merge_window_min = v;
    }
    if let Some(v) = f(doc, "merge_min_overlap") {
        c.merge_min_overlap = v;
    }

    // ---------- wyjście limitem ----------
    if let Some(v) = b(doc, "exit_via_limit") {
        c.exit_via_limit = v;
    }
    if let Some(v) = f(doc, "exit_limit_offset") {
        c.exit_limit_offset = v;
    }
    if let Some(v) = f(doc, "exit_limit_wait_s") {
        c.exit_limit_wait_s = v;
    }
    if let Some(v) = f(doc, "exit_limit_min_profit") {
        c.exit_limit_min_profit = v;
    }

    if let Some(v) = b(doc, "reenter_after_tp") {
        c.reenter_after_tp = v;
    }
    if let Some(v) = f(doc, "reenter_min_tp_stage") {
        c.reenter_min_tp_stage = v.max(0.0) as usize;
    }
    if let Some(v) = u(doc, "reenter_max") {
        c.reenter_max = v;
    }

    // ---------- bramki ----------
    if let Some(v) = b(doc, "session_filter") {
        c.session_filter = v;
    }
    if let Some(v) = s(doc, "session_hours") {
        c.session_hours = v;
    }
    if let Some(v) = u(doc, "max_open_positions") {
        c.max_open_positions = v;
    }
    if let Some(v) = b(doc, "exposure_count_pendings") {
        c.exposure_count_pendings = v;
    }
    // Bez tego `max_open_positions` jest bramką sprawdzaną WYŁĄCZNIE w chwili
    // sygnału, a wiszące zlecenia obchodzą ją przy wypełnieniu. Backtest już
    // to pole czytał (`engine.rs`), panel nie miał jak go włączyć.
    // Brak klucza = `false`, czyli zachowanie dotychczasowe.
    if let Some(v) = b(doc, "enforce_position_limit_on_fill") {
        c.enforce_position_limit_on_fill = v;
    }
    // Towarzysz powyższego (FAZA 1 poz. 7): kasuj tylko NADMIAR ponad limit,
    // nie cały świeży fill. Kontrakt zera: brak klucza = `false` = stare
    // zachowanie — pole musi mieć most od dnia narodzin, inaczej edycja
    // presetu z panelu zeruje je tak, jak 37 pól Fazy 6 niżej.
    if let Some(v) = b(doc, "limit_kasuje_tylko_nadmiar") {
        c.limit_kasuje_tylko_nadmiar = v;
    }
    // Wierność symulatora, nie strategia (jak `sim_stops_level`): czy przy
    // aktywacji zlecenia bez wolnego depozytu wypełnienie jest kasowane, tak
    // jak robi to realny MT5. Domyślnie WŁĄCZONE — `false` służy wyłącznie do
    // odtworzenia starego, błędnego wyniku w teście A/B.
    if let Some(v) = b(doc, "sim_margin_check_on_fill") {
        c.sim_margin_check_on_fill = v;
    }
    // Też wierność symulatora: SL/TP zlecenia oczekującego mierzone od CENY
    // AKTYWACJI (10016), czyli koniec „szczebli-widm". Brak klucza = `false`,
    // czyli zachowanie sprzed 31.07.2026.
    if let Some(v) = b(doc, "sim_validate_pending_stops") {
        c.sim_validate_pending_stops = v;
    }
    // Przeliczanie wolumenu leżących limitów po wzroście salda (anuluj
    // i złóż od nowa). Brak klucza = `false`, czyli zachowanie sprzed 31.07.
    if let Some(v) = b(doc, "pending_relot_on_balance") {
        c.pending_relot_on_balance = v;
    }
    // Metoda: `true` = dokładka osobnym zleceniem, `false` = anuluj i złóż.
    if let Some(v) = b(doc, "pending_relot_topup") {
        c.pending_relot_topup = v;
    }
    // KIERUNKI OSOBNO. Brak klucza = `true` po obu stronach, czyli dokładnie
    // to, co robił relot przed rozdzieleniem — HYPER-X1 nie drga.
    if let Some(v) = b(doc, "pending_relot_up") {
        c.pending_relot_up = v;
    }
    if let Some(v) = b(doc, "pending_relot_down") {
        c.pending_relot_down = v;
    }
    // Próg kapitału dla kierunku w górę (0 = bez progu).
    if let Some(v) = f(doc, "pending_relot_up_od_salda") {
        c.pending_relot_up_od_salda = v;
    }
    // Cel wg PLANU (wagi RR + limit ryzyka koszyka) zamiast gołego lota.
    // Brak klucza = `false`, czyli zachowanie sprzed 03.08.
    if let Some(v) = b(doc, "pending_relot_wg_planu") {
        c.pending_relot_wg_planu = v;
    }
    if let Some(v) = b(doc, "pending_relot_reconcile_target") {
        c.pending_relot_reconcile_target = v;
    }
    // Dokładka po celu pyta o budżet ryzyka koszyka. Brak klucza = `false`.
    if let Some(v) = b(doc, "reenter_respect_cap") {
        c.reenter_respect_cap = v;
    }
    // ---- NAPRAWY Z AUDYTU FABLE (Z-3, Z-5, Z-7…Z-10) ----
    // Wszystkie domyślnie `false`; brak klucza = zachowanie sprzed 31.07.2026.
    // Bez tego mapowania panel gubiłby je przy zapisie — dokładnie klasa błędu
    // `drop_unplaceable_levels`. Test `zaden_klucz_presetu_nie_ginie_po_tlumaczeniu`.
    if let Some(v) = b(doc, "sl_edit_reaches_pendings") {
        c.sl_edit_reaches_pendings = v;
    }
    if let Some(v) = b(doc, "honor_stop_orders") {
        c.honor_stop_orders = v;
    }
    if let Some(v) = b(doc, "hint_veto") {
        c.hint_veto = v;
    }
    // Pakiet F1: weto odpowiedzi — komunikat z `reply_to` do wiadomości,
    // której nie mamy, nie spada na najnowszy żywy koszyk.
    if let Some(v) = b(doc, "reply_veto") {
        c.reply_veto = v;
    }
    if let Some(v) = b(doc, "sync_only_live_levels") {
        c.sync_only_live_levels = v;
    }
    if let Some(v) = b(doc, "tp_correction_to_broker") {
        c.tp_correction_to_broker = v;
    }
    if let Some(v) = b(doc, "runner_max_hold_rule_only") {
        c.runner_max_hold_rule_only = v;
    }
    // M15: limit trzymania runnera działa TAKŻE przy `riskfree_enabled = false`.
    if let Some(v) = b(doc, "runner_max_hold_bez_reguly") {
        c.runner_max_hold_bez_reguly = v;
    }
    if let Some(v) = b(doc, "spp_arms_runner_clock") {
        c.spp_arms_runner_clock = v;
    }
    if let Some(v) = b(doc, "tp_hit_match_level") {
        c.tp_hit_match_level = v;
    }
    if let Some(v) = b(doc, "tp_unindexed_pips_require_price") {
        c.tp_unindexed_pips_require_price = v;
    }
    if let Some(v) = b(doc, "tp_price_only_strict") {
        c.tp_price_only_strict = v;
    }
    if let Some(v) = f(doc, "rf_level_sanity_max_usd") {
        c.rf_level_sanity_max_usd = v.max(0.0);
    }
    if let Some(v) = b(doc, "reply_graph_transitive") {
        c.reply_graph_transitive = v;
    }
    if let Some(v) = u(doc, "streak_pause_n") {
        c.streak_pause_n = v;
    }
    if let Some(v) = f(doc, "streak_pause_min") {
        c.streak_pause_min = v;
    }
    // HAMULEC SL-HIT — cała rodzina naraz. Dwa starsze pola (`slhit_pause_n`,
    // `slhit_pause_min`) NIE MIAŁY tu mostu od dnia powstania, więc panel ich
    // nie ustawiał i nie kasował: zmiana czegokolwiek innego cofała je do
    // domyślnego zera, czyli WYŁĄCZAŁA hamulec presetu produkcyjnego bez
    // słowa w dzienniku. To ta sama klasa błędu co `drop_unplaceable_levels`.
    if let Some(v) = u(doc, "slhit_pause_n") {
        c.slhit_pause_n = v;
    }
    if let Some(v) = f(doc, "slhit_pause_min") {
        c.slhit_pause_min = v;
    }
    // Pakiet F2b: 0 = hamulec twardy (blokada), > 0 = mnożnik lota.
    if let Some(v) = f(doc, "slhit_pause_lot_mult") {
        c.slhit_pause_lot_mult = v;
    }
    if let Some(v) = b(doc, "signal_filter") {
        c.signal_filter = v;
    }
    if let Some(v) = s(doc, "skip_tags") {
        c.skip_tags = v;
    }
    if let Some(v) = s(doc, "require_tags") {
        c.require_tags = v;
    }
    if let Some(v) = s(doc, "side_filter") {
        // obie konwencje (patrz `regime_filter`); tu wada jeszcze nie ugryzła,
        // bo żaden preset nie zawęża kierunku — ale ugryzłaby pierwszy, który
        // to zrobi, i to bez śladu w dzienniku
        c.side_filter = match v.as_str() {
            "buy" | "BuyOnly" => SideFilter::BuyOnly,
            "sell" | "SellOnly" => SideFilter::SellOnly,
            _ => SideFilter::Both,
        };
    }
    if let Some(v) = s(doc, "regime_filter") {
        // obie konwencje: panel (`"counter"`) i preset z pliku (`"CounterMa"`);
        // bez nazw rdzenia filtr reżimu po cichu WYŁĄCZAŁ się przy wczytaniu
        // presetu — zmierzone −490 $ na oknie czerwiec–lipiec
        c.regime_filter = match v.as_str() {
            "trend" | "TrendMa" => RegimeFilter::TrendMa,
            "counter" | "CounterMa" => RegimeFilter::CounterMa,
            _ => RegimeFilter::Off,
        };
    }
    if let Some(v) = f(doc, "regime_ma_hours") {
        c.regime_ma_hours = v;
    }
    if let Some(v) = u(doc, "max_open_baskets") {
        c.max_open_baskets = v;
    }
    if let Some(v) = f(doc, "exposure_bonus_profit_pct") {
        c.exposure_bonus_profit_pct = v;
    }
    if let Some(v) = u(doc, "exposure_bonus_positions") {
        c.exposure_bonus_positions = v;
    }
    if let Some(v) = u(doc, "exposure_bonus_baskets") {
        c.exposure_bonus_baskets = v;
    }
    if let Some(v) = f(doc, "max_directional_lots") {
        c.max_directional_lots = v;
    }
    if let Some(v) = f(doc, "equity_floor_pct") {
        c.equity_floor_pct = v;
    }
    if let Some(v) = f(doc, "fast_fill_reject_s") {
        c.fast_fill_reject_s = v;
    }
    if let Some(v) = u(doc, "fast_fill_layers") {
        c.fast_fill_layers = v;
    }
    if let Some(v) = f(doc, "fast_fill_soft_age_min") {
        c.fast_fill_soft_age_min = v;
    }
    if let Some(v) = f(doc, "zone_exit_adverse_s") {
        c.zone_exit_adverse_s = v;
    }
    if let Some(v) = b(doc, "zone_exit_adverse_close") {
        c.zone_exit_adverse_close = v;
    }
    if let Some(v) = f(doc, "reenter_min_return_s") {
        c.reenter_min_return_s = v;
    }
    if let Some(v) = u(doc, "pyramid_after_stage") {
        c.pyramid_after_stage = v;
    }
    if let Some(v) = f(doc, "pyramid_lot_mult") {
        c.pyramid_lot_mult = v;
    }
    if let Some(v) = u(doc, "pyramid_regime_lookback") {
        c.pyramid_regime_lookback = v;
    }
    if let Some(v) = f(doc, "pyramid_regime_max_fast_pct") {
        c.pyramid_regime_max_fast_pct = v;
    }
    if let Some(v) = f(doc, "fast_addon_move_usd") {
        c.fast_addon_move_usd = v;
    }
    if let Some(v) = f(doc, "fast_addon_window_s") {
        c.fast_addon_window_s = v;
    }
    if let Some(v) = u(doc, "fast_addon_max") {
        c.fast_addon_max = v;
    }
    if let Some(v) = f(doc, "fast_addon_lot_mult") {
        c.fast_addon_lot_mult = v;
    }
    if let Some(v) = u(doc, "fast_addon_min_stage") {
        c.fast_addon_min_stage = v;
    }
    if let Some(v) = f(doc, "fast_addon_cooldown_s") {
        c.fast_addon_cooldown_s = v;
    }
    if let Some(v) = f(doc, "pyramid_min_equity_mult") {
        c.pyramid_min_equity_mult = v;
    }

    // ---------- bramki kapitałowe (rodzina `*_small`) ----------
    if let Some(v) = u(doc, "entry_units_small") {
        c.entry_units_small = v;
    }
    if let Some(v) = f(doc, "entry_units_small_mult") {
        c.entry_units_small_mult = v;
    }
    if let Some(v) = f(doc, "risk_per_basket_pct_small") {
        c.risk_per_basket_pct_small = v;
    }
    if let Some(v) = f(doc, "risk_per_basket_pct_small_mult") {
        c.risk_per_basket_pct_small_mult = v;
    }
    if let Some(v) = u(doc, "reenter_max_small") {
        c.reenter_max_small = v;
    }
    if let Some(v) = f(doc, "reenter_max_small_mult") {
        c.reenter_max_small_mult = v;
    }
    if let Some(v) = u(doc, "max_open_positions_small") {
        c.max_open_positions_small = v;
    }
    if let Some(v) = f(doc, "max_open_positions_small_mult") {
        c.max_open_positions_small_mult = v;
    }
    if let Some(v) = u(doc, "max_open_baskets_small") {
        c.max_open_baskets_small = v;
    }
    if let Some(v) = f(doc, "max_open_baskets_small_mult") {
        c.max_open_baskets_small_mult = v;
    }
    if let Some(v) = f(doc, "basket_max_age_min_small") {
        c.basket_max_age_min_small = v;
    }
    if let Some(v) = f(doc, "basket_max_age_min_small_mult") {
        c.basket_max_age_min_small_mult = v;
    }
    if let Some(v) = f(doc, "fast_fill_soft_age_min_small") {
        c.fast_fill_soft_age_min_small = v;
    }
    if let Some(v) = f(doc, "fast_fill_soft_age_min_small_mult") {
        c.fast_fill_soft_age_min_small_mult = v;
    }
    if let Some(v) = f(doc, "market_entry_step_small") {
        c.market_entry_step_small = v;
    }
    if let Some(v) = f(doc, "market_entry_step_small_mult") {
        c.market_entry_step_small_mult = v;
    }
    if let Some(v) = f(doc, "sl_min_dist_small") {
        c.sl_min_dist_small = v;
    }
    if let Some(v) = f(doc, "sl_min_dist_small_mult") {
        c.sl_min_dist_small_mult = v;
    }
    if let Some(v) = f(doc, "lot_percent_small") {
        c.lot_percent_small = v;
    }
    if let Some(v) = f(doc, "lot_percent_small_mult") {
        c.lot_percent_small_mult = v;
    }

    // ---------- pułap ekspozycji / własny stop-out ----------
    // Sześć pól dołożonych 04.08.2026 do `conduit_core::Settings`. Bez
    // mapowania TU nie ginęły po cichu przy starcie, tylko przy PIERWSZEJ
    // edycji dowolnego pola presetu z panelu — `core_from_ui` odbudowuje
    // `Settings` od domyślnych, więc wszystko niewymienione wracało do
    // wartości domyślnej. Test `zaden_klucz_presetu_nie_ginie_po_tlumaczeniu`
    // pilnuje, żeby następne pole nie powtórzyło tej drogi.
    if let Some(v) = f(doc, "expo_cap_pct") {
        c.expo_cap_pct = v;
    }
    if let Some(v) = b(doc, "expo_cap_close") {
        c.expo_cap_close = v;
    }
    if let Some(v) = f(doc, "expo_cap_s") {
        c.expo_cap_s = v;
    }
    if let Some(v) = f(doc, "expo_cap_ml_pct") {
        c.expo_cap_ml_pct = v;
    }
    if let Some(v) = s(doc, "lot_base") {
        // Panel przysyła nazwę wariantu; nieznana nazwa NIE ma prawa cicho
        // przestawić podstawy lota — zostaje ta, którą preset już ma.
        c.lot_base = match v.as_str() {
            "Equity" | "equity" => conduit_core::settings::PodstawaLota::Equity,
            "MinOfBoth" | "min_of_both" => conduit_core::settings::PodstawaLota::MinOfBoth,
            "Balance" | "balance" => conduit_core::settings::PodstawaLota::Balance,
            _ => c.lot_base,
        };
    }
    if let Some(v) = b(doc, "sim_margin_at_market") {
        c.sim_margin_at_market = v;
    }

    // ---------- ochrona kapitału ----------
    if let Some(v) = f(doc, "max_dd_pct") {
        c.max_dd_pct = v;
    }
    if let Some(v) = f(doc, "max_dd_usd") {
        c.max_dd_usd = v;
    }
    // --- hamulec miękki (dławik) ---
    if let Some(v) = f(doc, "max_portfolio_risk_pct") {
        c.max_portfolio_risk_pct = v;
    }
    if let Some(v) = f(doc, "dd_soft_pct") {
        c.dd_soft_pct = v;
    }
    if let Some(v) = f(doc, "dd_soft_mult") {
        c.dd_soft_mult = v;
    }
    if let Some(v) = f(doc, "dd_hard_pct") {
        c.dd_hard_pct = v;
    }
    if let Some(v) = f(doc, "dd_hard_mult") {
        c.dd_hard_mult = v;
    }
    if let Some(v) = f(doc, "day_target_usd") {
        c.day_target_usd = v;
    }
    if let Some(v) = b(doc, "day_target_close") {
        c.day_target_close = v;
    }
    if let Some(v) = b(doc, "day_target_scale_lot") {
        c.day_target_scale_lot = v;
    }
    if let Some(v) = f(doc, "day_trail_stop_usd") {
        c.day_trail_stop_usd = v;
    }
    if let Some(v) = b(doc, "usd_scale_with_lot") {
        c.usd_scale_with_lot = v;
    }
    if let Some(v) = f(doc, "eod_flat_hour") {
        c.eod_flat_hour = v;
    }
    if let Some(v) = b(doc, "flat_weekend") {
        c.flat_weekend = v;
    }
    if let Some(v) = f(doc, "flat_weekend_hour") {
        c.flat_weekend_hour = v;
    }

    if let Some(v) = s(doc, "dd_guard_scope") {
        // obie konwencje (patrz `regime_filter`)
        c.dd_guard_scope = match v.as_str() {
            "lifetime" | "Lifetime" => DdGuardScope::Lifetime,
            "lifetime_daily_reset" | "LifetimePeakDailyReset" => {
                DdGuardScope::LifetimePeakDailyReset
            }
            _ => DdGuardScope::Daily,
        };
    }

    // ---------- most dla osi Fazy 6 (37 pól, AUDYT_SILNIKA.md TOP 1) ----------
    //
    // Te pola żyły w silniku i backteście, ale NIE MIAŁY drogi panel→silnik:
    // `core_from_ui` odbudowuje `Settings` od domyślnych, więc KAŻDA edycja
    // presetu z panelu (`rest.rs::patch_preset_settings`) trwale je zerowała —
    // ta sama klasa błędu co `drop_unplaceable_levels`, tylko ×37 naraz
    // (m.in. sesja_bramka z pomiaru SESJAFILL 9615 $, cały reżim v2, vol_size_*).
    //
    // Nazwy kluczy są wspólne z rdzeniem. Enumy przyjmują słownik serde ORAZ
    // małe litery — panel nie ma dla nich kontrolek, więc wartość w dokumencie
    // pochodzi z presetu, ale plik bywa pisany ręcznie. Nierozpoznana nazwa
    // wariantu NIE MOŻE po cichu przestawić bramki handlowej — zostaje wartość,
    // którą konfiguracja już ma (wzorzec `lot_base`).

    // --- bramka sesji: od której godziny liczona (sygnał / wypełnienie) ---
    if let Some(v) = s(doc, "sesja_bramka") {
        c.sesja_bramka = match v.as_str() {
            "Sygnal" | "sygnal" => SesjaBramka::Sygnal,
            "Wypelnienie" | "wypelnienie" => SesjaBramka::Wypelnienie,
            "Oba" | "oba" => SesjaBramka::Oba,
            _ => c.sesja_bramka,
        };
    }
    // --- filtr reżimu v2: która cena, jaka miara progu ---
    if let Some(v) = s(doc, "regime_cena") {
        c.regime_cena = match v.as_str() {
            "Rynkowa" | "rynkowa" => RegimeCena::Rynkowa,
            "Wejscia" | "wejscia" => RegimeCena::Wejscia,
            "Obie" | "obie" => RegimeCena::Obie,
            _ => c.regime_cena,
        };
    }
    if let Some(v) = s(doc, "regime_miara") {
        c.regime_miara = match v.as_str() {
            "Srednia" | "srednia" => RegimeMiara::Srednia,
            "Mediana" | "mediana" => RegimeMiara::Mediana,
            "Kanal" | "kanal" => RegimeMiara::Kanal,
            "Wykladnicza" | "wykladnicza" => RegimeMiara::Wykladnicza,
            "Percentyl" | "percentyl" => RegimeMiara::Percentyl,
            _ => c.regime_miara,
        };
    }
    if let Some(v) = b(doc, "regime_pilnuj_limitow") {
        c.regime_pilnuj_limitow = v;
    }
    if let Some(v) = f(doc, "regime_percentyl") {
        c.regime_percentyl = v;
    }
    if let Some(v) = f(doc, "regime_strefa_martwa") {
        c.regime_strefa_martwa = v;
    }
    if let Some(v) = f(doc, "regime_okno2_h") {
        c.regime_okno2_h = v;
    }
    if let Some(v) = f(doc, "regime_zmiennosc_min") {
        c.regime_zmiennosc_min = v;
    }
    // Stara nazwa (serde alias `regime_range_mute_usd`) czytana jako zapasowa:
    // niosą ją 4 presety z dysku (OMEGA-2/X1/X2, MONOLIT-SYN — po 150,0), więc
    // bez niej droga panelu zerowała próg, który btp czyta przez serde.
    if let Some(v) = f(doc, "regime_zmiennosc_max").or_else(|| f(doc, "regime_range_mute_usd")) {
        c.regime_zmiennosc_max = v;
    }
    // Rdzeń zna też starą nazwę pola (serde alias `regime_range_mute_mode`) —
    // preset sprzed zmiany nazwy musi przez panel przejść tak samo jak przez btp.
    if let Some(v) = s(doc, "regime_gdy_rozerwany").or_else(|| s(doc, "regime_range_mute_mode")) {
        c.regime_gdy_rozerwany = match v.as_str() {
            "Milcz" | "milcz" | "Pass" => RegimeGdyRozerwany::Milcz,
            "KrotkieOkno" | "krotkie_okno" => RegimeGdyRozerwany::KrotkieOkno,
            "Miekko" | "miekko" | "Soft" => RegimeGdyRozerwany::Miekko,
            _ => c.regime_gdy_rozerwany,
        };
    }
    // --- tryb miękki reżimu: wejdź mniejszą stawką zamiast milczeć ---
    if let Some(v) = b(doc, "regime_soft") {
        c.regime_soft = v;
    }
    if let Some(v) = f(doc, "regime_soft_units_mult") {
        c.regime_soft_units_mult = v;
    }
    if let Some(v) = f(doc, "regime_soft_lot_mult") {
        c.regime_soft_lot_mult = v;
    }
    if let Some(v) = u(doc, "regime_soft_max_positions") {
        c.regime_soft_max_positions = v;
    }
    if let Some(v) = f(doc, "regime_soft_risk_mult") {
        c.regime_soft_risk_mult = v;
    }
    // --- rozmiar sterowany zmiennością (mnożnik lota, nie bramka) ---
    if let Some(v) = s(doc, "vol_size_mode") {
        c.vol_size_mode = match v.as_str() {
            "Off" | "off" => VolSizeMode::Off,
            "Target" | "target" => VolSizeMode::Target,
            "Percentile" | "percentile" => VolSizeMode::Percentile,
            _ => c.vol_size_mode,
        };
    }
    if let Some(v) = f(doc, "vol_size_target") {
        c.vol_size_target = v;
    }
    if let Some(v) = f(doc, "vol_size_min_mult") {
        c.vol_size_min_mult = v;
    }
    if let Some(v) = f(doc, "vol_size_max_mult") {
        c.vol_size_max_mult = v;
    }
    if let Some(v) = u(doc, "vol_size_percentile_okno") {
        c.vol_size_percentile_okno = v;
    }
    if let Some(v) = b(doc, "vol_size_odsezonuj") {
        c.vol_size_odsezonuj = v;
    }
    // --- sanity sygnału: odrzuć geometrię, która nie może być prawdziwa ---
    if let Some(v) = f(doc, "sanity_zone_max") {
        c.sanity_zone_max = v;
    }
    if let Some(v) = f(doc, "sanity_tp_max") {
        c.sanity_tp_max = v;
    }
    if let Some(v) = b(doc, "sanity_tp_rosnace") {
        c.sanity_tp_rosnace = v;
    }
    if let Some(v) = b(doc, "sanity_tp_strona") {
        c.sanity_tp_strona = v;
    }
    // --- parser geometryczny ---
    if let Some(v) = b(doc, "parser_geometryczny") {
        c.parser_geometryczny = v;
    }
    if let Some(v) = f(doc, "parser_min_pewnosc") {
        c.parser_min_pewnosc = v;
    }
    // --- cel z przeciwnej strefy ---
    if let Some(v) = s(doc, "cel_z_przeciwnego") {
        c.cel_z_przeciwnego = match v.as_str() {
            "Off" | "off" => CelZPrzeciwnego::Off,
            // serde niesie ogonek („BliższaKrawedz"); wariant bez ogonka też
            // przyjmujemy, bo plik pisany ręcznie łatwo go gubi
            "BliższaKrawedz" | "BlizszaKrawedz" => CelZPrzeciwnego::BliższaKrawedz,
            "DalszaKrawedz" => CelZPrzeciwnego::DalszaKrawedz,
            "Srodek" | "srodek" => CelZPrzeciwnego::Srodek,
            _ => c.cel_z_przeciwnego,
        };
    }
    if let Some(v) = f(doc, "cel_z_przeciwnego_zapas") {
        c.cel_z_przeciwnego_zapas = v;
    }
    // --- bramka dnia zależna od salda (0 = zawsze aktywna) ---
    if let Some(v) = f(doc, "day_gate_od_salda") {
        c.day_gate_od_salda = v;
    }
    if let Some(v) = f(doc, "day_gate_do_salda") {
        c.day_gate_do_salda = v;
    }
    // --- zachowanie po etapie celu (0 = wyłączone) ---
    if let Some(v) = u(doc, "no_tp_after_stage") {
        c.no_tp_after_stage = v.min(u8::MAX as u32) as u8;
    }
    if let Some(v) = u(doc, "no_reenter_from_stage") {
        c.no_reenter_from_stage = v.min(u8::MAX as u32) as u8;
    }
    // --- risk-free wyłącza późniejsze reakcje ---
    if let Some(v) = b(doc, "oae_skip_after_riskfree") {
        c.oae_skip_after_riskfree = v;
    }
    if let Some(v) = b(doc, "reenter_stop_after_riskfree") {
        c.reenter_stop_after_riskfree = v;
    }
    // --- sufit lota z salda (dzielnik, 0 = bez sufitu) + zapadka ATR ---
    if let Some(v) = f(doc, "lot_max_z_salda") {
        c.lot_max_z_salda = v;
    }
    if let Some(v) = f(doc, "trail_atr_mult") {
        c.trail_atr_mult = v;
    }
    if let Some(v) = b(doc, "trail_adaptive_enabled") {
        c.trail_adaptive_enabled = v;
    }
    if let Some(v) = b(doc, "trail_adaptive_runners_only") {
        c.trail_adaptive_runners_only = v;
    }
    if let Some(v) = f(doc, "trail_adaptive_window_s") {
        c.trail_adaptive_window_s = v;
    }
    if let Some(v) = u(doc, "trail_adaptive_min_samples") {
        c.trail_adaptive_min_samples = v;
    }
    if let Some(v) = f(doc, "trail_adaptive_trend_er") {
        c.trail_adaptive_trend_er = v;
    }
    if let Some(v) = f(doc, "trail_adaptive_reversal_er") {
        c.trail_adaptive_reversal_er = v;
    }
    if let Some(v) = f(doc, "trail_adaptive_trend_gap_mult") {
        c.trail_adaptive_trend_gap_mult = v;
    }
    if let Some(v) = f(doc, "trail_adaptive_chop_gap_mult") {
        c.trail_adaptive_chop_gap_mult = v;
    }
    if let Some(v) = f(doc, "trail_adaptive_reversal_gap_mult") {
        c.trail_adaptive_reversal_gap_mult = v;
    }
    if let Some(v) = f(doc, "trail_adaptive_fast_vol_s") {
        c.trail_adaptive_fast_vol_s = v;
    }
    if let Some(v) = f(doc, "trail_adaptive_slow_vol_s") {
        c.trail_adaptive_slow_vol_s = v;
    }
    if let Some(v) = f(doc, "trail_adaptive_vol_ratio") {
        c.trail_adaptive_vol_ratio = v;
    }
    if let Some(v) = f(doc, "trail_adaptive_vol_favorable_mult") {
        c.trail_adaptive_vol_favorable_mult = v;
    }
    if let Some(v) = f(doc, "trail_adaptive_vol_adverse_mult") {
        c.trail_adaptive_vol_adverse_mult = v;
    }
    if let Some(v) = f(doc, "trail_adaptive_min_peak") {
        c.trail_adaptive_min_peak = v;
    }
    if let Some(v) = f(doc, "trail_adaptive_min_gap") {
        c.trail_adaptive_min_gap = v;
    }
    if let Some(v) = f(doc, "trail_adaptive_max_gap") {
        c.trail_adaptive_max_gap = v;
    }

    // ---------- broker ----------
    if let Some(v) = f(doc, "sim_stops_level") {
        c.stops_level = v;
    }
    if let Some(v) = f(doc, "commission_per_lot") {
        c.commission_per_lot = v;
    }
    if let Some(v) = f(doc, "exec_latency_ms") {
        c.exec_latency_ms = v as i64;
    }
    if let Some(v) = f(doc, "slippage_pts") {
        c.slippage_pts = v;
    }
    if let Some(v) = f(doc, "server_tz_offset_h") {
        c.server_tz_offset_ms = (v * 3_600_000.0) as i64;
    }
    // `null`/brak = weź offset serwera; wartość jawna przydaje się, gdy źródło
    // wiadomości ma własną strefę czasową
    match doc.get("msg_clock_offset_h") {
        Some(Value::Null) | None => {}
        Some(x) => c.msg_clock_offset_ms = x.as_f64().map(|v| (v * 3_600_000.0) as i64),
    }

    // ---------- AI ----------
    if let Some(v) = b(doc, "ai_mode") {
        c.ai_enabled = v;
    }
    if let Some(v) = s(doc, "ai_model") {
        c.ai_model = v;
    }
    if let Some(v) = f(doc, "ai_decision_interval_s") {
        c.ai_decision_interval_s = v;
    }

    // ---------- nadzór nad terminalem MT5 ----------
    if let Some(v) = b(doc, "mt5_autostart") {
        c.mt5_autostart = v;
    }
    if let Some(v) = b(doc, "mt5_watchdog") {
        c.mt5_watchdog = v;
    }
    if let Some(v) = s(doc, "mt5_terminal_path") {
        c.mt5_terminal_path = v;
    }
    if let Some(v) = u(doc, "mt5_retry_attempts") {
        // Zero prób znaczyłoby „poddaj się od razu" — a bot z otwartymi
        // pozycjami nie ma prawa przestać próbować.
        c.mt5_retry_attempts = v.max(1);
    }
    if let Some(v) = f(doc, "mt5_retry_delay_s") {
        c.mt5_retry_delay_s = v.max(0.5);
    }
    if let Some(v) = u(doc, "mt5_restart_after") {
        c.mt5_restart_after = v;
    }
    if let Some(v) = f(doc, "mt5_health_interval_s") {
        c.mt5_health_interval_s = v.max(1.0);
    }

    // ---------- dziennik zdarzeń ----------
    if let Some(v) = b(doc, "journal_enabled") {
        c.journal_enabled = v;
    }
    if let Some(v) = s(doc, "journal_min_level") {
        // Nierozpoznana nazwa NIE wycisza dziennika po cichu — zostaje
        // wartość domyślna, czyli `info`.
        if let Some(l) = conduit_core::journal::EventLevel::parse(&v) {
            c.journal_min_level = l;
        }
    }
    if let Some(v) = b(doc, "journal_snapshots") {
        c.journal_snapshots = v;
    }
    if let Some(v) = b(doc, "journal_excursions") {
        c.journal_excursions = v;
    }
    if let Some(v) = b(doc, "journal_text_mirror") {
        c.journal_text_mirror = v;
    }
    if let Some(v) = u(doc, "journal_retention_days") {
        c.journal_retention_days = v;
    }
    if let Some(v) = u(doc, "journal_buffer_cap") {
        // Sufit poniżej 64 znaczyłby, że bufor gubi zdarzenia szybciej, niż
        // pętla zdąży je zabrać — a cicha utrata linii dziennika jest
        // dokładnie tym, czego ten mechanizm ma nie robić.
        c.journal_buffer_cap = v.max(64);
    }

    c
}

/// Klucze obecne w dokumencie UI, których mapowanie nie dotyka.
/// Do pokazania w panelu diagnostycznym — zamiast cichego pomijania.
///
/// Liczy to samo, co czyta [`core_from_ui`], więc dokument w nazewnictwie
/// SILNIKA jest najpierw tłumaczony. Bez tego audyt zgłaszał 32 klucze jako
/// „niepodpięte", choć po tłumaczeniu wszystkie docierają do silnika —
/// czyli mylił brak MAPOWANIA z brakiem TŁUMACZENIA i wskazywał na naprawę
/// tam, gdzie nic nie było zepsute.
pub fn unmapped_keys(doc: &Value) -> Vec<String> {
    if PRZETLUMACZONE.iter().any(|k| doc.get(*k).is_some()) {
        return unmapped_keys(&preset_to_ui(doc));
    }
    const MAPPED: &[&str] = &[
        "lot_scale_step",
        "entry_offset_dir",
        "custom_entry",
        "entry_high_offset",
        "entry_low_offset",
        "entry_deep_offset",
        "entry_tol_offset",
        "only_limit_signals",
        "auto_limit",
        "ignore_old_after_min",
        "entry_units",
        "entry_units_limit",
        "ppm",
        "ppm_enabled",
        "entry_risk_budget",
        "entry_tp1_budget",
        "entry_touch_units",
        "entry_touch_tp",
        "entry_touch_levels",
        "pending_ttl_h",
        "explicit_pending_until_cancel",
        "pending_never_cancel",
        "valid_till_tp2",
        "sl_min_dist",
        "sl_dist_limit",
        "sl_dist_max",
        "virtual_sl",
        "virtual_sl_only_when_rejected",
        "vsl_eval_s",
        "vsl_net_off",
        "all_runners",
        "official_mode",
        "official_use_counts",
        "scale_out",
        "scale_out_pct",
        "official_pct_tp1",
        "official_pct_tp2",
        "official_pct_tp3",
        "official_pct_spp",
        "official_counts",
        "official_spp",
        "official_assign_tps",
        "tp_open_offset",
        "tp_freeze_after_ladder",
        "tp_source",
        "tp_price_tolerance",
        "tp_price_front_run_usd",
        "tp_signal_max_lead_s",
        "tp_signal_max_lag_s",
        "tp_stage_from_broker_fill",
        "tp_hit_fill_stages",
        "ignore_risk_free",
        "risk_free_mode",
        "risk_free_runners",
        "ignore_out_at_entry",
        "sl_hit_verify_tol",
        "oae_timeout_min",
        "oae_profit_min",
        "be_lock",
        "be_lock_points",
        "be_at_tp1",
        "smart_sl",
        "runner_trail",
        "trail_mode",
        "runner_trail_start",
        "runner_trail_gap",
        "trail_lock_pct",
        "trail_tiers",
        "trail_split",
        "trail_runners_n",
        "trail_runner_mode",
        "trail_runner_start",
        "trail_runner_gap",
        "trail_runner_lock_pct",
        // --- trailing S/R po strukturze 1M (OS_SR_SPEC.md, 24.08.2026) ---
        "trail_sr_enabled",
        "trail_sr_scope",
        "trail_sr_activation",
        "trail_sr_min_gain",
        "trail_sr_min_dist_price",
        // --- EA-CORE: szkielet warstwy EA (FALA 0, 24.08.2026) ---
        // Pominięcie klucza tutaj = panel ZERUJE pole przy zapisie presetu
        // (incydent ×3 w historii) — dlatego wpis stoi obok mapowania.
        "ea_enabled",
        "ea_tick_s",
        "ea_state_src",
        "ea_defense_enter",
        "ea_defense_exit",
        "ea_offense_enter",
        "ea_offense_exit",
        "ea_state_dwell_s",
        "ea_state_ratchet",
        "ea_state_journal",
        "ea_dozor_sl",
        // --- RODZINA A: ekspozycja wobec stanu rachunku (FALA 1) ---
        // Dopisane 25.08.2026 razem z mapowaniem. Do tej chwili osie A1–A4
        // działały w backteście i ginęły na żywo — patrz komentarz przy nich
        // w `core_from_ui`.
        "ea_lot_z_wolnego_marginesu",
        "ea_stop_dokladek_przy_stracie",
        "ea_stop_dokladek_powrot",
        "ea_redukcja_przy_zageszczeniu",
        "ea_zageszczenie_podloga",
        "ea_stan_dnia",
        "ea_stan_dnia_prog_sl",
        "ea_stan_dnia_jednostki_mult",
        "trail_runner_tiers",
        "trail_min_dist",
        "ladder_from_tp",
        "ladder_lag",
        "ladder_offset",
        "harvest",
        "harvest_retrace_pct",
        "harvest_start",
        "stale_take_min",
        "stale_take_profit",
        "stale_take_min2",
        "stale_take_profit2",
        "session_filter",
        "session_hours",
        "max_open_positions",
        "exposure_count_pendings",
        "enforce_position_limit_on_fill",
        "limit_kasuje_tylko_nadmiar",
        "sim_margin_check_on_fill",
        "sim_validate_pending_stops",
        "reenter_respect_cap",
        // naprawy z audytu Fable (Z-3, Z-5, Z-7…Z-10)
        "sl_edit_reaches_pendings",
        "honor_stop_orders",
        "hint_veto",
        // --- Pakiet F1: weto odpowiedzi ---
        "reply_veto",
        "sync_only_live_levels",
        "tp_correction_to_broker",
        "runner_max_hold_rule_only",
        "spp_arms_runner_clock",
        "tp_hit_match_level",
        "tp_unindexed_pips_require_price",
        "tp_price_only_strict",
        "rf_level_sanity_max_usd",
        "reply_graph_transitive",
        // --- sekcja M z 18.08: wyjście runnera, próg BE, głębokość siatki ---
        "runner_max_hold_bez_reguly",
        "risk_free_be_min_profit",
        "entry_deep_frac_to_sl",
        "streak_pause_n",
        "streak_pause_min",
        // --- hamulec SL-HIT (Pakiet F2) ---
        "slhit_pause_n",
        "slhit_pause_min",
        "slhit_pause_lot_mult",
        "signal_filter",
        "skip_tags",
        "require_tags",
        "max_dd_pct",
        "max_dd_usd",
        // --- pułap ekspozycji / własny stop-out po poziomie marginesu ---
        "expo_cap_pct",
        "expo_cap_close",
        "expo_cap_s",
        "expo_cap_ml_pct",
        "lot_base",
        "sim_margin_at_market",
        // --- hamulec miękki: sufit portfelowy i dławik obsunięcia ---
        "max_portfolio_risk_pct",
        "dd_soft_pct",
        "dd_soft_mult",
        "dd_hard_pct",
        "dd_hard_mult",
        "day_target_usd",
        "day_target_close",
        "day_target_scale_lot",
        "day_trail_stop_usd",
        "usd_scale_with_lot",
        "eod_flat_hour",
        "flat_weekend",
        "flat_weekend_hour",
        "sim_stops_level",
        "ai_mode",
        "ai_model",
        "mt5_autostart",
        "mt5_watchdog",
        "mt5_terminal_path",
        "mt5_retry_attempts",
        "mt5_retry_delay_s",
        "mt5_restart_after",
        "mt5_health_interval_s",
        // --- podłączenie mostu MT5 (czyta je `live.rs` w conduit-app) ---
        // Nie przechodzą przez `core_from_ui`, bo nie są ustawieniami SILNIKA,
        // tylko parametrami połączenia z terminalem. Są tu, żeby panel nie
        // zgłaszał ich jako „nieobsługiwane".
        "mt5_symbol",
        "mt5_magic",
        "mt5_python",
        "mt5_deviation_points",
        "mt5_login",
        "mt5_server",
        "mt5_follow_terminal_account",
        "mt5_allow_real_account",
        // Hasło rachunku MT5 — SKRZYNKA PODAWCZA: `apply_settings_patch`
        // przenosi wartość do secrets.json i USUWA klucz z dokumentu, więc
        // w zapisanych ustawieniach nigdy go nie ma. Wpis tutaj jest po to,
        // żeby audyt nie zgłaszał pola panelu jako „niepodpięte".
        "mt5_password",
        // --- łączność i raport życia (czyta je live.rs / main.rs, nie rdzeń) ---
        // Przerwa DOBOWA notowań w godzinach czasu SERWERA (złoto: 00:00–01:00
        // + zapas). `od == do` = wyłączona. Parametr ŁĄCZNOŚCI: mówi
        // watchdogowi ciszy, kiedy brak ticków jest snem rynku, a nie awarią.
        "przerwa_dobowa_od_h",
        "przerwa_dobowa_do_h",
        // Puls: mail „żyję" co N godzin (0 = wyłączony). Cisza bez pulsu
        // znaczy śmierć bota — a nie spokojną noc.
        "puls_h",
        // --- dołożone przy odtwarzaniu funkcji bot.py ---
        "entry_weights",
        "risk_per_basket_pct",
        // CAŁA rodzina lota jedzie normalną drogą od 04.08.2026 — patrz
        // komentarz przy „wielkość pozycji" w `core_from_ui`.
        "lot_mode_percent",
        "lot_fixed",
        "lot_percent",
        "lot_min",
        "lot_max",
        "skip_if_sl_breached",
        "max_chase_beyond_zone",
        "sl_max_dist",
        "ppm_for_limits",
        "ppm_immediate",
        "market_entry_step",
        "market_entry_mode",
        "pending_ttl_from_basket",
        "pending_drop_on_target",
        "smart_sl_floor_be_after_rf",
        "vol_window_min",
        "vol_range_usd",
        "vol_units_mult",
        "pending_resize_on_vol",
        "pending_resize_sec",
        "virtual_sl_all",
        // PENDING-RELOT — bez tych dwóch wpisów wczytanie HYPER-X1 z panelu
        // po cichu WYŁĄCZAŁO dokładkę: pola istnieją w pliku presetu i są
        // czytane przez `core_from_ui`, ale `preset_to_ui` ich nie przepuszczał,
        // więc panel dostawał preset bez nich i zapisywał z powrotem `false`.
        // Wyłapane przez `zaden_klucz_presetu_nie_ginie_po_tlumaczeniu` 03.08.2026.
        "pending_relot_on_balance",
        "pending_relot_topup",
        "pending_relot_up",
        "pending_relot_down",
        "pending_relot_wg_planu",
        "pending_relot_reconcile_target",
        "pending_relot_up_od_salda",
        "scale_out_round",
        "official_round",
        "scale_out_from",
        "scale_out_last_runner",
        "official_close_last",
        "partial_close",
        "partial_min_lot",
        "partial_pct_od_pierwotnego",
        "cele_na_ostatnim",
        "sl_polowa_od_konca",
        "sl_polowa_ulamek",
        "spp_max_age_h",
        "spp_keep_tp",
        "spp_sl_mode",
        "spp_sl_pad",
        "risk_free_runner_target",
        "risk_free_trail",
        "out_at_entry_mode",
        "oae_band_pts",
        "sl_hit_mode",
        "honor_cancel",
        "honor_close_all",
        "honor_market_open",
        // --- odzysk STORM (W30/W31/W33, 24.08.2026) ---
        "close_all_scope",
        "partials_wykonuj",
        "partials_pct",
        "parser_luz_interpunkcyjny",
        "recap_guard",
        "profit_update_telemetry_only",
        // --- pełny obieg preset→panel→live (audyt 30.08.2026) ---
        "be_min_pozycji",
        "be_od_etapu",
        "cele_pomin_za_cena",
        "entry_jeden_na_glebokiej",
        "sl_po_tp1_na_krawedz",
        "sl_wlasny_na_pozycje",
        "entry_uklad",
        "entry_uklad_kotwica",
        "entry_krzywa_kotwica",
        "tp_drabinka_kotwica",
        "entry_warstwy_offset",
        "entry_warstwy_z_tekstu",
        "runner_cele_n",
        "runner_cele_krok",
        "runner_partial_pct",
        "trail_sr_tf_min",
        "trail_sr_fractal_n",
        "trail_sr_offset",
        "trail_sr_min_dist_tp",
        "trail_sr_struct_window_h",
        "trail_sr_min_prominence_atr",
        "trail_sr_offset_atr_mult",
        "trail_sr_offset_spread_mult",
        "trail_sr_atr_period",
        "oae_pod_woda",
        "rearm_bez_pozycji",
        "rearm_bez_pozycji_max_h",
        "be_covers_late_fills",
        "entry_allowance_usd",
        "entry_allowance_units",
        "dedup_edited_signals",
        "price_tol",
        "risk_free_smart_sl",
        // --- Pakiet A: osie dedupu i edycji ---
        // Pominięcie klucza na tej liście = panel zeruje pole (incydent ×3
        // w historii) — dlatego wpis stoi tuż przy mapowaniu w `core_from_ui`.
        "dedup_pelny_status",
        "edycja_wykonuje_reszte_akcji",
        "dedup_klucz_z_wartoscia",
        "edycja_sieroty_nie_otwiera",
        "entry_idempotencja",
        "dedup_management_po_restarcie",
        // --- Pakiet B: osie z audytu TYLER ---
        // Pominięcie klucza na tej liście = panel zeruje pole (incydent ×3
        // w historii) — dlatego wpis stoi tuż przy mapowaniu w `core_from_ui`.
        "rf_wymaga_wykonania",
        "market_entry_units",
        "market_hybrid_now_units",
        "market_hybrid_pending_units",
        "market_hybrid_lot_mult",
        "market_hybrid_max_chase_usd",
        "market_hybrid_tp_stage",
        "market_unfilled_cancel_stage",
        "pending_cancel_on_riskfree",
        "bank_all_at_stage",
        // --- Pakiet E: statystyki ---
        "stat_be_prog_usd",
        "tp_detect_price",
        "tp_detect_signal",
        "breakeven_protection",
        "trail_after_tp2",
        "be_offset",
        "sltp_retry_s",
        "rev_exit_range",
        "rev_exit_slope",
        "rev_exit_profit",
        "rev_exit_window_min",
        // --- reguły doświadczonego tradera (nazwy wspólne z rdzeniem) ---
        "exit_min_hold_min",
        "exit_min_profit",
        "exit_r_multiple",
        "basket_target_usd",
        "exit_round_dist",
        "exit_round_step",
        "exit_spread_mult",
        "exit_on_opposite_signal",
        "hold_after_tp_hit_min",
        "toucher_tp_one_based",
        "pending_drop_arm",
        // --- rodzina MARGINESOWA i rodzina OCZEKUJACYCH ---
        "ml_licz_wiszace",
        "ml_min_wejscie",
        "ml_min_warstwa",
        "ml_min_reentry",
        "ml_min_rearm",
        "ml_min_piramida",
        "ml_min_fast_addon",
        "ml_min_relot_up",
        "ml_min_drabina",
        "konto_dzwignia",
        "wiek_od_wypelnienia",
        "pending_drop_grace_min",
        "pending_drop_grace_max_dist",
        "pending_drop_keep_n",
        "grid_anchor_absolute",
        "units_per_level",
        "units_per_level_zone",
        "pending_cross_policy",
        "tp_open_extra",
        // --- mądre wyjście (nazwy wspólne z rdzeniem) ---
        "smart_exit",
        "smart_exit_take",
        "smart_exit_giveback",
        "smart_exit_min_peak",
        "smart_exit_drop_speed",
        "smart_exit_speed_window_s",
        "smart_exit_hold_if_pending",
        "smart_exit_min_pendings",
        "smart_exit_pending_scope",
        "smart_exit_pending_min_dist",
        "reenter_after_tp",
        "reenter_min_tp_stage",
        "reenter_max",
        "side_filter",
        "regime_filter",
        "regime_ma_hours",
        "max_open_baskets",
        "max_directional_lots",
        "equity_floor_pct",
        "dd_guard_scope",
        "exposure_bonus_profit_pct",
        "exposure_bonus_positions",
        "exposure_bonus_baskets",
        "fast_fill_reject_s",
        "fast_fill_layers",
        "fast_fill_soft_age_min",
        "zone_exit_adverse_s",
        "zone_exit_adverse_close",
        "reenter_min_return_s",
        "pyramid_after_stage",
        "pyramid_lot_mult",
        "pyramid_regime_lookback",
        "pyramid_regime_max_fast_pct",
        "pyramid_min_equity_mult",
        "fast_addon_move_usd",
        "fast_addon_window_s",
        "fast_addon_max",
        "fast_addon_lot_mult",
        "fast_addon_min_stage",
        "fast_addon_cooldown_s",
        // --- bramki kapitałowe (rodzina `*_small`) ---
        "entry_units_small",
        "entry_units_small_mult",
        "risk_per_basket_pct_small",
        "risk_per_basket_pct_small_mult",
        "reenter_max_small",
        "reenter_max_small_mult",
        "max_open_positions_small",
        "max_open_positions_small_mult",
        "max_open_baskets_small",
        "max_open_baskets_small_mult",
        "basket_max_age_min_small",
        "basket_max_age_min_small_mult",
        "fast_fill_soft_age_min_small",
        "fast_fill_soft_age_min_small_mult",
        "market_entry_step_small",
        "market_entry_step_small_mult",
        "sl_min_dist_small",
        "sl_min_dist_small_mult",
        "lot_percent_small",
        "lot_percent_small_mult",
        "commission_per_lot",
        "exec_latency_ms",
        "slippage_pts",
        "server_tz_offset_h",
        "msg_clock_offset_h",
        "ai_decision_interval_s",
        // --- dziennik zdarzeń (JSON Lines) ---
        "journal_enabled",
        "journal_min_level",
        "journal_snapshots",
        "journal_excursions",
        "journal_text_mirror",
        "journal_retention_days",
        "journal_buffer_cap",
        // --- czyta je `live.rs`, nie rdzeń ---
        "archive_retention_days",
        // Katalog docelowy scalania alllogs — czyta go `alllogs.rs` przy
        // zapisie; puste = katalog `logs` bota. Ścieżka SERWERA (panel bywa
        // na innej maszynie), wybierana modalem `GET /api/fs/dirs`.
        "alllogs_dir",
        // Próg OSTRZEŻENIA mailem o obsunięciu. Świadomie poza `Settings`, bo
        // nie zmienia ani jednej decyzji handlowej — nie zatrzymuje bota,
        // tylko wysyła list. Wchodzi w grę zwłaszcza przy WYŁĄCZONYM strażniku
        // (0/0), gdzie bez niego kategoria „drawdown" nie wysłałaby niczego,
        // cokolwiek działoby się z kontem w nocy.
        "alert_dd_pct",
        // Bramka WIEKU sygnału. Tak samo jak `alert_dd_pct` — świadomie poza
        // `Settings`, bo pilnuje jej `live.rs`, a nie rdzeń. Do 07.08.2026
        // klucz był czytany, ale NIE ISTNIAŁ w panelu ani w `defaultSettings`:
        // działał wyłącznie sztywny próg 5 minut z kodu, którego nie dało się
        // ani zobaczyć, ani zmienić — a po każdej dłuższej przerwie w moście
        // do MT5 cicho odrzucał całą zaległą kolejkę otwarć.
        "signal_max_age_min",
        // --- RISK FREE jako reguła (RDZEŃ, 29.07) ---
        "riskfree_enabled",
        "riskfree_trigger_usd",
        "riskfree_trigger_r",
        "riskfree_keep_units",
        "riskfree_be_offset",
        "riskfree_runner_target",
        "riskfree_runner_stop",
        "riskfree_runner_gap",
        "riskfree_runner_max_hold_min",
        // --- poprawki istniejących reguł ---
        "pending_drop_require_zone_touch",
        "trail_runners_by_depth",
        // --- model kosztów brokera (Priorytet 0) ---
        "ai_replaces_management",
        "swap_enabled",
        "swap_long_points",
        "swap_short_points",
        "swap_point_value",
        "swap_rollover_weekday",
        "swap_rollover_mult",
        "slippage_pending_pts",
        // --- Pakiet D1/D1b: weekend i doba rolowania z serwera ---
        "swap_pomijaj_weekend",
        "swap_rollover_z_serwera",
        "swap_rollover3days_mt5",
        // --- Pakiet D5/D6/D4 + D3: wierność pętli backtestu ---
        "runner_ksiegowanie_v2",
        "msg_kurs_sprzed_luki",
        "live_tick_order_strict",
        "stop_out_level_pct",
        "margin_call_level_pct",
        "entry_depth_curve",
        // --- twardy czas życia koszyka + filtr trendu wyższego rzędu ---
        "basket_max_age_min",
        "trend_filter_enabled",
        "trend_filter_window_h",
        "trend_filter_drop_pct",
        "trend_filter_mode",
        "trend_filter_shrink",
        // --- wielkość pozycji wg jakości szczebla ---
        "entry_weights_from_rr",
        "entry_weights_rr_power",
        "entry_weights_rr_cap",
        "drop_unplaceable_levels",
        // --- niezmiennik krawędzi strefy (25.08.2026) ---
        "zakaz_ponizej_krawedzi",
        // --- parametry liczone z sygnału ---
        "adaptive_params",
        "sl_min_dist_zone_mult",
        "sl_min_dist_atr_mult",
        "sl_min_dist_floor",
        "sl_min_dist_cap",
        "entry_deep_zone_mult",
        "entry_units_zone_ref",
        "adaptive_atr_window_min",
        "units_by_hour",
        // --- skalowanie po zdarzeniu ---
        "basket_realized_broker_only",
        "confirmed_exit_retry",
        "defer_entry_until_receipts",
        "deferred_entry_max_age_s",
        "rearm_grid_on_return",
        "rearm_keep_empty_alive",
        "rearm_block_after_secured",
        "spp_blocks_rearm_when_flat",
        "rearm_min_basket_profit",
        "rearm_max_times",
        "rearm_min_gap_min",
        // --- konto ---
        "day_target_pct",
        "day_trail_stop_pct",
        "day_trail_arm_pct",
        "day_trail_basis",
        "profit_budget_arm_pct",
        "profit_budget_keep_pct",
        "profit_budget_deploy_pct",
        // --- budżet transakcji ---
        "daily_signal_budget",
        "signal_min_rr",
        "signal_min_zone_width",
        "signal_max_zone_width",
        // --- łączenie koszyków ---
        "merge_same_side",
        "merge_window_min",
        "merge_min_overlap",
        // --- wyjście limitem ---
        "exit_via_limit",
        "exit_limit_offset",
        "exit_limit_wait_s",
        "exit_limit_min_profit",
        // --- kredyt bonusowy (podstawa wielkości pozycji) ---
        "odlicz_kredyt",
        "credit_balance_separate",
        "close_receipt_reconcile",
        "closed_profit_net_costs",
        "restore_strategy_continuation",
        "order_volume_contract_v2",
        "entry_edit_geometry_v2",
        "sr_warmup_exact_ticks",
        "be_never_loosen",
        "retarget_respects_final_target",
        "kredyt_reczny",
        // --- most dla osi Fazy 6 (37 pól, AUDYT_SILNIKA.md TOP 1) ---
        // Pominięcie klucza tutaj = fałszywy alarm `unmapped_keys`; pominięcie
        // w `core_from_ui` = panel zeruje pole przy edycji. Oba wpisy idą parą.
        "sesja_bramka",
        "regime_cena",
        "regime_miara",
        "regime_pilnuj_limitow",
        "regime_percentyl",
        "regime_strefa_martwa",
        "regime_okno2_h",
        "regime_zmiennosc_min",
        "regime_zmiennosc_max",
        "regime_gdy_rozerwany",
        // stare nazwy pól (aliasy serde) — czytane jako zapasowe, patrz wyżej
        "regime_range_mute_mode",
        "regime_range_mute_usd",
        "regime_soft",
        "regime_soft_units_mult",
        "regime_soft_lot_mult",
        "regime_soft_max_positions",
        "regime_soft_risk_mult",
        "vol_size_mode",
        "vol_size_target",
        "vol_size_min_mult",
        "vol_size_max_mult",
        "vol_size_percentile_okno",
        "vol_size_odsezonuj",
        "sanity_zone_max",
        "sanity_tp_max",
        "sanity_tp_rosnace",
        "sanity_tp_strona",
        "parser_geometryczny",
        "parser_min_pewnosc",
        "cel_z_przeciwnego",
        "cel_z_przeciwnego_zapas",
        "day_gate_od_salda",
        "day_gate_do_salda",
        "no_tp_after_stage",
        "no_reenter_from_stage",
        "oae_skip_after_riskfree",
        "reenter_stop_after_riskfree",
        "lot_max_z_salda",
        "trail_atr_mult",
        "trail_adaptive_enabled",
        "trail_adaptive_runners_only",
        "trail_adaptive_window_s",
        "trail_adaptive_min_samples",
        "trail_adaptive_trend_er",
        "trail_adaptive_reversal_er",
        "trail_adaptive_trend_gap_mult",
        "trail_adaptive_chop_gap_mult",
        "trail_adaptive_reversal_gap_mult",
        "trail_adaptive_fast_vol_s",
        "trail_adaptive_slow_vol_s",
        "trail_adaptive_vol_ratio",
        "trail_adaptive_vol_favorable_mult",
        "trail_adaptive_vol_adverse_mult",
        "trail_adaptive_min_peak",
        "trail_adaptive_min_gap",
        "trail_adaptive_max_gap",
        // Klucz liczbowy obok `trail_after_tp2`: `core_from_ui` czyta go,
        // a `preset_to_ui` wypisuje od dnia powstania (`smart_sl_delay = 2`
        // w NEWALPHA-2/OMEGA-1 musi przeżyć obieg) — brak wpisu tutaj zapalał
        // fałszywy alarm `unmapped_keys` przy każdym takim presecie.
        "smart_sl_delay_n",
    ];
    let obj = match doc.as_object() {
        Some(o) => o,
        None => return Vec::new(),
    };
    obj.keys()
        .filter(|k| !MAPPED.contains(&k.as_str()) && !UI_ONLY_KEYS.contains(&k.as_str()))
        .cloned()
        .collect()
}

// ============================================================
//  PRESET (kształt SILNIKA) → DOKUMENT PANELU
// ============================================================

/// Tłumaczy preset zapisany w kluczach SILNIKA na klucze PANELU.
///
/// # Po co to w ogóle istnieje
///
/// `bt.exe` wczytuje preset wprost do [`Settings`] przez serde, więc backtest
/// widzi dokładnie to, co w pliku. Bot na żywo idzie inną drogą: preset ląduje
/// w `settings.json`, a silnik dostaje wynik [`core_from_ui`] — który czyta
/// klucze PANELU. Dla większości pól obie nazwy są takie same i nic się nie
/// dzieje, ale kilkanaście pól panel trzyma pod inną nazwą albo rozbite na
/// kilka przełączników. Te pola po wczytaniu presetu **wracały do wartości
/// domyślnych**, choć w pliku miały inną wartość.
///
/// Skutek był dokładnie taki, jakiego nikt nie chce zobaczyć na koncie:
/// preset zmierzony jako `AllRunners` z `pending_lifetime = UntilTp1`
/// handlował na żywo drabinką `Ladder`, bo `tp_schedule` i `pending_lifetime`
/// nie mają w panelu pól o tej nazwie. Backtest i bot liczyły dwie różne
/// konfiguracje pod jedną nazwą.
///
/// Funkcja NIE zmienia zachowania silnika — zmienia to, czy silnik w ogóle
/// dostaje ustawienia z presetu.
/// Nazwa trybu zapadki w konwencji PANELU, przyjmowana w obu konwencjach.
///
/// `preset_to_ui` dostaje dwa rodzaje dokumentów: preset z pliku (kształt
/// SILNIKA — `"Tiered"`) oraz katalog presetów wbudowany w interfejs, który
/// przychodzi w `ApplyPreset { values }` już w kształcie PANELU (`"tiered"`).
/// Rozpoznawanie wyłącznie nazw rdzenia dawało przy tych drugich cichą
/// podmianę: `"tiered"` nie pasowało do żadnej gałęzi, więc wpadało do
/// przypadku „Off" i zapisywało `runner_trail = false` z nazwą `"gap"` —
/// czyli **wyłączało zapadkę** w presecie, który ją miał włączoną, i robiło
/// to bez jednego słowa w dzienniku.
///
/// `None` znaczy „tryb wyłączony albo nierozpoznany" — wywołujący decyduje,
/// co z tym zrobić.
fn nazwa_trybu_zapadki(tryb: &str) -> Option<&'static str> {
    match tryb {
        "Gap" | "gap" => Some("gap"),
        "LockPct" | "lock_pct" => Some("lock_pct"),
        "Tiered" | "tiered" => Some("tiered"),
        "Atr" | "atr" => Some("atr"),
        "Chandelier" | "chandelier" => Some("chandelier"),
        _ => None,
    }
}

pub fn preset_to_ui(preset: &Value) -> Value {
    let mut out = preset.clone();
    let Some(o) = out.as_object_mut() else {
        return out;
    };
    macro_rules! set {
        ($k:expr, $v:expr) => {
            o.insert($k.to_string(), $v)
        };
    }

    // ---------- strefa wejścia ----------
    match preset.get("zone_offset_mode").and_then(|v| v.as_str()) {
        Some("Directional") => {
            set!("entry_offset_dir", Value::Bool(true));
            set!("custom_entry", Value::Bool(false));
        }
        Some("Price") => {
            set!("entry_offset_dir", Value::Bool(false));
            set!("custom_entry", Value::Bool(true));
        }
        Some(_) => {
            set!("entry_offset_dir", Value::Bool(false));
            set!("custom_entry", Value::Bool(false));
        }
        None => {}
    }
    kopiuj(preset, o, "entry_hi_offset", "entry_high_offset");
    kopiuj(preset, o, "entry_lo_offset", "entry_low_offset");

    // ---------- dotykacz ----------
    kopiuj(preset, o, "toucher_units", "entry_touch_units");
    kopiuj(preset, o, "toucher_bands", "entry_touch_levels");
    // panel liczy cele od 1, rdzeń od 0
    if let Some(i) = preset.get("toucher_tp_index").and_then(|v| v.as_f64()) {
        set!("entry_touch_tp", Value::from(i + 1.0));
    }

    // ---------- życie zleceń oczekujących ----------
    match preset.get("pending_lifetime").and_then(|v| v.as_str()) {
        Some("Never") => {
            set!("pending_never_cancel", Value::Bool(true));
            set!("valid_till_tp2", Value::Bool(false));
        }
        Some("UntilTp2") => {
            set!("pending_never_cancel", Value::Bool(false));
            set!("valid_till_tp2", Value::Bool(true));
        }
        Some(_) => {
            set!("pending_never_cancel", Value::Bool(false));
            set!("valid_till_tp2", Value::Bool(false));
        }
        None => {}
    }

    // ---------- stop loss ----------
    if let Some(v) = preset.get("entry_sl_dist_limit").and_then(|v| v.as_f64()) {
        // zero znaczy „bez limitu", więc włącznik idzie w parze z wartością
        set!("sl_dist_limit", Value::Bool(v > 0.0));
        set!("sl_dist_max", Value::from(v));
    }
    kopiuj(preset, o, "vsl_broker_offset", "vsl_net_off");

    // ---------- cele ----------
    match preset.get("tp_schedule").and_then(|v| v.as_str()) {
        Some(tryb) => {
            set!("all_runners", Value::Bool(tryb == "AllRunners"));
            set!(
                "official_mode",
                Value::Bool(matches!(tryb, "OfficialPct" | "OfficialCounts"))
            );
            set!("official_use_counts", Value::Bool(tryb == "OfficialCounts"));
            set!("scale_out", Value::Bool(tryb == "ScaleOutPct"));
        }
        None => {}
    }
    if let Some(a) = preset.get("official_pct").and_then(|v| v.as_array()) {
        for (i, k) in [
            "official_pct_tp1",
            "official_pct_tp2",
            "official_pct_tp3",
            "official_pct_spp",
        ]
        .iter()
        .enumerate()
        {
            if let Some(v) = a.get(i).and_then(|x| x.as_f64()) {
                set!(k, Value::from(v));
            }
        }
    }
    kopiuj(preset, o, "assign_tp_per_position", "official_assign_tps");

    // ---------- breakeven / trailing ----------
    if let Some(v) = preset.get("be_lock_pts").and_then(|v| v.as_f64()) {
        set!("be_lock", Value::Bool(v != 0.0));
        set!("be_lock_points", Value::from(v));
    }
    if let Some(tryb) = preset.get("trail_mode").and_then(|v| v.as_str()) {
        let (wl, nazwa) = match nazwa_trybu_zapadki(tryb) {
            Some(n) => (true, n),
            None => (false, "gap"),
        };
        set!("runner_trail", Value::Bool(wl));
        set!("trail_mode", Value::String(nazwa.to_string()));
    }
    if let Some(tryb) = preset.get("trail_runner_mode").and_then(|v| v.as_str()) {
        // Tu włącznika nie ma — sama nazwa trybu wystarcza, ale musi być
        // w konwencji panelu (małe litery), inaczej `mode_of` jej nie pozna.
        set!(
            "trail_runner_mode",
            Value::String(nazwa_trybu_zapadki(tryb).unwrap_or("off").to_string())
        );
    }
    kopiuj(preset, o, "trail_start", "runner_trail_start");
    kopiuj(preset, o, "trail_gap", "runner_trail_gap");
    match preset.get("smart_sl_mode").and_then(|v| v.as_str()) {
        Some(tryb) => {
            let drabinka = matches!(tryb, "Ladder" | "LadderWithBe");
            let be = matches!(tryb, "BreakevenOnly" | "LadderWithBe");
            let tylko_po_rf = preset
                .get("smart_sl_only_after_rf")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            set!("smart_sl", Value::Bool(drabinka && !tylko_po_rf));
            set!("risk_free_smart_sl", Value::Bool(drabinka && tylko_po_rf));
            set!("breakeven_protection", Value::Bool(be));
        }
        None => {}
    }
    if let Some(v) = preset.get("smart_sl_delay").and_then(|v| v.as_f64()) {
        set!("trail_after_tp2", Value::Bool(v >= 1.0));
        // WARTOŚĆ LICZBOWA MUSI PRZEŻYĆ OBIEG. Sam przełącznik dwustanowy
        // zgniata `smart_sl_delay = 2` do 1: silnik(2) → panel(true) →
        // silnik(1). NEWALPHA-2 i OMEGA-1 stoją na dwójce (zapadka wchodzi
        // dwa szczeble później — to ta liczba, którą niezależnie potwierdza
        // szkolenie nadawcy: „SL to BE is now" dopiero przy TP2), więc
        // PIERWSZA edycja takiego presetu z panelu po cichu go psuła.
        // Klucz liczbowy jedzie obok przełącznika i ma nad nim pierwszeństwo
        // przy odczycie — patrz `core_from_ui`.
        set!("smart_sl_delay_n", Value::from(v));
    }

    // ---------- żniwa ----------
    if let Some(v) = preset.get("harvest_retrace_pct").and_then(|v| v.as_f64()) {
        set!("harvest", Value::Bool(v > 0.0));
    }

    // ---------- wykrywanie celów ----------
    // Para `tp_detect_*` jest w `core_from_ui` czytana PRZED `tp_source`, więc
    // sam `tp_source` z presetu i tak wygrywa. Zostawiamy obie wartości.

    // ---------- zegar serwera ----------
    if let Some(ms) = preset.get("server_tz_offset_ms").and_then(|v| v.as_f64()) {
        set!("server_tz_offset_h", Value::from(ms / 3_600_000.0));
    }

    // ---------- bankowanie transz ----------
    kopiuj(preset, o, "bank_close_last", "official_close_last");
    if let Some(v) = preset.get("bank_from").and_then(|v| v.as_str()) {
        set!(
            "scale_out_from",
            Value::String(if v == "Best" { "best" } else { "worst" }.into())
        );
    }
    if let Some(v) = preset.get("bank_rounding").and_then(|v| v.as_str()) {
        let nazwa = match v {
            "Nearest" => "nearest",
            "Down" => "down",
            _ => "up",
        };
        // panel ma dwa pola (oficjalny harmonogram / scale-out); rdzeń jedno,
        // a `core_from_ui` czyta to, które pasuje do włączonego trybu — więc
        // wpisujemy w oba, inaczej wartość zależałaby od kolejności ustawień
        set!("official_round", Value::String(nazwa.to_string()));
        set!("scale_out_round", Value::String(nazwa.to_string()));
    }
    if let Some(v) = preset.get("last_runner").and_then(|v| v.as_str()) {
        let nazwa = match v {
            "NextTp" => "next_tp",
            "NoTp" => "no_tp",
            _ => "runner",
        };
        set!("scale_out_last_runner", Value::String(nazwa.to_string()));
    }

    // ---------- pozostałe różnice nazw ----------
    kopiuj(preset, o, "basket_hint_tolerance", "price_tol");
    kopiuj(preset, o, "pending_resize_s", "pending_resize_sec");
    kopiuj(preset, o, "ppm_for_market", "ppm_immediate");
    if let Some(ms) = preset.get("msg_clock_offset_ms").and_then(|v| v.as_f64()) {
        set!("msg_clock_offset_h", Value::from(ms / 3_600_000.0));
    }

    // ---------- słownik wariantów: kształt SILNIKA → słownik PANELU ----------
    //
    // Te pola mają w panelu i w rdzeniu TĘ SAMĄ nazwę klucza, więc audyt
    // (`unmapped_keys`) pokazywał je jako „podpięte" — a mimo to nie działały,
    // bo różnią się SŁOWNIKIEM WARTOŚCI: panel zapisuje `"counter"`, preset
    // niesie `"CounterMa"`. `core_from_ui` rozumie już obie konwencje (patrz
    // gałęzie tam), ale samo to nie wystarcza: lista rozwijana w panelu
    // porównuje wartość ze swoim `options[].value`, więc nieprzetłumaczona
    // nazwa rdzenia pokazywałaby PUSTE pole i pierwsza edycja czegokolwiek
    // obok zapisałaby je jako puste.
    //
    // Dlatego tłumaczymy tu wartość, a klucz zostawiamy — nazwa jest wspólna.
    {
        let mut przetlumacz = |klucz: &str, f: &dyn Fn(&str) -> Option<&'static str>| {
            if let Some(v) = preset.get(klucz).and_then(|v| v.as_str()) {
                if let Some(n) = f(v) {
                    o.insert(klucz.to_string(), Value::String(n.to_string()));
                }
            }
        };
        przetlumacz("out_at_entry_mode", &|v| match v {
            "CloseLosersOnly" => Some("losers"),
            "CloseFlatOnly" => Some("flat"),
            "MoveSlToBe" => Some("be"),
            "CloseAll" => Some("close_all"),
            _ => None,
        });
        przetlumacz("sl_hit_mode", &|v| match v {
            "CloseAll" => Some("close_all"),
            "VerifyByPrice" => Some("verify"),
            "Ignore" => Some("ignore"),
            "CancelPendings" => Some("cancel_pendings"),
            _ => None,
        });
        przetlumacz("regime_filter", &|v| match v {
            "TrendMa" => Some("trend"),
            "CounterMa" => Some("counter"),
            "Off" => Some("off"),
            _ => None,
        });
        przetlumacz("side_filter", &|v| match v {
            "BuyOnly" => Some("buy"),
            "SellOnly" => Some("sell"),
            "Both" => Some("both"),
            _ => None,
        });
        przetlumacz("dd_guard_scope", &|v| match v {
            "Lifetime" => Some("lifetime"),
            "LifetimePeakDailyReset" => Some("lifetime_daily_reset"),
            "Daily" => Some("daily"),
            _ => None,
        });
        przetlumacz("day_trail_basis", &|v| match v {
            "EquityPeak" => Some("equity_peak"),
            "ProfitPeak" => Some("profit_peak"),
            _ => None,
        });
        przetlumacz("risk_free_runner_target", &|v| match v {
            "KeepTp" => Some("keep"),
            "NextTp" => Some("next"),
            "NoTpTrailOnly" => Some("none"),
            "LastTp" => Some("last"),
            _ => None,
        });
    }
    // Osie Fazy 6 (sesja_bramka, regime_*, vol_size_*, sanity_*,
    // cel_z_przeciwnego, …) przechodzą tędy BEZ tłumaczenia wartości: klucz
    // jest wspólny z rdzeniem, a panel nie ma dla nich kontrolek, więc słownik
    // panelu nie istnieje — dokument niesie nazwy wariantów serde („Miekko"),
    // które `core_from_ui` czyta wprost. Tłumaczenie na małe litery bez listy
    // rozwijanej po drugiej stronie niczego by nie chroniło, a dodałoby trzecią
    // konwencję nazw. Gdy pole dostanie kontrolkę, dopisać je do bloku wyżej.

    // `risk_free_mode` ma w panelu tylko dwa warianty, bo trzeci („nie reaguj")
    // mieszka w osobnym przełączniku `ignore_risk_free`.
    match preset.get("risk_free_mode").and_then(|v| v.as_str()) {
        Some("Ignore") => {
            set!("ignore_risk_free", Value::Bool(true));
        }
        Some("MoveSlToBeOnly") => {
            set!("ignore_risk_free", Value::Bool(false));
            set!("risk_free_mode", Value::String("all_runners".into()));
        }
        Some("CloseAllKeepNearest") => {
            set!("ignore_risk_free", Value::Bool(false));
            set!("risk_free_mode", Value::String("scale_out".into()));
        }
        _ => {}
    }

    // ---------- AI ----------
    kopiuj(preset, o, "ai_enabled", "ai_mode");

    // ---------- symulacja ----------
    // `stops_level` z presetu dotyczy backtestu; na żywo wartość i tak
    // przychodzi z serwera brokera (patrz `live.rs`).
    kopiuj(preset, o, "stops_level", "sim_stops_level");

    // Klucze w nazewnictwie SILNIKA, które właśnie przetłumaczyliśmy, znikają
    // z dokumentu panelu. Zostawione zaśmiecałyby diagnostykę („nieobsługiwane
    // ustawienie") i kusiły, żeby kiedyś zacząć je czytać — a wtedy byłyby dwa
    // źródła prawdy dla jednego pola.
    for k in PRZETLUMACZONE {
        o.remove(*k);
    }

    out
}

/// Klucze presetu (nazewnictwo silnika), które [`preset_to_ui`] zamienia na
/// odpowiedniki panelu i usuwa z dokumentu.
const PRZETLUMACZONE: &[&str] = &[
    "zone_offset_mode",
    "entry_hi_offset",
    "entry_lo_offset",
    "toucher_units",
    "toucher_bands",
    "toucher_tp_index",
    "pending_lifetime",
    "entry_sl_dist_limit",
    "vsl_broker_offset",
    "tp_schedule",
    "official_pct",
    "assign_tp_per_position",
    "be_lock_pts",
    "trail_start",
    "trail_gap",
    "smart_sl_mode",
    "smart_sl_delay",
    "smart_sl_only_after_rf",
    "bank_close_last",
    "bank_from",
    "bank_rounding",
    "last_runner",
    "basket_hint_tolerance",
    "pending_resize_s",
    "ppm_for_market",
    "msg_clock_offset_ms",
    "server_tz_offset_ms",
    "ai_enabled",
    "stops_level",
];

fn kopiuj(
    zrodlo: &Value,
    cel: &mut serde_json::Map<String, Value>,
    z_klucza: &str,
    do_klucza: &str,
) {
    if let Some(v) = zrodlo.get(z_klucza) {
        cel.insert(do_klucza.to_string(), v.clone());
    }
}

/// Wpisuje wielkość pozycji z karty panelu do konfiguracji SILNIKA.
///
/// `core_from_ui` świadomie pomija trzy pola lota (patrz komentarz przy
/// „wielkość pozycji"), bo mają własny sterownik `Command::SetLot`. Tryb DEMO
/// stosował je od początku — ścieżka ŻYWA nie stosowała ich NIGDZIE, więc
/// `lot_mode_percent` zostawał przy domyślnym `false`, a bot handlował stałym
/// `lot_fixed = 0,01` NIEZALEŻNIE od presetu i od tego, co pokazywał panel.
///
/// Objaw z 30.07.2026: 72 transakcje na koncie mają wolumen dokładnie 0,01,
/// choć ULTRA-X3 każe grać 0,5 % kapitału (0,02 przy saldzie 483 $, 0,04 przy
/// 787 $). Backtest tego samego presetu przy 483 $ używa lotów 0,01–0,46.
/// Konto nie miało więc prawa się złożyć — rosłoby liniowo zamiast wykładniczo,
/// a wszystkie wyniki presetu zakładają skalowanie wolumenu saldem.
pub fn apply_lot(core: &mut Settings, lot: &crate::ui::LotConfig) {
    core.lot_mode_percent = lot.mode == "percent";
    core.lot_fixed = lot.fixed;
    core.lot_percent = lot.percent;
}

/// Wielkość pozycji zapisana w presecie, w kształcie panelu.
///
/// Lot świadomie NIE przechodzi przez `core_from_ui` (ma własny sterownik
/// `Command::SetLot`), więc wczytanie presetu musi ustawić go osobno — inaczej
/// preset zmierzony na 0,5 % kapitału handlowałby lotem, który akurat został
/// w panelu z poprzedniego razu.
pub fn preset_lot(preset: &Value) -> Option<crate::ui::LotConfig> {
    let percent = preset.get("lot_mode_percent")?.as_bool()?;
    Some(crate::ui::LotConfig {
        mode: if percent {
            "percent".into()
        } else {
            "fixed".into()
        },
        fixed: preset
            .get("lot_fixed")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.01),
        percent: preset
            .get("lot_percent")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0),
    })
}

/// Scala łatkę w dokument ustawień (płytko, klucz po kluczu).
pub fn merge_patch(doc: &mut Value, patch: &Value) {
    if !doc.is_object() {
        *doc = Value::Object(Default::default());
    }
    let (Some(d), Some(p)) = (doc.as_object_mut(), patch.as_object()) else {
        return;
    };
    for (k, v) in p {
        d.insert(k.clone(), v.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_pending_validity_is_a_real_ui_control_and_roundtrips() {
        assert!(!core_from_ui(&serde_json::json!({})).explicit_pending_until_cancel);
        for enabled in [false, true] {
            let ui=serde_json::json!({"explicit_pending_until_cancel":enabled});
            assert!(unmapped_keys(&ui).is_empty());
            let mapped=core_from_ui(&ui);
            assert_eq!(mapped.explicit_pending_until_cancel,enabled);
            let doc=preset_to_ui(&serde_json::to_value(&mapped).unwrap());
            assert_eq!(doc["explicit_pending_until_cancel"],enabled);
            assert_eq!(core_from_ui(&doc).explicit_pending_until_cancel,enabled);
        }
    }

    #[test]
    fn profit_budget_fields_roundtrip_without_changing_other_strategy_axes() {
        let original=serde_json::json!({"profit_budget_arm_pct":12.0,"profit_budget_keep_pct":75.0,
            "profit_budget_deploy_pct":40.0});
        let c=core_from_ui(&original);assert_eq!(c.profit_budget_arm_pct,12.0);
        assert_eq!(c.profit_budget_keep_pct,75.0);assert_eq!(c.profit_budget_deploy_pct,40.0);
        let ui=preset_to_ui(&serde_json::to_value(&c).unwrap());
        for name in ["profit_budget_arm_pct","profit_budget_keep_pct","profit_budget_deploy_pct"] {
            assert_eq!(ui[name],original[name]);
        }
        let restored=core_from_ui(&ui);assert_eq!(restored.profit_budget_arm_pct,12.0);
        assert_eq!(restored.day_trail_stop_pct,Settings::default().day_trail_stop_pct);
        let defaults=core_from_ui(&serde_json::json!({}));
        assert_eq!((defaults.profit_budget_arm_pct,defaults.profit_budget_keep_pct,defaults.profit_budget_deploy_pct),(0.0,50.0,100.0));
    }

    #[test]
    fn day_trail_basis_roundtrips_both_ui_and_core_names() {
        for (basis, ui_name, core_name) in [
            (DayTrailBasis::EquityPeak, "equity_peak", "EquityPeak"),
            (DayTrailBasis::ProfitPeak, "profit_peak", "ProfitPeak"),
        ] {
            for name in [ui_name, core_name] {
                let typed = core_from_ui(&serde_json::json!({"day_trail_basis": name}));
                assert_eq!(typed.day_trail_basis, basis);
                let ui = preset_to_ui(&serde_json::to_value(&typed).unwrap());
                assert_eq!(ui["day_trail_basis"], ui_name);
                assert_eq!(core_from_ui(&ui).day_trail_basis, basis);
            }
        }
        assert_eq!(core_from_ui(&serde_json::json!({})).day_trail_basis, DayTrailBasis::EquityPeak);
    }
    use serde_json::json;

    #[test]
    fn pusty_dokument_daje_domyslne_ustawienia() {
        let c = core_from_ui(&json!({}));
        assert_eq!(c, Settings::default());
    }

    #[test]
    fn tryb_trailingu_sklada_sie_z_kilku_kluczy() {
        // sam `trail_mode` bez włącznika nie może włączyć trailingu
        let c = core_from_ui(&json!({ "trail_mode": "tiered" }));
        assert_eq!(c.trail_mode, TrailMode::Off);

        let c = core_from_ui(&json!({ "runner_trail": true, "trail_mode": "tiered" }));
        assert_eq!(c.trail_mode, TrailMode::Tiered);
    }

    #[test]
    fn smart_sl_to_osobna_rzecz_niz_trailing() {
        // Trailing podąża za CENĄ, smart SL wspina się po drabince CELÓW.
        // Włączenie jednego nie może włączać drugiego — w poprzednim mostku
        // `smart_sl` po cichu ustawiał trailing i drabinka nie działała wcale.
        let c = core_from_ui(&json!({ "smart_sl": true, "trail_mode": "tiered" }));
        assert_eq!(
            c.trail_mode,
            TrailMode::Off,
            "smart SL nie włącza trailingu"
        );
        assert_eq!(c.smart_sl_mode, SmartSlMode::Ladder);

        let c = core_from_ui(&json!({ "smart_sl": true, "breakeven_protection": true }));
        assert_eq!(c.smart_sl_mode, SmartSlMode::LadderWithBe);

        let c = core_from_ui(&json!({ "breakeven_protection": true }));
        assert_eq!(c.smart_sl_mode, SmartSlMode::BreakevenOnly);

        let c = core_from_ui(&json!({ "smart_sl": true, "trail_after_tp2": true }));
        assert_eq!(
            c.smart_sl_delay, 1,
            "„tylko po TP2\" to opóźnienie drabinki"
        );
    }

    #[test]
    fn wagi_glebokosci_i_limit_ryzyka_docieraja_z_panelu() {
        let c = core_from_ui(&json!({
            "entry_weights": "1,2,4",
            "risk_per_basket_pct": 3.5,
        }));
        assert_eq!(c.entry_weights, "1,2,4");
        assert_eq!(c.risk_per_basket_pct, 3.5);
        assert!(unmapped_keys(&json!({ "entry_weights": "1,2,4" })).is_empty());
    }

    #[test]
    fn zaokraglanie_transzy_bierze_pole_wlasciwe_dla_trybu() {
        // panel ma dwa pola, rdzeń jedno — liczy się to, które odpowiada
        // włączonemu harmonogramowi
        let c = core_from_ui(&json!({
            "official_mode": true,
            "official_round": "down",
            "scale_out_round": "up",
        }));
        assert_eq!(c.bank_rounding, BankRounding::Down);

        let c = core_from_ui(&json!({
            "official_mode": false,
            "official_round": "down",
            "scale_out_round": "nearest",
        }));
        assert_eq!(c.bank_rounding, BankRounding::Nearest);
    }

    #[test]
    fn stara_para_tp_detect_wyprowadza_zrodlo_celu() {
        // presety zapisane przed wprowadzeniem `tp_source` muszą zachować się
        // dokładnie tak, jak zachowywały się w poprzednim bocie
        let c = core_from_ui(&json!({ "tp_detect_price": true, "tp_detect_signal": false }));
        assert_eq!(c.tp_source, TpSource::PriceOnly);
        let c = core_from_ui(&json!({ "tp_detect_price": false, "tp_detect_signal": true }));
        assert_eq!(c.tp_source, TpSource::SignalOnly);
        let c = core_from_ui(&json!({ "tp_detect_price": true, "tp_detect_signal": true }));
        assert_eq!(c.tp_source, TpSource::Either);
        // oba wyłączone znaczyło „niech działa cena", nie „nie wykrywaj wcale"
        let c = core_from_ui(&json!({ "tp_detect_price": false, "tp_detect_signal": false }));
        assert_eq!(c.tp_source, TpSource::PriceOnly);

        // jawny wybór wygrywa ze starą parą
        let c = core_from_ui(&json!({
            "tp_detect_price": true,
            "tp_detect_signal": false,
            "tp_source": "SignalConfirmedByPrice",
        }));
        assert_eq!(c.tp_source, TpSource::SignalConfirmedByPrice);
    }

    #[test]
    fn front_run_tp_dociera_z_panelu_do_rdzenia() {
        let c = core_from_ui(&json!({ "tp_price_front_run_usd": 0.25 }));
        assert_eq!(c.tp_price_front_run_usd, 0.25);
        assert!(unmapped_keys(&json!({ "tp_price_front_run_usd": 0.25 })).is_empty());
    }

    #[test]
    fn smart_sl_po_risk_free_nie_wlacza_drabinki_wszedzie() {
        // `risk_free_smart_sl` dotyczył koszyka PO komunikacie, a nie całego
        // bota — inaczej włączenie go po cichu zmienia stopy każdego setupu
        let c = core_from_ui(&json!({ "risk_free_smart_sl": true }));
        assert_eq!(c.smart_sl_mode, SmartSlMode::Ladder);
        assert!(
            c.smart_sl_only_after_rf,
            "drabinka tylko dla zabezpieczonych koszyków"
        );

        // globalny `smart_sl` zdejmuje to ograniczenie
        let c = core_from_ui(&json!({ "smart_sl": true, "risk_free_smart_sl": true }));
        assert_eq!(c.smart_sl_mode, SmartSlMode::Ladder);
        assert!(!c.smart_sl_only_after_rf);
    }

    #[test]
    fn latki_na_bledy_pythona_nie_sa_zglaszane_jako_braki() {
        // Obie flagi opisywały błędy, których w tym silniku nie da się
        // popełnić. Zgłaszanie ich jako „niedziałających" byłoby myleniem.
        assert!(unmapped_keys(&json!({
            "grid_fallback_best_edge": true,
            "day_flat_broker_clock": true,
        }))
        .is_empty());
    }

    #[test]
    fn tolerancja_cen_sluzy_do_wskazania_koszyka() {
        // „TOLERANCJA CEN" przestała być ustawieniem wyłącznie panelu —
        // rdzeń dopasowuje nią komunikat do koszyka po podanym poziomie
        let c = core_from_ui(&json!({ "price_tol": 1.25 }));
        assert_eq!(c.basket_hint_tolerance, 1.25);
        assert!(!UI_ONLY_KEYS.contains(&"price_tol"));
    }

    #[test]
    fn harmonogram_celow_wybiera_sie_z_flag() {
        assert_eq!(core_from_ui(&json!({})).tp_schedule, TpSchedule::Ladder);
        assert_eq!(
            core_from_ui(&json!({"all_runners":true})).tp_schedule,
            TpSchedule::AllRunners
        );
        assert_eq!(
            core_from_ui(&json!({"official_mode":true,"official_use_counts":true})).tp_schedule,
            TpSchedule::OfficialCounts
        );
        assert_eq!(
            core_from_ui(&json!({"scale_out":true})).tp_schedule,
            TpSchedule::ScaleOutPct
        );
    }

    #[test]
    fn strefa_kierunkowa_wygrywa_z_cenowa() {
        let c = core_from_ui(&json!({ "custom_entry": true, "entry_offset_dir": true }));
        assert_eq!(c.zone_offset_mode, ZoneOffsetMode::Directional);
    }

    #[test]
    fn nieznane_klucze_sa_raportowane_a_nie_ciche() {
        // `rev_exit_range` jest już obsługiwane; klucz spoza obu list nadal
        // musi być zgłoszony, zamiast po cichu nic nie robić
        let brak = unmapped_keys(&json!({
            "max_dd_pct": 60,
            "rev_exit_range": 5,
            "poll_ms": 250,
            "wymyslony_klucz": 1,
        }));
        assert_eq!(brak, vec!["wymyslony_klucz".to_string()]);
    }

    #[test]
    fn nadzor_mt5_dociera_z_panelu_do_konfiguracji() {
        let c = core_from_ui(&json!({
            "mt5_autostart": false,
            "mt5_watchdog": true,
            "mt5_terminal_path": "D:/MT5/terminal64.exe",
            "mt5_retry_attempts": 4,
            "mt5_retry_delay_s": 9,
            "mt5_restart_after": 0,
            "mt5_health_interval_s": 30,
        }));
        assert!(!c.mt5_autostart);
        assert!(c.mt5_watchdog);
        assert_eq!(c.mt5_terminal_path, "D:/MT5/terminal64.exe");
        assert_eq!(c.mt5_retry_attempts, 4);
        assert_eq!(c.mt5_retry_delay_s, 9.0);
        assert_eq!(c.mt5_restart_after, 0);
        assert_eq!(c.mt5_health_interval_s, 30.0);

        // i najważniejsze: żaden z tych kluczy nie może trafić na listę braków
        assert!(unmapped_keys(&json!({ "mt5_autostart": true })).is_empty());
    }

    #[test]
    fn zerowa_liczba_prob_nie_przechodzi() {
        // bot z otwartymi pozycjami nie ma prawa przestać próbować
        let c = core_from_ui(&json!({ "mt5_retry_attempts": 0, "mt5_retry_delay_s": 0 }));
        assert_eq!(c.mt5_retry_attempts, 1);
        assert_eq!(c.mt5_retry_delay_s, 0.5);
    }

    #[test]
    fn latka_scala_sie_klucz_po_kluczu() {
        let mut doc = json!({ "a": 1, "b": 2 });
        merge_patch(&mut doc, &json!({ "b": 9, "c": 3 }));
        assert_eq!(doc, json!({ "a": 1, "b": 9, "c": 3 }));
    }

    // ============================================================
    //  PRESET → PANEL → SILNIK
    // ============================================================

    /// Konfiguracja celowo ODLEGŁA od domyślnej we wszystkich polach, które
    /// panel trzyma pod inną nazwą. Gdyby tłumaczenie któregoś nie obejmowało,
    /// pole wróciłoby do domyślnego i test to pokaże.
    fn preset_probny() -> Settings {
        let mut s = Settings::default();
        s.zone_offset_mode = ZoneOffsetMode::Directional;
        s.entry_hi_offset = 1.5;
        s.entry_lo_offset = -2.5;
        s.entry_sl_dist_limit = 12.0;
        s.toucher_units = 4;
        s.toucher_tp_index = 2;
        s.toucher_bands = "1,2,3".into();
        s.pending_lifetime = PendingLifetime::Never;
        s.vsl_broker_offset = 0.7;
        s.tp_schedule = TpSchedule::AllRunners;
        s.official_pct = [11.0, 22.0, 33.0, 34.0];
        s.assign_tp_per_position = true;
        s.be_lock_pts = 3.5;
        s.trail_mode = TrailMode::Tiered;
        s.trail_runner_mode = TrailMode::LockPct;
        s.trail_start = 25.0;
        s.trail_gap = 9.0;
        s.smart_sl_mode = SmartSlMode::LadderWithBe;
        s.smart_sl_delay = 1;
        s.harvest_retrace_pct = 40.0;
        s.server_tz_offset_ms = 3 * 3_600_000;
        s.msg_clock_offset_ms = Some(-2 * 3_600_000);
        s.ai_enabled = true;
        s.bank_close_last = !Settings::default().bank_close_last;
        s.bank_from = BankFrom::Best;
        s.bank_rounding = BankRounding::Down;
        s.last_runner = LastRunner::NextTp;
        s.basket_hint_tolerance = 1.75;
        s.pending_resize_s = 42.0;
        s.ppm_for_market = !Settings::default().ppm_for_market;
        s.pending_drop_on_target = !Settings::default().pending_drop_on_target;
        s.smart_sl_floor_be_after_rf = !Settings::default().smart_sl_floor_be_after_rf;
        s.smart_exit = true;
        s.smart_exit_take = 21.0;
        s.smart_exit_giveback = 0.45;
        s.smart_exit_min_peak = 6.5;
        s.smart_exit_drop_speed = 1.25;
        s.smart_exit_speed_window_s = 90.0;
        s.smart_exit_hold_if_pending = 3.5;
        s.smart_exit_min_pendings = 2;
        s.smart_exit_pending_scope = PendingScope::AnyBasket;
        s.smart_exit_pending_min_dist = 0.9;
        // osie Fazy 6 — klucz wspólny z rdzeniem, wartość w słowniku serde;
        // wartości NIE-domyślne, żeby zgubienie któregokolwiek pola było widać
        s.sesja_bramka = SesjaBramka::Oba;
        s.regime_cena = RegimeCena::Obie;
        s.regime_miara = RegimeMiara::Percentyl;
        s.regime_pilnuj_limitow = true;
        s.regime_percentyl = 25.0;
        s.regime_strefa_martwa = 1.5;
        s.regime_okno2_h = 6.0;
        s.regime_zmiennosc_min = 4.0;
        s.regime_zmiennosc_max = 80.0;
        s.regime_gdy_rozerwany = RegimeGdyRozerwany::Miekko;
        s.regime_soft = true;
        s.regime_soft_units_mult = 0.5;
        s.regime_soft_lot_mult = 0.5;
        s.regime_soft_max_positions = 7;
        s.regime_soft_risk_mult = 0.25;
        s.vol_size_mode = VolSizeMode::Target;
        s.vol_size_target = 12.0;
        s.vol_size_min_mult = 0.3;
        s.vol_size_max_mult = 2.0;
        s.vol_size_percentile_okno = 96;
        s.vol_size_odsezonuj = true;
        s.sanity_zone_max = 30.0;
        s.sanity_tp_max = 120.0;
        s.sanity_tp_rosnace = true;
        s.sanity_tp_strona = true;
        s.parser_geometryczny = true;
        s.parser_min_pewnosc = 0.65;
        s.cel_z_przeciwnego = CelZPrzeciwnego::BliższaKrawedz;
        s.cel_z_przeciwnego_zapas = 0.8;
        s.day_gate_od_salda = 350.0;
        s.day_gate_do_salda = 900.0;
        s.no_tp_after_stage = 2;
        s.no_reenter_from_stage = 3;
        s.oae_skip_after_riskfree = true;
        s.reenter_stop_after_riskfree = true;
        s.lot_max_z_salda = 400.0;
        s.trail_atr_mult = 1.5;
        s.trail_adaptive_enabled = true;
        s.trail_adaptive_runners_only = false;
        s.trail_adaptive_window_s = 75.0;
        s.trail_adaptive_min_samples = 11;
        s.trail_adaptive_trend_er = 0.61;
        s.trail_adaptive_reversal_er = 0.37;
        s.trail_adaptive_trend_gap_mult = 1.7;
        s.trail_adaptive_chop_gap_mult = 0.92;
        s.trail_adaptive_reversal_gap_mult = 0.38;
        s.trail_adaptive_fast_vol_s = 15.0;
        s.trail_adaptive_slow_vol_s = 150.0;
        s.trail_adaptive_vol_ratio = 2.1;
        s.trail_adaptive_vol_favorable_mult = 1.35;
        s.trail_adaptive_vol_adverse_mult = 0.55;
        s.trail_adaptive_min_peak = 3.5;
        s.trail_adaptive_min_gap = 1.25;
        s.trail_adaptive_max_gap = 28.0;
        // sekcja M z 18.08 + FAZA 1 poz. 7 — te pola też idą przez most bez
        // słownika, więc literówka w kluczu byłaby niewidoczna bez wartości
        // NIE-domyślnej tutaj
        s.entry_deep_frac_to_sl = 0.4;
        s.risk_free_be_min_profit = 21.0;
        s.runner_max_hold_bez_reguly = true;
        s.limit_kasuje_tylko_nadmiar = true;
        s
    }

    /// REGRESJA: preset z katalogu WBUDOWANEGO W INTERFEJS przychodzi już
    /// w kształcie panelu (`"tiered"`, nie `"Tiered"`). Rozpoznawanie samych
    /// nazw rdzenia cicho WYŁĄCZAŁO zapadkę takiego presetu.
    #[test]
    fn zapadka_przezywa_obie_konwencje_nazw() {
        for (wejscie, oczekiwane) in [
            ("Tiered", "tiered"),
            ("tiered", "tiered"),
            ("LockPct", "lock_pct"),
            ("lock_pct", "lock_pct"),
            ("Atr", "atr"),
            ("atr", "atr"),
            ("Chandelier", "chandelier"),
            ("chandelier", "chandelier"),
        ] {
            let ui = preset_to_ui(&json!({ "trail_mode": wejscie, "trail_runner_mode": wejscie }));
            assert_eq!(ui["trail_mode"], oczekiwane, "trail_mode dla {wejscie}");
            assert_eq!(
                ui["trail_runner_mode"], oczekiwane,
                "trail_runner_mode dla {wejscie}"
            );
            assert_eq!(
                ui["runner_trail"],
                json!(true),
                "zapadka wyłączona dla {wejscie}"
            );
            assert_eq!(core_from_ui(&ui).trail_mode, mode_z_nazwy(oczekiwane));
        }
        // „Off" nadal ma wyłączać
        let ui = preset_to_ui(&json!({ "trail_mode": "Off" }));
        assert_eq!(ui["runner_trail"], json!(false));
        assert_eq!(core_from_ui(&ui).trail_mode, TrailMode::Off);
    }

    fn mode_z_nazwy(n: &str) -> TrailMode {
        match n {
            "gap" => TrailMode::Gap,
            "lock_pct" => TrailMode::LockPct,
            "tiered" => TrailMode::Tiered,
            "atr" => TrailMode::Atr,
            "chandelier" => TrailMode::Chandelier,
            _ => TrailMode::Off,
        }
    }

    /// REGRESJA: cała rodzina `smart_exit_*` przechodziła przez `settings.json`
    /// nietknięta, ale `core_from_ui` jej nie czytało. Preset zmierzony
    /// z mądrym wyjściem handlowałby na żywo bez niego — pod tą samą nazwą.
    #[test]
    fn madre_wyjscie_dociera_z_panelu_do_silnika() {
        let c = core_from_ui(&json!({
            "smart_exit": true,
            "smart_exit_take": 18.0,
            "smart_exit_giveback": 0.4,
            "smart_exit_min_peak": 5.0,
            "smart_exit_drop_speed": 2.0,
            "smart_exit_speed_window_s": 45.0,
            "smart_exit_hold_if_pending": 2.5,
            "smart_exit_min_pendings": 3,
            "smart_exit_pending_scope": "AnyBasket",
            "smart_exit_pending_min_dist": 0.8,
        }));
        assert!(c.smart_exit);
        assert_eq!(c.smart_exit_take, 18.0);
        assert_eq!(c.smart_exit_giveback, 0.4);
        assert_eq!(c.smart_exit_min_peak, 5.0);
        assert_eq!(c.smart_exit_drop_speed, 2.0);
        assert_eq!(c.smart_exit_speed_window_s, 45.0);
        assert_eq!(c.smart_exit_hold_if_pending, 2.5);
        assert_eq!(c.smart_exit_min_pendings, 3);
        assert_eq!(c.smart_exit_pending_scope, PendingScope::AnyBasket);
        assert_eq!(c.smart_exit_pending_min_dist, 0.8);

        // domyślnie WYŁĄCZONE — brak klucza nie może niczego włączyć
        assert!(!core_from_ui(&json!({})).smart_exit);
    }

    /// NAJWAŻNIEJSZY test tego pliku: preset wczytany w panelu ma dawać
    /// DOKŁADNIE tę konfigurację, którą zmierzył backtest.
    ///
    /// Bez `preset_to_ui` ten test przechodził tylko dla pól o zgodnych
    /// nazwach, a `tp_schedule`, `pending_lifetime` czy `trail_start` cicho
    /// wracały do domyślnych — czyli bot na żywo grał inną konfiguracją niż
    /// ta, której wynik pokazywał raport.
    #[test]
    fn preset_przechodzi_przez_panel_bez_strat() {
        let oczekiwane = preset_probny();
        let jako_json = serde_json::to_value(&oczekiwane).unwrap();
        let dla_panelu = preset_to_ui(&jako_json);
        let wynik = core_from_ui(&dla_panelu);

        // `stops_level` i lot mają własne drogi (serwer brokera / SetLot),
        // więc porównujemy resztę.
        let mut a = wynik.clone();
        let mut b = oczekiwane.clone();
        a.stops_level = 0.0;
        b.stops_level = 0.0;
        a.lot_mode_percent = false;
        b.lot_mode_percent = false;
        a.lot_fixed = 0.0;
        b.lot_fixed = 0.0;
        a.lot_percent = 0.0;
        b.lot_percent = 0.0;
        assert_eq!(a, b, "preset zgubił ustawienia po drodze przez panel");
    }

    /// Dokument, który mimo wszystko niesie klucze w nazewnictwie SILNIKA,
    /// ma dawać tę samą konfigurację, co przejście przez `preset_to_ui`.
    ///
    /// Historia: przez długi czas było odwrotnie i test w tym miejscu
    /// SPRAWDZAŁ, że surowy preset gubi ustawienia (`tp_schedule` wracało do
    /// `Ladder`, `trail_start` do domyślnej). Dowodziło to, że problem jest
    /// realny — ale zostawiało otwartą dziurę: `settings.json` zapisany bez
    /// tłumaczenia dawał bota grającego inną konfiguracją niż zmierzona.
    /// Dokładnie to zastano 30.07.2026 w `PACKAGE/settings.json`.
    /// Dziś `core_from_ui` sam wykrywa taki dokument i go tłumaczy.
    #[test]
    fn dokument_w_nazewnictwie_silnika_tez_dziala() {
        let p = serde_json::to_value(preset_probny()).unwrap();
        let surowo = core_from_ui(&p);
        let przez_panel = core_from_ui(&preset_to_ui(&p));

        assert_eq!(surowo.tp_schedule, przez_panel.tp_schedule);
        assert_eq!(surowo.pending_lifetime, przez_panel.pending_lifetime);
        assert_eq!(surowo.trail_start, przez_panel.trail_start);
        assert_eq!(surowo.zone_offset_mode, przez_panel.zone_offset_mode);
        assert_eq!(surowo.last_runner, przez_panel.last_runner);
        assert_eq!(surowo, przez_panel, "obie drogi muszą dać ten sam silnik");
    }

    /// Pola o WSPÓLNEJ nazwie klucza, ale różnym SŁOWNIKU WARTOŚCI.
    ///
    /// Audyt `unmapped_keys` pokazywał je jako „podpięte", bo klucz się zgadza
    /// — a mimo to nie działały, bo panel zapisuje `"counter"`, preset niesie
    /// `"CounterMa"`, i nierozpoznana wartość po cichu wracała do domyślnej.
    /// Zmierzony koszt na ULTRA-X3 (okno czerwiec–lipiec, tryb dzienny):
    /// 1936 $ → −7 $.
    #[test]
    fn slownik_wartosci_dziala_w_obu_konwencjach() {
        for (klucz, rdzen, panel) in [
            ("out_at_entry_mode", "CloseLosersOnly", "losers"),
            ("sl_hit_mode", "VerifyByPrice", "verify"),
            ("regime_filter", "CounterMa", "counter"),
            ("side_filter", "SellOnly", "sell"),
            ("dd_guard_scope", "Lifetime", "lifetime"),
            ("risk_free_runner_target", "NextTp", "next"),
            ("risk_free_mode", "MoveSlToBeOnly", "all_runners"),
        ] {
            let a = core_from_ui(&json!({ klucz: rdzen }));
            let b = core_from_ui(&json!({ klucz: panel }));
            assert_eq!(
                a, b,
                "`{klucz}`: nazwa rdzenia `{rdzen}` musi znaczyć to samo, co `{panel}`"
            );
            assert_ne!(
                a,
                Settings::default(),
                "`{klucz}`: test nic nie sprawdza, bo wartość równa się domyślnej"
            );
        }
    }

    /// `trail_runner_mode = Off` to JEDYNY sposób na wyłączenie rodziny
    /// runnerowej — a domyślną wartością rdzenia jest `Tiered`, więc
    /// nierozpoznane „off" WŁĄCZAŁO to, co miało wyłączyć. Zmierzone −923 $
    /// na ULTRA-X3 (okno czerwiec–lipiec, tryb dzienny).
    #[test]
    fn wylaczenie_runnera_nie_moze_go_wlaczac() {
        assert_eq!(Settings::default().trail_runner_mode, TrailMode::Tiered);
        for v in ["off", "Off"] {
            assert_eq!(
                core_from_ui(&json!({ "trail_runner_mode": v })).trail_runner_mode,
                TrailMode::Off,
                "`{v}` musi WYŁĄCZAĆ rodzinę runnerową"
            );
        }
        let p = json!({ "trail_runner_mode": "Off" });
        assert_eq!(
            core_from_ui(&preset_to_ui(&p)).trail_runner_mode,
            TrailMode::Off,
            "także przez pełną drogę presetu"
        );
    }

    /// Sito na przyszłość: KAŻDE pole silnika musi mieć w panelu swój
    /// odpowiednik, albo być na krótkiej liście świadomych wyjątków.
    ///
    /// Dopisanie do `Settings` nowego pola o nazwie, której panel nie zna,
    /// zapali ten test — zamiast po cichu wyciąć ustawienie z presetu.
    #[test]
    fn zaden_klucz_presetu_nie_ginie_po_tlumaczeniu() {
        // Lista wyjątków jest PUSTA od 04.08.2026. Trzy pola lota
        // (`lot_mode_percent`, `lot_fixed`, `lot_percent`) miały tu zwolnienie
        // z uzasadnieniem „mają własny sterownik `Command::SetLot`" — a skutek
        // był taki, że zapis dowolnego pola presetu z panelu cofał jego lot do
        // domyślnego 0,01. Jeśli ktoś chce tu dopisać nazwę, musi najpierw
        // odpowiedzieć, co się stanie z tym polem przy edycji presetu.
        const WYJATKI: &[&str] = &[];

        let pelny = serde_json::to_value(Settings::default()).unwrap();
        let po = preset_to_ui(&pelny);
        let braki: Vec<String> = unmapped_keys(&po)
            .into_iter()
            .filter(|k| !WYJATKI.contains(&k.as_str()))
            .collect();
        assert!(
            braki.is_empty(),
            "te pola presetu nie docierają do silnika przez panel: {braki:?}"
        );
    }

    /// Przełączniki z audytu Fable (Z-3, Z-5…Z-10) plus weto odpowiedzi
    /// z Pakietu F muszą przejść przez panel bez zmiany znaczenia — a BRAK
    /// klucza musi zostawić DOMYŚLNĄ WARTOŚĆ SILNIKA nietkniętą.
    /// To jest ta sama klasa błędu, przez którą przepadło
    /// `drop_unplaceable_levels`.
    ///
    /// Do 18.08.2026 test pisał „brak klucza musi zostawić `false`", bo
    /// wszystkie te osie miały wtedy domyślne `false`. Od naprawy semantyki
    /// wypełnienia `sync_only_live_levels` jest domyślnie `true`, więc
    /// właściwym odniesieniem jest `Settings::default()`, a nie stała.
    /// Zapisanie odniesienia na sztywno zamieniłoby test „panel nie gubi
    /// klucza" w test „domyślna nigdy się nie zmieni".
    #[test]
    fn przelaczniki_z_audytu_dociera_do_silnika() {
        const KLUCZE: &[&str] = &[
            "sl_edit_reaches_pendings",
            "honor_stop_orders",
            "hint_veto",
            "reply_veto",
            "sync_only_live_levels",
            "tp_correction_to_broker",
            "runner_max_hold_rule_only",
            "spp_arms_runner_clock",
            "tp_hit_match_level",
            "tp_unindexed_pips_require_price",
            "tp_price_only_strict",
            "reply_graph_transitive",
            "parser_luz_interpunkcyjny",
            "profit_update_telemetry_only",
            "basket_realized_broker_only",
            "confirmed_exit_retry",
            "rearm_keep_empty_alive",
            "spp_blocks_rearm_when_flat",
        ];
        let odczyt = |c: &Settings, k: &str| -> bool {
            match k {
                "sl_edit_reaches_pendings" => c.sl_edit_reaches_pendings,
                "honor_stop_orders" => c.honor_stop_orders,
                "hint_veto" => c.hint_veto,
                "reply_veto" => c.reply_veto,
                "sync_only_live_levels" => c.sync_only_live_levels,
                "tp_correction_to_broker" => c.tp_correction_to_broker,
                "runner_max_hold_rule_only" => c.runner_max_hold_rule_only,
                "spp_arms_runner_clock" => c.spp_arms_runner_clock,
                "tp_hit_match_level" => c.tp_hit_match_level,
                "tp_unindexed_pips_require_price" => c.tp_unindexed_pips_require_price,
                "tp_price_only_strict" => c.tp_price_only_strict,
                "reply_graph_transitive" => c.reply_graph_transitive,
                "parser_luz_interpunkcyjny" => c.parser_luz_interpunkcyjny,
                "profit_update_telemetry_only" => c.profit_update_telemetry_only,
                "basket_realized_broker_only" => c.basket_realized_broker_only,
                "confirmed_exit_retry" => c.confirmed_exit_retry,
                "rearm_keep_empty_alive" => c.rearm_keep_empty_alive,
                "spp_blocks_rearm_when_flat" => c.spp_blocks_rearm_when_flat,
                _ => unreachable!(),
            }
        };
        for k in KLUCZE {
            let wl = core_from_ui(&json!({ *k: true }));
            assert!(odczyt(&wl, k), "klucz {k} nie dociera do silnika");
            let wyl = core_from_ui(&json!({ *k: false }));
            assert!(
                !odczyt(&wyl, k),
                "klucz {k} nie dociera do silnika przy `false`"
            );
            let brak = core_from_ui(&json!({ "entry_units": 3 }));
            assert_eq!(
                odczyt(&brak, k),
                odczyt(&Settings::default(), k),
                "brak klucza {k} musi zostawić domyślną wartość silnika"
            );
        }
        let rf = core_from_ui(&json!({ "rf_level_sanity_max_usd": 20.0 }));
        assert_eq!(
            rf.rf_level_sanity_max_usd, 20.0,
            "próg sanity RF musi dotrzeć z panelu"
        );
        let rf_off = core_from_ui(&json!({ "rf_level_sanity_max_usd": -5.0 }));
        assert_eq!(
            rf_off.rf_level_sanity_max_usd, 0.0,
            "ujemny próg ma wyłączać oś"
        );
    }

    #[test]
    fn basket_realized_broker_only_roundtrip_ui_preset_false_default() {
        assert!(!Settings::default().basket_realized_broker_only);
        assert!(!core_from_ui(&json!({})).basket_realized_broker_only);
        for enabled in [false, true] {
            let doc = json!({"basket_realized_broker_only": enabled});
            assert!(unmapped_keys(&doc).is_empty());
            let config = core_from_ui(&doc);
            assert_eq!(config.basket_realized_broker_only, enabled);
            let serialized = serde_json::to_value(&config).unwrap();
            assert_eq!(serialized["basket_realized_broker_only"], json!(enabled));
            let loaded: Settings = serde_json::from_value(serialized.clone()).unwrap();
            assert_eq!(loaded.basket_realized_broker_only, enabled);
            assert_eq!(
                core_from_ui(&serialized).basket_realized_broker_only,
                enabled
            );
        }
    }

    #[test]
    fn confirmed_exit_retry_roundtrip_ui_preset_false_default() {
        assert!(!Settings::default().confirmed_exit_retry);
        assert!(!core_from_ui(&json!({})).confirmed_exit_retry);
        for enabled in [false, true] {
            let doc = json!({"confirmed_exit_retry": enabled});
            assert!(unmapped_keys(&doc).is_empty());
            let config = core_from_ui(&doc);
            assert_eq!(config.confirmed_exit_retry, enabled);
            let serialized = serde_json::to_value(&config).unwrap();
            assert_eq!(serialized["confirmed_exit_retry"], json!(enabled));
            let loaded: Settings = serde_json::from_value(serialized.clone()).unwrap();
            assert_eq!(loaded.confirmed_exit_retry, enabled);
            assert_eq!(core_from_ui(&serialized).confirmed_exit_retry, enabled);
        }
    }

    #[test]
    fn preset_niesie_wielkosc_pozycji() {
        let mut s = Settings::default();
        s.lot_mode_percent = true;
        s.lot_percent = 0.5;
        s.lot_fixed = 0.02;
        let l = preset_lot(&serde_json::to_value(&s).unwrap()).expect("preset ma pola lota");
        assert_eq!(l.mode, "percent");
        assert_eq!(l.percent, 0.5);
        assert_eq!(l.fixed, 0.02);

        // preset bez pól lota nie ma prawa nadpisać wyboru z panelu
        assert!(preset_lot(&json!({ "entry_units": 3 })).is_none());
    }

    #[test]
    fn rozklad_wejscia_rynkowego_dociera_do_silnika() {
        // Pole dołożone 31.07.2026. Bez wpisu w `core_from_ui` panel gubiłby
        // je po cichu — tak właśnie przepadło `drop_unplaceable_levels`.
        for (wpis, oczekiwany) in [
            ("Single", MarketEntryMode::Single),
            ("single", MarketEntryMode::Single),
            ("Laddered", MarketEntryMode::Laddered),
            ("laddered", MarketEntryMode::Laddered),
            ("GridAtOnce", MarketEntryMode::GridAtOnce),
            ("cokolwiek", MarketEntryMode::GridAtOnce),
        ] {
            let c = core_from_ui(&json!({ "market_entry_mode": wpis }));
            assert_eq!(c.market_entry_mode, oczekiwany, "wpis {wpis:?}");
        }
        // BRAK klucza = zachowanie sprzed rozdzielenia pól. Preset wydany
        // przed tą zmianą musi dostać dokładnie to, co mierzył backtest.
        let c = core_from_ui(&json!({ "entry_units": 3 }));
        assert_eq!(c.market_entry_mode, MarketEntryMode::GridAtOnce);
    }

    /// Poziom stopu od sygnalisty ma własny przełącznik i musi przejść przez
    /// panel bez zmiany znaczenia. Nierozpoznana wartość spada na `Off`,
    /// czyli NIE zmienia zachowania handlowego (odwrotnie niż `risk_free_mode`,
    /// gdzie gałąź `_` kosztowała −976 $ — patrz `REGULY_AUDYT.md` §2.7).
    #[test]
    fn poziom_stopu_z_spp_dociera_do_silnika() {
        for (wpis, oczekiwany) in [
            ("Stop", SppSlMode::Stop),
            ("set_sl", SppSlMode::Stop),
            ("OnlyIfBetter", SppSlMode::OnlyIfBetter),
            ("RunnersOnly", SppSlMode::RunnersOnly),
            ("RunnersOnlyIfBetter", SppSlMode::RunnersOnlyIfBetter),
            ("BankersOnly", SppSlMode::BankersOnly),
            ("cokolwiek", SppSlMode::Off),
        ] {
            assert_eq!(
                core_from_ui(&json!({ "spp_sl_mode": wpis })).spp_sl_mode,
                oczekiwany,
                "wpis {wpis:?}"
            );
        }
        // BRAK klucza = zachowanie sprzed 31.07.2026
        assert_eq!(
            core_from_ui(&json!({ "entry_units": 3 })).spp_sl_mode,
            SppSlMode::Off
        );
        // i pełna droga presetu, z buforem
        let p = json!({ "spp_sl_mode": "RunnersOnly", "spp_sl_pad": 1.5 });
        let c = core_from_ui(&preset_to_ui(&p));
        assert_eq!(c.spp_sl_mode, SppSlMode::RunnersOnly);
        assert_eq!(c.spp_sl_pad, 1.5);
    }

    /// PACKAGE MUSI STARTOWAĆ NA TYM PRESECIE, KTÓRY DEKLARUJE — dowód, nie
    /// deklaracja.
    ///
    /// `settings.json` zapisuje panel, a pliki presetów pisze backtest, i te
    /// dwa źródła używają RÓŻNEJ pisowni enumów: panel `"counter"`, preset
    /// `"CounterMa"`. Mapowanie zna obie konwencje, ale to trzeba sprawdzać,
    /// a nie zakładać — nierozpoznana wartość spada po cichu na wariant
    /// domyślny i bot gra czymś, czego nikt nie wybrał.
    ///
    /// Zmierzone 31.07.2026: przestawiając PACKAGE z ULTRA-X3 na HYPER-2
    /// trzeba było przepisać 47 pól, w tym siedem enumów właśnie tej klasy
    /// (`risk_free_mode`, `out_at_entry_mode`, `sl_hit_mode`, `trail_mode`,
    /// `side_filter`, `regime_filter`, `dd_guard_scope`).
    ///
    /// Test NIE ZNA nazwy czempiona — czyta ją z `presetId` i porównuje rdzeń
    /// zbudowany dwiema drogami. Dzięki temu przeniesienie korony (np. na
    /// HYPER-X1 01.08.2026) nie zapala go bez powodu, ale podmiana samego
    /// `presetId` bez przepisania pól — owszem.
    #[test]
    fn package_startuje_na_presecie_ktory_deklaruje() {
        let baza = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../PACKAGE");
        let Ok(sj) = std::fs::read_to_string(baza.join("settings.json")) else {
            // Katalog PACKAGE nie jest częścią repozytorium silnika; brak
            // pliku to nie awaria testu, tylko inne środowisko.
            return;
        };
        let doc: serde_json::Value = serde_json::from_str(&sj).unwrap();
        let nazwa = doc
            .get("presetId")
            .and_then(|v| v.as_str())
            .expect("settings.json bez presetId — bot nie wie, co gra");
        let Ok(pj) = std::fs::read_to_string(baza.join(format!("presets/{nazwa}.json"))) else {
            panic!("presetId mówi \"{nazwa}\", ale takiego pliku presetu NIE MA");
        };

        let z_panelu = core_from_ui(doc.get("settings").unwrap());
        // UWAGA: NIE `preset_to_ui`. Plik presetu trzyma pola w zagnieżdżonej
        // sekcji `settings`, a `preset_to_ui` oczekuje innego kształtu i po
        // cichu zwróciłby same wartości domyślne — tak właśnie ten test
        // zapalił się za pierwszym razem. Obie strony idą tą samą drogą.
        let preset: serde_json::Value = serde_json::from_str(&pj).unwrap();
        let sekcja = preset.get("settings").unwrap_or(&preset);
        let z_presetu = core_from_ui(sekcja);

        // Pola, które ustawia UŻYTKOWNIK w panelu, a preset ich nie dyktuje:
        // progi DD (wyzerowane w presetach od zawsze) oraz — od 04.08 —
        // KREDYT BONUSOWY. Kredyt to pole RACHUNKU (`POLA_RACHUNKU`): bonus
        // daje broker kontu, nie strategii, więc `odlicz_kredyt: true`
        // w settings.json przy `false` w presecie NIE jest rozjazdem
        // konfiguracji handlu — jest decyzją właściciela konta. Bez tej
        // neutralizacji włączenie kredytu w panelu wywracałoby bramkę
        // wydania, choć rdzeń HANDLOWY zgadza się co do pola.
        let recznie = |c: &mut conduit_core::Settings| {
            c.max_dd_pct = 0.0;
            c.max_dd_usd = 0.0;
            c.odlicz_kredyt = false;
            c.credit_balance_separate = false;
            c.kredyt_reczny = 0.0;
            // Straż ekspozycji — od 24.08 pole RACHUNKU (`POLA_RACHUNKU`,
            // uzasadnienie przy wpisie): bezpiecznik ostatniej linii liczy
            // procent od wspólnego equity konta, więc 80 w settings.json przy
            // 0 w NEWALPHA-1 to decyzja właściciela konta, nie rozjazd
            // strategii. Preset wolno mieć ostrzejszy własnymi pułapami —
            // nie luźniejszy od konta.
            c.expo_cap_pct = 0.0;
            for field in ["expo_cap_close", "expo_cap_ml_pct"] {
                assert!(
                    conduit_core::wielosilnik::POLA_RACHUNKU.contains(&field),
                    "test may neutralize only an explicitly classified account override"
                );
            }
            c.expo_cap_close = false;
            c.expo_cap_ml_pct = 0.0;
        };
        let (mut a, mut b) = (z_panelu, z_presetu);
        recznie(&mut a);
        recznie(&mut b);

        let av = serde_json::to_value(&a).unwrap();
        let bv = serde_json::to_value(&b).unwrap();
        let differences: Vec<_> = av.as_object().unwrap().iter()
            .filter(|(key, value)| bv.get(*key) != Some(*value))
            .map(|(key, value)| serde_json::json!({"field": key, "panel": value, "preset": bv.get(key)}))
            .collect();
        assert!(differences.is_empty(),
            "settings.json w PACKAGE nie daje tego samego rdzenia co {nazwa}.json; różniące pola:\n{}",
            serde_json::to_string_pretty(&differences).unwrap());
    }

    /// SKRYPT KONTROLNY z planu napraw (FAZA 0, poz. 1): KAŻDY preset z dysku
    /// musi dać przez drogę panelu (`preset_to_ui` → `core_from_ui`) DOKŁADNIE
    /// ten rdzeń, który `btp` czyta z pliku serde wprost.
    ///
    /// To jest sito WARTOŚCI, nie kluczy: `zaden_klucz_presetu_nie_ginie…`
    /// widzi tylko NAZWY, więc pole wpisane do MAPPED bez odczytu w
    /// `core_from_ui` przeszłoby tamto sito, a preset i tak by je gubił —
    /// dokładnie tą szczeliną zniknęło 37 pól Fazy 6 (i wcześniej
    /// `smart_sl_delay = 2`, zgnieciony przełącznikiem do 1).
    #[test]
    fn kazdy_preset_z_dysku_przechodzi_przez_panel_bez_diff() {
        let katalog =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../PACKAGE/presets");
        let Ok(wpisy) = std::fs::read_dir(&katalog) else {
            // Katalog PACKAGE nie jest częścią repozytorium silnika; brak
            // katalogu to nie awaria testu, tylko inne środowisko.
            return;
        };
        let mut sprawdzone = 0usize;
        let mut rozjazdy = Vec::new();
        for wpis in wpisy.flatten() {
            let sciezka = wpis.path();
            if sciezka.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let nazwa = sciezka.file_stem().unwrap().to_string_lossy().to_string();
            let tekst = std::fs::read_to_string(&sciezka).unwrap();
            let plik: serde_json::Value =
                serde_json::from_str(&tekst).unwrap_or_else(|e| panic!("{nazwa}: zły JSON: {e}"));
            let sekcja = plik.get("settings").unwrap_or(&plik);
            // droga btp: serde wprost do `Settings` (nieznane klucze pomijane)
            let z_serde: Settings = serde_json::from_value(sekcja.clone())
                .unwrap_or_else(|e| panic!("{nazwa}: serde nie czyta presetu: {e}"));
            // droga panelu: tłumaczenie kształtu + odczyt kluczy panelu
            let z_panelu = core_from_ui(&preset_to_ui(sekcja));
            if z_serde != z_panelu {
                // diff po liniach zrzutu Debug — wskazuje POLE, nie tylko fakt
                let (a, b) = (format!("{z_serde:#?}"), format!("{z_panelu:#?}"));
                let pola: Vec<String> = a
                    .lines()
                    .zip(b.lines())
                    .filter(|(x, y)| x != y)
                    .map(|(x, y)| format!("btp: {} | panel: {}", x.trim(), y.trim()))
                    .collect();
                rozjazdy.push(format!("{nazwa}: {pola:?}"));
            }
            sprawdzone += 1;
        }
        assert!(
            sprawdzone > 0,
            "katalog presetów pusty: {}",
            katalog.display()
        );
        assert!(
            rozjazdy.is_empty(),
            "presety gubią wartości na drodze przez panel:\n{}",
            rozjazdy.join("\n")
        );
    }

    /// Osie Fazy 6 muszą docierać z dokumentu do silnika — enumy w OBU
    /// pisowniach, liczby wprost, a BRAK klucza zostawia domyślną rdzenia.
    #[test]
    fn osie_fazy6_docieraja_z_panelu_do_silnika() {
        // enumy: nazwa serde i małe litery znaczą to samo
        for (wejscie, oczekiwane) in [
            ("Wypelnienie", SesjaBramka::Wypelnienie),
            ("oba", SesjaBramka::Oba),
        ] {
            assert_eq!(
                core_from_ui(&json!({ "sesja_bramka": wejscie })).sesja_bramka,
                oczekiwane,
                "sesja_bramka {wejscie:?}"
            );
        }
        assert_eq!(
            core_from_ui(&json!({ "regime_gdy_rozerwany": "Soft" })).regime_gdy_rozerwany,
            RegimeGdyRozerwany::Miekko,
            "stary alias serde `Soft` musi znaczyć Miekko"
        );
        assert_eq!(
            core_from_ui(&json!({ "regime_range_mute_mode": "KrotkieOkno" })).regime_gdy_rozerwany,
            RegimeGdyRozerwany::KrotkieOkno,
            "stara nazwa POLA (alias serde) też musi być czytana"
        );
        assert_eq!(
            core_from_ui(&json!({ "cel_z_przeciwnego": "BlizszaKrawedz" })).cel_z_przeciwnego,
            CelZPrzeciwnego::BliższaKrawedz,
            "pisownia bez ogonka nie może wyłączyć osi"
        );
        // nierozpoznany wariant NIE przestawia bramki (wzorzec `lot_base`)
        assert_eq!(
            core_from_ui(&json!({ "vol_size_mode": "cokolwiek" })).vol_size_mode,
            VolSizeMode::Off
        );
        // liczby i przełączniki
        let c = core_from_ui(&json!({
            "regime_pilnuj_limitow": true,
            "regime_percentyl": 25.0,
            "no_tp_after_stage": 2,
            "day_gate_od_salda": 350.0,
            "lot_max_z_salda": 400.0,
            "trail_atr_mult": 1.5,
            "parser_geometryczny": true,
            "parser_min_pewnosc": 0.65,
        }));
        assert!(c.regime_pilnuj_limitow);
        assert_eq!(c.regime_percentyl, 25.0);
        assert_eq!(c.no_tp_after_stage, 2);
        assert_eq!(c.day_gate_od_salda, 350.0);
        assert_eq!(c.lot_max_z_salda, 400.0);
        assert_eq!(c.trail_atr_mult, 1.5);
        assert!(c.parser_geometryczny);
        assert_eq!(c.parser_min_pewnosc, 0.65);
        // brak klucza = domyślne rdzenia (kontrakt zera nietknięty)
        let brak = core_from_ui(&json!({ "entry_units": 3 }));
        assert_eq!(brak.sesja_bramka, Settings::default().sesja_bramka);
        assert_eq!(brak.vol_size_mode, Settings::default().vol_size_mode);
        assert_eq!(
            brak.no_tp_after_stage,
            Settings::default().no_tp_after_stage
        );
    }

    /// Każdy preset wymieniony w rankingu panelu MUSI istnieć na dysku.
    ///
    /// Ranking (`src/data/presets.ts`) decyduje o kolejności i o tym, kto nosi
    /// koronę. Wpis wskazujący na nieistniejący plik daje pustą pozycję
    /// w interfejsie i koronę zawieszoną w próżni — objaw, którego nikt nie
    /// zauważy, dopóki nie kliknie.
    #[test]
    fn kazdy_preset_z_rankingu_istnieje_na_dysku() {
        let korzen = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let Ok(ts) = std::fs::read_to_string(korzen.join("src/data/presets.ts")) else {
            return;
        };
        let Some(blok) = ts
            .split_once("export const RANKING_PRESETOW")
            .and_then(|(_, r)| r.split_once("];"))
            .map(|(b, _)| b)
        else {
            panic!("nie znalazłem RANKING_PRESETOW w presets.ts");
        };

        let mut brakuje = Vec::new();
        for linia in blok.lines() {
            let bez_kom = linia.split("//").next().unwrap_or("").trim();
            let Some(nazwa) = bez_kom
                .strip_prefix('"')
                .and_then(|r| r.split('"').next())
                .filter(|n| !n.is_empty())
            else {
                continue;
            };
            if !korzen
                .join(format!("PACKAGE/presets/{nazwa}.json"))
                .exists()
            {
                brakuje.push(nazwa.to_string());
            }
        }
        assert!(
            brakuje.is_empty(),
            "ranking wymienia presety, których nie ma na dysku: {brakuje:?}"
        );
    }

    /// EA-CORE: jedenaście pól szkieletu przechodzi przez panel BEZ ZMIANY
    /// WARTOŚCI, a BRAK klucza zostawia domyślną (czyli warstwę wyłączoną).
    ///
    /// Test istnieje, bo sito `zaden_klucz_presetu_nie_ginie_po_tlumaczeniu`
    /// sprawdza tylko, czy klucz jest ZNANY — nie, czy jego wartość dojechała.
    /// Rozjazd tej drugiej klasy zaliczyliśmy już trzy razy („panel zeruje
    /// pole przy zapisie presetu") i za każdym razem był cichy.
    #[test]
    fn market_hybrid_przechodzi_przez_panel_bez_zmiany() {
        let mut z = Settings::default();
        z.market_hybrid_now_units = 3;
        z.market_hybrid_pending_units = 5;
        z.market_hybrid_lot_mult = 0.65;
        z.market_hybrid_max_chase_usd = 0.8;
        z.market_hybrid_tp_stage = 2;
        z.market_unfilled_cancel_stage = 1;
        let po = core_from_ui(&preset_to_ui(&serde_json::to_value(&z).unwrap()));
        assert_eq!(po.market_hybrid_now_units, 3);
        assert_eq!(po.market_hybrid_pending_units, 5);
        assert_eq!(po.market_hybrid_lot_mult, 0.65);
        assert_eq!(po.market_hybrid_max_chase_usd, 0.8);
        assert_eq!(po.market_hybrid_tp_stage, 2);
        assert_eq!(po.market_unfilled_cancel_stage, 1);
    }

    #[test]
    fn ea_core_przechodzi_przez_panel_bez_zmiany_wartosci() {
        // 1. BRAK KLUCZA = domyślna wartość rdzenia, czyli warstwa wyłączona.
        let pusty = core_from_ui(&json!({}));
        assert!(!pusty.ea_enabled, "brak klucza włączył warstwę EA");
        assert_eq!(pusty.ea_tick_s, 0.0);
        assert_eq!(pusty.ea_state_src, EaStateSrc::FloatR);
        assert_eq!(pusty.ea_state_ratchet, EaRatchet::NieLuzujWKoszyku);
        assert!(pusty.ea_state_journal);
        assert!(!pusty.ea_dozor_sl);

        // 2. Wartości nietypowe przechodzą co do bitu — w obu konwencjach
        //    zapisu enumów (rdzeniowej i małymi literami).
        for (src, ratchet) in [
            ("FloatPctEquity", "Swobodny"),
            ("float_pct_equity", "swobodny"),
        ] {
            let doc = json!({
                "ea_enabled": true, "ea_tick_s": 2.5,
                "ea_state_src": src,
                "ea_defense_enter": 1.5, "ea_defense_exit": 0.75,
                "ea_offense_enter": 3.0, "ea_offense_exit": 1.25,
                "ea_state_dwell_s": 90.0,
                "ea_state_ratchet": ratchet,
                "ea_state_journal": false, "ea_dozor_sl": true,
            });
            let c = core_from_ui(&doc);
            assert!(c.ea_enabled);
            assert_eq!(c.ea_tick_s, 2.5);
            assert_eq!(c.ea_state_src, EaStateSrc::FloatPctEquity, "zapis `{src}`");
            assert_eq!(c.ea_defense_enter, 1.5);
            assert_eq!(c.ea_defense_exit, 0.75);
            assert_eq!(c.ea_offense_enter, 3.0);
            assert_eq!(c.ea_offense_exit, 1.25);
            assert_eq!(c.ea_state_dwell_s, 90.0);
            assert_eq!(c.ea_state_ratchet, EaRatchet::Swobodny, "zapis `{ratchet}`");
            assert!(!c.ea_state_journal);
            assert!(c.ea_dozor_sl);
        }

        // 3. PEŁNA DROGA PRESETU: rdzeń → dokument panelu → rdzeń.
        //    To jest ta ścieżka, którą przechodzi każdy zapis z Ustawień.
        let mut zrodlo = Settings::default();
        zrodlo.ea_enabled = true;
        zrodlo.ea_tick_s = 5.0;
        zrodlo.ea_defense_enter = 2.0;
        zrodlo.ea_defense_exit = 1.0;
        zrodlo.ea_state_dwell_s = 120.0;
        zrodlo.ea_dozor_sl = true;
        let po = core_from_ui(&preset_to_ui(&serde_json::to_value(&zrodlo).unwrap()));
        assert!(po.ea_enabled, "zapis przez panel wyłączył warstwę");
        assert_eq!(
            po.ea_tick_s, 5.0,
            "zapis przez panel zgubił kadencję zegara"
        );
        assert_eq!(po.ea_defense_enter, 2.0);
        assert_eq!(po.ea_defense_exit, 1.0);
        assert_eq!(po.ea_state_dwell_s, 120.0);
        assert!(po.ea_dozor_sl, "zapis przez panel zgasił dozór SL");
    }

    /// RODZINA A PRZEŻYWA OBIEG PANELU — osie A1–A4 (25.08.2026).
    ///
    /// # Dlaczego ten test istnieje osobno
    ///
    /// `unmapped_keys` sprawdza klucze OBECNE W DOKUMENCIE, więc dopóki żaden
    /// preset nie niósł pól rodziny A, alarm milczał — a pola i tak ginęły,
    /// bo `core_from_ui` startuje od `Settings::default()` i przypisuje
    /// wyłącznie to, co zna. Backtest (serde wprost do `Settings`) je
    /// honorował, bot na żywo zerował. Preset zmierzony jako zyskowny grałby
    /// więc BEZ warstwy, która dała mu ten wynik.
    ///
    /// Test idzie od strony `Settings`, a nie dokumentu: ustawia każde pole
    /// na wartość RÓŻNĄ od domyślnej i sprawdza, że wraca po pełnym obiegu.
    /// Wartość różna od domyślnej jest tu warunkiem sensu — pole zerowane
    /// przez brak mapowania i pole o wartości domyślnej wyglądają identycznie.
    #[test]
    fn rodzina_a_przezywa_obieg_panelu() {
        let mut zrodlo = Settings::default();
        zrodlo.ea_enabled = true;
        zrodlo.ea_lot_z_wolnego_marginesu = 40.0;
        zrodlo.ea_stop_dokladek_przy_stracie = 25.0;
        zrodlo.ea_stop_dokladek_powrot = 10.0;
        zrodlo.ea_redukcja_przy_zageszczeniu = 0.15;
        zrodlo.ea_zageszczenie_podloga = 0.4;
        zrodlo.ea_stan_dnia = EaStanDnia::TylkoInkaso;
        zrodlo.ea_stan_dnia_prog_sl = 3;
        zrodlo.ea_stan_dnia_jednostki_mult = 0.5;

        // Każda z tych wartości MUSI różnić się od domyślnej, inaczej test
        // przechodziłby także przy zerwanym mapowaniu.
        let dom = Settings::default();
        assert_ne!(
            zrodlo.ea_lot_z_wolnego_marginesu,
            dom.ea_lot_z_wolnego_marginesu
        );
        assert_ne!(
            zrodlo.ea_stop_dokladek_przy_stracie,
            dom.ea_stop_dokladek_przy_stracie
        );
        assert_ne!(zrodlo.ea_stop_dokladek_powrot, dom.ea_stop_dokladek_powrot);
        assert_ne!(
            zrodlo.ea_redukcja_przy_zageszczeniu,
            dom.ea_redukcja_przy_zageszczeniu
        );
        assert_ne!(zrodlo.ea_zageszczenie_podloga, dom.ea_zageszczenie_podloga);
        assert_ne!(zrodlo.ea_stan_dnia, dom.ea_stan_dnia);
        assert_ne!(zrodlo.ea_stan_dnia_prog_sl, dom.ea_stan_dnia_prog_sl);
        assert_ne!(
            zrodlo.ea_stan_dnia_jednostki_mult,
            dom.ea_stan_dnia_jednostki_mult
        );

        let doc = serde_json::to_value(&zrodlo).unwrap();
        let po = core_from_ui(&preset_to_ui(&doc));

        assert_eq!(
            po.ea_lot_z_wolnego_marginesu, 40.0,
            "A1 zgubiony w obiegu panelu"
        );
        assert_eq!(po.ea_stop_dokladek_przy_stracie, 25.0, "A2 (próg) zgubiony");
        assert_eq!(po.ea_stop_dokladek_powrot, 10.0, "A2 (powrót) zgubiony");
        assert_eq!(po.ea_redukcja_przy_zageszczeniu, 0.15, "A3 zgubiony");
        assert_eq!(po.ea_zageszczenie_podloga, 0.4, "A3 (podłoga) zgubiona");
        assert_eq!(
            po.ea_stan_dnia,
            EaStanDnia::TylkoInkaso,
            "A4 (tryb) zgubiony"
        );
        assert_eq!(po.ea_stan_dnia_prog_sl, 3, "A4 (próg SL) zgubiony");
        assert_eq!(po.ea_stan_dnia_jednostki_mult, 0.5, "A4 (mnożnik) zgubiony");

        // Dokument niosący te klucze nie może zapalać fałszywego alarmu.
        let brakujace = unmapped_keys(&preset_to_ui(&doc));
        for k in [
            "ea_lot_z_wolnego_marginesu",
            "ea_stop_dokladek_przy_stracie",
            "ea_stop_dokladek_powrot",
            "ea_redukcja_przy_zageszczeniu",
            "ea_zageszczenie_podloga",
            "ea_stan_dnia",
            "ea_stan_dnia_prog_sl",
            "ea_stan_dnia_jednostki_mult",
        ] {
            assert!(
                !brakujace.contains(&k.to_string()),
                "`{k}` nie stoi w MAPPED"
            );
        }
    }

    /// Regresja audytu 30.08: pola obecne w presetach muszą przejść pełną
    /// drogę, nie tylko znaleźć się na białej liście diagnostyki.
    #[test]
    fn osie_god_x4_i_nowsze_przezywaja_obieg_panelu() {
        let mut z = Settings::default();
        z.be_min_pozycji = 3;
        z.be_od_etapu = 2;
        z.cele_pomin_za_cena = true;
        z.entry_jeden_na_glebokiej = true;
        z.entry_uklad = "0,2,5".into();
        z.entry_uklad_kotwica = "Strefa".into();
        z.entry_krzywa_kotwica = "Krawedz".into();
        z.tp_drabinka_kotwica = "Tp1".into();
        z.entry_warstwy_offset = 0.7;
        z.entry_warstwy_z_tekstu = true;
        z.oae_pod_woda = OaePodWoda::DociagnijStop;
        z.rearm_bez_pozycji = true;
        z.rearm_bez_pozycji_max_h = 2.5;
        z.runner_cele_n = 6;
        z.runner_cele_krok = 12.5;
        z.runner_partial_pct = 17.0;
        z.sl_po_tp1_na_krawedz = true;
        z.sl_wlasny_na_pozycje = 1.25;
        z.trail_sr_tf_min = 2;
        z.trail_sr_fractal_n = 5;
        z.trail_sr_offset = 0.75;
        z.trail_sr_min_dist_tp = 3.5;
        z.trail_sr_struct_window_h = 36;
        z.trail_sr_min_prominence_atr = 0.8;
        z.trail_sr_offset_atr_mult = 0.4;
        z.trail_sr_offset_spread_mult = 2.5;
        z.trail_sr_atr_period = 21;

        let po = core_from_ui(&preset_to_ui(&serde_json::to_value(&z).unwrap()));
        assert_eq!(po.be_min_pozycji, 3);
        assert_eq!(po.be_od_etapu, 2);
        assert!(po.cele_pomin_za_cena && po.entry_jeden_na_glebokiej);
        assert_eq!(po.entry_uklad, "0,2,5");
        assert_eq!(po.entry_uklad_kotwica, "Strefa");
        assert_eq!(po.entry_krzywa_kotwica, "Krawedz");
        assert_eq!(po.tp_drabinka_kotwica, "Tp1");
        assert_eq!(po.entry_warstwy_offset, 0.7);
        assert!(po.entry_warstwy_z_tekstu);
        assert_eq!(po.oae_pod_woda, OaePodWoda::DociagnijStop);
        assert!(po.rearm_bez_pozycji);
        assert_eq!(po.rearm_bez_pozycji_max_h, 2.5);
        assert_eq!(
            (po.runner_cele_n, po.runner_cele_krok, po.runner_partial_pct),
            (6, 12.5, 17.0)
        );
        assert!(po.sl_po_tp1_na_krawedz);
        assert_eq!(po.sl_wlasny_na_pozycje, 1.25);
        assert_eq!((po.trail_sr_tf_min, po.trail_sr_fractal_n), (2, 5));
        assert_eq!((po.trail_sr_offset, po.trail_sr_min_dist_tp), (0.75, 3.5));
        assert_eq!(po.trail_sr_struct_window_h, 36);
        assert_eq!(po.trail_sr_min_prominence_atr, 0.8);
        assert_eq!(po.trail_sr_offset_atr_mult, 0.4);
        assert_eq!(po.trail_sr_offset_spread_mult, 2.5);
        assert_eq!(po.trail_sr_atr_period, 21);
    }
}

#[cfg(test)]
mod credit_balance_separate_tests {
    use super::*;
    #[test]
    fn credit_balance_separate_chain_account_overrides_watchdog_and_autostart() {
        let preset = conduit_core::Settings {
            mt5_autostart: true,
            mt5_watchdog: true,
            credit_balance_separate: false,
            ..Default::default()
        };
        let mut account = core_from_ui(
            &serde_json::json!({"mt5_autostart":false,"mt5_watchdog":false,"credit_balance_separate":true}),
        );
        apply_lot(&mut account, &crate::ui::LotConfig::default());
        let effective = conduit_core::wielosilnik::ustawienia_formatu(&preset, &account);
        assert!(!effective.mt5_autostart);
        assert!(!effective.mt5_watchdog);
        assert!(effective.credit_balance_separate);
    }
    #[test]
    fn credit_balance_separate_maps_roundtrips_and_is_account_override() {
        let doc = serde_json::json!({"credit_balance_separate":true,"odlicz_kredyt":true,"kredyt_reczny":300.0});
        let cfg = core_from_ui(&doc);
        assert!(cfg.credit_balance_separate);
        assert!(unmapped_keys(&doc).is_empty());
        let raw = serde_json::to_value(&cfg).unwrap();
        assert!(core_from_ui(&raw).credit_balance_separate);
        let preset = conduit_core::Settings::default();
        let merged = conduit_core::wielosilnik::ustawienia_formatu(&preset, &cfg);
        assert!(merged.credit_balance_separate);
        assert_eq!(merged.saldo_wlasne(159.8, 300.0), 159.8);
        let off = core_from_ui(&serde_json::json!({"credit_balance_separate":false}));
        assert!(
            !conduit_core::wielosilnik::ustawienia_formatu(&merged, &off).credit_balance_separate
        );
    }
}

#[cfg(test)]
mod close_receipt_reconcile_mapping_tests {
    use super::*;

    #[test]
    fn close_receipt_reconcile_roundtrip_defaults_off_and_account_wins() {
        assert!(!Settings::default().close_receipt_reconcile);
        assert!(!core_from_ui(&serde_json::json!({})).close_receipt_reconcile);
        assert!(conduit_core::wielosilnik::POLA_RACHUNKU.contains(&"close_receipt_reconcile"));
        for enabled in [false, true] {
            let doc = serde_json::json!({"close_receipt_reconcile": enabled});
            assert!(unmapped_keys(&doc).is_empty());
            let mut account = core_from_ui(&doc);
            apply_lot(&mut account, &crate::ui::LotConfig::default());
            assert_eq!(account.close_receipt_reconcile, enabled);
            let raw = serde_json::to_value(&account).unwrap();
            assert_eq!(core_from_ui(&raw).close_receipt_reconcile, enabled);
            let ui = preset_to_ui(&raw);
            assert_eq!(core_from_ui(&ui).close_receipt_reconcile, enabled);
            let mut contradictory_preset = Settings::default();
            contradictory_preset.close_receipt_reconcile = !enabled;
            let effective =
                conduit_core::wielosilnik::ustawienia_formatu(&contradictory_preset, &account);
            assert_eq!(effective.close_receipt_reconcile, enabled);
        }
    }
}

#[cfg(test)]
mod ui_contract_flags_3108_tests {
    use super::*;

    #[test]
    fn volume_be_and_retarget_flags_default_off_roundtrip_and_keep_their_owner() {
        let empty = core_from_ui(&serde_json::json!({}));
        assert!(!empty.order_volume_contract_v2);
        assert!(!empty.be_never_loosen);
        assert!(!empty.retarget_respects_final_target);
        assert!(conduit_core::wielosilnik::POLA_RACHUNKU.contains(&"order_volume_contract_v2"));
        assert!(!conduit_core::wielosilnik::POLA_RACHUNKU.contains(&"be_never_loosen"));
        assert!(
            !conduit_core::wielosilnik::POLA_RACHUNKU.contains(&"retarget_respects_final_target")
        );

        for volume in [false, true] {
            for be in [false, true] {
                for retarget in [false, true] {
                    let doc = serde_json::json!({
                        "order_volume_contract_v2": volume,
                        "be_never_loosen": be,
                        "retarget_respects_final_target": retarget,
                    });
                    assert!(unmapped_keys(&doc).is_empty());
                    let mut account = core_from_ui(&doc);
                    apply_lot(&mut account, &crate::ui::LotConfig::default());
                    let raw = serde_json::to_value(&account).unwrap();
                    for roundtrip in [core_from_ui(&raw), core_from_ui(&preset_to_ui(&raw))] {
                        assert_eq!(roundtrip.order_volume_contract_v2, volume);
                        assert_eq!(roundtrip.be_never_loosen, be);
                        assert_eq!(roundtrip.retarget_respects_final_target, retarget);
                    }
                    let preset = Settings {
                        order_volume_contract_v2: !volume,
                        be_never_loosen: !be,
                        retarget_respects_final_target: !retarget,
                        ..Default::default()
                    };
                    let effective =
                        conduit_core::wielosilnik::ustawienia_formatu(&preset, &account);
                    assert_eq!(
                        effective.order_volume_contract_v2, volume,
                        "global broker contract wins"
                    );
                    assert_eq!(effective.be_never_loosen, !be, "selected strategy wins");
                    assert_eq!(
                        effective.retarget_respects_final_target, !retarget,
                        "selected strategy wins"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod deferred_relot_ui_mapping_tests {
    use super::*;

    #[test]
    fn deferred_and_relot_defaults_legacy_roundtrip_and_strategy_owner() {
        for empty in [
            Settings::default(),
            core_from_ui(&serde_json::json!({})),
            serde_json::from_value::<Settings>(serde_json::json!({})).unwrap(),
        ] {
            assert!(!empty.defer_entry_until_receipts);
            assert_eq!(empty.deferred_entry_max_age_s, 300.0);
            assert!(!empty.pending_relot_reconcile_target);
        }
        for key in [
            "defer_entry_until_receipts",
            "deferred_entry_max_age_s",
            "pending_relot_reconcile_target",
        ] {
            assert!(
                !conduit_core::wielosilnik::POLA_RACHUNKU.contains(&key),
                "strategy field {key}"
            );
        }
        for defer in [false, true] {
            for relot in [false, true] {
                for max_age in [1.0, 300.0, 901.5] {
                    let doc = serde_json::json!({
                        "defer_entry_until_receipts": defer,
                        "deferred_entry_max_age_s": max_age,
                        "pending_relot_reconcile_target": relot,
                    });
                    assert!(unmapped_keys(&doc).is_empty());
                    let mut account = core_from_ui(&doc);
                    apply_lot(&mut account, &crate::ui::LotConfig::default());
                    let raw = serde_json::to_value(&account).unwrap();
                    for roundtrip in [core_from_ui(&raw), core_from_ui(&preset_to_ui(&raw))] {
                        assert_eq!(roundtrip.defer_entry_until_receipts, defer);
                        assert_eq!(roundtrip.deferred_entry_max_age_s, max_age);
                        assert_eq!(roundtrip.pending_relot_reconcile_target, relot);
                    }
                    let preset = Settings {
                        defer_entry_until_receipts: !defer,
                        deferred_entry_max_age_s: max_age + 17.0,
                        pending_relot_reconcile_target: !relot,
                        ..Default::default()
                    };
                    let effective =
                        conduit_core::wielosilnik::ustawienia_formatu(&preset, &account);
                    assert_eq!(effective.defer_entry_until_receipts, !defer);
                    assert_eq!(effective.deferred_entry_max_age_s, max_age + 17.0);
                    assert_eq!(effective.pending_relot_reconcile_target, !relot);
                }
            }
        }
        // The mapper must not silently turn invalid zero into unlimited or 300.
        // Execution validates the strict positive/finite contract separately.
        assert_eq!(
            core_from_ui(&serde_json::json!({"deferred_entry_max_age_s":0}))
                .deferred_entry_max_age_s,
            0.0
        );
    }
}

#[cfg(test)]
mod closed_profit_net_costs_mapping_tests {
    use super::*;

    #[test]
    fn closed_net_default_off_account_owner_and_no_implicit_prerequisite_changes() {
        for empty in [
            Settings::default(),
            core_from_ui(&serde_json::json!({})),
            serde_json::from_value::<Settings>(serde_json::json!({})).unwrap(),
        ] {
            assert!(!empty.closed_profit_net_costs);
        }
        assert!(conduit_core::wielosilnik::POLA_RACHUNKU.contains(&"closed_profit_net_costs"));
        for net in [false, true] {
            for receipt in [false, true] {
                for ledger in [false, true] {
                    let doc = serde_json::json!({"closed_profit_net_costs":net,
                        "close_receipt_reconcile":receipt, "basket_realized_broker_only":ledger});
                    assert!(unmapped_keys(&doc).is_empty());
                    let mut account = core_from_ui(&doc);
                    apply_lot(&mut account, &crate::ui::LotConfig::default());
                    let raw = serde_json::to_value(&account).unwrap();
                    for roundtrip in [core_from_ui(&raw), core_from_ui(&preset_to_ui(&raw))] {
                        assert_eq!(roundtrip.closed_profit_net_costs, net);
                        assert_eq!(
                            roundtrip.close_receipt_reconcile, receipt,
                            "mapping never enables a prerequisite"
                        );
                        assert_eq!(roundtrip.basket_realized_broker_only, ledger);
                    }
                    let preset = Settings {
                        closed_profit_net_costs: !net,
                        close_receipt_reconcile: !receipt,
                        basket_realized_broker_only: !ledger,
                        ..Default::default()
                    };
                    let effective =
                        conduit_core::wielosilnik::ustawienia_formatu(&preset, &account);
                    assert_eq!(
                        effective.closed_profit_net_costs, net,
                        "global convention wins"
                    );
                    assert_eq!(
                        effective.close_receipt_reconcile, receipt,
                        "global receipt contract wins"
                    );
                    assert_eq!(
                        effective.basket_realized_broker_only, !ledger,
                        "strategy-owned ledger prerequisite is not silently overwritten"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod strategy_continuation_mapping_tests {
    use super::*;

    #[test]
    fn continuation_roundtrips_as_account_state_without_enabling_other_contracts() {
        for empty in [
            Settings::default(),
            core_from_ui(&serde_json::json!({})),
            serde_json::from_value::<Settings>(serde_json::json!({})).unwrap(),
        ] {
            assert!(!empty.restore_strategy_continuation);
        }
        assert!(conduit_core::wielosilnik::POLA_RACHUNKU.contains(&"restore_strategy_continuation"));
        for enabled in [false, true] {
            for receipts in [false, true] {
                let doc = serde_json::json!({"restore_strategy_continuation": enabled,
                    "close_receipt_reconcile": receipts});
                assert!(unmapped_keys(&doc).is_empty());
                let account = core_from_ui(&doc);
                let raw = serde_json::to_value(&account).unwrap();
                for copy in [core_from_ui(&raw), core_from_ui(&preset_to_ui(&raw))] {
                    assert_eq!(copy.restore_strategy_continuation, enabled);
                    assert_eq!(
                        copy.close_receipt_reconcile, receipts,
                        "a mapper must not silently change another runtime contract"
                    );
                    assert!(!copy.closed_profit_net_costs);
                }
                let preset = Settings {
                    restore_strategy_continuation: !enabled,
                    close_receipt_reconcile: !receipts,
                    ..Default::default()
                };
                let effective = conduit_core::wielosilnik::ustawienia_formatu(&preset, &account);
                assert_eq!(effective.restore_strategy_continuation, enabled);
                assert_eq!(effective.close_receipt_reconcile, receipts);
            }
        }
    }
}

#[cfg(test)]
mod entry_edit_sr_v2_mapping_tests {
    use super::*;

    #[test]
    fn v2_strategy_flags_roundtrip_without_enabling_parents_or_becoming_account_overrides() {
        let empty = core_from_ui(&serde_json::json!({}));
        assert!(!empty.entry_edit_geometry_v2 && !empty.sr_warmup_exact_ticks);
        for key in ["entry_edit_geometry_v2", "sr_warmup_exact_ticks"] {
            assert!(!conduit_core::wielosilnik::POLA_RACHUNKU.contains(&key));
        }
        for edit in [false, true] {
            for warmup in [false, true] {
                for sr in [false, true] {
                    let doc = serde_json::json!({
                        "entry_edit_geometry_v2": edit,
                        "sr_warmup_exact_ticks": warmup,
                        "trail_sr_enabled": sr,
                    });
                    assert!(unmapped_keys(&doc).is_empty());
                    let core = core_from_ui(&doc);
                    let raw = serde_json::to_value(&core).unwrap();
                    for copy in [core_from_ui(&raw), core_from_ui(&preset_to_ui(&raw))] {
                        assert_eq!(copy.entry_edit_geometry_v2, edit);
                        assert_eq!(copy.sr_warmup_exact_ticks, warmup);
                        assert_eq!(
                            copy.trail_sr_enabled, sr,
                            "never implicitly enable a parent"
                        );
                    }
                    let account = Settings {
                        entry_edit_geometry_v2: !edit,
                        sr_warmup_exact_ticks: !warmup,
                        trail_sr_enabled: !sr,
                        ..Default::default()
                    };
                    let effective = conduit_core::wielosilnik::ustawienia_formatu(&core, &account);
                    assert_eq!(effective.entry_edit_geometry_v2, edit);
                    assert_eq!(effective.sr_warmup_exact_ticks, warmup);
                    assert_eq!(effective.trail_sr_enabled, sr);
                }
            }
        }
    }
}
