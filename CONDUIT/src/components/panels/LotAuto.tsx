
import { useEffect, useMemo, useState } from "react";
import { Badge, Button, Field, Icon, Modal, NumberInput, Segmented, Select, Switch, Tooltip } from "@/components/ui";
import { useApp } from "@/store/AppStore";
import { useT } from "@/i18n";
import { api } from "@/store/transport";
import { money, num, pct } from "@/lib/format";
import { strategyOrderCeiling } from "@/lib/orderLimits";
import {
  brakiRachunku,
  kontraktSymbolu,
  kosztLota,
  policzSufit,
  type BrakDanej,
  type KosztLota,
  type Rachunek,
  type SufitAuto,
} from "@/lib/ekspozycja";

type Noga = ReturnType<typeof useApp>["stats"]["lotNogi"][number];

/** Czy noga bierze nowe sygnały (starsze backendy nie mają pola). */
function handluje(n: Noga): boolean {
  return n.handluje ?? !n.zamrozona;
}

/** Grupa, do której trafia kafelek. Starszy backend bez `stan` → z `handluje`. */
function grupa(n: Noga): "aktywna" | "kolejka" | "nieaktywna" {
  if (n.stan === "kolejka") return "kolejka";
  if (n.stan === "aktywna") return "aktywna";
  if (n.stan === "nieaktywna") return "nieaktywna";
  return handluje(n) ? "aktywna" : "nieaktywna";
}

/** Klucz nogi — format+preset, bo ten sam preset może wisieć na dwóch formatach. */
function kluczNogi(n: Noga): string {
  return `${n.format}|${n.preset}`;
}

/* ============================================================
   EDYTOR — komplet pól lota JEDNEJ nogi, z przełącznikiem nogi
   ============================================================ */

/** Pola lota w kształcie PANELU (tak nazywa je `/api/presets/{n}/ui`). */
type PolaLota = {
  lot_mode_percent: boolean;
  lot_percent: number;
  lot_fixed: number;
  lot_min: number;
  lot_max: number;
  lot_scale_step: number;
  lot_percent_small: number;
  lot_percent_small_mult: number;
  /* RELOT — przeliczanie wolumenu NIEWYPEŁNIONYCH szczebli, gdy saldo
     drgnie. To też jest ustawienie lota tej nogi, więc mieszka tutaj,
     a nie w globalnych ustawieniach. */
  pending_relot_on_balance: boolean;
  pending_relot_topup: boolean;
  pending_relot_wg_planu: boolean;
};

const PUSTE: PolaLota = {
  lot_mode_percent: true,
  lot_percent: 0.5,
  lot_fixed: 0.01,
  lot_min: 0.01,
  lot_max: 0,
  lot_scale_step: 0,
  lot_percent_small: 0,
  lot_percent_small_mult: 0,
  pending_relot_on_balance: false,
  pending_relot_topup: false,
  pending_relot_wg_planu: true,
};

