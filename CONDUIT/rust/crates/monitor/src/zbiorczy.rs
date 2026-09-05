
use crate::{pl_duza, Postep};

/// Sufit szacunku, jak w [`crate::Raport`]: 30 dni. Wyżej to i tak znaczy
/// „nie wiadomo", a nie „za 4 lata".
const SUFIT_ETA_S: f64 = 30.0 * 86_400.0;

/// Zadanie musi się liczyć CHOĆ CHWILĘ, zanim uwierzymy w jego tempo.
const MINIMALNY_CZAS_MS: f64 = 300.0;

/// Poniżej tego postępu iloraz „ile zostało / jak szybko" jest zbyt czuły,
/// żeby dawać liczbę, którą ktoś mógłby zaplanować sobie dzień.
const MINIMALNY_POSTEP: f64 = 0.01;

/// Zadanie wchodzące do zestawienia RAZEM z liczbami po wygładzeniu w oknie.
///
/// Prędkość i ETA bierzemy wygładzone, a nie surowe z pliku: zestawienie to
/// suma kilku zadań, więc skoki każdego z nich by się DODAŁY i liczba na górze
/// ekranu drgałaby mocniej niż na kartach, które przecież już są wygładzone.
#[derive(Clone, Copy, Debug)]
pub struct Skladnik<'a> {
    pub zadanie: &'a Postep,
    /// prędkość wygładzona; `<= 0` = jeszcze nie wiadomo
    pub szybkosc: f64,
    /// czas do końca wygładzony; `< 0` = jeszcze nie wiadomo
    pub eta_s: f64,
}

impl<'a> Skladnik<'a> {
    /// Składnik z liczbami prosto z pliku — do testów i do wywołań, w których
    /// nie ma osobnego wygładzania.
    pub fn surowy(zadanie: &'a Postep) -> Self {
        Skladnik {
            zadanie,
            szybkosc: zadanie.szybkosc,
            eta_s: zadanie.eta_s,
        }
    }
}

/// Gotowe zestawienie. Wszystkie pola są już policzone — okno tylko rysuje.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Zbiorczy {
    /// ile ŻYWYCH zadań weszło do zestawienia
    pub zadan: usize,
    /// ile z nich nie podaje skali, więc nie ma udziału w procencie
    pub bez_skali: usize,
    /// `None` = nie ma z czego policzyć procentu (żadne zadanie nie podało
    /// skali). Okno wtedy NIE rysuje paska, zamiast rysować zero.
    pub postep: Option<f64>,
    /// suma zrobionych jednostek — tylko przy jednorodnej jednostce
    pub zrobione: f64,
    /// suma wszystkich jednostek — tylko przy jednorodnej jednostce
    pub calosc: f64,
    /// nazwa jednostki, gdy wszystkie zadania mierzą w tym samym; inaczej pusta
    pub jednostka: String,
    /// `true` = procent to średnia z ułamków, bo jednostek nie wolno dodać
    pub srednia_z_ulamkow: bool,
    /// łączna prędkość, osobno w KAŻDEJ jednostce, w kolejności pojawienia się
    pub szybkosci: Vec<(String, f64)>,
    /// czas do końca CAŁOŚCI w sekundach; `< 0` = jeszcze nie wiadomo
    pub eta_s: f64,
    /// `true` = część zadań nie umie się oszacować, więc to DOLNA granica
    pub eta_dolna_granica: bool,
}

