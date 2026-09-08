import { Tooltip } from "@/components/ui";
import { useT } from "@/i18n";
import { money, num } from "@/lib/format";
import { realDrawdown } from "@/lib/realDrawdown";
import type { Stats } from "@/types";

export function RealDrawdown({ day, currency }: { day: Stats["realDrawdownDay"]; currency: string }) {
  const tt = useT();
  const rdd = realDrawdown(day);
  const value = rdd && rdd.percent !== null ? `${num(rdd.percent, 1)}%` : "—";
  const description = rdd
    ? tt("stats.rdd.value", { amount: money(rdd.amount, currency, { decimals: 2 }), percent: rdd.percent === null ? "—%" : `${num(rdd.percent, 2)}%` })
      + (rdd.percent === null ? ` ${tt("stats.rdd.percentUnavailable")}` : "")
    : tt("stats.rdd.unavailable");
  return <Tooltip content={<>{tt("stats.rdd.definition")}<br />{description}</>}>
    <span className="stat__rdd num" tabIndex={0} aria-label={`${tt("stats.rdd")}: ${value}. ${description}`}>
      {tt("stats.rdd")}: {value}
    </span>
  </Tooltip>;
}
