//! POCZEKALNIA — kolejka backtestów czekających na uruchomienie.
//!
//! # Po co to jest
//!
//! Sweep zajmuje maszynę na kilkadziesiąt minut. Człowiek, który wie, co chce
//! policzyć jako drugie i trzecie, dziś musi albo siedzieć przy komputerze
//! i czekać na znak zachęty, albo puścić wszystko naraz i pozwolić, żeby
//! zadania odbierały sobie rdzenie. Poczekalnia to trzecia możliwość: wpisujesz
//! polecenia teraz, ruszają po kolei, kiedy maszyna będzie wolna.
//!
//! # Czym to NIE jest
//!
//! To nie jest harmonogram ani „pseudo-uruchamianie". Wpis w poczekalni to
//! DOKŁADNIE ten wiersz polecenia, który poszedłby do terminala; okno startuje
//! go przez [`uruchom`] jako zwykły proces potomny i śledzi jego kod wyjścia.
//! Stan „gotowe" pojawia się dlatego, że proces skończył się z zerem, a nie
//! dlatego, że minął jakiś czas. Kiedy program nie da się uruchomić, wpis
//! dostaje stan `Padło` z treścią błędu systemu, a nie ciche „ok".
//!
//! # Dlaczego plik obok plików postępu
//!
//! Ta sama zasada, co w całym module: **stan mieszka na dysku, nie w pamięci
//! okna**. Kolejkę można ułożyć, zamknąć okno, otworzyć je za godzinę i dalej
//! tam będzie. Zapis jest atomowy (tymczasowy plik + zmiana nazwy), więc
//! zamknięcie okna w trakcie zapisu nie zostawia połówki dokumentu.
//!
//! # Wyjście z procesu
//!
//! Wyjście uruchomionego zadania idzie do `poczekalnia/<id>.log`. Bez tego
//! nieudany przebieg mówiłby tylko „kod 101" i trzeba by go powtarzać ręcznie
//! w terminalu, żeby w ogóle zobaczyć komunikat.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::path::{Path, PathBuf};

/// Podkatalog poczekalni w katalogu postępu: kolejka i logi uruchomionych zadań.
///
/// PODKATALOG, a nie plik obok stanów zadań — i to nie jest kwestia porządku.
/// [`crate::wczytaj_wszystkie`] czyta KAŻDY plik `*.json` z katalogu postępu,
/// więc `poczekalnia.json` położona obok wczytywała się jako zadanie
/// (wszystkie pola mają wartości domyślne) i okno rysowało widmo z podpisem
/// „ZADANIE PADŁO". Zobaczone na własne oczy przy pierwszym uruchomieniu.
pub const KATALOG: &str = "poczekalnia";

/// Nazwa pliku kolejki wewnątrz [`KATALOG`].
pub const PLIK: &str = "kolejka.json";

// ============================================================
//  STAN WPISU
// ============================================================

/// Co się dzieje z pozycją kolejki.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stan {
    /// czeka na swoją kolej
    #[default]
    Czeka,
    /// proces wystartował i jeszcze się nie skończył
    Liczy,
    /// proces skończył się kodem 0
    Gotowe,
    /// proces skończył się błędem albo w ogóle nie dał się uruchomić
    Padlo,
    /// okno zostało zamknięte, gdy zadanie liczyło — nie wiemy, jak skończyło
    Nieznane,
}

impl Stan {
    pub fn tekst(self) -> &'static str {
        match self {
            Stan::Czeka => "czeka",
            Stan::Liczy => "liczy",
            Stan::Gotowe => "gotowe",
            Stan::Padlo => "padlo",
            Stan::Nieznane => "nieznane",
        }
    }
    /// Nieznana nazwa stanu daje [`Stan::Nieznane`], a nie błąd wczytania.
    ///
    /// Kolejka jest własnością UŻYTKOWNIKA — nowsza albo starsza wersja okna
    /// nie ma prawa skasować mu ułożonej listy tylko dlatego, że nie zna
    /// jednego słowa.
    pub fn z_tekstu(s: &str) -> Stan {
        match s {
            "czeka" => Stan::Czeka,
            "liczy" => Stan::Liczy,
            "gotowe" => Stan::Gotowe,
            "padlo" => Stan::Padlo,
            _ => Stan::Nieznane,
        }
    }
    /// Czy pozycja jest już zamknięta i wolno ją usunąć bez pytania.
    pub fn skonczony(self) -> bool {
        matches!(self, Stan::Gotowe | Stan::Padlo | Stan::Nieznane)
    }
}

