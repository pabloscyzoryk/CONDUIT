
use crate::ui::TradingMode;
use conduit_core::engine::Engine;

/// Tryb handlu instancji symulacji: pole `"mode"` jej rekordu albo —
/// gdy pola nie ma, jest nieznane lub mówi MANUAL — tryb głównego bota.
///
/// Nieznany string celowo NIE jest błędem: rekord jest surowym JSON-em
/// i literówka w nim ma degradować do dotychczasowego zachowania
/// (dziedziczenie), a nie wywracać pętli, która po nim iteruje —
/// ten sam kontrakt co `mode` w `settings.json` (test
/// `nieznany_tryb_nie_panikuje...` w `store.rs`).
pub fn tryb_instancji(rec: &serde_json::Value, glowny: TradingMode) -> TradingMode {
    match rec
        .get("mode")
        .map(|v| serde_json::from_value::<TradingMode>(v.clone()))
    {
        Some(Ok(TradingMode::Manual)) | Some(Err(_)) | None => glowny,
        Some(Ok(tryb)) => tryb,
    }
}

/// Czy silniki instancji mają grać z flagą warstwy EA.
pub fn auto_ea_instancji(rec: &serde_json::Value, glowny: TradingMode) -> bool {
    matches!(tryb_instancji(rec, glowny), TradingMode::AutoEa)
}

/// Ustawia flagę `tryb_auto_ea` na WSZYSTKICH silnikach instancji — wg
/// rekordu TEJ instancji, niezależnie od tego, czym gra główny bot.
///
/// Lustrzane odbicie pętli z `crates/app/src/live.rs` (tam: silniki
/// głównego łańcucha wg `UiSnapshot::mode`). Kto buduje silniki dla
/// instancji symulacji, woła to raz na obrót swojej pętli — flaga jest
/// tania, a tryb instancji może się zmienić bez restartu.
pub fn zastosuj_tryb_instancji<'a>(
    silniki: impl IntoIterator<Item = &'a mut Engine>,
    rec: &serde_json::Value,
    glowny: TradingMode,
) {
    let auto_ea = auto_ea_instancji(rec, glowny);
    for e in silniki {
        e.tryb_auto_ea = auto_ea;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn silnik() -> Engine {
        Engine::new(conduit_core::Settings::default(), 200.0)
    }

    /// KONTRAKT ZERA: rekord bez pola `mode` (każda instancja sprzed tej
    /// zmiany) dziedziczy tryb głównego bota — w OBIE strony.
    #[test]
    fn rekord_bez_pola_dziedziczy_tryb_glowny() {
        let rec = json!({ "id": "sim1", "preset": "HYPER-2" });
        assert_eq!(tryb_instancji(&rec, TradingMode::Auto), TradingMode::Auto);
        assert_eq!(
            tryb_instancji(&rec, TradingMode::AutoEa),
            TradingMode::AutoEa
        );
        assert!(!auto_ea_instancji(&rec, TradingMode::Auto));
        assert!(auto_ea_instancji(&rec, TradingMode::AutoEa));
    }

    /// Sedno zmiany: główny bot AUTO-EA + instancja AUTO i na odwrót.
    /// Flaga na silnikach INSTANCJI idzie za rekordem, nie za trybem głównym.
    #[test]
    fn flaga_na_silnikach_instancji_niezalezna_od_trybu_glownego() {
        // główny bot: AUTO-EA, instancja: AUTO → silniki instancji BEZ flagi
        let mut a = vec![silnik(), silnik()];
        zastosuj_tryb_instancji(
            a.iter_mut(),
            &json!({ "mode": "AUTO" }),
            TradingMode::AutoEa,
        );
        assert!(
            a.iter().all(|e| !e.tryb_auto_ea),
            "instancja AUTO nie może dostać flagi EA po bocie głównym"
        );

        // główny bot: AUTO, instancja: AUTO-EA → silniki instancji Z flagą
        let mut b = vec![silnik(), silnik()];
        zastosuj_tryb_instancji(
            b.iter_mut(),
            &json!({ "mode": "AUTO-EA" }),
            TradingMode::Auto,
        );
        assert!(
            b.iter().all(|e| e.tryb_auto_ea),
            "instancja AUTO-EA musi dostać flagę mimo głównego AUTO"
        );
    }

    /// Zmiana trybu instancji w locie przestawia flagę w obu kierunkach —
    /// ta sama własność, którą pętla żywa ma dla trybu głównego.
    #[test]
    fn zmiana_trybu_instancji_przestawia_flage_w_locie() {
        let mut s = vec![silnik()];
        zastosuj_tryb_instancji(
            s.iter_mut(),
            &json!({ "mode": "AUTO-EA" }),
            TradingMode::Auto,
        );
        assert!(s[0].tryb_auto_ea);
        zastosuj_tryb_instancji(s.iter_mut(), &json!({ "mode": "AUTO" }), TradingMode::Auto);
        assert!(!s[0].tryb_auto_ea);
    }

    /// Degradacja, nie panika: śmieć i MANUAL w rekordzie działają jak brak
    /// pola. AI to pełnoprawny tryb instancji — ale nie tryb EA.
    #[test]
    fn manual_i_smiec_dziedzicza_a_ai_jest_pelnoprawne() {
        assert_eq!(
            tryb_instancji(&json!({ "mode": "MANUAL" }), TradingMode::AutoEa),
            TradingMode::AutoEa
        );
        assert_eq!(
            tryb_instancji(&json!({ "mode": "TURBO-9000" }), TradingMode::Auto),
            TradingMode::Auto
        );
        assert_eq!(
            tryb_instancji(&json!({ "mode": 42 }), TradingMode::Auto),
            TradingMode::Auto
        );
        assert_eq!(
            tryb_instancji(&json!({ "mode": "AI" }), TradingMode::AutoEa),
            TradingMode::Ai
        );
        assert!(!auto_ea_instancji(
            &json!({ "mode": "AI" }),
            TradingMode::AutoEa
        ));
    }
}
