
use std::collections::{HashMap, VecDeque};

use crate::types::{
    Account, Basket, BasketState, BasketView, Position, Px, Quote, Side, Ts, XAU_CONTRACT,
};

/// Wersja układu wektora. Rośnie przy KAŻDEJ zmianie kolejności lub znaczenia
/// cech. Model zapisuje ją razem z wagami; niezgodność = wagi do wyrzucenia.
pub const WERSJA_OBSERWACJI: u32 = 1;

/// Ile cech czysto losowych siedzi na końcu wektora.
pub const N_LOSOWYCH: usize = 20;

// ============================================================
//  METADANE CECH
// ============================================================

/// Rodzina cechy — tak, jak nazwał je właściciel projektu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rodzina {
    /// A — dynamika dojścia ceny do strefy
    Dynamika,
    /// B — stan rynku
    Rynek,
    /// C — struktura i trend
    Struktura,
    /// D — kontekst kanału i geometria sygnału
    Kanal,
    /// E — stan portfela
    Portfel,
    /// F — stan koszyka i pozycji
    Koszyk,
    /// kontrola: cecha czysto losowa, wyznacza próg istotności
    Losowa,
}

/// Warstwa wdrożeniowa. Model może startować od samego rdzenia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Warstwa {
    Rdzen,
    Rozszerzenie,
    /// kontrola losowa
    Kontrola,
}

/// Kiedy cecha w ogóle może mieć wartość.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dostepnosc {
    /// przy sygnale i przy zarządzaniu
    Zawsze,
    /// dopiero gdy istnieje koszyk (zarządzanie)
    TylkoKoszyk,
}

#[derive(Debug, Clone, Copy)]
pub struct OpisCechy {
    pub nazwa: &'static str,
    pub rodzina: Rodzina,
    pub warstwa: Warstwa,
    pub dostepnosc: Dostepnosc,
}

/// Generuje jednocześnie: indeksy (wariantami enuma), nazwy i metadane.
///
/// Dzięki temu nie da się dodać cechy i zapomnieć o jej nazwie ani przesunąć
/// nazwy względem indeksu — jedno i drugie pochodzi z tej samej linijki.
macro_rules! cechy {
    ($( $wariant:ident, $rodzina:ident, $warstwa:ident, $dost:ident );* $(;)?) => {
        /// Indeks cechy w wektorze. `C::nazwa as usize` = pozycja.
        #[allow(non_camel_case_types)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(usize)]
        pub enum C {
            $( $wariant, )*
            /// wartownik — nie jest cechą
            KONIEC,
        }

        /// Długość wektora obserwacji.
        pub const N_CECH: usize = C::KONIEC as usize;

        /// Metadane wszystkich cech, w kolejności wektora.
        pub static OPISY: &[OpisCechy] = &[
            $( OpisCechy {
                nazwa: stringify!($wariant),
                rodzina: Rodzina::$rodzina,
                warstwa: Warstwa::$warstwa,
                dostepnosc: Dostepnosc::$dost,
            }, )*
        ];
    };
}

cechy! {
    // ---------- B. STAN RYNKU ----------
    cena_mid_usd,                    Rynek,     Rozszerzenie, Zawsze;
    spread_usd,                      Rynek,     Rdzen,        Zawsze;
    spread_do_mediany,               Rynek,     Rdzen,        Zawsze;
    zakres_5m_usd,                   Rynek,     Rozszerzenie, Zawsze;
    zakres_60m_usd,                  Rynek,     Rdzen,        Zawsze;
    zakres_240m_usd,                 Rynek,     Rozszerzenie, Zawsze;
    zakres_1440m_usd,                Rynek,     Rozszerzenie, Zawsze;
    zmiennosc_5m_usd,                Rynek,     Rozszerzenie, Zawsze;
    zmiennosc_15m_usd,               Rynek,     Rdzen,        Zawsze;
    zmiennosc_60m_usd,               Rynek,     Rdzen,        Zawsze;
    zmiennosc_240m_usd,              Rynek,     Rozszerzenie, Zawsze;
    atr60_usd,                       Rynek,     Rdzen,        Zawsze;
    tickow_na_minute,                Rynek,     Rozszerzenie, Zawsze;
    minut_ciszy_rynku,               Rynek,     Rozszerzenie, Zawsze;
    od_otwarcia_dnia_usd,            Rynek,     Rozszerzenie, Zawsze;
    do_maks_dnia_usd,                Rynek,     Rozszerzenie, Zawsze;
    do_min_dnia_usd,                 Rynek,     Rozszerzenie, Zawsze;
    pozycja_w_zakresie_dnia,         Rynek,     Rozszerzenie, Zawsze;
    do_okraglego_10_usd,             Rynek,     Rozszerzenie, Zawsze;
    do_okraglego_50_usd,             Rynek,     Rozszerzenie, Zawsze;
    do_okraglego_100_usd,            Rynek,     Rozszerzenie, Zawsze;
    sesja_azja,                      Rynek,     Rozszerzenie, Zawsze;
    sesja_londyn,                    Rynek,     Rdzen,        Zawsze;
    sesja_ny,                        Rynek,     Rozszerzenie, Zawsze;
    nakladanie_ldn_ny,               Rynek,     Rozszerzenie, Zawsze;
    nakladanie_azja_ldn,             Rynek,     Rozszerzenie, Zawsze;
    min_do_konca_sesji,              Rynek,     Rozszerzenie, Zawsze;
    godz_sin,                        Rynek,     Rdzen,        Zawsze;
    godz_cos,                        Rynek,     Rdzen,        Zawsze;
    dzien_pon,                       Rynek,     Rozszerzenie, Zawsze;
    dzien_wt,                        Rynek,     Rozszerzenie, Zawsze;
    dzien_sr,                        Rynek,     Rozszerzenie, Zawsze;
    dzien_czw,                       Rynek,     Rozszerzenie, Zawsze;
    dzien_pt,                        Rynek,     Rozszerzenie, Zawsze;
    dzien_miesiaca,                  Rynek,     Rozszerzenie, Zawsze;
    minut_od_polnocy,                Rynek,     Rozszerzenie, Zawsze;

    // ---------- C. STRUKTURA I TREND (znakowane kierunkiem) ----------
    mom_1m_usd,                      Struktura, Rdzen,        Zawsze;
    mom_5m_usd,                      Struktura, Rdzen,        Zawsze;
    mom_15m_usd,                     Struktura, Rozszerzenie, Zawsze;
    mom_60m_usd,                     Struktura, Rdzen,        Zawsze;
    mom_240m_usd,                    Struktura, Rozszerzenie, Zawsze;
    mom_60m_w_atr,                   Struktura, Rdzen,        Zawsze;
    cena_minus_sr_60m_usd,           Struktura, Rdzen,        Zawsze;
    cena_minus_sr_240m_usd,          Struktura, Rozszerzenie, Zawsze;
    cena_minus_sr_1440m_usd,         Struktura, Rozszerzenie, Zawsze;
    cena_minus_sr_tydzien_usd,       Struktura, Rozszerzenie, Zawsze;
    nachylenie_sr_60m_usd_h,         Struktura, Rozszerzenie, Zawsze;
    nachylenie_sr_240m_usd_h,        Struktura, Rozszerzenie, Zawsze;
    nachylenie_sr_1440m_usd_h,       Struktura, Rozszerzenie, Zawsze;
    do_maks_1d_usd,                  Struktura, Rozszerzenie, Zawsze;
    do_min_1d_usd,                   Struktura, Rozszerzenie, Zawsze;
    do_maks_5d_usd,                  Struktura, Rozszerzenie, Zawsze;
    do_min_5d_usd,                   Struktura, Rozszerzenie, Zawsze;
    udzial_minut_zgodnych_60m,       Struktura, Rozszerzenie, Zawsze;
    cena_wzgledem_sygnalu_usd,       Struktura, Rdzen,        TylkoKoszyk;

    // ---------- D. KONTEKST KANAŁU I GEOMETRIA SYGNAŁU ----------
    n_sygnalow_1h,                   Kanal,     Rozszerzenie, Zawsze;
    n_sygnalow_24h,                  Kanal,     Rdzen,        Zawsze;
    min_od_poprzedniego_sygnalu,     Kanal,     Rozszerzenie, Zawsze;
    zgodnosc_kierunku_5,             Kanal,     Rozszerzenie, Zawsze;
    udzial_buy_24h,                  Kanal,     Rozszerzenie, Zawsze;
    seria_wygranych,                 Kanal,     Rozszerzenie, Zawsze;
    seria_przegranych,               Kanal,     Rozszerzenie, Zawsze;
    wynik_ostatnich_5_usd,           Kanal,     Rozszerzenie, Zawsze;
    min_od_tp_hit_kanalu,            Kanal,     Rozszerzenie, Zawsze;
    szerokosc_strefy_usd,            Kanal,     Rdzen,        Zawsze;
    szerokosc_strefy_w_atr,          Kanal,     Rozszerzenie, Zawsze;
    dystans_sl_usd,                  Kanal,     Rdzen,        Zawsze;
    rr_krawedz_lepsza,               Kanal,     Rdzen,        Zawsze;
    rr_srodek_strefy,                Kanal,     Rozszerzenie, Zawsze;
    rr_krawedz_gorsza,               Kanal,     Rozszerzenie, Zawsze;
    rr_ostatni_cel_srodek,           Kanal,     Rozszerzenie, Zawsze;
    rozstaw_celow_usd,               Kanal,     Rozszerzenie, Zawsze;
    n_celow,                         Kanal,     Rozszerzenie, Zawsze;
    tp_open,                         Kanal,     Rozszerzenie, Zawsze;
    tag_high_risk,                   Kanal,     Rozszerzenie, Zawsze;
    tag_may_not,                     Kanal,     Rozszerzenie, Zawsze;
    tag_first_entry,                 Kanal,     Rozszerzenie, Zawsze;
    jest_limit,                      Kanal,     Rdzen,        Zawsze;
    jest_stop,                       Kanal,     Rozszerzenie, Zawsze;
    cena_do_strefy_usd,              Kanal,     Rdzen,        Zawsze;
    cena_wzgl_strefy_norm,           Kanal,     Rdzen,        Zawsze;
    nakladanie_z_otwartym,           Kanal,     Rozszerzenie, Zawsze;
    jest_koszyk_przeciwny,           Kanal,     Rozszerzenie, Zawsze;
    n_koszykow_zgodnych,             Kanal,     Rozszerzenie, Zawsze;

    // ---------- A. DYNAMIKA DOJŚCIA DO STREFY ----------
    min_od_sygnalu,                  Dynamika,  Rdzen,        TylkoKoszyk;
    min_do_1_dotkniecia,             Dynamika,  Rdzen,        TylkoKoszyk;
    predkosc_dojscia_usd_min,        Dynamika,  Rdzen,        TylkoKoszyk;
    predkosc_60m_przed_sygnalem,     Dynamika,  Rozszerzenie, TylkoKoszyk;
    luka_wejscia_usd,                Dynamika,  Rozszerzenie, TylkoKoszyk;
    weszla_luka,                     Dynamika,  Rozszerzenie, TylkoKoszyk;
    n_dotkniec_strefy,               Dynamika,  Rdzen,        TylkoKoszyk;
    max_glebokosc_usd,               Dynamika,  Rozszerzenie, TylkoKoszyk;
    max_glebokosc_w_szer,            Dynamika,  Rdzen,        TylkoKoszyk;
    przebita_na_wylot,               Dynamika,  Rozszerzenie, TylkoKoszyk;
    min_w_strefie,                   Dynamika,  Rozszerzenie, TylkoKoszyk;
    min_od_wyjscia_ze_strefy,        Dynamika,  Rozszerzenie, TylkoKoszyk;
    w_strefie_teraz,                 Dynamika,  Rdzen,        TylkoKoszyk;

    // ---------- E. STAN PORTFELA ----------
    n_koszykow_otwartych,            Portfel,   Rdzen,        Zawsze;
    n_pozycji_otwartych,             Portfel,   Rdzen,        Zawsze;
    ekspozycja_lotow,                Portfel,   Rdzen,        Zawsze;
    ekspozycja_netto_lotow,          Portfel,   Rozszerzenie, Zawsze;
    saldo_usd,                       Portfel,   Rdzen,        Zawsze;
    equity_usd,                      Portfel,   Rdzen,        Zawsze;
    wolny_margines_usd,              Portfel,   Rdzen,        Zawsze;
    margines_uzyty_pct,              Portfel,   Rdzen,        Zawsze;
    wynik_dnia_zrealizowany_usd,     Portfel,   Rdzen,        Zawsze;
    wynik_niezrealizowany_usd,       Portfel,   Rdzen,        Zawsze;
    equity_do_szczytu_pct,           Portfel,   Rdzen,        Zawsze;
    equity_do_startu_dnia_pct,       Portfel,   Rozszerzenie, Zawsze;
    equity_do_salda_startowego_pct,  Portfel,   Rozszerzenie, Zawsze;
    min_od_ostatniego_zamkniecia,    Portfel,   Rozszerzenie, Zawsze;

    // ---------- F. STAN KOSZYKA (całościowy!) ----------
    wiek_koszyka_min,                Koszyk,    Rdzen,        TylkoKoszyk;
    wiek_najstarszej_pozycji_min,    Koszyk,    Rdzen,        TylkoKoszyk;
    wiek_najnowszej_pozycji_min,     Koszyk,    Rozszerzenie, TylkoKoszyk;
    lot_koszyka,                     Koszyk,    Rdzen,        TylkoKoszyk;
    srednia_wazona_cena_wejscia,     Koszyk,    Rdzen,        TylkoKoszyk;
    cena_minus_srednia_wejscia_usd,  Koszyk,    Rdzen,        TylkoKoszyk;
    zysk_koszyka_niezreal_usd,       Koszyk,    Rdzen,        TylkoKoszyk;
    zrealizowane_koszyka_usd,        Koszyk,    Rdzen,        TylkoKoszyk;
    wynik_koszyka_lacznie_usd,       Koszyk,    Rdzen,        TylkoKoszyk;
    ryzyko_koszyka_usd,              Koszyk,    Rdzen,        TylkoKoszyk;
    ryzyko_poczatkowe_koszyka_usd,   Koszyk,    Rdzen,        TylkoKoszyk;
    // Dwie jednostki R, bo odpowiadają na DWA różne pytania. Bieżąca: „ile
    // jeszcze mogę stracić" — to jest jednostka decyzji „mam 2R, uwalniam
    // koszyk". Początkowa: stała jednostka, jedyna porównywalna MIĘDZY
    // koszykami. Ryzyko bieżące maleje przy domykaniu warstw, więc ta sama
    // liczba R znaczy co innego na początku i na końcu życia koszyka.
    wynik_koszyka_w_r,               Koszyk,    Rdzen,        TylkoKoszyk;
    wynik_koszyka_w_r_poczatkowym,   Koszyk,    Rdzen,        TylkoKoszyk;
    szczyt_wyniku_koszyka_usd,       Koszyk,    Rdzen,        TylkoKoszyk;
    spadek_od_szczytu_koszyka_usd,   Koszyk,    Rdzen,        TylkoKoszyk;
    udzial_szczytu_koszyka,          Koszyk,    Rdzen,        TylkoKoszyk;
    min_od_szczytu_koszyka,          Koszyk,    Rdzen,        TylkoKoszyk;
    warstwy_wypelnione,              Koszyk,    Rdzen,        TylkoKoszyk;
    warstwy_czekajace,               Koszyk,    Rdzen,        TylkoKoszyk;
    warstwy_zaplanowane,             Koszyk,    Rozszerzenie, TylkoKoszyk;
    udzial_wypelnienia,              Koszyk,    Rdzen,        TylkoKoszyk;
    udzial_ryzyka_w_rynku,           Koszyk,    Rdzen,        TylkoKoszyk;
    glebokosc_wejscia_w_szer,        Koszyk,    Rozszerzenie, TylkoKoszyk;
    do_sl_usd,                       Koszyk,    Rdzen,        TylkoKoszyk;
    do_sl_w_atr,                     Koszyk,    Rozszerzenie, TylkoKoszyk;
    do_tp1_usd,                      Koszyk,    Rdzen,        TylkoKoszyk;
    do_tp2_usd,                      Koszyk,    Rozszerzenie, TylkoKoszyk;
    do_tp3_usd,                      Koszyk,    Rozszerzenie, TylkoKoszyk;
    do_najblizszego_celu_w_atr,      Koszyk,    Rozszerzenie, TylkoKoszyk;
    etap_tp,                         Koszyk,    Rdzen,        TylkoKoszyk;
    stan_armed,                      Koszyk,    Rozszerzenie, TylkoKoszyk;
    stan_working,                    Koszyk,    Rdzen,        TylkoKoszyk;
    stan_riskfree,                   Koszyk,    Rdzen,        TylkoKoszyk;
    zabezpieczony,                   Koszyk,    Rozszerzenie, TylkoKoszyk;
    n_reentries,                     Koszyk,    Rozszerzenie, TylkoKoszyk;
    n_rearms,                        Koszyk,    Rozszerzenie, TylkoKoszyk;
    min_od_kom_tp_hit,               Koszyk,    Rdzen,        TylkoKoszyk;
    min_od_kom_risk_free,            Koszyk,    Rdzen,        TylkoKoszyk;
    min_od_kom_spp,                  Koszyk,    Rdzen,        TylkoKoszyk;
    min_od_kom_be,                   Koszyk,    Rozszerzenie, TylkoKoszyk;
    min_od_kom_out_at_entry,         Koszyk,    Rozszerzenie, TylkoKoszyk;
    min_od_kom_cancel,               Koszyk,    Rozszerzenie, TylkoKoszyk;
    min_od_kom_close_all,            Koszyk,    Rozszerzenie, TylkoKoszyk;
    min_od_dowolnego_komunikatu,     Koszyk,    Rozszerzenie, TylkoKoszyk;
    byl_kom_tp_hit,                  Koszyk,    Rozszerzenie, TylkoKoszyk;
    byl_kom_risk_free,               Koszyk,    Rozszerzenie, TylkoKoszyk;
    byl_kom_spp,                     Koszyk,    Rozszerzenie, TylkoKoszyk;

    // ---------- F2. PRÓG OPŁACALNOŚCI I RISK FREE CAŁEGO KOSZYKA ----------
    //
    // Kanał domyka część warstw tak, żeby CAŁOŚĆ wyszła na zero, a stop reszty
    // ląduje na średniej ważonej. Bez tych liczb tej decyzji nie da się ani
    // odtworzyć, ani ocenić — i to jest dokładnie ta luka, którą zgłosił
    // właściciel („ile w sumie są warte wszystkie pozycje z koszyka").
    dystans_do_be_koszyka_usd,       Koszyk,    Rdzen,        TylkoKoszyk;
    dystans_do_be_koszyka_w_atr,     Koszyk,    Rdzen,        TylkoKoszyk;
    rf_wykonalne,                    Koszyk,    Rdzen,        TylkoKoszyk;
    rf_ile_warstw_domknac,           Koszyk,    Rdzen,        TylkoKoszyk;
    rf_zysk_po_domknieciu_usd,       Koszyk,    Rdzen,        TylkoKoszyk;
    rf_be_reszty_usd,                Koszyk,    Rdzen,        TylkoKoszyk;
    rf_lot_do_domkniecia,            Koszyk,    Rozszerzenie, TylkoKoszyk;

    // Warstwy osobno — do POLICZENIA, którą domknąć, a nie do zgadywania.
    // Wszystkie warstwy jednego koszyka mają identyczną przyszłą ścieżkę ceny;
    // różnią się wyłącznie ceną wejścia, znaną w chwili decyzji.
    warstwa_1_wynik_usd,             Koszyk,    Rozszerzenie, TylkoKoszyk;
    warstwa_2_wynik_usd,             Koszyk,    Rozszerzenie, TylkoKoszyk;
    warstwa_3_wynik_usd,             Koszyk,    Rozszerzenie, TylkoKoszyk;
    warstwa_4_wynik_usd,             Koszyk,    Rozszerzenie, TylkoKoszyk;
    warstwa_5_wynik_usd,             Koszyk,    Rozszerzenie, TylkoKoszyk;
    warstwa_6_wynik_usd,             Koszyk,    Rozszerzenie, TylkoKoszyk;
    warstwa_1_cena_wejscia,          Koszyk,    Rozszerzenie, TylkoKoszyk;
    warstwa_2_cena_wejscia,          Koszyk,    Rozszerzenie, TylkoKoszyk;
    warstwa_3_cena_wejscia,          Koszyk,    Rozszerzenie, TylkoKoszyk;
    warstwa_4_cena_wejscia,          Koszyk,    Rozszerzenie, TylkoKoszyk;
    warstwa_5_cena_wejscia,          Koszyk,    Rozszerzenie, TylkoKoszyk;
    warstwa_6_cena_wejscia,          Koszyk,    Rozszerzenie, TylkoKoszyk;
    warstwa_najplytsza_wynik_usd,    Koszyk,    Rdzen,        TylkoKoszyk;
    rozpietosc_wejsc_koszyka_usd,    Koszyk,    Rdzen,        TylkoKoszyk;

    // ---------- KONTROLA LOSOWA ----------
    losowa_01, Losowa, Kontrola, Zawsze;
    losowa_02, Losowa, Kontrola, Zawsze;
    losowa_03, Losowa, Kontrola, Zawsze;
    losowa_04, Losowa, Kontrola, Zawsze;
    losowa_05, Losowa, Kontrola, Zawsze;
    losowa_06, Losowa, Kontrola, Zawsze;
    losowa_07, Losowa, Kontrola, Zawsze;
    losowa_08, Losowa, Kontrola, Zawsze;
    losowa_09, Losowa, Kontrola, Zawsze;
    losowa_10, Losowa, Kontrola, Zawsze;
    losowa_11, Losowa, Kontrola, Zawsze;
    losowa_12, Losowa, Kontrola, Zawsze;
    losowa_13, Losowa, Kontrola, Zawsze;
    losowa_14, Losowa, Kontrola, Zawsze;
    losowa_15, Losowa, Kontrola, Zawsze;
    losowa_16, Losowa, Kontrola, Zawsze;
    losowa_17, Losowa, Kontrola, Zawsze;
    losowa_18, Losowa, Kontrola, Zawsze;
    losowa_19, Losowa, Kontrola, Zawsze;
    losowa_20, Losowa, Kontrola, Zawsze;
}

