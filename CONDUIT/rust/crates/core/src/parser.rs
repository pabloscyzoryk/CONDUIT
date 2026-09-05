
use crate::types::{Px, Side};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Signal {
    Entry(EntrySignal),
    TpHit {
        index: Option<usize>,
    },
    SlHit,
    RiskFree {
        level: Option<Px>,
    },
    SecuringPartial {
        targets: Vec<Px>,
        sl: Option<Px>,
        #[serde(default)]
        spp_be_level: Option<Px>,
    },
    OutAtEntry,
    CloseAll,
    TakePartials,
    Cancel,
    TpCorrection {
        index: usize,
        value: Px,
    },
    SetSl {
        value: Px,
    },
    BreakEven,
    MarketOpen {
        side: Side,
    },
    Info,
}

impl Signal {
    pub fn action_key(&self) -> String {
        match self {
            Signal::Entry(_) => "entry".into(),
            Signal::TpHit { index } => match index {
                Some(i) => format!("tp{i}"),
                None => "tp".into(),
            },
            Signal::SlHit => "sl".into(),
            Signal::RiskFree { .. } => "rf".into(),
            Signal::SecuringPartial { .. } => "spp".into(),
            Signal::OutAtEntry => "oae".into(),
            Signal::CloseAll => "closeall".into(),
            Signal::TakePartials => "partials".into(),
            Signal::Cancel => "cancel".into(),
            Signal::TpCorrection { index, .. } => format!("corr{index}"),
            Signal::SetSl { .. } => "setsl".into(),
            Signal::BreakEven => "be".into(),
            Signal::MarketOpen { side } => format!("mkt{side:?}"),
            Signal::Info => "info".into(),
        }
    }

