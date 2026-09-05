//! `postep.exe` — okienko postępu backtestów i treningu AI.
//!
//! Program jest CELOWO osobny od `conduit.exe`: ma stać obok terminala,
//! ważyć kilka megabajtów, nie potrzebować WebView2 i nie mieć nic wspólnego z
//! cyklem życia dużej aplikacji. Uruchamia się sam (patrz
//! [`conduit_monitor::uruchom_okno_jesli_trzeba`]) i zamyka się sam, gdy przez
//! [`BEZCZYNNOSC_MS`] nie ma żadnego żywego zadania.
//!
//! Okno niczego nie liczy. Czyta katalog plików stanu i rysuje to, co znajdzie;
//! jedyne, co robi „od siebie", to wygładzanie prędkości i przewidywanego czasu
//! (żeby liczby dało się przeczytać) oraz utworzenie pliku `<id>.stop` po
//! naciśnięciu PRZERWIJ.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use conduit_monitor as m;
use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Ile probek predkosci trzymamy na zadanie. Przy odswiezaniu co 120 ms to
/// jest okolo trzech minut historii — tyle, ile trzeba, zeby zobaczyc trend,
/// i na tyle malo, zeby nie urosnac w pamieci przy przebiegu na dobe.
const HIST_MAX: usize = 240;
/// O ile pikseli przewija jedno nacisniecie strzalki.
const KROK_PRZEWIJANIA: f32 = 48.0;

// ============================================================
//  PALETA — ta sama, co w panelu webowym CONDUIT
// ============================================================

const TLO: egui::Color32 = egui::Color32::from_rgb(0x0b, 0x0e, 0x14);
const KARTA: egui::Color32 = egui::Color32::from_rgb(0x11, 0x15, 0x20);
const INSET: egui::Color32 = egui::Color32::from_rgb(0x0d, 0x10, 0x17);
const KRESKA: egui::Color32 = egui::Color32::from_rgb(0x22, 0x28, 0x39);
const KRESKA_MOCNA: egui::Color32 = egui::Color32::from_rgb(0x2f, 0x37, 0x4b);
const TEKST: egui::Color32 = egui::Color32::from_rgb(0xe6, 0xeb, 0xf5);
const TEKST_SLABY: egui::Color32 = egui::Color32::from_rgb(0x85, 0x90, 0xa8);
const ZIELEN: egui::Color32 = egui::Color32::from_rgb(0x26, 0xd9, 0xa3);
const FIOLET: egui::Color32 = egui::Color32::from_rgb(0xc8, 0x8d, 0xff);
const BURSZTYN: egui::Color32 = egui::Color32::from_rgb(0xf5, 0xb7, 0x4e);
const CZERWIEN: egui::Color32 = egui::Color32::from_rgb(0xff, 0x5c, 0x7a);

/// Kolor wiodący karty zależy od rodzaju zadania — z odległości widać, czy to
/// backtest, czy trening, bez czytania nagłówka.
fn kolor_rodzaju(rodzaj: &str) -> egui::Color32 {
    match rodzaj {
        m::TRENING => FIOLET,
        _ => ZIELEN,
    }
}

// ============================================================
//  STAN OKNA
// ============================================================

/// Wygładzanie po stronie okna. Zadanie już raz uśredniło prędkość, ale ETA
/// liczona z ilorazu potrafi skakać przy zmianie tempa; drugi, lekki filtr
/// kosztuje ułamek sekundy opóźnienia i daje liczbę, którą da się przeczytać.
const ALFA: f64 = 0.35;

#[derive(Default)]
struct Wygladzanie {
    szybkosc: f64,
    eta: f64,
}

impl Wygladzanie {
    fn wciagnij(&mut self, szybkosc: f64, eta: f64) {
        self.szybkosc = if self.szybkosc <= 0.0 {
            szybkosc
        } else {
            ALFA * szybkosc + (1.0 - ALFA) * self.szybkosc
        };
        if eta >= 0.0 {
            self.eta = if self.eta <= 0.0 {
                eta
            } else {
                ALFA * eta + (1.0 - ALFA) * self.eta
            };
        } else {
            self.eta = -1.0;
        }
    }
}

/// Ślad po zadaniu, którego pliku już nie ma.
struct Zakonczone {
    nazwa: String,
    rodzaj: String,
    przerwane: bool,
    kiedy: Instant,
    podsumowanie: String,
}

/// Proces uruchomiony z poczekalni i pilnowany przez okno.
///
/// Uchwyt trzymamy w pamięci, bo tylko przez niego poznamy KOD WYJŚCIA.
/// Sam plik postępu nie wystarcza: zadanie, które padło z błędem argumentów,
/// nigdy nie zdąży żadnego pliku napisać.
struct Uruchomione {
    id: String,
    proces: std::process::Child,
    log: PathBuf,
}

/// Co użytkownik kliknął w poczekalni.
///
/// Kliknięcia zbieramy do listy i wykonujemy PO narysowaniu panelu. Rysowanie
/// pożycza `self` na niezmienne, a każda z tych akcji zmienia kolejkę — bez
/// odroczenia nie da się tego napisać bez klonowania całej listy co klatkę.
enum Akcja {
    WGore(String),
    WDol(String),
    Usun(String),
    UruchomTeraz(String),
    Dodaj,
    SprzatajSkonczone,
}

/// Stan przeglądarki presetów dla JEDNEGO zadania.
///
/// Osobny na zadanie, bo przemiatań potrafi biec kilka naraz i każde ma swój
/// katalog wyników, swoje kryterium i swoją pozycję w kolejności.
struct Przeglad {
    dane: Option<m::przesiane::Przesiane>,
    /// indeks w [`m::przesiane::KRYTERIA`]; 0 = zysk końcowy (domyślne)
    kryterium: usize,
    /// pozycja w `kolejnosc`, nie indeks wyniku
    wybrany: usize,
    /// indeksy do `dane.wyniki`, od najlepszego
    kolejnosc: Vec<usize>,
    /// Panel startuje ROZWINIĘTY. Człowiek prosił o przeglądarkę w karcie
    /// przemiatania, a schowana za kliknięciem nie jest w karcie — jest o
    /// jedno kliknięcie od karty. Zwinięcie zostaje na wypadek, gdy biegnie
    /// kilka przemiatań naraz i okno robi się za wysokie.
    rozwiniete: bool,
    /// kiedy ostatnio zaglądaliśmy na dysk
    ostatnie_zajrzenie: Option<Instant>,
    /// Człowiek właśnie w tym panelu coś kliknął — po tej fladze okno
    /// rozstrzyga, którą listę mają przesuwać strzałki z klawiatury.
    wlasnie_dotkniety: bool,
    /// Prostokąt WIDOCZNEJ części tabelki statystyk z ostatniej klatki.
    ///
    /// Po nim okno rozstrzyga, czy kliknięcie albo kursor wypadły w tabelce.
    /// `None` = tabelka nie jest w tej chwili rysowana (panel zwinięty).
    rect_tabelki: Option<egui::Rect>,
    /// Ile pikseli przewinąć SAMĄ tabelkę w tej klatce (strzałki góra-dół,
    /// gdy przewijanie należy do niej, a nie do okna).
    przewin_tabelki: f32,
}

impl Default for Przeglad {
    fn default() -> Self {
        Przeglad {
            dane: None,
            kryterium: 0,
            wybrany: 0,
            kolejnosc: Vec::new(),
            rozwiniete: true,
            ostatnie_zajrzenie: None,
            wlasnie_dotkniety: false,
            rect_tabelki: None,
            przewin_tabelki: 0.0,
        }
    }
}

/// Jak często wolno zaglądać do pliku wyników.
///
/// Okno rysuje się kilkanaście razy na sekundę, a plik po trzech tysiącach
/// przebiegów waży kilka megabajtów. Czytanie go co klatkę zajęłoby dysk i
/// procesor bez żadnego pożytku: nowy przebieg dochodzi raz na kilkanaście
/// sekund, nie raz na klatkę.
const ODSTEP_ODCZYTU: Duration = Duration::from_secs(2);

impl Przeglad {
    /// Zagląda na dysk, ale nie częściej niż co [`ODSTEP_ODCZYTU`] i przelicza
    /// kolejność tylko wtedy, gdy plik faktycznie się zmienił.
    fn odswiez(&mut self, katalog: &std::path::Path) {
        let teraz = Instant::now();
        if let Some(t) = self.ostatnie_zajrzenie {
            if teraz.duration_since(t) < ODSTEP_ODCZYTU {
                return;
            }
        }
        self.ostatnie_zajrzenie = Some(teraz);

        let stary_stempel = self.dane.as_ref().and_then(|d| d.stempel);
        let Some(nowe) = m::przesiane::wczytaj(katalog) else {
            // Plik przyłapany w połowie zapisu albo jeszcze nie istnieje.
            // ZOSTAWIAMY to, co mamy: mrugnięcie panelu co dwie sekundy przy
            // każdym zapisie przemiatania byłoby gorsze niż chwila zwłoki.
            return;
        };
        if nowe.stempel.is_some() && nowe.stempel == stary_stempel && !self.kolejnosc.is_empty() {
            return; // nic nowego nie doszło
        }

        // Nazwa wybranego presetu ma PRZEŻYĆ dopisanie nowych wyników.
        // Trzymanie samej pozycji przesuwałoby wybór pod palcami za każdym
        // razem, gdy przemiatanie znajdzie coś lepszego.
        let trzymana = self
            .kolejnosc
            .get(self.wybrany)
            .and_then(|&i| self.dane.as_ref().map(|d| d.wyniki[i].nazwa.clone()));

        self.przelicz(&nowe);
        if let Some(nazwa) = trzymana {
            if let Some(poz) = self
                .kolejnosc
                .iter()
                .position(|&i| nowe.wyniki[i].nazwa == nazwa)
            {
                self.wybrany = poz;
            }
        }
        self.dane = Some(nowe);
    }

    /// Układa kolejność wg bieżącego kryterium.
    ///
    /// Dane bierze Z ZEWNĄTRZ, a nie z `self.dane`, bo panel na czas rysowania
    /// WYPOŻYCZA je ze stanu (patrz `Apka::przeglad_presetow`). Sięgnięcie tu
    /// po `self.dane` wyzerowałoby kolejność akurat w chwili, gdy człowiek
    /// klika „najlepszy preset".
    fn przelicz(&mut self, d: &m::przesiane::Przesiane) {
        self.kolejnosc =
            m::przesiane::kolejnosc(&d.wyniki, &m::przesiane::KRYTERIA[self.kryterium]);
        self.wybrany = self.wybrany.min(self.kolejnosc.len().saturating_sub(1));
    }
}

struct Apka {
    dir: PathBuf,
    zadania: Vec<m::Postep>,
    wygl: HashMap<String, Wygladzanie>,
    /// id → ostatni znany stan; służy do rozpoznania „zadanie właśnie zniknęło"
    widziane: HashMap<String, m::Postep>,
    /// id, dla których nacisnęliśmy PRZERWIJ
    poproszone: HashMap<String, ()>,
    /// id, dla którego pokazujemy pytanie „na pewno?"
    pytanie: Option<String>,
    zakonczone: Vec<Zakonczone>,
    ostatnie_zywe: Instant,
    start: Instant,
    na_wierzchu: bool,
    /// Okno podniosło się SAMO (uruchomiła je binarka zadania) — wtedy wolno mu
    /// zniknąć po robocie. Okno otwarte ręcznie zostaje, dopóki człowiek go nie
    /// zamknie.
    automatyczne: bool,
    /// czy w ogóle widzieliśmy kiedyś żywe zadanie
    widzialem_zadanie: bool,
    // ---------- poczekalnia ----------
    kolejka: m::kolejka::Kolejka,
    /// proces, który okno samo uruchomiło; najwyżej jeden naraz
    dziecko: Option<Uruchomione>,
    pocz_rozwinieta: bool,
    f_nazwa: String,
    f_polecenie: String,
    f_katalog: String,
    /// komunikat pod formularzem (błąd dodania albo wynik szukania `btp.exe`)
    f_komunikat: String,
    /// Ile pikseli przewinac w tej klatce — wynik klawiszy strzalek.
    ///
    /// Zapisujemy DELTE, a nie pozycje. Wymuszanie pozycji co klatke
    /// (`vertical_scroll_offset`) zabraloby kolko myszy, bo egui nadpisywaloby
    /// jego wynik przy kazdym rysowaniu. Delta dziala obok kolka, nie zamiast.
    przewin: f32,
    /// Historia predkosci na zadanie: (znacznik czasu, jednostki/s).
    ///
    /// Sama liczba „1,6 mln ticków/s" nie mowi, czy przebieg zwalnia, czy
    /// przyspiesza — a to jest pierwsza rzecz, ktora chce sie wiedziec przy
    /// przebiegu na dziesiec godzin. Trzymamy okno ostatnich `HIST_MAX`
    /// probek i rysujemy je jako iskierke pod paskiem.
    historia: HashMap<String, Vec<(i64, f64)>>,
    /// Przegladarka juz policzonych presetow, osobna na zadanie.
    przeglad: HashMap<String, Przeglad>,
    /// Karta, po której chodzą strzałki lewo/prawo z klawiatury.
    ///
    /// Przemiatań potrafi biec kilka naraz, więc klawisz musi wiedzieć, którą
    /// listę przesuwa. Ustawia się na tę, w której człowiek ostatnio czegoś
    /// dotknął; przy braku wyboru bierzemy pierwszą rozwiniętą.
    aktywny_przeglad: Option<String>,
    /// O ile pozycji przesunąć wybór w tej klatce (wynik strzałek lewo/prawo).
    skok_presetu: i64,
    /// Do czego trafiają strzałki GÓRA-DÓŁ: `None` = całe okno, `Some(id)` =
    /// tabelka statystyk tego zadania.
    ///
    /// Ustawia się KLIKNIĘCIEM, jak ognisko w przeglądarce: klik w tabelkę
    /// oddaje jej przewijanie, klik gdziekolwiek indziej — oknu.
    focus_przewijania: Option<String>,
    _zamek: m::Zamek,
}