/// Nazwa cechy.
#[inline]
pub fn nazwa(c: C) -> &'static str {
    OPISY[c as usize].nazwa
}

/// Wszystkie nazwy w kolejności wektora — do nagłówka eksportu.
pub fn nazwy() -> Vec<&'static str> {
    OPISY.iter().map(|o| o.nazwa).collect()
}

/// Indeks pierwszej cechy losowej. Wszystko od tego miejsca to kontrola.
pub fn pierwsza_losowa() -> usize {
    C::losowa_01 as usize
}

// ============================================================
//  WYNIK
// ============================================================

/// Wektor obserwacji wraz z maską dostępności.
///
/// `maska[i] == false` znaczy „**nie wiem**", a nie „zero". Model ma prawo
/// traktować te dwa przypadki różnie i musi mieć jak je rozróżnić.
#[derive(Debug, Clone, PartialEq)]
pub struct Obserwacje {
    pub v: Vec<f32>,
    pub maska: Vec<bool>,
    /// wersja układu, przepisywana do artefaktów modelu
    pub wersja: u32,
}

impl Default for Obserwacje {
    fn default() -> Self {
        Obserwacje {
            v: vec![0.0; N_CECH],
            maska: vec![false; N_CECH],
            wersja: WERSJA_OBSERWACJI,
        }
    }
}

impl Obserwacje {
    #[inline]
    fn ustaw(&mut self, c: C, x: f64) {
        if x.is_finite() {
            self.v[c as usize] = x as f32;
            self.maska[c as usize] = true;
        }
    }

    #[inline]
    fn ustaw_opt(&mut self, c: C, x: Option<f64>) {
        if let Some(x) = x {
            self.ustaw(c, x);
        }
    }

    #[inline]
    fn ustaw_flage(&mut self, c: C, b: bool) {
        self.ustaw(c, if b { 1.0 } else { 0.0 });
    }

    #[inline]
    pub fn wartosc(&self, c: C) -> f32 {
        self.v[c as usize]
    }

    #[inline]
    pub fn wiadomo(&self, c: C) -> bool {
        self.maska[c as usize]
    }

    /// Ile cech w ogóle udało się policzyć. Diagnostyka: nagły spadek znaczy,
    /// że bufor rynku jest pusty albo krok został wywołany za wcześnie.
    pub fn ile_znanych(&self) -> usize {
        self.maska.iter().filter(|b| **b).count()
    }
}

// ============================================================
//  KONFIGURACJA
// ============================================================

#[derive(Debug, Clone, Copy)]
pub struct KonfigObserwacji {
    /// O ile znacznik ticka wyprzedza UTC. Ticki są w czasie serwera brokera
    /// (UTC+3) — sesje liczymy w UTC, więc trzeba to odjąć.
    pub offset_serwera_ms: i64,
    /// Okno, na którym mierzymy prędkość dojścia do strefy (minuty).
    pub okno_predkosci_min: i64,
    /// Od ilu dolarów wskoczenie w strefę uznajemy za lukę, a nie za płynne
    /// wejście.
    pub prog_luki_usd: f64,
    /// Ile minutowych świec trzymamy (24 h + zapas).
    pub minut_w_ringu: usize,
    /// Ile godzinowych świec trzymamy (dwa tygodnie + zapas).
    pub godzin_w_ringu: usize,
    pub ziarno: u64,
}

pub const ZIARNO_DOMYSLNE: u64 = 0x0B5E_7AC1_0000_2907;

impl Default for KonfigObserwacji {
    fn default() -> Self {
        KonfigObserwacji {
            offset_serwera_ms: 3 * 3_600_000,
            okno_predkosci_min: 5,
            prog_luki_usd: 0.30,
            minut_w_ringu: 1500,
            godzin_w_ringu: 400,
            ziarno: ZIARNO_DOMYSLNE,
        }
    }
}

// ============================================================
//  ŚWIECE I BUFOR RYNKU
// ============================================================

#[derive(Debug, Clone, Copy)]
struct Swieca {
    /// numer kubełka (minuta albo godzina od epoki)
    kubelek: i64,
    o: f64,
    h: f64,
    l: f64,
    c: f64,
    n: u32,
    suma_spreadu: f64,
}

impl Swieca {
    #[inline]
    fn nowa(kubelek: i64, p: f64, spread: f64) -> Self {
        Swieca {
            kubelek,
            o: p,
            h: p,
            l: p,
            c: p,
            n: 1,
            suma_spreadu: spread,
        }
    }
    #[inline]
    fn dodaj(&mut self, p: f64, spread: f64) {
        if p > self.h {
            self.h = p;
        }
        if p < self.l {
            self.l = p;
        }
        self.c = p;
        self.n += 1;
        self.suma_spreadu += spread;
    }
    #[inline]
    fn zakres(&self) -> f64 {
        self.h - self.l
    }
    #[inline]
    fn sredni_spread(&self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            self.suma_spreadu / self.n as f64
        }
    }
}

/// Agregaty przeliczane RAZ na zamkniętą minutę.
///
/// To jest cały sekret taniości tego modułu: obserwacja wywołana przy każdym
/// kroku zarządzania czyta gotowe liczby i dokłada do nich tylko bieżącą cenę.
#[derive(Debug, Clone, Default)]
struct Agregaty {
    sr_60m: Option<f64>,
    sr_240m: Option<f64>,
    sr_1440m: Option<f64>,
    sr_tydzien: Option<f64>,
    nachylenie_60m: Option<f64>,
    nachylenie_240m: Option<f64>,
    nachylenie_1440m: Option<f64>,
    zakres_5m: Option<f64>,
    zakres_60m: Option<f64>,
    zakres_240m: Option<f64>,
    zakres_1440m: Option<f64>,
    zmiennosc_5m: Option<f64>,
    zmiennosc_15m: Option<f64>,
    zmiennosc_60m: Option<f64>,
    zmiennosc_240m: Option<f64>,
    atr60: Option<f64>,
    tickow_na_minute: Option<f64>,
    mediana_spreadu: Option<f64>,
    maks_1d: Option<f64>,
    min_1d: Option<f64>,
    maks_5d: Option<f64>,
    min_5d: Option<f64>,
    /// zamknięcia sprzed 1 / 5 / 15 / 60 / 240 minut
    cena_sprzed: [Option<f64>; 5],
    /// udział minut zamkniętych w górę w ostatniej godzinie
    udzial_wzrostowych_60m: Option<f64>,
}

const OKNA_MOM: [usize; 5] = [1, 5, 15, 60, 240];

/// Stan rynku — jedyne miejsce, w którym ten moduł pamięta ceny.
#[derive(Debug, Clone)]
pub struct StanRynku {
    cfg: KonfigObserwacji,
    minuty: VecDeque<Swieca>,
    godziny: VecDeque<Swieca>,
    biezaca_minuta: Option<Swieca>,
    biezaca_godzina: Option<Swieca>,
    agr: Agregaty,
    dzien: i64,
    otwarcie_dnia: f64,
    maks_dnia: f64,
    min_dnia: f64,
    ostatni_ts: Option<Ts>,
    ostatni_quote: Option<Quote>,
}

impl StanRynku {
    pub fn new(cfg: KonfigObserwacji) -> Self {
        StanRynku {
            cfg,
            minuty: VecDeque::with_capacity(cfg.minut_w_ringu + 2),
            godziny: VecDeque::with_capacity(cfg.godzin_w_ringu + 2),
            biezaca_minuta: None,
            biezaca_godzina: None,
            agr: Agregaty::default(),
            dzien: i64::MIN,
            otwarcie_dnia: 0.0,
            maks_dnia: f64::MIN,
            min_dnia: f64::MAX,
            ostatni_ts: None,
            ostatni_quote: None,
        }
    }

    /// Ile minutowych świec już mamy. Poniżej ~60 większość agregatów jest
    /// nieznana i idzie z maską `false`.
    pub fn dlugosc_historii_min(&self) -> usize {
        self.minuty.len()
    }

    fn na_ticku(&mut self, q: &Quote) {
        let p = q.mid();
        let spread = q.spread();
        let min_kub = q.ts.div_euclid(60_000);
        let godz_kub = q.ts.div_euclid(3_600_000);

        // --- doba handlowa (czas serwera, offset 0 — patrz Settings::session_offset) ---
        let d = q.ts.div_euclid(86_400_000);
        if d != self.dzien {
            self.dzien = d;
            self.otwarcie_dnia = p;
            self.maks_dnia = p;
            self.min_dnia = p;
        }
        if p > self.maks_dnia {
            self.maks_dnia = p;
        }
        if p < self.min_dnia {
            self.min_dnia = p;
        }

        // --- świeca godzinowa ---
        match &mut self.biezaca_godzina {
            Some(s) if s.kubelek == godz_kub => s.dodaj(p, spread),
            Some(_) => {
                let stara = self.biezaca_godzina.take().unwrap();
                self.godziny.push_back(stara);
                while self.godziny.len() > self.cfg.godzin_w_ringu {
                    self.godziny.pop_front();
                }
                self.biezaca_godzina = Some(Swieca::nowa(godz_kub, p, spread));
            }
            None => self.biezaca_godzina = Some(Swieca::nowa(godz_kub, p, spread)),
        }

        // --- świeca minutowa; przeliczenie agregatów TYLKO na zamknięciu ---
        let zamknij = match &mut self.biezaca_minuta {
            Some(s) if s.kubelek == min_kub => {
                s.dodaj(p, spread);
                false
            }
            Some(_) => true,
            None => {
                self.biezaca_minuta = Some(Swieca::nowa(min_kub, p, spread));
                false
            }
        };
        if zamknij {
            let stara = self.biezaca_minuta.take().unwrap();
            self.minuty.push_back(stara);
            while self.minuty.len() > self.cfg.minut_w_ringu {
                self.minuty.pop_front();
            }
            self.biezaca_minuta = Some(Swieca::nowa(min_kub, p, spread));
            self.przelicz_agregaty();
        }

        self.ostatni_ts = Some(q.ts);
        self.ostatni_quote = Some(*q);
    }

    fn srednia(&self, w: usize) -> Option<f64> {
        let n = self.minuty.len();
        if n < w || w == 0 {
            return None;
        }
        let mut s = 0.0;
        for i in (n - w)..n {
            s += self.minuty[i].c;
        }
        Some(s / w as f64)
    }

    fn srednia_poprzednia(&self, w: usize) -> Option<f64> {
        let n = self.minuty.len();
        if n < 2 * w || w == 0 {
            return None;
        }
        let mut s = 0.0;
        for i in (n - 2 * w)..(n - w) {
            s += self.minuty[i].c;
        }
        Some(s / w as f64)
    }

    fn zakres_okna(&self, w: usize) -> Option<f64> {
        let n = self.minuty.len();
        if n < w || w == 0 {
            return None;
        }
        let mut lo = f64::MAX;
        let mut hi = f64::MIN;
        for i in (n - w)..n {
            lo = lo.min(self.minuty[i].l);
            hi = hi.max(self.minuty[i].h);
        }
        Some(hi - lo)
    }

    // Okna dobowe i dłuższe liczymy z ringu GODZINOWEGO, nie minutowego.
    //
    // Powód jest praktyczny i został znaleziony przez test pokrycia: nachylenie
    // średniej dobowej wymaga DWÓCH dób zamkniętych minut, czyli 2880 świec —
    // więcej, niż mieści ring. Cecha liczona z minut byłaby martwa zawsze,
    // a nikt by tego nie zauważył, bo maska po prostu zostawałaby na `false`.

    fn srednia_godz(&self, w: usize) -> Option<f64> {
        let n = self.godziny.len();
        if n < w || w == 0 {
            return None;
        }
        let mut s = 0.0;
        for i in (n - w)..n {
            s += self.godziny[i].c;
        }
        Some(s / w as f64)
    }

    fn srednia_godz_poprz(&self, w: usize) -> Option<f64> {
        let n = self.godziny.len();
        if n < 2 * w || w == 0 {
            return None;
        }
        let mut s = 0.0;
        for i in (n - 2 * w)..(n - w) {
            s += self.godziny[i].c;
        }
        Some(s / w as f64)
    }

    /// (min, max) z ostatnich `w` godzin.
    fn skrajne_godz(&self, w: usize) -> Option<(f64, f64)> {
        let n = self.godziny.len();
        if n < w || w == 0 {
            return None;
        }
        let mut lo = f64::MAX;
        let mut hi = f64::MIN;
        for i in (n - w)..n {
            lo = lo.min(self.godziny[i].l);
            hi = hi.max(self.godziny[i].h);
        }
        Some((lo, hi))
    }

    /// Zmienność zrealizowana: pierwiastek sumy kwadratów zwrotów minutowych.
    /// Jednostka: dolary na okno.
    fn zmiennosc_okna(&self, w: usize) -> Option<f64> {
        let n = self.minuty.len();
        if n < w + 1 || w == 0 {
            return None;
        }
        let mut s = 0.0;
        for i in (n - w)..n {
            let d = self.minuty[i].c - self.minuty[i - 1].c;
            s += d * d;
        }
        Some(s.sqrt())
    }

    fn przelicz_agregaty(&mut self) {
        let n = self.minuty.len();
        let mut a = Agregaty::default();

        a.sr_60m = self.srednia(60);
        a.sr_240m = self.srednia(240);
        a.sr_1440m = self.srednia_godz(24);
        a.nachylenie_60m = match (a.sr_60m, self.srednia_poprzednia(60)) {
            (Some(x), Some(y)) => Some((x - y) / 1.0),
            _ => None,
        };
        a.nachylenie_240m = match (a.sr_240m, self.srednia_poprzednia(240)) {
            (Some(x), Some(y)) => Some((x - y) / 4.0),
            _ => None,
        };
        a.nachylenie_1440m = match (a.sr_1440m, self.srednia_godz_poprz(24)) {
            (Some(x), Some(y)) => Some((x - y) / 24.0),
            _ => None,
        };

        a.zakres_5m = self.zakres_okna(5);
        a.zakres_60m = self.zakres_okna(60);
        a.zakres_240m = self.zakres_okna(240);
        a.zakres_1440m = self.skrajne_godz(24).map(|(lo, hi)| hi - lo);

        a.zmiennosc_5m = self.zmiennosc_okna(5);
        a.zmiennosc_15m = self.zmiennosc_okna(15);
        a.zmiennosc_60m = self.zmiennosc_okna(60);
        a.zmiennosc_240m = self.zmiennosc_okna(240);

        if n >= 60 {
            let mut s = 0.0;
            let mut w_gore = 0usize;
            for i in (n - 60)..n {
                s += self.minuty[i].zakres();
                if self.minuty[i].c >= self.minuty[i].o {
                    w_gore += 1;
                }
            }
            a.atr60 = Some(s / 60.0);
            a.udzial_wzrostowych_60m = Some(w_gore as f64 / 60.0);
        }

        if n >= 5 {
            let mut s = 0u32;
            for i in (n - 5)..n {
                s += self.minuty[i].n;
            }
            a.tickow_na_minute = Some(s as f64 / 5.0);
        }

        // mediana spreadu z ostatnich 60 minut
        if n >= 10 {
            let ile = 60.min(n);
            let mut v: Vec<f64> = ((n - ile)..n)
                .map(|i| self.minuty[i].sredni_spread())
                .collect();
            v.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
            a.mediana_spreadu = Some(v[v.len() / 2]);
        }

        if let Some((lo, hi)) = self.skrajne_godz(24) {
            a.min_1d = Some(lo);
            a.maks_1d = Some(hi);
        }
        if let Some((lo, hi)) = self.skrajne_godz(120) {
            a.min_5d = Some(lo);
            a.maks_5d = Some(hi);
        }
        a.sr_tydzien = self.srednia_godz(168);

        for (k, w) in OKNA_MOM.iter().enumerate() {
            a.cena_sprzed[k] = if n >= *w {
                Some(self.minuty[n - *w].c)
            } else {
                None
            };
        }

        self.agr = a;
    }

    /// Cena sprzed `w` minut — tylko z zamkniętych świec, nigdy z przyszłości.
    fn cena_sprzed_min(&self, w: usize) -> Option<f64> {
        OKNA_MOM
            .iter()
            .position(|x| *x == w)
            .and_then(|k| self.agr.cena_sprzed[k])
    }

    /// Ruch ceny w oknie `w` minut, w dolarach, bez znaku kierunku.
    fn ruch(&self, teraz: f64, w: usize) -> Option<f64> {
        self.cena_sprzed_min(w).map(|p| teraz - p)
    }
}

// ============================================================
//  ŚLAD KOSZYKA — dynamika dojścia do strefy i szczyt wyniku
// ============================================================

/// Rodzaj komunikatu zarządzającego z kanału.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Komunikat {
    TpHit,
    RiskFree,
    Spp,
    Be,
    OutAtEntry,
    Cancel,
    CloseAll,
    SlHit,
}

#[derive(Debug, Clone)]
struct SladKoszyka {
    utworzony_ts: Ts,
    strefa_lo: Px,
    strefa_hi: Px,
    cena_sygnalu: f64,
    predkosc_przed_sygnalem: Option<f64>,

    // --- dojście do strefy ---
    w_strefie: bool,
    /// `true` = pierwsze wejście nastąpiło z góry (cena spadała)
    weszla_z_gory: bool,
    pierwsze_dotkniecie_ts: Option<Ts>,
    predkosc_dojscia: Option<f64>,
    luka_wejscia: f64,
    n_dotkniec: u32,
    max_glebokosc: f64,
    przebita_na_wylot: bool,
    ms_w_strefie: i64,
    ostatnie_wyjscie_ts: Option<Ts>,
    ostatni_ts: Option<Ts>,

    // --- szczyt ŁĄCZNEGO wyniku koszyka ---
    szczyt_wyniku: f64,
    szczyt_ts: Ts,
    byl_szczyt: bool,

    // --- komunikaty kanału ---
    kom: HashMap<u8, Ts>,
    ostatni_komunikat_ts: Option<Ts>,
}

