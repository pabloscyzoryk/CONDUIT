//! Predykcja SZCZYTU i rozpoznawanie BIEGACZY.
//!
//! # Po co
//!
//! Moduł bada lukę pomiędzy wczesnym wyjściem a późniejszym szczytem zyskownej
//! pozycji. Celem jest rozpoznanie runnerów bez udawania, że każdą stratną
//! pozycję da się uratować.
//!
//! Stąd dwie głowy, obie na tych samych cechach:
//!
//! | głowa | pytanie | etykieta |
//! |---|---|---|
//! | regresyjna | ile jeszcze zostało do szczytu | `max(wynik w oknie h) − wynik teraz` |
//! | klasyfikacyjna | czy ta pozycja POBIEGNIE | `max(wynik od teraz) ≥ N × ryzyko` |
//!
//! Głowa klasyfikacyjna odpowiada na koncentrację wyniku w nielicznych dużych
//! ruchach. To nie jest zadanie „kiedy wyjść z każdej pozycji", tylko „czy ta
//! jest jedną z nielicznych, które pobiegną daleko".
//! Jeśli model to rozpozna, polityka jest banalna: biegaczy trzymamy bez celu,
//! resztę zamykamy na TP1.
//!
//! # Etykieta — dlaczego okno CZASOWE, a nie „do wyjścia"
//!
//! Etykieta zależna od momentu wyjścia zależałaby od polityki, którą właśnie
//! uczymy: model uczyłby się przewidywać własne zachowanie, a każda iteracja
//! zmieniałaby zbiór treningowy pod sobą. Tutaj etykieta jest funkcją wyłącznie
//! rynku.
//!
//! # PUŁAPKA WYROCZNI, której tu nie ma
//!
//! W `etykiety_wyroczni.json` pola `tp1_ts … sl_ts` liczone są od chwili
//! SYGNAŁU, nie od wejścia — 949 z 1 644 wypełnień ma TP1 „trafiony" przed
//! własnym wejściem, bo limit stoi po drugiej stronie celu. Ten generator nie
//! ma jak w to wpaść: ścieżka ZACZYNA SIĘ w chwili pierwszego wypełnienia i
//! wszystko — szczyt, TP1, SL — jest mierzone po wejściu. Odpowiednik pól
//! `*_po_wejsciu`, tyle że gęsty (próbka co `krok_s`), a nie jeden na sygnał.
//!
//! # Ścieżka to KOSZYK, nie pojedyncza pozycja
//!
//! Czempion wchodzi siatką (3 jednostki, strefa zwężona o 2 $ i pogłębiona
//! o 3 $, `sl_min_dist 3`) i decyduje o całości. Ścieżka odwzorowuje tę siatkę,
//! dzięki czemu cechy koszykowe — łączny wynik, liczba i odległość
//! niezrealizowanych limitów — są prawdziwe, a nie doklejone.

use crate::obs::MarketWindow;
use crate::policy::{bce_logits, mse, BackScratch, Grads, Mlp, Scratch};
use conduit_backtest::{RawSignal, TickData};
use conduit_core::types::*;
use serde::{Deserialize, Serialize};

pub const POSLIZG_OCZEKUJACE_USD: f64 = 0.092;

/// Swap za dobę trzymania, w dolarach na 0,01 lota.
///
/// Wartości poniżej są przykładowym profilem kosztów używanym przez narzędzie
/// badawcze. W środę rolowanie może być potrójne; przed użyciem podaj stawki
/// właściwe dla własnego rachunku i symbolu.
pub const SWAP_LONG_USD_DOBA: f64 = -0.7582;
pub const SWAP_SHORT_USD_DOBA: f64 = 0.2741;

/// Liczba cech opisujących stan ścieżki.
pub const F: usize = 30;
/// Horyzonty etykiety regresyjnej w minutach.
pub const HORYZONTY: [i64; 3] = [15, 60, 240];

pub const NAZWY_CECH: [&str; F] = [
    "wych_na_jedn",      // wynik koszyka na jednostkę / ATR
    "szczyt_dotad",      // najlepszy wynik od wejścia / ATR
    "min_od_wejscia",    // najgorszy wynik od wejścia / ATR
    "spadek_od_szczytu", // ile oddaliśmy od szczytu / ATR
    "dyst_do_sl",        // odległość ceny do SL / ATR
    "dyst_do_tp1",       // odległość ceny do TP1 / ATR
    "glebokosc_wejscia", // jak głęboko w strefie stoi średnie wejście
    "szerokosc_strefy",  // szerokość strefy / ATR
    "czas_od_wejscia",   // log-skala
    "czas_od_szczytu",   // log-skala
    "zmiennosc_5m",
    "zmiennosc_60m",
    "ret_1m",
    "ret_5m",
    "ret_15m",
    "ret_60m",
    "atr_wzgl", // ATR / cena × 1000
    "spread",   // spread / ATR
    "godz_sin",
    "godz_cos",
    // --- rodzina reguł tradera (te same wielkości, co w `Engine`) ---
    "spread_do_mediany", // spread / mediana spreadu − 1
    "czas_od_tp_hit",    // log-minuty od ostatniego komunikatu TP_HIT w kanale
    "zysk_w_R",          // wynik koszyka / ryzyko koszyka
    "dyst_okragly",      // odległość do okrągłego poziomu PRZED nami / ATR
    "koszyk_usd",        // łączny wynik koszyka w $ / 10
    "limity_ponizej",    // niezrealizowane limity po stronie straty / jednostki
    "dyst_limitu",       // odległość do najbliższego takiego limitu / ATR
    "otwarte_jedn",      // wypełnionych jednostek / jednostki
    "aktywnosc_24h",     // log z liczby sygnałów w kanale w ostatnich 24 h
    "do_konca_sesji",    // (18 − godzina) / 10
];

/// Jedna próbka: stan koszyka w chwili `ts` plus etykiety.
///
/// Wynik jest w DOLARACH przy 0,01 lota na jednostkę — na złocie 0,01 lota
/// zarabia 1 $ na 1 $ ruchu ceny, więc dolary i punkty są tu tożsame.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Probka {
    pub ts: Ts,
    pub x: Vec<f32>,
    /// zapas do szczytu koszyka dla każdego z [`HORYZONTY`], w dolarach
    pub y: [f32; 3],
    /// 1.0 = szczyt OD TERAZ do końca ścieżki przekroczy N × ryzyko koszyka
    pub y_kl: f32,
    pub szczyt_od_teraz: f32,
    /// bieżący wynik koszyka w dolarach
    pub wych: f32,
    /// bieżący wynik koszyka W TRYBIE CZEMPIONA: najgłębsza warstwa liczona po
    /// TP1, jeśli cel już padł, reszta po cenie rynkowej.
    ///
    /// Bez tego pola wycena czempiona w dowolnej chwili PRZED końcem ścieżki
    /// jest zawyżona — liczyłaby trzy warstwy tam, gdzie jedna jest już
    /// zamknięta. Na twardym czasie życia 12 h dawało to +7732 $ zamiast prawdy.
    pub wych_run: f32,
    /// bieżące ryzyko koszyka (Σ |wejście − SL|) w dolarach
    pub ryzyko: f32,
    /// ile jednostek jest otwartych
    pub otw: u8,
    /// identyfikator ścieżki (indeks sygnału)
    pub sciezka: u32,
}

/// Warianty **RISK FREE** — mechanizm, którego używa sam kanał.
///
/// Domykamy część zyskownych warstw koszyka tak, żeby zrealizowany zysk był
/// nieujemny, a resztę zostawiamy jako runnera ze stopem na **ważonej średniej
/// cenie wejścia tej reszty**. Taki runner nie może stracić: przy dotknięciu
/// stopu jego wynik to zero, a zrealizowany zysk zostaje w kieszeni.
///
/// To jest struktura **asymetryczna** — i tym różni się od wszystkiego, co
/// mierzyliśmy dotąd. Każda polityka symetryczna (TP1/TP2/TP3, zapadka, limit
/// czasu) na tej strukturze sygnału przegrywa.
///
/// `(wyzwalacz, ile najgłębszych warstw zostaje runnerem)`; wyzwalacz `0.0`
/// oznacza „w chwili dotknięcia TP1", wartość dodatnia — „gdy koszyk osiągnie
/// k × ryzyko".
/// Trzecie pole: `0` = runnerem zostaje warstwa NAJGŁĘBSZA (najlepsza cena),
/// `1` = NAJPŁYTSZA. Wariant `1` jest KONTROLĄ: skoro wszystkie warstwy
/// jednego koszyka mają tę samą przyszłą ścieżkę ceny, warstwa o lepszej cenie
/// wejścia dominuje każdą gorszą **arytmetycznie**. Jeśli różnica między tymi
/// wariantami jest duża, to znaczy, że „który runner" rozstrzyga rachunek,
/// a nie model.
/// Czwarte pole — TRYB, i to on rozstrzyga, czemu przypisać ewentualną przewagę:
///
/// | tryb | co robi |
/// |---|---|
/// | 0 | pełny RISK FREE: domknij część warstw z nieujemnym zyskiem + SL reszty na jej BE |
/// | 1 | **KONTROLA**: nie domykaj nic, tylko przesuń SL całego koszyka na jego BE |
/// | 2 | **KONTROLA**: nie domykaj nic, nie ruszaj SL — po prostu porzuć TP1 i trzymaj |
///
/// Bez trybów 1 i 2 nie da się odróżnić „domykanie warstw zarabia" od „samo
/// przesunięcie stopu na próg opłacalności zarabia" i od „samo porzucenie TP1
/// zarabia". To jest pułapka nr 5 z PROMPT0: najprostsza rzecz, która robi to samo.
pub const RF_WARIANTY: [(f64, usize, u8, u8); 12] = [
    (0.0, 1, 0, 0),
    (0.0, 2, 0, 0),
    (1.0, 1, 0, 0),
    (2.0, 1, 0, 0),
    (3.0, 1, 0, 0),
    (0.0, 1, 1, 0),
    (1.0, 1, 1, 0),
    (1.0, 0, 0, 1),
    (2.0, 0, 0, 1),
    (3.0, 0, 0, 1),
    (2.0, 0, 0, 2),
    (3.0, 0, 0, 2),
];

