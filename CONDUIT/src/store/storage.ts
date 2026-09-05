/** Cienka warstwa nad localStorage — bezpieczna przy blokadzie ciasteczek. */

const PREFIX = "conduit.";

/** Czy to zwykły obiekt (a nie tablica, `null`, data, mapa…)? */
function zwyklyObiekt(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v) && Object.getPrototypeOf(v) === Object.prototype;
}

/**
 * Scala ZAPISANY dokument z DOMYŚLNYM: klucze, których w zapisie nie ma,
 * biorą wartość domyślną. Tablice i wartości proste bierzemy z zapisu w całości.
 *
 * # Po co
 *
 * Bez tego każde NOWE pole konfiguracji budziło się u każdego, kto już raz
 * uruchomił panel, jako `undefined` — czyli w przełączniku jako „wyłączone".
 * Dokładnie tak zniknąłby świeżo dodany alarm „Nieczytelny sygnał z kanału":
 * domyślnie włączony w kodzie, a u użytkownika z historią w `localStorage`
 * martwy od pierwszej sekundy i bez śladu, dlaczego.
 */
function scal<T>(zapis: unknown, domyslne: T): T {
  if (!zwyklyObiekt(zapis) || !zwyklyObiekt(domyslne)) return (zapis as T) ?? domyslne;
  const out: Record<string, unknown> = { ...domyslne };
  for (const [k, v] of Object.entries(zapis)) {
    out[k] = k in domyslne ? scal(v, (domyslne as Record<string, unknown>)[k]) : v;
  }
  return out as T;
}

export function load<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(PREFIX + key);
    if (raw === null) return fallback;
    return scal(JSON.parse(raw), fallback);
  } catch {
    return fallback;
  }
}

export function save(key: string, value: unknown): void {
  try {
    localStorage.setItem(PREFIX + key, JSON.stringify(value));
  } catch {
    /* pamięć niedostępna — ignorujemy, design ma działać także w trybie prywatnym */
  }
}

export function remove(key: string): void {
  try {
    localStorage.removeItem(PREFIX + key);
  } catch {
    /* jw. */
  }
}
