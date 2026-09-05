
use conduit_core::parser::{self, Signal};
use std::collections::BTreeMap;

fn tekst(m: &serde_json::Value) -> String {
    match m.get("text") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .map(|p| match p {
                serde_json::Value::String(s) => s.as_str(),
                other => other.get("text").and_then(|t| t.as_str()).unwrap_or(""),
            })
            .collect::<String>(),
        _ => String::new(),
    }
}

/// Czy wiadomość WYGLĄDA na sygnał wejścia — niezależnie od tego, co powie parser.
///
/// To jest celowo prymitywne i celowo NIEZALEŻNE od parsera: gdyby korzystało
/// z tej samej logiki, mierzyłoby zgodność parsera z samym sobą. Chodzi o to,
/// żeby wyłapać wiadomości, które człowiek uzna za sygnał, a parser przeoczy.
fn wyglada_na_wejscie(t: &str) -> bool {
    let u = t.to_uppercase();
    (u.contains("BUY") || u.contains("SELL"))
        && (u.contains("LIMIT") || u.contains("GOLD") || u.contains("XAU"))
        && u.contains("SL")
}

fn eksport(plik: &str, cel: &str, czas_edycji: bool) -> anyhow::Result<()> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(plik)?)?;
    let puste = Vec::new();
    let mut msgs: Vec<&serde_json::Value> = v
        .get("messages")
        .and_then(|m| m.as_array())
        .unwrap_or(&puste)
        .iter()
        .filter(|m| m.get("type").and_then(|t| t.as_str()) == Some("message"))
        .collect();
    let pole = |m: &serde_json::Value, k: &str| -> i64 {
        m.get(k)
            .and_then(|x| x.as_str())
            .and_then(|x| x.parse().ok())
            .unwrap_or(0)
    };
    let chwila = move |m: &serde_json::Value| -> i64 {
        let p = pole(m, "date_unixtime");
        let e = pole(m, "edited_unixtime");
        if czas_edycji && e > p {
            e
        } else {
            p
        }
    };
    msgs.sort_by_key(|m| chwila(m));

    let mut sygnaly: Vec<serde_json::Value> = Vec::new();
    // id wiadomosci -> indeks sygnalu, do ktorego nalezy
    let mut wlasciciel: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    // id wiadomosci -> na co odpowiada (do wspinania sie po lancuchu)
    let mut rodzic: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    let mut biezacy: Option<usize> = None;
    let (mut we, mut zd, mut po_lancuchu) = (0u32, 0u32, 0u32);
    for m in &msgs {
        let t = tekst(m);
        if t.trim().is_empty() {
            continue;
        }
        let ts = chwila(m);
        let id = m.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
        if let Some(r) = m.get("reply_to_message_id").and_then(|x| x.as_i64()) {
            rodzic.insert(id, r);
        }
        let rozp = parser::parse(&t);
        if let Some(Signal::Entry(e)) = rozp.iter().find(|x| matches!(x, Signal::Entry(_))) {
            let edycja = pole(m, "edited_unixtime");
            sygnaly.push(serde_json::json!({
                "id": id, "ts": ts,
                "edited": if edycja > pole(m, "date_unixtime") {
                    serde_json::json!(edycja)
                } else {
                    serde_json::Value::Null
                },
                "dir": if e.side == conduit_core::types::Side::Buy { "BUY" } else { "SELL" },
                "limit": e.is_limit, "stop": e.is_stop,
                "lo": e.lo, "hi": e.hi, "sl": e.sl, "sl_z_tekstu": e.sl.is_some(),
                "edit_delay_s": (edycja - pole(m, "date_unixtime")).max(0),
                "tps": e.tps, "tp_open": e.tp_open,
                "tag_high_risk": e.tag_high_risk, "tag_may_not": e.tag_may_not_be_around,
                "warstwy_offset": e.warstwy_offset,
                "tag_first_entry": e.tag_first_entry, "text": t,
                "kanal": "Synergy",
                "reply_to": pole(m, "reply_to_message_id"),
                "events": []
            }));
            biezacy = Some(sygnaly.len() - 1);
            wlasciciel.insert(id, sygnaly.len() - 1);
            we += 1;
            continue;
        }
        if rozp.iter().any(|s| !matches!(s, Signal::Info)) {
            // wspinaczka po lancuchu odpowiedzi; limit kroków chroni przed
            // cyklem w uszkodzonym eksporcie
            let mut cel_idx: Option<usize> = None;
            let mut kursor = id;
            for _ in 0..16 {
                let Some(&r) = rodzic.get(&kursor) else { break };
                if let Some(&i) = wlasciciel.get(&r) {
                    cel_idx = Some(i);
                    break;
                }
                kursor = r;
            }
            if cel_idx.is_some() {
                po_lancuchu += 1;
            }
            if let Some(i) = cel_idx.or(biezacy) {
                // komunikat nalezy do tego koszyka — jego dalsze odpowiedzi
                // maja isc tam samo, nawet jesli odpowiadaja na komunikat
                wlasciciel.insert(id, i);
                if let Some(tab) = sygnaly[i]["events"].as_array_mut() {
                    tab.push(serde_json::json!({
                        "ts": ts, "msg_id": id, "kind": "CMD",
                        "val": serde_json::Value::Null, "text": t,
                        "lvl": serde_json::Value::Null
                    }));
                    zd += 1;
                }
            }
        }
    }
    // Zdarzenie nie moze wyprzedzac wlasnego wejscia. Przy `--czas-edycji`
    // zdarza sie to na ZEN-ie masowo i psuje kolejnosc odtwarzania.
    let wsteczne: usize = sygnaly
        .iter()
        .map(|s| {
            let t0 = s["ts"].as_i64().unwrap_or(0);
            s["events"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter(|e| e["ts"].as_i64().unwrap_or(0) < t0)
                        .count()
                })
                .unwrap_or(0)
        })
        .sum();
    std::fs::write(
        cel,
        serde_json::to_string(&serde_json::json!({ "signals": sygnaly }))?,
    )?;
    println!("zapisano {cel}");
    println!(
        "  czas wejscia    : {}",
        if czas_edycji {
            "EDYCJA (stare)"
        } else {
            "PUBLIKACJA"
        }
    );
    println!("  sygnalow wejscia: {we}");
    println!("  zdarzen         : {zd}   (po lancuchu odpowiedzi: {po_lancuchu})");
    println!("  zdarzen PRZED wlasnym wejsciem: {wsteczne}   <- ma byc 0");
    let lim = sygnaly
        .iter()
        .filter(|s| s["limit"].as_bool() == Some(true))
        .count();
    println!(
        "  w tym LIMIT     : {lim}  ({:.0} %)",
        100.0 * lim as f64 / we.max(1) as f64
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let plik = args.next().unwrap_or_else(|| {
        eprintln!("użycie: synergy <result.json> [--pokaz-niedopasowane N]");
        std::process::exit(2);
    });
    let mut pokaz = 0usize;
    let mut pokaz_wejscia = 0usize;
    let mut podejrzanych = 0usize;
    let reszta: Vec<String> = args.collect();
    if let Some(i) = reszta.iter().position(|a| a == "--pokaz-niedopasowane") {
        pokaz = reszta.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(15);
    }
    if let Some(i) = reszta.iter().position(|a| a == "--pokaz-wejscia") {
        pokaz_wejscia = reszta.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(8);
    }

    // Tryb eksportu: zamien eksport Telegrama na zbior do backtestow.
    //
    // Konwersja idzie przez NASZ PARSER, nie przez wlasne wyrazenia regularne
    // — dzieki temu dziala dla kazdego kanalu, ktory parser rozumie, i nie
    // trzeba pisac nowego skryptu przy kazdym nowym zrodle.
    if let Some(i) = reszta.iter().position(|a| a == "--eksport") {
        let cel = reszta
            .get(i + 1)
            .cloned()
            .unwrap_or_else(|| "signals_nowe.json".into());
        let czas_edycji = reszta.iter().any(|a| a == "--czas-edycji");
        return eksport(&plik, &cel, czas_edycji);
    }

    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&plik)?)?;
    let puste = Vec::new();
    let msgs = v
        .get("messages")
        .and_then(|m| m.as_array())
        .unwrap_or(&puste);

    let mut rodzaje: BTreeMap<&str, usize> = BTreeMap::new();
    let (mut z_tekstem, mut rozpoznane, mut wejscia) = (0usize, 0usize, 0usize);
    let mut wyglada_a_nie_zlapane: Vec<(i64, String)> = Vec::new();
    let mut cele: BTreeMap<usize, usize> = BTreeMap::new();
    let mut bez_sl = 0usize;
    let mut bez_strefy = 0usize;

    for m in msgs {
        if m.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }
        let t = tekst(m);
        if t.trim().is_empty() {
            continue;
        }
        z_tekstem += 1;
        let id = m.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
        let sygnaly = parser::parse(&t);
        let tresciwe: Vec<&Signal> = sygnaly
            .iter()
            .filter(|s| !matches!(s, Signal::Info))
            .collect();
        if !tresciwe.is_empty() {
            rozpoznane += 1;
        }
        let mut ma_wejscie = false;
        for s in &tresciwe {
            let n = match s {
                Signal::Entry(e) => {
                    ma_wejscie = true;
                    wejscia += 1;
                    *cele.entry(e.tps.len()).or_default() += 1;
                    if e.sl.is_none() {
                        bez_sl += 1;
                    }
                    if (e.hi - e.lo).abs() < 1e-9 {
                        bez_strefy += 1;
                    }
                    "Entry"
                }
                Signal::TpHit { .. } => "TpHit",
                Signal::SlHit => "SlHit",
                Signal::RiskFree { .. } => "RiskFree",
                Signal::SecuringPartial { .. } => "SecuringPartial",
                Signal::OutAtEntry => "OutAtEntry",
                Signal::CloseAll => "CloseAll",
                Signal::Cancel => "Cancel",
                _ => "inne",
            };
            *rodzaje.entry(n).or_default() += 1;
        }
        if wyglada_na_wejscie(&t) && !ma_wejscie {
            wyglada_a_nie_zlapane.push((id, t.clone()));
        }
        // Podglad ODCZYTANYCH wartosci obok tresci zrodlowej.
        //
        // "Parser cos zwrocil" to nie to samo co "parser zrozumial". Przy
        // NOWYM formacie trzeba zobaczyc, czy strefa, stop i cele naprawde
        // odpowiadaja temu, co pisze kanal — inaczej mierzymy bzdury
        // z pelnym przekonaniem.
        if pokaz_wejscia > 0 && ma_wejscie {
            if let Some(Signal::Entry(e)) = sygnaly.iter().find(|x| matches!(x, Signal::Entry(_))) {
                if podejrzanych < pokaz_wejscia {
                    podejrzanych += 1;
                    println!("--- #{id}");
                    for l in t.lines().filter(|l| !l.trim().is_empty()).take(7) {
                        println!("      | {l}");
                    }
                    println!(
                        "      ODCZYT: {:?} strefa {}..{} SL {:?} cele {:?} tp_open={} limit={}",
                        e.side, e.lo, e.hi, e.sl, e.tps, e.tp_open, e.is_limit
                    );
                }
            }
        }
    }

    println!("PLIK: {plik}");
    println!(
        "  kanał        : {}",
        v.get("name").and_then(|x| x.as_str()).unwrap_or("?")
    );
    println!("  wiadomości   : {}", msgs.len());
    println!("  z tekstem    : {z_tekstem}");
    println!(
        "  rozpoznanych : {rozpoznane}  ({:.1} %)",
        100.0 * rozpoznane as f64 / z_tekstem.max(1) as f64
    );
    println!();
    println!("ROZPOZNANE ZDARZENIA:");
    for (k, n) in &rodzaje {
        println!("  {k:<18} {n:>5}");
    }
    println!();
    println!("SYGNAŁY WEJŚCIA: {wejscia}");
    println!("  bez SL       : {bez_sl}");
    println!("  bez strefy   : {bez_strefy}   (lo == hi, czyli pojedyncza cena)");
    println!("  rozkład liczby celów:");
    for (n, ile) in &cele {
        println!("    {n} celów: {ile}");
    }
    println!();
    println!(
        "WYGLĄDA NA WEJŚCIE, A PARSER NIE ZŁAPAŁ: {}",
        wyglada_a_nie_zlapane.len()
    );
    for (id, t) in wyglada_a_nie_zlapane.iter().take(pokaz) {
        println!("  --- #{id}");
        for l in t.lines().take(8) {
            println!("      {l}");
        }
    }
    Ok(())
}
