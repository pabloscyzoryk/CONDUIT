import type { ConnectionState, Quote } from "@/types";

/** An AUTO setting is an instruction to the bridge, not a symbol name.
 * Never infer its result from a watchlist, broker name, or an arbitrary quote.
 * The runtime binding travels atomically with the account and quote snapshot.
 */
export function resolveBotSymbol(
  live: boolean,
  connection: Pick<ConnectionState, "mt5" | "resolvedSymbol">,
  configured: string,
  demoDefault: string,
): string {
  if (!live) return configured.trim() || demoDefault;
  if (connection.mt5 !== "connected") return "";
  if (typeof connection.resolvedSymbol === "string") return connection.resolvedSymbol.trim();
  // Compatibility with old explicit-symbol backends only. AUTO stays unresolved.
  return configured.trim();
}

export function unavailableQuote(symbol: string): Quote {
  return { symbol, bid: NaN, ask: NaN, spread: NaN, time: 0,
    change: NaN, changePct: NaN, dayHigh: NaN, dayLow: NaN };
}

/** Missing/wrong-symbol quotes must not activate the manual ticket. */
export function runtimeQuote(symbol: string, quotes: Record<string, Quote>): Quote {
  const quote = symbol ? quotes[symbol] : undefined;
  return quote?.symbol === symbol && Number.isFinite(quote.bid) && quote.bid > 0
    && Number.isFinite(quote.ask) && quote.ask >= quote.bid
    ? quote : unavailableQuote(symbol);
}
