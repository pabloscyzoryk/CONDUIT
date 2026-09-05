//! Rozpoznawanie plików danych **PO ZAWARTOŚCI**, nie po nazwie.
//!
//! **Dlaczego nie po nazwie.** Plik z tickami bywa `ticks.bin`, `XAUUSD.bin`,
//! `dane_kwiecien.dat` albo `export (3).csv`, a eksport z Telegrama zawsze
//! nazywa się `result.json` — czyli tak samo jak połowa plików na dysku.
//! Nazwa jest podpowiedzią, a nie dowodem; dowodem jest nagłówek `CDTK`,
//! zestaw kluczy JSON-a albo dający się sparsować wiersz CSV. Dzięki temu
//! wyszukiwarka nie pokazuje śmieci i nie gubi pliku, który ktoś przemianował.
//!
//! Każdy kandydat wraca z tym, co UDAŁO SIĘ ODCZYTAĆ: liczbą rekordów,
//! zakresem dat i rozpoznanym ZEGAREM. To ostatnie jest tu najważniejsze —
//! znaczniki w `ticks.bin` są w czasie serwera brokera (UTC+3), a `ts`
//! w `signals.json` w sekundach UTC. Pomylenie tych dwóch zegarów daje
//! backtestowi darmowy zysk (silnik dostaje sygnał razem ze strumieniem cen
//! sprzed trzech godzin), więc sniffer sam proponuje przesunięcie.

use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Nagłówek pliku binarnego: magia „CDTK".
const MAGIC: [u8; 4] = *b"CDTK";
/// Rozmiar nagłówka i pojedynczego rekordu w `ticks.bin`.
const HEADER: u64 = 64;
const REC: u64 = 16;

/// Ile bajtów wystarczy, żeby rozpoznać rodzaj pliku.
const PROBE: usize = 8192;

/// Powyżej tego rozmiaru CSV nie liczymy wierszy dokładnie — szacujemy.
const CSV_EXACT_MAX: u64 = 4 << 20;

/// Powyżej tego rozmiaru JSON-a nie parsujemy w całości.
const JSON_PARSE_MAX: u64 = 256 << 20;

/// Godzina w milisekundach.
const H: i64 = 3_600_000;

/// Domyślne przesunięcie zegara serwera brokera względem UTC (Vantage: +3 h).
pub const SERVER_TZ_MS: i64 = 3 * H;

// ============================================================
//  WYNIK ROZPOZNANIA
// ============================================================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub path: String,
    pub name: String,
    pub bytes: u64,
    /// `ticksBin` | `ticksCsv` | `signalsJson` | `telegramJson` | `telegramHtml`
    pub kind: String,
    /// do czego plik się nadaje: `ticks` | `signals`
    pub role: String,
    /// czy tryb demo umie z tego pliku odtwarzać
    pub usable: bool,
    pub records: u64,
    /// czy `records` policzono dokładnie (`false` = oszacowano z rozmiaru)
    pub exact: bool,
    pub first_ts: i64,
    pub last_ts: i64,
    pub first_day: String,
    pub last_day: String,
    /// `server` (czas brokera) | `utc` | `unknown`
    pub clock: String,
    /// godzina dobowej przerwy sesyjnej w zegarze pliku (dla ticków)
    pub break_hour: Option<u32>,
    /// ile dodać do znacznika WIADOMOŚCI, żeby trafić w zegar tego pliku ticków
    pub suggested_msg_offset_ms: Option<i64>,
    pub note: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub candidates: Vec<Candidate>,
    /// katalogi, które faktycznie przeszukano
    pub roots: Vec<String>,
    pub files_seen: u64,
    pub dirs_seen: u64,
    pub elapsed_ms: u64,
    /// czy przeszukiwanie przerwał limit czasu albo liczby plików
    pub truncated: bool,
}

// ============================================================
//  ROZPOZNANIE POJEDYNCZEGO PLIKU
// ============================================================

/// Rozpoznaje plik po zawartości. `None` = to nie jest nic, co znamy.
pub fn sniff(path: &Path) -> Option<Candidate> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() < 32 {
        return None;
    }
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; PROBE.min(meta.len() as usize)];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);

    let base = Candidate {
        path: path.display().to_string(),
        name: path
            .file_name()
            .map(|x| x.to_string_lossy().to_string())
            .unwrap_or_default(),
        bytes: meta.len(),
        ..Default::default()
    };

    if buf.len() >= 16 && buf[0..4] == MAGIC {
        return sniff_ticks_bin(path, base, meta.len());
    }

    // Dalej pracujemy na tekście. Plik binarny odpada tutaj, bo `from_utf8_lossy`
    // zamieni bajty na znaki zastępcze i żaden z testów niżej nie przejdzie.
    if buf.iter().take(1024).any(|b| *b == 0) {
        return None;
    }
    let txt = String::from_utf8_lossy(&buf);
    let head = txt.trim_start();

    if head.starts_with('{') || head.starts_with('[') {
        return sniff_json(path, base, meta.len(), head);
    }
    if wyglada_na_html_telegrama(head) {
        return Some(Candidate {
            kind: "telegramHtml".into(),
            role: "signals".into(),
            usable: false,
            clock: "unknown".into(),
            note: "eksport HTML z Telegrama — rozpoznany, ale odtwarzanie go NIE używa: \
                   eksport HTML gubi edycje i skasowane wiadomości. Użyj signals.json \
                   albo eksportu JSON."
                .into(),
            ..base
        });
    }
    sniff_ticks_csv(path, base, meta.len(), &txt)
}

