import type { SymbolMeta } from "@/types";

/**
 * Instrumenty widoczne w wyszukiwarce wykresow (odpowiednik
 * symbols_search / market watch z MT5).
 */
export const SYMBOLS: SymbolMeta[] = [
  { symbol: "XAUUSD", name: "Gold / US dollar", digits: 2, group: "Metals", base: 4118.4, vol: 0.9, contractSize: 100, pointValue: 1 },
  { symbol: "XAGUSD", name: "Srebro / dolar", digits: 3, group: "Metals", base: 48.62, vol: 1.6, contractSize: 5000, pointValue: 50 },
  { symbol: "XPTUSD", name: "Platyna / dolar", digits: 2, group: "Metals", base: 1092.5, vol: 1.2, contractSize: 100, pointValue: 1 },
  { symbol: "EURUSD", name: "Euro / dolar", digits: 5, group: "Forex", base: 1.08425, vol: 0.42, contractSize: 100000, pointValue: 100000 },
  { symbol: "GBPUSD", name: "Funt / dolar", digits: 5, group: "Forex", base: 1.27180, vol: 0.5, contractSize: 100000, pointValue: 100000 },
  { symbol: "USDJPY", name: "Dolar / jen", digits: 3, group: "Forex", base: 151.42, vol: 0.48, contractSize: 100000, pointValue: 660 },
  { symbol: "AUDUSD", name: "Dolar australijski / dolar", digits: 5, group: "Forex", base: 0.65940, vol: 0.55, contractSize: 100000, pointValue: 100000 },
  { symbol: "USDCAD", name: "Dolar / dolar kanadyjski", digits: 5, group: "Forex", base: 1.36210, vol: 0.4, contractSize: 100000, pointValue: 73000 },
  { symbol: "USDCHF", name: "Dolar / frank", digits: 5, group: "Forex", base: 0.88240, vol: 0.38, contractSize: 100000, pointValue: 113000 },
  { symbol: "BTCUSD", name: "Bitcoin / dolar", digits: 1, group: "Crypto", base: 97420, vol: 2.8, contractSize: 1, pointValue: 1 },
  { symbol: "ETHUSD", name: "Ethereum / dolar", digits: 2, group: "Crypto", base: 3684.2, vol: 3.4, contractSize: 1, pointValue: 1 },
  { symbol: "SOLUSD", name: "Solana / dolar", digits: 3, group: "Crypto", base: 214.86, vol: 4.6, contractSize: 1, pointValue: 1 },
  { symbol: "US30", name: "Dow Jones 30", digits: 1, group: "Indices", base: 43180, vol: 0.85, contractSize: 1, pointValue: 1 },
  { symbol: "NAS100", name: "Nasdaq 100", digits: 1, group: "Indices", base: 20614, vol: 1.15, contractSize: 1, pointValue: 1 },
  { symbol: "SPX500", name: "S&P 500", digits: 1, group: "Indices", base: 5872.4, vol: 0.78, contractSize: 1, pointValue: 1 },
  { symbol: "GER40", name: "DAX 40", digits: 1, group: "Indices", base: 19842, vol: 0.95, contractSize: 1, pointValue: 1 },
  { symbol: "JP225", name: "Nikkei 225", digits: 0, group: "Indices", base: 71823, vol: 1.05, contractSize: 1, pointValue: 1 },
  { symbol: "UK100", name: "FTSE 100", digits: 1, group: "Indices", base: 8204.6, vol: 0.7, contractSize: 1, pointValue: 1 },
  { symbol: "USOIL", name: "Ropa WTI", digits: 2, group: "Energy", base: 71.34, vol: 1.9, contractSize: 1000, pointValue: 1000 },
  { symbol: "UKOIL", name: "Ropa Brent", digits: 2, group: "Energy", base: 74.86, vol: 1.8, contractSize: 1000, pointValue: 1000 },
  { symbol: "NATGAS", name: "Gaz ziemny", digits: 3, group: "Energy", base: 3.284, vol: 3.2, contractSize: 10000, pointValue: 10000 },
];

export const PRIMARY_SYMBOL = "XAUUSD";

const BY_SYMBOL = new Map(SYMBOLS.map((s) => [s.symbol, s]));

export function getSymbol(symbol: string): SymbolMeta {
  return BY_SYMBOL.get(symbol) ?? SYMBOLS[0];
}

export function searchSymbols(q: string): SymbolMeta[] {
  const needle = q.trim().toUpperCase();
  if (!needle) return SYMBOLS.slice(0, 10);
  return SYMBOLS.filter(
    (s) =>
      s.symbol.includes(needle) ||
      s.name.toUpperCase().includes(needle) ||
      s.group.toUpperCase().includes(needle),
  ).slice(0, 12);
}
