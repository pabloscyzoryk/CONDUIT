
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(u8)]
pub enum Aktuator0 {
    /// `modify_position(sl, ..)` — stop pozycji
    StopPozycji = 0,
    /// `modify_position(.., tp)` — cel pozycji
    CelPozycji = 1,
    /// `close_position` — zamknięcie CAŁOŚCI pozycji
    ZyciePozycji = 2,
    /// `close_partial` — zamknięcie CZĘŚCI pozycji
    WolumenPozycji = 3,
    /// `cancel_pending` — anulowanie zlecenia oczekującego
    ZycieZlecenia = 4,
    /// `modify_pending` / relot — cena, wolumen, SL/TP zlecenia
    KsztaltZlecenia = 5,
    /// `place_pending` × n — GEOMETRIA WEJŚCIA (plan próby)
    PlanProby = 6,
    /// bramki wejściowe sygnału (`handle_entry`)
    WejscieSygnalu = 7,
    /// slot POMYSŁU (duplikat setupu, scalenia)
    SlotPomyslu = 8,
    /// dołożenie ekspozycji do istniejącej próby (`open_market` dokładający)
    DolozenieDoProby = 9,
    /// kontra do ramy (dziś nie istnieje — zostaje pusta kolumna)
    KontraRamy = 10,
    /// życie ramy: `Wygaszana` / `Zamknieta` (dziś == życie koszyka)
    ZycieRamy = 11,
    /// odroczenie / wykonanie komendy kanału
    BramaKomendy = 12,
    /// halt, pauza, zatrzymanie doby
    BramaSilnika = 13,
}

/// Ile wariantów — rozmiar tablic zbiorczych w [`crate::cien`].
pub const ILE: usize = 14;

/// Krótkie, stabilne nazwy do raportu. Indeks == dyskryminator.
pub const NAZWY: [&str; ILE] = [
    "StopPozycji",
    "CelPozycji",
    "ZyciePozycji",
    "WolumenPozycji",
    "ZycieZlecenia",
    "KsztaltZlecenia",
    "PlanProby",
    "WejscieSygnalu",
    "SlotPomyslu",
    "DolozenieDoProby",
    "KontraRamy",
    "ZycieRamy",
    "BramaKomendy",
    "BramaSilnika",
];

// Stałe do użycia we wpięciach — `u8`, żeby wpięcie w `engine.rs` było
// jedną linią bez importu typu.
pub const A_STOP: u8 = Aktuator0::StopPozycji as u8;
pub const A_CEL: u8 = Aktuator0::CelPozycji as u8;
pub const A_ZYCIE_POZ: u8 = Aktuator0::ZyciePozycji as u8;
pub const A_WOLUMEN_POZ: u8 = Aktuator0::WolumenPozycji as u8;
pub const A_ZYCIE_ZLEC: u8 = Aktuator0::ZycieZlecenia as u8;
pub const A_KSZTALT_ZLEC: u8 = Aktuator0::KsztaltZlecenia as u8;
pub const A_PLAN_PROBY: u8 = Aktuator0::PlanProby as u8;
pub const A_WEJSCIE_SYG: u8 = Aktuator0::WejscieSygnalu as u8;
pub const A_SLOT_POMYSLU: u8 = Aktuator0::SlotPomyslu as u8;
pub const A_DOLOZENIE: u8 = Aktuator0::DolozenieDoProby as u8;
pub const A_KONTRA: u8 = Aktuator0::KontraRamy as u8;
pub const A_ZYCIE_RAMY: u8 = Aktuator0::ZycieRamy as u8;
pub const A_BRAMA_KOMENDY: u8 = Aktuator0::BramaKomendy as u8;
pub const A_BRAMA_SILNIKA: u8 = Aktuator0::BramaSilnika as u8;

/// Nazwa aktuatora albo `"?"` — raport nie ma prawa panikować przez literówkę.
#[inline]
pub fn nazwa(a: u8) -> &'static str {
    let i = a as usize;
    if i < ILE {
        NAZWY[i]
    } else {
        "?"
    }
}