// ---------------- ticks.bin (CDTK) ----------------

fn sniff_ticks_bin(path: &Path, base: Candidate, bytes: u64) -> Option<Candidate> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut hdr = [0u8; 16];
    f.read_exact(&mut hdr).ok()?;
    let count = u64::from_le_bytes(hdr[8..16].try_into().ok()?);
    if count == 0 {
        return Some(Candidate {
            kind: "ticksBin".into(),
            role: "ticks".into(),
            usable: false,
            note: "nagłówek CDTK jest, ale plik nie ma ani jednego ticka".into(),
            ..base
        });
    }
    let need = HEADER + count * REC;
    let obciety = bytes < need;
    // Liczba ticków z NAGŁÓWKA bywa większa niż to, co realnie leży w pliku
    // (przerwany zapis). Bierzemy mniejszą z dwóch — inaczej `TickData::open`
    // odmówi otwarcia, a my zdążylibyśmy obiecać użytkownikowi 54 mln ticków.
    let realne = if obciety {
        bytes.saturating_sub(HEADER) / REC
    } else {
        count
    };
    if realne == 0 {
        return None;
    }

    let first_ts = czytaj_ts(&mut f, HEADER)?;
    let last_ts = czytaj_ts(&mut f, HEADER + (realne - 1) * REC)?;
    let (break_hour, clock, offset) = zegar_tickow(&mut f, realne);

    let mut note = String::new();
    if obciety {
        note.push_str(&format!(
            "plik obcięty: nagłówek zapowiada {count} ticków, w pliku jest {realne}. "
        ));
    }
    match break_hour {
        Some(h) => note.push_str(&format!(
            "dobowa przerwa sesyjna zaczyna się o {h:02}:00 w zegarze pliku → {}",
            if clock == "server" {
                "znaczniki są już w czasie serwera brokera".to_string()
            } else {
                format!(
                    "przesunięcie wiadomości UTC: {:+} h",
                    offset.unwrap_or(0) / H
                )
            }
        )),
        None => note.push_str("nie udało się rozpoznać zegara (za mało dobowych przerw w danych)"),
    }

    Some(Candidate {
        kind: "ticksBin".into(),
        role: "ticks".into(),
        usable: !obciety || realne > 1000,
        records: realne,
        exact: true,
        first_ts,
        last_ts,
        first_day: crate::lab::dzien(first_ts),
        last_day: crate::lab::dzien(last_ts),
        clock,
        break_hour,
        suggested_msg_offset_ms: offset,
        note,
        ..base
    })
}

fn czytaj_ts(f: &mut std::fs::File, off: u64) -> Option<i64> {
    f.seek(SeekFrom::Start(off)).ok()?;
    let mut b = [0u8; 8];
    f.read_exact(&mut b).ok()?;
    Some(i64::from_le_bytes(b))
}

/// Rozpoznaje zegar pliku ticków po **dobowej przerwie sesyjnej złota**.
///
/// Złoto stoi godzinę na dobę i ta przerwa wypada o 00:00 czasu serwera
/// brokera. Wystarczy więc znaleźć godzinę, o której zaczynają się największe
/// luki między tickami, żeby wiedzieć, w jakim zegarze zapisano plik:
/// 0 = czas serwera, 21 = UTC (bo serwer to UTC+3).
///
/// Próbkujemy co `krok` rekordów zamiast czytać 54 mln — luka godzinna jest
/// widoczna nawet przy rzadkim próbkowaniu, a odczyt schodzi z minut do
/// milisekund. Zwraca `(godzina przerwy, nazwa zegara, proponowany offset
/// wiadomości)`.
fn zegar_tickow(f: &mut std::fs::File, count: u64) -> (Option<u32>, String, Option<i64>) {
    const PROBEK: u64 = 20_000;
    let krok = (count / PROBEK).max(1);
    let mut hist = [0u32; 24];
    let mut poprzedni: Option<i64> = None;
    let mut i = 0u64;
    let mut trafien = 0u32;
    while i < count {
        let ts = match czytaj_ts(f, HEADER + i * REC) {
            Some(t) => t,
            None => break,
        };
        if let Some(p) = poprzedni {
            let luka = ts - p;
            // 20 min … 4 h = DOBOWA przerwa sesyjna. Górne ograniczenie odcina
            // weekendy: gdyby wpadły do histogramu, ich środek wypadałby
            // w sobotę w południe i zaszumiał godzinę, której szukamy.
            if luka > 20 * 60_000 && luka < 4 * H {
                // Bierzemy ŚRODEK luki, nie jej początek: ostatni tick przed
                // przerwą leży jeszcze w godzinie poprzedniej (23:59), a nas
                // interesuje godzina, w której rynek stoi (00:xx).
                let srodek = p + luka / 2;
                let h = (srodek.div_euclid(H) % 24).rem_euclid(24) as usize;
                hist[h] += 1;
                trafien += 1;
            }
        }
        poprzedni = Some(ts);
        i += krok;
    }
    if trafien < 3 {
        return (None, "unknown".into(), None);
    }
    let h = hist
        .iter()
        .enumerate()
        .max_by_key(|(_, n)| **n)
        .map(|(i, _)| i as u32)
        .unwrap_or(0);
    // ile dodać do znacznika PLIKU, żeby wyszedł czas serwera
    let shift = ((24 - h as i64) % 24) * H;
    // ile dodać do znacznika WIADOMOŚCI (UTC), żeby trafić w zegar pliku
    let msg_offset = SERVER_TZ_MS - shift;
    let clock = if shift == 0 {
        "server"
    } else if msg_offset == 0 {
        "utc"
    } else {
        "other"
    };
    (Some(h), clock.into(), Some(msg_offset))
}