impl Apka {
    fn nowa(cc: &eframe::CreationContext<'_>, zamek: m::Zamek, automatyczne: bool) -> Apka {
        styl(&cc.egui_ctx);
        let dir = m::katalog();

        // Poczekalnia przeżywa zamknięcie okna, więc wczytujemy ją z dysku.
        // Pozycje zostawione w stanie „liczy" to sieroty po poprzednim oknie —
        // patrz `oznacz_sieroty`: NIE wznawiamy ich samoczynnie.
        let mut kolejka = m::kolejka::Kolejka::wczytaj(&dir);
        if kolejka.oznacz_sieroty() > 0 {
            let _ = kolejka.zapisz(&dir);
        }

        // Formularz podpowiada pełną ścieżkę do `btp.exe`, bo to jest program,
        // który wpisuje się tu w dziewięciu przypadkach na dziesięć. Gdy go nie
        // ma — mówimy, GDZIE szukaliśmy, zamiast zostawić puste pole.
        let (polecenie, komunikat) = match m::kolejka::znajdz_btp() {
            Ok(p) => (format!("\"{}\" ", p.display()), String::new()),
            Err(sprawdzone) => (
                String::new(),
                format!(
                    "nie znalazłem btp.exe — wpisz pełną ścieżkę. Sprawdziłem: {}",
                    sprawdzone
                        .iter()
                        .take(6)
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(" · ")
                ),
            ),
        };
        let katalog = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        Apka {
            dir,
            zadania: Vec::new(),
            wygl: HashMap::new(),
            widziane: HashMap::new(),
            poproszone: HashMap::new(),
            pytanie: None,
            zakonczone: Vec::new(),
            ostatnie_zywe: Instant::now(),
            start: Instant::now(),
            // okno podniesione samo nie ma prawa zasłaniać cudzej pracy
            na_wierzchu: !automatyczne,
            automatyczne,
            widzialem_zadanie: false,
            // Kolejka z pozycjami sama się rozwija: skoro coś w niej stoi, to
            // pierwsze pytanie brzmi „co i w jakiej kolejności".
            pocz_rozwinieta: !kolejka.wpisy.is_empty(),
            kolejka,
            dziecko: None,
            f_nazwa: String::new(),
            f_polecenie: polecenie,
            f_katalog: katalog,
            f_komunikat: komunikat,
            przewin: 0.0,
            historia: HashMap::new(),
            przeglad: HashMap::new(),
            aktywny_przeglad: None,
            skok_presetu: 0,
            focus_przewijania: None,
            _zamek: zamek,
        }
    }

    /// Odczyt katalogu i rozpoznanie, co zniknęło od poprzedniej klatki.
    fn odswiez(&mut self) {
        let teraz = m::teraz_ms();
        self.zadania = m::wczytaj_wszystkie(&self.dir);

        let obecne: HashMap<String, m::Postep> = self
            .zadania
            .iter()
            .map(|z| (z.id.clone(), z.clone()))
            .collect();

        // zadania, które właśnie się skończyły (plik zniknął)
        let znikniete: Vec<String> = self
            .widziane
            .keys()
            .filter(|id| !obecne.contains_key(*id))
            .cloned()
            .collect();
        for id in znikniete {
            if let Some(ost) = self.widziane.remove(&id) {
                // Zadanie żywe do ostatniej chwili + skasowany plik = normalne
                // zakończenie albo przerwanie z zapisem. Zadanie, które
                // wcześniej zamilkło, po prostu padło i nie ma czego ogłaszać.
                if ost.zywy(teraz) {
                    let przerwane = ost.przerywanie || self.poproszone.contains_key(&id);
                    self.zakonczone.push(Zakonczone {
                        nazwa: if ost.nazwa.is_empty() {
                            id.clone()
                        } else {
                            ost.nazwa.clone()
                        },
                        rodzaj: ost.rodzaj.clone(),
                        przerwane,
                        kiedy: Instant::now(),
                        podsumowanie: format!(
                            "{} · {} w {}",
                            if przerwane {
                                format!("zatrzymano na {:.0} %", ost.postep * 100.0)
                            } else {
                                "ukończono".to_string()
                            },
                            if ost.calosc > 0.0 {
                                format!("{} {}", m::pl_duza(ost.zrobione), ost.jednostka)
                            } else {
                                String::new()
                            },
                            m::pl_czas(ost.trwa_s(teraz))
                        ),
                    });
                }
            }
            self.poproszone.remove(&id);
            self.wygl.remove(&id);
            if self.pytanie.as_deref() == Some(id.as_str()) {
                self.pytanie = None;
            }
        }
        self.widziane = obecne;

        // wygładzanie liczb żywych zadań
        for z in &self.zadania {
            if z.zywy(teraz) {
                self.wygl
                    .entry(z.id.clone())
                    .or_default()
                    .wciagnij(z.szybkosc, z.eta_s);
                self.ostatnie_zywe = Instant::now();
                self.widzialem_zadanie = true;
            }
        }
        // komunikaty o zakończeniu trzymamy 25 s — dość, żeby wrócić do okna
        self.zakonczone.retain(|z| z.kiedy.elapsed().as_secs() < 25);
    }

    // ============================================================
    //  POCZEKALNIA — silnik
    // ============================================================

    /// Czy maszyna jest zajęta liczeniem CZEGOKOLWIEK.
    ///
    /// Kolejka czeka nie tylko na własne pozycje, ale i na sweep puszczony
    /// z terminala albo z panelu. Dwa sweepy naraz odbierają sobie rdzenie —
    /// poczekalnia istnieje właśnie po to, żeby tego nie robić. Przycisk
    /// „URUCHOM TERAZ" pozwala tę zasadę świadomie złamać.
    fn maszyna_zajeta(&self, teraz: i64) -> bool {
        self.zadania.iter().any(|z| z.zywy(teraz))
    }

    /// Pilnowanie kolejki: sprzątnięcie procesu, który się skończył, i start
    /// następnego, gdy jest wolno.
    fn dogladaj_kolejke(&mut self, teraz: i64) {
        let mut zmiana = false;

        // --- czy uruchomiony proces już się skończył ---
        if let Some(u) = self.dziecko.as_mut() {
            let wynik = u.proces.try_wait();
            match wynik {
                Ok(Some(status)) => {
                    let kod = status.code();
                    // Ogon logu czytamy RAZ, w chwili zakończenia. Czytanie go
                    // co klatkę byłoby odczytem z dysku 8 razy na sekundę.
                    let ogon = m::kolejka::ogon_logu(&u.log, 3);
                    let log = u.log.display().to_string();
                    let id = u.id.clone();
                    self.dziecko = None;
                    if let Some(w) = self.kolejka.znajdz_mut(&id) {
                        w.koniec_ts = teraz;
                        w.kod = kod;
                        w.stan = if kod == Some(0) {
                            m::kolejka::Stan::Gotowe
                        } else {
                            m::kolejka::Stan::Padlo
                        };
                        w.uwaga = if kod == Some(0) {
                            String::new()
                        } else if ogon.is_empty() {
                            format!("bez wyjścia na ekran · log: {log}")
                        } else {
                            ogon
                        };
                    }
                    zmiana = true;
                }
                Ok(None) => {}
                Err(e) => {
                    // Utrata uchwytu do procesu znaczy, że przestaliśmy wiedzieć,
                    // co się z nim dzieje. Mówimy to wprost zamiast trzymać wpis
                    // w „liczy się" na zawsze.
                    let id = u.id.clone();
                    let opis = e.to_string();
                    self.dziecko = None;
                    if let Some(w) = self.kolejka.znajdz_mut(&id) {
                        w.stan = m::kolejka::Stan::Nieznane;
                        w.koniec_ts = teraz;
                        w.uwaga = format!("straciłem kontakt z procesem: {opis}");
                    }
                    zmiana = true;
                }
            }
        }

        // --- czy wolno ruszyć następną ---
        if self.dziecko.is_none() && !self.maszyna_zajeta(teraz) {
            if let Some(i) = self.kolejka.nastepny_do_startu() {
                self.startuj(i, teraz);
                zmiana = true;
            }
        }

        if zmiana {
            let _ = self.kolejka.zapisz(&self.dir);
        }
    }

    /// Startuje pozycję o podanym numerze. Nieudany start to `Padło` z treścią
    /// błędu systemu — nigdy ciche przejście dalej.
    fn startuj(&mut self, i: usize, teraz: i64) {
        let wpis = self.kolejka.wpisy[i].clone();
        match m::kolejka::uruchom(&self.dir, &wpis) {
            Ok((proces, log)) => {
                let pid = proces.id();
                let w = &mut self.kolejka.wpisy[i];
                w.stan = m::kolejka::Stan::Liczy;
                w.start_ts = teraz;
                w.koniec_ts = 0;
                w.kod = None;
                w.pid = pid;
                w.uwaga = String::new();
                self.dziecko = Some(Uruchomione {
                    id: wpis.id,
                    proces,
                    log,
                });
            }
            Err(e) => {
                let w = &mut self.kolejka.wpisy[i];
                w.stan = m::kolejka::Stan::Padlo;
                w.start_ts = teraz;
                w.koniec_ts = teraz;
                w.uwaga = format!("nie dało się uruchomić: {e}");
            }
        }
    }

    /// Karta postępu należąca do pozycji kolejki, jeśli taka jest.
    ///
    /// Wiążemy po numerze procesu: `Raport` nadaje zadaniu identyfikator
    /// `<rodzaj>-<pid>-<n>`, więc pozycja poczekalni potrafi pokazać PRAWDZIWY
    /// postęp swojego procesu, a nie samo „liczy się".
    fn karta_pozycji(&self, pid: u32) -> Option<&m::Postep> {
        if pid == 0 {
            return None;
        }
        let ogon = format!("-{pid}-");
        self.zadania.iter().find(|z| z.id.contains(&ogon))
    }

    /// Czy wypada się zamknąć: żadnego żywego zadania od [`m::BEZCZYNNOSC_MS`].
    fn czas_sie_zamknac(&self) -> bool {
        // Okno otwarte ręcznie nie znika NIGDY samo. Użytkownik, który klika
        // `postep.exe` przed uruchomieniem backtestu, ma prawo zobaczyć, że
        // program czeka — a nie patrzeć, jak okno gaśnie mu przed nosem.
        if !self.automatyczne {
            return false;
        }
        // Okno z niepustą poczekalnią nie znika NIGDY samo — to ono jest
        // jedynym procesem, który tę kolejkę popycha. Zniknięcie oznaczałoby,
        // że ustawione zadania nigdy nie ruszą, a użytkownik dowiedziałby się
        // o tym po godzinie patrzenia w pusty pulpit.
        if self.dziecko.is_some() || self.kolejka.wpisy.iter().any(|w| !w.stan.skonczony()) {
            return false;
        }
        let bez = self.ostatnie_zywe.elapsed().as_millis() as i64;
        let od_startu = self.start.elapsed().as_millis() as i64;
        // Nieświeże pliki (zadania, które padły) TRZYMAJĄ okno otwarte — to
        // jest informacja, którą użytkownik ma zobaczyć, a nie powód do
        // zniknięcia. Tak samo świeży komunikat „przerwano, zapisano".
        bez > m::BEZCZYNNOSC_MS
            && od_startu > m::BEZCZYNNOSC_MS
            && self.zadania.is_empty()
            && self.zakonczone.is_empty()
    }
}

// ============================================================
//  STYL
// ============================================================

fn styl(ctx: &egui::Context) {
    // Motyw wymuszamy na ciemny niezależnie od ustawień systemu — okno ma
    // pasować do reszty CONDUIT-a, a nie do jasnego pulpitu.
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut v = egui::Visuals::dark();
    v.panel_fill = TLO;
    v.window_fill = KARTA;
    v.extreme_bg_color = INSET;
    v.override_text_color = Some(TEKST);
    v.widgets.noninteractive.bg_fill = KARTA;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, KRESKA);
    v.widgets.inactive.bg_fill = egui::Color32::from_rgb(0x18, 0x1e, 0x2c);
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, KRESKA);
    v.widgets.hovered.bg_fill = egui::Color32::from_rgb(0x1f, 0x26, 0x36);
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, KRESKA_MOCNA);
    v.widgets.active.bg_fill = egui::Color32::from_rgb(0x26, 0x2e, 0x40);
    v.window_stroke = egui::Stroke::new(1.0, KRESKA);
    ctx.set_visuals(v);

    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 7.0);
        s.spacing.button_padding = egui::vec2(12.0, 7.0);
        // Domyślny pasek przewijania PŁYWA nad treścią. Wartości w tabelkach
        // są dosunięte do prawej krawędzi, więc suwak lądował dokładnie na
        // cyfrach i zjadał ostatnie znaki. Wariant „solid" dostaje własną
        // kolumnę i nie przykrywa niczego.
        s.spacing.scroll = egui::style::ScrollStyle::solid();
    });
}

// ============================================================
//  ELEMENTY INTERFEJSU
// ============================================================

fn etykieta(ui: &mut egui::Ui, t: &str, kolor: egui::Color32, rozmiar: f32) {
    ui.label(egui::RichText::new(t).color(kolor).size(rozmiar));
}

fn mono(ui: &mut egui::Ui, t: &str, kolor: egui::Color32, rozmiar: f32) {
    ui.label(
        egui::RichText::new(t)
            .color(kolor)
            .size(rozmiar)
            .monospace(),
    );
}

