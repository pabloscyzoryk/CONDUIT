
use anyhow::{bail, Result};
use conduit_ai::peak::*;
use conduit_backtest::{load_signals, TickData};
use conduit_core::types::Ts;

// ============================================================
//  ARGUMENTY
// ============================================================

struct Args {
    ticks: String,
    signals: String,
    krok_s: i64,
    horyzont_h: i64,
    msg_offset_min: f64,
    jednostki: usize,
    n_r: f64,
    od: Option<String>,
    kapital: f64,
    placebo_h: i64,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            ticks: "data/ticks.bin".into(),
            signals: "data/signals.json".into(),
            krok_s: 60,
            horyzont_h: 48,
            msg_offset_min: 180.0,
            jednostki: 3,
            n_r: 6.0,
            od: None,
            kapital: 200.0,
            placebo_h: 0,
        }
    }
}

fn parse() -> Result<Args> {
    let mut a = Args::default();
    let v: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < v.len() {
        let k = v[i].clone();
        macro_rules! nast {
            () => {{
                i += 1;
                v.get(i)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("brak wartości dla {}", k))?
            }};
        }
        match k.as_str() {
            "--ticks" => a.ticks = nast!(),
            "--signals" => a.signals = nast!(),
            "--krok-s" => a.krok_s = nast!().parse()?,
            "--horyzont-h" => a.horyzont_h = nast!().parse()?,
            "--msg-offset-min" => a.msg_offset_min = nast!().parse()?,
            "--jednostki" => a.jednostki = nast!().parse()?,
            "--n-r" => a.n_r = nast!().parse()?,
            "--od" => a.od = Some(nast!()),
            "--kapital" => a.kapital = nast!().parse()?,
            "--placebo-h" => a.placebo_h = nast!().parse()?,
            other => bail!("nieznany argument: {other}"),
        }
        i += 1;
    }
    Ok(a)
}

fn dzien_z_daty(s: &str) -> Result<i64> {
    use chrono::NaiveDate;
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d")?;
    let ep = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    Ok((d - ep).num_days())
}

fn data_ts(ts: Ts) -> String {
    use chrono::{DateTime, Utc};
    DateTime::<Utc>::from_timestamp_millis(ts)
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "?".into())
}

// ============================================================
//  POLITYKI — plan życia jednego koszyka
// ============================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pol {
    Tp1,
    BezCelu,
    Czempion,
}

impl Pol {
    fn nazwa(self) -> &'static str {
        match self {
            Pol::Tp1 => "wszystko na TP1",
            Pol::BezCelu => "nic bez celu",
            Pol::Czempion => "czempion (1×TP1 + runnery)",
        }
    }
}

/// Co koszyk robi w czasie: kiedy się zamyka, z jakim wynikiem i jak wygląda
/// jego wychylenie po drodze (do liczenia equity mark-to-market).
struct Plan {
    ts0: Ts,
    ts_zam: Ts,
    dzien: i64,
    wynik: f64,
    /// (chwila, wychylenie) — wyłącznie do `ts_zam`
    marks: Vec<(Ts, f32)>,
}

