
use crate::oczy::{Oczy, Okres};
use crate::rama::Strona;
use crate::wejscie::{Koszyk, Rachunek};

/// Ile cech liczy [`cechy`]. Stała, bo wektor wag musi mieć tę samą długość,
/// a niezgodność ma być błędem kompilacji, nie cichym przesunięciem indeksów.
pub const N: usize = 43;

/// Nazwy cech — w TEJ SAMEJ kolejności co wektor. Służą do wypisania modelu
/// w postaci czytelnej dla człowieka: „która obserwacja ile waży" jest
/// jedynym powodem, dla którego wybraliśmy model liniowy zamiast większego.
pub const NAZWY: [&str; N] = [
    "stala",
    // POŁOŻENIE w zakresie — gdzie jesteśmy
    "poz_m1",
    "poz_m5",
    "poz_m15",
    "poz_h1",
    "poz_h4",
    "poz_d1",
    "poz_w1",
    // TREND — dokąd idziemy, nachyleniem z regresji, nie różnicą dwóch punktów
    "trend_m5",
    "trend_m15",
    "trend_h1",
    "trend_h4",
    "trend_d1",
    // STRUKTURA — gdzie leży opór i wsparcie, w jednostkach zmienności
    "do_oporu_m15",
    "do_wsparcia_m15",
    "do_oporu_h1",
    "do_wsparcia_h1",
    "do_oporu_h4",
    "do_wsparcia_h4",
    // SERIE — jednostajność ruchu, której nachylenie nie oddaje
    "seria_m5",
    "seria_h1",
    // TEMPO i KOSZT
    "zmiana_m5_znorm",
    "zmiana_h1_znorm",
    "zakres_m5_do_h1",
    "spread_do_zakresu",
    // NASZA POZYCJA
    "wynik_do_ryzyka",
    "oddane_ze_szczytu",
    "wiek_do_doby",
    "nog_znorm",
    "glebokosc_srednia",
    // POZIOMY, KTÓRE WIDZI CAŁY RYNEK (wzorzec `DailyHighLow`)
    "do_szczytu_wczoraj",
    "do_dolka_wczoraj",
    "do_szczytu_tydzien",
    // REŻIM: konsolidacja czy impuls (wzorzec pudełek Darvasa i „squeeze")
    "rezim_m15",
    "rezim_h1",
    // ODCHYLENIE od średniej (rdzeń kopert Nadaraya-Watsona i Kijun-Sen)
    "odchyl_m15",
    "odchyl_h1",
    // BLOK ZLECEŃ (wzorzec `OrderBlock`, rodzina „smart money")
    "do_bloku_h1",
    // WŁASNE ZDANIE EA kontra ZDANIE KANAŁU — informacja, której nie da się
    // wyprowadzić ani z sygnału, ani z rynku osobno
    "zgodnosc_z_sygnalem",
    "konflikt_silny",
    "zgodnosc_silna",
    // RACHUNEK
    "margines_znorm",
    "dzien_znorm",
];

/// Położenie ceny w zakresie N ostatnich domkniętych świec, ustawione tak,
/// żeby 1 zawsze znaczyło „daleko w NASZĄ stronę".
///
/// Lustrzane odbicie dla SELL robimy TUTAJ, raz, zamiast w każdej regule
/// z osobna — inaczej pierwsza zapomniana gałąź daje politykę, która dla
/// jednej strony rynku działa odwrotnie, niż ktokolwiek zamierzał.
#[inline]
fn poz(o: &Oczy, okres: Okres, n: usize, mid: f64, strona: Strona) -> f64 {
    let p = o.okno(okres).polozenie_w_zakresie(n, mid);
    if !p.is_finite() {
        return 0.5;
    }
    match strona {
        Strona::Buy => p,
        Strona::Sell => 1.0 - p,
    }
}