impl Serialize for Stan {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.tekst())
    }
}

impl<'de> Deserialize<'de> for Stan {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(Stan::z_tekstu(&s))
    }
}

// ============================================================
//  WPIS I KOLEJKA
// ============================================================

/// Jedna pozycja poczekalni.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Wpis {
    pub id: String,
    /// nazwa dla człowieka; pusta = pokazujemy samo polecenie
    pub nazwa: String,
    /// PEŁNY wiersz polecenia, dokładnie taki, jaki poszedłby do terminala
    pub polecenie: String,
    /// katalog roboczy; pusty = ten sam, co okna
    pub katalog: String,
    pub stan: Stan,
    pub dodane_ts: i64,
    pub start_ts: i64,
    pub koniec_ts: i64,
    /// kod wyjścia procesu, gdy już się skończył
    pub kod: Option<i32>,
    /// identyfikator procesu — po nim wiążemy wpis z kartą postępu
    pub pid: u32,
    /// wyjaśnienie stanu: treść błędu, ostatnia linia logu, powód „nieznanego"
    pub uwaga: String,
}

impl Wpis {
    /// Podpis pozycji: nazwa, a gdy jej nie ma — samo polecenie.
    pub fn podpis(&self) -> &str {
        if self.nazwa.trim().is_empty() {
            self.polecenie.trim()
        } else {
            self.nazwa.trim()
        }
    }
    /// Ile trwało (albo trwa) w sekundach; `< 0` = jeszcze nie ruszyło.
    pub fn trwa_s(&self, teraz: i64) -> f64 {
        if self.start_ts <= 0 {
            return -1.0;
        }
        let koniec = if self.koniec_ts > 0 {
            self.koniec_ts
        } else {
            teraz
        };
        (koniec - self.start_ts).max(0) as f64 / 1000.0
    }
}

/// Cała poczekalnia — to, co leży w `poczekalnia.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Kolejka {
    pub wpisy: Vec<Wpis>,
    /// Czy okno ma samo startować kolejne pozycje.
    ///
    /// Domyślnie TAK — kolejka, która nie rusza sama, byłaby notatnikiem.
    /// Wyłącznik jest po to, żeby dało się ułożyć listę bez natychmiastowego
    /// zajęcia maszyny.
    pub auto: bool,
    /// licznik do nadawania identyfikatorów, żeby nie powtórzyły się po restarcie
    pub licznik: u64,
}

impl Default for Kolejka {
    fn default() -> Self {
        Kolejka {
            wpisy: Vec::new(),
            auto: true,
            licznik: 0,
        }
    }
}

impl Kolejka {
    /// Katalog poczekalni wewnątrz katalogu postępu.
    pub fn katalog(dir: &Path) -> PathBuf {
        dir.join(KATALOG)
    }

    pub fn sciezka(dir: &Path) -> PathBuf {
        Kolejka::katalog(dir).join(PLIK)
    }

    /// Wczytuje kolejkę. Brak pliku = pusta kolejka.
    ///
    /// Plik USZKODZONY nie znika po cichu: odkładamy go obok pod nazwą
    /// `poczekalnia.uszkodzona.json`. Kolejka bywa efektem kwadransa układania
    /// poleceń i skasowanie jej bez śladu byłoby najgorszą możliwą reakcją na
    /// literówkę w JSON-ie.
    pub fn wczytaj(dir: &Path) -> Kolejka {
        let p = Kolejka::sciezka(dir);
        let Ok(txt) = std::fs::read_to_string(&p) else {
            return Kolejka::default();
        };
        match serde_json::from_str::<Kolejka>(&txt) {
            Ok(k) => k,
            Err(_) => {
                let _ = std::fs::rename(&p, Kolejka::katalog(dir).join("kolejka.uszkodzona.json"));
                Kolejka::default()
            }
        }
    }

