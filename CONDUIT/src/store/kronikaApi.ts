/* ============================================================
   KRONIKA — klient REST.

   Osobny plik od `transport.ts`, bo używają go DWIE aplikacje:
   panel Conduita (który ma cały `AppStore`, WebSocket i timery)
   oraz `kronika.exe` (który nie ma z tego nic i mieć nie powinien).
   Wciągnięcie `transport.ts` do samodzielnej binarki oznaczałoby
   podnoszenie klienta WebSocket do serwera, którego tam nie ma.

   `backendBase()` jest współdzielone, bo rozwiązuje ten sam problem:
   przy pracy nad UI strona stoi na :5180, a API na :8787.
   ============================================================ */

import { backendBase } from "@/store/transport";
import type {
  KronikaKanal,
  KronikaPodsumowanieEksportu,
  KronikaStan,
  KronikaStatystyki,
  KronikaUstawienia,
} from "@/types/kronika";

async function json<T>(sciezka: string, init?: RequestInit): Promise<T> {
  const r = await fetch(`${backendBase()}${sciezka}`, {
    ...init,
    headers: { "Content-Type": "application/json", ...(init?.headers ?? {}) },
  });
  if (!r.ok) {
    let tresc = `HTTP ${r.status}`;
    try {
      const b = await r.json();
      if (b?.error) tresc = b.error;
    } catch {
      /* odpowiedź nie jest JSON-em — zostaje kod stanu */
    }
    throw new Error(tresc);
  }
  return (await r.json()) as T;
}

export const kronikaApi = {
  /** Co się dzieje TERAZ: liczniki sesji, podgląd ostatnich wierszy, opcje. */
  stan: () => json<KronikaStan>("/api/kronika/stan"),

  /** Liczby z PLIKU, nie z liczników procesu — te zerują się przy restarcie. */
  statystyki: () => json<KronikaStatystyki>("/api/kronika/statystyki"),

  kanaly: () =>
    json<{ ok: boolean; kanaly: KronikaKanal[]; blad: string | null }>("/api/kronika/kanaly"),

  ustaw: (u: KronikaUstawienia) =>
    json<{ ok: boolean; ustawienia: KronikaUstawienia }>("/api/kronika/ustawienia", {
      method: "PUT",
      body: JSON.stringify(u),
    }),

  /** Zbiór backtestowy. WYGODA — plik `.jsonl` jest kompletny sam z siebie. */
  eksport: (plik?: string, tylkoZaznaczone = false) =>
    json<{ ok: boolean; wynik: KronikaPodsumowanieEksportu }>("/api/kronika/eksport", {
      method: "POST",
      body: JSON.stringify({ plik, tylko_zaznaczone: tylkoZaznaczone }),
    }),

  
  plikUrl: () => `${backendBase()}/api/kronika/plik`,
};