function EdytorLota({
  noga,
  wszystkie,
  onNoga,
  onClose,
}: {
  noga: Noga;
  wszystkie: Noga[];
  onNoga: (n: Noga) => void;
  onClose: () => void;
}) {
  const tt = useT();
  const app = useApp();
  const [pola, setPola] = useState<PolaLota | null>(null);
  const [blad, setBlad] = useState<string | null>(null);
  const [zapis, setZapis] = useState(false);
  const [brudne, setBrudne] = useState(false);

  useEffect(() => {
    let zywy = true;
    setPola(null);
    setBrudne(false);
    setBlad(null);
    api
      .presetUi(noga.preset)
      .then((r) => {
        if (!zywy) return;
        const s = r.settings as Record<string, unknown>;
        const l = (k: keyof PolaLota, d: number) => (typeof s[k] === "number" ? (s[k] as number) : d);
        setPola({
          lot_mode_percent: s.lot_mode_percent === true,
          lot_percent: l("lot_percent", PUSTE.lot_percent),
          lot_fixed: l("lot_fixed", PUSTE.lot_fixed),
          lot_min: l("lot_min", PUSTE.lot_min),
          lot_max: l("lot_max", PUSTE.lot_max),
          lot_scale_step: l("lot_scale_step", 0),
          lot_percent_small: l("lot_percent_small", 0),
          lot_percent_small_mult: l("lot_percent_small_mult", 0),
          pending_relot_on_balance: s.pending_relot_on_balance === true,
          pending_relot_topup: s.pending_relot_topup === true,
          pending_relot_wg_planu: s.pending_relot_wg_planu !== false,
        });
      })
      .catch((e: unknown) => zywy && setBlad(String(e)));
    return () => {
      zywy = false;
    };
  }, [noga.preset]);

  const zmien = <K extends keyof PolaLota>(k: K, v: PolaLota[K]) => {
    setBrudne(true);
    setPola((p) => (p ? { ...p, [k]: v } : p));
  };

  const zapisz = () => {
    if (!pola) return;
    setZapis(true);
    setBlad(null);
    api
      .savePresetSettings(noga.preset, pola as unknown as Record<string, unknown>)
      .then((result) => {
        if (!result.ok) throw new Error(tt("set.save.failed", { name: noga.preset }));
        app.toast("success", tt("lotauto.saved", { p: noga.preset }), tt("lotauto.saved.text"));
        setBrudne(false);
        onClose();
      })
      .catch((e: unknown) => setBlad(String(e)))
      .finally(() => setZapis(false));
  };

  /* PRZEŁĄCZNIK NOGI — żeby poprawka lota na trzech presetach nie znaczyła
     trzykrotnego zamykania okna i szukania właściwego kafelka. Zmiana nogi
     przy niezapisanych polach pyta, bo cicha utrata edycji jest gorsza
     niż jedno kliknięcie więcej. */
  const przelacz = (klucz: string) => {
    const n = wszystkie.find((x) => kluczNogi(x) === klucz);
    if (!n || kluczNogi(n) === kluczNogi(noga)) return;
    if (brudne && !window.confirm(tt("lotauto.edit.dirty"))) return;
    onNoga(n);
  };

  return (
    <Modal open onClose={onClose} title={tt("lotauto.edit.title", { f: noga.format, p: noga.preset })}>
      <div className="col" style={{ gap: 10 }}>
        {wszystkie.length > 1 && (
          <Field label={tt("lotauto.edit.switch")} hint={tt("lotauto.edit.switch.hint")}>
            <Select
              value={kluczNogi(noga)}
              onChange={przelacz}
              options={wszystkie.map((n) => ({
                value: kluczNogi(n),
                label: `${n.format || tt("lotauto.noFormat")} → ${n.preset} · ${tt(
                  `lotauto.group.${grupa(n)}`,
                )}`,
              }))}
            />
          </Field>
        )}

        <p className="hint" style={{ margin: 0 }}>
          {tt("lotauto.edit.intro", { p: noga.preset })}
        </p>

        {blad && <Badge tone="warn">{blad}</Badge>}
        {!pola && !blad && <span className="hint">{tt("common.loading")}</span>}

        {pola && (
          <>
            <Segmented<"fixed" | "percent">
              value={pola.lot_mode_percent ? "percent" : "fixed"}
              onChange={(m) => zmien("lot_mode_percent", m === "percent")}
              size="sm"
              options={[
                { value: "fixed", label: tt("lot.fixed") },
                { value: "percent", label: tt("lot.percent") },
              ]}
            />

            {pola.lot_mode_percent ? (
              <Field label={tt("lot.pct")} hint={tt("lot.pct.hint")}>
                <NumberInput
                  value={pola.lot_percent}
                  onChange={(v) => zmien("lot_percent", v)}
                  step={0.05}
                  min={0}
                  unit="%"
                />
              </Field>
            ) : (
              <Field label={tt("lot.size")} hint={tt("lot.size.hint")}>
                <NumberInput
                  value={pola.lot_fixed}
                  onChange={(v) => zmien("lot_fixed", v)}
                  step={0.01}
                  min={0.01}
                  unit="lot"
                />
              </Field>
            )}

            <div className="ticket__grid">
              <Field label={tt("lotauto.f.min")} hint={tt("lotauto.f.min.hint")}>
                <NumberInput value={pola.lot_min} onChange={(v) => zmien("lot_min", v)} step={0.01} min={0} unit="lot" />
              </Field>
              <Field label={tt("lotauto.f.max")} hint={tt("lotauto.f.max.hint")}>
                <NumberInput value={pola.lot_max} onChange={(v) => zmien("lot_max", v)} step={0.01} min={0} unit="lot" />
              </Field>
              <Field label={tt("lotauto.f.step")} hint={tt("lotauto.f.step.hint")}>
                <NumberInput
                  value={pola.lot_scale_step}
                  onChange={(v) => zmien("lot_scale_step", v)}
                  step={50}
                  min={0}
                  unit="$"
                />
              </Field>
              <Field label={tt("lotauto.f.small")} hint={tt("lotauto.f.small.hint")}>
                <NumberInput
                  value={pola.lot_percent_small}
                  onChange={(v) => zmien("lot_percent_small", v)}
                  step={0.05}
                  min={0}
                  unit="%"
                />
              </Field>
              <Field label={tt("lotauto.f.smallMult")} hint={tt("lotauto.f.smallMult.hint")}>
                <NumberInput
                  value={pola.lot_percent_small_mult}
                  onChange={(v) => zmien("lot_percent_small_mult", v)}
                  step={0.1}
                  min={0}
                  unit="×"
                />
              </Field>
            </div>

            {/* RELOT — przeliczanie wolumenu NIEWYPEŁNIONYCH szczebli po
                zmianie salda. Należy do lota TEJ nogi: przy dwóch nogach
                jedna może dowolnie doważać siatkę, a druga nie. */}
            <div className="col" style={{ gap: 6 }}>
              <b style={{ fontSize: 11, opacity: 0.8 }}>{tt("lotauto.relot.head")}</b>
              <Switch
                checked={pola.pending_relot_on_balance}
                onChange={(v) => zmien("pending_relot_on_balance", v)}
                label={tt("lotauto.relot.onBalance")}
              />
              <Switch
                checked={pola.pending_relot_topup}
                onChange={(v) => zmien("pending_relot_topup", v)}
                label={tt("lotauto.relot.topup")}
              />
              <Switch
                checked={pola.pending_relot_wg_planu}
                onChange={(v) => zmien("pending_relot_wg_planu", v)}
                label={tt("lotauto.relot.wgPlanu")}
              />
              <span className="hint">{tt("lotauto.relot.hint")}</span>
            </div>

            {noga.pulapLancucha > 0 && (
              <p className="hint" style={{ margin: 0 }}>
                {tt("lotauto.cap.gate", { v: noga.pulapLancucha.toFixed(2) })}
              </p>
            )}

            <div className="row" style={{ justifyContent: "flex-end", gap: 8 }}>
              <Button variant="ghost" onClick={onClose}>
                {tt("common.cancel")}
              </Button>
              <Button variant="primary" icon="check" disabled={zapis} onClick={zapisz}>
                {tt("lotauto.save")}
              </Button>
            </div>
          </>
        )}
      </div>
    </Modal>
  );
}