/// Buduje plan koszyka dla zadanej polityki i twardego czasu życia.
///
/// `zycie_h == 0` oznacza brak limitu.
fn plan(s: &Sciezka, pol: Pol, zycie_min: i64) -> Plan {
    let ts0 = s.ts0();
    let kres = if zycie_min > 0 {
        ts0 + zycie_min * 60_000
    } else {
        Ts::MAX
    };

    // chwila i wynik zamknięcia wg polityki, PRZED nałożeniem limitu czasu
    let (mut ts_zam, mut wynik) = match pol {
        Pol::Tp1 => match (s.ts_tp1, s.wych_tp1) {
            (Some(t), Some(w)) => (t, w as f64),
            _ => (s.probki[s.probki.len() - 1].ts, s.wych_koniec as f64),
        },
        Pol::BezCelu => (s.probki[s.probki.len() - 1].ts, s.wych_koniec as f64),
        // czempion: jedna jednostka wychodzi na TP1, reszta biegnie — koszyk
        // żyje do końca, a wynik jest już policzony w generatorze
        Pol::Czempion => (s.probki[s.probki.len() - 1].ts, s.wych_runner as f64),
    };

    // twardy czas życia ucina wcześniej i realizuje po cenie rynkowej.
    // Dla czempiona wycena MUSI iść przez `wych_run` — jedna jednostka jest
    // już zamknięta na TP1, więc `wych` (trzy warstwy) zawyżałby wynik.
    let mark = |p: &Probka| {
        if pol == Pol::Czempion {
            p.wych_run
        } else {
            p.wych
        }
    };
    if ts_zam > kres {
        if let Some(p) = s.probki.iter().rev().find(|p| p.ts <= kres) {
            ts_zam = p.ts;
            wynik = mark(p) as f64;
        }
    }

    let marks: Vec<(Ts, f32)> = s
        .probki
        .iter()
        .filter(|p| p.ts <= ts_zam)
        .map(|p| (p.ts, mark(p)))
        .collect();
    Plan {
        ts0,
        ts_zam,
        dzien: s.dzien,
        wynik,
        marks,
    }
}

// ============================================================
//  SYMULACJA PORTFELA
// ============================================================

/// Jeden epizod obsunięcia: od szczytu equity do dołka.
struct Epizod {
    szczyt_ts: Ts,
    dolek_ts: Ts,
    glebokosc: f64,
    otwartych_w_dolku: usize,
    najgorszy_w_dolku: f64,
    otwarte_w_dolku: f64,
    zrealizowane_w_dolku: f64,
}

struct WynikPortfela {
    pnl: f64,
    maxdd: f64,
    /// obsunięcie liczone PO STAREMU — na dziennych sumach zamkniętych
    maxdd_zamkniete: f64,
    przyjetych: usize,
    odrzuconych: usize,
    max_jednoczesnie: usize,
    sr_jednoczesnie: f64,
    min_equity: f64,
    zerowan: usize,
    dni_plus: f64,
    pf: f64,
    epizody: Vec<Epizod>,
    // --- STRATY: to jest teraz kryterium, obsunięcie jest tylko informacją ---
    /// minimum SKUMULOWANEJ krzywej dziennej po zamkniętych — odpowiedź na
    /// pytanie „czy to obsunięcie konta, czy rozpiętość krzywej P&L"
    min_skum_zam: f64,
    dni_str: usize,
    dni_wsz: usize,
    najgorszy_dzien: f64,
    najdluzsza_seria: usize,
    najgorsza_seria: f64,
    suma_strat: f64,
    udzial_10_najgorszych: f64,
    najgorsze_koszyki: Vec<f64>,
}

