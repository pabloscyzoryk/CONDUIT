/** Strategy ceiling before broker rounding, signal risk and margin gates.
 * null in a known result means no strategy cap. Unknown never means unlimited. */
export function strategyOrderCeiling(maximum: unknown, capitalPerLot: unknown, capital: unknown): { known: boolean; ceiling: number | null } {
  if (typeof maximum !== "number" || !Number.isFinite(maximum) || maximum < 0 ||
      typeof capitalPerLot !== "number" || !Number.isFinite(capitalPerLot) || capitalPerLot < 0) return { known: false, ceiling: null };
  if (capitalPerLot > 0 && (typeof capital !== "number" || !Number.isFinite(capital) || capital <= 0)) return { known: false, ceiling: null };
  const limits = [maximum > 0 ? maximum : Infinity, capitalPerLot > 0 ? (capital as number) / capitalPerLot : Infinity];
  const ceiling = Math.min(...limits);
  return { known: true, ceiling: Number.isFinite(ceiling) ? ceiling : null };
}