#[inline]
fn kod(k: Komunikat) -> u8 {
    match k {
        Komunikat::TpHit => 0,
        Komunikat::RiskFree => 1,
        Komunikat::Spp => 2,
        Komunikat::Be => 3,
        Komunikat::OutAtEntry => 4,
        Komunikat::Cancel => 5,
        Komunikat::CloseAll => 6,
        Komunikat::SlHit => 7,
    }
}

impl SladKoszyka {
    fn nowy(b: &Basket, q: &Quote, predkosc: Option<f64>) -> Self {
        SladKoszyka {
            utworzony_ts: b.created_ts,
            strefa_lo: b.zone_lo,
            strefa_hi: b.zone_hi,
            cena_sygnalu: q.mid(),
            predkosc_przed_sygnalem: predkosc,
            w_strefie: false,
            weszla_z_gory: true,
            pierwsze_dotkniecie_ts: None,
            predkosc_dojscia: None,
            luka_wejscia: 0.0,
            n_dotkniec: 0,
            max_glebokosc: 0.0,
            przebita_na_wylot: false,
            ms_w_strefie: 0,
            ostatnie_wyjscie_ts: None,
            ostatni_ts: None,
            szczyt_wyniku: 0.0,
            szczyt_ts: b.created_ts,
            byl_szczyt: false,
            kom: HashMap::new(),
            ostatni_komunikat_ts: None,
        }
    }

    #[inline]
    fn szerokosc(&self) -> f64 {
        (self.strefa_hi - self.strefa_lo).max(0.0)
    }

    /// Aktualizacja per tick. Musi być tania — chodzi przy każdym ticku dla
    /// każdego żywego koszyka.
    fn na_ticku(&mut self, q: &Quote, predkosc_okna: Option<f64>) {
        let p = q.mid();
        let w = p >= self.strefa_lo && p <= self.strefa_hi;

        if w && !self.w_strefie {
            // WEJŚCIE do strefy
            self.n_dotkniec += 1;
            if self.pierwsze_dotkniecie_ts.is_none() {
                self.pierwsze_dotkniecie_ts = Some(q.ts);
                self.predkosc_dojscia = predkosc_okna;
                // z której strony przyszła cena — z ostatniego znanego stanu
                self.weszla_z_gory = self.cena_poza_strefa_z_gory();
                // luka = jak głęboko wylądował PIERWSZY tick w strefie
                self.luka_wejscia = if self.weszla_z_gory {
                    (self.strefa_hi - p).max(0.0)
                } else {
                    (p - self.strefa_lo).max(0.0)
                };
            }
        } else if !w && self.w_strefie {
            self.ostatnie_wyjscie_ts = Some(q.ts);
        }

        if w {
            if let (Some(t0), true) = (self.ostatni_ts, self.w_strefie) {
                self.ms_w_strefie += (q.ts - t0).max(0);
            }
            let g = if self.weszla_z_gory {
                self.strefa_hi - p
            } else {
                p - self.strefa_lo
            };
            if g > self.max_glebokosc {
                self.max_glebokosc = g;
            }
        } else if self.pierwsze_dotkniecie_ts.is_some() {
            // wyszła drugą stroną = przebicie na wylot
            let szer = self.szerokosc();
            let g = if self.weszla_z_gory {
                self.strefa_hi - p
            } else {
                p - self.strefa_lo
            };
            if g > szer {
                self.przebita_na_wylot = true;
                if g > self.max_glebokosc {
                    self.max_glebokosc = g;
                }
            }
        }

        self.w_strefie = w;
        self.ostatni_ts = Some(q.ts);
    }

    /// Czy przed pierwszym dotknięciem cena stała POWYŻEJ strefy.
    /// Bez zapamiętanej poprzedniej ceny zakładamy kierunek z ceny sygnału —
    /// to jedyna informacja, jaką mamy w chwili powstania koszyka.
    #[inline]
    fn cena_poza_strefa_z_gory(&self) -> bool {
        self.cena_sygnalu >= self.strefa_hi
            || (self.cena_sygnalu > self.strefa_lo
                && self.cena_sygnalu > (self.strefa_lo + self.strefa_hi) * 0.5)
    }

    fn na_wyniku(&mut self, ts: Ts, wynik: f64) {
        if !self.byl_szczyt || wynik > self.szczyt_wyniku {
            self.szczyt_wyniku = wynik;
            self.szczyt_ts = ts;
            self.byl_szczyt = true;
        }
    }
}

// ============================================================
//  STAN KANAŁU
// ============================================================

#[derive(Debug, Clone, Default)]
struct StanKanalu {
    /// (ts, czy BUY) — ostatnie sygnały, przycinane do 24 h
    sygnaly: VecDeque<(Ts, bool)>,
    /// wyniki zamkniętych koszyków, w kolejności
    wyniki: VecDeque<(Ts, f64)>,
    ostatni_tp_hit_ts: Option<Ts>,
    ostatnie_zamkniecie_ts: Option<Ts>,
}

impl StanKanalu {
    fn przytnij(&mut self, ts: Ts) {
        let granica = ts - 24 * 3_600_000;
        while self
            .sygnaly
            .front()
            .map(|(t, _)| *t < granica)
            .unwrap_or(false)
        {
            self.sygnaly.pop_front();
        }
        while self.wyniki.len() > 50 {
            self.wyniki.pop_front();
        }
    }
}

// ============================================================
//  OBSERWATOR
// ============================================================

/// Kontekst wspólny obu wywołań.
pub struct KontekstOgolny<'a> {
    pub ts: Ts,
    pub q: &'a Quote,
    pub konto: &'a Account,
    pub koszyki: &'a [Basket],
    pub pozycje: &'a [Position],
    /// equity szczytowe i equity na starcie doby — do miar obsunięcia
    pub szczyt_equity: f64,
    pub equity_startu_dnia: f64,
    pub saldo_startowe: f64,
    pub zrealizowane_dzis: f64,
}

/// Chwila decyzji o WEJŚCIU. Geometria pochodzi z sygnału, nie z koszyka —
/// koszyk jeszcze nie istnieje.
pub struct KontekstSygnalu<'a> {
    pub ogolny: KontekstOgolny<'a>,
    pub side: Side,
    pub strefa_lo: Px,
    pub strefa_hi: Px,
    pub sl: Option<Px>,
    pub tps: &'a [Px],
    pub tp_open: bool,
    pub jest_limit: bool,
    pub jest_stop: bool,
    pub tag_high_risk: bool,
    pub tag_may_not: bool,
    pub tag_first_entry: bool,
    /// klucz do cech losowych — identyfikator wiadomości
    pub klucz: i64,
}

/// Chwila decyzji o ZARZĄDZANIU otwartym koszykiem.
pub struct KontekstKoszyka<'a> {
    pub ogolny: KontekstOgolny<'a>,
    pub koszyk: &'a Basket,
    /// Wielkości koszykowe policzone przez RDZEŃ (`Engine::basket_view`).
    ///
    /// Gdy jest — bierzemy je ŻYWCEM i niczego nie przeliczamy. To jest
    /// warunek, żeby cecha „wynik koszyka” znaczyła co do centa to samo, co
    /// liczba, na której bot podejmuje decyzję. Rozjazd tych dwóch byłby
    /// niewykrywalny w testach i widoczny dopiero w porównaniu model-reguły.
    ///
    /// `None` tylko poza silnikiem (harness treningowy, testy jednostkowe);
    /// wtedy liczymy tym samym wzorem, a `widok_zgodny_z_rdzeniem` pilnuje,
    /// żeby wzór nie odpłynął.
    pub widok: Option<&'a BasketView>,
}

/// Jedyny obiekt, który trzeba trzymać w silniku.
///
/// Karmiony jednym wywołaniem na tick; wypluwa wektor na żądanie.
#[derive(Debug, Clone)]
pub struct Obserwator {
    cfg: KonfigObserwacji,
    rynek: StanRynku,
    slady: HashMap<u32, SladKoszyka>,
    kanal: StanKanalu,
}

impl Default for Obserwator {
    fn default() -> Self {
        Obserwator::new(KonfigObserwacji::default())
    }
}

impl Obserwator {
    pub fn new(cfg: KonfigObserwacji) -> Self {
        Obserwator {
            cfg,
            rynek: StanRynku::new(cfg),
            slady: HashMap::new(),
            kanal: StanKanalu::default(),
        }
    }

    pub fn konfiguracja(&self) -> KonfigObserwacji {
        self.cfg
    }

    pub fn stan_rynku(&self) -> &StanRynku {
        &self.rynek
    }

    /// Czy wolno pytać o obserwację w chwili `ts`.
    ///
    /// Bufor rynku jest stanem ŻYWYM: zawiera wszystko, czym go dotąd
    /// nakarmiono. Zapytanie o chwilę WCZEŚNIEJSZĄ niż ostatni tick dostałoby
    /// odpowiedź policzoną z danych, których wtedy jeszcze nie było — czyli
    /// zaglądanie w przyszłość tylnymi drzwiami. Dlatego pytanie o przeszłość
    /// jest błędem wywołania, a nie sytuacją do cichego obsłużenia.
    #[inline]
    pub fn czy_aktualny(&self, ts: Ts) -> bool {
        self.rynek.ostatni_ts.map(|t| ts >= t).unwrap_or(true)
    }

    // ---------- karmienie ----------

    /// JEDNO wywołanie na tick. Aktualizuje bufor rynku i ślady koszyków.
    ///
    /// `wyniki_koszykow` przekazuje łączny wynik każdego żywego koszyka
    /// (niezrealizowany + zrealizowany) — dzięki temu szczyt wyniku liczy się
    /// na CAŁOŚCI koszyka, a nie na pojedynczej pozycji.
    pub fn na_ticku(&mut self, q: &Quote, koszyki: &[Basket], pozycje: &[Position]) {
        self.rynek.na_ticku(q);

        let predkosc = self.predkosc_okna(q.mid());

        for b in koszyki {
            if !b.alive() {
                continue;
            }
            let slad = self
                .slady
                .entry(b.id)
                .or_insert_with(|| SladKoszyka::nowy(b, q, predkosc));
            slad.na_ticku(q, predkosc);
            let w = wynik_koszyka_lacznie(b, pozycje, q);
            slad.na_wyniku(q.ts, w);
        }

        // ślady koszyków, których już nie ma, znikają — inaczej mapa rośnie
        // bez końca przez 85 dni backtestu
        if self.slady.len() > 256 {
            let zywe: std::collections::HashSet<u32> =
                koszyki.iter().filter(|b| b.alive()).map(|b| b.id).collect();
            self.slady.retain(|id, _| zywe.contains(id));
        }
    }

    /// Rejestruje nowy sygnał kanału (do cech kontekstu kanału).
    pub fn na_sygnale(&mut self, ts: Ts, side: Side) {
        self.kanal
            .sygnaly
            .push_back((ts, matches!(side, Side::Buy)));
        self.kanal.przytnij(ts);
    }

    /// Rejestruje komunikat zarządzający kanału dla konkretnego koszyka.
    pub fn na_komunikacie(&mut self, basket_id: u32, ts: Ts, k: Komunikat) {
        if matches!(k, Komunikat::TpHit) {
            self.kanal.ostatni_tp_hit_ts = Some(ts);
        }
        if let Some(s) = self.slady.get_mut(&basket_id) {
            s.kom.insert(kod(k), ts);
            s.ostatni_komunikat_ts = Some(ts);
        }
    }

    /// Rejestruje zamknięcie (wynik koszyka albo pozycji) — seria i przerwa.
    pub fn na_zamknieciu(&mut self, ts: Ts, wynik: f64) {
        self.kanal.wyniki.push_back((ts, wynik));
        self.kanal.ostatnie_zamkniecie_ts = Some(ts);
        self.kanal.przytnij(ts);
    }

    /// Kasuje ślad zamkniętego koszyka.
    pub fn zapomnij(&mut self, basket_id: u32) {
        self.slady.remove(&basket_id);
    }

    /// Prędkość ruchu ceny w oknie `okno_predkosci_min`, w $/min.
    /// Znak dodatni = cena rośnie.
    fn predkosc_okna(&self, teraz: f64) -> Option<f64> {
        let w = self.cfg.okno_predkosci_min.max(1) as usize;
        // korzystamy z gotowego okna 5 min, jeśli konfiguracja go nie zmieniła
        let p = self
            .rynek
            .cena_sprzed_min(w)
            .or_else(|| self.rynek.cena_sprzed_min(5))?;
        Some((teraz - p) / w as f64)
    }

    // ---------- obserwacja ----------

    /// Wektor w chwili decyzji o wejściu.
    pub fn obserwuj_sygnal(&self, k: &KontekstSygnalu) -> Obserwacje {
        debug_assert!(
            self.czy_aktualny(k.ogolny.ts),
            "obserwacja pytana o chwilę sprzed ostatniego ticka — to byłoby zaglądanie w przyszłość"
        );
        let mut o = Obserwacje::default();
        let znak = k.side.sign();
        self.wypelnij_rynek(&mut o, &k.ogolny);
        self.wypelnij_strukture(&mut o, &k.ogolny, znak, None);
        self.wypelnij_kanal(
            &mut o,
            &k.ogolny,
            k.side,
            k.strefa_lo,
            k.strefa_hi,
            k.sl,
            k.tps,
            k.tp_open,
            k.jest_limit,
            k.jest_stop,
            k.tag_high_risk,
            k.tag_may_not,
            k.tag_first_entry,
            None,
        );
        self.wypelnij_portfel(&mut o, &k.ogolny);
        self.wypelnij_losowe(&mut o, k.ogolny.ts, k.klucz as u64);
        o
    }

    /// Wektor w chwili kroku zarządzania otwartym koszykiem.
    pub fn obserwuj_koszyk(&self, k: &KontekstKoszyka) -> Obserwacje {
        debug_assert!(
            self.czy_aktualny(k.ogolny.ts),
            "obserwacja pytana o chwilę sprzed ostatniego ticka — to byłoby zaglądanie w przyszłość"
        );
        let b = k.koszyk;
        let mut o = Obserwacje::default();
        let znak = b.side.sign();
        self.wypelnij_rynek(&mut o, &k.ogolny);
        let slad = self.slady.get(&b.id);
        self.wypelnij_strukture(&mut o, &k.ogolny, znak, slad.map(|s| s.cena_sygnalu));
        self.wypelnij_kanal(
            &mut o,
            &k.ogolny,
            b.side,
            b.zone_lo,
            b.zone_hi,
            b.sl,
            &b.tps,
            b.tp_open,
            b.is_limit,
            false,
            false,
            false,
            false,
            Some(b.id),
        );
        self.wypelnij_portfel(&mut o, &k.ogolny);
        self.wypelnij_dynamike(&mut o, &k.ogolny, slad);
        self.wypelnij_koszyk(&mut o, &k.ogolny, b, slad);
        if let Some(w) = k.widok {
            nadpisz_z_widoku(&mut o, w);
        }
        self.wypelnij_losowe(&mut o, k.ogolny.ts, b.id as u64);
        o
    }

    // ---------- rodziny ----------

    fn wypelnij_rynek(&self, o: &mut Obserwacje, g: &KontekstOgolny) {
        let a = &self.rynek.agr;
        let q = g.q;
        let p = q.mid();

        o.ustaw(C::cena_mid_usd, p);
        o.ustaw(C::spread_usd, q.spread());
        if let Some(m) = a.mediana_spreadu {
            if m > 1e-9 {
                o.ustaw(C::spread_do_mediany, q.spread() / m);
            }
        }
        o.ustaw_opt(C::zakres_5m_usd, a.zakres_5m);
        o.ustaw_opt(C::zakres_60m_usd, a.zakres_60m);
        o.ustaw_opt(C::zakres_240m_usd, a.zakres_240m);
        o.ustaw_opt(C::zakres_1440m_usd, a.zakres_1440m);
        o.ustaw_opt(C::zmiennosc_5m_usd, a.zmiennosc_5m);
        o.ustaw_opt(C::zmiennosc_15m_usd, a.zmiennosc_15m);
        o.ustaw_opt(C::zmiennosc_60m_usd, a.zmiennosc_60m);
        o.ustaw_opt(C::zmiennosc_240m_usd, a.zmiennosc_240m);
        o.ustaw_opt(C::atr60_usd, a.atr60);
        o.ustaw_opt(C::tickow_na_minute, a.tickow_na_minute);

        if let Some(t) = self.rynek.ostatni_ts {
            o.ustaw(C::minut_ciszy_rynku, ((g.ts - t).max(0)) as f64 / 60_000.0);
        }

        if self.rynek.dzien != i64::MIN {
            o.ustaw(C::od_otwarcia_dnia_usd, p - self.rynek.otwarcie_dnia);
            o.ustaw(C::do_maks_dnia_usd, self.rynek.maks_dnia - p);
            o.ustaw(C::do_min_dnia_usd, p - self.rynek.min_dnia);
            let zakres = self.rynek.maks_dnia - self.rynek.min_dnia;
            if zakres > 1e-9 {
                o.ustaw(
                    C::pozycja_w_zakresie_dnia,
                    (p - self.rynek.min_dnia) / zakres,
                );
            }
        }

        o.ustaw(C::do_okraglego_10_usd, do_okraglego(p, 10.0));
        o.ustaw(C::do_okraglego_50_usd, do_okraglego(p, 50.0));
        o.ustaw(C::do_okraglego_100_usd, do_okraglego(p, 100.0));

        // --- kalendarz ---
        let utc = g.ts - self.cfg.offset_serwera_ms;
        let h_utc = (utc.rem_euclid(86_400_000)) as f64 / 3_600_000.0;
        let azja = w_oknie(h_utc, 0.0, 9.0);
        let ldn = w_oknie(h_utc, 7.0, 16.0);
        let ny = w_oknie(h_utc, 12.5, 21.0);
        o.ustaw_flage(C::sesja_azja, azja);
        o.ustaw_flage(C::sesja_londyn, ldn);
        o.ustaw_flage(C::sesja_ny, ny);
        o.ustaw_flage(C::nakladanie_ldn_ny, ldn && ny);
        o.ustaw_flage(C::nakladanie_azja_ldn, azja && ldn);
        let koniec = [(azja, 9.0f64), (ldn, 16.0), (ny, 21.0)]
            .iter()
            .filter(|(akt, _)| *akt)
            .map(|(_, k)| (*k - h_utc) * 60.0)
            .fold(f64::NAN, f64::min);
        if koniec.is_finite() {
            o.ustaw(C::min_do_konca_sesji, koniec);
        }

        // godzina w czasie SERWERA — doba handlowa złota łamie się o 00:00
        // znacznika, więc to jest właściwy zegar dla sezonowości
        let h_srv = (g.ts.rem_euclid(86_400_000)) as f64 / 3_600_000.0;
        let kat = std::f64::consts::TAU * h_srv / 24.0;
        o.ustaw(C::godz_sin, kat.sin());
        o.ustaw(C::godz_cos, kat.cos());
        o.ustaw(C::minut_od_polnocy, h_srv * 60.0);

        let dow = ((g.ts.div_euclid(86_400_000) + 3).rem_euclid(7)) as usize;
        o.ustaw_flage(C::dzien_pon, dow == 0);
        o.ustaw_flage(C::dzien_wt, dow == 1);
        o.ustaw_flage(C::dzien_sr, dow == 2);
        o.ustaw_flage(C::dzien_czw, dow == 3);
        o.ustaw_flage(C::dzien_pt, dow == 4);
        o.ustaw(C::dzien_miesiaca, dzien_miesiaca(g.ts) as f64);
    }

