//! Analiza dziennika zdarzeń (`.jsonl`).
//!
//! Odpowiada na pytania, na które log `bot.py` odpowiedzieć nie potrafił:
//!
//!  * **ile bot zarobił danego dnia** — grupowanie po dobie handlowej serwera,
//!    a nie po samej porze dnia,
//!  * **jak wyglądała krzywa kapitału i największe obsunięcie**,
//!  * **jak rozkładają się powody zamknięć** — i ile każdy z nich przynosi,
//!  * **ile pieniędzy zostawiono na stole**: dla każdej zamkniętej pozycji
//!    różnica między zyskiem zrealizowanym a maksymalnym możliwym (z MFE),
//!    zagregowana po powodach zamknięcia. To jest miara KOSZTU REGUŁY: jeśli
//!    `Trail` zostawia trzy razy więcej niż `Tp`, wiadomo, którą regułę
//!    poprawiać,
//!  * **na czym tracimy sygnały** — rozkład kodów odrzucenia.
//!
//! Logika mieszka w bibliotece (a nie w samym `bin/`), żeby dało się ją
//! przetestować na ręcznie policzonym przykładzie.

use conduit_core::journal::{EventKind, JournalEvent, JOURNAL_SCHEMA};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ============================================================
//  WYNIK
// ============================================================

/// Jeden dzień handlowy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DayRow {
    pub day: String,
    /// suma wyników NETTO zamkniętych pozycji tego dnia
    pub realized: f64,
    pub trades: u32,
    pub wins: u32,
    pub losses: u32,
    pub volume: f64,
    /// największe obsunięcie equity w obrębie dnia (z migawek)
    pub max_dd: f64,
    /// equity na pierwszej i ostatniej migawce dnia
    pub equity_open: f64,
    pub equity_close: f64,
    /// suma „zostawionego na stole" tego dnia
    pub left_on_table: f64,
    pub signals: u32,
    pub rejects: u32,
}

/// Wiersz rozkładu powodów zamknięcia — sedno raportu.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ReasonRow {
    pub reason: String,
    pub count: u32,
    pub wins: u32,
    /// suma wyników netto
    pub net: f64,
    pub volume: f64,
    pub left_on_table: f64,
    pub measured: u32,
    /// najgorszy pojedynczy przypadek
    pub worst_left: f64,
    pub worst_ticket: u64,
    pub avg_hold_s: f64,
}

impl ReasonRow {
    pub fn avg_net(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.net / self.count as f64
        }
    }
    pub fn avg_left(&self) -> f64 {
        if self.measured == 0 {
            0.0
        } else {
            self.left_on_table / self.measured as f64
        }
    }
    pub fn win_rate(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.wins as f64 / self.count as f64 * 100.0
        }
    }
}