    /// Zapis atomowy — jak stan zadania.
    pub fn zapisz(&self, dir: &Path) -> std::io::Result<()> {
        let kat = Kolejka::katalog(dir);
        std::fs::create_dir_all(&kat)?;
        let cel = Kolejka::sciezka(dir);
        let tmp = kat.join("kolejka.json.tmp");
        let txt = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, txt)?;
        match std::fs::rename(&tmp, &cel) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }

    /// Dokłada pozycję na KONIEC kolejki. Zwraca jej identyfikator.
    ///
    /// Puste polecenie odrzucamy tutaj, a nie przy uruchomieniu — pozycja,
    /// która na pewno nie ma czego odpalić, nie ma po co czekać w kolejce.
    pub fn dodaj(
        &mut self,
        nazwa: &str,
        polecenie: &str,
        katalog: &str,
        teraz: i64,
    ) -> Result<String, String> {
        let polecenie = polecenie.trim();
        if polecenie.is_empty() {
            return Err("puste polecenie".into());
        }
        if podziel_polecenie(polecenie).is_empty() {
            return Err("nie umiem odczytać programu z tego polecenia".into());
        }
        self.licznik += 1;
        let id = format!("p{}-{}", teraz, self.licznik);
        self.wpisy.push(Wpis {
            id: id.clone(),
            nazwa: nazwa.trim().to_string(),
            polecenie: polecenie.to_string(),
            katalog: katalog.trim().to_string(),
            stan: Stan::Czeka,
            dodane_ts: teraz,
            ..Default::default()
        });
        Ok(id)
    }

    pub fn znajdz(&self, id: &str) -> Option<&Wpis> {
        self.wpisy.iter().find(|w| w.id == id)
    }

    pub fn znajdz_mut(&mut self, id: &str) -> Option<&mut Wpis> {
        self.wpisy.iter_mut().find(|w| w.id == id)
    }

    /// Wyrzuca pozycję z kolejki. Pozycji, która LICZY SIĘ, nie ruszamy —
    /// usunięcie wpisu nie zatrzymałoby procesu, a okno straciłoby jedyny ślad
    /// po tym, co samo uruchomiło.
    pub fn usun(&mut self, id: &str) -> bool {
        let Some(i) = self.wpisy.iter().position(|w| w.id == id) else {
            return false;
        };
        if self.wpisy[i].stan == Stan::Liczy {
            return false;
        }
        self.wpisy.remove(i);
        true
    }

    /// Kasuje wszystkie pozycje domknięte (gotowe, padłe, nieznane).
    pub fn sprzataj_skonczone(&mut self) -> usize {
        let przed = self.wpisy.len();
        self.wpisy.retain(|w| !w.stan.skonczony());
        przed - self.wpisy.len()
    }

    /// Przesuwa pozycję o jedno miejsce. Zwraca `false`, gdy nie ma dokąd.
    pub fn przesun(&mut self, id: &str, w_gore: bool) -> bool {
        let Some(i) = self.wpisy.iter().position(|w| w.id == id) else {
            return false;
        };
        let j = if w_gore {
            if i == 0 {
                return false;
            }
            i - 1
        } else {
            if i + 1 >= self.wpisy.len() {
                return false;
            }
            i + 1
        };
        self.wpisy.swap(i, j);
        true
    }

    /// Indeks pozycji, którą wolno TERAZ uruchomić.
    ///
    /// `None`, gdy kolejka jest wyłączona, gdy coś już z niej liczy albo gdy
    /// nie ma nic czekającego. Jednoczesne uruchomienie dwóch sweepów odbiera
    /// im nawzajem rdzenie — kolejka istnieje właśnie po to, żeby tego nie
    /// robić.
    pub fn nastepny_do_startu(&self) -> Option<usize> {
        if !self.auto {
            return None;
        }
        if self.wpisy.iter().any(|w| w.stan == Stan::Liczy) {
            return None;
        }
        self.wpisy.iter().position(|w| w.stan == Stan::Czeka)
    }

    pub fn czekajace(&self) -> usize {
        self.wpisy.iter().filter(|w| w.stan == Stan::Czeka).count()
    }

    pub fn liczy_sie(&self) -> Option<&Wpis> {
        self.wpisy.iter().find(|w| w.stan == Stan::Liczy)
    }

    /// Po wczytaniu: pozycja zostawiona w stanie `Liczy` to sierota po oknie,
    /// które zamknięto w trakcie pracy.
    ///
    /// NIE wznawiamy jej samoczynnie. Backtest mógł dojść do końca i zapisać
    /// wyniki — powtórzenie go zajęłoby maszynę na kwadranse i nadpisało plik,
    /// który być może jest już dobry. Człowiek widzi wprost, że nie wiemy, i
    /// decyduje sam.
    pub fn oznacz_sieroty(&mut self) -> usize {
        let mut n = 0;
        for w in self.wpisy.iter_mut().filter(|w| w.stan == Stan::Liczy) {
            w.stan = Stan::Nieznane;
            w.uwaga = "okno zamknięto w trakcie — nie wiem, czy to zadanie doszło do końca".into();
            n += 1;
        }
        n
    }
}