// ---------------- JSON ----------------

fn sniff_json(path: &Path, base: Candidate, bytes: u64, head: &str) -> Option<Candidate> {
    let natywny =
        head.contains("\"signals\"") && (head.contains("\"tps\"") || head.contains("\"dir\""));
    let telegram = head.contains("\"messages\"") && head.contains("\"date\"");
    if !natywny && !telegram {
        return None;
    }
    if bytes > JSON_PARSE_MAX {
        return Some(Candidate {
            kind: if natywny {
                "signalsJson".into()
            } else {
                "telegramJson".into()
            },
            role: "signals".into(),
            usable: false,
            note: format!(
                "plik JSON ma {} MB — za dużo, żeby go wczytać w całości",
                bytes >> 20
            ),
            ..base
        });
    }

    if natywny {
        let sygnaly = conduit_backtest::load_signals(path).ok()?;
        if sygnaly.is_empty() {
            return None;
        }
        let (a, b) = zakres(
            sygnaly
                .iter()
                .flat_map(|s| std::iter::once(s.ts).chain(s.events.iter().map(|e| e.ts))),
        )?;
        return Some(Candidate {
            kind: "signalsJson".into(),
            role: "signals".into(),
            usable: true,
            records: sygnaly.len() as u64,
            exact: true,
            first_ts: a,
            last_ts: b,
            first_day: crate::lab::dzien(a),
            last_day: crate::lab::dzien(b),
            clock: "utc".into(),
            suggested_msg_offset_ms: Some(SERVER_TZ_MS),
            note: format!(
                "natywny eksport sygnałów, {} komunikatów zarządzających; `ts` w sekundach UTC",
                sygnaly.iter().map(|s| s.events.len()).sum::<usize>()
            ),
            ..base
        });
    }

    // eksport z Telegrama
    let raw = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let msgs = v.get("messages")?.as_array()?;
    let znaczniki: Vec<i64> = msgs
        .iter()
        .filter_map(|m| {
            m.get("date_unixtime")
                .and_then(|x| x.as_str())
                .and_then(|s| s.parse::<i64>().ok())
        })
        .collect();
    let (a, b) = if znaczniki.is_empty() {
        let iso: Vec<i64> = msgs
            .iter()
            .filter_map(|m| {
                m.get("date")
                    .and_then(|x| x.as_str())
                    .and_then(parse_iso_ms)
            })
            .collect();
        (iso.iter().copied().min()?, iso.iter().copied().max()?)
    } else {
        (
            znaczniki.iter().copied().min()? * 1000,
            znaczniki.iter().copied().max()? * 1000,
        )
    };
    Some(Candidate {
        kind: "telegramJson".into(),
        role: "signals".into(),
        usable: true,
        records: msgs.len() as u64,
        exact: true,
        first_ts: a,
        last_ts: b,
        first_day: crate::lab::dzien(a),
        last_day: crate::lab::dzien(b),
        clock: "utc".into(),
        suggested_msg_offset_ms: Some(SERVER_TZ_MS),
        note: format!("eksport JSON z Telegrama, {} wiadomości", msgs.len()),
        ..base
    })
}

fn zakres(it: impl Iterator<Item = i64>) -> Option<(i64, i64)> {
    let mut lo = i64::MAX;
    let mut hi = i64::MIN;
    for t in it {
        // znaczniki bywają w sekundach albo w milisekundach — rozstrzyga rząd
        // wielkości: 1e12 ms to rok 2001, 1e12 s to rok 33 658
        let ms = if t.abs() > 1_000_000_000_000 {
            t
        } else {
            t * 1000
        };
        lo = lo.min(ms);
        hi = hi.max(ms);
    }
    if lo > hi {
        None
    } else {
        Some((lo, hi))
    }
}

fn wyglada_na_html_telegrama(head: &str) -> bool {
    let h = head.to_ascii_lowercase();
    h.contains("<html")
        && (h.contains("class=\"message default clearfix")
            || (h.contains("telegram") && h.contains("class=\"history")))
}

// ---------------- CSV z tickami ----------------