/// Ile z plakietki zbiorczej mieści się w pasku tytułu.
///
/// Kolejność wariantów jest kolejnością WAŻNOŚCI i celowo NIE jest kolejnością
/// estetyczną: najpierw procent, potem czas do końca, dopiero potem nitka,
/// a prędkość na końcu. Pytanie, z którym się patrzy na to okno kątem oka,
/// brzmi „ile jeszcze" — nitka ładnie to ilustruje, ale odpowiada na to samo
/// pytanie mniej dokładnie niż napis „1m57", a kosztuje tyle samo miejsca.
/// Gdy oba rodzaje zadań liczą się naraz w oknie o domyślnej szerokości, na
/// plakietkę przypada około stu punktów — akurat na procent i czas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Stopien {
    /// sam procent
    Procent,
    /// procent + czas do końca
    Czas,
    /// jw. + nitka postępu
    Pasek,
    /// jw. + łączna prędkość
    Pelny,
}

/// Wszystkie stopnie od najbogatszego — do przymierzania.
const STOPNIE: [Stopien; 4] = [
    Stopien::Pelny,
    Stopien::Pasek,
    Stopien::Czas,
    Stopien::Procent,
];

/// Kawałek plakietki. Osobny typ, bo zawartość trzeba najpierw ZMIERZYĆ,
/// a dopiero potem narysować.
enum Czesc {
    Tekst(String, egui::Color32, f32),
    Nitka(f64),
}

/// Szerokość nitki postępu w plakietce.
const NITKA_SZER: f32 = 42.0;
/// Odstęp między kawałkami plakietki.
const NITKA_ODSTEP: f32 = 5.0;
/// Marginesy ramki plakietki (7 z każdej strony) plus kreska.
const PLAKIETKA_RAMKA: f32 = 16.0;

/// Zawartość plakietki w KOLEJNOŚCI CZYTANIA dla zadanego stopnia.
///
/// Osobno od rysowania, żeby dało się to i zmierzyć, i sprawdzić testem.
fn czesci_plakietki(skrot: &str, z: &m::zbiorczy::Zbiorczy, st: Stopien) -> Vec<Czesc> {
    // „~" = procent jest ŚREDNIĄ z zadań o różnych jednostkach, a nie udziałem
    // przemielonych danych. Jeden znak zamiast zdania, bo zdanie się tu nie
    // mieści — pełne wyjaśnienie jest w podpowiedzi pod kursorem.
    let procent = match z.postep {
        Some(p) => format!(
            "{}{} %",
            if z.srednia_z_ulamkow { "~" } else { "" },
            m::pl_liczba(p * 100.0, 0)
        ),
        None => "bez skali".to_string(),
    };
    // „≥" = część zadań nie umie się oszacować, więc to dolna granica.
    let czas = if z.eta_s >= 0.0 {
        format!(
            "{}{}",
            if z.eta_dolna_granica { "≥" } else { "" },
            czas_zwiezly(z.eta_s)
        )
    } else {
        "—".to_string()
    };

    let mut v = vec![Czesc::Tekst(skrot.to_string(), TEKST, 10.0)];
    if st >= Stopien::Pasek {
        if let Some(p) = z.postep {
            v.push(Czesc::Nitka(p));
        }
    }
    v.push(Czesc::Tekst(procent, TEKST, 11.0));
    // Kropka między liczbami nie jest ozdobą: bez niej wychodziło
    // „4 %5 min 09 s" i oko musiało się zatrzymać, żeby to rozdzielić.
    if st >= Stopien::Czas {
        v.push(Czesc::Tekst(
            "·".into(),
            TEKST_SLABY.gamma_multiply(0.7),
            10.0,
        ));
        v.push(Czesc::Tekst(czas, BURSZTYN, 10.5));
    }
    if st >= Stopien::Pelny && !z.szybkosci.is_empty() {
        v.push(Czesc::Tekst(
            "·".into(),
            TEKST_SLABY.gamma_multiply(0.7),
            10.0,
        ));
        v.push(Czesc::Tekst(z.opis_szybkosci(), TEKST_SLABY, 10.5));
    }
    v
}

/// Ile miejsca zajmie taka plakietka.
fn szerokosc_czesci(ui: &egui::Ui, czesci: &[Czesc]) -> f32 {
    let mut w = PLAKIETKA_RAMKA;
    for (i, c) in czesci.iter().enumerate() {
        if i > 0 {
            w += NITKA_ODSTEP;
        }
        w += match c {
            Czesc::Tekst(t, _, r) => {
                ui.painter()
                    .layout_no_wrap(t.clone(), egui::FontId::monospace(*r), TEKST)
                    .size()
                    .x
            }
            Czesc::Nitka(_) => NITKA_SZER,
        };
    }
    w
}

/// Najbogatszy stopień, który zmieści się w podanym budżecie.
///
/// MIERZYMY tekst, a nie zgadujemy progi. Pierwsza wersja miała progi wpisane
/// na sztywno i wysypała się natychmiast: „12 ocen/s" i „520,0 tys. ticków/s"
/// to ta sama pozycja plakietki o szerokości różniącej się dwukrotnie, więc
/// jeden próg dla obu musiał być albo za ciasny, albo za luźny. Za luźny
/// oznaczał plakietkę wjeżdżającą na napis „CONDUIT · postęp" — widziane na
/// ekranie.
///
/// `None` = nie mieści się nawet sam procent; wtedy plakietki nie ma wcale.
/// Karty pod spodem i tak pokazują wszystko.
fn dobierz_stopien(
    ui: &egui::Ui,
    skrot: &str,
    z: &m::zbiorczy::Zbiorczy,
    budzet: f32,
) -> Option<Stopien> {
    STOPNIE
        .iter()
        .copied()
        .find(|st| szerokosc_czesci(ui, &czesci_plakietki(skrot, z, *st)) <= budzet)
}

/// Czas do końca w postaci zwięzłej: „47s", „1m57", „2h14".
///
/// W plakietce nie ma miejsca na „1 min 57 s", a skracanie zależne od miejsca
/// byłoby najgorsze z możliwych: ta sama liczba zmieniałaby zapis przy każdej
/// zmianie szerokości okna. Karty zadań zostają przy pełnym [`m::pl_czas`] —
/// tam miejsce jest i tam czyta się to spokojnie, a nie kątem oka.
fn czas_zwiezly(sekundy: f64) -> String {
    if !sekundy.is_finite() || sekundy < 0.0 {
        return "—".into();
    }
    let s = sekundy.round() as i64;
    if s >= 3600 {
        format!("{}h{:02}", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}m{:02}", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

/// Szerokość napisu „CONDUIT · postęp" — miejsce, którego plakietkom zabierać
/// NIE WOLNO, bo tytuł miałby się skurczyć i tekst zacząłby skakać.
fn szerokosc_tytulu(ui: &egui::Ui) -> f32 {
    let f = egui::FontId::proportional(13.0);
    let a = ui
        .painter()
        .layout_no_wrap("CONDUIT".into(), f.clone(), TEKST)
        .size()
        .x;
    let b = ui
        .painter()
        .layout_no_wrap("· postęp".into(), f, TEKST)
        .size()
        .x;
    a + b + ui.spacing().item_spacing.x * 2.0
}

/// Kolorowa plakietka stanu — ta sama forma, co znacznik BACKTEST/TRENING
/// w nagłówku karty, żeby oko czytało je tak samo.
fn plakietka(ui: &mut egui::Ui, tekst: &str, kolor: egui::Color32) {
    egui::Frame::default()
        .fill(kolor.gamma_multiply(0.16))
        .corner_radius(egui::CornerRadius::same(5))
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| etykieta(ui, tekst, kolor, 10.0));
}

/// Skrót po ZNAKACH, nie po bajtach — polecenia mają w sobie polskie nazwy
/// katalogów, a cięcie w środku znaku wielobajtowego wywala program.
fn skroc(s: &str, n: usize) -> String {
    let ile = s.chars().count();
    if ile <= n {
        return s.to_string();
    }
    let mut w: String = s.chars().take(n.saturating_sub(1)).collect();
    w.push('…');
    w
}

/// „1 zadanie", „2 zadania", „5 zadań" — polski wymaga trzech form, a „2 zadań"
/// kłuje w oczy tak samo jak literówka.
fn odmien_zadania(n: usize) -> String {
    let d = n % 10;
    let s = n % 100;
    let forma = if n == 1 {
        "zadanie"
    } else if (2..=4).contains(&d) && !(12..=14).contains(&s) {
        "zadania"
    } else {
        "zadań"
    };
    format!("{n} {forma}")
}

/// „klucz ————— wartość" w jednej linii, na całą szerokość karty.
///
/// Świadomie [`egui::Sides`], a nie zagnieżdżony układ `right_to_left`: ten
/// drugi w komórce siatki bierze CAŁĄ dostępną szerokość i dokłada ją do
/// szerokości kolumny, przez co karta rozjeżdża się poza okno i gubi nagłówek
/// razem z paskiem. `shrink_left` przycina długi klucz, zamiast rozpychać
/// rodzica.
/// Wartość DŁUŻSZA niż zmieści się obok klucza idzie do własnej linii, ze
/// zawijaniem. `Sides` przycina tylko lewą stronę — prawa bierze tyle, ile
/// chce, i przy długiej wartości rozpycha KARTĘ szerzej niż okno. Wtedy
/// nagłówek i pasek uciekają poza krawędź, a to jest jedyne, po co się na to
/// okno patrzy. Zdarzyło się to od razu po dołożeniu wierszy w rodzaju
/// „najgorszy dzień −87 $ · dno 112 $ · dni+ 61 %".
fn wiersz_klucz_wartosc(ui: &mut egui::Ui, klucz: &str, wartosc: &str) {
    // Dwa sufity, jak przy pasku: bez `max_rect` pojedynczy szeroki element
    // sprawia, że „dostępna szerokość" robi się tysiącami pikseli.
    let dostepna = ui.available_width().min(ui.max_rect().width());
    let font = egui::FontId::monospace(12.0);
    let szerokosc_wartosci = ui
        .painter()
        .layout_no_wrap(wartosc.to_string(), font, TEKST)
        .size()
        .x;
    // 70 px to miejsce na najkrótszy sensowny klucz plus oddech między nimi.
    if szerokosc_wartosci <= dostepna - 70.0 {
        egui::Sides::new().shrink_left().show(
            ui,
            |ui| etykieta(ui, klucz, TEKST_SLABY, 11.5),
            |ui| mono(ui, wartosc, TEKST, 12.0),
        );
    } else {
        etykieta(ui, klucz, TEKST_SLABY, 11.5);
        ui.add(
            egui::Label::new(
                egui::RichText::new(wartosc)
                    .color(TEKST)
                    .size(12.0)
                    .monospace(),
            )
            .wrap(),
        );
    }
}

/// Pasek postępu rysowany ręcznie — z procentem w środku i miękkim wypełnieniem.
fn pasek(ui: &mut egui::Ui, ulamek: f64, kolor: egui::Color32, martwy: bool) {
    rysuj_pasek(ui, ulamek, kolor, martwy, None, 26.0, true);
}

/// Nitka postępu o STAŁEJ szerokości i bez procentu w środku — do plakietek
/// zbiorczych w pasku tytułu, gdzie na tekst w środku nie ma ani milimetra.
///
/// Świadomie ten sam rysunek, co pasek na karcie (promień, tło, ramka, kolor
/// wypełnienia): dwa niezależne rysowania paska rozjechałyby się przy pierwszej
/// zmianie palety.
fn pasek_nitka(ui: &mut egui::Ui, ulamek: f64, kolor: egui::Color32, szer: f32, wys: f32) {
    rysuj_pasek(ui, ulamek, kolor, false, Some(szer), wys, false);
}

fn rysuj_pasek(
    ui: &mut egui::Ui,
    ulamek: f64,
    kolor: egui::Color32,
    martwy: bool,
    stala_szer: Option<f32>,
    wys: f32,
    z_procentem: bool,
) {
    // Szerokość ograniczamy DWOMA sufitami: dostępną i maksymalną. Bez tego
    // pojedynczy element rozpychający kartę zamienia pasek w prostokąt o
    // szerokości kilku tysięcy pikseli, którego środek (a więc i procent)
    // wypada daleko poza ekranem.
    let szer = match stala_szer {
        Some(s) => s,
        None => ui.available_width().min(ui.max_rect().width()).max(60.0),
    };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(szer, wys), egui::Sense::hover());
    let p = ui.painter();
    // Promień skalowany wysokością: pełny na pasku karty, mały na nitce.
    // Stałe 7 px na ośmiopikselowej nitce robiło z krótkiego wypełnienia
    // KROPKĘ, którą oko czyta jako „zero", a nie „osiemnaście procent".
    let r = egui::CornerRadius::same((wys * 0.28).clamp(2.0, 7.0) as u8);
    p.rect_filled(rect, r, INSET);
    p.rect_stroke(
        rect,
        r,
        egui::Stroke::new(1.0, KRESKA),
        egui::StrokeKind::Inside,
    );

    let u = ulamek.clamp(0.0, 1.0) as f32;
    if u > 0.001 {
        let mut wyp = rect;
        wyp.set_width((rect.width() * u).max(4.0));
        let k = if martwy {
            CZERWIEN.gamma_multiply(0.55)
        } else {
            kolor
        };
        p.rect_filled(wyp, r, k.gamma_multiply(0.9));
    }
    if z_procentem {
        let txt = format!("{} %", m::pl_liczba(ulamek * 100.0, 1));
        p.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            txt,
            egui::FontId::monospace((wys * 0.5).clamp(10.0, 13.0)),
            if u > 0.5 { TLO } else { TEKST },
        );
    }
}

