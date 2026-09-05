//! Warstwa bezpieczeństwa — twarde ograniczenia wykonania.
//!
//! **To nie jest element funkcji nagrody.** Kara w nagrodzie mówi modelowi, że
//! wyzerowanie konta jest złe; ta warstwa sprawia, że jest NIEMOŻLIWE. Nagroda
//! jest miękka i podlega optymalizacji — a optymalizator zawsze w końcu znajdzie
//! dziurę. Ograniczenie w kodzie dziury nie ma.
//!
//! Cztery niezależne bezpieczniki:
//!
//! 1. **Podłoga equity** — gdy kapitał spadnie do `equity_floor_pct` % stanu
//!    startowego, wszystko jest likwidowane, a silnik zostaje zatrzymany
//!    (`Engine::halted`), więc nie powstanie żaden nowy koszyk. Sprawdzane na
//!    KAŻDYM ticku, nie co 2 sekundy.
//! 2. **Sufit wykorzystania marginu** — żadna akcja modelu nie może podnieść
//!    `margin / equity` powyżej `max_margin_util_pct` %.
//! 3. **Limity ekspozycji** — maksymalna liczba pozycji, oczekujących i łączny
//!    wolumen.
//! 4. **Zapadka SL** — model może przesuwać stop-loss wyłącznie w stronę zysku.
//!    Nigdy nie poszerzy straty ani nie zdejmie już ustawionego SL.

use conduit_core::broker::{sl_is_valid, tp_is_valid, Broker};
use conduit_core::types::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SafetyCfg {
    /// twarda podłoga equity jako % kapitału startowego
    pub equity_floor_pct: f64,
    /// maksymalne `margin / equity` w %
    pub max_margin_util_pct: f64,
    /// margines swobody: nowa ekspozycja wymaga equity powyżej podłogi × ten mnożnik
    pub floor_headroom: f64,
    pub max_positions: usize,
    pub max_pendings: usize,
    /// maksymalny łączny wolumen otwartych pozycji (loty), sufit bezwzględny
    pub max_total_lots: f64,
    /// Maksymalny łączny wolumen na każde 1000 $ KAPITAŁU STARTOWEGO.
    ///
    /// To jest ogranicznik, bez którego cała reszta jest dekoracją. Sam
    /// `max_total_lots` skalował się z bieżącym equity przez limit marginu, więc
    /// rosnące konto odblokowywało coraz większą pozycję i optymalizator znalazł
    /// dokładnie to: z 200 $ robił 121 000 $, jadąc jednym lotem złota, czyli
    /// 400 000 $ nominału na koncie wielkości roweru. Odniesienie do kapitału
    /// STARTOWEGO (a nie bieżącego) sprawia, że model nie może się rozpędzić na
    /// własnych zyskach — a przy okazji porównanie z linią bazową, która handluje
    /// stałym lotem 0.01, w ogóle ma sens.
    pub max_lots_per_1000_start: f64,
    /// maksymalny wolumen pojedynczego zlecenia modelu (loty)
    pub max_order_lots: f64,
    /// SL wolno przesuwać wyłącznie w stronę zysku
    pub ratchet_sl_only: bool,
    /// Maksymalne ryzyko JEDNEGO wejścia jako % equity: |wejście − SL| × 100 × wolumen.
    ///
    /// To jest bezpiecznik, który decyduje o ryzyku ruiny. Na koncie 200 $ przy
    /// locie 0.01 i SL 6 $ jedna pozycja to 3 % kapitału; przy kilku jednostkach
    /// naraz ruina przestaje być hipotezą. Model może wybierać głębokość wejścia
    /// i szerokość stopa dowolnie — ale iloczyn tych dwóch rzeczy z wolumenem
    /// przechodzi przez ten limit.
    pub max_risk_pct_per_entry: f64,
    /// Maksymalne ŁĄCZNE otwarte ryzyko jako % equity.
    pub max_total_risk_pct: f64,
}

