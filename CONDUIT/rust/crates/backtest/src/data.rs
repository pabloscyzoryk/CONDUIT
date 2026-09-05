//! Wczytywanie danych historycznych.
//!
//! Ticki trzymamy w pliku binarnym mapowanym w pamięć — 54 mln ticków to
//! 866 MB, a `mmap` sprawia, że wczytanie kosztuje milisekundy i nie zjada RAM.

use anyhow::{bail, Context, Result};
use conduit_core::types::{Px, Quote, Ts};
use memmap2::Mmap;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::Path;

const MAGIC: u32 = 0x4B54_4443; // "CDTK"
const HEADER: usize = 64;
const REC: usize = 16;

#[repr(C)]
#[derive(Clone, Copy)]
struct RawTick {
    ts: i64,
    bid: f32,
    ask: f32,
}

pub struct TickData {
    _map: Mmap,
    ptr: *const RawTick,
    len: usize,
    price_digits: Option<u32>,
    price_factor: Option<f64>,
}

// Mmap jest tylko do odczytu i żyje tak długo jak struktura.
unsafe impl Send for TickData {}
unsafe impl Sync for TickData {}

impl TickData {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let p = path.as_ref();
        let f = File::open(p).with_context(|| format!("nie mogę otworzyć {}", p.display()))?;
        let map = unsafe { Mmap::map(&f)? };
        if map.len() < HEADER {
            bail!("plik ticków za krótki");
        }
        let magic = u32::from_le_bytes(map[0..4].try_into().unwrap());
        if magic != MAGIC {
            bail!("zły nagłówek pliku ticków (magic {magic:#x})");
        }
        let count = u64::from_le_bytes(map[8..16].try_into().unwrap()) as usize;
        let need = HEADER + count * REC;
        if map.len() < need {
            bail!("plik ticków obcięty: {} < {}", map.len(), need);
        }
        let ptr = unsafe { map.as_ptr().add(HEADER) } as *const RawTick;
        Ok(TickData {
            _map: map,
            ptr,
            len: count,
            price_digits: None,
            price_factor: None,
        })
    }

    pub fn set_price_digits(&mut self, digits: Option<u32>) -> Result<()> {
        if digits.is_some_and(|d| d > 8) { bail!("sim-price-digits musi być w zakresie 0..8"); }
        self.price_digits = digits;
        self.price_factor = digits.map(|d| 10f64.powi(d as i32));
        Ok(())
    }
    pub fn price_digits(&self) -> Option<u32> { self.price_digits }
    #[inline]
    fn norm_price(&self, px: f64) -> f64 {
        self.price_factor.map_or(px, |f| (px * f).round() / f)
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn ts(&self, i: usize) -> Ts {
        unsafe { (*self.ptr.add(i)).ts }
    }
    #[inline]
    pub fn bid(&self, i: usize) -> Px {
        self.norm_price(unsafe { (*self.ptr.add(i)).bid as f64 })
    }
    #[inline]
    pub fn ask(&self, i: usize) -> Px {
        self.norm_price(unsafe { (*self.ptr.add(i)).ask as f64 })
    }
    #[inline]
    pub fn quote(&self, i: usize) -> Quote {
        let r = unsafe { *self.ptr.add(i) };
        Quote {
            ts: r.ts,
            bid: self.norm_price(r.bid as f64),
            ask: self.norm_price(r.ask as f64),
        }
    }

    /// Pierwszy indeks o znaczniku >= ts.
    pub fn index_at(&self, ts: Ts) -> usize {
        let (mut lo, mut hi) = (0usize, self.len);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.ts(mid) < ts {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    pub fn first_ts(&self) -> Ts {
        if self.len == 0 {
            0
        } else {
            self.ts(0)
        }
    }
    pub fn last_ts(&self) -> Ts {
        if self.len == 0 {
            0
        } else {
            self.ts(self.len - 1)
        }
    }
}

// ============================================================
//  SYGNAŁY
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawSignal {
    pub id: i64,
    pub ts: i64,
    #[serde(default)]
    pub edited: Option<i64>,
    pub dir: String,
    pub limit: bool,
    pub lo: f64,
    pub hi: f64,
    pub sl: f64,
    pub tps: Vec<f64>,
    #[serde(default)]
    pub tag_high_risk: bool,
    #[serde(default)]
    pub tag_may_not: bool,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub events: Vec<RawEvent>,
    #[serde(default)]
    pub kanal: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEvent {
    pub ts: i64,
    /// Identyfikator WIADOMOŚCI źródłowej z Telegrama.
    ///
    /// Jedna wiadomość może nieść kilka poleceń („+30 PIPS HIT / RISK FREE
    /// 4108"), a generator zapisuje każde osobnym zdarzeniem.
    /// Dopóki wszystkie zdarzenia niosą tę samą, PEŁNĄ treść, odtwarzanie musi
    /// zrobić z nich JEDNĄ wiadomość, bo inaczej silnik wykonałby ją tyle razy,
    /// ile poleceń rozpoznał generator. Ten identyfikator jest tu kluczem.
    ///
    /// Zero = starszy eksport bez tego pola; wtedy każde zdarzenie dostaje
    /// własny sztuczny numer, tak jak dotąd.
    #[serde(default)]
    pub msg_id: i64,
    pub kind: String,
    #[serde(default)]
    pub val: Option<f64>,
    #[serde(default)]
    pub be: Option<f64>,
    /// ORYGINALNA treść komunikatu zarządzającego, jeśli eksport ją zachował.
    ///
    /// Bez niej komunikat odtwarzamy z samego `kind`, a `kind` jest JEDEN na
    /// wiadomość — podczas gdy jedna wiadomość kanału niesie zwykle kilka
    /// poleceń naraz. Wzorcowy przypadek: „+20 PIPS HIT 🔥 / RISK FREE 4090"
    /// zapisuje się jako `TP_HIT` i polecenie RISK FREE znika bez śladu.
    /// Parser silnika (`core::parser::parse`) zwraca wiele akcji z jednego
    /// tekstu, dlatego adapter musi zachować oryginalną treść rekordu.
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_of: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SignalFile {
    signals: Vec<RawSignal>,
}

/// Kanoniczny, surowy strumień wiadomości do odtwarzania ścieżki live.
///
/// To jest drugi, niezależny kontrakt wejściowy obok historycznego
/// `{"signals":[...]}`. Pole `ts` jest chwilą **rzeczywistego odebrania**
/// wiadomości przez Conduit, w milisekundach Unix. Loader nie dodaje do niego
/// opóźnienia modelowego z presetu i nie próbuje go odgadywać z czasu
/// publikacji Telegrama.
///
/// `reply_to` jest surową, bezpośrednią odpowiedzią Telegrama. Szczególnie
/// ważne: loader NIE wspina się po łańcuchu odpowiedzi i NIE przypina
/// wiadomości do rozpoznanego sygnału. Taka interpretacja należy do silnika,
/// bo właśnie jego zachowanie chcemy odtworzyć i audytować.
#[derive(Debug, Clone, Deserialize)]
struct RawMessageFile {
    messages: Vec<CanonicalRawMessage>,
}

#[derive(Debug, Clone, Deserialize)]
struct CanonicalRawMessage {
    /// Rzeczywista chwila wejścia zdarzenia do Conduita, Unix ms.
    #[serde(alias = "received_at_ms", alias = "arrival_ts")]
    ts: Ts,
    /// Telegram publication timestamp.  Raw live chronicles and synthetic
    /// live-backtests keep it separate from `ts` (arrival), exactly like the
    /// production listener keeps `IncomingMessage.ts` separate from
    /// `received_utc`.  Older raw files may instead provide `latency_ms`.
    #[serde(default, alias = "telegram_ts", alias = "published_at_ms")]
    telegram_published_ts: Option<Ts>,
    #[serde(alias = "id")]
    msg_id: i64,
    #[serde(default, alias = "reply_to_message_id")]
    reply_to: Option<i64>,
    #[serde(default)]
    edit_of: Option<i64>,
    text: String,
    #[serde(default, alias = "channel", alias = "source_name")]
    kanal: String,
    /// Rozstrzyga zdarzenia o identycznym `ts`. Brak = stabilna kolejność
    /// z pliku, po wszystkich rekordach z jawnym numerem odbioru.
    #[serde(default)]
    receive_seq: Option<u64>,
    /// Metadana audytowa. `ts` już jest czasem rzeczywistego przyjścia, więc
    /// ponowne dodanie tej wartości oznaczałoby podwójne naliczenie latencji.
    #[serde(default, rename = "latency_ms", alias = "latency")]
    latency_ms: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct ReplayMessage {
    pub ts: Ts,
    /// Telegram publication time for the production stale-entry gate.
    /// `None` is the historical/ordinary replay contract and never activates
    /// that live-only gate.
    pub telegram_published_ts: Option<Ts>,
    pub msg_id: i64,
    pub reply_to: Option<i64>,
    pub edit_of: Option<i64>,
    pub text: String,
    /// Format, którym podano tę wiadomość (`Synergy`, `ZEN`, …).
    ///
    /// Komunikat zarządzający DZIEDZICZY kanał po swoim sygnale — inaczej
    /// „RISK FREE" z ZEN trafiłby do silnika Synergy i przestawił stopy
    /// w cudzym koszyku. Puste = zbiór jednokanałowy.
    pub kanal: String,
}

pub fn load_messages(path: impl AsRef<Path>) -> Result<Vec<ReplayMessage>> {
    let txt = std::fs::read_to_string(path.as_ref())
        .with_context(|| format!("nie mogę wczytać {}", path.as_ref().display()))?;
    let root: serde_json::Value = serde_json::from_str(&txt)?;

    // Autodetekcja jest po nazwie kontenera, a nie po rozszerzeniu ani nazwie
    // pliku. Dzięki temu to samo CLI potrafi odtworzyć legacy `signals` oraz
    // kronikę `messages`, bez ukrytego przełącznika zmieniającego semantykę.
    if root.get("messages").is_some() {
        let f: RawMessageFile =
            serde_json::from_value(root).context("niepoprawny kanoniczny strumień raw messages")?;
        return Ok(load_raw_messages(f));
    }

    let f: SignalFile =
        serde_json::from_value(root).context("niepoprawny historyczny eksport nested signals")?;
    load_legacy_messages(f)
}

pub fn load_messages_with_time_offset(
    path: impl AsRef<Path>,
    offset_min: i64,
) -> Result<Vec<ReplayMessage>> {
    let mut messages = load_messages(path)?;
    if offset_min == 0 {
        return Ok(messages);
    }

    let offset_ms = offset_min.checked_mul(60_000).ok_or_else(|| {
        anyhow::anyhow!("przesunięcie czasu sygnałów poza zakresem: {offset_min} min")
    })?;
    for m in &mut messages {
        m.ts = m.ts.checked_add(offset_ms).ok_or_else(|| {
            anyhow::anyhow!(
                "przesunięcie czasu sygnałów przepełnia i64: ts={} offset={} ms",
                m.ts,
                offset_ms
            )
        })?;
    }
    // Dodajemy jedną stałą, więc kolejność pozostaje niezmieniona.
    Ok(messages)
}

/// Surowa ścieżka live. Nie ma tu parsera, grupowania, pre-resolve reply ani
/// deduplikacji wersji: każdy niepusty rekord staje się jednym zdarzeniem.
fn load_raw_messages(f: RawMessageFile) -> Vec<ReplayMessage> {
    let mut out: Vec<(usize, Option<u64>, ReplayMessage)> = f
        .messages
        .into_iter()
        .enumerate()
        .filter(|(_, m)| !m.text.trim().is_empty())
        .map(|(source_order, m)| {
            (
                source_order,
                m.receive_seq,
                ReplayMessage {
                    ts: m.ts,
                    telegram_published_ts: m.telegram_published_ts.or_else(|| {
                        m.latency_ms
                            .and_then(|delay| m.ts.checked_sub(delay.max(0)))
                    }),
                    msg_id: m.msg_id,
                    reply_to: m.reply_to,
                    edit_of: m.edit_of,
                    text: m.text,
                    kanal: m.kanal,
                },
            )
        })
        .collect();

    // `receive_seq` jest mocniejszym dowodem kolejności niż położenie w
    // pliku. Gdy go brak, `source_order` daje deterministyczny, stabilny tie
    // break zamiast dawnego sortowania po msg_id (id są per kanał i edycje
    // celowo mają ten sam id co oryginał).
    out.sort_by_key(|(source_order, receive_seq, m)| {
        (m.ts, receive_seq.unwrap_or(u64::MAX), *source_order)
    });
    out.into_iter().map(|(_, _, m)| m).collect()
}

/// Dokładnie dotychczasowa rekonstrukcja zagnieżdżonego eksportu. Trzymamy
/// ją w osobnej funkcji, aby dodanie raw replay nie zmieniło nawet kolejności
/// ani syntetycznych identyfikatorów starego formatu.
fn load_legacy_messages(f: SignalFile) -> Result<Vec<ReplayMessage>> {
    let mut out: Vec<ReplayMessage> = Vec::with_capacity(f.signals.len() * 4);
    let mut next_id = f.signals.iter().map(|s| s.id).max().unwrap_or(0) + 1_000_000;

    let mut widziane: std::collections::HashSet<(String, i64)> =
        f.signals.iter().map(|s| (s.kanal.clone(), s.id)).collect();

    for s in &f.signals {
        let tps = s
            .tps
            .iter()
            .map(|t| format!("TP {t:.2}"))
            .collect::<Vec<_>>()
            .join("\n");
        let kind = if s.limit {
            format!("{} LIMITS", s.dir)
        } else {
            s.dir.clone()
        };
        let text = if s.text.trim().is_empty() {
            format!(
                "{kind} GOLD @ {:.2}/{:.2}\n{tps}\nTP OPEN\nSL {:.2}\n{}",
                s.hi,
                s.lo,
                s.sl,
                if s.tag_high_risk {
                    "HIGH RISK TRADE"
                } else {
                    ""
                }
            )
        } else {
            s.text.clone()
        };
        out.push(ReplayMessage {
            ts: s.ts * 1000,
            telegram_published_ts: None,
            msg_id: s.id,
            reply_to: None,
            edit_of: None,
            text,
            kanal: s.kanal.clone(),
        });

        for e in &s.events {
            // Oryginał ma pierwszeństwo — rekonstrukcja z `kind` zostaje
            // wyłącznie dla starszych eksportów, które tekstu nie niosą.
            if !e.text.trim().is_empty() {
                let klucz = if e.msg_id > 0 {
                    e.msg_id
                } else {
                    next_id += 1;
                    next_id
                };
                // kolejne zdarzenie tej samej WERSJI — treść już poszła
                if !widziane.insert((s.kanal.clone(), klucz)) {
                    continue;
                }
                out.push(ReplayMessage {
                    ts: e.ts * 1000,
                    telegram_published_ts: None,
                    msg_id: e.edit_of.unwrap_or(klucz),
                    reply_to: Some(s.id),
                    edit_of: e.edit_of,
                    text: e.text.clone(),
                    // komunikat zarządzający NALEŻY do kanału swojego sygnału
                    kanal: s.kanal.clone(),
                });
                continue;
            }
            let text = match e.kind.as_str() {
                // NUMER CELU MUSI ZOSTAĆ W TREŚCI. Odtwarzanie stałym napisem
                // „TP HIT +PIPS" gubiło go w całości: parser nie rozpoznawał
                // takiej wiadomości i wszystkie 4 414 komunikatów o trafionym
                // celu lądowało jako `Info`. Ścieżka „cel z kanału" — a więc
                // etapy koszyka, kasowanie siatki i inkaso na celu z sygnału —
                // w backteście po prostu nie istniała.
                "TP_HIT" => match e.val {
                    Some(v) if v >= 1.0 => format!("TP{} HIT", v as i64),
                    _ => "TP HIT".to_string(),
                },
                "SL_HIT" => "SL HIT".to_string(),
                "RISK_FREE" => match e.val {
                    Some(v) => format!("RISK FREE {v:.2}"),
                    None => "RISK FREE".to_string(),
                },
                "OUT_AT_ENTRY" => "OUT AT ENTRY ON THE REST".to_string(),
                "CANCEL" => "CANCEL THE LIMITS".to_string(),
                "SPP" => match e.be {
                    Some(v) => format!("SECURING PARTIAL PROFITS\nSL IS SET TO BE AT {v:.2}"),
                    None => "SECURING PARTIAL PROFITS".to_string(),
                },
                "BE" => match e.be.or(e.val) {
                    Some(v) => format!("SL IS SET TO BE AT {v:.2}"),
                    None => "SL IS SET TO BE".to_string(),
                },
                "CLOSE_ALL" => "CLOSE ALL".to_string(),
                _ => continue,
            };
            next_id += 1;
            out.push(ReplayMessage {
                ts: e.ts * 1000,
                telegram_published_ts: None,
                msg_id: next_id,
                reply_to: Some(s.id),
                edit_of: None,
                text,
                kanal: s.kanal.clone(),
            });
        }
    }

    out.sort_by_key(|m| (m.ts, m.msg_id));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broker_price_precision_tickdata_is_optional_and_consistent() {
        let path = std::env::temp_dir().join(format!("conduit_price_tick_{}.bin", std::process::id()));
        let mut bytes = vec![0u8; HEADER];
        bytes[..4].copy_from_slice(&MAGIC.to_le_bytes());
        bytes[8..16].copy_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&123i64.to_le_bytes());
        bytes.extend_from_slice(&4599.21f32.to_le_bytes());
        bytes.extend_from_slice(&4599.43f32.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        let mut ticks = TickData::open(&path).unwrap();
        assert_eq!(ticks.price_digits(), None);
        assert_eq!(ticks.ask(0), 4599.43f32 as f64);
        ticks.set_price_digits(Some(2)).unwrap();
        assert_eq!(ticks.bid(0), 4599.21);
        assert_eq!(ticks.ask(0), 4599.43);
        assert_eq!(ticks.quote(0).ask, ticks.ask(0));
        assert_eq!(ticks.quote(0).bid, ticks.bid(0));
        assert!(ticks.set_price_digits(Some(9)).is_err());
        ticks.set_price_digits(None).unwrap();
        assert_eq!(ticks.quote(0).ask, 4599.43f32 as f64);
        drop(ticks);
        std::fs::remove_file(path).unwrap();
    }

    /// LICZNIK, NIE ZEGAR. Nazwa pliku brała nanosekundy z `SystemTime`,
    /// a zegar systemowy Windows tyka co ~15 ms — dwa testy startujące
    /// równolegle dostawały tę samą nazwę i czytały nawzajem swoje dane.
    /// Objaw był losowy (raz na kilka przebiegów, zawsze inny test), czyli
    /// najgorszy z możliwych. Licznik atomowy jest unikalny z definicji.
    static NR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn zapisz(tresc: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "conduit_sygnaly_{}_{}.json",
            std::process::id(),
            NR.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::write(&p, tresc).unwrap();
        p
    }

    // ============ KANONICZNY RAW MESSAGE STREAM ============

    /// Brak odpowiedzi jest informacją. Loader nie może zamienić go na
    /// ostatni sygnał ani na sztuczny korzeń rozmowy.
    #[test]
    fn raw_no_reply_pozostaje_none() {
        let p = zapisz(
            r#"{"messages":[{"ts":100000,"msg_id":7,"reply_to":null,
            "edit_of":null,"text":"GOOD MORNING TRADERS","kanal":"Synergy"}]}"#,
        );
        let m = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].ts, 100_000, "raw ts jest już w ms, bez mnożenia");
        assert_eq!(m[0].reply_to, None);
        assert_eq!(m[0].edit_of, None);
    }

    /// Odpowiedź do odpowiedzi pozostaje odpowiedzią do jej BEZPOŚREDNIEGO
    /// rodzica. Pre-resolve 3 -> 2 -> 1 do 3 -> 1 fałszowałby zachowanie live.
    #[test]
    fn raw_transitive_reply_pozostaje_direct() {
        let p = zapisz(
            r#"{"messages":[
              {"ts":100,"msg_id":1,"text":"ROOT","kanal":"Synergy"},
              {"ts":200,"msg_id":2,"reply_to":1,"text":"CHILD","kanal":"Synergy"},
              {"ts":300,"msg_id":3,"reply_to":2,"text":"GRANDCHILD","kanal":"Synergy"}
            ]}"#,
        );
        let m = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(
            m.iter().map(|x| x.reply_to).collect::<Vec<_>>(),
            vec![None, Some(1), Some(2)]
        );
    }

    /// Każda wersja jest osobnym zdarzeniem w czasie, mimo identycznego
    /// Telegramowego `msg_id`. Nie wolno deduplikować ich jak legacy events.
    #[test]
    fn raw_wiele_edycji_tego_samego_msg_id_przechodzi_w_calosci() {
        let p = zapisz(
            r#"{"messages":[
              {"ts":100,"msg_id":41,"text":"BUY GOLD","kanal":"Synergy"},
              {"ts":110,"msg_id":41,"edit_of":41,"text":"BUY GOLD @ 4600","kanal":"Synergy"},
              {"ts":120,"msg_id":41,"edit_of":41,"text":"BUY GOLD @ 4600 SL 4590","kanal":"Synergy"}
            ]}"#,
        );
        let m = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(m.len(), 3);
        assert!(m.iter().all(|x| x.msg_id == 41));
        assert_eq!(
            m.iter().map(|x| x.edit_of).collect::<Vec<_>>(),
            vec![None, Some(41), Some(41)]
        );
        assert_eq!(m[2].text, "BUY GOLD @ 4600 SL 4590");
    }

    /// Przy jednakowym zegarze decyduje kolejność rzeczywistego odbioru,
    /// a nie msg_id. Rekord bez seq zachowuje kolejność pliku i idzie po
    /// rekordach, dla których kronika ma mocniejszy dowód kolejności.
    #[test]
    fn raw_tie_sortuje_po_receive_seq() {
        let p = zapisz(
            r#"{"messages":[
              {"ts":500,"msg_id":30,"receive_seq":30,"latency_ms":12,"text":"THIRD","kanal":"Synergy"},
              {"ts":500,"msg_id":99,"text":"NO SEQ","kanal":"Synergy"},
              {"ts":500,"msg_id":10,"receive_seq":10,"latency":8,"text":"FIRST","kanal":"Synergy"},
              {"ts":500,"msg_id":20,"receive_seq":20,"text":"SECOND","kanal":"Synergy"}
            ]}"#,
        );
        let m = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(
            m.iter().map(|x| x.msg_id).collect::<Vec<_>>(),
            vec![10, 20, 30, 99]
        );
    }

    /// Raw replay nie jest raportem parsera. Każdy niepusty tekst przechodzi,
    /// nawet jeśli dzisiejszy parser klasyfikuje go wyłącznie jako Info.
    #[test]
    fn raw_przepuszcza_info_i_pomija_tylko_pusty_tekst() {
        let info = "THIS IS AN UNSTRUCTURED COMMUNITY ANNOUNCEMENT";
        assert_eq!(
            conduit_core::parser::parse(info),
            vec![conduit_core::parser::Signal::Info]
        );
        let p = zapisz(&format!(
            r#"{{"messages":[
              {{"received_at_ms":700,"id":1,"text":"{info}","channel":"Synergy"}},
              {{"arrival_ts":701,"id":2,"text":"   ","channel":"Synergy"}}
            ]}}"#,
        ));
        let m = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].text, info);
        assert_eq!(m[0].kanal, "Synergy");
    }

    /// Dodanie autodetekcji `messages` nie zmienia jednostki czasu, kanału,
    /// wiązania zdarzeń ani sortowania historycznego `signals`.
    #[test]
    fn legacy_nested_signals_zachowuje_dotychczasowy_kontrakt() {
        let p = zapisz(
            r#"{"signals":[{"id":10,"ts":100,"dir":"BUY","limit":false,
            "lo":4100.0,"hi":4105.0,"sl":4095.0,"tps":[4110.0],"kanal":"Synergy",
            "text":"BUY GOLD @ 4105/4100\nTP 4110\nSL 4095",
            "events":[{"ts":200,"msg_id":11,"kind":"CMD","text":"GOOD MORNING"}]}]}"#,
        );
        let m = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].ts, m[0].msg_id, m[0].reply_to), (100_000, 10, None));
        assert_eq!(
            (m[1].ts, m[1].msg_id, m[1].reply_to),
            (200_000, 11, Some(10))
        );
        assert!(m.iter().all(|x| x.kanal == "Synergy"));
    }

    /// Przesunięcie obejmuje zarówno wiadomość początkową, jak i każde
    /// zdarzenie/edycję z legacy `signals`. Bez tego geometria wejścia i
    /// późniejsze zarządzanie chodziłyby po dwóch różnych zegarach.
    #[test]
    fn jawny_offset_przesuwa_legacy_initial_event_i_edit() {
        let p = zapisz(
            r#"{"signals":[{"id":10,"ts":100,"dir":"BUY","limit":false,
            "lo":4100.0,"hi":4105.0,"sl":4095.0,"tps":[4110.0],
            "text":"BUY GOLD @ 4105/4100\nTP 4110\nSL 4095","events":[
              {"ts":200,"msg_id":11,"kind":"CMD","text":"TP1 HIT"},
              {"ts":220,"msg_id":10000011,"edit_of":11,"kind":"EDIT","text":"TP1 HIT\nRISK FREE"}
            ]}]}"#,
        );
        let bez = load_messages(&p).unwrap();
        let z = load_messages_with_time_offset(&p, 180).unwrap();
        let _ = std::fs::remove_file(&p);

        assert_eq!(
            bez.iter().map(|m| m.ts).collect::<Vec<_>>(),
            vec![100_000, 200_000, 220_000]
        );
        assert_eq!(
            z.iter().map(|m| m.ts).collect::<Vec<_>>(),
            vec![10_900_000, 11_000_000, 11_020_000]
        );
        assert_eq!(z[2].edit_of, Some(11));
    }

    /// Kronika raw jest z definicji dokładna i przy domyślnym zerze nie
    /// zmienia ani jednej wartości. Operator może ją przesunąć tylko jawnie.
    #[test]
    fn raw_received_at_domyslnie_bez_zmiany_a_jawny_offset_dziala() {
        let p = zapisz(
            r#"{"messages":[
              {"received_at_ms":700,"id":1,"text":"A","channel":"Synergy"},
              {"received_at_ms":900,"id":1,"edit_of":1,"text":"B","channel":"Synergy"}
            ]}"#,
        );
        let domyslne = load_messages_with_time_offset(&p, 0).unwrap();
        let jawne = load_messages_with_time_offset(&p, -15).unwrap();
        let _ = std::fs::remove_file(&p);

        assert_eq!(
            domyslne.iter().map(|m| m.ts).collect::<Vec<_>>(),
            vec![700, 900]
        );
        assert_eq!(
            jawne.iter().map(|m| m.ts).collect::<Vec<_>>(),
            vec![-899_300, -899_100]
        );
    }

    #[test]
    fn raw_live_replay_keeps_arrival_and_telegram_publication_separate() {
        let p = zapisz(
            r#"{"messages":[
              {"ts":11000,"telegram_published_ts":10000,"msg_id":1,"text":"A"},
              {"ts":22000,"latency_ms":2500,"msg_id":2,"text":"B"},
              {"ts":33000,"msg_id":3,"text":"C"}
            ]}"#,
        );
        let messages = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(
            messages.iter().map(|m| m.ts).collect::<Vec<_>>(),
            vec![11000, 22000, 33000]
        );
        assert_eq!(
            messages
                .iter()
                .map(|m| m.telegram_published_ts)
                .collect::<Vec<_>>(),
            vec![Some(10000), Some(19500), None]
        );
    }

    /// REGRESJA: wiadomość niosąca DWA polecenia zapisuje się jako dwa
    /// zdarzenia, ale do strumienia trafia RAZ.
    ///
    /// Generator musi rozbić „+30 PIPS HIT / RISK FREE 4108" na `TP_HIT`
    /// i `RISK_FREE`, bo inaczej drugie polecenie znika z danych. Skoro jednak
    /// oba zdarzenia niosą tę samą pełną treść, a parser silnika wyciąga z niej
    /// wszystkie polecenia, odtworzenie dwóch wiadomości byłoby wykonaniem
    /// tego samego dwa razy: koszyk drugi raz inkasowałby transzę na TP1.
    #[test]
    fn dwa_zdarzenia_jednej_wiadomosci_to_jedna_wiadomosc() {
        let p = zapisz(
            r#"{"signals":[{"id":10,"ts":100,"dir":"BUY","limit":false,"lo":4100.0,
            "hi":4105.0,"sl":4095.0,"tps":[4110.0],"text":"BUY GOLD @ 4105/4100\nTP 4110\nSL 4095",
            "events":[
              {"ts":200,"msg_id":11,"kind":"TP_HIT","val":1.0,"text":"+30 PIPS HIT\n\nRISK FREE 4108"},
              {"ts":200,"msg_id":11,"kind":"RISK_FREE","val":4108.0,"text":"+30 PIPS HIT\n\nRISK FREE 4108"}
            ]}]}"#,
        );
        let m = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(m.len(), 2, "wejście + JEDNA wiadomość zarządzająca");
        assert_eq!(m[1].msg_id, 11);
        assert_eq!(m[1].reply_to, Some(10));
        // treść musi zostać PEŁNA — z niej silnik wyciąga oba polecenia
        assert!(m[1].text.contains("RISK FREE"));
        assert!(m[1].text.contains("PIPS HIT"));
    }

    /// Starszy eksport bez `msg_id` i bez treści zachowuje się jak dotąd:
    /// każde zdarzenie odtwarza własną wiadomość z samego `kind`.
    #[test]
    fn stary_eksport_bez_msg_id_dziala_po_staremu() {
        let p = zapisz(
            r#"{"signals":[{"id":10,"ts":100,"dir":"BUY","limit":false,"lo":4100.0,
            "hi":4105.0,"sl":4095.0,"tps":[4110.0],
            "events":[{"ts":200,"kind":"TP_HIT","val":1.0},{"ts":300,"kind":"RISK_FREE","val":4108.0}]}]}"#,
        );
        let m = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(m.len(), 3);
        assert!(m[1].text.contains("TP1 HIT"));
        assert!(m[2].text.contains("RISK FREE"));
        assert_ne!(m[1].msg_id, m[2].msg_id);
    }

    /// Wiadomość będąca JEDNOCZEŚNIE wejściem i poleceniem nie może wejść do
    /// strumienia dwa razy.
    #[test]
    fn wiadomosc_bedaca_wejsciem_i_poleceniem_idzie_raz() {
        let p = zapisz(
            r#"{"signals":[{"id":10,"ts":100,"dir":"BUY","limit":false,"lo":4100.0,
            "hi":4105.0,"sl":4095.0,"tps":[4110.0],
            "text":"CANCEL THE LIMITS\nBUY GOLD @ 4105/4100\nTP 4110\nSL 4095",
            "events":[{"ts":100,"msg_id":10,"kind":"CANCEL","text":"CANCEL THE LIMITS\nBUY GOLD @ 4105/4100\nTP 4110\nSL 4095"}]}]}"#,
        );
        let m = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].msg_id, 10);
    }

    #[test]
    fn spp_odtworzony_z_kind_niesie_poziom_stopu() {
        let p = zapisz(
            r#"{"signals":[{"id":10,"ts":100,"dir":"BUY","limit":false,"lo":4100.0,
            "hi":4105.0,"sl":4095.0,"tps":[4110.0],
            "events":[{"ts":200,"kind":"SPP","be":4108.0}]}]}"#,
        );
        let m = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(m.len(), 2);
        assert!(m[1].text.contains("SECURING PARTIAL PROFITS"));
        let poziom = conduit_core::parser::parse(&m[1].text)
            .into_iter()
            .find_map(|s| match s {
                conduit_core::parser::Signal::SecuringPartial { spp_be_level, .. } => {
                    Some(spp_be_level)
                }
                _ => None,
            })
            .expect("SPP musi się rozpoznać");
        assert_eq!(poziom, Some(4108.0));
    }


    const SYNTHETIC_EDIT_FIXTURE: &str = r#"{"signals":[{"id":10,"ts":100,"dir":"BUY",
      "limit":true,"lo":4100.0,"hi":4105.0,"sl":4095.0,"tps":[4110.0],"kanal":"Synergy",
      "text":"BUY LIMITS GOLD @ 4105/4100\nTP 4110\nTP 4120\nSL 4095","reply_to":null,
      "events":[
        {"kind":"EDIT","msg_id":10000010,"ts":140,"edit_of":10,"reply_to":null,
         "text":"BUY LIMITS GOLD @ 4105/4100\nTP 4110\nTP 4120\nSL 4095"},
        {"kind":"CMD","msg_id":11,"ts":200,"reply_to":10,"text":"TP1 HIT +30 PIPS"},
        {"kind":"EDIT","msg_id":10000011,"ts":220,"edit_of":11,"reply_to":10,
         "text":"TP1 HIT +30 PIPS\n\nRISK FREE 4108"}
      ],"edited":140}]}"#;

    #[test]
    fn edycja_wchodzi_jako_edycja_a_nie_nowa_wiadomosc() {
        let p = zapisz(SYNTHETIC_EDIT_FIXTURE);
        let m = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);

        assert_eq!(
            m.len(),
            4,
            "wejście + jego edycja + komunikat + jego edycja"
        );

        // 1. wejście — bez zmian, nie jest edycją
        assert_eq!((m[0].ts, m[0].msg_id, m[0].edit_of), (100_000, 10, None));
        // 2. edycja WEJŚCIA: chwila poprawki, ten sam identyfikator wiadomości
        assert_eq!(
            (m[1].ts, m[1].msg_id, m[1].edit_of),
            (140_000, 10, Some(10))
        );
        assert!(
            m[1].text.contains("BUY LIMITS"),
            "edycja niesie treść finalną"
        );
        // 3. komunikat zarządzający — po staremu
        assert_eq!((m[2].ts, m[2].msg_id, m[2].edit_of), (200_000, 11, None));
        // 4. edycja KOMUNIKATU: dokłada polecenie, którego oryginał nie miał
        assert_eq!(
            (m[3].ts, m[3].msg_id, m[3].edit_of),
            (220_000, 11, Some(11))
        );
        assert!(m[3].text.contains("RISK FREE 4108"));

        // kanał dziedziczy się na edycjach tak samo jak na komunikatach —
        // inaczej edycja z Synergy trafiłaby do silnika ZEN
        assert!(m.iter().all(|x| x.kanal == "Synergy"));
        // każda edycja wskazuje sygnał, do którego należy
        assert_eq!(m[1].reply_to, Some(10));
        assert_eq!(m[3].reply_to, Some(10));
    }

    #[test]
    fn dedup_nie_zjada_edycji_mimo_tego_samego_msg_id() {
        let p = zapisz(SYNTHETIC_EDIT_FIXTURE);
        let m = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);

        let wersje_10: Vec<_> = m.iter().filter(|x| x.msg_id == 10).collect();
        assert_eq!(wersje_10.len(), 2, "oryginał wejścia i jego edycja");
        assert_eq!(wersje_10.iter().filter(|x| x.edit_of.is_some()).count(), 1);
        let wersje_11: Vec<_> = m.iter().filter(|x| x.msg_id == 11).collect();
        assert_eq!(wersje_11.len(), 2, "oryginał komunikatu i jego edycja");
        // żaden sztuczny numer 10-milionowy nie ma prawa wyjść na zewnątrz
        assert!(m.iter().all(|x| x.msg_id < 10_000_000));
    }

    /// Kilka poleceń w JEDNEJ edycji to nadal JEDNA wiadomość — ta sama
    /// zasada, która rządzi oryginałami (patrz test dwóch zdarzeń wyżej).
    #[test]
    fn dwa_zdarzenia_jednej_edycji_to_jedna_wiadomosc() {
        let p = zapisz(
            r#"{"signals":[{"id":10,"ts":100,"dir":"BUY","limit":false,"lo":4100.0,
            "hi":4105.0,"sl":4095.0,"tps":[4110.0],"text":"BUY GOLD @ 4105/4100\nTP 4110\nSL 4095",
            "events":[
              {"kind":"EDIT","msg_id":10000011,"ts":220,"edit_of":11,"text":"+30 PIPS HIT\n\nRISK FREE 4108"},
              {"kind":"EDIT","msg_id":10000011,"ts":220,"edit_of":11,"text":"+30 PIPS HIT\n\nRISK FREE 4108"}
            ]}]}"#,
        );
        let m = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(m.len(), 2, "wejście + JEDNA edycja");
        assert_eq!(m[1].edit_of, Some(11));
    }

    #[test]
    fn syntetyczny_legacy_bez_pol_edycyjnych_nie_ma_ani_jednej_edycji() {
        let p = zapisz(
            r#"{"signals":[{"id":10,"ts":100,"dir":"BUY","limit":false,"lo":4100.0,
            "hi":4105.0,"sl":4095.0,"tps":[4110.0],"text":"BUY GOLD @ 4105/4100\nTP 4110\nSL 4095",
            "events":[
              {"ts":200,"msg_id":11,"kind":"TP_HIT","val":1.0,"text":"+30 PIPS HIT"},
              {"ts":300,"kind":"RISK_FREE","val":4108.0}
            ]}]}"#,
        );
        let m = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(m.len(), 3);
        assert!(m.iter().all(|x| x.edit_of.is_none()), "nic nie jest edycją");
        assert_eq!(m[1].msg_id, 11, "prawdziwy identyfikator zostaje");
        assert!(
            m[2].msg_id > 1_000_000,
            "zdarzenie bez id dostaje syntetyczny"
        );
    }
}

pub fn load_signals(path: impl AsRef<Path>) -> Result<Vec<RawSignal>> {
    let txt = std::fs::read_to_string(path.as_ref())?;
    let f: SignalFile = serde_json::from_str(&txt)?;
    Ok(f.signals)
}
