//! Kodowanie stanu bota w komentarzu zlecenia MT5.
//!
//! Po restarcie bota jedynym śladem po tym, CO to była za pozycja, jest to, co
//! zostało na koncie: `magic` i `comment`. `magic` mówi tylko „to nasze".
//! Numer koszyka, poziom siatki i znacznik touchera muszą zmieścić się
//! w komentarzu — inaczej rekoncyliacja odtworzy pozycje jako bezpańskie
//! i silnik przestanie nimi zarządzać jako grupą.
//!
//! Komentarz w MT5 to maksymalnie 31 znaków, a część brokerów obcina go jeszcze
//! mocniej albo dokleja własny sufiks. Dlatego część maszynowa idzie NA POCZĄTEK
//! i jest krótka; komentarz silnika (np. `B12`) doklejamy dopiero na końcu, jako
//! rzecz, którą wolno stracić.
//!
//! Format: `<TAG><basket>.<level>[t]`, np. `CD12.3`, `CD12.-1t`, `CDx.0`
//! (`x` = brak koszyka).

/// Domyślny znacznik. Krótki, bo każdy znak walczy o miejsce w 31 znakach.
pub const DEFAULT_TAG: &str = "CD";

pub const MAX_COMMENT: usize = 29;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tagged {
    pub basket: Option<u32>,
    pub level: i32,
    pub is_toucher: bool,
    /// oryginalny komentarz silnika, jeśli się zmieścił
    pub note: String,
}

/// Buduje komentarz do wysłania do brokera.
pub fn encode(tag: &str, basket: Option<u32>, level: i32, is_toucher: bool, note: &str) -> String {
    let mut s = String::with_capacity(MAX_COMMENT);
    s.push_str(tag);
    match basket {
        Some(b) => {
            use std::fmt::Write;
            let _ = write!(s, "{b}");
        }
        None => s.push('x'),
    }
    s.push('.');
    {
        use std::fmt::Write;
        let _ = write!(s, "{level}");
    }
    if is_toucher {
        s.push('t');
    }
    let note = note.trim();
    if !note.is_empty() && s.len() + 1 < MAX_COMMENT {
        s.push('-');
        for c in note.chars() {
            if s.len() >= MAX_COMMENT {
                break;
            }
            // MT5 nie lubi znaków spoza ASCII w komentarzu
            if c.is_ascii_alphanumeric() || c == '_' {
                s.push(c);
            }
        }
    }
    if s.len() > MAX_COMMENT {
        s.truncate(MAX_COMMENT);
    }
    s
}

/// Odczytuje stan z komentarza. `None`, gdy komentarz nie jest nasz.
///
/// Odporne na obcięcie ogona: wystarczy `<TAG><basket>.<level>`, reszta jest
/// opcjonalna. Odporne też na doklejenie sufiksu przez brokera (np. `[sl 4000]`),
/// bo liczby czytamy do pierwszego nie-pasującego znaku.
pub fn decode(tag: &str, comment: &str) -> Option<Tagged> {
    let rest = comment.strip_prefix(tag)?;
    let b = rest.as_bytes();

    // --- numer koszyka ---
    let mut i = 0;
    let basket = if b.first() == Some(&b'x') {
        i = 1;
        None
    } else {
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return None;
        }
        Some(rest[start..i].parse::<u32>().ok()?)
    };

    // --- separator ---
    if b.get(i) != Some(&b'.') {
        return None;
    }
    i += 1;

    // --- poziom (może być ujemny: toucher ma -1) ---
    let start = i;
    if b.get(i) == Some(&b'-') {
        i += 1;
    }
    let ds = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == ds {
        return None;
    }
    let level = rest[start..i].parse::<i32>().ok()?;

    // --- flaga touchera ---
    let is_toucher = b.get(i) == Some(&b't');
    if is_toucher {
        i += 1;
    }

    // --- notatka ---
    let note = if b.get(i) == Some(&b'-') {
        rest[i + 1..].to_string()
    } else {
        String::new()
    };

    Some(Tagged {
        basket,
        level,
        is_toucher,
        note,
    })
}