fn sniff_ticks_csv(path: &Path, base: Candidate, bytes: u64, probka: &str) -> Option<Candidate> {
    let sep = wykryj_separator(probka)?;
    let mut wiersze = 0usize;
    let mut pierwszy: Option<i64> = None;
    let mut suma_dlugosci = 0usize;
    for l in probka.lines().skip(1).take(40) {
        // `skip(1)` — pierwszy wiersz próbki może być nagłówkiem albo urwanym
        // ogonem poprzedniego odczytu; i tak mamy ich kilkadziesiąt
        if let Some((ts, _, _)) = parsuj_wiersz(l, sep) {
            if pierwszy.is_none() {
                pierwszy = Some(ts);
            }
            wiersze += 1;
            suma_dlugosci += l.len() + 1;
        }
    }
    if wiersze < 3 {
        return None;
    }
    let first_ts = pierwszy?;
    let srednia = (suma_dlugosci as f64 / wiersze as f64).max(1.0);

    // ostatni pełny wiersz z końca pliku
    let last_ts = ostatni_znacznik(path, bytes, sep).unwrap_or(first_ts);

    let (records, exact) = if bytes <= CSV_EXACT_MAX {
        match std::fs::read(path) {
            Ok(b) => (b.iter().filter(|c| **c == b'\n').count() as u64, true),
            Err(_) => ((bytes as f64 / srednia) as u64, false),
        }
    } else {
        ((bytes as f64 / srednia) as u64, false)
    };

    Some(Candidate {
        kind: "ticksCsv".into(),
        role: "ticks".into(),
        // CSV rozpoznajemy i pokazujemy, ale odtwarzanie chodzi z `.bin` —
        // mówimy to wprost zamiast milczącego „nie działa"
        usable: false,
        records,
        exact,
        first_ts,
        last_ts,
        first_day: crate::lab::dzien(first_ts),
        last_day: crate::lab::dzien(last_ts),
        clock: "unknown".into(),
        note: format!(
            "CSV z tickami (separator „{}\u{201d}). Odtwarzanie w trybie demo wymaga formatu \
             binarnego CDTK — przekonwertuj plik (analiza/convert_ticks.py).",
            if sep == '\t' {
                "TAB".to_string()
            } else {
                sep.to_string()
            }
        ),
        ..base
    })
}

fn wykryj_separator(probka: &str) -> Option<char> {
    for sep in ['\t', ',', ';'] {
        let ile = probka
            .lines()
            .skip(1)
            .take(20)
            .filter(|l| parsuj_wiersz(l, sep).is_some())
            .count();
        if ile >= 3 {
            return Some(sep);
        }
    }
    None
}

pub fn parsuj_wiersz(linia: &str, sep: char) -> Option<(i64, f64, f64)> {
    let pola: Vec<&str> = linia.trim().split(sep).map(|x| x.trim()).collect();
    if pola.len() < 2 {
        return None;
    }
    let (ts, zjedzone) = parsuj_czas(&pola)?;
    let liczby: Vec<f64> = pola[zjedzone..]
        .iter()
        .filter_map(|x| x.replace(',', ".").parse::<f64>().ok())
        .filter(|x| x.is_finite() && *x > 0.0)
        .collect();
    if liczby.is_empty() {
        return None;
    }
    let bid = liczby[0];
    let ask = liczby.get(1).copied().unwrap_or(bid);
    // kwotowanie musi mieć sens: ASK nigdy poniżej BID-u, spread nie z kosmosu
    if ask < bid || ask - bid > bid * 0.05 {
        return None;
    }
    Some((ts, bid, ask))
}

fn parsuj_czas(pola: &[&str]) -> Option<(i64, usize)> {
    // 1) data i godzina w OSOBNYCH polach (eksport MT5)
    if pola.len() >= 2 {
        if let (Some(d), Some(t)) = (parsuj_date(pola[0]), parsuj_godzine(pola[1])) {
            return Some((d + t, 2));
        }
    }
    // 2) data i godzina w JEDNYM polu
    let p0 = pola[0];
    let rozdzielony = p0.replacen('T', " ", 1);
    if let Some((d, t)) = rozdzielony.split_once(' ') {
        if let (Some(d), Some(t)) = (parsuj_date(d), parsuj_godzine(t)) {
            return Some((d + t, 1));
        }
    }
    // 3) epoka
    if let Ok(n) = p0.parse::<i64>() {
        if (1_000_000_000..=4_000_000_000).contains(&n) {
            return Some((n * 1000, 1));
        }
        if (1_000_000_000_000..=4_000_000_000_000).contains(&n) {
            return Some((n, 1));
        }
    }
    None
}

/// `RRRR-MM-DD` albo `RRRR.MM.DD` albo `RRRR/MM/DD` → ms północy UTC.
fn parsuj_date(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 8 || s.len() > 10 {
        return None;
    }
    let sep = s.chars().find(|c| *c == '-' || *c == '.' || *c == '/')?;
    let p: Vec<&str> = s.split(sep).collect();
    if p.len() != 3 {
        return None;
    }
    let y: i64 = p[0].parse().ok()?;
    let m: i64 = p[1].parse().ok()?;
    let d: i64 = p[2].parse().ok()?;
    if !(1970..=2200).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let yy = if m <= 2 { y - 1 } else { y };
    let era = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe = yy - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400_000)
}

