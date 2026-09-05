
use memmap2::Mmap;
use serde::Deserialize;
use std::fs::File;
use std::path::Path;

// ============================================================
//  ZRZUT KOSZYKÓW (`koszyki.json`)
// ============================================================

#[derive(Debug, Clone, Deserialize)]
pub struct WarstwaDump {
    pub poziom: i32,
    pub cena_zlecenia: f64,
    pub wolumen: f64,
    #[serde(default)]
    pub fill_ts: i64,
    #[serde(default)]
    pub fill_px: f64,
    #[serde(default)]
    pub anulowana: bool,
    #[serde(default)]
    pub toucher: bool,
    #[serde(default)]
    pub filled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KoszykDump {
    pub id: u32,
    pub msg_id: i64,
    pub side: String,
    pub zone_lo: f64,
    pub zone_hi: f64,
    pub sl: Option<f64>,
    #[serde(default)]
    pub tps: Vec<f64>,
    pub created_ts: i64,
    #[serde(default)]
    pub tp_stage: usize,
    #[serde(default)]
    pub tp_touch_ts: Vec<i64>,
    #[serde(default)]
    pub sl_touch_ts: i64,
    #[serde(default)]
    pub warstwy: Vec<WarstwaDump>,
    #[serde(default)]
    pub pl: f64,
    #[serde(default)]
    pub n_trades: u32,
    #[serde(default)]
    pub first_open_ts: i64,
    #[serde(default)]
    pub last_close_ts: i64,
    #[serde(default)]
    pub reentries: u32,
    #[serde(default)]
    pub rearms: u32,
    #[serde(default)]
    pub had_positions: bool,
    #[serde(default)]
    pub secured: bool,
    #[serde(default)]
    pub peak_pl_usd: f64,
    #[serde(default)]
    pub entry_lo: f64,
    #[serde(default)]
    pub entry_hi: f64,
    #[serde(default)]
    pub state: String,
}

// ============================================================
//  ZRZUT TRANSAKCJI (`transakcje.json`)
// ============================================================

#[derive(Debug, Clone, Deserialize)]
pub struct TransakcjaDump {
    pub ticket: u64,
    pub side: String,
    pub volume: f64,
    pub open_price: f64,
    pub close_price: f64,
    pub open_ts: i64,
    pub close_ts: i64,
    pub profit: f64,
    #[serde(default)]
    pub commission: f64,
    #[serde(default)]
    pub swap: f64,
    pub reason: String,
    pub basket: Option<u32>,
}

impl TransakcjaDump {
    /// wynik NETTO — to, co naprawdę weszło na saldo
    #[inline]
    pub fn netto(&self) -> f64 {
        self.profit - self.commission.abs() + self.swap
    }
    #[inline]
    pub fn stop_zabral(&self) -> bool {
        self.reason == "Sl" || self.reason == "VirtualSl"
    }
}


#[derive(Debug, Clone, Deserialize)]
pub struct ZdarzenieKorpusu {
    pub ts: i64,
    pub kind: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub msg_id: i64,
    #[serde(default)]
    pub edit_of: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SygnalKorpusu {
    pub id: i64,
    /// SEKUNDY (tak jest w pliku) — przeliczamy na ms przy budowie ramy
    pub ts: i64,
    pub dir: String,
    #[serde(default)]
    pub limit: bool,
    pub lo: f64,
    pub hi: f64,
    #[serde(default)]
    pub sl: f64,
    #[serde(default)]
    pub tps: Vec<f64>,
    #[serde(default)]
    pub kanal: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub events: Vec<ZdarzenieKorpusu>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlikKorpusu {
    pub signals: Vec<SygnalKorpusu>,
}

// ============================================================
//  DZIENNIK ZDARZEŃ (`.jsonl`)
// ============================================================

/// Otwarcie pozycji odczytane z dziennika — daje DOKŁADNE przypisanie
/// `ticket → (koszyk, poziom siatki, stop w chwili otwarcia)`.
/// Bez tego przypisanie idzie po cenie i czasie, a to myli się dokładnie
/// tam, gdzie dwa szczeble wypełniają się w tej samej milisekundzie.
#[derive(Debug, Clone)]
pub struct OtwarcieZDziennika {
    pub ticket: u64,
    pub basket: Option<u32>,
    pub level: i32,
    pub sl: Option<f64>,
    pub volume: f64,
    pub open_price: f64,
    pub open_ts: i64,
}

#[derive(Debug, Clone)]
pub struct ZamkniecieZDziennika {
    pub basket: Option<u32>,
    pub mfe_usd: f64,
    pub mae_usd: f64,
    pub left_on_table: f64,
    pub samples: u64,
}

#[derive(Default)]
pub struct Dziennik {
    pub otwarcia: Vec<OtwarcieZDziennika>,
    pub zamkniecia: Vec<ZamkniecieZDziennika>,
    pub linii: u64,
}

pub fn wczytaj_dziennik(p: impl AsRef<Path>) -> std::io::Result<Dziennik> {
    use std::io::{BufRead, BufReader};
    let f = File::open(p)?;
    let r = BufReader::with_capacity(1 << 20, f);
    let mut out = Dziennik::default();
    for l in r.lines() {
        let l = l?;
        out.linii += 1;
        if l.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(&l) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match v.get("kind").and_then(|x| x.as_str()) {
            Some("position_opened") => {
                let d = v.get("data").cloned().unwrap_or(serde_json::Value::Null);
                out.otwarcia.push(OtwarcieZDziennika {
                    ticket: v.get("ticket").and_then(|x| x.as_u64()).unwrap_or(0),
                    basket: v
                        .get("basket_id")
                        .and_then(|x| x.as_u64())
                        .map(|x| x as u32),
                    level: d.get("level").and_then(|x| x.as_i64()).unwrap_or(i64::MIN) as i32,
                    sl: d.get("sl").and_then(|x| x.as_f64()),
                    volume: d.get("volume").and_then(|x| x.as_f64()).unwrap_or(0.0),
                    open_price: d.get("open_price").and_then(|x| x.as_f64()).unwrap_or(0.0),
                    open_ts: d
                        .get("open_ts")
                        .and_then(|x| x.as_i64())
                        .unwrap_or_else(|| {
                            v.get("ts_broker_ms").and_then(|x| x.as_i64()).unwrap_or(0)
                        }),
                });
            }
            Some("position_closed") => {
                if let Some(c) = v.get("close") {
                    let e = c.get("excursion");
                    out.zamkniecia.push(ZamkniecieZDziennika {
                        basket: v
                            .get("basket_id")
                            .and_then(|x| x.as_u64())
                            .map(|x| x as u32),
                        mfe_usd: e
                            .and_then(|x| x.get("mfe_usd"))
                            .and_then(|x| x.as_f64())
                            .unwrap_or(0.0),
                        mae_usd: e
                            .and_then(|x| x.get("mae_usd"))
                            .and_then(|x| x.as_f64())
                            .unwrap_or(0.0),
                        left_on_table: c
                            .get("left_on_table")
                            .and_then(|x| x.as_f64())
                            .unwrap_or(0.0),
                        samples: e
                            .and_then(|x| x.get("samples"))
                            .and_then(|x| x.as_u64())
                            .unwrap_or(0),
                    });
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

pub fn wczytaj_koszyki(p: impl AsRef<Path>) -> anyhow_lite::Result<Vec<KoszykDump>> {
    let s = std::fs::read_to_string(p)?;
    Ok(serde_json::from_str(&s)?)
}

pub fn wczytaj_transakcje(p: impl AsRef<Path>) -> anyhow_lite::Result<Vec<TransakcjaDump>> {
    let s = std::fs::read_to_string(p)?;
    Ok(serde_json::from_str(&s)?)
}

pub fn wczytaj_korpus(p: impl AsRef<Path>) -> anyhow_lite::Result<Vec<SygnalKorpusu>> {
    let s = std::fs::read_to_string(p)?;
    let f: PlikKorpusu = serde_json::from_str(&s)?;
    Ok(f.signals)
}

pub mod anyhow_lite {
    pub type Blad = Box<dyn std::error::Error>;
    pub type Result<T> = std::result::Result<T, Blad>;
}

// ============================================================
//  TICKI (format `CDTK`)
// ============================================================

const MAGIC: u32 = 0x4B54_4443; // "CDTK"
const HEADER: usize = 64;
const REC: usize = 16;

#[repr(C)]
#[derive(Clone, Copy)]
struct SurowyTick {
    ts: i64,
    bid: f32,
    ask: f32,
}

/// Ticki mapowane w pamięć. Kopia czytnika z `conduit_backtest::data` —
/// świadoma, bo zależność od `crates/backtest` wciągnęłaby cały silnik
/// i skasowała granicę 1.
pub struct Ticki {
    _map: Mmap,
    ptr: *const SurowyTick,
    len: usize,
}

unsafe impl Send for Ticki {}
unsafe impl Sync for Ticki {}

impl Ticki {
    pub fn otworz(p: impl AsRef<Path>) -> anyhow_lite::Result<Self> {
        let f = File::open(p.as_ref())?;
        let map = unsafe { Mmap::map(&f)? };
        if map.len() < HEADER {
            return Err("plik ticków za krótki".into());
        }
        let magic = u32::from_le_bytes(map[0..4].try_into().unwrap());
        if magic != MAGIC {
            return Err(format!("zły nagłówek pliku ticków ({magic:#x})").into());
        }
        let count = u64::from_le_bytes(map[8..16].try_into().unwrap()) as usize;
        if map.len() < HEADER + count * REC {
            return Err("plik ticków obcięty".into());
        }
        let ptr = unsafe { map.as_ptr().add(HEADER) } as *const SurowyTick;
        Ok(Ticki {
            _map: map,
            ptr,
            len: count,
        })
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
    pub fn ts(&self, i: usize) -> i64 {
        unsafe { (*self.ptr.add(i)).ts }
    }
    #[inline]
    pub fn bid(&self, i: usize) -> f64 {
        unsafe { (*self.ptr.add(i)).bid as f64 }
    }
    #[inline]
    pub fn ask(&self, i: usize) -> f64 {
        unsafe { (*self.ptr.add(i)).ask as f64 }
    }
    /// pierwszy indeks o znaczniku >= ts
    pub fn indeks_od(&self, ts: i64) -> usize {
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
}
