import { useEffect, useMemo, useState } from "react";
import { Badge, Button, Icon, Modal, NumberInput, Select, Switch, TextInput, Tooltip } from "@/components/ui";
import { useApp } from "@/store/AppStore";
import { RichT, t, useT } from "@/i18n";
import { opisFormatu, opisPresetu } from "@/i18n/silnik";
import { PUSTE_PULAPY, nazwaWolna, presetDlaFormatu, sufit } from "@/data/formaty";
import { czyCzempion, formatPresetu, presetyDlaFormatu } from "@/data/presets";
import type { Lancuch, Lancuchy, PulapyGlobalne, Preset, Settings } from "@/types";
import "./panels.css";

/* ============================================================
   PANEL ZARZĄDZANIA ŁAŃCUCHAMI

   Łańcuch to komplet decyzji „czym gram na którym kanale": mapa
   `format → preset` plus pułapy obowiązujące ponad presetami.

   Dwie rzeczy, których ten ekran NIE MOŻE zrobić:

   1. **Pokazać presetu spoza formatu.** Wymaganie twarde użytkownika:
      w liście formatu ATFX nie ma prawa pojawić się preset Synergy.
   2. **Zapisać zmiany po cichu.** Edycja idzie na kopii roboczej i wchodzi
      w życie dopiero przyciskiem „Zapisz".
   ============================================================ */

/** Wartość „nie handluj tym formatem". Pusty łańcuch znaków znaczy dokładnie
 *  to samo co brak klucza — tak samo jak `Lancuch::preset_dla` po stronie Rusta. */
const NIE_HANDLUJ = "";