pub const RF_NAZWY: [&str; 12] = [
    "RF w TP1, runner = 1 NAJGŁĘBSZA",
    "RF w TP1, runner = 2 najgłębsze",
    "RF przy +1 R, runner = 1 najgłębsza",
    "RF przy +2 R, runner = 1 najgłębsza",
    "RF przy +3 R, runner = 1 najgłębsza",
    "KONTR. warstwy: RF w TP1, runner = NAJPŁYTSZA",
    "KONTR. warstwy: RF +1 R, runner = NAJPŁYTSZA",
    "KONTR. mech.: +1 R → sam SL na BE, nic nie domykaj",
    "KONTR. mech.: +2 R → sam SL na BE, nic nie domykaj",
    "KONTR. mech.: +3 R → sam SL na BE, nic nie domykaj",
    "KONTR. mech.: +2 R → samo porzucenie TP1 (SL bez zmian)",
    "KONTR. mech.: +3 R → samo porzucenie TP1 (SL bez zmian)",
];

/// Powód zakończenia ścieżki.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Powod {
    Sl,
    Horyzont,
}

/// Cała ścieżka jednego koszyka.
///
/// Wyniki polityk STAŁYCH są policzone dokładnie w chwili zdarzenia (dotknięcie
/// TP1, dotknięcie SL), a nie na siatce próbek co minutę — inaczej porównanie
/// modelu z odniesieniem mierzyłoby rozdzielczość próbkowania.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sciezka {
    pub sygnal: u32,
    /// kierunek koszyka — potrzebny do policzenia swapu
    pub side_buy: bool,
    pub id_sygnalu: i64,
    pub dzien: i64,
    pub probki: Vec<Probka>,
    /// wynik koszyka przy pierwszym dotknięciu TP1 po wejściu
    pub wych_tp1: Option<f32>,
    /// chwila pierwszego dotknięcia TP1 po wejściu
    pub ts_tp1: Option<Ts>,
    /// wynik koszyka na końcu ścieżki (SL albo horyzont)
    pub wych_koniec: f32,
    /// jedna jednostka na TP1, reszta bez celu — reguła czempiona
    pub wych_runner: f32,
    /// najlepszy wynik koszyka na całej ścieżce
    pub szczyt: f32,
    /// ryzyko koszyka po pierwszym wypełnieniu
    pub ryzyko_wej: f32,
    pub powod: Powod,
    /// wyniki wariantów [`RF_WARIANTY`] — liczone NA TICKACH, nie na siatce próbek
    pub rf: Vec<f32>,
    /// czy wariant w ogóle się wyzwolił (koszyk musiał mieć co domknąć)
    pub rf_ok: Vec<bool>,
    /// ile warstw siatki zostało wypełnionych na całej ścieżce
    pub wypelnionych: u8,
    /// chwile wypełnienia kolejnych warstw, w kolejności wypełniania.
    /// Potrzebne do pytania: czy TEMPO wypełniania w pierwszych minutach
    /// przewiduje, jak głęboko koszyk ostatecznie zejdzie.
    pub ts_wyp: Vec<Ts>,
}

impl Sciezka {
    /// Wynik polityki „wyjdź na TP1, a jak nie ma TP1 — na końcu".
    #[inline]
    pub fn tp1(&self) -> f32 {
        self.wych_tp1.unwrap_or(self.wych_koniec)
    }
    #[inline]
    pub fn ts0(&self) -> Ts {
        self.probki[0].ts
    }
}

// ============================================================
//  GENERATOR ETYKIET
// ============================================================

#[derive(Clone, Debug)]
pub struct GenCfg {
    /// co ile sekund pobieramy próbkę na ścieżce
    pub krok_s: i64,
    /// maksymalna długość ścieżki w godzinach
    pub horyzont_h: i64,
    /// rozgrzewka pamięci rynku przed wejściem (minuty)
    pub rozgrzewka_min: i64,
    /// przesunięcie zegara wiadomości względem ticków
    pub msg_offset_ms: i64,
    /// ile jednostek w siatce (czempion: 3)
    pub jednostki: usize,
    /// pogłębienie strefy w stronę lepszych wejść (czempion: 3 $)
    pub deep_off: f64,
    /// przesunięcie krawędzi gorszej (czempion: −2 $)
    pub tol_off: f64,
    /// minimalna odległość SL od środka strefy (czempion: 3 $)
    pub sl_min_dist: f64,
    /// krotność ryzyka definiująca BIEGACZA
    pub n_r: f64,
    /// filtr godzin serwera, np. `Some((8, 18))`
    pub sesja: Option<(u32, u32)>,
    /// KONTROLA PLACEBO: przesuń sygnał o tyle godzin i przenieś całą jego
    /// geometrię (strefa, SL, TP) o różnicę ceny między starą a nową chwilą.
    ///
    /// Reguła zostaje identyczna, kształt setupu zostaje identyczny, znika
    /// wyłącznie związek z sygnałem. Wariant, który zarabia tak samo na
    /// placebo, zarabia na dryfie próbki, nie na sygnale. Kontrola jest więc
    /// obowiązkowa przy każdej silnie kierunkowej próbce.
    pub placebo_h: i64,
}

impl Default for GenCfg {
    fn default() -> Self {
        GenCfg {
            krok_s: 60,
            horyzont_h: 6,
            rozgrzewka_min: 120,
            msg_offset_ms: 3 * 3_600_000,
            jednostki: 3,
            deep_off: 3.0,
            tol_off: -2.0,
            sl_min_dist: 3.0,
            n_r: 3.0,
            sesja: None,
            placebo_h: 0,
        }
    }
}

#[inline]
fn norm(x: f64) -> f32 {
    if x.is_nan() {
        0.0
    } else {
        x.clamp(-8.0, 8.0) as f32
    }
}

#[inline]
fn ln_min(m: f64) -> f64 {
    (1.0 + m.max(0.0)).ln() / 1441.0f64.ln()
}

/// Maksimum w oknie CZASOWYM do przodu: `out[i] = max(v[j])` dla `ts[j] ∈ [ts[i], ts[i]+h]`.
///
/// Kolejka monotoniczna, O(n). Wersja naiwna (skan po oknie) kosztowała na tych
/// danych rzędu 10¹⁰ operacji i to ona, a nie uczenie, była wąskim gardłem.
fn max_w_oknie(ts: &[Ts], v: &[f32], h_ms: i64) -> Vec<f32> {
    let n = v.len();
    let mut out = vec![f32::NEG_INFINITY; n];
    // odwracamy: okno w przód staje się oknem wstecz, czyli klasycznym
    // „sliding window maximum" z lewą granicą rosnącą monotonicznie
    let mut dq: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for k in 0..n {
        let i = n - 1 - k; // idziemy od końca
        while let Some(&b) = dq.back() {
            if v[b] <= v[i] {
                dq.pop_back();
            } else {
                break;
            }
        }
        dq.push_back(i);
        let kres = ts[i] + h_ms;
        while let Some(&f) = dq.front() {
            if ts[f] > kres {
                dq.pop_front();
            } else {
                break;
            }
        }
        out[i] = v[*dq.front().unwrap()];
    }
    out
}

/// Znaczniki komunikatów „cel trafiony" z całego kanału, posortowane.
///
/// Silnik trzyma `last_tp_hit_ts` i wstrzymuje reguły wyjścia zaraz po takim
/// komunikacie — model dostaje tę samą wielkość jako cechę.
pub fn znaczniki_tp_hit(sygnaly: &[RawSignal], msg_offset_ms: i64) -> Vec<Ts> {
    let mut v: Vec<Ts> = Vec::new();
    for s in sygnaly {
        for e in &s.events {
            if e.kind == "TP_HIT" {
                v.push(e.ts * 1000 + msg_offset_ms);
            }
        }
    }
    v.sort_unstable();
    v
}

/// Poziomy siatki wejścia — odwzorowanie `Engine::compute_zone` + drabinki.
fn poziomy(side: Side, lo: f64, hi: f64, cfg: &GenCfg) -> (f64, f64, Vec<f64>) {
    let (mut zlo, mut zhi) = (lo, hi);
    match side {
        Side::Buy => {
            zlo -= cfg.deep_off;
            zhi += cfg.tol_off;
        }
        Side::Sell => {
            zhi += cfg.deep_off;
            zlo -= cfg.tol_off;
        }
    }
    let (zlo, zhi) = (zlo.min(zhi), zlo.max(zhi));
    let u = cfg.jednostki.max(1);
    let mut p = Vec::with_capacity(u);
    if u == 1 {
        p.push(side.worse_edge(zlo, zhi));
    } else {
        for i in 0..u {
            let f = i as f64 / (u - 1) as f64;
            p.push(match side {
                // poziom 0 jest NAJGŁĘBSZY (najlepsza cena), ostatni najpłytszy
                Side::Buy => zlo + f * (zhi - zlo),
                Side::Sell => zhi - f * (zhi - zlo),
            });
        }
    }
    (zlo, zhi, p)
}