/* ============================================================
   DYMEK — rozbicie na nogi (ikona przy kaflu pulpitu)
   ============================================================ */


function lotKoszyka(n: Noga): number {
  return typeof n.lotKoszyka === "number" && n.lotKoszyka > 0 ? n.lotKoszyka : n.lot;
}

/** Czy backend w ogóle zna ekspozycję koszyka (inaczej nie obiecujemy prawdy). */
function znaKoszyk(n: Noga): boolean {
  return typeof n.lotKoszyka === "number" && n.lotKoszyka > 0;
}



export interface EkspozycjaAuto {
  sufit: SufitAuto;
  rachunek: Rachunek;
  /** czego brakuje, żeby policzyć margines (pusto = policzone) */
  braki: BrakDanej[];
  /** koszt sufitu w marginesie */
  koszt: KosztLota;
  /** koszt granicy z pułapu koszyków (`null` = pułapu nie ma albo nie ma z czego liczyć) */
  granica: KosztLota | null;
  /** poziom marginesu TERAZ — prosto z backendu, nie liczony tutaj */
  mlTeraz: number | null;
  symbol: string;
  /** `konto_dzwignia` nóg różna od dźwigni terminala (`0` = brak rozjazdu) */
  dzwigniaSilnika: number;
}

/**
 * Sufit ekspozycji automatu i jego cena w marginesie.
 *
 * ŹRÓDŁA — wszystkie z backendu, żadnego zgadywania:
 *   nogi i koszyki   `stats.lotNogi` (`Engine::lot_koszyka_planowany`)
 *   pułap lotów      `stats.lotNogi[].pulapLancucha` (= `silniki.pulapy.max_lotow`)
 *   pułapy pozycji   `lancuch.pulapy` (łańcuch BIEŻĄCEGO trybu)
 *   saldo i equity   `stats`
 *   dźwignia         `connection.account.leverage`
 *   cena             `primary` (ask, spadek na bid)
 *   symbol           `primary.symbol` (runtime binding, not AUTO preference)
 */