/// Cienki pasek POD głównym: postęp JEDNEGO elementu — jednego przebiegu
/// sweepu, jednego pokolenia treningu — a nie całości.
///
/// Ciężar wzrokowy jest celowo mniejszy: cztery piksele zamiast dwudziestu
/// sześciu i kolor przygaszony do 55 %. Ten pasek ma być czytany DRUGI; gdyby
/// wyglądał tak samo jak główny, oko musiałoby za każdym razem sprawdzać, który
/// jest który.
///
/// Procent nie mieści się w czterech pikselach, więc idzie do podpisu obok
/// nazwy elementu. Podpis jest OBOWIĄZKOWY i nie jest ozdobą: przy dwudziestu
/// czterech przebiegach liczonych równolegle sama liczba nie mówi, czego
/// dotyczy, a pasek bez tej informacji sugerowałby precyzję, której nie ma.
/// ISKIERKA PREDKOSCI — maly wykres pod paskiem postepu.
///
/// # Po co
///
/// „1,6 mln ticków/s" to jedna liczba i nie odpowiada na pytanie, ktore zadaje
/// sie przy przebiegu na dziesiec godzin: czy to zwalnia. Roznica miedzy
/// „stale 1,6" a „bylo 3,0, jest 1,6" decyduje o tym, czy warto czekac, czy
/// przerwac — a z samej liczby jej nie widac.
///
/// # Skala
///
/// Pionowo od ZERA do maksimum z okna, nie od minimum. Skala od minimum
/// rozciagalaby szum przy stalej predkosci na cala wysokosc i kazdy przebieg
/// wygladalby jak trzesienie ziemi. Od zera widac proporcje: spadek o polowe
/// jest widoczny jako spadek o polowe.
fn iskierka(ui: &mut egui::Ui, dane: &[(i64, f64)], kolor: egui::Color32, szer: f32, wys: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(szer, wys), egui::Sense::hover());
    if dane.len() < 2 {
        return;
    }
    let maks = dane.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max);
    if maks <= 0.0 {
        return;
    }
    let p = ui.painter();
    p.rect_filled(
        rect,
        egui::CornerRadius::same(3),
        kolor.gamma_multiply(0.06),
    );
    let n = dane.len();
    let punkty: Vec<egui::Pos2> = dane
        .iter()
        .enumerate()
        .map(|(i, (_, v))| {
            let x = rect.left() + rect.width() * (i as f32) / ((n - 1) as f32);
            let y = rect.bottom() - rect.height() * (*v / maks).clamp(0.0, 1.0) as f32;
            egui::pos2(x, y)
        })
        .collect();
    p.add(egui::Shape::line(
        punkty.clone(),
        egui::Stroke::new(1.4, kolor.gamma_multiply(0.85)),
    ));
    // Kropka na ostatniej probce — oko od razu wie, gdzie jest „teraz".
    if let Some(ost) = punkty.last() {
        p.circle_filled(*ost, 2.0, kolor);
    }
}

fn pasek_cienki(ui: &mut egui::Ui, ulamek: f64, kolor: egui::Color32, martwy: bool, podpis: &str) {
    let wys = 4.0;
    let szer = ui.available_width().min(ui.max_rect().width()).max(60.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(szer, wys), egui::Sense::hover());
    let p = ui.painter();
    let r = egui::CornerRadius::same(2);
    p.rect_filled(rect, r, INSET);

    let u = ulamek.clamp(0.0, 1.0) as f32;
    if u > 0.001 {
        let mut wyp = rect;
        wyp.set_width((rect.width() * u).max(3.0));
        let k = if martwy {
            CZERWIEN.gamma_multiply(0.45)
        } else {
            kolor.gamma_multiply(0.55)
        };
        p.rect_filled(wyp, r, k);
    }

    ui.add_space(2.0);
    egui::Sides::new().shrink_left().show(
        ui,
        |ui| etykieta(ui, podpis, TEKST_SLABY.gamma_multiply(0.85), 10.5),
        |ui| {
            mono(
                ui,
                &format!("{} %", m::pl_liczba(ulamek * 100.0, 1)),
                TEKST_SLABY,
                10.5,
            )
        },
    );
}

/// „ok. 15:42" — ten sam czas do końca, tylko na zegarze.
///
/// „zostało 2 h 14 min" wymaga liczenia w głowie i za każdym spojrzeniem daje
/// inną liczbę; godzina stoi w miejscu i od razu odpowiada na pytanie, o które
/// naprawdę chodzi („zdążę przed wyjściem"). Gdy koniec wypada jutro albo
/// później, dokładamy datę — sam „02:10" wyglądałby jak za dziesięć minut.
fn godzina_konca(eta_s: f64) -> String {
    if !eta_s.is_finite() || eta_s < 0.0 {
        return String::new();
    }
    let teraz = chrono::Local::now();
    let koniec =
        teraz + chrono::TimeDelta::seconds(eta_s.round().clamp(0.0, 30.0 * 86_400.0) as i64);
    if koniec.date_naive() == teraz.date_naive() {
        format!("ok. {}", koniec.format("%H:%M"))
    } else {
        format!("ok. {}", koniec.format("%d.%m %H:%M"))
    }
}

/// „14,2 z 54,7 mln ticków" — obie liczby w TEJ SAMEJ skali, żeby dało się je
/// porównać wzrokiem.
fn liczby_skali(zrobione: f64, calosc: f64, jednostka: &str) -> String {
    if calosc <= 0.0 {
        return String::new();
    }
    let (dz, sufiks) = if calosc >= 1e9 {
        (1e9, " mld")
    } else if calosc >= 1e6 {
        (1e6, " mln")
    } else if calosc >= 1e4 {
        (1e3, " tys.")
    } else {
        (1.0, "")
    };
    let miejsca = if dz > 1.0 { 1 } else { 0 };
    format!(
        "{} z {}{} {}",
        m::pl_liczba(zrobione / dz, miejsca),
        m::pl_liczba(calosc / dz, miejsca),
        sufiks,
        jednostka
    )
}

fn liczby_bezwzgledne(z: &m::Postep) -> String {
    liczby_skali(z.zrobione, z.calosc, &z.jednostka)
}

impl Apka {
    /// Zadanie, którego tabelka statystyk obejmuje ten punkt.
    fn tabelka_pod(&self, poz: egui::Pos2) -> Option<String> {
        self.przeglad
            .iter()
            .find(|(_, x)| x.rect_tabelki.map(|r| r.contains(poz)).unwrap_or(false))
            .map(|(id, _)| id.clone())
    }

    // ============================================================
    //  PRZEGLADARKA PRESETOW JUZ POLICZONYCH
    // ============================================================

    /// Panel „przesiane presety" w karcie przemiatania.
    ///
    /// Rysowany TYLKO wtedy, gdy zadanie podało katalog wyników i coś w nim
    /// leży. Zadanie bez wyników (trening) i przemiatanie w pierwszej minucie
    /// nie dostają pustej ramki — pusta ramka na stałe byłaby szumem.
    fn przeglad_presetow(&mut self, ui: &mut egui::Ui, z: &m::Postep) {
        if z.katalog_wynikow.is_empty() {
            return;
        }
        // Strzałki z klawiatury dotyczą JEDNEJ karty — patrz `aktywny_przeglad`.
        let skok_klawiszem = if self.aktywny_przeglad.as_deref() == Some(z.id.as_str()) {
            self.skok_presetu
        } else {
            0
        };
        let stan = self.przeglad.entry(z.id.clone()).or_default();
        stan.odswiez(std::path::Path::new(&z.katalog_wynikow));
        // Prostokąt jest ważny tylko tak długo, jak tabelka jest rysowana.
        // Zwinięty panel z zapamiętanym prostokątem połykałby przewijanie
        // okna w miejscu, gdzie nic już nie ma.
        stan.rect_tabelki = None;
        // WYPOŻYCZAMY dane na czas rysowania. Panel jednocześnie czyta listę
        // wyników i zmienia wybór, a oba mieszkają w `stan`; drugim wyjściem
        // byłoby klonowanie trzech tysięcy wyników co klatkę.
        let Some(dane) = stan.dane.take() else { return };
        if !dane.wyniki.is_empty() {
            Apka::rysuj_przeglad(ui, z, stan, &dane, skok_klawiszem);
        }
        // Oddanie MUSI się wykonać na każdej ścieżce — stąd osobna funkcja
        // rysująca zamiast wcześniejszych wyjść w środku tej.
        stan.dane = Some(dane);
    }

    /// Sam rysunek panelu; `dane` są wypożyczone, `stan` zmienialny.
    fn rysuj_przeglad(
        ui: &mut egui::Ui,
        z: &m::Postep,
        stan: &mut Przeglad,
        dane: &m::przesiane::Przesiane,
        skok_klawiszem: i64,
    ) {
        let ile = dane.wyniki.len();
        // Pozycja mogła wyjść poza listę po zmianie kryterium albo po tym, jak
        // przemiatanie dopisało wyniki — przycinamy przed każdym użyciem.
        stan.wybrany = stan.wybrany.min(ile.saturating_sub(1));
        let kryt = &m::przesiane::KRYTERIA[stan.kryterium];
        let czolo = stan.kolejnosc.first().copied().unwrap_or(0);

        ui.add_space(5.0);
        egui::Frame::default()
            .fill(INSET)
            .corner_radius(egui::CornerRadius::same(7))
            .inner_margin(egui::Margin::symmetric(9, 7))
            .show(ui, |ui| {
                // ---------- nagłówek: czempiona widać nawet po zwinięciu ----------
                let odpadlo = dane.wyniki.iter().filter(|w| w.zdyskwalifikowany()).count();
                let ogon = if odpadlo > 0 {
                    format!("   ({odpadlo} bez handlu / na zerze — na końcu listy)")
                } else {
                    String::new()
                };
                let tryb = if dane.approximate {
                    format!(
                        "APPROX N={} · NIE DO KORONACJI · ",
                        dane.quick_tick_stride
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "?".into())
                    )
                } else {
                    String::new()
                };
                let naglowek = format!(
                    "{tryb}przesiane presety · {}   ·   czoło: {}  {}{}",
                    ile,
                    dane.wyniki[czolo].nazwa,
                    dane.wyniki[czolo].napis(kryt),
                    ogon
                );
                // Font okna ma z trojkatow tylko te, ktore sa znakami EMOJI
                // (◀ U+25C0 i ▶ U+25B6). ▼, ▸, ▾, ↑, → emoji NIE sa i wychodza
                // jako pusty kwadrat -- stad zwykly plus i minus.
                let strzalka = if stan.rozwiniete { "-" } else { "+" };
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(format!("{strzalka} {naglowek}"))
                                .color(TEKST)
                                .size(12.0)
                                .strong(),
                        )
                        .fill(egui::Color32::TRANSPARENT)
                        .stroke(egui::Stroke::NONE),
                    )
                    .on_hover_text("statystyki presetów policzonych do tej pory (wyniki_czastkowe.json)")
                    .clicked()
                {
                    stan.rozwiniete = !stan.rozwiniete;
                    stan.wlasnie_dotkniety = stan.rozwiniete;
                }
                if !stan.rozwiniete {
                    return;
                }