/// Buduje ścieżki koszyków ze wszystkich sygnałów.
pub fn generuj(td: &TickData, sygnaly: &[RawSignal], cfg: &GenCfg) -> Vec<Sciezka> {
    use rayon::prelude::*;

    let tp_hity = znaczniki_tp_hit(sygnaly, cfg.msg_offset_ms);
    let czasy_syg: Vec<Ts> = {
        let mut v: Vec<Ts> = sygnaly
            .iter()
            .map(|s| s.ts * 1000 + cfg.msg_offset_ms)
            .collect();
        v.sort_unstable();
        v
    };

    let mut out: Vec<Sciezka> = sygnaly
        .par_iter()
        .enumerate()
        .filter_map(|(si, s)| jedna_sciezka(td, s, si, cfg, &tp_hity, &czasy_syg))
        .collect();

    out.sort_by_key(|s| s.ts0());
    out
}

#[allow(clippy::too_many_lines)]
fn jedna_sciezka(
    td: &TickData,
    s: &RawSignal,
    si: usize,
    cfg: &GenCfg,
    tp_hity: &[Ts],
    czasy_syg: &[Ts],
) -> Option<Sciezka> {
    let side = if s.dir.eq_ignore_ascii_case("BUY") {
        Side::Buy
    } else {
        Side::Sell
    };
    let sgn = side.sign();
    let (mut lo, mut hi) = (s.lo.min(s.hi), s.lo.max(s.hi));
    let mut tp1 = *s.tps.first()?;
    let mut sl_syg = s.sl;

    // --- KONTROLA PLACEBO ---
    // Przesuwamy chwilę o `placebo_h` i przenosimy CAŁĄ geometrię o różnicę
    // ceny, żeby setup miał identyczny kształt względem rynku, a stracił
    // związek z sygnałem. Bez przeniesienia poziomów koszyk po prostu nigdy
    // by się nie wypełnił i „kontrola" mierzyłaby brak handlu.
    let mut przesuniecie_ms = 0i64;
    if cfg.placebo_h != 0 {
        let t_a = s.ts * 1000 + cfg.msg_offset_ms;
        let t_b = t_a + cfg.placebo_h * 3_600_000;
        let (i_a, i_b) = (td.index_at(t_a), td.index_at(t_b));
        if i_a >= td.len() || i_b >= td.len() {
            return None;
        }
        let delta = td.quote(i_b).mid() - td.quote(i_a).mid();
        lo += delta;
        hi += delta;
        tp1 += delta;
        sl_syg += delta;
        przesuniecie_ms = cfg.placebo_h * 3_600_000;
    }

    let (zlo, zhi, poz) = poziomy(side, lo, hi, cfg);

    // SL jak w silniku: nie bliżej niż `sl_min_dist` od środka strefy
    let mid = (zlo + zhi) * 0.5;
    let sl = match side {
        Side::Buy => sl_syg.min(mid - cfg.sl_min_dist),
        Side::Sell => sl_syg.max(mid + cfg.sl_min_dist),
    };
    // SL po złej stronie strefy = sygnał bez sensu
    if (zlo - sl) * sgn <= 0.0 && (zhi - sl) * sgn <= 0.0 {
        return None;
    }
    if (tp1 - zhi) * sgn <= 0.0 && (tp1 - zlo) * sgn <= 0.0 {
        return None;
    }

    let t0 = s.ts * 1000 + cfg.msg_offset_ms + przesuniecie_ms;
    if let Some((a, b)) = cfg.sesja {
        let g = hour_of(t0, 0);
        if g < a || g >= b {
            return None;
        }
    }
    let i_sig = td.index_at(t0);
    if i_sig >= td.len() {
        return None;
    }

    // --- rozgrzewka pamięci rynku ---
    let mut mw = MarketWindow::new();
    let i_warm = td.index_at(t0 - cfg.rozgrzewka_min * 60_000);
    let mut spread_buf: Vec<f64> = Vec::with_capacity(512);
    let mut spread_med = 0.0f64;
    for i in i_warm..i_sig {
        let q = td.quote(i);
        if q.bid > 0.0 && q.ask >= q.bid {
            mw.on_tick(&q);
            spread_buf.push(q.ask - q.bid);
            if spread_buf.len() >= 512 {
                spread_buf.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                spread_med = spread_buf[256];
                spread_buf.clear();
            }
        }
    }

    let i_end = td.index_at(t0 + cfg.horyzont_h * 3_600_000).min(td.len());
    let krok_ms = cfg.krok_s * 1000;
    let szer = (zhi - zlo).abs().max(0.01);

    // stan koszyka: (numer poziomu, cena wejścia). Poziom 0 jest NAJGŁĘBSZY i to
    // on dostaje TP1 w silniku (`official_counts = "1"`, `target_for_ex(0)`),
    // a nie ten, który wypełnił się pierwszy — cena wchodzi w strefę od
    // krawędzi płytkiej, więc kolejność wypełnień jest odwrotna do drabinki.
    let mut wypelnione: Vec<(usize, f64)> = Vec::with_capacity(poz.len());
    let mut ts_wyp: Vec<Ts> = Vec::with_capacity(poz.len());
    let mut czy_wyp = vec![false; poz.len()];
    let mut ts_wejscia: Ts = 0;
    let mut ryzyko_wej = 0.0f64;

    // surowa ścieżka (co tick, od pierwszego wypełnienia)
    let mut rts: Vec<Ts> = Vec::with_capacity(8192);
    let mut rw: Vec<f32> = Vec::with_capacity(8192);
    // próbki
    let mut prb: Vec<Probka> = Vec::with_capacity(512);
    let mut idx_prb: Vec<usize> = Vec::with_capacity(512); // pozycja próbki w `rts`

    let mut nast: Ts = 0;
    let mut szczyt = f64::NEG_INFINITY;
    let mut minim = f64::INFINITY;
    let mut ts_szczyt: Ts = 0;
    let mut wych_tp1: Option<f32> = None;
    let mut ts_tp1: Option<Ts> = None;
    let mut cena_koniec = 0.0f64;
    let mut powod = Powod::Horyzont;

    // --- stan RISK FREE, jeden komplet na wariant ---
    let nrf = RF_WARIANTY.len();
    let mut rf_faza = vec![0u8; nrf]; // 0 czeka · 1 runner biegnie · 2 zamknięte
    let mut rf_zysk = vec![0.0f64; nrf];
    let mut rf_be = vec![0.0f64; nrf];
    let mut rf_reszta: Vec<Vec<(usize, f64)>> = vec![Vec::new(); nrf];
    let mut rf_wynik = vec![0.0f64; nrf];
    let mut rf_ok = vec![false; nrf];

    for i in i_sig..i_end {
        let q = td.quote(i);
        if !(q.bid > 0.0 && q.ask >= q.bid) {
            continue;
        }
        mw.on_tick(&q);
        spread_buf.push(q.ask - q.bid);
        if spread_buf.len() >= 512 {
            spread_buf.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            spread_med = spread_buf[256];
            spread_buf.clear();
        }

        // --- wypełnienia limitów ---
        // Zlecenie limit wyzwala się ceną wejścia (BUY po ASK), a wypełnia po
        // SWOJEJ cenie — wariant zachowawczy, bo przy przeskoku dostalibyśmy
        // lepiej, nigdy gorzej.
        for (k, p) in poz.iter().enumerate() {
            if czy_wyp[k] {
                continue;
            }
            let dotyk = match side {
                Side::Buy => q.ask <= *p,
                Side::Sell => q.bid >= *p,
            };
            if dotyk {
                czy_wyp[k] = true;
                // Zlecenie WYZWALA się na poziomie, ale REALIZUJE z poślizgiem
                // — zawsze na naszą niekorzyść. Bez tego każda warstwa jest
                // warta 0,092 $ więcej niż w rzeczywistości.
                wypelnione.push((k, *p + POSLIZG_OCZEKUJACE_USD * sgn));
                ts_wyp.push(q.ts);
                if wypelnione.len() == 1 {
                    ts_wejscia = q.ts;
                    nast = q.ts;
                }
            }
        }
        if wypelnione.is_empty() {
            continue;
        }
        if ryzyko_wej == 0.0 {
            ryzyko_wej = (wypelnione[0].1 - sl).abs();
        }

        let cena = q.exit(side);
        cena_koniec = cena;
        let wych: f64 = wypelnione.iter().map(|o| (cena - o.1) * sgn).sum();
        let ryzyko: f64 = wypelnione
            .iter()
            .map(|o| (o.1 - sl).abs())
            .sum::<f64>()
            .max(1e-6);

        rts.push(q.ts);
        rw.push(wych as f32);
        if wych > szczyt {
            szczyt = wych;
            ts_szczyt = q.ts;
        }
        if wych < minim {
            minim = wych;
        }

        // --- TP1 po wejściu ---
        if wych_tp1.is_none() {
            let hit = match side {
                Side::Buy => q.bid >= tp1,
                Side::Sell => q.ask <= tp1,
            };
            if hit {
                wych_tp1 = Some(wych as f32);
                ts_tp1 = Some(q.ts);
            }
        }

        // --- SL kończy ścieżkę ---
        let sl_hit = match side {
            Side::Buy => q.bid <= sl,
            Side::Sell => q.ask >= sl,
        };

        // --- RISK FREE, liczony CO TICK ---
        //
        // Kolejność ma znaczenie: stop na średniej cenie wejścia (BE) leży
        // ZAWSZE bliżej rynku niż oryginalny SL, więc gdyby oba wypadły w tym
        // samym ticku, pierwszy zostałby trafiony BE. Sprawdzamy go przed
        // przerwaniem pętli na SL — inaczej runner „przeżywałby" własny stop.
        for (vi, (wyz, zostaw, ktory, tryb)) in RF_WARIANTY.iter().enumerate() {
            match rf_faza[vi] {
                0 => {
                    // dla trybu 0 musi zostać co domknąć: przynajmniej jedna
                    // warstwa ponad runnera. Tryby kontrolne nie domykają nic.
                    if (*tryb == 0 && wypelnione.len() > *zostaw) || *tryb > 0 {
                        let wyzwolony = if *wyz == 0.0 {
                            wych_tp1.is_some()
                        } else {
                            wych >= *wyz * ryzyko
                        };
                        if wyzwolony {
                            let mut w = wypelnione.clone();
                            w.sort_by_key(|o| o.0); // poziom 0 = najgłębszy, najlepsza cena
                            let (reszta, domyk): (Vec<_>, Vec<_>) = match *tryb {
                                0 if *ktory == 0 => (w[..*zostaw].to_vec(), w[*zostaw..].to_vec()),
                                0 => {
                                    let g = w.len() - *zostaw;
                                    (w[g..].to_vec(), w[..g].to_vec())
                                }
                                // tryby kontrolne: cały koszyk zostaje, nic nie domykamy
                                _ => (w.clone(), Vec::new()),
                            };
                            let zysk: f64 = domyk.iter().map(|o| (cena - o.1) * sgn).sum();
                            // warunek RISK FREE: domknięcie musi być nieujemne,
                            // inaczej „runner bez ryzyka" jest tylko nazwą
                            if zysk >= 0.0 {
                                rf_zysk[vi] = zysk;
                                rf_be[vi] = if *tryb == 2 {
                                    // KONTROLA: stop zostaje tam, gdzie był
                                    sl
                                } else {
                                    reszta.iter().map(|o| o.1).sum::<f64>() / reszta.len() as f64
                                };
                                rf_reszta[vi] = reszta;
                                rf_faza[vi] = 1;
                                rf_ok[vi] = true;
                            }
                        }
                    }
                }
                1 => {
                    let be_hit = match side {
                        Side::Buy => q.bid <= rf_be[vi],
                        Side::Sell => q.ask >= rf_be[vi],
                    };
                    if be_hit {
                        rf_wynik[vi] = rf_zysk[vi]
                            + rf_reszta[vi]
                                .iter()
                                .map(|o| (cena - o.1) * sgn)
                                .sum::<f64>();
                        rf_faza[vi] = 2;
                    }
                }
                _ => {}
            }
        }

        if q.ts >= nast {
            nast = q.ts + krok_ms;
            let atr = mw.atr().max(1e-6);
            let px = q.mid().max(1.0);
            let godz = hour_of(q.ts, 0) as f64;
            let ang = godz / 24.0 * std::f64::consts::TAU;
            let n_otw = wypelnione.len();
            let sr_wej = wypelnione.iter().map(|o| o.1).sum::<f64>() / n_otw as f64;

            // --- limity czekające PO STRONIE STRATY ---
            // Dla kupna: nasze niewypełnione limity POD ceną. Pozycja spadająca
            // w stronę własnej siatki to inna sytuacja niż spadająca w próżnię.
            let mut ile_ponizej = 0usize;
            let mut najbl = f64::INFINITY;
            for (k, p) in poz.iter().enumerate() {
                if czy_wyp[k] {
                    continue;
                }
                let d = match side {
                    Side::Buy => q.bid - *p,
                    Side::Sell => *p - q.ask,
                };
                if d > 0.0 {
                    ile_ponizej += 1;
                    if d < najbl {
                        najbl = d;
                    }
                }
            }

            // --- okrągły poziom PRZED nami (wielokrotność 10 $) ---
            let krok_okr = 10.0f64;
            let d_okr = {
                let nad = (cena / krok_okr).ceil() * krok_okr;
                let pod = (cena / krok_okr).floor() * krok_okr;
                match side {
                    Side::Buy => nad - cena,
                    Side::Sell => cena - pod,
                }
            };

            // --- czas od ostatniego komunikatu TP_HIT w kanale ---
            let od_tp = match tp_hity.partition_point(|t| *t <= q.ts) {
                0 => 1440.0,
                k => ((q.ts - tp_hity[k - 1]) as f64 / 60_000.0).min(1440.0),
            };
            // --- aktywność kanału w ostatnich 24 h ---
            let akt = {
                let a = czasy_syg.partition_point(|t| *t < q.ts - 86_400_000);
                let b = czasy_syg.partition_point(|t| *t <= q.ts);
                (b - a) as f64
            };

            let mut x = vec![0.0f32; F];
            x[0] = norm(wych / n_otw as f64 / atr);
            x[1] = norm(szczyt / n_otw as f64 / atr);
            x[2] = norm(minim / n_otw as f64 / atr);
            x[3] = norm((szczyt - wych) / n_otw as f64 / atr);
            x[4] = norm((cena - sl) * sgn / atr);
            x[5] = norm((tp1 - cena) * sgn / atr);
            x[6] = norm((side.worse_edge(zlo, zhi) - sr_wej) * sgn / szer);
            x[7] = norm(szer / atr);
            x[8] = norm(ln_min((q.ts - ts_wejscia) as f64 / 60_000.0));
            x[9] = norm(ln_min((q.ts - ts_szczyt) as f64 / 60_000.0));
            x[10] = norm(mw.vol(5) / atr);
            x[11] = norm(mw.vol(60) / atr);
            x[12] = norm(mw.ret(1) / atr);
            x[13] = norm(mw.ret(5) / atr);
            x[14] = norm(mw.ret(15) / atr);
            x[15] = norm(mw.ret(60) / atr);
            x[16] = norm(atr / px * 1000.0);
            x[17] = norm(q.spread() / atr);
            x[18] = norm(ang.sin());
            x[19] = norm(ang.cos());
            x[20] = norm(if spread_med > 1e-9 {
                q.spread() / spread_med - 1.0
            } else {
                0.0
            });
            x[21] = norm(ln_min(od_tp));
            x[22] = norm(wych / ryzyko);
            x[23] = norm(d_okr / atr);
            x[24] = norm(wych / 10.0);
            x[25] = norm(ile_ponizej as f64 / poz.len() as f64);
            x[26] = norm(if najbl.is_finite() { najbl / atr } else { 8.0 });
            x[27] = norm(n_otw as f64 / poz.len() as f64);
            x[28] = norm((1.0 + akt).ln() / 30.0f64.ln());
            x[29] = norm((18.0 - godz) / 10.0);

            // wycena w trybie czempiona: jedna jednostka (najgłębsza) siedzi
            // już na TP1, jeśli cel padł; reszta jest po cenie rynkowej
            let wych_run_biez: f64 = if wych_tp1.is_some() {
                let naj = wypelnione.iter().min_by_key(|o| o.0).unwrap().0;
                wypelnione
                    .iter()
                    .map(|(lv, we)| {
                        if *lv == naj {
                            (tp1 - we) * sgn
                        } else {
                            (cena - we) * sgn
                        }
                    })
                    .sum()
            } else {
                wych
            };

            idx_prb.push(rts.len() - 1);
            prb.push(Probka {
                ts: q.ts,
                x,
                y: [0.0; 3],
                y_kl: 0.0,
                szczyt_od_teraz: 0.0,
                wych: wych as f32,
                wych_run: wych_run_biez as f32,
                ryzyko: ryzyko as f32,
                otw: n_otw as u8,
                sciezka: si as u32,
            });
        }

        if sl_hit {
            powod = Powod::Sl;
            break;
        }
    }

    if prb.is_empty() || rw.is_empty() {
        return None;
    }

    // --- etykiety regresyjne: maksimum w oknie MINUS wynik bieżący ---
    let mut okna: Vec<Vec<f32>> = Vec::with_capacity(3);
    for h in HORYZONTY {
        okna.push(max_w_oknie(&rts, &rw, h * 60_000));
    }
    // --- etykieta klasyfikacyjna: szczyt OD TERAZ do końca ścieżki ---
    let mut sufiks = vec![f32::NEG_INFINITY; rw.len()];
    {
        let mut mx = f32::NEG_INFINITY;
        for i in (0..rw.len()).rev() {
            if rw[i] > mx {
                mx = rw[i];
            }
            sufiks[i] = mx;
        }
    }
    for (k, p) in prb.iter_mut().enumerate() {
        let j = idx_prb[k];
        for hi_ in 0..3 {
            p.y[hi_] = okna[hi_][j] - p.wych;
        }
        p.y_kl = if (sufiks[j] as f64) >= cfg.n_r * p.ryzyko as f64 {
            1.0
        } else {
            0.0
        };
        p.szczyt_od_teraz = sufiks[j];
    }

    // --- wyniki polityk stałych, liczone dokładnie ---
    let wych_koniec: f32 = wypelnione
        .iter()
        .map(|o| ((cena_koniec - o.1) * sgn) as f32)
        .sum();
    // czempion: JEDNA jednostka (najgłębsza) bierze TP1, reszta biegnie bez celu
    let wych_runner: f32 = match wych_tp1 {
        None => wych_koniec,
        Some(_) => {
            let naj = wypelnione.iter().min_by_key(|o| o.0).unwrap().0;
            let mut suma = 0.0f64;
            for (lv, cena_we) in &wypelnione {
                suma += if *lv == naj {
                    (tp1 - cena_we) * sgn
                } else {
                    (cena_koniec - cena_we) * sgn
                };
            }
            suma as f32
        }
    };

    // wyniki RISK FREE: faza 0 = nigdy się nie wyzwolił, więc gra jak „bez celu"
    let rf: Vec<f32> = (0..nrf)
        .map(|vi| match rf_faza[vi] {
            0 => wych_koniec,
            1 => {
                (rf_zysk[vi]
                    + rf_reszta[vi]
                        .iter()
                        .map(|o| (cena_koniec - o.1) * sgn)
                        .sum::<f64>()) as f32
            }
            _ => rf_wynik[vi] as f32,
        })
        .collect();

    Some(Sciezka {
        sygnal: si as u32,
        side_buy: side == Side::Buy,
        id_sygnalu: s.id,
        dzien: prb[0].ts.div_euclid(86_400_000),
        rf,
        rf_ok,
        wypelnionych: wypelnione.len() as u8,
        ts_wyp,
        wych_tp1,
        ts_tp1,
        wych_koniec,
        wych_runner,
        szczyt: szczyt as f32,
        ryzyko_wej: ryzyko_wej as f32,
        powod,
        probki: prb,
    })
}

