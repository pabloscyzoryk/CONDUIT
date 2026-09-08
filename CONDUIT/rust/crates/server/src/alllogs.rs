
use crate::coalesce::{Section, Sections};
use crate::state::StateHandle;
use anyhow::Result;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::io::Read as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

#[path = "alllogs_capture.rs"]
mod capture_export;

/// ANULOWANIE scalania — jedna flaga wystarcza, bo `uruchom` dopuszcza
/// najwyżej JEDEN job naraz (bail przy `aktywne`). Ustawia ją REST
/// `POST /api/logs/merge/cancel`, sprawdza pętla po plikach dziennika —
/// czyli dokładnie tam, gdzie schodzi >90 % czasu pracy.
pub static ANULUJ: AtomicBool = AtomicBool::new(false);

/// Szerokość linii nagłówka sekcji — stała, żeby plik dało się skanować wzrokiem.
const SZER: usize = 78;

fn naglowek(out: &mut String, tytul: &str) {
    let _ = writeln!(out, "\n{}", "═".repeat(SZER));
    let _ = writeln!(out, "  {tytul}");
    let _ = writeln!(out, "{}", "═".repeat(SZER));
}

fn podsekcja(out: &mut String, tytul: &str) {
    let _ = writeln!(
        out,
        "\n── {tytul} {}",
        "─".repeat(SZER.saturating_sub(tytul.len() + 4))
    );
}

/// Wartość JSON jako jedna linijka, bez cudzysłowów wokół łańcuchów.
fn plask(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "—".into(),
        other => other.to_string(),
    }
}

fn pole(v: &Value, klucz: &str) -> String {
    v.get(klucz).map(plask).unwrap_or_else(|| "—".into())
}

/// Czas w milisekundach → `YYYY-MM-DD HH:MM:SS` w strefie lokalnej maszyny.
fn czas(ms: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(ms).single() {
        Some(t) => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => format!("ms={ms}"),
    }
}

fn melduj(st: &StateHandle, etap: &str, zrobione: u64, wszystkich: u64, start: std::time::Instant) {
    let up = start.elapsed().as_millis() as i64;
    let postep = if wszystkich > 0 {
        (zrobione as f64 / wszystkich as f64).min(1.0)
    } else {
        0.0
    };
    let mb_na_s = if up > 300 {
        zrobione as f64 / 1_048_576.0 * 1000.0 / up as f64
    } else {
        0.0
    };
    let eta = if postep > 0.01 && postep < 1.0 && up > 300 {
        ((up as f64) * (1.0 - postep) / postep) as i64
    } else {
        0
    };
    st.update_transient(Sections::one(Section::Scalanie), |s| {
        s.scalanie.aktywne = true;
        s.scalanie.faza = "trwa".into();
        s.scalanie.etap = etap.to_string();
        s.scalanie.postep = postep;
        s.scalanie.zrobione = zrobione;
        s.scalanie.wszystkich = wszystkich;
        s.scalanie.predkosc = if mb_na_s > 0.0 {
            format!("{mb_na_s:.1} MB/s")
        } else {
            String::new()
        };
        s.scalanie.eta_ms = eta;
        s.scalanie.czas_ms = up;
        s.scalanie.blad = None;
    });
}


/// Wybór źródeł z panelu (`settings.merge_config`).
///
/// ⚠ **Brak klucza znaczy TAK** (poza wrażliwymi). To nie jest wygoda, tylko
/// warunek bezpieczeństwa aktualizacji: `merge_config` zapisane starszą wersją
/// bota nie zna źródeł dołożonych później, a domyślne „nie" kasowałoby je
/// z pliku po cichu — czyli dokładnie ta klasa usterki, którą ten plik ma
/// diagnozować.
#[derive(Debug, Clone, Default)]
pub struct Wybor(Option<serde_json::Map<String, Value>>);

impl Wybor {
    pub fn z_ustawien(st: &StateHandle) -> Wybor {
        Wybor(st.read(|s| {
            s.settings
                .get("merge_config")
                .and_then(|v| v.as_object())
                .cloned()
        }))
    }

    /// Czy źródło ma wejść. `wrazliwe` odwraca domyślną: dane logowania
    /// wchodzą WYŁĄCZNIE na wyraźne zaznaczenie.
    pub fn chce(&self, klucz: &str, wrazliwe: bool) -> bool {
        match self
            .0
            .as_ref()
            .and_then(|m| m.get(klucz))
            .and_then(|v| v.as_bool())
        {
            Some(v) => v,
            None => !wrazliwe,
        }
    }
}

/// Jedna pozycja SPISU ŹRÓDEŁ drukowanego w nagłówku.
///
/// Spis jest odpowiedzią na pytanie, którego bez niego nie da się zadać:
/// **czego w tym pliku NIE MA.** Zrzut bez tej tabelki wygląda tak samo,
/// gdy źródło było puste, gdy je odznaczono i gdy zapomniano je podłączyć.
#[derive(Debug, Clone)]
pub struct Pozycja {
    pub klucz: &'static str,
    pub tytul: String,
    pub wlaczone: bool,
    pub rekordow: u64,
    pub bajtow: u64,
    /// zakres czasu rekordów; 0 = nie dotyczy albo nie dało się ustalić
    pub od_ms: i64,
    pub do_ms: i64,
    /// dlaczego pusto / dlaczego pominięte — nigdy cisza
    pub uwaga: String,
}

impl Pozycja {
    fn nowa(klucz: &'static str, tytul: impl Into<String>, wlaczone: bool) -> Pozycja {
        Pozycja {
            klucz,
            tytul: tytul.into(),
            wlaczone,
            rekordow: 0,
            bajtow: 0,
            od_ms: 0,
            do_ms: 0,
            uwaga: String::new(),
        }
    }
    fn zakres(&mut self, ms: i64) {
        if ms <= 0 {
            return;
        }
        if self.od_ms == 0 || ms < self.od_ms {
            self.od_ms = ms;
        }
        if ms > self.do_ms {
            self.do_ms = ms;
        }
    }
    fn opis_zakresu(&self) -> String {
        if self.od_ms == 0 && self.do_ms == 0 {
            return "—".into();
        }
        format!("{} → {}", czas(self.od_ms), czas(self.do_ms))
    }
}

/// Jedna linia strumienia z czasem — materiał do przeplotu chronologicznego.
struct Linia {
    ms: i64,
    seq: u64,
    priorytet: u8,
    kolejnosc: u64,
    zrodlo: &'static str,
    plik: String,
    nr: u64,
    tresc: String,
}

/// Wyciąga znacznik czasu z wiersza JSONL dowolnego z naszych formatów.
///
/// Formaty powstawały w różnych miesiącach i nazywają to samo pole inaczej
/// (`odebrano_ms`, `received_at_ms`, `ts`, `t`, `time_ms`). Przeplot bez
/// wspólnego mianownika ustawiłby całe źródło na początku pliku i wyglądałby
/// jak awaria zegara — dlatego czytamy WSZYSTKIE znane nazwy, a przy braku
/// zwracamy 0 i mówimy o tym w spisie.
fn czas_wiersza(v: &Value) -> i64 {
    for k in [
        "odebrano_ms",
        "received_at_ms",
        "ts_ms",
        "ts",
        "t",
        "time_ms",
        "zapisano",
    ] {
        if let Some(n) = v.get(k).and_then(|x| x.as_i64()) {
            // sekundy vs milisekundy: 10^11 ms to rok 1973, 10^11 s to rok 5138
            return if n > 0 && n < 100_000_000_000 {
                n * 1000
            } else {
                n
            };
        }
    }
    0
}

/// Pliki źródła posortowane po nazwie (nazwy niosą datę, więc alfabetycznie
/// = chronologicznie) z sumą bajtów.
fn pliki_z_katalogu(dir: &std::path::Path, rozsz: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().map(|e| e == rozsz).unwrap_or(false))
        .collect();
    v.sort();
    v
}

fn bajty(pliki: &[PathBuf]) -> u64 {
    pliki
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum()
}

/// Kolejność w obrębie jednego strumienia. Kronika i archiwum mają jawne
/// `seq`; starsze dzienniki nie, więc tam numer linii jest jedynym uczciwym
/// porządkiem (i dokładnie porządkiem zapisu na dysk).
fn seq_wiersza(v: &Value, nr: u64) -> u64 {
    v.get("seq").and_then(|x| x.as_u64()).unwrap_or(nr)
}