    fn wypelnij_strukture(
        &self,
        o: &mut Obserwacje,
        g: &KontekstOgolny,
        znak: f64,
        cena_sygnalu: Option<f64>,
    ) {
        let a = &self.rynek.agr;
        let p = g.q.mid();

        o.ustaw_opt(C::mom_1m_usd, self.rynek.ruch(p, 1).map(|x| x * znak));
        o.ustaw_opt(C::mom_5m_usd, self.rynek.ruch(p, 5).map(|x| x * znak));
        o.ustaw_opt(C::mom_15m_usd, self.rynek.ruch(p, 15).map(|x| x * znak));
        o.ustaw_opt(C::mom_60m_usd, self.rynek.ruch(p, 60).map(|x| x * znak));
        o.ustaw_opt(C::mom_240m_usd, self.rynek.ruch(p, 240).map(|x| x * znak));
        if let (Some(m), Some(atr)) = (self.rynek.ruch(p, 60), a.atr60) {
            if atr > 1e-9 {
                o.ustaw(C::mom_60m_w_atr, m * znak / atr);
            }
        }

        o.ustaw_opt(C::cena_minus_sr_60m_usd, a.sr_60m.map(|s| (p - s) * znak));
        o.ustaw_opt(C::cena_minus_sr_240m_usd, a.sr_240m.map(|s| (p - s) * znak));
        o.ustaw_opt(
            C::cena_minus_sr_1440m_usd,
            a.sr_1440m.map(|s| (p - s) * znak),
        );
        o.ustaw_opt(
            C::cena_minus_sr_tydzien_usd,
            a.sr_tydzien.map(|s| (p - s) * znak),
        );
        o.ustaw_opt(
            C::nachylenie_sr_60m_usd_h,
            a.nachylenie_60m.map(|x| x * znak),
        );
        o.ustaw_opt(
            C::nachylenie_sr_240m_usd_h,
            a.nachylenie_240m.map(|x| x * znak),
        );
        o.ustaw_opt(
            C::nachylenie_sr_1440m_usd_h,
            a.nachylenie_1440m.map(|x| x * znak),
        );

        o.ustaw_opt(C::do_maks_1d_usd, a.maks_1d.map(|x| x - p));
        o.ustaw_opt(C::do_min_1d_usd, a.min_1d.map(|x| p - x));
        o.ustaw_opt(C::do_maks_5d_usd, a.maks_5d.map(|x| x - p));
        o.ustaw_opt(C::do_min_5d_usd, a.min_5d.map(|x| p - x));

        // udział minut idących ZGODNIE z kierunkiem pozycji
        o.ustaw_opt(
            C::udzial_minut_zgodnych_60m,
            a.udzial_wzrostowych_60m
                .map(|u| if znak > 0.0 { u } else { 1.0 - u }),
        );

        if let Some(c0) = cena_sygnalu {
            o.ustaw(C::cena_wzgledem_sygnalu_usd, (p - c0) * znak);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn wypelnij_kanal(
        &self,
        o: &mut Obserwacje,
        g: &KontekstOgolny,
        side: Side,
        strefa_lo: Px,
        strefa_hi: Px,
        sl: Option<Px>,
        tps: &[Px],
        tp_open: bool,
        jest_limit: bool,
        jest_stop: bool,
        tag_high_risk: bool,
        tag_may_not: bool,
        tag_first_entry: bool,
        pomijany_id: Option<u32>,
    ) {
        let p = g.q.mid();
        let ts = g.ts;

        // --- rytm kanału ---
        let h1 = ts - 3_600_000;
        let d1 = ts - 24 * 3_600_000;
        let n1h = self
            .kanal
            .sygnaly
            .iter()
            .filter(|(t, _)| *t >= h1 && *t <= ts)
            .count();
        let n24: Vec<&(Ts, bool)> = self
            .kanal
            .sygnaly
            .iter()
            .filter(|(t, _)| *t >= d1 && *t <= ts)
            .collect();
        o.ustaw(C::n_sygnalow_1h, n1h as f64);
        o.ustaw(C::n_sygnalow_24h, n24.len() as f64);
        if !n24.is_empty() {
            let buy = n24.iter().filter(|(_, b)| *b).count();
            o.ustaw(C::udzial_buy_24h, buy as f64 / n24.len() as f64);
        }
        if let Some((t, _)) = self.kanal.sygnaly.iter().rev().find(|(t, _)| *t < ts) {
            o.ustaw(C::min_od_poprzedniego_sygnalu, (ts - t) as f64 / 60_000.0);
        }
        // zgodność kierunku ostatnich pięciu
        let ostatnie: Vec<bool> = self
            .kanal
            .sygnaly
            .iter()
            .rev()
            .filter(|(t, _)| *t <= ts)
            .take(5)
            .map(|(_, b)| *b)
            .collect();
        if !ostatnie.is_empty() {
            let ten = matches!(side, Side::Buy);
            let zgodne = ostatnie.iter().filter(|b| **b == ten).count();
            o.ustaw(
                C::zgodnosc_kierunku_5,
                zgodne as f64 / ostatnie.len() as f64,
            );
        }

        // --- seria wyników ---
        let mut wygrane = 0.0;
        let mut przegrane = 0.0;
        for (_, w) in self.kanal.wyniki.iter().rev() {
            if *w > 0.0 && przegrane == 0.0 {
                wygrane += 1.0;
            } else if *w <= 0.0 && wygrane == 0.0 {
                przegrane += 1.0;
            } else {
                break;
            }
        }
        o.ustaw(C::seria_wygranych, wygrane);
        o.ustaw(C::seria_przegranych, przegrane);
        if !self.kanal.wyniki.is_empty() {
            let s: f64 = self
                .kanal
                .wyniki
                .iter()
                .rev()
                .take(5)
                .map(|(_, w)| *w)
                .sum();
            o.ustaw(C::wynik_ostatnich_5_usd, s);
        }
        if let Some(t) = self.kanal.ostatni_tp_hit_ts {
            o.ustaw(C::min_od_tp_hit_kanalu, (ts - t).max(0) as f64 / 60_000.0);
        }

        // --- geometria sygnału ---
        let szer = (strefa_hi - strefa_lo).max(0.0);
        o.ustaw(C::szerokosc_strefy_usd, szer);
        if let Some(atr) = self.rynek.agr.atr60 {
            if atr > 1e-9 {
                o.ustaw(C::szerokosc_strefy_w_atr, szer / atr);
            }
        }
        let lepsza = side.better_edge(strefa_lo, strefa_hi);
        let gorsza = side.worse_edge(strefa_lo, strefa_hi);
        let srodek = (strefa_lo + strefa_hi) * 0.5;

        if let Some(sl) = sl {
            o.ustaw(C::dystans_sl_usd, (srodek - sl) * side.sign());
            if let Some(tp1) = tps.first() {
                o.ustaw_opt(C::rr_krawedz_lepsza, rr(side, lepsza, sl, *tp1));
                o.ustaw_opt(C::rr_srodek_strefy, rr(side, srodek, sl, *tp1));
                o.ustaw_opt(C::rr_krawedz_gorsza, rr(side, gorsza, sl, *tp1));
            }
            if let Some(ost) = tps.last() {
                o.ustaw_opt(C::rr_ostatni_cel_srodek, rr(side, srodek, sl, *ost));
            }
        }
        if tps.len() >= 2 {
            let mut s = 0.0;
            for i in 1..tps.len() {
                s += (tps[i] - tps[i - 1]).abs();
            }
            o.ustaw(C::rozstaw_celow_usd, s / (tps.len() - 1) as f64);
        }
        o.ustaw(C::n_celow, tps.len() as f64);
        o.ustaw_flage(C::tp_open, tp_open);
        o.ustaw_flage(C::tag_high_risk, tag_high_risk);
        o.ustaw_flage(C::tag_may_not, tag_may_not);
        o.ustaw_flage(C::tag_first_entry, tag_first_entry);
        o.ustaw_flage(C::jest_limit, jest_limit);
        o.ustaw_flage(C::jest_stop, jest_stop);

        // ile cena musi jeszcze przejść, żeby dotknąć strefy (>0 = poza strefą)
        let do_strefy = if p > strefa_hi {
            p - strefa_hi
        } else if p < strefa_lo {
            strefa_lo - p
        } else {
            0.0
        };
        o.ustaw(C::cena_do_strefy_usd, do_strefy);
        if szer > 1e-9 {
            // 0 = na lepszej krawędzi, 1 = na gorszej
            o.ustaw(C::cena_wzgl_strefy_norm, (p - lepsza) * side.sign() / szer);
        }

        // --- nakładanie z otwartymi koszykami ---
        let mut nakl: f64 = 0.0;
        let mut przeciwny = false;
        let mut zgodnych = 0usize;
        for b in g.koszyki {
            if !b.alive() {
                continue;
            }
            if Some(b.id) == pomijany_id {
                continue;
            }
            if b.side == side {
                zgodnych += 1;
                let lo = b.zone_lo.max(strefa_lo);
                let hi = b.zone_hi.min(strefa_hi);
                if hi > lo && szer > 1e-9 {
                    nakl = nakl.max((hi - lo) / szer);
                }
            } else if b.had_positions || !b.tickets.is_empty() {
                przeciwny = true;
            }
        }
        o.ustaw(C::nakladanie_z_otwartym, nakl);
        o.ustaw_flage(C::jest_koszyk_przeciwny, przeciwny);
        o.ustaw(C::n_koszykow_zgodnych, zgodnych as f64);
    }

    fn wypelnij_portfel(&self, o: &mut Obserwacje, g: &KontekstOgolny) {
        let zywe = g.koszyki.iter().filter(|b| b.alive()).count();
        o.ustaw(C::n_koszykow_otwartych, zywe as f64);
        o.ustaw(C::n_pozycji_otwartych, g.pozycje.len() as f64);

        let mut brutto = 0.0;
        let mut netto = 0.0;
        let mut niezreal = 0.0;
        for p in g.pozycje {
            brutto += p.volume;
            netto += p.volume * p.side.sign();
            niezreal += p.profit_usd(g.q);
        }
        o.ustaw(C::ekspozycja_lotow, brutto);
        o.ustaw(C::ekspozycja_netto_lotow, netto);
        o.ustaw(C::wynik_niezrealizowany_usd, niezreal);

        o.ustaw(C::saldo_usd, g.konto.balance);
        o.ustaw(C::equity_usd, g.konto.equity);
        o.ustaw(C::wolny_margines_usd, g.konto.free_margin);
        if g.konto.equity.abs() > 1e-9 {
            o.ustaw(
                C::margines_uzyty_pct,
                g.konto.margin / g.konto.equity * 100.0,
            );
        }
        o.ustaw(C::wynik_dnia_zrealizowany_usd, g.zrealizowane_dzis);
        if g.szczyt_equity.abs() > 1e-9 {
            o.ustaw(
                C::equity_do_szczytu_pct,
                (g.konto.equity - g.szczyt_equity) / g.szczyt_equity * 100.0,
            );
        }
        if g.equity_startu_dnia.abs() > 1e-9 {
            o.ustaw(
                C::equity_do_startu_dnia_pct,
                (g.konto.equity - g.equity_startu_dnia) / g.equity_startu_dnia * 100.0,
            );
        }
        if g.saldo_startowe.abs() > 1e-9 {
            o.ustaw(
                C::equity_do_salda_startowego_pct,
                (g.konto.equity - g.saldo_startowe) / g.saldo_startowe * 100.0,
            );
        }
        if let Some(t) = self.kanal.ostatnie_zamkniecie_ts {
            o.ustaw(
                C::min_od_ostatniego_zamkniecia,
                (g.ts - t).max(0) as f64 / 60_000.0,
            );
        }
    }

    fn wypelnij_dynamike(
        &self,
        o: &mut Obserwacje,
        g: &KontekstOgolny,
        slad: Option<&SladKoszyka>,
    ) {
        let Some(s) = slad else { return };
        o.ustaw(
            C::min_od_sygnalu,
            (g.ts - s.utworzony_ts).max(0) as f64 / 60_000.0,
        );
        o.ustaw_flage(C::w_strefie_teraz, s.w_strefie);
        o.ustaw(C::n_dotkniec_strefy, s.n_dotkniec as f64);

        // Cechy dostępne DOPIERO po pierwszym dotknięciu. Zanim ono nastąpi,
        // maska zostaje `false` — to jest różnica między „zero" a „nie wiem",
        // i akurat tutaj jest ona całą treścią cechy.
        if let Some(t1) = s.pierwsze_dotkniecie_ts {
            o.ustaw(
                C::min_do_1_dotkniecia,
                (t1 - s.utworzony_ts).max(0) as f64 / 60_000.0,
            );
            o.ustaw_opt(C::predkosc_dojscia_usd_min, s.predkosc_dojscia);
            o.ustaw(C::luka_wejscia_usd, s.luka_wejscia);
            o.ustaw_flage(C::weszla_luka, s.luka_wejscia >= self.cfg.prog_luki_usd);
            o.ustaw(C::max_glebokosc_usd, s.max_glebokosc);
            let szer = s.szerokosc();
            if szer > 1e-9 {
                o.ustaw(C::max_glebokosc_w_szer, s.max_glebokosc / szer);
            }
            o.ustaw_flage(C::przebita_na_wylot, s.przebita_na_wylot);
            o.ustaw(C::min_w_strefie, s.ms_w_strefie as f64 / 60_000.0);
        }
        o.ustaw_opt(C::predkosc_60m_przed_sygnalem, s.predkosc_przed_sygnalem);
        if let Some(t) = s.ostatnie_wyjscie_ts {
            o.ustaw(
                C::min_od_wyjscia_ze_strefy,
                (g.ts - t).max(0) as f64 / 60_000.0,
            );
        }
    }

    fn wypelnij_koszyk(
        &self,
        o: &mut Obserwacje,
        g: &KontekstOgolny,
        b: &Basket,
        slad: Option<&SladKoszyka>,
    ) {
        let q = g.q;
        let ts = g.ts;
        let moje: Vec<&Position> = g
            .pozycje
            .iter()
            .filter(|p| p.basket == Some(b.id))
            .collect();

        o.ustaw(
            C::wiek_koszyka_min,
            (ts - b.created_ts).max(0) as f64 / 60_000.0,
        );
        if let Some(najstarsza) = moje.iter().map(|p| p.open_ts).min() {
            o.ustaw(
                C::wiek_najstarszej_pozycji_min,
                (ts - najstarsza).max(0) as f64 / 60_000.0,
            );
        }
        if let Some(najnowsza) = moje.iter().map(|p| p.open_ts).max() {
            o.ustaw(
                C::wiek_najnowszej_pozycji_min,
                (ts - najnowsza).max(0) as f64 / 60_000.0,
            );
        }

        // ====== CAŁOŚĆ KOSZYKA, NIE POJEDYNCZA POZYCJA ======
        //
        // Kanał zarządza KOSZYKIEM: jego „risk free" to domknięcie części
        // warstw tak, żeby CAŁOŚĆ wyszła na zero, liczone od średniej ważonej.
        // Patrząc na pozycje osobno nie da się tej decyzji ani odtworzyć,
        // ani ocenić — dlatego te cechy są tu pierwszej klasy.
        let lot: f64 = moje.iter().map(|p| p.volume).sum();
        o.ustaw(C::lot_koszyka, lot);
        if lot > 1e-12 {
            let srednia: f64 = moje.iter().map(|p| p.open_price * p.volume).sum::<f64>() / lot;
            o.ustaw(C::srednia_wazona_cena_wejscia, srednia);
            // po cenie WYJŚCIA, nie po środku — to jest kurs, po którym koszyk
            // faktycznie by się domknął, i po którym kanał liczy swój breakeven
            o.ustaw(
                C::cena_minus_srednia_wejscia_usd,
                (q.exit(b.side) - srednia) * b.side.sign(),
            );
        }

        let niezreal: f64 = moje.iter().map(|p| p.profit_usd(q)).sum();
        o.ustaw(C::zysk_koszyka_niezreal_usd, niezreal);
        o.ustaw(C::zrealizowane_koszyka_usd, b.realized);
        let lacznie = niezreal + b.realized;
        o.ustaw(C::wynik_koszyka_lacznie_usd, lacznie);

        // ryzyko CAŁEGO koszyka: suma odległości do SL po wszystkich warstwach
        let mut ryzyko = 0.0;
        let mut maja_sl = 0usize;
        for p in &moje {
            if let Some(sl) = p.sl.or(p.vsl) {
                ryzyko += (p.open_price - sl).abs() * XAU_CONTRACT * p.volume;
                maja_sl += 1;
            }
        }
        if maja_sl > 0 && ryzyko > 1e-9 {
            o.ustaw(C::ryzyko_koszyka_usd, ryzyko);
            o.ustaw(C::wynik_koszyka_w_r, lacznie / ryzyko);
        }
        // ryzyko PEŁNEGO planu siatki — stała jednostka, jedyna porównywalna
        // między koszykami; zapamiętane w koszyku przez RDZEŃ w chwili
        // rozstawienia, bo z migawki nie da się go odtworzyć
        if b.risk_initial_usd > 1e-9 {
            o.ustaw(C::ryzyko_poczatkowe_koszyka_usd, b.risk_initial_usd);
            o.ustaw(
                C::wynik_koszyka_w_r_poczatkowym,
                lacznie / b.risk_initial_usd,
            );
        }

        if let Some(s) = slad {
            if s.byl_szczyt {
                o.ustaw(C::szczyt_wyniku_koszyka_usd, s.szczyt_wyniku);
                o.ustaw(C::spadek_od_szczytu_koszyka_usd, s.szczyt_wyniku - lacznie);
                if s.szczyt_wyniku.abs() > 1e-9 {
                    o.ustaw(C::udzial_szczytu_koszyka, lacznie / s.szczyt_wyniku);
                }
                o.ustaw(
                    C::min_od_szczytu_koszyka,
                    (ts - s.szczyt_ts).max(0) as f64 / 60_000.0,
                );
            }
        }

        // --- warstwy ---
        let zaplanowane = b.levels.len();
        let wypelnione = if zaplanowane > 0 {
            b.levels.iter().filter(|l| l.filled).count()
        } else {
            moje.len()
        };
        let czekajace = b.pendings.len();
        o.ustaw(C::warstwy_zaplanowane, zaplanowane as f64);
        o.ustaw(C::warstwy_wypelnione, wypelnione as f64);
        o.ustaw(C::warstwy_czekajace, czekajace as f64);
        if zaplanowane > 0 {
            o.ustaw(
                C::udzial_wypelnienia,
                wypelnione as f64 / zaplanowane as f64,
            );
            // jaka część ZAPLANOWANEGO ryzyka jest już w rynku
            let plan_lot: f64 = b.levels.iter().map(|l| l.volume.max(0.0)).sum();
            if plan_lot > 1e-12 {
                o.ustaw(C::udzial_ryzyka_w_rynku, lot / plan_lot);
            }
        }

        // głębokość faktycznie osiągnięta przez wejścia
        let szer = b.width();
        if lot > 1e-12 && szer > 1e-9 {
            let srednia: f64 = moje.iter().map(|p| p.open_price * p.volume).sum::<f64>() / lot;
            let lepsza = b.side.better_edge(b.zone_lo, b.zone_hi);
            o.ustaw(
                C::glebokosc_wejscia_w_szer,
                (srednia - lepsza) * b.side.sign() / szer,
            );
        }

        // --- odległości do poziomów ---
        let px = q.exit(b.side);
        if let Some(sl) = b.sl {
            o.ustaw(C::do_sl_usd, (px - sl) * b.side.sign());
            if let Some(atr) = self.rynek.agr.atr60 {
                if atr > 1e-9 {
                    o.ustaw(C::do_sl_w_atr, (px - sl) * b.side.sign() / atr);
                }
            }
        }
        for (i, c) in [C::do_tp1_usd, C::do_tp2_usd, C::do_tp3_usd]
            .iter()
            .enumerate()
        {
            if let Some(tp) = b.tps.get(i) {
                o.ustaw(*c, (tp - px) * b.side.sign());
            }
        }
        if let Some(tp) = b.tps.get(b.tp_stage) {
            if let Some(atr) = self.rynek.agr.atr60 {
                if atr > 1e-9 {
                    o.ustaw(
                        C::do_najblizszego_celu_w_atr,
                        (tp - px) * b.side.sign() / atr,
                    );
                }
            }
        }

        o.ustaw(C::etap_tp, b.tp_stage as f64);
        o.ustaw_flage(C::stan_armed, matches!(b.state, BasketState::Armed));
        o.ustaw_flage(C::stan_working, matches!(b.state, BasketState::Working));
        o.ustaw_flage(C::stan_riskfree, matches!(b.state, BasketState::RiskFree));
        o.ustaw_flage(C::zabezpieczony, b.secured);
        o.ustaw(C::n_reentries, b.reentries as f64);
        o.ustaw(C::n_rearms, b.rearms as f64);

        // --- komunikaty kanału dla TEGO koszyka ---
        if let Some(s) = slad {
            let pary = [
                (
                    Komunikat::TpHit,
                    C::min_od_kom_tp_hit,
                    Some(C::byl_kom_tp_hit),
                ),
                (
                    Komunikat::RiskFree,
                    C::min_od_kom_risk_free,
                    Some(C::byl_kom_risk_free),
                ),
                (Komunikat::Spp, C::min_od_kom_spp, Some(C::byl_kom_spp)),
                (Komunikat::Be, C::min_od_kom_be, None),
                (Komunikat::OutAtEntry, C::min_od_kom_out_at_entry, None),
                (Komunikat::Cancel, C::min_od_kom_cancel, None),
                (Komunikat::CloseAll, C::min_od_kom_close_all, None),
            ];
            for (k, cmin, cflaga) in pary {
                let byl = s.kom.get(&kod(k)).copied();
                if let Some(t) = byl {
                    o.ustaw(cmin, (ts - t).max(0) as f64 / 60_000.0);
                }
                if let Some(cf) = cflaga {
                    o.ustaw_flage(cf, byl.is_some());
                }
            }
            if let Some(t) = s.ostatni_komunikat_ts {
                o.ustaw(
                    C::min_od_dowolnego_komunikatu,
                    (ts - t).max(0) as f64 / 60_000.0,
                );
            }
        }

        self.wypelnij_warstwy_i_rf(o, b, &moje, q, lacznie, lot);
    }

    /// Próg opłacalności CAŁEGO koszyka i arytmetyka „risk free".
    ///
    /// Wszystko tutaj jest **rachunkiem**, nie prognozą: warstwy jednego
    /// koszyka mają identyczną przyszłą ścieżkę ceny i różnią się wyłącznie
    /// ceną wejścia, znaną teraz. Model ma to dostać policzone, żeby nie
    /// musiał zgadywać czegoś, co jest deterministyczne.
    fn wypelnij_warstwy_i_rf(
        &self,
        o: &mut Obserwacje,
        b: &Basket,
        moje: &[&Position],
        q: &Quote,
        lacznie: f64,
        lot: f64,
    ) {
        if moje.is_empty() {
            return;
        }

        // --- ile ceny brakuje, żeby CAŁOŚĆ wyszła na zero ---
        //
        // wynik(px) = (px − średnia_ważona)·znak·100·lot + zrealizowane
        // Stąd droga do zera to po prostu −wynik / (100·lot).
        if lot > 1e-12 {
            let d = -lacznie / (XAU_CONTRACT * lot);
            o.ustaw(C::dystans_do_be_koszyka_usd, d);
            if let Some(atr) = self.rynek.agr.atr60 {
                if atr > 1e-9 {
                    o.ustaw(C::dystans_do_be_koszyka_w_atr, d / atr);
                }
            }
        }

        // --- warstwy uporządkowane od NAJGŁĘBSZEJ (najlepsze wejście) ---
        let znak = b.side.sign();
        let px = q.exit(b.side);
        let mut warstwy: Vec<(f64, f64, f64)> = moje
            .iter()
            .map(|p| {
                let wynik = (px - p.open_price) * p.side.sign() * XAU_CONTRACT * p.volume;
                (p.open_price, p.volume, wynik)
            })
            .collect();
        // lepsze wejście = niższa cena dla BUY, wyższa dla SELL
        warstwy.sort_by(|a, c| {
            (a.0 * znak)
                .partial_cmp(&(c.0 * znak))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let sloty_w = [
            C::warstwa_1_wynik_usd,
            C::warstwa_2_wynik_usd,
            C::warstwa_3_wynik_usd,
            C::warstwa_4_wynik_usd,
            C::warstwa_5_wynik_usd,
            C::warstwa_6_wynik_usd,
        ];
        let sloty_c = [
            C::warstwa_1_cena_wejscia,
            C::warstwa_2_cena_wejscia,
            C::warstwa_3_cena_wejscia,
            C::warstwa_4_cena_wejscia,
            C::warstwa_5_cena_wejscia,
            C::warstwa_6_cena_wejscia,
        ];
        for (i, (cena, _, wynik)) in warstwy.iter().enumerate().take(sloty_w.len()) {
            o.ustaw(sloty_w[i], *wynik);
            o.ustaw(sloty_c[i], *cena);
        }
        o.ustaw(
            C::warstwa_najplytsza_wynik_usd,
            warstwy[warstwy.len() - 1].2,
        );
        o.ustaw(
            C::rozpietosc_wejsc_koszyka_usd,
            (warstwy[warstwy.len() - 1].0 - warstwy[0].0).abs(),
        );

        // --- RISK FREE: ile najpłytszych warstw domknąć ---
        //
        // Warunek kanału: zrealizowane + zysk z domkniętych warstw pokrywa
        // stratę, jaką reszta poniosłaby na SL koszyka. Runner musi zostać,
        // więc domykamy najwyżej n−1 warstw.
        let Some(sl) = b.sl else { return };
        let n = warstwy.len();
        let mut suma_domkniec = 0.0;
        let mut lot_domkniety = 0.0;
        let mut wykonalne = false;

        for k in 0..n {
            // reszta = warstwy [0..n−k) (najgłębsze zostają jako runner)
            let reszta = &warstwy[..(n - k)];
            let ryzyko_reszty: f64 = reszta
                .iter()
                .map(|(cena, vol, _)| ((cena - sl) * znak * XAU_CONTRACT * vol).max(0.0))
                .sum();
            let w_kieszeni = b.realized + suma_domkniec;
            if !reszta.is_empty() && w_kieszeni >= ryzyko_reszty {
                o.ustaw_flage(C::rf_wykonalne, true);
                o.ustaw(C::rf_ile_warstw_domknac, k as f64);
                o.ustaw(C::rf_zysk_po_domknieciu_usd, w_kieszeni);
                o.ustaw(C::rf_lot_do_domkniecia, lot_domkniety);
                let lot_reszty: f64 = reszta.iter().map(|(_, v, _)| *v).sum();
                if lot_reszty > 1e-12 {
                    let be: f64 = reszta.iter().map(|(c, v, _)| c * v).sum::<f64>() / lot_reszty;
                    o.ustaw(C::rf_be_reszty_usd, be);
                }
                wykonalne = true;
                break;
            }
            if k + 1 < n {
                // domykamy kolejną NAJPŁYTSZĄ warstwę
                let (_, vol, wynik) = warstwy[n - 1 - k];
                suma_domkniec += wynik;
                lot_domkniety += vol;
            }
        }
        if !wykonalne {
            o.ustaw_flage(C::rf_wykonalne, false);
        }
    }

    fn wypelnij_losowe(&self, o: &mut Obserwacje, ts: Ts, klucz: u64) {
        let a = splitmix64(ts as u64);
        let b = splitmix64(klucz ^ self.cfg.ziarno);
        let wspolny = splitmix64(a ^ b);
        for i in 0..N_LOSOWYCH {
            let idx = pierwsza_losowa() + i;
            let c = splitmix64(
                (i as u64)
                    .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                    .wrapping_add(1),
            );
            o.v[idx] = losowa01(wspolny ^ c) as f32;
            o.maska[idx] = true;
        }
    }
}

// ============================================================
//  FUNKCJE POMOCNICZE
// ============================================================

/// Przepisuje wielkości koszykowe Z RDZENIA, kasując cokolwiek policzył moduł.
///
/// Kolejność jest istotna: to wywołanie idzie PO wypełnieniu rodziny F, więc
/// liczba z `Engine::basket_view` zawsze wygrywa. Dzięki temu na ścieżce
/// silnika istnieje dokładnie JEDNA definicja „wyniku koszyka" — ta sama,
/// na której bot podejmuje decyzję RISK FREE.
fn nadpisz_z_widoku(o: &mut Obserwacje, w: &BasketView) {
    o.ustaw(C::wynik_koszyka_lacznie_usd, w.pl_usd);
    o.ustaw(C::wynik_koszyka_w_r, w.pl_r_current);
    o.ustaw(C::wynik_koszyka_w_r_poczatkowym, w.pl_r_initial);
    o.ustaw(C::srednia_wazona_cena_wejscia, w.avg_entry);
    o.ustaw(C::ryzyko_koszyka_usd, w.risk_usd);
    o.ustaw(C::ryzyko_poczatkowe_koszyka_usd, w.risk_initial_usd);
    o.ustaw(C::szczyt_wyniku_koszyka_usd, w.peak_pl_usd);
    o.ustaw(C::spadek_od_szczytu_koszyka_usd, w.drawdown_from_peak);
    if w.peak_pl_usd.abs() > 1e-9 {
        o.ustaw(C::udzial_szczytu_koszyka, w.pl_usd / w.peak_pl_usd);
    }
    o.ustaw(C::warstwy_wypelnione, w.filled_layers as f64);
    // `warstwy_czekajace` CELOWO nie idzie z widoku.
    //
    // `engine::widok_koszyka` (wolna funkcja, bez brokera) zwraca tu twarde
    // zero, bo zleceń oczekujących nie da się policzyć z samych pozycji — zna
    // je wyłącznie broker. Gdyby ta wartość przechodziła do wektora, cecha
    // byłaby stałym zerem wszędzie poza ścieżką silnika, czyli DOKŁADNIE tą
    // zdegenerowaną cechą, którą wyłapaliśmy w rodzinie F. Lokalne
    // `b.pendings.len()` jest poprawne w obu przypadkach, więc zostaje ono.
    o.ustaw(C::udzial_ryzyka_w_rynku, w.planned_risk_in_market);
    o.ustaw(C::wiek_koszyka_min, w.age_min);
    o.ustaw_flage(C::zabezpieczony, w.secured);
    o.ustaw(C::etap_tp, w.tp_stage as f64);
}

/// Łączny wynik koszyka: niezrealizowany po wszystkich warstwach + zrealizowany.
///
/// Publiczna, bo to jest dokładnie ta liczba, o którą chodzi w „ile w sumie są
/// warte wszystkie pozycje z koszyka" — i nikt nie powinien liczyć jej drugi
/// raz po swojemu.
pub fn wynik_koszyka_lacznie(b: &Basket, pozycje: &[Position], q: &Quote) -> f64 {
    let niezreal: f64 = pozycje
        .iter()
        .filter(|p| p.basket == Some(b.id))
        .map(|p| p.profit_usd(q))
        .sum();
    niezreal + b.realized
}

/// Średnia ważona cena wejścia koszyka — poziom, od którego kanał liczy swoje
/// „risk free". `None`, gdy koszyk nie ma jeszcze żadnej wypełnionej warstwy.
pub fn srednia_wazona_wejscia(b: &Basket, pozycje: &[Position]) -> Option<f64> {
    let mut lot = 0.0;
    let mut suma = 0.0;
    for p in pozycje.iter().filter(|p| p.basket == Some(b.id)) {
        lot += p.volume;
        suma += p.open_price * p.volume;
    }
    if lot > 1e-12 {
        Some(suma / lot)
    } else {
        None
    }
}

#[inline]
fn rr(side: Side, wejscie: Px, sl: Px, tp: Px) -> Option<f64> {
    let ryzyko = (wejscie - sl) * side.sign();
    let zysk = (tp - wejscie) * side.sign();
    if ryzyko > 1e-9 {
        Some(zysk / ryzyko)
    } else {
        None
    }
}

/// Odległość do najbliższego okrągłego poziomu o danym kroku, ze znakiem:
/// dodatnia = poziom jest wyżej.
#[inline]
fn do_okraglego(p: f64, krok: f64) -> f64 {
    let n = (p / krok).round();
    n * krok - p
}

#[inline]
fn w_oknie(h: f64, od: f64, do_: f64) -> bool {
    h >= od && h < do_
}

/// Dzień miesiąca (1–31) dla znacznika w czasie serwera.
fn dzien_miesiaca(ts: Ts) -> u32 {
    // algorytm Howarda Hinnanta (civil_from_days)
    let z = ts.div_euclid(86_400_000) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    (doy - (153 * mp + 2) / 5 + 1) as u32
}

#[inline]
fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[inline]
fn losowa01(ziarno: u64) -> f64 {
    (splitmix64(ziarno) >> 11) as f64 / ((1u64 << 53) as f64)
}

/// Ile bitów zmienia się w wyniku przy przewróceniu jednego bitu wejścia.
///
/// Dobry mikser daje ~32 z 64 (efekt lawinowy). Ta funkcja istnieje, żeby
/// jakość ziarna kontroli była SPRAWDZANA, a nie zakładana — próg istotności
/// całego projektu stoi na dwudziestu kolumnach z tego generatora.
#[cfg(test)]
fn lawina(f: impl Fn(u64) -> u64, wejscie: u64) -> f64 {
    let baza = f(wejscie);
    let mut suma = 0u32;
    for bit in 0..64 {
        suma += (f(wejscie ^ (1u64 << bit)) ^ baza).count_ones();
    }
    suma as f64 / 64.0
}

// ============================================================
//  TESTY
// ============================================================

#[cfg(test)]
mod testy {
    use super::*;
    use crate::types::{BasketEvent, GridLevel, SourceKey};

    fn q(ts_min: i64, bid: f64, ask: f64) -> Quote {
        Quote {
            ts: ts_min * 60_000,
            bid,
            ask,
        }
    }

    fn konto(saldo: f64) -> Account {
        Account {
            balance: saldo,
            equity: saldo,
            margin: 0.0,
            free_margin: saldo,
            leverage: 500,
            credit: 0.0,
        }
    }

    fn koszyk(id: u32, side: Side, lo: f64, hi: f64, ts: Ts) -> Basket {
        Basket {
            id,
            source: SourceKey::new(1, None),
            source_name: "test".into(),
            msg_id: id as i64,
            pending_exit: None,
            pending_relot_review: Vec::new(),
            entry_edit_state: None,
            msg_aliases: vec![],
            persisted_done_actions: vec![],
            side,
            is_limit: true,
            is_stop: false,
            entry_lo: lo,
            entry_hi: hi,
            zone_lo: lo,
            zone_hi: hi,
            sl: Some(if matches!(side, Side::Buy) {
                lo - 5.0
            } else {
                hi + 5.0
            }),
            tps: vec![
                if matches!(side, Side::Buy) {
                    hi + 3.0
                } else {
                    lo - 3.0
                },
                if matches!(side, Side::Buy) {
                    hi + 7.0
                } else {
                    lo - 7.0
                },
                if matches!(side, Side::Buy) {
                    hi + 12.0
                } else {
                    lo - 12.0
                },
            ],
            tp_stage: 0,
            plan_wykonany_do: 0,
            created_ts: ts,
            drop_po_ts: 0,
            state: BasketState::Working,
            tickets: vec![],
            pendings: vec![],
            realized: 0.0,
            events: Vec::<BasketEvent>::new(),
            levels: vec![],
            reentries: 0,
            last_entry_px: None,
            tp_touch_px: Vec::new(),
            sl_touch_ts: 0,
            sl_touch_px: 0.0,
            secured: false,
            rearm_blocked_by_spp: false,
            secured_by_rule: false,
            had_positions: false,
            tp_open: false,
            warstwy_offset: None,
            rearms: 0,
            last_rearm_ts: 0,
            secured_ts: 0,
            zone_touched: false,
            be_ts: 0,
            drop_armed: false,
            tp_touch_ts: vec![],
            peak_pl_usd: 0.0,
            risk_initial_usd: 0.0,
            adverse_since: 0,
            age_limit_min: 0.0,
            last_tp_ts: 0,
            pyramided: false,
            // ⚠ ODBLOKOWANIE DRZEWA, NIE ZMIANA ZAMIARU — do autora
            // `fast_addon_*`. Pola `last_addon_ts` i `fast_addons` doszły
            // dziś do `types::Basket`, ale ta pomocnicza funkcja testowa
            // nie została zaktualizowana i `conduit-core` NIE KOMPILUJE SIĘ
            // W TRYBIE TESTOWYM — czyli bramka parytetu jest zamknięta dla
            // wszystkich naraz. Wstawiam WARTOŚCI ZEROWE, czyli „ten koszyk
            // nie dostał żadnej dokładki tempowej"; to jest stan neutralny
            // i taki, jaki miały wszystkie koszyki przed dodaniem tych pól.
            last_addon_ts: 0,
            fast_addons: 0,
            tempo_fast: false,
            tempo_checked: false,
            wol_pierwotny: Vec::new(),
        }
    }

    fn pozycja(ticket: u64, basket: u32, side: Side, vol: f64, open: f64, ts: Ts) -> Position {
        Position {
            ticket,
            side,
            volume: vol,
            open_price: open,
            open_ts: ts,
            sl: None,
            tp: None,
            vsl: None,
            basket: Some(basket),
            level: 0,
            frozen: false,
            peak_pts: 0.0,
            last_peak_ts: 0,
            is_runner: false,
            is_toucher: false,
            comment: String::new(),
        }
    }

    /// Karmi obserwator prostą, znaną ścieżką ceny: `n` minut po jednym ticku.
    fn nakarm(o: &mut Obserwator, od_min: i64, ceny: &[f64]) {
        for (i, p) in ceny.iter().enumerate() {
            let t = od_min + i as i64;
            o.na_ticku(&q(t, *p - 0.10, *p + 0.10), &[], &[]);
        }
    }

    fn nakarm_gesto(o: &mut Obserwator, od_min: i64, ceny: &[f64]) {
        for (i, p) in ceny.iter().enumerate() {
            let t = (od_min + i as i64) * 60_000;
            o.na_ticku(
                &Quote {
                    ts: t,
                    bid: p - 0.10,
                    ask: p + 0.10,
                },
                &[],
                &[],
            );
            o.na_ticku(
                &Quote {
                    ts: t + 30_000,
                    bid: p + 0.65,
                    ask: p + 0.85,
                },
                &[],
                &[],
            );
        }
    }

    static KONTO_TEST: Account = Account {
        balance: 200.0,
        equity: 200.0,
        margin: 0.0,
        free_margin: 200.0,
        leverage: 500,
        credit: 0.0,
    };

    /// Najprostszy możliwy kontekst sygnału — do testów, które badają wyłącznie
    /// rynek albo kontrolę losową.
    fn kontekst_prosty(ts: Ts, qq: &Quote) -> KontekstSygnalu<'_> {
        KontekstSygnalu {
            ogolny: KontekstOgolny {
                ts,
                q: qq,
                konto: &KONTO_TEST,
                koszyki: &[],
                pozycje: &[],
                szczyt_equity: 200.0,
                equity_startu_dnia: 200.0,
                saldo_startowe: 200.0,
                zrealizowane_dzis: 0.0,
            },
            side: Side::Buy,
            strefa_lo: 3995.0,
            strefa_hi: 4005.0,
            sl: Some(3990.0),
            tps: &[4010.0],
            tp_open: false,
            jest_limit: true,
            jest_stop: false,
            tag_high_risk: false,
            tag_may_not: false,
            tag_first_entry: false,
            klucz: 1,
        }
    }

    fn ogolny<'a>(
        ts: Ts,
        qq: &'a Quote,
        k: &'a Account,
        koszyki: &'a [Basket],
        pozycje: &'a [Position],
    ) -> KontekstOgolny<'a> {
        KontekstOgolny {
            ts,
            q: qq,
            konto: k,
            koszyki,
            pozycje,
            szczyt_equity: 200.0,
            equity_startu_dnia: 200.0,
            saldo_startowe: 200.0,
            zrealizowane_dzis: 0.0,
        }
    }

    // ---------- szkielet ----------

    #[test]
    fn metadane_sa_spojne() {
        assert_eq!(
            OPISY.len(),
            N_CECH,
            "liczba opisów musi równać się długości wektora"
        );
        let mut widziane = std::collections::HashSet::new();
        for o in OPISY {
            assert!(
                widziane.insert(o.nazwa),
                "zduplikowana nazwa cechy: {}",
                o.nazwa
            );
        }
        // losowe muszą siedzieć na KOŃCU — inaczej dopisanie cechy przesuwa
        // kontrolę i unieważnia wagi modelu
        for i in pierwsza_losowa()..N_CECH {
            assert_eq!(
                OPISY[i].rodzina,
                Rodzina::Losowa,
                "cecha {i} nie jest losowa"
            );
        }
        assert_eq!(N_CECH - pierwsza_losowa(), N_LOSOWYCH);
    }

    /// UKŁAD WEKTORA JEST ZAMROŻONY — ten test pilnuje, żeby nikt go nie
    /// przestawił po cichu.
    ///
    /// Model AI zapisuje wagi indeksami. Przestawienie albo wstawienie cechy
    /// w środku unieważnia KAŻDY wytrenowany model, a objawia się dopiero
    /// gorszym wynikiem — czyli w miejscu, w którym nikt nie szuka przyczyny
    /// w kolejności kolumn. Dlatego zmiana układu ma boleć TUTAJ.
    ///
    /// Wolno dopisywać wyłącznie NA KOŃCU, przed blokiem losowych, i wtedy
    /// trzeba świadomie podnieść `WERSJA_OBSERWACJI` i odcisk poniżej.
    #[test]
    fn uklad_wektora_jest_zamrozony() {
        assert_eq!(WERSJA_OBSERWACJI, 1);
        assert_eq!(N_CECH, 199, "zmiana liczby cech = zmiana układu");
        assert_eq!(pierwsza_losowa(), 179);

        // kotwice: pierwsza cecha każdej rodziny musi stać tam, gdzie stała
        assert_eq!(nazwa(C::cena_mid_usd), "cena_mid_usd");
        assert_eq!(C::cena_mid_usd as usize, 0);
        assert_eq!(nazwa(C::mom_1m_usd), "mom_1m_usd");
        assert_eq!(nazwa(C::n_sygnalow_1h), "n_sygnalow_1h");
        assert_eq!(nazwa(C::min_od_sygnalu), "min_od_sygnalu");
        assert_eq!(nazwa(C::n_koszykow_otwartych), "n_koszykow_otwartych");
        assert_eq!(nazwa(C::wiek_koszyka_min), "wiek_koszyka_min");
        assert_eq!(nazwa(C::losowa_01), "losowa_01");
        assert_eq!(OPISY[N_CECH - 1].nazwa, "losowa_20");

        // odcisk całej listy nazw w kolejności
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for o in OPISY {
            for b in o.nazwa.as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01B3);
            }
            h ^= 0xFF;
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
        assert_eq!(
            h, 0x411b_d351_abe2_147d,
            "kolejność albo nazwy cech się zmieniły — jeśli świadomie, \
             podnieś WERSJA_OBSERWACJI i wpisz nowy odcisk {h:#018x}"
        );
    }

