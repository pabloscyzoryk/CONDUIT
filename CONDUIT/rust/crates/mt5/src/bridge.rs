
use std::collections::HashMap;
#[path = "receipt_volume.rs"]
mod receipt_volume;
use receipt_volume::ReceiptVolumes;
use std::time::{Duration, Instant};

use conduit_core::broker::*;
use conduit_core::types::*;
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use crate::comment;
use crate::errors::{self, retcode};
use crate::proto::{RawAccount, RawClosed, RawOrder, RawPosition, RawTick, SymbolInfo};
use crate::transport::{CallError, SidecarConfig, Transport};

/// Skąd wzięła się pozycja/zlecenie widoczne na rachunku.
///
/// CONDUIT jest ogólną platformą tradingową — pokazuje WSZYSTKO, co dzieje się
/// na koncie, a nie tylko własny handel. To pole mówi, czym bot **zarządza**:
/// `Bot` tak, pozostałe NIE. Rozróżnienie jest wyłącznie informacyjne —
/// filtrowanie wykonawcze robi nadal `is_ours`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// nasz `magic` i nasz symbol — prowadzone przez silnik
    Bot,
    External,
    /// `magic` 0 — otwarte ręcznie z terminala
    Manual,
}

impl Origin {
    /// Klasyfikacja po `magic`. `own` to `magic` bota.
    pub fn classify(magic: i64, own: i64, ours: bool) -> Origin {
        if ours {
            Origin::Bot
        } else if magic == 0 {
            Origin::Manual
        } else {
            let _ = own;
            Origin::External
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Bot => "BOT",
            Origin::External => "EXTERNAL",
            Origin::Manual => "MANUAL",
        }
    }
}

/// Pozycja, której bot NIE prowadzi — cudzy automat albo ręczny handel.
///
/// Trzymana osobno od `positions`, bo silnik nie ma prawa jej dotknąć
/// (`close_everything()` zamknęłoby ją razem ze swoimi). Ale ma być WIDOCZNA:
/// realnie zużywa margines i wchodzi w equity, więc jej ukrycie robiło
/// z panelu kłamcę — equity się nie zgadzało z listą pozycji.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignPosition {
    pub ticket: Ticket,
    pub symbol: String,
    pub side: Side,
    pub volume: f64,
    pub open_price: f64,
    pub open_ts: i64,
    pub sl: Option<Px>,
    pub tp: Option<Px>,
    /// zysk pływający PROSTO OD BROKERA — my nie mamy kwotowania tego symbolu
    pub profit: f64,
    pub magic: i64,
    pub comment: String,
    pub origin: Origin,
}

/// Zamknięta transakcja spoza bota. Nie wchodzi do statystyk silnika —
/// wchodzi do historii pokazywanej użytkownikowi, z etykietą źródła.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignClosed {
    pub ticket: Ticket,
    pub symbol: String,
    pub side: Side,
    pub volume: f64,
    pub open_price: f64,
    pub close_price: f64,
    pub open_ts: i64,
    pub close_ts: i64,
    pub profit: f64,
    pub commission: f64,
    pub swap: f64,
    pub magic: i64,
    pub comment: String,
    pub origin: Origin,
}

/// Zlecenie oczekujące spoza bota — jak wyżej.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignOrder {
    pub ticket: Ticket,
    pub symbol: String,
    pub kind: PendingKind,
    pub volume: f64,
    pub price: f64,
    pub sl: Option<Px>,
    pub tp: Option<Px>,
    pub placed_ts: i64,
    pub magic: i64,
    pub comment: String,
    pub origin: Origin,
}

/// Podsumowanie odtworzenia stanu po restarcie.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    /// pozycje rozpoznane jako nasze
    pub positions: usize,
    /// zlecenia oczekujące rozpoznane jako nasze
    pub pendings: usize,
    /// nasze wg `magic`, ale komentarz nieczytelny — koszyk nieznany
    pub orphans: usize,
    /// cudze (inny `magic` albo inny symbol) — świadomie pomijane
    pub foreign: usize,
    /// numery koszyków, które udało się odtworzyć
    pub baskets: Vec<u32>,
}

/// Rozjazd między tym, czego spodziewa się symulator, a tym, co mówi broker.
///
/// Powstało z konkretnej dziury: minimalny dystans SL/TP symulator bierze
/// z presetu (`sim_stops_level`, dziś 0,20), a most z serwera brokera.
/// **Dziś obie wartości są równe i NIC tego nie pilnuje.** Gdy Vantage
/// zmieni `stops_level` — a robi to przed danymi makro — backtest i konto
/// zaczną opisywać inny handel, bez jednego słowa ostrzeżenia. Rozpoznamy
/// to po wynikach dopiero po tygodniach, jeśli w ogóle.
#[derive(Debug, Clone, PartialEq)]
pub struct StopsMismatch {
    pub symbol: String,
    /// wartość z presetu/ustawień, w jednostkach CENY
    pub expected_price: f64,
    /// wartość z serwera brokera, w jednostkach CENY
    pub broker_price: f64,
    pub expected_points: f64,
    pub broker_points: f64,
}

impl StopsMismatch {
    /// Komunikat dla dziennika i dla człowieka. Mówi, co z tego wynika,
    /// a nie tylko że liczby są różne.
    pub fn opis(&self) -> String {
        let (mniej, wiecej) = if self.broker_price > self.expected_price {
            (
                "łagodniejszy",
                "ODRZUCI część zleceń, które backtest przyjmuje",
            )
        } else {
            ("ostrzejszy", "przyjmie zlecenia, które backtest odrzuca")
        };
        format!(
            "ROZJAZD stops_level na {}: preset zakłada {:.5} ({:.0} pkt), broker wymaga {:.5} ({:.0} pkt). \
             Preset jest {mniej} od brokera, więc w SYMULACJI rachunek {wiecej}. \
             Żywy handel tego nie dotyczy — most nadpisuje `stops_level` wartością z serwera brokera, \
             więc na konto nie pójdzie ani jedno złe zlecenie. Psuje się PARYTET: backtest i sprawdzenie \
             w panelu opisują inny handel niż konto, a różnicy nie widać inaczej niż w wynikach. \
             Popraw `sim_stops_level` w presecie na {:.5} albo sprawdź, czy broker nie zmienił warunków.",
            self.symbol,
            self.expected_price,
            self.expected_points,
            self.broker_price,
            self.broker_points,
            self.broker_price,
        )
    }
}

pub struct Mt5Bridge {
    tr: Transport,
    sym: SymbolInfo,
    /// Niezgodność `stops_level` wykryta przy starcie — `None`, gdy zgodne
    /// albo gdy nikt nie podał wartości oczekiwanej.
    stops_mismatch: Option<StopsMismatch>,
    tag: String,
    magic: i64,

    q: Quote,
    acc: Account,
    /// Kto jest właścicielem rachunku i u kogo on leży. Odświeżane razem
    /// z saldem, bo przychodzi tą samą odpowiedzią.
    ident: crate::proto::AccountIdent,
    positions: Vec<Position>,
    pendings: Vec<PendingOrder>,
    closed: Vec<ClosedTrade>,

    /// Pozycje i zlecenia SPOZA bota. Osobne wektory, bo silnik dostaje tylko
    /// `positions`/`pendings` — panel dostaje jedno i drugie.
    foreign_pos: Vec<ForeignPosition>,
    foreign_ord: Vec<ForeignOrder>,
    foreign_closed: Vec<ForeignClosed>,

    /// powód zamknięcia zapamiętany w chwili wydania polecenia — MT5 go nie zna
    reason_of: HashMap<Ticket, CloseReason>,
    /// F18A: session-only receipt journal. The transport pins the account;
    /// each record is additionally restricted to this bridge's magic/symbol.
    /// Do not mistake this for a durable consumer ACK (that is a separate contract).
    receipts: ReceiptJournal,
    /// Session-latched application safety barrier; never changes ledger basis.
    runtime_entry_hold: Option<String>,
    /// Ostatnie wywołanie handlowe nie dało rozstrzygającej odpowiedzi
    /// transportu.  Rozróżnia timeout/malformed od jednoznacznej odmowy
    /// brokera; tylko pierwszy przypadek wolno później rekoncyliować stanem.
    last_trade_outcome_unknown: bool,

    last_state: Instant,
    state_interval: Duration,
    max_retries: u32,
    retry_delay: Duration,

    // --- statystyki wykonania, do audytu realizmu ---
    pub rejected_stops: u64,
    pub requotes: u64,
    pub market_instead_of_limit: u64,
    pub send_failures: u64,
    /// wysłania, których wynik jest NIEZNANY (timeout) — wymagają rekoncyliacji
    pub unknown_sends: u64,
}

#[derive(Default)]
struct ReceiptJournal {
    volumes: ReceiptVolumes,
    /// POSITION_IDENTIFIER -> latest engine-owned metadata, retained after disappearance.
    positions: HashMap<u64, Position>,
    /// Current and former physical tickets -> stable identifier (never guessed).
    aliases: HashMap<Ticket, u64>,
    seen: HashMap<u64, RawClosed>,
    reasons: HashMap<u64, (u64, CloseReason)>,
    waiting: Vec<RawClosed>,
    incomplete: Vec<RawClosed>,
    /// Snapshot contradictions are retained for operator reconciliation. They
    /// must not be resurrected by a later stale snapshot of the old direction.
    quarantined_positions: HashMap<Ticket, RawPosition>,
    fault: Option<String>,
    /// `true` wyłącznie gdy jedynym faultem jest nierozstrzygnięty OPEN,
    /// dla którego znamy pełną intencję i możemy szukać dokładnego faktu MT5.
    recoverable_unknown_open_fault: bool,
    unknown_opens: Vec<UnknownOpenIntent>,
}

#[derive(Debug, Clone)]
struct UnknownOpenIntent {
    side: Side,
    requested_volume: f64,
    basket: Option<u32>,
    level: i32,
    is_toucher: bool,
    submitted_quote_ts: Ts,
    machine_comment: String,
}