/// Rozkład powodów odrzucenia sygnału / zignorowania komunikatu.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RejectRow {
    pub reason: String,
    pub count: u32,
    /// ile z tego to odrzucone WEJŚCIA (reszta to zignorowane komunikaty)
    pub entries: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Totals {
    pub events: u64,
    /// linie, których nie dało się sparsować — jawnie, nie po cichu
    pub bad_lines: u64,
    pub trades: u32,
    pub wins: u32,
    pub net: f64,
    pub gross: f64,
    pub commission: f64,
    pub swap: f64,
    pub volume: f64,
    pub left_on_table: f64,
    pub measured: u32,
    pub max_dd_abs: f64,
    pub max_dd_pct: f64,
    pub equity_start: f64,
    pub equity_end: f64,
    pub messages: u32,
    pub baskets: u32,
    pub rejects: u32,
    pub risk_stops: u32,
    pub first_ts: String,
    pub last_ts: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Report {
    pub schema: u32,
    pub totals: Totals,
    pub days: Vec<DayRow>,
    /// krzywa kapitału: (znacznik ticka w ms, equity)
    pub equity_curve: Vec<(i64, f64)>,
    pub reasons: Vec<ReasonRow>,
    pub rejects: Vec<RejectRow>,
    /// pojedyncze zamknięcia o największym „zostawionym na stole"
    pub worst_trades: Vec<WorstTrade>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WorstTrade {
    pub ticket: u64,
    pub basket_id: Option<u32>,
    pub day: String,
    pub ts_broker: String,
    pub reason: String,
    pub net: f64,
    pub mfe_usd: f64,
    pub mae_usd: f64,
    pub left_on_table: f64,
    pub hold_s: f64,
}

// ============================================================
//  ANALIZA
// ============================================================

/// Stan zbierany po drodze — osobno od wyniku, żeby dało się karmić go
/// zdarzeniami z kilku plików naraz.
#[derive(Debug, Default)]
pub struct Analiza {
    dni: BTreeMap<String, DayRow>,
    powody: BTreeMap<String, ReasonRow>,
    odrzucenia: BTreeMap<String, RejectRow>,
    krzywa: Vec<(i64, f64)>,
    najgorsze: Vec<WorstTrade>,
    tot: Totals,
    schema: u32,
    /// szczyt equity do liczenia obsunięcia
    szczyt: f64,
    /// szczyt equity w obrębie doby
    szczyt_dnia: f64,
    biezacy_dzien: String,
    /// gdy ustawione, liczymy WYŁĄCZNIE tę dobę handlową
    only_day: Option<String>,
}

impl Analiza {
    pub fn new() -> Self {
        Analiza {
            schema: JOURNAL_SCHEMA,
            ..Default::default()
        }
    }

    /// Ogranicza analizę do jednej doby.
    ///
    /// Filtr działa PRZY WCZYTYWANIU, a nie na gotowym raporcie — inaczej
    /// sumy, obsunięcia i „na stole" pokazywałyby cały plik, a tabela dni
    /// jeden wiersz. Taki raport kłamie ciszej niż brak raportu.
    pub fn tylko_dzien(mut self, d: impl Into<String>) -> Self {
        self.only_day = Some(d.into());
        self
    }

    /// Wciąga jedną linię pliku. Zwraca `false`, gdy linii nie dało się
    /// odczytać — liczymy takie przypadki zamiast je pomijać w ciszy.
    pub fn linia(&mut self, l: &str) -> bool {
        let l = l.trim();
        if l.is_empty() {
            return true;
        }
        match serde_json::from_str::<JournalEvent>(l) {
            Ok(ev) => {
                self.zdarzenie(&ev);
                true
            }
            Err(_) => {
                self.tot.bad_lines += 1;
                false
            }
        }
    }

    pub fn zdarzenie(&mut self, ev: &JournalEvent) {
        if let Some(d) = &self.only_day {
            if &ev.session_day != d {
                return;
            }
        }
        self.tot.events += 1;
        if ev.v > self.schema {
            self.schema = ev.v;
        }
        if self.tot.first_ts.is_empty() {
            self.tot.first_ts = ev.ts_broker.clone();
        }
        self.tot.last_ts = ev.ts_broker.clone();

        let dzien = if ev.session_day.is_empty() {
            "0000-00-00".to_string()
        } else {
            ev.session_day.clone()
        };
        if dzien != self.biezacy_dzien {
            self.biezacy_dzien = dzien.clone();
            self.szczyt_dnia = 0.0;
        }
        let d = self.dni.entry(dzien.clone()).or_insert_with(|| DayRow {
            day: dzien.clone(),
            ..Default::default()
        });

        // ---- krzywa kapitału i obsunięcia z migawek ----
        if let Some(m) = &ev.market {
            if m.equity != 0.0 {
                if d.equity_open == 0.0 {
                    d.equity_open = m.equity;
                }
                d.equity_close = m.equity;
                if self.tot.equity_start == 0.0 {
                    self.tot.equity_start = m.equity;
                }
                self.tot.equity_end = m.equity;

                if m.equity > self.szczyt {
                    self.szczyt = m.equity;
                }
                if m.equity > self.szczyt_dnia {
                    self.szczyt_dnia = m.equity;
                }
                let dd = self.szczyt - m.equity;
                if dd > self.tot.max_dd_abs {
                    self.tot.max_dd_abs = dd;
                    self.tot.max_dd_pct = dd / self.szczyt.max(1.0) * 100.0;
                }
                let ddd = self.szczyt_dnia - m.equity;
                if ddd > d.max_dd {
                    d.max_dd = ddd;
                }
                // krzywą próbkujemy rzadko — raport ma być czytelny, a nie
                // zawierać po jednym punkcie na każdą wiadomość z kanału
                if self
                    .krzywa
                    .last()
                    .map(|(t, _)| ev.ts_broker_ms - *t >= 60_000)
                    .unwrap_or(true)
                {
                    self.krzywa.push((ev.ts_broker_ms, m.equity));
                }
            }
        }

        match ev.kind {
            EventKind::MessageReceived => {
                self.tot.messages += 1;
                d.signals += 1;
            }
            EventKind::BasketCreated => self.tot.baskets += 1,
            EventKind::RiskStop => self.tot.risk_stops += 1,
            EventKind::SignalRejected | EventKind::TargetIgnored => {
                self.tot.rejects += 1;
                d.rejects += 1;
                let kod = ev
                    .reason
                    .map(|r| r.as_str().to_string())
                    .unwrap_or_else(|| "(brak kodu)".into());
                let r = self
                    .odrzucenia
                    .entry(kod.clone())
                    .or_insert_with(|| RejectRow {
                        reason: kod,
                        ..Default::default()
                    });
                r.count += 1;
                if ev.kind == EventKind::SignalRejected {
                    r.entries += 1;
                }
            }
            EventKind::PositionClosed => {
                let Some(c) = &ev.close else { return };
                d.trades += 1;
                d.realized += c.net;
                d.volume += c.volume;
                d.left_on_table += c.left_on_table;
                if c.net > 0.0 {
                    d.wins += 1;
                } else {
                    d.losses += 1;
                }

                self.tot.trades += 1;
                self.tot.net += c.net;
                self.tot.gross += c.gross;
                self.tot.commission += c.commission;
                self.tot.swap += c.swap;
                self.tot.volume += c.volume;
                if c.net > 0.0 {
                    self.tot.wins += 1;
                }

                let row = self
                    .powody
                    .entry(c.reason.clone())
                    .or_insert_with(|| ReasonRow {
                        reason: c.reason.clone(),
                        ..Default::default()
                    });
                row.count += 1;
                row.net += c.net;
                row.volume += c.volume;
                row.avg_hold_s += c.hold_s;
                if c.net > 0.0 {
                    row.wins += 1;
                }
                if c.excursion.samples > 0 {
                    row.measured += 1;
                    row.left_on_table += c.left_on_table;
                    self.tot.measured += 1;
                    self.tot.left_on_table += c.left_on_table;
                    if c.left_on_table > row.worst_left {
                        row.worst_left = c.left_on_table;
                        row.worst_ticket = ev.ticket.unwrap_or(0);
                    }
                    self.najgorsze.push(WorstTrade {
                        ticket: ev.ticket.unwrap_or(0),
                        basket_id: ev.basket_id,
                        day: dzien,
                        ts_broker: ev.ts_broker.clone(),
                        reason: c.reason.clone(),
                        net: c.net,
                        mfe_usd: c.excursion.mfe_usd,
                        mae_usd: c.excursion.mae_usd,
                        left_on_table: c.left_on_table,
                        hold_s: c.hold_s,
                    });
                }
            }
            _ => {}
        }
    }

    /// Domyka raport: sortuje, uśrednia, przycina listy.
    pub fn raport(mut self) -> Report {
        for r in self.powody.values_mut() {
            if r.count > 0 {
                r.avg_hold_s /= r.count as f64;
            }
        }
        let mut powody: Vec<ReasonRow> = self.powody.into_values().collect();
        // sortujemy po TYM, ILE KOSZTUJĄ — bo po to jest ten raport
        powody.sort_by(|a, b| b.left_on_table.total_cmp(&a.left_on_table));

        let mut odrzucenia: Vec<RejectRow> = self.odrzucenia.into_values().collect();
        odrzucenia.sort_by(|a, b| b.count.cmp(&a.count));

        self.najgorsze
            .sort_by(|a, b| b.left_on_table.total_cmp(&a.left_on_table));
        self.najgorsze.truncate(20);

        Report {
            schema: self.schema,
            totals: self.tot,
            days: self.dni.into_values().collect(),
            equity_curve: self.krzywa,
            reasons: powody,
            rejects: odrzucenia,
            worst_trades: self.najgorsze,
        }
    }
}

/// Wczytuje jeden plik `.jsonl`.
pub fn wczytaj(sciezka: &std::path::Path, a: &mut Analiza) -> std::io::Result<()> {
    use std::io::BufRead;
    let f = std::fs::File::open(sciezka)?;
    let r = std::io::BufReader::new(f);
    for l in r.lines() {
        a.linia(&l?);
    }
    Ok(())
}

/// Wczytuje wszystkie `*.jsonl` z katalogu (albo pojedynczy plik).
///
/// Pliki bierzemy w kolejności NAZW, a te zawierają datę doby — więc
/// zdarzenia wchodzą chronologicznie nawet wtedy, gdy katalog zwróci je
/// w przypadkowej kolejności.
pub fn wczytaj_sciezke(p: &std::path::Path) -> std::io::Result<Analiza> {
    wczytaj_sciezke_dnia(p, None)
}

/// To samo, z opcjonalnym ograniczeniem do jednej doby handlowej.
pub fn wczytaj_sciezke_dnia(p: &std::path::Path, dzien: Option<&str>) -> std::io::Result<Analiza> {
    let mut a = Analiza::new();
    if let Some(d) = dzien {
        a = a.tylko_dzien(d);
    }
    if p.is_dir() {
        let mut pliki: Vec<std::path::PathBuf> = std::fs::read_dir(p)?
            .flatten()
            .map(|e| e.path())
            .filter(|x| x.extension().and_then(|e| e.to_str()) == Some("jsonl"))
            .collect();
        pliki.sort();
        for f in pliki {
            wczytaj(&f, &mut a)?;
        }
    } else {
        wczytaj(p, &mut a)?;
    }
    Ok(a)
}

// ============================================================
//  WYDRUK
// ============================================================

fn linia(n: usize) -> String {
    "─".repeat(n)
}

/// Raport w postaci tabel dla człowieka.
pub fn tekst(r: &Report) -> String {
    let mut s = String::with_capacity(8192);
    let t = &r.totals;

    s.push_str(&format!("{}\n", linia(96)));
    s.push_str("DZIENNIK ZDARZEŃ — ANALIZA\n");
    s.push_str(&format!("{}\n", linia(96)));
    s.push_str(&format!(
        "zdarzeń: {}   linii nieczytelnych: {}   schemat: v{}\n",
        t.events, t.bad_lines, r.schema
    ));
    s.push_str(&format!("okres:   {} → {}\n", t.first_ts, t.last_ts));
    s.push_str(&format!(
        "wiadomości: {}   koszyków: {}   odrzuceń: {}   zamknięć awaryjnych: {}\n",
        t.messages, t.baskets, t.rejects, t.risk_stops
    ));
    s.push_str(&format!(
        "transakcje: {}   trafność: {:.1} %   wynik netto: {:+.2} $   (brutto {:+.2}, prowizja {:.2}, swap {:+.2})\n",
        t.trades,
        if t.trades > 0 { t.wins as f64 / t.trades as f64 * 100.0 } else { 0.0 },
        t.net,
        t.gross,
        t.commission,
        t.swap
    ));
    s.push_str(&format!(
        "equity: {:.2} $ → {:.2} $   max obsunięcie: {:.2} $ ({:.2} %)\n",
        t.equity_start, t.equity_end, t.max_dd_abs, t.max_dd_pct
    ));
    s.push('\n');

    // ---------- dni ----------
    s.push_str("WYNIK DZIENNY (doba handlowa serwera)\n");
    s.push_str(&format!("{}\n", linia(96)));
    s.push_str(&format!(
        "{:<12} {:>10} {:>7} {:>7} {:>9} {:>11} {:>13}\n",
        "dzień", "netto $", "transakcji", "trafnych", "obsunięcie", "equity koniec", "na stole $"
    ));
    for d in &r.days {
        s.push_str(&format!(
            "{:<12} {:>10.2} {:>10} {:>8} {:>9.2} {:>11.2} {:>13.2}\n",
            d.day, d.realized, d.trades, d.wins, d.max_dd, d.equity_close, d.left_on_table
        ));
    }
    if r.days.len() > 1 {
        let suma: f64 = r.days.iter().map(|d| d.realized).sum();
        let dni_plus = r.days.iter().filter(|d| d.realized > 0.0).count();
        s.push_str(&format!("{}\n", linia(96)));
        s.push_str(&format!(
            "RAZEM {:.2} $ w {} dni · dni dodatnich {}/{} ({:.0} %) · średnio {:.2} $/dzień\n",
            suma,
            r.days.len(),
            dni_plus,
            r.days.len(),
            dni_plus as f64 / r.days.len() as f64 * 100.0,
            suma / r.days.len() as f64
        ));
    }
    s.push('\n');

    // ---------- powody zamknięć + pieniądze na stole ----------
    s.push_str("ILE PIENIĘDZY ZOSTAWIONO NA STOLE (wg powodu zamknięcia)\n");
    s.push_str("różnica między zyskiem zrealizowanym a maksymalnym możliwym (z MFE)\n");
    s.push_str(&format!("{}\n", linia(96)));
    s.push_str(&format!(
        "{:<12} {:>6} {:>8} {:>10} {:>9} {:>12} {:>10} {:>10}\n",
        "powód", "ile", "trafność", "netto $", "śr. netto", "NA STOLE $", "śr./poz.", "najgorsza"
    ));
    for x in &r.reasons {
        s.push_str(&format!(
            "{:<12} {:>6} {:>7.1}% {:>10.2} {:>9.2} {:>12.2} {:>10.2} {:>10.2}\n",
            x.reason,
            x.count,
            x.win_rate(),
            x.net,
            x.avg_net(),
            x.left_on_table,
            x.avg_left(),
            x.worst_left
        ));
    }
    s.push_str(&format!("{}\n", linia(96)));
    let potencjal = t.net + t.left_on_table;
    s.push_str(&format!(
        "RAZEM na stole {:.2} $ przy wyniku netto {:+.2} $\n",
        t.left_on_table, t.net
    ));
    s.push_str(&format!(
        "potencjał (co dałoby wyjście w szczycie każdej pozycji): {:+.2} $ — zainkasowano {:.0} % z tego\n",
        potencjal,
        if potencjal > 0.0 { t.net / potencjal * 100.0 } else { 0.0 }
    ));
    s.push_str(&format!(
        "(zmierzono {} z {} zamknięć; pozycje bez pomiaru wychylenia nie wchodzą do sumy)\n",
        t.measured, t.trades
    ));
    s.push('\n');

    // ---------- odrzucenia ----------
    if !r.rejects.is_empty() {
        s.push_str("NA CZYM TRACIMY SYGNAŁY (kody odrzucenia)\n");
        s.push_str(&format!("{}\n", linia(96)));
        s.push_str(&format!(
            "{:<34} {:>8} {:>12}\n",
            "kod", "ile", "w tym wejść"
        ));
        for x in &r.rejects {
            s.push_str(&format!(
                "{:<34} {:>8} {:>12}\n",
                x.reason, x.count, x.entries
            ));
        }
        s.push('\n');
    }

    // ---------- najgorsze pojedyncze ----------
    if !r.worst_trades.is_empty() {
        s.push_str("POJEDYNCZE ZAMKNIĘCIA, KTÓRE KOSZTOWAŁY NAJWIĘCEJ\n");
        s.push_str(&format!("{}\n", linia(96)));
        s.push_str(&format!(
            "{:<10} {:<6} {:<12} {:>10} {:>10} {:>12} {:>10}\n",
            "zlecenie", "koszyk", "powód", "netto $", "MFE $", "na stole $", "trzymane"
        ));
        for w in r.worst_trades.iter().take(10) {
            s.push_str(&format!(
                "#{:<9} {:<6} {:<12} {:>10.2} {:>10.2} {:>12.2} {:>9.0}s\n",
                w.ticket,
                w.basket_id
                    .map(|b| format!("B{b}"))
                    .unwrap_or_else(|| "—".into()),
                w.reason,
                w.net,
                w.mfe_usd,
                w.left_on_table,
                w.hold_s
            ));
        }
    }
    s
}

// ============================================================
//  TESTY
// ============================================================

#[cfg(test)]
mod testy {
    use super::*;
    use conduit_core::journal::*;
    use conduit_core::types::*;

    /// Buduje linię `.jsonl` z zamknięciem o zadanych liczbach.
    #[allow(clippy::too_many_arguments)]
    fn zamkniecie(
        dzien_ms: i64,
        ticket: Ticket,
        volume: f64,
        open: f64,
        close: f64,
        reason: CloseReason,
        mfe_pts: f64,
        equity: f64,
    ) -> String {
        let mut buf = JournalBuf::new(
            JournalConfig {
                enabled: true,
                min_level: EventLevel::Debug,
                ..Default::default()
            },
            "t",
        );
        let side = if close >= open { Side::Buy } else { Side::Buy };
        let profit = (close - open) * XAU_CONTRACT * volume;
        let tr = ClosedTrade {
            profit_basis: None, cost_receipt: None,
            ticket,
            side,
            volume,
            open_price: open,
            close_price: close,
            open_ts: dzien_ms - 600_000,
            close_ts: dzien_ms,
            profit,
            commission: 0.0,
            swap: 0.0,
            reason,
            basket: Some(1),
        };
        let exc = Excursion {
            mfe_price: open + mfe_pts,
            mae_price: open,
            mfe_pts,
            mae_pts: 0.0,
            mfe_usd: mfe_pts * XAU_CONTRACT * volume,
            mae_usd: 0.0,
            mfe_ts: dzien_ms - 100,
            mae_ts: dzien_ms - 500,
            samples: 100,
        };
        buf.push(
            Ev::new(
                dzien_ms,
                EventLevel::Ok,
                EventCategory::Trade,
                EventKind::PositionClosed,
            )
            .text("zamknięcie")
            .ticket(ticket)
            .basket(1)
            .market(Some(MarketSnapshot {
                equity,
                ..Default::default()
            }))
            .close(CloseDetail::new(&tr, exc).expect("locally constructed legacy trade"))
            .build(),
        );
        serde_json::to_string(&buf.drain()[0]).unwrap()
    }

    /// Raport „zostawione na stole" na przykładzie policzonym RĘCZNIE.
    ///
    /// Trzy pozycje BUY po 0.10 lota (1 $ ruchu = 10 $):
    ///  * #1 Tp:    3300 → 3310, MFE 15 $/pkt → +100 netto, MFE 150 → na stole 50
    ///  * #2 Trail: 3300 → 3305, MFE 30 pkt   → +50 netto,  MFE 300 → na stole 250
    ///  * #3 Sl:    3300 → 3295, MFE 2 pkt    → −50 netto,  MFE  20 → na stole 70
    ///
    /// Suma na stole = 50 + 250 + 70 = 370 $, wynik netto = +100 $.
    /// Najdroższą regułą jest `Trail` (250 $), i to musi być w raporcie
    /// PIERWSZY wiersz.
    #[test]
    fn zostawione_na_stole_liczone_recznie() {
        let d = 1_784_800_800_000i64; // 2026-07-23
        let mut a = Analiza::new();
        a.linia(&zamkniecie(
            d,
            1,
            0.10,
            3300.0,
            3310.0,
            CloseReason::Tp,
            15.0,
            1100.0,
        ));
        a.linia(&zamkniecie(
            d + 1000,
            2,
            0.10,
            3300.0,
            3305.0,
            CloseReason::Trail,
            30.0,
            1150.0,
        ));
        a.linia(&zamkniecie(
            d + 2000,
            3,
            0.10,
            3300.0,
            3295.0,
            CloseReason::Sl,
            2.0,
            1100.0,
        ));
        let r = a.raport();

        assert_eq!(r.totals.trades, 3);
        assert_eq!(r.totals.wins, 2);
        assert!(
            (r.totals.net - 100.0).abs() < 1e-6,
            "netto = {}",
            r.totals.net
        );
        assert!(
            (r.totals.left_on_table - 370.0).abs() < 1e-6,
            "na stole = {}",
            r.totals.left_on_table
        );
        assert_eq!(r.totals.measured, 3);

        // wiersze posortowane po KOSZCIE reguły
        assert_eq!(r.reasons[0].reason, "Trail");
        assert!((r.reasons[0].left_on_table - 250.0).abs() < 1e-6);
        assert_eq!(r.reasons[1].reason, "Sl");
        assert!((r.reasons[1].left_on_table - 70.0).abs() < 1e-6);
        assert_eq!(r.reasons[2].reason, "Tp");
        assert!((r.reasons[2].left_on_table - 50.0).abs() < 1e-6);

        // rozkład per powód
        let tp = r.reasons.iter().find(|x| x.reason == "Tp").unwrap();
        assert_eq!(tp.count, 1);
        assert_eq!(tp.wins, 1);
        assert!((tp.net - 100.0).abs() < 1e-6);

        // dzień: jeden, z pełną datą
        assert_eq!(r.days.len(), 1);
        assert_eq!(r.days[0].day, "2026-07-23");
        assert!((r.days[0].realized - 100.0).abs() < 1e-6);
        assert_eq!(r.days[0].trades, 3);

        // obsunięcie z migawek: szczyt 1150 → 1100 = 50 $
        assert!(
            (r.totals.max_dd_abs - 50.0).abs() < 1e-6,
            "DD = {}",
            r.totals.max_dd_abs
        );

        // najgorszy pojedynczy przypadek to #2
        assert_eq!(r.worst_trades[0].ticket, 2);
        assert!((r.worst_trades[0].left_on_table - 250.0).abs() < 1e-6);
    }

    /// Dni się nie mieszają — to jest cały sens daty w znaczniku.
    #[test]
    fn dni_sie_nie_mieszaja() {
        let d23 = 1_784_800_800_000i64;
        let d24 = d23 + 24 * 3_600_000;
        let mut a = Analiza::new();
        a.linia(&zamkniecie(
            d23,
            1,
            0.10,
            3300.0,
            3310.0,
            CloseReason::Tp,
            10.0,
            1100.0,
        ));
        a.linia(&zamkniecie(
            d24,
            2,
            0.10,
            3300.0,
            3290.0,
            CloseReason::Sl,
            1.0,
            1000.0,
        ));
        let r = a.raport();
        assert_eq!(r.days.len(), 2);
        assert_eq!(r.days[0].day, "2026-07-23");
        assert_eq!(r.days[1].day, "2026-07-24");
        assert!((r.days[0].realized - 100.0).abs() < 1e-6);
        assert!((r.days[1].realized + 100.0).abs() < 1e-6);
    }

    /// Odrzucenia zliczają się po KODZIE — dawne „TARGET_ignore" bez powodu
    /// już nie ma jak powstać.
    #[test]
    fn odrzucenia_zliczane_po_kodzie() {
        let mut buf = JournalBuf::new(
            JournalConfig {
                enabled: true,
                min_level: EventLevel::Debug,
                ..Default::default()
            },
            "t",
        );
        for _ in 0..3 {
            buf.push(
                Ev::new(
                    0,
                    EventLevel::Warn,
                    EventCategory::Decision,
                    EventKind::SignalRejected,
                )
                .reason(RejectCode::SlBreached)
                .build(),
            );
        }
        buf.push(
            Ev::new(
                0,
                EventLevel::Warn,
                EventCategory::Decision,
                EventKind::TargetIgnored,
            )
            .reason(RejectCode::NoTargetBasket)
            .build(),
        );
        let mut a = Analiza::new();
        for ev in buf.drain() {
            a.linia(&serde_json::to_string(&ev).unwrap());
        }
        let r = a.raport();
        assert_eq!(r.totals.rejects, 4);
        assert_eq!(r.rejects[0].reason, "sl_breached");
        assert_eq!(r.rejects[0].count, 3);
        assert_eq!(r.rejects[0].entries, 3);
        assert_eq!(r.rejects[1].reason, "no_target_basket");
        assert_eq!(r.rejects[1].entries, 0);
    }

    /// Uszkodzona linia jest LICZONA, nie pomijana w ciszy.
    #[test]
    fn uszkodzona_linia_jest_widoczna() {
        let mut a = Analiza::new();
        assert!(!a.linia("{to nie jest json"));
        assert!(a.linia(""));
        let r = a.raport();
        assert_eq!(r.totals.bad_lines, 1);
        assert_eq!(r.totals.events, 0);
    }
}
