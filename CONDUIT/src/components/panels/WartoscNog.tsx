

import { Icon, Tooltip } from "@/components/ui";
import { useApp, type UstawieniaNogi } from "@/store/AppStore";
import { useT } from "@/i18n";
import type { Settings } from "@/types";
import "./panels.css";

/** Ton kropki „strażnik aktywny": u wszystkich / u części / u żadnej nogi. */
export type Ton = "wszystkie" | "czesc" | "zadna";


export function tonStraznika(nogi: UstawieniaNogi[], aktywne: (s: Settings) => boolean): Ton {
  const g = nogi.filter((n) => n.handluje);
  if (g.length === 0) return "zadna";
  const ile = g.filter((n) => aktywne(n.doc)).length;
  return ile === g.length ? "wszystkie" : ile > 0 ? "czesc" : "zadna";
}

export function WartoscNog<K extends keyof Settings>({
  pole,
  format: fmt = (v) => String(v),
  wartosc,
  zapas,
}: {
  pole: K;
  /** Jak sformatować wartość JEDNEJ nogi. Ignorowane, gdy podano `wartosc`. */
  format?: (v: Settings[K]) => string;
  /** Jak policzyć napis z CAŁEGO dokumentu nogi — dla strażników z pary pól. */
  wartosc?: (d: Settings) => string;
  /** Co pokazać, gdy nóg nie ma (offline/makieta) — zwykle wartość dokumentu. */
  zapas?: string;
}) {
  const { ustawieniaNog } = useApp();
  const tt = useT();
  const grajace = ustawieniaNog.filter((n) => n.handluje);

  // Brak nóg = tryb offline albo makieta. Nie zgadujemy — pokazujemy to,
  // co wołający uznał za sensowne (zwykle wartość dokumentu panelu).
  if (grajace.length === 0) return <>{zapas ?? tt("undo.none")}</>;

  const napis = (d: Settings) => (wartosc ? wartosc(d) : fmt(d[pole]));
  const wartosci = grajace.map((n) => ({ n, v: napis(n.doc) }));
  const napisy = [...new Set(wartosci.map((x) => x.v))];
  const zgodne = napisy.length === 1;

  return (
    <Tooltip
      szeroki
      content={
        <div className="wartnog__tip">
          <b>{tt("nog.tipHead")}</b>
          <ul>
            {wartosci.map(({ n, v }) => (
              <li key={`${n.format}|${n.preset}`}>
                <b>{n.format || "—"}</b> → <span>{n.preset || "—"}</span>
                <b className="num">{v}</b>
                {!n.zPliku && <em>{tt("nog.fromDoc")}</em>}
              </li>
            ))}
          </ul>
        </div>
      }
    >
      <span className="wartnog" data-zgodne={zgodne}>
        <span className="num">
          {zgodne ? napisy[0] : napisy.length <= 3 ? napisy.join(" · ") : tt("nog.different", { n: napisy.length })}
        </span>
        {grajace.length > 1 && <Icon name="layers" size={11} />}
      </span>
    </Tooltip>
  );
}