impl Mt5Bridge {
    /// Podnosi sidecar, pobiera parametry instrumentu i odtwarza stan z konta.
    pub fn connect(cfg: SidecarConfig) -> anyhow::Result<Self> {
        anyhow::ensure!(!cfg.closed_profit_net_costs || cfg.close_receipt_reconcile,
            "closed_profit_net_costs requires close_receipt_reconcile before connecting");
        let magic = cfg.magic;
        let symbol = cfg.symbol.clone();
        let follow_account = cfg.follow_terminal_account;
        let reconcile_receipts = cfg.close_receipt_reconcile;
        let allow_real = cfg.allow_real_account;
        // wyjmujemy PRZED `start`, bo konfiguracja jest tam przenoszona
        let expected_stops = cfg.expected_stops_level_price;
        let tr = Transport::start(cfg)?;
        if !tr.wait_connected(Duration::from_secs(120)) {
            anyhow::bail!("sidecar MT5 nie podłączył się w ciągu 120 s");
        }

        if follow_account || reconcile_receipts {
            let initial: RawAccount = tr.call_as("account", Value::Null)?;
            anyhow::ensure!(initial.login != 0 && !initial.server.is_empty(), "follow: konto niepotwierdzone");
            tr.bind_account(initial.login, &initial.server, initial.trade_mode as i32);
            anyhow::ensure!(!follow_account || initial.trade_mode == 0 || allow_real,
                "follow: terminal jest na rachunku REAL/CONTEST; handel zablokowany. DEMO jest domyślne; REAL wymaga jawnego mt5_allow_real_account=true");
        }

        let sym: SymbolInfo = tr
            .call_as("symbol_info", if follow_account { Value::Null } else { json!({ "symbol": symbol }) })
            .map_err(|e| anyhow::anyhow!("nie udało się pobrać parametrów {symbol}: {e}"))?;
        tr.bind_symbol(&sym.symbol);
        info!(
            symbol = %sym.symbol,
            digits = sym.digits,
            stops_level_points = sym.stops_level_points,
            stops_level_price = sym.stops_level_price(),
            filling_market = sym.filling_market,
            "MT5: parametry instrumentu Z SERWERA"
        );

        // Kontrola zgodności z symulatorem. Robimy ją TU, przy pierwszym
        // odczycie parametrów, bo to jedyna chwila, w której obie liczby są
        // dostępne naraz i jeszcze nikt na nich nie handlował.
        let stops_mismatch = sprawdz_stops_level(&sym, expected_stops);
        if let Some(m) = &stops_mismatch {
            warn!(
                symbol = %m.symbol,
                preset = m.expected_price,
                broker = m.broker_price,
                "MT5: {}", m.opis()
            );
        }

        let mut b = Mt5Bridge {
            tr,
            sym,
            stops_mismatch,
            tag: comment::DEFAULT_TAG.to_string(),
            magic,
            q: Quote {
                ts: 0,
                bid: 0.0,
                ask: 0.0,
            },
            acc: Account {
                balance: 0.0,
                equity: 0.0,
                margin: 0.0,
                free_margin: 0.0,
                leverage: 0,
                credit: 0.0,
            },
            ident: crate::proto::AccountIdent::default(),
            positions: Vec::new(),
            pendings: Vec::new(),
            closed: Vec::new(),
            foreign_pos: Vec::new(),
            foreign_ord: Vec::new(),
            foreign_closed: Vec::new(),
            reason_of: HashMap::new(),
            receipts: ReceiptJournal::default(),
            runtime_entry_hold: None,
            last_trade_outcome_unknown: false,
            last_state: Instant::now() - Duration::from_secs(3600),
            state_interval: Duration::from_millis(500),
            max_retries: 3,
            retry_delay: Duration::from_millis(120),
            rejected_stops: 0,
            requotes: 0,
            market_instead_of_limit: 0,
            send_failures: 0,
            unknown_sends: 0,
        };
        b.refresh_quote()?;
        b.refresh_account()?;
        let rep = b.reconcile()?;
        info!(?rep, "MT5: stan odtworzony z konta");
        b.subscribe_ticks()?;
        Ok(b)
    }

    pub fn transport(&self) -> &Transport {
        &self.tr
    }

    pub fn symbol_info(&self) -> &SymbolInfo {
        &self.sym
    }

    /// Niezgodność `stops_level` między presetem a brokerem, wykryta przy
    /// starcie. Wołający ma to wpisać do dziennika — `warn!` idzie do logu
    /// tekstowego, a ta rzecz jest za groźna, żeby zostać tylko tam.
    pub fn stops_mismatch(&self) -> Option<&StopsMismatch> {
        self.stops_mismatch.as_ref()
    }

    pub fn set_tag(&mut self, tag: impl Into<String>) {
        self.tag = tag.into();
    }

    /// Co ile most sam odświeża pozycje/zlecenia/konto w `poll()`.
    pub fn set_state_interval(&mut self, d: Duration) {
        self.state_interval = d;
    }

    // ============================================================
    //  ODCZYTY
    // ============================================================

    fn subscribe_ticks(&self) -> anyhow::Result<()> {
        self.tr
            .call("subscribe_ticks", json!({ "symbol": self.sym.symbol }))
            .map_err(|e| anyhow::anyhow!("subskrypcja ticków: {e}"))?;
        Ok(())
    }

    pub fn refresh_quote(&mut self) -> anyhow::Result<()> {
        let t: RawTick = self
            .tr
            .call_as("quote", json!({ "symbol": self.sym.symbol }))
            .map_err(|e| anyhow::anyhow!("odczyt kwotowania: {e}"))?;
        if let Some(q) = self.sanitize_tick(&t) {
            self.q = q;
        }
        Ok(())
    }

    pub fn refresh_account(&mut self) -> anyhow::Result<()> {
        let a: RawAccount = self
            .tr
            .call_as("account", Value::Null)
            .map_err(|e| anyhow::anyhow!("odczyt konta: {e}"))?;
        self.acc = Account {
            balance: a.balance,
            equity: a.equity,
            margin: a.margin,
            free_margin: a.margin_free,
            leverage: a.leverage,
            credit: a.credit,
        };
        self.ident = crate::proto::AccountIdent {
            login: a.login,
            server: a.server,
            company: a.company,
            holder: a.holder,
            currency: a.currency,
            leverage: a.leverage,
            trade_mode: a.trade_mode,
        };
        Ok(())
    }

    /// Tożsamość rachunku (numer, serwer, broker, demo/real).
    pub fn ident(&self) -> &crate::proto::AccountIdent {
        &self.ident
    }

    pub fn close_receipt_issue(&self) -> Option<&str> {
        self.runtime_entry_hold.as_deref().or(self.receipts.fault.as_deref()).or_else(|| {
            self.receipts.volumes.scope_mismatch(self.execution_session().as_ref())
                .then_some("oczekiwania wolumenu należą do innej sesji/konta; wymagana rekoncyliacja")
        }).or_else(|| {
            self.receipts.volumes.pending()
                .then_some("ubytek wolumenu pozycji lub transakcja nie ma jeszcze zgodnego snapshotu i pełnego rozliczenia; nowe wejścia HOLD")
        }).or_else(|| {
            (!self.receipts.waiting.is_empty()).then_some("potwierdzenia zamknięć oczekują na odczyt stanu brokera")
        }).or_else(|| {
            self.receipts.reasons.keys().any(|deal| !self.receipts.seen.contains_key(deal))
                .then_some("broker wykonał zamknięcie; oczekiwanie na transakcję rozliczającą")
        }).or_else(|| {
            (!self.closed.is_empty() && self.tr.config().close_receipt_reconcile)
                .then_some("transakcje zamknięte oczekują na zaksięgowanie przez silnik")
        }).or_else(|| {
            (self.tr.config().close_receipt_reconcile && self.tr.close_receipts_waiting())
                .then_some("odebrane potwierdzenia zamknięć oczekują na rekoncyliację")
        })
    }

    pub fn quarantined_positions(&self) -> impl Iterator<Item = &RawPosition> {
        self.receipts.quarantined_positions.values()
    }

    /// Block new market/pending risk without disconnecting, changing costs,
    /// hiding positions, or disabling close/SL/TP/cancel. No generic Resume clears it.
    pub fn hold_new_entries(&mut self, reason: &str) {
        if self.runtime_entry_hold.is_none() {
            warn!(reason, "MT5 runtime entry HOLD; protective operations remain available");
            self.runtime_entry_hold = Some(reason.to_owned());
        }
    }

    pub fn pending_volume_receipts(&self) -> Vec<serde_json::Value> {
        self.receipts.volumes.pending_details()
    }

    fn observe_receipt_volumes(&mut self, raw: &[RawPosition]) {
        if !self.tr.config().close_receipt_reconcile { return; }
        let Some(session)=self.execution_session() else {
            self.receipt_fault("snapshot wolumenu bez potwierdzonej sesji rachunku".into());return;
        };
        let rows:Vec<_>=raw.iter().filter(|r|self.is_ours(r.magic,&r.symbol))
            .map(|r|(r.identifier,r.ticket,r.volume)).collect();
        if let Err(reason)=self.receipts.volumes.observe(&session,&rows) {
            self.receipt_fault(reason);
        }
    }

    fn quarantine_snapshot_if_needed(&mut self, r: &RawPosition) -> bool {
        if !self.tr.config().close_receipt_reconcile { return false; }
        let ours=self.is_ours(r.magic,&r.symbol);
        let invalid_kind=!matches!(r.kind,0|1);
        let invalid_volume=ours && (!r.volume.is_finite() || r.volume<=0.0);
        let prior=ours && self.receipts.quarantined_positions.values().any(|old|
            old.ticket==r.ticket || (r.identifier!=0 && old.identifier==r.identifier));
        let mismatch=ours && self.receipts.positions.get(&r.identifier).is_some_and(|old|
            old.side != if r.kind==0 { Side::Buy } else { Side::Sell });
        let alias_mismatch=ours && self.receipts.aliases.get(&r.ticket).is_some_and(|id|*id!=r.identifier);
        if !invalid_kind && !invalid_volume && !prior && !mismatch && !alias_mismatch { return false; }
        self.receipts.quarantined_positions.entry(r.ticket).or_insert_with(||r.clone());
        self.receipt_fault(format!(
            "kwarantanna pozycji ticket={} identifier={} kind={}: nieznana strona/wolumen, reversal lub sprzeczna tożsamość; automatyczne zarządzanie tą pozycją wyłączone, wymagana rekoncyliacja",
            r.ticket,r.identifier,r.kind));
        true
    }

    fn quarantined_display(r: &RawPosition) -> Option<ForeignPosition> {
        let side=match r.kind { 0=>Side::Buy,1=>Side::Sell,_=>return None };
        Some(ForeignPosition{ticket:r.ticket,symbol:r.symbol.clone(),side,volume:r.volume,
            open_price:r.price_open,open_ts:r.time_msc,sl:opt_px(r.sl),tp:opt_px(r.tp),
            profit:r.profit,magic:r.magic,comment:format!("[QUARANTINE] {}",r.comment),
            origin:Origin::External})
    }

    fn receipt_fault(&mut self, message: String) {
        warn!(%message, "MT5 CLOSE RECEIPT INCOMPLETE: nowe wejścia zablokowane; ochrona istniejących pozycji pozostaje aktywna");
        // Każda druga sprzeczność czyni automatyczne zdjęcie faultu OPEN
        // niedozwolonym, nawet gdy pierwszy komunikat w Option pozostaje bez zmian.
        self.receipts.recoverable_unknown_open_fault = false;
        if self.receipts.fault.is_none() {
            self.receipts.fault = Some(message);
        }
    }