                ui.add_space(4.0);
                if dane.approximate {
                    let n = dane
                        .quick_tick_stride
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "?".into());
                    let warning = dane.warning.as_deref().unwrap_or(
                        "Wynik przybliżony: finalista musi przejść dokładny backtest N=1.",
                    );
                    etykieta(
                        ui,
                        &format!(
                            "APPROX N={n} · coronation_eligible={} · {warning}",
                            dane.coronation_eligible
                        ),
                        CZERWIEN,
                        11.0,
                    );
                    ui.add_space(4.0);
                }

                // ---------- rząd sterujący ----------
                //
                // Kliknięcia zbieramy do zmiennych i wykonujemy PO rzędzie:
                // przestawienie kolejności w środku rysowania rzędu zmieniłoby
                // listę, po której zaraz iterujemy niżej.
                // Klawiatura i przyciski wpadają do tej samej zmiennej: to ma
                // być jeden ruch, nie dwa osobne mechanizmy.
                let mut skok: i64 = skok_klawiszem;
                let mut na_czolo = false;
                let mut nowe_kryt = stan.kryterium;
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(stan.wybrany > 0, egui::Button::new(egui::RichText::new("◀").size(12.0)))
                        .on_hover_text("poprzedni w kolejności")
                        .clicked()
                    {
                        skok = -1;
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(if dane.approximate {
                                    "CZOŁO SITA (APPROX)"
                                } else {
                                    "NAJLEPSZY PRESET"
                                }).size(11.5).color(TLO).strong(),
                            )
                            .fill(ZIELEN),
                        )
                        .on_hover_text("skacze na czoło i porządkuje resztę wg wybranego kryterium")
                        .clicked()
                    {
                        na_czolo = true;
                    }
                    if ui
                        .add_enabled(
                            stan.wybrany + 1 < ile,
                            egui::Button::new(egui::RichText::new("▶").size(12.0)),
                        )
                        .on_hover_text("następny w kolejności")
                        .clicked()
                    {
                        skok = 1;
                    }

                    ui.add_space(2.0);
                    etykieta(ui, "(strzałki ◀ ▶ działają też z klawiatury)", TEKST_SLABY, 10.0);
                    ui.add_space(8.0);
                    etykieta(ui, "sortuj wg:", TEKST_SLABY, 11.0);
                    egui::ComboBox::from_id_salt(format!("kryt-{}", z.id))
                        .selected_text(egui::RichText::new(kryt.etykieta).size(11.5))
                        .width(170.0)
                        .show_ui(ui, |ui| {
                            for (i, k) in m::przesiane::KRYTERIA.iter().enumerate() {
                                ui.selectable_value(&mut nowe_kryt, i, egui::RichText::new(k.etykieta).size(11.5));
                            }
                        });
                });

                if nowe_kryt != stan.kryterium {
                    stan.wlasnie_dotkniety = true;
                    stan.kryterium = nowe_kryt;
                    // Zmiana kryterium przestawia CAŁĄ kolejność, więc trzymanie
                    // pozycji nie miałoby sensu — wracamy na czoło, czyli do
                    // tego, co w nowym ujęciu jest najlepsze.
                    stan.przelicz(dane);
                    stan.wybrany = 0;
                }
                if na_czolo {
                    stan.wlasnie_dotkniety = true;
                    stan.przelicz(dane);
                    stan.wybrany = 0;
                }
                if skok != 0 {
                    let n = stan.wybrany as i64 + skok;
                    stan.wybrany = n.clamp(0, ile as i64 - 1) as usize;
                }

                // kryterium mogło się przed chwilą zmienić
                let kryt = &m::przesiane::KRYTERIA[stan.kryterium];

                // ---------- wybór presetu z listy ----------
                ui.add_space(4.0);
                let mut wybrany = stan.wybrany;
                let podpis = stan
                    .kolejnosc
                    .get(stan.wybrany)
                    .map(|&i| {
                        format!(
                            "#{}  {}   {}",
                            stan.wybrany + 1,
                            dane.wyniki[i].nazwa,
                            dane.wyniki[i].napis(kryt)
                        )
                    })
                    .unwrap_or_else(|| "—".into());
                egui::ComboBox::from_id_salt(format!("lista-{}", z.id))
                    .selected_text(egui::RichText::new(podpis).size(11.5).monospace())
                    .width(ui.available_width().min(430.0))
                    .height(320.0)
                    .show_ui(ui, |ui| {
                        for (poz, &i) in stan.kolejnosc.iter().enumerate() {
                            let w = &dane.wyniki[i];
                            // Liczba transakcji stoi w każdym wierszu, bo bez
                            // niej „100 % dni dodatnich" z jednej transakcji
                            // wygląda dokładnie tak samo jak z tysiąca.
                            let powod = w.powod_dyskwalifikacji();
                            let txt = format!(
                                "#{}  {}   {}   · {} trans.{}{}",
                                poz + 1,
                                w.nazwa,
                                w.napis(kryt),
                                w.transakcje(),
                                if powod.is_empty() { "" } else { "   " },
                                powod
                            );
                            ui.selectable_value(
                                &mut wybrany,
                                poz,
                                egui::RichText::new(txt)
                                    .size(11.0)
                                    .monospace()
                                    .color(if w.zdyskwalifikowany() { CZERWIEN } else { TEKST }),
                            );
                        }
                    });
                if wybrany != stan.wybrany {
                    stan.wlasnie_dotkniety = true;
                }
                stan.wybrany = wybrany.min(ile.saturating_sub(1));

                // ---------- statystyki wybranego ----------
                let Some(&i) = stan.kolejnosc.get(stan.wybrany) else { return };
                let w = &dane.wyniki[i];
                if w.wysadzony() {
                    ui.add_space(3.0);
                    etykieta(
                        ui,
                        "ten przebieg WYZEROWAŁ KONTO — liczby niżej są zapisem drogi do zera, nie wynikiem",
                        CZERWIEN,
                        11.0,
                    );
                } else if w.bez_handlu() {
                    ui.add_space(3.0);
                    etykieta(
                        ui,
                        "ten przebieg NIE ZAWARŁ ANI JEDNEJ TRANSAKCJI — zerowe ryzyko niżej to brak strategii, nie jej zaleta",
                        CZERWIEN,
                        11.0,
                    );
                }
                ui.add_space(4.0);
                // Wszystkie statystyki są widoczne jednocześnie. Zamiast
                // osobnego, niskiego obszaru przewijania układamy je w tyle
                // kolumn, ile faktycznie mieści szerokość okna. Na szerokim
                // monitorze pełne ~90 pól mieści się w 11–15 wierszach.
                let wiersze = w.wiersze();
                let kolumny = ((ui.available_width() / 315.0).floor() as usize)
                    .clamp(1, 8)
                    .min(wiersze.len().max(1));
                let na_kolumne = wiersze.len().div_ceil(kolumny);
                ui.columns(kolumny, |uis| {
                    for (nr, kolumna) in uis.iter_mut().enumerate() {
                        let od = nr * na_kolumne;
                        let do_ = (od + na_kolumne).min(wiersze.len());
                        for (k, v) in &wiersze[od..do_] {
                            wiersz_klucz_wartosc(kolumna, k, v);
                        }
                    }
                });
                // Nie ma już wewnętrznego scrolla tabeli; kółko i strzałki
                // zawsze należą do głównego okna.
                stan.rect_tabelki = None;
                stan.przewin_tabelki = 0.0;
            });
    }

    fn karta(&mut self, ui: &mut egui::Ui, z: &m::Postep, teraz: i64) {
        let zywy = z.zywy(teraz);
        let kolor = if zywy {
            kolor_rodzaju(&z.rodzaj)
        } else {
            CZERWIEN
        };
        let w = self
            .wygl
            .get(&z.id)
            .map(|x| (x.szybkosc, x.eta))
            .unwrap_or((z.szybkosc, z.eta_s));

        egui::Frame::default()
            .fill(KARTA)
            .stroke(egui::Stroke::new(
                1.0,
                if zywy {
                    KRESKA
                } else {
                    CZERWIEN.gamma_multiply(0.5)
                },
            ))
            .corner_radius(egui::CornerRadius::same(12))
            .inner_margin(egui::Margin::same(14))
            .show(ui, |ui| {
                // ---------- nagłówek ----------
                ui.horizontal(|ui| {
                    let znacznik = if z.rodzaj == m::TRENING {
                        "TRENING"
                    } else {
                        "BACKTEST"
                    };
                    egui::Frame::default()
                        .fill(kolor.gamma_multiply(0.16))
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(egui::Margin::symmetric(7, 3))
                        .show(ui, |ui| etykieta(ui, znacznik, kolor, 11.0));
                    ui.label(
                        egui::RichText::new(&z.nazwa)
                            .color(TEKST)
                            .size(14.0)
                            .strong(),
                    );
                });
                ui.add_space(2.0);

                // ---------- pasek ----------
                pasek(ui, z.postep, kolor, !zywy);

                // ---------- cienki pasek: JEDEN element, nie całość ----------
                if !z.etykieta_biezacego.is_empty() {
                    ui.add_space(3.0);
                    pasek_cienki(ui, z.postep_biezacy, kolor, !zywy, &z.etykieta_biezacego);
                }

                // ---------- iskierka prędkości ----------
                //
                // Zapisujemy PRÓBKĘ TYLKO wtedy, gdy zadanie przyslalo nowy
                // znacznik czasu. Bez tego warunku okno odswiezane co 120 ms
                // wpisywaloby te sama wartosc dziesiatki razy i wykres
                // pokazywalby plaska kreske niezaleznie od tego, co sie dzieje.
                if zywy && w.0 > 0.0 {
                    let h = self.historia.entry(z.id.clone()).or_default();
                    if h.last()
                        .map(|(t, _)| *t != z.aktualizacja_ts)
                        .unwrap_or(true)
                    {
                        h.push((z.aktualizacja_ts, w.0));
                        if h.len() > HIST_MAX {
                            let nadmiar = h.len() - HIST_MAX;
                            h.drain(0..nadmiar);
                        }
                    }
                }
                if let Some(h) = self.historia.get(&z.id) {
                    if h.len() >= 2 {
                        ui.add_space(4.0);
                        let szer = ui.available_width();
                        iskierka(ui, h, kolor, szer, 26.0);
                        let pierwsza = h[0].1;
                        let ostatnia = h[h.len() - 1].1;
                        if pierwsza > 0.0 {
                            let zmiana = (ostatnia / pierwsza - 1.0) * 100.0;
                            // Bez glifu: ▲ i ▼ nie sa emoji, wiec font okna
                            // rysowal w tym miejscu pusty kwadrat. Kierunek
                            // niesie kolor i znak przy liczbie -- trojkat nic
                            // nie dokladal poza dziura.
                            let barwa = if zmiana >= 5.0 {
                                ZIELEN
                            } else if zmiana <= -5.0 {
                                CZERWIEN
                            } else {
                                TEKST_SLABY
                            };
                            ui.add_space(1.0);
                            etykieta(
                                ui,
                                &format!(
                                    "{zmiana:+.0} % od {} próbek · szczyt {} {}",
                                    h.len(),
                                    m::pl_duza(h.iter().map(|(_, v)| *v).fold(0.0, f64::max)),
                                    z.jednostka_szybkosci
                                ),
                                barwa,
                                10.0,
                            );
                        }
                    }
                }

                // ---------- liczby ----------
                let szybkosc = if zywy && w.0 > 0.0 {
                    format!("{} {}", m::pl_duza(w.0), z.jednostka_szybkosci)
                } else {
                    String::new()
                };
                egui::Sides::new().shrink_left().show(
                    ui,
                    |ui| mono(ui, &liczby_bezwzgledne(z), TEKST, 12.0),
                    |ui| mono(ui, &szybkosc, TEKST_SLABY, 12.0),
                );

                ui.horizontal(|ui| {
                    etykieta(ui, "minęło", TEKST_SLABY, 11.5);
                    mono(ui, &m::pl_czas(z.trwa_s(teraz)), TEKST, 12.0);
                    ui.add_space(10.0);
                    etykieta(ui, "zostało", TEKST_SLABY, 11.5);
                    let eta = if zywy {
                        m::pl_czas(w.1)
                    } else {
                        "—".to_string()
                    };
                    mono(ui, &eta, if zywy { BURSZTYN } else { TEKST_SLABY }, 12.0);
                    if zywy {
                        let g = godzina_konca(w.1);
                        if !g.is_empty() {
                            ui.add_space(6.0);
                            mono(ui, &g, TEKST_SLABY, 11.5);
                        }
                    }
                });

                // ---------- co teraz ----------
                if !z.co_teraz.is_empty() {
                    ui.add_space(3.0);
                    egui::Frame::default()
                        .fill(INSET)
                        .corner_radius(egui::CornerRadius::same(7))
                        .inner_margin(egui::Margin::symmetric(9, 6))
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                etykieta(ui, "teraz:", TEKST_SLABY, 11.5);
                                ui.label(
                                    egui::RichText::new(&z.co_teraz)
                                        .color(TEKST)
                                        .size(12.0)
                                        .monospace(),
                                );
                            });
                        });
                }

                // ---------- statystyki ----------
                if !z.statystyki.is_empty() {
                    ui.add_space(4.0);
                    egui::Frame::default()
                        .fill(INSET)
                        .corner_radius(egui::CornerRadius::same(7))
                        .inner_margin(egui::Margin::symmetric(9, 7))
                        .show(ui, |ui| {
                            for (k, v) in z.statystyki.iter() {
                                wiersz_klucz_wartosc(ui, k, v);
                            }
                        });
                }

                // ---------- przesiane presety ----------
                self.przeglad_presetow(ui, z);

                ui.add_space(6.0);

                // ---------- akcje ----------
                if !zywy {
                    let usun = egui::Sides::new()
                        .shrink_left()
                        .show(
                            ui,
                            |ui| {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "ZADANIE PADŁO — brak znaku życia od {}",
                                        m::pl_czas(z.cisza_s(teraz))
                                    ))
                                    .color(CZERWIEN)
                                    .size(12.0)
                                    .strong(),
                                );
                            },
                            |ui| {
                                ui.button(egui::RichText::new("usuń wpis").size(12.0))
                                    .clicked()
                            },
                        )
                        .1;
                    if usun {
                        m::sprzataj(&self.dir, &z.id);
                        self.widziane.remove(&z.id);
                    }
                } else if z.przerywanie || self.poproszone.contains_key(&z.id) {
                    ui.label(
                        egui::RichText::new(
                            "PRZERYWAM — zadanie domyka bieżący krok i zapisuje wynik…",
                        )
                        .color(BURSZTYN)
                        .size(12.0)
                        .strong(),
                    );
                } else if self.pytanie.as_deref() == Some(z.id.as_str()) {
                    let (tak, nie) = egui::Sides::new()
                        .shrink_left()
                        .show(
                            ui,
                            |ui| {
                                etykieta(
                                    ui,
                                    "Przerwać? Wynik cząstkowy zostanie ZAPISANY.",
                                    BURSZTYN,
                                    12.0,
                                );
                            },
                            |ui| {
                                // Sides układa prawą stronę OD PRAWEJ, więc
                                // „nie" dodane pierwsze wypada skrajnie z brzegu.
                                let n = ui.button(egui::RichText::new("nie").size(12.0)).clicked();
                                let t = ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new("TAK, PRZERWIJ")
                                                .size(12.0)
                                                .color(TLO)
                                                .strong(),
                                        )
                                        .fill(CZERWIEN),
                                    )
                                    .clicked();
                                (t, n)
                            },
                        )
                        .1;
                    if nie {
                        self.pytanie = None;
                    }
                    if tak {
                        let _ = m::popros_o_stop(&self.dir, &z.id);
                        self.poproszone.insert(z.id.clone(), ());
                        self.pytanie = None;
                    }
                } else {
                    let klik = egui::Sides::new()
                        .show(
                            ui,
                            |_ui| {},
                            |ui| {
                                ui.add(
                                    egui::Button::new(
                                        egui::RichText::new("PRZERWIJ")
                                            .size(12.5)
                                            .color(CZERWIEN)
                                            .strong(),
                                    )
                                    .fill(CZERWIEN.gamma_multiply(0.14))
                                    .stroke(egui::Stroke::new(1.0, CZERWIEN.gamma_multiply(0.6))),
                                )
                                .on_hover_text(
                                    "zadanie zatrzyma się i ZAPISZE to, co zdążyło policzyć",
                                )
                                .clicked()
                            },
                        )
                        .1;
                    if klik {
                        self.pytanie = Some(z.id.clone());
                    }
                }
            });
    }

    // ============================================================
    //  PASKI ZBIORCZE
    // ============================================================

    /// Zestawienie dla JEDNEGO rodzaju zadań.
    ///
    /// `None` znaczy „nic takiego się nie liczy" i wtedy paska w ogóle nie ma.
    /// Pusty pasek na stałe byłby szumem — jego widok musi coś znaczyć.
    fn zbiorczo(&self, trening: bool, teraz: i64) -> Option<m::zbiorczy::Zbiorczy> {
        let skladniki: Vec<m::zbiorczy::Skladnik<'_>> = self
            .zadania
            .iter()
            .filter(|z| (z.rodzaj == m::TRENING) == trening)
            .map(|z| {
                // te same wygładzone liczby, co na karcie — inaczej suma na
                // górze przeczyłaby składnikom pod nią
                let (v, e) = self
                    .wygl
                    .get(&z.id)
                    .map(|x| (x.szybkosc, x.eta))
                    .unwrap_or((z.szybkosc, z.eta_s));
                m::zbiorczy::Skladnik {
                    zadanie: z,
                    szybkosc: v,
                    eta_s: e,
                }
            })
            .collect();
        m::zbiorczy::zestaw(&skladniki, teraz)
    }

    /// Plakietka zbiorcza w PASKU TYTUŁU: nitka postępu i liczby.
    ///
    /// # Dlaczego w pasku tytułu, a nie osobnym rzędzie
    ///
    /// To okno ogląda się kątem oka przez kilka godzin. Każdy rząd zabrany
    /// kartom zadań to jedna karta mniej widoczna bez przewijania, a pasek
    /// tytułu i tak miał w środku pustkę. Plakietki dokładają się do niej
    /// z prawej, tuż obok licznika zadań, więc ani napis „CONDUIT · postęp",
    /// ani przełącznik „na wierzchu" nie drgną, gdy plakietka się pojawi albo
    /// zniknie — to jest jedyny układ, w którym nic nie skacze.
    ///
    /// # Co ustępuje przy ciasnocie
    ///
    /// Trzy liczby (procent, czas, prędkość) mieszczą się w jednym rzędzie
    /// tylko wtedy, gdy liczy się JEDEN rodzaj zadań. Przy dwóch plakietkach
    /// naraz i domyślnej szerokości okna trzeba coś oddać, więc oddajemy
    /// w kolejności od najmniej pilnego: najpierw prędkość, potem czas, na
    /// końcu nitkę. Sam procent zostaje ZAWSZE — [`Stopien`]. Pełne zdanie
    /// (z prędkością, godziną końca i zastrzeżeniami) jest pod kursorem.
    fn plakietka_zbiorcza(
        ui: &mut egui::Ui,
        skrot: &str,
        pelna_nazwa: &str,
        kolor: egui::Color32,
        z: &m::zbiorczy::Zbiorczy,
        st: Stopien,
    ) {
        // Zawartość mamy w KOLEJNOŚCI CZYTANIA, a rysujemy ją od tyłu.
        //
        // Plakietka siedzi w prawej połowie paska tytułu, a ta układa się od
        // prawej do lewej — pierwszy dodany element ląduje najbardziej z prawej.
        // Pierwsza wersja wyglądała więc jak „12 min · 78 % · nitka · BT".
        // Wymuszenie układu od lewej wewnątrz ramki jest jeszcze gorsze: taki
        // podrzędny widok bierze CAŁĄ wolną szerokość i plakietki nachodzą
        // wtedy na siebie i na napis „CONDUIT · postęp" (widziane na ekranie).
        let czesci = czesci_plakietki(skrot, z, st);

        let odp = egui::Frame::default()
            .fill(kolor.gamma_multiply(0.12))
            .stroke(egui::Stroke::new(1.0, kolor.gamma_multiply(0.35)))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(7, 1))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = NITKA_ODSTEP;
                for (i, c) in czesci.iter().enumerate().rev() {
                    match c {
                        // Skrót rodzaju („BT"/„AI") bierze kolor grupy — to on
                        // razem z barwą ramki mówi, na co się patrzy.
                        Czesc::Tekst(t, k, r) => mono(ui, t, if i == 0 { kolor } else { *k }, *r),
                        Czesc::Nitka(p) => pasek_nitka(ui, *p, kolor, NITKA_SZER, 8.0),
                    }
                }
            })
            .response;

        // Pod kursorem CAŁA prawda, łącznie z tym, co przy ciasnocie wypadło
        // z plakietki. Podpowiedź jest dodatkiem, nie jedyną drogą do liczby.
        let mut opis = format!("{pelna_nazwa} · {}", odmien_zadania(z.zadan));
        if !z.srednia_z_ulamkow && z.calosc > 0.0 {
            opis.push_str(&format!(
                "\n{}",
                liczby_skali(z.zrobione, z.calosc, &z.jednostka)
            ));
        }
        if z.srednia_z_ulamkow {
            opis.push_str("\nprocent to ŚREDNIA z zadań o różnych jednostkach — ticków nie wolno dodać do ocen");
        }
        if !z.szybkosci.is_empty() {
            opis.push_str(&format!("\nłącznie {}", z.opis_szybkosci()));
        }
        if z.eta_s >= 0.0 {
            opis.push_str(&format!(
                "\nzostało {}{} · {}",
                if z.eta_dolna_granica {
                    "co najmniej "
                } else {
                    ""
                },
                m::pl_czas(z.eta_s),
                godzina_konca(z.eta_s)
            ));
        } else {
            opis.push_str("\nczasu do końca jeszcze nie da się policzyć");
        }
        if z.bez_skali > 0 {
            opis.push_str(&format!(
                "\n{} zadań nie podaje skali — nie wchodzą do procentu",
                z.bez_skali
            ));
        }
        odp.on_hover_text(opis);
    }

    // ============================================================
    //  POCZEKALNIA — rysowanie
    // ============================================================

    fn panel_poczekalni(&mut self, ui: &mut egui::Ui, teraz: i64) {
        // Migawka na czas rysowania. Rysowanie pożycza `self` na niezmienne,
        // a każde kliknięcie zmienia kolejkę — bez tego nie da się tego napisać
        // bez walki z pożyczkami. Lista ma kilka pozycji, koszt jest żaden.
        let wpisy = self.kolejka.wpisy.clone();
        let procenty: Vec<Option<f64>> = wpisy
            .iter()
            .map(|w| self.karta_pozycji(w.pid).map(|z| z.postep))
            .collect();
        let zajeta = self.maszyna_zajeta(teraz);
        let mut akcje: Vec<Akcja> = Vec::new();
        let mut auto = self.kolejka.auto;
        let mut rozwin = self.pocz_rozwinieta;
        let czeka = wpisy
            .iter()
            .filter(|w| w.stan == m::kolejka::Stan::Czeka)
            .count();
        let skonczone = wpisy.iter().filter(|w| w.stan.skonczony()).count();

        // ---------- nagłówek: zawsze widoczny ----------
        egui::Sides::new().shrink_left().show(
            ui,
            |ui| {
                ui.label(
                    egui::RichText::new("POCZEKALNIA")
                        .color(BURSZTYN)
                        .size(11.5)
                        .strong(),
                );
                let opis = if wpisy.is_empty() {
                    "pusta — wpisz polecenie, ruszy, gdy maszyna będzie wolna".to_string()
                } else {
                    let mut cz = Vec::new();
                    if czeka > 0 {
                        cz.push(format!("{czeka} czeka"));
                    }
                    if self.dziecko.is_some() {
                        cz.push("1 liczy się".to_string());
                    }
                    if skonczone > 0 {
                        cz.push(format!("{skonczone} po wszystkim"));
                    }
                    cz.join(" · ")
                };
                ui.label(egui::RichText::new(opis).color(TEKST_SLABY).size(11.0));
            },
            |ui| {
                // `Sides` układa prawą stronę OD PRAWEJ
                if ui
                    .button(egui::RichText::new(if rozwin { "zwiń" } else { "rozwiń" }).size(11.0))
                    .clicked()
                {
                    rozwin = !rozwin;
                }
                ui.checkbox(&mut auto, egui::RichText::new("po kolei").size(11.0))
                    .on_hover_text(
                        "zaznaczone: okno samo startuje kolejną pozycję, gdy maszyna się zwolni",
                    );
            },
        );

        if !rozwin {
            self.pocz_rozwinieta = rozwin;
            self.zapisz_auto(auto);
            return;
        }

        ui.add_space(5.0);

        // ---------- co się teraz dzieje z kolejką ----------
        if czeka > 0 {
            let powod = if !auto {
                "kolejka wyłączona — nic samo nie ruszy".to_string()
            } else if self.dziecko.is_some() {
                "czekam na koniec pozycji, która się liczy".to_string()
            } else if zajeta {
                "czekam, aż maszyna się zwolni — coś już liczy".to_string()
            } else {
                "za chwilę ruszam".to_string()
            };
            etykieta(ui, &powod, TEKST_SLABY, 11.0);
            ui.add_space(3.0);
        }

        // ---------- lista ----------
        egui::ScrollArea::vertical()
            .id_salt("lista-poczekalni")
            .max_height(190.0)
            .show(ui, |ui| {
                for (i, w) in wpisy.iter().enumerate() {
                    Apka::wiersz_poczekalni(ui, i, w, procenty[i], teraz, &mut akcje);
                    ui.add_space(4.0);
                }
            });

        // ---------- formularz ----------
        ui.add_space(4.0);
        egui::Frame::default()
            .fill(INSET)
            .corner_radius(egui::CornerRadius::same(7))
            .inner_margin(egui::Margin::symmetric(9, 7))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    etykieta(ui, "nazwa", TEKST_SLABY, 11.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.f_nazwa)
                            .hint_text("nieobowiązkowa")
                            .desired_width(150.0),
                    );
                    etykieta(ui, "katalog", TEKST_SLABY, 11.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.f_katalog)
                            .hint_text("katalog roboczy")
                            .desired_width(ui.available_width() - 4.0),
                    );
                });
                ui.horizontal(|ui| {
                    let dodaj = ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("DODAJ").size(11.5).color(TLO).strong(),
                            )
                            .fill(BURSZTYN),
                        )
                        .clicked();
                    if dodaj {
                        akcje.push(Akcja::Dodaj);
                    }
                    let pole = ui.add(
                        egui::TextEdit::singleline(&mut self.f_polecenie)
                            .hint_text("pełny wiersz polecenia, np. \"…\\btp.exe\" --presets presets_x --out out_x")
                            .font(egui::TextStyle::Monospace)
                            .desired_width(ui.available_width() - 4.0),
                    );
                    // Enter w polu polecenia = to samo, co przycisk. Kolejkę
                    // układa się seriami i sięganie po mysz co pozycję męczy.
                    if pole.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        akcje.push(Akcja::Dodaj);
                    }
                });
                if !self.f_komunikat.is_empty() {
                    ui.label(
                        egui::RichText::new(&self.f_komunikat)
                            .color(BURSZTYN)
                            .size(10.5),
                    );
                }
                egui::Sides::new().shrink_left().show(
                    ui,
                    |ui| {
                        etykieta(
                            ui,
                            "zamknięcie okna NIE zatrzyma uruchomionego zadania",
                            TEKST_SLABY.gamma_multiply(0.7),
                            10.0,
                        );
                    },
                    |ui| {
                        if skonczone > 0
                            && ui
                                .button(egui::RichText::new("usuń domknięte").size(10.5))
                                .clicked()
                        {
                            akcje.push(Akcja::SprzatajSkonczone);
                        }
                    },
                );
            });

        self.pocz_rozwinieta = rozwin;
        self.zapisz_auto(auto);
        self.wykonaj(akcje, teraz);
    }

    /// Zmiana wyłącznika „po kolei" idzie od razu na dysk — inaczej po zamknięciu
    /// okna kolejka ruszyłaby wbrew temu, co użytkownik ustawił.
    fn zapisz_auto(&mut self, auto: bool) {
        if self.kolejka.auto != auto {
            self.kolejka.auto = auto;
            let _ = self.kolejka.zapisz(&self.dir);
        }
    }

    fn wiersz_poczekalni(
        ui: &mut egui::Ui,
        i: usize,
        w: &m::kolejka::Wpis,
        procent: Option<f64>,
        teraz: i64,
        akcje: &mut Vec<Akcja>,
    ) {
        use m::kolejka::Stan;
        let (napis, kolor) = match w.stan {
            Stan::Czeka => ("CZEKA", TEKST_SLABY),
            Stan::Liczy => ("LICZY SIĘ", ZIELEN),
            Stan::Gotowe => ("GOTOWE", ZIELEN),
            Stan::Padlo => ("PADŁO", CZERWIEN),
            Stan::Nieznane => ("NIE WIEM", BURSZTYN),
        };
        egui::Frame::default()
            .fill(INSET)
            .stroke(egui::Stroke::new(1.0, kolor.gamma_multiply(0.30)))
            .corner_radius(egui::CornerRadius::same(7))
            .inner_margin(egui::Margin::symmetric(9, 6))
            .show(ui, |ui| {
                egui::Sides::new().shrink_left().show(
                    ui,
                    |ui| {
                        mono(ui, &format!("{}.", i + 1), TEKST_SLABY, 11.0);
                        plakietka(ui, napis, kolor);
                        ui.label(
                            egui::RichText::new(skroc(w.podpis(), 46)).color(TEKST).size(11.5),
                        );
                    },
                    |ui| {
                        // prawa strona układa się OD PRAWEJ
                        if w.stan == Stan::Liczy {
                            etykieta(
                                ui,
                                "przerwij w karcie zadania wyżej",
                                TEKST_SLABY.gamma_multiply(0.7),
                                10.0,
                            );
                        } else {
                            if ui
                                .small_button(egui::RichText::new("usuń").size(10.5))
                                .clicked()
                            {
                                akcje.push(Akcja::Usun(w.id.clone()));
                            }
                            if ui
                                .small_button(egui::RichText::new("niżej").size(10.5))
                                .clicked()
                            {
                                akcje.push(Akcja::WDol(w.id.clone()));
                            }
                            if ui
                                .small_button(egui::RichText::new("wyżej").size(10.5))
                                .clicked()
                            {
                                akcje.push(Akcja::WGore(w.id.clone()));
                            }
                            if w.stan == Stan::Czeka
                                && ui
                                    .small_button(egui::RichText::new("uruchom teraz").size(10.5))
                                    .on_hover_text(
                                        "startuje mimo zajętej maszyny — sweepy odbiorą sobie rdzenie",
                                    )
                                    .clicked()
                            {
                                akcje.push(Akcja::UruchomTeraz(w.id.clone()));
                            }
                        }
                    },
                );

                // pełne polecenie — to ono naprawdę pójdzie do systemu
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(skroc(&w.polecenie, 96))
                            .color(TEKST_SLABY.gamma_multiply(0.85))
                            .size(10.5)
                            .monospace(),
                    )
                    .truncate(),
                )
                .on_hover_text(if w.katalog.is_empty() {
                    w.polecenie.clone()
                } else {
                    format!("{}\n\nkatalog: {}", w.polecenie, w.katalog)
                });

                // ---------- szczegóły stanu ----------
                match w.stan {
                    Stan::Liczy => {
                        let mut opis = format!("minęło {}", m::pl_czas(w.trwa_s(teraz)));
                        match procent {
                            Some(p) => {
                                opis.push_str(&format!(" · {} %", m::pl_liczba(p * 100.0, 1)))
                            }
                            // Proces żyje, ale jeszcze nie napisał pliku postępu
                            // — mówimy to, zamiast rysować zero.
                            None => opis.push_str(" · jeszcze nie melduje postępu"),
                        }
                        mono(ui, &opis, ZIELEN, 10.5);
                    }
                    Stan::Gotowe => {
                        mono(
                            ui,
                            &format!("skończone w {} · kod 0", m::pl_czas(w.trwa_s(teraz))),
                            TEKST_SLABY,
                            10.5,
                        );
                    }
                    Stan::Padlo | Stan::Nieznane => {
                        let kod = match w.kod {
                            Some(k) => format!("kod {k} · "),
                            None => String::new(),
                        };
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!("{kod}{}", w.uwaga))
                                    .color(kolor)
                                    .size(10.5),
                            )
                            .wrap(),
                        );
                    }
                    Stan::Czeka => {}
                }
            });
    }

    /// Wykonanie kliknięć zebranych przy rysowaniu.
    fn wykonaj(&mut self, akcje: Vec<Akcja>, teraz: i64) {
        let mut zmiana = false;
        // Enter w polu polecenia i kliknięcie DODAJ mogą trafić się w TEJ SAMEJ
        // klatce (przycisk odbiera ognisko polu). Bez tego zamka jedno
        // zamówienie wchodziłoby do kolejki dwa razy i backtest liczyłby się
        // dwukrotnie.
        let mut juz_dodano = false;
        for a in akcje {
            match a {
                Akcja::WGore(id) => zmiana |= self.kolejka.przesun(&id, true),
                Akcja::WDol(id) => zmiana |= self.kolejka.przesun(&id, false),
                Akcja::Usun(id) => zmiana |= self.kolejka.usun(&id),
                Akcja::SprzatajSkonczone => {
                    zmiana |= self.kolejka.sprzataj_skonczone() > 0;
                }
                Akcja::UruchomTeraz(id) => {
                    // Jeden proces z kolejki naraz — inaczej okno przestałoby
                    // panować nad tym, co samo uruchomiło.
                    if self.dziecko.is_some() {
                        self.f_komunikat =
                            "najpierw musi skończyć pozycja, która już się liczy".into();
                    } else if let Some(i) = self.kolejka.wpisy.iter().position(|w| w.id == id) {
                        self.startuj(i, teraz);
                        zmiana = true;
                    }
                }
                Akcja::Dodaj if juz_dodano => {}
                Akcja::Dodaj => {
                    juz_dodano = true;
                    let nazwa = self.f_nazwa.clone();
                    let polecenie = self.f_polecenie.clone();
                    let katalog = self.f_katalog.clone();
                    match self.kolejka.dodaj(&nazwa, &polecenie, &katalog, teraz) {
                        Ok(_) => {
                            self.f_nazwa.clear();
                            self.f_komunikat.clear();
                            zmiana = true;
                        }
                        Err(e) => self.f_komunikat = format!("nie dodałem: {e}"),
                    }
                }
            }
        }
        if zmiana {
            let _ = self.kolejka.zapisz(&self.dir);
        }
    }

    fn karta_zakonczonego(ui: &mut egui::Ui, z: &Zakonczone) {
        let kolor = if z.przerwane { BURSZTYN } else { ZIELEN };
        egui::Frame::default()
            .fill(KARTA)
            .stroke(egui::Stroke::new(1.0, kolor.gamma_multiply(0.45)))
            .corner_radius(egui::CornerRadius::same(12))
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new(if z.przerwane {
                            "PRZERWANO — POSTĘP ZAPISANY"
                        } else {
                            "GOTOWE"
                        })
                        .color(kolor)
                        .size(12.0)
                        .strong(),
                    );
                    ui.label(egui::RichText::new(&z.nazwa).color(TEKST).size(12.5));
                });
                ui.label(
                    egui::RichText::new(format!("{} · {}", z.rodzaj, z.podsumowanie))
                        .color(TEKST_SLABY)
                        .size(11.5),
                );
            });
    }
}

