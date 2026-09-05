
use anyhow::{bail, Result};
use conduit_ai::peak::*;
use conduit_ai::policy::Scratch;
use conduit_backtest::{load_signals, TickData};

struct Args {
    ticks: String,
    signals: String,
    krok_s: i64,
    horyzont_h: i64,
    epok: usize,
    lr: f32,
    ukryte: Vec<usize>,
    seed: u64,
    horyzont_idx: usize,
    msg_offset_min: f64,
    jednostki: usize,
    n_r: f64,
    sesja: Option<(u32, u32)>,
    folds: usize,
    od: Option<String>,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            ticks: "data/ticks.bin".into(),
            signals: "data/signals.json".into(),
            krok_s: 60,
            horyzont_h: 6,
            epok: 40,
            lr: 0.05,
            ukryte: vec![24, 16],
            seed: 7,
            horyzont_idx: 1, // 60 minut
            msg_offset_min: 180.0,
            jednostki: 3,
            n_r: 3.0,
            sesja: None,
            folds: 0,
            od: None,
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
            "--epok" => a.epok = nast!().parse()?,
            "--lr" => a.lr = nast!().parse()?,
            "--ukryte" => {
                a.ukryte = nast!()
                    .split(',')
                    .filter_map(|x| x.trim().parse().ok())
                    .collect()
            }
            "--seed" => a.seed = nast!().parse()?,
            "--horyzont" => {
                let m: i64 = nast!().parse()?;
                a.horyzont_idx = HORYZONTY.iter().position(|h| *h == m).unwrap_or(1);
            }
            "--msg-offset-min" => a.msg_offset_min = nast!().parse()?,
            "--jednostki" => a.jednostki = nast!().parse()?,
            "--n-r" => a.n_r = nast!().parse()?,
            "--sesja" => {
                let s = nast!();
                let (x, y) = s
                    .split_once('-')
                    .ok_or_else(|| anyhow::anyhow!("--sesja 8-18"))?;
                a.sesja = Some((x.trim().parse()?, y.trim().parse()?));
            }
            "--folds" => a.folds = nast!().parse()?,
            "--od" => a.od = Some(nast!()),
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

fn wiersz(nazwa: &str, w: &Wynik, ci: Option<(f64, f64)>) {
    let pf = if w.pf >= 999.0 {
        "∞".to_string()
    } else {
        format!("{:.2}", w.pf)
    };
    match ci {
        Some((lo, hi)) => println!(
            "  {:<38} {:>+9.2}  {:>6}  {:>6.1} %  {:>6.1} %  {:>8.2}   [{:+.0} … {:+.0}]",
            nazwa, w.pnl, pf, w.trafien, w.dni_plus, w.maxdd, lo, hi
        ),
        None => println!(
            "  {:<38} {:>+9.2}  {:>6}  {:>6.1} %  {:>6.1} %  {:>8.2}",
            nazwa, w.pnl, pf, w.trafien, w.dni_plus, w.maxdd
        ),
    }
}

fn progi_z_kwantyli(mut v: Vec<f32>, ile: usize) -> Vec<f32> {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if v.is_empty() {
        return vec![0.0];
    }
    let mut out: Vec<f32> = (1..ile).map(|k| v[k * v.len() / ile]).collect();
    out.dedup();
    out
}

/// Czy zestaw progów w ogóle coś różnicuje.
fn sprawdz_rozroznialnosc(nazwa: &str, wyniki: &[f64]) {
    let (mn, mx) = wyniki
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
            (a.min(*v), b.max(*v))
        });
    if (mx - mn).abs() < 1e-6 {
        println!(
            "  UWAGA [{nazwa}]: wszystkie progi dają identyczny wynik ({mn:+.2} $) — próg nie zachodzi, pomiar jest pusty"
        );
    }
}

fn naglowek() {
    println!(
        "  {:<38} {:>9}  {:>6}  {:>8}  {:>8}  {:>8}   {}",
        "polityka", "PnL $", "PF", "trafień", "dni +", "maxDD $", "95 % (bootstrap po dniach)"
    );
}