    fn remember_unknown_open(&mut self, r: &OrderReq, machine_comment: &str, reason: String) {
        if !self.tr.config().close_receipt_reconcile { return; }
        let clean = self.receipts.fault.is_none()
            && self.receipts.unknown_opens.is_empty()
            && self.receipts.incomplete.is_empty()
            && self.receipts.quarantined_positions.is_empty();
        self.receipts.unknown_opens.push(UnknownOpenIntent {
            side: r.side,
            requested_volume: r.volume,
            basket: r.basket,
            level: r.level,
            is_toucher: r.is_toucher,
            submitted_quote_ts: self.q.ts,
            machine_comment: machine_comment.to_owned(),
        });
        self.receipts.recoverable_unknown_open_fault = clean;
        if self.receipts.fault.is_none() {
            self.receipts.fault = Some(format!(
                "nierozstrzygnięty OPEN: {reason}; oczekiwanie na dokładnie zgodną pozycję MT5"
            ));
        }
        self.last_state = Instant::now() - self.state_interval;
    }

    /// Snapshot może rozstrzygnąć brak ACK otwarcia bez ponawiania zlecenia.
    /// Wymagamy jednego i tylko jednego kandydata o pełnej maszynowej
    /// tożsamości.  Jakakolwiek inna sprzeczność zachowuje permanentny HOLD.
    fn reconcile_unknown_opens(&mut self) {
        if !self.receipts.recoverable_unknown_open_fault || self.receipts.unknown_opens.is_empty() {
            return;
        }
        let mut matched_tickets = std::collections::HashSet::new();
        let mut resolved = Vec::new();
        for (idx, intent) in self.receipts.unknown_opens.iter().enumerate() {
            let candidates: Vec<_> = self.positions.iter().filter(|p| {
                if matched_tickets.contains(&p.ticket)
                    || p.side != intent.side || p.basket != intent.basket || p.level != intent.level
                    || p.is_toucher != intent.is_toucher
                    || p.volume <= 0.0 || p.volume > intent.requested_volume + 1e-9
                    || p.open_ts + 2_000 < intent.submitted_quote_ts { return false; }
                let same_machine_tag = comment::decode(&self.tag, &p.comment).is_some_and(|tag| {
                    tag.basket == intent.basket && tag.level == intent.level
                        && tag.is_toucher == intent.is_toucher
                });
                let stable_identity = self.receipts.aliases.get(&p.ticket).copied()
                    .filter(|id| *id != 0)
                    .is_some_and(|id| self.receipts.positions.contains_key(&id));
                same_machine_tag && stable_identity
                    && (p.comment == intent.machine_comment
                        || p.comment.starts_with(intent.machine_comment.as_str())
                        || intent.machine_comment.starts_with(p.comment.as_str()))
            }).collect();
            if candidates.len() != 1 { return; }
            matched_tickets.insert(candidates[0].ticket);
            resolved.push(idx);
        }
        if resolved.len() != self.receipts.unknown_opens.len()
            || !self.receipts.incomplete.is_empty()
            || !self.receipts.quarantined_positions.is_empty()
            || self.receipts.volumes.pending()
            || self.receipts.volumes.scope_mismatch(self.execution_session().as_ref())
            || self.tr.receipt_decode_issue().is_some() || !self.tr.is_connected() { return; }
        warn!(count = resolved.len(), "MT5: nierozstrzygnięty OPEN potwierdzony jednoznacznym snapshotem; nowe wejścia odblokowane bez ponowienia zlecenia");
        self.receipts.unknown_opens.clear();
        self.receipts.fault = None;
        self.receipts.recoverable_unknown_open_fault = false;
    }

    fn remember_position(&mut self, identifier: u64, position: Position) {
        if !self.tr.config().close_receipt_reconcile { return; }
        if identifier == 0 {
            self.receipt_fault(format!("brak POSITION_IDENTIFIER dla ticket={}", position.ticket));
            return;
        }
        if let Some(old) = self.receipts.aliases.get(&position.ticket) {
            if *old != identifier {
                self.receipt_fault(format!("sprzeczny identyfikator ticket={}: {} != {identifier}", position.ticket, old));
                return;
            }
        }
        if let Some(old) = self.receipts.positions.get(&identifier) {
            if old.side != position.side || (old.basket.is_some() && position.basket.is_some() && old.basket != position.basket) {
                self.receipt_fault(format!("zmiana właściciela/strony POSITION_IDENTIFIER={identifier}; netting/reversal wymaga osobnej rekoncyliacji"));
                return;
            }
        }
        self.receipts.aliases.insert(position.ticket, identifier);
        if position.basket.is_some() {
            let result=self.execution_session().ok_or_else(||"metadane wolumenu bez potwierdzonej sesji".to_string())
                .and_then(|session|self.receipts.volumes.remember(&session,identifier,position.ticket,position.volume));
            if let Err(reason)=result { self.receipt_fault(reason); }
        }
        self.receipts.positions.insert(identifier, position);
    }

    /// Capture runtime changes made through Broker::positions_mut before removing rows.
    fn remember_live_positions(&mut self) {
        if !self.tr.config().close_receipt_reconcile { return; }
        for p in self.positions.clone() {
            if let Some(id) = self.receipts.aliases.get(&p.ticket).copied() {
                self.remember_position(id, p);
            }
        }
    }

    fn receipt_entry_gate(&self) -> BResult<()> {
        if self.close_receipts_pending() {
            return Err(BrokerError::Rejected);
        }
        Ok(())
    }

    /// Kwotowanie jednostronne (bid albo ask == 0) potrafi przyjść z MT5 przy
    /// otwarciu sesji. Do silnika NIE MOŻE trafić — `Quote::new` by je odrzuciło,
    /// więc brakującą stronę uzupełniamy ostatnim znanym spreadem.
    fn sanitize_tick(&self, t: &RawTick) -> Option<Quote> {
        let (mut bid, mut ask) = (t.bid, t.ask);
        let last_spread = if self.q.ask > self.q.bid {
            self.q.ask - self.q.bid
        } else {
            0.0
        };
        if bid <= 0.0 && ask > 0.0 {
            bid = ask - last_spread;
        }
        if ask <= 0.0 && bid > 0.0 {
            ask = bid + last_spread;
        }
        if ask < bid {
            std::mem::swap(&mut bid, &mut ask);
        }
        Quote::new(t.ts, bid, ask)
    }

    /// Pobiera wyłącznie nowe kwotowania w kolejności napłynięcia.
    ///
    /// Wydzielenie od [`Self::poll_state`] pozwala pętli live najpierw
    /// odtworzyć każdy tick z właściwą kwotą brokera, a dopiero potem wchłonąć
    /// stan terminala z końca paczki.
    pub fn poll_ticks(&mut self) -> Vec<Quote> {
        if (self.tr.config().follow_terminal_account || self.tr.config().close_receipt_reconcile) && !self.tr.is_connected() {
            self.tr.drain_ticks();
            return Vec::new();
        }
        let mut out = Vec::new();
        for t in self.tr.drain_ticks() {
            if let Some(q) = self.sanitize_tick(&t) {
                self.q = q;
                out.push(q);
            }
        }
        out
    }

    /// Pobiera zamknięcia i okresowo odświeża konto, pozycje oraz zlecenia.
    pub fn poll_state(&mut self) {
        let mut raw_closed = self.tr.drain_closed();
        let obce_closed;
        if self.tr.config().close_receipt_reconcile {
            self.remember_live_positions();
            // Closing a bot position manually may produce a deal with magic=0.
            // Its known position ownership is stronger evidence than close-deal magic.
            let mut truly_foreign = Vec::new();
            for c in self.tr.drain_foreign_closed() {
                if self.receipt_position_owned(&c) { raw_closed.push(c); }
                else { truly_foreign.push(c); }
            }
            obce_closed = truly_foreign;
            self.poll_receipts(raw_closed);
        } else {
        if !raw_closed.is_empty() {
            self.absorb_closed(raw_closed);
            // po zamknięciu stan pozycji na pewno jest nieaktualny
            self.last_state = Instant::now() - self.state_interval;
        }
        obce_closed = self.tr.drain_foreign_closed();
        }
        if !obce_closed.is_empty() {
            self.absorb_foreign_closed(obce_closed);
            self.last_state = Instant::now() - self.state_interval;
        }
        if self.last_state.elapsed() >= self.state_interval {
            if let Err(e) = self.refresh_state() {
                warn!(%e, "MT5: odświeżenie stanu nieudane");
            }
        }
    }

    /// Stara, atomowa ścieżka odpytywania. Zostaje dla przełączalnego
    /// odtworzenia zachowania sprzed ścisłej kolejności live.
    pub fn poll(&mut self) -> Vec<Quote> {
        let out = self.poll_ticks();
        self.poll_state();
        out
    }

    /// Ustawia kwotę widoczną przez `Broker::quote()` na właśnie odtwarzany
    /// tick. Wołane wyłącznie przez ścisłą pętlę live przed `Engine::on_tick`.
    pub fn set_replay_quote(&mut self, q: Quote) {
        self.q = q;
    }

    /// Odświeża konto, pozycje i zlecenia oczekujące (scalając z pamięcią bota).
    pub fn refresh_state(&mut self) -> anyhow::Result<()> {
        self.last_state = Instant::now();
        self.refresh_account()?;

        // `all: true` — pytamy o WSZYSTKIE symbole, nie tylko o nasz. Panel jest
        // podglądem całego rachunku; ograniczenie zapytania do `self.sym` ukrywało
        // pozycje na innych instrumentach tak skutecznie, że nie było ich nawet
        // z czego pokazać. Silnik i tak dostanie wyłącznie to, co przejdzie
        // przez `is_ours` (a ten nadal wymaga zgodnego symbolu).
        let raw: Vec<RawPosition> = self
            .tr
            .call_as(
                "positions",
                json!({ "symbol": self.sym.symbol, "all": true }),
            )
            .map_err(|e| anyhow::anyhow!("odczyt pozycji: {e}"))?;
        self.merge_positions(raw);

        let orders: Vec<RawOrder> = self
            .tr
            .call_as("orders", json!({ "symbol": self.sym.symbol, "all": true }))
            .map_err(|e| anyhow::anyhow!("odczyt zleceń: {e}"))?;
        self.merge_pendings(orders);
        Ok(())
    }

    /// Czy bot ma prawo TYM ZARZĄDZAĆ. Świadomie niezmienione: cudzych pozycji
    /// nie dotykamy. Widoczność w panelu jest osobną sprawą — patrz
    /// `foreign_positions()`.
    fn is_ours(&self, magic: i64, symbol: &str) -> bool {
        magic == self.magic && symbol == self.sym.symbol
    }

