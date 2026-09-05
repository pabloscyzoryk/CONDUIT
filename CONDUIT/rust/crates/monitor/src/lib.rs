//! Protokół okienka postępu (`postep.exe`).
//!
//! # Dlaczego pliki, a nie gniazdo/potok
//!
//! Zadanie (sweep backtestów, trening AI) i okno postępu to dwa niezależne
//! procesy o zupełnie różnym czasie życia. Każde rozwiązanie z połączeniem —
//! gniazdo TCP, nazwany potok, kanał w pamięci — wymaga, żeby ktoś to
//! połączenie utrzymywał, i psuje się, gdy druga strona padnie w złym
//! momencie. Tutaj nie ma połączenia: **zadanie pisze plik, okno go czyta**.
//!
//! Konsekwencje tej decyzji, wszystkie pożądane:
//!  * okno można zamknąć i otworzyć w środku ośmiogodzinnego treningu — liczby
//!    wrócą przy pierwszym odczycie katalogu;
//!  * zadanie, które padło albo zostało ubite, zostawia NIEŚWIEŻY plik. Nie ma
//!    stanu „zawieszone na zawsze": plik starszy niż [`SWIEZOSC_MS`] jest
//!    martwy i okno mówi to wprost, zamiast udawać, że coś się liczy;
//!  * kilka zadań naraz to po prostu kilka plików w katalogu.
//!
//! # Protokół
//!
//! Katalog: `%LOCALAPPDATA%\CONDUIT\postep\` (nadpisywalny zmienną środowiskową
//! `CONDUIT_POSTEP_DIR` — używają jej testy i uruchomienia równoległe).
//!
//! | plik                | kto pisze | znaczenie                                        |
//! |---------------------|-----------|--------------------------------------------------|
//! | `<id>.json`         | zadanie   | stan zadania, zapis atomowy co ~250 ms           |
//! | `<id>.stop`         | okno      | prośba o przerwanie z zapisem                    |
//! | `zamek.lock`        | okno      | pojedyncza instancja okna (wyłączny uchwyt)      |
//!
//! Zapis stanu jest **atomowy**: najpierw `<id>.json.tmp`, potem zmiana nazwy.
//! Czytający nigdy nie zobaczy połowy pliku — zobaczy albo poprzedni stan, albo
//! nowy. Dzięki temu okno nie musi w ogóle obsługiwać uszkodzonego JSON-a jako
//! sytuacji normalnej (obsługuje ją mimo to, ale jako awarię: pomija plik).
//!
//! # Przerwanie
//!
//! Okno tworzy `<id>.stop`. Zadanie sprawdza istnienie tego pliku w swojej
//! pętli postępu i kończy się **z zapisem tego, co zdążyło policzyć**:
//!  * backtest — przez `ProgressFn` zwracające `false`
//!    (`conduit_backtest::runner::run_with_progress`);
//!  * trening — przez `TrainCfg.cancel`, sprawdzane PO zapisie punktu
//!    kontrolnego, więc przerwanie kosztuje najwyżej jedno pokolenie.
//!
//! Nie ma tu drugiego mechanizmu przerywania — obie ścieżki istniały wcześniej,
//! ten moduł tylko podłącza do nich plik.

// Poczekalnia i zestawienie zbiorcze są tu, a nie w binarce okna, z tego
// samego powodu, co reszta protokołu: to są reguły, które muszą mieć testy.
// Okno ma je tylko rysować.
pub mod badanie;
pub mod kolejka;
pub mod language;
/// Przeglądarka presetów już policzonych w trwającym przemiataniu.
pub mod przesiane;
pub mod zbiorczy;

use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

// ============================================================
//  STAŁE PROTOKOŁU
// ============================================================

/// Plik stanu starszy niż tyle milisekund oznacza ZADANIE MARTWE.
///
/// Zadanie zapisuje stan co ~250 ms, więc 10 s to czterdziestokrotny zapas.
/// Nawet całkowicie zapchana maszyna nie wygeneruje fałszywego alarmu, a
/// prawdziwa awaria (panika, ubicie procesu, zawieszenie) jest widoczna po
/// kilku sekundach zamiast nigdy.
pub const SWIEZOSC_MS: i64 = 10_000;

/// Co ile milisekund zadanie zapisuje stan.
pub const OKRES_ZAPISU_MS: i64 = 250;

/// Po tylu milisekundach bez ŻADNEGO żywego zadania okno zamyka się samo,
/// żeby nie zaśmiecać pulpitu.
pub const BEZCZYNNOSC_MS: i64 = 15_000;

/// Rodzaj zadania — tylko do rozróżnienia w interfejsie.
pub const BACKTEST: &str = "backtest";
/// Rodzaj zadania — trening modelu AI.
pub const TRENING: &str = "trening";

// ============================================================
//  CZAS I ŚCIEŻKI
// ============================================================

/// Teraz, w milisekundach od epoki.
pub fn teraz_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Katalog plików stanu.
///
/// `%LOCALAPPDATA%\CONDUIT\postep\`, bo backtesty uruchamiane są z różnych
/// katalogów roboczych, a okno musi je znaleźć wszystkie w jednym miejscu.
/// `CONDUIT_POSTEP_DIR` nadpisuje — testy i równoległe eksperymenty potrzebują
/// własnej piaskownicy.
pub fn katalog() -> PathBuf {
    if let Some(x) = std::env::var_os("CONDUIT_POSTEP_DIR") {
        return PathBuf::from(x);
    }
    match std::env::var_os("LOCALAPPDATA") {
        Some(x) => PathBuf::from(x).join("CONDUIT").join("postep"),
        // Linux/awaryjnie: katalog obok pliku wykonywalnego byłby często
        // tylko do odczytu, więc bierzemy katalog tymczasowy systemu.
        None => std::env::temp_dir().join("conduit-postep"),
    }
}

