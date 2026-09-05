

import { useCallback, useEffect, useState } from "react";
import { Badge, Button, Empty, Icon, Modal, Tooltip } from "@/components/ui";
import { useApp } from "@/store/AppStore";
import { useT } from "@/i18n";
import {
  odwroc,
  przeszkodaCofniecia,
  tylkoRozne,
  type WpisOperacji,
  type Zmiana,
} from "@/store/historiaOperacji";
import { num, timeShort } from "@/lib/format";
import "./cofnij.css";

/** Wartość liczbowa w wierszu różnicy; `null`/0 to „nie ustawione". */
function Wartosc({ v }: { v: number | null }) {
  const tt = useT();
  if (v === null || v === 0) return <span className="undo__none">{tt("undo.none")}</span>;
  return <b className="num">{num(v, 2)}</b>;
}

/** Nazwy pól wypisane DOSŁOWNIE — klucz sklejany ze zmiennej nie ma typu
    literalnego i wypadłby spod kontroli kompletności słownika. */
const KLUCZ_POLA = {
  cena: "undo.field.cena",
  sl: "undo.field.sl",
  tp: "undo.field.tp",
  wolumen: "undo.field.wolumen",
} as const;


function WierszZmiany({ z }: { z: Zmiana }) {
  const tt = useT();
  return (
    <div className="undo__row">
      <span className="undo__field">{tt(KLUCZ_POLA[z.pole])}</span>
      <Wartosc v={z.z} />
      <Icon name="arrow-up-right" size={11} className="undo__arrow" />
      <Wartosc v={z.na} />
    </div>
  );
}

function Naglowek({ w }: { w: WpisOperacji }) {
  const tt = useT();
  return (
    <span className="undo__head">
      <span className="undo__kind">
        {tt(w.rodzaj === "pending" ? "undo.pending" : w.rodzaj === "position" ? "undo.position" : "undo.basket")}
      </span>
      <b className="num">{w.ticket === null ? "—" : `#${w.ticket}`}</b>
      {w.opis && <span className="undo__note">{w.opis}</span>}
      <span className="undo__time">{timeShort(w.czas)}</span>
    </span>
  );
}

/** Podgląd jednej operacji — używany i w oknie cofania, i na liście. */
function Podglad({ w, kierunek }: { w: WpisOperacji; kierunek: "cofnij" | "ponow" }) {
  // Przy cofaniu pokazujemy zmiany ODWRÓCONE: to jest stan, który za chwilę
  // wróci na rachunek. Pokazanie oryginalnego kierunku byłoby myleniem
  // operatora dokładnie w chwili, w której podejmuje decyzję.
  const zmiany = tylkoRozne(kierunek === "cofnij" ? odwroc(w.zmiany) : w.zmiany);
  return (
    <div className="undo__prev">
      <Naglowek w={w} />
      {zmiany.length > 0 ? (
        zmiany.map((z) => <WierszZmiany key={z.pole} z={z} />)
      ) : (
        <div className="undo__row undo__row--empty">—</div>
      )}
    </div>
  );
}