export function useEkspozycjaAuto(): EkspozycjaAuto {
  const app = useApp();
  const nogi = app.stats.lotNogi ?? [];
  const pulapy = app.lancuch?.pulapy;
  const symbol = app.primary.symbol;

  
  const cena = Number.isFinite(app.primary.ask) ? app.primary.ask : app.primary.bid;

  /* ROZJAZD DŹWIGNI. `konto_dzwignia` w presecie nogi przestawia dźwignię,
     którą liczą BRAMKI MARGINESU SILNIKA (`Engine::dzwignia_efektywna`) —
     ale nie rusza dźwigni, którą margines liczy BROKER. Panel liczy
     margines rachunku, więc bierze dźwignię terminala; gdy preset ustawia
     inną, mówimy o tym osobnym zdaniem zamiast po cichu wybierać jedną. */
  const dzwigniaSilnika = useMemo(() => {
    const grajace = app.ustawieniaNog.filter((n) => n.handluje);
    const inne = grajace
      .map((n) => Number(n.doc.konto_dzwignia) || 0)
      .filter((v) => v > 0 && Math.abs(v - app.connection.account.leverage) > 0.5);
    return inne.length > 0 ? Math.max(...inne) : 0;
  }, [app.ustawieniaNog, app.connection.account.leverage]);

  return useMemo(() => {
    const sufit = policzSufit(
      nogi.map((n) => ({
        lot: n.lot,
        lotKoszyka: n.lotKoszyka,
        handluje: n.handluje ?? !n.zamrozona,
        pulapLancucha: n.pulapLancucha,
      })),
      {
        maxLotow: pulapy?.maxLotow ?? 0,
        maxPozycji: pulapy?.maxPozycji ?? 0,
        maxKoszykow: pulapy?.maxKoszykow ?? 0,
      },
    );
    const rachunek: Rachunek = {
      balance: app.stats.balance,
      equity: app.stats.equity,
      dzwignia: app.connection.account.leverage,
      cena,
      kontrakt: kontraktSymbolu(symbol),
    };
    return {
      sufit,
      rachunek,
      braki: brakiRachunku(rachunek),
      koszt: kosztLota(sufit.sufit, rachunek),
      granica: sufit.granicaKoszykow > 0 ? kosztLota(sufit.granicaKoszykow, rachunek) : null,
      mlTeraz: app.stats.marginLevel > 0 ? app.stats.marginLevel : null,
      symbol,
      dzwigniaSilnika,
    };
  }, [
    nogi,
    pulapy,
    app.stats.balance,
    app.stats.equity,
    app.stats.marginLevel,
    app.connection.account.leverage,
    cena,
    symbol,
    dzwigniaSilnika,
  ]);
}

/** Zdanie „czego brakuje" — kody na tekst, po jednym literalnym kluczu. */
function tekstBraku(b: BrakDanej, symbol: string, tt: ReturnType<typeof useT>): string {
  switch (b) {
    case "cena":
      return tt("lotauto.brak.cena", { s: symbol });
    case "dzwignia":
      return tt("lotauto.brak.dzwignia");
    case "kontrakt":
      return tt("lotauto.brak.kontrakt", { s: symbol });
    case "saldo":
      return tt("lotauto.brak.saldo");
    case "koszyk":
      return tt("lotauto.brak.koszyk");
  }
}

/**
 * SUFIT + PRZELICZENIE NA RACHUNEK — blok wspólny dla karty i dymka.
 *
 * Kolejność zdań jest celowa: najpierw liczba, potem KTÓRY pułap ją ustawia,
 * potem ile to kosztuje marginesu, a na końcu granica, której pułapy nie
 * gwarantują. Odwrotna kolejność (najpierw zastrzeżenia) sprawia, że
 * czytelnik dochodzi do liczby już zmęczony i bierze ją na wiarę.
 */
