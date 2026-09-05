//! Koalescencja delt.
//!
//! Silnik potrafi ruszyć stanem setki razy na sekundę (każdy tick to nowy
//! zysk pływający). Interfejs nie potrzebuje więcej niż ~10 Hz — przy 4K
//! każda dodatkowa klatka to realna praca kompozytora, a człowiek i tak
//! nie zobaczy różnicy.
//!
//! Zamiast wysyłać każdą zmianę, oznaczamy „brudne" SEKCJE stanu i raz na
//! `min_interval` wypuszczamy jedną deltę zawierającą tylko te sekcje.
//! Dzięki temu 500 zmian pozycji w ciągu 100 ms to JEDNA ramka z aktualnym
//! stanem pozycji, a nie 500 ramek z historią, której nikt nie użyje.

use serde::{Deserialize, Serialize};

/// Sekcja stanu — najmniejsza jednostka, jaką wysyłamy w delcie.
///
/// Świadomie gruboziarnista: wysyłamy CAŁĄ sekcję, nie różnicę wewnątrz niej.
/// Powód — pozycji jest kilkadziesiąt, nie kilkadziesiąt tysięcy. Różnicowanie
/// per rekord kosztowałoby więcej kodu (i błędów scalania) niż oszczędza bajtów,
/// a scalanie po stronie React sprowadza się do podmiany jednego pola.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Section {
    Quotes,
    Positions,
    Pendings,
    Baskets,
    Closed,
    PendingHistory,
    Stats,
    Logs,
    Messages,
    Settings,
    Auth,
    Connection,
    Halt,
    Sims,
    Bindings,
    Mode,
    /// Postęp laboratorium (backtesty i trening AI). Osobna sekcja, bo zmienia
    /// się kilka razy na sekundę wyłącznie w trakcie liczenia — dopisanie jej
    /// do `stats` kazałoby wysyłać statystyki konta w tym samym rytmie.
    Lab,
    /// Postęp trybu demo: wirtualny zegar, tempo odtwarzania, licznik ticków.
    /// Osobna sekcja z tego samego powodu co `lab` — tyka w rytmie odtwarzania,
    /// a nie w rytmie zmian na koncie.
    Demo,
    Scalanie,
}

impl Section {
    pub const ALL: [Section; 19] = [
        Section::Quotes,
        Section::Positions,
        Section::Pendings,
        Section::Baskets,
        Section::Closed,
        Section::PendingHistory,
        Section::Stats,
        Section::Logs,
        Section::Messages,
        Section::Settings,
        Section::Auth,
        Section::Connection,
        Section::Halt,
        Section::Sims,
        Section::Bindings,
        Section::Mode,
        Section::Lab,
        Section::Demo,
        Section::Scalanie,
    ];

    #[inline]
    pub fn bit(self) -> u32 {
        1u32 << (self as u32)
    }
}

/// Zbiór sekcji jako maska bitowa — tanie łączenie i sprawdzanie.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sections(pub u32);

impl Sections {
    pub const NONE: Sections = Sections(0);

    pub fn all() -> Sections {
        let mut m = 0;
        for s in Section::ALL {
            m |= s.bit();
        }
        Sections(m)
    }

    #[inline]
    pub fn one(s: Section) -> Sections {
        Sections(s.bit())
    }

    /// Dwie sekcje naraz — dla zmian, które z natury dotykają obu.
    ///
    /// Przykład: lista odbiorców podsumowań żyje w ustawieniach, ale jej
    /// zaznaczenia rysują się przy kafelkach kanałów. Oznaczenie jednej
    /// sekcji zostawiłoby drugą stronę panelu z nieaktualnym widokiem.
    pub fn two(a: Section, b: Section) -> Sections {
        Sections(a.bit() | b.bit())
    }

