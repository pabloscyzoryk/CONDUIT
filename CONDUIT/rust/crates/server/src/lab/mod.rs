//! LABORATORIUM — backtesty i trening AI uruchamiane z okna aplikacji.
//!
//! **Dlaczego to mieszka w serwerze, a nie w osobnym procesie.**
//! Okno (Tauri/WebView2) i przeglądarka są równorzędnymi klientami TEGO SAMEGO
//! serwera — cały stan jedzie już przez WebSocket, z koalescencją do ~10 Hz
//! (`coalesce.rs`). Uruchamianie osobnego `bt.exe` wymagałoby wymyślenia
//! drugiego kanału na postęp (parsowanie stdout albo plik-strumień), a potem
//! utrzymywania go równolegle z istniejącym. Zamiast tego postęp jest ZWYKŁĄ
//! SEKCJĄ STANU: pojawia się w snapshocie, płynie deltami, widzą go wszystkie
//! podłączone powłoki naraz, a nowo otwarta karta od razu dostaje aktualny
//! obraz trwającego zadania. Zero nowego transportu, zero rozjazdu.
//!
//! **Dlaczego osobny WĄTEK, a nie `spawn_blocking`.** Backtest i trening liczą
//! się przez rayona na wszystkich rdzeniach i trwają minuty. Pula blokująca
//! tokia jest współdzielona z resztą wejścia/wyjścia; zajęcie jej wątku na
//! dziesięć minut to proszenie się o zakleszczenie obsługi HTTP. Dedykowany
//! wątek systemowy nie ma tego problemu i nie potrzebuje ani środowiska
//! asynchronicznego, ani `async` w kodzie liczącym.
//!
//! Zadania są WYŁĄCZNE: naraz liczy się jedno. Dwa backtesty na 24 rdzeniach
//! nie policzą się szybciej niż jeden po drugim, a rozdzieliłyby pamięć
//! i pasek postępu na dwoje.

pub mod backtests;
pub mod rest;
pub mod training;

use crate::coalesce::{Section, Sections};
use crate::state::StateHandle;
use crate::store::Workspace;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

// ============================================================
//  STAN WIDOCZNY W INTERFEJSIE
// ============================================================

/// Sekcja `lab` snapshotu. Celowo mała: opis bieżącego zadania i krótka
/// historia. Katalog wyników, listy presetów i zakres danych czyta się
/// REST-em (`/api/lab/info`) — nie zmieniają się w trakcie liczenia, więc
/// nie ma po co wozić ich 10 razy na sekundę.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabState {
    /// czy właśnie coś się liczy
    pub busy: bool,
    /// bieżące zadanie (albo ostatnio zakończone, dopóki nie ruszy następne)
    pub job: Option<LabJob>,
    /// zakończone zadania tej sesji, najnowsze pierwsze
    pub history: Vec<LabJob>,
}

/// Ile zakończonych zadań pamiętamy w stanie.
const HISTORY_KEEP: usize = 6;

/// Ile wierszy wyniku zostawiamy w pozycji historii (czołówka rankingu).
const HISTORY_ROWS: usize = 12;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabJob {
    pub id: String,
    /// `backtest` albo `train`
    pub kind: String,
    /// jednozdaniowy opis: co i na jakim oknie
    pub title: String,
    /// `running` | `done` | `cancelled` | `failed`
    pub phase: String,
    /// CO AKTUALNIE LICZY — np. „przebieg 12/40 · preset R-sl25-p2"
    pub label: String,
    /// 0…1
    pub progress: f64,
    /// jednostki logiczne (przebiegi albo pokolenia)
    pub done: u64,
    pub total: u64,
    /// pierwsza miara szybkości (ticki/s albo pokolenia/min)
    pub speed: String,
    /// druga miara szybkości (przebiegi/min albo osobniki/s)
    pub speed2: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub elapsed_ms: i64,
    /// szacowany czas do końca; 0 = jeszcze nie wiadomo
    pub eta_ms: i64,
    /// katalog z wynikami (do przycisku „Otwórz katalog")
    pub out_dir: String,
    /// podsumowanie po zakończeniu
    pub note: String,
    pub error: Option<String>,
    /// wyniki przebiegów backtestu, narastająco
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<LabRow>,
    /// OCENA CZTEROTRYBOWA — po jednym wpisie na preset, w środku cztery tryby.
    /// Puste, gdy zadanie liczyło jeden tryb (zwykły backtest).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quads: Vec<LabQuad>,
    /// postęp fitness po pokoleniach
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gens: Vec<LabGen>,
    /// dodatkowe liczby treningu
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub train: Option<LabTrain>,
    /// nazwy plików SVG w katalogu wyników
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub charts: Vec<String>,
}

/// Jeden wiersz tabeli wyników backtestu.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabRow {
    pub name: String,
    pub profit: f64,
    pub per_day: f64,
    pub max_dd: f64,
    pub max_dd_pct: f64,
    /// maksymalne OTWARTE RYZYKO — bez tej kolumny szeroki SL udaje darmowy zysk
    pub risk: f64,
    pub risk_pct: f64,
    /// `null` w JSON, gdy nieskończony (brak strat)
    pub profit_factor: Option<f64>,
    pub win_days_pct: f64,
    pub win_rate: f64,
    pub trades: u32,
    /// Unique parsed entry sources; absent in archived results made before this telemetry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_entry_sources: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_full_entry_sources: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_sources_first_seen_as_edit: Option<u32>,
    pub max_open_positions: u32,
    pub blown: bool,
    /// ocena wg tej samej reguły co w CLI `bt.exe`
    pub score: f64,
    /// plik SVG z wykresem tego przebiegu
    pub chart: String,
    /// przebieg przerwany — wynik jest CZĄSTKOWY i nie wolno go porównywać
    pub partial: bool,
}

// ============================================================
//  OCENA CZTEROTRYBOWA (NAUKOWIEC §4)
// ============================================================