/// Swap naliczony między dwiema chwilami, w dolarach na JEDNĄ warstwę 0,01 lota.
///
/// Naliczany za każde przekroczenie północy czasu serwera (znaczniki są już
/// w czasie serwera po przesunięciu `msg_offset_ms`). Rolowanie ze środy na
/// czwartek liczy się potrójnie.
///
/// 1970-01-01 był czwartkiem, więc dla numeru doby `d` dzień PRZED północą
/// jest środą dokładnie wtedy, gdy `d % 7 == 0`.
pub fn swap_usd(ts_wejscia: Ts, ts_wyjscia: Ts, buy: bool) -> f64 {
    if ts_wyjscia <= ts_wejscia {
        return 0.0;
    }
    let d0 = ts_wejscia.div_euclid(86_400_000);
    let d1 = ts_wyjscia.div_euclid(86_400_000);
    let stawka = if buy {
        SWAP_LONG_USD_DOBA
    } else {
        SWAP_SHORT_USD_DOBA
    };
    let mut suma = 0.0;
    for d in (d0 + 1)..=d1 {
        suma += stawka * if d.rem_euclid(7) == 0 { 3.0 } else { 1.0 };
    }
    suma
}

// ============================================================
//  PODZIAŁ CZASOWY
// ============================================================

