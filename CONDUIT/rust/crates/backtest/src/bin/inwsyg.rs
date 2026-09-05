
use conduit_backtest::data::{load_messages, ReplayMessage};
use conduit_core::parser::{self, OpcjeParsera, Signal};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Klasyfikacja JEDNEJ wiadomości.
struct Odczyt {
    /// klucze akcji rozpoznane przy PEŁNYM zestawie przełączników parsera
    akcje_max: Vec<String>,
    /// klucze akcji rozpoznane przy USTAWIENIACH DOMYŚLNYCH (to, co silnik
    /// robi dziś bez włączania osi)
    akcje_dom: Vec<String>,
    /// wiadomość ma kształt bloku wejścia (cel + stop w jednym tekście)
    ksztalt_wejscia: bool,
    /// „CLOSE N LAYERS" — polecenie warstwowe; `Some(true)` = WARUNKOWE
    warstwy: Option<bool>,
    /// „TP OPEN" / „TP: HOLD" — czwarty cel bez poziomu
    tp_open: bool,
    /// trafienie podane POZIOMEM, nie numerem („4460 HIT")
    hit_poziomem: bool,
    /// ile wskazówek cenowych niesie treść (`basket_hints`)
    wskazowki: usize,
    /// wejście typu „BUY STOP" (zlecenie PRZEBICIOWE, nie limit)
    entry_stop: bool,
    /// wejście rynkowe (bez słowa LIMIT/STOP)
    entry_rynkowe: bool,
    /// SPP niosący JAWNY poziom break-even („SL IS SET TO BE AT 4668")
    spp_z_be: bool,
    /// trafienie celu BEZ numeru w treści
    tp_bez_numeru: bool,
    /// polecenie warstwowe niosące LICZBĘ warstw albo POZIOM warstwy —
    /// informacja, której silnik dziś nie czyta (ma własną transzę)
    warstwy_z_trescia: bool,
}

fn opcje_max() -> OpcjeParsera {
    OpcjeParsera {
        geometryczny: false,
        min_pewnosc: 0.0,
        rf_wymaga_wykonania: false,
        // Różnice wobec domyślnych — obie po to, żeby dało się POLICZYĆ to,
        // czego domyślny odczyt nie widzi:
        // * polecenia warstwowe jako osobny typ,
        partials_jako_komenda: true,
        // * poboczne zapisy poleceń („Set SL to BE", „Out. At BE", „RISK FREEE",
        //   „ALL TP'S HIT", „zone is no longer valid", „TP3 should be 4043").
        //   Różnica `maksymalne − domyślne` jest MIARĄ TEJ OSI.
        luz_interpunkcyjny: true,
        recap_guard: false,
    }
}

fn klucze(sygnaly: &[Signal]) -> Vec<String> {
    let mut v: Vec<String> = sygnaly
        .iter()
        .filter(|s| !matches!(s, Signal::Info))
        .map(|s| s.action_key())
        .collect();
    v.sort();
    v.dedup();
    v
}

fn czytaj(m: &ReplayMessage) -> Odczyt {
    let max = parser::parse_z_opcjami(&m.text, opcje_max());
    let dom = parser::parse_z_opcjami(&m.text, OpcjeParsera::default());
    let l = parser::close_layers(&m.text);
    let warstwy = l.as_ref().map(|x| x.optional);
    let mut entry_stop = false;
    let mut entry_rynkowe = false;
    let mut spp_z_be = false;
    let mut tp_bez_numeru = false;
    for s in &max {
        match s {
            Signal::Entry(e) => {
                entry_stop |= e.is_stop;
                entry_rynkowe |= !e.is_limit && !e.is_stop;
            }
            Signal::SecuringPartial { spp_be_level, .. } => spp_z_be |= spp_be_level.is_some(),
            Signal::TpHit { index } => tp_bez_numeru |= index.is_none(),
            _ => {}
        }
    }
    Odczyt {
        akcje_max: klucze(&max),
        akcje_dom: klucze(&dom),
        ksztalt_wejscia: parser::wyglada_na_wejscie(&m.text),
        warstwy,
        tp_open: m.text.to_uppercase().contains("TP OPEN")
            || m.text.to_uppercase().contains("TP: HOLD")
            || m.text.to_uppercase().contains("TP4 OPEN"),
        hit_poziomem: parser::hit_level(&m.text).is_some(),
        wskazowki: parser::basket_hints(&m.text).len(),
        entry_stop,
        entry_rynkowe,
        spp_z_be,
        tp_bez_numeru,
        warstwy_z_trescia: l
            .as_ref()
            .map(|x| x.count.is_some() || x.level.is_some())
            .unwrap_or(false),
    }
}

