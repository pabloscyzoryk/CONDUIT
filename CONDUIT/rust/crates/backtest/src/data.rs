//! Wczytywanie danych historycznych.
//!
//! Ticki trzymamy w pliku binarnym mapowanym w pamięć — 54 mln ticków to
//! 866 MB, a `mmap` sprawia, że wczytanie kosztuje milisekundy i nie zjada RAM.

use anyhow::{bail, Context, Result};
use conduit_core::types::{Px, Quote, Ts};
use memmap2::Mmap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::{Arc, Mutex};

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
    // Quick replays run many presets over the same immutable tape.  Building
    // the extrema-preserving index once is important: rescanning 100+ million
    // raw rows for every preset would move the cost rather than remove it.
    quick_index_cache: Mutex<HashMap<QuickIndexKey, Arc<Vec<usize>>>>,
}

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
struct QuickIndexKey {
    start: usize,
    end: usize,
    stride: usize,
    forced: Vec<usize>,
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
            quick_index_cache: Mutex::new(HashMap::new()),
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

    /// Indices for an explicitly approximate, extrema-preserving replay.
    ///
    /// Every raw `stride` block retains its first and last row plus the rows
    /// carrying BID/ASK minima and maxima.  `forced` rows split a block before
    /// extrema are selected; callers use this for message-arrival and trading
    /// day boundaries.  Consequently an order created at a forced row can
    /// never consume an extreme which physically happened earlier in its
    /// block.  Long market gaps retain both adjacent rows as well.
    ///
    /// The returned indices are strictly increasing and refer to the original
    /// immutable tape, so SimBroker still receives real timestamps, spreads,
    /// prices and source-row identities.  This is an approximation: repeated
    /// intrablock crossings which are not extrema can still be omitted.
    pub fn quick_extrema_indices(
        &self,
        start: usize,
        end: usize,
        stride: usize,
        forced: &[usize],
    ) -> Arc<Vec<usize>> {
        let start = start.min(self.len);
        let end = end.min(self.len).max(start);
        let stride = stride.max(2);
        let mut forced: Vec<usize> = forced
            .iter()
            .copied()
            .filter(|&i| i >= start && i < end)
            .collect();
        forced.sort_unstable();
        forced.dedup();
        let key = QuickIndexKey { start, end, stride, forced };

        let mut cache = self.quick_index_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(indices) = cache.get(&key) {
            return Arc::clone(indices);
        }

        let mut out = Vec::with_capacity((end - start).saturating_mul(6) / stride + 1024);
        let mut forced_pos = 0usize;
        let mut block_start = start;
        while block_start < end {
            let block_end = block_start.saturating_add(stride).min(end);
            while forced_pos < key.forced.len() && key.forced[forced_pos] < block_start {
                forced_pos += 1;
            }
            let mut segment_start = block_start;
            let mut p = forced_pos;
            while p < key.forced.len() && key.forced[p] < block_end {
                let cut = key.forced[p];
                if cut > segment_start {
                    self.push_quick_segment(segment_start, cut, &mut out);
                }
                segment_start = cut;
                p += 1;
            }
            if segment_start < block_end {
                self.push_quick_segment(segment_start, block_end, &mut out);
            }
            forced_pos = p;
            block_start = block_end;
        }
        out.sort_unstable();
        out.dedup();
        let out = Arc::new(out);
        cache.insert(key, Arc::clone(&out));
        out
    }