export function SufitEkspozycji({ ramka = true }: { ramka?: boolean }) {
  const tt = useT();
  const e = useEkspozycjaAuto();
  /* Waluta WYŚWIETLANIA panelu — ta sama, w której pasek pokazuje saldo.
     Margines w dolarach obok salda w złotówkach byłby nieporównywalny. */
  const cur = useApp().settings.display_currency;
  const s = e.sufit;

  if (!s.sanNogi) return null;

  const wiaze =
    s.wiaze === "loty"
      ? tt("lotauto.wiaze.loty", { v: num(s.pulapLotow, 2), f: num(s.fala, 2) })
      : s.wiaze === "pozycje"
        ? tt("lotauto.wiaze.pozycje", {
            n: String(s.maxPozycji),
            z: num(s.najwiekszeZlecenie, 2),
            v: num(s.pulapZPozycji, 2),
            f: num(s.fala, 2),
          })
        : tt("lotauto.wiaze.fala", { f: num(s.fala, 2) });

  return (
    <section className="lotsuf" data-stan={e.koszt.stan} data-ramka={ramka}>
      <div className="lotsuf__top">
        <span className="lotsuf__label">{tt("lotauto.sufit.head")}</span>
        <b className="num lotsuf__val">{num(s.sufit, 2)}</b>
        <span className="lotsuf__unit">{tt("lotauto.sufit.unit")}</span>
        <span className="lotsuf__chip">{tt(`lotauto.stan.${e.koszt.stan}`)}</span>
      </div>

      <p className="lotsuf__line">{wiaze}</p>

      {/* PRZELICZENIE NA RACHUNEK — albo komplet liczb, albo jawne „nie da
          się". Trzeciej drogi tu nie ma: podstawiona liczba obok
          prawdziwego salda jest gorsza niż brak liczby. */}
      {e.braki.length === 0 && e.koszt.margines !== null ? (
        <p className="lotsuf__line lotsuf__rach">
          {tt("lotauto.rach.line", {
            m: money(e.koszt.margines, cur),
            p: pct(e.koszt.pctSalda ?? 0, 0),
            b: money(e.rachunek.balance, cur),
            ml: pct(e.koszt.ml ?? 0, 0),
          })}
          {e.mlTeraz !== null && ` · ${tt("lotauto.rach.teraz", { v: pct(e.mlTeraz, 0) })}`}
        </p>
      ) : (
        <p className="lotsuf__line lotsuf__brak">
          {tt("lotauto.rach.brak", {
            v: e.braki.map((b) => tekstBraku(b, e.symbol, tt)).join(", "),
          })}
        </p>
      )}

      {e.braki.length === 0 && (
        <p className="lotsuf__wzor">
          {tt("lotauto.rach.wzor", {
            k: num(e.rachunek.kontrakt ?? 0, 0),
            c: num(e.rachunek.cena, 2),
            d: String(e.rachunek.dzwignia),
          })}
        </p>
      )}

      {}
      {e.granica && e.granica.margines !== null && (
        <p className="lotsuf__granica" data-stan={e.granica.stan}>
          {tt("lotauto.granica", {
            n: String(s.maxKoszykow),
            k: num(s.najwiekszyKoszyk, 2),
            v: num(s.granicaKoszykow, 2),
            m: money(e.granica.margines, cur),
            ml: pct(e.granica.ml ?? 0, 0),
          })}{" "}
          <span className="lotsuf__hint">{tt("lotauto.granica.hint")}</span>
        </p>
      )}

      {!s.koszykZnany && <p className="lotsuf__ostrz">{tt("lotauto.sufit.koszykNieznany")}</p>}
      {e.dzwigniaSilnika > 0 && (
        <p className="lotsuf__ostrz">
          {tt("lotauto.dzwignia.rozjazd", {
            v: String(e.dzwigniaSilnika),
            d: String(e.rachunek.dzwignia),
          })}
        </p>
      )}
    </section>
  );
}