#[derive(Default)]
struct Kubelek {
    n: usize,
    edycje: usize,
    odpowiedzi: usize,
    sieroty: usize,
    /// wiadomość niosąca WIĘCEJ niż jedną akcję
    z_towarzystwem: usize,
    przyklady: Vec<String>,
}

/// Czy PROZA zawiera slowo, ktore w tych kanalach zwykle znaczy POLECENIE?
///
/// To jest generator LISTY DZIUR, a nie klasyfikator: kazde trafienie trzeba
/// obejrzec recznie. Sam fakt, ze wiadomosc bez rozpoznanej akcji mowi „HIT"
/// albo „CLOSE", nie dowodzi bledu — dowodzi, ze warto tam zajrzec.
fn podejrzana(t: &str) -> bool {
    let u = t.to_uppercase();
    const SLOWA: [&str; 14] = [
        " HIT",
        "CLOSE",
        "CANCEL",
        "RISK FREE",
        "PARTIAL",
        "MOVE SL",
        "MOVE STOP",
        "BREAKEVEN",
        "BREAK EVEN",
        " BE ",
        "INVALID",
        "EXIT",
        "SECURE",
        "STOPPED",
    ];
    SLOWA.iter().any(|w| u.contains(w))
}

fn skrot(t: &str, n: usize) -> String {
    let jedna: String = t
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let s: String = jedna.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s
    }
}

struct Raport {
    plik: String,
    wiadomosci: usize,
    /// wiadomości z akcją (przy pełnych przełącznikach)
    z_akcja_max: usize,
    /// wiadomości z akcją przy ustawieniach DOMYŚLNYCH
    z_akcja_dom: usize,
    proza: usize,
    proza_ksztalt_wejscia: usize,
    edycje: usize,
    edycje_bez_zmiany: usize,
    odpowiedzi: usize,
    sieroty: usize,
    duplikaty: usize,
    wieloakcyjne: usize,
    warstwy_polecenie: usize,
    warstwy_warunkowe: usize,
    tp_open: usize,
    hit_poziomem: usize,
    bez_wskazowek: usize,
    entry_stop: usize,
    entry_rynkowe: usize,
    spp_z_be: usize,
    tp_bez_numeru: usize,
    tp_bez_numeru_z_poziomem: usize,
    warstwy_z_trescia: usize,
    /// PROZA, ktora zawiera slowo-klucz polecenia — kandydatki na dziure
    podejrzana_proza_n: usize,
    podejrzana_proza: Vec<String>,
    typy: BTreeMap<String, Kubelek>,
    proza_przyklady: Vec<String>,
    ksztalt_przyklady: Vec<String>,
}