    fn push_quick_segment(&self, start: usize, end: usize, out: &mut Vec<usize>) {
        debug_assert!(start < end && end <= self.len);
        let last = end - 1;
        let mut min_bid = start;
        let mut max_bid = start;
        let mut min_ask = start;
        let mut max_ask = start;
        let mut min_bid_px = self.bid(start);
        let mut max_bid_px = min_bid_px;
        let mut min_ask_px = self.ask(start);
        let mut max_ask_px = min_ask_px;
        out.push(start);
        for i in (start + 1)..end {
            let bid = self.bid(i);
            let ask = self.ask(i);
            if bid < min_bid_px { min_bid = i; min_bid_px = bid; }
            if bid > max_bid_px { max_bid = i; max_bid_px = bid; }
            if ask < min_ask_px { min_ask = i; min_ask_px = ask; }
            if ask > max_ask_px { max_ask = i; max_ask_px = ask; }
            // A block spanning a feed/weekend gap must retain the quote on
            // both sides.  Besides swap/day handling, this prevents a timer
            // from appearing to react before the first post-gap market row.
            if self.ts(i).saturating_sub(self.ts(i - 1)) > 60_000 {
                out.push(i - 1);
                out.push(i);
            }
        }
        out.extend([min_bid, max_bid, min_ask, max_ask, last]);
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
    /// KANAŁ ŹRÓDŁOWY — nazwa FORMATU, którym ten sygnał został podany.
    ///
    /// Do 03.08.2026 zbiór sygnałów pochodził z jednego kanału i pole było
    /// niepotrzebne. Odkąd backtest liczy kilka presetów naraz
    /// (`--preset-format`), to ono rozstrzyga, KTÓRY silnik dostanie
    /// wiadomość — dokładnie tak, jak na żywo rozstrzyga o tym format
    /// przypisany kanałowi w panelu.
    ///
    /// Puste = zbiór jednokanałowy. Przy jednym presecie nikt tego pola nie
    /// czyta, więc wszystkie starsze pliki działają bez zmiany.
    #[serde(default)]
    pub kanal: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEvent {
    pub ts: i64,
    /// Identyfikator WIADOMOŚCI źródłowej z Telegrama.
    ///
    /// Jedna wiadomość niesie zwykle kilka poleceń („+30 PIPS HIT / RISK FREE
    /// 4668"), a generator zapisuje każde osobnym zdarzeniem — inaczej znikały:
    /// 615 wiadomości z „RISK FREE" zostawiało w danych 209 zdarzeń.
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
    /// JAWNY POZIOM STOPU podany przez sygnalistę („⛔ SL IS SET TO BE AT 4668").
    ///
    /// Niesie go 530 z 531 komunikatów `SPP` w korpusie 04–07.2026. Do 31.07
    /// rekonstrukcja z `kind` produkowała goły napis „SECURING PARTIAL PROFITS"
    /// i liczba przepadała — backtest nie miał czego czytać, nawet gdyby rdzeń
    /// umiał na nią reagować. Odtwarzamy ją **słowami sygnalisty**, żeby
    /// wyciągnął ją prawdziwy parser (`core::parser::RE_BE_AT`), a nie kanał
    /// boczny: inaczej mierzylibyśmy inną ścieżkę niż ta, którą chodzi bot
    /// na żywo.
    #[serde(default)]
    pub be: Option<f64>,
    /// ORYGINALNA treść komunikatu zarządzającego, jeśli eksport ją zachował.
    ///
    /// Bez niej komunikat odtwarzamy z samego `kind`, a `kind` jest JEDEN na
    /// wiadomość — podczas gdy jedna wiadomość kanału niesie zwykle kilka
    /// poleceń naraz. Wzorcowy przypadek: „+20 PIPS HIT 🔥 / RISK FREE 4090"
    /// zapisuje się jako `TP_HIT` i polecenie RISK FREE znika bez śladu.
    /// W zbiorze 04–07.2026 dotyczy to 401 z 614 komunikatów zawierających
    /// „RISK FREE" (65 %). Parser silnika (`core::parser::parse`) od początku
    /// zwraca WIELE sygnałów z jednego tekstu, więc wystarczy dać mu tekst.
    #[serde(default)]
    pub text: String,
    /// EDYCJA WIADOMOŚCI — identyfikator oryginału, który ta wersja zastępuje.
    ///
    /// Korpus rozszerzony (`signals_SYN_edycje_0817.json`, kontrakt
    /// `wiedza/KORPUS_EDYCJE.md` §5.2) zapisuje każdą poprawkę jako OSOBNE
    /// zdarzenie `kind = "EDIT"` z `ts` = chwila edycji i `text` = treść
    /// finalna. Bez tego pola loader odtwarzał taką poprawkę jako zwykłą,
    /// niezależną wiadomość — czyli DUBLOWAŁ treść: kanał, który edytuje 65 %
    /// wiadomości, dostawał w backteście drugie „TP1 HIT" i drugie wejście.
    ///
    /// `None` = starszy eksport (brak pola) albo zdarzenie `CMD`; wtedy
    /// ścieżka jest dokładnie ta sama co przed 17.08.2026.
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

/// Jedna wiadomość w strumieniu odtwarzania — dokładnie taka, jaką dostałby
/// bot na żywo, wraz z odpowiedziami i edycjami.
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

/// Buduje strumień wiadomości z wyeksportowanych sygnałów.
/// Komunikaty zarządzające dostają `reply_to` wskazujące na sygnał, do którego
/// należą — dzięki temu silnik wiąże je tak samo jak na żywo.
///
/// # Edycje (od 17.08.2026, kontrakt `wiedza/KORPUS_EDYCJE.md` §5)
///
/// Korpus rozszerzony niesie każdą poprawkę jako zdarzenie `kind = "EDIT"`
/// z polem `edit_of`. Takie zdarzenie staje się `ReplayMessage` w chwili
/// EDYCJI, z `edit_of = Some(oryginał)` i tym samym `msg_id` co oryginał —
/// czyli dokładnie tym, co na żywo przynosi `MessageEdited`. Kanał Synergy
/// edytuje 65 % wiadomości (mediana +27 s dla wejść, +32 s dla komunikatów),
/// więc bez tego backtest widział poprawioną treść już w chwili publikacji.
///
/// Uwaga do wierności (ograniczenie ŹRÓDŁA, nie kodu): eksport Telegrama
/// przechowuje wyłącznie treść OSTATECZNĄ, więc oryginał niesie tu tekst po
/// poprawce. Informacją jest CHWILA edycji, nie różnica treści — pomiar na
/// takim korpusie jest dla oryginału optymistyczny.
///
/// Pliki bez pól edycyjnych (cała rodzina `signals_*_v2.json`,
/// `signals_SYN_final_0817.json`) wczytują się dokładnie jak dotąd: `edit_of`
/// jest wtedy `None` na każdym zdarzeniu i żadna gałąź nie zmienia zdania.
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

/// Wczytuje wiadomości i przesuwa cały strumień o JAWNIE podaną liczbę
/// minut.
///
/// Jest to normalizacja zegara WYŁĄCZNIE dla replayu. Historyczne eksporty
/// Telegrama zapisują Unix UTC, podczas gdy część eksportów ticków MT5
/// zachowuje zegar serwera brokera. Nie jest to oś strategii i nie wolno jej
/// ukrywać w presecie: ten sam preset ma podejmować te same decyzje po
/// dostarczeniu mu poprawnie zsynchronizowanego strumienia.
///
/// `0` zachowuje dotychczasową ścieżkę co do znacznika. Dotyczy to również
/// kanonicznej kroniki `messages`: jej `received_at_ms` jest już rzeczywistą
/// chwilą odbioru i pozostaje nietknięte, chyba że operator świadomie poda
/// niezerowe przesunięcie w CLI dla tego konkretnego korpusu.
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
        // Both timestamps describe the same clock. Moving only arrival
        // fabricates latency and can reject a fresh signal as hours old.
        if let Some(published) = m.telegram_published_ts {
            m.telegram_published_ts = Some(published.checked_add(offset_ms).ok_or_else(|| {
                anyhow::anyhow!("publication timestamp overflows after explicit clock shift")
            })?);
        }
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
        // An empty edit is still a revision: live uses it to withdraw a
        // deferred entry. Dropping it makes replay open the obsolete signal.
        .filter(|(_, m)| m.edit_of.is_some() || !m.text.trim().is_empty())
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

    // JEDNA WIADOMOŚĆ KANAŁU = JEDNA WIADOMOŚĆ W STRUMIENIU.
    //
    // Generator rozbija wiadomość na tyle zdarzeń, ile niesie poleceń, i każde
    // dostaje tę samą pełną treść. Silnik z jednej treści i tak wyciąga
    // wszystkie polecenia (`core::parser::parse` zwraca `Vec<Signal>`), więc
    // odtworzenie kilku wiadomości z jednej byłoby wykonaniem jej kilka razy:
    // koszyk dwa razy inkasowałby transzę na tym samym celu.
    //
    // Identyfikatory sygnałów wchodzą do zbioru z góry — wiadomość będąca
    // JEDNOCZEŚNIE wejściem i poleceniem (np. „CANCEL … BUY GOLD @ …") jest już
    // w strumieniu jako wejście i drugi raz się nie pojawi.
    // Klucz dedupu MUSI zawierać kanał: identyfikatory Telegrama są per-czat,
    // więc w korpusie dwukanałowym (PARA) id zdarzeń ZEN kolidują z id
    // Synergy — dedup po samym `i64` zjadał drugi kanał (zmierzone: 690
    // z 4827 zdarzeń, w tym ~90 % komunikatów zarządzających ZEN).
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
        // ORYGINALNA treść wiadomości, jeśli eksport ją zachował.
        //
        // Odtwarzanie tekstu z pól było luką wierności: na żywo bot dostaje
        // pełną wiadomość i widzi w niej wszystkie znaczniki, a odtworzona
        // wersja niosła tylko „HIGH RISK TRADE". Znikały m.in. „FIRST ENTRY CAN
        // BE" i „MAY NOT BE AROUND", więc `skip_tags` w backteście nie miał na
        // czym pracować i po cichu nic nie filtrował — konfiguracja z filtrem
        // dawała wynik co do centa taki sam jak bez niego.
        //
        // Zapasowa rekonstrukcja zostaje dla starszych eksportów bez pola
        // `text`; nowe dane idą oryginałem.
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
                // KLUCZ DEDUPU to identyfikator WERSJI wiadomości w pliku, nie
                // ten, który pójdzie do silnika. Generator korpusu daje
                // oryginałowi prawdziwy `msg_id` Telegrama, a jego edycji
                // `edit_of + 10_000_000` (KORPUS_EDYCJE §5.2) — dwie wersje
                // tej samej wiadomości są więc rozróżnialne i dedup „kolejne
                // polecenie z tej samej wiadomości" NIE zjada edycji. Gdyby
                // kluczem był identyfikator wysyłany do silnika (patrz `msg_id`
                // niżej), każda edycja przepadałaby jako duplikat oryginału —
                // a to 3544 zdarzenia w korpusie Synergy.
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
                // EDYCJA JEST TĄ SAMĄ WIADOMOŚCIĄ, NIE NOWĄ. Na żywo Telethon
                // przynosi poprawkę z TYM SAMYM `msg_id` co oryginał i z
                // `edit_of = Some(ten sam id)` (`telegram::incoming::
                // from_message`, gałąź `is_edit`). Strumień odtwarzania musi
                // wyglądać identycznie, bo silnik szuka koszyka w
                // `msg_to_basket` pod kluczem `edit_of.unwrap_or(msg_id)`:
                // ze sztucznym numerem 10-milionowym edycja byłaby SIEROTĄ,
                // a cały Pakiet A (przezbrojenie strefy, dedup akcji) mierzyłby
                // co innego niż produkcja.
                //
                // Starszy eksport pola `edit_of` nie ma, więc `unwrap_or`
                // oddaje dawne zachowanie co do bitu.
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
                // POZIOM STOPU MUSI ZOSTAĆ W TREŚCI — dokładnie tak samo, jak
                // musiał w niej zostać numer celu (patrz `TP_HIT` wyżej).
                // Formuła jest dosłownym zdaniem sygnalisty, więc wyciąga ją
                // ten sam `RE_BE_AT`, który pracuje na żywo.
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

    fn quick_tape(prices: &[f32]) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "conduit_quick_ticks_{}_{}.bin",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut bytes = vec![0u8; HEADER];
        bytes[..4].copy_from_slice(&MAGIC.to_le_bytes());
        bytes[8..16].copy_from_slice(&(prices.len() as u64).to_le_bytes());
        for (i, bid) in prices.iter().copied().enumerate() {
            bytes.extend_from_slice(&(1_700_000_000_000i64 + i as i64 * 1000).to_le_bytes());
            bytes.extend_from_slice(&bid.to_le_bytes());
            bytes.extend_from_slice(&(bid + 0.2).to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn quick_selector_is_ordered_extrema_preserving_and_cached() {
        let path = quick_tape(&[10.0, 5.0, 11.0, 8.0, 20.0, 12.0, 9.0, 13.0]);
        let ticks = TickData::open(&path).unwrap();
        let a = ticks.quick_extrema_indices(0, ticks.len(), 8, &[]);
        let b = ticks.quick_extrema_indices(0, ticks.len(), 8, &[]);
        assert!(std::sync::Arc::ptr_eq(&a, &b), "ten sam sweep współdzieli indeks");
        assert!(a.windows(2).all(|w| w[0] < w[1]));
        for required in [0, 1, 4, 7] {
            assert!(a.contains(&required), "brak endpoint/extremum {required}: {a:?}");
        }
        drop(ticks);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn forced_message_row_restarts_extrema_so_future_is_not_lost() {
        let path = quick_tape(&[10.0, 1.0, 100.0, 50.0, 40.0, 60.0, 55.0, 90.0, 70.0, 65.0]);
        let ticks = TickData::open(&path).unwrap();
        let unsplit = ticks.quick_extrema_indices(0, ticks.len(), 10, &[]);
        let split = ticks.quick_extrema_indices(0, ticks.len(), 10, &[5]);
        assert!(!unsplit.contains(&7), "global max before message hides local post-message max");
        assert!(split.contains(&4), "physical quote immediately before message is retained");
        assert!(split.contains(&5), "message is dispatched on its first physical row");
        assert!(split.contains(&7), "post-message extrema are selected independently");
        drop(ticks);
        std::fs::remove_file(path).unwrap();
    }

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
            "lo":4660.0,"hi":4665.0,"sl":4655.0,"tps":[4670.0],"kanal":"Synergy",
            "text":"BUY GOLD @ 4665/4660\nTP 4670\nSL 4655",
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
            "lo":4660.0,"hi":4665.0,"sl":4655.0,"tps":[4670.0],
            "text":"BUY GOLD @ 4665/4660\nTP 4670\nSL 4655","events":[
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

    #[test]
    fn raw_empty_edit_is_not_lost_before_deferred_entry_management() {
        let p = zapisz(r#"{"messages":[
            {"ts":1000,"msg_id":101,"text":"BUY GOLD @ 3100/3098 SL 3090 TP 3110"},
            {"ts":1100,"msg_id":101,"edit_of":101,"text":""},
            {"ts":1200,"msg_id":102,"text":"   "}
        ]}"#);
        let messages = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].edit_of, Some(101));
        assert_eq!(messages[1].ts, 1100);
        assert!(messages[1].text.is_empty());
    }

    #[test]
    fn raw_explicit_clock_shift_preserves_receipt_age() {
        let p = zapisz(r#"{"messages":[
            {"ts":11000,"telegram_published_ts":10000,"msg_id":101,"text":"A"},
            {"ts":22000,"latency_ms":2500,"msg_id":102,"text":"B"},
            {"ts":33000,"msg_id":103,"text":"C"}
        ]}"#);
        for offset in [-180, 180] {
            let messages = load_messages_with_time_offset(&p, offset).unwrap();
            assert_eq!(messages[0].ts - messages[0].telegram_published_ts.unwrap(), 1000);
            assert_eq!(messages[1].ts - messages[1].telegram_published_ts.unwrap(), 2500);
            assert_eq!(messages[2].telegram_published_ts, None);
        }
        let _ = std::fs::remove_file(&p);
    }

    /// REGRESJA: wiadomość niosąca DWA polecenia zapisuje się jako dwa
    /// zdarzenia, ale do strumienia trafia RAZ.
    ///
    /// Generator musi rozbić „+30 PIPS HIT / RISK FREE 4668" na `TP_HIT`
    /// i `RISK_FREE`, bo inaczej polecenie RISK FREE znika z danych (615
    /// wiadomości w eksporcie dawało 209 zdarzeń). Skoro jednak oba zdarzenia
    /// niosą tę samą pełną treść, a parser silnika wyciąga z jednej treści
    /// wszystkie polecenia, odtworzenie dwóch wiadomości byłoby wykonaniem
    /// tego samego dwa razy: koszyk drugi raz inkasowałby transzę na TP1.
    #[test]
    fn dwa_zdarzenia_jednej_wiadomosci_to_jedna_wiadomosc() {
        let p = zapisz(
            r#"{"signals":[{"id":10,"ts":100,"dir":"BUY","limit":false,"lo":4660.0,
            "hi":4665.0,"sl":4655.0,"tps":[4670.0],"text":"BUY GOLD @ 4665/4660\nTP 4670\nSL 4655",
            "events":[
              {"ts":200,"msg_id":11,"kind":"TP_HIT","val":1.0,"text":"+30 PIPS HIT\n\nRISK FREE 4668"},
              {"ts":200,"msg_id":11,"kind":"RISK_FREE","val":4668.0,"text":"+30 PIPS HIT\n\nRISK FREE 4668"}
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
            r#"{"signals":[{"id":10,"ts":100,"dir":"BUY","limit":false,"lo":4660.0,
            "hi":4665.0,"sl":4655.0,"tps":[4670.0],
            "events":[{"ts":200,"kind":"TP_HIT","val":1.0},{"ts":300,"kind":"RISK_FREE","val":4668.0}]}]}"#,
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
            r#"{"signals":[{"id":10,"ts":100,"dir":"BUY","limit":false,"lo":4660.0,
            "hi":4665.0,"sl":4655.0,"tps":[4670.0],
            "text":"CANCEL THE LIMITS\nBUY GOLD @ 4665/4660\nTP 4670\nSL 4655",
            "events":[{"ts":100,"msg_id":10,"kind":"CANCEL","text":"CANCEL THE LIMITS\nBUY GOLD @ 4665/4660\nTP 4670\nSL 4655"}]}]}"#,
        );
        let m = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].msg_id, 10);
    }

    /// REGRESJA: poziom stopu podany przez sygnalistę przeżywa rekonstrukcję
    /// z samego `kind` i daje się wyciągnąć PRAWDZIWYM parserem.
    ///
    /// Do 31.07.2026 `"SPP"` odtwarzało się jako goły napis „SECURING PARTIAL
    /// PROFITS", więc liczba z „SL IS SET TO BE AT 4668" przepadała — jedyna
    /// informacja pochodząca od AUTORA sygnału nie docierała do silnika.
    #[test]
    fn spp_odtworzony_z_kind_niesie_poziom_stopu() {
        let p = zapisz(
            r#"{"signals":[{"id":10,"ts":100,"dir":"BUY","limit":false,"lo":4660.0,
            "hi":4665.0,"sl":4655.0,"tps":[4670.0],
            "events":[{"ts":200,"kind":"SPP","be":4668.0}]}]}"#,
        );
        let m = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(m.len(), 2);
        assert!(m[1].text.contains("SECURING PARTIAL PROFITS"));
        // ten sam parser, którym chodzi bot na żywo
        let poziom = conduit_core::parser::parse(&m[1].text)
            .into_iter()
            .find_map(|s| match s {
                conduit_core::parser::Signal::SecuringPartial { spp_be_level, .. } => {
                    Some(spp_be_level)
                }
                _ => None,
            })
            .expect("SPP musi się rozpoznać");
        assert_eq!(poziom, Some(4668.0));
    }

    // ============ PAKIET C1: EDYCJE W KORPUSIE (KORPUS_EDYCJE §5) ============

    /// Syntetyczny korpus rozszerzony: wejście edytowane po 40 s i komunikat
    /// zarządzający edytowany po 20 s. Struktura 1:1 z
    /// `data/signals_SYN_edycje_0817.json` (`msg_id` edycji = `edit_of`
    /// + 10 000 000, `ts` = chwila poprawki, `text` = treść finalna).
    const KORPUS_Z_EDYCJAMI: &str = r#"{"signals":[{"id":10,"ts":100,"dir":"BUY",
      "limit":true,"lo":4660.0,"hi":4665.0,"sl":4655.0,"tps":[4670.0],"kanal":"Synergy",
      "text":"BUY LIMITS GOLD @ 4665/4660\nTP 4670\nTP 4680\nSL 4655","reply_to":null,
      "events":[
        {"kind":"EDIT","msg_id":10000010,"ts":140,"edit_of":10,"reply_to":null,
         "text":"BUY LIMITS GOLD @ 4665/4660\nTP 4670\nTP 4680\nSL 4655"},
        {"kind":"CMD","msg_id":11,"ts":200,"reply_to":10,"text":"TP1 HIT +30 PIPS"},
        {"kind":"EDIT","msg_id":10000011,"ts":220,"edit_of":11,"reply_to":10,
         "text":"TP1 HIT +30 PIPS\n\nRISK FREE 4668"}
      ],"edited":140}]}"#;

    /// Zdarzenie `EDIT` wchodzi do strumienia jako EDYCJA: własna chwila,
    /// `edit_of` oryginału i ten sam `msg_id` co oryginał — czyli to samo,
    /// co na żywo przynosi Telethon.
    #[test]
    fn edycja_wchodzi_jako_edycja_a_nie_nowa_wiadomosc() {
        let p = zapisz(KORPUS_Z_EDYCJAMI);
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
        assert!(m[3].text.contains("RISK FREE 4668"));

        // kanał dziedziczy się na edycjach tak samo jak na komunikatach —
        // inaczej edycja z Synergy trafiłaby do silnika ZEN
        assert!(m.iter().all(|x| x.kanal == "Synergy"));
        // każda edycja wskazuje sygnał, do którego należy
        assert_eq!(m[1].reply_to, Some(10));
        assert_eq!(m[3].reply_to, Some(10));
    }

    /// REGRESJA KLUCZOWA: dedup „jedna wiadomość kanału = jedna wiadomość
    /// w strumieniu" nie ma prawa zjeść edycji.
    ///
    /// Edycja jedzie do silnika z `msg_id` oryginału, więc gdyby to on był
    /// kluczem dedupu, przepadłyby wszystkie 3544 edycje korpusu Synergy —
    /// i to CICHO, bo brak wiadomości nie jest błędem. Kluczem jest
    /// identyfikator WERSJI z pliku (`edit_of + 10 000 000`).
    #[test]
    fn dedup_nie_zjada_edycji_mimo_tego_samego_msg_id() {
        let p = zapisz(KORPUS_Z_EDYCJAMI);
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
            r#"{"signals":[{"id":10,"ts":100,"dir":"BUY","limit":false,"lo":4660.0,
            "hi":4665.0,"sl":4655.0,"tps":[4670.0],"text":"BUY GOLD @ 4665/4660\nTP 4670\nSL 4655",
            "events":[
              {"kind":"EDIT","msg_id":10000011,"ts":220,"edit_of":11,"text":"+30 PIPS HIT\n\nRISK FREE 4668"},
              {"kind":"EDIT","msg_id":10000011,"ts":220,"edit_of":11,"text":"+30 PIPS HIT\n\nRISK FREE 4668"}
            ]}]}"#,
        );
        let m = load_messages(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        assert_eq!(m.len(), 2, "wejście + JEDNA edycja");
        assert_eq!(m[1].edit_of, Some(11));
    }

    /// PARYTET: korpus BEZ pól edycyjnych wczytuje się dokładnie jak dotąd —
    /// żadna wiadomość nie staje się edycją, identyfikatory bez zmian.
    #[test]
    fn stary_korpus_bez_pol_edycyjnych_nie_ma_ani_jednej_edycji() {
        let p = zapisz(
            r#"{"signals":[{"id":10,"ts":100,"dir":"BUY","limit":false,"lo":4660.0,
            "hi":4665.0,"sl":4655.0,"tps":[4670.0],"text":"BUY GOLD @ 4665/4660\nTP 4670\nSL 4655",
            "events":[
              {"ts":200,"msg_id":11,"kind":"TP_HIT","val":1.0,"text":"+30 PIPS HIT"},
              {"ts":300,"kind":"RISK_FREE","val":4668.0}
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
