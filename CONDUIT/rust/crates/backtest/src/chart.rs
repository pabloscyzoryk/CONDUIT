//! Wykresy wyników jako SVG — bez zależności zewnętrznych.
//!
//! Po każdej symulacji generujemy jeden plik: krzywa kapitału, obsunięcie
//! pod spodem i słupki dzienne. Kolorystyka zgodna z interfejsem aplikacji.

use crate::metrics::{DayStat, Metrics};
use conduit_core::types::Ts;
use std::fmt::Write;

const BG: &str = "#0b0e14";
const SURF: &str = "#111520";
const GRID: &str = "#1c2231";
const TEXT: &str = "#e8ecf5";
const DIM: &str = "#8590a8";
const UP: &str = "#26d9a3";
const DOWN: &str = "#ff5c7a";
const ACC: &str = "#6d7bff";

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Wykres pojedynczego przebiegu.
pub fn run_chart(title: &str, m: &Metrics, curve: &[(Ts, f64)], daily: &[DayStat]) -> String {
    let w = 1200.0;
    let h = 760.0;
    let pad_l = 70.0;
    let pad_r = 24.0;
    let head = 118.0;

    let eq_h = 300.0;
    let dd_h = 110.0;
    let day_h = 150.0;
    let gap = 34.0;

    let pw = w - pad_l - pad_r;
    let mut s = String::with_capacity(64 * 1024);

    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="Inter,Segoe UI,system-ui,sans-serif">
<rect width="{w}" height="{h}" fill="{BG}"/>"#
    );

    // ---------- nagłówek ----------
    let _ = write!(
        s,
        r#"<text x="{pad_l}" y="34" fill="{TEXT}" font-size="19" font-weight="700">{}</text>"#,
        esc(title)
    );

    let tone = |v: f64| if v >= 0.0 { UP } else { DOWN };
    let cells: Vec<(String, String, &str)> = vec![
        (
            format!(
                "{}{:.2} $",
                if m.total_profit >= 0.0 { "+" } else { "−" },
                m.total_profit.abs()
            ),
            "wynik".into(),
            tone(m.total_profit),
        ),
        (
            format!("{:+.1}%", m.return_pct),
            "zwrot".into(),
            tone(m.return_pct),
        ),
        (
            format!(
                "{}{:.2} $",
                if m.avg_per_day >= 0.0 { "+" } else { "−" },
                m.avg_per_day.abs()
            ),
            "śr. na dzień".into(),
            tone(m.avg_per_day),
        ),
        (format!("−{:.2} $", m.max_dd_abs), "max DD".into(), DOWN),
        (format!("{:.1}%", m.max_dd_pct), "max DD %".into(), DOWN),
        (
            if m.profit_factor.is_finite() {
                format!("{:.2}", m.profit_factor)
            } else {
                "∞".into()
            },
            "profit factor".into(),
            if m.profit_factor >= 1.0 { UP } else { DOWN },
        ),
        (
            format!("{:.0}%", m.win_days_pct),
            "dni na plusie".into(),
            DIM,
        ),
        (format!("{}", m.trades), "transakcji".into(), DIM),
    ];
    let cw = pw / cells.len() as f64;
    for (i, (val, lab, col)) in cells.iter().enumerate() {
        let x = pad_l + i as f64 * cw;
        let _ = write!(
            s,
            r#"<text x="{x:.1}" y="72" fill="{col}" font-size="17" font-weight="700" font-family="ui-monospace,Consolas,monospace">{}</text>
<text x="{x:.1}" y="90" fill="{DIM}" font-size="10.5" letter-spacing="0.06em">{}</text>"#,
            esc(val),
            esc(&lab.to_uppercase())
        );
    }

    // ---------- krzywa kapitału ----------
    let y0 = head;
    let _ = write!(
        s,
        r#"<rect x="{pad_l}" y="{y0}" width="{pw:.1}" height="{eq_h}" fill="{SURF}" rx="8"/>"#
    );
    if curve.len() > 1 {
        let t0 = curve[0].0 as f64;
        let t1 = curve[curve.len() - 1].0 as f64;
        let span_t = (t1 - t0).max(1.0);
        let mut lo = f64::MAX;
        let mut hi = f64::MIN;
        for &(_, e) in curve {
            lo = lo.min(e);
            hi = hi.max(e);
        }
        let pad_v = ((hi - lo) * 0.08).max(1.0);
        lo -= pad_v;
        hi += pad_v;
        let sx = |t: f64| pad_l + (t - t0) / span_t * pw;
        let sy = |e: f64| y0 + (hi - e) / (hi - lo).max(1e-9) * eq_h;

        // siatka + oś wartości
        for k in 0..=4 {
            let v = lo + (hi - lo) * k as f64 / 4.0;
            let y = sy(v);
            let _ = write!(
                s,
                r#"<line x1="{pad_l}" y1="{y:.1}" x2="{:.1}" y2="{y:.1}" stroke="{GRID}"/>
<text x="{:.1}" y="{:.1}" fill="{DIM}" font-size="10" text-anchor="end" font-family="ui-monospace,Consolas,monospace">{v:.0} $</text>"#,
                pad_l + pw,
                pad_l - 8.0,
                y + 3.5
            );
        }
        // linia kapitału startowego
        if m.start_balance > lo && m.start_balance < hi {
            let y = sy(m.start_balance);
            let _ = write!(
                s,
                r#"<line x1="{pad_l}" y1="{y:.1}" x2="{:.1}" y2="{y:.1}" stroke="{DIM}" stroke-dasharray="4 4" opacity="0.6"/>"#,
                pad_l + pw
            );
        }

        let mut pts = String::with_capacity(curve.len() * 16);
        for &(t, e) in curve {
            let _ = write!(pts, "{:.1},{:.1} ", sx(t as f64), sy(e));
        }
        let col = if m.total_profit >= 0.0 { UP } else { DOWN };
        let _ = write!(
            s,
            r#"<polygon points="{pad_l:.1},{:.1} {pts}{:.1},{:.1}" fill="{col}" opacity="0.10"/>
<polyline points="{pts}" fill="none" stroke="{col}" stroke-width="1.8" stroke-linejoin="round"/>"#,
            y0 + eq_h,
            pad_l + pw,
            y0 + eq_h
        );
    }
    let _ = write!(
        s,
        r#"<text x="{:.1}" y="{:.1}" fill="{DIM}" font-size="11" font-weight="600">KAPITAŁ</text>"#,
        pad_l + 10.0,
        y0 + 18.0
    );

    // ---------- obsunięcie ----------
    let y1 = y0 + eq_h + gap;
    let _ = write!(
        s,
        r#"<rect x="{pad_l}" y="{y1}" width="{pw:.1}" height="{dd_h}" fill="{SURF}" rx="8"/>"#
    );
    if curve.len() > 1 {
        let t0 = curve[0].0 as f64;
        let span_t = (curve[curve.len() - 1].0 as f64 - t0).max(1.0);
        let mut peak = curve[0].1;
        let mut maxdd = 0.0f64;
        let dds: Vec<(f64, f64)> = curve
            .iter()
            .map(|&(t, e)| {
                if e > peak {
                    peak = e;
                }
                let d = (peak - e) / peak.max(1.0) * 100.0;
                maxdd = maxdd.max(d);
                (t as f64, d)
            })
            .collect();
        let scale = maxdd.max(1.0);
        let mut pts = String::with_capacity(dds.len() * 16);
        for &(t, d) in &dds {
            let x = pad_l + (t - t0) / span_t * pw;
            let y = y1 + d / scale * (dd_h - 16.0);
            let _ = write!(pts, "{x:.1},{y:.1} ");
        }
        let _ = write!(
            s,
            r#"<polygon points="{pad_l:.1},{y1:.1} {pts}{:.1},{y1:.1}" fill="{DOWN}" opacity="0.22"/>
<polyline points="{pts}" fill="none" stroke="{DOWN}" stroke-width="1.2"/>"#,
            pad_l + pw
        );
    }
    let _ = write!(
        s,
        r#"<text x="{:.1}" y="{:.1}" fill="{DIM}" font-size="11" font-weight="600">OBSUNIĘCIE (%)</text>"#,
        pad_l + 10.0,
        y1 + 18.0
    );

    // ---------- słupki dzienne ----------
    let y2 = y1 + dd_h + gap;
    let _ = write!(
        s,
        r#"<rect x="{pad_l}" y="{y2}" width="{pw:.1}" height="{day_h}" fill="{SURF}" rx="8"/>"#
    );
    let act: Vec<&DayStat> = daily.iter().filter(|d| d.trades > 0).collect();
    if !act.is_empty() {
        let maxa = act.iter().map(|d| d.profit.abs()).fold(1.0, f64::max);
        let bw = (pw / act.len() as f64).min(22.0);
        let mid = y2 + day_h / 2.0;
        let _ = write!(
            s,
            r#"<line x1="{pad_l}" y1="{mid:.1}" x2="{:.1}" y2="{mid:.1}" stroke="{GRID}"/>"#,
            pad_l + pw
        );
        for (i, d) in act.iter().enumerate() {
            let x = pad_l + (i as f64 + 0.5) * (pw / act.len() as f64) - bw * 0.4;
            let hh = (d.profit.abs() / maxa * (day_h / 2.0 - 12.0)).max(0.7);
            let (y, col) = if d.profit >= 0.0 {
                (mid - hh, UP)
            } else {
                (mid, DOWN)
            };
            let _ = write!(
                s,
                r#"<rect x="{x:.1}" y="{y:.1}" width="{:.1}" height="{hh:.1}" fill="{col}" opacity="0.85"><title>{} : {:+.2} $ ({} trejdów)</title></rect>"#,
                bw * 0.8,
                esc(&d.date),
                d.profit,
                d.trades
            );
        }
    }
    let _ = write!(
        s,
        r#"<text x="{:.1}" y="{:.1}" fill="{DIM}" font-size="11" font-weight="600">WYNIK DZIENNY</text>"#,
        pad_l + 10.0,
        y2 + 18.0
    );

    // ---------- stopka ----------
    let _ = write!(
        s,
        r#"<text x="{pad_l}" y="{:.1}" fill="{DIM}" font-size="10.5" font-family="ui-monospace,Consolas,monospace">dni {} · trejdów {} · win {:.1}% · śr. trzymanie {:.0} min · najlepszy dzień {:+.0} $ · najgorszy {:+.0} $ · Sharpe {:.2} · recovery {:.2}{}</text>"#,
        h - 16.0,
        m.trading_days,
        m.trades,
        m.win_rate,
        m.median_hold_min,
        m.best_day,
        m.worst_day,
        m.sharpe,
        m.recovery_factor,
        if m.blown {
            " · ⚠ KONTO WYZEROWANE"
        } else {
            ""
        }
    );

    s.push_str("</svg>");
    s
}