/// Poziom ruiny jako procent kapitału startowego.
///
/// NAUKOWIEC §4: „Jedyny próg bezwzględny: konto nie może dojść do zera — ani
/// do poziomu, z którego minimalny lot już nie odrobi (na 200 $ okolice
/// 30–40 $)". 20 % z 200 $ to 40 $, czyli górna krawędź tego przedziału —
/// bierzemy ostrożniejszą stronę, bo pomyłka w tę stronę kosztuje odrzucony
/// wariant, a w drugą: wyzerowane konto.
///
/// To jest JEDYNY próg, po którym wolno cokolwiek odrzucić. maxDD nim NIE JEST.
pub const PODLOGA_RUINY_PCT: f64 = 20.0;

/// Wynik JEDNEGO miesiąca. §4 wymaga miesięcy osobno, bo średnia z całości
/// ukrywa reżim: `SWEEP-A-5` miał świetną całość i zerował konto w lipcu.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabMonth {
    /// „2026-06"
    pub month: String,
    pub profit: f64,
    /// dni handlowe (z transakcjami) w tym miesiącu
    pub days: u32,
    pub loss_days: u32,
}

/// Jedna komórka tabeli 2×2 — jeden tryb oceny jednego presetu.
///
/// Zestaw liczb jest DOKŁADNIE ten z §4: zysk · % dni stratnych · najgorszy
/// dzień · najniższe equity · liczba wyzerowań · miesiące osobno · liczba
/// wypełnionych jednostek. maxDD siedzi obok jako liczba informacyjna.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabCell {
    /// klucz trybu — patrz [`TRYBY`]
    pub mode: String,
    pub profit: f64,
    /// procent dni HANDLOWYCH zakończonych stratą
    pub loss_days_pct: f64,
    pub worst_day: f64,
    /// najniższe equity na całej ścieżce (z pozycjami otwartymi)
    pub min_equity: f64,
    /// ile razy konto spadło do poziomu ruiny (patrz [`PODLOGA_RUINY_PCT`])
    pub ruins: u32,
    /// konto doszło do zera — dyskwalifikacja bezwzględna
    pub blown: bool,
    /// liczba WYPEŁNIONYCH jednostek (§3C — kontrola ekspozycji)
    pub units: u32,
    /// Unique parsed entry sources; absent in archived results made before this telemetry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_entry_sources: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_full_entry_sources: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_sources_first_seen_as_edit: Option<u32>,
    /// maxDD — INFORMACYJNIE. Nie rangujemy po nim i nie odrzucamy po nim.
    pub max_dd: f64,
    pub max_open_risk_pct: f64,
    pub end_equity: f64,
    pub trading_days: u32,
    pub months: Vec<LabMonth>,
    /// przebieg przerwany — wynik CZĄSTKOWY, nieporównywalny
    pub partial: bool,
    /// Przebieg SKOŃCZYŁ SIĘ PRZED KOŃCEM OKNA, bo konto padło.
    ///
    /// Symulator przerywa pętlę na wyzerowanym koncie. W trybie „dzień po
    /// dniu" to znaczy, że pozostałych dni NIKT NIE ZMIERZYŁ — zysk z takiej
    /// komórki dotyczy krótszego okna niż z pozostałych i nie wolno ich
    /// zestawiać bez tej informacji.
    pub cut_short: bool,
    pub chart: String,
}

/// Jeden preset oceniony we wszystkich czterech trybach.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabQuad {
    pub name: String,
    pub cells: Vec<LabCell>,
    /// Czy preset przechodzi próg bezwzględny WE WSZYSTKICH czterech trybach.
    /// To jedyne „zaliczone/niezaliczone", jakie wolno tu postawić.
    pub survives: bool,
    /// suma zysków ze wszystkich czterech trybów — wyłącznie do porządkowania
    /// tabeli, NIE jest miarą jakości
    pub sum_profit: f64,
    /// najsłabszy z czterech trybów; §4 wymaga, żeby preset działał w każdym
    pub worst_profit: f64,
}

/// Jeden punkt wykresu postępu treningu.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabGen {
    pub gen: usize,
    pub center: f64,
    pub best: f64,
    pub median: f64,
    pub worst: f64,
    pub pnl: f64,
    pub max_dd: f64,
    pub trades: u32,
    pub sigma: f64,
    pub elapsed_s: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabTrain {
    /// najlepszy oceniony środek rozkładu (to jego wagi się zapisuje)
    pub best_fitness: f64,
    pub best_gen: usize,
    /// mediana populacji w ostatnim pokoleniu
    pub median_fitness: f64,
    /// obsunięcie NAJLEPSZEGO osobnika
    pub best_dd: f64,
    pub best_pnl: f64,
    /// linia bazowa: model startowy, który nic nie robi
    pub baseline_train: f64,
    pub baseline_valid: f64,
    /// ocena końcowa na oknach ROZŁĄCZNYCH z treningiem
    pub valid_fitness: f64,
    pub valid_pnl: f64,
    /// postęp wewnątrz pokolenia
    pub eval_done: u64,
    pub eval_total: u64,
    pub model_path: String,
    pub checkpoint_path: String,
    /// czy da się wznowić (istnieje punkt kontrolny)
    pub resumable: bool,
}

// ============================================================
//  STEROWANIE
// ============================================================

/// Uchwyt trzymany w [`crate::state::Shared`]. Sam nie liczy — pilnuje tylko,
/// żeby zadania nie nachodziły na siebie i żeby było co ustawić przy „PRZERWIJ".
#[derive(Default)]
pub struct LabControl {
    busy: AtomicBool,
    cancel: parking_lot::Mutex<Option<Arc<AtomicBool>>>,
}

impl LabControl {
    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    /// Rezerwuje laboratorium. `false` = już coś liczy.
    fn try_claim(&self, cancel: Arc<AtomicBool>) -> bool {
        if self
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        *self.cancel.lock() = Some(cancel);
        true
    }

    fn release(&self) {
        *self.cancel.lock() = None;
        self.busy.store(false, Ordering::SeqCst);
    }