/// Symulacja chronologiczna z limitem jednoczesnych koszyków.
///
/// Limit działa **przy wejściu**: koszyk, który przychodzi, gdy zajęte są
/// wszystkie miejsca, jest odrzucany w całości — nie „czeka w kolejce".
/// Tak zachowuje się silnik i tak zachowuje się broker przy braku marginesu.
fn symuluj(plany: &[Plan], limit: usize, kapital: f64) -> WynikPortfela {
    let mut idx: Vec<usize> = (0..plany.len()).collect();
    idx.sort_by_key(|i| plany[*i].ts0);

    // --- kto zostaje przyjęty ---
    let mut przyjete: Vec<usize> = Vec::with_capacity(plany.len());
    let mut odrzuconych = 0usize;
    // kopiec po chwili zamknięcia (najwcześniejsze na wierzchu)
    let mut czynne: std::collections::BinaryHeap<std::cmp::Reverse<(Ts, usize)>> =
        std::collections::BinaryHeap::new();
    for &i in &idx {
        let t = plany[i].ts0;
        while let Some(std::cmp::Reverse((tz, _))) = czynne.peek() {
            if *tz <= t {
                czynne.pop();
            } else {
                break;
            }
        }
        if limit == 0 || czynne.len() < limit {
            przyjete.push(i);
            czynne.push(std::cmp::Reverse((plany[i].ts_zam, i)));
        } else {
            odrzuconych += 1;
        }
    }

    // --- oś zdarzeń: aktualizacje mark-to-market i zamknięcia ---
    // typ 0 = mark, typ 1 = zamknięcie (musi być PO markach tej samej chwili)
    let mut zd: Vec<(Ts, u8, usize, f64)> = Vec::with_capacity(plany.len() * 64);
    for &i in &przyjete {
        for (t, w) in &plany[i].marks {
            zd.push((*t, 0, i, *w as f64));
        }
        zd.push((plany[i].ts_zam, 1, i, plany[i].wynik));
    }
    zd.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let mut otw: std::collections::HashMap<usize, f64> = std::collections::HashMap::new();
    let mut suma_otw = 0.0f64;
    let mut zreal = 0.0f64;
    let mut szczyt = 0.0f64;
    let mut szczyt_ts: Ts = zd.first().map(|z| z.0).unwrap_or(0);
    let mut maxdd = 0.0f64;
    let mut min_equity = 0.0f64;
    let mut zerowan = 0usize;
    let mut pod_woda = false;
    let mut max_jedn = 0usize;
    let mut suma_jedn = 0.0f64;
    let mut n_jedn = 0.0f64;
    // bieżący epizod obsunięcia
    let mut ep: Option<Epizod> = None;
    let mut epizody: Vec<Epizod> = Vec::new();

    for (t, typ, i, v) in zd {
        if typ == 0 {
            let stary = otw.insert(i, v).unwrap_or(0.0);
            suma_otw += v - stary;
        } else {
            if let Some(stary) = otw.remove(&i) {
                suma_otw -= stary;
            }
            zreal += v;
        }
        let equity = zreal + suma_otw;
        max_jedn = max_jedn.max(otw.len());
        suma_jedn += otw.len() as f64;
        n_jedn += 1.0;
        min_equity = min_equity.min(equity);
        // zerowanie konta: equity spada poniżej −kapitał
        if equity <= -kapital {
            if !pod_woda {
                zerowan += 1;
                pod_woda = true;
            }
        } else {
            pod_woda = false;
        }

        if equity > szczyt {
            szczyt = equity;
            szczyt_ts = t;
            if let Some(e) = ep.take() {
                epizody.push(e);
            }
        } else {
            let g = szczyt - equity;
            if g > maxdd {
                maxdd = g;
            }
            let glebszy = ep.as_ref().map_or(true, |e| g > e.glebokosc);
            if glebszy {
                let najgorszy = otw.values().cloned().fold(0.0f64, f64::min);
                ep = Some(Epizod {
                    szczyt_ts,
                    dolek_ts: t,
                    glebokosc: g,
                    otwartych_w_dolku: otw.len(),
                    najgorszy_w_dolku: najgorszy,
                    otwarte_w_dolku: suma_otw,
                    zrealizowane_w_dolku: zreal,
                });
            }
        }
    }
    if let Some(e) = ep.take() {
        epizody.push(e);
    }
    epizody.sort_by(|a, b| b.glebokosc.partial_cmp(&a.glebokosc).unwrap());
    epizody.truncate(5);

    // --- odniesienie: obsunięcie liczone PO STAREMU, na zamkniętych ---
    let mut dni: std::collections::BTreeMap<i64, f64> = std::collections::BTreeMap::new();
    for &i in &przyjete {
        *dni.entry(plany[i].dzien).or_insert(0.0) += plany[i].wynik;
    }
    let (mut eq, mut sz, mut mdd_z) = (0.0f64, 0.0f64, 0.0f64);
    for v in dni.values() {
        eq += v;
        sz = sz.max(eq);
        mdd_z = mdd_z.max(sz - eq);
    }
    // minimum skumulowanej krzywej — czy w ogóle schodzi pod zero
    let mut min_skum = 0.0f64;
    {
        let mut e = 0.0f64;
        for v in dni.values() {
            e += v;
            min_skum = min_skum.min(e);
        }
    }
    // serie dni stratnych
    let (mut seria, mut najdl, mut biez_suma, mut najgorsza_seria) =
        (0usize, 0usize, 0.0f64, 0.0f64);
    let mut najgorszy_dzien = 0.0f64;
    for v in dni.values() {
        najgorszy_dzien = najgorszy_dzien.min(*v);
        if *v < 0.0 {
            seria += 1;
            biez_suma += v;
            najdl = najdl.max(seria);
            najgorsza_seria = najgorsza_seria.min(biez_suma);
        } else {
            seria = 0;
            biez_suma = 0.0;
        }
    }
    // najgorsze pojedyncze koszyki
    let mut straty: Vec<f64> = przyjete
        .iter()
        .map(|i| plany[*i].wynik)
        .filter(|v| *v < 0.0)
        .collect();
    straty.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let suma_strat: f64 = straty.iter().sum();
    let top10: f64 = straty.iter().take(10).sum();
    let najgorsze_koszyki: Vec<f64> = straty.iter().take(5).cloned().collect();
    let dni_str = dni.values().filter(|v| **v < 0.0).count();
    let dni_wsz = dni.len();

    let dodatnie = dni.values().filter(|v| **v > 0.0).count();
    let zysk: f64 = przyjete
        .iter()
        .map(|i| plany[*i].wynik)
        .filter(|v| *v > 0.0)
        .sum();
    let strata: f64 = przyjete
        .iter()
        .map(|i| plany[*i].wynik)
        .filter(|v| *v < 0.0)
        .sum::<f64>()
        .abs();

    WynikPortfela {
        pnl: zreal,
        maxdd,
        maxdd_zamkniete: mdd_z,
        przyjetych: przyjete.len(),
        odrzuconych,
        max_jednoczesnie: max_jedn,
        sr_jednoczesnie: suma_jedn / n_jedn.max(1.0),
        min_equity,
        zerowan,
        dni_plus: if dni.is_empty() {
            0.0
        } else {
            dodatnie as f64 / dni.len() as f64 * 100.0
        },
        pf: if strata > 1e-9 { zysk / strata } else { 999.0 },
        epizody,
        min_skum_zam: min_skum,
        dni_str,
        dni_wsz,
        najgorszy_dzien,
        najdluzsza_seria: najdl,
        najgorsza_seria,
        suma_strat,
        udzial_10_najgorszych: top10 / suma_strat.min(-1e-9) * 100.0,
        najgorsze_koszyki,
    }
}