fn policz(plik: &str, przykladow: usize) -> anyhow::Result<Raport> {
    let msgs = load_messages(plik)?;
    let mut r = Raport {
        plik: plik.to_string(),
        wiadomosci: msgs.len(),
        z_akcja_max: 0,
        z_akcja_dom: 0,
        proza: 0,
        proza_ksztalt_wejscia: 0,
        edycje: 0,
        edycje_bez_zmiany: 0,
        odpowiedzi: 0,
        sieroty: 0,
        duplikaty: 0,
        wieloakcyjne: 0,
        warstwy_polecenie: 0,
        warstwy_warunkowe: 0,
        tp_open: 0,
        hit_poziomem: 0,
        bez_wskazowek: 0,
        entry_stop: 0,
        entry_rynkowe: 0,
        spp_z_be: 0,
        tp_bez_numeru: 0,
        tp_bez_numeru_z_poziomem: 0,
        warstwy_z_trescia: 0,
        podejrzana_proza_n: 0,
        podejrzana_proza: Vec::new(),
        typy: BTreeMap::new(),
        proza_przyklady: Vec::new(),
        ksztalt_przyklady: Vec::new(),
    };

    // DUPLIKAT = ta sama treść, ten sam kanał, ten sam adresat `reply_to`.
    // Bez adresata w kluczu „TP1 HIT" z dwóch różnych setupów liczyłoby się
    // jako powtórzenie, a to są dwa różne polecenia.
    let mut widziane_tresci: BTreeSet<(String, Option<i64>, String)> = BTreeSet::new();
    // ostatnia treść danej wiadomości — do wykrycia edycji BEZ ZMIANY TREŚCI
    let mut ostatnia_tresc: HashMap<(String, i64), String> = HashMap::new();

    for m in &msgs {
        let o = czytaj(m);
        let ma_akcje = !o.akcje_max.is_empty();
        if ma_akcje {
            r.z_akcja_max += 1;
        }
        if !o.akcje_dom.is_empty() {
            r.z_akcja_dom += 1;
        }
        if !ma_akcje {
            r.proza += 1;
            if o.ksztalt_wejscia {
                r.proza_ksztalt_wejscia += 1;
                if r.ksztalt_przyklady.len() < przykladow {
                    r.ksztalt_przyklady
                        .push(format!("#{} {}", m.msg_id, skrot(&m.text, 160)));
                }
            } else {
                if podejrzana(&m.text) {
                    r.podejrzana_proza_n += 1;
                    let wzor = skrot(&m.text, 130);
                    if !r
                        .podejrzana_proza
                        .iter()
                        .any(|x: &String| x.ends_with(&wzor))
                    {
                        r.podejrzana_proza.push(format!("#{} {}", m.msg_id, wzor));
                    }
                }
                if r.proza_przyklady.len() < przykladow {
                    r.proza_przyklady
                        .push(format!("#{} {}", m.msg_id, skrot(&m.text, 120)));
                }
            }
        }
        if o.akcje_max.len() > 1 {
            r.wieloakcyjne += 1;
        }
        if let Some(warunkowe) = o.warstwy {
            if warunkowe {
                r.warstwy_warunkowe += 1;
            } else {
                r.warstwy_polecenie += 1;
            }
        }
        if o.tp_open {
            r.tp_open += 1;
        }
        if o.hit_poziomem {
            r.hit_poziomem += 1;
        }
        if o.entry_stop {
            r.entry_stop += 1;
        }
        if o.entry_rynkowe {
            r.entry_rynkowe += 1;
        }
        if o.spp_z_be {
            r.spp_z_be += 1;
        }
        if o.tp_bez_numeru {
            r.tp_bez_numeru += 1;
            if o.hit_poziomem {
                r.tp_bez_numeru_z_poziomem += 1;
            }
        }
        if o.warstwy_z_trescia {
            r.warstwy_z_trescia += 1;
        }
        if ma_akcje && o.wskazowki == 0 && m.reply_to.is_none() {
            r.bez_wskazowek += 1;
        }
        if let Some(orig) = m.edit_of {
            r.edycje += 1;
            let k = (m.kanal.clone(), orig);
            if ostatnia_tresc
                .get(&k)
                .map(|t| t == &m.text)
                .unwrap_or(false)
            {
                r.edycje_bez_zmiany += 1;
            }
            ostatnia_tresc.insert(k, m.text.clone());
        } else {
            ostatnia_tresc.insert((m.kanal.clone(), m.msg_id), m.text.clone());
        }
        if m.reply_to.is_some() {
            r.odpowiedzi += 1;
        } else if ma_akcje
            && !o
                .akcje_max
                .iter()
                .any(|k| k == "entry" || k.starts_with("mkt"))
        {
            // komunikat ZARZĄDZAJĄCY bez adresata — musi trafić do koszyka
            // regułą „najnowszy żywy", czyli zgadywaniem
            r.sieroty += 1;
        }
        let klucz_tresci = (m.kanal.clone(), m.reply_to, m.text.clone());
        if ma_akcje && !widziane_tresci.insert(klucz_tresci) {
            r.duplikaty += 1;
        }

        for k in &o.akcje_max {
            let kub = r.typy.entry(k.clone()).or_default();
            kub.n += 1;
            if m.edit_of.is_some() {
                kub.edycje += 1;
            }
            if m.reply_to.is_some() {
                kub.odpowiedzi += 1;
            } else {
                kub.sieroty += 1;
            }
            if o.akcje_max.len() > 1 {
                kub.z_towarzystwem += 1;
            }
            if kub.przyklady.len() < przykladow {
                kub.przyklady
                    .push(format!("#{} {}", m.msg_id, skrot(&m.text, 110)));
            }
        }
    }
    Ok(r)
}

