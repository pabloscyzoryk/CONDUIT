//! Optional research metadata carried by the existing statistics map.
//! Old producers remain readable. Missing fidelity is never inferred from a
//! preset name, profit, speed, or the number of completed runs.

use crate::Statystyki;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrybObliczen {
    Quick,
    Full,
    Mixed,
    Niepodany,
}

pub fn wartosc<'a>(stat: &'a Statystyki, key: &str) -> Option<&'a str> {
    stat.0
        .iter()
        .rev()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.trim())
        .filter(|v| !v.is_empty())
}

pub fn tryb(stat: &Statystyki) -> TrybObliczen {
    match wartosc(stat, "tryb_obliczen")
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("quick") => TrybObliczen::Quick,
        Some("full" | "exact") => TrybObliczen::Full,
        Some("mixed") => TrybObliczen::Mixed,
        _ => TrybObliczen::Niepodany,
    }
}

pub fn walidacja(stat: &Statystyki) -> &str {
    walidacja_w_jezyku(stat, crate::language::Language::Pl)
}

pub fn walidacja_w_jezyku(stat: &Statystyki, language: crate::language::Language) -> &str {
    match wartosc(stat, "status_walidacji") {
        Some("in_sample") => language.text(
            "Próba ucząca · wynik historyczny",
            "Training sample · historical result",
        ),
        Some("holdout") => language.text(
            "Holdout · wydzielone dane historyczne",
            "Holdout · reserved historical data",
        ),
        Some("walk_forward") => language.text(
            "Walk-forward · walidacja historyczna",
            "Walk-forward · historical validation",
        ),
        Some("coronation") => language.text(
            "Koronacja · porównanie kandydatów",
            "Final selection · candidate comparison",
        ),
        Some(other) => other,
        None => language.text(
            "Walidacja poza próbą: niepotwierdzona",
            "Out-of-sample validation: unconfirmed",
        ),
    }
}

/// Keys promoted into the overview. Other metrics stay in the complete table.
pub fn metadane(key: &str) -> bool {
    matches!(
        key,
        "etap_badania"
            | "tryb_obliczen"
            | "watki"
            | "max_lot"
            | "depozyt"
            | "okno"
            | "kanal"
            | "status_walidacji"
            | "kandydaci"
            | "zaplanowane"
            | "ukonczone"
            | "nieudane"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absence_or_similar_text_never_claims_full_fidelity() {
        let mut s = Statystyki::nowe();
        assert_eq!(tryb(&s), TrybObliczen::Niepodany);
        s.dodaj("etap_badania", "full GOD-X8");
        s.dodaj("tryb_obliczen", "nearly-full");
        assert_eq!(tryb(&s), TrybObliczen::Niepodany);
        s.dodaj("tryb_obliczen", " FULL ");
        assert_eq!(tryb(&s), TrybObliczen::Full);
        s.dodaj("tryb_obliczen", "quick");
        assert_eq!(tryb(&s), TrybObliczen::Quick);
    }

    #[test]
    fn full_ticks_do_not_imply_out_of_sample_validation() {
        let mut s = Statystyki::nowe();
        s.dodaj("tryb_obliczen", "full");
        assert!(walidacja(&s).contains("niepotwierdzona"));
        s.dodaj("status_walidacji", "coronation");
        assert!(walidacja(&s).contains("Koronacja"));
    }

    #[test]
    fn exact_is_an_explicit_full_tick_alias_without_claiming_validation() {
        let mut s = Statystyki::nowe();
        s.dodaj("tryb_obliczen", " EXACT ");
        assert_eq!(tryb(&s), TrybObliczen::Full);
        assert!(walidacja(&s).contains("niepotwierdzona"));
        s.dodaj("tryb_obliczen", "exact-ish");
        assert_eq!(tryb(&s), TrybObliczen::Niepodany);
    }
}