// ============================================================
//  PĘTLA OKNA
// ============================================================

impl eframe::App for Apka {
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] {
        [0.043, 0.055, 0.078, 1.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _f: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.odswiez();
        let teraz = m::teraz_ms();

        // ---------- PRZEWIJANIE Z KLAWIATURY ----------
        //
        // Okno rosnie razem z liczba zadan i przy kilku przebiegach naraz
        // dolne karty wychodza poza ekran. Kolko myszy dzialalo od zawsze,
        // ale wymagalo trzymania kursora nad lista — przy pracy w terminalu
        // obok to jest jedno siegniecie po mysz za duzo.
        //
        // Zapisujemy DELTE do zastosowania w tej klatce, a nie pozycje:
        // wymuszanie pozycji (`vertical_scroll_offset`) co klatke zabraloby
        // kolko, bo egui nadpisywaloby jego wynik przy kazdym rysowaniu.
        //
        // `Home`/`End` daja skok o wielka wartosc — `ScrollArea` i tak
        // przycina do zakresu, wiec nie trzeba znac wysokosci zawartosci.
        self.przewin = ctx.input_mut(|i| {
            let mut d = 0.0_f32;
            let strzalki = [
                (egui::Key::ArrowDown, -KROK_PRZEWIJANIA),
                (egui::Key::ArrowUp, KROK_PRZEWIJANIA),
                (egui::Key::PageDown, -KROK_PRZEWIJANIA * 6.0),
                (egui::Key::PageUp, KROK_PRZEWIJANIA * 6.0),
                (egui::Key::Home, 1.0e6),
                (egui::Key::End, -1.0e6),
            ];
            for (k, krok) in strzalki {
                // `count` zamiast `pressed`: przy przytrzymaniu klawisza egui
                // powtarza zdarzenie, wiec lista przewija sie plynnie zamiast
                // skakac raz na nacisniecie.
                let n = i.count_and_consume_key(egui::Modifiers::NONE, k);
                if n > 0 {
                    d += krok * n as f32;
                }
            }
            d
        });

        // STRZAŁKI W BOK: poprzedni / następny preset w przeglądarce wyników.
        //
        // Osobno od góra-dół, bo tamte przewijają listę zadań, a te chodzą po
        // presetach. Klawiszy NIE zabieramy, gdy kursor stoi w polu tekstowym
        // poczekalni albo gdy otwarta jest rozwijana lista — tam lewo i prawo
        // mają własne, oczywiste znaczenie i podkradzenie ich byłoby usterką.
        let zajete = ctx.memory(|m| m.focused().is_some()) || egui::Popup::is_any_open(&ctx);
        self.skok_presetu = if zajete {
            0
        } else {
            ctx.input_mut(|i| {
                let w_prawo =
                    i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight) as i64;
                let w_lewo =
                    i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft) as i64;
                w_prawo - w_lewo
            })
        };
        // Wybór karty musi być załatwiony PRZED rysowaniem, bo panel czyta
        // `aktywny_przeglad` w trakcie.
        let aktywna_zyje = self
            .aktywny_przeglad
            .as_ref()
            .map(|id| self.przeglad.get(id).map(|p| p.rozwiniete).unwrap_or(false))
            .unwrap_or(false);
        if !aktywna_zyje {
            self.aktywny_przeglad = self
                .przeglad
                .iter()
                .find(|(_, p)| p.rozwiniete)
                .map(|(id, _)| id.clone());
        }

        // KOMU trafiają strzałki góra-dół.
        //
        // Zasada jak ognisko w przeglądarce: liczy się to, co człowiek
        // OSTATNIO KLIKNĄŁ. Klik w tabelkę statystyk oddaje jej przewijanie,
        // klik gdziekolwiek indziej — całemu oknu. Kursor trzymany nad
        // tabelką ma pierwszeństwo, bo tak samo zachowuje się kółko myszy i
        // rozjazd między jednym a drugim byłby niespodzianką.
        //
        // Prostokąt tabelki pochodzi z POPRZEDNIEJ klatki — wcześniej go nie
        // ma, bo bierze się z rysowania. Przy 120 ms na klatkę to poniżej
        // progu zauważalności.
        if ctx.input(|i| i.pointer.primary_pressed()) {
            let klik = ctx.input(|i| i.pointer.interact_pos());
            self.focus_przewijania = klik.and_then(|poz| self.tabelka_pod(poz));
        }
        let cel = ctx
            .pointer_latest_pos()
            .and_then(|poz| self.tabelka_pod(poz))
            .or_else(|| self.focus_przewijania.clone());
        if self.przewin != 0.0 {
            if let Some(id) = cel {
                if let Some(x) = self.przeglad.get_mut(&id) {
                    x.przewin_tabelki = self.przewin;
                    // Okno NIE dostaje tej samej delty: przewinęłoby się
                    // razem z tabelką i wyszłoby podwójne szarpnięcie.
                    self.przewin = 0.0;
                }
            }
        }

        // Kolejkę doglądamy PO odczycie katalogu, a przed rysowaniem: dzięki
        // temu pozycja, która właśnie ruszyła, jest w tej samej klatce widoczna
        // jako „liczy się", a nie dopiero za 120 ms.
        self.dogladaj_kolejke(teraz);

        if self.czas_sie_zamknac() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // Zestawienia liczymy PRZED rysowaniem paska tytułu, bo to od nich
        // zależy, ile miejsca w tym pasku zostaje na cokolwiek innego.
        // `None` = nic takiego się nie liczy i plakietki po prostu nie ma.
        let bt = self.zbiorczo(false, teraz);
        let tr = self.zbiorczo(true, teraz);

        egui::Panel::top(egui::Id::new("gora"))
            .frame(
                egui::Frame::default()
                    .fill(TLO)
                    .inner_margin(egui::Margin::symmetric(14, 9)),
            )
            .show(ui, |ui| {
                let zywe = self.zadania.iter().filter(|z| z.zywy(teraz)).count();
                let mut nw = self.na_wierzchu;
                let zmiana = egui::Sides::new()
                    .shrink_left()
                    // Siatka bezpieczeństwa: gdyby plakietki mimo wszystko
                    // nie zmieściły się co do punktu, tytuł ma się PRZYCIĄĆ,
                    // a nie dać się zamalować plakietką.
                    .truncate()
                    .show(
                        ui,
                        |ui| {
                            ui.label(
                                egui::RichText::new("CONDUIT")
                                    .color(ZIELEN)
                                    .size(13.0)
                                    .strong(),
                            );
                            ui.label(
                                egui::RichText::new("· postęp")
                                    .color(TEKST_SLABY)
                                    .size(13.0),
                            );
                        },
                        |ui| {
                            // Prawa strona układa się OD PRAWEJ, więc dodane tu
                            // pierwsze zostają przy krawędzi na zawsze. To jest
                            // powód, dla którego plakietki wchodzą DALEJ:
                            // przełącznik i licznik nie drgną, gdy plakietka
                            // pojawi się albo zniknie.
                            let z = ui
                                .checkbox(&mut nw, egui::RichText::new("na wierzchu").size(11.5))
                                .changed();
                            ui.label(
                                egui::RichText::new(odmien_zadania(zywe))
                                    .color(TEKST_SLABY)
                                    .size(11.5),
                            );

                            let ile = bt.is_some() as usize + tr.is_some() as usize;
                            if ile > 0 {
                                // Budżet dzielimy PO RÓWNO, a stopień
                                // szczegółowości bierzemy WSPÓLNY — ten gorszy
                                // z dwóch. Gdyby każda plakietka dobierała
                                // sobie sama, jedna miałaby prędkość, a druga
                                // nie, i porównanie ich wzrokiem przestałoby
                                // działać.
                                //
                                // Od `available_width` odejmujemy miejsce
                                // NALEŻNE tytułowi: prawa strona rysuje się
                                // pierwsza i bez tego zabrałaby cały pasek.
                                let budzet = (ui.available_width()
                                    - szerokosc_tytulu(ui)
                                    - ui.spacing().item_spacing.x * (ile as f32 + 1.0))
                                    / ile as f32;
                                let st = [
                                    tr.as_ref().map(|z| dobierz_stopien(ui, "AI", z, budzet)),
                                    bt.as_ref().map(|z| dobierz_stopien(ui, "BT", z, budzet)),
                                ]
                                .into_iter()
                                .flatten()
                                .min()
                                .flatten();
                                if let Some(st) = st {
                                    if let Some(z) = &tr {
                                        Apka::plakietka_zbiorcza(
                                            ui,
                                            "AI",
                                            "wszystkie treningi AI",
                                            FIOLET,
                                            z,
                                            st,
                                        );
                                    }
                                    if let Some(z) = &bt {
                                        Apka::plakietka_zbiorcza(
                                            ui,
                                            "BT",
                                            "wszystkie backtesty",
                                            ZIELEN,
                                            z,
                                            st,
                                        );
                                    }
                                }
                            }
                            z
                        },
                    )
                    .1;
                if zmiana {
                    self.na_wierzchu = nw;
                    ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if nw {
                        egui::WindowLevel::AlwaysOnTop
                    } else {
                        egui::WindowLevel::Normal
                    }));
                }
            });

        // ---------- stopka: co robia klawisze ----------
        //
        //  Funkcja, o ktorej nie wiadomo, ze istnieje, jest funkcja, ktorej
        //  nie ma. Przewijanie klawiatura dodano 26.08.2026 i bez tej linijki
        //  nikt by go nie znalazl — okno nie ma menu ani pomocy.
        egui::Panel::bottom(egui::Id::new("stopka-klawisze"))
            .frame(
                egui::Frame::default()
                    .fill(TLO)
                    .inner_margin(egui::Margin::symmetric(12, 4)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    etykieta(ui, "↑ ↓", TEKST_SLABY, 10.5);
                    etykieta(ui, "przewijanie", TEKST_SLABY.gamma_multiply(0.75), 10.5);
                    ui.add_space(10.0);
                    etykieta(ui, "PgUp PgDn", TEKST_SLABY, 10.5);
                    etykieta(ui, "strona", TEKST_SLABY.gamma_multiply(0.75), 10.5);
                    ui.add_space(10.0);
                    etykieta(ui, "Home End", TEKST_SLABY, 10.5);
                    etykieta(
                        ui,
                        "poczatek / koniec",
                        TEKST_SLABY.gamma_multiply(0.75),
                        10.5,
                    );
                    ui.add_space(10.0);
                    etykieta(ui, "kółko", TEKST_SLABY, 10.5);
                    etykieta(ui, "też działa", TEKST_SLABY.gamma_multiply(0.75), 10.5);
                });
            });

        // ---------- poczekalnia, przymocowana na dole ----------
        egui::Panel::bottom(egui::Id::new("poczekalnia"))
            .frame(
                egui::Frame::default()
                    .fill(TLO)
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(ui, |ui| self.panel_poczekalni(ui, teraz));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(TLO)
                    .inner_margin(egui::Margin::symmetric(12, 10)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // Delta z klawiszy — patrz komentarz przy odczycie wyzej.
                        // Znak jest odwrocony, bo `scroll_with_delta` przesuwa
                        // ZAWARTOSC, a strzalka w dol ma przesunac WIDOK w dol.
                        if self.przewin != 0.0 {
                            ui.scroll_with_delta(egui::vec2(0.0, self.przewin));
                        }
                        let zadania = self.zadania.clone();
                        if zadania.is_empty() && self.zakonczone.is_empty() {
                            ui.add_space(28.0);
                            let auto = self.automatyczne;
                            ui.vertical_centered(|ui| {
                                etykieta(ui, "nic się nie liczy", TEKST_SLABY, 13.5);
                                ui.add_space(6.0);
                                etykieta(
                                    ui,
                                    if auto {
                                        "okno zamknie się samo za chwilę"
                                    } else {
                                        "czekam na backtest albo trening — uruchom go w terminalu"
                                    },
                                    TEKST_SLABY.gamma_multiply(0.75),
                                    11.5,
                                );
                                if !auto {
                                    ui.add_space(3.0);
                                    etykieta(
                                        ui,
                                        "zadanie podchwycę sam, w ciągu pół sekundy",
                                        TEKST_SLABY.gamma_multiply(0.55),
                                        11.0,
                                    );
                                }
                            });
                        }
                        for z in &zadania {
                            self.karta(ui, z, teraz);
                            ui.add_space(9.0);
                        }
                        let zak: Vec<usize> = (0..self.zakonczone.len()).collect();
                        for i in zak {
                            Apka::karta_zakonczonego(ui, &self.zakonczone[i]);
                            ui.add_space(9.0);
                        }
                    });
            });

        // Odświeżamy dwa razy częściej niż zadanie zapisuje stan — pasek płynie,
        // a okno bezczynne kosztuje ułamek procenta rdzenia.
        // Panel, w którym człowiek właśnie klikał, przejmuje strzałki — jeszcze
        // w tej klatce, żeby klawisz zadziałał od razu po kliknięciu myszą.
        if let Some(id) = self
            .przeglad
            .iter()
            .find(|(_, p)| p.wlasnie_dotkniety)
            .map(|(id, _)| id.clone())
        {
            self.aktywny_przeglad = Some(id);
        }
        for p in self.przeglad.values_mut() {
            p.wlasnie_dotkniety = false;
        }

        ctx.request_repaint_after(std::time::Duration::from_millis(120));
    }
}