    /// Prosi bieżące zadanie o zatrzymanie się i zapisanie stanu.
    /// Zwraca `false`, gdy nie ma czego przerywać.
    pub fn request_cancel(&self) -> bool {
        match self.cancel.lock().as_ref() {
            Some(c) => {
                c.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }
}

// ============================================================
//  KONTEKST ZADANIA
// ============================================================

/// Jedyne miejsce, przez które postęp trafia do stanu.
///
/// Publikacja jest DŁAWIONA: silnik potrafi zgłosić postęp tysiąc razy na
/// sekundę (kilkanaście wątków rayona × co 262 tys. ticków), a interfejs
/// potrzebuje kilku odświeżeń na sekundę. Bez dławienia sam pasek postępu
/// zjadałby więcej czasu na zapisy pod `RwLock` niż liczenie.
pub struct JobCtx {
    st: StateHandle,
    pub cancel: Arc<AtomicBool>,
    pub out_dir: PathBuf,
    start: Instant,
    view: parking_lot::Mutex<LabJob>,
    last_pub: parking_lot::Mutex<Instant>,
    /// Meldunek do OSOBNEGO okienka postępu (`LAB\postep.exe`).
    ///
    /// Zadanie z panelu ma podnosić to samo okno, co `bt.exe` z wiersza
    /// poleceń — inaczej użytkownik ma dwa różne obrazy tej samej pracy
    /// zależnie od tego, skąd ją uruchomił.
    raport: parking_lot::Mutex<Option<conduit_monitor::Raport>>,
    /// skala paska w okienku: (całość, jednostka, jednostka szybkości)
    skala: parking_lot::Mutex<(f64, String, String)>,
    /// cienki pasek okienka: (ułamek, podpis). Pusty podpis = okno go nie rysuje.
    biezacy: parking_lot::Mutex<(f64, String)>,
    /// dodatkowe wiersze tabelki w okienku, w kolejności podanej przez zadanie
    staty: parking_lot::Mutex<Vec<(String, String)>>,
}

/// Odstęp między publikacjami postępu. 200 ms to ~5 odświeżeń na sekundę:
/// pasek płynie płynnie dla oka, a delty i tak koalescencjonują się do 10 Hz.
const PUBLISH_EVERY_MS: u128 = 200;

impl JobCtx {
    fn new(st: StateHandle, cancel: Arc<AtomicBool>, job: LabJob, out_dir: PathBuf) -> Self {
        // Okno podnosimy PRZED pierwszym meldunkiem: `Raport::nowy` sam wywołuje
        // `uruchom_okno_jesli_trzeba`, ale jego wyszukiwarka `postep.exe` nie
        // zna układu tego repozytorium (patrz `podnies_okno`).
        let language = st.read(|snapshot| snapshot.language.clone());
        podnies_okno(&language);
        let raport = conduit_monitor::Raport::nowy(
            job.title.clone(),
            if job.kind == "train" {
                conduit_monitor::TRENING
            } else {
                conduit_monitor::BACKTEST
            },
        );
        JobCtx {
            st,
            cancel,
            out_dir,
            start: Instant::now(),
            view: parking_lot::Mutex::new(job),
            last_pub: parking_lot::Mutex::new(Instant::now() - std::time::Duration::from_secs(60)),
            raport: parking_lot::Mutex::new(Some(raport)),
            skala: parking_lot::Mutex::new((1.0, "postępu".into(), "/s".into())),
            biezacy: parking_lot::Mutex::new((0.0, String::new())),
            staty: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Skala paska w osobnym okienku postępu. Wołane raz, gdy zadanie już wie,
    /// ile ma do przemielenia.
    pub fn skala_okna(&self, calosc: f64, jednostka: &str, jednostka_szybkosci: &str) {
        *self.skala.lock() = (
            calosc.max(1.0),
            jednostka.into(),
            jednostka_szybkosci.into(),
        );
    }

    /// Cienki pasek okienka — postęp POJEDYNCZEGO elementu. Pusty podpis
    /// wygasza pasek; tak ma być, gdy nie da się wskazać jednego elementu.
    pub fn biezacy_okna(&self, ulamek: f64, podpis: impl Into<String>) {
        *self.biezacy.lock() = (ulamek.clamp(0.0, 1.0), podpis.into());
    }

    /// Tabelka w okienku. Zastępuje poprzednią zawartość w całości.
    pub fn staty_okna(&self, v: Vec<(String, String)>) {
        *self.staty.lock() = v;
    }

    #[inline]
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Katalog roboczy — stąd biorą się ścieżki danych i wyników.
    pub fn workspace(&self) -> &Workspace {
        &self.st.workspace
    }

    /// Wpis do dziennika aplikacji z wnętrza zadania.
    pub fn log(&self, level: &str, title: impl Into<String>, content: impl Into<String>) {
        self.st.log("backtests", level, title, content);
    }

    pub fn elapsed_ms(&self) -> i64 {
        self.start.elapsed().as_millis() as i64
    }

    /// Czy minął już odstęp publikacji.
    ///
    /// Sygnalizator postępu woła się z kilkunastu wątków rayona po kilkadziesiąt
    /// razy na sekundę. Bez tego taniego sprawdzenia każde z tych wywołań
    /// składałoby napisy („12,4 mln ticków/s"), których nikt nigdy nie zobaczy.
    pub fn due(&self) -> bool {
        self.last_pub.lock().elapsed().as_millis() >= PUBLISH_EVERY_MS
    }

    /// Zmienia opis zadania i (jeśli minął odstęp albo `force`) publikuje go.
    pub fn edit(&self, force: bool, f: impl FnOnce(&mut LabJob)) {
        let snapshot = {
            let mut v = self.view.lock();
            f(&mut v);
            v.elapsed_ms = self.elapsed_ms();
            v.eta_ms = if v.progress > 0.01 && v.progress < 1.0 {
                ((v.elapsed_ms as f64) * (1.0 - v.progress) / v.progress) as i64
            } else {
                0
            };
            if !force {
                let mut last = self.last_pub.lock();
                if last.elapsed().as_millis() < PUBLISH_EVERY_MS {
                    return;
                }
                *last = Instant::now();
            } else {
                *self.last_pub.lock() = Instant::now();
            }
            v.clone()
        };
        self.publish(snapshot);
    }

    fn publish(&self, job: LabJob) {
        self.melduj_oknu(&job);
        // `update_transient`: postęp laboratorium NIE jest stanem bota, więc
        // nie ma powodu, żeby wymuszał zapis `backup_memory/` co 15 sekund
        self.st.update_transient(Sections::one(Section::Lab), |s| {
            s.lab.job = Some(job);
            s.lab.busy = true;
        });
    }

    /// Ten sam postęp, co w panelu, do osobnego okienka `postep.exe`.
    ///
    /// Okno jest DRUGIM równorzędnym klientem, nie kopią: użytkownik zwija
    /// panel, zostawia okienko na pasku zadań i widzi pracę bez trzymania
    /// otwartej przeglądarki. Przycisk PRZERWIJ w okienku wraca tutaj przez
    /// plik `<id>.stop`, który `Raport::postep` sprawdza za nas — dlatego
    /// zwrócenie `false` MUSI ustawić ten sam znacznik, co REST `/cancel`.
    fn melduj_oknu(&self, j: &LabJob) {
        let mut g = self.raport.lock();
        let Some(r) = g.as_mut() else { return };
        let (calosc, jedn, jedn_v) = self.skala.lock().clone();
        r.calosc(calosc, &jedn, &jedn_v);
        let (ul, podpis) = self.biezacy.lock().clone();
        r.biezacy(ul, podpis);
        let mut st = conduit_monitor::Statystyki::nowe();
        for (k, v) in self.staty.lock().iter() {
            st.dodaj(k.clone(), v.clone());
        }
        if !r.postep(j.progress * calosc, j.label.clone(), st) {
            // prośba przyszła z okienka — od tej chwili nie ma różnicy
            // między nią a kliknięciem PRZERWIJ w panelu
            self.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Domknięcie zadania: przenosi je do historii i zwalnia laboratorium.
    fn finish(&self, phase: &str, note: String, error: Option<String>) {
        let job = {
            let mut v = self.view.lock();
            v.phase = phase.to_string();
            v.note = note;
            v.error = error;
            v.elapsed_ms = self.elapsed_ms();
            v.eta_ms = 0;
            v.finished_at = Some(crate::now_ms());
            if phase == "done" {
                v.progress = 1.0;
            }
            v.clone()
        };
        // Do historii idzie wersja ODCHUDZONA. Pełny przemiał to nawet setka
        // wierszy i tyle samo nazw wykresów; sześć takich zadań w historii
        // rozdęłoby KAŻDY snapshot wysyłany nowo podłączonej karcie. Komplet
        // zostaje w `raport.json` w katalogu zadania i w bieżącym `job`.
        let mut skrot = job.clone();
        skrot.rows.truncate(HISTORY_ROWS);
        skrot.charts.clear();
        skrot.gens.clear();
        // Ocena czterotrybowa to cztery komórki × dwanaście miesięcy na preset —
        // w historii sześciu zadań rozdęłaby snapshot bardziej niż wiersze.
        // Komplet zostaje w `ocena4.json` i w bieżącym `job`.
        skrot.quads.clear();

        // Okienko postępu ma zniknąć razem z zadaniem, a nie wisieć jako
        // „zadanie martwe" — plik stanu kasujemy JAWNIE, nie licząc na `Drop`
        // (profil `release` ma `panic = "abort"`, więc destruktor bywa pomijany).
        if let Some(mut r) = self.raport.lock().take() {
            r.zakoncz();
        }

        self.st.update(Sections::one(Section::Lab), |s| {
            s.lab.busy = false;
            s.lab.history.insert(0, skrot);
            s.lab.history.truncate(HISTORY_KEEP);
            s.lab.job = Some(job);
        });
    }
}

// ============================================================
//  URUCHAMIANIE
// ============================================================

fn stamp_id(kind: &str) -> String {
    format!("{kind}-{}", crate::store::stamp(crate::now_ms()))
}

/// Strażnik blokady laboratorium — zwalnia ją także wtedy, gdy wątek zadania
/// zakończy się w sposób nieprzewidziany.
struct Zwolnij(StateHandle);

impl Drop for Zwolnij {
    fn drop(&mut self) {
        self.0.lab.release();
    }
}

/// Startuje zadanie w tle. Wraca NATYCHMIAST — serwer nie może czekać na
/// dziesięciominutowy backtest.
///
/// `f` dostaje kontekst i zwraca podsumowanie albo błąd. Wszystko, co
/// dotyczy widoczności zadania w interfejsie (rejestracja, publikacja,
/// domknięcie, zwolnienie blokady), dzieje się TUTAJ — pojedyncze zadanie
/// nie ma jak zapomnieć o zwolnieniu laboratorium.
fn spawn_job(
    st: &StateHandle,
    kind: &'static str,
    title: String,
    f: impl FnOnce(&JobCtx) -> anyhow::Result<String> + Send + 'static,
) -> anyhow::Result<String> {
    let cancel = Arc::new(AtomicBool::new(false));
    if !st.lab.try_claim(cancel.clone()) {
        anyhow::bail!("laboratorium jest zajęte — najpierw zatrzymaj bieżące zadanie");
    }

    let id = stamp_id(kind);
    let out_dir = st.workspace.lab_dir().join(&id);
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        st.lab.release();
        anyhow::bail!(
            "nie mogę utworzyć katalogu wyników {}: {e}",
            out_dir.display()
        );
    }

    let job = LabJob {
        id: id.clone(),
        kind: kind.to_string(),
        title,
        phase: "running".into(),
        label: "przygotowanie danych…".into(),
        started_at: crate::now_ms(),
        out_dir: out_dir.display().to_string(),
        ..Default::default()
    };

    // od razu widoczne w interfejsie — zanim wczytają się ticki
    st.update(Sections::one(Section::Lab), |s| {
        s.lab.busy = true;
        s.lab.job = Some(job.clone());
    });

    let ctx = JobCtx::new(st.clone(), cancel, job, out_dir);
    let st2 = st.clone();
    let id2 = id.clone();

    std::thread::Builder::new()
        .name(format!("lab-{kind}"))
        .spawn(move || {
            // Zwolnienie blokady w `Drop`, a nie na końcu funkcji: gdyby
            // zadanie się załamało, laboratorium zostałoby zajęte NA ZAWSZE
            // i przycisk „Uruchom" przestałby cokolwiek robić aż do restartu.
            let _straznik = Zwolnij(st2.clone());
            let wynik = f(&ctx);
            match wynik {
                Ok(note) if ctx.cancelled() => {
                    ctx.finish("cancelled", note, None);
                    st2.log(
                        "backtests",
                        "warn",
                        "Zadanie przerwane — postęp zapisany",
                        id2,
                    );
                }
                Ok(note) => {
                    ctx.finish("done", note.clone(), None);
                    st2.log(
                        "backtests",
                        "success",
                        "Zadanie zakończone",
                        format!("{id2}: {note}"),
                    );
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    ctx.finish("failed", String::new(), Some(msg.clone()));
                    st2.log(
                        "backtests",
                        "error",
                        "Zadanie nie powiodło się",
                        format!("{id2}: {msg}"),
                    );
                }
            }
        })
        .map_err(|e| {
            st.lab.release();
            anyhow::anyhow!("nie udało się uruchomić wątku zadania: {e}")
        })?;

    Ok(id)
}

// ============================================================
//  OSOBNE OKIENKO POSTĘPU
// ============================================================

fn podnies_okno(language: &str) {
    if std::env::var_os("CONDUIT_BEZ_OKNA").is_some() || conduit_monitor::okno_dziala() {
        return;
    }
    let nazwa = if cfg!(windows) {
        "postep.exe"
    } else {
        "postep"
    };
    let mut kandydaci: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        let mut d = exe.parent().map(|x| x.to_path_buf());
        // Cztery poziomy, nie trzy: `conduit.exe` bywa uruchamiany z `PACKAGE\`,
        // z `rust\target\debug\` i z osobnego katalogu budowania. Najdalszy
        // z tych przypadków potrzebuje czterech kroków, żeby dosięgnąć
        // katalogu głównego repozytorium, w którym leży `LAB\postep.exe`.
        for _ in 0..4 {
            let Some(p) = d else { break };
            push_unique(&mut kandydaci, p.join(nazwa));
            push_unique(&mut kandydaci, p.join("LAB").join(nazwa));
            push_unique(
                &mut kandydaci,
                p.join("rust").join("target").join("release").join(nazwa),
            );
            push_unique(
                &mut kandydaci,
                p.join("rust").join("target").join("debug").join(nazwa),
            );
            d = p.parent().map(|x| x.to_path_buf());
        }
    }
    if let Some(p) = kandydaci.into_iter().find(|p| p.is_file()) {
        let mut c = std::process::Command::new(p);
        c.arg("--auto")
            .env("CONDUIT_LANGUAGE", if language == "en" { "en" } else { "pl" })
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            c.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        }
        if c.spawn().is_ok() {
            return;
        }
    }
    conduit_monitor::uruchom_okno_z_jezykiem(Some(language));
}

// ============================================================
//  CZTERY TRYBY OCENY (NAUKOWIEC §4)
// ============================================================

/// Jeden z czterech trybów oceny.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tryb {
    /// klucz w JSON-ie i w zleceniu
    pub key: &'static str,
    pub label: &'static str,
    /// konto wraca do kwoty startowej co dobę
    pub daily_reset: bool,
    /// lot rośnie razem z saldem
    pub compounding: bool,
}

/// Tabela 2×2 z §4. Kolejność jest częścią umowy z interfejsem: najpierw
/// wiersz „dzień po dniu", potem „długoterminowo"; w każdym najpierw bez
/// compoundingu.
pub const TRYBY: [Tryb; 4] = [
    Tryb {
        key: "dpd-staly",
        label: "dzień po dniu · lot stały",
        daily_reset: true,
        compounding: false,
    },
    Tryb {
        key: "dpd-comp",
        label: "dzień po dniu · compounding",
        daily_reset: true,
        compounding: true,
    },
    Tryb {
        key: "dlugo-staly",
        label: "długoterminowo · lot stały",
        daily_reset: false,
        compounding: false,
    },
    Tryb {
        key: "dlugo-comp",
        label: "długoterminowo · compounding",
        daily_reset: false,
        compounding: true,
    },
];

pub fn tryb(key: &str) -> Option<&'static Tryb> {
    TRYBY.iter().find(|t| t.key == key)
}

/// Ustawia OŚ COMPOUNDINGU presetu, nie ruszając niczego innego.
///
/// Sedno jest w tym, że oba ramiona muszą **startować od tego samego lota**.
/// Gdyby „z compoundingiem" znaczyło „weź `lot_percent` presetu", a „bez
/// compoundingu" — „weź jego `lot_fixed`", to porównanie mierzyłoby WIELKOŚĆ
/// POZYCJI, a nie compounding: dokładnie ta pomyłka opisana w NAUKOWIEC §3C
/// jako „mnożnik ekspozycji w przebraniu" (cztery przypadki jednego dnia).
///
/// Dlatego liczymy lot, jaki preset dałby przy kapitale startowym, i:
///  * bez compoundingu — zamrażamy go jako `lot_fixed`,
///  * z compoundingiem — wyrażamy go procentem, który przy kapitale startowym
///    daje dokładnie tę samą wartość, więc pierwsze wejście jest identyczne,
///    a różnica narasta dopiero wraz z kontem.
///
/// Lot startowy bierzemy z `Engine::lot_size` — tej samej funkcji, której
/// używa silnik. Przepisanie tych sześciu linijek tutaj byłoby drugą
/// implementacją, która pewnego dnia rozjedzie się z pierwszą.
pub fn ustaw_compounding(
    s: &mut conduit_core::settings::Settings,
    compounding: bool,
    balance: f64,
) {
    // PODSTAWA, NIE SALDO — i to jest różnica, nie kosmetyka.
    //
    // Przy włączonym `odlicz_kredyt` silnik liczy lot od salda POMNIEJSZONEGO
    // o bonus (`podstawa_lota`). Gdyby `lot0` policzyć tu z gołego `balance`,
    // laboratorium mierzyłoby lot, którego bot nigdy nie złoży:
    //
    //  * ramię BEZ compoundingu zamraża `lot_fixed = lot0`, a lot stały nie
    //    patrzy już na saldo — więc zamroziłby się lot LICZONY OD BONUSU,
    //    czyli na koncie 300 $ + 300 $ bonusu **dwa razy za duży**;
    //  * ramię Z compoundingiem wyraża `lot0` procentem salda, a silnik mnoży
    //    ten procent przez PODSTAWĘ — więc pierwsze wejście przestałoby być
    //    identyczne w obu ramionach, czyli oś przestałaby mierzyć compounding,
    //    a zaczęła mierzyć wielkość pozycji. Dokładnie to, czemu ta funkcja
    //    ma zapobiegać.
    //
    // Przy wyłączonym kredycie (domyślnie) `podstawa_lota() == balance`
    // i obie drogi dają tę samą liczbę co do centa — parytet nietknięty.
    let mut e = conduit_core::engine::Engine::new(s.clone(), balance);
    if s.credit_balance_separate {
        // Match the hypothetical opening account to SimBroker's MT5 model.
        // Offline manual C is broker C, even when lot deduction is OFF.
        let credit = if s.kredyt_reczny.is_finite() && s.kredyt_reczny > 0.0 {
            s.kredyt_reczny
        } else { 0.0 };
        e.stats.credit = credit;
        e.stats.equity = balance + credit;
    }
    let podstawa = e.podstawa_lota();
    let lot0 = e.lot_size(podstawa);
    // `lot_scale_step` to DRUGI, niezależny mechanizm compoundingu
    // (+0,01 lota na każde X $). Gdyby został włączony, ramię „bez
    // compoundingu" i tak by skalowało — czyli przełącznik nie zmieniałby
    // nic co do centa. Patrz NAUKOWIEC §5 pkt 9.
    s.lot_scale_step = 0.0;
    if compounding {
        s.lot_mode_percent = true;
        // Mianownik to PODSTAWA, nie saldo — bo silnik pomnoży ten procent
        // przez podstawę (`lot_size(podstawa_lota())`). Dzielenie przez saldo
        // dawałoby przy włączonym kredycie pierwsze wejście mniejsze niż
        // w ramieniu bez compoundingu, w stosunku podstawa/saldo (na koncie
        // 300 $ + 300 $ bonusu — o połowę).
        s.lot_percent = lot0 / podstawa.max(1.0) * 10_000.0;
    } else {
        s.lot_mode_percent = false;
        s.lot_fixed = lot0;
    }
}

// ============================================================
//  DANE WEJŚCIOWE
// ============================================================

/// Co laboratorium ZNALAZŁO na dysku — nie co powinno tam być.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabData {
    pub ok: bool,
    pub ticks_path: String,
    pub signals_path: String,
    pub ticks: u64,
    pub first_day: String,
    pub last_day: String,
    pub messages: u64,
    pub preset_dirs: Vec<PresetDir>,
    pub models: Vec<String>,
    pub out_root: String,
    /// czy leży punkt kontrolny treningu do wznowienia
    pub checkpoint: Option<CheckpointInfo>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetDir {
    /// nazwa katalogu — TO ONA wraca w zleceniu, nigdy pełna ścieżka
    pub name: String,
    pub path: String,
    pub count: usize,
    /// nazwy presetów (do wyboru pojedynczego)
    pub presets: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointInfo {
    pub gen: usize,
    pub best_center: f64,
    pub seed: u64,
    pub algo: String,
    pub saved_at: i64,
}

fn push_unique(v: &mut Vec<PathBuf>, p: PathBuf) {
    if !v.contains(&p) {
        v.push(p);
    }
}

/// Katalogi, w których szukamy `data/` i `presets*`.
///
/// Kolejność nie jest przypadkowa: najpierw katalog roboczy aplikacji, potem
/// bieżący katalog procesu, potem cztery poziomy nad plikiem wykonywalnym.
/// Ten ostatni przypadek to `rust/target/release/conduit.exe`, które ma dane
/// w `rust/data` — czyli dokładnie to, jak program uruchamia się przy pracy
/// nad kodem.
fn roots(ws: &Workspace) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    push_unique(&mut v, ws.root.clone());
    if let Ok(cwd) = std::env::current_dir() {
        push_unique(&mut v, cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut d = exe.parent().map(|x| x.to_path_buf());
        for _ in 0..4 {
            match d {
                Some(p) => {
                    push_unique(&mut v, p.clone());
                    d = p.parent().map(|x| x.to_path_buf());
                }
                None => break,
            }
        }
    }
    v
}

/// Podkatalogi z danymi sprawdzane w każdym katalogu wyszukiwania.
///
/// `rust/data` jest tu nie przez przypadek: w repozytorium dane leżą właśnie
/// tam, a `conduit.exe` uruchamia się z `PACKAGE/`. Zanim ta pozycja tu
/// trafiła, laboratorium mówiło „nie mogę otworzyć PACKAGE\data\ticks.bin"
/// i jedynym wyjściem było ręczne złącze katalogowe.
const PODKATALOGI_DANYCH: [&str; 4] = ["data", "rust/data", "DANE", "dane"];

/// Gdzie SZUKAMY danych — pełna lista kandydatów, w kolejności sprawdzania.
///
/// Zwracana także po to, żeby komunikat o błędzie mógł wypisać, gdzie
/// program zaglądał. „Nie znalazłem pliku" bez tej listy jest bezużyteczne.
pub fn data_candidates(ws: &Workspace) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    // 1. Jawne wskazanie użytkownika — wygrywa ze wszystkim.
    if let Ok(d) = std::env::var("CONDUIT_DATA") {
        push_unique(&mut out, PathBuf::from(d));
    }
    // 2. `data_dir.txt` obok konfiguracji: ustawienie trwałe, bez zmiennych
    //    środowiskowych i bez przekompilowywania.
    let wskaznik = ws.root.join("data_dir.txt");
    if let Ok(t) = std::fs::read_to_string(&wskaznik) {
        let t = t.trim();
        if !t.is_empty() {
            push_unique(&mut out, PathBuf::from(t));
        }
    }
    // 3. Automat: każdy katalog wyszukiwania × każdy znany podkatalog danych.
    for r in roots(ws) {
        for sub in PODKATALOGI_DANYCH {
            push_unique(&mut out, r.join(sub));
        }
    }
    out
}

/// Ścieżki do ticków i sygnałów.
///
/// Kolejność: `CONDUIT_DATA` → `data_dir.txt` w katalogu konfiguracji →
/// automatyczne przeszukanie. Gdy nic nie ma, zwracamy pierwszego kandydata,
/// żeby komunikat mówił o konkretnej ścieżce, a nie o pustce.
pub fn data_paths(ws: &Workspace) -> (PathBuf, PathBuf) {
    let kand = data_candidates(ws);
    for d in &kand {
        let t = d.join("ticks.bin");
        if t.is_file() {
            return (t, d.join("signals.json"));
        }
    }
    let d = kand
        .into_iter()
        .next()
        .unwrap_or_else(|| ws.root.join("data"));
    (d.join("ticks.bin"), d.join("signals.json"))
}

/// Czytelny opis „gdzie szukałem" — do komunikatu o braku danych.
pub fn gdzie_szukalem(ws: &Workspace) -> String {
    let k = data_candidates(ws);
    let lista: Vec<String> = k
        .iter()
        .take(12)
        .map(|p| format!("  • {}", p.join("ticks.bin").display()))
        .collect();
    format!(
        "Nie znalazłem pliku ticków. Sprawdziłem po kolei:\n{}\n\n\
         Jak to naprawić (dowolny sposób):\n\
           1. połóż `ticks.bin` i `signals.json` w jednym z tych katalogów,\n\
           2. albo zapisz ścieżkę do katalogu z danymi w pliku `{}`,\n\
           3. albo uruchom program ze zmienną `CONDUIT_DATA=<katalog>`.",
        lista.join("\n"),
        ws.root.join("data_dir.txt").display(),
    )
}

/// Katalogi presetów: `presets/` katalogu roboczego + wszystkie `presets*`
/// znalezione w katalogach wyszukiwania.
pub fn preset_dirs(ws: &Workspace) -> Vec<PresetDir> {
    let mut out: Vec<PresetDir> = Vec::new();
    let mut widziane: Vec<PathBuf> = Vec::new();

    let dodaj = |p: PathBuf, out: &mut Vec<PresetDir>, widziane: &mut Vec<PathBuf>| {
        if !p.is_dir() {
            return;
        }
        let kanon = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        if widziane.contains(&kanon) {
            return;
        }
        let mut presets: Vec<String> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&p) {
            for e in rd.flatten() {
                let f = e.path();
                if f.extension().map(|x| x == "json").unwrap_or(false) {
                    if let Some(n) = f.file_stem().and_then(|x| x.to_str()) {
                        presets.push(n.to_string());
                    }
                }
            }
        }
        if presets.is_empty() {
            return;
        }
        presets.sort();
        widziane.push(kanon);
        out.push(PresetDir {
            name: p
                .file_name()
                .map(|x| x.to_string_lossy().to_string())
                .unwrap_or_default(),
            path: p.display().to_string(),
            count: presets.len(),
            presets,
        });
    };