// ============================================================
//  WIERSZ POLECENIA
// ============================================================

/// Dzieli wiersz polecenia na program i argumenty.
///
/// Cudzysłowy trzymają razem ścieżki ze spacjami (`"C:\Program Files\…"`).
/// Ukośnika wstecznego CELOWO nie traktujemy jako znaku ucieczki — w ścieżkach
/// Windows jest ich pełno i „escapowanie" zamieniałoby `C:\rust\target` w
/// `C: ust arget`.
pub fn podziel_polecenie(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut biezacy = String::new();
    let mut w_cudzyslowie = false;
    let mut cokolwiek = false;
    for c in s.chars() {
        match c {
            '"' => {
                w_cudzyslowie = !w_cudzyslowie;
                // pusty argument w cudzysłowie ("") też jest argumentem
                cokolwiek = true;
            }
            c if c.is_whitespace() && !w_cudzyslowie => {
                if cokolwiek {
                    out.push(std::mem::take(&mut biezacy));
                    cokolwiek = false;
                }
            }
            c => {
                biezacy.push(c);
                cokolwiek = true;
            }
        }
    }
    if cokolwiek {
        out.push(biezacy);
    }
    out
}

/// Uruchamia pozycję kolejki jako proces potomny.
///
/// Wyjście (oba strumienie) idzie do `<dir>/poczekalnia/<id>.log`. Na Windows
/// dokładamy `CREATE_NO_WINDOW`: `postep.exe` jest programem okienkowym, więc
/// każdy potomek konsolowy otwierałby czarne okno — a użytkownik trzyma to
/// okienko właśnie po to, żeby NIE mieć terminala na wierzchu.
pub fn uruchom(dir: &Path, w: &Wpis) -> std::io::Result<(std::process::Child, PathBuf)> {
    let czesci = podziel_polecenie(&w.polecenie);
    if czesci.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "puste polecenie",
        ));
    }
    let logi = Kolejka::katalog(dir);
    std::fs::create_dir_all(&logi)?;
    let plik_logu = logi.join(format!("{}.log", w.id));
    let f = std::fs::File::create(&plik_logu)?;
    let f2 = f.try_clone()?;

    let mut c = std::process::Command::new(&czesci[0]);
    c.args(&czesci[1..]);
    if !w.katalog.trim().is_empty() {
        c.current_dir(w.katalog.trim());
    }
    c.stdin(std::process::Stdio::null());
    c.stdout(std::process::Stdio::from(f));
    c.stderr(std::process::Stdio::from(f2));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    let dziecko = c.spawn()?;
    Ok((dziecko, plik_logu))
}