/// An explicit environment override wins. Otherwise a portable text file next
/// to postep.exe selects the same directory when launched from Explorer/Sky,
/// which do not inherit a research runner's process-local environment.
fn portable_monitor_dir(value: &str) -> Result<PathBuf, &'static str> {
    let value = value.trim_start_matches('\u{feff}').trim();
    if value.is_empty() || value.len() > 4096 || value.contains(['\n', '\r', '\0']) {
        return Err("postep-dir.txt must contain one non-empty absolute directory");
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err("postep-dir.txt requires an absolute path");
    }
    Ok(path)
}

fn main() -> eframe::Result<()> {
    if std::env::var_os("CONDUIT_POSTEP_DIR").is_none() {
        if let Ok(exe) = std::env::current_exe() {
            let file = exe.with_file_name("postep-dir.txt");
            if file.exists() {
                let directory = std::fs::read_to_string(&file)
                    .map_err(|_| "cannot read postep-dir.txt")
                    .and_then(|value| portable_monitor_dir(&value));
                match directory {
                    Ok(path) if path.is_dir() => {
                        std::env::set_var("CONDUIT_POSTEP_DIR", path);
                    }
                    _ => {
                        // Do not silently show unrelated, stale AppData jobs.
                        eprintln!("Invalid portable monitor directory: {}", file.display());
                        return Ok(());
                    }
                }
            }
        }
    }
    // Druga instancja nie ma czego pokazywać — pierwsza pokazuje to samo.
    // Zamek jest wyłącznym uchwytem pliku, więc zwalnia się nawet po ubiciu
    // procesu; nie ma stanu „okno nie wstanie, bo poprzednie padło".
    let Some(zamek) = m::zajmij_zamek() else {
        return Ok(());
    };
    let automatyczne = std::env::args().any(|x| x == "--auto");

    // Okno uruchomione SAMO (przez backtest albo trening) jest od razu widoczne,
    // ale nie kradnie ognia. Wymuszona minimalizacja sprawiała, że monitor był
    // technicznie uruchomiony, lecz człowiek nie widział ani jego treści, ani —
    // zależnie od pulpitu, z którego go sprawdzał — nawet głównego uchwytu.
    //
    // Uruchomione RĘCZNIE (podwójne kliknięcie `postep.exe`) zachowuje się
    // odwrotnie — skoro ktoś je otworzył, to chce je widzieć.
    let mut widok = egui::ViewportBuilder::default()
        .with_inner_size([520.0, 640.0])
        .with_min_inner_size([380.0, 220.0])
        .with_title("CONDUIT — postęp");
    widok = if automatyczne {
        // `with_active(false)` = pokaż, ale nie kradnij ognia.
        widok.with_active(false)
    } else {
        widok.with_always_on_top()
    };

    let opcje = eframe::NativeOptions {
        viewport: widok,
        ..Default::default()
    };
    eframe::run_native(
        "conduit-postep",
        opcje,
        Box::new(move |cc| Ok(Box::new(Apka::nowa(cc, zamek, automatyczne)))),
    )
}