/// Ścieżka pliku stanu zadania.
pub fn sciezka_stanu(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// Ścieżka pliku „przerwij".
pub fn sciezka_stop(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.stop"))
}

/// Ścieżka przekazywana osobnemu procesowi okna.
///
/// `canonicalize` na Windows zwraca ścieżki urządzeniowe (`\\?\C:\...` albo
/// `\\?\UNC\serwer\udział\...`). Poprzedni kod usuwał tylko środkowe
/// `\?\`, zostawiając odpowiednio `\C:\...`; Windows rozwija to do
/// `C:\C:\...`, więc przeglądarka gotowych presetów zawsze patrzyła w
/// nieistniejący katalog. Prefiks wolno zdjąć wyłącznie z początku i trzeba
/// osobno odtworzyć zwykłą postać ścieżki UNC.
fn sciezka_dla_okna(txt: &str) -> String {
    if let Some(reszta) = txt.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{reszta}")
    } else if let Some(reszta) = txt.strip_prefix(r"\\?\") {
        reszta.to_string()
    } else {
        txt.to_string()
    }
}

// ============================================================
//  STATYSTYKI (mapa z ZACHOWANĄ kolejnością)
// ============================================================

/// Tabelka statystyk: pary klucz → wartość, w kolejności podanej przez zadanie.
///
/// Świadomie NIE jest to `BTreeMap`: „pokolenie", „ocena najlepszego",
/// „mediana" mają sens czytane w tej kolejności, a alfabetycznie układają się w
/// przypadkowy bałagan. W JSON-ie to zwykły obiekt — kolejność bierze się stąd,
/// że deserializacja idzie przez `MapAccess`, czyli po kolei po dokumencie, a
/// nie przez pośrednią mapę haszującą.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Statystyki(pub Vec<(String, String)>);

impl Statystyki {
    pub fn nowe() -> Self {
        Statystyki(Vec::new())
    }
    /// Dopisuje wiersz. Wartość formatujemy u źródła — okno niczego nie liczy.
    pub fn dodaj(&mut self, klucz: impl Into<String>, wartosc: impl Into<String>) {
        self.0.push((klucz.into(), wartosc.into()));
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = &(String, String)> {
        self.0.iter()
    }
}

impl From<Vec<(String, String)>> for Statystyki {
    fn from(v: Vec<(String, String)>) -> Self {
        Statystyki(v)
    }
}

impl Serialize for Statystyki {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            m.serialize_entry(k, v)?;
        }
        m.end()
    }
}

impl<'de> Deserialize<'de> for Statystyki {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct W;
        impl<'de> Visitor<'de> for W {
            type Value = Statystyki;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("obiekt JSON z parami klucz→wartość")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> Result<Statystyki, A::Error> {
                let mut v = Vec::with_capacity(m.size_hint().unwrap_or(8));
                // Wartości przyjmujemy jako dowolny JSON i sprowadzamy do
                // tekstu: dzięki temu zadanie może wpisać liczbę, a okno i tak
                // pokaże ją w tabelce, zamiast wysypać się na typie.
                while let Some((k, val)) = m.next_entry::<String, serde_json::Value>()? {
                    let s = match val {
                        serde_json::Value::String(s) => s,
                        serde_json::Value::Null => String::new(),
                        inny => inny.to_string(),
                    };
                    v.push((k, s));
                }
                Ok(Statystyki(v))
            }
        }
        d.deserialize_map(W)
    }
}

// ============================================================
//  STAN ZADANIA
// ============================================================