/// Nachylenie trendu, ustawione tak, żeby DODATNIE zawsze znaczyło „w naszą
/// stronę". Lustrzane odbicie dla SELL robimy raz, tutaj — inaczej pierwsza
/// zapomniana gałąź daje politykę działającą odwrotnie dla jednej strony rynku.
#[inline]
fn tr(o: &Oczy, okres: Okres, n: usize, strona: Strona) -> f64 {
    let t = o.okno(okres).trend(n);
    if !t.is_finite() {
        return 0.0;
    }
    (t * strona.znak()).clamp(-2.0, 2.0) / 2.0
}

/// Odległość do oporu (przed nami) w jednostkach ZMIENNOŚCI, nie w dolarach.
///
/// „Trzy dolary do oporu" znaczy co innego w spokojny wtorek i co innego po
/// danych z USA. Dzielimy więc przez średni zakres świecy tego okresu — wtedy
/// liczba mówi „ile świec ruchu dzieli nas od przeszkody", i to jest
/// porównywalne między reżimami.
///
/// Brak potwierdzonego swingu daje 1,0, czyli „daleko" — bo brak przeszkody
/// w pamięci jest bliższy „nie ma przeszkody" niż „stoimy na niej".
#[inline]
fn do_oporu(o: &Oczy, okres: Okres, r: usize, mid: f64, strona: Strona) -> f64 {
    let w = o.okno(okres);
    let poziom = match strona {
        Strona::Buy => w.swing_high(r),
        Strona::Sell => w.swing_low(r),
    };
    let skala = w.sredni_zakres(10);
    if !poziom.is_finite() || !skala.is_finite() || skala <= 0.0 {
        return 1.0;
    }
    (((poziom - mid) * strona.znak()) / skala).clamp(-2.0, 4.0) / 4.0
}

/// Odległość do wsparcia (za nami) — lustrzane do [`do_oporu`].
#[inline]
fn do_wsparcia(o: &Oczy, okres: Okres, r: usize, mid: f64, strona: Strona) -> f64 {
    let w = o.okno(okres);
    let poziom = match strona {
        Strona::Buy => w.swing_low(r),
        Strona::Sell => w.swing_high(r),
    };
    let skala = w.sredni_zakres(10);
    if !poziom.is_finite() || !skala.is_finite() || skala <= 0.0 {
        return 1.0;
    }
    (((mid - poziom) * strona.znak()) / skala).clamp(-2.0, 4.0) / 4.0
}

/// Seria świec w naszą stronę, znormalizowana. Dodatnia = ruch jednostajny
/// w naszą stronę, ujemna = jednostajnie przeciw.
#[inline]
fn ser(o: &Oczy, okres: Okres, strona: Strona) -> f64 {
    let s = o.okno(okres).seria() as f64 * strona.znak();
    (s / 6.0).clamp(-1.0, 1.0)
}

/// Odległość do poziomu z poprzedniej świecy okresu, w jednostkach zmienności
/// i w orientacji „w naszą stronę".
#[inline]
fn do_poziomu(o: &Oczy, okres: Okres, gorny: bool, mid: f64, strona: Strona) -> f64 {
    let w = o.okno(okres);
    let (poziom, sk) = match (w.poprzednia(), w.sredni_zakres(5)) {
        (Some(p), s) if s.is_finite() && s > 0.0 => (if gorny { p.h } else { p.l }, s),
        _ => return 0.0,
    };
    (((poziom - mid) * strona.znak()) / sk).clamp(-4.0, 4.0) / 4.0
}