export function LancuchyPanel({ open, onClose }: { open: boolean; onClose: () => void }) {
  const app = useApp();
  const tt = useT();

  /* Kopia robocza CAŁEGO zbioru. Trzymamy zbiór, a nie pojedynczy łańcuch,
     bo „nowy", „zmień nazwę" i „usuń" to operacje na liście. */
  const [robocze, setRobocze] = useState<Lancuchy>(app.lancuchy);
  /* WSKAŹNIK WARSTWY EA W KOPII ROBOCZEJ (projekt EA-2).

     Trzymamy go OSOBNO od `robocze.aktywny`, dokładnie tak, jak leży na
     dysku: to dwa niezależne wskazania nad JEDNĄ listą. Gdyby panel scalał
     je w jedno pole „aktywny", przełączenie składu w AUTO-EA przy zapisie
     przestawiłoby też skład, którym gra AUTO. */
  const [roboczyEa, setRoboczyEa] = useState(app.aktywnyEa);
  const [wybrany, setWybrany] = useState(app.aktywnaNazwaLancucha);
  const [zmienione, setZmienione] = useState(false);
  const [nowaNazwa, setNowaNazwa] = useState("");
  const [tryb, setTryb] = useState<"brak" | "nowy" | "nazwa">("brak");

  /* CZY OKNO PRACUJE NA WSKAŹNIKU WARSTWY EA. Jedna flaga na cały komponent:
     w AUTO-EA „uczyń aktywnym" pisze do `aktywnyEa`, w pozostałych trybach do
     `aktywny`, a plakietka „●" ma świecić przy tym wskazaniu, które w TYM
     trybie naprawdę rządzi rachunkiem. */
  const ea = app.mode === "AUTO-EA";
  /* Nazwa obowiązująca w kopii roboczej — bliźniak `aktywnyDla()` liczony na
     szkicu, a nie na stanie z serwera (ten jeszcze nie wie o zmianach). */
  const aktywnyRoboczy =
    ea && roboczyEa && robocze.lista.some((l) => l.nazwa === roboczyEa) ? roboczyEa : robocze.aktywny;

  /* Otwarcie okna zawsze zaczyna od stanu prawdziwego. Bez tego druga wizyta
     pokazywałaby porzucone szkice sprzed zamknięcia i wyglądałaby, jakby
     zmiany zostały zapisane. */
  useEffect(() => {
    if (!open) return;
    setRobocze(app.lancuchy);
    setRoboczyEa(app.aktywnyEa);
    setWybrany(app.aktywnaNazwaLancucha);
    setZmienione(false);
    setTryb("brak");
    setNowaNazwa("");
    // celowo tylko na otwarciu — dalej rządzi kopia robocza
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const lancuch = robocze.lista.find((l) => l.nazwa === wybrany) ?? robocze.lista[0];

  const zmien = (patch: Partial<Lancuch>) => {
    if (!lancuch) return;
    setRobocze((z) => ({
      ...z,
      lista: z.lista.map((l) => (l.nazwa === lancuch.nazwa ? { ...l, ...patch } : l)),
    }));
    setZmienione(true);
  };

  const ustawPreset = (format: string, preset: string) => {
    if (!lancuch) return;
    zmien({ presety: { ...lancuch.presety, [format]: preset } });
  };

  const ustawPulap = (patch: Partial<PulapyGlobalne>) => {
    if (!lancuch) return;
    zmien({ pulapy: { ...lancuch.pulapy, ...patch } });
  };

  const dodaj = () => {
    const n = nowaNazwa.trim();
    if (!nazwaWolna(robocze, n)) return;
    // Nowy łańcuch startuje jako KOPIA bieżącego, nie jako pustka: pusty
    // łańcuch znaczy „nic nie handluje", więc byłby to najgorszy możliwy
    // domyślny stan do przypadkowego zapisania.
    const bazowy: Lancuch = lancuch
      ? { ...lancuch, nazwa: n, opis: tt("lanc.copyOf", { name: lancuch.nazwa }) }
      : { nazwa: n, opis: "", presety: {}, pulapy: { ...PUSTE_PULAPY } };
    setRobocze((z) => ({ ...z, lista: [...z.lista, bazowy] }));
    setWybrany(n);
    setZmienione(true);
    setTryb("brak");
    setNowaNazwa("");
  };

  const przemianuj = () => {
    const n = nowaNazwa.trim();
    if (!lancuch || !nazwaWolna(robocze, n, lancuch.nazwa)) return;
    const stara = lancuch.nazwa;
    setRobocze((z) => ({
      aktywny: z.aktywny === stara ? n : z.aktywny,
      lista: z.lista.map((l) => (l.nazwa === stara ? { ...l, nazwa: n } : l)),
    }));
    // Wskaźnik EA jeździ za nazwą TAK SAMO jak `aktywny`. Bez tego zmiana
    // nazwy łańcucha, którym gra AUTO-EA, zamieniałaby wskazanie w sierotę
    // — a sierota po cichu spada na fallback, czyli na CUDZY skład.
    setRoboczyEa((v) => (v === stara ? n : v));
    setWybrany(n);
    setZmienione(true);
    setTryb("brak");
    setNowaNazwa("");
  };

  const usun = () => {
    if (!lancuch || robocze.lista.length <= 1) return;
    const reszta = robocze.lista.filter((l) => l.nazwa !== lancuch.nazwa);
    setRobocze({
      aktywny: robocze.aktywny === lancuch.nazwa ? reszta[0].nazwa : robocze.aktywny,
      lista: reszta,
    });
    // Skasowanie łańcucha wskazanego przez warstwę EA zdejmuje wskazanie
    // wprost (pustka = fallback na `aktywny`), zamiast zostawiać nazwę,
    // której już nie ma.
    setRoboczyEa((v) => (v === lancuch.nazwa ? "" : v));
    setWybrany(reszta[0].nazwa);
    setZmienione(true);
  };

  const zapisz = () => {
    /* WSKAŹNIK EA LECI TYLKO Z TRYBU EA. W pozostałych trybach pomijamy pole,
       a serwer czyta to jako „nie ruszaj" — dzięki temu edycja pułapów zrobiona
       w AUTO nie kasuje składu warstwy EA. */
    app.setLancuchy(robocze, ea ? roboczyEa : undefined);
    app.toast(
      "success",
      tt("lanc.toast.saved"),
      tt("lanc.toast.savedText", { n: robocze.lista.length, name: aktywnyRoboczy }),
    );
    setZmienione(false);
    onClose();
  };

  const aktywuj = () => {
    if (!lancuch) return;
    if (ea) setRoboczyEa(lancuch.nazwa);
    else setRobocze((z) => ({ ...z, aktywny: lancuch.nazwa }));
    setZmienione(true);
  };

  const handlujace = lancuch
    ? app.formaty.filter((f) => presetDlaFormatu(lancuch, f.nazwa) !== null).length
    : 0;

  return (
    <Modal
      open={open}
      onClose={onClose}
      width={880}
      title={
        <span className="row row--tight">
          <Icon name={ea ? "robot" : "layers"} size={16} />
          {tt(ea ? "lanc.title.ea" : "lanc.title")}
        </span>
      }
      subtitle={
        <>
          <RichT k={ea ? "lanc.subtitle.ea" : "lanc.subtitle"} />
          {!app.lancuchyZSerwera && <> {tt("lanc.subtitle.local")}</>}
        </>
      }
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {tt("lanc.closeNoSave")}
          </Button>
          <Button variant="primary" icon="check" disabled={!zmienione} onClick={zapisz}>
            {zmienione ? tt("lanc.saveBtn") : tt("lanc.noChanges")}
          </Button>
        </>
      }
    >
      {/* ---------------- wybór łańcucha ---------------- */}
      <div className="lanc__pick">
        <div style={{ flex: 1, minWidth: 220 }}>
          <label className="setfield__label" htmlFor="lanc-wybor">
            {tt("lanc.chain")}
          </label>
          <Select
            id="lanc-wybor"
            value={lancuch?.nazwa ?? ""}
            onChange={(v) => {
              setWybrany(v);
              setTryb("brak");
            }}
            options={robocze.lista.map((l) => ({
              value: l.nazwa,
              label: `${l.nazwa === aktywnyRoboczy ? "● " : ""}${l.nazwa}`,
            }))}
          />
        </div>
        <div className="row row--tight" style={{ alignItems: "flex-end", paddingBottom: 1 }}>
          <Button
            size="sm"
            variant={lancuch?.nazwa === aktywnyRoboczy ? "outline" : "primary"}
            icon="bolt"
            disabled={lancuch?.nazwa === aktywnyRoboczy}
            onClick={aktywuj}
            title={tt(ea ? "lanc.makeActive.ea.title" : "lanc.makeActive.title")}
          >
            {lancuch?.nazwa === aktywnyRoboczy ? tt("common.active") : tt("lanc.makeActive")}
          </Button>
          <Button
            size="sm"
            variant="outline"
            icon="plus"
            onClick={() => {
              setTryb("nowy");
              setNowaNazwa("");
            }}
          >
            {tt("common.new")}
          </Button>
          <Button
            size="sm"
            variant="outline"
            icon="pencil"
            disabled={!lancuch}
            onClick={() => {
              setTryb("nazwa");
              setNowaNazwa(lancuch?.nazwa ?? "");
            }}
          >
            {tt("common.rename")}
          </Button>
          <Tooltip content={robocze.lista.length <= 1 ? tt("lanc.delete.last") : tt("lanc.delete.title")}>
            <Button size="sm" variant="danger" icon="trash" disabled={robocze.lista.length <= 1} onClick={usun}>
              {tt("common.delete")}
            </Button>
          </Tooltip>
        </div>
      </div>

      {tryb !== "brak" && (
        <div className="lanc__rename">
          <TextInput
            value={nowaNazwa}
            onChange={setNowaNazwa}
            autoFocus
            placeholder={tryb === "nowy" ? tt("lanc.name.new") : tt("lanc.name.rename")}
            onKeyDown={(e) => {
              if (e.key === "Enter") (tryb === "nowy" ? dodaj : przemianuj)();
              if (e.key === "Escape") setTryb("brak");
            }}
          />
          <Button
            size="sm"
            variant="primary"
            icon="check"
            disabled={!nazwaWolna(robocze, nowaNazwa, tryb === "nazwa" ? lancuch?.nazwa : undefined)}
            onClick={tryb === "nowy" ? dodaj : przemianuj}
          >
            {tryb === "nowy" ? tt("lanc.createCopy") : tt("lanc.renameBtn")}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setTryb("brak")}>
            {tt("common.cancel")}
          </Button>
          {nowaNazwa.trim() !== "" && !nazwaWolna(robocze, nowaNazwa, tryb === "nazwa" ? lancuch?.nazwa : undefined) && (
            <span className="setfield__warn">{tt("lanc.nameTaken")}</span>
          )}
        </div>
      )}

      {lancuch && (
        <>
          <div className="lanc__opis">
            <TextInput
              value={lancuch.opis}
              onChange={(v) => zmien({ opis: v })}
              placeholder={tt("lanc.desc.placeholder")}
            />
          </div>

          {/* ---------------- formaty → presety ---------------- */}
          <div className="lanc__sekcja">
            <span className="eyebrow">
              {tt("lanc.formats")}
              <Badge tone={handlujace > 0 ? "accent" : "warn"}>
                {tt("lanc.trading", { n: handlujace, m: app.formaty.length })}
              </Badge>
            </span>
            {handlujace === 0 && (
              <p className="setfield__warn" style={{ marginBottom: "var(--sp-2)" }}>
                <Icon name="alert" size={12} /> {tt("lanc.noTrading")}
              </p>
            )}
            <div className="lanc__grid">
              {app.formaty.map((f) => (
                <WierszFormatu
                  key={f.nazwa}
                  format={f.nazwa}
                  parser={f.parser}
                  opis={opisFormatu(f.nazwa, f.opis)}
                  wartosc={lancuch.presety[f.nazwa] ?? NIE_HANDLUJ}
                  presety={app.presets}
                  onChange={(v) => ustawPreset(f.nazwa, v)}
                  /* SKRÓT DO PARAMETRÓW EA TEGO FORMATU — tylko w AUTO-EA.

                     Parametry warstwy EA to ZWYKŁE pola presetu (`zakres:
                     "preset"` w schemacie), więc „EA formatu Synergy" znaczy
                     dosłownie „sekcja EA presetu, którym Synergy gra". Skrót
                     prowadzi wprost tam, zamiast kazać przechodzić przez
                     Ustawienia → wybór presetu → szukanie sekcji.

                     Zapisu tu NIE MA: szkic łańcucha zostaje w tym oknie,
                     a parametry idą do PLIKU presetu — to dwie różne
                     warstwy i mieszanie ich jednym przyciskiem kazałoby
                     zgadywać, co zostało zapisane. */
                  onParametryEa={
                    ea
                      ? (preset) => {
                          app.pokazParametryEa(preset);
                          onClose();
                        }
                      : undefined
                  }
                />
              ))}
            </div>
          </div>

          {/* ---------------- pułapy globalne ---------------- */}
          <PulapySekcja
            pulapy={lancuch.pulapy}
            onChange={ustawPulap}
            presety={app.formaty
              .map((f) => lancuch.presety[f.nazwa])
              .filter((n): n is string => !!n && n.trim() !== "")
              .map((n) => app.findPreset(n))
              .filter((p): p is Preset => !!p)}
          />
        </>
      )}
    </Modal>
  );
}