export function CofnijPonow() {
  const app = useApp();
  const tt = useT();
  const { historia, snapshot } = app;
  const [okno, setOkno] = useState<"cofnij" | "ponow" | "lista" | null>(null);

  const cel = historia.doCofniecia;
  const celPonow = historia.doPonowienia;
  const przeszkoda = cel ? przeszkodaCofniecia(cel, snapshot) : null;
  const mozna = !!cel && przeszkoda === null;

  /* Skróty: Ctrl+Z otwiera PODGLĄD, nie wykonuje. Ctrl+Y i Ctrl+Shift+Z
     ponawiają. Gdy fokus siedzi w polu tekstowym, oddajemy skrót
     przeglądarce — inaczej nie dałoby się cofnąć wpisanej cyfry. */
  useEffect(() => {
    const wPolu = () => {
      const el = document.activeElement as HTMLElement | null;
      if (!el) return false;
      return ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName) || el.isContentEditable;
    };
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
      const k = e.key.toLowerCase();
      if (k !== "z" && k !== "y") return;
      if (wPolu()) return;
      e.preventDefault();
      const ponawia = k === "y" || (k === "z" && e.shiftKey);
      if (ponawia) {
        if (celPonow) setOkno("ponow");
      } else if (cel) {
        setOkno("cofnij");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [cel, celPonow]);

  const wykonajCofniecie = useCallback(() => {
    historia.cofnij();
    setOkno(null);
    app.toast("info", tt("undo.sent"), tt("undo.sentText"));
  }, [app, historia, tt]);

  const wykonajPonowienie = useCallback(() => {
    historia.ponow();
    setOkno(null);
    app.toast("info", tt("undo.sentRedo"), tt("undo.sentText"));
  }, [app, historia, tt]);

  return (
    <div className="undo">
      {}
      <Tooltip content={cel ? tt("undo.undo.title") : tt("undo.nothingToUndo")}>
        <Button
          size="sm"
          variant="ghost"
          icon="undo"
          disabled={!cel}
          title={cel ? tt("undo.undo.title") : tt("undo.nothingToUndo")}
          onClick={() => setOkno("cofnij")}
        />
      </Tooltip>
      <Tooltip content={celPonow ? tt("undo.redo.title") : tt("undo.redo.idle")}>
        <Button
          size="sm"
          variant="ghost"
          icon="redo"
          disabled={!celPonow}
          title={celPonow ? tt("undo.redo.title") : tt("undo.redo.idle")}
          onClick={() => setOkno("ponow")}
        />
      </Tooltip>
      <Tooltip content={tt("undo.open")}>
        <Button
          size="sm"
          variant="ghost"
          icon="book"
          title={tt("undo.open")}
          onClick={() => setOkno("lista")}
        >
          {historia.wpisy.length > 0 ? String(historia.wpisy.length) : ""}
        </Button>
      </Tooltip>

      {/* ---------- podgląd cofnięcia ---------- */}
      <Modal
        open={okno === "cofnij"}
        onClose={() => setOkno(null)}
        title={tt("undo.preview")}
        subtitle={tt("undo.subtitle")}
        width={520}
        footer={
          <>
            <Button variant="ghost" onClick={() => setOkno(null)}>
              {tt("potw.no")}
            </Button>
            <Button variant="primary" icon="undo" disabled={!mozna} onClick={wykonajCofniecie}>
              {tt("undo.confirmUndo")}
            </Button>
          </>
        }
      >
        {cel ? (
          <>
            <Podglad w={cel} kierunek="cofnij" />
            {przeszkoda && <div className="undo__blocked">{tt(przeszkoda)}</div>}
            {!przeszkoda && historia.nieodwracalnePo > 0 && (
              <div className="undo__warn">{tt("undo.afterWarn", { n: historia.nieodwracalnePo })}</div>
            )}
          </>
        ) : (
          <Empty icon="info" title={tt("undo.nothingToUndo")} text={tt("undo.empty")} />
        )}
      </Modal>

      {/* ---------- podgląd ponowienia ---------- */}
      <Modal
        open={okno === "ponow"}
        onClose={() => setOkno(null)}
        title={tt("undo.previewRedo")}
        width={520}
        footer={
          <>
            <Button variant="ghost" onClick={() => setOkno(null)}>
              {tt("potw.no")}
            </Button>
            <Button variant="primary" icon="redo" disabled={!celPonow} onClick={wykonajPonowienie}>
              {tt("undo.confirmRedo")}
            </Button>
          </>
        }
      >
        {celPonow ? (
          <Podglad w={celPonow} kierunek="ponow" />
        ) : (
          <Empty icon="info" title={tt("undo.nothingToRedo")} />
        )}
      </Modal>

      {/* ---------- pełny dziennik ręcznych operacji (K17) ---------- */}
      <Modal
        open={okno === "lista"}
        onClose={() => setOkno(null)}
        title={tt("undo.title")}
        subtitle={tt("undo.subtitle")}
        width={640}
        footer={
          <Button variant="ghost" icon="trash" disabled={!historia.wpisy.length} onClick={historia.wyczysc}>
            {tt("undo.clear")}
          </Button>
        }
      >
        {historia.wpisy.length === 0 ? (
          <Empty icon="book" title={tt("undo.empty")} />
        ) : (
          <ul className="undo__list">
            {historia.wpisy.map((w) => {
              const blokada = przeszkodaCofniecia(w, snapshot);
              return (
                <li key={w.id} className="undo__item" data-cofniety={w.cofniety}>
                  <Podglad w={w} kierunek="ponow" />
                  <div className="undo__tags">
                    {w.cofniety && <Badge tone="warn">{tt("undo.undone")}</Badge>}
                    {!w.odwrotna && <Badge tone="muted">{tt("undo.irreversible")}</Badge>}
                    {blokada && !w.cofniety && <span className="undo__why">{tt(blokada)}</span>}
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </Modal>
    </div>
  );
}