    dodaj(ws.presets_dir(), &mut out, &mut widziane);
    for r in roots(ws) {
        if let Ok(rd) = std::fs::read_dir(&r) {
            let mut kandydaci: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_dir()
                        && p.file_name()
                            .and_then(|x| x.to_str())
                            .map(|n| n.starts_with("presets"))
                            .unwrap_or(false)
                })
                .collect();
            kandydaci.sort();
            for k in kandydaci {
                dodaj(k, &mut out, &mut widziane);
            }
        }
    }
    // katalogi o tej samej nazwie w różnych miejscach są nierozróżnialne
    // w zleceniu — zostawiamy pierwszy, bo to on wygra przy wyszukiwaniu
    let mut nazwy: Vec<String> = Vec::new();
    out.retain(|d| {
        if nazwy.contains(&d.name) {
            false
        } else {
            nazwy.push(d.name.clone());
            true
        }
    });
    out
}

/// Zamienia NAZWĘ katalogu presetów na ścieżkę.
///
/// Klient nigdy nie podaje ścieżki — podaje nazwę z listy, którą sam dostał.
/// Dzięki temu nie ma czego walidować pod kątem `..` i nie da się namówić
/// serwera na czytanie dowolnego miejsca na dysku.
pub fn resolve_preset_dir(ws: &Workspace, name: &str) -> anyhow::Result<PathBuf> {
    preset_dirs(ws)
        .into_iter()
        .find(|d| d.name == name)
        .map(|d| PathBuf::from(d.path))
        .ok_or_else(|| anyhow::anyhow!("nie znam katalogu presetów „{name}”"))
}

