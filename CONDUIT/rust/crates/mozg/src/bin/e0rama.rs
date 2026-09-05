
use conduit_mozg::glebokosc::{self, NogaPlanu};
use conduit_mozg::portfel::{self, Pozycja};
use conduit_mozg::przebieg::{self, PomyslDoObserwacji, PozycjaRamy};
use conduit_mozg::rama::{EtapRamy, GeometriaPomyslu, PowodKonca, Rama, Strona, Ts};
use conduit_mozg::we::{self, anyhow_lite::Result};
use conduit_mozg::{mediana, percentyl, srednia};
use std::collections::HashMap;
use std::path::PathBuf;

const MIN: i64 = 60_000;

struct Args {
    przebieg: PathBuf,
    korpus: PathBuf,
    ticki: PathBuf,
    etykieta: String,
    horyzont_h: f64,
    json: Option<PathBuf>,
    krok_odcisku: f64,
}

fn args() -> Result<Args> {
    let mut a = Args {
        przebieg: PathBuf::new(),
        korpus: PathBuf::new(),
        ticki: PathBuf::new(),
        etykieta: "KANAL".into(),
        horyzont_h: 168.0,
        json: None,
        krok_odcisku: 1.0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut v = || it.next().ok_or_else(|| format!("brak wartości po {k}"));
        match k.as_str() {
            "--przebieg" => a.przebieg = v()?.into(),
            "--korpus" => a.korpus = v()?.into(),
            "--ticki" => a.ticki = v()?.into(),
            "--etykieta" => a.etykieta = v()?,
            "--horyzont-h" => a.horyzont_h = v()?.parse()?,
            "--krok-odcisku" => a.krok_odcisku = v()?.parse()?,
            "--json" => a.json = Some(v()?.into()),
            _ => return Err(format!("nieznany argument: {k}").into()),
        }
    }
    if a.przebieg.as_os_str().is_empty()
        || a.korpus.as_os_str().is_empty()
        || a.ticki.as_os_str().is_empty()
    {
        return Err("wymagane: --przebieg --korpus --ticki".into());
    }
    Ok(a)
}

/// Klasyfikacja komunikatu kanału. Zamknięta lista fraz — „zdanie po polsku
/// się nie policzy", więc każdy komunikat dostaje etykietę z listy.
fn komunikat_zyciowy(t: &str) -> bool {
    let t = t.to_ascii_uppercase();
    const ZYCIOWE: &[&str] = &[
        "STILL VALID",
        "STILL ACTIVE",
        "STILL RUNNING",
        "HOLD",
        "RISK FREE",
        "RISKFREE",
        "SECURING PARTIAL",
        "SL IS SET",
        "SET BE",
        "BREAKEVEN",
        "BREAK EVEN",
        "PIPS HIT",
        "TP HIT",
        "TP1",
        "TP2",
        "TP3",
        "TARGET",
        "PARTIAL",
        "ADD",
        "RUNNING",
    ];
    ZYCIOWE.iter().any(|f| t.contains(f))
}

fn komunikat_konczacy(t: &str) -> bool {
    let t = t.to_ascii_uppercase();
    const KONCZACE: &[&str] = &[
        "CLOSE ALL",
        "CLOSE EVERYTHING",
        "CANCEL",
        "SL HIT",
        "STOPPED OUT",
        "OUT AT ENTRY",
        "CLOSED",
    ];
    KONCZACE.iter().any(|f| t.contains(f))
}

struct WynikRamyPelny {
    r: Rama,
    kanal: String,
    // budżet
    przyznany: f64,
    wydany: f64,
    wykorzystany_pct: Option<f64>,
    // wynik i wychylenia
    pl: f64,
    mfe: f64,
    mae: f64,
    mfe_silnika: f64,
    mfe_suma_pozycji: f64,
    zostawione: f64,
    // życie
    ts_zawiazania: Ts,
    koniec_pomyslu: Ts,
    powod_konca: PowodKonca,
    ts_pierwszego_otwarcia: Ts,
    ts_konca_ekspozycji: Ts,
    // stop
    ts_stopu: Ts,
    stop_zakonczyl_ekspozycje: bool,
    wazna_cena: bool,
    wazna_silnik: bool,
    wazna_kanal: bool,
    tp1_po_stopie_min: Option<f64>,
    komend_po_stopie: u32,
    komend_zyciowych_po_stopie: u32,
    // liczności
    pozycji: u32,
    prob: u32,
    miala_pozycje: bool,
}

