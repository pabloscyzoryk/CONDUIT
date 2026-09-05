
use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::json;

use crate::proto::{Bar, Candles, Costs, Deals, SymbolInfo, SymbolsList};
use crate::transport::{CallError, TransportHandle};

/// Jak długo wolno podać zbuforowaną paczkę Z BIEŻĄCĄ świecą.
///
/// 800 ms, a nie 5 s: ostatnia świeca żyje i użytkownik ma prawo widzieć,
/// jak rośnie. Krócej nie ma sensu — panel i tak dolepia do niej tick.
const TTL_BIEZACE: Duration = Duration::from_millis(800);

/// Historia domknięta się nie zmienia; trzymamy ją, dopóki nie zabraknie miejsca.
const TTL_HISTORIA: Duration = Duration::from_secs(600);

/// Parametry instrumentu zmieniają się rzadko, ale JEDNAK zmieniają — broker
/// potrafi rozszerzyć `stops_level` przed danymi makro. Minuta to kompromis
/// między „nie odpytuj bez sensu" a „nie kłam użytkownikowi o poziomie stopu".
const TTL_SYMBOL: Duration = Duration::from_secs(60);

/// Ile paczek historii trzymamy, zanim zaczniemy wyrzucać najstarsze.
const MAX_PACZEK: usize = 64;

/// Sufit liczby świec na jedno żądanie — ten sam, co w sidecarze.
pub const MAX_BARS: usize = 5000;

/// Sufit liczby dealów na jedno żądanie — ten sam, co w sidecarze.
pub const MAX_DEALS: usize = 5000;

#[derive(Clone)]
struct Wpis {
    kiedy: Instant,
    dane: Candles,
}

#[derive(Default)]
struct Pamiec {
    /// klucz: (symbol, interwał) — wyłącznie paczka „od teraz wstecz"
    biezace: HashMap<(String, String), Wpis>,
    /// klucz: (symbol, interwał, `to`, liczba)
    historia: HashMap<(String, String, i64, usize), Wpis>,
    symbole: HashMap<String, (Instant, SymbolInfo)>,
}

/// Odczyty rynkowe dla panelu. Klonowalny, bezpieczny między wątkami.
pub struct MarketData {
    tr: TransportHandle,
    pam: Mutex<Pamiec>,
    /// ile żądań obsłużyliśmy z pamięci, a ile poszło do terminala —
    /// do `/api/diag`, żeby dało się zobaczyć, czy bufor w ogóle działa
    trafienia: std::sync::atomic::AtomicU64,
    pudla: std::sync::atomic::AtomicU64,
}