/// Podział CHRONOLOGICZNY na poziomie ŚCIEŻEK: uczenie / strojenie / test.
///
/// Ścieżka trafia w całości tam, gdzie się zaczyna — pozycja nie może być
/// rozcięta między uczenie i test. Podział losowy po próbkach dawałby wyciek:
/// sąsiednie próbki tej samej pozycji są niemal identyczne.
pub fn podziel_sciezki(sc: &[Sciezka], frac_ucz: f64, frac_stroj: f64) -> (usize, usize) {
    let n = sc.len();
    let a = (n as f64 * frac_ucz) as usize;
    let b = a + (n as f64 * frac_stroj) as usize;
    (a.min(n), b.min(n))
}

/// Spłaszcza ścieżki do listy próbek (kolejność ścieżek zachowana).
pub fn plaskie(sc: &[Sciezka]) -> Vec<&Probka> {
    sc.iter().flat_map(|s| s.probki.iter()).collect()
}

// ============================================================
//  MODELE LINIOWE (ODNIESIENIE)
// ============================================================

/// Regresja grzbietowa rozwiązywana równaniami normalnymi.
///
/// Model liniowy jest OBOWIĄZKOWYM punktem odniesienia: jeśli sieć go nie bije
/// poza próbą, to znaczy, że się przeucza, a nie że znalazła coś głębszego.
pub struct Ridge {
    pub w: Vec<f64>,
    pub b: f64,
}

impl Ridge {
    pub fn ucz(x: &[Vec<f32>], y: &[f32], alpha: f64) -> Ridge {
        let n = x.len();
        let d = F;
        let m = d + 1;
        let mut a = vec![0.0f64; m * m];
        let mut rhs = vec![0.0f64; m];
        for k in 0..n {
            let mut v = vec![0.0f64; m];
            for i in 0..d {
                v[i] = x[k][i] as f64;
            }
            v[d] = 1.0;
            for i in 0..m {
                for j in 0..m {
                    a[i * m + j] += v[i] * v[j];
                }
                rhs[i] += v[i] * y[k] as f64;
            }
        }
        for i in 0..d {
            a[i * m + i] += alpha; // wyrazu wolnego nie regularyzujemy
        }
        let sol = rozwiaz(&mut a, &mut rhs, m);
        Ridge {
            w: sol[..d].to_vec(),
            b: sol[d],
        }
    }

    #[inline]
    pub fn pred(&self, x: &[f32]) -> f32 {
        let mut s = self.b;
        for i in 0..F {
            s += self.w[i] * x[i] as f64;
        }
        s as f32
    }
}

/// Regresja logistyczna — odniesienie dla GŁOWY KLASYFIKACYJNEJ.
///
/// Ta sama zasada, co przy regresji: sieć, która nie bije modelu liniowego poza
/// próbą, jest przeuczona. Uczenie zwykłym spadkiem gradientu z L2; zbiór jest
/// mały (rzędu 10⁵ × 30), więc nie ma po co sięgać po nic sprytniejszego.
pub struct RegLog {
    pub w: Vec<f64>,
    pub b: f64,
}

impl RegLog {
    pub fn ucz(x: &[Vec<f32>], y: &[f32], kroki: usize, lr: f64, l2: f64) -> RegLog {
        let mut w = vec![0.0f64; F];
        let mut b = 0.0f64;
        let n = x.len().max(1) as f64;
        for _ in 0..kroki {
            let mut gw = vec![0.0f64; F];
            let mut gb = 0.0f64;
            for k in 0..x.len() {
                let mut z = b;
                for i in 0..F {
                    z += w[i] * x[k][i] as f64;
                }
                let p = 1.0 / (1.0 + (-z).exp());
                let d = p - y[k] as f64;
                for i in 0..F {
                    gw[i] += d * x[k][i] as f64;
                }
                gb += d;
            }
            for i in 0..F {
                w[i] -= lr * (gw[i] / n + l2 * w[i]);
            }
            b -= lr * gb / n;
        }
        RegLog { w, b }
    }

    #[inline]
    pub fn pred(&self, x: &[f32]) -> f32 {
        let mut z = self.b;
        for i in 0..F {
            z += self.w[i] * x[i] as f64;
        }
        (1.0 / (1.0 + (-z).exp())) as f32
    }
}

/// Eliminacja Gaussa z częściowym wyborem elementu głównego.
fn rozwiaz(a: &mut [f64], b: &mut [f64], m: usize) -> Vec<f64> {
    for col in 0..m {
        let mut piv = col;
        for r in col + 1..m {
            if a[r * m + col].abs() > a[piv * m + col].abs() {
                piv = r;
            }
        }
        if piv != col {
            for c in 0..m {
                a.swap(col * m + c, piv * m + c);
            }
            b.swap(col, piv);
        }
        let d = a[col * m + col];
        if d.abs() < 1e-12 {
            continue;
        }
        for r in col + 1..m {
            let f = a[r * m + col] / d;
            if f == 0.0 {
                continue;
            }
            for c in col..m {
                a[r * m + c] -= f * a[col * m + c];
            }
            b[r] -= f * b[col];
        }
    }
    let mut x = vec![0.0f64; m];
    for r in (0..m).rev() {
        let mut s = b[r];
        for c in r + 1..m {
            s -= a[r * m + c] * x[c];
        }
        let d = a[r * m + r];
        x[r] = if d.abs() < 1e-12 { 0.0 } else { s / d };
    }
    x
}

// ============================================================
//  MIARY
// ============================================================

/// Współczynnik determinacji.
pub fn r2(pred: &[f32], y: &[f32]) -> f64 {
    let n = y.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let sr = y.iter().map(|v| *v as f64).sum::<f64>() / n;
    let ss_tot: f64 = y.iter().map(|v| (*v as f64 - sr).powi(2)).sum();
    let ss_res: f64 = pred
        .iter()
        .zip(y)
        .map(|(p, v)| (*p as f64 - *v as f64).powi(2))
        .sum();
    if ss_tot < 1e-12 {
        0.0
    } else {
        1.0 - ss_res / ss_tot
    }
}

pub fn korelacja(pred: &[f32], y: &[f32]) -> f64 {
    let n = y.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let (mp, my) = (
        pred.iter().map(|v| *v as f64).sum::<f64>() / n,
        y.iter().map(|v| *v as f64).sum::<f64>() / n,
    );
    let mut num = 0.0;
    let mut dp = 0.0;
    let mut dy = 0.0;
    for i in 0..y.len() {
        let a = pred[i] as f64 - mp;
        let b = y[i] as f64 - my;
        num += a * b;
        dp += a * a;
        dy += b * b;
    }
    if dp < 1e-12 || dy < 1e-12 {
        0.0
    } else {
        num / (dp * dy).sqrt()
    }
}