/// Stan jednego zadania — dokładnie to, co leży w `<id>.json`.
///
/// Każde pole ma wartość domyślną (`#[serde(default)]` na całej strukturze),
/// więc plik zapisany przez STARSZĄ wersję zadania wczytuje się bez błędu.
/// Okno postępu nie może się wywracać dlatego, że ktoś przebudował backtest.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Postep {
    /// identyfikator zadania = nazwa pliku bez rozszerzenia
    pub id: String,
    /// nazwa czytelna dla człowieka, np. „sweep presets_re (73 presety)"
    pub nazwa: String,
    /// [`BACKTEST`] albo [`TRENING`]
    pub rodzaj: String,
    /// 0.0 … 1.0
    pub postep: f64,
    /// prędkość w jednostkach na sekundę, już wygładzona przez zadanie
    pub szybkosc: f64,
    /// np. „ticków/s", „osobników/s"
    pub jednostka_szybkosci: String,
    /// szacowany czas do końca w sekundach; `< 0` = nie wiadomo
    pub eta_s: f64,
    /// „przebieg 15/73 — C-chase0"
    pub co_teraz: String,
    /// Postęp POJEDYNCZEGO elementu, nie całości: jednego przebiegu sweepu,
    /// jednego pokolenia treningu. 0.0 … 1.0.
    pub postep_biezacy: f64,
    /// Podpis cienkiego paska — MUSI mówić, czego dokładnie dotyczy liczba.
    ///
    /// Pusty łańcuch = zadanie nie umie wskazać jednego elementu i okno wtedy
    /// **nie rysuje** drugiego paska. To jest celowe: przy dwudziestu czterech
    /// przebiegach liczonych naraz „postęp bieżącego" jest pojęciem umownym i
    /// lepiej nie pokazać nic, niż pokazać procent, którego nie ma czym
    /// podpisać.
    pub etykieta_biezacego: String,
    /// początek zadania, ms epoki
    pub start_ts: i64,
    /// ostatni zapis, ms epoki — po tym poznajemy zadanie martwe
    pub aktualizacja_ts: i64,
    /// liczby bezwzględne do paska: ile zrobione
    pub zrobione: f64,
    /// liczby bezwzględne do paska: ile w sumie
    pub calosc: f64,
    /// jednostka liczb bezwzględnych, np. „ticków"
    pub jednostka: String,
    /// zadanie WIDZIAŁO prośbę o przerwanie i właśnie się domyka
    pub przerywanie: bool,
    /// tabelka do pokazania w karcie
    pub statystyki: Statystyki,
    /// Katalog, do którego zadanie zapisuje wyniki (`--out` backtestu).
    ///
    /// Bez tej ścieżki okno WIE, że przemiatanie idzie, ale nie ma jak
    /// zajrzeć do tego, co już policzone: nazwa zadania nie mówi, gdzie
    /// leży `wyniki_czastkowe.json`. Puste = zadanie nie ma czego pokazać
    /// (trening) i okno wtedy nie rysuje przeglądarki presetów.
    ///
    /// `serde(default)` jest tu warunkiem zgodności wstecz: pliki stanu
    /// zapisane przez starsze binarki nie mają tego klucza, a okno musi je
    /// dalej czytać — inaczej aktualizacja monitora wygasiłaby podgląd
    /// zadań, które już biegną.
    #[serde(default)]
    pub katalog_wynikow: String,
}

impl Postep {
    /// Czy plik jest świeży, czyli zadanie faktycznie żyje.
    pub fn zywy(&self, teraz: i64) -> bool {
        teraz - self.aktualizacja_ts <= SWIEZOSC_MS
    }
    /// Ile sekund temu ostatni znak życia.
    pub fn cisza_s(&self, teraz: i64) -> f64 {
        (teraz - self.aktualizacja_ts).max(0) as f64 / 1000.0
    }
    /// Czas od startu w sekundach.
    pub fn trwa_s(&self, teraz: i64) -> f64 {
        (teraz - self.start_ts).max(0) as f64 / 1000.0
    }
}

// ============================================================
//  ZAPIS I ODCZYT
// ============================================================

/// Zapis atomowy: plik tymczasowy + zmiana nazwy.
///
/// `fs::rename` na Windows woła `MoveFileEx` z `MOVEFILE_REPLACE_EXISTING`,
/// czyli podmiana jest niepodzielna na poziomie systemu plików. Czytający widzi
/// zawsze KOMPLETNY dokument — poprzedni albo nowy.
pub fn zapisz_atomowo(dir: &Path, p: &Postep) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let cel = sciezka_stanu(dir, &p.id);
    let tmp = dir.join(format!("{}.json.tmp", p.id));
    let txt = serde_json::to_string(p).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, txt)?;
    match std::fs::rename(&tmp, &cel) {
        Ok(()) => Ok(()),
        Err(e) => {
            // nie zostawiamy śmiecia, gdy podmiana się nie uda
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Wczytuje stany wszystkich zadań z katalogu.
///
/// Pliki nieczytelne albo uszkodzone są POMIJANE, nie zgłaszane jako błąd:
/// jedno zepsute zadanie nie może wygasić okna dla pozostałych.
pub fn wczytaj_wszystkie(dir: &Path) -> Vec<Postep> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let Ok(txt) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(mut st) = serde_json::from_str::<Postep>(&txt) else {
            continue;
        };
        // Każde pole ma wartość domyślną, więc DOWOLNY dokument JSON wczytuje
        // się tu jako „zadanie" — cudzy plik odłożony w tym katalogu pokazywał
        // się w oknie jako bezimienne „ZADANIE PADŁO". Znacznik czasu ustawia
        // `Raport::nowy` przed pierwszym zapisem, więc jego brak znaczy tyle,
        // że to nie jest plik zadania.
        if st.aktualizacja_ts == 0 {
            continue;
        }
        if st.id.is_empty() {
            // id zawsze bierzemy z nazwy pliku, gdy w środku go brakuje
            st.id = p
                .file_stem()
                .and_then(|x| x.to_str())
                .unwrap_or("?")
                .to_string();
        }
        out.push(st);
    }
    out.sort_by(|a, b| a.start_ts.cmp(&b.start_ts).then_with(|| a.id.cmp(&b.id)));
    out
}

/// Prosi zadanie o przerwanie (tworzy `<id>.stop`).
pub fn popros_o_stop(dir: &Path, id: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(sciezka_stop(dir, id), b"stop")
}

/// Czy leży prośba o przerwanie dla tego zadania.
pub fn czy_stop(dir: &Path, id: &str) -> bool {
    sciezka_stop(dir, id).exists()
}