// ============================================================
//  MAIN
// ============================================================

#[allow(clippy::too_many_lines)]
fn main() -> Result<()> {
    let a = parse()?;
    let td = TickData::open(&a.ticks)?;
    let syg = load_signals(&a.signals)?;
    println!("dane: {} ticków · {} sygnałów", td.len(), syg.len());

    let cfg = GenCfg {
        krok_s: a.krok_s,
        horyzont_h: a.horyzont_h,
        rozgrzewka_min: 120,
        msg_offset_ms: (a.msg_offset_min * 60_000.0) as i64,
        jednostki: a.jednostki,
        deep_off: 3.0,
        tol_off: -2.0,
        sl_min_dist: 3.0,
        n_r: a.n_r,
        sesja: None,
        placebo_h: a.placebo_h,
    };
    let t0 = std::time::Instant::now();
    let mut sc = generuj(&td, &syg, &cfg);
    if let Some(od) = &a.od {
        let d = dzien_z_daty(od)?;
        sc.retain(|s| s.dzien >= d);
    }
    println!(
        "ścieżki: {} koszyków ({:.1} s) · horyzont {} h{} · okres {} … {}",
        sc.len(),
        t0.elapsed().as_secs_f64(),
        a.horyzont_h,
        if a.placebo_h != 0 {
            format!(" · PLACEBO +{} h", a.placebo_h)
        } else {
            String::new()
        },
        sc.first().map(|s| data_ts(s.ts0())).unwrap_or_default(),
        sc.last().map(|s| data_ts(s.ts0())).unwrap_or_default()
    );
    if sc.len() < 100 {
        bail!("za mało ścieżek");
    }

    // ==================================================================
    //  1. CZYM SA TE 1816-3697 $ - ROZSTRZYGNIECIE
    // ==================================================================
    println!("\n╔════════════════════════════════╗");
    println!("║ 1. CZYM NAPRAWDE JEST „maxDD 1816-3697 $” — rozstrzygniecie");
    println!("╚════════════════════════════════╝");
    println!(
        "  Model NIE SYMULUJE KONTA: lot staly 0,01 na warstwe, bez salda, marginesu\n\
         \x20 i skalowania kapitalem. „maxDD” byl wiec ROZPIETOSCIA SKUMULOWANEJ KRZYWEJ P&L,\n\
         \x20 a nie obsunieciem rachunku. Rozstrzyga to jedna liczba: najnizszy punkt tej\n\
         \x20 krzywej. Jesli nie schodzi ponizej -{:.0} $, konta nie da sie wyzerowac.",
        a.kapital
    );
    println!(
        "\n  {:<28} {:>10} {:>15} {:>15} {:>12} {:>12} {:>12}",
        "polityka",
        "PnL $",
        "min krzywej $",
        "min equity $",
        "maxDD stary",
        "maxDD m2m",
        "konto do zera"
    );
    for pol in [Pol::Tp1, Pol::Czempion, Pol::BezCelu] {
        let pl: Vec<Plan> = sc.iter().map(|s| plan(s, pol, 0)).collect();
        let w = symuluj(&pl, 0, a.kapital);
        println!(
            "  {:<28} {:>+10.2} {:>15.2} {:>15.2} {:>12.2} {:>12.2} {:>12}",
            pol.nazwa(),
            w.pnl,
            w.min_skum_zam,
            w.min_equity,
            w.maxdd_zamkniete,
            w.maxdd,
            if w.zerowan > 0 {
                format!("TAK x{}", w.zerowan)
            } else {
                "NIE".into()
            }
        );
    }
    println!(
        "\n  „min krzywej” = najnizszy punkt skumulowanego P&L po ZAMKNIETYCH (tak liczylismy dotad).\n\
         \x20 „min equity” = to samo, ale z otwartymi pozycjami wycenianymi co minute — czyli to,\n\
         \x20 co widzi broker. Dopiero ta druga liczba mowi, czy rachunek przezyje."
    );

    // ==================================================================
    //  2. ROZBICIE STRAT - to jest teraz kryterium
    // ==================================================================
    println!("\n\n╔════════════════════════════════╗");
    println!("║ 2. ROZBICIE STRAT — dni tracace, ich glebokosc, serie i koncentracja");
    println!("╚════════════════════════════════╝");
    println!(
        "\n  {:<28} {:>10} {:>9} {:>9} {:>13} {:>8} {:>13} {:>10} {:>9}",
        "polityka",
        "PnL $",
        "dni str.",
        "dni tot.",
        "najg. dzien",
        "seria",
        "seria str. $",
        "suma str.",
        "10 najg."
    );
    for pol in [Pol::Tp1, Pol::Czempion, Pol::BezCelu] {
        let pl: Vec<Plan> = sc.iter().map(|s| plan(s, pol, 0)).collect();
        let w = symuluj(&pl, 0, a.kapital);
        println!(
            "  {:<28} {:>+10.2} {:>8.1} % {:>9} {:>+13.2} {:>8} {:>+13.2} {:>+10.2} {:>8.0} %",
            pol.nazwa(),
            w.pnl,
            w.dni_str as f64 / w.dni_wsz.max(1) as f64 * 100.0,
            w.dni_wsz,
            w.najgorszy_dzien,
            w.najdluzsza_seria,
            w.najgorsza_seria,
            w.suma_strat,
            w.udzial_10_najgorszych
        );
    }
    for pol in [Pol::Czempion, Pol::BezCelu] {
        let pl: Vec<Plan> = sc.iter().map(|s| plan(s, pol, 0)).collect();
        let w = symuluj(&pl, 0, a.kapital);
        println!(
            "\n  {} — piec najgorszych pojedynczych koszykow: {}",
            pol.nazwa(),
            w.najgorsze_koszyki
                .iter()
                .map(|v| format!("{v:+.2} $"))
                .collect::<Vec<_>>()
                .join(" · ")
        );
        println!(
            "  dziesiec najgorszych koszykow to {:.0} % wszystkich strat ({:+.2} $ z {:+.2} $) — {}",
            w.udzial_10_najgorszych,
            w.suma_strat * w.udzial_10_najgorszych / 100.0,
            w.suma_strat,
            if w.udzial_10_najgorszych > 40.0 {
                "straty SKONCENTROWANE, warto szukac ich wspolnej cechy"
            } else {
                "straty ROZPROSZONE, to staly koszt, nie zdarzenie"
            }
        );
    }

    // ==================================================================
    //  3. LIMIT POZYCJI - czy WIECEJ zysku przy tym samym marginesie
    // ==================================================================
    println!("\n\n╔════════════════════════════════╗");
    println!("║ 3. LIMIT JEDNOCZESNYCH KOSZYKOW — poprzeczka dla decyzji portfelowej (3.6)");
    println!("╚════════════════════════════════╝");
    println!(
        "  Limit NIE jest hamulcem, tylko sposobem na wiecej zysku przy tym samym\n\
         \x20 marginesie. Zanim model dostanie role portfelowa, musi pobic STALY limit."
    );
    println!(
        "\n  {:<28} {:>7} {:>10} {:>9} {:>13} {:>13} {:>9} {:>9}",
        "polityka",
        "limit",
        "PnL $",
        "dni str.",
        "najg. dzien",
        "min equity",
        "przyjete",
        "do zera"
    );
    for pol in [Pol::Tp1, Pol::Czempion, Pol::BezCelu] {
        for limit in [0usize, 9, 5, 3, 2, 1] {
            let pl: Vec<Plan> = sc.iter().map(|s| plan(s, pol, 0)).collect();
            let w = symuluj(&pl, limit, a.kapital);
            println!(
                "  {:<28} {:>7} {:>+10.2} {:>8.1} % {:>+13.2} {:>13.2} {:>9} {:>9}",
                pol.nazwa(),
                if limit == 0 {
                    "bez".to_string()
                } else {
                    limit.to_string()
                },
                w.pnl,
                w.dni_str as f64 / w.dni_wsz.max(1) as f64 * 100.0,
                w.najgorszy_dzien,
                w.min_equity,
                w.przyjetych,
                if w.zerowan > 0 {
                    format!("TAK x{}", w.zerowan)
                } else {
                    "nie".into()
                }
            );
        }
    }

    // ==================================================================
    //  4. TWARDY CZAS ZYCIA - oceniany stratami
    // ==================================================================
    println!("\n\n╔════════════════════════════════╗");
    println!("║ 4. TWARDY CZAS ZYCIA — oceniany stratami, nie obsunieciem");
    println!("╚════════════════════════════════╝");
    println!(
        "\n  {:<28} {:>8} {:>10} {:>9} {:>13} {:>13} {:>12} {:>9}",
        "polityka",
        "zycie min",
        "PnL $",
        "dni str.",
        "najg. dzien",
        "min equity",
        "maxDD m2m",
        "do zera"
    );
    for pol in [Pol::Tp1, Pol::Czempion, Pol::BezCelu] {
        for zycie in [0i64, 2880, 1440, 720, 360, 240, 120, 90, 60, 30, 15] {
            let pl: Vec<Plan> = sc.iter().map(|s| plan(s, pol, zycie)).collect();
            let w = symuluj(&pl, 0, a.kapital);
            println!(
                "  {:<28} {:>8} {:>+10.2} {:>8.1} % {:>+13.2} {:>13.2} {:>12.2} {:>9}",
                pol.nazwa(),
                if zycie == 0 {
                    "bez".to_string()
                } else {
                    zycie.to_string()
                },
                w.pnl,
                w.dni_str as f64 / w.dni_wsz.max(1) as f64 * 100.0,
                w.najgorszy_dzien,
                w.min_equity,
                w.maxdd,
                if w.zerowan > 0 {
                    format!("TAK x{}", w.zerowan)
                } else {
                    "nie".into()
                }
            );
        }
    }

    for pol in [Pol::BezCelu] {
        let pl: Vec<Plan> = sc.iter().map(|s| plan(s, pol, 0)).collect();
        let w = symuluj(&pl, 0, a.kapital);
        println!(
            "\n  -- INFORMACYJNIE: piec najwiekszych epizodow obsuniecia - {} - jednoczesnie otwartych maks. {}, srednio {:.1} --",
            pol.nazwa(),
            w.max_jednoczesnie,
            w.sr_jednoczesnie
        );
        println!(
            "  {:<18} {:<18} {:>9} {:>7} {:>10} {:>12} {:>12}",
            "od (szczyt)", "do (dolek)", "gleb. $", "godz.", "koszykow", "najgorszy $", "otwarte $"
        );
        for e in &w.epizody {
            println!(
                "  {:<18} {:<18} {:>9.2} {:>7.1} {:>10} {:>12.2} {:>12.2}",
                data_ts(e.szczyt_ts),
                data_ts(e.dolek_ts),
                e.glebokosc,
                (e.dolek_ts - e.szczyt_ts) as f64 / 3_600_000.0,
                e.otwartych_w_dolku,
                e.najgorszy_w_dolku,
                e.otwarte_w_dolku
            );
        }
    }

    println!(
        "\nUWAGA: pomiar NA ŚCIEŻKACH. Lot stały 0,01 na warstwę, bez skalowania kapitałem,\n\
         bez opóźnienia wykonania. Equity liczone mark-to-market co minutę — to jest\n\
         właściwa miara obsunięcia i różni się od dotychczasowej wielokrotnie."
    );
    Ok(())
}