fn main() -> Result<()> {
    let a = args()?;
    let koszyki = we::wczytaj_koszyki(a.przebieg.join("koszyki.json"))?;
    let transakcje = we::wczytaj_transakcje(a.przebieg.join("transakcje.json"))?;
    let korpus = we::wczytaj_korpus(&a.korpus)?;
    let dz = we::wczytaj_dziennik(a.przebieg.join("dziennik.jsonl")).unwrap_or_default();
    let horyzont_ms = (a.horyzont_h * 3_600_000.0) as i64;

    let p = portfel::zbuduj(&koszyki, &transakcje, &korpus, &dz.otwarcia, a.krok_odcisku);

    // --- indeks koszyków i sygnałów ---
    let kosz_po_id: HashMap<u32, &we::KoszykDump> = koszyki.iter().map(|k| (k.id, k)).collect();
    let mut sygnal_po_kluczu: HashMap<(String, i64), &we::SygnalKorpusu> = HashMap::new();
    for s in &korpus {
        sygnal_po_kluczu.insert((s.kanal.clone(), s.id), s);
    }

    // --- chwila STOPU: kiedy stop zabrał ostatnią ekspozycję ramy ---
    let mut ts_stopu: Vec<Ts> = vec![0; p.ramy.len()];
    let mut stop_konczy: Vec<bool> = vec![false; p.ramy.len()];
    for (i, poz) in p.pozycje.iter().enumerate() {
        if poz.is_empty() {
            continue;
        }
        let ostatnie = poz.iter().map(|x| x.close_ts).max().unwrap_or(0);
        let stopem = poz.iter().any(|x| x.stop_zabral && x.close_ts == ostatnie);
        if poz.iter().any(|x| x.stop_zabral) {
            ts_stopu[i] = poz
                .iter()
                .filter(|x| x.stop_zabral)
                .map(|x| x.close_ts)
                .max()
                .unwrap_or(0);
        }
        stop_konczy[i] = stopem;
    }

    // --- przelot tickowy ---
    let pomysly: Vec<PomyslDoObserwacji> = p
        .ramy
        .iter()
        .enumerate()
        .map(|(i, r)| PomyslDoObserwacji {
            strona: r.geometria.strona,
            ts: r.ts_zawiazania,
            sl: r.geometria.sl,
            cele: r.geometria.cele.clone(),
            ts_stopu: if stop_konczy[i] { ts_stopu[i] } else { 0 },
        })
        .collect();
    let mut pozycje_plaskie: Vec<PozycjaRamy> = Vec::new();
    for (i, poz) in p.pozycje.iter().enumerate() {
        for x in poz {
            pozycje_plaskie.push(PozycjaRamy {
                rama: i,
                strona: x.strona,
                open_ts: x.open_ts,
                close_ts: x.close_ts,
                open_px: x.open_px,
                wolumen: x.wolumen,
                netto: x.netto,
            });
        }
    }

    let t = we::Ticki::otworz(&a.ticki)?;
    let od = p.ramy.iter().map(|r| r.ts_zawiazania).min().unwrap_or(0);
    let do_ts = p
        .ramy
        .iter()
        .map(|r| r.ts_zawiazania + horyzont_ms)
        .chain(pozycje_plaskie.iter().map(|x| x.close_ts))
        .max()
        .unwrap_or(0);
    let i0 = t.indeks_od(od);
    let i1 = t.indeks_od(do_ts).min(t.len());
    eprintln!(
        "[{}] ram {} · pozycji {} · ticków w oknie {} ({:.1} mln)",
        a.etykieta,
        p.ramy.len(),
        pozycje_plaskie.len(),
        i1.saturating_sub(i0),
        (i1.saturating_sub(i0)) as f64 / 1e6
    );
    let wyniki = przebieg::przelot(
        &pomysly,
        &pozycje_plaskie,
        i1.saturating_sub(i0),
        |i| {
            let j = i0 + i;
            (t.ts(j), t.bid(j), t.ask(j))
        },
        horyzont_ms,
    );

    // --- MFE per pozycja z dziennika (świadek kontrolny) ---
    let mut mfe_poz_koszyka: HashMap<u32, f64> = HashMap::new();
    for z in &dz.zamkniecia {
        if let Some(b) = z.basket {
            *mfe_poz_koszyka.entry(b).or_insert(0.0) += z.mfe_usd.max(0.0);
        }
    }

    // --- składanie rozliczeń ---
    let mut pelne: Vec<WynikRamyPelny> = Vec::new();
    for (i, r) in p.ramy.iter().enumerate() {
        let w = &wyniki[i];
        let (koniec_ceny, powod_ceny) =
            w.koniec_okna(r.ts_zawiazania, horyzont_ms, r.geometria.cele.len());

        // komendy kanału: koniec pomysłu może ogłosić także KANAŁ
        let sygnal = sygnal_po_kluczu.get(&(r.kanal.clone(), r.msg_id));
        let mut koniec_kanalu: Ts = Ts::MAX;
        let mut komend = 0u32;
        let mut komend_po = 0u32;
        let mut komend_zyc_po = 0u32;
        if let Some(s) = sygnal {
            for e in &s.events {
                if e.kind != "CMD" {
                    continue;
                }
                komend += 1;
                let ts = e.ts * 1000;
                if komunikat_konczacy(&e.text) && ts > r.ts_zawiazania && ts < koniec_kanalu {
                    koniec_kanalu = ts;
                }
                if ts_stopu[i] > 0 && ts > ts_stopu[i] {
                    komend_po += 1;
                    if komunikat_zyciowy(&e.text) && !komunikat_konczacy(&e.text) {
                        komend_zyc_po += 1;
                    }
                }
            }
        }
        let (koniec, powod) = if koniec_kanalu < koniec_ceny {
            (koniec_kanalu, PowodKonca::KomendaKanalu)
        } else {
            (koniec_ceny, powod_ceny)
        };

        let pl = r.wynik_usd();
        let mfe = w.mfe_usd;
        let koszyki_ramy: Vec<u32> = r.proby.iter().map(|x| x.koszyk).collect();
        let mfe_silnika: f64 = koszyki_ramy
            .iter()
            .filter_map(|k| kosz_po_id.get(k))
            .map(|k| k.peak_pl_usd)
            .sum();
        let mfe_suma_poz: f64 = koszyki_ramy
            .iter()
            .filter_map(|k| mfe_poz_koszyka.get(k))
            .sum();

        // --- ŚWIADKOWIE „rama ważna po stopie" ---
        // 1. CENA (własny przelot): stop zabrał ekspozycję, SL POMYSŁU nie był
        //    wtedy dotknięty, a rynek doszedł potem do TP1 przed dotknięciem SL.
        let stop = ts_stopu[i];
        let sl_przed_stopem = w.sl_ts > 0 && stop > 0 && w.sl_ts <= stop;
        let tp1_po = w.tp1_po_stopie_ts;
        let sl_po = w.sl_po_stopie_ts;
        let wazna_cena =
            stop_konczy[i] && !sl_przed_stopem && tp1_po > 0 && (sl_po == 0 || tp1_po <= sl_po);
        let tp1_po_min = if wazna_cena {
            Some((tp1_po - stop) as f64 / MIN as f64)
        } else {
            None
        };

        // 2. SILNIK (odczyt `tp_touch_ts` z koszyka — mierzony TYLKO dopóki
        //    koszyk żyje, więc z definicji nie widzi dalszej części historii)
        let wazna_silnik = stop_konczy[i]
            && koszyki_ramy
                .iter()
                .filter_map(|k| kosz_po_id.get(k))
                .any(|k| k.tp_touch_ts.first().copied().unwrap_or(0) > stop);

        // 3. KANAŁ (komunikat po stopie, który traktuje pomysł jak żywy)
        let wazna_kanal = stop_konczy[i] && komend_zyc_po > 0;

        pelne.push(WynikRamyPelny {
            kanal: r.kanal.clone(),
            przyznany: p.budzety[i].przyznany(),
            wydany: p.budzety[i].wydany(),
            wykorzystany_pct: p.budzety[i].wykorzystany_pct(),
            pl,
            mfe,
            mae: w.mae_usd,
            mfe_silnika,
            mfe_suma_pozycji: mfe_suma_poz,
            zostawione: mfe - pl,
            ts_zawiazania: r.ts_zawiazania,
            koniec_pomyslu: koniec,
            powod_konca: powod,
            ts_pierwszego_otwarcia: p.pozycje[i].iter().map(|x| x.open_ts).min().unwrap_or(0),
            ts_konca_ekspozycji: p.pozycje[i].iter().map(|x| x.close_ts).max().unwrap_or(0),
            ts_stopu: stop,
            stop_zakonczyl_ekspozycje: stop_konczy[i],
            wazna_cena,
            wazna_silnik,
            wazna_kanal,
            tp1_po_stopie_min: tp1_po_min,
            komend_po_stopie: komend_po,
            komend_zyciowych_po_stopie: komend_zyc_po,
            pozycji: p.pozycje[i].len() as u32,
            prob: r.proby.len() as u32,
            miala_pozycje: r.miala_pozycje(),
            r: r.clone(),
        });
        let _ = komend;
    }

    // --- NOGI PLANU do kontrfaktyku głębokości ---
    let mut nogi: Vec<NogaPlanu> = Vec::new();
    for (i, r) in p.ramy.iter().enumerate() {
        for proba in &r.proby {
            let Some(k) = kosz_po_id.get(&proba.koszyk) else {
                continue;
            };
            for w in &k.warstwy {
                let netto: f64 = dopasuj_nogi(&p.pozycje[i], w);
                nogi.push(NogaPlanu {
                    rama: i,
                    poziom: w.poziom,
                    glebokosc_zlecenia: r.geometria.glebokosc(w.cena_zlecenia),
                    glebokosc_fillu: if w.fill_px > 0.0 {
                        Some(r.geometria.glebokosc(w.fill_px))
                    } else {
                        None
                    },
                    wolumen: w.wolumen,
                    wypelniona: w.filled || w.fill_ts > 0,
                    netto,
                    wyjscie_koszykowe: false,
                });
            }
        }
    }
    // wyjścia koszykowe (łamią addytywność) — po powodzie zamknięcia
    let koszykowe: u32 = transakcje
        .iter()
        .filter(|t| {
            matches!(
                t.reason.as_str(),
                "BasketClose" | "Harvest" | "RiskFree" | "DayTarget" | "MaxDd" | "EodFlat"
            )
        })
        .count() as u32;

    // ============================================================
    //  AGREGATY
    // ============================================================
    let n = pelne.len();
    let z_ekspozycja: Vec<&WynikRamyPelny> = pelne.iter().filter(|x| x.miala_pozycje).collect();

    // --- 1. ŻYCIE RAMY ---
    let mut zycie_pomyslu: Vec<f64> = pelne
        .iter()
        .map(|x| (x.koniec_pomyslu - x.ts_zawiazania) as f64 / MIN as f64)
        .collect();
    let cenzura = pelne
        .iter()
        .filter(|x| x.powod_konca == PowodKonca::Horyzont)
        .count();
    let mut zycie_ekspo: Vec<f64> = z_ekspozycja
        .iter()
        .filter(|x| x.ts_pierwszego_otwarcia > 0)
        .map(|x| (x.ts_konca_ekspozycji - x.ts_pierwszego_otwarcia) as f64 / MIN as f64)
        .collect();
    let mut zycie_ramy_silnik: Vec<f64> = z_ekspozycja
        .iter()
        .map(|x| (x.ts_konca_ekspozycji.max(x.ts_zawiazania) - x.ts_zawiazania) as f64 / MIN as f64)
        .collect();

    // --- 2. RAMY WAŻNE PO STOPIE ---
    let ze_stopem: Vec<&WynikRamyPelny> = pelne
        .iter()
        .filter(|x| x.stop_zakonczyl_ekspozycje)
        .collect();
    let sw_cena = ze_stopem.iter().filter(|x| x.wazna_cena).count();
    let sw_silnik = ze_stopem.iter().filter(|x| x.wazna_silnik).count();
    let sw_kanal = ze_stopem.iter().filter(|x| x.wazna_kanal).count();
    let zgodni = ze_stopem
        .iter()
        .filter(|x| x.wazna_cena && x.wazna_silnik)
        .count();
    let tylko_cena = ze_stopem
        .iter()
        .filter(|x| x.wazna_cena && !x.wazna_silnik)
        .count();
    let tylko_silnik = ze_stopem
        .iter()
        .filter(|x| !x.wazna_cena && x.wazna_silnik)
        .count();
    let mut opoznienia: Vec<f64> = ze_stopem
        .iter()
        .filter_map(|x| x.tp1_po_stopie_min)
        .collect();
    let w_60 = ze_stopem
        .iter()
        .filter(|x| x.tp1_po_stopie_min.map(|m| m <= 60.0).unwrap_or(false))
        .count();
    let w_240 = ze_stopem
        .iter()
        .filter(|x| x.tp1_po_stopie_min.map(|m| m <= 240.0).unwrap_or(false))
        .count();

    // --- 3. BUDŻET ---
    let mut wyk: Vec<f64> = pelne
        .iter()
        .filter_map(|x| x.wykorzystany_pct)
        .filter(|x| *x > 0.0)
        .collect();
    let bez_mianownika = pelne
        .iter()
        .filter(|x| x.wykorzystany_pct.is_none())
        .count();
    let ponad_100 = wyk.iter().filter(|x| **x > 100.0).count();
    let ponad_150 = wyk.iter().filter(|x| **x > 150.0).count();

    // --- 4. MFE / MAE / ZOSTAWIONE ---
    let mut zostawione: Vec<f64> = z_ekspozycja.iter().map(|x| x.zostawione).collect();
    let mut mfe_v: Vec<f64> = z_ekspozycja.iter().map(|x| x.mfe).collect();
    let mut mae_v: Vec<f64> = z_ekspozycja.iter().map(|x| x.mae).collect();
    let suma_zostawione: f64 = zostawione.iter().sum();
    let suma_pl: f64 = z_ekspozycja.iter().map(|x| x.pl).sum();
    let suma_mfe: f64 = mfe_v.iter().sum();
    let suma_mfe_silnika: f64 = z_ekspozycja.iter().map(|x| x.mfe_silnika).sum();
    let suma_mfe_pozycji: f64 = z_ekspozycja.iter().map(|x| x.mfe_suma_pozycji).sum();
    let rozjazd_mfe: Vec<f64> = z_ekspozycja
        .iter()
        .filter(|x| x.mfe_silnika.abs() > 0.01 || x.mfe.abs() > 0.01)
        .map(|x| x.mfe - x.mfe_silnika)
        .collect();

    // --- 5. GŁĘBOKOŚĆ ---
    let rozklad = glebokosc::rozklad_glebokosci(&nogi);
    let progi = [0.0, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0];
    let krzywa = glebokosc::krzywa(&nogi, p.ramy.len(), &progi);

    // ============================================================
    //  WYDRUK
    // ============================================================
    println!(
        "\n================  E0-RAMA · {}  ================",
        a.etykieta
    );
    println!(
        "ram {n} (z ekspozycją {}) · prób {} · pozycji {} · koszyków {} · bez korpusu {}",
        z_ekspozycja.len(),
        pelne.iter().map(|x| x.prob).sum::<u32>(),
        pelne.iter().map(|x| x.pozycji).sum::<u32>(),
        koszyki.len(),
        p.koszykow_bez_korpusu
    );
    println!("slot→kanał: {:?}", p.slot_do_kanalu);

    println!("\n--- 1. ŻYCIE RAMY (min) ---");
    println!(
        "POMYSŁ  (cena+kanał, horyzont {:.0} h): p50 {:.1} · p90 {:.1} · p99 {:.1} · max {:.1} · ucięte horyzontem {}",
        a.horyzont_h,
        percentyl(&mut zycie_pomyslu, 50.0),
        percentyl(&mut zycie_pomyslu, 90.0),
        percentyl(&mut zycie_pomyslu, 99.0),
        percentyl(&mut zycie_pomyslu, 100.0),
        cenzura
    );
    println!(
        "EKSPOZYCJA (pierwsze otwarcie→ostatnie zamknięcie): p50 {:.1} · p90 {:.1} · p99 {:.1} · max {:.1}",
        percentyl(&mut zycie_ekspo, 50.0),
        percentyl(&mut zycie_ekspo, 90.0),
        percentyl(&mut zycie_ekspo, 99.0),
        percentyl(&mut zycie_ekspo, 100.0)
    );
    println!(
        "RAMA W SILNIKU (sygnał→ostatnie zamknięcie): p50 {:.1} · p90 {:.1} · p99 {:.1} · max {:.1}",
        percentyl(&mut zycie_ramy_silnik, 50.0),
        percentyl(&mut zycie_ramy_silnik, 90.0),
        percentyl(&mut zycie_ramy_silnik, 99.0),
        percentyl(&mut zycie_ramy_silnik, 100.0)
    );
    let mut powody: Vec<(String, u32)> = Vec::new();
    for x in &pelne {
        let nazwa = x.powod_konca.nazwa().to_string();
        match powody.iter_mut().find(|(n, _)| *n == nazwa) {
            Some((_, c)) => *c += 1,
            None => powody.push((nazwa, 1)),
        }
    }
    powody.sort();
    println!("powody końca pomysłu: {powody:?}");

    println!("\n--- 2. RAMY WAŻNE PO STOPIE (dwóch niezależnych świadków + kanał) ---");
    println!("ram, którym stop zabrał ekspozycję: {}", ze_stopem.len());
    println!(
        "  ŚWIADEK CENA   (mój przelot tickowy, TP1 po stopie przed SL pomysłu): {} = {:.1} %",
        sw_cena,
        100.0 * sw_cena as f64 / ze_stopem.len().max(1) as f64
    );
    println!(
        "  ŚWIADEK SILNIK (`tp_touch_ts[0]` z koszyka, mierzone tylko za życia koszyka): {} = {:.1} %",
        sw_silnik,
        100.0 * sw_silnik as f64 / ze_stopem.len().max(1) as f64
    );
    println!(
        "  ŚWIADEK KANAŁ  (komunikat traktujący pomysł jak żywy PO stopie): {} = {:.1} %",
        sw_kanal,
        100.0 * sw_kanal as f64 / ze_stopem.len().max(1) as f64
    );
    println!(
        "  zgodni CENA∧SILNIK {zgodni} · tylko CENA {tylko_cena} · tylko SILNIK {tylko_silnik}"
    );
    println!(
        "  TP1 po stopie w 60 min: {} · w 240 min: {} · mediana opóźnienia {:.1} min",
        w_60,
        w_240,
        mediana(&mut opoznienia)
    );

    println!("\n--- 3. BUDŻET RAMY — KSIĘGA BEZ SUFITU ---");
    println!(
        "ram z mianownikiem {} · bez SL (mianownik = 0, N11) {}",
        wyk.len(),
        bez_mianownika
    );
    println!(
        "budzet_wykorzystany_pct: p10 {:.1} · p25 {:.1} · MEDIANA {:.1} · p75 {:.1} · p90 {:.1} · p99 {:.1} · max {:.1}",
        percentyl(&mut wyk, 10.0),
        percentyl(&mut wyk, 25.0),
        percentyl(&mut wyk, 50.0),
        percentyl(&mut wyk, 75.0),
        percentyl(&mut wyk, 90.0),
        percentyl(&mut wyk, 99.0),
        percentyl(&mut wyk, 100.0)
    );
    println!(
        "ponad 100 %: {} ({:.1} %) · ponad 150 %: {} · średnia {:.1} %",
        ponad_100,
        100.0 * ponad_100 as f64 / wyk.len().max(1) as f64,
        ponad_150,
        srednia(&wyk)
    );

    println!("\n--- 4. MFE / MAE / ZOSTAWIONE NA STOLE ---");
    println!(
        "MFE ramy:   mediana {:.2} · p90 {:.2} · suma {:.2} $",
        percentyl(&mut mfe_v, 50.0),
        percentyl(&mut mfe_v, 90.0),
        suma_mfe
    );
    println!(
        "MAE ramy:   mediana {:.2} · p10 {:.2} · min {:.2} $",
        percentyl(&mut mae_v, 50.0),
        percentyl(&mut mae_v, 10.0),
        percentyl(&mut mae_v, 0.0)
    );
    println!(
        "ZOSTAWIONE: MEDIANA {:.2} · p25 {:.2} · p75 {:.2} · p90 {:.2} · suma {:.2} $ (wynik ram {:.2} $)",
        percentyl(&mut zostawione, 50.0),
        percentyl(&mut zostawione, 25.0),
        percentyl(&mut zostawione, 75.0),
        percentyl(&mut zostawione, 90.0),
        suma_zostawione,
        suma_pl
    );
    println!(
        "ŚWIADKOWIE MFE: rama(ticki) {:.2} · silnik(peak_pl_usd) {:.2} · suma po pozycjach(dziennik) {:.2}",
        suma_mfe, suma_mfe_silnika, suma_mfe_pozycji
    );
    println!(
        "  rozjazd rama−silnik: mediana {:.4} · maks |{:.2}| · ram porównanych {}",
        mediana(&mut rozjazd_mfe.clone()),
        rozjazd_mfe.iter().fold(0.0f64, |a, b| a.max(b.abs())),
        rozjazd_mfe.len()
    );

    println!("\n--- 5. KONTRFAKTYK GŁĘBOKOŚCI (0 stopni swobody) ---");
    println!(
        "wypełnień {} · PONIŻEJ DALSZEJ KRAWĘDZI {} = {:.1} % · mediana głębokości {:.2} · p90 {:.2} · max {:.2}",
        rozklad.fillow,
        rozklad.ponizej_dalszej_krawedzi,
        100.0 * rozklad.ponizej_dalszej_krawedzi as f64 / rozklad.fillow.max(1) as f64,
        rozklad.mediana,
        rozklad.p90,
        rozklad.max
    );
    println!("  D  szczebli  fillów   wolumen      wynik $   ram bez wejścia   odciętych z wyjściem koszykowym");
    for w in &krzywa {
        println!(
            " {:>4.2}  {:>4}/{:<4} {:>4}/{:<4} {:>6.2}/{:<6.2} {:>9.2}/{:<9.2} {:>4}/{:<4} {:>6}",
            w.prog_d,
            w.szczebli_zostaje,
            w.szczebli_planu,
            w.fillow_zostaje,
            w.fillow,
            w.wolumen_zostaje,
            w.wolumen,
            w.wynik_zostaje_usd,
            w.wynik_usd,
            w.ram_bez_wejscia,
            w.ram_z_fillami,
            w.odcietych_z_wyjsciem_koszykowym
        );
    }
    println!(
        "  ZASTRZEŻENIE addytywności: {} transakcji ({:.1} %) wyszło regułą KOSZYKOWĄ",
        koszykowe,
        100.0 * koszykowe as f64 / transakcje.len().max(1) as f64
    );

    // --- JSON ---
    if let Some(jp) = &a.json {
        let ramy_json: Vec<serde_json::Value> = pelne
            .iter()
            .map(|x| {
                serde_json::json!({
                    "rama": x.r.id, "kanal": x.kanal, "msg_id": x.r.msg_id,
                    "odcisk": x.r.odcisk, "etap": x.r.etap.nazwa(),
                    "strona": if x.r.geometria.strona == Strona::Buy { "Buy" } else { "Sell" },
                    "ts_zawiazania": x.ts_zawiazania,
                    "koniec_pomyslu": x.koniec_pomyslu, "powod_konca": x.powod_konca.nazwa(),
                    "zycie_pomyslu_min": (x.koniec_pomyslu - x.ts_zawiazania) as f64 / MIN as f64,
                    "budzet_przyznany": x.przyznany, "budzet_wydany": x.wydany,
                    "budzet_wykorzystany_pct": x.wykorzystany_pct,
                    "pl": x.pl, "mfe": x.mfe, "mae": x.mae,
                    "mfe_silnika": x.mfe_silnika, "mfe_suma_pozycji": x.mfe_suma_pozycji,
                    "zostawione_usd": x.zostawione,
                    "pozycji": x.pozycji, "prob": x.prob,
                    "stop_zakonczyl": x.stop_zakonczyl_ekspozycje,
                    "wazna_cena": x.wazna_cena, "wazna_silnik": x.wazna_silnik, "wazna_kanal": x.wazna_kanal,
                    "tp1_po_stopie_min": x.tp1_po_stopie_min,
                    "komend_po_stopie": x.komend_po_stopie,
                })
            })
            .collect();
        let krzywa_json: Vec<serde_json::Value> = krzywa
            .iter()
            .map(|w| {
                serde_json::json!({
                    "d": w.prog_d,
                    "szczebli_zostaje": w.szczebli_zostaje, "szczebli_planu": w.szczebli_planu,
                    "fillow_zostaje": w.fillow_zostaje, "fillow": w.fillow,
                    "wolumen_zostaje": w.wolumen_zostaje, "wolumen": w.wolumen,
                    "wynik_zostaje_usd": w.wynik_zostaje_usd, "wynik_usd": w.wynik_usd,
                    "ram_bez_wejscia": w.ram_bez_wejscia, "ram_z_fillami": w.ram_z_fillami,
                })
            })
            .collect();
        let out = serde_json::json!({
            "etykieta": a.etykieta,
            "ram": n, "ram_z_ekspozycja": z_ekspozycja.len(),
            "zycie_pomyslu_min": {"p50": percentyl(&mut zycie_pomyslu, 50.0), "p90": percentyl(&mut zycie_pomyslu, 90.0), "p99": percentyl(&mut zycie_pomyslu, 99.0), "max": percentyl(&mut zycie_pomyslu, 100.0), "cenzura": cenzura},
            "zycie_ekspozycji_min": {"p50": percentyl(&mut zycie_ekspo, 50.0), "p90": percentyl(&mut zycie_ekspo, 90.0), "p99": percentyl(&mut zycie_ekspo, 99.0)},
            "zycie_ramy_silnik_min": {"p50": percentyl(&mut zycie_ramy_silnik, 50.0), "p90": percentyl(&mut zycie_ramy_silnik, 90.0), "p99": percentyl(&mut zycie_ramy_silnik, 99.0)},
            "po_stopie": {"ram": ze_stopem.len(), "cena": sw_cena, "silnik": sw_silnik, "kanal": sw_kanal,
                          "zgodni": zgodni, "tylko_cena": tylko_cena, "tylko_silnik": tylko_silnik,
                          "w_60min": w_60, "w_240min": w_240},
            "budzet": {"n": wyk.len(), "bez_sl": bez_mianownika, "ponad_100": ponad_100, "ponad_150": ponad_150,
                       "p50": percentyl(&mut wyk, 50.0), "p90": percentyl(&mut wyk, 90.0), "p99": percentyl(&mut wyk, 99.0), "max": percentyl(&mut wyk, 100.0)},
            "zostawione": {"mediana": percentyl(&mut zostawione, 50.0), "p90": percentyl(&mut zostawione, 90.0), "suma": suma_zostawione,
                           "suma_pl": suma_pl, "suma_mfe": suma_mfe, "suma_mfe_silnika": suma_mfe_silnika, "suma_mfe_pozycji": suma_mfe_pozycji},
            "glebokosc": {"fillow": rozklad.fillow, "ponizej_dalszej": rozklad.ponizej_dalszej_krawedzi,
                          "mediana": rozklad.mediana, "p90": rozklad.p90, "max": rozklad.max, "krzywa": krzywa_json},
            "ramy": ramy_json,
        });
        std::fs::write(jp, serde_json::to_string_pretty(&out)?)?;
        println!("\nzapisano {}", jp.display());
    }
    Ok(())
}

/// Wynik NETTO nogi przypisanej do szczebla. Dopasowanie po CHWILI i CENIE
/// wypełnienia — `fill_ts`/`fill_px` są odczytem z realizacji zlecenia, więc
/// są tą samą liczbą co `open_ts`/`open_price` pozycji.
fn dopasuj_nogi(pozycje: &[Pozycja], w: &we::WarstwaDump) -> f64 {
    if w.fill_ts == 0 {
        return 0.0;
    }
    pozycje
        .iter()
        .filter(|p| p.open_ts == w.fill_ts && (p.open_px - w.fill_px).abs() < 1e-6)
        .map(|p| p.netto)
        .sum()
}

// `EtapRamy` i `GeometriaPomyslu` są w użyciu przez `Rama`; import jawny
// trzyma ostrzeżenia kompilatora w ryzach.
#[allow(dead_code)]
fn _typy(_: EtapRamy, _: GeometriaPomyslu) {}
