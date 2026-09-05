

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { t as tSlownik } from "@/i18n";
import {
  Transport,
  backendBase,
  isNativeShell,
  mergeDelta,
  probeBackend,
  type Command,
  type DeltaPatch,
  type ServerEvent,
  type TransportStatus,
  type UiSnapshot,
} from "./transport";

export interface BackendBridge {
  /** backend odpowiedział na `/api/health` i mamy z niego snapshot */
  live: boolean;
  status: TransportStatus | "brak";
  snapshot: UiSnapshot | null;
  latencyMs: number;
  /** adres serwera — do pokazania w panelu diagnostycznym */
  base: string;
  /** czy działamy w oknie natywnym (Tauri) */
  nativeShell: boolean;
  send: (cmd: Command) => Promise<{ ok: boolean; error?: string }>;
  patchSettings: (patch: Record<string, unknown>) => Promise<{ ok: boolean; error?: string }>;
}

/** Co ile próbować ponownie, gdy backendu nie było przy starcie. */
const PROBE_INTERVAL_MS = 5000;

export function useBackend(onEvent?: (e: ServerEvent) => void): BackendBridge {
  const [status, setStatus] = useState<TransportStatus | "brak">("brak");
  const [snapshot, setSnapshot] = useState<UiSnapshot | null>(null);
  const [latencyMs, setLatency] = useState(0);

  const transport = useRef<Transport | null>(null);
  const eventRef = useRef(onEvent);
  eventRef.current = onEvent;

  /* --- delty scalamy w referencji, a do Reacta oddajemy nowy obiekt ---
     Serwer koalescencjonuje do ~10 Hz, więc to jest 10 renderów na sekundę
     całego store'u — dokładnie tyle, ile założono w budżecie wydajności. */
  const stan = useRef<UiSnapshot | null>(null);

  const przyjmijSnapshot = useCallback((s: UiSnapshot) => {
    stan.current = s;
    setSnapshot(s);
  }, []);

  const przyjmijDelte = useCallback((p: DeltaPatch) => {
    if (!stan.current) return;
    stan.current = mergeDelta(stan.current, p);
    setSnapshot(stan.current);
  }, []);

  useEffect(() => {
    let anulowane = false;
    let probeTimer: ReturnType<typeof setInterval> | undefined;

    const podlacz = () => {
      if (anulowane || transport.current) return;
      const t = new Transport({
        onSnapshot: przyjmijSnapshot,
        onDelta: przyjmijDelte,
        onEvent: (e) => eventRef.current?.(e),
        onStatus: (s) => {
          setStatus(s);
          if (s === "closed") {
            stan.current = null;
            setSnapshot(null);
          }
        },
        onLatency: setLatency,
      });
      transport.current = t;
      t.connect();
    };

    const sprobuj = async () => {
      if (anulowane || transport.current) return;
      if (await probeBackend()) {
        if (probeTimer) clearInterval(probeTimer);
        podlacz();
      }
    };

    void sprobuj();
    // backend uruchomiony PO otwarciu strony też zostanie wykryty —
    // bez tego trzeba by odświeżać kartę po każdym starcie `conduit.exe`
    probeTimer = setInterval(() => void sprobuj(), PROBE_INTERVAL_MS);

    return () => {
      anulowane = true;
      if (probeTimer) clearInterval(probeTimer);
      transport.current?.close();
      transport.current = null;
    };
  }, [przyjmijSnapshot, przyjmijDelte]);

  // Capture the render's identity. Reading stan.current here would silently
  // re-authorize a stale modal/queued command after A->B->A.
  const renderedAccountSession = snapshot?.connection.accountSession;
  const send = useCallback(async (cmd: Command) => {
    const t = transport.current;
    if (!t) return { ok: false, error: tSlownik("net.noServer") };
    return t.send(cmd, renderedAccountSession);
  }, [renderedAccountSession]);

  const patchSettings = useCallback(async (patch: Record<string, unknown>) => {
    const t = transport.current;
    if (!t) return { ok: false, error: tSlownik("net.noServer") };
    return t.patchSettings(patch);
  }, []);

  const nativeShell = useMemo(() => isNativeShell(), []);

  return {
    live: snapshot !== null && status === "open",
    status,
    snapshot,
    latencyMs,
    base: backendBase(),
    nativeShell,
    send,
    patchSettings,
  };
}