    #[test]
    fn nazwy_niosa_jednostke_albo_sa_jawne() {
        // reguła właściciela: `zmiennosc_1h_usd`, nie `vol1`
        for o in OPISY {
            assert!(o.nazwa.len() >= 5, "nazwa za krótka: {}", o.nazwa);
            assert!(
                !o.nazwa.chars().next().unwrap().is_ascii_digit(),
                "nazwa zaczyna się cyfrą: {}",
                o.nazwa
            );
        }
    }

    // ---------- rodzina B: rynek ----------

    #[test]
    fn zakres_i_atr_licza_sie_z_zamknietych_minut() {
        let mut o = Obserwator::default();
        // 120 minut piły 4000 -> 4010 -> 4000
        let mut ceny = Vec::new();
        for i in 0..120 {
            ceny.push(4000.0 + (i % 11) as f64);
        }
        nakarm(&mut o, 100_000, &ceny);
        let qq = q(100_120, 4004.9, 4005.1);
        let k = konto(200.0);
        let ctx = KontekstSygnalu {
            ogolny: ogolny(qq.ts, &qq, &k, &[], &[]),
            side: Side::Buy,
            strefa_lo: 4000.0,
            strefa_hi: 4005.0,
            sl: Some(3995.0),
            tps: &[4010.0, 4015.0],
            tp_open: false,
            jest_limit: true,
            jest_stop: false,
            tag_high_risk: false,
            tag_may_not: false,
            tag_first_entry: false,
            klucz: 7,
        };
        let ob = o.obserwuj_sygnal(&ctx);
        // zakres 60 min piły o okresie 11 i amplitudzie 10 $ = dokładnie 10 $
        assert!(ob.wiadomo(C::zakres_60m_usd));
        assert!((ob.wartosc(C::zakres_60m_usd) - 10.0).abs() < 1e-3);
        // jeden tick na minutę => ATR (średni zakres świecy) = 0
        assert!(ob.wiadomo(C::atr60_usd));
        assert!(ob.wartosc(C::atr60_usd).abs() < 1e-6);
        assert!((ob.wartosc(C::tickow_na_minute) - 1.0).abs() < 1e-6);
        assert!((ob.wartosc(C::spread_usd) - 0.2).abs() < 1e-3);
    }