    /// Pozycje spoza bota — do pokazania, nie do zarządzania.
    pub fn foreign_positions(&self) -> &[ForeignPosition] {
        &self.foreign_pos
    }

    /// Zlecenia oczekujące spoza bota — do pokazania, nie do zarządzania.
    pub fn foreign_orders(&self) -> &[ForeignOrder] {
        &self.foreign_ord
    }

    /// Zamknięte transakcje spoza bota — historia całego rachunku.
    pub fn foreign_closed(&self) -> &[ForeignClosed] {
        &self.foreign_closed
    }

    /* ------------------------------------------------------------------ *
     * RĘCZNE AKCJE NA POZYCJACH SPOZA BOTA
     *
     * Rozróżnienie, na którym stoi bezpieczeństwo tego rachunku:
     *
     *   ZARZĄDZANIE AUTOMATYCZNE  — tylko własny magic (`is_ours`). Cudzej
     *       pozycji nie ruszy ani silnik, ani AI, ani strażnicy ryzyka.
     *       Te pozycje nie mają koszyka i nie wchodzą do księgi wyników bota.
     *
     *   AKCJA RĘCZNA Z PANELU     — dowolny numer zlecenia. To rachunek
     *       użytkownika; jeśli klika „zamknij", ma się zamknąć.
     *
     * Poniższe trzy metody obsługują wyłącznie ten drugi przypadek. Wywołuje
     * je warstwa panelu, nigdy silnik. Świadomie NIE dotykają `self.positions`
     * — tam mieszka księga bota i musi zostać czysta.
     * ------------------------------------------------------------------ */

    fn obca(&self, t: Ticket) -> BResult<(Side, String, f64)> {
        self.foreign_pos
            .iter()
            .find(|p| p.ticket == t)
            .map(|p| (p.side, p.symbol.clone(), p.volume))
            .ok_or(BrokerError::NoSuchTicket)
    }

    /// Ręczna zmiana SL/TP pozycji spoza bota.
    pub fn foreign_modify(&mut self, t: Ticket, sl: Option<Px>, tp: Option<Px>) -> BResult<()> {
        let (side, symbol, _) = self.obca(t)?;
        // Zaokrąglenie i sprawdzenie minimalnego dystansu robimy TYLKO dla
        // własnego instrumentu. Dla obcego nie mamy ani jego kwotowania, ani
        // liczby miejsc po przecinku — zgadywanie skończyłoby się wysłaniem
        // ceny, która nie istnieje. Walidację zostawiamy brokerowi, a jego
        // odmowa i tak trafi do panelu z kodem i treścią.
        let (sl_o, tp_o) = if symbol == self.sym.symbol {
            self.precheck_stops(side, sl, tp)?;
            (
                sl.map(|x| self.sym.round_price(x)),
                tp.map(|x| self.sym.round_price(x)),
            )
        } else {
            (sl, tp)
        };
        self.trade_call(
            "modify_position",
            json!({ "ticket": t, "sl": sl_o, "tp": tp_o }),
        )?;
        if let Some(p) = self.foreign_pos.iter_mut().find(|p| p.ticket == t) {
            p.sl = sl_o;
            p.tp = tp_o;
        }
        Ok(())
    }

    /// Ręczne zamknięcie całej pozycji spoza bota. Zwraca zrealizowany wynik.
    pub fn foreign_close(&mut self, t: Ticket) -> BResult<f64> {
        self.obca(t)?;
        let v = self.trade_call("close_position", json!({ "ticket": t }))?;
        let res: crate::proto::SendResult =
            serde_json::from_value(v).map_err(|_| BrokerError::Rejected)?;
        self.foreign_pos.retain(|p| p.ticket != t);
        Ok(res.profit)
    }

    /// Ręczne zamknięcie CZĘŚCI pozycji spoza bota.
    pub fn foreign_close_partial(&mut self, t: Ticket, volume: f64) -> BResult<f64> {
        let (_, symbol, cur) = self.obca(t)?;
        if !volume.is_finite() || volume <= 0.0 {
            return Err(BrokerError::InvalidVolume);
        }
        // krok wolumenu znamy tylko dla własnego instrumentu
        let vol = if symbol == self.sym.symbol {
            self.sym.round_volume(volume)
        } else {
            volume
        };
        // resztka poniżej minimalnego lota byłaby niehandlowalna — wtedy
        // zamykamy całość, zamiast zostawiać ogarek, którego nikt nie ruszy
        if symbol == self.sym.symbol
            && (vol >= cur - 1e-9 || cur - vol < self.sym.volume_min - 1e-9)
        {
            return self.foreign_close(t);
        }
        if vol >= cur - 1e-9 {
            return self.foreign_close(t);
        }
        let v = self.trade_call("close_partial", json!({ "ticket": t, "volume": vol }))?;
        let res: crate::proto::SendResult =
            serde_json::from_value(v).map_err(|_| BrokerError::Rejected)?;
        if let Some(p) = self.foreign_pos.iter_mut().find(|p| p.ticket == t) {
            p.volume = (p.volume - vol).max(0.0);
        }
        Ok(res.profit)
    }

    /// Czy ten numer zlecenia to pozycja spoza bota. Warstwa panelu pyta o to
    /// zanim wybierze ścieżkę wykonania.
    pub fn is_foreign(&self, t: Ticket) -> bool {
        self.foreign_pos.iter().any(|p| p.ticket == t)
    }

    /// To samo dla zleceń oczekujących.
    pub fn is_foreign_order(&self, t: Ticket) -> bool {
        self.foreign_ord.iter().any(|o| o.ticket == t)
    }

    /// Numery WSZYSTKICH cudzych zleceń oczekujących — do operacji zbiorczych.
    pub fn foreign_order_tickets(&self) -> Vec<Ticket> {
        self.foreign_ord.iter().map(|o| o.ticket).collect()
    }

    /// Ręczne skasowanie zlecenia oczekującego spoza bota.
    pub fn foreign_cancel_pending(&mut self, t: Ticket) -> BResult<()> {
        if !self.is_foreign_order(t) {
            return Err(BrokerError::NoSuchTicket);
        }
        self.trade_call("cancel_pending", json!({ "ticket": t }))?;
        self.foreign_ord.retain(|o| o.ticket != t);
        Ok(())
    }

    /// Ręczna zmiana ceny/SL/TP zlecenia oczekującego spoza bota.
    pub fn foreign_modify_pending(
        &mut self,
        t: Ticket,
        price: Px,
        sl: Option<Px>,
        tp: Option<Px>,
    ) -> BResult<()> {
        let symbol = self
            .foreign_ord
            .iter()
            .find(|o| o.ticket == t)
            .map(|o| o.symbol.clone())
            .ok_or(BrokerError::NoSuchTicket)?;
        // zaokrąglamy tylko dla własnego instrumentu — patrz `foreign_modify`
        let nasz = symbol == self.sym.symbol;
        let (cena, sl_o, tp_o) = if nasz {
            (
                self.sym.round_price(price),
                sl.map(|x| self.sym.round_price(x)),
                tp.map(|x| self.sym.round_price(x)),
            )
        } else {
            (price, sl, tp)
        };
        self.trade_call(
            "modify_pending",
            json!({ "ticket": t, "price": cena, "sl": sl_o, "tp": tp_o }),
        )?;
        if let Some(o) = self.foreign_ord.iter_mut().find(|o| o.ticket == t) {
            o.price = cena;
            o.sl = sl_o;
            o.tp = tp_o;
        }
        Ok(())
    }

    /// Przetwarza cudze zamknięcia. Świadomie NIE dotyka `self.closed`:
    /// tam mieszka księga wyników bota i musi zostać czysta.
    fn absorb_foreign_closed(&mut self, raw: Vec<RawClosed>) {
        for c in raw {
            // deal zamykający ma stronę ODWROTNĄ do pozycji
            let side = if c.deal_type == 0 {
                Side::Sell
            } else {
                Side::Buy
            };
            self.foreign_closed.push(ForeignClosed {
                ticket: c.position,
                symbol: c.symbol.clone(),
                side,
                volume: c.volume,
                open_price: if c.price_open > 0.0 {
                    c.price_open
                } else {
                    c.price
                },
                close_price: c.price,
                open_ts: if c.time_open_msc > 0 {
                    c.time_open_msc
                } else {
                    c.time_msc
                },
                close_ts: c.time_msc,
                profit: c.profit,
                commission: c.commission,
                swap: c.swap,
                magic: c.magic,
                comment: c.comment.clone(),
                origin: Origin::classify(c.magic, self.magic, false),
            });
        }
        // podgląd, nie księgowość — trzymamy rozsądny ogon
        if self.foreign_closed.len() > 1000 {
            let nadmiar = self.foreign_closed.len() - 1000;
            self.foreign_closed.drain(..nadmiar);
        }
    }