/// Modele AI leżące w `models/`.
pub fn models(ws: &Workspace) -> Vec<String> {
    let mut out = Vec::new();
    for r in roots(ws) {
        let d = r.join("models");
        if !d.is_dir() {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().map(|x| x == "json").unwrap_or(false) {
                    if let Some(n) = p.file_name().and_then(|x| x.to_str()) {
                        if !out.contains(&n.to_string()) {
                            out.push(n.to_string());
                        }
                    }
                }
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out.sort();
    out
}

/// Pełny opis tego, z czym laboratorium może pracować.
pub fn discover(ws: &Workspace) -> LabData {
    let (ticks, signals) = data_paths(ws);
    let mut d = LabData {
        ticks_path: ticks.display().to_string(),
        signals_path: signals.display().to_string(),
        preset_dirs: preset_dirs(ws),
        models: models(ws),
        out_root: ws.lab_dir().display().to_string(),
        checkpoint: training::checkpoint_info(ws),
        ..Default::default()
    };

    match conduit_backtest::TickData::open(&ticks) {
        Ok(td) => {
            d.ticks = td.len() as u64;
            d.first_day = dzien(td.first_ts());
            d.last_day = dzien(td.last_ts());
            d.ok = td.len() > 0;
        }
        // Brak pliku to inny problem niż plik uszkodzony — i wymaga innej
        // odpowiedzi. Przy braku wypisujemy, GDZIE szukaliśmy.
        Err(e) if !ticks.is_file() => d.error = Some(format!("{}\n\n({e:#})", gdzie_szukalem(ws))),
        Err(e) => d.error = Some(format!("{e:#}")),
    }
    if d.ok {
        match conduit_backtest::load_messages(&signals) {
            Ok(m) => d.messages = m.len() as u64,
            Err(e) => {
                d.ok = false;
                d.error = Some(format!("{e:#}"));
            }
        }
    }
    d
}

// ============================================================
//  DATY
// ============================================================

/// „RRRR-MM-DD" z milisekund epoki.
pub fn dzien(ts: i64) -> String {
    let day = ts.div_euclid(86_400_000);
    let z = day + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// „RRRR-MM-DD" → milisekundy epoki (północ UTC).
pub fn parse_dzien(s: &str) -> anyhow::Result<i64> {
    let p: Vec<&str> = s.trim().split('-').collect();
    if p.len() != 3 {
        anyhow::bail!("data musi mieć format RRRR-MM-DD, dostałem „{s}”");
    }
    let (y, m, d): (i64, i64, i64) = (p[0].parse()?, p[1].parse()?, p[2].parse()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        anyhow::bail!("data poza zakresem: „{s}”");
    }
    let yy = if m <= 2 { y - 1 } else { y };
    let era = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe = yy - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Ok((era * 146_097 + doe - 719_468) * 86_400_000)
}

/// Nazwa pliku bezpieczna dla systemu plików.
pub fn bezpieczna_nazwa(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        "wynik".into()
    } else {
        out
    }
}

/// Ścieżka do pliku wyniku danego zadania — z kontrolą, że nie wychodzi
/// poza katalog zadania. Używane przez podgląd wykresów w REST.
pub fn plik_zadania(st: &StateHandle, job_id: &str, nazwa: &str) -> anyhow::Result<PathBuf> {
    if job_id.contains('/') || job_id.contains('\\') || job_id.contains("..") {
        anyhow::bail!("niedozwolony identyfikator zadania");
    }
    if nazwa.contains('/') || nazwa.contains('\\') || nazwa.contains("..") {
        anyhow::bail!("niedozwolona nazwa pliku");
    }
    let dir = st.workspace.lab_dir().join(job_id);
    let p = dir.join(nazwa);
    if !p.is_file() {
        anyhow::bail!("nie ma pliku „{nazwa}” w wynikach zadania „{job_id}”");
    }
    Ok(p)
}

/// Katalog wyników zadania — do przycisku „Otwórz katalog".
pub fn katalog_zadania(st: &StateHandle, job_id: &str) -> anyhow::Result<PathBuf> {
    if job_id.contains('/') || job_id.contains('\\') || job_id.contains("..") {
        anyhow::bail!("niedozwolony identyfikator zadania");
    }
    let d = st.workspace.lab_dir().join(job_id);
    if !d.is_dir() {
        anyhow::bail!("nie ma katalogu wyników zadania „{job_id}”");
    }
    Ok(d)
}

/// Zapis dokumentu JSON w katalogu zadania.
pub(crate) fn zapisz_json<T: Serialize>(dir: &Path, nazwa: &str, v: &T) -> anyhow::Result<()> {
    crate::store::write_json_atomic(&dir.join(nazwa), v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_balance_separate_compounding_first_lot_matches_sim_account() {
        use conduit_core::{engine::Engine, settings::{Settings, PodstawaLota}};
        for balance in [159.8, 600.0, 1000.0] {
            for base in [PodstawaLota::Balance, PodstawaLota::Equity, PodstawaLota::MinOfBoth] {
                for deduct in [false, true] {
                    let original=Settings {credit_balance_separate:true,odlicz_kredyt:deduct,kredyt_reczny:300.0,lot_base:base,lot_mode_percent:true,lot_percent:0.5,lot_max:0.0,..Settings::default()};
                    let expected_basis=original.podstawa_lota_z_konta(balance,balance+300.0,300.0);
                    let expected=Engine::new(original.clone(),balance).lot_size(expected_basis);
                    for compound in [false,true] {
                        let mut candidate=original.clone();
                        ustaw_compounding(&mut candidate,compound,balance);
                        let mut engine=Engine::new(candidate,balance);
                        engine.stats.equity=balance+300.0;engine.stats.credit=300.0;
                        assert_eq!(engine.lot_size(engine.podstawa_lota()),expected,"balance={balance},base={base:?},deduct={deduct},compound={compound}");
                    }
                }
            }
        }
    }

    #[test]
    fn daty_tam_i_z_powrotem() {
        for s in ["2026-04-01", "2026-07-27", "1970-01-01", "2026-12-31"] {
            assert_eq!(dzien(parse_dzien(s).unwrap()), s);
        }
    }

    #[test]
    fn zla_data_jest_bledem_a_nie_zgadywaniem() {
        assert!(parse_dzien("27.07.2026").is_err());
        assert!(parse_dzien("2026-13-01").is_err());
        assert!(parse_dzien("").is_err());
    }

    #[test]
    fn nazwa_pliku_jest_odkazana() {
        assert_eq!(bezpieczna_nazwa("R-sl25/p2"), "R-sl25_p2");
        assert_eq!(bezpieczna_nazwa("../../etc"), "______etc");
        assert_eq!(bezpieczna_nazwa(""), "wynik");
    }

    #[test]
    fn laboratorium_przyjmuje_tylko_jedno_zadanie() {
        let c = LabControl::default();
        assert!(!c.busy());
        assert!(c.try_claim(Arc::new(AtomicBool::new(false))));
        assert!(c.busy());
        assert!(
            !c.try_claim(Arc::new(AtomicBool::new(false))),
            "drugie zadanie nie ma prawa wejść"
        );
        assert!(c.request_cancel());
        c.release();
        assert!(!c.busy());
        assert!(!c.request_cancel(), "nie ma czego przerywać");
    }
}