/// SHA-256 pliku wejściowego. Zrzut diagnostyczny ma być dowodem, a nie
/// tylko kopią tekstu bez możliwości wykazania, z którego dokładnie pliku
/// powstał. Czytanie jest strumieniowe, więc kronika 100 MB nie jest drugi
/// raz ładowana do pamięci.
fn sha256_pliku(path: &std::path::Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 128 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

fn zmieniono_ms(path: &std::path::Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Klucze pól wyboru, które odpowiadają KATEGORIOM logu panelu.
///
/// Kategoria spoza tej listy (`telegram`, `settings`, `kronika`, `backtests`…)
/// jedzie pod kluczem `events`. Bez tej reguły dołożenie nowej kategorii
/// w silniku wycinałoby ją ze zrzutu po cichu — bo `merge_config` z panelu
/// takiego klucza nie ma, a domyślne „nie" jest gorsze niż nadmiar.
const MERGE_KLUCZE: &[&str] = &[
    "commands",
    "events",
    "messages",
    "signals",
    "trades",
    "unpredicted_signals",
    "signal_formats",
    "backup_memory",
    "poll_interval",
    "update_performance",
    "price_log",
    "session_string",
    "smtp",
];

/// Strumień liniowy (`.jsonl` / `.log`) — materiał sekcji 6.
struct ZrodloLiniowe {
    klucz: &'static str,
    /// krótki znacznik doklejany do KAŻDEJ linii, żeby po przeplocie dało się
    /// jednym `findstr` wyciągnąć z powrotem samo jedno źródło
    znacznik: &'static str,
    /// Rozstrzyga remis tej samej milisekundy. Surowy odbiór Telegrama musi
    /// stać przed decyzją silnika, która powstała w reakcji na ten odbiór.
    priorytet: u8,
    tytul: String,
    katalog: String,
    pliki: Vec<PathBuf>,
    wlaczone: bool,
}

/// Plik stanu/konfiguracji — materiał sekcji 10.
struct ZrodloJson {
    klucz: &'static str,
    tytul: String,
    katalog: String,
    pliki: Vec<PathBuf>,
    wrazliwe: bool,
    /// tylko nazwy i rozmiary; treść byłaby megabajtami maszynowego JSON-a
    tylko_spis: bool,
}

/// WSZYSTKIE strumienie liniowe, jakie bot produkuje.
///
/// Kolejność jest kolejnością sekcji przy scalaniu „źródło po źródle";
/// przy przeplocie chronologicznym nie ma znaczenia.
fn zrodla_liniowe(st: &StateHandle, wybor: &Wybor) -> Vec<ZrodloLiniowe> {
    let journal = st.workspace.journal_dir();
    let archiwum = st.workspace.archive_dir();
    // Kronika bywa POZA katalogiem bota (domyślnie na pulpicie) — bierzemy
    // ścieżkę z jej własnych ustawień, razem z plikami obróconymi.
    let ust_kroniki = st.workspace.load_kronika();
    let plik_kroniki = ust_kroniki.sciezka(&st.workspace.root);
    let pliki_kroniki = crate::kronika::pliki_kroniki(&plik_kroniki);

    vec![
        ZrodloLiniowe {
            klucz: "journal",
            znacznik: "DECYZJA",
            priorytet: 2,
            tytul: "dziennik decyzji".into(),
            katalog: journal.display().to_string(),
            pliki: pliki_z_katalogu(&journal, "jsonl"),
            wlaczone: wybor.chce("journal", false),
        },
        ZrodloLiniowe {
            klucz: "kronika",
            znacznik: "KRONIKA",
            priorytet: 0,
            tytul: "kronika (strumień Telegrama)".into(),
            katalog: plik_kroniki.display().to_string(),
            pliki: pliki_kroniki,
            wlaczone: wybor.chce("kronika", false),
        },
        ZrodloLiniowe {
            klucz: "wiadomosci",
            znacznik: "ARCHIWUM",
            priorytet: 1,
            tytul: "archiwum wiadomości".into(),
            katalog: archiwum.display().to_string(),
            pliki: pliki_z_katalogu(&archiwum, "jsonl"),
            wlaczone: wybor.chce("wiadomosci", false),
        },
        ZrodloLiniowe {
            klucz: "journal_log",
            znacznik: "DZIENNIK-TXT",
            priorytet: 3,
            // ⚠ To jest LUSTRO tekstowe `.jsonl` z tego samego katalogu —
            // te same zdarzenia, format dla oka. Domyślnie włączone, bo
            // wymaganie brzmi „wszystko"; jeżeli plik ma być dwa razy
            // mniejszy, to jest pierwsze pole do odznaczenia.
            tytul: "dziennik decyzji (lustro .log)".into(),
            katalog: journal.display().to_string(),
            pliki: pliki_z_katalogu(&journal, "log"),
            wlaczone: wybor.chce("journal_log", false),
        },
    ]
}

/// Pliki stanu i konfiguracji leżące na dysku obok bota.
fn zrodla_plikowe_json(st: &StateHandle) -> Vec<ZrodloJson> {
    let ws = &st.workspace;
    let root = ws.root.clone();
    let istniejace = |p: PathBuf| -> Vec<PathBuf> {
        if p.is_file() {
            vec![p]
        } else {
            Vec::new()
        }
    };
    let migawki = pliki_z_katalogu(&ws.backup_dir(), "json");
    let ostatnia_migawka = migawki.last().cloned().map(istniejace).unwrap_or_default();

    vec![
        ZrodloJson {
            klucz: "backup_memory",
            tytul: "backup_memory — OSTATNIA migawka (pełna treść)".into(),
            katalog: ws.backup_dir().display().to_string(),
            pliki: ostatnia_migawka,
            wrazliwe: false,
            tylko_spis: false,
        },
        ZrodloJson {
            klucz: "backup_memory",
            tytul: "backup_memory — wszystkie migawki (spis)".into(),
            katalog: ws.backup_dir().display().to_string(),
            pliki: migawki,
            wrazliwe: false,
            // Setki migawek po ~20 kB to kilkanaście MB tej samej treści
            // z przesuniętym zegarem. Spis mówi WSZYSTKO, co z nich wynika:
            // kiedy bot żył i jak gęsto zapisywał.
            tylko_spis: true,
        },
        ZrodloJson {
            klucz: "koszyki",
            tytul: "koszyki.json — koszyki silnika".into(),
            katalog: root.display().to_string(),
            pliki: istniejace(root.join("koszyki.json")),
            wrazliwe: false,
            tylko_spis: false,
        },
        ZrodloJson {
            klucz: "signal_formats",
            tytul: "lancuchy.json — łańcuchy format→preset".into(),
            katalog: root.display().to_string(),
            pliki: istniejace(ws.lancuchy_path()),
            wrazliwe: false,
            tylko_spis: false,
        },
        ZrodloJson {
            klucz: "konfiguracja",
            tytul: "konfiguracja na dysku".into(),
            katalog: root.display().to_string(),
            pliki: [
                ws.settings_path(),
                ws.channels_path(),
                ws.demo_path(),
                ws.kronika_path(),
            ]
            .into_iter()
            .filter(|p| p.is_file())
            .collect(),
            wrazliwe: false,
            tylko_spis: false,
        },
        ZrodloJson {
            klucz: "presety",
            tytul: "presety strategii (spis)".into(),
            katalog: ws.presets_dir().display().to_string(),
            pliki: pliki_z_katalogu(&ws.presets_dir(), "json"),
            wrazliwe: false,
            // Aktywny preset jest w całości w sekcji 2 (pełna lista ustawień);
            // pozostałe to biblioteka, nie stan tego przebiegu.
            tylko_spis: true,
        },
        ZrodloJson {
            klucz: "mail_queue",
            tytul: "kolejka poczty (niewysłane alerty)".into(),
            katalog: root.display().to_string(),
            pliki: istniejace(root.join("mail_queue.json")),
            wrazliwe: false,
            tylko_spis: false,
        },
        ZrodloJson {
            klucz: "lab",
            tytul: "laboratorium — wyniki (spis)".into(),
            katalog: ws.lab_dir().display().to_string(),
            pliki: pliki_z_katalogu(&ws.lab_dir(), "json"),
            wrazliwe: false,
            tylko_spis: true,
        },
        ZrodloJson {
            klucz: "smtp",
            tytul: "smtp.json — konfiguracja poczty ⚠".into(),
            katalog: root.display().to_string(),
            pliki: istniejace(ws.smtp_path()),
            // ⚠ WRAŻLIWE: plik niesie login i serwer nadawcy. Hasło mieszka
            // osobno (`secrets.json`) i nie trafia tu nigdy, ale samo konto
            // nadawcy też jest daną, której nie wkleja się na czat.
            wrazliwe: true,
            tylko_spis: false,
        },
        ZrodloJson {
            klucz: "session_string",
            // ŚWIADOMIE SAM SPIS, także przy ZAZNACZONYM polu.
            //
            // `secrets.json` niesie klucz sesji Telegrama i hasło MT5. Ten
            // plik wysyła się mailem i wkleja na czat — sesja Telegrama
            // w takim zrzucie to przejęcie konta, a nie diagnostyka.
            // Diagnostycznie liczy się WYŁĄCZNIE to, czy poświadczenia
            // istnieją i z kiedy są; jedno i drugie widać po nazwie i dacie.
            tytul: "poświadczenia — SAM FAKT ISTNIENIA ⚠".into(),
            katalog: root.display().to_string(),
            pliki: istniejace(ws.secrets_path()),
            wrazliwe: true,
            tylko_spis: true,
        },
    ]
}

/// Skraca do `n` znaków (po ZNAKACH, nie bajtach — tabelka jest po polsku).
fn skroc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
}

/// Rozmiar po ludzku — bo „74 300 129" nikomu nic nie mówi.
fn mb(b: u64) -> String {
    if b >= 1024 * 1024 {
        format!("{:.1} MB", b as f64 / 1_048_576.0)
    } else if b >= 1024 {
        format!("{:.0} kB", b as f64 / 1024.0)
    } else {
        format!("{b} B")
    }
}

pub fn uruchom(st: &StateHandle) -> Result<()> {
    if st.read(|s| s.scalanie.aktywne) {
        anyhow::bail!("scalanie już trwa");
    }
    // WALIDACJA KATALOGU DOCELOWEGO PRZED STARTEM — plik-sonda, nie
    // zgadywanie. Błąd wychodzi z przycisku (ack → toast), a nie po minucie
    // pracy w fazie zapisu. Puste ustawienie = katalog `logs` bota.
    if let Some(dir) = katalog_docelowy(st) {
        sprawdz_katalog(&dir)?;
    }
    ANULUJ.store(false, Ordering::Relaxed);
    st.update_transient(Sections::one(Section::Scalanie), |s| {
        s.scalanie = crate::ui::PostepScalania {
            aktywne: true,
            faza: "trwa".into(),
            etap: "start".into(),
            ..Default::default()
        };
    });
    let st2 = st.clone();
    std::thread::Builder::new()
        .name("conduit-scalanie".into())
        .spawn(move || {
            let start = std::time::Instant::now();
            let wynik = zapisz_z_postepem(&st2, start);
            let up = start.elapsed().as_millis() as i64;
            match wynik {
                Ok((sciezka, znakow)) => {
                    let nazwa = sciezka
                        .file_name()
                        .map(|x| x.to_string_lossy().to_string())
                        .unwrap_or_default();
                    st2.update_transient(Sections::one(Section::Scalanie), |s| {
                        s.scalanie.aktywne = false;
                        s.scalanie.faza = "gotowe".into();
                        s.scalanie.etap = "zakończone".into();
                        s.scalanie.postep = 1.0;
                        s.scalanie.eta_ms = 0;
                        s.scalanie.czas_ms = up;
                        s.scalanie.plik = nazwa.clone();
                        s.scalanie.znakow = znakow as u64;
                        s.scalanie.sciezka = sciezka.display().to_string();
                        s.scalanie.blad = None;
                    });
                    st2.log(
                        "logs",
                        "success",
                        format!("Scalono dziennik do {nazwa}"),
                        format!("{znakow} znaków · {} ms · {}", up, sciezka.display()),
                    );
                }
                Err(e) => {
                    let opis = e.to_string();
                    let anulowane = opis.contains("anulowane przez");
                    st2.update_transient(Sections::one(Section::Scalanie), |s| {
                        s.scalanie.aktywne = false;
                        s.scalanie.faza = if anulowane {
                            "anulowane".into()
                        } else {
                            "blad".into()
                        };
                        s.scalanie.etap = "przerwane".into();
                        s.scalanie.czas_ms = up;
                        s.scalanie.blad = if anulowane { None } else { Some(opis.clone()) };
                    });
                    if anulowane {
                        // Anulowanie zostawia system CZYSTY: żadnego pliku
                        // częściowego (zapis idzie przez .tmp+rename na końcu),
                        // stan wraca do spoczynku.
                        st2.log("logs", "info", "Scalanie ANULOWANE przez użytkownika", opis);
                    } else {
                        st2.log("logs", "error", "Scalenie dziennika NIE UDAŁO SIĘ", opis);
                    }
                }
            }
        })?;
    Ok(())
}

/// Buduje CAŁY dokument. Osobno od zapisu, żeby dało się go przetestować
/// bez dotykania dysku.
pub fn zbuduj(st: &StateHandle) -> String {
    // Bez postępu nie ma też anulowania, więc jedyny możliwy `Err` nie
    // występuje — pusty dokument zamiast paniki, gdyby to się zmieniło.
    zbuduj_z_postepem(st, None).unwrap_or_default()
}

/// Wersja z meldowaniem postępu. `None` = bez meldunków (testy, wywołanie
/// synchroniczne).
pub fn zbuduj_z_postepem(st: &StateHandle, postep: Option<std::time::Instant>) -> Result<String> {
    let snap = st.snapshot();
    let v = serde_json::to_value(&snap).unwrap_or(Value::Null);
    let mut o = String::with_capacity(256 * 1024);
    let wybor = Wybor::z_ustawien(st);
    let chronologicznie = st.read(|s| {
        s.settings
            .get("merge_chronological")
            .and_then(|x| x.as_bool())
            .unwrap_or(true)
    });
    // SPIS ŹRÓDEŁ powstaje w trakcie, a drukuje się na GÓRZE — dlatego
    // nagłówek dokleja się na końcu (`insert_str`), a nie tutaj.
    let mut spis: Vec<Pozycja> = Vec::new();
    let teraz = crate::now_ms();

    // ---------- 1. rachunek i broker ----------
    naglowek(&mut o, "1. RACHUNEK I BROKER");
    let conn = v.get("connection").cloned().unwrap_or(Value::Null);
    let acc = conn.get("account").cloned().unwrap_or(Value::Null);
    let _ = writeln!(o, "  telegram      : {}", pole(&conn, "telegram"));
    let _ = writeln!(o, "  mt5           : {}", pole(&conn, "mt5"));
    let _ = writeln!(o, "  login         : {}", pole(&acc, "login"));
    let _ = writeln!(o, "  serwer        : {}", pole(&acc, "server"));
    let _ = writeln!(o, "  broker        : {}", pole(&acc, "broker"));
    let _ = writeln!(o, "  waluta        : {}", pole(&acc, "currency"));
    let _ = writeln!(o, "  dźwignia      : 1:{}", pole(&acc, "leverage"));
    let _ = writeln!(o, "  typ konta     : {}", pole(&acc, "type"));
    let _ = writeln!(o, "  tryb pracy    : {}", pole(&v, "mode"));
    let _ = writeln!(o, "  opóźnienie ms : {}", pole(&conn, "latencyMs"));

    podsekcja(&mut o, "kwotowania w chwili zrzutu");
    if let Some(q) = v.get("quotes").and_then(|x| x.as_object()) {
        for (sym, d) in q {
            let _ = writeln!(
                o,
                "  {sym:10} bid {} / ask {}   spread {}   czas serwera {}",
                pole(d, "bid"),
                pole(d, "ask"),
                pole(d, "spread"),
                pole(d, "time")
            );
        }
    }
    let _ = writeln!(
        o,
        "\n  UWAGA O CZASIE: znaczniki kwotowań są w zegarze SERWERA BROKERA\n  \
         (dla Vantage to UTC+3). Wiadomości z Telegrama są w UTC. Silnik\n  \
         przelicza to przez `msg_offset()`. Przy porównywaniu linijek z różnych\n  \
         sekcji tego pliku trzeba o tym pamiętać — to jest najczęstsze źródło\n  \
         fałszywych „rozjazdów” w analizie."
    );

    // ---------- 2. konfiguracja ----------
    naglowek(&mut o, "2. KONFIGURACJA SILNIKA — CZYM BOT GRAŁ");
    let _ = writeln!(o, "  preset        : {}", {
        let p = pole(&v, "presetId");
        if p == "—" || p.is_empty() {
            "(brak etykiety — ustawienia zmieniane ręcznie)".into()
        } else {
            p
        }
    });
    let aktywny_lancuch = snap.aktywny_lancuch_nazwa().to_string();
    let _ = writeln!(
        o,
        "  aktywny łańcuch: {aktywny_lancuch}  (tryb {:?})",
        snap.mode
    );
    podsekcja(
        &mut o,
        "AKTYWNE NOGI format → preset (stan rzeczywiście ładowany przez live)",
    );
    match snap.aktywny_lancuch() {
        Some(l) => {
            let mut ile = 0usize;
            for (format, preset) in &l.presety {
                if preset.trim().is_empty() {
                    continue;
                }
                ile += 1;
                let p = st.workspace.presets_dir().join(format!("{preset}.json"));
                let hash = sha256_pliku(&p).unwrap_or_else(|e| format!("BŁĄD: {e}"));
                let _ = writeln!(
                    o,
                    "  {format:12} → {preset:24} plik={}  sha256={hash}",
                    p.display()
                );
            }
            if ile == 0 {
                let _ = writeln!(o, "  (łańcuch nie ma żadnej handlującej nogi)");
            }
            let _ = writeln!(
                o,
                "  pułapy łańcucha: {}",
                serde_json::to_string(&l.pulapy).unwrap_or_default()
            );
        }
        None => {
            let _ = writeln!(o, "  ! BRAK aktywnego łańcucha o tej nazwie");
        }
    }

    podsekcja(&mut o, "ODCISK URUCHOMIONEJ BINARKI");
    match std::env::current_exe() {
        Ok(p) => {
            let meta = std::fs::metadata(&p).ok();
            let hash = sha256_pliku(&p).unwrap_or_else(|e| format!("BŁĄD: {e}"));
            let _ = writeln!(
                o,
                "  plik={}  bajty={}  zmieniono_ms={}  sha256={hash}",
                p.display(),
                meta.as_ref().map(|m| m.len()).unwrap_or(0),
                zmieniono_ms(&p)
            );
        }
        Err(e) => {
            let _ = writeln!(o, "  ! nie da się ustalić binarki procesu: {e}");
        }
    }
    let lot = v.get("lot").cloned().unwrap_or(Value::Null);
    let _ = writeln!(
        o,
        "  wielkość lota : tryb={} stały={} procent={}",
        pole(&lot, "mode"),
        pole(&lot, "fixed"),
        pole(&lot, "percent")
    );

    let ustaw = v.get("settings").cloned().unwrap_or(Value::Null);
    if let Some(map) = ustaw.as_object() {
        let mut klucze: Vec<&String> = map.keys().collect();
        klucze.sort();
        let _ = writeln!(o, "\n  wszystkich ustawień: {}", klucze.len());
        podsekcja(&mut o, "pełna lista (klucz = wartość)");
        for k in klucze {
            let _ = writeln!(o, "  {k:38} = {}", plask(&map[k]));
        }
    }

    // Trzy diagnostyki, które odpowiadają na „czemu to pole nic nie robi".
    podsekcja(&mut o, "ustawienia, których SILNIK NIE CZYTA");
    let nieczytane = crate::settings_map::unmapped_keys(&ustaw);
    if nieczytane.is_empty() {
        let _ = writeln!(o, "  (brak — wszystkie klucze panelu docierają do silnika)");
    } else {
        for k in &nieczytane {
            let _ = writeln!(o, "  {k}");
        }
    }

    let core = crate::settings_map::core_from_ui(&ustaw);
    podsekcja(
        &mut o,
        "ustawienia MARTWE (mają wartość, ale wyłącznik nadrzędny jest zgaszony)",
    );
    let martwe = core.martwe_ustawienia();
    if martwe.is_empty() {
        let _ = writeln!(o, "  (brak)");
    } else {
        for m in &martwe {
            let _ = writeln!(o, "  {m:?}");
        }
    }
    podsekcja(&mut o, "PUŁAPKI konfiguracji");
    let pulapki = core.pulapki_konfiguracji();
    if pulapki.is_empty() {
        let _ = writeln!(o, "  (brak)");
    } else {
        for p in &pulapki {
            let _ = writeln!(o, "  ! {p}");
        }
    }

    // Czwarta diagnostyka: pola SPRZECZNE ZE SOBĄ. Trzy powyższe patrzą na
    // pole pojedynczo (nieczytane / zbramkowane / mylące), ta na PARY —
    // runner bez celu i bez zapadki, straż, która nic nie zamyka. Dokładamy
    // też cztery martwe pułapy AKTYWNEGO łańcucha (D27), bo `martwe_ustawienia`
    // rdzenia obejmuje wyłącznie pola `Settings`, a `PulapyGlobalne` to osobna
    // struktura i nikt ich nie sprawdzał.
    podsekcja(
        &mut o,
        "USTAWIENIA WEWNĘTRZNIE SPRZECZNE (bramka spójności)",
    );
    let mut niespojne = crate::bramka_spojnosci::sprawdz(&core);
    if let Some(a) = snap.aktywny_lancuch() {
        niespojne.extend(crate::bramka_spojnosci::sprawdz_pulapy(&a.pulapy));
    }
    if niespojne.is_empty() {
        let _ = writeln!(o, "  (brak)");
    } else {
        for n in &niespojne {
            let _ = writeln!(o, "  ! {n}");
        }
    }

    // ---------- 3. kanały ----------
    naglowek(&mut o, "3. OBSERWOWANE KANAŁY");
    if let Some(b) = v.get("bindings").and_then(|x| x.as_object()) {
        if b.is_empty() {
            let _ = writeln!(
                o,
                "  (żaden kanał nie jest powiązany — bot nie ma skąd brać sygnałów)"
            );
        }
        for (id, d) in b {
            let _ = writeln!(
                o,
                "  {id:16} monitorowany={} notify={} formaty={} tematy={}",
                pole(d, "monitored"),
                pole(d, "notify"),
                pole(d, "formats"),
                pole(d, "topics")
            );
        }
    }

    // ---------- 4. stan ----------
    naglowek(&mut o, "4. STAN RACHUNKU W CHWILI ZRZUTU");
    let _ = writeln!(o, "  saldo         : {}", pole(&v, "balance"));
    let _ = writeln!(o, "  wstrzymanie   : {}", pole(&v, "halt"));
    let _ = writeln!(o, "  nadpisanie ryz: {}", pole(&v, "riskOverride"));
    let _ = writeln!(o, "  statystyki    : {}", pole(&v, "stats"));
    let _ = writeln!(
        o,
        "  OBCE (nie nasze pozycje/zlecenia): {}",
        pole(&v, "foreign")
    );

    for (klucz, tytul) in [
        ("positions", "POZYCJE OTWARTE"),
        ("pendings", "ZLECENIA OCZEKUJĄCE"),
        ("baskets", "KOSZYKI"),
    ] {
        podsekcja(&mut o, tytul);
        match v.get(klucz).and_then(|x| x.as_array()) {
            Some(a) if !a.is_empty() => {
                for x in a {
                    let _ = writeln!(o, "  {}", serde_json::to_string(x).unwrap_or_default());
                }
            }
            _ => {
                let _ = writeln!(o, "  (puste)");
            }
        }
    }

    // ---------- 5. wiadomości z pamięci ----------
    naglowek(
        &mut o,
        "5. WIADOMOŚCI Z KANAŁÓW — CO BOT ZOBACZYŁ (pamięć procesu)",
    );
    let _ = writeln!(
        o,
        "  Surowa treść razem z wynikiem rozbioru. Jeśli bot zignorował sygnał,\n  \
         powód znajdziesz w sekcji 6 po tym samym `msg_id`.\n  \
         ⚠ To jest OKNO PAMIĘCI PROCESU — po restarcie jest puste. Pełny,\n  \
         trwały zapis strumienia leży w KRONICE i w ARCHIWUM WIADOMOŚCI\n  \
         (sekcja 6), i to tam należy szukać czegokolwiek sprzed restartu."
    );
    let mut poz = Pozycja::nowa(
        "messages",
        "wiadomości w pamięci procesu",
        wybor.chce("messages", false),
    );
    if !poz.wlaczone {
        let _ = writeln!(o, "  (POMINIĘTE — odznaczone w panelu)");
        poz.uwaga = "odznaczone w panelu".into();
    } else {
        match v.get("messages").and_then(|x| x.as_array()) {
            Some(a) if !a.is_empty() => {
                for m in a {
                    poz.rekordow += 1;
                    poz.zakres(m.get("t").and_then(|x| x.as_i64()).unwrap_or(0));
                    let _ = writeln!(o, "\n  ── wiadomość {} ──", pole(m, "id"));
                    let _ = writeln!(o, "  czas   : {}", pole(m, "ts"));
                    let _ = writeln!(
                        o,
                        "  kanał  : {} ({})",
                        pole(m, "channelId"),
                        pole(m, "channel")
                    );
                    let _ = writeln!(o, "  status : {}", pole(m, "status"));
                    let _ = writeln!(o, "  rozbiór: {}", pole(m, "parsed"));
                    let _ = writeln!(o, "  treść  :");
                    for l in pole(m, "text").lines() {
                        let _ = writeln!(o, "    | {l}");
                    }
                }
            }
            _ => {
                let _ = writeln!(o, "  (brak wiadomości w pamięci procesu)");
                poz.uwaga = "pamięć procesu pusta (świeży start?)".into();
            }
        }
    }
    spis.push(poz);

    let zrodla_plikowe = zrodla_liniowe(st, &wybor);
    let bajty_razem: u64 = zrodla_plikowe
        .iter()
        .map(|z| bajty(&z.pliki))
        .sum::<u64>()
        .max(1);
    let mut bajty_zrobione: u64 = 0;

    naglowek(
        &mut o,
        if chronologicznie {
            "6. STRUMIENIE Z DYSKU — PRZEPLOT CHRONOLOGICZNY"
        } else {
            "6. STRUMIENIE Z DYSKU — ŹRÓDŁO PO ŹRÓDLE"
        },
    );
    let _ = writeln!(
        o,
        "  Jedno zdarzenie na linię, w postaci JSON, tak jak zapisał je bot.\n  \
         Dziennik decyzji: `kind` (co się stało), `reason` (dlaczego, przy\n  \
         odmowach), `market` (migawka rynku W CHWILI DECYZJI), `basket`,\n  \
         `ticket`, `msg_id`. Kronika i archiwum: `rodzaj`/`event`, `chat`,\n  \
         `msg_id`, `edit_of`, `text` — z KAŻDĄ wersją wiadomości osobno.\n  \
         Każda linia jest poprzedzona znacznikiem źródła w nawiasach\n  \
         kwadratowych, żeby dało się ją odsiać jednym `findstr`."
    );
    if chronologicznie {
        let _ = writeln!(
            o,
            "\n  PRZEPLOT: linie ze wszystkich źródeł są posortowane po czasie\n  \
             zdarzenia. Wiersze bez czytelnego znacznika czasu (0) idą na\n  \
             koniec — wymienione osobno, żeby nie udawały, że stało się to\n  \
             w 1970 roku."
        );
    }

    podsekcja(
        &mut o,
        "MANIFEST PLIKÓW ŹRÓDŁOWYCH — SHA-256 + CZAS MODYFIKACJI",
    );
    let _ = writeln!(
        o,
        "  Każdy wiersz sekcji 6 wskazuje niżej plik i numer linii. SHA-256\n  \
         pozwala potem dowieść, że analizowano dokładnie te same bajty."
    );
    let mut manifest_n = 0usize;
    for z in &zrodla_plikowe {
        if !z.wlaczone {
            continue;
        }
        for p in &z.pliki {
            manifest_n += 1;
            let nazwa = p
                .file_name()
                .map(|x| x.to_string_lossy())
                .unwrap_or_default();
            let meta = std::fs::metadata(p).ok();
            let hash = sha256_pliku(p).unwrap_or_else(|e| format!("BŁĄD: {e}"));
            let _ = writeln!(
                o,
                "  [{:12}] plik={}  bajty={}  zmieniono_ms={}  sha256={hash}",
                z.znacznik,
                nazwa,
                meta.as_ref().map(|m| m.len()).unwrap_or(0),
                zmieniono_ms(p)
            );
        }
    }
    if manifest_n == 0 {
        let _ = writeln!(o, "  (brak włączonych plików źródłowych)");
    }

    let mut strumien: Vec<Linia> = Vec::new();
    let mut kolejnosc = 0u64;
    for z in &zrodla_plikowe {
        let mut poz = Pozycja::nowa(z.klucz, z.tytul.clone(), z.wlaczone);
        poz.bajtow = bajty(&z.pliki);
        if !z.wlaczone {
            poz.uwaga = "odznaczone w panelu".into();
            if !chronologicznie {
                podsekcja(
                    &mut o,
                    &format!("{} — POMINIĘTE (odznaczone w panelu)", z.tytul),
                );
            }
            spis.push(poz);
            continue;
        }
        if z.pliki.is_empty() {
            poz.uwaga = format!("brak plików w {}", z.katalog);
            if !chronologicznie {
                podsekcja(
                    &mut o,
                    &format!("{} — brak plików ({})", z.tytul, z.katalog),
                );
            }
            spis.push(poz);
            continue;
        }
        if !chronologicznie {
            podsekcja(
                &mut o,
                &format!("{} ({}, {})", z.tytul, z.katalog, mb(poz.bajtow)),
            );
        }
        for p in &z.pliki {
            let nazwa = p
                .file_name()
                .map(|x| x.to_string_lossy().to_string())
                .unwrap_or_default();
            if postep.is_some() && ANULUJ.load(Ordering::Relaxed) {
                anyhow::bail!("scalanie anulowane przez użytkownika (na pliku {nazwa})");
            }
            // Meldunek PRZED czytaniem pliku: gdyby któryś okazał się ogromny,
            // użytkownik ma widzieć, na czym stanęło, a nie ostatni ukończony.
            if let Some(start) = postep {
                melduj(
                    st,
                    &format!("{} · {nazwa}", z.tytul),
                    bajty_zrobione,
                    bajty_razem,
                    start,
                );
            }
            bajty_zrobione += std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            let Ok(tresc) = std::fs::read_to_string(p) else {
                poz.uwaga = format!("pliku {nazwa} nie dało się odczytać");
                continue;
            };
            if !chronologicznie {
                let _ = writeln!(o, "\n  ── plik: {nazwa} ──");
            }
            for (nr0, linia) in tresc.lines().enumerate() {
                if linia.trim().is_empty() {
                    continue;
                }
                let nr = nr0 as u64 + 1;
                poz.rekordow += 1;
                let parsed = serde_json::from_str::<Value>(linia).ok();
                let ms = parsed.as_ref().map(czas_wiersza).unwrap_or(0);
                let seq = parsed.as_ref().map(|v| seq_wiersza(v, nr)).unwrap_or(nr);
                poz.zakres(ms);
                if chronologicznie {
                    kolejnosc += 1;
                    strumien.push(Linia {
                        ms,
                        seq,
                        priorytet: z.priorytet,
                        kolejnosc,
                        zrodlo: z.znacznik,
                        plik: nazwa.clone(),
                        nr,
                        tresc: linia.to_string(),
                    });
                } else {
                    let _ = writeln!(o, "  [{}] [PROV {nazwa}:{nr}] {linia}", z.znacznik);
                }
            }
        }
        spis.push(poz);
    }

    // Wszystkie strumienie wczytane — pasek ma to POKAZAĆ niezależnie od trybu.
    // Wcześniej meldunek domykający siedział wyłącznie w gałęzi przeplotu,
    // więc przy scalaniu „źródło po źródle" pasek zatrzymywał się tuż pod
    // setką i wyglądał jak zawieszony na ostatnim pliku.
    if let Some(start) = postep {
        melduj(
            st,
            if chronologicznie {
                "przeplot chronologiczny"
            } else {
                "składanie dokumentu"
            },
            bajty_razem,
            bajty_razem,
            start,
        );
    }
    if chronologicznie {
        // Stabilne sortowanie po czasie: linie z tej samej milisekundy
        // zachowują kolejność, w jakiej stoją w swoich plikach. Bez tego
        // edycja potrafiłaby wyprzedzić wiadomość, którą poprawia.
        let (mut z_czasem, bez_czasu): (Vec<Linia>, Vec<Linia>) =
            strumien.into_iter().partition(|l| l.ms > 0);
        z_czasem.sort_by_key(|l| (l.ms, l.priorytet, l.seq, l.kolejnosc));
        let _ = writeln!(
            o,
            "\n  wierszy w przeplocie: {}",
            z_czasem.len() + bez_czasu.len()
        );
        for l in &z_czasem {
            let _ = writeln!(
                o,
                "  {}  [{}] [PROV {}:{}] {}",
                czas(l.ms),
                l.zrodlo,
                l.plik,
                l.nr,
                l.tresc
            );
        }
        if !bez_czasu.is_empty() {
            podsekcja(
                &mut o,
                &format!("bez czytelnego znacznika czasu ({})", bez_czasu.len()),
            );
            for l in &bez_czasu {
                let _ = writeln!(o, "  [{}] [PROV {}:{}] {}", l.zrodlo, l.plik, l.nr, l.tresc);
            }
        }
    }

    let zdarzen: u64 = spis
        .iter()
        .filter(|p| p.klucz != "messages")
        .map(|p| p.rekordow)
        .sum();
    if zdarzen == 0 {
        let _ = writeln!(
            o,
            "\n  (BRAK ZDARZEŃ Z DYSKU. Jeśli bot pracował, to znaczy, że dziennik\n   \
             i kronika są wyłączone w ustawieniach albo ich katalogi są puste —\n   \
             i wtedy tego przebiegu NIE DA SIĘ zbadać. Sprawdź to w pierwszej\n   \
             kolejności, zanim zaczniesz szukać czegokolwiek innego.)"
        );
    }

    // ---------- 7. log panelu ----------
    naglowek(
        &mut o,
        "7. DZIENNIK PANELU — POŁĄCZENIA, BŁĘDY, ZDARZENIA CYKLU ŻYCIA",
    );
    // Kategorie logu panelu odpowiadają polom wyboru jeden do jednego:
    // odznaczenie `commands` ma wyciąć komendy, a nie cały dziennik.
    let mut poz = Pozycja::nowa("events", "dziennik panelu", true);
    let mut odsiane = 0u64;
    match v.get("logs").and_then(|x| x.as_array()) {
        Some(a) if !a.is_empty() => {
            for l in a {
                let kat = pole(l, "category");
                // Kategoria, dla której nie ma pola wyboru (`telegram`,
                // `settings`, `kronika`…), jedzie pod kluczem `events` —
                // inaczej dołożenie kategorii w silniku po cichu wycinałoby
                // ją ze zrzutu.
                let klucz = if MERGE_KLUCZE.contains(&kat.as_str()) {
                    kat.clone()
                } else {
                    "events".to_string()
                };
                let wrazliwy = klucz == "session_string" || klucz == "smtp";
                if !wybor.chce(&klucz, wrazliwy) {
                    odsiane += 1;
                    continue;
                }
                let t = l.get("t").and_then(|x| x.as_i64()).unwrap_or(0);
                poz.rekordow += 1;
                poz.zakres(t);
                let _ = writeln!(
                    o,
                    "  {}  [{:9}] [{:5}] {}",
                    czas(t),
                    kat,
                    pole(l, "level"),
                    pole(l, "title")
                );
                // LogEntry serializes the actual diagnostic body as `content`.
                // Reading `detail` silently discarded MT5/configuration errors
                // and made the exported title look like the whole evidence.
                // Keep the old field only as a compatibility fallback.
                let d = l
                    .get("content")
                    .and_then(serde_json::Value::as_str)
                    .filter(|s| !s.is_empty())
                    .or_else(|| l.get("detail").and_then(serde_json::Value::as_str))
                    .unwrap_or("");
                if d != "—" && !d.is_empty() {
                    for lin in d.lines() {
                        let _ = writeln!(o, "                          | {lin}");
                    }
                }
            }
        }
        _ => {
            let _ = writeln!(o, "  (pusty)");
            poz.uwaga = "dziennik panelu pusty".into();
        }
    }
    if odsiane > 0 {
        let _ = writeln!(
            o,
            "\n  ({odsiane} wpisów ODSIANYCH — kategorie odznaczone w panelu)"
        );
        poz.uwaga = format!("{odsiane} wpisów odsianych przez pola wyboru");
    }
    spis.push(poz);

    // ---------- 8/9. historia ----------
    let chce_trades = wybor.chce("trades", false);
    for (klucz, kl_spis, tytul, nazwa_w_spisie) in [
        (
            "closed",
            "trades",
            "8. HISTORIA POZYCJI ZAMKNIĘTYCH",
            "historia pozycji zamkniętych",
        ),
        (
            "pendingHistory",
            "trades",
            "9. HISTORIA ZLECEŃ",
            "historia zleceń",
        ),
    ] {
        naglowek(&mut o, tytul);
        let mut poz = Pozycja::nowa(kl_spis, nazwa_w_spisie.to_string(), chce_trades);
        if !chce_trades {
            let _ = writeln!(o, "  (POMINIĘTE — `trades` odznaczone w panelu)");
            poz.uwaga = "odznaczone w panelu".into();
            spis.push(poz);
            continue;
        }
        match v.get(klucz).and_then(|x| x.as_array()) {
            Some(a) if !a.is_empty() => {
                for x in a {
                    poz.rekordow += 1;
                    poz.zakres(czas_wiersza(x));
                    let _ = writeln!(o, "  {}", serde_json::to_string(x).unwrap_or_default());
                }
            }
            _ => {
                let _ = writeln!(o, "  (puste)");
                poz.uwaga = "brak historii w pamięci procesu".into();
            }
        }
        spis.push(poz);
    }

    // ---------- 10. stan na dysku ----------
    naglowek(&mut o, "10. STAN I KONFIGURACJA NA DYSKU");
    let _ = writeln!(
        o,
        "  Pliki, które bot trzyma OBOK siebie. Sekcje 1–4 pokazują to, co bot\n  \
         ma w PAMIĘCI; tutaj jest to, co naprawdę leży na dysku — i rozjazd\n  \
         między jednym a drugim jest sam w sobie diagnozą."
    );
    for zp in zrodla_plikowe_json(st) {
        let mut poz = Pozycja::nowa(
            zp.klucz,
            zp.tytul.clone(),
            wybor.chce(zp.klucz, zp.wrazliwe),
        );
        podsekcja(&mut o, &zp.tytul);
        if !poz.wlaczone {
            let _ = writeln!(
                o,
                "  (POMINIĘTE — {} w panelu){}",
                if zp.wrazliwe {
                    "niezaznaczone"
                } else {
                    "odznaczone"
                },
                if zp.wrazliwe {
                    " ⚠ dane logowania"
                } else {
                    ""
                }
            );
            poz.uwaga = "odznaczone w panelu".into();
            spis.push(poz);
            continue;
        }
        if zp.pliki.is_empty() {
            let _ = writeln!(o, "  (brak — {})", zp.katalog);
            poz.uwaga = format!("brak plików w {}", zp.katalog);
            spis.push(poz);
            continue;
        }
        for p in &zp.pliki {
            let nazwa = p
                .file_name()
                .map(|x| x.to_string_lossy().to_string())
                .unwrap_or_default();
            let rozmiar = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            poz.bajtow += rozmiar;
            poz.rekordow += 1;
            if zp.tylko_spis {
                let _ = writeln!(o, "  {nazwa:44} {}", mb(rozmiar));
                continue;
            }
            let _ = writeln!(o, "\n  ── {nazwa} ({}) ──", mb(rozmiar));
            match std::fs::read_to_string(p) {
                Ok(t) => {
                    for l in t.lines() {
                        let _ = writeln!(o, "  {l}");
                    }
                }
                Err(e) => {
                    let _ = writeln!(o, "  (nie dało się odczytać: {e})");
                    poz.uwaga = format!("{nazwa}: {e}");
                }
            }
        }
        if zp.tylko_spis {
            let _ = writeln!(
                o,
                "\n  (sam spis — treść tych plików jest maszynowa i mierzy się\n   \
                 w megabajtach; są na dysku pod powyższymi nazwami)"
            );
        }
        spis.push(poz);
    }

    // Capture is exported separately from timestamp-sorted diagnostics: replay
    // requires its complete bootstrap and original sequence, even across days.
    spis.push(capture_export::append(
        &mut o,
        &st.workspace.logs_dir(),
        wybor.chce("replay_capture", false),
        postep.is_some(),
    )?);

    // ---------- 11. stopka ----------
    naglowek(&mut o, "11. CZEGO W TYM PLIKU NIE MA");
    let _ = writeln!(
        o,
        "  * **poświadczeń** — `secrets.json` (klucz sesji Telegrama, hasło\n    \
           MT5) i hasło SMTP nie trafiają tu NIGDY, także przy zaznaczonym\n    \
           polu `session_string`. Zrzut diagnostyczny wysyła się mailem\n    \
           i wkleja na czat; sesja Telegrama w takim pliku to przejęcie konta.\n    \
           W spisie źródeł widać wyłącznie, CZY poświadczenia istnieją;\n  \
         * `logs/alllogs_*.txt` — poprzednie zrzuty (plik nie zawiera sam\n    \
           siebie w kółko);\n  \
         * surowego strumienia `tracing` ze stdout — jeśli uruchamiasz bota\n    \
           przez `autostart.bat`, przekieruj go do pliku i dołącz osobno.\n\n  \
         Statystyki zrzutu: rekordów z dysku {zdarzen}, źródeł w spisie {}, \
         znaków {}.",
        spis.len(),
        o.len()
    );

    // ---------- nagłówek + SPIS ŹRÓDEŁ na GÓRZE ----------
    let mut czolo = String::with_capacity(4096);
    let _ = writeln!(czolo, "{}", "═".repeat(SZER));
    let _ = writeln!(czolo, "  CONDUIT — ZRZUT DIAGNOSTYCZNY");
    let _ = writeln!(czolo, "{}", "═".repeat(SZER));
    let _ = writeln!(
        czolo,
        "  wygenerowano  : {}  (czas lokalny maszyny)",
        czas(teraz)
    );
    let _ = writeln!(czolo, "  wersja        : {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(czolo, "  katalog danych: {}", st.workspace.root.display());
    let _ = writeln!(
        czolo,
        "  scalanie      : {}",
        if chronologicznie {
            "chronologiczne (przeplot źródeł)"
        } else {
            "źródło po źródle"
        }
    );
    let _ = writeln!(
        czolo,
        "\n  Ten plik ma być SAMOWYSTARCZALNY. Jeśli czegoś w nim brakuje do\n  \
         zrozumienia zachowania bota — to jest błąd tego eksportu, nie Twój."
    );
    naglowek(
        &mut czolo,
        "0. SPIS ŹRÓDEŁ — CO JEST W TYM PLIKU, A CZEGO NIE MA",
    );
    let _ = writeln!(
        czolo,
        "  Tabelka istnieje po to, żeby dało się zadać pytanie „czego tu nie ma”.\n  \
         Bez niej źródło puste, źródło odznaczone i źródło zapomniane przez\n  \
         programistę wyglądają dokładnie tak samo.\n"
    );
    let _ = writeln!(
        czolo,
        "  {:<34} {:>3} {:>10} {:>9}  {}",
        "źródło", "we?", "rekordów", "rozmiar", "zakres czasu / uwaga"
    );
    let _ = writeln!(czolo, "  {}", "─".repeat(SZER - 2));
    let mut razem_rek = 0u64;
    let mut razem_baj = 0u64;
    let mut weszlo = 0u64;
    for p in &spis {
        if p.wlaczone {
            weszlo += 1;
            razem_rek += p.rekordow;
            razem_baj += p.bajtow;
        }
        let ogon = if p.uwaga.is_empty() {
            p.opis_zakresu()
        } else {
            p.uwaga.clone()
        };
        let _ = writeln!(
            czolo,
            "  {:<34} {:>3} {:>10} {:>9}  {}",
            skroc(&p.tytul, 34),
            if p.wlaczone { "TAK" } else { "—" },
            p.rekordow,
            mb(p.bajtow),
            ogon
        );
    }
    let _ = writeln!(czolo, "  {}", "─".repeat(SZER - 2));
    let _ = writeln!(
        czolo,
        "  {:<34} {:>3} {:>10} {:>9}",
        "RAZEM (włączone)",
        weszlo,
        razem_rek,
        mb(razem_baj)
    );
    let _ = writeln!(
        czolo,
        "\n  Źródło z „—” w kolumnie „we?” NIE JEST w tym pliku. Zaznacza się je\n  \
         w panelu: Logi i raporty → Scalanie do alllogs.txt."
    );
    o.insert_str(0, &czolo);

    if postep.is_some() {
        let pominietych = spis.len() as u64 - weszlo;
        st.update_transient(Sections::one(Section::Scalanie), |s| {
            s.scalanie.zrodel = weszlo;
            s.scalanie.pominietych = pominietych;
        });
    }

    Ok(o)
}

/// Buduje i zapisuje. Zwraca ścieżkę oraz rozmiar w znakach.
pub fn zapisz(st: &StateHandle) -> Result<(PathBuf, usize)> {
    let tresc = zbuduj_z_postepem(st, None)?;
    let sciezka = zapisz_do_celu(st, &tresc)?;
    Ok((sciezka, tresc.len()))
}

fn zapisz_z_postepem(st: &StateHandle, start: std::time::Instant) -> Result<(PathBuf, usize)> {
    let tresc = zbuduj_z_postepem(st, Some(start))?;
    melduj(
        st,
        "zapis pliku",
        tresc.len() as u64,
        tresc.len() as u64,
        start,
    );
    let sciezka = zapisz_do_celu(st, &tresc)?;
    Ok((sciezka, tresc.len()))
}

pub fn katalog_docelowy(st: &StateHandle) -> Option<PathBuf> {
    st.read(|s| {
        s.settings
            .get("alllogs_dir")
            .and_then(|v| v.as_str())
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .map(PathBuf::from)
    })
}

/// Katalog istnieje i DA SIĘ w nim pisać — dowód plikiem-sondą, nie
/// metadanymi (ACL na Windows potrafią kłamać w metadanych).
pub fn sprawdz_katalog(dir: &std::path::Path) -> Result<()> {
    if !dir.is_dir() {
        anyhow::bail!("katalog docelowy nie istnieje: {}", dir.display());
    }
    let sonda = dir.join(format!(".conduit-sonda-{}", std::process::id()));
    std::fs::write(&sonda, b"sonda")
        .map_err(|e| anyhow::anyhow!("katalog {} nie jest zapisywalny: {e}", dir.display()))?;
    let _ = std::fs::remove_file(&sonda);
    Ok(())
}

fn zapisz_do_celu(st: &StateHandle, tresc: &str) -> Result<PathBuf> {
    match katalog_docelowy(st) {
        Some(dir) => {
            sprawdz_katalog(&dir)?;
            let nazwa = format!("alllogs_{}.txt", crate::store::stamp(crate::now_ms()));
            let cel = dir.join(nazwa);
            // .tmp + rename: anulowanie albo błąd NIGDY nie zostawia
            // częściowego pliku pod docelową nazwą.
            let tmp = cel.with_extension("tmp");
            std::fs::write(&tmp, tresc.as_bytes())?;
            std::fs::rename(&tmp, &cel)?;
            Ok(cel)
        }
        None => st
            .workspace
            .merge_logs(&[tresc.to_string()], crate::now_ms()),
    }
}

#[cfg(test)]
mod testy {
    use super::*;

    fn stan(tag: &str) -> StateHandle {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "conduit-alllogs-{tag}-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let cfg = crate::ServerConfig {
            workspace: dir,
            ..Default::default()
        };
        crate::bootstrap(&cfg, crate::default_auth()).unwrap()
    }

    #[test]
    fn panel_log_keeps_actual_content_and_halt_snapshot() {
        let st = stan("panel-content");
        st.log(
            "settings", "error", "USTAWIENIA TESTOWE",
            "SZCZEGOL-DIAGNOZY: account mismatch\nSZCZEGOL-DRUGA-LINIA",
        );
        st.update(crate::coalesce::Sections::one(crate::coalesce::Section::Halt), |s| {
            s.halt.ustaw(crate::ui::KlasaHaltu::Diagnoza, "TEST-HALT-DIAGNOZA");
        });
        let report = zbuduj(&st);
        assert!(report.contains("USTAWIENIA TESTOWE"));
        assert!(report.contains("SZCZEGOL-DIAGNOZY: account mismatch"));
        assert!(report.contains("| SZCZEGOL-DRUGA-LINIA"));
        assert!(report.contains("TEST-HALT-DIAGNOZA"));
        assert!(report.contains("\"active\":true"));
    }

    #[test]
    fn dokument_ma_wszystkie_sekcje() {
        let st = stan("sekcje");
        let d = zbuduj(&st);
        for s in [
            "0. SPIS ŹRÓDEŁ",
            "1. RACHUNEK I BROKER",
            "2. KONFIGURACJA SILNIKA",
            "3. OBSERWOWANE KANAŁY",
            "4. STAN RACHUNKU",
            "5. WIADOMOŚCI Z KANAŁÓW",
            "6. STRUMIENIE Z DYSKU",
            "7. DZIENNIK PANELU",
            "8. HISTORIA POZYCJI",
            "9. HISTORIA ZLECEŃ",
            "10. STAN I KONFIGURACJA NA DYSKU",
            "11. CZEGO W TYM PLIKU NIE MA",
        ] {
            assert!(d.contains(s), "brakuje sekcji: {s}");
        }
        // Spis źródeł MUSI wymieniać kronikę — to jest zgłoszenie, dla
        // którego cała ta sekcja powstała.
        assert!(
            d.contains("kronika (strumień Telegrama)"),
            "spis źródeł bez KRONIKI"
        );
        assert!(
            d.contains("archiwum wiadomości"),
            "spis źródeł bez archiwum wiadomości"
        );
    }

    #[test]
    fn scalenie_zawiera_kronike_archiwum_i_stan_z_dysku() {
        let st = stan("zrodla");
        let ws = &st.workspace;

        std::fs::create_dir_all(ws.archive_dir()).unwrap();
        std::fs::write(
            ws.archive_dir().join("wiadomosci-2026-08-04.jsonl"),
            "{\"v\":1,\"event\":\"edited\",\"received_at_ms\":1785000002000,\"text\":\"ZNACZNIK-ARCHIWUM\"}\n",
        )
        .unwrap();

        // kronika leży tam, gdzie mówią JEJ ustawienia — także poza `logs/`
        let plik_kroniki = ws.load_kronika().sciezka(&ws.root);
        std::fs::create_dir_all(plik_kroniki.parent().unwrap()).unwrap();
        std::fs::write(
            &plik_kroniki,
            "{\"v\":2,\"rodzaj\":\"nowa\",\"odebrano_ms\":1785000001000,\"text\":\"ZNACZNIK-KRONIKA\"}\n",
        )
        .unwrap();

        std::fs::create_dir_all(ws.backup_dir()).unwrap();
        std::fs::write(
            ws.backup_dir().join("2026-08-04_120000-1.json"),
            "{\"znacznik\":\"ZNACZNIK-BACKUP\"}",
        )
        .unwrap();
        std::fs::write(
            ws.root.join("koszyki.json"),
            "{\"znacznik\":\"ZNACZNIK-KOSZYKI\"}",
        )
        .unwrap();

        let d = zbuduj(&st);
        for znacznik in [
            "ZNACZNIK-KRONIKA",
            "ZNACZNIK-ARCHIWUM",
            "ZNACZNIK-BACKUP",
            "ZNACZNIK-KOSZYKI",
        ] {
            assert!(d.contains(znacznik), "scalenie zgubiło źródło: {znacznik}");
        }
    }

    #[test]
    fn odznaczone_zrodlo_naprawde_wypada_z_pliku() {
        let st = stan("wybor");
        let plik_kroniki = st.workspace.load_kronika().sciezka(&st.workspace.root);
        std::fs::create_dir_all(plik_kroniki.parent().unwrap()).unwrap();
        std::fs::write(
            &plik_kroniki,
            "{\"v\":2,\"rodzaj\":\"nowa\",\"odebrano_ms\":1785000001000,\"text\":\"ZNACZNIK-KRONIKA\"}\n",
        )
        .unwrap();

        assert!(
            zbuduj(&st).contains("ZNACZNIK-KRONIKA"),
            "domyślnie kronika MA wchodzić"
        );

        crate::commands::apply_settings_patch(
            &st,
            &serde_json::json!({ "merge_config": { "kronika": false } }),
        )
        .unwrap();
        let d = zbuduj(&st);
        assert!(
            !d.contains("ZNACZNIK-KRONIKA"),
            "odznaczone źródło nie ma prawa wejść"
        );
        // …ale spis MUSI o tym powiedzieć, bo inaczej brak treści wygląda
        // jak pusty plik kroniki
        assert!(
            d.contains("odznaczone w panelu"),
            "spis milczy o pominiętym źródle"
        );
    }

    /// Poświadczenia NIE WYCHODZĄ, nawet gdy ktoś zaznaczy pole.
    #[test]
    fn poswiadczenia_nigdy_nie_ida_do_zrzutu() {
        let st = stan("sekrety");
        std::fs::write(
            st.workspace.secrets_path(),
            "{\"telegram_session\":\"TAJNY-KLUCZ-SESJI\",\"mt5_password\":\"TAJNE-HASLO\"}",
        )
        .unwrap();
        crate::commands::apply_settings_patch(
            &st,
            &serde_json::json!({ "merge_config": { "session_string": true, "smtp": true } }),
        )
        .unwrap();
        let d = zbuduj(&st);
        assert!(
            !d.contains("TAJNY-KLUCZ-SESJI"),
            "klucz sesji Telegrama W ZRZUCIE"
        );
        assert!(!d.contains("TAJNE-HASLO"), "hasło MT5 W ZRZUCIE");
        assert!(
            d.contains("SAM FAKT ISTNIENIA"),
            "zrzut ma powiedzieć, że poświadczenia SĄ"
        );
    }

    /// PRZEPLOT CHRONOLOGICZNY — przełącznik istniał w panelu, a backend
    /// go nie czytał. Wiersze z dwóch źródeł mają wyjść w kolejności czasu,
    /// a nie „najpierw całe jedno źródło".
    #[test]
    fn przeplot_chronologiczny_uklada_zrodla_po_czasie() {
        let st = stan("przeplot");
        let ws = &st.workspace;
        std::fs::create_dir_all(ws.journal_dir()).unwrap();
        std::fs::create_dir_all(ws.archive_dir()).unwrap();
        // dziennik: t=1000 i t=3000; archiwum: t=2000 — przeplot MUSI je rozdzielić
        std::fs::write(
            ws.journal_dir().join("live-2026-08-04.jsonl"),
            "{\"ts\":1785000001000,\"kind\":\"A-PIERWSZE\"}\n{\"ts\":1785000003000,\"kind\":\"C-TRZECIE\"}\n",
        )
        .unwrap();
        std::fs::write(
            ws.archive_dir().join("wiadomosci-2026-08-04.jsonl"),
            "{\"received_at_ms\":1785000002000,\"text\":\"B-DRUGIE\"}\n",
        )
        .unwrap();

        crate::commands::apply_settings_patch(
            &st,
            &serde_json::json!({ "merge_chronological": true }),
        )
        .unwrap();
        let d = zbuduj(&st);
        let a = d.find("A-PIERWSZE").expect("brak wiersza dziennika");
        let b = d.find("B-DRUGIE").expect("brak wiersza archiwum");
        let c = d
            .find("C-TRZECIE")
            .expect("brak drugiego wiersza dziennika");
        assert!(
            a < b && b < c,
            "przeplot nie posortował po czasie: {a} {b} {c}"
        );

        // …a bez przeplotu każde źródło jedzie w całości, więc kolejność
        // jest odwrotna (całe A i C, potem B)
        crate::commands::apply_settings_patch(
            &st,
            &serde_json::json!({ "merge_chronological": false }),
        )
        .unwrap();
        let d = zbuduj(&st);
        let (a, b, c) = (
            d.find("A-PIERWSZE").unwrap(),
            d.find("B-DRUGIE").unwrap(),
            d.find("C-TRZECIE").unwrap(),
        );
        assert!(
            a < c && c < b,
            "bez przeplotu źródła mają iść blokami: {a} {b} {c}"
        );
    }

    /// Przy tej samej milisekundzie surowy odbiór Telegrama jest przyczyną,
    /// a decyzja silnika skutkiem. Stary stabilny sort zachowywał kolejność
    /// listy źródeł (journal przed kroniką) i odwracał ten związek.
    #[test]
    fn remis_czasu_ma_proweniencje_i_kolejnosc_przyczynowa() {
        let st = stan("proweniencja");
        let ws = &st.workspace;
        std::fs::create_dir_all(ws.journal_dir()).unwrap();
        let kronika = ws.load_kronika().sciezka(&ws.root);
        std::fs::create_dir_all(kronika.parent().unwrap()).unwrap();
        std::fs::write(
            &kronika,
            "{\"v\":2,\"seq\":7,\"rodzaj\":\"nowa\",\"odebrano_ms\":1785000001000,\"text\":\"RAW-PRZYCZYNA\"}\n",
        )
        .unwrap();
        std::fs::write(
            ws.journal_dir().join("live-2026-08-04.jsonl"),
            "{\"ts\":1785000001000,\"kind\":\"DECYZJA-SKUTEK\"}\n",
        )
        .unwrap();

        let d = zbuduj(&st);
        let raw = d.find("RAW-PRZYCZYNA").unwrap();
        let decyzja = d.find("DECYZJA-SKUTEK").unwrap();
        assert!(raw < decyzja, "decyzja wyprzedziła odebranie wiadomości");
        assert!(
            d.contains("[PROV kronika.jsonl:1]"),
            "brak pliku i numeru linii kroniki"
        );
        assert!(
            d.contains("sha256="),
            "manifest nie ma kryptograficznego odcisku"
        );
    }

    /// Zrzut MUSI zawierać pełną listę ustawień — bez niej nie da się
    /// odtworzyć, czym bot grał, a to jest jedyny powód istnienia tego pliku.
    #[test]
    fn zawiera_ustawienia_i_ich_liczbe() {
        let st = stan("ustawienia");
        crate::commands::apply_settings_patch(
            &st,
            &serde_json::json!({ "entry_units": 5, "session_hours": "8-16" }),
        )
        .unwrap();
        let d = zbuduj(&st);
        assert!(
            d.contains("wszystkich ustawień: 2"),
            "brak licznika ustawień"
        );
        assert!(
            d.contains("entry_units"),
            "brak konkretnego ustawienia silnika"
        );
        assert!(d.contains("session_hours"), "brak drugiego ustawienia");
    }

    #[test]
    fn pusta_konfiguracja_jest_widoczna() {
        let st = stan("pusto");
        let d = zbuduj(&st);
        assert!(
            d.contains("wszystkich ustawień: 0"),
            "zrzut ukrywa pustą konfigurację"
        );
    }

    #[test]
    fn postep_po_bajtach_i_anulowanie() {
        let st = stan("bajty");
        let dir = st.workspace.journal_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let linia = format!(
            "{{\"kind\":\"note\",\"text\":\"{}\"}}
",
            "x".repeat(180)
        );
        let plik = linia.repeat(11_000); // ~2 MB każdy
        for n in ["2026-08-01", "2026-08-02", "2026-08-03"] {
            std::fs::write(dir.join(format!("live-{n}.jsonl")), &plik).unwrap();
        }
        let razem: u64 = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter_map(|e| e.metadata().ok())
            .map(|m| m.len())
            .sum();
        assert!(razem > 5_000_000, "warsztat ma mieć >5 MB, jest {razem}");

        // pełny przebieg z postępem
        ANULUJ.store(false, Ordering::Relaxed);
        let start = std::time::Instant::now();
        let d = zbuduj_z_postepem(&st, Some(start)).expect("scalenie ma się udać");
        assert!(
            d.len() as u64 >= razem,
            "dokument nie może zgubić treści dziennika"
        );
        let (zrobione, wszystkich, predkosc) = st.read(|s| {
            (
                s.scalanie.zrobione,
                s.scalanie.wszystkich,
                s.scalanie.predkosc.clone(),
            )
        });
        assert!(
            wszystkich >= razem,
            "mianownik postępu = BAJTY wejścia ze wszystkich źródeł ({wszystkich} < {razem})"
        );
        assert_eq!(zrobione, wszystkich, "po zakończeniu licznik = całość");
        assert!(
            predkosc.is_empty() || predkosc.ends_with("MB/s"),
            "prędkość w MB/s, nie w plikach/s (jest: {predkosc})"
        );

        // anulowanie: flaga ustawiona → pętla przerywa się błędem „anulowane"
        ANULUJ.store(true, Ordering::Relaxed);
        let start2 = std::time::Instant::now();
        let e = zbuduj_z_postepem(&st, Some(start2));
        ANULUJ.store(false, Ordering::Relaxed);
        let opis = e.expect_err("anulowanie MUSI przerwać budowę").to_string();
        assert!(
            opis.contains("anulowane przez"),
            "błąd ma mówić o anulowaniu: {opis}"
        );
        // i żadnego pliku częściowego w logs (zapis idzie na samym końcu)
        let logs = st.workspace.logs_dir();
        let czesciowe = std::fs::read_dir(&logs)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().map(|x| x == "tmp").unwrap_or(false))
            .count();
        assert_eq!(czesciowe, 0, "anulowanie nie ma prawa zostawić .tmp");
    }

    /// SCALENIE Z DUŻĄ KRONIKĄ — próba na skalę, w której to się psuje.
    ///
    /// ⚠ `#[ignore]`: syntezuje ~40 MB i chodzi kilkanaście sekund, więc nie
    /// ma prawa siedzieć w zwykłym przebiegu. Uruchamianie:
    ///
    /// ```text
    /// cargo test -p conduit-server --target-dir target-kronika \
    ///     duza_kronika -- --ignored --nocapture
    /// ```
    ///
    /// Sprawdza trzy rzeczy naraz, bo przy tej wielkości każda z nich potrafi
    /// się urwać osobno: KOMPLETNOŚĆ (nie zgubić ani jednej linii),
    /// CHRONOLOGIĘ (przeplot naprawdę posortowany) i POSTĘP (mianownik
    /// w bajtach, licznik dochodzący do całości).
    #[test]
    #[ignore = "syntezuje ~40 MB — uruchamiać ręcznie: -- --ignored"]
    fn duza_kronika_wchodzi_w_calosci_i_w_kolejnosci() {
        let st = stan("duza");
        let ws = &st.workspace;
        let plik_kroniki = ws.load_kronika().sciezka(&ws.root);
        std::fs::create_dir_all(plik_kroniki.parent().unwrap()).unwrap();
        std::fs::create_dir_all(ws.journal_dir()).unwrap();

        // ~40 MB kroniki: 120 000 wierszy po ~330 B, czasy PARZYSTE
        const N: i64 = 120_000;
        let t0 = 1_785_000_000_000i64;
        let wypelniacz = "x".repeat(240);
        let mut kron = String::with_capacity(42 * 1024 * 1024);
        for i in 0..N {
            kron.push_str(&format!(
                "{{\"v\":2,\"seq\":{i},\"rodzaj\":\"nowa\",\"odebrano_ms\":{},\"chat_id\":-100,\"chat\":\"ATFX\",\"msg_id\":{i},\"text\":\"K{i} {wypelniacz}\"}}\n",
                t0 + i * 2
            ));
        }
        std::fs::write(&plik_kroniki, &kron).unwrap();

        // dziennik decyzji: czasy NIEPARZYSTE, żeby przeplot musiał je wpleść
        // POMIĘDZY wiersze kroniki, a nie dokleić blokiem
        let mut dzien = String::with_capacity(4 * 1024 * 1024);
        for i in 0..20_000i64 {
            dzien.push_str(&format!(
                "{{\"ts\":{},\"kind\":\"D{i}\",\"reason\":\"{wypelniacz}\"}}\n",
                t0 + i * 12 + 1
            ));
        }
        std::fs::write(ws.journal_dir().join("live-2026-08-04.jsonl"), &dzien).unwrap();

        let wejscie = kron.len() + dzien.len();
        assert!(
            wejscie > 40_000_000,
            "warsztat ma mieć >40 MB, ma {wejscie}"
        );

        crate::commands::apply_settings_patch(
            &st,
            &serde_json::json!({ "merge_chronological": true }),
        )
        .unwrap();

        ANULUJ.store(false, Ordering::Relaxed);
        let start = std::time::Instant::now();
        let d = zbuduj_z_postepem(&st, Some(start)).expect("scalenie ma się udać");
        let ms = start.elapsed().as_millis();
        println!("scalono {} MB w {ms} ms", d.len() / 1_048_576);
        // Spis źródeł na oczy — po to ten test uruchamia się ręcznie.
        for l in d.lines().take(40) {
            println!("{l}");
        }

        // --- KOMPLETNOŚĆ ---
        assert_eq!(
            d.matches("[KRONIKA]").count(),
            N as usize,
            "przeplot zgubił wiersze kroniki"
        );
        assert_eq!(
            d.matches("[DECYZJA]").count(),
            20_000,
            "przeplot zgubił wiersze dziennika"
        );
        assert!(d.contains("\"K0 "), "brak pierwszego wiersza kroniki");
        assert!(
            d.contains(&format!("\"K{} ", N - 1)),
            "brak ostatniego wiersza kroniki"
        );

        // --- CHRONOLOGIA --- pierwsze wiersze mają iść na przemian
        let k1 = d.find("\"K0 ").unwrap();
        let d1 = d.find("\"kind\":\"D0\"").unwrap();
        let k2 = d.find("\"K1 ").unwrap();
        assert!(k1 < d1 && d1 < k2, "przeplot nie przeplata: {k1} {d1} {k2}");

        // --- POSTĘP --- mianownik w bajtach, licznik na całości
        let (zrobione, wszystkich, predkosc, zrodel) = st.read(|s| {
            (
                s.scalanie.zrobione,
                s.scalanie.wszystkich,
                s.scalanie.predkosc.clone(),
                s.scalanie.zrodel,
            )
        });
        assert!(
            wszystkich as usize >= wejscie,
            "mianownik < wejścia: {wszystkich}"
        );
        assert_eq!(zrobione, wszystkich, "licznik ma dojść do całości");
        assert!(
            predkosc.ends_with("MB/s"),
            "prędkość w MB/s (jest: {predkosc})"
        );
        let _ = zrodel;

        // --- SPIS ŹRÓDEŁ --- liczby w nagłówku muszą się zgadzać z treścią
        assert!(
            d.contains(&format!("{N}")),
            "spis źródeł bez liczby rekordów kroniki"
        );
    }

    /// Katalog docelowy: sonda odrzuca nieistniejący, przyjmuje istniejący.
    #[test]
    fn walidacja_katalogu_docelowego() {
        assert!(sprawdz_katalog(std::path::Path::new("Q:/nie/ma/takiego")).is_err());
        let tmp = std::env::temp_dir();
        assert!(sprawdz_katalog(&tmp).is_ok());
    }
}
