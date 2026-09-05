
use anyhow::Result;
use conduit_backtest::data::{load_messages, ReplayMessage};
use conduit_core::engine::IncomingMessage;
use conduit_core::parser::{self, EntrySignal, Signal};
use conduit_core::telegram_ingress::{opens_basket, stale_entry_age_minutes, ContentMemory};
use conduit_core::types::SourceKey;
use std::io::Write;

/// Wersja rekordu AKCJI w pliku mostu. Otoczka `M|...|akcja` zostaje ta
/// sama, dzięki czemu jeden ekspert może czytać stare i nowe pliki.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WireSchema {
    Legacy,
    V2,
}

impl WireSchema {
    fn parse(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "legacy" | "1" => Ok(Self::Legacy),
            "v2" | "2" => Ok(Self::V2),
            _ => anyhow::bail!("--schema musi byc legacy albo v2 (jest {s:?})"),
        }
    }

    fn number(self) -> u8 {
        match self {
            Self::Legacy => 1,
            Self::V2 => 2,
        }
    }
}

/// Klucz v2 potrafi zawierać `|` (SPP: `...|sl:...|be:...`), czyli separator
/// REKORDU mostu. Kodujemy całe UTF-8 szesnastkowo. Ekspert nie musi klucza
/// dekodować: dedup wymaga wyłącznie stabilnej równości.
fn wire_action_key(s: &Signal, value_aware: bool) -> String {
    let raw = if value_aware {
        s.action_key_v2()
    } else {
        s.action_key()
    };
    if !value_aware {
        return raw;
    }
    let mut out = String::with_capacity(4 + raw.len() * 2);
    out.push_str("v2h_");
    for b in raw.as_bytes() {
        use std::fmt::Write as _;
        write!(&mut out, "{b:02x}").expect("String nie zwraca bledu zapisu");
    }
    out
}

fn serialize_entry(e: &EntrySignal, key: &str, schema: WireSchema) -> String {
    let side = if e.side == conduit_core::types::Side::Buy {
        "BUY"
    } else {
        "SELL"
    };
    let tps: Vec<String> = e.tps.iter().map(|t| format!("{t:.5}")).collect();
    match schema {
        WireSchema::Legacy => format!(
            "ENTRY:{key},{side},{},{:.5},{:.5},{},{},{}",
            if e.is_limit { 1 } else { 0 },
            e.lo,
            e.hi,
            n(e.sl),
            if e.tp_open { 1 } else { 0 },
            tps.join(",")
        ),
        // Stały prefiks + jawne `ntp`: nowe metadane nigdy nie zostaną
        // pomylone z końcową, zmiennej długości listą celów.
        WireSchema::V2 => format!(
            "ENTRY2:{key},{side},{},{},{:.5},{:.5},{},{},{},{},{},{},{},{}",
            if e.is_limit { 1 } else { 0 },
            if e.is_stop { 1 } else { 0 },
            e.lo,
            e.hi,
            n(e.sl),
            if e.tp_open { 1 } else { 0 },
            n(e.warstwy_offset),
            if e.tag_high_risk { 1 } else { 0 },
            if e.tag_may_not_be_around { 1 } else { 0 },
            if e.tag_first_entry { 1 } else { 0 },
            e.tps.len(),
            tps.join(",")
        ),
    }
}

fn serialize_tp_hit(index: Option<usize>, key: &str, raw_text: &str, schema: WireSchema) -> String {
    let idx = index.map(|x| x as i64).unwrap_or(-1);
    match schema {
        WireSchema::Legacy => format!("TPHIT:{key},{idx}"),
        WireSchema::V2 => format!(
            "TPHIT2:{key},{idx},{},{}",
            n(parser::hit_level(raw_text)),
            if parser::unindexed_pips_hit(raw_text) {
                1
            } else {
                0
            }
        ),
    }
}

fn parse_date(s: &str) -> Result<i64> {
    let p: Vec<&str> = s.split('-').collect();
    anyhow::ensure!(p.len() == 3, "data w formacie RRRR-MM-DD");
    let (y, m, d): (i64, i64, i64) = (p[0].parse()?, p[1].parse()?, p[2].parse()?);
    // dni od epoki — algorytm Howarda Hinnanta (days_from_civil)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Ok((era * 146_097 + doe - 719_468) * 86_400_000)
}