// ============================================================
//  TESTY
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(ts: Ts, w: f32) -> Probka {
        Probka {
            ts,
            x: vec![0.0; F],
            y: [0.0; 3],
            y_kl: 0.0,
            szczyt_od_teraz: 0.0,
            wych: w,
            wych_run: w,
            ryzyko: 6.0,
            otw: 1,
            sciezka: 0,
        }
    }

    fn sc(probki: Vec<Probka>, tp1: Option<f32>, ts_tp1: Option<Ts>, koniec: f32) -> Sciezka {
        Sciezka {
            sygnal: 0,
            side_buy: true,
            id_sygnalu: 1,
            dzien: probki[0].ts.div_euclid(86_400_000),
            probki,
            wych_tp1: tp1,
            ts_tp1,
            wych_koniec: koniec,
            wych_runner: koniec,
            szczyt: 0.0,
            ryzyko_wej: 6.0,
            powod: Powod::Horyzont,
            rf: vec![0.0; RF_WARIANTY.len()],
            rf_ok: vec![false; RF_WARIANTY.len()],
            wypelnionych: 1,
            ts_wyp: vec![0],
        }
    }

    const M: Ts = 60_000;

    #[test]
    fn obsuniecie_widzi_otwarta_pozycje_pod_woda() {
        // jeden koszyk: schodzi do −100 $, wraca na +10 i tam się zamyka.
        // Rachunek po ZAMKNIĘTYCH nie zobaczy obsunięcia w ogóle.
        let s = sc(
            vec![pr(0, 0.0), pr(M, -100.0), pr(2 * M, -50.0), pr(3 * M, 10.0)],
            None,
            None,
            10.0,
        );
        let pl = vec![plan(&s, Pol::BezCelu, 0)];
        let w = symuluj(&pl, 0, 200.0);
        assert!((w.pnl - 10.0).abs() < 1e-6, "PnL {}", w.pnl);
        assert!(
            w.maxdd >= 100.0,
            "prawdziwe obsunięcie musi widzieć −100: {}",
            w.maxdd
        );
        assert!(
            w.maxdd_zamkniete < 1e-6,
            "rachunek po zamkniętych nie widzi nic: {}",
            w.maxdd_zamkniete
        );
        assert!((w.min_equity + 100.0).abs() < 1e-6);
    }

    #[test]
    fn limit_jednoczesnych_odrzuca_nadmiarowe() {
        // trzy koszyki startujące w tej samej chwili, limit 2 → jeden odrzucony
        let a = sc(vec![pr(0, 0.0), pr(10 * M, 5.0)], None, None, 5.0);
        let b = sc(vec![pr(0, 0.0), pr(10 * M, 5.0)], None, None, 5.0);
        let c = sc(vec![pr(0, 0.0), pr(10 * M, 5.0)], None, None, 5.0);
        let pl: Vec<Plan> = [a, b, c].iter().map(|s| plan(s, Pol::BezCelu, 0)).collect();
        let w = symuluj(&pl, 2, 200.0);
        assert_eq!(w.przyjetych, 2);
        assert_eq!(w.odrzuconych, 1);
        assert!((w.pnl - 10.0).abs() < 1e-6);
        // bez limitu wchodzą wszystkie trzy
        let w2 = symuluj(&pl, 0, 200.0);
        assert_eq!(w2.przyjetych, 3);
        assert!((w2.pnl - 15.0).abs() < 1e-6);
    }

    #[test]
    fn miejsce_zwalnia_sie_po_zamknieciu() {
        // pierwszy zamyka się w 10. minucie, drugi startuje w 20. → limit 1 wystarczy
        let a = sc(vec![pr(0, 0.0), pr(10 * M, 5.0)], None, None, 5.0);
        let b = sc(vec![pr(20 * M, 0.0), pr(30 * M, 7.0)], None, None, 7.0);
        let pl: Vec<Plan> = [a, b].iter().map(|s| plan(s, Pol::BezCelu, 0)).collect();
        let w = symuluj(&pl, 1, 200.0);
        assert_eq!(w.przyjetych, 2, "miejsce musi się zwolnić po zamknięciu");
        assert!((w.pnl - 12.0).abs() < 1e-6);
    }

    #[test]
    fn twardy_czas_zycia_ucina_po_cenie_rynkowej() {
        let s = sc(
            vec![pr(0, 0.0), pr(60 * M, 20.0), pr(180 * M, -30.0)],
            None,
            None,
            -30.0,
        );
        // bez limitu: kończy na −30
        assert!((plan(&s, Pol::BezCelu, 0).wynik + 30.0).abs() < 1e-6);
        // limit 2 h: ostatnia próbka w granicy to 60 min → +20
        let p = plan(&s, Pol::BezCelu, 120);
        assert!((p.wynik - 20.0).abs() < 1e-6, "wynik {}", p.wynik);
        assert_eq!(p.ts_zam, 60 * M);
        // marki nie mogą wychodzić poza chwilę zamknięcia
        assert!(p.marks.iter().all(|(t, _)| *t <= p.ts_zam));
    }

    #[test]
    fn tp1_zamyka_w_chwili_dotkniecia() {
        let s = sc(
            vec![pr(0, 0.0), pr(M, 3.0), pr(5 * M, -40.0)],
            Some(3.0),
            Some(M),
            -40.0,
        );
        let p = plan(&s, Pol::Tp1, 0);
        assert_eq!(p.ts_zam, M);
        assert!((p.wynik - 3.0).abs() < 1e-6);
        // po zamknięciu koszyk nie może już ciągnąć equity w dół
        let w = symuluj(&[p], 0, 200.0);
        assert!(
            w.maxdd < 1e-6,
            "zamknięty koszyk nie obsuwa konta: {}",
            w.maxdd
        );
    }

    #[test]
    fn zerowanie_konta_jest_wykrywane() {
        let s = sc(
            vec![pr(0, 0.0), pr(M, -250.0), pr(2 * M, 5.0)],
            None,
            None,
            5.0,
        );
        let pl = vec![plan(&s, Pol::BezCelu, 0)];
        let w = symuluj(&pl, 0, 200.0);
        assert_eq!(w.zerowan, 1, "equity −250 na koncie 200 $ zeruje rachunek");
    }
}