impl Default for SafetyCfg {
    fn default() -> Self {
        SafetyCfg {
            equity_floor_pct: 60.0,
            max_margin_util_pct: 30.0,
            floor_headroom: 1.10,
            max_positions: 40,
            max_pendings: 60,
            max_total_lots: 1.00,
            max_lots_per_1000_start: 0.15,
            max_order_lots: 0.10,
            ratchet_sl_only: true,
            max_risk_pct_per_entry: 4.0,
            max_total_risk_pct: 8.0,
        }
    }
}

/// Powód odmowy — trafia do liczników diagnostycznych, nie do logu w pętli.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deny {
    EquityFloor,
    MarginUtil,
    FreeMargin,
    TooManyPositions,
    TooManyPendings,
    TooManyLots,
    OrderTooBig,
    Ratchet,
    BrokerStops,
    RiskTooBig,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DenyCounters {
    pub equity_floor: u32,
    pub margin: u32,
    pub exposure: u32,
    pub ratchet: u32,
    pub broker_stops: u32,
    pub risk: u32,
}

impl DenyCounters {
    pub fn bump(&mut self, d: Deny) {
        match d {
            Deny::EquityFloor => self.equity_floor += 1,
            Deny::MarginUtil | Deny::FreeMargin => self.margin += 1,
            Deny::TooManyPositions
            | Deny::TooManyPendings
            | Deny::TooManyLots
            | Deny::OrderTooBig => self.exposure += 1,
            Deny::Ratchet => self.ratchet += 1,
            Deny::BrokerStops => self.broker_stops += 1,
            Deny::RiskTooBig => self.risk += 1,
        }
    }
    pub fn total(&self) -> u32 {
        self.equity_floor
            + self.margin
            + self.exposure
            + self.ratchet
            + self.broker_stops
            + self.risk
    }
}

pub struct Safety {
    pub cfg: SafetyCfg,
    pub start_balance: f64,
    pub denies: DenyCounters,
}

impl Safety {
    pub fn new(cfg: SafetyCfg, start_balance: f64) -> Self {
        Safety {
            cfg,
            start_balance,
            denies: DenyCounters::default(),
        }
    }

    #[inline]
    pub fn floor(&self) -> f64 {
        self.start_balance * self.cfg.equity_floor_pct / 100.0
    }

    /// Czy trzeba NATYCHMIAST zlikwidować pozycje i zatrzymać handel?
    #[inline]
    pub fn must_liquidate(&self, acc: &Account) -> bool {
        acc.equity <= self.floor()
    }

    /// Margin wymagany dla zlecenia o danym wolumenie i cenie.
    #[inline]
    pub fn margin_for(&self, volume: f64, price: Px, leverage: u32) -> f64 {
        volume * XAU_CONTRACT * price / leverage.max(1) as f64
    }

    /// Bramka dla KAŻDEJ akcji modelu zwiększającej ekspozycję.
    ///
    /// Zwraca `Err(powód)`, gdy akcja jest zabroniona. Nie ma ścieżki obejścia —
    /// runtime nie potrafi otworzyć niczego, nie przechodząc tędy.
    pub fn allow_new_exposure<B: Broker>(&self, b: &B, volume: f64, price: Px) -> Result<(), Deny> {
        let acc = b.account();
        let c = &self.cfg;

        if volume > c.max_order_lots + 1e-9 {
            return Err(Deny::OrderTooBig);
        }
        // przy nowej ekspozycji wymagamy zapasu NAD podłogą, nie samej podłogi
        if acc.equity <= self.floor() * c.floor_headroom {
            return Err(Deny::EquityFloor);
        }
        if b.positions().len() >= c.max_positions {
            return Err(Deny::TooManyPositions);
        }
        if b.pendings().len() >= c.max_pendings {
            return Err(Deny::TooManyPendings);
        }
        let lots: f64 = b.positions().iter().map(|p| p.volume).sum();
        if lots + volume > self.lot_cap() + 1e-9 {
            return Err(Deny::TooManyLots);
        }

        let need = self.margin_for(volume, price, acc.leverage);
        if (acc.margin + need) / acc.equity.max(1e-9) * 100.0 > c.max_margin_util_pct {
            return Err(Deny::MarginUtil);
        }
        if acc.free_margin < need {
            return Err(Deny::FreeMargin);
        }
        Ok(())
    }

