//! Spięcie: świece z terminala MT5 -> kształt, którego oczekuje panel.
//!
//! Cały ten plik to tłumaczenie jednej struktury na drugą. Jest osobno,
//! bo `conduit-server` świadomie nie zna `conduit-mt5` (patrz `state::Runtime`
//! i ten sam podział przy komendach handlowych), a `conduit-mt5` nie ma prawa
//! wiedzieć, jak wygląda REST panelu.
//!
//! Rzecz, której tu NIE MA i być nie może: żadnej ścieżki awaryjnej,
//! która przy braku terminala oddałaby cokolwiek zamiast błędu.

use conduit_mt5::{MarketData, TransportHandle};
use conduit_server::market::{
    Candle, CandlesDoc, CostsDoc, DealDoc, DealsDoc, MarketSource, SlipDoc, SymbolDoc, SymbolRow,
};

pub struct Mt5Market {
    md: MarketData,
}

impl Mt5Market {
    pub fn new(tr: TransportHandle) -> Self {
        Mt5Market {
            md: MarketData::new(tr),
        }
    }
}

impl MarketSource for Mt5Market {
    fn symbols(&self, q: Option<&str>) -> anyhow::Result<Vec<SymbolRow>> {
        let l = self
            .md
            .symbols(q)
            .map_err(|e| anyhow::anyhow!("lista symboli z MetaTradera 5: {e}"))?;
        Ok(l.symbols
            .into_iter()
            .map(|s| SymbolRow {
                name: s.name,
                visible: s.visible,
                digits: s.digits,
                trade_mode: s.trade_mode,
            })
            .collect())
    }

    fn candles(
        &self,
        symbol: &str,
        tf: &str,
        count: usize,
        to: Option<i64>,
    ) -> anyhow::Result<CandlesDoc> {
        let c = self.md.candles(symbol, tf, count, to).map_err(|e| {
            anyhow::anyhow!("nie udało się pobrać świec {symbol} {tf} z MetaTradera 5: {e}")
        })?;

        // Świadomie NIE odsiewamy pustej listy jako błędu. Pusta odpowiedź to
        // prawdziwa informacja: instrument bez historii albo zapytanie o czas
        // sprzed pierwszej świecy przy przewijaniu w lewo. Panel ma wtedy
        // przestać doładowywać, a nie zobaczyć awarię.
        let candles: Vec<Candle> = c
            .bars
            .iter()
            .map(|b| Candle {
                t: b.t(),
                o: b.open(),
                h: b.high(),
                l: b.low(),
                c: b.close(),
                v: b.volume(),
                s: b.spread(),
            })
            .collect();

        Ok(CandlesDoc {
            symbol: c.symbol.clone(),
            tf: c.tf.clone(),
            // Jedyna wartość, jaka może się tu pojawić. Gdyby świece kiedyś
            // przyszły skądinąd, MUSI się tu pojawić inna nazwa — panel zapala
            // po tym polu chorągiewkę „nie z brokera".
            source: "MT5".to_string(),
            bar_ms: c.bar_ms,
            digits: c.digits,
            point: c.point,
            server_offset_ms: c.server_offset_ms(),
            server_time: c.fresh_server_time_ms(),
            market_open: c.market_open(),
            complete: c.last_closed(),
            oldest: c.oldest(),
            newest: c.newest(),
            count: candles.len(),
            candles,
        })
    }

    fn symbol(&self, symbol: &str) -> anyhow::Result<SymbolDoc> {
        let s = self.md.symbol_info(symbol).map_err(|e| {
            anyhow::anyhow!("nie udało się pobrać parametrów {symbol} z MetaTradera 5: {e}")
        })?;
        Ok(SymbolDoc {
            symbol: s.symbol.clone(),
            description: s.description.clone(),
            digits: s.digits,
            point: s.point,
            stops_level_points: s.stops_level_points,
            stops_level_price: s.stops_level_price(),
            freeze_level_points: s.freeze_level_points,
            freeze_level_price: s.freeze_level_points * s.point,
            volume_min: s.volume_min,
            volume_max: s.volume_max,
            volume_step: s.volume_step,
            contract_size: s.contract_size,
            trade_mode: s.trade_mode,
            visible: s.visible,
        })
    }