/// `GG:MM:SS[.mmm]` (albo `GG:MM`) → milisekundy od północy.
fn parsuj_godzine(s: &str) -> Option<i64> {
    let s = s.trim().trim_end_matches('Z');
    let p: Vec<&str> = s.split(':').collect();
    if p.len() < 2 || p.len() > 3 {
        return None;
    }
    let h: i64 = p[0].parse().ok()?;
    let m: i64 = p[1].parse().ok()?;
    if !(0..24).contains(&h) || !(0..60).contains(&m) {
        return None;
    }
    let mut ms = h * 3_600_000 + m * 60_000;
    if let Some(sec) = p.get(2) {
        let (ss, frak) = match sec.split_once('.') {
            Some((a, b)) => (a, b),
            None => (*sec, ""),
        };
        let s2: i64 = ss.parse().ok()?;
        if !(0..62).contains(&s2) {
            return None;
        }
        ms += s2 * 1000;
        if !frak.is_empty() {
            let cyfry: String = frak
                .chars()
                .filter(|c| c.is_ascii_digit())
                .take(3)
                .collect();
            if !cyfry.is_empty() {
                let skala = 10i64.pow(3 - cyfry.len() as u32);
                ms += cyfry.parse::<i64>().ok()? * skala;
            }
        }
    }
    Some(ms)
}

fn parse_iso_ms(s: &str) -> Option<i64> {
    let s = s.replacen('T', " ", 1);
    let (d, t) = s.split_once(' ')?;
    Some(parsuj_date(d)? + parsuj_godzine(t)?)
}

fn ostatni_znacznik(path: &Path, bytes: u64, sep: char) -> Option<i64> {
    let mut f = std::fs::File::open(path).ok()?;
    let ile = 16_384u64.min(bytes);
    f.seek(SeekFrom::Start(bytes - ile)).ok()?;
    let mut buf = vec![0u8; ile as usize];
    f.read_exact(&mut buf).ok()?;
    let txt = String::from_utf8_lossy(&buf);
    txt.lines()
        .rev()
        .find_map(|l| parsuj_wiersz(l, sep).map(|(ts, _, _)| ts))
}

// ============================================================
//  PRZESZUKIWANIE KATALOGÓW
// ============================================================

/// Katalogi, których nigdy nie warto przeglądać — mieliłyby budżet czasu
/// na plikach, wśród których z definicji nie ma danych rynkowych.
const POMIJANE: &[&str] = &[
    "node_modules",
    ".git",
    ".svn",
    "target",
    "dist",
    "build",
    "__pycache__",
    ".venv",
    "venv",
    "backup_memory",
    ".cache",
    ".idea",
    ".vscode",
];

/// Rozszerzenia, których nie ma sensu nawet otwierać.
const ODPADY: &[&str] = &[
    "exe", "dll", "pdb", "lib", "rlib", "obj", "o", "so", "dylib", "zip", "7z", "rar", "gz", "png",
    "jpg", "jpeg", "gif", "ico", "svg", "mp4", "mp3", "wav", "ttf", "woff", "woff2", "pyc",
    "class",
];

#[derive(Debug, Clone)]
pub struct ScanOpts {
    pub depth: usize,
    pub budget: Duration,
    pub max_files: u64,
    /// dodatkowy katalog wskazany ręcznie przez użytkownika
    pub extra_root: Option<PathBuf>,
}

impl Default for ScanOpts {
    fn default() -> Self {
        ScanOpts {
            depth: 3,
            budget: Duration::from_millis(4000),
            max_files: 20_000,
            extra_root: None,
        }
    }
}

pub fn scan(ws_root: &Path, opts: &ScanOpts) -> ScanResult {
    let start = Instant::now();
    let mut wynik = ScanResult::default();
    let mut korzenie: Vec<PathBuf> = Vec::new();
    let dodaj = |p: PathBuf, v: &mut Vec<PathBuf>| {
        let k = std::fs::canonicalize(&p).unwrap_or(p);
        if k.is_dir() && !v.contains(&k) {
            v.push(k);
        }
    };
    if let Some(e) = &opts.extra_root {
        dodaj(e.clone(), &mut korzenie);
    }
    dodaj(ws_root.to_path_buf(), &mut korzenie);
    if let Ok(cwd) = std::env::current_dir() {
        dodaj(cwd, &mut korzenie);
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut d = exe.parent().map(|x| x.to_path_buf());
        for _ in 0..4 {
            match d {
                Some(p) => {
                    dodaj(p.clone(), &mut korzenie);
                    d = p.parent().map(|x| x.to_path_buf());
                }
                None => break,
            }
        }
    }

    let mut widziane: Vec<PathBuf> = Vec::new();
    for r in &korzenie {
        wynik.roots.push(r.display().to_string());
        chodz(r, 0, opts, start, &mut wynik, &mut widziane);
        if przekroczony(&wynik, opts, start) {
            wynik.truncated = true;
            break;
        }
    }

    // najpierw ticki, potem sygnały; w obrębie roli — więcej rekordów wyżej
    wynik.candidates.sort_by(|a, b| {
        a.role
            .cmp(&b.role)
            .then(b.usable.cmp(&a.usable))
            .then(b.records.cmp(&a.records))
    });
    wynik.elapsed_ms = start.elapsed().as_millis() as u64;
    wynik
}