impl Zbiorczy {
    /// „12,4 mln ticków/s · 30 ocen/s" — każda jednostka osobno, bo dodanie
    /// ticków do ocen nie znaczy nic.
    pub fn opis_szybkosci(&self) -> String {
        self.szybkosci
            .iter()
            .map(|(j, v)| format!("{} {}", pl_duza(*v), j))
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// Składa zestawienie. `None` = nie ma ANI JEDNEGO żywego zadania, czyli okno
/// nie ma czego pokazać i paska w ogóle nie rysuje.
///
/// To jest cała reguła widoczności paska zbiorczego: pusty pasek na stałe to
/// szum, a nie informacja.
pub fn zestaw(skladniki: &[Skladnik<'_>], teraz: i64) -> Option<Zbiorczy> {
    let zywe: Vec<&Skladnik<'_>> = skladniki.iter().filter(|s| s.zadanie.zywy(teraz)).collect();
    if zywe.is_empty() {
        return None;
    }

    let mierzalne: Vec<&&Skladnik<'_>> = zywe.iter().filter(|s| s.zadanie.calosc > 0.0).collect();

    // --- jednostka wspólna albo jej brak ---
    let mut jednostki: Vec<&str> = Vec::new();
    for s in &mierzalne {
        let j = s.zadanie.jednostka.trim();
        if !jednostki.contains(&j) {
            jednostki.push(j);
        }
    }
    let jednorodne = jednostki.len() == 1;

    let mut z = Zbiorczy {
        zadan: zywe.len(),
        bez_skali: zywe.len() - mierzalne.len(),
        eta_s: -1.0,
        ..Default::default()
    };

    if mierzalne.is_empty() {
        // Wszystkie żywe zadania są bez skali. Procentu NIE MA i nie wolno go
        // wymyślić — okno pokaże sam nagłówek z liczbą zadań.
        z.postep = None;
    } else if jednorodne {
        // Jednostka ta sama, więc wolno DODAĆ: to jest prawdziwy procent całej
        // roboty, ważony wielkością zadań. Mały sweep obok wielkiego nie
        // podbija wtedy wyniku do połowy.
        z.zrobione = mierzalne.iter().map(|s| s.zadanie.zrobione).sum();
        z.calosc = mierzalne.iter().map(|s| s.zadanie.calosc).sum();
        z.jednostka = jednostki[0].to_string();
        z.postep = Some((z.zrobione / z.calosc).clamp(0.0, 1.0));
    } else {
        // Ticków nie wolno dodać do ocen. Zostaje średnia z ułamków — mówimy
        // to na ekranie, żeby nikt nie czytał tego jako „przemielono 40 % danych".
        let suma: f64 = mierzalne
            .iter()
            .map(|s| (s.zadanie.zrobione / s.zadanie.calosc).clamp(0.0, 1.0))
            .sum();
        z.srednia_z_ulamkow = true;
        z.postep = Some(suma / mierzalne.len() as f64);
    }

    // --- łączna prędkość, osobno w każdej jednostce ---
    for s in &zywe {
        if s.szybkosc <= 0.0 {
            continue;
        }
        let j = s.zadanie.jednostka_szybkosci.trim();
        if j.is_empty() {
            // Prędkość bez jednostki jest nie do podpisania, a liczba bez
            // podpisu w wierszu zbiorczym byłaby zgadywanką czytelnika.
            continue;
        }
        match z.szybkosci.iter_mut().find(|(k, _)| k == j) {
            Some((_, v)) => *v += s.szybkosc,
            None => z.szybkosci.push((j.to_string(), s.szybkosc)),
        }
    }

    // --- czas do końca CAŁOŚCI ---
    let (eta, dolna) = policz_ete(&zywe, &z, teraz);
    z.eta_s = eta;
    z.eta_dolna_granica = dolna;
    Some(z)
}

/// Czas do końca całości.
///
/// Ścieżka główna: `ile zostało / łączna prędkość`. Wolno tak liczyć, bo
/// zadania to osobne procesy walczące o TE SAME rdzenie — gdy jedno skończy,
/// pozostałe dostają jego rdzenie i łączna przepustowość maszyny zostaje mniej
/// więcej ta sama. Maksimum z pojedynczych szacunków byłoby tu przeszacowaniem:
/// zakładałoby, że najwolniejsze zadanie do końca dostaje tylko tyle rdzeni, co
/// teraz.
///
/// Ścieżka zapasowa (mieszane jednostki albo brak łącznej prędkości): maksimum
/// z tego, co zadania zgłaszają same. Wtedy, jeśli któreś nie umie się
/// oszacować, wynik jest DOLNĄ GRANICĄ i okno pisze „co najmniej".
///
/// Zwraca `(sekundy, czy_dolna_granica)`; sekundy `< 0` znaczą „nie wiadomo".
fn policz_ete(zywe: &[&Skladnik<'_>], z: &Zbiorczy, teraz: i64) -> (f64, bool) {
    let najdluzej_pracuje = zywe
        .iter()
        .map(|s| s.zadanie.trwa_s(teraz))
        .fold(0.0_f64, f64::max)
        * 1000.0;
    let postep = z.postep.unwrap_or(0.0);

    // Ścieżka główna. Brakująca prędkość pojedynczego zadania NIE psuje tu
    // wyniku w niebezpieczną stronę: jego `calosc` i tak jest po stronie „ile
    // zostało", więc szacunek wychodzi ostrożny, a nie za krótki. Dlatego
    // znacznik dolnej granicy tej ścieżki nie dotyczy.
    if z.szybkosci.len() == 1
        && !z.srednia_z_ulamkow
        && z.calosc > 0.0
        && najdluzej_pracuje > MINIMALNY_CZAS_MS
        && postep > MINIMALNY_POSTEP
        && postep < 1.0
    {
        let v = z.szybkosci[0].1;
        if v > 1e-9 {
            return (
                ((z.calosc - z.zrobione).max(0.0) / v).min(SUFIT_ETA_S),
                false,
            );
        }
    }

    let znane: Vec<f64> = zywe.iter().map(|s| s.eta_s).filter(|e| *e >= 0.0).collect();
    if znane.is_empty() {
        return (-1.0, false);
    }
    let max = znane
        .iter()
        .fold(0.0_f64, |a, b| a.max(*b))
        .min(SUFIT_ETA_S);
    (max, znane.len() < zywe.len())
}

// ============================================================
//  TESTY
// ============================================================

#[cfg(test)]
mod testy {
    use super::*;
    use crate::{teraz_ms, BACKTEST, SWIEZOSC_MS};

    fn zadanie(jedn: &str, zrobione: f64, calosc: f64, wiek_ms: i64) -> Postep {
        let t = teraz_ms();
        Postep {
            id: format!("z-{zrobione}-{calosc}"),
            rodzaj: BACKTEST.into(),
            zrobione,
            calosc,
            jednostka: jedn.into(),
            jednostka_szybkosci: format!("{jedn}/s"),
            start_ts: t - 60_000,
            aktualizacja_ts: t - wiek_ms,
            ..Default::default()
        }
    }

    /// Nie ma żywego zadania → nie ma paska. To jest cała reguła widoczności
    /// i musi być egzekwowana tutaj, a nie w rysowaniu.
    #[test]
    fn martwe_zadania_nie_daja_paska() {
        let a = zadanie("ticków", 30.0, 100.0, SWIEZOSC_MS + 1);
        let b = zadanie("ticków", 10.0, 100.0, SWIEZOSC_MS + 5_000);
        let we = [Skladnik::surowy(&a), Skladnik::surowy(&b)];
        assert!(zestaw(&we, teraz_ms()).is_none(), "same trupy = brak paska");
    }

    /// Procent całości ma być WAŻONY wielkością zadań. Średnia z ułamków
    /// pokazałaby 5 % tam, gdzie przemielono 1 % roboty.
    #[test]
    fn procent_wazy_sie_wielkoscia_zadania() {
        let maly = zadanie("ticków", 10.0, 100.0, 100);
        let duzy = zadanie("ticków", 0.0, 900.0, 100);
        let we = [Skladnik::surowy(&maly), Skladnik::surowy(&duzy)];
        let z = zestaw(&we, teraz_ms()).unwrap();
        assert_eq!(z.zadan, 2);
        assert!((z.postep.unwrap() - 0.01).abs() < 1e-12, "1 %, nie 5 %");
        assert!(!z.srednia_z_ulamkow);
        assert_eq!(z.jednostka, "ticków");
        assert_eq!(z.calosc, 1000.0);
    }

    /// Ticków nie wolno dodać do ocen. Wtedy zostaje średnia i MUSI być
    /// oznaczona, żeby nikt nie czytał jej jako sumy przemielonych danych.
    #[test]
    fn rozne_jednostki_to_srednia_a_nie_suma() {
        let bt = zadanie("ticków", 50.0, 100.0, 100);
        let tr = zadanie("ocen", 0.0, 1000.0, 100);
        let we = [Skladnik::surowy(&bt), Skladnik::surowy(&tr)];
        let z = zestaw(&we, teraz_ms()).unwrap();
        assert!(z.srednia_z_ulamkow);
        assert!((z.postep.unwrap() - 0.25).abs() < 1e-12);
        assert_eq!(z.jednostka, "", "brak wspólnej jednostki = brak nazwy");
        assert_eq!(z.calosc, 0.0, "sumy nie ma, bo nie wolno jej policzyć");
    }

    /// Zadanie bez skali nie ma procentu. Ma być POLICZONE i pokazane osobno,
    /// a nie po cichu wliczone jako zero.
    #[test]
    fn zadanie_bez_skali_nie_wchodzi_do_procentu() {
        let ze_skala = zadanie("ticków", 40.0, 100.0, 100);
        let bez = zadanie("ticków", 0.0, 0.0, 100);
        let we = [Skladnik::surowy(&ze_skala), Skladnik::surowy(&bez)];
        let z = zestaw(&we, teraz_ms()).unwrap();
        assert_eq!(z.zadan, 2);
        assert_eq!(z.bez_skali, 1);
        assert!((z.postep.unwrap() - 0.4).abs() < 1e-12);
    }

    /// Same zadania bez skali → procentu NIE MA. `None`, nie zero.
    #[test]
    fn same_zadania_bez_skali_nie_maja_procentu() {
        let a = zadanie("", 0.0, 0.0, 100);
        let we = [Skladnik::surowy(&a)];
        let z = zestaw(&we, teraz_ms()).unwrap();
        assert_eq!(z.postep, None);
        assert_eq!(z.bez_skali, 1);
    }

    /// Prędkość sumuje się TYLKO w obrębie jednej jednostki.
    #[test]
    fn predkosc_sumuje_sie_w_obrebie_jednostki() {
        let a = zadanie("ticków", 10.0, 100.0, 100);
        let b = zadanie("ticków", 10.0, 100.0, 100);
        let c = zadanie("ocen", 10.0, 100.0, 100);
        let we = [
            Skladnik {
                zadanie: &a,
                szybkosc: 1_000.0,
                eta_s: 10.0,
            },
            Skladnik {
                zadanie: &b,
                szybkosc: 3_000.0,
                eta_s: 20.0,
            },
            Skladnik {
                zadanie: &c,
                szybkosc: 7.0,
                eta_s: 5.0,
            },
        ];
        let z = zestaw(&we, teraz_ms()).unwrap();
        assert_eq!(z.szybkosci.len(), 2);
        assert_eq!(z.szybkosci[0], ("ticków/s".to_string(), 4_000.0));
        assert_eq!(z.szybkosci[1], ("ocen/s".to_string(), 7.0));
        assert_eq!(z.opis_szybkosci(), "4\u{202f}000 ticków/s · 7 ocen/s");
    }

    /// Pierwszy meldunek nie ma prawa powiedzieć „zostało 0 s". Bez prędkości
    /// i bez postępu odpowiedź brzmi „nie wiem" (ujemna ETA).
    #[test]
    fn eta_nie_klamie_na_starcie() {
        let mut a = zadanie("ticków", 0.0, 1_000_000.0, 50);
        a.start_ts = teraz_ms() - 40; // ledwo ruszyło
        a.eta_s = -1.0;
        let we = [Skladnik {
            zadanie: &a,
            szybkosc: 0.0,
            eta_s: -1.0,
        }];
        let z = zestaw(&we, teraz_ms()).unwrap();
        assert!(z.eta_s < 0.0, "brak danych = brak szacunku");
        assert!(!z.eta_dolna_granica);
    }

    /// Ścieżka główna: zostało 900 jednostek, maszyna robi 300/s → 3 s.
    #[test]
    fn eta_calosci_z_lacznej_predkosci() {
        let a = zadanie("ticków", 50.0, 500.0, 100);
        let b = zadanie("ticków", 50.0, 500.0, 100);
        let we = [
            Skladnik {
                zadanie: &a,
                szybkosc: 100.0,
                eta_s: 4.5,
            },
            Skladnik {
                zadanie: &b,
                szybkosc: 200.0,
                eta_s: 2.25,
            },
        ];
        let z = zestaw(&we, teraz_ms()).unwrap();
        assert!(
            (z.eta_s - 3.0).abs() < 1e-9,
            "900 / 300 = 3 s, a nie max(4,5)"
        );
        assert!(!z.eta_dolna_granica);
    }

    /// Ścieżka zapasowa: mieszane jednostki → maksimum z pojedynczych
    /// szacunków, a brak choćby jednego czyni z wyniku DOLNĄ granicę.
    #[test]
    fn mieszane_jednostki_daja_dolna_granice() {
        let a = zadanie("ticków", 50.0, 100.0, 100);
        let b = zadanie("ocen", 50.0, 100.0, 100);
        let we = [
            Skladnik {
                zadanie: &a,
                szybkosc: 10.0,
                eta_s: 12.0,
            },
            Skladnik {
                zadanie: &b,
                szybkosc: 5.0,
                eta_s: -1.0,
            },
        ];
        let z = zestaw(&we, teraz_ms()).unwrap();
        assert!((z.eta_s - 12.0).abs() < 1e-9);
        assert!(
            z.eta_dolna_granica,
            "jedno zadanie bez szacunku = co najmniej tyle"
        );
    }
}
