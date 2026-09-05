
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Wersja schematu wiersza. Analizator sprawdza ją przed liczeniem.
pub const ARCHIVE_SCHEMA: u32 = 1;

/// Co się stało z wiadomością.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MsgEvent {
    /// pierwsza wersja — tak, jak przyszła
    Received,
    /// sygnalista poprawił treść; `edit_of` wskazuje oryginał
    Edited,
    /// wiadomość zniknęła z kanału
    Deleted,
}

impl MsgEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            MsgEvent::Received => "received",
            MsgEvent::Edited => "edited",
            MsgEvent::Deleted => "deleted",
        }
    }
}

/// Jeden wiersz archiwum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MsgRecord {
    pub v: u32,
    /// numer w obrębie uruchomienia — porządkuje zdarzenia z tej samej milisekundy
    pub seq: u64,
    pub event: MsgEvent,

    /// **Nasz** czas odbioru — jedyny znacznik, którego Telegram nie przepisuje.
    /// Przy edycji to jest moment, w którym MY zobaczyliśmy nową treść.
    pub received_at: String,
    pub received_at_ms: i64,

    /// Znacznik od Telegrama: `date` dla nowej, `edit_date` dla edycji.
    /// Przy edycji potrafi być STARSZY niż `received_at` — i to jest właśnie
    /// powód, dla którego oba pola muszą istnieć osobno.
    pub msg_ts_ms: i64,

    pub chat_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_id: Option<i64>,
    pub msg_id: i64,
    /// KLUCZOWE dla kanałów typu ATFX: zarządzają pozycją, ODPOWIADAJĄC na
    /// wiadomość z sygnałem. Bez tego numeru komunikatu „TP1 HIT" nie da się
    /// przypisać do koszyka.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<i64>,
    /// ustawione, gdy wiersz jest edycją — wskazuje numer poprawianej wiadomości
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit_of: Option<i64>,

    pub source_name: String,
    pub text: String,
    pub monitored: bool,
}

/// Zapis archiwum: jeden plik na dobę, dopisywanie, bez nadpisywania.
pub struct MessageArchive {
    dir: PathBuf,
    day: String,
    file: Option<File>,
    seq: u64,
    pub written: u64,
    pub rotations: u64,
    /// po ilu dobach kasować stare pliki (0 = nigdy)
    pub retention_days: u32,
    /// strefa, w której liczymy dobę i renderujemy `received_at`
    pub local_offset_ms: i64,
}

impl MessageArchive {
    pub fn new(dir: impl Into<PathBuf>, local_offset_ms: i64, retention_days: u32) -> Self {
        MessageArchive {
            dir: dir.into(),
            day: String::new(),
            file: None,
            seq: 0,
            written: 0,
            rotations: 0,
            retention_days,
            local_offset_ms,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path_for(&self, day: &str) -> PathBuf {
        self.dir.join(format!("wiadomosci-{day}.jsonl"))
    }

    /// Buduje wiersz i zapisuje go. `now_ms` to zegar ścienny procesu.
    #[allow(clippy::too_many_arguments)]
    pub fn zapisz(
        &mut self,
        now_ms: i64,
        event: MsgEvent,
        chat_id: i64,
        topic_id: Option<i64>,
        msg_id: i64,
        reply_to: Option<i64>,
        edit_of: Option<i64>,
        msg_ts_ms: i64,
        source_name: &str,
        text: &str,
        monitored: bool,
    ) -> Result<()> {
        self.seq += 1;
        let rec = MsgRecord {
            v: ARCHIVE_SCHEMA,
            seq: self.seq,
            event,
            received_at: iso8601(now_ms, self.local_offset_ms),
            received_at_ms: now_ms,
            msg_ts_ms,
            chat_id,
            topic_id,
            msg_id,
            reply_to,
            edit_of,
            source_name: source_name.to_string(),
            text: text.to_string(),
            monitored,
        };
        self.push(&rec, now_ms)
    }

    /// Dopisuje gotowy wiersz.
    pub fn push(&mut self, rec: &MsgRecord, now_ms: i64) -> Result<()> {
        let day = dzien(now_ms, self.local_offset_ms);
        self.rotate(&day)?;
        // JEDNA linia budowana w całości i wypychana JEDNYM zapisem —
        // dopisywanie w trybie `append` jest niepodzielne, więc wiersze
        // nie wchodzą sobie w środek nawet przy kilku pisarzach.
        let mut linia = serde_json::to_string(rec)?;
        linia.push('\n');
        if let Some(f) = self.file.as_mut() {
            f.write_all(linia.as_bytes())
                .context("zapis archiwum wiadomości")?;
            self.written += 1;
        }
        Ok(())
    }

    fn rotate(&mut self, day: &str) -> Result<()> {
        if self.day == day && self.file.is_some() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("nie udało się utworzyć {}", self.dir.display()))?;
        if let Some(f) = self.file.take() {
            let _ = f.sync_all();
        }
        let p = self.path_for(day);
        let f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&p)
            .with_context(|| format!("nie udało się otworzyć {}", p.display()))?;
        self.file = Some(f);
        if !self.day.is_empty() {
            self.rotations += 1;
        }
        self.day = day.to_string();
        self.sprzataj();
        Ok(())
    }