    /// Scalenie faktów brokera z pamięcią bota. Pola bota przeżywają; pola
    /// brokera (wolumen, SL, TP, cena otwarcia) są nadpisywane, bo to on ma rację.
    fn merge_positions(&mut self, raw: Vec<RawPosition>) {
        self.remember_live_positions();
        self.observe_receipt_volumes(&raw);
        let mut seen: Vec<Ticket> = Vec::with_capacity(raw.len());
        let mut obce: Vec<ForeignPosition> = Vec::new();
        for r in raw {
            if self.quarantine_snapshot_if_needed(&r) {
                if let Some(display)=Self::quarantined_display(&r) { obce.push(display); }
                continue;
            }
            if !self.is_ours(r.magic, &r.symbol) {
                // NIE zarządzamy — ale pokazujemy. Zysk bierzemy prosto od
                // brokera: dla obcego symbolu nie mamy własnego kwotowania.
                obce.push(ForeignPosition {
                    ticket: r.ticket,
                    symbol: r.symbol.clone(),
                    side: if r.kind == 0 { Side::Buy } else { Side::Sell },
                    volume: r.volume,
                    open_price: r.price_open,
                    open_ts: r.time_msc,
                    sl: opt_px(r.sl),
                    tp: opt_px(r.tp),
                    profit: r.profit,
                    magic: r.magic,
                    comment: r.comment.clone(),
                    origin: Origin::classify(r.magic, self.magic, false),
                });
                continue;
            }
            seen.push(r.ticket);
            let side = if r.kind == 0 { Side::Buy } else { Side::Sell };
            let sl = opt_px(r.sl);
            let tp = opt_px(r.tp);
            if self.tr.config().close_receipt_reconcile && !self.positions.iter().any(|p| p.ticket == r.ticket) {
                if let Some(mut old) = self.receipts.positions.get(&r.identifier).cloned() {
                    old.ticket = r.ticket;
                    self.positions.push(old);
                }
            }
            if let Some(p) = self.positions.iter_mut().find(|p| p.ticket == r.ticket) {
                p.volume = r.volume;
                p.open_price = r.price_open;
                if self.tr.config().close_receipt_reconcile { p.open_ts = r.time_msc; }
                p.sl = sl;
                p.tp = tp;
                // uwaga: `frozen`, `vsl`, `peak_pts`, `is_runner`, `basket`,
                // `level` NIE są dotykane — to jest pamięć silnika
            } else {
                let tg = comment::decode(&self.tag, &r.comment).unwrap_or_default();
                debug!(
                    ticket = r.ticket,
                    "MT5: pozycja spoza pamięci bota — adoptowana"
                );
                self.positions.push(Position {
                    ticket: r.ticket,
                    side,
                    volume: r.volume,
                    open_price: r.price_open,
                    open_ts: r.time_msc,
                    sl,
                    tp,
                    vsl: None,
                    basket: tg.basket,
                    level: tg.level,
                    frozen: false,
                    peak_pts: 0.0,
                    last_peak_ts: r.time_msc,
                    is_runner: tp.is_none(),
                    is_toucher: tg.is_toucher,
                    comment: r.comment.clone(),
                });
            }
            if self.tr.config().close_receipt_reconcile {
                if let Some(p) = self.positions.iter().find(|p| p.ticket == r.ticket).cloned() {
                    self.remember_position(r.identifier, p);
                }
            }
        }
        // pozycji, których broker już nie widzi, u nas też nie ma
        self.positions.retain(|p| seen.contains(&p.ticket));
        obce.sort_by_key(|p| p.ticket);
        self.foreign_pos = obce;
        self.reconcile_unknown_opens();
    }

    fn merge_pendings(&mut self, raw: Vec<RawOrder>) {
        let mut fresh = Vec::with_capacity(raw.len());
        let mut obce: Vec<ForeignOrder> = Vec::new();
        for r in raw {
            if !self.is_ours(r.magic, &r.symbol) {
                if let Some(kind) = pending_kind(r.kind) {
                    obce.push(ForeignOrder {
                        ticket: r.ticket,
                        symbol: r.symbol.clone(),
                        kind,
                        volume: r.volume,
                        price: r.price_open,
                        sl: opt_px(r.sl),
                        tp: opt_px(r.tp),
                        placed_ts: r.time_msc,
                        magic: r.magic,
                        comment: r.comment.clone(),
                        origin: Origin::classify(r.magic, self.magic, false),
                    });
                }
                continue;
            }
            let Some(kind) = pending_kind(r.kind) else {
                continue;
            };
            let old = self.pendings.iter().find(|o| o.ticket == r.ticket);
            let tg = comment::decode(&self.tag, &r.comment).unwrap_or_default();
            fresh.push(PendingOrder {
                ticket: r.ticket,
                kind,
                volume: r.volume,
                price: r.price_open,
                sl: opt_px(r.sl),
                tp: opt_px(r.tp),
                placed_ts: r.time_msc,
                basket: old.and_then(|o| o.basket).or(tg.basket),
                level: old.map(|o| o.level).unwrap_or(tg.level),
                frozen: old.map(|o| o.frozen).unwrap_or(false),
                is_toucher: old.map(|o| o.is_toucher).unwrap_or(tg.is_toucher),
                comment: r.comment.clone(),
                is_topup: false,
            });
        }
        self.pendings = fresh;
        obce.sort_by_key(|o| o.ticket);
        self.foreign_ord = obce;
    }

    /// Odtworzenie stanu po restarcie bota. Czyści pamięć i buduje ją od nowa
    /// wyłącznie z tego, co widać na koncie.
    pub fn reconcile(&mut self) -> anyhow::Result<ReconcileReport> {
        let mut rep = ReconcileReport::default();
        self.remember_live_positions();

        let raw: Vec<RawPosition> = self
            .tr
            .call_as(
                "positions",
                json!({ "symbol": self.sym.symbol, "all": true }),
            )
            .map_err(|e| anyhow::anyhow!("rekoncyliacja pozycji: {e}"))?;
        self.observe_receipt_volumes(&raw);
        self.positions.clear();
        if self.tr.config().close_receipt_reconcile { self.foreign_pos.clear(); }
        for r in raw {
            if self.quarantine_snapshot_if_needed(&r) {
                rep.orphans+=1;
                if let Some(display)=Self::quarantined_display(&r) { self.foreign_pos.push(display); }
                continue;
            }
            if !self.is_ours(r.magic, &r.symbol) {
                rep.foreign += 1;
                continue;
            }
            let tg = comment::decode(&self.tag, &r.comment);
            if tg.is_none() {
                rep.orphans += 1;
            }
            let tg = tg.unwrap_or_default();
            if let Some(b) = tg.basket {
                if !rep.baskets.contains(&b) {
                    rep.baskets.push(b);
                }
            }
            let tp = opt_px(r.tp);
            self.positions.push(Position {
                ticket: r.ticket,
                side: if r.kind == 0 { Side::Buy } else { Side::Sell },
                volume: r.volume,
                open_price: r.price_open,
                open_ts: r.time_msc,
                sl: opt_px(r.sl),
                tp,
                // wirtualny SL żyje tylko w pamięci bota i restartu NIE przeżywa
                vsl: None,
                basket: tg.basket,
                level: tg.level,
                frozen: false,
                peak_pts: 0.0,
                last_peak_ts: r.time_msc,
                is_runner: tp.is_none(),
                is_toucher: tg.is_toucher,
                comment: r.comment.clone(),
            });
            if self.tr.config().close_receipt_reconcile {
                self.remember_position(r.identifier, self.positions.last().unwrap().clone());
            }
            rep.positions += 1;
        }

        let orders: Vec<RawOrder> = self
            .tr
            .call_as("orders", json!({ "symbol": self.sym.symbol, "all": true }))
            .map_err(|e| anyhow::anyhow!("rekoncyliacja zleceń: {e}"))?;
        self.pendings.clear();
        for r in orders {
            if !self.is_ours(r.magic, &r.symbol) {
                rep.foreign += 1;
                continue;
            }
            let Some(kind) = pending_kind(r.kind) else {
                continue;
            };
            let tg = comment::decode(&self.tag, &r.comment);
            if tg.is_none() {
                rep.orphans += 1;
            }
            let tg = tg.unwrap_or_default();
            if let Some(b) = tg.basket {
                if !rep.baskets.contains(&b) {
                    rep.baskets.push(b);
                }
            }
            self.pendings.push(PendingOrder {
                ticket: r.ticket,
                kind,
                volume: r.volume,
                price: r.price_open,
                sl: opt_px(r.sl),
                tp: opt_px(r.tp),
                placed_ts: r.time_msc,
                basket: tg.basket,
                level: tg.level,
                frozen: false,
                is_toucher: tg.is_toucher,
                comment: r.comment.clone(),
                is_topup: false,
            });
            rep.pendings += 1;
        }
        rep.baskets.sort_unstable();
        Ok(rep)
    }

    /// Przetwarza zdarzenia zamknięcia z sidecara na `ClosedTrade`.
    fn absorb_closed(&mut self, raw: Vec<RawClosed>) {
        for c in raw {
            if c.magic != self.magic {
                continue;
            }
            // deal zamykający ma stronę ODWROTNĄ do pozycji
            let side = if c.deal_type == 0 {
                Side::Sell
            } else {
                Side::Buy
            };
            let known = self
                .positions
                .iter()
                .find(|p| p.ticket == c.position)
                .cloned();
            let reason = self
                .reason_of
                .remove(&c.position)
                .unwrap_or_else(|| deal_reason(c.reason));
            let open_price = if c.price_open > 0.0 {
                c.price_open
            } else {
                known.as_ref().map(|p| p.open_price).unwrap_or(c.price)
            };
            let open_ts = if c.time_open_msc > 0 {
                c.time_open_msc
            } else {
                known.as_ref().map(|p| p.open_ts).unwrap_or(c.time_msc)
            };
            self.closed.push(ClosedTrade {
                profit_basis: None, cost_receipt: None,
                ticket: c.position,
                side,
                volume: c.volume,
                open_price,
                close_price: c.price,
                open_ts,
                close_ts: c.time_msc,
                profit: c.profit,
                commission: c.commission,
                swap: c.swap,
                reason,
                basket: known.as_ref().and_then(|p| p.basket),
            });
            // częściowe zamknięcie zmniejsza wolumen, pełne — usuwa pozycję
            if let Some(i) = self.positions.iter().position(|p| p.ticket == c.position) {
                let left = self.positions[i].volume - c.volume;
                if left <= 1e-9 {
                    self.positions.remove(i);
                } else {
                    self.positions[i].volume = left;
                }
            }
        }
    }

    /// Receipt processing never applies a second volume subtraction. A fresh
    /// broker snapshot is taken first; metadata survives that snapshot's removals.
    fn receipt_position_owned(&self, c: &RawClosed) -> bool {
        c.symbol == self.sym.symbol && self.receipts.positions.get(&c.position)
            .is_some_and(|p| p.basket.is_some())
    }