    #[inline]
    pub fn contains(self, s: Section) -> bool {
        self.0 & s.bit() != 0
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub fn insert(&mut self, s: Section) {
        self.0 |= s.bit();
    }

    #[inline]
    pub fn union(self, other: Sections) -> Sections {
        Sections(self.0 | other.0)
    }

    #[inline]
    pub fn intersect(self, other: Sections) -> Sections {
        Sections(self.0 & other.0)
    }

    pub fn from_list(list: &[Section]) -> Sections {
        let mut m = Sections::NONE;
        for s in list {
            m.insert(*s);
        }
        m
    }

    pub fn to_vec(self) -> Vec<Section> {
        Section::ALL
            .into_iter()
            .filter(|s| self.contains(*s))
            .collect()
    }
}

impl std::ops::BitOr for Sections {
    type Output = Sections;
    fn bitor(self, rhs: Sections) -> Sections {
        self.union(rhs)
    }
}

impl std::ops::BitOr<Section> for Sections {
    type Output = Sections;
    fn bitor(self, rhs: Section) -> Sections {
        Sections(self.0 | rhs.bit())
    }
}

impl std::ops::BitOr for Section {
    type Output = Sections;
    fn bitor(self, rhs: Section) -> Sections {
        Sections(self.bit() | rhs.bit())
    }
}

/// Akumulator brudnych sekcji z limitem częstotliwości.
///
/// Nie ma tu zegara systemowego — czas przychodzi w argumencie. Ta sama
/// zasada, co w `conduit_core`: dzięki temu test jest deterministyczny
/// i nie potrzebuje `sleep`.
#[derive(Debug)]
pub struct Coalescer {
    dirty: Sections,
    last_emit_ms: i64,
    min_interval_ms: i64,
}

impl Coalescer {
    pub fn new(min_interval_ms: i64) -> Self {
        Coalescer {
            dirty: Sections::NONE,
            last_emit_ms: i64::MIN / 4,
            min_interval_ms,
        }
    }

    /// Domyślnie ~10 Hz.
    pub fn default_rate() -> Self {
        Coalescer::new(100)
    }

    #[inline]
    pub fn mark(&mut self, s: Sections) {
        self.dirty = self.dirty.union(s);
    }

    #[inline]
    pub fn pending(&self) -> Sections {
        self.dirty
    }

    /// Zwraca sekcje do wysłania, jeśli minął już minimalny odstęp.
    /// `None` = albo nic się nie zmieniło, albo za wcześnie (zmiany czekają).
    pub fn take(&mut self, now_ms: i64) -> Option<Sections> {
        if self.dirty.is_empty() {
            return None;
        }
        if now_ms - self.last_emit_ms < self.min_interval_ms {
            return None;
        }
        self.last_emit_ms = now_ms;
        Some(std::mem::replace(&mut self.dirty, Sections::NONE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maska_sekcji_dziala() {
        let m = Section::Positions | Section::Stats;
        assert!(m.contains(Section::Positions));
        assert!(m.contains(Section::Stats));
        assert!(!m.contains(Section::Logs));
        assert_eq!(m.to_vec().len(), 2);
        assert_eq!(Sections::all().to_vec().len(), Section::ALL.len());
    }

    #[test]
    fn setki_zmian_w_oknie_daja_jedna_delte() {
        let mut c = Coalescer::new(100);
        // pierwsze wywołanie może wyjść od razu — ustalamy punkt odniesienia
        c.mark(Sections::one(Section::Positions));
        assert!(c.take(1_000).is_some());

        // 500 zmian w ciągu 90 ms
        for i in 0..500 {
            c.mark(Sections::one(Section::Positions));
            c.mark(Sections::one(Section::Stats));
            assert!(c.take(1_000 + i % 90).is_none(), "za wcześnie na deltę");
        }

        // po upływie okna wychodzi DOKŁADNIE jedna delta z obiema sekcjami
        let out = c.take(1_100).expect("delta po upływie okna");
        assert!(out.contains(Section::Positions));
        assert!(out.contains(Section::Stats));
        assert!(!out.contains(Section::Logs));

        // i nic już nie zostaje w kolejce
        assert!(c.take(1_500).is_none());
    }

    #[test]
    fn brak_zmian_to_brak_ramki() {
        let mut c = Coalescer::new(100);
        assert!(c.take(0).is_none());
        assert!(c.take(10_000).is_none());
    }

    #[test]
    fn zmiany_z_okna_nie_gina() {
        let mut c = Coalescer::new(100);
        c.mark(Sections::one(Section::Logs));
        assert!(c.take(0).is_some());

        // zmiana w środku okna
        c.mark(Sections::one(Section::Baskets));
        assert!(c.take(50).is_none());
        // …wychodzi w kolejnym oknie, nie przepada
        assert_eq!(c.take(100).unwrap(), Sections::one(Section::Baskets));
    }

    #[test]
    fn czestotliwosc_nie_przekracza_10hz() {
        let mut c = Coalescer::default_rate();
        let mut emisje = 0;
        // sekunda symulowanego czasu, zmiana co 1 ms
        for t in 0..1000 {
            c.mark(Sections::one(Section::Quotes));
            if c.take(t).is_some() {
                emisje += 1;
            }
        }
        assert!(emisje <= 11, "za dużo ramek: {emisje}");
        assert!(emisje >= 10, "za mało ramek: {emisje}");
    }
}