    #[test]
    fn brak_danych_daje_maske_false_a_nie_zero() {
        let o = Obserwator::default();
        let qq = q(0, 3999.9, 4000.1);
        let k = konto(200.0);
        let ctx = KontekstSygnalu {
            ogolny: ogolny(qq.ts, &qq, &k, &[], &[]),
            side: Side::Buy,
            strefa_lo: 4000.0,
            strefa_hi: 4005.0,
            sl: Some(3995.0),
            tps: &[4010.0],
            tp_open: false,
            jest_limit: true,
            jest_stop: false,
            tag_high_risk: false,
            tag_may_not: false,
            tag_first_entry: false,
            klucz: 1,
        };
        let ob = o.obserwuj_sygnal(&ctx);
        // pusty bufor rynku: zmienność NIE JEST zerem, jest nieznana
        assert!(!ob.wiadomo(C::zmiennosc_60m_usd));
        assert_eq!(ob.wartosc(C::zmiennosc_60m_usd), 0.0);
        assert!(!ob.wiadomo(C::atr60_usd));
        // a to, co da się policzyć z samego kwotowania, jest znane
        assert!(ob.wiadomo(C::spread_usd));
    }

    #[test]
    fn okragle_poziomy_i_kalendarz() {
        // znak dodatni = najbliższy okrągły poziom leży WYŻEJ
        assert!(
            (do_okraglego(4003.0, 10.0) + 3.0).abs() < 1e-9,
            "najbliższy to 4000"
        );
        assert!(
            (do_okraglego(4007.0, 10.0) - 3.0).abs() < 1e-9,
            "najbliższy to 4010"
        );
        assert!(
            (do_okraglego(4049.0, 100.0) + 49.0).abs() < 1e-9,
            "najbliższy to 4000"
        );
        assert!(
            (do_okraglego(4051.0, 100.0) - 49.0).abs() < 1e-9,
            "najbliższy to 4100"
        );
        assert!((do_okraglego(4025.0, 50.0) - 25.0).abs() < 1e-9);
        // dni liczone od 1970-01-01, którą znamy na pamięć
        let d = |n: i64| dzien_miesiaca(n * 86_400_000);
        assert_eq!(d(0), 1); // 1970-01-01
        assert_eq!(d(30), 31); // 1970-01-31
        assert_eq!(d(31), 1); // 1970-02-01
        assert_eq!(d(58), 28); // 1970-02-28
        assert_eq!(d(59), 1); // 1970-03-01
        assert_eq!(d(364), 31); // 1970-12-31
        assert_eq!(d(365), 1); // 1971-01-01
    }

    #[test]
    fn sesje_licza_sie_w_utc_mimo_zegara_serwera() {
        let mut o = Obserwator::default();
        nakarm(&mut o, 0, &[4000.0; 5]);
        let k = konto(200.0);
        // 10:00 czasu serwera = 07:00 UTC = otwarcie Londynu
        let ts = 10 * 3_600_000;
        let qq = Quote {
            ts,
            bid: 3999.9,
            ask: 4000.1,
        };
        let ctx = KontekstSygnalu {
            ogolny: ogolny(ts, &qq, &k, &[], &[]),
            side: Side::Buy,
            strefa_lo: 3999.0,
            strefa_hi: 4001.0,
            sl: Some(3995.0),
            tps: &[4005.0],
            tp_open: false,
            jest_limit: true,
            jest_stop: false,
            tag_high_risk: false,
            tag_may_not: false,
            tag_first_entry: false,
            klucz: 1,
        };
        let ob = o.obserwuj_sygnal(&ctx);
        assert_eq!(ob.wartosc(C::sesja_londyn), 1.0);
        assert_eq!(ob.wartosc(C::sesja_azja), 1.0, "07:00 UTC to jeszcze Azja");
        assert_eq!(ob.wartosc(C::nakladanie_azja_ldn), 1.0);
        assert_eq!(ob.wartosc(C::sesja_ny), 0.0);
    }

    // ---------- rodzina C: struktura ----------

    #[test]
    fn momentum_jest_znakowane_kierunkiem() {
        let mut o = Obserwator::default();
        // 70 minut równomiernego wzrostu po 1 $/min
        let ceny: Vec<f64> = (0..70).map(|i| 4000.0 + i as f64).collect();
        nakarm(&mut o, 200_000, &ceny);
        let qq = q(200_070, 4069.9, 4070.1);
        let k = konto(200.0);

        let mk = |side: Side| KontekstSygnalu {
            ogolny: ogolny(qq.ts, &qq, &k, &[], &[]),
            side,
            strefa_lo: 4060.0,
            strefa_hi: 4070.0,
            sl: Some(4050.0),
            tps: &[4080.0],
            tp_open: false,
            jest_limit: true,
            jest_stop: false,
            tag_high_risk: false,
            tag_may_not: false,
            tag_first_entry: false,
            klucz: 3,
        };
        let buy = o.obserwuj_sygnal(&mk(Side::Buy));
        let sell = o.obserwuj_sygnal(&mk(Side::Sell));
        assert!(buy.wartosc(C::mom_60m_usd) > 55.0, "wzrost 60 $ w godzinę");
        assert!(
            (buy.wartosc(C::mom_60m_usd) + sell.wartosc(C::mom_60m_usd)).abs() < 1e-3,
            "ten sam ruch dla SELL musi mieć znak przeciwny"
        );
        assert!(buy.wartosc(C::cena_minus_sr_60m_usd) > 0.0);
        assert!(sell.wartosc(C::cena_minus_sr_60m_usd) < 0.0);
        assert!(buy.wartosc(C::udzial_minut_zgodnych_60m) > 0.9);
        assert!(sell.wartosc(C::udzial_minut_zgodnych_60m) < 0.1);
    }

    // ---------- rodzina D: kanał i geometria ----------

    #[test]
    fn rr_rosnie_z_glebokoscia_wejscia() {
        let o = Obserwator::default();
        let k = konto(200.0);
        let qq = q(10, 4004.9, 4005.1);
        let ctx = KontekstSygnalu {
            ogolny: ogolny(qq.ts, &qq, &k, &[], &[]),
            side: Side::Buy,
            strefa_lo: 4000.0,
            strefa_hi: 4010.0,
            sl: Some(3995.0),
            tps: &[4020.0, 4030.0],
            tp_open: true,
            jest_limit: true,
            jest_stop: false,
            tag_high_risk: true,
            tag_may_not: false,
            tag_first_entry: false,
            klucz: 42,
        };
        let ob = o.obserwuj_sygnal(&ctx);
        // BUY, lepsza krawędź = 4000: ryzyko 5, zysk 20 -> 4.0
        assert!((ob.wartosc(C::rr_krawedz_lepsza) - 4.0).abs() < 1e-4);
        assert!((ob.wartosc(C::rr_srodek_strefy) - 1.5).abs() < 1e-4);
        // gorsza krawędź 4010: ryzyko 15, zysk 10 -> 0.6667
        assert!((ob.wartosc(C::rr_krawedz_gorsza) - 2.0 / 3.0).abs() < 1e-4);
        assert!(ob.wartosc(C::rr_krawedz_lepsza) > ob.wartosc(C::rr_krawedz_gorsza));
        assert!((ob.wartosc(C::szerokosc_strefy_usd) - 10.0).abs() < 1e-6);
        assert!((ob.wartosc(C::rozstaw_celow_usd) - 10.0).abs() < 1e-6);
        assert_eq!(ob.wartosc(C::n_celow), 2.0);
        assert_eq!(ob.wartosc(C::tag_high_risk), 1.0);
        assert_eq!(ob.wartosc(C::tp_open), 1.0);
        // cena w środku strefy -> znormalizowana pozycja 0.5, dystans 0
        assert!((ob.wartosc(C::cena_wzgl_strefy_norm) - 0.5).abs() < 1e-4);
        assert!(ob.wartosc(C::cena_do_strefy_usd).abs() < 1e-6);
    }

    #[test]
    fn rr_dla_sell_jest_lustrzane() {
        let o = Obserwator::default();
        let k = konto(200.0);
        let qq = q(10, 4004.9, 4005.1);
        let ctx = KontekstSygnalu {
            ogolny: ogolny(qq.ts, &qq, &k, &[], &[]),
            side: Side::Sell,
            strefa_lo: 4000.0,
            strefa_hi: 4010.0,
            sl: Some(4015.0),
            tps: &[3990.0],
            tp_open: false,
            jest_limit: true,
            jest_stop: false,
            tag_high_risk: false,
            tag_may_not: false,
            tag_first_entry: false,
            klucz: 43,
        };
        let ob = o.obserwuj_sygnal(&ctx);
        // SELL: lepsza krawędź = 4010, ryzyko 5, zysk 20 -> 4.0
        assert!((ob.wartosc(C::rr_krawedz_lepsza) - 4.0).abs() < 1e-4);
        assert!((ob.wartosc(C::dystans_sl_usd) - 10.0).abs() < 1e-4);
    }

    #[test]
    fn rytm_kanalu_liczy_tylko_przeszlosc() {
        let mut o = Obserwator::default();
        let baza = 1_000_000i64 * 60_000;
        o.na_sygnale(baza, Side::Buy);
        o.na_sygnale(baza + 10 * 60_000, Side::Buy);
        o.na_sygnale(baza + 20 * 60_000, Side::Sell);
        // sygnał z PRZYSZŁOŚCI względem punktu decyzyjnego
        o.na_sygnale(baza + 90 * 60_000, Side::Buy);
        o.na_zamknieciu(baza + 5 * 60_000, 12.0);
        o.na_zamknieciu(baza + 15 * 60_000, 8.0);

        let ts = baza + 30 * 60_000;
        let qq = Quote {
            ts,
            bid: 3999.9,
            ask: 4000.1,
        };
        let k = konto(200.0);
        let ctx = KontekstSygnalu {
            ogolny: ogolny(ts, &qq, &k, &[], &[]),
            side: Side::Buy,
            strefa_lo: 3995.0,
            strefa_hi: 4005.0,
            sl: Some(3990.0),
            tps: &[4010.0],
            tp_open: false,
            jest_limit: true,
            jest_stop: false,
            tag_high_risk: false,
            tag_may_not: false,
            tag_first_entry: false,
            klucz: 5,
        };
        let ob = o.obserwuj_sygnal(&ctx);
        assert_eq!(
            ob.wartosc(C::n_sygnalow_1h),
            3.0,
            "czwarty jest w przyszłości"
        );
        assert_eq!(ob.wartosc(C::n_sygnalow_24h), 3.0);
        assert!((ob.wartosc(C::min_od_poprzedniego_sygnalu) - 10.0).abs() < 1e-4);
        assert!((ob.wartosc(C::udzial_buy_24h) - 2.0 / 3.0).abs() < 1e-4);
        assert_eq!(ob.wartosc(C::seria_wygranych), 2.0);
        assert_eq!(ob.wartosc(C::seria_przegranych), 0.0);
        assert!((ob.wartosc(C::wynik_ostatnich_5_usd) - 20.0).abs() < 1e-4);
        assert!((ob.wartosc(C::min_od_ostatniego_zamkniecia) - 15.0).abs() < 1e-4);
    }

    #[test]
    fn nakladanie_stref_i_koszyk_przeciwny() {
        let o = Obserwator::default();
        let k = konto(200.0);
        let ts = 60_000i64 * 500;
        let qq = Quote {
            ts,
            bid: 3999.9,
            ask: 4000.1,
        };
        let mut b1 = koszyk(1, Side::Buy, 3998.0, 4004.0, ts - 60_000);
        b1.had_positions = true;
        let mut b2 = koszyk(2, Side::Sell, 4100.0, 4110.0, ts - 60_000);
        b2.had_positions = true;
        let koszyki = vec![b1, b2];
        let ctx = KontekstSygnalu {
            ogolny: ogolny(ts, &qq, &k, &koszyki, &[]),
            side: Side::Buy,
            strefa_lo: 4000.0,
            strefa_hi: 4010.0, // nakłada się z b1 na odcinku 4000-4004 = 40 %
            sl: Some(3995.0),
            tps: &[4020.0],
            tp_open: false,
            jest_limit: true,
            jest_stop: false,
            tag_high_risk: false,
            tag_may_not: false,
            tag_first_entry: false,
            klucz: 9,
        };
        let ob = o.obserwuj_sygnal(&ctx);
        assert!((ob.wartosc(C::nakladanie_z_otwartym) - 0.4).abs() < 1e-4);
        assert_eq!(ob.wartosc(C::jest_koszyk_przeciwny), 1.0);
        assert_eq!(ob.wartosc(C::n_koszykow_zgodnych), 1.0);
    }

    // ---------- rodzina A: dynamika dojścia ----------

    #[test]
    fn dynamika_dojscia_do_strefy_mierzy_predkosc_i_dotkniecia() {
        let mut o = Obserwator::default();
        let start = 300_000i64;
        // rozgrzewka: 60 minut płasko na 4020
        nakarm(&mut o, start, &[4020.0; 60]);

        let ts0 = (start + 60) * 60_000;
        let b = koszyk(1, Side::Buy, 4000.0, 4005.0, ts0);
        let koszyki = vec![b];

        // cena spada 4020 -> 4004 przez 8 minut (2 $/min), wchodzi w strefę
        for i in 0..9 {
            let p = 4020.0 - 2.0 * i as f64;
            let t = (start + 60 + i) * 60_000;
            o.na_ticku(
                &Quote {
                    ts: t,
                    bid: p - 0.1,
                    ask: p + 0.1,
                },
                &koszyki,
                &[],
            );
        }
        // wychodzi w górę
        for i in 9..14 {
            let p = 4010.0;
            let t = (start + 60 + i) * 60_000;
            o.na_ticku(
                &Quote {
                    ts: t,
                    bid: p - 0.1,
                    ask: p + 0.1,
                },
                &koszyki,
                &[],
            );
        }
        // wraca do strefy drugi raz
        for i in 14..18 {
            let p = 4002.0;
            let t = (start + 60 + i) * 60_000;
            o.na_ticku(
                &Quote {
                    ts: t,
                    bid: p - 0.1,
                    ask: p + 0.1,
                },
                &koszyki,
                &[],
            );
        }

        let ts = (start + 60 + 17) * 60_000;
        let qq = Quote {
            ts,
            bid: 4001.9,
            ask: 4002.1,
        };
        let k = konto(200.0);
        let ctx = KontekstKoszyka {
            ogolny: ogolny(ts, &qq, &k, &koszyki, &[]),
            koszyk: &koszyki[0],
            widok: None,
        };
        let ob = o.obserwuj_koszyk(&ctx);

        assert_eq!(
            ob.wartosc(C::n_dotkniec_strefy),
            2.0,
            "dwa wejścia do strefy"
        );
        assert_eq!(ob.wartosc(C::w_strefie_teraz), 1.0);
        assert!(ob.wiadomo(C::min_do_1_dotkniecia));
        assert!((ob.wartosc(C::min_do_1_dotkniecia) - 8.0).abs() < 1e-3);
        assert!(ob.wiadomo(C::predkosc_dojscia_usd_min));
        assert!(
            (ob.wartosc(C::predkosc_dojscia_usd_min) + 2.0).abs() < 0.05,
            "spadek 2 $/min, znak ujemny: {}",
            ob.wartosc(C::predkosc_dojscia_usd_min)
        );
        // pierwszy tick w strefie wylądował na 4004, czyli 1 $ pod górną
        // krawędzią 4005 — to jest luka wejścia
        assert!((ob.wartosc(C::luka_wejscia_usd) - 1.0).abs() < 1e-3);
        assert_eq!(ob.wartosc(C::weszla_luka), 1.0);
        // najgłębiej cena była na 4002 -> 3 $ od krawędzi 4005, szerokość 5
        assert!((ob.wartosc(C::max_glebokosc_usd) - 3.0).abs() < 1e-3);
        assert!((ob.wartosc(C::max_glebokosc_w_szer) - 0.6).abs() < 1e-3);
        assert_eq!(ob.wartosc(C::przebita_na_wylot), 0.0);
        assert!(ob.wartosc(C::min_od_wyjscia_ze_strefy) > 0.0);
        assert!((ob.wartosc(C::min_od_sygnalu) - 17.0).abs() < 1e-3);
    }

