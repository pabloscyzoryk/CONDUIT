
/// Ile źródeł zadeklarowano (do sanity-checku raportu).
pub const ILE: u16 = 64;

// ---------- LEJKI (tożsamość rodziny = linia wołającego) ----------

/// `Engine::try_modify` — JEDYNA ścieżka SL/TP z kolejką ponowień (16 wołających).
pub const L_TRY_MODIFY: u16 = 1;
/// `Engine::cancel_pendings` — kasowanie CAŁEJ siatki koszyka (19 wołających).
pub const L_CANCEL_PENDINGS: u16 = 2;
/// `Engine::cancel_pendings_keep` — kasowanie z zachowaniem N szczebli.
pub const L_CANCEL_KEEP: u16 = 3;
/// `Engine::close_or_queue` — wyjścia „bez pośpiechu" (9 wołających).
pub const L_CLOSE_OR_QUEUE: u16 = 4;
/// `Engine::close_everything` — likwidacja całego rachunku (10 wołających).
pub const L_CLOSE_EVERYTHING: u16 = 5;
/// `Engine::close_basket` — zamknięcie koszyka.
pub const L_CLOSE_BASKET: u16 = 6;
/// `Engine::sync_grid` — rozstawianie/dostawianie siatki (6 wołających).
pub const L_SYNC_GRID: u16 = 7;

// ---------- PISARZE BEZPOŚREDNI ----------

pub const Z_RETRY_STOPS: u16 = 10;
pub const Z_CEL_ZE_STREFY_PRZECIWNEJ: u16 = 11;
pub const Z_KANAL_TP_KOREKTA_PENDING: u16 = 12;
pub const Z_MARKET_OPEN: u16 = 13;
pub const Z_CLOSE_OPPOSITE: u16 = 14;
pub const Z_SYNC_GRID_RYNEK: u16 = 15;
pub const Z_SYNC_GRID_PENDING: u16 = 16;
pub const Z_SYNC_GRID_KASUJ: u16 = 17;
pub const Z_TP_ZAMKNIJ: u16 = 18;
pub const Z_TP_ZDEJMIJ_CEL_RUNNERA: u16 = 19;
pub const Z_PIRAMIDA_PO_TP: u16 = 20;
pub const Z_BANK_ON_TP_CZESC: u16 = 21;
pub const Z_BANK_ON_TP_CALOSC: u16 = 22;
pub const Z_CLOSE_BASKET_POZ: u16 = 23;
pub const Z_INKASO_CZESC: u16 = 24;
pub const Z_INKASO_CALOSC: u16 = 25;
pub const Z_RISK_FREE_ZAMKNIJ: u16 = 26;
pub const Z_OUT_AT_ENTRY_ZAMKNIJ: u16 = 27;
pub const Z_SL_HIT_ZAMKNIJ: u16 = 28;
pub const Z_SET_BASKET_SL_PENDING: u16 = 29;
pub const Z_CANCEL_PENDINGS_ZLEC: u16 = 30;
pub const Z_CLOSE_EVERYTHING_POZ: u16 = 31;
pub const Z_CLOSE_EVERYTHING_PEND: u16 = 32;
pub const Z_PENDING_TTL: u16 = 33;
pub const Z_CLOSE_OR_QUEUE_POZ: u16 = 34;
pub const Z_KOLEJKA_WYJSC: u16 = 35;
pub const Z_VIRTUAL_SL: u16 = 36;
pub const Z_OAE_TIMEOUT: u16 = 37;
pub const Z_RELOT_KASUJ: u16 = 38;
pub const Z_RELOT_ZMNIEJSZ: u16 = 39;
pub const Z_RELOT_DOSTAW: u16 = 40;
pub const Z_RELOT_PRZESTAW: u16 = 41;
pub const Z_REDUKCJA_POZ: u16 = 42;
pub const Z_REDUKCJA_PEND: u16 = 43;
pub const Z_REDUKCJA_POZ_LEWAR: u16 = 44;
pub const Z_LIMIT_RUNNERA: u16 = 45;
pub const Z_RISKFREE_PASS_ZAMKNIJ: u16 = 46;
pub const Z_EXPIRE_OLD: u16 = 47;
pub const Z_LIMIT_KASUJ_NADMIAR: u16 = 48;
pub const Z_REJECT_FAST_FILLED: u16 = 49;
pub const Z_FAST_ADDON: u16 = 50;
pub const Z_ZONE_EXIT_ADVERSE: u16 = 51;
pub const Z_REENTRY: u16 = 52;
pub const Z_REV_EXIT: u16 = 53;
pub const Z_CANCEL_KEEP_ZLEC: u16 = 54;
pub const Z_VIRTUAL_SL_ZAPIS: u16 = 55;

// --- WARSTWA EA (`crates/core/src/ea.rs`) ---
//
// Zarezerwowane z góry, bo `ea.rs` jest w tej chwili edytowany przez
// RÓWNOLEGŁY przepływ (prototypy PROW-1A/1B). Wpięcie oddane jako gotowa łata
// w raporcie E0 — numery są już stabilne, więc łatę można nałożyć bez ruszania
// tego pliku i bez przenumerowania raportów.
/// N15 — dozór SL (`EaRdzen::dozor_sl`), stop dostawiany pozycji bez stopu.
pub const Z_EA_DOZOR_SL: u16 = 60;
/// Warstwa EA: własny zapis stopu.
pub const Z_EA_STOP: u16 = 61;
/// Warstwa EA: własny zapis celu.
pub const Z_EA_CEL: u16 = 62;
/// Warstwa EA: zamknięcie pozycji.
pub const Z_EA_ZAMKNIJ: u16 = 63;
/// Warstwa EA: zamknięcie części pozycji.
pub const Z_EA_CZESC: u16 = 64;
/// Warstwa EA: skasowanie zlecenia oczekującego.
pub const Z_EA_KASUJ: u16 = 65;
/// Warstwa EA: otwarcie pozycji rynkowej.
pub const Z_EA_RYNEK: u16 = 66;
/// Warstwa EA: złożenie zlecenia oczekującego.
pub const Z_EA_ZLECENIE: u16 = 67;