fn drukuj(r: &Raport, przykladow: usize) {
    println!("\n================================================================");
    println!("KORPUS {}", r.plik);
    println!("================================================================");
    println!("wiadomości w strumieniu odtwarzania : {}", r.wiadomosci);
    println!(
        "z akcją (przełączniki DOMYŚLNE)     : {} ({:.1} %)",
        r.z_akcja_dom,
        100.0 * r.z_akcja_dom as f64 / r.wiadomosci.max(1) as f64
    );
    println!(
        "z akcją (przełączniki MAKSYMALNE)   : {} ({:.1} %)",
        r.z_akcja_max,
        100.0 * r.z_akcja_max as f64 / r.wiadomosci.max(1) as f64
    );
    println!(
        "PROZA bez akcji                     : {} ({:.1} %)",
        r.proza,
        100.0 * r.proza as f64 / r.wiadomosci.max(1) as f64
    );
    println!(
        "  … a MA KSZTAŁT SYGNAŁU (TP + SL)  : {}   <-- DZIURA",
        r.proza_ksztalt_wejscia
    );
    println!(
        "edycje                              : {} (bez zmiany treści: {})",
        r.edycje, r.edycje_bez_zmiany
    );
    println!("z `reply_to`                        : {}", r.odpowiedzi);
    println!("komunikaty zarządzające BEZ adresata: {}", r.sieroty);
    println!(
        "duplikaty treści (ta sama, ten sam adresat): {}",
        r.duplikaty
    );
    println!("wiadomości WIELOPOLECENIOWE         : {}", r.wieloakcyjne);
    println!(
        "CLOSE N LAYERS polecenie/warunkowe        : {} / {}",
        r.warstwy_polecenie, r.warstwy_warunkowe
    );
    println!("TP OPEN / TP: HOLD w tresci             : {}", r.tp_open);
    println!("trafienie podane POZIOMEM           : {}", r.hit_poziomem);
    println!("akcja bez adresata i bez wskazówki  : {}", r.bez_wskazowek);
    println!("--- INFORMACJA, KTOREJ SILNIK DZIS NIE UZYWA ---");
    println!(
        "wejscie BUY/SELL STOP (przebiciowe): {}   (honor_stop_orders dom. OFF)",
        r.entry_stop
    );
    println!(
        "wejscie bez slowa LIMIT/STOP       : {}   (o trybie decyduje auto_limit)",
        r.entry_rynkowe
    );
    println!(
        "SPP z jawnym poziomem BE           : {}   (spp_sl_mode dom. Off)",
        r.spp_z_be
    );
    println!(
        "cel BEZ numeru / z tego z poziomem : {} / {}   (tp_hit_match_level dom. OFF)",
        r.tp_bez_numeru, r.tp_bez_numeru_z_poziomem
    );
    println!(
        "warstwy z LICZBA/POZIOMEM w tresci : {}   (silnik ma wlasna transze)",
        r.warstwy_z_trescia
    );
    if !r.podejrzana_proza.is_empty() {
        println!(
            "
  PODEJRZANA PROZA — wyglada na polecenie, a nie jest ({}):",
            r.podejrzana_proza_n
        );
        for x in &r.podejrzana_proza {
            println!("    {x}");
        }
    }

    println!("\n  TYP AKCJI      |     n | edycji | reply | sierot | z tow. |");
    println!("  ---------------|-------|--------|-------|--------|--------|");
    let mut wiersze: Vec<(&String, &Kubelek)> = r.typy.iter().collect();
    wiersze.sort_by(|a, b| b.1.n.cmp(&a.1.n));
    for (k, v) in &wiersze {
        println!(
            "  {:<14} | {:>5} | {:>6} | {:>5} | {:>6} | {:>6} |",
            k, v.n, v.edycje, v.odpowiedzi, v.sieroty, v.z_towarzystwem
        );
    }
    if przykladow > 0 {
        for (k, v) in &wiersze {
            println!("\n  --- {k} ---");
            for p in &v.przyklady {
                println!("    {p}");
            }
        }
        if !r.ksztalt_przyklady.is_empty() {
            println!("\n  --- PROZA O KSZTAŁCIE SYGNAŁU (dziura parsera) ---");
            for p in &r.ksztalt_przyklady {
                println!("    {p}");
            }
        }
        if !r.proza_przyklady.is_empty() {
            println!("\n  --- PROZA (bez akcji, bez kształtu sygnału) ---");
            for p in &r.proza_przyklady {
                println!("    {p}");
            }
        }
    }
}