// ============================================================
//  TESTY (reguły układu — rysowania nie da się sprawdzić testem)
// ============================================================

#[cfg(test)]
mod testy {
    use super::*;

    #[test]
    fn portable_monitor_directory_is_explicit_and_single_line() {
        let path = std::env::current_dir().unwrap();
        let text = path.to_string_lossy();
        assert_eq!(portable_monitor_dir(&text).unwrap(), path);
        assert_eq!(portable_monitor_dir(&format!("\u{feff}{text}\r\n")).unwrap(), path);
        for bad in ["", "relative/path", "a\nb", "a\0b"] {
            assert!(portable_monitor_dir(bad).is_err(), "{bad:?}");
        }
    }

    fn przyklad() -> m::zbiorczy::Zbiorczy {
        m::zbiorczy::Zbiorczy {
            zadan: 2,
            postep: Some(0.427),
            zrobione: 14_200_000.0,
            calosc: 54_700_000.0,
            jednostka: "ticków".into(),
            szybkosci: vec![("ticków/s".into(), 12_400_000.0)],
            eta_s: 8_040.0,
            ..Default::default()
        }
    }

    fn tekst(cz: &[Czesc]) -> String {
        cz.iter()
            .map(|c| match c {
                Czesc::Tekst(t, _, _) => t.clone(),
                Czesc::Nitka(_) => "[nitka]".to_string(),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Drabinka ustępstw: procent zostaje ZAWSZE, potem dochodzi czas, potem
    /// nitka, na końcu prędkość. Kolejność jest treścią decyzji projektowej —
    /// gdyby ktoś ją odwrócił, plakietka przestałaby odpowiadać na pytanie
    /// „ile jeszcze" pierwsza w kolejce do ucięcia byłaby właśnie odpowiedź.
    #[test]
    fn drabinka_ustepstw_zachowuje_wazne_liczby() {
        let z = przyklad();
        assert_eq!(
            tekst(&czesci_plakietki("BT", &z, Stopien::Procent)),
            "BT 43 %"
        );
        assert_eq!(
            tekst(&czesci_plakietki("BT", &z, Stopien::Czas)),
            "BT 43 % · 2h14"
        );
        assert_eq!(
            tekst(&czesci_plakietki("BT", &z, Stopien::Pasek)),
            "BT [nitka] 43 % · 2h14"
        );
        assert_eq!(
            tekst(&czesci_plakietki("BT", &z, Stopien::Pelny)),
            "BT [nitka] 43 % · 2h14 · 12,4 mln ticków/s"
        );
    }

    /// Znaki ostrzegawcze muszą być w plakietce, a nie tylko w podpowiedzi:
    /// „~" przy średniej z różnych jednostek, „≥" przy dolnej granicy czasu.
    #[test]
    fn plakietka_nie_ukrywa_zastrzezen() {
        let z = m::zbiorczy::Zbiorczy {
            zadan: 2,
            postep: Some(0.25),
            srednia_z_ulamkow: true,
            eta_s: 117.0,
            eta_dolna_granica: true,
            ..Default::default()
        };
        let t = tekst(&czesci_plakietki("BT", &z, Stopien::Czas));
        assert!(t.contains("~25 %"), "średnia musi być oznaczona: {t}");
        assert!(t.contains("≥1m57"), "dolna granica musi być oznaczona: {t}");
    }

    /// Brak skali to „bez skali", a nie zero procent, i nie ma wtedy nitki —
    /// pasek narysowany na zero udawałby pomiar, którego nie ma.
    #[test]
    fn brak_skali_nie_udaje_zera() {
        let z = m::zbiorczy::Zbiorczy {
            zadan: 1,
            postep: None,
            eta_s: -1.0,
            ..Default::default()
        };
        let t = tekst(&czesci_plakietki("BT", &z, Stopien::Pelny));
        assert_eq!(t, "BT bez skali · —");
        assert!(!t.contains("nitka"));
    }

    #[test]
    fn zwiezly_czas_jest_jednoznaczny() {
        assert_eq!(czas_zwiezly(47.0), "47s");
        assert_eq!(czas_zwiezly(117.0), "1m57");
        assert_eq!(czas_zwiezly(8_040.0), "2h14");
        assert_eq!(czas_zwiezly(-1.0), "—");
        // sekundy i minuty dopełniane zerem, żeby liczba nie zmieniała
        // szerokości w kółko i nie rozpychała plakietki
        assert_eq!(czas_zwiezly(61.0), "1m01");
        assert_eq!(czas_zwiezly(3_601.0), "1h00");
    }

    /// Skracanie po ZNAKACH — polecenia mają w sobie polskie nazwy katalogów,
    /// a cięcie po bajtach wywala program na pierwszym „ó".
    #[test]
    fn skracanie_nie_tnie_w_srodku_znaku() {
        assert_eq!(skroc("krótko", 10), "krótko");
        assert_eq!(skroc("ćśążźćśążź", 5), "ćśąż…");
        assert_eq!(skroc("abcdef", 6), "abcdef");
    }
}
