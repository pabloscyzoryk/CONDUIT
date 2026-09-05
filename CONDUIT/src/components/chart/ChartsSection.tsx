import { tSilnik } from "@/i18n/silnik";
import { useEffect, useMemo, useState } from "react";
import { Badge, Button, Card, Icon, TextInput } from "@/components/ui";
import { TradingChart } from "./TradingChart";
import { useApp } from "@/store/AppStore";
import { api } from "@/store/transport";
import { searchSymbols, SYMBOLS } from "@/data/symbols";
import { num, pct, toneOf } from "@/lib/format";
import { useT, RichT, useLanguage } from "@/i18n";

/**
 * Sekcja wykresow — odpowiednik karty "Wykresy" z bot.py:
 * wyszukiwarka instrumentow, ulubione, wiele paneli, kolejnosc,
 * pelny ekran, rysowanie, eksport.
 */
export function ChartsSection() {
  const app = useApp();
  const t = useT();
  const { lang } = useLanguage();
  // One runtime instrument shared with the ticker, health indicator and ticket.
  // AUTO preferences deliberately contain no symbol; a watchlist is not a bind.
  const botSymbol = app.primary.symbol;
  const [panels, setPanels] = useState<string[]>(() => {
    const favs = app.favorites.filter((f) => f !== botSymbol);
    return [botSymbol, ...favs.slice(0, 1)].filter(Boolean);
  });
  /* Zmiana symbolu runtime w trakcie pracy (np. przełączenie brokera) PRZEPINA
     kafel bota: stary symbol bota znika z listy (chyba że jest ulubiony),
     nowy wchodzi na pierwsze miejsce. Pozostałe panele nietknięte. */
  useEffect(() => {
    setPanels((p) => {
      if (p[0] === botSymbol) return p;
      const bez = p.filter((x) => x !== botSymbol && (x !== p[0] || app.favorites.includes(x)));
      return [botSymbol, ...bez].filter(Boolean);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [botSymbol]);
  const [q, setQ] = useState("");
  const [open, setOpen] = useState(false);

  
  const [zMostu, setZMostu] = useState<{ name: string; visible: boolean }[]>([]);
  useEffect(() => {
    if (!open) return;
    let aktualne = true;
    api
      .symbols(q)
      .then((r) => {
        if (aktualne) setZMostu(r);
      })
      // 503 = most padnięty — zostaje lista statyczna + symbol bota
      .catch(() => setZMostu([]));
    return () => {
      aktualne = false;
    };
  }, [q, open]);
  const results = useMemo(() => {
    const statyczne = [...searchSymbols(q), ...SYMBOLS.filter(s => tSilnik(s.name).toLowerCase().includes(q.trim().toLowerCase()))].map(s => s.symbol);
    const mostowe = zMostu.map((s) => s.name);
    const zestaw = [...new Set([...mostowe, ...statyczne])];
    const ql = q.trim().toLowerCase();
    if (botSymbol && botSymbol.toLowerCase().includes(ql) && !zestaw.includes(botSymbol)) {
      zestaw.unshift(botSymbol);
    }
    return zestaw.slice(0, 20);
  }, [q, zMostu, botSymbol, lang]);

  const add = (s: string) => {
    setPanels((p) => (p.includes(s) ? p : [...p, s]));
    setQ("");
    setOpen(false);
  };

  const move = (s: string, d: -1 | 1) => {
    setPanels((p) => {
      const i = p.indexOf(s);
      const j = i + d;
      if (i < 0 || j < 0 || j >= p.length) return p;
      const out = [...p];
      [out[i], out[j]] = [out[j], out[i]];
      return out;
    });
  };

  return (
    <div className="charts">
      <Card
        title={t("charts.title")}
        icon="chart"
        subtitle={t("charts.instruments", { n: panels.length })}
        accent="var(--accent)"
        tight
        actions={
          <div className="symsearch">
            <TextInput
              value={q}
              onChange={(v) => {
                setQ(v);
                setOpen(true);
              }}
              placeholder={t("charts.search")}
              icon="search"
              size="sm"
              style={{ width: 260 }}
            />
            {open && q && (
              <>
                <div className="symsearch__scrim" onClick={() => setOpen(false)} />
                <div className="symsearch__list">
                  {results.length === 0 && (
                    <div className="hint" style={{ padding: 10 }}>
                      {t("charts.noResults")}
                    </div>
                  )}
                  {results.map((sym) => {
                    const qt = app.quotes[sym];
                    const meta = SYMBOLS.find((x) => x.symbol === sym);
                    return (
                      <button key={sym} className="symsearch__item" onClick={() => add(sym)}>
                        <span
                          className="symsearch__fav"
                          onClick={(e) => {
                            e.stopPropagation();
                            app.toggleFavorite(sym);
                          }}
                          title={t("charts.favorite")}
                        >
                          <Icon name={app.favorites.includes(sym) ? "star-filled" : "star"} size={12} />
                        </span>
                        <b>{sym}</b>
                        <span className="truncate hint">{meta ? tSilnik(meta.name) : (sym === botSymbol ? t("charts.botSymbol") : t("charts.bridge"))}</span>
                        <Badge tone="muted">{meta?.group ?? "broker"}</Badge>
                        {qt && (
                          <span className={`num ${toneOf(qt.change)}`} style={{ fontSize: "var(--fs-2xs)" }}>
                            {pct(qt.changePct, 2)}
                          </span>
                        )}
                      </button>
                    );
                  })}
                </div>
              </>
            )}
          </div>
        }
      >
        {}
        <div className="watchlist">
          {[...new Set([...panels, ...app.favorites])].slice(0, 8).map((sym) => {
            const qt = app.quotes[sym];
            if (!qt) return null;
            const ton = toneOf(qt.change);
            return (
              <button key={sym} className="watch" data-active={panels.includes(sym)} onClick={() => add(sym)}>
                <span className="watch__sym">{sym}</span>
                <span className="watch__px num">{num(qt.bid, SYMBOLS.find((x) => x.symbol === sym)?.digits ?? 2)}</span>
                <span className={`watch__chg num ${ton}`}>{pct(qt.changePct, 2)}</span>
              </button>
            );
          })}
        </div>
      </Card>

      {panels.map((sym, i) => (
        <Card
          key={sym}
          title={
            <span className="row row--tight">
              <b>{sym}</b>
              {app.quotes[sym] && (
                <>
                  <span className="num">{num(app.quotes[sym].bid, 2)}</span>
                  <span className={`num ${toneOf(app.quotes[sym].change)}`} style={{ fontSize: "var(--fs-xs)" }}>
                    {pct(app.quotes[sym].changePct, 2)}
                  </span>
                </>
              )}
              {sym === botSymbol && <Badge tone="accent">{t("charts.botSymbol")}</Badge>}
            </span>
          }
          icon="chart"
          tight
          flush
          actions={
            <>
              <Button
                size="sm"
                variant="ghost"
                icon={app.favorites.includes(sym) ? "star-filled" : "star"}
                title={t("charts.favorite")}
                onClick={() => app.toggleFavorite(sym)}
              />
              <Button size="sm" variant="ghost" icon="arrow-up" title={t("charts.up")} onClick={() => move(sym, -1)} disabled={i === 0} />
              <Button
                size="sm"
                variant="ghost"
                icon="arrow-down"
                title={t("charts.down")}
                onClick={() => move(sym, 1)}
                disabled={i === panels.length - 1}
              />
              {sym !== botSymbol && (
                <Button size="sm" variant="ghost" icon="x" title={t("charts.close")} onClick={() => setPanels((p) => p.filter((x) => x !== sym))} />
              )}
            </>
          }
        >
          {/* Na wykresie rysujemy to, co naprawdę stoi NA TYM instrumencie —
              także pozycje spoza bota. Filtr po symbolu jest konieczny, odkąd
              panel widzi cały rachunek: pozycja na innym instrumencie miałaby
              cenę zupełnie poza skalą tego wykresu. */}
          <TradingChart
            symbol={sym}
            positions={app.snapshot.positions.filter((p) => p.symbol === sym)}
            pendings={app.snapshot.pendings.filter((o) => o.symbol === sym)}
            livePrice={app.quotes[sym]?.bid ?? 0}
            showPositions={app.settings.show_positions_on_chart}
            showPotential={app.settings.show_potential_tpsl}
            height={i === 0 ? 420 : 300}
            compactToolbar={i > 0}
          />
        </Card>
      ))}

      <details className="context-help">
        <summary>{t("charts.help.title")} · {app.settings.one_click ? t("charts.help.oneClick") : t("charts.help.confirm")}</summary>
        <p className="hint"><RichT
          k="charts.help"
          vars={{ mode: app.settings.one_click ? t("charts.help.oneClick") : t("charts.help.confirm") }}
        /></p>
      </details>
    </div>
  );
}