/// Liczba w formacie, który MQL5 czyta bez niespodzianek.
/// `nan` znaczy „brak wartości" — MQL5 `StringToDouble("nan")` daje 0,
/// więc ekspert sprawdza NAPIS, a nie liczbę.
fn n(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{x:.5}"),
        None => "nan".into(),
    }
}

/// Exactly the runner's pre-engine LIVE gate, before Info records are removed.
/// The memory key namespace is arbitrary for this single-source bridge; all
/// messages use the same namespace, as the single-preset replay runner does.
fn pass_live_ingress(m: &ReplayMessage, memory: &mut ContentMemory) -> bool {
    if m.kanal == "__CONDUIT_CONTROL__"
        && m.text == "__CONDUIT_LIVEBACKTEST_INGRESS_RESTART__"
    {
        *memory = ContentMemory::new();
        return false;
    }
    let incoming = IncomingMessage {
        ts: m.ts,
        source: SourceKey::new(0, None),
        source_name: m.kanal.clone(),
        msg_id: m.msg_id,
        reply_to: m.reply_to,
        edit_of: m.edit_of,
        text: m.text.clone(),
    };
    if memory.duplikat_tresci(&incoming) {
        return false;
    }
    !m.telegram_published_ts.is_some_and(|published| {
        stale_entry_age_minutes(m.ts, published, 5.0).is_some()
            && opens_basket(&m.text, m.edit_of)
    })
}