/// Ostatnie niepuste linie logu — do pokazania przy pozycji, która padła.
///
/// Bez tego „kod 101" trzeba by powtarzać w terminalu, żeby w ogóle zobaczyć,
/// co się stało.
pub fn ogon_logu(plik: &Path, ile: usize) -> String {
    let Ok(txt) = std::fs::read_to_string(plik) else {
        return String::new();
    };
    let linie: Vec<&str> = txt
        .lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect();
    let od = linie.len().saturating_sub(ile);
    linie[od..].join(" | ")
}

/// Szuka `btp.exe` — binarki backtestu, którą poczekalnia uruchamia najczęściej.
///
/// Zwraca `Err` z listą SPRAWDZONYCH ścieżek, a nie samo „nie znalazłem":
/// katalogów `target-*` jest w tym repozytorium kilkanaście i bez tej listy nie
/// da się zgadnąć, czego okno szukało.
pub fn znajdz_btp() -> Result<PathBuf, Vec<PathBuf>> {
    let nazwa = if cfg!(windows) { "btp.exe" } else { "btp" };
    let mut sprawdzone: Vec<PathBuf> = Vec::new();
    let mut korzenie: Vec<PathBuf> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            // obok binarki (tak jest po zwykłym `cargo build`)
            korzenie.push(d.to_path_buf());
            // `LAB\postep.exe` → `..\rust`
            if let Some(g) = d.parent() {
                korzenie.push(g.join("rust"));
                korzenie.push(g.to_path_buf());
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        korzenie.push(cwd.clone());
        korzenie.push(cwd.join("rust"));
    }

    for k in &korzenie {
        let wprost = k.join(nazwa);
        sprawdzone.push(wprost.clone());
        if wprost.is_file() {
            return Ok(wprost);
        }
        // `target/release` i wszystkie równoległe `target-*/release` — inni
        // agenci budują do własnych katalogów i tam leży najświeższa binarka
        let Ok(rd) = std::fs::read_dir(k) else {
            continue;
        };
        let mut cele: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && p.file_name()
                        .and_then(|x| x.to_str())
                        .is_some_and(|n| n == "target" || n.starts_with("target-"))
            })
            .collect();
        cele.sort();
        for t in cele {
            let p = t.join("release").join(nazwa);
            sprawdzone.push(p.clone());
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    Err(sprawdzone)
}

// ============================================================
//  TESTY
// ============================================================

#[cfg(test)]
mod testy {
    use super::*;

