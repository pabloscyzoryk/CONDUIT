/** Shared navigation/settings search. Tokens are ANDed across all metadata.
 * Underscores, accents and word order never prevent finding an axis. */
export function normalizeSearch(value: string): string {
  return value.normalize("NFD").replace(/[\u0300-\u036f]/g, "").replace(/ł/g, "l").replace(/Ł/g, "L")
    .toLowerCase().replace(/[_\-]+/g, " ").replace(/\s+/g, " ").trim();
}

export function matchesSearch(query: string, ...values: (string | undefined | null)[]): boolean {
  const tokens = normalizeSearch(query).split(" ").filter(Boolean);
  const text = normalizeSearch(values.filter(Boolean).join(" "));
  return tokens.every(token => text.includes(token));
}

/** Names and exact configuration keys outrank incidental words in long help. */
export function searchScore(query: string, key: string, label: string): number {
  const q = normalizeSearch(query), k = normalizeSearch(key), l = normalizeSearch(label);
  const ordered = (v: string) => v.split(" ").sort().join(" ");
  if (ordered(q) === ordered(k)) return 100;
  if (q === l) return 90;
  if (k.includes(q)) return 80;
  if (l.includes(q)) return 70;
  if (matchesSearch(q, k)) return 60;
  if (matchesSearch(q, l)) return 50;
  return 0;
}