/// Kasuje ślady zadania: stan i ewentualną prośbę o stop.
pub fn sprzataj(dir: &Path, id: &str) {
    let _ = std::fs::remove_file(sciezka_stanu(dir, id));
    let _ = std::fs::remove_file(sciezka_stop(dir, id));
    let _ = std::fs::remove_file(dir.join(format!("{id}.json.tmp")));
}

// ============================================================
//  POJEDYNCZA INSTANCJA OKNA I JEGO URUCHAMIANIE
// ============================================================

fn sciezka_zamka() -> PathBuf {
    katalog().join("zamek.lock")
}

/// Uchwyt wyłącznego zamka. Dopóki żyje, żaden inny proces nie otworzy pliku.
pub struct Zamek(#[allow(dead_code)] std::fs::File);

/// Próbuje zająć zamek okna.
///
/// Na Windows plik otwieramy z `share_mode(0)`, czyli BEZ prawa współdzielenia:
/// dopóki trzymamy uchwyt, każde inne otwarcie kończy się błędem. To jest
/// gwarancja systemu plików, a nie umowa oparta na znacznikach czasu — nie ma
/// tu okna wyścigu ani „nieświeżego PID-u po awarii", bo system zwalnia uchwyt
/// przy śmierci procesu, jakakolwiek by ona nie była.
#[cfg(windows)]
pub fn zajmij_zamek() -> Option<Zamek> {
    use std::os::windows::fs::OpenOptionsExt;
    let p = sciezka_zamka();
    let _ = std::fs::create_dir_all(p.parent()?);
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(&p)
        .ok()
        .map(Zamek)
}

/// Wariant awaryjny dla systemów bez `share_mode` — zamek doradczy po czasie
/// modyfikacji. Projekt jest windowsowy, to jest wyłącznie po to, żeby crate
/// dał się zbudować i przetestować gdzie indziej.
#[cfg(not(windows))]
pub fn zajmij_zamek() -> Option<Zamek> {
    let p = sciezka_zamka();
    let _ = std::fs::create_dir_all(p.parent()?);
    if let Ok(m) = std::fs::metadata(&p) {
        if let Ok(t) = m.modified() {
            if t.elapsed()
                .map(|d| d.as_millis() as i64)
                .unwrap_or(i64::MAX)
                < SWIEZOSC_MS
            {
                return None;
            }
        }
    }
    std::fs::write(&p, b"lock").ok()?;
    std::fs::File::open(&p).ok().map(Zamek)
}

/// Czy okno postępu już działa.
///
/// Sprawdzamy przez PRÓBĘ zajęcia zamka: jeśli się uda, to znaczy, że nikt go
/// nie trzymał — czyli okna nie ma. Uchwyt natychmiast zwalniamy.
pub fn okno_dziala() -> bool {
    match zajmij_zamek() {
        Some(z) => {
            drop(z);
            false
        }
        None => true,
    }
}

/// Znajduje `postep.exe`.
///
/// Kolejność: obok bieżącej binarki (tak jest po zbudowaniu — `bt.exe` i
/// `postep.exe` leżą w `target/release`), potem katalog roboczy,
/// potem `target/release` względem katalogu roboczego.
pub fn znajdz_okno() -> Option<PathBuf> {
    let nazwa = if cfg!(windows) {
        "postep.exe"
    } else {
        "postep"
    };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            let p = d.join(nazwa);
            if p.exists() {
                return Some(p);
            }
        }
    }
    for k in ["", "target/release", "target/debug"] {
        let p = if k.is_empty() {
            PathBuf::from(nazwa)
        } else {
            Path::new(k).join(nazwa)
        };
        if p.exists() {
            return Some(p);
        }
    }

    // KATALOGI `target-*` — okno przestało się pokazywać, bo ich nie znaliśmy.
    //
    // Odkąd równolegle pracuje kilku agentów, każdy buduje do WŁASNEGO
    // `--target-dir` (`target-h2`, `target-relot`, `target-l2`…), żeby sobie
    // nawzajem nie unieważniać kompilacji. `postep.exe` jest budowany razem
    // z resztą tylko wtedy, gdy ktoś zbuduje CAŁY workspace — przy
    // `cargo build --bin btp` go tam nie ma, więc okno milkło bez słowa.
    //
    // Bierzemy najświeższy plik, bo starszy mógłby pochodzić z niekompatybilnej
    // wersji formatu raportu.
    let mut najlepszy: Option<(std::time::SystemTime, PathBuf)> = None;
    if let Ok(wpisy) = std::fs::read_dir(".") {
        for w in wpisy.flatten() {
            let d = w.path();
            if !d.is_dir() {
                continue;
            }
            let Some(n) = d.file_name().and_then(|x| x.to_str()) else {
                continue;
            };
            if !n.starts_with("target") {
                continue;
            }
            for profil in ["release", "debug"] {
                let p = d.join(profil).join(nazwa);
                let Ok(meta) = std::fs::metadata(&p) else {
                    continue;
                };
                let Ok(czas) = meta.modified() else { continue };
                if najlepszy.as_ref().is_none_or(|(t, _)| czas > *t) {
                    najlepszy = Some((czas, p));
                }
            }
        }
    }
    najlepszy.map(|(_, p)| p)
}