    /// Kasuje pliki starsze niż `retention_days`.
    ///
    /// Świadomie po NAZWIE, nie po czasie modyfikacji: nazwa niesie dobę,
    /// której plik dotyczy, a czas modyfikacji zmienia się przy każdym
    /// dopisaniu i przy kopiowaniu katalogu na inny dysk.
    fn sprzataj(&self) {
        if self.retention_days == 0 || self.day.is_empty() {
            return;
        }
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let granica = odejmij_dni(&self.day, self.retention_days);
        for e in rd.filter_map(|x| x.ok()) {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(stem) = p.file_stem().and_then(|x| x.to_str()) else {
                continue;
            };
            let Some(d) = stem.strip_prefix("wiadomosci-") else {
                continue;
            };
            if d < granica.as_str() {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
}

fn iso8601(ms: i64, offset_ms: i64) -> String {
    use chrono::{FixedOffset, TimeZone};
    let strefa = FixedOffset::east_opt((offset_ms / 1000) as i32)
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("offset zerowy jest zawsze poprawny"));
    match strefa.timestamp_millis_opt(ms).single() {
        Some(t) => t.to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
        None => ms.to_string(),
    }
}

fn dzien(ms: i64, offset_ms: i64) -> String {
    use chrono::{FixedOffset, TimeZone};
    let strefa = FixedOffset::east_opt((offset_ms / 1000) as i32)
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("offset zerowy jest zawsze poprawny"));
    match strefa.timestamp_millis_opt(ms).single() {
        Some(t) => t.format("%Y-%m-%d").to_string(),
        None => "0000-00-00".to_string(),
    }
}

fn odejmij_dni(day: &str, dni: u32) -> String {
    use chrono::NaiveDate;
    match NaiveDate::parse_from_str(day, "%Y-%m-%d") {
        Ok(d) => (d - chrono::Duration::days(dni as i64))
            .format("%Y-%m-%d")
            .to_string(),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn katalog(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "conduit-arch-{tag}-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        p
    }

    fn zapisz_prosty(a: &mut MessageArchive, now: i64, ev: MsgEvent, id: i64, text: &str) {
        a.zapisz(
            now,
            ev,
            -100,
            Some(7),
            id,
            Some(id - 1),
            None,
            now,
            "KANAL",
            text,
            true,
        )
        .unwrap();
    }

    #[test]
    fn edycja_nie_nadpisuje_oryginalu() {
        // TO JEST CAŁY SENS TEGO MODUŁU. Eksport z Telegrama pokazałby tylko
        // wersję drugą — a bot decydował na podstawie pierwszej.
        let dir = katalog("edycja");
        let mut a = MessageArchive::new(&dir, 0, 0);
        let t0 = 1_785_000_000_000;

        zapisz_prosty(
            &mut a,
            t0,
            MsgEvent::Received,
            500,
            "BUY GOLD 4020 SL 4010 TP 4030",
        );
        a.zapisz(
            t0 + 900_000,
            MsgEvent::Edited,
            -100,
            Some(7),
            500,
            None,
            Some(500),
            t0,
            "KANAL",
            "BUY GOLD 4020 SL 4005 TP 4030",
            true,
        )
        .unwrap();
        drop(a);

        let plik = dir.join("wiadomosci-1970-01-01.jsonl");
        let plik = if plik.exists() {
            plik
        } else {
            dir.join(format!("wiadomosci-{}.jsonl", dzien(t0, 0)))
        };
        let tresc = std::fs::read_to_string(&plik).unwrap();
        let linie: Vec<&str> = tresc.lines().collect();
        assert_eq!(
            linie.len(),
            2,
            "edycja ma być OSOBNYM wierszem, nie podmianą"
        );

        let a1: MsgRecord = serde_json::from_str(linie[0]).unwrap();
        let a2: MsgRecord = serde_json::from_str(linie[1]).unwrap();
        assert_eq!(a1.event, MsgEvent::Received);
        assert!(
            a1.text.contains("SL 4010"),
            "pierwsza wersja musi zostać nienaruszona"
        );
        assert_eq!(a2.event, MsgEvent::Edited);
        assert_eq!(a2.edit_of, Some(500));
        assert!(a2.text.contains("SL 4005"));
        // czas odbioru edycji jest PÓŹNIEJSZY, choć Telegram poda znacznik oryginału
        assert!(a2.received_at_ms > a1.received_at_ms);
        assert_eq!(a2.msg_ts_ms, a1.msg_ts_ms);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reply_to_przezywa_zapis_i_odczyt() {
        // Kanał ATFX zarządza pozycją, ODPOWIADAJĄC na sygnał. Bez tego pola
        // komunikatu „TP1 HIT" nie da się przypisać do koszyka.
        let dir = katalog("reply");
        let mut a = MessageArchive::new(&dir, 0, 0);
        let t = 1_785_000_000_000;
        a.zapisz(
            t,
            MsgEvent::Received,
            -100,
            None,
            900,
            Some(842),
            None,
            t,
            "ATFX",
            "TP1 HIT",
            true,
        )
        .unwrap();
        drop(a);
        let tresc =
            std::fs::read_to_string(dir.join(format!("wiadomosci-{}.jsonl", dzien(t, 0)))).unwrap();
        let r: MsgRecord = serde_json::from_str(tresc.lines().next().unwrap()).unwrap();
        assert_eq!(r.reply_to, Some(842));
        assert_eq!(r.msg_id, 900);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn skasowanie_jest_zdarzeniem_a_nie_znikniecie() {
        let dir = katalog("kasowanie");
        let mut a = MessageArchive::new(&dir, 0, 0);
        let t = 1_785_000_000_000;
        zapisz_prosty(&mut a, t, MsgEvent::Received, 11, "SELL GOLD 4050");
        a.zapisz(
            t + 60_000,
            MsgEvent::Deleted,
            -100,
            Some(7),
            11,
            None,
            None,
            t + 60_000,
            "KANAL",
            "",
            true,
        )
        .unwrap();
        drop(a);
        let tresc =
            std::fs::read_to_string(dir.join(format!("wiadomosci-{}.jsonl", dzien(t, 0)))).unwrap();
        let linie: Vec<&str> = tresc.lines().collect();
        assert_eq!(linie.len(), 2);
        let d: MsgRecord = serde_json::from_str(linie[1]).unwrap();
        assert_eq!(d.event, MsgEvent::Deleted);
        assert_eq!(d.msg_id, 11);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn doba_dzieli_pliki_a_restart_dopisuje_do_tego_samego() {
        let dir = katalog("doba");
        let t1 = 1_785_000_000_000; // pewna doba
        let t2 = t1 + 86_400_000; // następna

        {
            let mut a = MessageArchive::new(&dir, 0, 0);
            zapisz_prosty(&mut a, t1, MsgEvent::Received, 1, "a");
            zapisz_prosty(&mut a, t2, MsgEvent::Received, 2, "b");
        }
        // nowy proces, ten sam katalog — MUSI dopisać, nie przewinąć
        {
            let mut a = MessageArchive::new(&dir, 0, 0);
            zapisz_prosty(&mut a, t1, MsgEvent::Received, 3, "c");
        }

        let p1 = dir.join(format!("wiadomosci-{}.jsonl", dzien(t1, 0)));
        let p2 = dir.join(format!("wiadomosci-{}.jsonl", dzien(t2, 0)));
        assert_eq!(
            std::fs::read_to_string(&p1).unwrap().lines().count(),
            2,
            "restart nie może skasować dnia"
        );
        assert_eq!(std::fs::read_to_string(&p2).unwrap().lines().count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn wielolinijkowa_tresc_zostaje_jednym_wierszem() {
        // Sygnały bywają pięciolinijkowe. Gdyby `\n` trafił do pliku surowy,
        // jedno zdarzenie rozpadłoby się na pięć wierszy i `wc -l` przestałby
        // cokolwiek znaczyć.
        let dir = katalog("wielolinia");
        let mut a = MessageArchive::new(&dir, 0, 0);
        let t = 1_785_000_000_000;
        zapisz_prosty(
            &mut a,
            t,
            MsgEvent::Received,
            1,
            "BUY GOLD\nTP1 4030\nTP2 4040\nSL 4010",
        );
        drop(a);
        let tresc =
            std::fs::read_to_string(dir.join(format!("wiadomosci-{}.jsonl", dzien(t, 0)))).unwrap();
        assert_eq!(tresc.lines().count(), 1);
        let r: MsgRecord = serde_json::from_str(tresc.lines().next().unwrap()).unwrap();
        assert_eq!(
            r.text.lines().count(),
            4,
            "treść po odczycie ma wrócić w całości"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