/// Czy komentarz w ogóle wygląda na nasz? (tańsze od pełnego `decode`)
#[inline]
pub fn is_ours(tag: &str, comment: &str) -> bool {
    comment.starts_with(tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn komentarz_nigdy_nie_przekracza_granicy_pakietu() {
        let znacznik = "TEST 2026-07-28 20:52"; // 21 znaków, realny przypadek
        for basket in [None, Some(1), Some(12), Some(999)] {
            for level in [-1, 0, 7] {
                let c = encode(znacznik, basket, level, level < 0, "panel");
                assert!(
                    c.len() <= MAX_COMMENT,
                    "komentarz {c:?} ma {} znaków, limit {MAX_COMMENT}",
                    c.len()
                );
                // znacznik użytkownika musi przetrwać w całości — po nim
                // użytkownik odróżnia swoje zlecenia od cudzych
                assert!(c.starts_with(znacznik), "znacznik przycięty: {c:?}");
            }
        }
    }

    #[test]
    fn runda_w_obie_strony() {
        let c = encode(DEFAULT_TAG, Some(12), 3, false, "B12");
        assert_eq!(c, "CD12.3-B12");
        let t = decode(DEFAULT_TAG, &c).unwrap();
        assert_eq!(t.basket, Some(12));
        assert_eq!(t.level, 3);
        assert!(!t.is_toucher);
        assert_eq!(t.note, "B12");
    }

    #[test]
    fn toucher_ma_poziom_ujemny_i_flage() {
        let c = encode(DEFAULT_TAG, Some(7), -1, true, "B7T");
        let t = decode(DEFAULT_TAG, &c).unwrap();
        assert_eq!(t.basket, Some(7));
        assert_eq!(t.level, -1);
        assert!(t.is_toucher);
    }

    #[test]
    fn brak_koszyka_zapisany_jako_x() {
        let c = encode(DEFAULT_TAG, None, 0, false, "");
        assert_eq!(c, "CDx.0");
        let t = decode(DEFAULT_TAG, &c).unwrap();
        assert_eq!(t.basket, None);
        assert_eq!(t.level, 0);
    }

    #[test]
    fn komentarz_nigdy_nie_przekracza_limitu_mt5() {
        let c = encode(
            DEFAULT_TAG,
            Some(4_294_967_295),
            -2_147_483_648,
            true,
            "bardzo_dluga_notatka_ktora_sie_nie_zmiesci",
        );
        assert!(c.len() <= MAX_COMMENT, "długość {} — {c}", c.len());
        // część maszynowa musi przetrwać obcięcie
        assert!(c.starts_with("CD4294967295.-2147483648"));
    }

    #[test]
    fn obciety_ogon_nadal_sie_czyta() {
        // broker uciął notatkę
        let t = decode(DEFAULT_TAG, "CD12.3").unwrap();
        assert_eq!(t.basket, Some(12));
        assert_eq!(t.level, 3);
        assert_eq!(t.note, "");
    }

    #[test]
    fn doklejony_sufiks_brokera_nie_psuje_odczytu() {
        let t = decode(DEFAULT_TAG, "CD12.3t[tp 4010.00]").unwrap();
        assert_eq!(t.basket, Some(12));
        assert_eq!(t.level, 3);
        assert!(t.is_toucher);
    }

    #[test]
    fn cudza_pozycja_nie_jest_nasza() {
        assert!(decode(DEFAULT_TAG, "ręczny scalping").is_none());
        assert!(decode(DEFAULT_TAG, "").is_none());
        assert!(!is_ours(DEFAULT_TAG, "XYZ12.3"));
    }

    #[test]
    fn urwany_prefiks_nie_daje_smieciowego_koszyka() {
        // sam tag, bez liczb — musi być None, nie Some(0)
        assert!(decode(DEFAULT_TAG, "CD").is_none());
        assert!(decode(DEFAULT_TAG, "CD12").is_none(), "brak separatora");
        assert!(decode(DEFAULT_TAG, "CD12.").is_none(), "brak poziomu");
        assert!(decode(DEFAULT_TAG, "CD12.-").is_none(), "sam minus");
    }

    #[test]
    fn wlasny_tag_dziala() {
        let c = encode("ZZ", Some(1), 0, false, "");
        assert_eq!(c, "ZZ1.0");
        assert!(decode("CD", &c).is_none());
        assert_eq!(decode("ZZ", &c).unwrap().basket, Some(1));
    }

    #[test]
    fn notatka_bez_znakow_spoza_ascii() {
        let c = encode(DEFAULT_TAG, Some(1), 0, false, "koszyk_żółć");
        assert!(c.is_ascii(), "komentarz {c}");
    }
}