/// Uruchamia okno postępu, jeśli jeszcze nie działa.
///
/// Proces potomny odczepiamy od konsoli rodzica (`DETACHED_PROCESS`) i od jego
/// grupy (`CREATE_NEW_PROCESS_GROUP`). Bez tego Ctrl+C w terminalu backtestu
/// zabijałby okno razem z zadaniem — a okno ma przeżyć zadanie, żeby pokazać,
/// że zadanie padło.
///
/// `CONDUIT_BEZ_OKNA=1` wyłącza całą mechanikę (skrypty wsadowe, CI).
pub fn uruchom_okno_jesli_trzeba() {
    uruchom_okno_z_jezykiem(None);
}

/// Optional presentation language from a caller's already loaded settings.
/// The monitor does not read account settings or secrets to discover it.
pub fn uruchom_okno_z_jezykiem(jezyk: Option<&str>) {
    if std::env::var_os("CONDUIT_BEZ_OKNA").is_some() {
        return;
    }
    if okno_dziala() {
        return;
    }
    let Some(exe) = znajdz_okno() else {
        return;
    };
    let mut c = std::process::Command::new(exe);
    if let Some(language) = jezyk.and_then(language::Language::parse) {
        c.env("CONDUIT_LANGUAGE", language.code());
    }
    // `--auto` mówi oknu, że podniosło się SAMO. Okno uruchomione ręcznie
    // (dwuklik w `LAB\postep.exe`) czeka na zadania i nie zamyka się z nudów —
    // skoro człowiek je otworzył, to znaczy, że chce patrzeć.
    c.arg("--auto");
    c.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        c.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let _ = c.spawn();
}

// ============================================================
//  STRONA ZADANIA
// ============================================================

static LICZNIK_ZADAN: AtomicU64 = AtomicU64::new(0);

pub struct Raport {
    dir: PathBuf,
    stan: Postep,
    ostatni_zapis: Instant,
    /// (chwila, zrobione) z poprzedniej próbki — do liczenia prędkości
    poprzednia_probka: (Instant, f64),
    /// wygładzona prędkość (średnia wykładnicza)
    szybkosc_wygl: f64,
    /// czy zauważyliśmy już plik stop
    stop: bool,
}

/// Współczynnik wygładzania prędkości.
///
/// Surowa prędkość liczona z 250-milisekundowej próbki skacze o kilkadziesiąt
/// procent (harmonogram wątków, cache dysku) i jest nieczytelna. 0,25 przy
/// próbce 250 ms daje stałą czasową ~1 s: liczba stoi spokojnie, a na realną
/// zmianę tempa reaguje w sekundę.
const ALFA: f64 = 0.25;

impl Raport {
    /// Zakłada zadanie i — jeśli trzeba — podnosi okno.
    pub fn nowy(nazwa: impl Into<String>, rodzaj: &str) -> Raport {
        let dir = katalog();
        let _ = std::fs::create_dir_all(&dir);
        let n = LICZNIK_ZADAN.fetch_add(1, Ordering::Relaxed);
        let id = format!("{rodzaj}-{}-{n}", std::process::id());
        // stary plik stop z poprzedniego uruchomienia o tym samym id
        // natychmiast przerwałby świeże zadanie — kasujemy na wejściu
        sprzataj(&dir, &id);
        uruchom_okno_jesli_trzeba();
        let t = teraz_ms();
        let stan = Postep {
            id,
            nazwa: nazwa.into(),
            rodzaj: rodzaj.to_string(),
            eta_s: -1.0,
            start_ts: t,
            aktualizacja_ts: t,
            ..Default::default()
        };
        let r = Raport {
            dir,
            stan,
            ostatni_zapis: Instant::now(),
            poprzednia_probka: (Instant::now(), 0.0),
            szybkosc_wygl: 0.0,
            stop: false,
        };
        let _ = zapisz_atomowo(&r.dir, &r.stan);
        r
    }

    pub fn id(&self) -> &str {
        &self.stan.id
    }

    /// Ustala skalę paska: ile jednostek w sumie i jak się nazywają.
    pub fn calosc(&mut self, calosc: f64, jednostka: &str, jednostka_szybkosci: &str) {
        self.stan.calosc = calosc;
        self.stan.jednostka = jednostka.to_string();
        self.stan.jednostka_szybkosci = jednostka_szybkosci.to_string();
    }

    /// Mówi oknu, gdzie szukać wyników już policzonych (patrz
    /// [`Postep::katalog_wynikow`]). Ścieżkę zapisujemy BEZWZGLĘDNĄ, bo okno
    /// jest osobnym procesem i prawie nigdy nie stoi w tym samym katalogu
    /// roboczym co zadanie — ścieżka względna trafiłaby w próżnię.
    pub fn katalog_wynikow(&mut self, p: &Path) {
        let bezwzgledna = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        self.stan.katalog_wynikow = sciezka_dla_okna(&bezwzgledna.to_string_lossy());
    }

    /// Postęp POJEDYNCZEGO elementu do cienkiego paska (patrz
    /// [`Postep::postep_biezacy`]).
    ///
    /// Osobna metoda, a nie kolejny argument [`Raport::postep`], bo nie każde
    /// zadanie ma sensowny „element bieżący", a dokładanie argumentu, który
    /// w połowie wywołań byłby atrapą, zamieniłoby brak informacji w zero
    /// procent — czyli w kłamstwo.
    ///
    /// Podpis PUSTY wygasza cienki pasek.
    pub fn biezacy(&mut self, ulamek: f64, podpis: impl Into<String>) {
        self.stan.postep_biezacy = ulamek.clamp(0.0, 1.0);
        self.stan.etykieta_biezacego = podpis.into();
    }