    fn piaskownica(nazwa: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("conduit-kolejka-test-{nazwa}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn brak_pliku_to_pusta_kolejka_ktora_rusza_sama() {
        let d = piaskownica("pusta");
        let k = Kolejka::wczytaj(&d);
        assert!(k.wpisy.is_empty());
        assert!(k.auto, "kolejka, która nie rusza sama, byłaby notatnikiem");
        assert_eq!(k.nastepny_do_startu(), None);
    }

    /// Kolejność pozycji JEST treścią kolejki — musi przetrwać zapis i odczyt
    /// co do miejsca, razem ze stanami.
    #[test]
    fn zapis_i_odczyt_zachowuje_kolejnosc_i_stany() {
        let d = piaskownica("zapis");
        let mut k = Kolejka::default();
        k.dodaj("pierwszy", "btp.exe --a", "C:\\rust", 1_000)
            .unwrap();
        let b = k.dodaj("", "btp.exe --b", "", 1_001).unwrap();
        k.dodaj("trzeci", "btp.exe --c", "", 1_002).unwrap();
        k.znajdz_mut(&b).unwrap().stan = Stan::Gotowe;
        k.znajdz_mut(&b).unwrap().kod = Some(0);
        k.zapisz(&d).unwrap();

        let z = Kolejka::wczytaj(&d);
        assert_eq!(z, k);
        assert_eq!(z.wpisy[1].stan, Stan::Gotowe);
        assert_eq!(z.wpisy[2].nazwa, "trzeci");
        // wpis bez nazwy pokazuje polecenie, a nie pustą linię
        assert_eq!(z.wpisy[1].podpis(), "btp.exe --b");
    }

    /// Nieznany stan z innej wersji okna NIE MOŻE skasować kolejki.
    #[test]
    fn nieznany_stan_nie_kasuje_kolejki() {
        let json = r#"{"wpisy":[{"id":"x","polecenie":"a","stan":"cos-nowego"}],"auto":false}"#;
        let k: Kolejka = serde_json::from_str(json).unwrap();
        assert_eq!(k.wpisy.len(), 1);
        assert_eq!(k.wpisy[0].stan, Stan::Nieznane);
        assert!(!k.auto);
    }

    /// Uszkodzony plik odkładamy obok zamiast kasować.
    #[test]
    fn uszkodzony_plik_nie_ginie() {
        let d = piaskownica("uszkodzony");
        std::fs::create_dir_all(Kolejka::katalog(&d)).unwrap();
        std::fs::write(Kolejka::sciezka(&d), b"{to nie jest json").unwrap();
        let k = Kolejka::wczytaj(&d);
        assert!(k.wpisy.is_empty());
        assert!(
            Kolejka::katalog(&d)
                .join("kolejka.uszkodzona.json")
                .is_file(),
            "kwadrans układania poleceń nie może zniknąć bez śladu"
        );
    }

    #[test]
    fn kolejka_nie_udaje_zadania() {
        let d = piaskownica("nie-zadanie");
        let mut k = Kolejka::default();
        k.dodaj("a", "cokolwiek", "", 1).unwrap();
        k.zapisz(&d).unwrap();
        assert!(
            crate::wczytaj_wszystkie(&d).is_empty(),
            "poczekalnia nie jest zadaniem i nie ma prawa pojawić się jako karta"
        );
        assert_eq!(Kolejka::wczytaj(&d).wpisy.len(), 1);
    }

    #[test]
    fn przesuwanie_i_usuwanie() {
        let mut k = Kolejka::default();
        let a = k.dodaj("a", "x", "", 1).unwrap();
        let b = k.dodaj("b", "y", "", 2).unwrap();
        assert!(k.przesun(&b, true));
        assert_eq!(k.wpisy[0].id, b);
        assert!(!k.przesun(&b, true), "z góry nie ma dokąd");
        assert!(k.usun(&a));
        assert_eq!(k.wpisy.len(), 1);

        // pozycji liczącej się nie wyrzucamy — usunięcie wpisu nie zatrzymuje
        // procesu, a okno straciłoby po nim jedyny ślad
        k.znajdz_mut(&b).unwrap().stan = Stan::Liczy;
        assert!(!k.usun(&b));
        assert_eq!(k.wpisy.len(), 1);
    }

    #[test]
    fn puste_polecenie_nie_wchodzi_do_kolejki() {
        let mut k = Kolejka::default();
        assert!(k.dodaj("a", "   ", "", 1).is_err());
        assert!(k.wpisy.is_empty());
    }

    /// Druga pozycja rusza DOPIERO, gdy pierwsza przestanie się liczyć.
    #[test]
    fn nastepny_dopiero_gdy_nic_z_kolejki_nie_liczy() {
        let mut k = Kolejka::default();
        let a = k.dodaj("a", "x", "", 1).unwrap();
        k.dodaj("b", "y", "", 2).unwrap();
        assert_eq!(k.nastepny_do_startu(), Some(0));

        k.znajdz_mut(&a).unwrap().stan = Stan::Liczy;
        assert_eq!(k.nastepny_do_startu(), None, "jeden sweep naraz");

        k.znajdz_mut(&a).unwrap().stan = Stan::Gotowe;
        assert_eq!(k.nastepny_do_startu(), Some(1));

        k.auto = false;
        assert_eq!(k.nastepny_do_startu(), None, "wyłącznik ma działać");
    }

    /// Sierota po zamkniętym oknie nie rusza sama drugi raz.
    #[test]
    fn sierota_nie_wznawia_sie_sama() {
        let mut k = Kolejka::default();
        let a = k.dodaj("a", "x", "", 1).unwrap();
        k.znajdz_mut(&a).unwrap().stan = Stan::Liczy;
        assert_eq!(k.oznacz_sieroty(), 1);
        assert_eq!(k.wpisy[0].stan, Stan::Nieznane);
        assert!(
            !k.wpisy[0].uwaga.is_empty(),
            "człowiek ma wiedzieć, czemu nie wiemy"
        );
        assert_eq!(k.nastepny_do_startu(), None);
    }

    #[test]
    fn podzial_polecenia_z_cudzyslowami() {
        assert_eq!(
            podziel_polecenie("btp.exe --a 1"),
            vec!["btp.exe", "--a", "1"]
        );
        assert_eq!(
            podziel_polecenie("\"C:\\Program Files\\btp.exe\" --out out_x"),
            vec!["C:\\Program Files\\btp.exe", "--out", "out_x"]
        );
        // ukośnik wsteczny NIE jest znakiem ucieczki
        assert_eq!(
            podziel_polecenie("C:\\rust\\btp.exe"),
            vec!["C:\\rust\\btp.exe"]
        );
        assert_eq!(podziel_polecenie("   "), Vec::<String>::new());
        assert_eq!(podziel_polecenie("a \"\" b"), vec!["a", "", "b"]);
    }

    /// Sprawdzenie, że kolejka NAPRAWDĘ uruchamia proces, zbiera jego wyjście
    /// i kod. Bez tego testu cała poczekalnia mogłaby być atrapą, która tylko
    /// przestawia napisy.
    #[test]
    #[cfg(windows)]
    fn uruchom_naprawde_odpala_proces_i_zbiera_wyjscie() {
        let d = piaskownica("uruchom");
        let mut k = Kolejka::default();
        let id = k
            .dodaj("echo", "cmd /c echo POCZEKALNIA-DZIALA", "", 1)
            .unwrap();
        let w = k.znajdz(&id).unwrap().clone();

        let (mut dziecko, log) = uruchom(&d, &w).unwrap();
        let wynik = dziecko.wait().unwrap();
        assert!(wynik.success(), "proces musi się skończyć zerem");
        let tresc = std::fs::read_to_string(&log).unwrap();
        assert!(
            tresc.contains("POCZEKALNIA-DZIALA"),
            "wyjście procesu trafia do logu"
        );
        assert_eq!(ogon_logu(&log, 1), "POCZEKALNIA-DZIALA");
    }

    /// Program, którego nie ma, MUSI dać błąd — nie ciche „gotowe".
    #[test]
    fn brak_programu_to_blad_a_nie_ciche_gotowe() {
        let d = piaskownica("brak");
        let mut k = Kolejka::default();
        let id = k
            .dodaj("nic", "tego-programu-na-pewno-nie-ma --x", "", 1)
            .unwrap();
        let w = k.znajdz(&id).unwrap().clone();
        assert!(uruchom(&d, &w).is_err());
    }

    /// Kod różny od zera to `Padło`, a log niesie powód.
    #[test]
    #[cfg(windows)]
    fn kod_wyjscia_rozny_od_zera_to_padlo() {
        let d = piaskownica("kod");
        let mut k = Kolejka::default();
        let id = k
            .dodaj("exit3", "cmd /c echo pech 1>&2 & exit /b 3", "", 1)
            .unwrap();
        let w = k.znajdz(&id).unwrap().clone();
        let (mut dziecko, log) = uruchom(&d, &w).unwrap();
        let wynik = dziecko.wait().unwrap();
        assert_eq!(wynik.code(), Some(3));
        assert!(
            ogon_logu(&log, 3).contains("pech"),
            "stderr też ma iść do logu"
        );
    }
}