    fn deals(
        &self,
        from: Option<i64>,
        to: Option<i64>,
        symbol: Option<&str>,
        magic: Option<i64>,
        out_only: bool,
        offset: usize,
        limit: usize,
    ) -> anyhow::Result<DealsDoc> {
        let d = self
            .md
            .deals(from, to, symbol, magic, out_only, offset, limit)
            .map_err(|e| anyhow::anyhow!("nie udało się pobrać historii z MetaTradera 5: {e}"))?;

        let deals: Vec<DealDoc> = d
            .deals
            .iter()
            .map(|x| DealDoc {
                ticket: x.ticket(),
                order: x.1,
                position: x.position(),
                time: x.time_msc(),
                kind: nazwa_typu(x.kind()).to_string(),
                entry: nazwa_wejscia(x.entry()).to_string(),
                volume: x.volume(),
                price: x.price(),
                profit: x.profit(),
                commission: x.commission(),
                swap: x.swap(),
                fee: x.11,
                net: x.net(),
                magic: x.magic(),
                reason: x.13,
                symbol: x.symbol().to_string(),
                comment: x.comment().to_string(),
            })
            .collect();

        Ok(DealsDoc {
            total: d.total,
            offset: d.offset,
            count: deals.len(),
            more: d.more,
            source: "MT5".to_string(),
            deals,
        })
    }

    fn costs(&self, symbol: &str, days: f64) -> anyhow::Result<CostsDoc> {
        let c = self
            .md
            .costs(days)
            .map_err(|e| anyhow::anyhow!("nie udało się zmierzyć kosztów wykonania: {e}"))?;
        let si = self
            .md
            .symbol_info(symbol)
            .map_err(|e| anyhow::anyhow!("nie udało się pobrać parametrów {symbol}: {e}"))?;

        Ok(CostsDoc {
            symbol: c.symbol.clone(),
            window_days: c.window_days,
            pending: slip(&c.pending),
            market: slip(&c.market),
            swap_long_raw: si.swap_long,
            swap_short_raw: si.swap_short,
            swap_mode: si.swap_mode,
            swap_rollover3days: si.swap_rollover3days,
            swap_rollover_weekday_mon0: si.swap_rollover_weekday_mon0(),
            swap_rollover_entry_weekday_mon0: si.swap_rollover_entry_weekday_mon0(),
            swap_long_usd_per_lot_day: si.swap_usd_per_lot_day(true),
            swap_short_usd_per_lot_day: si.swap_usd_per_lot_day(false),
            usd_per_point: si.usd_per_point(),
        })
    }

    fn default_symbol(&self) -> String {
        self.md.default_symbol().to_string()
    }

    fn is_connected(&self) -> bool {
        self.md.is_connected()
    }
}

fn slip(s: &conduit_mt5::SlipStats) -> SlipDoc {
    SlipDoc {
        n: s.n,
        mean: s.mean,
        sd: s.sd,
        ci95: s.ci95,
        exact: if s.exact > 0 {
            s.exact
        } else {
            s.exact_at_level
        },
        max_abs: s.max_abs,
        source: s.source.clone(),
    }
}

/// `DEAL_TYPE_*` na nazwę. 2 to operacja SALDA, nie handel — i musi być
/// widoczna jako taka, bo wpłata 1000 $ w statystykach handlu wygląda
/// jak najlepsza transakcja w historii konta.
fn nazwa_typu(t: i32) -> &'static str {
    match t {
        0 => "BUY",
        1 => "SELL",
        2 => "BALANCE",
        _ => "INNE",
    }
}

fn nazwa_wejscia(e: i32) -> &'static str {
    match e {
        0 => "IN",
        1 => "OUT",
        2 => "INOUT",
        3 => "OUT_BY",
        _ => "INNE",
    }
}