    /// Ostatnio policzony czas do końca w sekundach; `< 0` = jeszcze nie wiadomo.
    pub fn eta_s(&self) -> f64 {
        self.stan.eta_s
    }

    /// Zgłasza postęp. Zwraca `false`, gdy poproszono o przerwanie.
    ///
    /// Wolno wołać bardzo często — zapis na dysk i sprawdzenie pliku stop są
    /// dławione do [`OKRES_ZAPISU_MS`].
    pub fn postep(
        &mut self,
        zrobione: f64,
        co_teraz: impl Into<String>,
        staty: Statystyki,
    ) -> bool {
        let teraz = Instant::now();
        let dt = teraz.duration_since(self.ostatni_zapis).as_millis() as i64;
        if dt < OKRES_ZAPISU_MS {
            return !self.stop;
        }
        self.ostatni_zapis = teraz;

        // --- prędkość (średnia wykładnicza) ---
        let odstep = teraz.duration_since(self.poprzednia_probka.0).as_secs_f64();
        if odstep > 0.01 {
            let chwilowa = ((zrobione - self.poprzednia_probka.1) / odstep).max(0.0);
            self.szybkosc_wygl = if self.szybkosc_wygl <= 0.0 {
                chwilowa
            } else {
                ALFA * chwilowa + (1.0 - ALFA) * self.szybkosc_wygl
            };
            self.poprzednia_probka = (teraz, zrobione);
        }

        self.stan.zrobione = zrobione;
        self.stan.postep = if self.stan.calosc > 0.0 {
            (zrobione / self.stan.calosc).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.stan.szybkosc = self.szybkosc_wygl;
        self.stan.eta_s = if self.szybkosc_wygl > 1e-9 && self.stan.calosc > 0.0 {
            ((self.stan.calosc - zrobione).max(0.0) / self.szybkosc_wygl).min(30.0 * 86_400.0)
        } else {
            -1.0
        };
        self.stan.co_teraz = co_teraz.into();
        self.stan.statystyki = staty;
        self.stan.aktualizacja_ts = teraz_ms();

        // --- prośba o przerwanie ---
        if !self.stop && czy_stop(&self.dir, &self.stan.id) {
            self.stop = true;
            self.stan.przerywanie = true;
        }

        let _ = zapisz_atomowo(&self.dir, &self.stan);
        !self.stop
    }

    /// Czy poproszono o przerwanie (bez aktualizacji liczb).
    pub fn przerwano(&self) -> bool {
        self.stop
    }

    /// Ostatni komunikat i skasowanie plików zadania.
    ///
    /// Wołane po zapisaniu wyników — od tej chwili okno wie, że zadania nie ma,
    /// i po [`BEZCZYNNOSC_MS`] bez innych zadań zamknie się samo.
    pub fn zakoncz(&mut self) {
        sprzataj(&self.dir, &self.stan.id);
    }
}

impl Drop for Raport {
    fn drop(&mut self) {
        // Nawet przy panice zadania nie zostawiamy pliku, który udaje żywy.
        // (Przy `panic = "abort"` destruktor się nie wykona — wtedy ratuje nas
        // wykrywanie nieświeżego pliku po stronie okna.)
        self.zakoncz();
    }
}

// ============================================================
//  FORMATOWANIE (wspólne dla zadań i okna)
// ============================================================

/// Liczba po polsku: przecinek dziesiętny, spacja co trzy cyfry.
pub fn pl_liczba(x: f64, miejsca: usize) -> String {
    let ujemna = x < 0.0;
    let s = format!("{:.*}", miejsca, x.abs());
    let (calk, ulamek) = match s.split_once('.') {
        Some((a, b)) => (a.to_string(), Some(b.to_string())),
        None => (s, None),
    };
    let mut wynik = String::new();
    for (i, c) in calk.chars().enumerate() {
        if i > 0 && (calk.len() - i) % 3 == 0 {
            wynik.push('\u{202f}'); // wąska spacja nierozdzielająca
        }
        wynik.push(c);
    }
    if let Some(u) = ulamek {
        wynik.push(',');
        wynik.push_str(&u);
    }
    if ujemna {
        format!("-{wynik}")
    } else {
        wynik
    }
}

/// Duża liczba w skrócie: 54 700 000 → „54,7 mln".
pub fn pl_duza(x: f64) -> String {
    let a = x.abs();
    if a >= 1e9 {
        format!("{} mld", pl_liczba(x / 1e9, 2))
    } else if a >= 1e6 {
        format!("{} mln", pl_liczba(x / 1e6, 1))
    } else if a >= 1e4 {
        format!("{} tys.", pl_liczba(x / 1e3, 1))
    } else {
        pl_liczba(x, 0)
    }
}

/// Czas trwania: „2 h 14 min", „8 min 03 s", „47 s".
pub fn pl_czas(sekundy: f64) -> String {
    if !sekundy.is_finite() || sekundy < 0.0 {
        return "—".into();
    }
    let s = sekundy.round() as i64;
    if s >= 3600 {
        format!("{} h {:02} min", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{} min {:02} s", s / 60, s % 60)
    } else {
        format!("{s} s")
    }
}

// ============================================================
//  TESTY
// ============================================================

#[cfg(test)]
mod testy {
    use super::*;

    #[test]
    fn prefiks_windows_jest_zdejmowany_bez_okaleczenia_sciezki() {
        assert_eq!(sciezka_dla_okna(r"\\?\C:\dane\wyniki"), r"C:\dane\wyniki");
        assert_eq!(
            sciezka_dla_okna(r"\\?\UNC\serwer\udzial\wyniki"),
            r"\\serwer\udzial\wyniki"
        );
        assert_eq!(sciezka_dla_okna(r"C:\dane\wyniki"), r"C:\dane\wyniki");
        assert_eq!(sciezka_dla_okna("/tmp/wyniki"), "/tmp/wyniki");
    }

    fn piaskownica(nazwa: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("conduit-postep-test-{nazwa}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Plik stanu musi przechodzić serializację w obie strony BEZ STRAT,
    /// łącznie z KOLEJNOŚCIĄ statystyk — tabelka czytana alfabetycznie
    /// przestaje mieć sens („mediana" nad „pokoleniem").
    #[test]
    fn stan_przechodzi_w_obie_strony() {
        let mut st = Statystyki::nowe();
        st.dodaj("pokolenie", "7 / 30");
        st.dodaj("ocena najlepszego", "+0,2211");
        st.dodaj("ocena środka", "+0,1234");
        let p = Postep {
            id: "backtest-123-0".into(),
            nazwa: "sweep presets_re".into(),
            rodzaj: BACKTEST.into(),
            postep: 0.6242,
            szybkosc: 4_820_000.0,
            jednostka_szybkosci: "ticków/s".into(),
            eta_s: 8.5,
            co_teraz: "przebieg 15/73 — C-chase0".into(),
            postep_biezacy: 0.8421,
            etykieta_biezacego: "C-chase0 · najdalej z 24 naraz".into(),
            start_ts: 1_700_000_000_000,
            aktualizacja_ts: 1_700_000_012_345,
            zrobione: 14_200_000.0,
            calosc: 54_700_000.0,
            jednostka: "ticków".into(),
            przerywanie: false,
            statystyki: st,
            katalog_wynikow: String::new(),
        };
        let txt = serde_json::to_string(&p).unwrap();
        let z: Postep = serde_json::from_str(&txt).unwrap();
        assert_eq!(p, z, "stan musi wrócić identyczny");
        assert_eq!(
            z.statystyki.0[0].0, "pokolenie",
            "kolejność statystyk to część protokołu, nie szczegół zapisu"
        );
        assert_eq!(z.statystyki.0[2].0, "ocena środka");
        // i przez dysk, atomowo
        let d = piaskownica("obie-strony");
        zapisz_atomowo(&d, &p).unwrap();
        let v = wczytaj_wszystkie(&d);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0], p);
        assert!(
            !d.join("backtest-123-0.json.tmp").exists(),
            "plik tymczasowy musi zniknąć"
        );
    }

    /// Stan zapisany przez STARSZĄ wersję zadania (bez nowych pól) musi się
    /// wczytać. Inaczej każda zmiana protokołu wygaszałaby okno.
    #[test]
    fn niepelny_plik_wczytuje_sie() {
        let json = r#"{"id":"x","nazwa":"stare","rodzaj":"backtest","postep":0.5,
                       "aktualizacja_ts":123}"#;
        let p: Postep = serde_json::from_str(json).unwrap();
        assert_eq!(p.postep, 0.5);
        assert_eq!(p.calosc, 0.0);
        assert!(p.statystyki.is_empty());
        // brak cienkiego paska w starym pliku = PUSTY podpis, czyli okno go nie
        // narysuje. Zero procent bez podpisu wyglądałoby jak zawieszony przebieg.
        assert!(p.etykieta_biezacego.is_empty());
        assert_eq!(p.postep_biezacy, 0.0);
        // wartości liczbowe w statystykach też są dopuszczalne
        let json2 = r#"{"id":"y","statystyki":{"a":1.5,"b":"tekst","c":null}}"#;
        let p2: Postep = serde_json::from_str(json2).unwrap();
        assert_eq!(
            p2.statystyki.0,
            vec![
                ("a".to_string(), "1.5".to_string()),
                ("b".to_string(), "tekst".to_string()),
                ("c".to_string(), String::new()),
            ]
        );
    }

    /// Zadanie, które padło, zostawia nieświeży plik. To MUSI być widoczne —
    /// pasek stojący w miejscu wygląda identycznie jak wolne liczenie.
    #[test]
    fn nieswiezy_plik_to_zadanie_martwe() {
        let teraz = teraz_ms();
        let zywy = Postep {
            aktualizacja_ts: teraz - 1_000,
            ..Default::default()
        };
        let martwy = Postep {
            aktualizacja_ts: teraz - SWIEZOSC_MS - 1,
            ..Default::default()
        };
        assert!(zywy.zywy(teraz));
        assert!(
            !martwy.zywy(teraz),
            "plik starszy niż {SWIEZOSC_MS} ms = zadanie padło"
        );
        assert!(martwy.cisza_s(teraz) > 10.0);
        // granica dokładnie na progu liczy się jeszcze jako żywa
        let na_progu = Postep {
            aktualizacja_ts: teraz - SWIEZOSC_MS,
            ..Default::default()
        };
        assert!(na_progu.zywy(teraz));
    }

    /// Pełna pętla przerwania: okno pisze `.stop`, zadanie to widzi przy
    /// najbliższym zgłoszeniu postępu i zwraca `false`.
    #[test]
    fn plik_stop_przerywa_zadanie() {
        let d = piaskownica("stop");
        std::env::set_var("CONDUIT_POSTEP_DIR", &d);
        std::env::set_var("CONDUIT_BEZ_OKNA", "1");

        let mut r = Raport::nowy("test", BACKTEST);
        r.calosc(1000.0, "ticków", "ticków/s");
        let id = r.id().to_string();
        assert!(
            sciezka_stanu(&d, &id).exists(),
            "stan pojawia się od razu po starcie"
        );

        // dławienie: pierwsze wywołanie tuż po starcie nie zapisuje, ale też
        // nie kłamie o przerwaniu
        assert!(r.postep(10.0, "start", Statystyki::nowe()));

        std::thread::sleep(std::time::Duration::from_millis(
            OKRES_ZAPISU_MS as u64 + 60,
        ));
        assert!(
            r.postep(100.0, "10 %", Statystyki::nowe()),
            "bez pliku stop pracujemy dalej"
        );

        popros_o_stop(&d, &id).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(
            OKRES_ZAPISU_MS as u64 + 60,
        ));
        assert!(
            !r.postep(200.0, "20 %", Statystyki::nowe()),
            "po pojawieniu się <id>.stop zadanie MUSI dostać sygnał przerwania"
        );
        assert!(r.przerwano());

        // stan na dysku niesie znacznik „domykam się"
        let v = wczytaj_wszystkie(&d);
        assert_eq!(v.len(), 1);
        assert!(v[0].przerywanie);
        assert!(v[0].postep > 0.0);

        r.zakoncz();
        assert!(
            wczytaj_wszystkie(&d).is_empty(),
            "po zakończeniu katalog jest pusty"
        );
        assert!(!sciezka_stop(&d, &id).exists(), "prośba o stop też znika");

        std::env::remove_var("CONDUIT_POSTEP_DIR");
        std::env::remove_var("CONDUIT_BEZ_OKNA");
    }

