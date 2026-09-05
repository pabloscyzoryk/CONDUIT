//! Zapis dziennika zdarzeń na dysk.
//!
//! Rdzeń produkuje zdarzenia ([`conduit_core::journal`]), ta warstwa je
//! utrwala. Podział jest celowy: rdzeń nie ma prawa dotknąć zegara ani pliku,
//! bo na tym stoi determinizm backtestu. Tutaj jedno i drugie jest na miejscu.
//!
//! Co dostajemy na dysku:
//!
//! ```text
//!   logs/journal/journal-2026-07-23.jsonl   ← jedna linia = jedno zdarzenie
//!   logs/journal/journal-2026-07-23.log     ← to samo dla oka
//! ```
//!
//! **Nazwa pliku zawiera DATĘ DOBY HANDLOWEJ SERWERA.** To nie jest ozdobnik:
//! poprzedni bot scalał sześć dni w jeden plik i sortował je po samej porze
//! dnia, przez co żadnego dnia nie dało się odtworzyć. Tutaj doba jest
//! JEDNOSTKĄ PLIKU, a nie sugestią.

use anyhow::{Context, Result};
use conduit_core::journal::JournalEvent;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub mod provenance;

/// Ustawienia zapisu.
#[derive(Debug, Clone)]
pub struct WriterConfig {
    /// pisać lustrzany plik tekstowy dla człowieka
    pub text_mirror: bool,
    /// po ilu dobach kasować stare pliki (0 = nigdy)
    pub retention_days: u32,
    /// strefa, w której renderujemy znacznik ŚCIENNY (`ts`)
    pub local_offset_ms: i64,
    /// przedrostek nazwy pliku
    pub prefix: String,
}

impl Default for WriterConfig {
    fn default() -> Self {
        WriterConfig {
            text_mirror: true,
            retention_days: 90,
            local_offset_ms: 0,
            prefix: "journal".into(),
        }
    }
}

/// Zapis atomowy: zapisz obok, potem podmień nazwę.
///
/// `rename` w obrębie jednego wolumenu jest operacją atomową, więc czytelnik
/// albo widzi starą zawartość, albo nową — nigdy połowy. Używamy tego do
/// plików, które powstają W CAŁOŚCI (raporty, podsumowania); do strumienia
/// zdarzeń służy dopisywanie, patrz [`JournalWriter::write`].
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("part")
    ));
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    // Windows nie pozwala nadpisać istniejącego pliku przez `rename`, więc
    // stary usuwamy tuż przed podmianą. Okno, w którym pliku nie ma, trwa
    // ułamek milisekundy i dotyczy wyłącznie plików generowanych w całości.
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Strumień zdarzeń zapisywany na dysk, z rotacją po dobie handlowej.
pub struct JournalWriter {
    dir: PathBuf,
    cfg: WriterConfig,
    /// doba, na którą mamy otwarte pliki (`YYYY-MM-DD`)
    day: String,
    jsonl: Option<File>,
    text: Option<File>,
    /// ile linii zapisano od startu — do diagnostyki
    pub written: u64,
    /// ile rotacji wykonano
    pub rotations: u64,
    pub last_path: Option<PathBuf>,
}

impl JournalWriter {
    pub fn new(dir: impl Into<PathBuf>, cfg: WriterConfig) -> Self {
        JournalWriter {
            dir: dir.into(),
            cfg,
            day: String::new(),
            jsonl: None,
            text: None,
            written: 0,
            rotations: 0,
            last_path: None,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Ścieżka pliku `.jsonl` dla danej doby.
    pub fn path_for(&self, day: &str) -> PathBuf {
        self.dir.join(format!("{}-{}.jsonl", self.cfg.prefix, day))
    }

    fn path_txt(&self, day: &str) -> PathBuf {
        self.dir.join(format!("{}-{}.log", self.cfg.prefix, day))
    }

    /// Przełącza pliki na wskazaną dobę. Wołane samo z siebie przy zapisie.
    fn rotate(&mut self, day: &str) -> Result<()> {
        if self.day == day && self.jsonl.is_some() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("nie udało się utworzyć {}", self.dir.display()))?;
        // domknięcie poprzedniej doby — dane muszą być na dysku, zanim
        // zaczniemy pisać do nowego pliku
        if let Some(f) = self.jsonl.take() {
            let _ = f.sync_all();
        }
        if let Some(f) = self.text.take() {
            let _ = f.sync_all();
        }
        let p = self.path_for(day);
        // `append` jest tu istotny: dopisywanie w tym trybie NIE przewija
        // pliku i nie gubi tego, co zapisała poprzednia sesja bota. Restart
        // procesu w środku dnia dopisuje do tej samej doby.
        let f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&p)
            .with_context(|| format!("nie udało się otworzyć {}", p.display()))?;
        self.jsonl = Some(f);
        if self.cfg.text_mirror {
            let pt = self.path_txt(day);
            self.text = Some(OpenOptions::new().create(true).append(true).open(&pt)?);
        }
        if !self.day.is_empty() {
            self.rotations += 1;
        }
        self.day = day.to_string();
        self.last_path = Some(p);
        self.sprzataj();
        Ok(())
    }