/* ============================================================
   JEDEN FORMAT + jego preset
   ============================================================ */
function WierszFormatu({
  format,
  parser,
  opis,
  wartosc,
  presety,
  onChange,
  onParametryEa,
}: {
  format: string;
  parser: string;
  opis: string;
  wartosc: string;
  presety: Preset[];
  onChange: (v: string) => void;
  /** Skrót „parametry EA tego formatu"; `undefined` = tryb inny niż AUTO-EA. */
  onParametryEa?: (preset: string) => void;
}) {
  const tt = useT();
  /* WYŁĄCZNIE presety tego formatu. To jest filtr, a nie sortowanie:
     preset innego formatu nie ma prawa się tu pojawić nawet na dole listy. */
  const moje = useMemo(() => presetyDlaFormatu(presety, format), [presety, format]);
  const handluje = wartosc.trim() !== "";
  /* Wybrana nazwa, której nie ma wśród presetów tego formatu — łańcuch
     wskazuje preset skasowany z dysku albo należący do innego formatu.
     Cichy powrót do „nie handluj" byłby gorszy: bot dalej gra, a panel
     twierdziłby, że wyłączono. */
  const osierocony = handluje && !moje.some((p) => p.name === wartosc);
  /* DWA RÓŻNE POWODY OSIEROCENIA, DWIE RÓŻNE PORADY — patrz komentarz
     w historii pliku: preset w innej kategorii to wybór świadomy,
     preset nieobecny na dysku to realna awaria. */
  const gdzieIndziej = useMemo(
    () => (osierocony ? presety.find((p) => p.name === wartosc) : undefined),
    [osierocony, presety, wartosc],
  );

  return (
    <div className="lancrow" data-on={handluje}>
      <div className="lancrow__fmt">
        <span className="lancrow__icon">
          <Icon name="book" size={14} />
        </span>
        <div style={{ minWidth: 0 }}>
          <div className="row row--tight">
            <b className="truncate">{format}</b>
            <Badge tone="muted">{tt("lanc.parser", { v: parser })}</Badge>
          </div>
          <span className="lancrow__desc">{opis}</span>
        </div>
      </div>

      <div className="lancrow__preset">
        <Select
          value={wartosc}
          onChange={onChange}
          options={[
            { value: NIE_HANDLUJ, label: tt("lanc.dontTrade") },
            ...(osierocony
              ? [
                  {
                    value: wartosc,
                    label: gdzieIndziej
                      ? tt("lanc.orphan.otherCat", { name: wartosc, cat: formatPresetu(gdzieIndziej) })
                      : tt("lanc.orphan.missing", { name: wartosc }),
                  },
                ]
              : []),
            ...moje.map((p) => ({
              value: p.name,
              label: `${czyCzempion(p.name) ? "👑 " : ""}${p.name} — ${opisPresetu(p.id, p.tagline)}`,
              group: `${format}`,
            })),
          ]}
        />
        {osierocony ? (
          gdzieIndziej ? (
            <span className="lancrow__note">
              <Icon name="alert" size={12} />{" "}
              <RichT k="lanc.orphan.otherCatNote" vars={{ name: wartosc, cat: formatPresetu(gdzieIndziej), fmt: format }} />
            </span>
          ) : (
            <span className="setfield__warn">
              <Icon name="alert" size={12} />{" "}
              <RichT k="lanc.orphan.missingNote" vars={{ name: wartosc, fmt: format }} />
            </span>
          )
        ) : (
          <span className="lancrow__note">
            {handluje ? tt("lanc.presetsInCategory", { n: moje.length, fmt: format }) : tt("lanc.listenOnly")}
          </span>
        )}
        {/* Przycisk pojawia się TYLKO przy formacie, który naprawdę gra:
            preset „nie handluj" nie ma sekcji EA do nastawiania, a przycisk
            prowadzący w pustkę jest gorszy niż jego brak. */}
        {onParametryEa && handluje && (
          <Button
            size="sm"
            variant="outline"
            icon="robot"
            onClick={() => onParametryEa(wartosc)}
            title={tt("lanc.eaParams.title", { name: wartosc })}
          >
            {tt("lanc.eaParams")}
          </Button>
        )}
      </div>
    </div>
  );
}