/// Wektor obserwacji w chwili decyzji.
///
/// `ryzyko_ceny` to odległość wejścia od stopa — jednostka, w której mierzymy
/// zysk. Dzięki niej „zarobiliśmy 40 $" zamienia się w „zarobiliśmy tyle, ile
/// ryzykowaliśmy", co jest wielkością porównywalną między sygnałami o różnej
/// geometrii i między saldami.
pub fn cechy(o: &Oczy, k: &Koszyk, r: &Rachunek, ts: i64, ryzyko_ceny: f64) -> [f64; N] {
    let mid = o.mid();
    let strona = k.geometria.strona;
    let m5 = o.okno(Okres::M5);
    let h1 = o.okno(Okres::H1);

    // Zakres świecy jako skala normalizująca: ruch „duży" w spokojny wtorek
    // i „duży" po danych z USA to dwie różne wielkości, więc dzielimy przez
    // to, ile rynek się rusza TERAZ.
    let zakres_m5 = m5.sredni_zakres(5);
    let zakres_h1 = h1.sredni_zakres(5);
    let sk_m5 = if zakres_m5.is_finite() && zakres_m5 > 0.0 {
        zakres_m5
    } else {
        1.0
    };
    let sk_h1 = if zakres_h1.is_finite() && zakres_h1 > 0.0 {
        zakres_h1
    } else {
        1.0
    };

    let zm_m5 = m5.zmiana(3);
    let zm_h1 = h1.zmiana(3);
    let znak = strona.znak();

    let wynik = k.wynik_usd();
    let ryz = if ryzyko_ceny > 0.0 {
        ryzyko_ceny
    } else {
        f64::NAN
    };
    // Wynik w jednostkach ryzyka. `tanh` ścina ogony: pozycja na +12 R i na
    // +40 R to dla decyzji „trzymać czy wyjść" prawie to samo, a bez ścięcia
    // jedna taka obserwacja przeważyłaby setkę zwykłych.
    let wynik_r = if ryz.is_finite() && k.wolumen() > 0.0 {
        (wynik / (ryz * 100.0 * k.wolumen())).tanh()
    } else {
        0.0
    };

    let oddane = k
        .najlepszy()
        .map(|s| s.oddane_ze_szczytu())
        .filter(|x| x.is_finite())
        .unwrap_or(0.0)
        .clamp(-1.0, 1.0);

    let wiek = ((ts - k.ts_zawiazania) as f64 / 86_400_000.0).clamp(0.0, 4.0) / 4.0;
    let nog = (k.szczeble.len() as f64 / 8.0).clamp(0.0, 1.0);
    let gl = if k.szczeble.is_empty() {
        0.0
    } else {
        k.szczeble.iter().map(|s| s.glebokosc as f64).sum::<f64>() / (k.szczeble.len() as f64 * 8.0)
    };
    // Poziom marginesu: 1 = komfort (≥300 %), 0 = ściana (100 %).
    let mar = ((r.poziom() - 100.0) / 200.0).clamp(0.0, 1.0);
    // Wynik dnia w jednostkach salda, ścięty — żeby jeden dzień z ogonem nie
    // zdominował wagi tej cechy.
    let dzien = (r.wynik_dnia_usd / r.saldo.max(1.0)).tanh();

    [
        1.0, // wyraz wolny — pozwala modelowi mieć własne „domyślnie trzymaj"
        poz(o, Okres::M1, 5, mid, strona),
        poz(o, Okres::M5, 3, mid, strona),
        poz(o, Okres::M15, 3, mid, strona),
        poz(o, Okres::H1, 5, mid, strona),
        poz(o, Okres::H4, 5, mid, strona),
        poz(o, Okres::D1, 5, mid, strona),
        poz(o, Okres::W1, 3, mid, strona),
        tr(o, Okres::M5, 8, strona),
        tr(o, Okres::M15, 8, strona),
        tr(o, Okres::H1, 8, strona),
        tr(o, Okres::H4, 8, strona),
        tr(o, Okres::D1, 5, strona),
        do_oporu(o, Okres::M15, 2, mid, strona),
        do_wsparcia(o, Okres::M15, 2, mid, strona),
        do_oporu(o, Okres::H1, 2, mid, strona),
        do_wsparcia(o, Okres::H1, 2, mid, strona),
        do_oporu(o, Okres::H4, 2, mid, strona),
        do_wsparcia(o, Okres::H4, 2, mid, strona),
        ser(o, Okres::M5, strona),
        ser(o, Okres::H1, strona),
        (zm_m5 * znak / sk_m5).clamp(-3.0, 3.0) / 3.0,
        (zm_h1 * znak / sk_h1).clamp(-3.0, 3.0) / 3.0,
        (sk_m5 / sk_h1).clamp(0.0, 2.0) / 2.0,
        (o.spread() / sk_m5).clamp(0.0, 1.0),
        wynik_r,
        oddane,
        wiek,
        nog,
        gl,
        do_poziomu(o, Okres::D1, true, mid, strona),
        do_poziomu(o, Okres::D1, false, mid, strona),
        do_poziomu(o, Okres::W1, true, mid, strona),
        {
            let r = o.okno(Okres::M15).rezim(5);
            if r.is_finite() {
                (r.clamp(0.0, 3.0)) / 3.0
            } else {
                0.33
            }
        },
        {
            let r = o.okno(Okres::H1).rezim(5);
            if r.is_finite() {
                (r.clamp(0.0, 3.0)) / 3.0
            } else {
                0.33
            }
        },
        {
            let d = o.okno(Okres::M15).odchylenie(10, mid);
            if d.is_finite() {
                (d * znak).clamp(-3.0, 3.0) / 3.0
            } else {
                0.0
            }
        },
        {
            let d = o.okno(Okres::H1).odchylenie(10, mid);
            if d.is_finite() {
                (d * znak).clamp(-3.0, 3.0) / 3.0
            } else {
                0.0
            }
        },
        {
            let w = o.okno(Okres::H1);
            let b = w.blok_zlecen(matches!(strona, Strona::Buy));
            let sk = w.sredni_zakres(10);
            if b.is_finite() && sk.is_finite() && sk > 0.0 {
                (((mid - b) * znak) / sk).clamp(-4.0, 4.0) / 4.0
            } else {
                0.0
            }
        },
        {
            // +1 = rynek idzie DOKŁADNIE tam, gdzie mówi kanał, −1 = przeciw.
            let k = o.wlasny_kierunek() * znak;
            k.clamp(-1.0, 1.0)
        },
        {
            // Konflikt SILNY: rynek wyraźnie przeciw sygnałowi. Osobna cecha,
            // bo model liniowy inaczej nie odróżni „lekko przeciw" od
            // „zdecydowanie przeciw" — a to są dwie różne sytuacje.
            let k = o.wlasny_kierunek() * znak;
            if k < -0.3 {
                1.0
            } else {
                0.0
            }
        },
        {
            let k = o.wlasny_kierunek() * znak;
            if k > 0.3 {
                1.0
            } else {
                0.0
            }
        },
        mar,
        dzien,
    ]
}

#[derive(Debug, Clone, Copy)]
pub struct Model {
    pub w: [f64; N],
    /// próg decyzji na wyniku
    pub prog: f64,
}

impl Default for Model {
    fn default() -> Self {
        // ZEROWE WAGI = model, który zawsze mówi to samo. To jest kontrakt
        // zera wyrażony liczbami: nienauczony model nie ma prawa niczego
        // zmieniać, a nie „ma przypadkowe zdanie".
        Model {
            w: [0.0; N],
            prog: 0.0,
        }
    }
}

impl Model {
    #[inline]
    pub fn wynik(&self, c: &[f64; N]) -> f64 {
        let mut s = 0.0;
        for i in 0..N {
            s += self.w[i] * c[i];
        }
        s
    }
    #[inline]
    pub fn decyzja(&self, c: &[f64; N]) -> bool {
        self.wynik(c) > self.prog
    }
    /// Czy model jest nauczony. Same zera znaczą „nikt go nie uczył" i wtedy
    /// polityka MUSI zachować się jak bez modelu, zamiast zgadywać.
    pub fn nauczony(&self) -> bool {
        self.w.iter().any(|x| *x != 0.0)
    }
}