function TrescDymka({ nogi }: { nogi: Noga[] }) {
  const tt = useT();
  const grajace = nogi.filter(handluje);
  const sumaLot = grajace.reduce((a, n) => a + n.lot, 0);
  const sumaKoszyk = grajace.reduce((a, n) => a + lotKoszyka(n), 0);
  const sumaWol = nogi.reduce((a, n) => a + n.wolumenWykonany, 0);
  /* Podpis mówi o koszyku tylko wtedy, gdy KAŻDA grająca noga umie go podać.
     Mieszanka „nowa noga + stara noga" dałaby sumę pół-prawdziwą, a od tej
     liczby zależy, czy właściciel konta uzna ekspozycję za bezpieczną. */
  const koszykZnany = grajace.length > 0 && grajace.every(znaKoszyk);

  return (
    <div className="lotauto__tip">
      <header className="lotauto__tip-top">
        <span className="lotauto__tip-title">{tt("lotauto.head")}</span>
        <b className="num lotauto__tip-suma">{(koszykZnany ? sumaKoszyk : sumaLot).toFixed(2)}</b>
      </header>

      {}
      <SufitEkspozycji ramka={false} />

      <ul className="lotauto__tip-list">
        {nogi.map((n) => {
          const g = grupa(n);
          const gra = handluje(n);
          return (
            <li className="tipleg" key={kluczNogi(n)} data-stan={g}>
              <div className="tipleg__row">
                <span className="tipleg__name">
                  <b className="tipleg__fmt">{n.format || tt("lotauto.noFormat")}</b>
                  <span className="tipleg__arrow">→</span>
                  <span className="tipleg__preset">{n.preset || "—"}</span>
                </span>
                <span className="tipleg__chip">{tt(`lotauto.group.${g}`)}</span>
              </div>
              <div className="tipleg__meta">
                <span>
                  {tt("lotauto.col.lot")} <b className="num">{n.lot.toFixed(2)}</b>
                </span>
                {}
                {znaKoszyk(n) && (
                  <span>
                    {tt("lotauto.col.basket")}{" "}
                    <b className="num">{lotKoszyka(n).toFixed(2)}</b>
                    {n.poziomyWejscia !== undefined && n.poziomyWejscia > 1 && (
                      <span className="tipleg__why">
                        {" "}
                        {tt("lotauto.tile.levels", { v: String(n.poziomyWejscia) })}
                      </span>
                    )}
                  </span>
                )}
                <span>
                  {tt("lotauto.col.static")}{" "}
                  <b className="num">
                    {gra && sumaLot > 0 ? `${((n.lot / sumaLot) * 100).toFixed(0)}%` : "—"}
                  </b>
                </span>
                <span>
                  {tt("lotauto.col.dynamic")}{" "}
                  <b className="num">
                    {sumaWol > 0 ? `${((n.wolumenWykonany / sumaWol) * 100).toFixed(0)}%` : "—"}
                  </b>
                </span>
                {!gra && (
                  <span className="tipleg__why">
                    {n.prog !== undefined && n.prog >= 0 && (g === "kolejka" || n.powod === "minieta")
                      ? tt("lotauto.tile.from", { v: num(n.prog, 0) })
                      : tt(`lotauto.why.${n.powod || "brakFormatu"}`)}
                  </span>
                )}
              </div>
            </li>
          );
        })}
      </ul>

      <footer className="lotauto__tip-foot">
        {/* PODPIS MÓWI O KOSZYKU, NIE O ZLECENIU. Stary tekst („tyle wejdzie
            na rachunek, gdy każda noga otworzy po jednej pozycji na tym samym
            poziomie") opisywał sytuację, która na siatce nie zachodzi NIGDY:
            sygnał stawia komplet szczebli naraz. Gdy backend nie umie podać
            koszyka, wracamy do starego zdania — ale wtedy jawnie mówimy, że
            to lot ZLECENIA, a nie ekspozycja. */}
        <span>
          {koszykZnany
            ? tt("lotauto.foot.basket", { v: sumaKoszyk.toFixed(2), z: sumaLot.toFixed(2) })
            : tt("lotauto.foot.sum", { v: sumaLot.toFixed(2) })}
        </span>
        <span>{tt("lotauto.foot.window", { v: sumaWol.toFixed(2) })}</span>
      </footer>

      {nogi.some((n) => n.zPliku === false && handluje(n)) && (
        <div className="lotauto__tip-warn">{tt("lotauto.warn.doc")}</div>
      )}
      {grajace.length === 0 && <div className="lotauto__tip-warn">{tt("lotauto.warn.none")}</div>}
    </div>
  );
}

/** Ikona z dymkiem — do wstawienia obok liczby (kafel, karta). */
export function DymekLotAuto({ nogi }: { nogi: Noga[] }) {
  if (nogi.length === 0) return null;
  return (
    <Tooltip szeroki content={<TrescDymka nogi={nogi} />}>
      <span className="lotauto__tip-icon">
        <Icon name="layers" size={12} />
      </span>
    </Tooltip>
  );
}

/* ============================================================
   PANEL NÓG — kafelki w trzech grupach + edycja lota każdej nogi
   ============================================================ */

