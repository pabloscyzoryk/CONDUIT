import { tSilnik } from "@/i18n/silnik";
/* ============================================================
   KRONIKA — rejestrator strumienia z Telegrama.

   JEDEN komponent dla DWÓCH programów:
     * zakładka „Kronika” w `conduit.exe` — podpięta pod istniejący
       strumień bota (żadnego drugiego połączenia z Telegramem),
     * `kronika.exe` — samodzielny proces z własną sesją.

   Oba wystawiają ten sam kontrakt `/api/kronika/*`, więc widok nie
   musi wiedzieć, w którym siedzi — poza jednym zdaniem w nagłówku.

   UKŁAD EKRANU wynika wprost z powodu istnienia narzędzia:
   statystyka edycji jest PIERWSZA, nad wszystkim innym. To dla niej
   ten rejestrator powstał; opcje zapisu i podgląd są obsługą.

   NAPISY idą przez słownik (`@/i18n`). `useT()` działa TAKŻE poza
   `I18nProvider` — samodzielna `kronika.exe` (`src/kronika.tsx`) nie
   stawia providera i ma zamiast tego WŁASNY wybór języka w pasku.
   Magazyn przeglądarki NIE jest tam wspólny z panelem: rejestrator
   startuje z `--port 0`, czyli na innym porcie przy każdym starcie,
   a `localStorage` jest per źródło. Brakuje tam tylko skrótu `L`.
   ============================================================ */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Badge,
  Button,
  Card,
  Checkbox,
  Empty,
  Field,
  Icon,
  NumberInput,
  Segmented,
  Switch,
  TextInput,
  Tooltip,
} from "@/components/ui";
import { WyborSciezki } from "@/components/WyborSciezki";
import { kronikaApi } from "@/store/kronikaApi";
import { ago, compact, num } from "@/lib/format";
import { RichT, useT } from "@/i18n";
import type {
  KronikaKanal,
  KronikaRozklad,
  KronikaStan,
  KronikaStatystyki,
  KronikaUstawienia,
  KronikaWpis,
} from "@/types/kronika";
import "./kronika.css";


const TEMPO_MS = 2000;

/* ------------------------------------------------------------
   POMOCNICZE
   ------------------------------------------------------------ */

/** Symbole jednostek (B, kB, MB, GB, s, min, h) są takie same w obu
 *  językach — nie idą przez słownik. */