    pub fn action_key_v2(&self) -> String {
        fn px(v: &Px) -> String {
            v.to_string()
        }
        match self {
            Signal::SetSl { value } => format!("setsl@{}", px(value)),
            Signal::TpCorrection { index, value } => format!("corr{index}@{}", px(value)),
            Signal::RiskFree { level } => match level {
                Some(v) => format!("rf@{}", px(v)),
                None => "rf@-".into(),
            },
            Signal::SecuringPartial {
                targets,
                sl,
                spp_be_level,
            } => {
                let cele = targets.iter().map(px).collect::<Vec<_>>().join("/");
                let sl = sl.as_ref().map(px).unwrap_or_else(|| "-".into());
                let be = spp_be_level.as_ref().map(px).unwrap_or_else(|| "-".into());
                format!("spp@{cele}|sl:{sl}|be:{be}")
            }
            _ => self.action_key(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntrySignal {
    pub side: Side,
    pub is_limit: bool,
    #[serde(default)]
    pub is_stop: bool,
    pub lo: Px,
    pub hi: Px,
    pub sl: Option<Px>,
    pub tps: Vec<Px>,
    pub tp_open: bool,
    #[serde(default)]
    pub warstwy_offset: Option<f64>,
    pub tag_high_risk: bool,
    pub tag_may_not_be_around: bool,
    pub tag_first_entry: bool,
}

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: Lazy<Regex> = Lazy::new(|| Regex::new($pat).unwrap());
    };
}

re!(
    RE_ENTRY_ZONE,
    concat!(
        r"(?i)\b(BUY|SELL)\s*(LIMITS?|STOPS?)?\s*(?:GOLD|XAUUSD|XAU)?\s*(LIMITS?|STOPS?)?\s*@?\s*",
        r"(\d{3,6}(?:[.,]\d{1,3})?)",
        r"\s*(?:[/_]|[-–—]{1,2}|\bTO\b)\s*",
        r"(\d{3,6}(?:[.,]\d{1,3})?)"
    )
);
re!(
    RE_ENTRY_SINGLE,
    concat!(
        r"(?i)\b(BUY|SELL)\s*(LIMITS?|STOPS?)?\s*(?:GOLD|XAUUSD|XAU)\s*(LIMITS?|STOPS?)?\s*@?\s*",
        r"(\d{3,6}(?:[.,]\d{1,3})?)"
    )
);
re!(
    RE_ENTRY_ORDER,
    concat!(
        r"(?i)\b(BUY|SELL)\s*(LIMITS?|STOPS?)\s*(?:GOLD|XAUUSD|XAU)?\s*@?\s*",
        r"(\d{3,6}(?:[.,]\d{1,3})?)"
    )
);

re!(
    RE_ENTRY_AREA_PRZED_AT,
    concat!(
        r"(?i)(\b(?:BUY|SELL)\s*(?:LIMITS?|STOPS?)?\s*",
        r"(?:GOLD|XAUUSD|XAU)\s*(?:LIMITS?|STOPS?)?)\s+AREA\s*(@?)"
    )
);

re!(
    RE_TP,
    r"(?i)\bA?TP\s*\d?\s*[:@=]?\s*\b(\d{3,6}(?:[.,]\d{1,3})?)"
);
re!(
    RE_TP_LIST,
    r"(?i)\bA?TP\s*\d?\s*[:@=]?\s*((?:\b\d{3,6}(?:[.,]\d{1,3})?\b[ \t,/]{0,3})+)"
);
re!(RE_NUMS, r"\d{3,6}(?:[.,]\d{1,3})?");
re!(RE_SL, r"(?i)\bSL\s*[:@=]?\s*\b(\d{3,6}(?:[.,]\d{1,3})?)");
re!(RE_TP_OPEN, r"(?i)\bTP\s*\d?\s*(?:OPEN|[:@=]\s*HOLD)\b");

re!(
    RE_STORM_WEJSCIE,
    concat!(
        r"(?i)\bGOLD\s+(BUY|SELL)\s+ZONE\s+",
        r"(\d{3,6}(?:[.,]\d{1,3})?)\s*[-–—]\s*(\d{3,6}(?:[.,]\d{1,3})?)"
    )
);

re!(
    RE_TP_HIT2,
    r"(?i)\bTP\s*(\d)\s*(?:AND|&|,|\+)\s*(\d)\s*HIT\b"
);
re!(
    RE_TP_HIT,
    r"(?i)\bTP\s*(\d)?\s*HIT\b|\bAT\s+TP\s*(\d)\b|\bHIT\s+TP\s*(\d)?\b"
);
re!(
    RE_TP_HIT_CONFIRMED_INDEXED,
    r"(?i)\bTP\s*\d\s*(?:AND\s+TP\s*\d\s*)?HIT\b|\bHIT\s+TP\s*\d\b"
);
re!(RE_AT_TP_PROXIMITY, r"(?i)\bAT\s+TP\s*\d\b");
re!(
    RE_PIPS_RUNNING,
    r"(?i)\b\d+(?:[.,]\d+)?\s*PIPS?\s+RUNNING\b"
);
re!(RE_TP_HIT_APO, r"(?i)\bTP['’]S\s+HIT\b");
re!(RE_PIPS_HIT, r"(?i)\b\d+\s*PIPS?\s*(?:HIT|DONE|SECURED)");
re!(RE_LEVEL_HIT, r"(?i)\b(\d{3,6}(?:[.,]\d{1,3})?)\s*HIT\b");
re!(RE_SL_HIT, r"(?i)\bSL\s*HIT\b|\bSTOP\s*(?:LOSS)?\s*HIT\b");
re!(
    RE_RF,
    r"(?i)\bRISK\s*FREE\b\s*(?:AT\s*)?[@(]?\s*(\d{3,6}(?:[.,]\d{1,3})?)?"
);
re!(
    RE_RF_TYPO,
    r"(?i)\bRISK\s*(?:FREE{2,}|FEEE|FRE)\b\s*(?:AT\s*)?[@(]?\s*(\d{3,6}(?:[.,]\d{1,3})?)?"
);
re!(
    RE_RF_INTENCJA,
    r"(?i)\b(?:WILL|LOOK\s+TO|TRY(?:ING)?\s+TO|IF\s+POSSIBLE|LOOKING|SCOPE\s+FOR)\b"
);
re!(RE_OAE, r"(?i)\bOUT\s+AT\s+(?:ENTRY|BE)\b");
re!(
    RE_OAE_LUZ,
    r"(?i)\bOUT\b[\s.,:;!—–-]{1,4}\bAT\s+(?:ENTRY|BE)\b"
);
re!(RE_CLOSE, r"(?i)\bCLOSE\s+(?:ALL|EVERYTHING|THE\s+REST)\b");
re!(RE_CANCEL, r"(?i)\bCANCEL\b|\bDELETE\b.*\bLIMIT");
re!(RE_INVALID_ONLY, r"(?im)^\W*INVALID\W*$");
re!(RE_NO_LONGER_VALID, r"(?i)\bNO\s+LONGER\s+VALID\b");
re!(RE_WAIT_NEXT, r"(?i)\bWAIT\s+FOR\s+THE\s+NEXT\s+TRADE\b");
re!(
    RE_NLV_PODMIOT,
    concat!(
        r"(?i)\b(?:ZONE|SETUP|TRADE|ENTRY|LIMITS?)\b[^\n]{0,20}?",
        r"\bIS\s+NO\s+LONGER\s+VALID\b"
    )
);
re!(RE_ZONE_FAILED, r"(?i)\b(?:ZONE|SETUP)\s+FAILED\b");
re!(
    RE_CLOSE_LAYERS,
    concat!(
        r"(?i)\bCLOSE\b[^\n]{0,40}?\bLAYERS?\b|\bCLOSE\s+PARTIALS?\b",
        r"|(?m)^[\W_]*TAKE\s+PARTIALS?\b"
    )
);
re!(
    RE_LAYER_COUNT,
    r"(?i)\bCLOSE\b(?:\s+(?:ALL|OFF|THE|WORST|TOP|BOTH|SOME))*\s*(\d+)?\s*(?:\w+\s+){0,2}?LAYERS?\b"
);
re!(
    RE_LAYER_LEVEL,
    r"(?i)\bCLOSE\s+(\d{3,6}(?:[.,]\d{1,3})?)\s+LAYER\b"
);
re!(
    RE_LAYER_OPTIONAL,
    r"(?i)\bYOU\s+CAN\b|\bIF\s+YOU\b|\bOR\s+(?:YOU\s+CAN\s+)?HOLD\b"
);
re!(RE_SPP, r"(?i)SECURING\s+PARTIAL");
re!(RE_SPP_LUZ, r"(?i)\bCLOSING\s+PARTIAL\s+PROFITS?\b");
re!(
    RE_SPP_TARGETS,
    r"(?i)TARGETS?\s*[;:,]?\s*((?:\s*\d{3,6}(?:[.,]\d{1,3})?)+)"
);
re!(
    RE_BE_AT,
    r"(?i)\bSL\b[^\n\d]{0,25}\bSET\b[^\n\d]{0,15}(\d{3,6}(?:[.,]\d{1,3})?)"
);
re!(
    RE_BE,
    r"(?i)\bSL\s+IS\s+SET\s+TO\s+BE\b|\bBREAK\s*EVEN\b|\bSET\s+BE\b"
);
re!(
    RE_BE_LUZ,
    concat!(
        r"(?i)\b(?:MOVE|SET|MOVING|SETTING|PUT)\b[^\n\d]{0,20}\bSL\b",
        r"[^\n\d]{0,20}\bTO\b[^\n\d]{0,10}\bBE\b"
    )
);
re!(
    RE_CORR,
    r"(?i)USE\s+(\d{3,6}(?:[.,]\d{1,3})?)\s+AS\s+TP\s*(\d)"
);
re!(
    RE_CORR2,
    r"(?i)TP\s*(\d)\s*(?:IS\s+)?(?:ADJUSTED|MOVED|CHANGED|SET)\s+TO\s+(\d{3,6}(?:[.,]\d{1,3})?)"
);
re!(
    RE_CORR3,
    r"(?i)\bTP\s*(\d)\s+SHOULD\s+BE\s+(\d{3,6}(?:[.,]\d{1,3})?)"
);
re!(
    RE_SETSL,
    r"(?i)(?:MOVE|SET)\s+SL\s+(?:TO\s+)?(\d{3,6}(?:[.,]\d{1,3})?)"
);
re!(RE_MARKET_OPEN, r"(?i)\b(BUY|SELL)\s+NOW\b");
re!(
    RE_RANGE_HINT,
    r"\(\s*(\d{3,6}(?:[.,]\d{1,3})?)\s*(?i:TO)\s*(\d{3,6}(?:[.,]\d{1,3})?)\s*\)"
);
re!(RE_HR, r"(?i)HIGH\s+RISK\s+TRADE");

re!(
    RE_WARSTWY,
    r"(?i)(?:ADDING|SUBTRACTING)\s+(\d+(?:[.,]\d+)?)\s*PIPS?\s+(?:TO|FROM)\s+EACH\s+LIMIT\s+ORDER"
);

fn pipsy_na_dolary(n: f64) -> f64 {
    n * 0.10
}

fn warstwy_z_tekstu(text: &str) -> Option<f64> {
    let c = RE_WARSTWY.captures(text)?;
    let n: f64 = c.get(1)?.as_str().replace(',', ".").parse().ok()?;
    if n <= 0.0 {
        return None;
    }
    Some(pipsy_na_dolary(n))
}
re!(RE_HR_LINIA, r"(?im)^[\W_]*HIGH\s*RISK[\W_]*$");
re!(
    RE_HR_LINIA_LOTY,
    r"(?im)^[\W_]*HIGH\s+RISK\b([^\n\d]{0,45})$"
);
re!(RE_MNBA, r"(?i)MAY\s+NOT\s+BE\s+AROUND");
re!(RE_FE, r"(?i)FIRST\s+ENTRY\s+CAN\s+BE");


re!(
    RE_NOVA_SL_AT,
    r"(?i)\b(?:BE|SL)\s*(?:TO\s*)?@\s*(\d{3,6}(?:[.,]\d{1,3})?)"
);

re!(
    RE_INVALID_LINIA,
    r"(?im)^[^\n\d]{0,15}\bINVALID\b[^\n\d]{0,30}$"
);

re!(RE_NOVA_SL_STRATA, r"(?im)\bSL\s*-\s*\d{1,3}\b");


re!(RE_PX_TP_OPEN, r"(?i)\bTP\b[^\n]*?\bOPEN\b");

re!(
    RE_PX_TP_KOTWICA,
    r"(?im)^\s*TP\s*[1-8]\b([\s:=@-]*\d{3,6})?"
);
re!(RE_PX_TP_NUMER, r"(?i)\bTP\s*([1-8])\b([\s:=@-]*\d{3,6})?");
re!(
    RE_PX_TP_REWIZJA,
    r"(?i)\b(?:UPDATED?|APPROACHING|WRONG|EDIT)\b"
);

re!(
    RE_PX_BE,
    concat!(
        r"(?i)\bPUT\s+(?:THE\s+)?(?:BE|BREAK\s*EVEN)\b",
        r"|\bBE\s+NOW\b",
        r"|\bBE\s+FOR\s+SAFETY\b",
        r"|\bBREAK\s*EVEN\s+ACTIVE\b"
    )
);
re!(RE_PX_BE_ODMOWA, r"(?i)\b(?:AT|TO)\s+BE\s+NOW\b");

re!(
    RE_STORM_BE,
    concat!(
        r"(?i)\bIF\s+HOLD\s+SET\s+BE\b",
        r"|\bHOLD\s+RISK\s+WITH\s+BREAK\s*EVEN\b"
    )
);

re!(RE_PX_CUT_LOSS, r"(?i)\bCUT\s+(?:THE\s+)?LOSS(?:ES)?\b");

re!(
    RE_PX_ZLECENIE_LINIA,
    concat!(
        r"(?im)^[\W_]*\b(BUY|SELL)\s+(STOPS?|LIMITS?)\s*(?:GOLD|XAUUSD|XAU)?\s*@?\s*",
        r"(\d{3,6}(?:[.,]\d{1,3})?)\s*$"
    )
);

fn px_wiele_zlecen(text: &str) -> Option<(Side, String, Px, Px)> {
    let mut strona: Option<Side> = None;
    let mut rodzaj: Option<String> = None;
    let mut ceny: Vec<Px> = Vec::new();
    for c in RE_PX_ZLECENIE_LINIA.captures_iter(text) {
        let s = if c[1].eq_ignore_ascii_case("BUY") {
            Side::Buy
        } else {
            Side::Sell
        };
        let r = c[2].to_uppercase();
        let r = if r.starts_with("STOP") {
            "STOP"
        } else {
            "LIMIT"
        }
        .to_string();
        match (&strona, &rodzaj) {
            (Some(s0), Some(r0)) if *s0 != s || *r0 != r => return None,
            _ => {}
        }
        strona = Some(s);
        rodzaj = Some(r);
        ceny.push(num(&c[3])?);
    }
    if ceny.len() < 2 {
        return None;
    }
    let lo = ceny.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = ceny.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    Some((strona?, rodzaj?, lo, hi))
}


re!(
    RE_ZEN_WEJSCIE,
    concat!(
        r"(?i)\bXAUUSD\s+(?:BUY|SELL)\s+\d{3,6}(?:[.,]\d{1,3})?",
        r"(?:\s*(?:[/_]|[-–—]{1,2})\s*\d{3,6}(?:[.,]\d{1,3})?)?"
    )
);
re!(
    RE_ZEN_WEJSCIE_1,
    r"(?i)\bXAUUSD\s+(BUY|SELL)\s+(\d{3,6}(?:[.,]\d{1,3})?)"
);
re!(RE_ZEN_ZONA, r"(?i)\b(?:ZONE|REGION)S?\b");

re!(
    RE_TWP_TP,
    r"(?i)\bTARGET\s+PROFIT\s*\d?\s*[:@=]?\s*(\d{3,6}(?:[.,]\d{1,3})?)"
);
re!(
    RE_TWP_SL,
    r"(?i)\bSTOP\s*LOSS\s*[:@=]?\s*(\d{3,6}(?:[.,]\d{1,3})?)"
);
re!(
    RE_TWP_WEJSCIE,
    r"(?im)\b(BUY|SELL)\s+MARKET\s+ORDER\s*@\s*(\d{3,6}(?:[.,]\d{1,3})?)"
);
re!(
    RE_DANGER_KIER,
    r"(?im)^[^\n]{0,40}?\b(?:XAU\s*[IU]SD|GOLD)\s*(BUY|SELL)\s*NOW\b"
);
re!(
    RE_DANGER_STREFA,
    concat!(
        r"(?m)^\s*(\d{3,6}(?:[.,]\d{1,3})?)",
        r"\s*[-\u{2013}\u{2014}/]\s*",
        r"(\d{3,6}(?:[.,]\d{1,3})?)\s*$"
    )
);

re!(
    RE_RETROSPEKCJA,
    concat!(
        r"(?i)\b(?:IF|FOR)\s+ANYONE\s+(?:THAT\s+|STILL\s+|WHO\s+)?",
        r"(?:TOOK|HELD|HOLDS|HOLDING)\b"
    )
);

re!(
    RE_RECAP_TITLE,
    r"(?i)\b(?:(?:DAILY|WEEKLY|FRIDAY|MONTHLY)\s+)?(?:TRADING\s+|TRADE\s+)?RECAP\b"
);
re!(RE_RECAP_TOTAL, r"(?i)\bTOTAL\s+TRADES?\s*:");
re!(RE_RECAP_RESULTS, r"(?i)\b(?:WINNING|LOSING)\s+TRADES?\s*:");

#[inline]
fn podsumowanie_wielu_transakcji(text: &str) -> bool {
    RE_RECAP_TITLE.is_match(text)
        || (RE_RECAP_TOTAL.is_match(text) && RE_RECAP_RESULTS.is_match(text))
}

fn zapowiedz_strefy(text: &str) -> bool {
    let Some(m) = RE_ZEN_WEJSCIE.find(text) else {
        return false;
    };
    let przed = &text[..m.start()];
    let litery: String = przed.chars().filter(|c| c.is_alphabetic()).collect();
    if litery.is_empty() {
        return false;
    }
    if litery.eq_ignore_ascii_case("HIGHRISK") {
        return false;
    }
    let konczy_dwukropkiem = przed
        .chars()
        .rev()
        .find(|c| c.is_alphanumeric() || *c == ':')
        .is_some_and(|c| c == ':');
    konczy_dwukropkiem || RE_ZEN_ZONA.is_match(przed)
}

#[inline]
fn num(s: &str) -> Option<f64> {
    s.replace(',', ".").parse::<f64>().ok()
}

fn rf_linia_z_intencja(text: &str, (rf_start, rf_end): (usize, usize)) -> bool {
    let od = text[..rf_start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let dokad = text[rf_end..]
        .find('\n')
        .map(|i| rf_end + i)
        .unwrap_or(text.len());
    RE_RF_INTENCJA.is_match(&text[od..dokad])
}

pub fn parse(text: &str) -> Vec<Signal> {
    parse_z_opcjami(text, OpcjeParsera::default())
}

pub fn parse_z_opcjami(text: &str, opcje: OpcjeParsera) -> Vec<Signal> {
    if RE_RETROSPEKCJA.is_match(text) && parse_entry(text).is_none() {
        return vec![Signal::Info];
    }

    if opcje.recap_guard && podsumowanie_wielu_transakcji(text) {
        return parse_entry(text)
            .map(Signal::Entry)
            .map(|s| vec![s])
            .unwrap_or_else(|| vec![Signal::Info]);
    }

    let mut out = Vec::new();

    if RE_SL_HIT.is_match(text) || RE_NOVA_SL_STRATA.is_match(text) || RE_PX_CUT_LOSS.is_match(text)
    {
        out.push(Signal::SlHit);
    }

    if let Some(c) = RE_TP_HIT2.captures(text) {
        for i in [1usize, 2] {
            if let Some(d) = c.get(i).and_then(|m| m.as_str().parse::<usize>().ok()) {
                out.push(Signal::TpHit { index: Some(d) });
            }
        }
    } else if let Some(c) = RE_TP_HIT.captures(text) {
        let d = c
            .get(1)
            .or_else(|| c.get(2))
            .or_else(|| c.get(3))
            .and_then(|m| m.as_str().parse::<usize>().ok());
        out.push(Signal::TpHit { index: d });
    } else if RE_PIPS_HIT.is_match(text) {
        out.push(Signal::TpHit { index: None });
    } else if RE_LEVEL_HIT.is_match(text) {
        out.push(Signal::TpHit { index: None });
    } else if opcje.luz_interpunkcyjny && RE_TP_HIT_APO.is_match(text) {
        out.push(Signal::TpHit { index: None });
    }

    let rf_traf = RE_RF.captures(text).or_else(|| {
        if opcje.luz_interpunkcyjny {
            RE_RF_TYPO.captures(text)
        } else {
            None
        }
    });
    if let Some(c) = rf_traf {
        let level = c.get(1).and_then(|m| num(m.as_str()));
        let intencja = opcje.rf_wymaga_wykonania
            && level.is_none()
            && rf_linia_z_intencja(
                text,
                c.get(0).map(|m| (m.start(), m.end())).unwrap_or((0, 0)),
            );
        if !intencja {
            out.push(Signal::RiskFree { level });
        }
    }

    let spp_luz =
        opcje.luz_interpunkcyjny && RE_SPP_LUZ.is_match(text) && RE_SPP_TARGETS.is_match(text);
    if RE_SPP.is_match(text) || spp_luz {
        out.push(Signal::SecuringPartial {
            spp_be_level: RE_BE_AT.captures(text).and_then(|c| num(&c[1])),
            targets: spp_targets(text),
            sl: RE_SL
                .captures(text)
                .or_else(|| RE_TWP_SL.captures(text))
                .and_then(|c| num(&c[1])),
        });
    }

    if let Some(c) = RE_CORR.captures(text) {
        if let (Some(v), Some(i)) = (num(&c[1]), c[2].parse::<usize>().ok()) {
            out.push(Signal::TpCorrection { index: i, value: v });
        }
    } else if let Some(c) = RE_CORR2.captures(text) {
        if let (Some(i), Some(v)) = (c[1].parse::<usize>().ok(), num(&c[2])) {
            out.push(Signal::TpCorrection { index: i, value: v });
        }
    } else if opcje.luz_interpunkcyjny {
        if let Some(c) = RE_CORR3.captures(text) {
            if let (Some(i), Some(v)) = (c[1].parse::<usize>().ok(), num(&c[2])) {
                out.push(Signal::TpCorrection { index: i, value: v });
            }
        }
    }

    if let Some(c) = RE_SETSL.captures(text) {
        if let Some(v) = num(&c[1]) {
            out.push(Signal::SetSl { value: v });
        }
    } else if let Some(c) = RE_NOVA_SL_AT.captures(text) {
        if let Some(v) = num(&c[1]) {
            out.push(Signal::SetSl { value: v });
        }
    }

    if RE_OAE.is_match(text) || (opcje.luz_interpunkcyjny && RE_OAE_LUZ.is_match(text)) {
        out.push(Signal::OutAtEntry);
    }
    if RE_CLOSE.is_match(text) && !RE_CLOSE_LAYERS.is_match(text) {
        out.push(Signal::CloseAll);
    }
    if opcje.partials_jako_komenda {
        if let Some(l) = close_layers(text) {
            if !l.optional {
                out.push(Signal::TakePartials);
            }
        }
    }
    if RE_CANCEL.is_match(text)
        || is_invalidated(text)
        || (opcje.luz_interpunkcyjny && RE_NLV_PODMIOT.is_match(text))
    {
        out.push(Signal::Cancel);
    }

    if out.is_empty()
        && !RE_SL.is_match(text)
        && !RE_PX_TP_REWIZJA.is_match(text)
        && RE_PX_TP_KOTWICA
            .captures_iter(text)
            .any(|c| c.get(1).is_none())
    {
        let mut widziane: Vec<usize> = Vec::new();
        for c in RE_PX_TP_NUMER.captures_iter(text) {
            if c.get(2).is_some() {
                continue;
            }
            if let Ok(i) = c[1].parse::<usize>() {
                if !widziane.contains(&i) {
                    widziane.push(i);
                    out.push(Signal::TpHit { index: Some(i) });
                }
            }
        }
    }

    if RE_PX_BE.is_match(text)
        && !RE_PX_BE_ODMOWA.is_match(text)
        && !out
            .iter()
            .any(|s| matches!(s, Signal::BreakEven | Signal::SetSl { .. }))
    {
        out.push(Signal::BreakEven);
    }

    if RE_STORM_BE.is_match(text)
        && !out
            .iter()
            .any(|s| matches!(s, Signal::BreakEven | Signal::SetSl { .. }))
    {
        out.push(Signal::BreakEven);
    }

    if opcje.luz_interpunkcyjny
        && RE_BE_LUZ.is_match(text)
        && !out
            .iter()
            .any(|s| matches!(s, Signal::BreakEven | Signal::SetSl { .. }))
    {
        out.push(Signal::BreakEven);
    }

    let has_hit = out
        .iter()
        .any(|s| matches!(s, Signal::TpHit { .. } | Signal::SlHit));

    if !has_hit {
        let entry = parse_entry(text).or_else(|| {
            opcje
                .luz_interpunkcyjny
                .then(|| parse_entry_area_przed_at(text))
                .flatten()
        });
        if let Some(e) = entry {
            out.push(Signal::Entry(e));
        }
    }

    if !out.iter().any(|s| matches!(s, Signal::Entry(_)))
        && !RE_TP.is_match(text)
        && !RE_SL.is_match(text)
    {
        if let Some(c) = RE_MARKET_OPEN.captures(text) {
            let side = if c[1].eq_ignore_ascii_case("BUY") {
                Side::Buy
            } else {
                Side::Sell
            };
            out.push(Signal::MarketOpen { side });
        }
    }

    if out.is_empty() {
        if RE_BE.is_match(text) {
            out.push(Signal::BreakEven);
        } else if opcje.geometryczny {
            if let Some(g) = odczyt_geometryczny(text) {
                if g.pewnosc >= opcje.min_pewnosc {
                    out.push(Signal::Entry(g.sygnal));
                }
            }
            if out.is_empty() {
                out.push(Signal::Info);
            }
        } else {
            out.push(Signal::Info);
        }
    }

    out
}

pub fn wyglada_na_wejscie(text: &str) -> bool {
    RE_SL.is_match(text) && (RE_TP.is_match(text) || RE_TP_LIST.is_match(text))
}

pub fn basket_hints(text: &str) -> Vec<Px> {
    let mut v = Vec::new();
    let mut push = |x: Option<Px>| {
        if let Some(x) = x {
            if !v.iter().any(|y: &Px| (*y - x).abs() < 1e-9) {
                v.push(x);
            }
        }
    };
    if let Some(c) = RE_RANGE_HINT.captures(text) {
        push(num(&c[1]));
        push(num(&c[2]));
    }
    if let Some(c) = RE_RF.captures(text) {
        push(c.get(1).and_then(|m| num(m.as_str())));
    }
    if let Some(c) = RE_SETSL.captures(text) {
        push(num(&c[1]));
    }
    push(hit_level(text));
    if let Some(c) = RE_BE_AT.captures(text) {
        push(num(&c[1]));
    }
    v
}

fn extract_tps(text: &str) -> Vec<Px> {
    let mut v: Vec<Px> = Vec::new();
    for c in RE_TWP_TP.captures_iter(text) {
        if let Some(x) = num(&c[1]) {
            if !v.contains(&x) {
                v.push(x);
            }
        }
    }
    for c in RE_TP_LIST.captures_iter(text) {
        for m in RE_NUMS.find_iter(&c[1]) {
            if let Some(x) = num(m.as_str()) {
                if !v.contains(&x) {
                    v.push(x);
                }
            }
        }
    }
    v
}

fn spp_targets(text: &str) -> Vec<Px> {
    let Some(c) = RE_SPP_TARGETS.captures(text) else {
        return Vec::new();
    };
    let nums: Vec<Px> = RE_NUMS
        .find_iter(&c[1])
        .filter_map(|m| num(m.as_str()))
        .collect();
    if nums.len() < 2 {
        return nums;
    }
    let limit = ((nums[1] - nums[0]).abs() * 3.0).max(30.0);
    let mut out = vec![nums[0]];
    for &x in &nums[1..] {
        let prev = *out.last().unwrap();
        if (x - prev).abs() > limit {
            break;
        }
        if !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

fn hr_linia_loty(text: &str) -> bool {
    RE_HR_LINIA_LOTY.captures_iter(text).any(|c| {
        let ogon = c[1].trim_start();
        !ogon
            .get(..4)
            .is_some_and(|p| p.eq_ignore_ascii_case("TRAD"))
    })
}

fn is_invalidated(text: &str) -> bool {
    RE_INVALID_ONLY.is_match(text)
        || RE_INVALID_LINIA.is_match(text)
        || (RE_NO_LONGER_VALID.is_match(text) && RE_WAIT_NEXT.is_match(text))
        || RE_ZONE_FAILED.is_match(text)
}

pub fn hit_level(text: &str) -> Option<Px> {
    RE_LEVEL_HIT
        .captures(text)
        .and_then(|c| num(&c[1]))
        .filter(|&v| poziom_sensowny(v))
}

#[inline]
fn poziom_sensowny(v: Px) -> bool {
    (1000.0..10_000.0).contains(&v)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerClose {
    pub count: Option<u32>,
    pub level: Option<Px>,
    pub optional: bool,
}

pub fn close_layers(text: &str) -> Option<LayerClose> {
    if !RE_CLOSE_LAYERS.is_match(text) {
        return None;
    }
    let level = RE_LAYER_LEVEL.captures(text).and_then(|c| num(&c[1]));
    let count = if level.is_some() {
        None
    } else {
        RE_LAYER_COUNT
            .captures(text)
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().parse::<u32>().ok())
    };
    Some(LayerClose {
        count,
        level,
        optional: RE_LAYER_OPTIONAL.is_match(text),
    })
}

fn parse_entry_area_przed_at(text: &str) -> Option<EntrySignal> {
    let poprawiony = RE_ENTRY_AREA_PRZED_AT.replace(text, "$1 $2");
    if poprawiony.as_ref() == text {
        return None;
    }
    parse_entry(poprawiony.as_ref())
}

fn parse_entry(text: &str) -> Option<EntrySignal> {
    if zapowiedz_strefy(text) && !RE_TWP_WEJSCIE.is_match(text) {
        return None;
    }
    let rodzaj = |c: &regex::Captures, i: usize, j: usize| {
        c.get(i)
            .or_else(|| c.get(j))
            .map(|m| m.as_str().to_uppercase())
            .unwrap_or_default()
    };
    let (side, kind, a, b) = if let Some(c) = RE_TWP_WEJSCIE.captures(text) {
        let s = if c[1].eq_ignore_ascii_case("BUY") {
            Side::Buy
        } else {
            Side::Sell
        };
        let p = num(&c[2])?;
        (s, String::new(), p, p)
    } else if let Some((s, r, lo, hi)) = px_wiele_zlecen(text) {
        (s, r, lo, hi)
    } else if let Some(c) = RE_ENTRY_ZONE.captures(text) {
        let s = if c[1].eq_ignore_ascii_case("BUY") {
            Side::Buy
        } else {
            Side::Sell
        };
        (s, rodzaj(&c, 2, 3), num(&c[4])?, num(&c[5])?)
    } else if let Some(c) = RE_ENTRY_SINGLE.captures(text) {
        let s = if c[1].eq_ignore_ascii_case("BUY") {
            Side::Buy
        } else {
            Side::Sell
        };
        let p = num(&c[4])?;
        (s, rodzaj(&c, 2, 3), p, p)
    } else if let Some(c) = RE_ENTRY_ORDER.captures(text) {
        let s = if c[1].eq_ignore_ascii_case("BUY") {
            Side::Buy
        } else {
            Side::Sell
        };
        let kind = c
            .get(2)
            .map(|m| m.as_str().to_uppercase())
            .unwrap_or_default();
        let p = num(&c[3])?;
        (s, kind, p, p)
    } else if let Some(k) = RE_DANGER_KIER.captures(text) {
        let s = if k[1].eq_ignore_ascii_case("BUY") {
            Side::Buy
        } else {
            Side::Sell
        };
        let z = RE_DANGER_STREFA.captures(text)?;
        let (a, b) = (num(&z[1])?, num(&z[2])?);
        (s, String::new(), a.min(b), a.max(b))
    } else if let Some(c) = RE_ZEN_WEJSCIE_1.captures(text) {
        let s = if c[1].eq_ignore_ascii_case("BUY") {
            Side::Buy
        } else {
            Side::Sell
        };
        let p = num(&c[2])?;
        (s, String::new(), p, p)
    } else if let Some(c) = RE_STORM_WEJSCIE.captures(text) {
        let s = if c[1].eq_ignore_ascii_case("BUY") {
            Side::Buy
        } else {
            Side::Sell
        };
        let (a, b) = (num(&c[2])?, num(&c[3])?);
        (s, "LIMITS".to_string(), a.min(b), a.max(b))
    } else {
        return None;
    };

    let (a, b) = if (a - b).abs() > 100.0 {
        let mut inne: Vec<Px> = extract_tps(text);
        if let Some(s) = RE_SL
            .captures(text)
            .or_else(|| RE_TWP_SL.captures(text))
            .and_then(|c| num(&c[1]))
        {
            inne.push(s);
        }
        if inne.is_empty() {
            (a, b)
        } else {
            inne.sort_by(|x, y| x.partial_cmp(y).unwrap());
            let sr = inne[inne.len() / 2];
            let zdrowa = if (a - sr).abs() <= (b - sr).abs() {
                a
            } else {
                b
            };
            (zdrowa, zdrowa)
        }
    } else {
        (a, b)
    };

    let lo = a.min(b);
    let hi = a.max(b);
    let mid = (lo + hi) * 0.5;

    let mut tps: Vec<Px> = extract_tps(text)
        .into_iter()
        .filter(|&t| match side {
            Side::Buy => t > mid,
            Side::Sell => t < mid,
        })
        .collect();
    tps.sort_by(|x, y| match side {
        Side::Buy => x.partial_cmp(y).unwrap(),
        Side::Sell => y.partial_cmp(x).unwrap(),
    });
    tps.dedup();

    let sl = RE_SL
        .captures(text)
        .or_else(|| RE_TWP_SL.captures(text))
        .and_then(|c| num(&c[1]))
        .map(|s| {
            let ok_side = match side {
                Side::Buy => s <= lo,
                Side::Sell => s >= hi,
            };
            if ok_side && (mid - s).abs() < 150.0 {
                s
            } else {
                match side {
                    Side::Buy => lo - 1.0,
                    Side::Sell => hi + 1.0,
                }
            }
        });

    if tps.is_empty() {
        return None;
    }

    Some(EntrySignal {
        side,
        is_limit: kind.starts_with("LIMIT"),
        is_stop: kind.starts_with("STOP"),
        lo,
        hi,
        sl,
        tps,
        tp_open: RE_TP_OPEN.is_match(text) || RE_PX_TP_OPEN.is_match(text),
        warstwy_offset: warstwy_z_tekstu(text),
        tag_high_risk: RE_HR.is_match(text) || RE_HR_LINIA.is_match(text) || hr_linia_loty(text),
        tag_may_not_be_around: RE_MNBA.is_match(text),
        tag_first_entry: RE_FE.is_match(text),
    })
}


#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpcjeParsera {
    pub geometryczny: bool,
    pub min_pewnosc: f64,
    pub rf_wymaga_wykonania: bool,
    pub partials_jako_komenda: bool,
    pub luz_interpunkcyjny: bool,
    pub recap_guard: bool,
}

pub fn unindexed_pips_hit(text: &str) -> bool {
    RE_PIPS_HIT.is_match(text)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfitUpdateKind {
    ConfirmedIndexedTp,
    ConfirmedPriceLevel,
    AtTpProximity,
    UnindexedPips,
    RunningPips,
    Other,
}

pub fn profit_update_kind(text: &str) -> ProfitUpdateKind {
    if RE_TP_HIT2.is_match(text) || RE_TP_HIT_CONFIRMED_INDEXED.is_match(text) {
        ProfitUpdateKind::ConfirmedIndexedTp
    } else if RE_LEVEL_HIT.is_match(text) {
        ProfitUpdateKind::ConfirmedPriceLevel
    } else if RE_AT_TP_PROXIMITY.is_match(text) {
        ProfitUpdateKind::AtTpProximity
    } else if RE_PIPS_HIT.is_match(text) {
        ProfitUpdateKind::UnindexedPips
    } else if RE_PIPS_RUNNING.is_match(text) {
        ProfitUpdateKind::RunningPips
    } else {
        ProfitUpdateKind::Other
    }
}

pub fn at_tp_is_telemetry(text: &str) -> bool {
    profit_update_kind(text) == ProfitUpdateKind::AtTpProximity
}

pub fn suppress_at_tp_hits(signals: &mut Vec<Signal>, text: &str) -> usize {
    if !at_tp_is_telemetry(text) {
        return 0;
    }
    let before = signals.len();
    signals.retain(|signal| !matches!(signal, Signal::TpHit { .. }));
    before - signals.len()
}

impl Default for OpcjeParsera {
    fn default() -> Self {
        Self {
            geometryczny: false,
            min_pewnosc: 0.0,
            rf_wymaga_wykonania: false,
            partials_jako_komenda: false,
            luz_interpunkcyjny: false,
            recap_guard: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OdczytGeo {
    pub sygnal: EntrySignal,
    pub pewnosc: f64,
    pub niezmienniki: Vec<String>,
}

re!(RE_GEO_BUY, r"(?i)\bBUY\b");
re!(RE_GEO_SELL, r"(?i)\bSELL\b");
re!(
    RE_GEO_RAPORT,
    concat!(
        r"(?i)\b(?:HIT|CLOSED|CLOSING|PROFITS?|PIPS?|SECURED|SECURING|RUNNING|",
        r"RECAP|RESULTS?|CANCELL?ED|INVALID|ENTRIES)\b",
        r"|\bBREAK\s*EVEN\b|\bRISK\s*FREE\b|\bIN\s+(?:GREEN|RED)\b"
    )
);
re!(
    RE_GEO_LIMIT,
    r"(?i)\b(?:BUY|SELL)\s+(?:GOLD\s+|XAUUSD\s+|XAU\s+)?LIMITS?\b"
);
re!(
    RE_GEO_STOP,
    r"(?i)\b(?:BUY|SELL)\s+(?:GOLD\s+|XAUUSD\s+|XAU\s+)?STOPS?\b"
);

fn geo_poziomy_linii(text: &str) -> Vec<Px> {
    let Some(l) = text
        .lines()
        .find(|l| RE_GEO_BUY.is_match(l) || RE_GEO_SELL.is_match(l))
    else {
        return Vec::new();
    };
    let mut v: Vec<Px> = RE_NUMS
        .find_iter(l)
        .filter_map(|m| num(m.as_str()))
        .filter(|x| poziom_sensowny(*x))
        .collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    v
}

pub fn odczyt_geometryczny(text: &str) -> Option<OdczytGeo> {
    if text.len() > 600 {
        return None;
    }
    if zapowiedz_strefy(text) || RE_RETROSPEKCJA.is_match(text) {
        return None;
    }
    if RE_GEO_RAPORT.is_match(text) {
        return None;
    }

    let buy = RE_GEO_BUY.is_match(text);
    let sell = RE_GEO_SELL.is_match(text);
    let side = match (buy, sell) {
        (true, false) => Side::Buy,
        (false, true) => Side::Sell,
        _ => return None,
    };

    let mut poziomy: Vec<Px> = RE_NUMS
        .find_iter(text)
        .filter_map(|m| num(m.as_str()))
        .filter(|x| poziom_sensowny(*x))
        .collect();
    poziomy.sort_by(|a, b| a.partial_cmp(b).unwrap());
    poziomy.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    if poziomy.len() < 3 || poziomy.len() > 8 {
        return None;
    }
    let sr = poziomy[poziomy.len() / 2];
    let przed = poziomy.len();
    poziomy.retain(|x| (x - sr).abs() <= sr * 0.03);
    if poziomy.len() < 3 || przed - poziomy.len() > 2 {
        return None;
    }

    let k = if side == Side::Buy { 1.0 } else { -1.0 };
    let mut u = poziomy.clone();
    u.sort_by(|a, b| (k * a).partial_cmp(&(k * b)).unwrap());
    let sl = u[0];

    let w_linii = geo_poziomy_linii(text);
    let m = if !w_linii.is_empty() && w_linii.len() <= 3 && u.len() >= w_linii.len() + 2 {
        let mut blok = u[1..1 + w_linii.len()].to_vec();
        blok.sort_by(|a, c| a.partial_cmp(c).unwrap());
        if blok
            .iter()
            .zip(w_linii.iter())
            .all(|(a, b)| (a - b).abs() < 1e-9)
        {
            w_linii.len()
        } else {
            1
        }
    } else {
        1
    };
    let zgodna_linia = !w_linii.is_empty() && m == w_linii.len();

    if u.len() < m + 2 {
        return None;
    }
    let wejscia = &u[1..1 + m];
    let cele: Vec<Px> = u[1 + m..].to_vec();

    let lo = wejscia.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = wejscia.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let mid = (lo + hi) * 0.5;
    if hi - lo > 100.0 {
        return None;
    }
    let d_sl = (mid - sl).abs();
    if d_sl > 150.0 {
        return None;
    }

    let mut pewnosc = 0.0f64;
    let mut niezm: Vec<String> = Vec::new();
    let mut dodaj = |w: f64, n: &str, p: &mut f64, v: &mut Vec<String>| {
        *p += w;
        v.push(n.to_string());
    };

    dodaj(0.20, "kierunek-slowo", &mut pewnosc, &mut niezm);
    if !w_linii.is_empty() {
        dodaj(0.15, "kierunek-przy-cenie", &mut pewnosc, &mut niezm);
    }
    if cele.len() >= 2 {
        dodaj(0.20, "trzy-grupy", &mut pewnosc, &mut niezm);
    } else {
        dodaj(0.10, "jeden-cel", &mut pewnosc, &mut niezm);
    }
    if zgodna_linia {
        dodaj(0.20, "nierownosc", &mut pewnosc, &mut niezm);
    }
    if (0.5..=60.0).contains(&d_sl) {
        dodaj(0.125, "odleglosc-sl", &mut pewnosc, &mut niezm);
    }
    let d_tp1 = (cele[0] - mid).abs();
    let d_tpn = (cele[cele.len() - 1] - mid).abs();
    if d_tp1 >= 0.3 && d_tpn <= 200.0 && d_tpn >= d_tp1 {
        dodaj(0.125, "odleglosc-tp", &mut pewnosc, &mut niezm);
    }

    Some(OdczytGeo {
        sygnal: EntrySignal {
            side,
            is_limit: RE_GEO_LIMIT.is_match(text),
            is_stop: RE_GEO_STOP.is_match(text),
            lo,
            hi,
            sl: Some(sl),
            tps: cele,
            tp_open: RE_TP_OPEN.is_match(text) || RE_PX_TP_OPEN.is_match(text),
            warstwy_offset: warstwy_z_tekstu(text),
            tag_high_risk: RE_HR.is_match(text)
                || RE_HR_LINIA.is_match(text)
                || hr_linia_loty(text),
            tag_may_not_be_around: RE_MNBA.is_match(text),
            tag_first_entry: RE_FE.is_match(text),
        },
        pewnosc: (pewnosc * 1000.0).round() / 1000.0,
        niezmienniki: niezm,
    })
}

#[cfg(test)]
mod public_synthetic_tests {
    use super::*;

    #[test]
    fn parses_synthetic_limit_entry() {
        let text = "BUY LIMITS GOLD @ 2100/2095 AREA\nTP1 2103\nTP2 2107\nTP OPEN\nSL 2090\nHIGH RISK TRADE";
        let parsed = parse(text);
        let entry = match &parsed[0] {
            Signal::Entry(entry) => entry,
            other => panic!("expected entry, got {other:?}"),
        };
        assert_eq!(entry.side, Side::Buy);
        assert!(entry.is_limit);
        assert_eq!((entry.lo, entry.hi), (2095.0, 2100.0));
        assert_eq!(entry.sl, Some(2090.0));
        assert_eq!(entry.tps, vec![2103.0, 2107.0]);
        assert!(entry.tp_open);
        assert!(entry.tag_high_risk);
    }

    #[test]
    fn parses_synthetic_management_actions() {
        assert_eq!(parse("TP1 HIT")[0].action_key(), "tp1");
        assert!(parse("SL HIT").iter().any(|s| matches!(s, Signal::SlHit)));
        assert!(parse("INVALID").iter().any(|s| matches!(s, Signal::Cancel)));
        assert!(matches!(
            parse("RISK FREE AT 2112")[0],
            Signal::RiskFree { level: Some(v) } if v == 2112.0
        ));
    }

    #[test]
    fn value_aware_action_keys_are_deterministic() {
        assert_eq!(
            Signal::SetSl { value: 2088.0 }.action_key_v2(),
            "setsl@2088"
        );
        assert_eq!(
            Signal::TpCorrection { index: 2, value: 2120.0 }.action_key_v2(),
            "corr2@2120"
        );
    }
}