function Kafelek({ n, onEdit }: { n: Noga; onEdit: () => void }) {
  const tt = useT();
  const g = grupa(n);
  return (
    <div className="legtile" data-stan={g}>
      <div className="legtile__head">
        <span className="legtile__fmt truncate">{n.format || tt("lotauto.noFormat")}</span>
        <button className="legtile__gear" onClick={onEdit} title={tt("lotauto.edit")} type="button">
          <Icon name="settings" size={13} />
        </button>
      </div>
      <div className="legtile__preset truncate" title={n.preset}>
        {n.preset || "—"}
      </div>
      <div className="legtile__foot">
        <b className="num legtile__lot">{n.lot.toFixed(2)}</b>
        <span className="legtile__note">
          {g === "aktywna"
            ? tt("lotauto.tile.trading")
            : n.prog !== undefined && n.prog >= 0 && (g === "kolejka" || n.powod === "minieta")
              ? tt("lotauto.tile.from", { v: num(n.prog, 0) })
              : tt(`lotauto.why.${n.powod || "brakFormatu"}`)}
        </span>
      </div>
    </div>
  );
}

/** Lista nóg pogrupowana: aktywne · w kolejce · nieaktywne. */
export function NogiLotu() {
  const tt = useT();
  const app = useApp();
  const nogi = app.stats.lotNogi ?? [];
  const eks = useEkspozycjaAuto();
  const [edytowana, setEdytowana] = useState<Noga | null>(null);

  // Noga trzymana w stanie musi nadążać za świeżymi danymi z backendu —
  // inaczej okno edycji zamraża lot sprzed sekundy.
  const zywaEdytowana = edytowana
    ? (nogi.find((n) => kluczNogi(n) === kluczNogi(edytowana)) ?? edytowana)
    : null;

  if (nogi.length === 0) return null;

  const grajace = nogi.filter(handluje);

  const grupy: { klucz: "aktywna" | "kolejka" | "nieaktywna"; lista: Noga[] }[] = [
    { klucz: "aktywna", lista: nogi.filter((n) => grupa(n) === "aktywna") },
    { klucz: "kolejka", lista: nogi.filter((n) => grupa(n) === "kolejka") },
    { klucz: "nieaktywna", lista: nogi.filter((n) => grupa(n) === "nieaktywna") },
  ];

  return (
    <div className="lotauto">
      {}
      <div className="lotauto__sum" data-stan={eks.koszt.stan}>
        <span className="lotauto__sum-label">
          {tt("lotauto.head")}
          <DymekLotAuto nogi={nogi} />
        </span>
        <b className="num lotauto__sum-value">{num(eks.sufit.sufit, 2)}</b>
      </div>
      <p className="hint lotauto__intro">{tt("lotauto.card.intro")}</p>

      <SufitEkspozycji />
      <section className="context-help order-limits">
        <b>{tt("lotauto.limits.title")}</b>
        <p>{tt("lotauto.limits.scope")}</p>
        {app.ustawieniaNog.filter(n => n.handluje).map(n => {
          const limit = strategyOrderCeiling(n.doc.lot_max, n.doc.lot_max_z_salda, app.stats.lotBase);
          return <div className="order-limits__row" key={`${n.format}|${n.preset}`}><span>{n.format} · {n.preset}</span><b className="num">{limit.known ? limit.ceiling === null ? tt("lotauto.limits.unlimited") : `${num(limit.ceiling, 2)} lot` : "—"}</b></div>;
        })}
        <p>{tt("lotauto.limits.broker")}</p>
      </section>

      {grajace.length === 0 && <div className="lotauto__alarm">{tt("lotauto.warn.none")}</div>}

      {grupy.map(
        (g) =>
          g.lista.length > 0 && (
            <section className="lotauto__group" key={g.klucz} data-stan={g.klucz}>
              <header className="lotauto__group-head">
                <span>{tt(`lotauto.group.${g.klucz}`)}</span>
                <Badge tone={g.klucz === "aktywna" ? "accent" : "muted"}>{g.lista.length}</Badge>
              </header>
              <div className="lotauto__tiles">
                {g.lista.map((n) => (
                  <Kafelek key={kluczNogi(n)} n={n} onEdit={() => setEdytowana(n)} />
                ))}
              </div>
            </section>
          ),
      )}

      {zywaEdytowana && (
        <EdytorLota
          noga={zywaEdytowana}
          wszystkie={nogi.filter((n) => n.preset)}
          onNoga={setEdytowana}
          onClose={() => setEdytowana(null)}
        />
      )}
    </div>
  );
}