fn przekroczony(w: &ScanResult, o: &ScanOpts, start: Instant) -> bool {
    w.files_seen >= o.max_files || start.elapsed() >= o.budget
}

fn chodz(
    dir: &Path,
    poziom: usize,
    opts: &ScanOpts,
    start: Instant,
    out: &mut ScanResult,
    widziane: &mut Vec<PathBuf>,
) {
    // Osiągnięta głębokość to NORMALNA granica przeszukiwania, nie awaria —
    // gdyby ustawiała `truncated`, ostrzeżenie „lista może być niepełna"
    // świeciłoby się przy każdym uruchomieniu i przestałoby cokolwiek znaczyć.
    if poziom > opts.depth {
        return;
    }
    if przekroczony(out, opts, start) {
        out.truncated = true;
        return;
    }
    let kanon = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    if widziane.contains(&kanon) {
        return;
    }
    widziane.push(kanon);
    out.dirs_seen += 1;

    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    let mut podkatalogi: Vec<PathBuf> = Vec::new();
    for e in rd.flatten() {
        if przekroczony(out, opts, start) {
            out.truncated = true;
            return;
        }
        let p = e.path();
        let nazwa = p
            .file_name()
            .map(|x| x.to_string_lossy().to_string())
            .unwrap_or_default();
        match e.file_type() {
            Ok(t) if t.is_dir() => {
                if !POMIJANE.contains(&nazwa.as_str()) && !nazwa.starts_with('.') {
                    podkatalogi.push(p);
                }
            }
            Ok(t) if t.is_file() => {
                let ext = p
                    .extension()
                    .map(|x| x.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                if ODPADY.contains(&ext.as_str()) {
                    continue;
                }
                out.files_seen += 1;
                if let Some(c) = sniff(&p) {
                    if !out.candidates.iter().any(|x| x.path == c.path) {
                        out.candidates.push(c);
                    }
                }
            }
            _ => {}
        }
    }
    podkatalogi.sort();
    for d in podkatalogi {
        chodz(&d, poziom + 1, opts, start, out, widziane);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Tmp {
            let mut p = std::env::temp_dir();
            p.push(format!(
                "conduit-sniff-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
        fn plik(&self, nazwa: &str, dane: &[u8]) -> PathBuf {
            let p = self.0.join(nazwa);
            let mut f = std::fs::File::create(&p).unwrap();
            f.write_all(dane).unwrap();
            p
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Buduje plik CDTK z podanych ticków `(ts, bid, ask)`.
    fn zbuduj_bin(t: &Tmp, nazwa: &str, ticki: &[(i64, f32, f32)]) -> PathBuf {
        let mut buf = Vec::with_capacity(64 + ticki.len() * 16);
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&(ticki.len() as u64).to_le_bytes());
        buf.resize(64, 0);
        for (ts, b, a) in ticki {
            buf.extend_from_slice(&ts.to_le_bytes());
            buf.extend_from_slice(&b.to_le_bytes());
            buf.extend_from_slice(&a.to_le_bytes());
        }
        t.plik(nazwa, &buf)
    }

    /// Ticki z dobową przerwą o zadanej godzinie zegara pliku.
    fn ticki_z_przerwa(dni: i64, godzina_przerwy: i64) -> Vec<(i64, f32, f32)> {
        let mut v = Vec::new();
        let mut ts = 0i64;
        for d in 0..dni {
            for h in 0..24 {
                if h == godzina_przerwy {
                    continue; // godzina przerwy — brak ticków
                }
                for m in 0..6 {
                    ts = d * 86_400_000 + h * H + m * 600_000;
                    v.push((ts, 4000.0, 4000.24));
                }
            }
        }
        let _ = ts;
        v
    }

    #[test]
    fn plik_bin_rozpoznaje_sie_po_naglowku_a_nie_po_nazwie() {
        let t = Tmp::new("bin");
        // NAZWA celowo myląca: rozszerzenie sugeruje tekst
        let p = zbuduj_bin(
            &t,
            "cokolwiek.txt",
            &[(1_000, 4000.0, 4000.2), (2_000, 4001.0, 4001.2)],
        );
        let c = sniff(&p).expect("plik z magią CDTK musi się rozpoznać");
        assert_eq!(c.kind, "ticksBin");
        assert_eq!(c.role, "ticks");
        assert_eq!(c.records, 2);
        assert_eq!(c.first_ts, 1_000);
        assert_eq!(c.last_ts, 2_000);
    }

    #[test]
    fn obciety_plik_bin_raportuje_realna_liczbe_tickow() {
        let t = Tmp::new("obciety");
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&1_000_000u64.to_le_bytes()); // nagłówek kłamie
        buf.resize(64, 0);
        for i in 0..10i64 {
            buf.extend_from_slice(&(i * 1000).to_le_bytes());
            buf.extend_from_slice(&4000f32.to_le_bytes());
            buf.extend_from_slice(&4000.2f32.to_le_bytes());
        }
        let p = t.plik("urwany.bin", &buf);
        let c = sniff(&p).unwrap();
        assert_eq!(
            c.records, 10,
            "liczba ticków musi pochodzić z pliku, nie z obietnicy nagłówka"
        );
        assert!(c.note.contains("obcięty"));
    }

    #[test]
    fn zegar_serwera_rozpoznaje_sie_po_przerwie_sesyjnej() {
        let t = Tmp::new("zegar-serwer");
        // przerwa o 00:00 zegara pliku = znaczniki już w czasie serwera
        let p = zbuduj_bin(&t, "serwer.bin", &ticki_z_przerwa(6, 0));
        let c = sniff(&p).unwrap();
        assert_eq!(c.break_hour, Some(0));
        assert_eq!(c.clock, "server");
        assert_eq!(c.suggested_msg_offset_ms, Some(SERVER_TZ_MS));
    }

    #[test]
    fn zegar_utc_daje_zerowe_przesuniecie_wiadomosci() {
        let t = Tmp::new("zegar-utc");
        // przerwa o 21:00 zegara pliku = plik jest w UTC (serwer to UTC+3)
        let p = zbuduj_bin(&t, "utc.bin", &ticki_z_przerwa(6, 21));
        let c = sniff(&p).unwrap();
        assert_eq!(c.break_hour, Some(21));
        assert_eq!(c.clock, "utc");
        assert_eq!(
            c.suggested_msg_offset_ms,
            Some(0),
            "gdy ticki i wiadomości są w UTC, nie wolno nic przesuwać"
        );
    }

    #[test]
    fn sygnaly_json_rozpoznaja_sie_po_kluczach() {
        let t = Tmp::new("sygnaly");
        let json = br#"{"signals":[
            {"id":1,"ts":1775483268,"dir":"BUY","limit":false,"lo":4665.0,"hi":4670.0,"sl":4664.0,
             "tps":[4673.0],"events":[{"ts":1775483328,"kind":"TP_HIT","val":null}]},
            {"id":2,"ts":1775569668,"dir":"SELL","limit":true,"lo":4700.0,"hi":4705.0,"sl":4710.0,
             "tps":[4690.0],"events":[]}]}"#;
        let p = t.plik("bez_wymownej_nazwy.dat", json);
        let c = sniff(&p).expect("JSON z kluczem signals musi się rozpoznać");
        assert_eq!(c.kind, "signalsJson");
        assert_eq!(c.role, "signals");
        assert_eq!(c.records, 2);
        assert_eq!(c.clock, "utc");
        assert_eq!(c.first_day, "2026-04-06");
    }

    #[test]
    fn eksport_telegrama_json_jest_rozpoznawany() {
        let t = Tmp::new("tg");
        let json = br#"{"name":"ATFX","type":"public_channel","messages":[
            {"id":1,"type":"message","date":"2026-04-06T10:00:00","date_unixtime":"1775469600","text":"BUY GOLD"},
            {"id":2,"type":"message","date":"2026-04-07T10:00:00","date_unixtime":"1775556000","text":"TP1 HIT"}]}"#;
        let p = t.plik("result.json", json);
        let c = sniff(&p).unwrap();
        assert_eq!(c.kind, "telegramJson");
        assert_eq!(c.records, 2);
        assert_eq!(c.first_day, "2026-04-06");
    }

    #[test]
    fn eksport_html_rozpoznany_ale_oznaczony_jako_nieuzywalny() {
        let t = Tmp::new("html");
        let html = br#"<!DOCTYPE html><html><head><title>ATFX</title></head><body>
            <div class="page_wrap"><div class="history">
            <div class="message default clearfix" id="message1"><div class="text">BUY GOLD</div></div>
            </div></div></body></html>"#;
        let p = t.plik("messages.html", html);
        let c = sniff(&p).unwrap();
        assert_eq!(c.kind, "telegramHtml");
        assert!(
            !c.usable,
            "HTML gubi edycje — nie wolno go po cichu wpuścić do odtwarzania"
        );
    }

    #[test]
    fn csv_z_tickami_rozpoznaje_sie_i_podaje_zakres() {
        let t = Tmp::new("csv");
        let mut s = String::from("<DATE>\t<TIME>\t<BID>\t<ASK>\n");
        for i in 0..50 {
            s.push_str(&format!(
                "2026.04.01\t00:{:02}:00.100\t4000.10\t4000.34\n",
                i % 60
            ));
        }
        s.push_str("2026.04.02\t12:00:00.000\t4010.10\t4010.34\n");
        let p = t.plik("eksport.csv", s.as_bytes());
        let c = sniff(&p).expect("CSV z tickami musi się rozpoznać");
        assert_eq!(c.kind, "ticksCsv");
        assert_eq!(c.first_day, "2026-04-01");
        assert_eq!(c.last_day, "2026-04-02");
        assert!(c.exact);
        assert_eq!(c.records, 52);
    }

    #[test]
    fn zwykly_tekst_i_kod_nie_sa_kandydatami() {
        let t = Tmp::new("smieci");
        let a = t.plik(
            "readme.md",
            b"# CONDUIT\n\nTo jest opis projektu, a nie dane rynkowe.\n",
        );
        let b = t.plik(
            "kod.rs",
            b"fn main() { println!(\"czesc\"); }\n// komentarz\n",
        );
        let c = t.plik(
            "konfig.json",
            br#"{"port":8787,"host":"127.0.0.1","debug":true}"#,
        );
        assert!(sniff(&a).is_none());
        assert!(sniff(&b).is_none());
        assert!(
            sniff(&c).is_none(),
            "JSON bez kluczy sygnałów nie jest sygnałami"
        );
    }

    #[test]
    fn wiersz_csv_parsuje_sie_w_kilku_ukladach_czasu() {
        assert_eq!(
            parsuj_wiersz("2026.04.01\t00:00:00.644\t4327.07\t4327.37", '\t')
                .unwrap()
                .0,
            1_775_001_600_644
        );
        assert_eq!(
            parsuj_wiersz("2026-04-01 00:00:00,4327.07,4327.37", ',')
                .unwrap()
                .0,
            1_775_001_600_000
        );
        assert_eq!(
            parsuj_wiersz("2026-04-01T00:00:00Z;4327.07;4327.37", ';')
                .unwrap()
                .0,
            1_775_001_600_000
        );
        assert_eq!(
            parsuj_wiersz("1775001600,4327.07,4327.37", ',').unwrap().0,
            1_775_001_600_000
        );
        assert_eq!(
            parsuj_wiersz("1775001600644,4327.07,4327.37", ',')
                .unwrap()
                .0,
            1_775_001_600_644
        );
        // ASK poniżej BID-u to nie jest kwotowanie
        assert!(parsuj_wiersz("2026-04-01 00:00:00,4327.07,4000.00", ',').is_none());
        // wiersz bez czasu odpada
        assert!(parsuj_wiersz("nazwa,4327.07,4327.37", ',').is_none());
    }

    #[test]
    fn przeszukiwanie_znajduje_pliki_w_podkatalogach_i_omija_smieci() {
        let t = Tmp::new("scan");
        std::fs::create_dir_all(t.0.join("dane")).unwrap();
        std::fs::create_dir_all(t.0.join("node_modules/paczka")).unwrap();
        zbuduj_bin(
            &t,
            "dane/x.bin",
            &[(1_000, 4000.0, 4000.2), (2_000, 4001.0, 4001.2)],
        );
        // ten sam plik w katalogu, którego nie wolno przeglądać
        zbuduj_bin(&t, "node_modules/paczka/y.bin", &[(1_000, 4000.0, 4000.2)]);

        let r = scan(
            &t.0,
            &ScanOpts {
                depth: 3,
                ..Default::default()
            },
        );
        let sciezki: Vec<&str> = r.candidates.iter().map(|c| c.path.as_str()).collect();
        assert!(
            sciezki.iter().any(|p| p.ends_with("x.bin")),
            "nie znaleziono {sciezki:?}"
        );
        assert!(
            !sciezki.iter().any(|p| p.contains("node_modules")),
            "node_modules ma być pomijane: {sciezki:?}"
        );
    }

    #[test]
    fn przeszukiwanie_szanuje_limit_czasu() {
        let t = Tmp::new("budzet");
        let r = scan(
            &t.0,
            &ScanOpts {
                depth: 8,
                budget: Duration::from_millis(0),
                ..Default::default()
            },
        );
        assert!(
            r.truncated,
            "zerowy budżet czasu musi przerwać przeszukiwanie"
        );
        assert!(r.elapsed_ms < 3_000);
    }

    /// Limit głębokości jest TWARDY: plik leżący niżej nie ma prawa wejść na
    /// listę. Sprawdzamy konkretny plik, a nie pustkę — `scan` z założenia
    /// przegląda też katalog procesu, więc lista rzadko bywa pusta.
    ///
    /// Flagi `truncated` NIE da się tu sprawdzić z tego samego powodu: zależy
    /// od tego, ile plików leży w katalogu procesu. Testuje ją
    /// `przeszukiwanie_szanuje_limit_czasu`, gdzie budżet jest zerowy i wynik
    /// nie zależy od otoczenia.
    #[test]
    fn limit_glebokosci_jest_twardy() {
        let t = Tmp::new("glebokosc");
        std::fs::create_dir_all(t.0.join("a/b/c/d/e")).unwrap();
        zbuduj_bin(&t, "a/b/c/d/e/gleboko.bin", &[(1_000, 4000.0, 4000.2)]);
        zbuduj_bin(&t, "a/plytko.bin", &[(1_000, 4000.0, 4000.2)]);

        let r = scan(
            &t.0,
            &ScanOpts {
                depth: 1,
                ..Default::default()
            },
        );
        let ma = |n: &str| r.candidates.iter().any(|c| c.path.contains(n));
        assert!(ma("plytko.bin"), "plik z głębokości 1 musi się znaleźć");
        assert!(
            !ma("gleboko.bin"),
            "plik z głębokości 5 nie ma prawa wejść przy limicie 1"
        );
    }
}
