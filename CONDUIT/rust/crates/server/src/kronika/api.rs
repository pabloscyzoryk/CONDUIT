//! WSPÓLNY KSZTAŁT ODPOWIEDZI dla obu produktów.
//!
//! `kronika.exe` i Conduit wystawiają **ten sam** zestaw tras pod `/api/kronika`
//! i oddają **ten sam** JSON. Dzięki temu interfejs jest jeden komponent
//! Reacta, a nie dwa, które cicho się rozjeżdżają — dokładnie ten sam wzorzec,
//! co `AiModelsView` dzielony między `conduit.exe` a `wizualizacja.exe`.
//!
//! ```text
//!   GET  /api/kronika/stan          co się dzieje TERAZ (liczniki, podgląd)
//!   GET  /api/kronika/statystyki    liczby z PLIKU — w tym statystyka edycji
//!   GET  /api/kronika/kanaly        lista źródeł do zaznaczenia
//!   PUT  /api/kronika/ustawienia    opcje zapisu
//!   POST /api/kronika/eksport       zbiór backtestowy (wygoda, nie warunek)
//! ```
//!
//! Struktury są **tylko do serializacji**. Logika mieszka w [`crate::kronika::zapis`]
//! i [`crate::kronika::odczyt`]; tutaj jest wyłącznie kontrakt sieciowy.

use crate::kronika::{Liczniki, Ustawienia, Wpis};
use serde::Serialize;

/// Który produkt odpowiada. Interfejs pokazuje to w nagłówku, bo od tego
/// zależy, co użytkownik może zrobić: samodzielna kronika ma własną sesję
/// i własny cykl życia, wbudowana dzieli strumień z botem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tryb {
    /// `kronika.exe` — osobny proces, osobna sesja `kronika.session`
    Samodzielna,
    /// zakładka w Conduicie — podpięta pod istniejący strumień bota
    Wbudowana,
}

/// Jedno źródło do zaznaczenia na liście.
#[derive(Debug, Clone, Serialize)]
pub struct KanalInfo {
    pub chat_id: i64,
    pub nazwa: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    /// grupa z tematami — tematy są niezależnymi źródłami
    pub forum: bool,
    /// tematy forum, jeśli udało się je pobrać
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tematy: Vec<TematInfo>,
    /// czy bot NASŁUCHUJE tego kanału (Conduit; w trybie samodzielnym zawsze `false`)
    pub nasluchiwany: bool,
    /// czy kronika go NAGRYWA przy obecnych ustawieniach
    pub nagrywany: bool,
    /// ile wierszy tego źródła jest już w pliku
    pub wpisow: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TematInfo {
    pub id: i64,
    pub nazwa: String,
    pub nagrywany: bool,
    pub wpisow: u64,
}

/// Odpowiedź `GET /api/kronika/stan`.
#[derive(Debug, Clone, Serialize)]
pub struct Stan {
    pub ok: bool,
    pub wersja: String,
    pub tryb: Tryb,
    /// bezwzględna ścieżka jednego ciągłego pliku
    pub plik: String,
    /// Ścieżka, którą program wybrałby SAM (domyślnie plik na pulpicie).
    ///
    /// Panel ma z czego zbudować przycisk „przywróć domyślną", nie zgadując
    /// nazwy katalogu domowego cudzej maszyny — ścieżka jest ścieżką SERWERA,
    /// a panel bywa otwierany z innego komputera.
    #[serde(default)]
    pub domyslny_plik: String,
    pub istnieje: bool,
    /// Co zastano w pliku przy ostatnim otwarciu — „kontynuuję N wpisów"
    /// kontra „założyłem nowy". Bez tego jedno wygląda jak drugie.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rozpoznanie: Option<crate::kronika::Rozpoznanie>,
    /// Gdzie leży kopia zapasowa zrobiona przed pierwszym dopisaniem.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kopia: Option<String>,
    pub bajtow: u64,
    /// ile plików obejmuje kronika (bieżący + obrócone + stare układy)
    pub plikow: usize,
    pub ustawienia: Ustawienia,
    pub liczniki: Liczniki,
    /// ostatnie wiersze, od najnowszego
    pub ostatnie: Vec<Wpis>,
    /// czy jest z czego nagrywać
    pub zrodlo_zywe: bool,
    /// jednozdaniowy opis stanu źródła — panel ma powiedzieć DLACZEGO nic nie wpada
    pub zrodlo_opis: String,
}

/// Odpowiedź `POST /api/kronika/eksport` i `PUT /api/kronika/ustawienia`.
#[derive(Debug, Clone, Serialize)]
pub struct Potwierdzenie<T: Serialize> {
    pub ok: bool,
    #[serde(flatten)]
    pub wynik: T,
}

/// Liczy, ile wierszy w kronice ma każde źródło — do listy kanałów.
///
/// Świadomie z PLIKU, a nie z liczników procesu: użytkownik zaznacza kanały,
/// patrząc na to, ile z nich już zebrano, a licznik sesji po restarcie pokazuje
/// zero i wygląda, jakby kanał milczał.
pub fn wpisow_wg_zrodla(wpisy: &[Wpis]) -> std::collections::HashMap<(i64, Option<i64>), u64> {
    let mut m = std::collections::HashMap::new();
    for w in wpisy.iter().filter(|w| w.rodzaj.to_wiadomosc()) {
        *m.entry((w.chat_id, w.temat)).or_insert(0u64) += 1;
    }
    m
}

#[cfg(test)]
mod testy {
    use super::*;
    use crate::kronika::{Rodzaj, SCHEMAT};

    fn w(chat_id: i64, temat: Option<i64>, rodzaj: Rodzaj) -> Wpis {
        Wpis {
            v: SCHEMAT,
            seq: 1,
            rodzaj,
            odebrano_ms: 1,
            odebrano: String::new(),
            ts_telegram_ms: 1,
            chat_id,
            chat: "K".into(),
            temat,
            msg_id: 1,
            reply_to: None,
            edit_of: None,
            text: String::new(),
            znakow: 0,
            nasluchiwany: false,
            format: None,
            rozpoznane: false,
            uwaga: None,
            ts_telegram: None,
        }
    }

    #[test]
    fn liczenie_wpisow_rozdziela_tematy_i_pomija_znaczniki() {
        let dane = vec![
            w(-100, None, Rodzaj::Nowa),
            w(-100, Some(7), Rodzaj::Nowa),
            w(-100, Some(7), Rodzaj::Edycja),
            w(0, None, Rodzaj::Start),
        ];
        let m = wpisow_wg_zrodla(&dane);
        assert_eq!(m.get(&(-100, None)), Some(&1));
        assert_eq!(m.get(&(-100, Some(7))), Some(&2));
        assert_eq!(
            m.get(&(0, None)),
            None,
            "znacznik sesji nie jest wiadomością"
        );
    }
}