/// Pole pod krzywą ROC, liczone na rangach (odporne na remisy).
pub fn auc(score: &[f32], y: &[f32]) -> f64 {
    let n = score.len();
    if n < 2 {
        return 0.5;
    }
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|a, b| {
        score[*a]
            .partial_cmp(&score[*b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut rangi = vec![0.0f64; n];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && score[idx[j + 1]] == score[idx[i]] {
            j += 1;
        }
        let r = (i + j) as f64 / 2.0 + 1.0;
        for k in i..=j {
            rangi[idx[k]] = r;
        }
        i = j + 1;
    }
    let np: f64 = y.iter().filter(|v| **v > 0.5).count() as f64;
    let nn = n as f64 - np;
    if np < 1.0 || nn < 1.0 {
        return 0.5;
    }
    let suma: f64 = (0..n).filter(|k| y[*k] > 0.5).map(|k| rangi[k]).sum();
    (suma - np * (np + 1.0) / 2.0) / (np * nn)
}

// ============================================================
//  GŁOWA REGRESYJNA (SIEĆ)
// ============================================================

/// Normalizacja etykiety.
///
/// Wyjście sieci jest LINIOWE, więc nienormalizowana etykieta o rozrzucie
/// rzędu dolarów potrafi rozbiegać uczenie do NaN. Skalujemy do odchylenia
/// jednostkowego i cofamy przy predykcji.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Skala {
    pub sr: f32,
    pub sd: f32,
}

impl Skala {
    pub fn z(y: &[f32]) -> Skala {
        let n = y.len().max(1) as f32;
        let sr = y.iter().sum::<f32>() / n;
        let war = y.iter().map(|v| (v - sr) * (v - sr)).sum::<f32>() / n;
        Skala {
            sr,
            sd: war.sqrt().max(1e-6),
        }
    }
    #[inline]
    pub fn wprzod(&self, v: f32) -> f32 {
        (v - self.sr) / self.sd
    }
    #[inline]
    pub fn wstecz(&self, v: f32) -> f32 {
        v * self.sd + self.sr
    }
}

fn init_siec(ukryte: &[usize], wyj: usize, seed: u64) -> Mlp {
    use rand::SeedableRng;
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
    let mut dims = vec![F];
    dims.extend_from_slice(ukryte);
    dims.push(wyj);
    let mut m = Mlp::init(&dims, &vec![0.0; wyj], &mut rng);
    // warstwa wyjściowa z `init` jest wyzerowana — potrzebne małe, ale
    // niezerowe wagi, inaczej pierwszy krok jest ślepy
    let l = m.w.len() - 1;
    for (k, v) in m.w[l].iter_mut().enumerate() {
        *v = ((k as f32 * 0.31).sin()) * 0.05;
    }
    m
}

/// Uczy sieć przewidywać zapas do szczytu dla wszystkich trzech horyzontów.
pub fn ucz_siec(
    x: &[Vec<f32>],
    y: &[[f32; 3]],
    ukryte: &[usize],
    epok: usize,
    lr: f32,
    seed: u64,
) -> (Mlp, [Skala; 3]) {
    let mut m = init_siec(ukryte, 3, seed);
    let sk = [
        Skala::z(&y.iter().map(|v| v[0]).collect::<Vec<_>>()),
        Skala::z(&y.iter().map(|v| v[1]).collect::<Vec<_>>()),
        Skala::z(&y.iter().map(|v| v[2]).collect::<Vec<_>>()),
    ];

    let mut bs = BackScratch::for_net(&m);
    let mut g = Grads::zeros_like(&m);
    let n = x.len();
    let porcja = 256usize;

    for _ in 0..epok {
        let mut i = 0;
        while i < n {
            let kres = (i + porcja).min(n);
            g.clear();
            for k in i..kres {
                m.forward_cached(&x[k], &mut bs);
                let cel = [
                    sk[0].wprzod(y[k][0]),
                    sk[1].wprzod(y[k][1]),
                    sk[2].wprzod(y[k][2]),
                ];
                let mut d = [0.0f32; 3];
                mse(bs.out(), &cel, &mut d);
                m.backward(&mut bs, &d, &mut g);
            }
            g.scale(1.0 / (kres - i) as f32);
            for li in 0..m.w.len() {
                for k in 0..m.w[li].len() {
                    m.w[li][k] -= lr * g.w[li][k];
                }
                for k in 0..m.b[li].len() {
                    m.b[li][k] -= lr * g.b[li][k];
                }
            }
            i = kres;
        }
    }
    (m, sk)
}

/// Predykcja zapasu dla wybranego horyzontu, w dolarach.
pub fn pred_siec(m: &Mlp, sk: &[Skala; 3], s: &mut Scratch, x: &[f32], h: usize) -> f32 {
    m.forward(x, s);
    sk[h].wstecz(s.out[h])
}

// ============================================================
//  GŁOWA KLASYFIKACYJNA (SIEĆ)
// ============================================================

/// Uczy sieć rozpoznawać BIEGACZA: czy szczyt od teraz przekroczy N × ryzyko.
///
/// Klasa dodatnia jest w mniejszości, więc próbki ważymy odwrotnie do
/// liczebności klas. Bez tego sieć zbiega do „nigdy nie biegnie" — a to jest
/// dokładnie ta odpowiedź, która kosztuje 74 % luki.
pub fn ucz_klas(
    x: &[Vec<f32>],
    y: &[f32],
    ukryte: &[usize],
    epok: usize,
    lr: f32,
    seed: u64,
) -> Mlp {
    let mut m = init_siec(ukryte, 1, seed);
    let np = y.iter().filter(|v| **v > 0.5).count().max(1) as f32;
    let nn = (y.len() - y.iter().filter(|v| **v > 0.5).count()).max(1) as f32;
    let (wp, wn) = (0.5 * y.len() as f32 / np, 0.5 * y.len() as f32 / nn);

    let mut bs = BackScratch::for_net(&m);
    let mut g = Grads::zeros_like(&m);
    let n = x.len();
    let porcja = 256usize;

    for _ in 0..epok {
        let mut i = 0;
        while i < n {
            let kres = (i + porcja).min(n);
            g.clear();
            let mut waga_sum = 0.0f32;
            for k in i..kres {
                m.forward_cached(&x[k], &mut bs);
                let mut d = [0.0f32; 1];
                bce_logits(bs.out(), &y[k..k + 1], &mut d);
                let w = if y[k] > 0.5 { wp } else { wn };
                d[0] *= w;
                waga_sum += w;
                m.backward(&mut bs, &d, &mut g);
            }
            g.scale(1.0 / waga_sum.max(1e-6));
            for li in 0..m.w.len() {
                for k in 0..m.w[li].len() {
                    m.w[li][k] -= lr * g.w[li][k];
                }
                for k in 0..m.b[li].len() {
                    m.b[li][k] -= lr * g.b[li][k];
                }
            }
            i = kres;
        }
    }
    m
}

/// Prawdopodobieństwo biegacza.
pub fn pred_klas(m: &Mlp, s: &mut Scratch, x: &[f32]) -> f32 {
    m.forward(x, s);
    1.0 / (1.0 + (-s.out[0]).exp())
}

// ============================================================
//  POLITYKI WYJŚCIA
// ============================================================

/// Wynik polityki: dolary dzień po dniu.
pub struct Wynik {
    pub pnl: f64,
    pub n: usize,
    pub trafien: f64,
    pub pf: f64,
    pub dni_plus: f64,
    pub maxdd: f64,
    pub per_dzien: Vec<(i64, f64)>,
}

pub fn podsumuj(wyn: &[(i64, f64)]) -> Wynik {
    let pnl: f64 = wyn.iter().map(|v| v.1).sum();
    let zysk: f64 = wyn.iter().map(|v| v.1).filter(|v| *v > 0.0).sum();
    let strata: f64 = wyn
        .iter()
        .map(|v| v.1)
        .filter(|v| *v < 0.0)
        .sum::<f64>()
        .abs();
    let trafien = if wyn.is_empty() {
        0.0
    } else {
        wyn.iter().filter(|v| v.1 > 0.0).count() as f64 / wyn.len() as f64 * 100.0
    };
    let mut dni: std::collections::BTreeMap<i64, f64> = std::collections::BTreeMap::new();
    for (d, v) in wyn {
        *dni.entry(*d).or_insert(0.0) += v;
    }
    let dodatnie = dni.values().filter(|v| **v > 0.0).count();
    let dni_plus = if dni.is_empty() {
        0.0
    } else {
        dodatnie as f64 / dni.len() as f64 * 100.0
    };
    let (mut eq, mut szczyt, mut mdd) = (0.0f64, 0.0f64, 0.0f64);
    for v in dni.values() {
        eq += v;
        if eq > szczyt {
            szczyt = eq;
        }
        if szczyt - eq > mdd {
            mdd = szczyt - eq;
        }
    }
    Wynik {
        pnl,
        n: wyn.len(),
        trafien,
        pf: if strata > 1e-9 { zysk / strata } else { 999.0 },
        dni_plus,
        maxdd: mdd,
        per_dzien: wyn.to_vec(),
    }
}

/// Wszystko na TP1 (albo koniec ścieżki, jeśli TP1 nie padł).
pub fn pol_tp1(sc: &[Sciezka]) -> Vec<(i64, f64)> {
    sc.iter().map(|s| (s.dzien, s.tp1() as f64)).collect()
}

/// Nic nie ma celu — trzymamy do SL albo do końca horyzontu.
pub fn pol_bez_celu(sc: &[Sciezka]) -> Vec<(i64, f64)> {
    sc.iter().map(|s| (s.dzien, s.wych_koniec as f64)).collect()
}

/// Reguła czempiona: jedna jednostka bierze TP1, reszta biegnie bez celu.
pub fn pol_runner(sc: &[Sciezka]) -> Vec<(i64, f64)> {
    sc.iter().map(|s| (s.dzien, s.wych_runner as f64)).collect()
}

/// SUFIT rodziny hybrydowej: doskonała wiedza, którą ścieżkę puścić.
pub fn pol_wyrocznia_hybryda(sc: &[Sciezka]) -> Vec<(i64, f64)> {
    sc.iter()
        .map(|s| (s.dzien, s.tp1().max(s.wych_koniec) as f64))
        .collect()
}

/// SUFIT bezwzględny: wyjście dokładnie w szczycie.
pub fn pol_szczyt(sc: &[Sciezka]) -> Vec<(i64, f64)> {
    sc.iter().map(|s| (s.dzien, s.szczyt as f64)).collect()
}

/// Regresja: wychodzimy na pierwszej próbce, gdzie przewidywany zapas < próg.
pub fn pol_regresja<P: FnMut(&Probka) -> f32>(
    sc: &[Sciezka],
    mut pred: P,
    prog: f32,
) -> Vec<(i64, f64)> {
    sc.iter()
        .map(|s| {
            for p in &s.probki {
                if pred(p) < prog {
                    return (s.dzien, p.wych as f64);
                }
            }
            (s.dzien, s.wych_koniec as f64)
        })
        .collect()
}

/// HYBRYDA — decyzja jednorazowa, przy wejściu.
///
/// „Biegacza puszczamy bez celu, resztę zamykamy na TP1". Decyzja zapada na
/// PIERWSZEJ próbce ścieżki i nie jest już zmieniana — najprostsza możliwa
/// forma i jedyna, której nie da się przestroić po fakcie.
pub fn pol_hybryda<P: FnMut(&Probka) -> f32>(
    sc: &[Sciezka],
    mut pred: P,
    prog: f32,
) -> Vec<(i64, f64)> {
    sc.iter()
        .map(|s| {
            let p = pred(&s.probki[0]);
            (
                s.dzien,
                if p >= prog {
                    s.wych_koniec as f64
                } else {
                    s.tp1() as f64
                },
            )
        })
        .collect()
}

/// HYBRYDA CIĄGŁA — domyślnie TP1, ale zdanie można zmienić do chwili TP1.
pub fn pol_hybryda_ciagla<P: FnMut(&Probka) -> f32>(
    sc: &[Sciezka],
    mut pred: P,
    prog: f32,
) -> Vec<(i64, f64)> {
    sc.iter()
        .map(|s| {
            for p in &s.probki {
                // po dotknięciu TP1 pozycja jest już zamknięta — decyzja musi
                // zapaść WCZEŚNIEJ, inaczej model „zmienia zdanie" po fakcie
                if let Some(t) = s.ts_tp1 {
                    if p.ts > t {
                        break;
                    }
                }
                if pred(p) >= prog {
                    return (s.dzien, s.wych_koniec as f64);
                }
            }
            (s.dzien, s.tp1() as f64)
        })
        .collect()
}

/// Wynik wariantu RISK FREE (indeks w [`RF_WARIANTY`]).
pub fn pol_rf(sc: &[Sciezka], vi: usize) -> Vec<(i64, f64)> {
    sc.iter().map(|s| (s.dzien, s.rf[vi] as f64)).collect()
}

/// RISK FREE tam, gdzie się wyzwolił; gdzie nie — zwykłe TP1.
///
/// Bez tego wariantu nie da się rozdzielić „struktura zarabia" od „struktura
/// po prostu rzadziej gra". Wariant czysty (`pol_rf`) w ścieżkach bez
/// wyzwolenia trzyma pozycję bez celu, co jest osobną, mocną decyzją.
pub fn pol_rf_lub_tp1(sc: &[Sciezka], vi: usize) -> Vec<(i64, f64)> {
    sc.iter()
        .map(|s| {
            (
                s.dzien,
                if s.rf_ok[vi] {
                    s.rf[vi] as f64
                } else {
                    s.tp1() as f64
                },
            )
        })
        .collect()
}

/// Stała zapadka: wyjście po oddaniu `pct` szczytu, aktywna od `start` dolarów.
pub fn pol_zapadka(sc: &[Sciezka], start: f32, pct: f32) -> Vec<(i64, f64)> {
    sc.iter()
        .map(|s| {
            let mut szczyt = f32::NEG_INFINITY;
            for p in &s.probki {
                if p.wych > szczyt {
                    szczyt = p.wych;
                }
                if szczyt >= start && p.wych <= szczyt * (1.0 - pct) {
                    return (s.dzien, p.wych as f64);
                }
            }
            (s.dzien, s.wych_koniec as f64)
        })
        .collect()
}

/// Limit czasu trzymania.
pub fn pol_czas(sc: &[Sciezka], minut: i64) -> Vec<(i64, f64)> {
    sc.iter()
        .map(|s| {
            let t0 = s.probki[0].ts;
            for p in &s.probki {
                if p.ts - t0 >= minut * 60_000 {
                    return (s.dzien, p.wych as f64);
                }
            }
            (s.dzien, s.wych_koniec as f64)
        })
        .collect()
}

/// Bootstrap PO DNIACH, nie po transakcjach.
///
/// Ścieżki z jednego dnia dzielą ten sam ruch rynku i są mocno skorelowane.
/// Losowanie pojedynczych transakcji udaje, że są niezależne, i daje przedział
/// sztucznie wąski — przy rozkładzie z tak ciężkim ogonem to różnica między
/// „przewaga istotna" a „nie wiadomo".
pub fn bootstrap_dni(wyniki: &[(i64, f64)], losowan: usize, seed: u64) -> (f64, f64) {
    let mut dni: std::collections::BTreeMap<i64, f64> = std::collections::BTreeMap::new();
    for (d, v) in wyniki {
        *dni.entry(*d).or_insert(0.0) += v;
    }
    let v: Vec<f64> = dni.values().cloned().collect();
    if v.len() < 3 {
        return (f64::NAN, f64::NAN);
    }
    let mut st = seed | 1;
    let mut nast = || {
        st ^= st >> 12;
        st ^= st << 25;
        st ^= st >> 27;
        st.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let mut sumy = Vec::with_capacity(losowan);
    for _ in 0..losowan {
        let mut s = 0.0;
        for _ in 0..v.len() {
            s += v[(nast() % v.len() as u64) as usize];
        }
        sumy.push(s);
    }
    sumy.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (sumy[losowan / 40], sumy[losowan - 1 - losowan / 40])
}

/// Bootstrap PO DNIACH dla RÓŻNICY dwóch polityk na tych samych ścieżkach.
///
/// Różnica sparowana ma dużo mniejszą wariancję niż różnica dwóch osobnych
/// przedziałów — i tylko ona odpowiada na pytanie „czy model dokłada".
pub fn bootstrap_roznicy(
    a: &[(i64, f64)],
    b: &[(i64, f64)],
    losowan: usize,
    seed: u64,
) -> (f64, f64) {
    assert_eq!(a.len(), b.len());
    let r: Vec<(i64, f64)> = a.iter().zip(b).map(|(x, y)| (x.0, x.1 - y.1)).collect();
    bootstrap_dni(&r, losowan, seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probka(ts: Ts, w: f32, sciezka: u32) -> Probka {
        Probka {
            ts,
            x: vec![0.0; F],
            y: [0.0; 3],
            y_kl: 0.0,
            szczyt_od_teraz: w,
            wych: w,
            wych_run: w,
            ryzyko: 6.0,
            otw: 1,
            sciezka,
        }
    }

    #[test]
    fn swap_nalicza_sie_za_kazda_polnoc_i_potraja_w_srode() {
        const D: Ts = 86_400_000;
        // w obrębie jednej doby — zero
        assert!((swap_usd(1000, 2000, true)).abs() < 1e-12);
        // jedna północ, doba docelowa d=1 (piątek) — stawka pojedyncza
        let a = swap_usd(D / 2, D + D / 2, true);
        assert!((a - SWAP_LONG_USD_DOBA).abs() < 1e-9, "{a}");
        // short płaci dodatnio
        let b = swap_usd(D / 2, D + D / 2, false);
        assert!((b - SWAP_SHORT_USD_DOBA).abs() < 1e-9, "{b}");
        // przekroczenie północy kończącej środę: doba docelowa d = 7 (d % 7 == 0)
        let c = swap_usd(6 * D + D / 2, 7 * D + D / 2, true);
        assert!(
            (c - 3.0 * SWAP_LONG_USD_DOBA).abs() < 1e-9,
            "środa ma być potrójna: {c}"
        );
        // swap jest kosztem dla BUY — znak ujemny
        assert!(swap_usd(0, 3 * D, true) < 0.0);
    }

    #[test]
    fn ridge_odtwarza_zaleznosc_liniowa() {
        let mut x = Vec::new();
        let mut y = Vec::new();
        for k in 0..500 {
            let mut v = vec![0.0f32; F];
            v[0] = (k as f32 * 0.017).sin();
            v[1] = (k as f32 * 0.031).cos();
            v[2] = (k % 7) as f32 * 0.1;
            y.push(2.0 * v[0] - 1.5 * v[1] + 0.5 * v[2] + 0.25);
            x.push(v);
        }
        let r = Ridge::ucz(&x, &y, 1e-6);
        let p: Vec<f32> = x.iter().map(|v| r.pred(v)).collect();
        assert!(r2(&p, &y) > 0.99, "R² {}", r2(&p, &y));
        assert!((r.w[0] - 2.0).abs() < 0.05, "w0 {}", r.w[0]);
        assert!((r.w[1] + 1.5).abs() < 0.05, "w1 {}", r.w[1]);
    }

    #[test]
    fn reglog_odtwarza_granice_liniowa() {
        let mut x = Vec::new();
        let mut y = Vec::new();
        for k in 0..600 {
            let mut v = vec![0.0f32; F];
            v[0] = ((k * 37) % 101) as f32 / 50.0 - 1.0;
            v[1] = ((k * 53) % 97) as f32 / 48.0 - 1.0;
            x.push(v.clone());
            y.push(if 1.5 * v[0] - v[1] > 0.0 { 1.0 } else { 0.0 });
        }
        let m = RegLog::ucz(&x, &y, 400, 2.0, 1e-4);
        let p: Vec<f32> = x.iter().map(|v| m.pred(v)).collect();
        assert!(auc(&p, &y) > 0.95, "AUC {}", auc(&p, &y));
    }

    #[test]
    fn auc_zachowuje_sie_sensownie() {
        let y = vec![0.0f32, 0.0, 1.0, 1.0];
        assert!((auc(&[0.1, 0.2, 0.8, 0.9], &y) - 1.0).abs() < 1e-9);
        assert!((auc(&[0.9, 0.8, 0.2, 0.1], &y) - 0.0).abs() < 1e-9);
        // stała predykcja = rzut monetą
        assert!((auc(&[0.5; 4], &y) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn skala_jest_odwracalna() {
        let y = vec![1.0f32, 2.5, -0.5, 4.0, 2.0];
        let s = Skala::z(&y);
        for v in &y {
            assert!((s.wstecz(s.wprzod(*v)) - v).abs() < 1e-5);
        }
        let z: Vec<f32> = y.iter().map(|v| s.wprzod(*v)).collect();
        let sk2 = Skala::z(&z);
        assert!((sk2.sd - 1.0).abs() < 1e-4, "sd {}", sk2.sd);
    }

    #[test]
    fn r2_i_korelacja_zachowuja_sie_sensownie() {
        let y = vec![1.0f32, 2.0, 3.0, 4.0];
        assert!((r2(&y, &y) - 1.0).abs() < 1e-6);
        assert!((korelacja(&y, &y) - 1.0).abs() < 1e-6);
        let odwr = vec![4.0f32, 3.0, 2.0, 1.0];
        assert!(korelacja(&odwr, &y) < -0.99);
        let stala = vec![2.5f32; 4];
        assert!(r2(&stala, &y).abs() < 1e-6);
    }

    #[test]
    fn siec_bije_stala_na_zaleznosci_nieliniowej() {
        let mut x = Vec::new();
        let mut y = Vec::new();
        for k in 0..800 {
            let mut v = vec![0.0f32; F];
            let a = (k as f32 * 0.013).sin();
            let b = (k as f32 * 0.021).cos();
            v[0] = a;
            v[1] = b;
            y.push([a * b * 2.0, a * b, a * a]);
            x.push(v);
        }
        let (m, sk) = ucz_siec(&x, &y, &[16, 16], 300, 0.05, 7);
        let mut s = Scratch::for_net(&m);
        let p: Vec<f32> = x.iter().map(|v| pred_siec(&m, &sk, &mut s, v, 0)).collect();
        let cel: Vec<f32> = y.iter().map(|v| v[0]).collect();
        let score = r2(&p, &cel);
        assert!(score > 0.5, "sieć nie nauczyła się iloczynu: R² {score:.4}");
    }

    #[test]
    fn klasyfikator_uczy_sie_xor() {
        // XOR jest nieliniowy: regresja logistyczna MUSI mieć AUC ≈ 0.5,
        // a sieć musi ją pobić. To jest test, który wyłapuje martwą głowę.
        let mut x = Vec::new();
        let mut y = Vec::new();
        for k in 0..1200 {
            let mut v = vec![0.0f32; F];
            let a = ((k * 37) % 101) as f32 / 50.0 - 1.0;
            let b = ((k * 53) % 97) as f32 / 48.0 - 1.0;
            v[0] = a;
            v[1] = b;
            x.push(v);
            y.push(if (a > 0.0) != (b > 0.0) { 1.0 } else { 0.0 });
        }
        let lin = RegLog::ucz(&x, &y, 300, 2.0, 1e-4);
        let pl: Vec<f32> = x.iter().map(|v| lin.pred(v)).collect();
        assert!(
            (auc(&pl, &y) - 0.5).abs() < 0.1,
            "liniowy nie powinien umieć XOR: {}",
            auc(&pl, &y)
        );

        let m = ucz_klas(&x, &y, &[24, 16], 400, 0.2, 3);
        let mut s = Scratch::for_net(&m);
        let pn: Vec<f32> = x.iter().map(|v| pred_klas(&m, &mut s, v)).collect();
        assert!(
            auc(&pn, &y) > 0.85,
            "sieć nie nauczyła się XOR: AUC {}",
            auc(&pn, &y)
        );
    }

    #[test]
    fn max_w_oknie_zgadza_sie_z_naiwnym() {
        let ts: Vec<Ts> = (0..300).map(|i| i as i64 * 1000).collect();
        let v: Vec<f32> = (0..300).map(|i| ((i as f32) * 0.7).sin() * 10.0).collect();
        for h in [0i64, 5_000, 37_000, 400_000] {
            let szybko = max_w_oknie(&ts, &v, h);
            for i in 0..ts.len() {
                let mut mx = f32::NEG_INFINITY;
                for j in i..ts.len() {
                    if ts[j] > ts[i] + h {
                        break;
                    }
                    mx = mx.max(v[j]);
                }
                assert!(
                    (szybko[i] - mx).abs() < 1e-6,
                    "h={h} i={i}: {} vs {mx}",
                    szybko[i]
                );
            }
        }
    }

    #[test]
    fn poziomy_odwzorowuja_siatke_czempiona() {
        let cfg = GenCfg::default(); // deep 3, tol −2, 3 jednostki
        let (zlo, zhi, p) = poziomy(Side::Buy, 100.0, 110.0, &cfg);
        assert!((zlo - 97.0).abs() < 1e-9, "dolna krawędź {zlo}");
        assert!((zhi - 108.0).abs() < 1e-9, "górna krawędź {zhi}");
        assert_eq!(p.len(), 3);
        // poziom 0 najgłębszy (najlepsza cena dla kupna)
        assert!(p[0] < p[1] && p[1] < p[2]);
        assert!((p[0] - 97.0).abs() < 1e-9 && (p[2] - 108.0).abs() < 1e-9);

        let (zlo2, zhi2, p2) = poziomy(Side::Sell, 100.0, 110.0, &cfg);
        assert!((zhi2 - 113.0).abs() < 1e-9 && (zlo2 - 102.0).abs() < 1e-9);
        assert!(p2[0] > p2[1] && p2[1] > p2[2]);
    }

    #[test]
    fn polityki_reaguja_na_prog() {
        // ścieżka: rośnie do 20, wraca do 2; TP1 padł na 5
        let probki: Vec<Probka> = [0.0f32, 5.0, 12.0, 20.0, 9.0, 2.0]
            .iter()
            .enumerate()
            .map(|(i, w)| probka(i as i64 * 60_000, *w, 0))
            .collect();
        let s = Sciezka {
            sygnal: 0,
            id_sygnalu: 1,
            dzien: 0,
            probki,
            wych_tp1: Some(5.0),
            ts_tp1: Some(60_000),
            wych_koniec: 2.0,
            wych_runner: 3.0,
            side_buy: true,
            szczyt: 20.0,
            ryzyko_wej: 6.0,
            powod: Powod::Horyzont,
            rf: vec![0.0; RF_WARIANTY.len()],
            rf_ok: vec![false; RF_WARIANTY.len()],
            wypelnionych: 1,
            ts_wyp: vec![0],
        };
        let sc = vec![s];
        assert_eq!(podsumuj(&pol_tp1(&sc)).pnl, 5.0);
        assert_eq!(podsumuj(&pol_bez_celu(&sc)).pnl, 2.0);
        assert_eq!(podsumuj(&pol_szczyt(&sc)).pnl, 20.0);
        // hybryda: próg niski → trzyma (2 $), próg wysoki → TP1 (5 $)
        assert_eq!(podsumuj(&pol_hybryda(&sc, |_| 1.0, 0.5)).pnl, 2.0);
        assert_eq!(podsumuj(&pol_hybryda(&sc, |_| 0.1, 0.5)).pnl, 5.0);
        // sufit hybrydy bierze lepszą z dwóch
        assert_eq!(podsumuj(&pol_wyrocznia_hybryda(&sc)).pnl, 5.0);
        // zapadka 50 % od 10 $: szczyt 20 → wyjście przy 9
        assert_eq!(podsumuj(&pol_zapadka(&sc, 10.0, 0.5)).pnl, 9.0);
    }

    #[test]
    fn podzial_jest_rozlaczny_i_chronologiczny() {
        let sc: Vec<Sciezka> = (0..100)
            .map(|i| Sciezka {
                sygnal: i,
                id_sygnalu: i as i64,
                dzien: i as i64,
                probki: vec![probka(i as i64 * 1000, 0.0, i)],
                wych_tp1: None,
                ts_tp1: None,
                wych_koniec: 0.0,
                wych_runner: 0.0,
                side_buy: true,
                szczyt: 0.0,
                ryzyko_wej: 6.0,
                powod: Powod::Sl,
                rf: vec![0.0; RF_WARIANTY.len()],
                rf_ok: vec![false; RF_WARIANTY.len()],
                wypelnionych: 1,
                ts_wyp: vec![0],
            })
            .collect();
        let (a, b) = podziel_sciezki(&sc, 0.5, 0.25);
        assert_eq!((a, b), (50, 75));
        assert!(
            sc[a - 1].ts0() < sc[a].ts0(),
            "części muszą być rozdzielone w czasie"
        );
        assert!(sc[b - 1].ts0() < sc[b].ts0());
    }

    #[test]
    fn bootstrap_roznicy_wychwytuje_stala_przewage() {
        // A jest o 1 $ lepsze od B na każdym dniu — przedział musi być cały > 0
        let a: Vec<(i64, f64)> = (0..40).map(|d| (d, 2.0)).collect();
        let b: Vec<(i64, f64)> = (0..40).map(|d| (d, 1.0)).collect();
        let (lo, hi) = bootstrap_roznicy(&a, &b, 400, 5);
        assert!(lo > 0.0 && hi > 0.0, "przedział {lo}..{hi}");
    }
}