    /// Zapisuje partię zdarzeń. Zwraca liczbę zapisanych linii.
    ///
    /// `wall_utc_ms` to zegar ścienny procesu — rdzeń go nie zna, więc
    /// stemplujemy tutaj. Dwa zegary zostają rozdzielone: `ts` to czas
    /// maszyny, `ts_broker` to czas serwera brokera z ticka.
    pub fn write(&mut self, events: &mut [JournalEvent], wall_utc_ms: i64) -> Result<usize> {
        let mut n = 0usize;
        for ev in events.iter_mut() {
            ev.stamp_wall(wall_utc_ms, self.cfg.local_offset_ms);
            // Pusta doba znaczy „zdarzenie zbudowane poza silnikiem"; nie
            // zgadujemy daty, tylko dokładamy je do doby aktualnie otwartej.
            let day = if ev.session_day.is_empty() {
                self.day.clone()
            } else {
                ev.session_day.clone()
            };
            let day = if day.is_empty() {
                "0000-00-00".to_string()
            } else {
                day
            };
            self.rotate(&day)?;

            // JEDNA linia budowana w całości i wypychana JEDNYM zapisem.
            // Zapis w trybie dopisywania jest niepodzielny wobec innych
            // dopisujących, więc linie nigdy nie wchodzą sobie w środek —
            // nawet gdy pisze kilka procesów naraz.
            let mut linia = serde_json::to_string(ev)?;
            linia.push('\n');
            if let Some(f) = self.jsonl.as_mut() {
                f.write_all(linia.as_bytes())?;
            }
            if let Some(f) = self.text.as_mut() {
                let mut h = ev.human();
                h.push('\n');
                f.write_all(h.as_bytes())?;
            }
            n += 1;
        }
        if n > 0 {
            self.flush()?;
            self.written += n as u64;
        }
        Ok(n)
    }

    pub fn flush(&mut self) -> Result<()> {
        if let Some(f) = self.jsonl.as_mut() {
            f.flush()?;
        }
        if let Some(f) = self.text.as_mut() {
            f.flush()?;
        }
        Ok(())
    }

    /// Kasuje pliki starsze niż `retention_days` dób.
    ///
    /// Liczymy po DACIE W NAZWIE, nie po czasie modyfikacji pliku: skopiowanie
    /// katalogu na inną maszynę odświeża znaczniki systemu plików, a data
    /// doby handlowej jest niezmienna.
    fn sprzataj(&self) {
        let keep = self.cfg.retention_days;
        if keep == 0 || self.day.is_empty() {
            return;
        }
        let Ok(dzis) = chrono::NaiveDate::parse_from_str(&self.day, "%Y-%m-%d") else {
            return;
        };
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for wpis in rd.flatten() {
            let nazwa = wpis.file_name().to_string_lossy().to_string();
            let Some(reszta) = nazwa.strip_prefix(&format!("{}-", self.cfg.prefix)) else {
                continue;
            };
            let data = &reszta[..reszta.len().min(10)];
            let Ok(d) = chrono::NaiveDate::parse_from_str(data, "%Y-%m-%d") else {
                continue;
            };
            if (dzis - d).num_days() > keep as i64 {
                let _ = std::fs::remove_file(wpis.path());
            }
        }
    }
}

#[cfg(test)]
mod testy {
    use super::*;
    use conduit_core::journal::*;