    fn poll_receipts(&mut self, raw: Vec<RawClosed>) {
        if self.receipts.fault.is_none() {
            if let Some(issue) = self.tr.receipt_decode_issue() { self.receipt_fault(issue); }
        }
        self.remember_live_positions();
        for c in raw {
            if self.is_ours(c.magic, &c.symbol) || self.receipt_position_owned(&c) {
                self.receipts.waiting.push(c);
            }
        }
        if self.receipts.waiting.is_empty() { return; }
        if let Err(e) = self.refresh_state() {
            warn!(%e, waiting = self.receipts.waiting.len(), "MT5: potwierdzenia czekają na autorytatywny stan pozycji");
            return;
        }
        for mut c in std::mem::take(&mut self.receipts.waiting) {
            // OFF retains the pre-cost economic fingerprint, including when a
            // newer external producer sends metadata this instance does not use.
            if !self.tr.config().closed_profit_net_costs { c.cost_receipt = None; }
            if c.deal == 0 || c.position == 0 || !c.volume.is_finite() || c.volume <= 0.0
                || !c.price.is_finite() || c.price <= 0.0 || !c.profit.is_finite()
                || !c.commission.is_finite() || !c.swap.is_finite() || !matches!(c.deal_type, 0 | 1) {
                self.receipt_fault(format!("niepełne potwierdzenie deal={} position={}", c.deal, c.position));
                // In particular, never put deal=0 in the deduplication key set.
                self.receipts.incomplete.push(c);
                continue;
            }
            if let Some(old) = self.receipts.seen.get(&c.deal) {
                if old != &c {
                    self.receipt_fault(format!("sprzeczne powtórzenie deal={}", c.deal));
                    self.receipts.incomplete.push(c);
                }
                continue;
            }
            let Some(known) = self.receipts.positions.get(&c.position).cloned() else {
                self.receipt_fault(format!("brak metadanych POSITION_IDENTIFIER={} dla deal={}", c.position, c.deal));
                self.receipts.incomplete.push(c);
                continue;
            };
            let side = if c.deal_type == 0 { Side::Sell } else { Side::Buy };
            if known.basket.is_none() || known.side != side {
                self.receipt_fault(format!("niepotwierdzony właściciel/strona deal={} position={}", c.deal, c.position));
                self.receipts.incomplete.push(c);
                continue;
            }
            let reason = match self.receipts.reasons.get(&c.deal) {
                Some((id, reason)) if *id == c.position => *reason,
                Some(_) => {
                    self.receipt_fault(format!("potwierdzenie RPC wskazuje inną pozycję dla deal={}", c.deal));
                    self.receipts.incomplete.push(c);
                    continue;
                }
                None => deal_reason(c.reason),
            };
            let trade = ClosedTrade {
                profit_basis: None, cost_receipt: None,
                ticket: known.ticket, side, volume: c.volume,
                open_price: if c.price_open > 0.0 { c.price_open } else { known.open_price },
                close_price: c.price,
                open_ts: if c.time_open_msc > 0 { c.time_open_msc } else { known.open_ts },
                close_ts: c.time_msc, profit: c.profit, commission: c.commission,
                swap: c.swap, reason, basket: known.basket,
            };
            let trade = if self.tr.config().closed_profit_net_costs {
                let receipt = self.execution_session().ok_or_else(|| "unverified account scope for cost receipt".to_string())
                    .and_then(|session| crate::cost_adapter::closed_receipt(&c, &session.scope, &self.ident.currency, known.open_ts))
                    .and_then(|receipt| trade.with_cost_receipt(receipt).map_err(|e|e.to_string()));
                match receipt {
                    Ok(trade) => trade,
                    Err(reason) => {
                        self.receipt_fault(format!("koszty niepotwierdzone deal={}: {reason}", c.deal));
                        self.receipts.incomplete.push(c);
                        continue;
                    }
                }
            } else { trade };
            let volume_result=self.execution_session().ok_or_else(||"rozliczenie wolumenu bez potwierdzonej sesji".to_string())
                .and_then(|session|self.receipts.volumes.accept_close(&session,c.position,c.volume));
            if let Err(reason)=volume_result {
                self.receipt_fault(reason);self.receipts.incomplete.push(c);continue;
            }
            self.closed.push(trade);
            self.receipts.seen.insert(c.deal, c);
        }
    }

    fn apply_close_receipt(&mut self, ticket: Ticket, reason: CloseReason, res: &crate::proto::SendResult) {
        self.remember_live_positions();
        let id = self.receipts.aliases.get(&ticket).copied().unwrap_or(0);
        // PUPrime (and some other MT5 servers) can execute a market close while
        // omitting POSITION_IDENTIFIER from the immediate MqlTradeResult.  The
        // physical ticket was verified before sending and is already mapped to
        // a stable identifier in `aliases`; a non-zero, matching identifier is
        // still required whenever the broker does send one.  With a missing ID
        // we keep the receipt barrier TEMPORARY and let the exact closed-deal
        // event prove the stable identifier before any PnL is booked.  This
        // avoids both the old permanent live deadlock and any ticket guessing.
        let ack_identifier_conflict = res.position_identifier != 0 && id != res.position_identifier;
        if id == 0 || ack_identifier_conflict || res.deal == 0 || res.position != ticket {
            self.receipt_fault(format!("niepełne/sprzeczne ACK zamknięcia ticket={ticket} deal={} identifier={}", res.deal, res.position_identifier));
            // The RPC may really have executed, but an uncorrelated receipt
            // must not mutate the wrong cached ticket. Defer the snapshot to
            // poll_state OUTSIDE routing::Widok (which temporarily hides other formats).
            self.last_state = Instant::now() - self.state_interval;
            return;
        }
        if res.position_identifier == 0 {
            warn!(ticket, deal = res.deal, expected_identifier = id,
                "MT5: ACK zamknięcia bez POSITION_IDENTIFIER; oczekiwanie na dokładny deal z historii");
        }
        if let Some((old_id, old_reason)) = self.receipts.reasons.get(&res.deal) {
            if *old_id != id || *old_reason != reason {
                self.receipt_fault(format!("sprzeczne ACK deal={}", res.deal));
            }
            // A duplicate ACK must not subtract volume twice either.
            return;
        } else {
            self.receipts.reasons.insert(res.deal, (id, reason));
        }
        if let Some(i) = self.positions.iter().position(|p| p.ticket == ticket) {
            let cur = self.positions[i].volume;
            if res.volume.is_finite() && res.volume > 0.0 && res.volume <= cur + 1e-9 {
                let left = (cur - res.volume).max(0.0);
                if left <= 1e-9 { self.positions.remove(i); }
                else { self.positions[i].volume = left; }
            } else {
                self.receipt_fault(format!("ACK bez poprawnego wykonanego wolumenu ticket={ticket}: {} z {cur}", res.volume));
                self.last_state = Instant::now() - self.state_interval;
            }
        }
    }

    // ============================================================
    //  WYSYŁKA
    // ============================================================

    fn decode_trade_ack(&mut self, cmd: &'static str, value: Value) -> BResult<crate::proto::SendResult> {
        match serde_json::from_value(value) {
            Ok(result)=>Ok(result),
            Err(e)=>{
                self.last_trade_outcome_unknown = true;
                if self.tr.config().close_receipt_reconcile {
                    self.unknown_sends+=1;
                    self.last_state=Instant::now()-self.state_interval;
                    if cmd != "open_market" {
                        self.receipt_fault(format!("nieczytelne potwierdzenie {cmd}: {e}; wykonanie wymaga rekoncyliacji"));
                    }
                }
                Err(BrokerError::Rejected)
            }
        }
    }

    /// Wywołanie handlowe z ponawianiem TYLKO tam, gdzie to bezpieczne.
    ///
    /// Timeout przy zleceniu handlowym **nie jest** ponawiany: nie wiemy, czy
    /// zlecenie doszło do serwera. Ponowienie mogłoby otworzyć drugą pozycję.
    /// Zamiast tego wymuszamy odświeżenie stanu — rzeczywistość rozstrzygnie.
    fn trade_call(&mut self, cmd: &'static str, args: Value) -> BResult<Value> {
        self.last_trade_outcome_unknown = false;
        let mut tries = 0u32;
        loop {
            match self.tr.call(cmd, args.clone()) {
                Ok(v) => return Ok(v),
                Err(CallError::Broker(e)) => {
                    if errors::is_no_op(e.code) {
                        return Ok(Value::Null);
                    }
                    if e.code == retcode::REQUOTE || e.code == retcode::PRICE_CHANGED {
                        self.requotes += 1;
                    }
                    if e.code == retcode::INVALID_STOPS {
                        self.rejected_stops += 1;
                    }
                    if errors::is_retryable(e.code) && tries < self.max_retries {
                        tries += 1;
                        debug!(
                            cmd,
                            code = e.code,
                            name = errors::name(e.code),
                            tries,
                            "MT5: ponawiam"
                        );
                        std::thread::sleep(self.retry_delay);
                        // odśwież cenę — requote znaczy, że nasza była stara
                        let _ = self.refresh_quote();
                        continue;
                    }
                    self.send_failures += 1;
                    warn!(cmd, code = e.code, name = errors::name(e.code), msg = %e.msg, "MT5: odmowa");
                    return Err(errors::classify(e.code));
                }
                Err(CallError::Timeout(d)) => {
                    self.last_trade_outcome_unknown = true;
                    self.unknown_sends += 1;
                    self.last_state = Instant::now() - self.state_interval;
                    if self.tr.config().close_receipt_reconcile
                        && matches!(cmd,"place_pending"|"close_position"|"close_partial"|"cancel_pending") {
                        self.receipt_fault(format!("nieznany wynik {cmd}: timeout po wysłaniu; odczyt stanu nie zastępuje potwierdzenia wykonania"));
                    }
                    warn!(
                        cmd,
                        ?d,
                        "MT5: BRAK ODPOWIEDZI — wynik zlecenia NIEZNANY, wymuszam rekoncyliację"
                    );
                    return Err(BrokerError::Rejected);
                }
                Err(e) => {
                    self.last_trade_outcome_unknown = true;
                    self.send_failures += 1;
                    if self.tr.config().close_receipt_reconcile
                        && matches!(cmd,"place_pending"|"close_position"|"close_partial"|"cancel_pending") {
                        self.receipt_fault(format!("nieznany wynik {cmd}: {e}; wymagane potwierdzenie wykonania przed nowymi wejściami"));
                        self.last_state = Instant::now() - self.state_interval;
                    }
                    warn!(cmd, %e, "MT5: wywołanie nieudane");
                    return Err(BrokerError::Rejected);
                }
            }
        }
    }

    /// Sprawdzenie SL/TP PRZED wysłaniem — dokładnie ta sama reguła, co
    /// w symulatorze. Broker odrzuciłby to samo, tylko dwie setne sekundy później.
    fn precheck_stops(&mut self, side: Side, sl: Option<Px>, tp: Option<Px>) -> BResult<()> {
        let stops = self.sym.stops_level_price();
        if let Some(s) = sl {
            if !sl_is_valid(side, s, &self.q, stops) {
                self.rejected_stops += 1;
                return Err(BrokerError::InvalidStops);
            }
        }
        if let Some(t) = tp {
            if !tp_is_valid(side, t, &self.q, stops) {
                self.rejected_stops += 1;
                return Err(BrokerError::InvalidStops);
            }
        }
        Ok(())
    }

    fn norm_volume(&self, v: f64) -> BResult<f64> {
        if !v.is_finite() || v <= 0.0 {
            return Err(BrokerError::InvalidVolume);
        }
        let r = self.sym.round_volume(v);
        if r < self.sym.volume_min - 1e-9 {
            return Err(BrokerError::InvalidVolume);
        }
        Ok(r)
    }
}

#[inline]
fn opt_px(v: f64) -> Option<Px> {
    if v > 0.0 && v.is_finite() {
        Some(v)
    } else {
        None
    }
}