    #[test]
    fn dynamika_nieznana_zanim_cena_dotknie_strefy() {
        let mut o = Obserwator::default();
        let start = 400_000i64;
        nakarm(&mut o, start, &[4050.0; 60]);
        let ts0 = (start + 60) * 60_000;
        let b = koszyk(1, Side::Buy, 4000.0, 4005.0, ts0);
        let koszyki = vec![b];
        for i in 0..5 {
            let t = (start + 60 + i) * 60_000;
            o.na_ticku(
                &Quote {
                    ts: t,
                    bid: 4049.9,
                    ask: 4050.1,
                },
                &koszyki,
                &[],
            );
        }
        let ts = (start + 64) * 60_000;
        let qq = Quote {
            ts,
            bid: 4049.9,
            ask: 4050.1,
        };
        let k = konto(200.0);
        let ob = o.obserwuj_koszyk(&KontekstKoszyka {
            ogolny: ogolny(ts, &qq, &k, &koszyki, &[]),
            koszyk: &koszyki[0],
            widok: None,
        });
        // TO JEST SEDNO: „nie dotknęła jeszcze" to NIE jest zero
        assert!(!ob.wiadomo(C::min_do_1_dotkniecia));
        assert!(!ob.wiadomo(C::predkosc_dojscia_usd_min));
        assert!(!ob.wiadomo(C::max_glebokosc_w_szer));
        assert_eq!(ob.wartosc(C::n_dotkniec_strefy), 0.0);
        assert!(
            ob.wiadomo(C::n_dotkniec_strefy),
            "zero dotknięć to znana liczba"
        );
        assert!((ob.wartosc(C::cena_do_strefy_usd) - 45.0).abs() < 1e-3);
    }

    #[test]
    fn przebicie_strefy_na_wylot() {
        let mut o = Obserwator::default();
        let start = 500_000i64;
        nakarm(&mut o, start, &[4020.0; 60]);
        let ts0 = (start + 60) * 60_000;
        let b = koszyk(1, Side::Buy, 4000.0, 4005.0, ts0);
        let koszyki = vec![b];
        for (i, p) in [4020.0, 4010.0, 4003.0, 3998.0, 3990.0].iter().enumerate() {
            let t = (start + 60 + i as i64) * 60_000;
            o.na_ticku(
                &Quote {
                    ts: t,
                    bid: p - 0.1,
                    ask: p + 0.1,
                },
                &koszyki,
                &[],
            );
        }
        let ts = (start + 64) * 60_000;
        let qq = Quote {
            ts,
            bid: 3989.9,
            ask: 3990.1,
        };
        let k = konto(200.0);
        let ob = o.obserwuj_koszyk(&KontekstKoszyka {
            ogolny: ogolny(ts, &qq, &k, &koszyki, &[]),
            koszyk: &koszyki[0],
            widok: None,
        });
        assert_eq!(ob.wartosc(C::przebita_na_wylot), 1.0);
        assert!(ob.wartosc(C::max_glebokosc_w_szer) > 1.0);
    }

    // ---------- rodzina E: portfel ----------

    #[test]
    fn portfel_sumuje_ekspozycje_i_wynik() {
        let o = Obserwator::default();
        let ts = 60_000i64 * 900;
        let qq = Quote {
            ts,
            bid: 4009.9,
            ask: 4010.1,
        };
        let b = koszyk(1, Side::Buy, 4000.0, 4005.0, ts - 600_000);
        let koszyki = vec![b];
        let pozycje = vec![
            pozycja(1, 1, Side::Buy, 0.01, 4000.0, ts - 600_000),
            pozycja(2, 1, Side::Buy, 0.02, 4002.0, ts - 300_000),
        ];
        let mut k = konto(200.0);
        k.equity = 215.0;
        k.margin = 40.0;
        k.free_margin = 175.0;
        let mut g = ogolny(ts, &qq, &k, &koszyki, &pozycje);
        g.zrealizowane_dzis = 5.0;
        g.szczyt_equity = 250.0;
        let ob = o.obserwuj_koszyk(&KontekstKoszyka {
            ogolny: g,
            koszyk: &koszyki[0],
            widok: None,
        });

        assert_eq!(ob.wartosc(C::n_pozycji_otwartych), 2.0);
        assert!((ob.wartosc(C::ekspozycja_lotow) - 0.03).abs() < 1e-6);
        assert!((ob.wartosc(C::ekspozycja_netto_lotow) - 0.03).abs() < 1e-6);
        assert!((ob.wartosc(C::wynik_niezrealizowany_usd) - 25.7).abs() < 1e-2);
        assert!((ob.wartosc(C::wynik_dnia_zrealizowany_usd) - 5.0).abs() < 1e-6);
        assert!((ob.wartosc(C::equity_do_szczytu_pct) + 14.0).abs() < 1e-2);
        assert!((ob.wartosc(C::margines_uzyty_pct) - 40.0 / 215.0 * 100.0).abs() < 1e-2);
    }

    // ---------- rodzina F: KOSZYK JAKO CAŁOŚĆ ----------

    #[test]
    fn koszyk_liczony_jest_w_calosci_a_nie_po_pozycjach() {
        let o = Obserwator::default();
        let ts = 60_000i64 * 1000;
        let qq = Quote {
            ts,
            bid: 4010.0,
            ask: 4010.2,
        };
        let mut b = koszyk(1, Side::Buy, 4000.0, 4006.0, ts - 3_600_000);
        b.realized = 7.5;
        b.tp_stage = 1;
        b.levels = vec![
            GridLevel {
                price: 4004.0,
                base_units: 1,
                volume: 0.01,
                sl: None,
                tp: None,
                level: 0,
                is_toucher: false,
                filled: true,
                fill_ts: 0,
                fill_px: 0.0,
                cancelled: false,
            },
            GridLevel {
                price: 4002.0,
                base_units: 1,
                volume: 0.01,
                sl: None,
                tp: None,
                level: 1,
                is_toucher: false,
                filled: true,
                fill_ts: 0,
                fill_px: 0.0,
                cancelled: false,
            },
            GridLevel {
                price: 4000.0,
                base_units: 1,
                volume: 0.02,
                sl: None,
                tp: None,
                level: 2,
                is_toucher: false,
                filled: false,
                fill_ts: 0,
                fill_px: 0.0,
                cancelled: false,
            },
        ];
        b.pendings = vec![99];
        let koszyki = vec![b];

        let mut p1 = pozycja(1, 1, Side::Buy, 0.01, 4004.0, ts - 3_000_000);
        p1.sl = Some(3999.0);
        let mut p2 = pozycja(2, 1, Side::Buy, 0.03, 4002.0, ts - 1_200_000);
        p2.sl = Some(3999.0);
        let pozycje = vec![p1, p2];

        let k = konto(200.0);
        let ob = o.obserwuj_koszyk(&KontekstKoszyka {
            ogolny: ogolny(ts, &qq, &k, &koszyki, &pozycje),
            koszyk: &koszyki[0],
            widok: None,
        });

        // lot łączny = 0.04
        assert!((ob.wartosc(C::lot_koszyka) - 0.04).abs() < 1e-6);
        // średnia ważona = (4004*0.01 + 4002*0.03)/0.04 = 4002.5
        assert!((ob.wartosc(C::srednia_wazona_cena_wejscia) - 4002.5).abs() < 1e-3);
        assert!((ob.wartosc(C::cena_minus_srednia_wejscia_usd) - 7.5).abs() < 1e-3);
        // niezrealizowany = 0.01*6*100 + 0.03*8*100 = 6 + 24 = 30
        assert!((ob.wartosc(C::zysk_koszyka_niezreal_usd) - 30.0).abs() < 1e-3);
        assert!((ob.wartosc(C::zrealizowane_koszyka_usd) - 7.5).abs() < 1e-3);
        // ŁĄCZNIE = 37.5 — to jest liczba, o którą chodziło właścicielowi
        assert!((ob.wartosc(C::wynik_koszyka_lacznie_usd) - 37.5).abs() < 1e-3);
        // ryzyko koszyka = 0.01*5*100 + 0.03*3*100 = 5 + 9 = 14
        assert!((ob.wartosc(C::ryzyko_koszyka_usd) - 14.0).abs() < 1e-3);
        assert!((ob.wartosc(C::wynik_koszyka_w_r) - 37.5 / 14.0).abs() < 1e-3);
        // warstwy
        assert_eq!(ob.wartosc(C::warstwy_zaplanowane), 3.0);
        assert_eq!(ob.wartosc(C::warstwy_wypelnione), 2.0);
        assert_eq!(ob.wartosc(C::warstwy_czekajace), 1.0);
        assert!((ob.wartosc(C::udzial_wypelnienia) - 2.0 / 3.0).abs() < 1e-4);
        // ryzyko w rynku = 0.04 / 0.04 zaplanowanych lotów = 1.0
        assert!((ob.wartosc(C::udzial_ryzyka_w_rynku) - 1.0).abs() < 1e-4);
        // głębokość: średnia 4002.5 wobec lepszej krawędzi 4000 przy szer. 6
        assert!((ob.wartosc(C::glebokosc_wejscia_w_szer) - 2.5 / 6.0).abs() < 1e-4);
        // odległości: exit dla BUY to bid = 4010
        assert!((ob.wartosc(C::do_sl_usd) - 15.0).abs() < 1e-3); // SL koszyka 3995
        assert!((ob.wartosc(C::do_tp1_usd) + 1.0).abs() < 1e-3); // TP1 = 4009 -> minięty
        assert_eq!(ob.wartosc(C::etap_tp), 1.0);
        assert_eq!(ob.wartosc(C::stan_working), 1.0);
        assert_eq!(ob.wartosc(C::stan_riskfree), 0.0);
    }

    /// Trzy warstwy o różnych cenach wejścia — arytmetyka „risk free" kanału.
    ///
    /// To jest liczba, której właściciel szukał: ile warstw domknąć, żeby
    /// CAŁOŚĆ przestała móc stracić, i gdzie wtedy ląduje stop reszty.
    #[test]
    fn risk_free_liczy_ile_warstw_domknac_i_gdzie_ladnie_stop() {
        let o = Obserwator::default();
        let ts = 60_000i64 * 2_000;
        // BUY, wyjście po bid = 4012
        let qq = Quote {
            ts,
            bid: 4012.0,
            ask: 4012.2,
        };
        let b = koszyk(1, Side::Buy, 4000.0, 4010.0, ts - 3_600_000); // SL = 3995
        let koszyki = vec![b];
        let pozycje = vec![
            pozycja(1, 1, Side::Buy, 0.01, 4008.0, ts - 3_000_000), // najpłytsza
            pozycja(2, 1, Side::Buy, 0.01, 4004.0, ts - 2_000_000),
            pozycja(3, 1, Side::Buy, 0.01, 4000.0, ts - 1_000_000), // najgłębsza
        ];
        let ob = o.obserwuj_koszyk(&KontekstKoszyka {
            ogolny: ogolny(ts, &qq, &KONTO_TEST, &koszyki, &pozycje),
            koszyk: &koszyki[0],
            widok: None,
        });

        // warstwy uporządkowane od NAJGŁĘBSZEJ
        assert!((ob.wartosc(C::warstwa_1_cena_wejscia) - 4000.0).abs() < 1e-3);
        assert!((ob.wartosc(C::warstwa_3_cena_wejscia) - 4008.0).abs() < 1e-3);
        assert!((ob.wartosc(C::warstwa_1_wynik_usd) - 12.0).abs() < 1e-3);
        assert!((ob.wartosc(C::warstwa_2_wynik_usd) - 8.0).abs() < 1e-3);
        assert!((ob.wartosc(C::warstwa_3_wynik_usd) - 4.0).abs() < 1e-3);
        assert!((ob.wartosc(C::warstwa_najplytsza_wynik_usd) - 4.0).abs() < 1e-3);
        assert!((ob.wartosc(C::rozpietosc_wejsc_koszyka_usd) - 8.0).abs() < 1e-3);
        // czwarta warstwa nie istnieje — maska musi to powiedzieć
        assert!(!ob.wiadomo(C::warstwa_4_wynik_usd));

        // całość: 12 + 8 + 4 = 24 $, lot 0.03 -> do BE brakuje −8 $ (jest ponad)
        assert!((ob.wartosc(C::wynik_koszyka_lacznie_usd) - 24.0).abs() < 1e-3);
        assert!((ob.wartosc(C::dystans_do_be_koszyka_usd) + 8.0).abs() < 1e-3);

        // RISK FREE: ryzyko całości na SL 3995 = 5+9+13 = 27 $.
        // Domknięcie dwóch najpłytszych daje 4+8 = 12 $, a ryzyko reszty
        // (sama warstwa 4000) to 5 $ — czyli dopiero wtedy koszyk nie może
        // już stracić.
        assert_eq!(ob.wartosc(C::rf_wykonalne), 1.0);
        assert!((ob.wartosc(C::rf_ile_warstw_domknac) - 2.0).abs() < 1e-6);
        assert!((ob.wartosc(C::rf_zysk_po_domknieciu_usd) - 12.0).abs() < 1e-3);
        assert!((ob.wartosc(C::rf_lot_do_domkniecia) - 0.02).abs() < 1e-6);
        assert!((ob.wartosc(C::rf_be_reszty_usd) - 4000.0).abs() < 1e-3);
    }

    #[test]
    fn risk_free_niewykonalne_gdy_koszyk_pod_woda() {
        let o = Obserwator::default();
        let ts = 60_000i64 * 2_100;
        let qq = Quote {
            ts,
            bid: 3998.0,
            ask: 3998.2,
        };
        let b = koszyk(1, Side::Buy, 4000.0, 4010.0, ts - 3_600_000);
        let koszyki = vec![b];
        let pozycje = vec![
            pozycja(1, 1, Side::Buy, 0.01, 4008.0, ts - 3_000_000),
            pozycja(2, 1, Side::Buy, 0.01, 4004.0, ts - 2_000_000),
            pozycja(3, 1, Side::Buy, 0.01, 4000.0, ts - 1_000_000),
        ];
        let ob = o.obserwuj_koszyk(&KontekstKoszyka {
            ogolny: ogolny(ts, &qq, &KONTO_TEST, &koszyki, &pozycje),
            koszyk: &koszyki[0],
            widok: None,
        });
        assert_eq!(ob.wartosc(C::rf_wykonalne), 0.0);
        assert!(
            ob.wiadomo(C::rf_wykonalne),
            "brak wykonalności to też znana odpowiedź"
        );
        assert!(!ob.wiadomo(C::rf_ile_warstw_domknac), "nie ma czego podać");
        // −2 −6 −10 = −18 $, lot 0.03 -> do BE brakuje +6 $ ceny
        assert!((ob.wartosc(C::wynik_koszyka_lacznie_usd) + 18.0).abs() < 1e-3);
        assert!((ob.wartosc(C::dystans_do_be_koszyka_usd) - 6.0).abs() < 1e-3);
    }

    #[test]
    fn dystans_do_be_dziala_lustrzanie_dla_sell() {
        let o = Obserwator::default();
        let ts = 60_000i64 * 2_200;
        // SELL wychodzi po ask
        let qq = Quote {
            ts,
            bid: 4009.8,
            ask: 4010.0,
        };
        let b = koszyk(1, Side::Sell, 4000.0, 4004.0, ts - 600_000);
        let koszyki = vec![b];
        let pozycje = vec![pozycja(1, 1, Side::Sell, 0.02, 4004.0, ts - 600_000)];
        let ob = o.obserwuj_koszyk(&KontekstKoszyka {
            ogolny: ogolny(ts, &qq, &KONTO_TEST, &koszyki, &pozycje),
            koszyk: &koszyki[0],
            widok: None,
        });
        // strata (4004−4010)·100·0.02 = −12 $; do BE cena musi spaść o 6 $
        assert!((ob.wartosc(C::wynik_koszyka_lacznie_usd) + 12.0).abs() < 1e-3);
        assert!((ob.wartosc(C::dystans_do_be_koszyka_usd) - 6.0).abs() < 1e-3);
        assert!((ob.wartosc(C::srednia_wazona_cena_wejscia) - 4004.0).abs() < 1e-3);
    }

    /// Gdy RDZEŃ poda swój `BasketView`, jego liczby WYGRYWAJĄ.
    ///
    /// To jest strukturalne zabezpieczenie przed rozjazdem: cecha „wynik
    /// koszyka" musi znaczyć co do centa to samo, co liczba, na której bot
    /// podejmuje decyzję RISK FREE. Test podaje widok z liczbami CELOWO
    /// innymi niż policzone lokalnie i sprawdza, że przeszły do wektora.
    #[test]
    fn widok_z_rdzenia_ma_pierwszenstwo_nad_wlasnym_rachunkiem() {
        let o = Obserwator::default();
        let ts = 60_000i64 * 3_000;
        let qq = Quote {
            ts,
            bid: 4012.0,
            ask: 4012.2,
        };
        let b = koszyk(1, Side::Buy, 4000.0, 4010.0, ts - 3_600_000);
        let koszyki = vec![b];
        let pozycje = vec![pozycja(1, 1, Side::Buy, 0.01, 4004.0, ts - 600_000)];

        let bez = o.obserwuj_koszyk(&KontekstKoszyka {
            ogolny: ogolny(ts, &qq, &KONTO_TEST, &koszyki, &pozycje),
            koszyk: &koszyki[0],
            widok: None,
        });
        assert!((bez.wartosc(C::wynik_koszyka_lacznie_usd) - 8.0).abs() < 1e-3);

        let w = BasketView {
            id: 1,
            side: Side::Buy,
            pl_usd: 123.45,
            pl_r_current: 2.5,
            pl_r_initial: 1.25,
            avg_entry: 4001.5,
            risk_usd: 49.38,
            risk_initial_usd: 98.76,
            peak_pl_usd: 200.0,
            drawdown_from_peak: 76.55,
            filled_layers: 3,
            pending_layers: 2,
            planned_risk_in_market: 0.6,
            age_min: 42.0,
            secured: true,
            tp_stage: 2,
        };
        let z = o.obserwuj_koszyk(&KontekstKoszyka {
            ogolny: ogolny(ts, &qq, &KONTO_TEST, &koszyki, &pozycje),
            koszyk: &koszyki[0],
            widok: Some(&w),
        });
        assert!((z.wartosc(C::wynik_koszyka_lacznie_usd) - 123.45).abs() < 1e-3);
        assert!((z.wartosc(C::wynik_koszyka_w_r) - 2.5).abs() < 1e-4);
        assert!((z.wartosc(C::wynik_koszyka_w_r_poczatkowym) - 1.25).abs() < 1e-4);
        assert!((z.wartosc(C::srednia_wazona_cena_wejscia) - 4001.5).abs() < 1e-3);
        assert!((z.wartosc(C::ryzyko_koszyka_usd) - 49.38).abs() < 1e-3);
        assert!((z.wartosc(C::ryzyko_poczatkowe_koszyka_usd) - 98.76).abs() < 1e-3);
        assert!((z.wartosc(C::szczyt_wyniku_koszyka_usd) - 200.0).abs() < 1e-3);
        assert!((z.wartosc(C::spadek_od_szczytu_koszyka_usd) - 76.55).abs() < 1e-3);
        assert_eq!(z.wartosc(C::warstwy_wypelnione), 3.0);
        // widok podaje 2 zlecenia oczekujące, ale my liczymy z `b.pendings`,
        // bo wolna funkcja RDZENIA zwraca tu twarde zero — patrz komentarz
        // w `nadpisz_z_widoku`
        assert_eq!(
            z.wartosc(C::warstwy_czekajace),
            bez.wartosc(C::warstwy_czekajace),
            "warstwy_czekajace nie mogą iść z widoku"
        );
        assert!((z.wartosc(C::udzial_ryzyka_w_rynku) - 0.6).abs() < 1e-4);
        assert!((z.wartosc(C::wiek_koszyka_min) - 42.0).abs() < 1e-4);
        assert_eq!(z.wartosc(C::zabezpieczony), 1.0);
        assert_eq!(z.wartosc(C::etap_tp), 2.0);
        // cechy spoza widoku zostają policzone lokalnie
        assert!(z.wiadomo(C::do_sl_usd));
    }

