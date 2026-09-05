
use crate::cien::ArbiterCieniowy;
use std::sync::{Mutex, OnceLock};

struct Globalny {
    arb: Mutex<ArbiterCieniowy>,
    sciezka: String,
}

static G: OnceLock<Option<Globalny>> = OnceLock::new();

#[inline]
fn g() -> Option<&'static Globalny> {
    G.get_or_init(|| {
        let p = std::env::var("MOZG_CIEN").ok()?;
        if p.trim().is_empty() {
            return None;
        }
        Some(Globalny {
            arb: Mutex::new(ArbiterCieniowy::nowy()),
            sciezka: p,
        })
    })
    .as_ref()
}

/// Czy arbiter cieniowy jest włączony. JEDEN odczyt `OnceLock`.
#[inline]
pub fn czynny() -> bool {
    g().is_some()
}

/// Granica pulsu: `on_tick` albo `on_message`.
#[inline]
pub fn puls(ts: i64, rodzaj: u8) {
    if let Some(x) = g() {
        if let Ok(mut a) = x.arb.lock() {
            a.puls(ts, rodzaj);
        }
    }
}

/// Zgłoszenie zamiaru przez pisarza. `linia` to linia WOŁAJĄCEGO dla lejków
/// (`#[track_caller]`), `0` dla pisarzy bezpośrednich.
#[inline]
pub fn z(akt: u8, id: u64, zrodlo: u16, linia: u32) {
    if let Some(x) = g() {
        if let Ok(mut a) = x.arb.lock() {
            a.zamiar(akt, id, zrodlo, linia);
        }
    }
}

/// Czy dwie ceny (albo ich brak) są RÓŻNE.
///
/// `None` znaczy „bez poziomu" i różni się od każdej liczby — zdjęcie stopu
/// jest zapisem tak samo jak jego przesunięcie. Próg `1e-9` jest progiem
/// tożsamości bitowej, nie tolerancją handlową.
#[inline]
pub fn rozne_px(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (None, None) => false,
        (Some(x), Some(y)) => (x - y).abs() > 1e-9,
        _ => true,
    }
}

/// Domyka ostatni puls i zapisuje raport pod ścieżką z `MOZG_CIEN`.
///
/// Zwraca ścieżkę, jeśli coś zapisano. Wołane z końca przebiegu; wołanie
/// wielokrotne jest bezpieczne (raport jest przepisywany od nowa).
pub fn zapisz_raport(naglowek: &str) -> Option<String> {
    let x = g()?;
    let tekst = {
        let mut a = x.arb.lock().ok()?;
        a.raport(naglowek)
    };
    std::fs::write(&x.sciezka, tekst).ok()?;
    Some(x.sciezka.clone())
}