impl MarketData {
    pub fn new(tr: TransportHandle) -> Self {
        MarketData {
            tr,
            pam: Mutex::new(Pamiec::default()),
            trafienia: std::sync::atomic::AtomicU64::new(0),
            pudla: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn is_connected(&self) -> bool {
        self.tr.is_connected()
    }

    /// Instrument silnika — domyślny, gdy pytający nie poda żadnego.
    pub fn default_symbol(&self) -> &str {
        self.tr.symbol()
    }

    pub fn stats(&self) -> (u64, u64) {
        use std::sync::atomic::Ordering::Relaxed;
        (self.trafienia.load(Relaxed), self.pudla.load(Relaxed))
    }

    pub fn symbol_info(&self, symbol: &str) -> Result<SymbolInfo, CallError> {
        let klucz = symbol.to_string();
        if let Some((kiedy, si)) = self.pam.lock().symbole.get(&klucz) {
            if kiedy.elapsed() < TTL_SYMBOL {
                self.trafienie();
                return Ok(si.clone());
            }
        }
        self.pudlo();
        let si: SymbolInfo = self
            .tr
            .call_as("symbol_info", json!({ "symbol": symbol }))?;
        self.pam
            .lock()
            .symbole
            .insert(klucz, (Instant::now(), si.clone()));
        Ok(si)
    }

    /// Lista symboli brokera — przelotka do operacji `symbols` sidecara.
    ///
    /// `q = None` — bez filtra. Bez bufora: wyszukiwarka pyta przy wpisywaniu,
    /// a obronę przed brokerem z tysiącami CFD robi sidecar (> 5000
    /// instrumentów = tylko Podgląd rynku + trafienia filtra). Sortowanie
    /// (widoczne najpierw, potem alfabetycznie) też robi sidecar — tu nic
    /// nie przestawiamy, żeby nie powstała druga, konkurencyjna reguła.
    pub fn symbols(&self, q: Option<&str>) -> Result<SymbolsList, CallError> {
        self.tr.call_as("symbols", json!({ "q": q, "filter": q }))
    }

    /// Świece. `to = None` — najnowsze; `to = Some(t)` — wyłącznie STARSZE niż `t`.
    ///
    /// `t` jest czasem otwarcia najstarszej świecy, którą pytający już ma;
    /// granica jest wyłączna, więc paczki się nie nakładają i da się je sklejać
    /// bez odsiewania duplikatów.
    pub fn candles(
        &self,
        symbol: &str,
        tf: &str,
        count: usize,
        to: Option<i64>,
    ) -> Result<Candles, CallError> {
        let count = count.clamp(1, MAX_BARS);
        match to {
            None => self.biezace(symbol, tf, count),
            Some(t) => self.historia(symbol, tf, count, t),
        }
    }

    fn biezace(&self, symbol: &str, tf: &str, count: usize) -> Result<Candles, CallError> {
        // Klucz po interwale W POSTACI PODANEJ przez pytającego, nie po
        // kanonicznej: normalizację robi sidecar, a my nie chcemy jej
        // powtarzać w drugim miejscu (dwie kopie tej samej tabeli to dwie
        // okazje, żeby się rozjechały). Koszt: „5m" i „M5" zajmą dwa wpisy.
        let klucz = (symbol.to_string(), tf.to_string());
        if let Some(w) = self.pam.lock().biezace.get(&klucz) {
            if w.kiedy.elapsed() < TTL_BIEZACE && w.dane.bars.len() >= count {
                self.trafienie();
                let mut cached = przytnij(&w.dane, count);
                cached.age_clock_by(w.kiedy.elapsed());
                return Ok(cached);
            }
        }
        self.pudlo();
        let mut dane: Candles = self.tr.call_as(
            "candles",
            json!({ "symbol": symbol, "tf": tf, "count": count }),
        )?;
        // Buforujemy CAŁOŚĆ tego, co przyszło — kolejne żądanie o mniej świec
        // obsłużymy z pamięci.
        self.pam.lock().biezace.insert(
            klucz,
            Wpis {
                kiedy: Instant::now(),
                dane: dane.clone(),
            },
        );
        dane = przytnij(&dane, count);
        Ok(dane)
    }

    fn historia(
        &self,
        symbol: &str,
        tf: &str,
        count: usize,
        to: i64,
    ) -> Result<Candles, CallError> {
        let klucz = (symbol.to_string(), tf.to_string(), to, count);
        if let Some(w) = self.pam.lock().historia.get(&klucz) {
            if w.kiedy.elapsed() < TTL_HISTORIA {
                self.trafienie();
                let mut cached = w.dane.clone();
                cached.age_clock_by(w.kiedy.elapsed());
                return Ok(cached);
            }
        }
        self.pudlo();
        let dane: Candles = self.tr.call_as(
            "candles",
            json!({ "symbol": symbol, "tf": tf, "count": count, "to": to }),
        )?;
        let mut p = self.pam.lock();
        if p.historia.len() >= MAX_PACZEK {
            // najstarszy wpis wylatuje; przy 64 paczkach po 5000 świec to i tak
            // najwyżej kilkanaście MB, a zwykle o rząd wielkości mniej
            if let Some(k) = najstarszy(&p.historia) {
                p.historia.remove(&k);
            }
        }
        p.historia.insert(
            klucz,
            Wpis {
                kiedy: Instant::now(),
                dane: dane.clone(),
            },
        );
        Ok(dane)
    }

    pub fn deals(
        &self,
        from: Option<i64>,
        to: Option<i64>,
        symbol: Option<&str>,
        magic: Option<i64>,
        out_only: bool,
        offset: usize,
        limit: usize,
    ) -> Result<Deals, CallError> {
        let mut args = serde_json::Map::new();
        if let Some(f) = from {
            args.insert("from".into(), f.into());
        }
        if let Some(t) = to {
            args.insert("to".into(), t.into());
        }
        if let Some(s) = symbol {
            args.insert("symbol".into(), s.into());
        }
        if let Some(m) = magic {
            args.insert("magic".into(), m.into());
        }
        if out_only {
            args.insert("out_only".into(), true.into());
        }
        args.insert("offset".into(), offset.into());
        args.insert("limit".into(), limit.clamp(1, MAX_DEALS).into());
        self.tr
            .call_as("history_deals", serde_json::Value::Object(args))
    }

    pub fn costs(&self, days: f64) -> Result<Costs, CallError> {
        self.tr.call_as("costs", json!({ "days": days }))
    }

    /// Czyści pamięć — wołane po zerwaniu połączenia z sidecarem, żeby po
    /// powrocie nie podać świec sprzed awarii jako bieżących.
    pub fn invalidate(&self) {
        let mut p = self.pam.lock();
        p.biezace.clear();
        p.historia.clear();
        p.symbole.clear();
    }

    fn trafienie(&self) {
        self.trafienia
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn pudlo(&self) {
        self.pudla
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Ostatnie `count` świec z paczki. Reszta metadanych bez zmian.
fn przytnij(c: &Candles, count: usize) -> Candles {
    let bars: Vec<Bar> = if c.bars.len() > count {
        c.bars[c.bars.len() - count..].to_vec()
    } else {
        c.bars.clone()
    };
    Candles { bars, ..c.clone() }
}

fn najstarszy<K: Clone>(m: &HashMap<K, Wpis>) -> Option<K> {
    m.iter()
        .min_by_key(|(_, w)| w.kiedy)
        .map(|(k, _)| k.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paczka(n: usize) -> Candles {
        Candles {
            symbol: "XAUUSD".into(),
            tf: "M5".into(),
            bar_ms: 300_000,
            digits: 2,
            point: 0.01,
            server_time_ms: Some(1_000_000),
            utc_time_ms: 0,
            quote_observed_utc_ms: None,
            quote_observation_age_ms: None,
            bars: (0..n)
                .map(|i| Bar(1000 * i as i64, 1.0, 2.0, 0.5, 1.5, 10, 23))
                .collect(),
        }
    }

    #[test]
    fn przycinanie_zostawia_NAJNOWSZE_swiece() {
        let p = paczka(10);
        let c = przytnij(&p, 3);
        assert_eq!(c.bars.len(), 3);
        // najnowsze = te o największym `t`; obcinamy POCZĄTEK, nie koniec
        assert_eq!(c.bars[0].t(), 7000);
        assert_eq!(c.bars[2].t(), 9000);
        assert_eq!(c.tf, "M5");
    }

    #[test]
    fn przycinanie_ponizej_rozmiaru_nie_dokleja_nicego() {
        let c = przytnij(&paczka(2), 500);
        assert_eq!(
            c.bars.len(),
            2,
            "brak danych nie może się zamienić w wymyślone świece"
        );
    }
}