fn main() -> Result<()> {
    let mut signals = String::from("data/signals.json");
    let mut out = String::from("most_ea.csv");
    let mut from = i64::MIN;
    let mut to = i64::MAX;
    let mut offset_ms: i64 = 3 * 3_600_000;
    let mut tylko_kanal = String::new();
    // Domyślnie legacy, żeby starszy CONDUIT_X/LX nie dostał nieznanego
    // rekordu. Harness XT żąda v2 jawnie.
    let mut wire_schema = WireSchema::Legacy;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut preset = String::new();
    let mut live_telegram_ingress = false;
    let mut trade_sessions = None;
    let mut exec_latency_ms = conduit_core::settings::Settings::default().exec_latency_ms;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let mut next = || {
            i += 1;
            args.get(i).cloned().unwrap_or_default()
        };
        match a {
            "--signals" => signals = next(),
            "--out" => out = next(),
            "--from" => from = parse_date(&next())?,
            "--to" => to = parse_date(&next())?,
            "--offset-ms" => offset_ms = next().parse()?,
            "--kanal" => tylko_kanal = next(),
            "--preset" => preset = next(),
            "--live-telegram-ingress" => live_telegram_ingress = true,
            "--sim-trade-sessions" => {
                trade_sessions = Some(conduit_backtest::trade_sessions::TradeSessionProfile::load(std::path::Path::new(&next()))?);
            }
            "--schema" => wire_schema = WireSchema::parse(&next())?,
            _ => anyhow::bail!("nieznany argument {a}"),
        }
        i += 1;
    }

    let mut dedup_value_aware = false;
    let mut profit_update_telemetry_only = false;
    let mut source_reply_links = false;
    let opcje = if preset.is_empty() {
        parser::OpcjeParsera::default()
    } else {
        let p: conduit_core::settings::Preset =
            serde_json::from_str(&std::fs::read_to_string(&preset)?)?;
        let c = &p.settings;
        exec_latency_ms = c.exec_latency_ms;
        dedup_value_aware = c.dedup_klucz_z_wartoscia;
        profit_update_telemetry_only = c.profit_update_telemetry_only;
        source_reply_links = !c.edycja_sieroty_nie_otwiera && c.reply_graph_transitive;
        eprintln!(
            "    opcje parsera z presetu {}: rf_wymaga_wykonania={} partials_wykonuj={} geometryczny={} min_pewnosc={} luz_interpunkcyjny={} dedup_value={} profit_telemetry_only={}",
            p.name, c.rf_wymaga_wykonania, c.partials_wykonuj,
            c.parser_geometryczny, c.parser_min_pewnosc, c.parser_luz_interpunkcyjny,
            dedup_value_aware, profit_update_telemetry_only,
        );
        parser::OpcjeParsera {
            geometryczny: c.parser_geometryczny,
            min_pewnosc: c.parser_min_pewnosc,
            rf_wymaga_wykonania: c.rf_wymaga_wykonania,
            partials_jako_komenda: c.partials_wykonuj,
            luz_interpunkcyjny: c.parser_luz_interpunkcyjny,
            recap_guard: c.recap_guard,
        }
    };

    let msgs = load_messages(&signals)?;
    let mut f = std::io::BufWriter::new(std::fs::File::create(&out)?);

    writeln!(
        f,
        "# CONDUIT most do eksperta — polecenia rozwiazane parserem silnika"
    )?;
    writeln!(
        f,
        "# CONTRACT schema={} dedup_value={} live_telegram_ingress={}",
        wire_schema.number(),
        if dedup_value_aware { 1 } else { 0 },
        if live_telegram_ingress { 1 } else { 0 }
    )?;
    writeln!(f, "# zrodlo={signals}")?;
    if let Some(profile) = &trade_sessions {
        use sha2::{Digest, Sha256};
        let encoded = serde_json::to_vec(profile)?;
        writeln!(f, "# BROKER_EXECUTION_PROFILE sha256={:x} clock=broker scope=native_execution_only_messages_unfiltered", Sha256::digest(encoded))?;
    }
    writeln!(
        f,
        "# offset_ms={offset_ms} (zegar tickow = ts_wiadomosci + offset)"
    )?;
    writeln!(
        f,
        "# M|ts_ms|msg_id|reply_to|edit_of|hints(,)|kanal|akcja|akcja|..."
    )?;
    writeln!(
        f,
        "# ENTRY:klucz,side,is_limit,lo,hi,sl,tp_open,tp1,tp2,..."
    )?;
    writeln!(f, "# ENTRY2:klucz,side,is_limit,is_stop,lo,hi,sl,tp_open,warstwy,tag_hr,tag_away,tag_first,ntp,tp1,...")?;
    writeln!(
        f,
        "# TPHIT:klucz,index   TPHIT2:klucz,index,hit_level,unindexed_pips"
    )?;
    writeln!(f, "# SLHIT:klucz   RF:klucz,level   OAE:klucz")?;
    writeln!(f, "# CANCEL:klucz  CLOSEALL:klucz  BE:klucz  SETSL:klucz,v")?;
    writeln!(
        f,
        "# SPP:klucz,sl,be,t1,t2,...   TPCORR:klucz,index,v   MKT:klucz,side"
    )?;

    let (mut n_msg, mut n_act, mut n_entry) = (0u64, 0u64, 0u64);
    let mut per_kanal: std::collections::BTreeMap<String, u64> = Default::default();
    let mut n_odsianych = 0u64;
    let mut ingress = ContentMemory::new();
    let mut ingress_dropped = 0u64;

    for m in &msgs {
        let ts = m.ts + offset_ms;
        // XT adds execution latency when dispatching this row. Include the
        // same effective window as btp, including arrivals around midnight.
        let effective_ts = ts.saturating_add(exec_latency_ms);
        if effective_ts < from || effective_ts >= to {
            continue;
        }
        if live_telegram_ingress && !pass_live_ingress(m, &mut ingress) {
            ingress_dropped += 1;
            continue;
        }
        if !tylko_kanal.is_empty() && m.kanal != tylko_kanal {
            n_odsianych += 1;
            continue;
        }
        let mut sigs = parser::parse_z_opcjami(&m.text, opcje.clone());
        if profit_update_telemetry_only {
            // Ten sam klasyfikator i ten sam filtr co Engine live: `AT TP`
            // znika jako TPHIT, ale SPP/RF/CANCEL/BE/SL z tej samej
            // wiadomosci zostaja w rekordzie mostu. Numerowane TPn HIT i
            // price HIT pozostaja; ekspert weryfikuje je przez In_TpSource=3.
            parser::suppress_at_tp_hits(&mut sigs, &m.text);
        }
        // Wiadomość bez ANI JEDNEGO polecenia nie zmienia stanu silnika —
        // w pliku byłaby tylko szumem, a ekspert i tak by ją pominął.
        let source_link = source_reply_links && m.reply_to.is_some();
        if sigs.iter().all(|s| matches!(s, Signal::Info)) && !source_link {
            continue;
        }

        let hints: Vec<String> = parser::basket_hints(&m.text)
            .iter()
            .map(|x| format!("{x:.5}"))
            .collect();

        let mut pola: Vec<String> = vec![
            "M".into(),
            ts.to_string(),
            m.msg_id.to_string(),
            m.reply_to.unwrap_or(0).to_string(),
            m.edit_of.unwrap_or(0).to_string(),
            hints.join(","),
            // ⚠ NAZWA KANAŁU NIE MOŻE ZAWIERAĆ '|' ANI ':' — pierwszy rozbija
            // rekord, drugi psuje rozpoznanie wersji pliku po stronie eksperta.
            m.kanal.replace(['|', ':'], "_"),
        ];

        for s in &sigs {
            let key = wire_action_key(s, dedup_value_aware);
            let a = match s {
                Signal::Info => continue,
                Signal::Entry(e) => {
                    n_entry += 1;
                    serialize_entry(e, &key, wire_schema)
                }
                Signal::TpHit { index } => serialize_tp_hit(*index, &key, &m.text, wire_schema),
                Signal::SlHit => format!("SLHIT:{key}"),
                Signal::RiskFree { level } => format!("RF:{key},{}", n(*level)),
                Signal::OutAtEntry => format!("OAE:{key}"),
                Signal::Cancel => format!("CANCEL:{key}"),
                Signal::CloseAll => format!("CLOSEALL:{key}"),
                Signal::TakePartials => format!("PARTIALS:{key}"),
                Signal::BreakEven => format!("BE:{key}"),
                Signal::SetSl { value } => format!("SETSL:{key},{value:.5}"),
                Signal::TpCorrection { index, value } => {
                    format!("TPCORR:{key},{index},{value:.5}")
                }
                Signal::MarketOpen { side } => format!(
                    "MKT:{key},{}",
                    if *side == conduit_core::types::Side::Buy {
                        "BUY"
                    } else {
                        "SELL"
                    }
                ),
                Signal::SecuringPartial {
                    targets,
                    sl,
                    spp_be_level,
                } => {
                    let t: Vec<String> = targets.iter().map(|x| format!("{x:.5}")).collect();
                    format!("SPP:{key},{},{},{}", n(*sl), n(*spp_be_level), t.join(","))
                }
            };
            pola.push(a);
            n_act += 1;
        }
        // 7 pól stałych (M, ts, msg_id, reply_to, edit_of, hints, kanal)
        if pola.len() == 7 && source_link {
            pola.push("INFO:source_reply_link".into());
        }
        if pola.len() <= 7 {
            continue;
        }
        writeln!(f, "{}", pola.join("|"))?;
        *per_kanal
            .entry(if m.kanal.is_empty() {
                "(brak)".into()
            } else {
                m.kanal.clone()
            })
            .or_default() += 1;
        n_msg += 1;
    }
    f.flush()?;

    eprintln!("wiadomosci ze zdarzeniem : {n_msg}");
    eprintln!("polecen razem            : {n_act}  (w tym wejsc: {n_entry})");
    if live_telegram_ingress {
        eprintln!("live ingress pominieto    : {ingress_dropped}");
    }
    let rozbicie: Vec<String> = per_kanal.iter().map(|(k, v)| format!("{k}={v}")).collect();
    eprintln!("kanaly                   : {}", rozbicie.join("  "));
    if !tylko_kanal.is_empty() {
        eprintln!("odsiane spoza --kanal {tylko_kanal} : {n_odsianych}");
    }
    eprintln!("zapisano                 : {out}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_bridge_filters_before_parser_and_preserves_orphan_new() {
        let mut memory = ContentMemory::new();
        let mut m = ReplayMessage {
            ts: 10_000, msg_id: 42, edit_of: Some(42),
            text: "TP1 HIT".into(), kanal: "Synergy".into(),
            ..Default::default()
        };
        assert!(pass_live_ingress(&m, &mut memory));
        assert!(!pass_live_ingress(&m, &mut memory));
        m.edit_of = None;
        assert!(pass_live_ingress(&m, &mut memory));
        let restart = ReplayMessage {
            kanal: "__CONDUIT_CONTROL__".into(),
            text: "__CONDUIT_LIVEBACKTEST_INGRESS_RESTART__".into(),
            ..Default::default()
        };
        assert!(!pass_live_ingress(&restart, &mut memory));
        assert!(pass_live_ingress(&m, &mut memory));
    }

    #[test]
    fn live_bridge_rejects_stale_market_open_but_keeps_management_edit() {
        let mut memory = ContentMemory::new();
        let mut m = ReplayMessage {
            ts: 600_001, telegram_published_ts: Some(1), msg_id: 101,
            text: "GOLD BUY NOW".into(), kanal: "Synergy".into(),
            ..Default::default()
        };
        assert!(opens_basket(&m.text, None));
        assert!(!pass_live_ingress(&m, &mut memory));
        m.edit_of = Some(101);
        m.text = "MOVE SL TO 2088".into();
        assert!(pass_live_ingress(&m, &mut memory));
    }

    fn parsed_entry(text: &str) -> EntrySignal {
        parser::parse(text)
            .into_iter()
            .find_map(|s| match s {
                Signal::Entry(e) => Some(e),
                _ => None,
            })
            .expect("testowy tekst ma byc sygnalem ENTRY")
    }

    #[test]
    fn legacy_entry_zostaje_starym_formatem() {
        let e = parsed_entry("BUY GOLD 4500-4502\nSL 4498\nTP 4505 4510 4515");
        let wire = serialize_entry(&e, "entry", WireSchema::Legacy);
        assert!(wire.starts_with("ENTRY:entry,BUY,"));
        assert!(!wire.starts_with("ENTRY2:"));
        assert_eq!(wire.split(',').count(), 10);
    }

    #[test]
    fn entry2_ma_staly_prefiks_i_jawna_liczbe_celow() {
        let e = parsed_entry(
            "BUY LIMITS GOLD 4000-4002\nSL 3998\nTP 4005 4010 4015\n\
             ADDING 3 PIPS TO EACH LIMIT ORDER",
        );
        let wire = serialize_entry(&e, "entry", WireSchema::V2);
        let f: Vec<&str> = wire.strip_prefix("ENTRY2:").unwrap().split(',').collect();
        assert_eq!(f[0], "entry");
        assert_eq!(f[1], "BUY");
        assert_eq!(f[2], "1");
        assert_eq!(f[12], "3");
        assert_eq!(f.len(), 13 + 3);
        assert_eq!(f[13..], ["4005.00000", "4010.00000", "4015.00000"]);
    }

    #[test]
    fn tphit2_niesie_poziom_i_flage_nienumerowanych_pipsow() {
        let w1 = serialize_tp_hit(None, "tp", "4460 HIT +170 PIPS", WireSchema::V2);
        assert_eq!(w1, "TPHIT2:tp,-1,4460.00000,0");
        let w2 = serialize_tp_hit(None, "tp", "+50 PIPS HIT", WireSchema::V2);
        assert_eq!(w2, "TPHIT2:tp,-1,nan,1");
    }

    #[test]
    fn most_profit_telemetry_tlumi_tylko_at_tp() {
        let at = "AT TP3 +90 PIPS\n\nSECURING PARTIAL PROFITS. \
                  SL IS SET TO BE AT 3966 AND TARGETS ARE:\n3980\n3990\n4000";
        let mut sigs = parser::parse(at);
        assert_eq!(parser::suppress_at_tp_hits(&mut sigs, at), 1);
        assert!(!sigs.iter().any(|s| matches!(s, Signal::TpHit { .. })));
        assert!(sigs
            .iter()
            .any(|s| matches!(s, Signal::SecuringPartial { .. })));

        let confirmed = "TP3 HIT +90 PIPS";
        let mut sigs = parser::parse(confirmed);
        assert_eq!(parser::suppress_at_tp_hits(&mut sigs, confirmed), 0);
        assert!(sigs
            .iter()
            .any(|s| matches!(s, Signal::TpHit { index: Some(3) })));
    }

    #[test]
    fn klucz_value_aware_nie_moze_rozerwac_rekordu() {
        let s = Signal::SecuringPartial {
            targets: vec![4510.0, 4520.0],
            sl: Some(4499.0),
            spp_be_level: Some(4501.0),
        };
        let key = wire_action_key(&s, true);
        assert!(key.starts_with("v2h_"));
        assert!(!key.contains('|'));
        assert!(!key.contains(','));
        assert_eq!(key, wire_action_key(&s, true));
    }
}