pub fn nazwa(z: u16) -> &'static str {
    match z {
        L_TRY_MODIFY => "lejek:try_modify",
        L_CANCEL_PENDINGS => "lejek:cancel_pendings",
        L_CANCEL_KEEP => "lejek:cancel_pendings_keep",
        L_CLOSE_OR_QUEUE => "lejek:close_or_queue",
        L_CLOSE_EVERYTHING => "lejek:close_everything",
        L_CLOSE_BASKET => "lejek:close_basket",
        L_SYNC_GRID => "lejek:sync_grid",
        Z_RETRY_STOPS => "retry_stops",
        Z_CEL_ZE_STREFY_PRZECIWNEJ => "cel_ze_strefy_przeciwnej",
        Z_KANAL_TP_KOREKTA_PENDING => "kanal_tp_korekta_pending",
        Z_MARKET_OPEN => "handle_market_open",
        Z_CLOSE_OPPOSITE => "close_opposite_baskets",
        Z_SYNC_GRID_RYNEK => "sync_grid:rynek",
        Z_SYNC_GRID_PENDING => "sync_grid:pending",
        Z_SYNC_GRID_KASUJ => "sync_grid:kasuj_nadmiar",
        Z_TP_ZAMKNIJ => "handle_tp_hit:zamknij",
        Z_TP_ZDEJMIJ_CEL_RUNNERA => "handle_tp_hit:zdejmij_cel_runnera",
        Z_PIRAMIDA_PO_TP => "piramida_po_tp",
        Z_BANK_ON_TP_CZESC => "bank_on_tp:czesc",
        Z_BANK_ON_TP_CALOSC => "bank_on_tp:calosc",
        Z_CLOSE_BASKET_POZ => "close_basket:pozycja",
        Z_INKASO_CZESC => "inkasuj_partials:czesc",
        Z_INKASO_CALOSC => "inkasuj_partials:calosc",
        Z_RISK_FREE_ZAMKNIJ => "handle_risk_free:zamknij",
        Z_OUT_AT_ENTRY_ZAMKNIJ => "handle_out_at_entry:zamknij",
        Z_SL_HIT_ZAMKNIJ => "handle_sl_hit:zamknij",
        Z_SET_BASKET_SL_PENDING => "set_basket_sl:pendingi",
        Z_CANCEL_PENDINGS_ZLEC => "cancel_pendings:zlecenie",
        Z_CLOSE_EVERYTHING_POZ => "close_everything:pozycja",
        Z_CLOSE_EVERYTHING_PEND => "close_everything:zlecenie",
        Z_PENDING_TTL => "pending_ttl",
        Z_CLOSE_OR_QUEUE_POZ => "close_or_queue:pozycja",
        Z_KOLEJKA_WYJSC => "sweep_queued_exits",
        Z_VIRTUAL_SL => "virtual_sl:zamkniecie",
        Z_OAE_TIMEOUT => "oae_timeout",
        Z_RELOT_KASUJ => "relot:kasuj_topup",
        Z_RELOT_ZMNIEJSZ => "relot:zmniejsz",
        Z_RELOT_DOSTAW => "relot:dostaw_roznice",
        Z_RELOT_PRZESTAW => "relot:przestaw",
        Z_REDUKCJA_POZ => "redukuj_ekspozycje:pozycja",
        Z_REDUKCJA_PEND => "redukuj_ekspozycje:zlecenie",
        Z_REDUKCJA_POZ_LEWAR => "redukuj_ekspozycje:pozycja_lewar",
        Z_LIMIT_RUNNERA => "limit_trzymania_runnera",
        Z_RISKFREE_PASS_ZAMKNIJ => "riskfree_pass:zamknij",
        Z_EXPIRE_OLD => "expire_old_baskets",
        Z_LIMIT_KASUJ_NADMIAR => "limit_kasuj_nadmiar",
        Z_REJECT_FAST_FILLED => "reject_fast_filled_baskets",
        Z_FAST_ADDON => "fast_addon_sweep",
        Z_ZONE_EXIT_ADVERSE => "zone_exit_adverse_sweep",
        Z_REENTRY => "reentry_pass",
        Z_REV_EXIT => "rev_exit_sweep",
        Z_CANCEL_KEEP_ZLEC => "cancel_pendings_keep:zlecenie",
        Z_VIRTUAL_SL_ZAPIS => "apply_virtual_sl",
        Z_EA_DOZOR_SL => "ea:dozor_sl(N15)",
        Z_EA_STOP => "ea:stop",
        Z_EA_CEL => "ea:cel",
        Z_EA_ZAMKNIJ => "ea:zamknij",
        Z_EA_CZESC => "ea:czesc",
        Z_EA_KASUJ => "ea:kasuj_zlecenie",
        Z_EA_RYNEK => "ea:rynek",
        Z_EA_ZLECENIE => "ea:zlecenie",
        _ => "?",
    }
}
