import { Badge, Tooltip } from "@/components/ui";
import { RichT, useT } from "@/i18n";
import { type Preset, type TrybOceny, type WynikTrybu } from "@/types";

/* Etykiety trybow biora sie ze SLOWNIKA (`ocena.tryb.<id>` / `.hint`), a nie
   ze stalych `TRYB_LABEL` / `TRYB_HINT` w `types/index.ts`. Te stale zostaja
   w typach jako opis dla czytajacego kod, ale UI ich nie uzywa — inaczej
   panel po angielsku mowilby po polsku. */



const TRYBY: TrybOceny[] = ["dzienny", "dziennyComp", "dlugi", "dlugiComp"];

/** Prog, ponizej ktorego minimalny lot nie odrobi konta (dla startu 200 $). */
const PROG_ODROBIENIA = 40;

function pieniadze(v: number): string {
  const znak = v > 0 ? "+" : v < 0 ? "−" : "";
  return `${znak}$${Math.abs(v).toFixed(0)}`;
}


function Tryb({ tryb, w }: { tryb: TrybOceny; w?: WynikTrybu }) {
  const t = useT();
  const zrujnowany = !!w && w.wyzerowania > 0;
  const blisko = !!w && !zrujnowany && w.najnizszeEquity > 0 && w.najnizszeEquity <= PROG_ODROBIENIA;

  return (
    <div className={`ocena__tryb ${zrujnowany ? "ocena__tryb--ruina" : ""}`}>
      <div className="ocena__naglowek">
        <Tooltip content={t(`ocena.tryb.${tryb}.hint`)}>
          <b>{t(`ocena.tryb.${tryb}`)}</b>
        </Tooltip>
        {zrujnowany && (
          <Badge tone="short" dot>
            {t("ocena.wiped", { n: w.wyzerowania })}
          </Badge>
        )}
        {blisko && <Badge tone="warn">{t("ocena.nearRuin")}</Badge>}
      </div>

      {!w ? (
        
        <p className="hint ocena__brak">{t("lab.notMeasuredMode")}</p>
      ) : (
        <div className="ocena__liczby">
          <span className="ocena__poz">
            <b className={w.zysk >= 0 ? "up" : "down"}>{pieniadze(w.zysk)}</b>
            <span>{t("ocena.profit")}</span>
          </span>
          <span className="ocena__poz">
            <b className={w.dniStratnePct > 50 ? "down" : ""}>{w.dniStratnePct.toFixed(0)}%</b>
            <span>{t("lab.c.lossDays")}</span>
          </span>
          <span className="ocena__poz">
            <b className="down">{pieniadze(w.najgorszyDzien)}</b>
            <span>{t("lab.c.worstDay")}</span>
          </span>
          <span className="ocena__poz">
            <b className={w.najnizszeEquity <= PROG_ODROBIENIA ? "down" : ""}>
              ${w.najnizszeEquity.toFixed(0)}
            </b>
            <span>{t("lab.c.minEquity")}</span>
          </span>
          <span className="ocena__poz">
            <b className={w.wyzerowania > 0 ? "down" : "up"}>{w.wyzerowania}</b>
            <span>{t("lab.col.ruins")}</span>
          </span>
        </div>
      )}

      {/* maxDD osobno i przygaszony — RAPORTUJEMY, ale nie rangujemy po nim. */}
      {w?.maxDd !== undefined && (
        <p className="hint ocena__dd">
          maxDD ${w.maxDd.toFixed(0)} <i>{t("ocena.maxDdNote")}</i>
        </p>
      )}
    </div>
  );
}

export function OcenaPresetu({ preset }: { preset: Preset }) {
  const t = useT();
  const tryby = preset.metrics.tryby;
  const zmierzone = TRYBY.filter((x) => tryby?.[x]).length;
  const zrujnowane = TRYBY.filter((x) => (tryby?.[x]?.wyzerowania ?? 0) > 0);

  return (
    <div className="ocena">
      <div className="ocena__head">
        <b>{t("lab.quad.title")}</b>
        <Badge tone={zmierzone === 4 ? "long" : "warn"}>{t("lab.verdict.partialSet", { n: zmierzone })}</Badge>
        {zrujnowane.length > 0 && (
          <Badge tone="short" dot>
            {t("lab.verdict.ruined")}
          </Badge>
        )}
      </div>

      {zrujnowane.length > 0 && (
        <p className="hint ocena__ostrzezenie">
          {t("ocena.ruinIn")} <b>{zrujnowane.map((x) => t(`ocena.tryb.${x}`)).join(", ")}</b>.{" "}
          {t("ocena.ruinNote")}
        </p>
      )}

      {zmierzone < 4 && (
        <p className="hint">
          <RichT k="ocena.incomplete" />
        </p>
      )}

      <div className="ocena__grid">
        {TRYBY.map((x) => (
          <Tryb key={x} tryb={x} w={tryby?.[x]} />
        ))}
      </div>
    </div>
  );
}