    /// Docelowy SL po nałożeniu zapadki i wymogów brokera.
    ///
    /// Zwraca `Err`, gdy modyfikacja jest zabroniona lub broker i tak by ją
    /// odrzucił — lepiej nie wysyłać, niż wysyłać i liczyć na odrzut.
    pub fn sanitize_sl(
        &self,
        side: Side,
        current: Option<Px>,
        want: Px,
        q: &Quote,
        stops_level: f64,
    ) -> Result<Px, Deny> {
        if self.cfg.ratchet_sl_only {
            if let Some(cur) = current {
                // „better" = korzystniejsza cena wejścia; dla SL korzystniejsza
                // cena to ta LUŹNIEJSZA, więc dopuszczamy tylko ruch przeciwny
                if side.better(want, cur) {
                    return Err(Deny::Ratchet);
                }
            }
        }
        if !sl_is_valid(side, want, q, stops_level) {
            return Err(Deny::BrokerStops);
        }
        Ok(want)
    }

    pub fn sanitize_tp(
        &self,
        side: Side,
        want: Px,
        q: &Quote,
        stops_level: f64,
    ) -> Result<Px, Deny> {
        if !tp_is_valid(side, want, q, stops_level) {
            return Err(Deny::BrokerStops);
        }
        Ok(want)
    }

    /// Czy wolno zdjąć TP (zamienić pozycję w runnera)?
    ///
    /// Tylko gdy pozycja ma SL — inaczej powstałaby pozycja bez żadnego wyjścia,
    /// czyli dokładnie taka, która potrafi zjeść konto.
    #[inline]
    pub fn allow_drop_tp(&self, has_sl: bool) -> Result<(), Deny> {
        if has_sl {
            Ok(())
        } else {
            Err(Deny::Ratchet)
        }
    }

    /// Sufit łącznego wolumenu: mniejszy z limitu bezwzględnego i limitu
    /// wynikającego z kapitału STARTOWEGO. Nigdy poniżej jednego minimalnego lota.
    #[inline]
    pub fn lot_cap(&self) -> f64 {
        let from_capital = self.start_balance / 1000.0 * self.cfg.max_lots_per_1000_start;
        from_capital.min(self.cfg.max_total_lots).max(0.01)
    }

    /// Bramka ryzyka: czy wolno dołożyć wejście o ryzyku `add_risk` USD,
    /// przy już otwartym ryzyku `open_risk`?
    ///
    /// Osobna od `allow_new_exposure`, bo mierzy co innego: margin pilnuje, czy
    /// broker w ogóle wpuści zlecenie, a to pilnuje, ile realnie stracimy, gdy
    /// wszystkie stopy zadziałają naraz.
    pub fn allow_risk(&self, acc: &Account, add_risk: f64, open_risk: f64) -> Result<(), Deny> {
        let eq = acc.equity.max(1e-9);
        if add_risk / eq * 100.0 > self.cfg.max_risk_pct_per_entry {
            return Err(Deny::RiskTooBig);
        }
        if (open_risk + add_risk) / eq * 100.0 > self.cfg.max_total_risk_pct {
            return Err(Deny::RiskTooBig);
        }
        Ok(())
    }

    /// Bieżące wykorzystanie marginu w %.
    #[inline]
    pub fn margin_util(&self, acc: &Account) -> f64 {
        acc.margin / acc.equity.max(1e-9) * 100.0
    }