    /// Cienki pasek jest ustawiany OSOBNO od głównego i musi dotrzeć na dysk
    /// przy najbliższym zapisie — inaczej okno rysowałoby postęp elementu
    /// z poprzedniej sekundy obok aktualnego postępu całości.
    #[test]
    fn cienki_pasek_dociera_na_dysk() {
        let d = piaskownica("cienki");
        std::env::set_var("CONDUIT_POSTEP_DIR", &d);
        std::env::set_var("CONDUIT_BEZ_OKNA", "1");

        let mut r = Raport::nowy("test", BACKTEST);
        r.calosc(1000.0, "ticków", "ticków/s");
        r.biezacy(0.37, "EU016 · najdalej z 22 naraz");
        std::thread::sleep(std::time::Duration::from_millis(
            OKRES_ZAPISU_MS as u64 + 60,
        ));
        r.postep(100.0, "gotowe 3/22", Statystyki::nowe());

        let v = wczytaj_wszystkie(&d);
        assert_eq!(v.len(), 1);
        assert!((v[0].postep_biezacy - 0.37).abs() < 1e-9);
        assert_eq!(v[0].etykieta_biezacego, "EU016 · najdalej z 22 naraz");
        // postęp CAŁOŚCI i postęp ELEMENTU to dwie różne liczby i nie wolno ich
        // mylić: tu całość jest na 10 %, a bieżący przebieg na 37 %.
        assert!((v[0].postep - 0.10).abs() < 1e-9);

        // ułamek poza zakresem obcinamy u źródła — okno nie ma niczego naprawiać
        r.biezacy(1.7, "x");
        assert!(r.stan.postep_biezacy <= 1.0);

        r.zakoncz();
        std::env::remove_var("CONDUIT_POSTEP_DIR");
        std::env::remove_var("CONDUIT_BEZ_OKNA");
    }