/* ============================================================
   PUŁAPY GLOBALNE — sufit nad presetami

   Preset pilnuje SIEBIE. Dwa presety po 30 pozycji nie łamią własnych
   limitów, a na rachunku robi się 60, bo rachunek jest jeden. Stąd ta
   warstwa: `limit skuteczny = min(limit presetu, limit globalny)`.

   `0` = BEZ PUŁAPU. Piszemy to przy każdym polu, bo pomyłka w tę stronę
   („zero to zakaz") wyłączyłaby handel — mieliśmy już tę klasę błędu przy
   `reenter_max` i kosztowała realny rozjazd wyników.
   ============================================================ */

type PulapKey = keyof PulapyGlobalne;

interface PulapDef {
  key: PulapKey;
  /** klucz słownika: `<labelKey>` = etykieta, `<labelKey>.hint` = opis */
  labelKey: string;
  hintKey: string;
  unit?: string;
  step?: number;
  /** klucz ustawienia presetu, którego SUMA mówi, czy pułap w ogóle ogranicza */
  suma?: keyof Settings;
  /** czy sumę liczyć jako maksimum, a nie sumę (np. procenty obsunięcia) */
  jakoMaks?: boolean;
}

const PULAPY_SEKCJE: { tytulKey: string; opisKey: string; pola: PulapDef[] }[] = [
  {
    tytulKey: "lanc.caps.exposure",
    opisKey: "lanc.caps.exposure.desc",
    pola: [
      { key: "maxPozycji", labelKey: "lanc.caps.maxPos", hintKey: "lanc.caps.maxPos.hint", suma: "max_open_positions" },
      { key: "maxKoszykow", labelKey: "lanc.caps.maxBaskets", hintKey: "lanc.caps.maxBaskets.hint", suma: "max_open_baskets" },
      { key: "maxLotow", labelKey: "lanc.caps.maxLots", hintKey: "lanc.caps.maxLots.hint", step: 0.01 },
      {
        key: "maxLotowKierunkowo",
        labelKey: "lanc.caps.maxDirLots",
        hintKey: "lanc.caps.maxDirLots.hint",
        step: 0.01,
        suma: "max_directional_lots",
      },
      { key: "maxRyzykoPct", labelKey: "lanc.caps.maxRisk", hintKey: "lanc.caps.maxRisk.hint", unit: "%", step: 1 },
    ],
  },
  {
    tytulKey: "lanc.caps.dd",
    opisKey: "lanc.caps.dd.desc",
    pola: [
      { key: "maxDdPct", labelKey: "lanc.caps.maxDd", hintKey: "lanc.caps.maxDd.hint", unit: "%", step: 1, suma: "max_dd_pct", jakoMaks: true },
      { key: "maxDdUsd", labelKey: "lanc.caps.maxDd", hintKey: "lanc.caps.maxDdUsd.hint", unit: "$", step: 10, suma: "max_dd_usd", jakoMaks: true },
      { key: "podlogaEquityUsd", labelKey: "lanc.caps.floor", hintKey: "lanc.caps.floor.hint", unit: "$", step: 10 },
    ],
  },
  {
    tytulKey: "lanc.caps.day",
    opisKey: "lanc.caps.day.desc",
    pola: [
      { key: "celDniaUsd", labelKey: "lanc.caps.dayTarget", hintKey: "lanc.caps.dayTarget.hint", unit: "$", step: 10, suma: "day_target_usd" },
      { key: "celDniaPct", labelKey: "lanc.caps.dayTarget", hintKey: "lanc.caps.dayTargetPct.hint", unit: "%", step: 0.5, suma: "day_target_pct" },
      { key: "limitStratyDniaUsd", labelKey: "lanc.caps.dayLoss", hintKey: "lanc.caps.dayLoss.hint", unit: "$", step: 10 },
      { key: "limitStratyDniaPct", labelKey: "lanc.caps.dayLoss", hintKey: "lanc.caps.dayLossPct.hint", unit: "%", step: 0.5 },
    ],
  },
  {
    tytulKey: "lanc.caps.conflicts",
    opisKey: "lanc.caps.conflicts.desc",
    pola: [
      { key: "pauzaPoStratachN", labelKey: "lanc.caps.pauseN", hintKey: "lanc.caps.pauseN.hint", suma: "streak_pause_n" },
      { key: "pauzaPoStratachMin", labelKey: "lanc.caps.pauseMin", hintKey: "lanc.caps.pauseMin.hint", unit: "min", step: 15 },
    ],
  },
];