    #[inline]
    pub fn over_margin(&self, acc: &Account) -> bool {
        self.margin_util(acc) > self.cfg.max_margin_util_pct
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use conduit_backtest::SimBroker;
    use conduit_core::broker::OrderReq;

    fn q(bid: f64) -> Quote {
        Quote {
            ts: 1_000,
            bid,
            ask: bid + 0.23,
        }
    }

    fn broker(balance: f64) -> SimBroker {
        let mut b = SimBroker::new(balance, 0.20, 0.0);
        b.on_quote(q(4000.0));
        b
    }

    fn order(vol: f64) -> OrderReq {
        OrderReq {
            side: Side::Buy,
            volume: vol,
            sl: None,
            tp: None,
            basket: None,
            level: 0,
            is_toucher: false,
            comment: String::new(),
        }
    }

    #[test]
    fn podloga_equity_blokuje_nowa_ekspozycje() {
        let s = Safety::new(SafetyCfg::default(), 1000.0);
        let mut b = broker(1000.0);
        // stan zdrowy — wolno
        assert_eq!(s.allow_new_exposure(&b, 0.01, 4000.0), Ok(()));
        // konto zjechało pod podłogę 60 % → nic nowego nie wolno
        b.balance = 550.0;
        assert_eq!(
            s.allow_new_exposure(&b, 0.01, 4000.0),
            Err(Deny::EquityFloor)
        );
        assert!(s.must_liquidate(&b.account()));
    }

    #[test]
    fn akcja_zerujaca_konto_jest_zablokowana() {
        // Scenariusz: model chce dołożyć wolumen, który przy dźwigni 500 zjadłby
        // cały depozyt. Warstwa wykonania musi odmówić, zanim zlecenie dotknie
        // brokera.
        let s = Safety::new(SafetyCfg::default(), 1000.0);
        let b = broker(1000.0);
        // 1 lot złota = 400 000 $ nominału → margin 800 $ przy equity 1000 $
        assert_eq!(
            s.allow_new_exposure(&b, 1.0, 4000.0),
            Err(Deny::OrderTooBig)
        );
        // nawet po podniesieniu limitu pojedynczego zlecenia zatrzyma margin
        let mut cfg = SafetyCfg::default();
        cfg.max_order_lots = 10.0;
        cfg.max_total_lots = 10.0;
        cfg.max_lots_per_1000_start = 100.0; // wyłączamy sufit kapitałowy, badamy margin
        let s2 = Safety::new(cfg, 1000.0);
        assert_eq!(
            s2.allow_new_exposure(&b, 1.0, 4000.0),
            Err(Deny::MarginUtil)
        );
    }

    #[test]
    fn sufit_marginu_dziala_na_realnym_brokerze() {
        let mut cfg = SafetyCfg::default();
        cfg.max_lots_per_1000_start = 100.0; // badamy wyłącznie limit marginu
        let s = Safety::new(cfg, 1000.0);
        let mut b = broker(1000.0);
        // dokładamy pozycje aż margin przekroczy 30 % equity
        for _ in 0..4 {
            b.open_market(order(0.10)).unwrap();
        }
        // 0.40 lota → margin = 0.4*100*4000/500 = 320 $ przy equity ≈ 1000 $
        assert!(
            s.over_margin(&b.account()),
            "util {}",
            s.margin_util(&b.account())
        );
        assert_eq!(
            s.allow_new_exposure(&b, 0.01, 4000.0),
            Err(Deny::MarginUtil)
        );
    }

    #[test]
    fn sufit_wolumenu_skaluje_sie_z_kapitalem_startowym() {
        // REGRESJA: bez tego ogranicznika model robił z 200 $ ponad 100 000 $,
        // bo limit marginu rósł razem z equity i odblokowywał coraz większy lot.
        let s200 = Safety::new(SafetyCfg::default(), 200.0);
        let s10k = Safety::new(SafetyCfg::default(), 10_000.0);
        assert!(
            (s200.lot_cap() - 0.03).abs() < 1e-9,
            "200 $ → {}",
            s200.lot_cap()
        );
        assert!(
            (s10k.lot_cap() - 1.0).abs() < 1e-9,
            "10 000 $ → {}",
            s10k.lot_cap()
        );

        // i faktycznie blokuje, nawet gdy konto urosło dziesięciokrotnie
        let mut b = broker(200.0);
        b.balance = 20_000.0;
        b.open_market(order(0.03)).unwrap();
        assert_eq!(
            s200.allow_new_exposure(&b, 0.01, 4000.0),
            Err(Deny::TooManyLots)
        );
    }

    #[test]
    fn limit_lacznego_wolumenu() {
        let mut cfg = SafetyCfg::default();
        cfg.max_total_lots = 0.05;
        cfg.max_margin_util_pct = 100.0;
        cfg.max_lots_per_1000_start = 100.0;
        let s = Safety::new(cfg, 100_000.0);
        let mut b = broker(100_000.0);
        b.open_market(order(0.05)).unwrap();
        assert_eq!(
            s.allow_new_exposure(&b, 0.01, 4000.0),
            Err(Deny::TooManyLots)
        );
    }

    #[test]
    fn limit_ryzyka_na_wejscie_blokuje_zbyt_szeroki_stop() {
        // konto 200 $, limit 4 % = 8 $ ryzyka na wejście
        let s = Safety::new(SafetyCfg::default(), 200.0);
        let acc = Account {
            balance: 200.0,
            equity: 200.0,
            margin: 0.0,
            free_margin: 200.0,
            leverage: 500,
            credit: 0.0,
        };
        // 0.01 lota ze stopem 6 $ = 6 $ ryzyka → wolno
        assert_eq!(s.allow_risk(&acc, 6.0, 0.0), Ok(()));
        // ten sam lot ze stopem 12 $ = 12 $ ryzyka → za dużo
        assert_eq!(s.allow_risk(&acc, 12.0, 0.0), Err(Deny::RiskTooBig));
        // i suma też jest pilnowana: 6 + 6 + 6 = 18 $ > 8 % z 200 $
        assert_eq!(s.allow_risk(&acc, 6.0, 12.0), Err(Deny::RiskTooBig));
        assert_eq!(s.allow_risk(&acc, 6.0, 6.0), Ok(()));
    }

    #[test]
    fn zapadka_nie_pozwala_poszerzyc_sl() {
        let s = Safety::new(SafetyCfg::default(), 1000.0);
        let qq = q(4000.0);
        // BUY, SL na 3990 — próba zejścia na 3980 to poszerzenie straty
        assert_eq!(
            s.sanitize_sl(Side::Buy, Some(3990.0), 3980.0, &qq, 0.20),
            Err(Deny::Ratchet)
        );
        // dociągnięcie w górę jest dozwolone
        assert_eq!(
            s.sanitize_sl(Side::Buy, Some(3990.0), 3995.0, &qq, 0.20),
            Ok(3995.0)
        );
        // ale nie bliżej niż stops level
        assert_eq!(
            s.sanitize_sl(Side::Buy, Some(3990.0), 3999.95, &qq, 0.20),
            Err(Deny::BrokerStops)
        );
        // SELL: lustrzanie
        assert_eq!(
            s.sanitize_sl(Side::Sell, Some(4010.0), 4020.0, &qq, 0.20),
            Err(Deny::Ratchet)
        );
        assert_eq!(
            s.sanitize_sl(Side::Sell, Some(4010.0), 4005.0, &qq, 0.20),
            Ok(4005.0)
        );
    }

    #[test]
    fn nie_wolno_zdjac_tp_bez_sl() {
        let s = Safety::new(SafetyCfg::default(), 1000.0);
        assert!(s.allow_drop_tp(false).is_err());
        assert!(s.allow_drop_tp(true).is_ok());
    }
}