#[allow(clippy::too_many_lines)]
fn fold(
    a: &Args,
    ucz: &[Sciezka],
    stroj: &[Sciezka],
    test: &[Sciezka],
    etykieta: &str,
) -> (f64, f64) {
    let h = a.horyzont_idx;
    let pu = plaskie(ucz);
    let pt = plaskie(test);
    let xu: Vec<Vec<f32>> = pu.iter().map(|p| p.x.clone()).collect();
    let yu: Vec<[f32; 3]> = pu.iter().map(|p| p.y).collect();
    let yu_h: Vec<f32> = pu.iter().map(|p| p.y[h]).collect();
    let yu_k: Vec<f32> = pu.iter().map(|p| p.y_kl).collect();
    let yt_h: Vec<f32> = pt.iter().map(|p| p.y[h]).collect();
    let yt_k: Vec<f32> = pt.iter().map(|p| p.y_kl).collect();

    println!("\n══════════ {etykieta} ══════════");
    println!(
        "  uczenie {} ścieżek / {} próbek · strojenie {} / {} · TEST {} / {}",
        ucz.len(),
        pu.len(),
        stroj.len(),
        plaskie(stroj).len(),
        test.len(),
        pt.len()
    );
    let udz_ucz = yu_k.iter().sum::<f32>() / yu_k.len().max(1) as f32 * 100.0;
    let udz_test = yt_k.iter().sum::<f32>() / yt_k.len().max(1) as f32 * 100.0;
    // Udział liczony PO PRÓBKACH jest zawyżony: ścieżka, która biegnie 6 h, daje
    // 360 próbek, a ta, która ginie na SL po 5 min — pięć. Decyzja hybrydy
    // zapada raz, przy wejściu, więc to udział PO ŚCIEŻKACH jest miarą rzadkości.
    let po_sc = |v: &[Sciezka]| {
        v.iter().filter(|z| z.probki[0].y_kl > 0.5).count() as f64 / v.len().max(1) as f64 * 100.0
    };
    println!(
        "  BIEGACZE (szczyt od teraz ≥ {:.1} × ryzyko): po próbkach uczenie {:.1} % / test {:.1} %  ·  \
         PO ŚCIEŻKACH (przy wejściu) uczenie {:.1} % / test {:.1} %",
        a.n_r,
        udz_ucz,
        udz_test,
        po_sc(ucz),
        po_sc(test)
    );

    // ---------- GŁOWA REGRESYJNA ----------
    let ridge = Ridge::ucz(&xu, &yu_h, 1.0);
    let pr_r: Vec<f32> = pt.iter().map(|p| ridge.pred(&p.x)).collect();
    let (m_r, sk) = ucz_siec(&xu, &yu, &a.ukryte, a.epok, a.lr, a.seed);
    let mut s1 = Scratch::for_net(&m_r);
    let pr_n: Vec<f32> = pt
        .iter()
        .map(|p| pred_siec(&m_r, &sk, &mut s1, &p.x, h))
        .collect();

    println!(
        "\n── 1. GŁOWA REGRESYJNA: zapas do szczytu w oknie {} min (TEST) ──",
        HORYZONTY[h]
    );
    println!(
        "  liniowy (ridge)   R² {:+.4}   korelacja {:+.4}",
        r2(&pr_r, &yt_h),
        korelacja(&pr_r, &yt_h)
    );
    println!(
        "  sieć {:?}       R² {:+.4}   korelacja {:+.4}",
        a.ukryte,
        r2(&pr_n, &yt_h),
        korelacja(&pr_n, &yt_h)
    );
    if r2(&pr_n, &yt_h) <= r2(&pr_r, &yt_h) {
        println!("  → sieć NIE bije liniowego poza próbą: przeuczenie, nie głębsza zależność");
    }

    // ---------- GŁOWA KLASYFIKACYJNA ----------
    let lin_k = RegLog::ucz(&xu, &yu_k, 300, 3.0, 1e-4);
    let pk_l: Vec<f32> = pt.iter().map(|p| lin_k.pred(&p.x)).collect();
    let m_k = ucz_klas(&xu, &yu_k, &a.ukryte, a.epok, a.lr * 4.0, a.seed + 1);
    let mut s2 = Scratch::for_net(&m_k);
    let pk_n: Vec<f32> = pt.iter().map(|p| pred_klas(&m_k, &mut s2, &p.x)).collect();

    let auc_l = auc(&pk_l, &yt_k);
    let auc_n = auc(&pk_n, &yt_k);
    println!(
        "\n── 2. GŁOWA KLASYFIKACYJNA: czy pobiegnie ≥ {:.1} R (TEST) ──",
        a.n_r
    );
    println!("  liniowy (logistyczny)   AUC {auc_l:.4}");
    println!("  sieć {:?}             AUC {auc_n:.4}", a.ukryte);
    if auc_n <= auc_l {
        println!("  → sieć NIE bije liniowego poza próbą");
    }

    // decyle predykcji klasyfikatora — czy w górnym decylu faktycznie biegną
    let mut pary: Vec<(f32, f32)> = pk_n.iter().cloned().zip(yt_k.iter().cloned()).collect();
    pary.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
    let d = (pary.len() / 10).max(1);
    let baza = yt_k.iter().sum::<f32>() as f64 / yt_k.len().max(1) as f64;
    println!(
        "  decyl predykcji → udział biegaczy (baza {:.1} %):",
        baza * 100.0
    );
    for k in [0usize, 4, 8, 9] {
        let s = &pary[k * d..((k + 1) * d).min(pary.len())];
        let rr = s.iter().map(|v| v.1 as f64).sum::<f64>() / s.len() as f64;
        println!(
            "    decyl {:>2}: {:5.1} %   (lift × {:.2})",
            k + 1,
            rr * 100.0,
            rr / baza.max(1e-9)
        );
    }

    // ---------- DOLARY ----------
    println!(
        "\n── 3. DOLARY NA TEŚCIE (0,01 lota na jednostkę, {} jednostek) ──",
        a.jednostki
    );

    // odniesienia stałe
    let w_tp1 = podsumuj(&pol_tp1(test));
    let w_bez = podsumuj(&pol_bez_celu(test));
    let w_run = podsumuj(&pol_runner(test));
    let w_sufit = podsumuj(&pol_szczyt(test));
    let w_wyr = podsumuj(&pol_wyrocznia_hybryda(test));

    // zapadka i limit czasu — parametr strojony na CZĘŚCI ŚRODKOWEJ
    let mut naj_zap = (0.0f32, 0.0f32, f64::NEG_INFINITY);
    for si in 0..6 {
        for pi in 0..7 {
            let st = 1.0 + si as f32 * 3.0;
            let pc = 0.2 + pi as f32 * 0.1;
            let p = podsumuj(&pol_zapadka(stroj, st, pc)).pnl;
            if p > naj_zap.2 {
                naj_zap = (st, pc, p);
            }
        }
    }
    let w_zap = podsumuj(&pol_zapadka(test, naj_zap.0, naj_zap.1));
    let mut naj_czas = (0i64, f64::NEG_INFINITY);
    for m in [15i64, 30, 60, 120, 180, 240, 360] {
        let p = podsumuj(&pol_czas(stroj, m)).pnl;
        if p > naj_czas.1 {
            naj_czas = (m, p);
        }
    }
    let w_czas = podsumuj(&pol_czas(test, naj_czas.0));

    // REGRESJA: próg zapasu strojony na części środkowej, KWANTYLAMI predykcji
    let ps = plaskie(stroj);
    let pred_stroj_reg: Vec<f32> = {
        let mut s = Scratch::for_net(&m_r);
        ps.iter()
            .map(|p| pred_siec(&m_r, &sk, &mut s, &p.x, h))
            .collect()
    };
    let kand_reg = progi_z_kwantyli(pred_stroj_reg.clone(), 20);
    println!(
        "\n  progi regresji z kwantyli predykcji (decyl 1 = {:.2} $, mediana = {:.2} $, decyl 9 = {:.2} $)",
        kand_reg[kand_reg.len() / 10],
        kand_reg[kand_reg.len() / 2],
        kand_reg[kand_reg.len() * 9 / 10]
    );
    let mut naj_reg = (0.0f32, f64::NEG_INFINITY);
    let mut wyniki_reg = Vec::new();
    for prog in &kand_reg {
        let mut s = Scratch::for_net(&m_r);
        let p = podsumuj(&pol_regresja(
            stroj,
            |q| pred_siec(&m_r, &sk, &mut s, &q.x, h),
            *prog,
        ))
        .pnl;
        wyniki_reg.push(p);
        if p > naj_reg.1 {
            naj_reg = (*prog, p);
        }
    }
    sprawdz_rozroznialnosc("regresja", &wyniki_reg);
    let mut s3 = Scratch::for_net(&m_r);
    let reg_test = pol_regresja(test, |q| pred_siec(&m_r, &sk, &mut s3, &q.x, h), naj_reg.0);
    let w_reg = podsumuj(&reg_test);
    let reg_lin_test = pol_regresja(test, |q| ridge.pred(&q.x), naj_reg.0);
    let w_reg_lin = podsumuj(&reg_lin_test);
    // ile ścieżek próg REGRESJI faktycznie zamyka wcześniej — bez tej liczby
    // nie da się odróżnić polityki od „nigdy nie wychodź"
    let mut s3b = Scratch::for_net(&m_r);
    let zamknietych = test
        .iter()
        .filter(|z| {
            z.probki
                .iter()
                .any(|p| pred_siec(&m_r, &sk, &mut s3b, &p.x, h) < naj_reg.0)
        })
        .count();

    // HYBRYDA: próg prawdopodobieństwa z KWANTYLI predykcji na PIERWSZYCH
    // próbkach ścieżek strojących — decyzja zapada przy wejściu, więc rozkład
    // musi być liczony na tym samym zbiorze, na którym próg będzie działał.
    let pred_stroj_kl: Vec<f32> = {
        let mut s = Scratch::for_net(&m_k);
        stroj
            .iter()
            .map(|z| pred_klas(&m_k, &mut s, &z.probki[0].x))
            .collect()
    };
    let kand_hyb = progi_z_kwantyli(pred_stroj_kl.clone(), 20);
    let mut naj_hyb = (0.0f32, f64::NEG_INFINITY);
    let mut wyniki_hyb = Vec::new();
    for prog in &kand_hyb {
        let mut s = Scratch::for_net(&m_k);
        let p = podsumuj(&pol_hybryda(
            stroj,
            |q| pred_klas(&m_k, &mut s, &q.x),
            *prog,
        ))
        .pnl;
        wyniki_hyb.push(p);
        if p > naj_hyb.1 {
            naj_hyb = (*prog, p);
        }
    }
    sprawdz_rozroznialnosc("hybryda", &wyniki_hyb);
    let mut s4 = Scratch::for_net(&m_k);
    let hyb_test = pol_hybryda(test, |q| pred_klas(&m_k, &mut s4, &q.x), naj_hyb.0);
    let w_hyb = podsumuj(&hyb_test);
    // ile ścieżek model faktycznie PUSZCZA — bez tej liczby nie da się odróżnić
    // modelu od zdegenerowanego „trzymaj wszystko" albo „nic nie trzymaj"
    let mut s5 = Scratch::for_net(&m_k);
    let trzymanych = test
        .iter()
        .filter(|s| pred_klas(&m_k, &mut s5, &s.probki[0].x) >= naj_hyb.0)
        .count();
    // hybryda na modelu LINIOWYM — odniesienie, ten sam sposób doboru progu
    let kand_hyb_l = progi_z_kwantyli(
        stroj.iter().map(|z| lin_k.pred(&z.probki[0].x)).collect(),
        20,
    );
    let mut naj_hyb_l = (0.0f32, f64::NEG_INFINITY);
    for prog in &kand_hyb_l {
        let p = podsumuj(&pol_hybryda(stroj, |q| lin_k.pred(&q.x), *prog)).pnl;
        if p > naj_hyb_l.1 {
            naj_hyb_l = (*prog, p);
        }
    }
    let hyb_lin_test = pol_hybryda(test, |q| lin_k.pred(&q.x), naj_hyb_l.0);
    let w_hyb_lin = podsumuj(&hyb_lin_test);
    // hybryda ciągła
    let mut s6 = Scratch::for_net(&m_k);
    let hyb_c_test = pol_hybryda_ciagla(test, |q| pred_klas(&m_k, &mut s6, &q.x), naj_hyb.0);
    let w_hyb_c = podsumuj(&hyb_c_test);

    let tp1_test = pol_tp1(test);
    let run_test = pol_runner(test);
    let ci = |v: &Vec<(i64, f64)>| Some(bootstrap_dni(v, 4000, a.seed));

    naglowek();
    wiersz("SUFIT: wyjście w szczycie", &w_sufit, None);
    wiersz("SUFIT hybrydy (wie, co puścić)", &w_wyr, None);
    println!("  {:-<38}", "");
    wiersz("ODNIESIENIE: wszystko na TP1", &w_tp1, ci(&tp1_test));
    wiersz(
        "ODNIESIENIE: nic bez celu (do SL)",
        &w_bez,
        ci(&pol_bez_celu(test)),
    );
    wiersz(
        "ODNIESIENIE: czempion (1 × TP1 + runnery)",
        &w_run,
        ci(&run_test),
    );
    wiersz(
        &format!(
            "zapadka stała ({:.0} $ / {:.0} %)",
            naj_zap.0,
            naj_zap.1 * 100.0
        ),
        &w_zap,
        None,
    );
    wiersz(&format!("limit czasu {} min", naj_czas.0), &w_czas, None);
    println!("  {:-<38}", "");
    wiersz(
        &format!("REGRESJA liniowa, próg {:.1}", naj_reg.0),
        &w_reg_lin,
        None,
    );
    wiersz(
        &format!(
            "REGRESJA sieć, próg {:.2} ({}/{} zamkn.)",
            naj_reg.0,
            zamknietych,
            test.len()
        ),
        &w_reg,
        ci(&reg_test),
    );
    wiersz(
        &format!("HYBRYDA liniowa, próg {:.2}", naj_hyb_l.0),
        &w_hyb_lin,
        None,
    );
    wiersz(
        &format!(
            "HYBRYDA sieć, próg {:.2} ({} / {} puszczonych)",
            naj_hyb.0,
            trzymanych,
            test.len()
        ),
        &w_hyb,
        ci(&hyb_test),
    );
    wiersz("HYBRYDA sieć, decyzja ciągła", &w_hyb_c, ci(&hyb_c_test));

    // ---------- RÓŻNICE SPAROWANE ----------
    println!("\n── 4. CZY MODEL DOKŁADA (różnica sparowana, bootstrap po dniach) ──");
    let poroj = |nazwa: &str, a_: &Vec<(i64, f64)>, b_: &Vec<(i64, f64)>| {
        let d: f64 = a_.iter().zip(b_).map(|(x, y)| x.1 - y.1).sum();
        let (lo, hi) = bootstrap_roznicy(a_, b_, 4000, a.seed);
        let werdykt = if lo > 0.0 {
            "DODAJE"
        } else if hi < 0.0 {
            "SZKODZI"
        } else {
            "nierozstrzygnięte"
        };
        println!(
            "  {:<44} {:>+9.2} $   [{:+.0} … {:+.0}]  {}",
            nazwa, d, lo, hi, werdykt
        );
    };
    let bez_test = pol_bez_celu(test);
    poroj("regresja sieć − wszystko na TP1", &reg_test, &tp1_test);
    poroj("regresja sieć − nic bez celu", &reg_test, &bez_test);
    poroj("hybryda sieć − wszystko na TP1", &hyb_test, &tp1_test);
    poroj(
        "hybryda sieć − NIC BEZ CELU (najmocniejsze odniesienie)",
        &hyb_test,
        &bez_test,
    );
    poroj(
        "hybryda sieć − czempion (1×TP1 + runnery)",
        &hyb_test,
        &run_test,
    );
    poroj("hybryda sieć − hybryda liniowa", &hyb_test, &hyb_lin_test);
    poroj("hybryda ciągła − NIC BEZ CELU", &hyb_c_test, &bez_test);
    poroj("hybryda ciągła − wszystko na TP1", &hyb_c_test, &tp1_test);

    // ---------- KONCENTRACJA ----------
    // Sprawdzamy koncentrację dodatniej luki wyjścia i to, czy klasyfikator
    // rozpoznaje nieliczne, ponadprzeciętnie długie ruchy.
    let mut luki: Vec<(usize, f64)> = test
        .iter()
        .enumerate()
        .map(|(i, s)| (i, (s.szczyt - s.tp1()) as f64))
        .collect();
    luki.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap());
    let suma_luk: f64 = luki.iter().map(|v| v.1.max(0.0)).sum();
    let top10: f64 = luki.iter().take(10).map(|v| v.1.max(0.0)).sum();
    let mut s7 = Scratch::for_net(&m_k);
    let mut oceny: Vec<(usize, f32)> = test
        .iter()
        .enumerate()
        .map(|(i, s)| (i, pred_klas(&m_k, &mut s7, &s.probki[0].x)))
        .collect();
    oceny.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap());
    let gorny_decyl: std::collections::HashSet<usize> = oceny
        .iter()
        .take((test.len() / 10).max(1))
        .map(|v| v.0)
        .collect();
    let zlapane = luki
        .iter()
        .take(10)
        .filter(|v| gorny_decyl.contains(&v.0))
        .count();
    let luka_w_decylu: f64 = luki
        .iter()
        .filter(|v| gorny_decyl.contains(&v.0))
        .map(|v| v.1.max(0.0))
        .sum();
    println!("\n── 5. KONCENTRACJA LUKI WYJŚCIA (szczyt − TP1) ──");
    println!(
        "  {} ścieżek testowych, łączna luka {:.0} $; 10 największych = {:.0} $ ({:.0} %)",
        test.len(),
        suma_luk,
        top10,
        top10 / suma_luk.max(1e-9) * 100.0
    );
    println!(
        "  górny decyl klasyfikatora ({} ścieżek) trzyma {:.0} $ luki ({:.0} %) i łapie {}/10 największych",
        gorny_decyl.len(),
        luka_w_decylu,
        luka_w_decylu / suma_luk.max(1e-9) * 100.0,
        zlapane
    );

    (w_hyb.pnl, w_run.pnl)
}

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
        sesja: a.sesja,
        // `peak` mierzy modele, nie przewagę — kontrola placebo należy do
        // `portfel --placebo-h`, tutaj przesunięcie jest zawsze wyłączone
        placebo_h: 0,
    };
    let t0 = std::time::Instant::now();
    let mut sc = generuj(&td, &syg, &cfg);
    if let Some(od) = &a.od {
        let d = dzien_z_daty(od)?;
        sc.retain(|s| s.dzien >= d);
    }
    let n_prob: usize = sc.iter().map(|s| s.probki.len()).sum();
    let sl = sc.iter().filter(|s| s.powod == Powod::Sl).count();
    println!(
        "ścieżki: {} koszyków · {} próbek ({:.1} s) · {} zakończonych na SL ({:.0} %)",
        sc.len(),
        n_prob,
        t0.elapsed().as_secs_f64(),
        sl,
        sl as f64 / sc.len().max(1) as f64 * 100.0
    );
    if sc.len() < 100 {
        bail!("za mało ścieżek — sprawdź zegar wiadomości i zakres ticków");
    }

    if a.folds == 0 {
        let (ka, kb) = podziel_sciezki(&sc, 0.50, 0.25);
        fold(
            &a,
            &sc[..ka],
            &sc[ka..kb],
            &sc[kb..],
            "PODZIAŁ 50 / 25 / 25",
        );
    } else {
        // walk-forward z oknem rozszerzającym: uczenie rośnie, test przesuwa się
        let nbl = a.folds + 2;
        let blok = sc.len() / nbl;
        let mut suma = (0.0, 0.0);
        for k in 0..a.folds {
            let ucz = &sc[..(k + 1) * blok];
            let stroj = &sc[(k + 1) * blok..(k + 2) * blok];
            let test = if k + 3 >= nbl {
                &sc[(k + 2) * blok..]
            } else {
                &sc[(k + 2) * blok..(k + 3) * blok]
            };
            let (m, r) = fold(
                &a,
                ucz,
                stroj,
                test,
                &format!("WALK-FORWARD {}/{}", k + 1, a.folds),
            );
            suma.0 += m;
            suma.1 += r;
        }
        println!("\n══════════ SUMA WALK-FORWARD ══════════");
        println!(
            "  hybryda sieć {:+.2} $ · czempion (1×TP1 + runnery) {:+.2} $",
            suma.0, suma.1
        );
    }

    println!(
        "\nUWAGA: to jest pomiar NA ŚCIEŻKACH, nie w silniku. Brak limitu jednoczesnych\n\
         pozycji, brak zarządzania kapitałem, brak opóźnienia wykonania, lot stały.\n\
         Liczby są porównywalne MIĘDZY SOBĄ (te same ścieżki, ten sam wolumen), ale nie\n\
         są porównywalne z wynikiem presetu w silniku."
    );
    Ok(())
}