/// `ORDER_TYPE_*` → nasz `PendingKind`. Zlecenia rynkowe (0/1) i stop-limity
/// (6/7) nie mają odpowiednika — bot ich nie wystawia.
#[inline]
/// Porównuje `stops_level` z serwera z wartością, której spodziewa się preset.
///
/// Tolerancja wynosi `point / 2`, czyli na złocie **0,005 w jednostkach ceny**
/// (pół setnej dolara) — a NIE „pół punktu `stops_level`", co czyta się jako
/// 0,5 i jest sto razy więcej. Rozjazd 0,50 wobec 0,20 alarm wywoła.
///
/// Zero byłoby złym progiem: obie liczby przechodzą przez zapis dziesiętny
/// i przez `f64`, więc wymaganie równości co do bitu dawałoby fałszywe alarmy
/// przy wartościach identycznych na oko. Połowa punktu jest mniejsza niż
/// jakakolwiek realna zmiana warunków brokera — te idą po całych punktach.
fn sprawdz_stops_level(sym: &SymbolInfo, oczekiwane: Option<f64>) -> Option<StopsMismatch> {
    let expected_price = oczekiwane?;
    // Wartość ujemna albo niebędąca liczbą znaczy „nie ustawiono" — nie ma
    // z czym porównywać i nie ma powodu do alarmu.
    if !expected_price.is_finite() || expected_price < 0.0 {
        return None;
    }
    let broker_price = sym.stops_level_price();
    let tolerancja = sym.point * 0.5;
    if (broker_price - expected_price).abs() <= tolerancja {
        return None;
    }
    Some(StopsMismatch {
        symbol: sym.symbol.clone(),
        expected_price,
        broker_price,
        expected_points: if sym.point > 0.0 {
            expected_price / sym.point
        } else {
            0.0
        },
        broker_points: sym.stops_level_points,
    })
}

pub fn pending_kind(k: i32) -> Option<PendingKind> {
    match k {
        2 => Some(PendingKind::BuyLimit),
        3 => Some(PendingKind::SellLimit),
        4 => Some(PendingKind::BuyStop),
        5 => Some(PendingKind::SellStop),
        _ => None,
    }
}

#[inline]
pub fn pending_kind_code(k: PendingKind) -> i32 {
    match k {
        PendingKind::BuyLimit => 2,
        PendingKind::SellLimit => 3,
        PendingKind::BuyStop => 4,
        PendingKind::SellStop => 5,
    }
}

/// `DEAL_REASON_*` → nasz powód zamknięcia, gdy bot go nie zapamiętał
/// (np. SL wykonany przez serwer, gdy bot był offline).
#[inline]
pub fn deal_reason(r: i32) -> CloseReason {
    match r {
        4 => CloseReason::Sl,
        5 => CloseReason::Tp,
        6 => CloseReason::MaxDd, // stop out
        _ => CloseReason::Manual,
    }
}

// ============================================================
//  BROKER
// ============================================================

impl Broker for Mt5Bridge {
    fn quote(&self) -> Quote {
        self.q
    }

    fn account(&self) -> Account {
        self.acc
    }

    fn stops_level(&self) -> f64 {
        self.sym.stops_level_price()
    }

    fn volume_min(&self) -> f64 {
        self.sym.volume_min
    }

    fn volume_step(&self) -> f64 {
        self.sym.volume_step
    }

    fn volume_max(&self) -> f64 {
        self.sym.volume_max
    }

    fn close_receipt_reconciliation_active(&self) -> bool {
        self.tr.config().close_receipt_reconcile
    }

    fn cost_net_supported(&self) -> bool {
        self.tr.config().closed_profit_net_costs && self.tr.config().close_receipt_reconcile
            && self.tr.is_connected() && self.receipts.fault.is_none()
            && self.tr.receipt_decode_issue().is_none()
    }
    fn report_cost_consumer_fault(&mut self, reason: &str) {
        if self.tr.config().closed_profit_net_costs {
            self.receipt_fault(format!("kanoniczny ledger kosztów odrzucony przez konsumenta: {reason}"));
        }
    }

    fn close_receipts_pending(&self) -> bool {
        self.runtime_entry_hold.is_some() || (self.close_receipt_reconciliation_active()
            && (self.close_receipt_issue().is_some() || self.tr.receipt_decode_issue().is_some())
        )
    }

    fn receipt_barrier(&self) -> conduit_core::broker::ReceiptBarrier {
        use conduit_core::broker::ReceiptBarrier;
        if self.runtime_entry_hold.is_some() || (self.close_receipt_reconciliation_active()
            && (self.receipts.fault.is_some() || self.tr.receipt_decode_issue().is_some() || !self.tr.is_connected()
                || self.receipts.volumes.scope_mismatch(self.execution_session().as_ref()))) {
            ReceiptBarrier::RequiresReview
        } else if self.close_receipts_pending() { ReceiptBarrier::Temporary }
        else { ReceiptBarrier::Clear }
    }

    fn execution_session(&self) -> Option<conduit_core::broker::ExecutionSession> {
        if !self.tr.is_connected() || self.ident.login == 0 || self.ident.server.is_empty()
            || !(self.tr.config().follow_terminal_account || self.tr.config().close_receipt_reconcile) { return None; }
        Some(conduit_core::broker::ExecutionSession {
            scope: serde_json::json!([self.ident.login, self.ident.server, self.ident.trade_mode,
                self.tr.config().magic, self.sym.symbol]).to_string(),
            generation: self.tr.execution_generation(),
        })
    }

    fn position_identifier(&self, ticket: Ticket) -> Option<u64> {
        // Only a presently observed/ACK-confirmed owned position in the same
        // verified session is evidence for restoration. Old ticket aliases,
        // an arbitrary matching ticket number, and faulted identity are not.
        if !self.tr.config().close_receipt_reconcile || self.receipts.fault.is_some()
            || self.tr.receipt_decode_issue().is_some() { return None; }
        let session=self.execution_session()?;
        if self.receipts.volumes.scope_mismatch(Some(&session))
            || !self.positions.iter().any(|p|p.ticket==ticket && p.basket.is_some()) { return None; }
        let id=*self.receipts.aliases.get(&ticket)?;
        if id==0 || self.receipts.quarantined_positions.values()
            .any(|p|p.ticket==ticket || p.identifier==id) { return None; }
        self.receipts.positions.get(&id)
            .filter(|p|p.ticket==ticket && p.basket.is_some()).map(|_|id)
    }

    fn positions(&self) -> &[Position] {
        &self.positions
    }

    fn pendings(&self) -> &[PendingOrder] {
        &self.pendings
    }

    fn positions_mut(&mut self) -> &mut Vec<Position> {
        &mut self.positions
    }

    fn pendings_mut(&mut self) -> &mut Vec<PendingOrder> {
        &mut self.pendings
    }

    fn open_market(&mut self, r: OrderReq) -> BResult<Ticket> {
        self.receipt_entry_gate()?;
        let vol = self.norm_volume(r.volume)?;
        self.precheck_stops(r.side, r.sl, r.tp)?;
        let cm = comment::encode(&self.tag, r.basket, r.level, r.is_toucher, &r.comment);
        let args = json!({
            "symbol": self.sym.symbol,
            "side": if r.side == Side::Buy { "buy" } else { "sell" },
            "volume": vol,
            "sl": r.sl.map(|x| self.sym.round_price(x)),
            "tp": r.tp.map(|x| self.sym.round_price(x)),
            "comment": cm,
        });
        let v = match self.trade_call("open_market", args) {
            Ok(v) => v,
            Err(e) => {
                if self.last_trade_outcome_unknown {
                    self.remember_unknown_open(&r, &cm, "brak rozstrzygającej odpowiedzi transportu".into());
                }
                return Err(e);
            }
        };
        let res = match self.decode_trade_ack("open_market",v) {
            Ok(res) => res,
            Err(e) => {
                self.remember_unknown_open(&r, &cm, "nieczytelny ACK".into());
                return Err(e);
            }
        };
        let invalid_open_proof = self.tr.config().close_receipt_reconcile
            && (!matches!(res.retcode,retcode::DONE|retcode::DONE_PARTIAL)
                || res.position_identifier==0 || res.position==0 || res.deal==0
                || !res.volume.is_finite() || res.volume<=0.0
                || res.volume>vol+vol.abs().max(1.0)*f64::EPSILON*64.0
                || !res.price.is_finite() || res.price<=0.0);
        if invalid_open_proof {
            self.remember_unknown_open(&r, &cm, format!(
                "niepełny/sprzeczny dowód wykonania OPEN: retcode={} deal={} order={} ticket={} identifier={} volume={} requested={} price={}; wynik wymaga rekoncyliacji, bez domyślnego wolumenu/ceny i bez ponowienia",
                res.retcode,res.deal,res.order,res.position,res.position_identifier,res.volume,vol,res.price));
            self.unknown_sends += 1;
            // A positive but incomplete response is not proof of exact execution.
            // Do not invent ticket/volume/price or resend. A later actual snapshot
            // may expose the position for protective operations, but cannot erase
            // this latch without the missing execution/ledger proof.
            return Err(BrokerError::Rejected);
        }
        let ticket = if res.position != 0 {
            res.position
        } else {
            res.order
        };
        if ticket == 0 {
            return Err(BrokerError::Rejected);
        }
        let px = if res.price > 0.0 {
            res.price
        } else {
            self.q.entry(r.side)
        };
        self.positions.push(Position {
            ticket,
            side: r.side,
            volume: if res.volume > 0.0 { res.volume } else { vol },
            open_price: px,
            open_ts: self.q.ts,
            sl: r.sl,
            tp: r.tp,
            vsl: None,
            basket: r.basket,
            level: r.level,
            frozen: false,
            peak_pts: 0.0,
            last_peak_ts: self.q.ts,
            is_runner: r.tp.is_none(),
            is_toucher: r.is_toucher,
            comment: cm,
        });
        if self.tr.config().close_receipt_reconcile {
            self.remember_position(res.position_identifier, self.positions.last().unwrap().clone());
        }
        Ok(ticket)
    }