fn json(r: &Raport) -> serde_json::Value {
    let typy: serde_json::Map<String, serde_json::Value> = r
        .typy
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                serde_json::json!({
                    "n": v.n,
                    "edycje": v.edycje,
                    "reply": v.odpowiedzi,
                    "sieroty": v.sieroty,
                    "z_towarzystwem": v.z_towarzystwem,
                }),
            )
        })
        .collect();
    serde_json::json!({
        "plik": r.plik,
        "wiadomosci": r.wiadomosci,
        "z_akcja_domyslne": r.z_akcja_dom,
        "z_akcja_maksymalne": r.z_akcja_max,
        "proza": r.proza,
        "proza_ksztalt_wejscia": r.proza_ksztalt_wejscia,
        "edycje": r.edycje,
        "edycje_bez_zmiany": r.edycje_bez_zmiany,
        "odpowiedzi": r.odpowiedzi,
        "sieroty": r.sieroty,
        "duplikaty": r.duplikaty,
        "wieloakcyjne": r.wieloakcyjne,
        "warstwy_polecenie": r.warstwy_polecenie,
        "warstwy_warunkowe": r.warstwy_warunkowe,
        "tp_open": r.tp_open,
        "hit_poziomem": r.hit_poziomem,
        "bez_wskazowek": r.bez_wskazowek,
        "entry_stop": r.entry_stop,
        "entry_rynkowe": r.entry_rynkowe,
        "spp_z_be": r.spp_z_be,
        "tp_bez_numeru": r.tp_bez_numeru,
        "tp_bez_numeru_z_poziomem": r.tp_bez_numeru_z_poziomem,
        "warstwy_z_trescia": r.warstwy_z_trescia,
        "podejrzana_proza": r.podejrzana_proza_n,
        "podejrzana_proza_wzory": r.podejrzana_proza,
        "typy": typy,
    })
}

fn main() -> anyhow::Result<()> {
    let mut pliki: Vec<String> = Vec::new();
    let mut wy: Option<String> = None;
    let mut przykladow = 0usize;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => wy = it.next(),
            "--przyklady" => przykladow = it.next().and_then(|v| v.parse().ok()).unwrap_or(5),
            _ => pliki.push(a),
        }
    }
    if pliki.is_empty() {
        eprintln!("użycie: inwsyg [--json out.json] [--przyklady N] korpus.json …");
        std::process::exit(2);
    }
    let mut zrzut = Vec::new();
    for p in &pliki {
        let r = policz(p, przykladow.max(if wy.is_some() { 3 } else { 0 }))?;
        drukuj(&r, przykladow);
        zrzut.push(json(&r));
    }
    if let Some(p) = wy {
        std::fs::write(&p, serde_json::to_string_pretty(&zrzut)?)?;
        println!("\nzrzut maszynowy → {p}");
    }
    Ok(())
}