    fn katalog(nazwa: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("conduit-journal-{nazwa}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    fn zdarzenie(dzien_ms: i64, tekst: &str) -> JournalEvent {
        let mut buf = JournalBuf::new(
            JournalConfig {
                enabled: true,
                min_level: EventLevel::Debug,
                server_offset_ms: 3 * 3_600_000,
                ..Default::default()
            },
            "t",
        );
        buf.push(
            Ev::new(
                dzien_ms,
                EventLevel::Info,
                EventCategory::Trade,
                EventKind::Note,
            )
            .text(tekst)
            .build(),
        );
        buf.drain().pop().unwrap()
    }

    /// Rotacja po dobie: dwa dni = dwa pliki, każdy z DATĄ w nazwie.
    #[test]
    fn rotacja_po_dobie_handlowej() {
        let dir = katalog("rotacja");
        let mut w = JournalWriter::new(&dir, WriterConfig::default());

        // 2026-07-23 10:00 i 2026-07-24 02:00 w zegarze serwera
        let d23 = 1_784_800_800_000i64; // 2026-07-23T10:00 (traktowane jako czas serwera)
        let d24 = d23 + 16 * 3_600_000; // przekracza północ serwera

        let mut a = [zdarzenie(d23, "dzień pierwszy")];
        let mut b = [zdarzenie(d24, "dzień drugi")];
        assert_eq!(w.write(&mut a, 1_784_800_800_000).unwrap(), 1);
        assert_eq!(w.write(&mut b, 1_784_858_400_000).unwrap(), 1);

        assert_eq!(w.rotations, 1, "powinna zajść dokładnie jedna rotacja");

        let p1 = dir.join("journal-2026-07-23.jsonl");
        let p2 = dir.join("journal-2026-07-24.jsonl");
        assert!(p1.exists(), "brak {}", p1.display());
        assert!(p2.exists(), "brak {}", p2.display());

        // każdy plik ma DOKŁADNIE swoje zdarzenie
        let t1 = std::fs::read_to_string(&p1).unwrap();
        let t2 = std::fs::read_to_string(&p2).unwrap();
        assert_eq!(t1.lines().count(), 1);
        assert_eq!(t2.lines().count(), 1);
        assert!(t1.contains("dzień pierwszy"));
        assert!(t2.contains("dzień drugi"));

        // lustro tekstowe powstało obok
        assert!(dir.join("journal-2026-07-23.log").exists());

        // i najważniejsze: linia niesie pełną datę ze strefą, w OBU zegarach
        let ev: JournalEvent = serde_json::from_str(t1.lines().next().unwrap()).unwrap();
        assert_eq!(ev.session_day, "2026-07-23");
        assert!(
            ev.ts_broker.starts_with("2026-07-23T10:00:00.000"),
            "{}",
            ev.ts_broker
        );
        assert!(ev.ts_broker.ends_with("+03:00"), "{}", ev.ts_broker);
        assert!(ev.ts.starts_with("2026-07-23T"), "{}", ev.ts);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Restart procesu w środku doby DOPISUJE, a nie kasuje.
    #[test]
    fn ponowne_otwarcie_dopisuje_do_tej_samej_doby() {
        let dir = katalog("dopisywanie");
        let d = 1_784_800_800_000i64;
        {
            let mut w = JournalWriter::new(&dir, WriterConfig::default());
            w.write(&mut [zdarzenie(d, "przed restartem")], d).unwrap();
        }
        {
            let mut w = JournalWriter::new(&dir, WriterConfig::default());
            w.write(&mut [zdarzenie(d, "po restarcie")], d).unwrap();
        }
        let t = std::fs::read_to_string(dir.join("journal-2026-07-23.jsonl")).unwrap();
        assert_eq!(t.lines().count(), 2, "restart nie może gubić linii");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Retencja kasuje po DACIE W NAZWIE, nie po czasie modyfikacji.
    #[test]
    fn retencja_kasuje_stare_doby() {
        let dir = katalog("retencja");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("journal-2026-01-01.jsonl"), b"{}\n").unwrap();
        std::fs::write(dir.join("journal-2026-07-22.jsonl"), b"{}\n").unwrap();

        let mut w = JournalWriter::new(
            &dir,
            WriterConfig {
                retention_days: 7,
                ..Default::default()
            },
        );
        let d = 1_784_800_800_000i64; // 2026-07-23
        w.write(&mut [zdarzenie(d, "dziś")], d).unwrap();

        assert!(
            !dir.join("journal-2026-01-01.jsonl").exists(),
            "stary plik powinien zniknąć"
        );
        assert!(
            dir.join("journal-2026-07-22.jsonl").exists(),
            "wczorajszy ma zostać"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Zapis atomowy podmienia zawartość w całości.
    #[test]
    fn zapis_atomowy_podmienia_calosc() {
        let dir = katalog("atomowy");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("raport.json");
        write_atomic(&p, b"{\"a\":1}").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{\"a\":1}");
        write_atomic(&p, b"{\"a\":2}").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{\"a\":2}");
        // plik tymczasowy nie zostaje po sobie
        assert!(!dir.join("raport.json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