    fn place_pending(&mut self, r: PendingReq) -> BResult<Ticket> {
        self.receipt_entry_gate()?;
        let vol = self.norm_volume(r.volume)?;
        let side = r.kind.side();

        // Poziom, na którym limit się nie położy: po złej stronie rynku ALBO
        // bliżej ceny niż `stops_level` — broker odrzuca oba jako
        // `10015 INVALID_PRICE`. Sam warunek „rynek minął poziom" (tak było
        // wcześniej) przepuszczał ten drugi przypadek prosto do odmowy, a
        // silnik kończył na „rozstawiono 0 zleceń", czyli bez wejścia.
        // Siatka ratunkowa jest ta sama co w symulatorze: wejście po rynku.
        let stops = self.sym.stops_level_price();
        let crossed = match r.kind {
            PendingKind::BuyLimit | PendingKind::SellLimit => {
                !limit_price_is_valid(side, r.price, &self.q, stops)
            }
            PendingKind::BuyStop | PendingKind::SellStop => {
                !stop_price_is_valid(side, r.price, &self.q, stops)
            }
        };
        if crossed {
            self.market_instead_of_limit += 1;
            return self.open_market(OrderReq {
                side,
                volume: r.volume,
                sl: r.sl,
                tp: r.tp,
                basket: r.basket,
                level: r.level,
                is_toucher: r.is_toucher,
                comment: r.comment,
            });
        }

        let cm = comment::encode(&self.tag, r.basket, r.level, r.is_toucher, &r.comment);
        let args = json!({
            "symbol": self.sym.symbol,
            "kind": pending_kind_code(r.kind),
            "volume": vol,
            "price": self.sym.round_price(r.price),
            "sl": r.sl.map(|x| self.sym.round_price(x)),
            "tp": r.tp.map(|x| self.sym.round_price(x)),
            "comment": cm,
        });
        let v = self.trade_call("place_pending", args)?;
        let res = self.decode_trade_ack("place_pending",v)?;
        if res.order == 0 {
            if self.tr.config().close_receipt_reconcile {
                self.unknown_sends+=1;
                self.last_state=Instant::now()-self.state_interval;
                self.receipt_fault("ACK place_pending bez order; nie wolno zgadywać czy zlecenie powstało".into());
            }
            return Err(BrokerError::Rejected);
        }
        self.pendings.push(PendingOrder {
            ticket: res.order,
            kind: r.kind,
            volume: vol,
            price: r.price,
            sl: r.sl,
            tp: r.tp,
            placed_ts: self.q.ts,
            basket: r.basket,
            level: r.level,
            frozen: false,
            is_toucher: r.is_toucher,
            comment: cm,
            is_topup: false,
        });
        Ok(res.order)
    }

    fn modify_position(&mut self, t: Ticket, sl: Option<Px>, tp: Option<Px>) -> BResult<()> {
        let side = self
            .positions
            .iter()
            .find(|p| p.ticket == t)
            .map(|p| p.side)
            .ok_or(BrokerError::NoSuchTicket)?;
        self.precheck_stops(side, sl, tp)?;
        let args = json!({
            "ticket": t,
            "sl": sl.map(|x| self.sym.round_price(x)),
            "tp": tp.map(|x| self.sym.round_price(x)),
        });
        self.trade_call("modify_position", args)?;
        if let Some(p) = self.positions.iter_mut().find(|p| p.ticket == t) {
            p.sl = sl;
            p.tp = tp;
        }
        Ok(())
    }

    fn modify_pending(
        &mut self,
        t: Ticket,
        price: Px,
        sl: Option<Px>,
        tp: Option<Px>,
    ) -> BResult<()> {
        if !self.pendings.iter().any(|o| o.ticket == t) {
            return Err(BrokerError::NoSuchTicket);
        }
        let args = json!({
            "ticket": t,
            "price": self.sym.round_price(price),
            "sl": sl.map(|x| self.sym.round_price(x)),
            "tp": tp.map(|x| self.sym.round_price(x)),
        });
        self.trade_call("modify_pending", args)?;
        if let Some(o) = self.pendings.iter_mut().find(|o| o.ticket == t) {
            o.price = price;
            o.sl = sl;
            o.tp = tp;
        }
        Ok(())
    }

    fn close_position(&mut self, t: Ticket, reason: CloseReason) -> BResult<f64> {
        self.remember_live_positions();
        if !self.positions.iter().any(|p| p.ticket == t) {
            return Err(BrokerError::NoSuchTicket);
        }
        // powód zapamiętujemy PRZED wysłaniem — zdarzenie zamknięcia potrafi
        // wrócić szybciej niż odpowiedź na samo polecenie
        if !self.tr.config().close_receipt_reconcile { self.reason_of.insert(t, reason); }
        let v = match self.trade_call("close_position", json!({ "ticket": t })) {
            Ok(v) => v,
            Err(e) => {
                self.reason_of.remove(&t);
                return Err(e);
            }
        };
        let res = self.decode_trade_ack("close_position",v)?;
        if self.tr.config().close_receipt_reconcile { self.apply_close_receipt(t, reason, &res); }
        else { self.positions.retain(|p| p.ticket != t); }
        Ok(res.profit)
    }

    fn close_partial(&mut self, t: Ticket, volume: f64, reason: CloseReason) -> BResult<f64> {
        self.remember_live_positions();
        let cur = self
            .positions
            .iter()
            .find(|p| p.ticket == t)
            .map(|p| p.volume)
            .ok_or(BrokerError::NoSuchTicket)?;
        let vol = self.sym.round_volume(volume);
        if vol < self.sym.volume_min - 1e-9 {
            return Err(BrokerError::InvalidVolume);
        }
        // resztka poniżej minimalnego lota jest niehandlowalna — zamykamy całość
        if vol >= cur - 1e-9 || cur - vol < self.sym.volume_min - 1e-9 {
            return self.close_position(t, reason);
        }
        if !self.tr.config().close_receipt_reconcile { self.reason_of.insert(t, reason); }
        let v = match self.trade_call("close_partial", json!({ "ticket": t, "volume": vol })) {
            Ok(v) => v,
            Err(e) => {
                self.reason_of.remove(&t);
                return Err(e);
            }
        };
        let res = self.decode_trade_ack("close_partial",v)?;
        if self.tr.config().close_receipt_reconcile { self.apply_close_receipt(t, reason, &res); }
        else if let Some(p) = self.positions.iter_mut().find(|p| p.ticket == t) {
            p.volume = (p.volume - vol).max(0.0);
        }
        Ok(res.profit)
    }

    fn cancel_pending(&mut self, t: Ticket) -> BResult<()> {
        if !self.pendings.iter().any(|o| o.ticket == t) {
            return Err(BrokerError::NoSuchTicket);
        }
        self.trade_call("cancel_pending", json!({ "ticket": t }))?;
        self.pendings.retain(|o| o.ticket != t);
        Ok(())
    }

    fn drain_closed(&mut self) -> Vec<ClosedTrade> {
        std::mem::take(&mut self.closed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typy_zlecen_w_obie_strony() {
        for k in [
            PendingKind::BuyLimit,
            PendingKind::SellLimit,
            PendingKind::BuyStop,
            PendingKind::SellStop,
        ] {
            assert_eq!(pending_kind(pending_kind_code(k)), Some(k));
        }
        // zlecenie rynkowe i stop-limit nie są zleceniami oczekującymi bota
        assert_eq!(pending_kind(0), None);
        assert_eq!(pending_kind(1), None);
        assert_eq!(pending_kind(6), None);
        assert_eq!(pending_kind(7), None);
    }

    #[test]
    fn zero_w_sl_znaczy_brak_a_nie_cene_zero() {
        assert_eq!(opt_px(0.0), None);
        assert_eq!(opt_px(-1.0), None);
        assert_eq!(opt_px(f64::NAN), None);
        assert_eq!(opt_px(3990.0), Some(3990.0));
    }

    #[test]
    fn powod_zamkniecia_z_deala() {
        assert_eq!(deal_reason(4), CloseReason::Sl);
        assert_eq!(deal_reason(5), CloseReason::Tp);
        assert_eq!(deal_reason(6), CloseReason::MaxDd);
        assert_eq!(deal_reason(0), CloseReason::Manual);
        assert_eq!(deal_reason(3), CloseReason::Manual);
    }

    #[test]
    fn strona_pozycji_jest_odwrotna_do_deala_zamykajacego() {
        // DEAL_TYPE_BUY (0) zamyka pozycję SELL
        let side = |dt: i32| if dt == 0 { Side::Sell } else { Side::Buy };
        assert_eq!(side(0), Side::Sell);
        assert_eq!(side(1), Side::Buy);
    }

    fn xau(stops_pkt: f64) -> SymbolInfo {
        SymbolInfo {
            symbol: "XAUUSD".into(),
            digits: 2,
            point: 0.01,
            stops_level_points: stops_pkt,
            freeze_level_points: 0.0,
            volume_min: 0.01,
            volume_max: 100.0,
            volume_step: 0.01,
            contract_size: 100.0,
            filling_mask: 2,
            filling_market: 1,
            filling_pending: 2,
            trade_mode: 4,
            visible: true,
            description: String::new(),
            swap_long: -75.82,
            swap_short: 27.41,
            swap_mode: 1,
            swap_rollover3days: 3,
            tick_value: 1.0,
            tick_size: 0.01,
        }
    }

    #[test]
    fn zgodny_stops_level_nie_podnosi_alarmu() {
        assert_eq!(sprawdz_stops_level(&xau(20.0), Some(0.20)), None);
        // brak wartości odniesienia = nie ma czego porównywać
        assert_eq!(sprawdz_stops_level(&xau(20.0), None), None);
        // szum zapisu dziesiętnego nie może dawać fałszywego alarmu
        assert_eq!(
            sprawdz_stops_level(&xau(20.0), Some(0.200_000_000_001)),
            None
        );

        // PRÓG jest przy point/2 = 0,005 — nie przy 0,5. Ta para pilnuje,
        // żeby nikt nie „poprawił" tolerancji na pół punktu stops_level.
        assert_eq!(
            sprawdz_stops_level(&xau(20.0), Some(0.204)),
            None,
            "0,004 mieści się w progu"
        );
        assert!(
            sprawdz_stops_level(&xau(20.0), Some(0.21)).is_some(),
            "0,01 to już rozjazd — próg wynosi 0,005, a nie 0,5"
        );
    }

    #[test]
    fn zmiana_stops_level_przez_brokera_jest_wykrywana() {
        // Vantage rozszerza dystans przed danymi makro: 20 -> 50 punktów
        let m = sprawdz_stops_level(&xau(50.0), Some(0.20)).expect("rozjazd musi być wykryty");
        assert!((m.broker_price - 0.50).abs() < 1e-9);
        assert!((m.expected_price - 0.20).abs() < 1e-9);
        assert!((m.expected_points - 20.0).abs() < 1e-6);
        assert!((m.broker_points - 50.0).abs() < 1e-9);
        let t = m.opis();
        // komunikat ma mówić, CO Z TEGO WYNIKA, nie tylko że liczby są różne
        assert!(t.contains("ODRZUCI"), "{t}");
        assert!(t.contains("0.50000"), "{t}");
        // i ma podać wartość do wpisania w preset
        assert!(t.contains("sim_stops_level"), "{t}");
    }

    #[test]
    fn preset_ostrzejszy_od_brokera_tez_jest_rozjazdem() {
        let m = sprawdz_stops_level(&xau(20.0), Some(0.50)).expect("rozjazd w drugą stronę");
        assert!(
            m.opis()
                .contains("przyjmie zlecenia, które backtest odrzuca"),
            "{}",
            m.opis()
        );
    }
}