    /// Kilka zadań naraz to po prostu kilka plików — nic więcej nie trzeba.
    #[test]
    fn wiele_zadan_obok_siebie() {
        let d = piaskownica("wiele");
        for (i, r) in [BACKTEST, TRENING, BACKTEST].iter().enumerate() {
            let p = Postep {
                id: format!("{r}-{i}"),
                rodzaj: r.to_string(),
                start_ts: 1000 - i as i64, // celowo odwrotnie
                aktualizacja_ts: teraz_ms(),
                ..Default::default()
            };
            zapisz_atomowo(&d, &p).unwrap();
        }
        let v = wczytaj_wszystkie(&d);
        assert_eq!(v.len(), 3);
        // kolejność stabilna: najstarsze zadanie na górze
        assert!(v[0].start_ts <= v[1].start_ts && v[1].start_ts <= v[2].start_ts);
    }

    #[test]
    fn formatowanie_po_polsku() {
        assert_eq!(pl_liczba(1234.5, 1), "1\u{202f}234,5");
        assert_eq!(pl_liczba(-7.25, 2), "-7,25");
        assert_eq!(pl_liczba(999.0, 0), "999");
        assert_eq!(pl_duza(54_700_000.0), "54,7 mln");
        assert_eq!(pl_duza(1_500_000_000.0), "1,50 mld");
        assert_eq!(pl_czas(47.0), "47 s");
        assert_eq!(pl_czas(483.0), "8 min 03 s");
        assert_eq!(pl_czas(8_040.0), "2 h 14 min");
        assert_eq!(pl_czas(-1.0), "—");
    }
}