    #[test]
    fn szczyt_wyniku_koszyka_liczy_sie_na_calosci() {
        let mut o = Obserwator::default();
        let start = 600_000i64;
        nakarm(&mut o, start, &[4000.0; 60]);
        let ts0 = (start + 60) * 60_000;
        let b = koszyk(1, Side::Buy, 3995.0, 4000.0, ts0);
        let koszyki = vec![b];
        let pozycje = vec![pozycja(1, 1, Side::Buy, 0.10, 4000.0, ts0)];

        // cena rośnie do 4010 (szczyt +100 $), potem spada do 4004 (+40 $)
        for (i, p) in [4000.0, 4005.0, 4010.0, 4007.0, 4004.0].iter().enumerate() {
            let t = (start + 60 + i as i64) * 60_000;
            o.na_ticku(
                &Quote {
                    ts: t,
                    bid: *p,
                    ask: p + 0.2,
                },
                &koszyki,
                &pozycje,
            );
        }
        let ts = (start + 64) * 60_000;
        let qq = Quote {
            ts,
            bid: 4004.0,
            ask: 4004.2,
        };
        let k = konto(200.0);
        let ob = o.obserwuj_koszyk(&KontekstKoszyka {
            ogolny: ogolny(ts, &qq, &k, &koszyki, &pozycje),
            koszyk: &koszyki[0],
            widok: None,
        });
        assert!((ob.wartosc(C::szczyt_wyniku_koszyka_usd) - 100.0).abs() < 1e-2);
        assert!((ob.wartosc(C::wynik_koszyka_lacznie_usd) - 40.0).abs() < 1e-2);
        assert!((ob.wartosc(C::spadek_od_szczytu_koszyka_usd) - 60.0).abs() < 1e-2);
        assert!((ob.wartosc(C::udzial_szczytu_koszyka) - 0.4).abs() < 1e-3);
        assert!((ob.wartosc(C::min_od_szczytu_koszyka) - 2.0).abs() < 1e-3);
    }

    #[test]
    fn komunikaty_kanalu_maja_wiek_i_flage() {
        let mut o = Obserwator::default();
        let start = 700_000i64;
        nakarm(&mut o, start, &[4000.0; 5]);
        let ts0 = (start + 5) * 60_000;
        let b = koszyk(1, Side::Buy, 3995.0, 4000.0, ts0);
        let koszyki = vec![b];
        o.na_ticku(
            &Quote {
                ts: ts0,
                bid: 3999.9,
                ask: 4000.1,
            },
            &koszyki,
            &[],
        );
        o.na_komunikacie(1, ts0 + 60_000, Komunikat::TpHit);
        o.na_komunikacie(1, ts0 + 120_000, Komunikat::RiskFree);

        let ts = ts0 + 300_000;
        let qq = Quote {
            ts,
            bid: 3999.9,
            ask: 4000.1,
        };
        let k = konto(200.0);
        let ob = o.obserwuj_koszyk(&KontekstKoszyka {
            ogolny: ogolny(ts, &qq, &k, &koszyki, &[]),
            koszyk: &koszyki[0],
            widok: None,
        });
        assert!((ob.wartosc(C::min_od_kom_tp_hit) - 4.0).abs() < 1e-4);
        assert!((ob.wartosc(C::min_od_kom_risk_free) - 3.0).abs() < 1e-4);
        assert_eq!(ob.wartosc(C::byl_kom_tp_hit), 1.0);
        assert_eq!(ob.wartosc(C::byl_kom_spp), 0.0);
        // SPP nie było: wiek nieznany, ale flaga „nie było" jest znana
        assert!(!ob.wiadomo(C::min_od_kom_spp));
        assert!(ob.wiadomo(C::byl_kom_spp));
        assert!((ob.wartosc(C::min_od_dowolnego_komunikatu) - 3.0).abs() < 1e-4);
    }

    // ---------- kontrola losowa ----------

    #[test]
    fn cechy_losowe_sa_deterministyczne_i_rozne() {
        let o = Obserwator::default();
        let k = konto(200.0);
        let ts = 60_000i64 * 12_345;
        let qq = Quote {
            ts,
            bid: 3999.9,
            ask: 4000.1,
        };
        let mk = |klucz: i64| KontekstSygnalu {
            ogolny: ogolny(ts, &qq, &k, &[], &[]),
            side: Side::Buy,
            strefa_lo: 3995.0,
            strefa_hi: 4005.0,
            sl: Some(3990.0),
            tps: &[4010.0],
            tp_open: false,
            jest_limit: true,
            jest_stop: false,
            tag_high_risk: false,
            tag_may_not: false,
            tag_first_entry: false,
            klucz,
        };
        let a = o.obserwuj_sygnal(&mk(1));
        let b = o.obserwuj_sygnal(&mk(1));
        let c = o.obserwuj_sygnal(&mk(2));
        assert_eq!(
            a.v, b.v,
            "ten sam punkt decyzyjny = ten sam wektor co do bitu"
        );
        let i0 = pierwsza_losowa();
        assert_ne!(a.v[i0], c.v[i0], "inny sygnał = inna kontrola losowa");
        // wszystkie dwadzieścia muszą być różne między sobą
        let mut zbior = std::collections::HashSet::new();
        for i in i0..N_CECH {
            assert!(a.v[i] >= 0.0 && a.v[i] < 1.0);
            assert!(
                zbior.insert(a.v[i].to_bits()),
                "powtórzona cecha losowa {i}"
            );
        }
    }

    /// Ziarno kontroli musi mieć pełny efekt lawinowy.
    ///
    /// Sąsiednie punkty decyzyjne różnią się znacznikiem o kilka bitów.
    /// Gdyby mikser był słaby, sąsiednie chwile dostawałyby podobne liczby
    /// „losowe" i kontrola odziedziczyłaby strukturę czasu — dokładnie ten
    /// błąd, który pierwsza wersja tego modułu popełniła (p = 0,010 wobec
    /// rozkładu zerowego).
    #[test]
    fn ziarno_kontroli_ma_efekt_lawinowy() {
        for wej in [0u64, 1, 42, 1_785_000_000_000, u64::MAX / 3] {
            let l = lawina(splitmix64, wej);
            assert!(
                (l - 32.0).abs() < 3.0,
                "slaby mikser: {l} zmienionych bitow na 64 dla wejscia {wej}"
            );
        }
    }

    /// Sąsiednie chwile decyzyjne muszą dawać NIESKORELOWANE liczby losowe.
    #[test]
    fn sasiednie_chwile_daja_nieskorelowana_kontrole() {
        let o = Obserwator::default();
        let k = konto(200.0);
        // Syntetyczny ciąg kolejnych minut z kluczem rosnącym razem z czasem.
        // Taki układ wykrywa korelację generatora bez danych użytkownika.
        let mut a: Vec<f64> = Vec::new();
        let mut b: Vec<f64> = Vec::new();
        for j in 0..400i64 {
            let ts = 60_000 * (500_000 + j);
            let qq = Quote {
                ts,
                bid: 3999.9,
                ask: 4000.1,
            };
            let mut kt = kontekst_prosty(ts, &qq);
            kt.klucz = 1000 + j;
            let ob = o.obserwuj_sygnal(&kt);
            a.push(ob.wartosc(C::losowa_01) as f64);
            b.push(ob.wartosc(C::losowa_02) as f64);
        }
        let kor = |x: &[f64], y: &[f64]| {
            let n = x.len() as f64;
            let mx = x.iter().sum::<f64>() / n;
            let my = y.iter().sum::<f64>() / n;
            let mut c = 0.0;
            let mut vx = 0.0;
            let mut vy = 0.0;
            for i in 0..x.len() {
                c += (x[i] - mx) * (y[i] - my);
                vx += (x[i] - mx).powi(2);
                vy += (y[i] - my).powi(2);
            }
            c / (vx * vy).sqrt()
        };
        // dwie rozne kolumny losowe w tych samych chwilach
        assert!(
            kor(&a, &b).abs() < 0.15,
            "kolumny losowe skorelowane: {}",
            kor(&a, &b)
        );
        // ta sama kolumna wobec UPLYWU CZASU — to jest ten blad
        let czas: Vec<f64> = (0..400).map(|j| j as f64).collect();
        assert!(
            kor(&a, &czas).abs() < 0.15,
            "kontrola skorelowana z czasem: {}",
            kor(&a, &czas)
        );
        // i wobec samej siebie przesunietej o jeden krok
        let a1 = &a[1..];
        let a0 = &a[..a.len() - 1];
        assert!(
            kor(a0, a1).abs() < 0.15,
            "autokorelacja kontroli: {}",
            kor(a0, a1)
        );
    }

    #[test]
    fn losowe_maja_rozklad_jednostajny() {
        let o = Obserwator::default();
        let k = konto(200.0);
        let mut suma = 0.0;
        let mut n = 0;
        for j in 0..500i64 {
            let ts = 60_000 * (100_000 + j);
            let qq = Quote {
                ts,
                bid: 3999.9,
                ask: 4000.1,
            };
            let ob = o.obserwuj_sygnal(&KontekstSygnalu {
                ogolny: ogolny(ts, &qq, &k, &[], &[]),
                side: Side::Buy,
                strefa_lo: 3995.0,
                strefa_hi: 4005.0,
                sl: Some(3990.0),
                tps: &[4010.0],
                tp_open: false,
                jest_limit: true,
                jest_stop: false,
                tag_high_risk: false,
                tag_may_not: false,
                tag_first_entry: false,
                klucz: j,
            });
            for i in pierwsza_losowa()..N_CECH {
                suma += ob.v[i] as f64;
                n += 1;
            }
        }
        let sr = suma / n as f64;
        assert!(
            (sr - 0.5).abs() < 0.02,
            "średnia cech losowych {sr}, oczekiwane ~0,5"
        );
    }

    // ---------- brak zaglądania w przyszłość ----------

    /// Niezamknięta minuta NIE WCHODZI do agregatów okiennych.
    ///
    /// Gdyby wchodziła, `zakres_60m_usd` znałby maksimum minuty, która jeszcze
    /// trwa — czyli częściowo przyszłość względem kolejnych ticków tej samej
    /// minuty. Ten test pilnuje granicy okna.
    #[test]
    fn biezaca_niezamknieta_minuta_nie_wchodzi_do_agregatow() {
        let mut o = Obserwator::default();
        nakarm(&mut o, 900_000, &vec![4000.0; 80]);
        let przed = {
            let ts = (900_000 + 79) * 60_000;
            let qq = Quote {
                ts,
                bid: 3999.9,
                ask: 4000.1,
            };
            o.obserwuj_sygnal(&kontekst_prosty(ts, &qq))
        };
        assert!(
            (przed.wartosc(C::zakres_60m_usd)).abs() < 1e-6,
            "80 minut płasko"
        );

        // gigantyczny skok W TRWAJĄCEJ minucie — nie wolno mu wejść do okna
        let ts = (900_000 + 79) * 60_000 + 30_000;
        o.na_ticku(
            &Quote {
                ts,
                bid: 4499.9,
                ask: 4500.1,
            },
            &[],
            &[],
        );
        let qq = Quote {
            ts,
            bid: 4499.9,
            ask: 4500.1,
        };
        let po = o.obserwuj_sygnal(&kontekst_prosty(ts, &qq));
        assert!(
            (po.wartosc(C::zakres_60m_usd)).abs() < 1e-6,
            "zakres 60 min policzony z ZAMKNIĘTYCH minut nie może znać trwającej"
        );
        // ale bieżąca cena jest teraźniejszością i wchodzić MUSI
        assert!((po.wartosc(C::cena_mid_usd) - 4500.0).abs() < 1e-3);
    }

    /// Pytanie o chwilę sprzed ostatniego ticka jest błędem wywołania —
    /// bufor rynku zawiera już dane, których wtedy nie było.
    #[test]
    fn zapytanie_o_przeszlosc_jest_rozpoznawane() {
        let mut o = Obserwator::default();
        nakarm(&mut o, 950_000, &vec![4000.0; 10]);
        let teraz = (950_000 + 9) * 60_000;
        assert!(o.czy_aktualny(teraz));
        assert!(o.czy_aktualny(teraz + 60_000));
        assert!(
            !o.czy_aktualny(teraz - 60_000),
            "to byłoby zaglądanie w przyszłość"
        );
    }

    #[test]
    fn dwa_przebiegi_daja_identyczny_wektor() {
        let ceny: Vec<f64> = (0..200)
            .map(|i| 4000.0 + (i as f64 * 0.37).sin() * 5.0)
            .collect();
        let policz = || {
            let mut o = Obserwator::default();
            nakarm(&mut o, 800_000, &ceny);
            let ts = (800_000 + 199) * 60_000;
            let qq = Quote {
                ts,
                bid: 3999.9,
                ask: 4000.1,
            };
            o.obserwuj_sygnal(&kontekst_prosty(ts, &qq))
        };
        let a = policz();
        let b = policz();
        for i in 0..N_CECH {
            assert_eq!(
                a.maska[i], b.maska[i],
                "maska {} niestabilna",
                OPISY[i].nazwa
            );
            assert_eq!(
                a.v[i].to_bits(),
                b.v[i].to_bits(),
                "cecha {} niestabilna",
                OPISY[i].nazwa
            );
        }
    }

    // ---------- pokrycie: KAŻDA cecha musi się kiedyś policzyć ----------

    #[test]
    fn kazda_cecha_daje_sie_policzyc_w_ktoryms_scenariuszu() {
        let mut pokryte = vec![false; N_CECH];
        let k = konto(200.0);

        // scenariusz 1: bogaty koszyk po ośmiu dniach historii rynku
        {
            let mut o = Obserwator::default();
            // 8 dni po jednej świecy na minutę to 11 520 punktów — dość na
            // średnią tygodniową i ekstrema pięciodniowe
            let start = 1_000_000i64;
            let ceny: Vec<f64> = (0..11_600)
                .map(|i| 4000.0 + ((i as f64) / 97.0).sin() * 20.0)
                .collect();
            nakarm_gesto(&mut o, start, &ceny);

            let ts0 = (start + 11_600) * 60_000;
            let mut b = koszyk(1, Side::Buy, 3995.0, 4005.0, ts0);
            // duży zrealizowany zysk, żeby ożyła gałąź „risk free wykonalny"
            b.realized = 60.0;
            b.risk_initial_usd = 45.0;
            b.tp_stage = 1;
            b.secured = true;
            b.reentries = 1;
            b.rearms = 1;
            b.state = BasketState::RiskFree;
            b.levels = vec![GridLevel {
                price: 4000.0,
                base_units: 1,
                volume: 0.01,
                sl: None,
                tp: None,
                level: 0,
                is_toucher: false,
                filled: true,
                fill_ts: 0,
                fill_px: 0.0,
                cancelled: false,
            }];
            b.pendings = vec![7];
            let mut b2 = koszyk(2, Side::Sell, 4100.0, 4110.0, ts0);
            b2.had_positions = true;
            let koszyki = vec![b, b2];
            // SZEŚĆ warstw — tylko wtedy ożywają sloty `warstwa_2..6`
            let pozycje: Vec<Position> = [4004.0, 4002.0, 4000.0, 3998.0, 3996.0, 3994.0]
                .iter()
                .enumerate()
                .map(|(i, cena)| {
                    let mut p = pozycja(i as u64 + 1, 1, Side::Buy, 0.01, *cena, ts0);
                    p.sl = Some(3993.0);
                    p
                })
                .collect();

            o.na_sygnale(ts0 - 3_600_000, Side::Buy);
            o.na_sygnale(ts0 - 600_000, Side::Sell);
            o.na_zamknieciu(ts0 - 300_000, 5.0);
            o.na_zamknieciu(ts0 - 200_000, -2.0);

            // przejście przez strefę i z powrotem, żeby ożyły cechy dynamiki
            for (i, p) in [4020.0, 4006.0, 4000.0, 3990.0, 4001.0, 4002.0]
                .iter()
                .enumerate()
            {
                let t = ts0 + (i as i64) * 60_000;
                o.na_ticku(
                    &Quote {
                        ts: t,
                        bid: p - 0.1,
                        ask: p + 0.1,
                    },
                    &koszyki,
                    &pozycje,
                );
            }
            for kom in [
                Komunikat::TpHit,
                Komunikat::RiskFree,
                Komunikat::Spp,
                Komunikat::Be,
                Komunikat::OutAtEntry,
                Komunikat::Cancel,
                Komunikat::CloseAll,
            ] {
                o.na_komunikacie(1, ts0 + 60_000, kom);
            }

            let ts = ts0 + 10 * 60_000;
            let qq = Quote {
                ts,
                bid: 4001.9,
                ask: 4002.1,
            };
            let mut g = ogolny(ts, &qq, &k, &koszyki, &pozycje);
            g.zrealizowane_dzis = 4.0;
            g.szczyt_equity = 260.0;
            let ob = o.obserwuj_koszyk(&KontekstKoszyka {
                ogolny: g,
                koszyk: &koszyki[0],
                widok: None,
            });
            for i in 0..N_CECH {
                pokryte[i] |= ob.maska[i];
            }
        }

        // scenariusz 2: sam sygnał, żeby dobić cechy wyłącznie wejściowe
        {
            let mut o = Obserwator::default();
            nakarm(&mut o, 2_000_000, &vec![4000.0; 300]);
            let ts = 2_000_300 * 60_000;
            let qq = Quote {
                ts,
                bid: 3989.9,
                ask: 3990.1,
            };
            let ob = o.obserwuj_sygnal(&KontekstSygnalu {
                ogolny: ogolny(ts, &qq, &k, &[], &[]),
                side: Side::Sell,
                strefa_lo: 4000.0,
                strefa_hi: 4010.0,
                sl: Some(4015.0),
                tps: &[3990.0, 3985.0, 3980.0],
                tp_open: true,
                jest_limit: false,
                jest_stop: true,
                tag_high_risk: true,
                tag_may_not: true,
                tag_first_entry: true,
                klucz: 11,
            });
            for i in 0..N_CECH {
                pokryte[i] |= ob.maska[i];
            }
        }

        let martwe: Vec<&str> = (0..N_CECH)
            .filter(|i| !pokryte[*i])
            .map(|i| OPISY[i].nazwa)
            .collect();
        assert!(
            martwe.is_empty(),
            "cechy, których żaden scenariusz nie policzył (martwa gałąź?): {martwe:?}"
        );
    }

    #[test]
    fn wynik_koszyka_lacznie_ignoruje_cudze_pozycje() {
        let ts = 60_000i64 * 100;
        let qq = Quote {
            ts,
            bid: 4010.0,
            ask: 4010.2,
        };
        let mut b = koszyk(1, Side::Buy, 4000.0, 4005.0, ts);
        b.realized = 2.0;
        let pozycje = vec![
            pozycja(1, 1, Side::Buy, 0.01, 4000.0, ts),
            pozycja(2, 9, Side::Buy, 0.50, 4000.0, ts), // CUDZY koszyk
        ];
        let w = wynik_koszyka_lacznie(&b, &pozycje, &qq);
        assert!(
            (w - 12.0).abs() < 1e-6,
            "0.01*10*100 + 2.0 = 12, dostałem {w}"
        );
        let sr = srednia_wazona_wejscia(&b, &pozycje).unwrap();
        assert!((sr - 4000.0).abs() < 1e-6);
    }
}