function PulapySekcja({
  pulapy,
  onChange,
  presety,
}: {
  pulapy: PulapyGlobalne;
  onChange: (p: Partial<PulapyGlobalne>) => void;
  presety: Preset[];
}) {
  const tt = useT();
  const ustawione = (Object.keys(PUSTE_PULAPY) as PulapKey[]).filter((k) => {
    const v = pulapy[k];
    return typeof v === "boolean" ? v : v > 0;
  }).length;

  /** Suma (albo maksimum) tego samego pola we WSZYSTKICH wybranych presetach. */
  const zPresetow = (def: PulapDef): number | null => {
    if (!def.suma || presety.length === 0) return null;
    const liczby = presety.map((p) => Number(p.values?.[def.suma!] ?? 0)).filter((n) => Number.isFinite(n) && n > 0);
    if (liczby.length === 0) return null;
    return def.jakoMaks ? Math.max(...liczby) : liczby.reduce((a, b) => a + b, 0);
  };

  return (
    <div className="lanc__sekcja">
      <span className="eyebrow">
        {tt("lanc.caps")}
        <Badge tone={ustawione > 0 ? "accent" : "muted"}>{tt("lanc.caps.set", { n: ustawione })}</Badge>
      </span>
      <p className="hint" style={{ marginBottom: "var(--sp-3)" }}>
        <RichT k="lanc.caps.intro" />
      </p>

      {PULAPY_SEKCJE.map((s) => (
        <div key={s.tytulKey} className="lanc__pulapy">
          <div className="lanc__pulapyhead">
            <b>{tt(s.tytulKey)}</b>
            <span className="hint">{tt(s.opisKey)}</span>
          </div>
          <div className="setgrid">
            {s.pola.map((def) => {
              const wartosc = Number(pulapy[def.key]);
              const suma = zPresetow(def);
              /* Najważniejsza informacja tej sekcji: czy pułap w ogóle
                 cokolwiek ogranicza. Pułap wyższy niż suma limitów presetów
                 jest napisem na ścianie, nie zabezpieczeniem. */
              const martwy = wartosc > 0 && suma !== null && wartosc >= suma;
              return (
                <div key={String(def.key)} className="setfield">
                  <label className="setfield__label">
                    {tt(def.labelKey)}
                    {def.unit ? ` (${def.unit})` : ""}
                  </label>
                  <NumberInput
                    value={wartosc}
                    onChange={(v) => onChange({ [def.key]: v } as Partial<PulapyGlobalne>)}
                    min={0}
                    step={def.step ?? 1}
                    unit={def.unit}
                    placeholder={tt("lanc.caps.noLimit")}
                    zeroLabel={tt("lanc.caps.noLimit")}
                  />
                  <span className="setfield__hint">
                    {tt(def.hintKey)} {wartosc === 0 && <b>{tt("lanc.caps.nowNoCap")}</b>}
                    {suma !== null && (
                      <>
                        {" "}
                        {def.jakoMaks ? tt("lanc.caps.maxOfPresets") : tt("lanc.caps.sumOfPresets")}{" "}
                        {tt("lanc.caps.ofSelected")}{" "}
                        <b>
                          {suma}
                          {def.unit ?? ""}
                        </b>
                        {wartosc > 0 && (
                          <>
                            {" "}
                            {tt("lanc.caps.effective")} <b>{sufit(wartosc, suma)}{def.unit ?? ""}</b>.
                          </>
                        )}
                      </>
                    )}
                  </span>
                  {martwy && (
                    <span className="setfield__warn">
                      <Icon name="alert" size={12} />{" "}
                      {tt("lanc.caps.dead", {
                        what: def.jakoMaks ? tt("lanc.caps.dead.max") : tt("lanc.caps.dead.sum"),
                      })}
                    </span>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      ))}

      <div className="setgrid">
        <div className="setfield setfield--bool" data-on={pulapy.celDniaZamyka}>
          <Switch
            checked={pulapy.celDniaZamyka}
            onChange={(v) => onChange({ celDniaZamyka: v })}
            label={<span className="setfield__label">{tt("lanc.caps.dayCloses")}</span>}
          />
          <span className="setfield__hint">{tt("lanc.caps.dayCloses.hint")}</span>
        </div>
        <div className="setfield setfield--bool" data-on={pulapy.blokujPrzeciwneKierunki}>
          <Switch
            checked={pulapy.blokujPrzeciwneKierunki}
            onChange={(v) => onChange({ blokujPrzeciwneKierunki: v })}
            label={<span className="setfield__label">{tt("lanc.caps.blockOpposite")}</span>}
          />
          <span className="setfield__hint">{tt("lanc.caps.blockOpposite.hint")}</span>
        </div>
      </div>
    </div>
  );
}

/* ============================================================
   WYBÓR ŁAŃCUCHA — ta sama kontrolka co w panelu, do górnego paska
   ============================================================ */
export function WyborLancucha({ size }: { size?: "sm" }) {
  const app = useApp();
  const tt = useT(); // subskrypcja: etykiety opcji przeliczają się po zmianie języka
  const nazwyFormatow = app.formaty.map((f) => f.nazwa);

  
  /* WSKAZANIE WŁAŚCIWE DLA TRYBU (projekt EA-2), nie `lancuchy.aktywny`:
     w AUTO-EA select ma pokazywać i przestawiać skład WARSTWY EA. Gdyby
     czytał wspólne pole, w trybie EA pokazywałby cudzy łańcuch, a jego
     zmiana przestawiałaby konfigurację drugiego trybu. */
  const aktywna = app.aktywnaNazwaLancucha;
  const lista = [...app.lancuchy.lista].sort((a, b) => {
    if (a.nazwa === aktywna) return -1;
    if (b.nazwa === aktywna) return 1;
    const ga = nazwyFormatow.filter((f) => presetDlaFormatu(a, f)).length;
    const gb = nazwyFormatow.filter((f) => presetDlaFormatu(b, f)).length;
    if ((ga > 0) !== (gb > 0)) return ga > 0 ? -1 : 1;
    return a.nazwa.localeCompare(b.nazwa);
  });

  return (
    <Select
      value={aktywna}
      onChange={(v) => v && v !== aktywna && app.setAktywnyLancuch(v)}
      size={size}
      style={{ minWidth: 190 }}
      options={lista.map((l) => ({
        value: l.nazwa,
        label: `${l.nazwa} · ${opisSkrotem(l, nazwyFormatow, tt)}`,
      }))}
    />
  );
}


function opisSkrotem(l: Lancuch, formaty: string[], tt: (k: "sel.noLegs" | "sel.legsOf", v?: Record<string, string | number>) => string): string {
  if (formaty.length === 0) return t("lanc.short.noFormats");
  const grajace = formaty.map((f) => [f, presetDlaFormatu(l, f)] as const).filter(([, p]) => !!p);
  if (grajace.length === 0) return tt("sel.noLegs");
  const opis = grajace.map(([f, p]) => `${f}→${p}`).join(" · ");
  return grajace.length === formaty.length
    ? opis
    : `${opis} · ${tt("sel.legsOf", { n: grajace.length, m: formaty.length })}`;
}