/// Wykres porównawczy wielu konfiguracji: zysk (oś X) vs obsunięcie (oś Y).
pub fn compare_chart(title: &str, rows: &[(String, Metrics)]) -> String {
    let w = 1200.0;
    let h = 700.0;
    let pad_l = 80.0;
    let pad_r = 30.0;
    let pad_t = 90.0;
    let pad_b = 70.0;
    let pw = w - pad_l - pad_r;
    let ph = h - pad_t - pad_b;

    let mut s = String::with_capacity(32 * 1024);
    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="Inter,Segoe UI,system-ui,sans-serif">
<rect width="{w}" height="{h}" fill="{BG}"/>
<text x="{pad_l}" y="36" fill="{TEXT}" font-size="19" font-weight="700">{}</text>
<text x="{pad_l}" y="58" fill="{DIM}" font-size="12">im wyżej i bardziej w prawo, tym lepiej — oś X: zysk, oś Y: maksymalne obsunięcie (mniej = lepiej)</text>
<rect x="{pad_l}" y="{pad_t}" width="{pw:.1}" height="{ph:.1}" fill="{SURF}" rx="8"/>"#,
        esc(title)
    );

    if rows.is_empty() {
        s.push_str("</svg>");
        return s;
    }

    let max_p = rows
        .iter()
        .map(|(_, m)| m.total_profit)
        .fold(f64::MIN, f64::max)
        .max(1.0);
    let min_p = rows
        .iter()
        .map(|(_, m)| m.total_profit)
        .fold(f64::MAX, f64::min)
        .min(0.0);
    let max_d = rows.iter().map(|(_, m)| m.max_dd_abs).fold(1.0, f64::max);

    let sx = |p: f64| pad_l + (p - min_p) / (max_p - min_p).max(1e-9) * pw;
    let sy = |d: f64| pad_t + d / max_d * ph;

    for k in 0..=4 {
        let p = min_p + (max_p - min_p) * k as f64 / 4.0;
        let x = sx(p);
        let _ = write!(
            s,
            r#"<line x1="{x:.1}" y1="{pad_t}" x2="{x:.1}" y2="{:.1}" stroke="{GRID}"/>
<text x="{x:.1}" y="{:.1}" fill="{DIM}" font-size="10" text-anchor="middle" font-family="ui-monospace,Consolas,monospace">{p:.0} $</text>"#,
            pad_t + ph,
            pad_t + ph + 16.0
        );
        let d = max_d * k as f64 / 4.0;
        let y = sy(d);
        let _ = write!(
            s,
            r#"<line x1="{pad_l}" y1="{y:.1}" x2="{:.1}" y2="{y:.1}" stroke="{GRID}"/>
<text x="{:.1}" y="{:.1}" fill="{DIM}" font-size="10" text-anchor="end" font-family="ui-monospace,Consolas,monospace">−{d:.0} $</text>"#,
            pad_l + pw,
            pad_l - 8.0,
            y + 3.5
        );
    }
    // linia zerowego zysku
    if min_p < 0.0 && max_p > 0.0 {
        let x = sx(0.0);
        let _ = write!(
            s,
            r#"<line x1="{x:.1}" y1="{pad_t}" x2="{x:.1}" y2="{:.1}" stroke="{DIM}" stroke-dasharray="4 4" opacity="0.7"/>"#,
            pad_t + ph
        );
    }

    let best = rows
        .iter()
        .enumerate()
        .max_by(|a, b| {
            let ra = a.1 .1.total_profit / a.1 .1.max_dd_abs.max(1.0);
            let rb = b.1 .1.total_profit / b.1 .1.max_dd_abs.max(1.0);
            ra.partial_cmp(&rb).unwrap()
        })
        .map(|(i, _)| i);

    for (i, (name, m)) in rows.iter().enumerate() {
        let x = sx(m.total_profit);
        let y = sy(m.max_dd_abs);
        let is_best = best == Some(i);
        let col = if m.blown {
            DOWN
        } else if m.total_profit > 0.0 {
            UP
        } else {
            DIM
        };
        let r = if is_best { 8.0 } else { 5.5 };
        let _ = write!(
            s,
            r#"<circle cx="{x:.1}" cy="{y:.1}" r="{r}" fill="{col}" opacity="0.9"><title>{}
zysk {:+.2} $ · DD −{:.2} $ ({:.1}%) · PF {:.2} · dni+ {:.0}%</title></circle>"#,
            esc(name),
            m.total_profit,
            m.max_dd_abs,
            m.max_dd_pct,
            m.profit_factor,
            m.win_days_pct
        );
        if is_best {
            let _ = write!(
                s,
                r#"<circle cx="{x:.1}" cy="{y:.1}" r="13" fill="none" stroke="{ACC}" stroke-width="2"/>
<text x="{x:.1}" y="{:.1}" fill="{ACC}" font-size="11" font-weight="700" text-anchor="middle">{}</text>"#,
                y - 20.0,
                esc(name)
            );
        }
    }

    let _ = write!(
        s,
        r#"<text x="{pad_l}" y="{:.1}" fill="{DIM}" font-size="11">porównano {} konfiguracji · obwódką oznaczono najlepszy stosunek zysku do obsunięcia</text>"#,
        h - 18.0,
        rows.len()
    );
    s.push_str("</svg>");
    s
}
