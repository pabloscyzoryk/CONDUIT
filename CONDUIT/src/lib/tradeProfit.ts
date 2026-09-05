import type { ClosedPosition } from "@/types";

/** One net contract for history, summaries and chart markers. Never infer basis. */
export function closedNetProfit(trade: Pick<ClosedPosition, "profit" | "swap" | "commission" | "profitBasis" | "netProfit">): number | null {
  const { profit, commission, swap, profitBasis } = trade;
  if (![profit, commission, swap].every(Number.isFinite)) return null;
  let net: number;
  switch (profitBasis) {
    case "PriceOnlyGross": net = profit + commission + swap; break;
    case "PricePlusSwap": net = profit + commission; break;
    case "CanonicalClosedNetV1": {
      const verified = trade.netProfit;
      if (verified == null || !Number.isFinite(verified) || Math.abs(verified - profit) > Math.max(1, Math.abs(verified)) * 1e-12) return null;
      net = verified;
      break;
    }
    case "ReportedNet": net = profit; break; // Local demo: net, without claiming receipt proof.
    default: return null;
  }
  return Number.isFinite(net) ? net : null;
}

/** Partial closes are independent realised tranches, including their own costs. */
export function closedProfitSummary(closed: ClosedPosition[]) {
  const values = closed.map(closedNetProfit);
  const complete = values.every((value): value is number => value !== null);
  const known = values.filter((value): value is number => value !== null);
  const wins = known.filter(value => value > 0);
  const losses = known.filter(value => value < 0);
  const grossWin = wins.reduce((sum, value) => sum + value, 0);
  const grossLoss = -losses.reduce((sum, value) => sum + value, 0);
  const total = known.reduce((sum, value) => sum + value, 0);
  const valid = complete && [total, grossWin, grossLoss].every(Number.isFinite);
  return {
    total: valid ? total : null,
    count: closed.length,
    winRate: valid ? (closed.length ? wins.length / closed.length * 100 : 0) : null,
    pf: valid ? (grossLoss > 0 ? grossWin / grossLoss : grossWin > 0 ? Infinity : 0) : null,
    best: valid ? (known.length ? known.reduce((a, b) => Math.max(a, b), -Infinity) : 0) : null,
    worst: valid ? (known.length ? known.reduce((a, b) => Math.min(a, b), Infinity) : 0) : null,
    avgHold: closed.length ? closed.reduce((sum, c) => sum + c.closeTime - c.openTime, 0) / closed.length : 0,
    volume: closed.reduce((sum, c) => sum + c.volume, 0),
  };
}