function bajty(b: number): string {
  if (b < 1024) return `${b} B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} kB`;
  if (b < 1024 * 1024 * 1024) return `${(b / 1024 / 1024).toFixed(1)} MB`;
  return `${(b / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

/** Sekundy po ludzku. Mediana 141 s ma się czytać „2 min 21 s”, nie „141". */
function sekundy(s: number): string {
  if (!Number.isFinite(s) || s <= 0) return "0 s";
  if (s < 60) return `${s < 10 ? s.toFixed(1) : Math.round(s)} s`;
  const m = Math.floor(s / 60);
  const r = Math.round(s % 60);
  if (m < 60) return r ? `${m} min ${r} s` : `${m} min`;
  const h = Math.floor(m / 60);
  return `${h} h ${m % 60} min`;
}

function czasKrotki(ms: number): string {
  if (!ms) return "—";
  const d = new Date(ms);
  return `${String(d.getUTCHours()).padStart(2, "0")}:${String(d.getUTCMinutes()).padStart(2, "0")}:${String(
    d.getUTCSeconds(),
  ).padStart(2, "0")}`;
}

/* ------------------------------------------------------------
   WIDOK
   ------------------------------------------------------------ */

export function KronikaView() {
  const tt = useT();
  const [stan, setStan] = useState<KronikaStan | null>(null);
  const [stat, setStat] = useState<KronikaStatystyki | null>(null);
  const [kanaly, setKanaly] = useState<KronikaKanal[]>([]);
  const [bladKanalow, setBladKanalow] = useState<string | null>(null);
  const [blad, setBlad] = useState<string | null>(null);
  const [zajete, setZajete] = useState(false);
  const [komunikat, setKomunikat] = useState<string | null>(null);
  const [karta, setKarta] = useState<"podglad" | "kanaly" | "opcje">("podglad");

  /* Robocza kopia opcji: użytkownik ma móc zmienić kilka pól i zapisać raz,
     a nie wysyłać żądania po każdym kliknięciu w pole tekstowe. */
  const [robocze, setRobocze] = useState<KronikaUstawienia | null>(null);
  const brudne = useMemo(
    () => !!robocze && !!stan && JSON.stringify(robocze) !== JSON.stringify(stan.ustawienia),
    [robocze, stan],
  );
  const brudneRef = useRef(brudne);
  brudneRef.current = brudne;

  const odswiezStan = useCallback(async () => {
    try {
      const s = await kronikaApi.stan();
      setStan(s);
      setBlad(null);
      // Nie nadpisujemy formularza, którego użytkownik właśnie dotyka —
      // odświeżanie co 2 s kasowałoby wpisywaną ścieżkę w połowie słowa.
      setRobocze((r) => (r === null || !brudneRef.current ? s.ustawienia : r));
    } catch (e) {
      setBlad(e instanceof Error ? e.message : String(e));
    }
  }, []);

  const odswiezStatystyki = useCallback(async () => {
    try {
      setStat(await kronikaApi.statystyki());
    } catch {
      /* statystyka jest dodatkiem do podglądu — jej brak nie gasi ekranu */
    }
  }, []);

  const odswiezKanaly = useCallback(async () => {
    try {
      const k = await kronikaApi.kanaly();
      setKanaly(k.kanaly);
      setBladKanalow(k.blad);
    } catch (e) {
      setBladKanalow(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    void odswiezStan();
    void odswiezStatystyki();
    void odswiezKanaly();
    const t = window.setInterval(() => void odswiezStan(), TEMPO_MS);
    // Statystyka czyta CAŁY plik, więc chodzi rzadziej niż podgląd —
    // przy tygodniowej kronice to kilkanaście megabajtów na przebieg.
    const t2 = window.setInterval(() => void odswiezStatystyki(), TEMPO_MS * 10);
    return () => {
      window.clearInterval(t);
      window.clearInterval(t2);
    };
  }, [odswiezStan, odswiezStatystyki, odswiezKanaly]);

  const zapisz = async (u: KronikaUstawienia) => {
    setZajete(true);
    setBlad(null);
    try {
      await kronikaApi.ustaw(u);
      setKomunikat(tt("kron.saved"));
      await odswiezStan();
      await odswiezKanaly();
    } catch (e) {
      setBlad(e instanceof Error ? e.message : String(e));
    } finally {
      setZajete(false);
      window.setTimeout(() => setKomunikat(null), 4000);
    }
  };

  const eksportuj = async () => {
    setZajete(true);
    setBlad(null);
    try {
      const r = await kronikaApi.eksport();
      setKomunikat(
        tt("kron.exported", {
          plik: r.wynik.plik,
          sygnalow: r.wynik.sygnalow,
          zdarzen: r.wynik.zdarzen,
          zEdycji: r.wynik.z_edycji,
        }),
      );
    } catch (e) {
      setBlad(e instanceof Error ? e.message : String(e));
    } finally {
      setZajete(false);
      window.setTimeout(() => setKomunikat(null), 8000);
    }
  };

  if (!stan) {
    return (
      <div className="view">
        <div className="view__head">
          <div className="view__headmain">
            <h1>{tt("kron.title")}</h1>
            <p>{tt("kron.connecting")}</p>
          </div>
        </div>
        {blad && (
          <Card title={tt("kron.noRecorder")} icon="alert" accent="var(--danger)">
            <p className="hint">{tSilnik(blad)}</p>
          </Card>
        )}
      </div>
    );
  }

  const u = robocze ?? stan.ustawienia;

  return (
    <div className="view">
      <div className="view__head">
        <div className="view__headmain">
          <h1>{tt("kron.title")}</h1>
          <p>
            <RichT k="kron.lead" />
          </p>
        </div>
        <div className="row row--tight">
          <Badge tone={stan.tryb === "wbudowana" ? "accent" : "info"} dot>
            {stan.tryb === "wbudowana" ? tt("kron.mode.embedded") : tt("kron.mode.standalone")}
          </Badge>
          <Badge tone={stan.ustawienia.wlaczona ? (stan.zrodlo_zywe ? "long" : "warn") : "short"} dot>
            {!stan.ustawienia.wlaczona
              ? tt("kron.state.off")
              : stan.zrodlo_zywe
                ? tt("kron.state.rec")
                : tt("kron.state.noSource")}
          </Badge>
        </div>
      </div>

      {/* ŹRÓDŁO — zawsze widoczne, bo „0 zdarzeń” bez przyczyny wygląda
          identycznie przy spokojnej nocy i przy martwej sesji MTProto.
          Sam opis źródła przychodzi z serwera i zostaje w jego brzmieniu. */}
      <div className={`kron__zrodlo ${stan.zrodlo_zywe ? "" : "kron__zrodlo--uwaga"}`}>
        <Icon name={stan.zrodlo_zywe ? "telegram" : "alert"} size={15} />
        <span>{stan.zrodlo_opis}</span>
        {stan.tryb === "wbudowana" && (
          <Tooltip content={tt("kron.oneConn.tip")}>
            <span className="kron__info">
              <Icon name="info" size={12} /> {tt("kron.oneConn")}
            </span>
          </Tooltip>
        )}
      </div>

      {blad && (
        <div className="kron__blad">
          <Icon name="alert" size={15} />
          <span>{tSilnik(blad)}</span>
        </div>
      )}
      {komunikat && (
        <div className="kron__ok">
          <Icon name="check" size={15} />
          <span>{komunikat}</span>
        </div>
      )}

      {/* ============================================================
          STATYSTYKA EDYCJI — POWÓD ISTNIENIA CAŁEGO NARZĘDZIA.
          Stoi jako pierwsza świadomie: to jest liczba, dla której
          ten rejestrator w ogóle powstał.
          ============================================================ */}
      <StatystykaEdycji stat={stat} />

      <Card
        title={tt("kron.file.title")}
        icon="logs"
        accent="var(--info)"
        subtitle={stan.istnieje ? bajty(stan.bajtow) : tt("kron.file.absent")}
        actions={
          <>
            <Button
              size="sm"
              variant="outline"
              icon="download"
              onClick={() => window.open(kronikaApi.plikUrl(), "_blank")}
              disabled={!stan.istnieje}
              title={tt("kron.file.downloadTip")}
            >
              {tt("kron.file.download")}
            </Button>
            <Tooltip content={tt("kron.file.exportTip")}>
              <Button size="sm" variant="primary" icon="flask" onClick={() => void eksportuj()} disabled={zajete}>
                {tt("kron.file.export")}
              </Button>
            </Tooltip>
          </>
        }
      >
        <div className="kron__plik">
          <code className="mono">{stan.plik}</code>
          <span className="hint">
            {stan.plikow > 1 ? tt("kron.file.rotatedN", { n: stan.plikow }) : tt("kron.file.single")}
          </span>
        </div>

        {}
        {stan.rozpoznanie && (
          <div className="kron__uwaga" style={{ marginTop: "var(--sp-2)" }}>
            <Icon
              name={stan.rozpoznanie.obcy_format ? "alert" : "info"}
              size={14}
              style={{ flex: "none" }}
            />
            <div>
              {stan.rozpoznanie.istnial && stan.rozpoznanie.bajtow > 0 ? (
                <RichT
                  k="kron.rec.continue"
                  vars={{
                    n: num(stan.rozpoznanie.wierszy),
                    data: stan.rozpoznanie.ostatni_opis || tt("kron.rec.unknownDate"),
                  }}
                />
              ) : (
                <RichT k="kron.rec.fresh" />
              )}
              {stan.rozpoznanie.obcy_format && (
                <>
                  <br />
                  <b style={{ color: "var(--warn-text)" }}>
                    {tt("kron.rec.newerFormat", { n: stan.rozpoznanie.schemat_ostatni })}
                  </b>{" "}
                  {tt("kron.rec.newerFormatTail")}
                </>
              )}
              {stan.rozpoznanie.nieczytelny && !stan.rozpoznanie.obcy_format && (
                <>
                  <br />
                  {tt("kron.rec.broken")}
                </>
              )}
              {stan.kopia && (
                <>
                  <br />
                  <span className="hint">
                    {tt("kron.rec.backup")} <code className="mono">{stan.kopia}</code>
                  </span>
                </>
              )}
            </div>
          </div>
        )}
        <div className="kron__licz">
          <Licznik k={tt("kron.c.saved")} v={compact(stan.liczniki.zapisanych)} />
          <Licznik k={tt("kron.c.new")} v={compact(stan.liczniki.nowych)} />
          <Licznik
            k={tt("kron.c.edits")}
            v={compact(stan.liczniki.edycji)}
            tone={stan.liczniki.edycji ? "up" : undefined}
          />
          <Licznik k={tt("kron.c.deleted")} v={compact(stan.liczniki.skasowanych)} />
          <Licznik
            k={tt("kron.c.skipped")}
            v={compact(stan.liczniki.pominietych)}
            tone={stan.liczniki.pominietych ? "down" : undefined}
            hint={tt("kron.c.skipped.hint")}
          />
          <Licznik
            k={tt("kron.c.errors")}
            v={compact(stan.liczniki.bledow)}
            tone={stan.liczniki.bledow ? "down" : undefined}
          />
          <Licznik k={tt("kron.c.last")} v={stan.liczniki.ostatnie_ms ? ago(stan.liczniki.ostatnie_ms) : "—"} />
          <Licznik k={tt("kron.c.size")} v={bajty(stan.bajtow)} />
        </div>
        {stan.liczniki.ostatni_blad && (
          <p className="hint" style={{ color: "var(--danger-text)", marginTop: "var(--sp-2)" }}>
            {tt("kron.lastError", { v: stan.liczniki.ostatni_blad })}
          </p>
        )}
        <p className="hint" style={{ marginTop: "var(--sp-2)" }}>
          <RichT k="kron.countersNote" />
        </p>
      </Card>

      <Segmented<"podglad" | "kanaly" | "opcje">
        value={karta}
        onChange={setKarta}
        options={[
          { value: "podglad", label: tt("kron.tab.preview") },
          {
            value: "kanaly",
            label: tt("kron.tab.channels", {
              v: u.zrodla.tryb === "wybrane" ? u.zrodla.lista.length : tt("kron.tab.all"),
            }),
          },
          { value: "opcje", label: tt("kron.tab.options") },
        ]}
      />

      {karta === "podglad" && <Podglad stan={stan} />}
      {karta === "kanaly" && (
        <Kanaly
          kanaly={kanaly}
          blad={bladKanalow}
          ustawienia={u}
          onZmien={(nowe) => void zapisz(nowe)}
          onOdswiez={() => void odswiezKanaly()}
          zajete={zajete}
        />
      )}
      {karta === "opcje" && (
        <Opcje
          wartosc={u}
          brudne={brudne}
          zajete={zajete}
          domyslnaSciezka={stan.domyslny_plik ?? "logs/kronika.jsonl"}
          onZmien={setRobocze}
          onZapisz={() => void zapisz(u)}
          onCofnij={() => setRobocze(stan.ustawienia)}
        />
      )}

      {stat && stat.przerwy.length > 0 && <Przerwy stat={stat} />}
    </div>
  );
}

/* ------------------------------------------------------------
   STATYSTYKA EDYCJI
   ------------------------------------------------------------ */

function StatystykaEdycji({ stat }: { stat: KronikaStatystyki | null }) {
  const tt = useT();

  if (!stat || stat.wiadomosci === 0) {
    return (
      <Card title={tt("kron.stat.title")} icon="edit" accent="var(--accent)">
        <Empty icon="hourglass" title={tt("kron.stat.emptyTitle")} text={tt("kron.stat.emptyText")} />
      </Card>
    );
  }

  const p = stat.procent_edytowanych;
  return (
    <Card
      title={tt("kron.stat.title")}
      icon="edit"
      accent="var(--accent)"
      subtitle={tt("kron.stat.subtitle", { n: compact(stat.wiadomosci), z: stat.zrodel })}
    >
      <div className="kron__glowna">
        <div className="kron__wielka">
          <span className="kron__wielka__v num">{num(p, 1)}%</span>
          <span className="kron__wielka__k">{tt("kron.stat.bigLabel")}</span>
          <span className="kron__wielka__d">
            {tt("kron.stat.bigOf", { a: compact(stat.edytowanych), b: compact(stat.wiadomosci) })}
          </span>
        </div>

        <div className="kron__pary">
          <ParaCzasu
            tytul={tt("kron.stat.first")}
            r={stat.do_pierwszej}
            akcent
            opis={tt("kron.stat.firstTip")}
          />
          <ParaCzasu tytul={tt("kron.stat.last")} r={stat.do_ostatniej} opis={tt("kron.stat.lastTip")} />
        </div>
      </div>

      <div className="kron__licz kron__licz--drobne">
        <Licznik k={tt("kron.stat.editsTotal")} v={compact(stat.edycji)} />
        <Licznik
          k={tt("kron.stat.multi")}
          v={compact(stat.wielokrotnie_edytowanych)}
          hint={tt("kron.stat.multiHint")}
        />
        <Licznik k={tt("kron.stat.maxOne")} v={String(stat.max_edycji_jednej)} />
        <Licznik k={tt("kron.stat.deleted")} v={compact(stat.skasowanych)} hint={tt("kron.stat.deletedHint")} />
        <Licznik k={tt("kron.stat.rows")} v={compact(stat.wpisow)} />
        <Licznik k={tt("kron.stat.noOrig")} v={compact(stat.bez_oryginalu)} hint={tt("kron.stat.noOrigHint")} />
        <Licznik
          k={tt("kron.stat.broken")}
          v={compact(stat.uszkodzonych)}
          tone={stat.uszkodzonych ? "down" : undefined}
        />
      </div>

      {}
      {stat.bez_oryginalu > 0 && (
        <p className="hint" style={{ marginTop: "var(--sp-2)" }}>
          <RichT
            k="kron.stat.denominator"
            vars={{ a: compact(stat.wiadomosci), b: compact(stat.bez_oryginalu) }}
          />
        </p>
      )}

      {/* ZŁAMANA PRZYCZYNOWOŚĆ — odkrycie z kanału ZEN. */}
      {stat.edycji_po_odpowiedzi > 0 && (
        <div className="kron__alarm">
          <Icon name="alert" size={16} />
          <div>
            <b>
              {stat.edycji_po_odpowiedzi === 1
                ? tt("kron.stat.causality.one", { n: compact(stat.edycji_po_odpowiedzi) })
                : tt("kron.stat.causality.many", { n: compact(stat.edycji_po_odpowiedzi) })}
            </b>
            <span>
              <RichT
                k="kron.stat.causality.text"
                vars={{ p50: sekundy(stat.po_odpowiedzi.p50_s), max: sekundy(stat.po_odpowiedzi.max_s) }}
              />
            </span>
          </div>
        </div>
      )}

      {stat.wg_zrodla.length > 0 && (
        <div className="kron__tabela">
          <table>
            <thead>
              <tr>
                <th>{tt("kron.tbl.source")}</th>
                <th className="num">{tt("kron.tbl.messages")}</th>
                <th className="num">{tt("kron.tbl.edited")}</th>
                <th className="num">%</th>
                <th className="num">{tt("kron.tbl.p50first")}</th>
                <th className="num">{tt("kron.tbl.deleted")}</th>
              </tr>
            </thead>
            <tbody>
              {stat.wg_zrodla.slice(0, 25).map((z) => (
                <tr key={`${z.chat_id}:${z.temat ?? ""}`}>
                  <td>
                    <span className="truncate">{z.chat || z.chat_id}</span>
                    {z.temat != null && <Badge tone="muted">{tt("kron.topic", { n: z.temat })}</Badge>}
                    {z.nasluchiwany && (
                      <Tooltip content={tt("kron.tradesTip")}>
                        <Badge tone="accent">{tt("kron.trades")}</Badge>
                      </Tooltip>
                    )}
                  </td>
                  <td className="num">{compact(z.wiadomosci)}</td>
                  <td className="num">{compact(z.edytowanych)}</td>
                  <td className="num">
                    <b className={z.procent_edytowanych >= 50 ? "up" : undefined}>
                      {num(z.procent_edytowanych, 1)}%
                    </b>
                  </td>
                  <td className="num">{z.p50_pierwsza_s > 0 ? sekundy(z.p50_pierwsza_s) : "—"}</td>
                  <td className="num">{compact(z.skasowanych)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Card>
  );
}

function ParaCzasu({
  tytul,
  r,
  opis,
  akcent,
}: {
  tytul: string;
  r: KronikaRozklad;
  opis: string;
  akcent?: boolean;
}) {
  const tt = useT();
  return (
    <div className={`kron__para ${akcent ? "kron__para--akcent" : ""}`}>
      <div className="kron__para__h">
        <span>{tytul}</span>
        <Tooltip content={opis}>
          <span className="kron__info">
            <Icon name="info" size={12} />
          </span>
        </Tooltip>
      </div>
      {r.n === 0 ? (
        <span className="hint">{tt("kron.para.noEdits")}</span>
      ) : (
        <>
          <div className="kron__para__v num">{sekundy(r.p50_s)}</div>
          <div className="kron__para__r">
            {/* Mediana i średnia obok siebie CELOWO: rozkład ma długi ogon,
                a różnica między nimi jest jedyną widoczną miarą tego ogona.
                „p90" i „max" to symbole, nie napisy — bez słownika. */}
            <span>
              p90 <b className="num">{sekundy(r.p90_s)}</b>
            </span>
            <span>
              {tt("kron.para.avg")} <b className="num">{sekundy(r.srednia_s)}</b>
            </span>
            <span>
              max <b className="num">{sekundy(r.max_s)}</b>
            </span>
          </div>
        </>
      )}
    </div>
  );
}

function Licznik({
  k,
  v,
  tone,
  hint,
}: {
  k: string;
  v: string;
  tone?: "up" | "down";
  hint?: string;
}) {
  const tresc = (
    <div className="kron__licznik">
      <span className="kron__licznik__k">{k}</span>
      <span className={`kron__licznik__v num ${tone ?? ""}`}>{v}</span>
    </div>
  );
  return hint ? <Tooltip content={hint}>{tresc}</Tooltip> : tresc;
}



function Podglad({ stan }: { stan: KronikaStan }) {
  const tt = useT();
  const [filtr, setFiltr] = useState<"wszystko" | "nowe" | "edycje">("wszystko");
  const lista = stan.ostatnie.filter((w) => {
    if (filtr === "nowe") return w.rodzaj === "nowa";
    if (filtr === "edycje") return w.rodzaj === "edycja";
    return true;
  });

  return (
    <Card
      title={tt("kron.tab.preview")}
      icon="activity"
      accent="var(--accent)"
      subtitle={tt("kron.prev.subtitle", { n: stan.ostatnie.length })}
      actions={
        <Segmented<"wszystko" | "nowe" | "edycje">
          size="sm"
          value={filtr}
          onChange={setFiltr}
          options={[
            { value: "wszystko", label: tt("kron.prev.all") },
            { value: "nowe", label: tt("kron.prev.new", { n: stan.liczniki.nowych }) },
            { value: "edycje", label: tt("kron.prev.edits", { n: stan.liczniki.edycji }) },
          ]}
        />
      }
    >
      {lista.length === 0 ? (
        <Empty
          icon="hourglass"
          title={tt("kron.prev.emptyTitle")}
          /* przy martwym źródle powód podaje serwer — w jego brzmieniu */
          text={stan.zrodlo_zywe ? tt("kron.prev.emptyText") : stan.zrodlo_opis}
        />
      ) : (
        <div className="kron__strumien">
          {lista.map((w) => (
            <Wiersz key={`${w.chat_id}:${w.msg_id}:${w.seq}`} w={w} />
          ))}
        </div>
      )}
    </Card>
  );
}

function Wiersz({ w }: { w: KronikaWpis }) {
  const tt = useT();
  const znacznik = w.rodzaj === "start" || w.rodzaj === "stop";
  if (znacznik) {
    return (
      <div className="kron__wiersz kron__wiersz--sesja">
        <span className="kron__czas mono">{czasKrotki(w.odebrano_ms)} UTC</span>
        <Badge tone="muted">{w.rodzaj === "start" ? tt("kron.row.start") : tt("kron.row.stop")}</Badge>
        <span className="hint truncate">{w.uwaga}</span>
      </div>
    );
  }

  /* Opóźnienie: ile minęło między znacznikiem Telegrama a chwilą, w której
     MY to zobaczyliśmy. Przy edycji potrafi być ujemne — Telegram podaje
     wtedy znacznik oryginału — i to jest właśnie ta różnica, o którą chodzi. */
  const opoznienie = w.ts_telegram_ms ? (w.odebrano_ms - w.ts_telegram_ms) / 1000 : null;

  return (
    <div className="kron__wiersz" data-rodzaj={w.rodzaj}>
      <span className="kron__czas mono">{czasKrotki(w.odebrano_ms)} UTC</span>
      <span className="kron__znak">
        <Badge tone={w.rodzaj === "edycja" ? "warn" : w.rodzaj === "skasowana" ? "short" : "long"}>
          {w.rodzaj === "edycja"
            ? tt("kron.row.edit")
            : w.rodzaj === "skasowana"
              ? tt("kron.row.deleted")
              : tt("kron.row.new")}
        </Badge>
      </span>
      <div className="kron__tresc">
        <div className="kron__meta">
          <b className="truncate">{w.chat || w.chat_id}</b>
          {w.temat != null && <Badge tone="muted">{tt("kron.topic", { n: w.temat })}</Badge>}
          {w.format && <Badge tone="info">{w.format}</Badge>}
          {w.nasluchiwany ? (
            <Tooltip content={tt("kron.row.tradesTip")}>
              <Badge tone="accent">{tt("kron.trades")}</Badge>
            </Tooltip>
          ) : (
            <Tooltip content={tt("kron.row.recOnlyTip")}>
              <Badge tone="muted">{tt("kron.row.recOnly")}</Badge>
            </Tooltip>
          )}
          {w.rozpoznane ? (
            <Badge tone="long">{tt("kron.row.parsed")}</Badge>
          ) : (
            <Badge tone="muted">{tt("kron.row.unparsed")}</Badge>
          )}
          {w.reply_to != null && <span className="hint">{tt("kron.row.replyTo", { n: w.reply_to })}</span>}
          {opoznienie != null && Math.abs(opoznienie) >= 1 && (
            <Tooltip content={opoznienie < 0 ? tt("kron.row.lagNeg") : tt("kron.row.lagPos")}>
              <span className="hint mono">
                {opoznienie > 0 ? "+" : "−"}
                {sekundy(Math.abs(opoznienie))}
              </span>
            </Tooltip>
          )}
        </div>
        {w.text ? (
          <pre className="kron__text">{w.text}</pre>
        ) : (
          <span className="hint">{tt("kron.row.noText")}</span>
        )}
      </div>
    </div>
  );
}

/* ------------------------------------------------------------
   WYBÓR KANAŁÓW
   ------------------------------------------------------------ */

function Kanaly({
  kanaly,
  blad,
  ustawienia,
  onZmien,
  onOdswiez,
  zajete,
}: {
  kanaly: KronikaKanal[];
  blad: string | null;
  ustawienia: KronikaUstawienia;
  onZmien: (u: KronikaUstawienia) => void;
  onOdswiez: () => void;
  zajete: boolean;
}) {
  const tt = useT();
  const [q, setQ] = useState("");
  const wszystkie = ustawienia.zrodla.tryb === "wszystkie";
  const lista = ustawienia.zrodla.tryb === "wybrane" ? ustawienia.zrodla.lista : [];

  const nagrywa = (chat_id: number, temat?: number | null) =>
    wszystkie ||
    lista.some((z) => z.chat_id === chat_id && (z.temat == null || z.temat === temat));

  const przelacz = (chat_id: number, temat: number | null, wl: boolean) => {
    if (wszystkie) return;
    let nowa = lista.filter((z) => !(z.chat_id === chat_id && (z.temat ?? null) === temat));
    if (wl) nowa = [...nowa, temat == null ? { chat_id } : { chat_id, temat }];
    onZmien({ ...ustawienia, zrodla: { tryb: "wybrane", lista: nowa } });
  };

  const widoczne = kanaly.filter(
    (k) =>
      k.nazwa.toLowerCase().includes(q.toLowerCase()) ||
      (k.handle ?? "").toLowerCase().includes(q.toLowerCase()) ||
      String(k.chat_id).includes(q),
  );

  return (
    <Card
      title={tt("kron.ch.title")}
      icon="channels"
      accent="var(--info)"
      subtitle={wszystkie ? tt("kron.tab.all") : tt("kron.ch.selectedN", { n: lista.length })}
      actions={
        <>
          <TextInput value={q} onChange={setQ} placeholder={tt("kron.ch.search")} icon="search" size="sm" style={{ width: 200 }} />
          <Button size="sm" variant="ghost" icon="refresh" onClick={onOdswiez} disabled={zajete}>
            {tt("kron.ch.refresh")}
          </Button>
        </>
      }
    >
      <Segmented<"wszystkie" | "wybrane">
        value={wszystkie ? "wszystkie" : "wybrane"}
        onChange={(v) =>
          onZmien({
            ...ustawienia,
            zrodla:
              v === "wszystkie"
                ? { tryb: "wszystkie" }
                : {
                    tryb: "wybrane",
                    // Przejście na wybór zaczyna od kanałów, z których COŚ już
                    // mamy — pusta lista znaczyłaby „nie nagrywaj niczego”,
                    // czyli cichą utratę wszystkiego od tej sekundy.
                    lista: kanaly.filter((k) => k.wpisow > 0).map((k) => ({ chat_id: k.chat_id })),
                  },
          })
        }
        options={[
          { value: "wszystkie", label: tt("kron.ch.modeAll") },
          { value: "wybrane", label: tt("kron.ch.modeSel") },
        ]}
      />

      <p className="hint" style={{ margin: "var(--sp-3) 0" }}>
        {wszystkie ? <RichT k="kron.ch.leadAll" /> : <RichT k="kron.ch.leadSel" />}
      </p>

      {blad && (
        <p className="hint" style={{ color: "var(--warn-text)" }}>
          {tt("kron.ch.partial", { v: blad })}
        </p>
      )}

      {widoczne.length === 0 ? (
        <Empty icon="channels" title={tt("kron.ch.emptyTitle")} text={tt("kron.ch.emptyText")} />
      ) : (
        <div className="kron__kanaly">
          {widoczne.map((k) => (
            <div key={k.chat_id} className="kron__kanal" data-on={nagrywa(k.chat_id, null)}>
              <div className="kron__kanal__h">
                <Checkbox
                  checked={nagrywa(k.chat_id, null)}
                  disabled={wszystkie}
                  onChange={(v) => przelacz(k.chat_id, null, v)}
                  label={
                    <span className="kron__kanal__n">
                      <b className="truncate">{k.nazwa}</b>
                      {/* „forum" to nazwa własna funkcji Telegrama — bez słownika */}
                      {k.forum && <Badge tone="muted">forum</Badge>}
                      {k.nasluchiwany && (
                        <Tooltip content={tt("kron.ch.tradesTip")}>
                          <Badge tone="accent">{tt("kron.trades")}</Badge>
                        </Tooltip>
                      )}
                    </span>
                  }
                />
                <span className="spacer" />
                <span className="hint mono">
                  {k.wpisow > 0 ? tt("kron.ch.inFile", { n: compact(k.wpisow) }) : "—"}
                </span>
              </div>
              {k.forum && k.tematy.length > 0 && (
                <div className="kron__tematy">
                  {k.tematy.map((temat) => (
                    <Checkbox
                      key={temat.id}
                      checked={nagrywa(k.chat_id, temat.id)}
                      disabled={wszystkie || nagrywa(k.chat_id, null)}
                      onChange={(v) => przelacz(k.chat_id, temat.id, v)}
                      label={
                        <span className="hint">
                          {temat.nazwa}
                          {temat.wpisow > 0 ? ` · ${compact(temat.wpisow)}` : ""}
                        </span>
                      }
                    />
                  ))}
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}

/* ------------------------------------------------------------
   OPCJE ZAPISU
   ------------------------------------------------------------ */

function Opcje({
  wartosc,
  brudne,
  zajete,
  domyslnaSciezka,
  onZmien,
  onZapisz,
  onCofnij,
}: {
  wartosc: KronikaUstawienia;
  brudne: boolean;
  zajete: boolean;
  /** ścieżka, którą program wybrałby sam — do przycisku „Domyślna" */
  domyslnaSciezka: string;
  onZmien: (u: KronikaUstawienia) => void;
  onZapisz: () => void;
  onCofnij: () => void;
}) {
  const tt = useT();
  const u = wartosc;
  const [wybor, setWybor] = useState<"katalog" | "plik" | null>(null);
  const set = <K extends keyof KronikaUstawienia>(k: K, v: KronikaUstawienia[K]) =>
    onZmien({ ...u, [k]: v });

  /** Katalog, od którego otworzyć przeglądarkę — ten z bieżącej ścieżki. */
  const katalogStartowy = (() => {
    const i = Math.max(u.plik.lastIndexOf("\\"), u.plik.lastIndexOf("/"));
    return i > 0 ? u.plik.slice(0, i) : undefined;
  })();

  /** Nazwa pliku z bieżącej ścieżki — doklejana po wyborze KATALOGU. */
  const nazwaPliku = (() => {
    const i = Math.max(u.plik.lastIndexOf("\\"), u.plik.lastIndexOf("/"));
    const n = i >= 0 ? u.plik.slice(i + 1) : u.plik;
    return n.trim() || "conduit_kronika.jsonl";
  })();

  return (
    <Card
      title={tt("kron.tab.options")}
      icon="sliders"
      accent="var(--warn)"
      footer={
        <div className="row">
          <span className="hint">{brudne ? tt("kron.opt.dirty") : tt("kron.opt.clean")}</span>
          <span className="spacer" />
          <Button variant="ghost" size="sm" onClick={onCofnij} disabled={!brudne || zajete}>
            {tt("kron.opt.revert")}
          </Button>
          <Button variant="primary" size="sm" icon="check" onClick={onZapisz} disabled={!brudne || zajete}>
            {tt("kron.opt.save")}
          </Button>
        </div>
      }
    >
      {wybor && (
        <WyborSciezki
          tryb={wybor}
          tytul={wybor === "plik" ? tt("kron.opt.pickFile") : tt("kron.opt.pickDir")}
          rozszerzenia="jsonl"
          start={katalogStartowy}
          onWybierz={(p) => {
            // Po wyborze KATALOGU dokleja się dotychczasowa nazwa pliku —
            // inaczej ścieżka zostawałaby katalogiem i zapis by nie ruszył.
            set("plik", wybor === "plik" ? p : `${p.replace(/[\\/]+$/, "")}\\${nazwaPliku}`);
            setWybor(null);
          }}
          onZamknij={() => setWybor(null)}
        />
      )}

      <div className="setgrid">
        <div className="setfield setfield--wide">
          <Field label={tt("kron.opt.on")} hint={tt("kron.opt.onHint")}>
            <Switch checked={u.wlaczona} onChange={(v) => set("wlaczona", v)} />
          </Field>
        </div>

        {/* ŚCIEŻKA PLIKU — najważniejsze pole całej karty.
            Plik ma ŻYĆ POZA katalogiem bota, żeby wgranie kolejnej paczki
            (VPSREADY, VPSREADY2…) nie zabierało ciągłości zapisu. Stąd
            domyślna na pulpicie i stąd te trzy przyciski: przeglądarka
            katalogów SERWERA (panel bywa otwarty z innej maszyny, więc
            natywne okno Windows pokazałoby cudze dyski), wskazanie
            istniejącego pliku i powrót do domyślnej. */}
        <div className="setfield setfield--wide">
          <Field label={tt("kron.opt.file")} hint={tt("kron.opt.fileHint")}>
            <div className="row row--tight" style={{ flexWrap: "wrap" }}>
              <div style={{ flex: 1, minWidth: 240 }}>
                <TextInput
                  value={u.plik}
                  onChange={(v) => set("plik", v)}
                  placeholder={tt("kron.opt.filePh")}
                />
              </div>
              <Button size="sm" variant="outline" icon="grid" onClick={() => setWybor("katalog")}>
                {tt("kron.opt.browse")}
              </Button>
              <Button size="sm" variant="outline" icon="logs" onClick={() => setWybor("plik")}>
                {tt("kron.opt.pickExisting")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                icon="refresh"
                onClick={() => set("plik", domyslnaSciezka)}
                title={tt("kron.opt.defaultTip", { v: domyslnaSciezka })}
              >
                {tt("kron.opt.default")}
              </Button>
            </div>
          </Field>
        </div>

        <div className="setfield setfield--wide">
          <Field label={tt("kron.opt.unparsed")} hint={tt("kron.opt.unparsedHint")}>
            <Switch checked={u.nierozpoznane} onChange={(v) => set("nierozpoznane", v)} />
          </Field>
        </div>

        <div className="setfield setfield--wide">
          <Field label={tt("kron.opt.empty")} hint={tt("kron.opt.emptyHint")}>
            <Switch checked={u.puste} onChange={(v) => set("puste", v)} />
          </Field>
        </div>

        {/* FSYNC — z ceną, nie tylko z nazwą. */}
        <div className="setfield setfield--wide">
          <Field
            label={tt("kron.opt.fsync")}
            warn={
              u.fsync.tryb === "nigdy"
                ? tt("kron.opt.fsync.never")
                : u.fsync.tryb === "co"
                  ? tt("kron.opt.fsync.co", { n: u.fsync.n })
                  : null
            }
            hint={tt("kron.opt.fsyncHint")}
          >
            <div className="row row--tight">
              <Segmented<"kazda" | "co" | "nigdy">
                value={u.fsync.tryb}
                onChange={(v) =>
                  set("fsync", v === "co" ? { tryb: "co", n: 20 } : { tryb: v })
                }
                options={[
                  { value: "kazda", label: tt("kron.opt.fsync.each") },
                  { value: "co", label: tt("kron.opt.fsync.everyN") },
                  { value: "nigdy", label: tt("kron.opt.fsync.no") },
                ]}
              />
              {u.fsync.tryb === "co" && (
                <NumberInput
                  value={u.fsync.n}
                  onChange={(n) => set("fsync", { tryb: "co", n: Math.max(1, Math.round(n)) })}
                  min={1}
                  step={10}
                  unit={tt("kron.opt.fsync.unit")}
                  size="sm"
                  style={{ width: 130 }}
                />
              )}
            </div>
          </Field>
        </div>

        <div className="setfield">
          <Field label={tt("kron.opt.rotate")} hint={tt("kron.opt.rotateHint")}>
            <NumberInput
              value={u.obrot_mb}
              onChange={(v) => set("obrot_mb", Math.max(0, Math.round(v)))}
              min={0}
              step={50}
              unit="MB"
              zeroLabel={tt("kron.opt.rotateZero")}
            />
          </Field>
        </div>

        <div className="setfield">
          <Field label={tt("kron.opt.keep")} hint={tt("kron.opt.keepHint")}>
            <NumberInput
              value={u.trzymaj_plikow}
              onChange={(v) => set("trzymaj_plikow", Math.max(0, Math.round(v)))}
              min={0}
              step={1}
              zeroLabel={tt("kron.opt.keepZero")}
            />
          </Field>
        </div>

        <div className="setfield">
          <Field label={tt("kron.opt.marks")} hint={tt("kron.opt.marksHint")}>
            <Switch checked={u.znaczniki_sesji} onChange={(v) => set("znaczniki_sesji", v)} />
          </Field>
        </div>

        <div className="setfield">
          <Field label={tt("kron.opt.tz")} hint={tt("kron.opt.tzHint")}>
            <NumberInput value={u.strefa_h} onChange={(v) => set("strefa_h", v)} step={1} min={-12} max={14} unit="h" />
          </Field>
        </div>
      </div>

      {/* Uczciwie o tym, czego NIE MA. */}
      <div className="kron__uwaga">
        <Icon name="info" size={14} />
        <div>
          <RichT k="kron.opt.media" />
        </div>
      </div>
    </Card>
  );
}

/* ------------------------------------------------------------
   PRZERWY W NAGRYWANIU
   ------------------------------------------------------------ */

function Przerwy({ stat }: { stat: KronikaStatystyki }) {
  const tt = useT();
  const nagle = stat.przerwy.filter((p) => p.nagle).length;
  return (
    <Card
      title={tt("kron.gap.title")}
      icon="hourglass"
      accent="var(--warn)"
      subtitle={tt("kron.gap.subtitle", { n: stat.przerwy.length, t: sekundy(stat.przerw_sekund) })}
    >
      <p className="hint" style={{ marginBottom: "var(--sp-3)" }}>
        <RichT k="kron.gap.lead" />
        {nagle > 0 && (
          <>
            {" "}
            {nagle === 1 ? <RichT k="kron.gap.abruptOne" vars={{ n: nagle }} /> : <RichT k="kron.gap.abruptMany" vars={{ n: nagle }} />}
          </>
        )}
      </p>
      <div className="kron__tabela">
        <table>
          <thead>
            <tr>
              <th>{tt("kron.gap.from")}</th>
              <th>{tt("kron.gap.to")}</th>
              <th className="num">{tt("kron.gap.len")}</th>
              <th>{tt("kron.gap.how")}</th>
            </tr>
          </thead>
          <tbody>
            {stat.przerwy.slice(-20).reverse().map((p) => (
              <tr key={`${p.od_ms}-${p.do_ms}`}>
                {/* format daty bierze locale ze słownika (`kron.locale`) —
                    sztywne „pl-PL" pokazywało polską datę w angielskim panelu */}
                <td className="mono">{new Date(p.od_ms).toLocaleString(tt("kron.locale"), { timeZone: "UTC", hourCycle: "h23" }) + " UTC"}</td>
                <td className="mono">{new Date(p.do_ms).toLocaleString(tt("kron.locale"), { timeZone: "UTC", hourCycle: "h23" }) + " UTC"}</td>
                <td className="num">{sekundy(p.sekund)}</td>
                <td>
                  {p.nagle ? (
                    <Badge tone="warn" dot>
                      {tt("kron.gap.abrupt")}
                    </Badge>
                  ) : (
                    <Badge tone="muted">{tt("kron.gap.clean")}</Badge>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Card>
  );
}
