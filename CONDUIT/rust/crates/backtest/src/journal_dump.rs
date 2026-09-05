
use conduit_core::journal::JournalEvent;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

pub struct JournalDump {
    w: BufWriter<File>,
    txt: Option<BufWriter<File>>,
    pub written: u64,
    pub path: PathBuf,
}

impl JournalDump {
    /// Otwiera plik do zapisu. Istniejący jest NADPISYWANY — przebieg
    /// backtestu jest powtarzalny, więc doklejanie do starego pliku dawałoby
    /// tylko podwojone liczby w raporcie.
    pub fn create(path: impl AsRef<Path>, text_mirror: bool) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(d) = path.parent() {
            if !d.as_os_str().is_empty() {
                std::fs::create_dir_all(d)?;
            }
        }
        let w = BufWriter::with_capacity(1 << 20, File::create(&path)?);
        let txt = if text_mirror {
            Some(BufWriter::with_capacity(
                1 << 20,
                File::create(path.with_extension("log"))?,
            ))
        } else {
            None
        };
        Ok(JournalDump {
            w,
            txt,
            written: 0,
            path,
        })
    }

    /// Zapisuje partię zdarzeń.
    ///
    /// `wall_utc_ms` to zegar ścienny — w backteście podajemy czas ticka
    /// przeliczony na UTC, żeby oba znaczniki opisywały TEN SAM moment
    /// symulacji. Mieszanie tu prawdziwego „teraz" dałoby plik, w którym
    /// zdarzenie z lipca ma znacznik z dnia uruchomienia programu.
    pub fn push(
        &mut self,
        events: &mut [JournalEvent],
        local_offset_ms: i64,
    ) -> std::io::Result<()> {
        for ev in events.iter_mut() {
            ev.stamp_wall(ev.ts_broker_ms - local_offset_ms, local_offset_ms);
            let linia = serde_json::to_string(ev).unwrap_or_default();
            self.w.write_all(linia.as_bytes())?;
            self.w.write_all(b"\n")?;
            if let Some(t) = self.txt.as_mut() {
                t.write_all(ev.human().as_bytes())?;
                t.write_all(b"\n")?;
            }
            self.written += 1;
        }
        Ok(())
    }

    pub fn finish(mut self) -> std::io::Result<u64> {
        self.w.flush()?;
        if let Some(t) = self.txt.as_mut() {
            t.flush()?;
        }
        Ok(self.written)
    }
}
