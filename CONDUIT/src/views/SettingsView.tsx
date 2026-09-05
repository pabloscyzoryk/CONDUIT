import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
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
  Select,
  Switch,
  TextInput,
  Tooltip,
  type IconName,
} from "@/components/ui";
import { useApp } from "@/store/AppStore";
import { LANGUAGES, RichT, useLanguage, useT } from "@/i18n";
import { opisPresetu } from "@/i18n/silnik";
import { przetlumaczGrupy } from "@/i18n/schema";
import { PALETTES, useTheme } from "@/store/useTheme";
import {
  AI_GROUP,
  BRAK_LIMITU_RYZYKOWNY,
  COVERED_KEYS,
  GENERAL_GROUPS,
  MANAGEMENT_GROUPS,
  ZNACZENIE_ZERA,
  type FieldDef,
  type GroupDef,
  type ZakresUstawien,
  type ZnaczenieZera,
} from "@/data/settingsSchema";
import { DEFAULT_SETTINGS } from "@/data/defaultSettings";
import { AI_MODELS } from "@/data/telegram";
import { OcenaPresetu } from "@/components/panels/OcenaPresetu";
import { WartoscNog } from "@/components/panels/WartoscNog";
import { powiazPoleUstawien, warstwaInnaNizGrupa, zakresPola } from "@/store/warstwaPola";
import { effectiveSmartSlDelay, isAdvancedTpSource, settingControlPatch, settingControlValue, type SettingControlPatch } from "@/store/settingAliases";
import { czyCzempion, grupujPoFormacie, RODZINA_Z_DYSKU } from "@/data/presets";
import { presetDlaFormatu } from "@/data/formaty";
import { time } from "@/lib/format";
import { api } from "@/store/transport";
import type { EmailConfig, MailCategories, Preset, SettingKey, Settings, SubjectPreview } from "@/types";
import "./views.css";

const RISK_COLOR = {
  low: "var(--long)",
  medium: "var(--accent)",
  high: "var(--warn)",
  extreme: "var(--short)",
} as const;

/* ============================================================
   EDYCJA PER PRESET — kontekst wiążący kontrolki z INNYM dokumentem

   Łańcuch z wieloma presetami (SENTINEL-0: HYPER-2 + FRESHQUEEN-3) znaczy,
   że dokument panelu opisuje wyłącznie RACHUNEK, a pola handlu każdy silnik
   bierze ze SWOJEGO pliku presetu. Kontrolki pól są jednak jedne
   (`SettingField` czyta `app.settings`). Zamiast dublować komponenty,
   grupy presetowe dostają przez ten kontekst dokument WYBRANEGO presetu
   i zapis do JEGO pliku (`POST /api/presets/{name}/settings`).
   `null` = tryb zwykły: dokument panelu, jak zawsze.
   ============================================================ */
const EdycjaPresetuCtx = createContext<null | {
  /** nazwa presetu — do podpisów */
  nazwa: string;
  doc: Settings;
  set: <K extends keyof Settings>(key: K, value: Settings[K]) => void;
  patch: (patch: SettingControlPatch) => void;
}>(null);

export function SettingsView() {
  const app = useApp();
  const t = useT();
  const { lang } = useLanguage();
  const [tab, setTab] = useState<"presets" | "config" | "advanced" | "appearance" | "notify">(
    "config",
  );

  /* Schemat przechodzi przez `przetlumaczGrupy`: dla EN etykiety/hinty/warny
     podmienia nakładka `settingsSchema.en.ts` scalana PO KLUCZU pola —
     klucze ustawień zostają nietknięte (kanarek Rusta ich pilnuje). */
  const groups = useMemo<GroupDef[]>(() => {
    /* AUTO-EA = zaawansowane AUTO: pokazuje te same grupy zarządzania
       (kontrakt zera) PLUS sekcję „Warstwa EA (AUTO-EA)" — SZKIELET warstwy
       (własny zegar, maszyna Obrona/Neutral/Agresja, zapadka, dozór SL)
       i dom nadchodzących osi rodzin A–G. W zwykłym AUTO sekcja jest
       odfiltrowana, bo bramą warstwy jest sam tryb. */
    const bazowe =
      app.mode === "AUTO" || app.mode === "AUTO-EA"
        ? [
            ...MANAGEMENT_GROUPS.filter((g) => g.id !== "ea_layer" || app.mode === "AUTO-EA"),
            ...GENERAL_GROUPS,
          ]
        : [...GENERAL_GROUPS];
    return przetlumaczGrupy(bazowe, lang);
  }, [app.mode, lang]);

  const [active, setActive] = useState(groups[0]?.id ?? "panel");
  const current = groups.find((g) => g.id === active) ?? groups[0];

  
  const [szukaj, setSzukaj] = useState("");
  const szukanie = szukaj.trim().length > 0;

  const wyniki = useMemo(() => {
    if (!szukanie) return [];
    const q = szukaj.trim().toLowerCase();
    // spacje→podkreślenia, żeby „lot max" trafiało w klucz `lot_max`
    const qKlucz = q.replace(/\s+/g, "_");
    const s = app.settings;
    const out: { group: GroupDef; fields: FieldDef[] }[] = [];
    for (const g of groups) {
      const trafione = g.fields.filter((f) => {
        const klucz = String(f.key).toLowerCase();
        if (klucz.includes(qKlucz) || klucz.includes(q)) return true;
        if (f.label.toLowerCase().includes(q)) return true;
        if (f.hint && f.hint.toLowerCase().includes(q)) return true;
        // ostrzeżenie jest funkcją stanu — przeszukujemy to, co faktycznie
        // wisi pod polem TERAZ, nie hipotetyczne teksty
        const w = f.warn?.(s);
        if (w && w.toLowerCase().includes(q)) return true;
        return false;
      });
      if (trafione.length > 0) out.push({ group: g, fields: trafione });
    }
    return out;
  }, [szukanie, szukaj, groups, app.settings]);

  // kliknięcie zakładki po lewej WYŁĄCZA tryb szukania — wybór kategorii
  // jest jednoznaczną deklaracją „chcę zwykły widok tej zakładki"
  const wybierzGrupe = (id: string) => {
    setSzukaj("");
    setActive(id);
  };

  /* ---------------- KONFIGURACJA PER PRESET ----------------
     Łańcuch z JEDNYM presetem handlującym: przycisk „Konfiguracja" działa
     jak zwykła zakładka (dokument panelu). Z WIELOMA (SENTINEL-0): przycisk
     staje się selectem „Konfiguracja: X" — pola HANDLU edytują wtedy plik
     wybranego presetu (każdy osobno, niezależnie), a sekcja RACHUNEK
     pozostaje jedna i wspólna. Zapis idzie do pliku presetu, więc przeżywa
     restart; żywy silnik formatu przeładowuje plik w ciągu 2 s. */
  /* ŁAŃCUCH BIEŻĄCEGO TRYBU, nie `lancuchy.aktywny` (projekt EA-2): w AUTO-EA
     nogi bierze się ze składu warstwy EA i to JEGO presety ma stroić ta
     zakładka. `app.lancuch` liczy to raz, w jednym miejscu. */
  const aktywnyLancuch = app.lancuch;
  const presetyLancucha = useMemo(() => {
    const p = Object.values(aktywnyLancuch?.presety ?? {}).filter((x) => x !== "");
    return [...new Set(p)].sort();
  }, [aktywnyLancuch]);
  
  const nogiZPliku = useMemo(
    () => (app.stats.lotNogi ?? []).filter((n) => n.zPliku && n.preset),
    [app.stats.lotNogi],
  );
  const wielosilnik = nogiZPliku.length > 0 || presetyLancucha.length > 1;

  /* Lista presetów do wyboru: nogi grające z pliku mają pierwszeństwo, bo to
     ICH pola widać w zakładce. Przy jednonogim łańcuchu preset bywa tylko
     w `lotNogi` (łańcuch może go nie wymieniać pod tą samą nazwą). */
  const presetyDoEdycji = useMemo(() => {
    const z = nogiZPliku.map((n) => n.preset);
    return [...new Set([...z, ...presetyLancucha])];
  }, [nogiZPliku, presetyLancucha]);

  const [presetKonfig, setPresetKonfig] = useState("");
  useEffect(() => {
    if (wielosilnik && (!presetKonfig || !presetyDoEdycji.includes(presetKonfig))) {
      
      setPresetKonfig(
        nogiZPliku.find((n) => n.handluje ?? !n.zamrozona)?.preset ?? presetyDoEdycji[0] ?? "",
      );
    }
  }, [wielosilnik, presetyDoEdycji, presetKonfig, nogiZPliku]);

  /* ŻĄDANIE „POKAŻ PARAMETRY EA PRESETU X" — skrót z panelu łańcuchów
     (projekt EA-2). Ustawiamy trzy rzeczy naraz, bo dopiero komplet daje
     odpowiedź na pytanie, z którym użytkownik tu przyszedł: zakładkę
     konfiguracji, EDYTOWANY PRESET i grupę „Warstwa EA".

     Żądanie zdejmujemy OD RAZU po obsłużeniu — inaczej każda późniejsza
     zmiana zakładki wracałaby tu z powrotem, bo efekt widziałby je nadal. */
  const zadanieEa = app.zadanieEa;
  const wyczyscZadanieEa = app.wyczyscZadanieEa;
  useEffect(() => {
    if (!zadanieEa) return;
    setTab("config");
    setPresetKonfig(zadanieEa.preset);
    setSzukaj("");
    setActive("ea_layer");
    wyczyscZadanieEa();
  }, [zadanieEa, wyczyscZadanieEa]);

  // Tag the loaded document with its owner. A selector change must never
  // render/save the previous document under the newly selected preset name.
  const [presetDoc, setPresetDoc] = useState<{ nazwa: string; doc: Settings } | null>(null);
  useEffect(() => {
    if (!wielosilnik || !presetKonfig) {
      setPresetDoc(null);
      return;
    }
    let aktualne = true;
    api
      .presetUi(presetKonfig)
      .then((r) => {
        // Dokument presetu bywa CZĘŚCIOWY (plik niesie tylko pola różne od
        // domyślnych) — dosypujemy domyślne, żeby `when`/`warn` miały pełen
        // stan, dokładnie jak dokument panelu.
        if (aktualne) setPresetDoc({ nazwa: presetKonfig, doc: { ...DEFAULT_SETTINGS, ...(r.settings as Partial<Settings>) } });
      })
      .catch(() => {
        if (aktualne) setPresetDoc(null);
      });
    return () => {
      aktualne = false;
    };
  }, [wielosilnik, presetKonfig]);

  const zapiszPolePresetu = useCallback(
    <K extends keyof Settings>(key: K, value: Settings[K]) => {
      setPresetDoc((d) => (d?.nazwa === presetKonfig ? { ...d, doc: { ...d.doc, [key]: value } } : d));
      void api.savePresetSettings(presetKonfig, { [key as string]: value });
    },
    [presetKonfig],
  );

  const zapiszLatkePresetu = useCallback(
    (patch: SettingControlPatch) => {
      setPresetDoc((d) => (d?.nazwa === presetKonfig ? { ...d, doc: { ...d.doc, ...patch } } : d));
      void api.savePresetSettings(presetKonfig, patch);
    },
    [presetKonfig],
  );

  const edycjaCtx =
    wielosilnik && presetDoc?.nazwa === presetKonfig && presetKonfig
      ? { nazwa: presetKonfig, doc: presetDoc.doc, set: zapiszPolePresetu, patch: zapiszLatkePresetu }
      : null;

  /** Kontekst edycji presetu dostaje KAŻDA grupa — o warstwie decyduje POLE,
      nie grupa (łatka Ł-3). `SettingField` sam odfiltruje pola rachunkowe:
      `konto_dzwignia` w grupie presetowej dalej jedzie dokumentem, a
      `lot_max` w grupie rachunkowej trafia wreszcie do pliku presetu. */
  const owinGrupe = (g: GroupDef, el: React.ReactNode) =>
    wielosilnik && !edycjaCtx ? (
      <fieldset key={g.id} disabled style={{ border: 0, padding: 0, margin: 0, minWidth: 0 }}>
        <p className="hint" role="status">{t("adv.owner.loading")}</p>
        {el}
      </fieldset>
    ) : edycjaCtx ? (
      <EdycjaPresetuCtx.Provider key={g.id} value={edycjaCtx}>
        {el}
      </EdycjaPresetuCtx.Provider>
    ) : (
      el
    );

  return (
    <div className="view">
      <div className="view__head">
        <div className="view__headmain">
          <h1>{t("set.title")}</h1>
          <p>
            {app.mode === "AUTO" && t("set.desc.auto")}
            {app.mode === "AUTO-EA" && t("set.desc.autoea")}
            {app.mode === "MANUAL" && t("set.desc.manual")}
            {app.mode === "AI" && t("set.desc.ai")}
          </p>
          {/* TRZY WARSTWY, TRZY MIEJSCA. Użytkownik ma prawo wiedzieć, która
              z nich przeżyje wczytanie presetu — bo dwie z nich nie przeżyją. */}
          <p className="hint">
            <RichT k="set.layers" />
          </p>
        </div>
        <div className="row row--tight">
          <Button variant="ghost" icon="refresh" onClick={app.resetSettings}>
            {t("set.restoreDefaults")}
          </Button>
        </div>
      </div>

      <div className="row" role="tablist">
        {(
          [
            ["config", t("set.tab.config"), "sliders"],
            ["notify", t("set.tab.notify"), "mail"],
            ["appearance", t("set.tab.appearance"), "palette"],
            /* NAZWA ZAKŁADKI IDZIE ZA TRYBEM (projekt EA-2): w AUTO-EA to są
               PRESETY EA — te same pliki, ale ich sekcja EA jest tym, po co
               się tu wchodzi, a drabinka pod spodem układa je po saldzie. */
            ["presets", t(app.mode === "AUTO-EA" ? "set.tab.presets.ea" : "set.tab.presets"), "clipboard"],
            ["advanced", t("set.tab.advanced"), "microscope"],
          ] as const
        ).map(([id, label, icon]) =>
          id === "config" && wielosilnik ? (
            /* ŁAŃCUCH Z WIELOMA PRESETAMI: przycisk staje się selectem
               „Konfiguracja: X". Wybór przełącza edytowany preset; kliknięcie
               samego selecta wchodzi w zakładkę konfiguracji. */
            <div
              key={id}
              className="row row--tight"
              style={{ alignItems: "stretch" }}
              onMouseDown={() => setTab("config")}
            >
              <Select
                value={presetKonfig}
                onChange={(v) => {
                  setPresetKonfig(v);
                  setTab("config");
                }}
                /* ETYKIETA NIESIE FORMAT: „Konfiguracja: ZEN → NEWKOAN-3".
                   Bez formatu operator nie wie, czyją nogę stroi — a przy
                   dwóch nogach to jest jedyna rzecz, którą musi wiedzieć. */
                options={presetyDoEdycji.map((p) => {
                  const noga = (app.stats.lotNogi ?? []).find((n) => n.preset === p);
                  return {
                    value: p,
                    label: t("set.configFor", { name: noga?.format ? `${noga.format} → ${p}` : p }),
                  };
                })}
              />
            </div>
          ) : (
            <Button
              key={id}
              variant={tab === id ? "primary" : "outline"}
              icon={icon as IconName}
              onClick={() => setTab(id)}
            >
              {label}
            </Button>
          ),
        )}
      </div>

      {tab === "notify" && <NotifySection />}

      {tab === "appearance" && <AppearanceSection />}

      {tab === "presets" && (
        <>
          <DrabinkaPresetow />
          <PresetGallery />
        </>
      )}

      {tab === "advanced" && (
        <EdycjaPresetuCtx.Provider key={presetKonfig || "global"} value={edycjaCtx}>
          <AdvancedSection zablokowane={wielosilnik && !edycjaCtx} />
        </EdycjaPresetuCtx.Provider>
      )}

      {tab === "config" && (
        <>
          {app.mode === "AI" && <AiSection />}
          {app.mode === "MANUAL" && (
            <Card>
              <div className="modenote" style={{ border: "none", padding: 0, background: "transparent" }}>
                <span className="modenote__icon" style={{ background: "var(--info-soft)", color: "var(--info-text)" }}>
                  <Icon name="hand" size={16} />
                </span>
                <div>
                  <b>{t("set.manualCard.title")}</b>
                  <p>
                    <RichT k="set.manualCard.text" />
                  </p>
                </div>
              </div>
            </Card>
          )}

          <div className="settings">
            {}
            <nav className="settings__nav">
              {/* SZUKAJKA nad kategoriami. Esc czyści; w trybie szukania
                  ŻADNA zakładka nie jest zaznaczona, a klik w zakładkę
                  wychodzi z szukania. */}
              <div className="settings__search">
                <Icon name="search" size={14} />
                <input
                  value={szukaj}
                  onChange={(e) => setSzukaj(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") setSzukaj("");
                  }}
                  placeholder={t("set.search.placeholder")}
                  spellCheck={false}
                />
                {szukanie && (
                  <button className="settings__searchclear" onClick={() => setSzukaj("")} title={t("set.search.clear")}>
                    <Icon name="x" size={12} />
                  </button>
                )}
              </div>
              {(["rachunek", "preset"] as const).map((zakres) => {
                const wZakresie = groups.filter((g) => g.zakres === zakres);
                if (wZakresie.length === 0) return null;
                return (
                  <div key={zakres}>
                    <div className="settings__scope" title={t(`set.zakresHint.${zakres}`)}>
                      <span>{t(`set.zakres.${zakres}`)}</span>
                      <i />
                    </div>
                    {wZakresie.map((g) => (
                      <button
                        key={g.id}
                        className="settings__navitem"
                        data-active={!szukanie && g.id === active}
                        onClick={() => wybierzGrupe(g.id)}
                      >
                        <Icon name={g.icon as IconName} size={14} />
                        <span>{g.title}</span>
                        <span className="settings__count">{g.fields.length}</span>
                      </button>
                    ))}
                  </div>
                );
              })}
            </nav>

            <div className="settings__groups">
              {szukanie ? (
                wyniki.length === 0 ? (
                  <Empty
                    icon="search"
                    title={t("set.search.empty.title")}
                    text={t("set.search.empty.text", { q: szukaj.trim() })}
                  />
                ) : (
                  /* Wyniki renderowane NORMALNIE — te same komponenty pól co
                     w zakładce, z nagłówkiem sekcji nad każdą grupą, żeby było
                     widać, skąd pole pochodzi (i czy to rachunek, czy preset). */
                  wyniki.map(({ group, fields }) =>
                    owinGrupe(
                      group,
                      <Card
                        key={group.id}
                        title={group.title}
                        icon={group.icon as IconName}
                        accent={group.accent}
                        subtitle={t("set.search.matches", { n: fields.length, m: group.fields.length })}
                        actions={
                          <Tooltip content={t(`set.zakresHint.${group.zakres}`)}>
                            <Badge tone={group.zakres === "rachunek" ? "info" : "accent"} dot>
                              {group.zakres === "rachunek"
                                ? t("set.scope.rachunek")
                                : edycjaCtx
                                  ? t("set.scope.presetName", { name: edycjaCtx.nazwa })
                                  : t("set.scope.preset")}
                            </Badge>
                          </Tooltip>
                        }
                      >
                        <div className="setgrid">
                          {fields.map((f) => (
                            <SettingField key={String(f.key)} field={f} grupa={group} />
                          ))}
                        </div>
                      </Card>,
                    ),
                  )
                )
              ) : (
                <>
                  {current && owinGrupe(current, <GroupCard group={current} />)}
                  {active === "journal" && <JournalHelp />}
                </>
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}

/* ============================================================
   DZIENNIK ZDARZEŃ — jak z niego korzystać

   Sam przełącznik nie wystarczy: dziennik jest wart tyle, ile umiemy
   z niego wyciągnąć. Ta karta pokazuje, gdzie leży plik i jakim jednym
   poleceniem odpowiada się na pytanie „która reguła kosztuje najwięcej”.
   ============================================================ */
const JOURNAL_PYTANIA: { q: string; a: string; cmd: string }[] = [
  { q: "j.q1", a: "j.a1", cmd: "loganaliza logs/journal" },
  { q: "j.q2", a: "j.a2", cmd: "loganaliza logs/journal --json raport.json" },
  { q: "j.q3", a: "j.a3", cmd: 'jq -r "select(.reason) | .reason" *.jsonl | sort | uniq -c' },
  { q: "j.q4", a: "j.a4", cmd: 'jq "select(.kind==\\"risk_stop\\")" *.jsonl' },
];

function JournalHelp() {
  const t = useT();
  return (
    <Card title={t("j.title")} icon="microscope" accent="var(--ai)" subtitle={t("j.subtitle")}>
      <p className="hint" style={{ marginBottom: "var(--sp-3)" }}>
        {t("j.intro")}
      </p>
      <div style={{ display: "grid", gap: "var(--sp-3)" }}>
        {JOURNAL_PYTANIA.map((x) => (
          <div key={x.q} style={{ display: "grid", gap: 4 }}>
            <b style={{ fontSize: 12 }}>{t(x.q)}</b>
            <span className="hint">{t(x.a)}</span>
            <code
              style={{
                fontSize: 11,
                padding: "4px 8px",
                borderRadius: 4,
                background: "var(--surface-2, rgba(127,127,127,.12))",
                overflowX: "auto",
                whiteSpace: "pre",
              }}
            >
              {x.cmd}
            </code>
          </div>
        ))}
      </div>
    </Card>
  );
}

/* ============================================================
   GRUPA USTAWIEŃ
   ============================================================ */
function GroupCard({ group }: { group: GroupDef }) {
  const app = useApp();
  const t = useT();
  const edycja = useContext(EdycjaPresetuCtx);
  /* Warunki `when`/`warn`/martwe gałęzie liczą się na TYM dokumencie,
     który kontrolki edytują — inaczej pola zwijałyby się według panelu,
     a edytowały preset.

     Od łatki Ł-3 kontekst edycji dostaje KAŻDA grupa (bo o warstwie decyduje
     pole), więc widoczność musi dalej patrzeć na ZAKRES GRUPY — inaczej
     grupa rachunkowa zaczęłaby zwijać pola według presetu. */
  const s = edycja && group.zakres === "preset" ? edycja.doc : app.settings;

  /* Nogi grające DOKUMENTEM (brak pliku presetu na dysku) — patrz Ł-6. */
  const nogiBezPliku = (app.stats.lotNogi ?? []).filter(
    (n) => !n.zPliku && (n.handluje ?? !n.zamrozona) && n.format,
  );
  const visible = group.fields.filter((f) => !f.when || f.when(s));

  
  const martwe = group.fields.filter((f) => {
    if (!f.when || f.when(s)) return false;
    return JSON.stringify(s[f.key]) !== JSON.stringify(DEFAULT_SETTINGS[f.key]);
  });

  return (
    <Card
      title={group.title}
      icon={group.icon as IconName}
      accent={group.accent}
      subtitle={t("set.group.subtitle", { n: visible.length, m: group.fields.length })}
      actions={
        <Tooltip content={t(`set.zakresHint.${group.zakres}`)}>
          <Badge tone={group.zakres === "rachunek" ? "info" : "accent"} dot>
            {group.zakres === "rachunek"
              ? t("set.scope.rachunek")
              : edycja
                ? t("set.scope.presetName", { name: edycja.nazwa })
                : t("set.scope.preset")}
          </Badge>
        </Tooltip>
      }
    >
      {/* ZAKRES NAD POLAMI, nie tylko w plakietce: to jest odpowiedź na
          pytanie „czy wczytanie presetu mi to skasuje", a pada ono dokładnie
          w chwili, gdy ktoś tu coś zmienia. */}
      <div className="scopenote">
        <Icon name={group.zakres === "rachunek" ? "wallet" : "clipboard"} size={14} style={{ flex: "none", marginTop: 1 }} />
        <div>
          <b>{t(`set.zakres.${group.zakres}`)}.</b> {t(`set.zakresHint.${group.zakres}`)}
          {}
          {group.zakres === "preset" && nogiBezPliku.length > 0 && (
            <p className="scopenote__warn">
              {t("nog.legsFromDoc", { n: nogiBezPliku.map((n) => n.format).join(", ") })}
            </p>
          )}
        </div>
      </div>

      <p className="hint" style={{ marginBottom: "var(--sp-3)" }}>
        {group.desc}
      </p>

      {martwe.length > 0 && (
        <div className="deadbranch">
          <Icon name="alert" size={14} style={{ flex: "none", marginTop: 2 }} />
          <div>
            <b>
              {martwe.length === 1 ? t("set.dead.one") : t("set.dead.many", { n: martwe.length })}
            </b>
            <p>
              {t("set.dead.text")} <b>{martwe.map((f) => f.label).join(", ")}</b>.
            </p>
          </div>
        </div>
      )}

      <div className="setgrid">
        {visible.map((f) => (
          <SettingField key={String(f.key)} field={f} grupa={group} />
        ))}
      </div>
    </Card>
  );
}

function SettingField({ field, grupa }: { field: FieldDef; grupa: GroupDef }) {
  const app = useApp();
  const t = useT();
  const edycja = useContext(EdycjaPresetuCtx);
  /* W trybie per preset kontrolka czyta i pisze dokument PRESETU, nie panelu —
     ALE tylko dla pól warstwy „preset". Pole RACHUNKU w grupie presetowej
     (`konto_dzwignia`) musi dalej jechać dokumentem, bo `ustawienia_formatu`
     i tak nadpisze je wartością z dokumentu; pole PRESETOWE w grupie
     rachunkowej (`lot_max`, `lot_min`, `price_tol`) musi jechać do pliku,
     inaczej edycja jest cichym no-opem (łatka Ł-3). */
  const warstwa = zakresPola(field.key, grupa.zakres);
  const doPresetu = !!edycja && warstwa === "preset";
  const s = doPresetu ? edycja!.doc : app.settings;
  const value = settingControlValue(s, field.key);
  const warn = field.warn?.(s) ?? null;
  // CO ZNACZY ZERO W TYM POLU — odznaka zamiast gołej cyfry albo pustki.
  // Tabela `ZNACZENIE_ZERA` jest jedynym źródłem prawdy; pole spoza niej
  // zostaje zwykłą liczbą. Patrz `wiedza/KONWENCJA_ZERA.md`.
  const znaczenieZera = ZNACZENIE_ZERA[field.key];
  const ryzykownyBrakLimitu =
    znaczenieZera === "bez-limitu" && BRAK_LIMITU_RYZYKOWNY.includes(field.key);
  // Klucze wypisane DOSŁOWNIE, żeby zobaczył je kanarek wielojęzyczności.
  // `npm run i18n` skanuje wyłącznie literały przekazane do tłumaczenia —
  // klucz sklejany ze zmiennej byłby dla niego niewidzialny i mógłby po
  // cichu zniknąć ze słownika.
  const napisZera = (z: ZnaczenieZera) =>
    z === "bez-limitu"
      ? t("set.zero.noLimit")
      : z === "wylaczone"
        ? t("set.zero.off")
        : t("set.zero.auto");

  const set = (v: unknown) => {
    const patch = settingControlPatch(s, field.key, v);
    if (Object.keys(patch).length > 1) {
      if (doPresetu) edycja!.patch(patch);
      else app.setSettings(patch);
    } else if (doPresetu) edycja!.set(field.key, v as Settings[typeof field.key]);
    else app.setSetting(field.key, v as Settings[typeof field.key]);
  };
  const aliasHint = field.key === "trail_after_tp2"
    ? t("set.alias.smartSlDelay", { n: effectiveSmartSlDelay(s) })
    : (field.key === "tp_detect_price" || field.key === "tp_detect_signal") && isAdvancedTpSource(s)
      ? t("set.alias.advancedTp") : null;
  const changed = JSON.stringify(value) !== JSON.stringify(DEFAULT_SETTINGS[field.key]);

  /* Pole stojące w grupie o INNEJ warstwie musi to napisać PRZY SOBIE —
     plakietka nad kartą mówi wtedy o czymś innym niż to jedno pole. */
  const notkaWarstwy = warstwaInnaNizGrupa(field.key, grupa.zakres)
    ? warstwa === "preset"
      ? t("nog.scopeField")
      : t("nog.scopeAccount")
    : null;

  if (field.type === "bool") {
    return (
      <div className={`setfield setfield--bool ${field.wide ? "setfield--wide" : ""}`} data-on={!!value}>
        <Switch checked={!!value} onChange={set} label={<span className="setfield__label">{field.label}</span>} />
        {field.hint && <span className="setfield__hint">{field.hint}</span>}
        {aliasHint && <span className="setfield__hint">{aliasHint}</span>}
        {notkaWarstwy && <span className="setfield__warstwa">{notkaWarstwy}</span>}
        <PodgladNog pole={field.key} warstwa={warstwa} />
        {warn && (
          <span className="setfield__warn">
            <Icon name="alert" size={12} style={{ flex: "none", marginTop: 1 }} />
            {warn}
          </span>
        )}
      </div>
    );
  }

  return (
    <div className={`setfield ${field.wide ? "setfield--wide" : ""}`}>
      <label className="setfield__label">
        {field.label}
        {changed && (
          <Tooltip content={t("set.defaultValue", { v: String(DEFAULT_SETTINGS[field.key]) })}>
            <span style={{ marginLeft: 5, color: "var(--accent-text)", fontSize: 9 }}>●</span>
          </Tooltip>
        )}
      </label>

      {field.type === "num" && (
        <NumberInput
          value={Number(value)}
          onChange={set}
          step={field.step ?? 1}
          min={field.min}
          max={field.max}
          unit={field.unit}
          zeroLabel={
            znaczenieZera
              ? `${napisZera(znaczenieZera)}${ryzykownyBrakLimitu ? " ⚠" : ""}`
              : undefined
          }
        />
      )}
      {field.type === "text" && <TextInput value={String(value ?? "")} onChange={set} />}
      {field.type === "select" && (
        <Select value={String(value)} onChange={set} options={field.options ?? []} />
      )}

      {field.hint && <span className="setfield__hint">{field.hint}</span>}
      {notkaWarstwy && <span className="setfield__warstwa">{notkaWarstwy}</span>}
      <PodgladNog pole={field.key} warstwa={warstwa} />
      {warn && (
        <span className="setfield__warn">
          <Icon name="alert" size={12} style={{ flex: "none", marginTop: 1 }} />
          {warn}
        </span>
      )}
    </div>
  );
}


function PodgladNog({ pole, warstwa }: { pole: SettingKey; warstwa: ZakresUstawien }) {
  const { ustawieniaNog } = useApp();
  if (warstwa !== "preset") return null;
  if (ustawieniaNog.filter((n) => n.handluje).length < 2) return null;
  return (
    <span className="setfield__nogi">
      <WartoscNog pole={pole} />
    </span>
  );
}

/* ============================================================
   TRYB AI — jedyny wybór to model
   ============================================================ */
function AiSection() {
  const app = useApp();
  const t = useT();
  const active = app.settings.ai_model;

  return (
    <>
      <div className="aihero">
        <span className="aihero__icon">
          <Icon name="brain" size={24} />
        </span>
        <div>
          <h2>{t("ai.hero.title")}</h2>
          <p>{t("ai.hero.text")}</p>
        </div>
      </div>

      <Card title={AI_GROUP.title} icon="brain" accent="var(--ai)" subtitle={t("ai.card.subtitle")}>
        <div className="col" style={{ maxWidth: 460 }}>
          <label className="field__label">{t("ai.model")}</label>
          <Select
            value={active}
            onChange={(v) => app.setSetting("ai_model", v)}
            options={AI_MODELS.map((m) => ({
              value: m.id,
              label: `${m.name}${m.recommended ? ` — ${t("common.recommended")}` : ""} · ${m.cadence}`,
            }))}
          />
          <span className="hint">{t("ai.model.hint")}</span>
        </div>
      </Card>

      <Card title={t("ai.available")} icon="layers" subtitle={`${AI_MODELS.length}`} accent="var(--ai)">
        <div className="aimodels">
          {AI_MODELS.map((m) => (
            <button
              key={m.id}
              className="aimodel"
              data-active={m.id === active}
              onClick={() => app.setSetting("ai_model", m.id)}
            >
              <div className="aimodel__head">
                <span className="aimodel__icon">
                  <Icon name="robot" size={16} />
                </span>
                <div style={{ minWidth: 0, flex: 1 }}>
                  <div className="aimodel__name">{m.name}</div>
                  <div className="hint" style={{ fontSize: "var(--fs-3xs)" }}>
                    v{m.version} · {m.params}
                  </div>
                </div>
                {m.recommended && <Badge tone="ai">{t("common.recommended")}</Badge>}
                {m.id === active && <Badge tone="accent">{t("ai.active")}</Badge>}
              </div>

              <p className="aimodel__desc">{t(m.description)}</p>

              <div className="aimodel__specs">
                <span>
                  {t("ai.cadence")} <b>{m.cadence}</b>
                </span>
                <span>
                  PF <b>{m.metrics.profitFactor.toFixed(1)}</b>
                </span>
                <span>
                  {t("ai.winDays")} <b>{m.metrics.winDays}%</b>
                </span>
                <span>
                  DD <b>${m.metrics.maxDd}</b>
                </span>
              </div>

              <div className="hint" style={{ fontSize: "var(--fs-3xs)" }}>
                {t("ai.trained", { v: t(m.trainedOn) })}
              </div>
            </button>
          ))}
        </div>
      </Card>
    </>
  );
}

/* ============================================================
   POWIADOMIENIA E-MAIL

   Trzy rzeczy, których nie miał bot.py:
    * kategorie z osobnymi przełącznikami (tam był JEDEN licznik na wszystkie
      błędy, więc alert o obsunięciu ginął, bo 9 minut wcześniej poszedł
      niezwiązany alert o MT5),
    * dławienie ze SCALANIEM zamiast gubienia powtórek,
    * przycisk „wyślij mail testowy", który zwraca wynik prawdziwej próby.

   Hasło SMTP jest polem JEDNOKIERUNKOWYM: serwer zawsze odsyła pusty ciąg
   (mieszka w secrets.json), a puste pole przy zapisie znaczy „zostaw stare".
   ============================================================ */

const KATEGORIE: { key: keyof MailCategories }[] = [
  { key: "lifecycle" },
  { key: "mt5Connection" },
  { key: "mt5RecoveryFailed" },
  { key: "drawdown" },
  { key: "orderError" },
  { key: "summary" },
  { key: "signalUnreadable" },
];

/* ============================================================
   TEMAT WYSYŁANYCH MAILI — własny szablon ze zmiennymi ${...}
   ============================================================

   Trzy rzeczy, których nie wolno tu zepsuć:

   1. **Pole ma WŁASNY stan lokalny.** Gdyby było sterowane wartością
      z migawki serwera (jak `to` czy `host`), każde naciśnięcie klawisza
      wywoływałoby `setEmail`, a echo z serwera wracałoby w środku pisania
      i przestawiało kursor na koniec. Dokładnie ten mechanizm zabił pole
      hasła SMTP (JUTRO_OD_RANA §0.1) — tam serwer zawsze odsyłał pustkę,
      więc nie dało się wpisać ANI JEDNEGO znaku.
   2. **Podgląd liczy SERWER.** Panel nie ma własnego silnika szablonów;
      to, co widać, jest tym samym łańcuchem, który wyjdzie w mailu.
   3. **Wybór z listy DOKLEJA, nie zastępuje.** Wstawiamy w miejscu kursora
      i nie kasujemy niczego — nawet zaznaczonego fragmentu. */
function TematMaila({
  zapisany,
  onZapisz,
}: {
  zapisany: string;
  onZapisz: (v: string) => void;
}) {
  const t = useT();
  const [tekst, setTekst] = useState(zapisany);
  const [podglad, setPodglad] = useState<SubjectPreview | null>(null);
  const [blad, setBlad] = useState<string | null>(null);
  const pole = useRef<HTMLInputElement>(null);
  /* Ostatnia wartość, którą SAMI wysłaliśmy. Bez niej pole z lokalnym stanem
     biłoby się z echem z migawki i kasowało literę wpisaną w trakcie lotu. */
  const wyslane = useRef(zapisany);
  /* Uchwyt do zapisu trzymamy w refie, bo rodzic tworzy go od nowa przy KAŻDYM
     renderze, a panel przerysowuje się kilka razy na sekundę (ticki, equity).
     Gdyby `onZapisz` był zależnością efektu, licznik 400 ms zerowałby się
     w kółko i zapis nigdy by nie ruszył. */
  const zapiszRef = useRef(onZapisz);
  zapiszRef.current = onZapisz;

  /* Zmiana Z ZEWNĄTRZ (druga karta przeglądarki, wczytanie presetu) — tylko
     wtedy nadpisujemy pole. Własne echo rozpoznajemy po `wyslane`. */
  useEffect(() => {
    if (zapisany !== wyslane.current) {
      wyslane.current = zapisany;
      setTekst(zapisany);
    }
  }, [zapisany]);

  /* Zapis 400 ms po ostatnim klawiszu. Wysyłanie po każdej literze zasypałoby
     `smtp.json` zapisami (jeden plik na znak) bez żadnego zysku. */
  useEffect(() => {
    if (tekst === wyslane.current) return;
    const t = setTimeout(() => {
      wyslane.current = tekst;
      zapiszRef.current(tekst);
    }, 400);
    return () => clearTimeout(t);
  }, [tekst]);

  /* Podgląd: 200 ms po ostatnim klawiszu ORAZ co 5 s. Odświeżanie w tle jest
     konieczne, bo podgląd ma odpowiadać na pytanie „jak wyglądałby temat,
     GDYBY MAIL WYSZEDŁ TERAZ" — a saldo i ceny zmieniają się same. */
  useEffect(() => {
    let zywy = true;
    const pobierz = async () => {
      try {
        const r = await api.emailSubject(tekst);
        if (!zywy) return;
        setPodglad(r);
        setBlad(null);
      } catch (e) {
        if (zywy) setBlad(String(e).replace(/^Error:\s*/, ""));
      }
    };
    const t = setTimeout(() => void pobierz(), 200);
    const i = setInterval(() => void pobierz(), 5000);
    return () => {
      zywy = false;
      clearTimeout(t);
      clearInterval(i);
    };
  }, [tekst]);

  const wstaw = (nazwa: string) => {
    if (!nazwa) return;
    const znacznik = "${" + nazwa + "}";
    const el = pole.current;
    const poz = el?.selectionStart ?? tekst.length;
    setTekst(tekst.slice(0, poz) + znacznik + tekst.slice(poz));
    /* Kursor za wstawioną zmienną, żeby dało się pisać dalej bez klikania. */
    requestAnimationFrame(() => {
      el?.focus();
      el?.setSelectionRange(poz + znacznik.length, poz + znacznik.length);
    });
  };

  const zmienne = podglad?.vars ?? [];

  return (
    <Card
      title={t("mail.subject.title")}
      icon="mail"
      accent="var(--info-text)"
      subtitle={tekst.trim() ? t("mail.subject.custom") : t("mail.subject.system")}
    >
      <div className="setgrid">
        <div className="setfield setfield--wide">
          <label className="setfield__label" htmlFor="temat-maila">
            {t("mail.subject.label")}
          </label>
          <input
            id="temat-maila"
            ref={pole}
            className="input"
            type="text"
            value={tekst}
            placeholder={t("mail.subject.ph")}
            onChange={(ev) => setTekst(ev.target.value)}
          />
          <span className="setfield__hint">
            <RichT k="mail.subject.hint" />
          </span>
        </div>

        <div className="setfield setfield--wide">
          <label className="setfield__label" htmlFor="temat-zmienne">
            {t("mail.vars.label")}
          </label>
          <Select
            id="temat-zmienne"
            value=""
            disabled={zmienne.length === 0}
            onChange={wstaw}
            options={[
              {
                value: "",
                label:
                  zmienne.length === 0
                    ? t("mail.vars.empty")
                    : t("mail.vars.pick", { n: zmienne.length }),
              },
              ...zmienne.map((v) => ({
                value: v.name,
                label: `${v.label}: \${${v.name}} = ${v.value}`,
              })),
            ]}
          />
          <span className="setfield__hint">
            <RichT k="mail.vars.hint" />
          </span>
        </div>

        <div className="setfield setfield--wide">
          <label className="setfield__label">{t("mail.preview")}</label>
          <div
            className="num"
            style={{
              padding: "var(--sp-2) var(--sp-3)",
              borderRadius: "var(--r-sm)",
              background: "var(--bg-inset)",
              border: "1px solid var(--border)",
              minHeight: 34,
              display: "flex",
              alignItems: "center",
              wordBreak: "break-word",
            }}
          >
            {blad ? <span className="setfield__warn">{blad}</span> : (podglad?.preview ?? "…")}
          </div>
          <span className="setfield__hint">
            {blad ? t("mail.preview.err") : podglad?.pusty ? t("mail.preview.system") : t("mail.preview.exact")}
          </span>
        </div>
      </div>
    </Card>
  );
}

function NotifySection() {
  const app = useApp();
  const t = useT();
  const e = app.email;
  const [haslo, setHaslo] = useState("");
  const [pokazHaslo, setPokazHaslo] = useState(false);
  const [wysylam, setWysylam] = useState(false);

  const zapisz = (patch: Partial<EmailConfig>) => app.setEmail({ ...e, ...patch, pass: "" });
  const kat = (patch: Partial<MailCategories>) =>
    app.setEmail({ ...e, pass: "", categories: { ...e.categories, ...patch } });

  const odbiorcy = e.to.split(/[,;\s]+/).filter((x) => x.includes("@"));

  const test = async () => {
    setWysylam(true);
    await app.sendTestEmail();
    setWysylam(false);
  };

  return (
    <>
      <Card
        title={t("mail.smtp.title")}
        icon="mail"
        accent="var(--accent)"
        subtitle={e.enabled ? t("mail.recipients.count", { n: odbiorcy.length }) : t("mail.disabled")}
        actions={
          <Button
            variant="outline"
            icon="bolt"
            disabled={wysylam || !e.host || odbiorcy.length === 0}
            onClick={() => void test()}
          >
            {wysylam ? t("mail.test.sending") : t("mail.test.send")}
          </Button>
        }
      >
        <div className="setgrid">
          <div className="setfield setfield--bool" data-on={e.enabled}>
            <Switch
              checked={e.enabled}
              onChange={(v) => zapisz({ enabled: v })}
              label={<span className="setfield__label">{t("mail.enable")}</span>}
            />
            <span className="setfield__hint">{t("mail.enable.hint")}</span>
          </div>

          <div className="setfield setfield--wide">
            <label className="setfield__label">{t("mail.to")}</label>
            <TextInput
              value={e.to}
              onChange={(v) => zapisz({ to: v })}
              placeholder={t("mail.to.ph")}
            />
            <span className="setfield__hint">
              {t("mail.to.hint")}
              {odbiorcy.length > 0 && t("mail.to.recognized", { n: odbiorcy.length })}
            </span>
          </div>

          <div className="setfield">
            <label className="setfield__label">{t("mail.host")}</label>
            <TextInput value={e.host} onChange={(v) => zapisz({ host: v })} />
            <span className="setfield__hint">{t("mail.host.hint")}</span>
          </div>

          <div className="setfield">
            <label className="setfield__label">{t("mail.port")}</label>
            <NumberInput value={e.port} onChange={(v) => zapisz({ port: v })} step={1} min={1} max={65535} />
          </div>

          <div className="setfield">
            <label className="setfield__label">{t("mail.security")}</label>
            <Select
              value={e.security}
              onChange={(v) => zapisz({ security: v as EmailConfig["security"] })}
              options={[
                { value: "starttls", label: "STARTTLS (port 587)" },
                { value: "ssl", label: "SSL/TLS (port 465)" },
                { value: "none", label: t("mail.security.none") },
              ]}
            />
            {e.security === "none" && (
              <span className="setfield__warn">
                <Icon name="alert" size={12} /> {t("mail.security.warn")}
              </span>
            )}
          </div>

          <div className="setfield">
            <label className="setfield__label">{t("mail.login")}</label>
            <TextInput value={e.user} onChange={(v) => zapisz({ user: v })} />
          </div>

          <div className="setfield">
            <label className="setfield__label">{t("mail.password")}</label>
            <div className="row row--tight">
              <TextInput
                value={haslo}
                onChange={setHaslo}
                type={pokazHaslo ? "text" : "password"}
                placeholder={t("mail.password.ph")}
              />
              <Button
                variant="ghost"
                icon={pokazHaslo ? "eye-off" : "eye"}
                onClick={() => setPokazHaslo((s) => !s)}
              />
              <Button
                variant="outline"
                icon="check"
                disabled={!haslo}
                onClick={() => {
                  app.setEmail({ ...e, pass: haslo });
                  setHaslo("");
                }}
              >
                {t("common.save")}
              </Button>
            </div>
            <span className="setfield__hint">
              <RichT k="mail.password.hint" />
            </span>
          </div>

          <div className="setfield setfield--wide">
            <label className="setfield__label">{t("mail.from")}</label>
            <TextInput
              value={e.from}
              onChange={(v) => zapisz({ from: v })}
              placeholder={t("mail.from.ph")}
            />
          </div>
        </div>
      </Card>

      <TematMaila zapisany={e.subject} onZapisz={(v) => zapisz({ subject: v })} />

      <Card
        title={t("mail.cat.title")}
        icon="layers"
        accent="var(--warn)"
        subtitle={t("set.group.subtitle", { n: KATEGORIE.filter((k) => e.categories[k.key]).length, m: KATEGORIE.length })}
      >
        <p className="hint" style={{ marginBottom: "var(--sp-3)" }}>
          {t("mail.cat.intro")}
        </p>
        <div className="setgrid">
          {KATEGORIE.map((k) => (
            <div key={k.key} className="setfield setfield--bool" data-on={e.categories[k.key]}>
              <Switch
                checked={e.categories[k.key]}
                onChange={(v) => kat({ [k.key]: v } as Partial<MailCategories>)}
                label={<span className="setfield__label">{t(`mail.cat.${k.key}`)}</span>}
              />
              <span className="setfield__hint">{t(`mail.cat.${k.key}.hint`)}</span>
            </div>
          ))}
        </div>
      </Card>

      <Card title={t("mail.throttle.title")} icon="filter" accent="var(--ai)">
        <p className="hint" style={{ marginBottom: "var(--sp-3)" }}>
          <RichT k="mail.throttle.intro" />
        </p>
        <div className="setgrid">
          <div className="setfield">
            <label className="setfield__label">{t("mail.throttle.window")}</label>
            <NumberInput
              value={e.throttle.windowMin}
              onChange={(v) => app.setEmail({ ...e, pass: "", throttle: { ...e.throttle, windowMin: v } })}
              step={1}
              min={0}
              unit="min"
            />
            <span className="setfield__hint">{t("mail.throttle.window.hint")}</span>
          </div>
          <div className="setfield">
            <label className="setfield__label">{t("mail.throttle.max")}</label>
            <NumberInput
              value={e.throttle.maxPerHour}
              onChange={(v) => app.setEmail({ ...e, pass: "", throttle: { ...e.throttle, maxPerHour: v } })}
              step={1}
              min={0}
              max={200}
            />
            <span className="setfield__hint">{t("mail.throttle.max.hint")}</span>
          </div>
          <div className="setfield">
            <label className="setfield__label">{t("mail.throttle.report")}</label>
            <NumberInput
              value={e.intervalMin}
              onChange={(v) => zapisz({ intervalMin: v })}
              step={15}
              min={1}
              unit="min"
            />
            <span className="setfield__hint">{t("mail.throttle.report.hint")}</span>
          </div>
        </div>
      </Card>
    </>
  );
}

/* ============================================================
   WYGLĄD — motyw i paleta
   ============================================================ */
function AppearanceSection() {
  const { theme, setTheme, palette, setPalette } = useTheme();
  const { lang, setLanguage } = useLanguage();
  const t = useT();

  return (
    <>
      {/* ---------------- JĘZYK / LANGUAGE ----------------
          Wiersz na język: flaga + nazwa W TYM języku (wymaganie nr 4).
          Klik = DOKŁADNIE ta sama ścieżka co skrót L (`setLanguage` z i18n):
          zapis do localStorage + PATCH klucza głównego `language` + toast. */}
      <Card title={t("lang.title")} icon="channels" accent="var(--accent)" subtitle={t("lang.subtitle", { n: LANGUAGES.length })}>
        <div className="row" style={{ gap: "var(--sp-3)", alignItems: "flex-start", flexWrap: "wrap" }}>
          <div className="palettes" style={{ minWidth: 260, flex: "0 1 420px" }}>
            {LANGUAGES.map((l) => (
              <button key={l.id} className="palette" data-active={lang === l.id} onClick={() => setLanguage(l.id)}>
                <span style={{ fontSize: 22, lineHeight: 1, flex: "none" }}>{l.flag}</span>
                <span className="palette__body">
                  <b>{l.native}</b>
                  <span>{l.id === "en" ? `${l.id.toUpperCase()} · ${t("common.default")}` : l.id.toUpperCase()}</span>
                </span>
                {lang === l.id && (
                  <span className="palette__check">
                    <Icon name="check" size={12} strokeWidth={3} />
                  </span>
                )}
              </button>
            ))}
          </div>
          <span className="hint" style={{ flex: 1, minWidth: 240 }}>
            <RichT k="lang.hint" />
          </span>
        </div>
      </Card>

      <Card title={t("app.theme.title")} icon="sun" accent="var(--accent)" subtitle={t("app.theme.subtitle")}>
        <div className="row" style={{ gap: "var(--sp-3)" }}>
          <Segmented<"dark" | "light">
            value={theme}
            onChange={setTheme}
            options={[
              {
                value: "dark",
                label: (
                  <>
                    <Icon name="moon" size={13} /> {t("app.theme.dark")}
                  </>
                ),
              },
              {
                value: "light",
                label: (
                  <>
                    <Icon name="sun" size={13} /> {t("app.theme.light")}
                  </>
                ),
              },
            ]}
          />
          <span className="hint" style={{ flex: 1, minWidth: 240 }}>
            <RichT k="app.theme.hint" />
          </span>
        </div>
      </Card>

      <Card
        title={t("app.palette.title")}
        icon="palette"
        accent="var(--accent)"
        subtitle={t("app.palette.subtitle", { n: PALETTES.length })}
      >
        <p className="hint" style={{ marginBottom: "var(--sp-3)" }}>
          {t("app.palette.hint")}
        </p>

        <div className="palettes">
          {PALETTES.map((p) => (
            <button key={p.id} className="palette" data-active={palette === p.id} onClick={() => setPalette(p.id)}>
              <span className="palette__swatches">
                <i style={{ background: p.swatch }} />
                <i className="palette__long" />
                <i className="palette__short" />
              </span>
              <span className="palette__body">
                <b>{t(`pal.${p.id}`)}</b>
                <span>{t(`pal.${p.id}.note`)}</span>
              </span>
              {palette === p.id && (
                <span className="palette__check">
                  <Icon name="check" size={12} strokeWidth={3} />
                </span>
              )}
            </button>
          ))}
        </div>
      </Card>

      <Card title={t("app.preview.title")} icon="eye" accent="var(--accent)">
        <div className="row" style={{ gap: "var(--sp-3)", alignItems: "flex-start" }}>
          <div className="col" style={{ minWidth: 200 }}>
            <span className="eyebrow">{t("app.preview.buttons")}</span>
            <div className="row row--tight">
              <Button variant="primary" icon="check">
                {t("app.preview.action")}
              </Button>
              <Button variant="long">BUY</Button>
              <Button variant="short">SELL</Button>
              <Button variant="danger" icon="trash" />
            </div>
          </div>
          <div className="col" style={{ minWidth: 200 }}>
            <span className="eyebrow">{t("app.preview.values")}</span>
            <div className="row row--tight">
              <span className="num up" style={{ fontSize: "var(--fs-lg)", fontWeight: 600 }}>
                +$248,10
              </span>
              <span className="num down" style={{ fontSize: "var(--fs-lg)", fontWeight: 600 }}>
                −$86,45
              </span>
            </div>
          </div>
          <div className="col" style={{ minWidth: 200 }}>
            <span className="eyebrow">{t("app.preview.badges")}</span>
            <div className="row row--tight">
              <Badge tone="accent">{t("app.preview.accent")}</Badge>
              <Badge tone="long">{t("app.preview.profit")}</Badge>
              <Badge tone="short">{t("app.preview.loss")}</Badge>
              <Badge tone="warn">{t("app.preview.warn")}</Badge>
              <Badge tone="ai">AI</Badge>
            </div>
          </div>
        </div>
      </Card>
    </>
  );
}

/* ============================================================
   PRESETY
   ============================================================ */

function DrabinkaPresetow() {
  
  const app = useApp();
  const t = useT();
  
  const d = app.drabinka;
  const ea = app.mode === "AUTO-EA";
  const balance = app.stats.balance;

  const opcjeLancuchow = useMemo(
    () =>
      app.lancuchy.lista.map((l) => ({
        value: l.nazwa,
        label: l.nazwa,
      })),
    [app.lancuchy.lista],
  );

  const zmien = (patch: Partial<typeof d>) => app.setDrabinka({ ...d, ...patch });

  /* GOTOWE DRABINKI Z SERWERA. Pobierane raz przy montowaniu — lista jest
     stała w binarce, więc odświeżanie jej w pętli byłoby ruchem bez treści.
     Błąd pobrania zostawia listę pustą i cała sekcja znika: brak gotowych
     drabinek nie może psuć ręcznej edycji, która działała wcześniej. */
  const [gotowe, setGotowe] = useState<
    { nazwa: string; opis: string; korona: boolean; szczeble: typeof d.szczeble; histerezaPct: number }[]
  >([]);
  useEffect(() => {
    let zywy = true;
    api
      .drabinki()
      .then((v) => zywy && setGotowe(v))
      .catch(() => undefined);
    return () => {
      zywy = false;
    };
  }, []);

  /* KTÓRA GOTOWA ODPOWIADA TEMU, CO STOI W BOCIE.
     Porównujemy same szczeble — histereza jest polem, które użytkownik może
     świadomie podkręcić, nie zmieniając tym drabinki. Brak dopasowania nie
     jest błędem: znaczy „konfiguracja własna" i tak też jest podpisany. */
  const kluczSzczebli = (sz: typeof d.szczeble) =>
    sz.map((s) => `${s.progBalance}:${s.lancuch}`).join("|");
  const pasujaca = gotowe.find((g) => kluczSzczebli(g.szczeble) === kluczSzczebli(d.szczeble))?.nazwa;
  const wybranaGotowa = gotowe.find((g) => g.nazwa === pasujaca);

  /* SZCZEBLE W STANIE ROBOCZYM, NIE WPROST DO SILNIKA.
     Każde uderzenie w pole progu leciało dotąd od razu jako `SetDrabinka`.
     Przy poprawianiu drabinki („500 → 1500, ale 1000 jest niżej") stan
     przejściowy jest sprzeczny, a serwer go teraz ODRZUCA — bez bufora
     użytkownik dostawałby czerwony komunikat po każdym znaku. Bufor trzyma
     wpis, pokazuje, co jest nie tak, i wysyła dopiero komplet, który przejdzie
     walidację silnika (te same reguły co `DrabinkaLancuchow::sprawdz`). */
  const [robocze, setRobocze] = useState(d.szczeble);
  const zdalne = JSON.stringify(d.szczeble);
  useEffect(() => {
    setRobocze(JSON.parse(zdalne));
  }, [zdalne]);

  /* Ta sama lista reguł co w Ruscie — panel ma powiedzieć to samo zdanie,
     zanim komenda w ogóle wyjdzie. Zwraca komunikat albo `null`. */
  const sprawdz = (sz: typeof d.szczeble): string | null => {
    if (sz.length === 0) return d.enabled ? t("drab.err.empty") : null;
    for (let i = 1; i < sz.length; i++) {
      if (sz[i].progBalance === sz[i - 1].progBalance)
        return t("drab.err.dup", { v: sz[i].progBalance.toFixed(0) });
      if (sz[i].progBalance < sz[i - 1].progBalance)
        return t("drab.err.order", {
          n: String(i + 1),
          v: sz[i].progBalance.toFixed(0),
          p: sz[i - 1].progBalance.toFixed(0),
        });
    }
    if (sz[0].progBalance !== 0) return t("drab.err.base", { v: sz[0].progBalance.toFixed(0) });
    return null;
  };
  const bladSzczebli = sprawdz(robocze);

  /* Zapis idzie do silnika TYLKO, gdy komplet jest poprawny. Niepoprawny
     zostaje w buforze z komunikatem — świadomie NIE sortujemy go po cichu,
     bo cichy sort zamienia literówkę w inną konfigurację. */
  const zapiszSzczeble = (sz: typeof d.szczeble) => {
    setRobocze(sz);
    if (!sprawdz(sz)) zmien({ szczeble: sz });
  };

  const ustawSzczebel = (i: number, patch: Partial<(typeof d.szczeble)[number]>) =>
    zapiszSzczeble(robocze.map((s, j) => (j === i ? { ...s, ...patch } : s)));

  /* Nowy szczebel ląduje NAD najwyższym — drabinka rośnie w górę, więc to
     jedyne miejsce, w którym nowy wpis nie łamie kolejności. Pierwszy w ogóle
     jest bazowy (0 $), bo bez niego konto poniżej progu nie ma czym grać. */
  const dodaj = () => {
    const naj = robocze.reduce((m, s) => Math.max(m, s.progBalance), 0);
    zapiszSzczeble([
      ...robocze,
      {
        progBalance: robocze.length === 0 ? 0 : naj + 500,
        lancuch: app.lancuchy.lista[0]?.nazwa ?? "",
      },
    ]);
  };

  const usun = (i: number) => zapiszSzczeble(robocze.filter((_, j) => j !== i));

  /* Bieżący szczebel: najwyższy próg ≤ balance (ta sama reguła co w silniku,
     bez histerezy — histereza dotyczy PRZEŁĄCZANIA, nie wyświetlania).
     Następny próg: najniższy próg > balance.

     Wiersze pokazujemy w KOLEJNOŚCI TABLICY, a nie posortowane do widoku:
     poprawna drabinka jest rosnąca z definicji (pilnuje tego walidacja), więc
     ekran i tak wychodzi rosnąco — a przy wpisie sprzecznym użytkownik widzi
     DOKŁADNIE to, co ma w konfiguracji, razem z miejscem błędu. Sortowanie do
     widoku ukrywałoby właśnie tę jedną rzecz, którą trzeba zobaczyć. */
  /* Warunek zapala się DOKŁADNIE wtedy, gdy start bota by się wywrócił:
     jest pieczęć, drabinka jest włączona i ma szczeble, a wśród nich NIE MA
     łańcucha z pieczęci. Ta sama reguła co w `lib.rs` przy `drabinka_rzadzi`
     — panel ma powiedzieć to samo zdanie, zanim maszyna je powie po fakcie. */
  const ostrzezeniePieczeci =
    !!app.pieczecLancuch &&
    d.enabled &&
    robocze.length > 0 &&
    !robocze.some((s) => s.lancuch === app.pieczecLancuch);

  const wiersze = robocze.map((s, i) => ({ s, i }));
  const malejaco = [...wiersze].sort((a, b) => b.s.progBalance - a.s.progBalance);
  const biezacy = malejaco.find(({ s }) => balance >= s.progBalance);
  const nastepny = [...malejaco].reverse().find(({ s }) => s.progBalance > balance);

  return (
    <Card
      
      /* NAZWA IDZIE ZA TRYBEM (EA-2c). W AUTO-EA karta nazywa się SKYNET-1 —
         tak właściciel nazwał produkcyjną drabinkę warstwy EA — bo dwie
         drabinki pod jednym napisem „Drabinka łańcuchów" to dokładnie ten
         stan, w którym nie wiadomo, którą się właśnie przestawia. */
      title={
        ea
          ? pasujaca
            ? t("drab.ea.title.named", { name: pasujaca })
            : t("drab.ea.title")
          : pasujaca
            ? t("drab.title.named", { name: pasujaca })
            : t("drab.title.custom")
      }
      icon="layers"
      accent="var(--long)"
      subtitle={
        d.enabled
          ? t("drab.subtitle.on", { n: robocze.length, h: d.histerezaPct })
          : t("drab.subtitle.off")
      }
      actions={<Switch checked={d.enabled} onChange={(v) => zmien({ enabled: v })} label={t("drab.enable")} />}
    >
      <p className="hint" style={{ marginBottom: "var(--sp-3)" }}>
        <RichT k="drab.intro" />
      </p>

      {/* CZYJA TO DRABINKA — jedno zdanie, zawsze widoczne.
          Karta wygląda identycznie w każdym trybie, więc bez tego zdania
          jedyną różnicą między „przestawiam AUTO" a „przestawiam warstwę EA"
          byłby napis w pasku na innym ekranie. */}
      <p className="hint" style={{ marginBottom: "var(--sp-3)" }}>
        <RichT k={ea ? "drab.scope.ea" : "drab.scope.auto"} />
      </p>

      {}
      {ostrzezeniePieczeci && (
        <p className="setfield__warn" style={{ marginBottom: "var(--sp-3)" }}>
          <Icon name="alert" size={12} />{" "}
          <RichT k="drab.seal.warn" vars={{ name: app.pieczecLancuch }} />
        </p>
      )}

      {}
      {gotowe.length > 0 && (
        <div className="row row--tight" style={{ marginBottom: "var(--sp-3)", alignItems: "flex-end" }}>
          <div style={{ flex: 1 }}>
            <Field label={t("drab.ready.label")} hint={wybranaGotowa?.opis ?? t("drab.ready.hint")}>
              <Select
                value={pasujaca ?? ""}
                onChange={(v) => {
                  const g = gotowe.find((x) => x.nazwa === v);
                  if (!g) return;
                  /* Włączamy drabinkę przy wyborze: wskazanie gotowej
                     konfiguracji, która potem nie działa, bo przełącznik stoi
                     na „wyłączona", to najgorszy możliwy stan — wygląda jak
                     zrobione, a nie gra. */
                  app.setDrabinka({
                    ...d,
                    enabled: true,
                    szczeble: g.szczeble.map((s) => ({ ...s })),
                    histerezaPct: g.histerezaPct,
                  });
                }}
                options={[
                  ...(pasujaca ? [] : [{ value: "", label: t("drab.ready.custom") }]),
                  ...gotowe.map((g) => ({
                    value: g.nazwa,
                    /* Korona ma DOKŁADNIE JEDNA pozycja — `korona` przychodzi
                       z serwera, panel jej nie wylicza. Gdyby liczył sam,
                       byłyby dwa źródła odpowiedzi na to samo pytanie. */
                    label: `${g.korona ? "👑 " : ""}${g.nazwa}`,
                  })),
                ]}
              />
            </Field>
          </div>
        </div>
      )}

      {/* BIEŻĄCE POŁOŻENIE NA DRABINCE — trzy liczby, bez szukania wzrokiem. */}
      <div className="row row--tight" style={{ marginBottom: "var(--sp-3)", flexWrap: "wrap", gap: 8 }}>
        <Badge tone="info" dot>
          {t("drab.balance", { v: balance.toFixed(2) })}
        </Badge>
        {biezacy ? (
          <Badge tone={d.enabled ? "accent" : "muted"} dot>
            {t("drab.rung", { name: biezacy.s.lancuch, v: biezacy.s.progBalance.toFixed(0) })}
          </Badge>
        ) : (
          <Badge tone="warn" dot>
            {t("drab.belowMin")}
          </Badge>
        )}
        {nastepny && (
          <Badge tone="muted">
            {t("drab.next", { v: nastepny.s.progBalance.toFixed(0), name: nastepny.s.lancuch })}
          </Badge>
        )}
        {d.biezacyProg >= 0 && d.histerezaPct > 0 && (
          <Badge tone="muted">
            {t("drab.downBelow", { v: (d.biezacyProg * (1 - d.histerezaPct / 100)).toFixed(0) })}
          </Badge>
        )}
      </div>

      {robocze.length === 0 ? (
        <Empty icon="layers" title={t("drab.empty.title")} text={t("drab.empty.text")} />
      ) : (
        <div className="stack">
          {wiersze.map(({ s, i }) => {
            const aktywnyWiersz = biezacy?.i === i;
            return (
              <div
                key={i}
                className="row row--tight"
                style={{
                  alignItems: "flex-end",
                  ...(aktywnyWiersz
                    ? {
                        background: "var(--accent-soft, rgba(120,120,255,.08))",
                        borderRadius: 6,
                        padding: "4px 6px",
                      }
                    : { padding: "4px 6px" }),
                }}
              >
                <div style={{ width: 150 }}>
                  <Field label={t("drab.from")}>
                    <NumberInput
                      value={s.progBalance}
                      onChange={(v) => ustawSzczebel(i, { progBalance: v })}
                      min={0}
                      step={50}
                    />
                  </Field>
                </div>
                <div style={{ flex: 1 }}>
                  <Field label={aktywnyWiersz ? t("drab.chain.here") : t("drab.chain")}>
                    <Select
                      value={s.lancuch}
                      onChange={(v) => ustawSzczebel(i, { lancuch: v })}
                      options={opcjeLancuchow}
                    />
                  </Field>
                </div>
                {/* SZCZEBLA BAZOWEGO nie wolno skasować: bez progu 0 $ konto
                    poniżej najniższego progu nie ma przypisanego łańcucha
                    i bot zostaje przy konfiguracji, której nikt nie wskazał. */}
                <Button
                  size="sm"
                  variant="danger"
                  icon="trash"
                  disabled={s.progBalance === 0 && robocze.length > 1}
                  title={s.progBalance === 0 && robocze.length > 1 ? t("drab.base.locked") : undefined}
                  onClick={() => usun(i)}
                >
                  {t("common.delete")}
                </Button>
              </div>
            );
          })}
        </div>
      )}

      {bladSzczebli && (
        <p
          className="hint"
          style={{ marginTop: "var(--sp-2)", color: "var(--warn-text)" }}
          role="alert"
        >
          {t("drab.err.badge")} {bladSzczebli}
        </p>
      )}

      {/* „DODAJ SZCZEBEL" NALEŻY DO LISTY SZCZEBLI, NIE DO HISTEREZY.
          Stały w jednym wierszu z polem histerezy, a wiersz miał
          `alignItems: flex-end`. Wysokość wiersza ustawiała czterowierszowa
          podpowiedź pod histerezą, więc przycisk zjeżdżał do jej dolnej
          krawędzi — kilkadziesiąt pikseli pod polem, z którym rzekomo
          sąsiadował. Wyrównywanie do dołu ma sens tylko wtedy, gdy sąsiedzi
          są podobnej wysokości; tutaj nie byli. Osobne wiersze rozwiązują to
          bez magicznych odstępów: przycisk siedzi tuż pod ostatnim
          szczeblem, którego dotyczy. */}
      <div className="row row--tight" style={{ marginTop: "var(--sp-2)" }}>
        <Button size="sm" variant="outline" icon="plus" onClick={dodaj}>
          {t("drab.add")}
        </Button>
      </div>

      <div className="row row--tight" style={{ marginTop: "var(--sp-3)" }}>
        <div style={{ width: 260 }}>
          <Field label={t("drab.hist")} hint={t("drab.hist.hint")}>
            <NumberInput
              value={d.histerezaPct}
              onChange={(v) => zmien({ histerezaPct: v })}
              min={0}
              max={100}
              step={0.5}
            />
          </Field>
        </div>
      </div>

      {d.enabled && d.histerezaPct === 0 && (
        <p className="hint" style={{ marginTop: "var(--sp-2)", color: "var(--warn-text)" }}>
          {t("drab.hist0.warn")}
        </p>
      )}

      {d.ostatniaZmianaTs > 0 && (
        <p className="hint" style={{ marginTop: "var(--sp-2)" }}>
          {t("drab.lastSwitch", { v: time(d.ostatniaZmianaTs) })}
        </p>
      )}
    </Card>
  );
}


function KafelekPresetu({ p }: { p: Preset }) {
  const app = useApp();
  const t = useT();
  const nogi = app.stats.lotNogi ?? [];
  const nogaTegoPresetu = nogi.find((n) => n.preset?.toUpperCase() === p.name.toUpperCase());
  const lancuchGra = nogi.some((n) => n.stan === "aktywna");
  const stan = nogaTegoPresetu?.stan;
  const aktywny = lancuchGra || nogi.length > 0 ? stan === "aktywna" : app.presetId === p.id;
  return (
    <button
      className="preset"
      data-active={aktywny}
      data-best={p.best}
      onClick={() => app.applyPreset(p.id)}
    >
      <div className="preset__head">
        {p.badge && <span style={{ fontSize: 15 }}>{p.badge}</span>}
        <span className="preset__name truncate">{p.name}</span>
        {/* KORONA NALEŻY DO DOKŁADNIE JEDNEGO PRESETU — pierwszego wpisu
            w `RANKING_PRESETOW`. Grupowanie po formacie przestawia tylko
            kolejność wyświetlania: gdyby korona szła „najlepszemu w grupie",
            byłoby ich tyle, ile formatów, i przestałaby cokolwiek znaczyć. */}
        {czyCzempion(p.name) && <Badge tone="warn">{t("pg.champion")}</Badge>}
        {aktywny && <Badge tone="accent">{t("pg.active")}</Badge>}
        {stan === "kolejka" && <Badge tone="muted">{t("pg.queued")}</Badge>}
        {stan === "nieaktywna" && nogaTegoPresetu?.powod === "minieta" && (
          <Badge tone="muted">{t("pg.past")}</Badge>
        )}
      </div>

      <div className="preset__tag">{opisPresetu(p.id, p.tagline)}</div>

      <div className="row" style={{ justifyContent: "space-between" }}>
        <Badge tone="muted">{p.family === RODZINA_Z_DYSKU ? t("pg.family.disk") : p.family}</Badge>
        <span className="riskbar" style={{ "--risk-color": RISK_COLOR[p.risk] } as React.CSSProperties}>
          {[1, 2, 3, 4].map((i) => (
            <i key={i} data-on={i <= { low: 1, medium: 2, high: 3, extreme: 4 }[p.risk]} />
          ))}
        </span>
      </div>

      {}
      {p.metrics.monthly > 0 || p.metrics.profitFactor > 0 ? (
        <div className="preset__metrics">
          <div className="preset__metric">
            <b className="up">{p.metrics.monthly > 0 ? `+${p.metrics.monthly}` : "—"}</b>
            <span>{t("pg.perMonth")}</span>
          </div>
          <div className="preset__metric">
            <b>{p.metrics.winDays > 0 ? `${p.metrics.winDays}%` : "—"}</b>
            <span>{t("pg.winDays")}</span>
          </div>
          <div className="preset__metric">
            <b className="down">{p.metrics.maxDd > 0 ? `−${p.metrics.maxDd}` : "—"}</b>
            <span>max DD</span>
          </div>
          <div className="preset__metric">
            <b>{p.metrics.profitFactor > 0 ? p.metrics.profitFactor.toFixed(1) : "—"}</b>
            <span>PF</span>
          </div>
        </div>
      ) : null}
    </button>
  );
}

function PresetGallery() {
  const app = useApp();
  const t = useT();
  const [q, setQ] = useState("");
  const [family, setFamily] = useState("all");

  // Zrodlem listy jest STORE, a nie katalog wbudowany w kod: z podlaczonym
  // botem sa to pliki z `presets/`, czyli dokladnie te, ktore czyta
  // `ApplyPreset`. Panel pokazujacy nazwy, ktorych bot nie zna, to obietnica
  // bez pokrycia — klikniecie takiego presetu zmienialoby ustawienia na cos,
  // czego nie ma w zadnym pliku uzytkownika.
  const wszystkie = app.presets;
  const families = useMemo(() => Array.from(new Set(wszystkie.map((p) => p.family))), [wszystkie]);
  const list = useMemo(
    () =>
      wszystkie.filter(
        (p) =>
          (family === "all" || p.family === family) &&
          /* Szukajka przegląda podpis W JĘZYKU, KTÓRY WIDAĆ — szukanie po
             ukrytym oryginale dawałoby trafienia w tekst, którego nie ma na ekranie. */
          (p.name.toLowerCase().includes(q.toLowerCase()) ||
            opisPresetu(p.id, p.tagline).toLowerCase().includes(q.toLowerCase())),
      ),
    [wszystkie, q, family],
  );

  /* PRESETY ROZDZIELONE PO FORMACIE — nie po rodzinie i nie po rankingu.
     Rodzina („Champion", „Runner") mówi, skąd preset się wziął; format mówi,
     CZY WOLNO GO TU UŻYĆ. Zmieszane w jednej siatce kuszą, żeby podpiąć
     preset ATFX pod kanał Synergy — a to są ustawienia opisujące inny
     sposób handlu, nie „trochę inne liczby". */
  const grupy = useMemo(() => grupujPoFormacie(list), [list]);

  return (
    <Card
      title={t("pg.title")}
      icon="clipboard"
      accent="var(--warn)"
      subtitle={`${list.length} z ${wszystkie.length} · ${app.presetsFromDisk ? t("pg.src.disk") : t("pg.src.builtin")}`}
      actions={
        <>
          <TextInput value={q} onChange={setQ} placeholder={t("pg.search")} icon="search" size="sm" style={{ width: 210 }} />
          <Select
            value={family}
            onChange={setFamily}
            size="sm"
            options={[{ value: "all", label: t("pg.family.all") }, ...families.map((f) => ({ value: f, label: f }))]}
          />
        </>
      }
    >
      <p className="hint" style={{ marginBottom: "var(--sp-3)" }}>
        {app.presetsFromDisk ? t("pg.intro.disk") : t("pg.intro.builtin")} <RichT k="pg.intro.split" />
      </p>

      {grupy.length === 0 ? (
        <Empty icon="search" title={t("pg.empty.title")} text={t("pg.empty.text")} />
      ) : (
        grupy.map(({ format, presety }) => {
          const gra = presetDlaFormatu(app.lancuch, format);
          return (
            <section key={format} className="presetgrp">
              <header className="presetgrp__head">
                <Icon name="book" size={13} />
                <b>{format}</b>
                <Badge tone="muted">{presety.length}</Badge>
                {gra ? (
                  <Badge tone="accent">{t("pg.playing", { chain: app.aktywnaNazwaLancucha, preset: gra })}</Badge>
                ) : (
                  <Badge tone="warn">{t("pg.notTrading", { chain: app.aktywnaNazwaLancucha })}</Badge>
                )}
              </header>
              <div className="presets">
                {presety.map((p) => (
                  <KafelekPresetu key={p.id} p={p} />
                ))}
              </div>
            </section>
          );
        })
      )}

      {/* OCENA CZTEROTRYBOWA — poza kafelkami, bo kafelek jest przyciskiem,
          a ocena ma podpowiedzi i musi dac sie czytac, a nie klikac.
          Pokazujemy ja dla presetu AKTYWNEGO (a gdy zadnego nie wczytano —
          dla czempiona), zeby liczba na ekranie dotyczyla tego, czym
          silnik naprawde gra. */}
      {(() => {
        const wybrany = list.find((p) => p.id === app.presetId) ?? list.find((p) => czyCzempion(p.name));
        return wybrany ? <OcenaPresetu preset={wybrany} /> : null;
      })()}
    </Card>
  );
}

/* ============================================================
   ZAAWANSOWANE — auto-generowane z listy kluczy
   ============================================================ */
function AdvancedSection({ zablokowane = false }: { zablokowane?: boolean } = {}) {
  const app = useApp();
  const t = useT();
  const edycja = useContext(EdycjaPresetuCtx);
  const [q, setQ] = useState("");

  const keys = useMemo(() => {
    const all = Object.keys(DEFAULT_SETTINGS) as (keyof Settings)[];
    return all
      .filter((k) => k !== "merge_config")
      .filter((k) => !COVERED_KEYS.has(k))
      .filter((k) => k.toLowerCase().includes(q.toLowerCase()));
  }, [q]);

  const covered = useMemo(() => {
    const all = Object.keys(DEFAULT_SETTINGS).filter((k) => k !== "merge_config");
    return all.length;
  }, []);

  return (
    <Card
      title={t("adv.title")}
      icon="microscope"
      accent="var(--ai)"
      subtitle={t("adv.subtitle", { n: keys.length, m: covered })}
      actions={<TextInput value={q} onChange={setQ} placeholder={t("adv.filter.ph")} icon="filter" size="sm" style={{ width: 280 }} />}
    >
      <p className="hint" style={{ marginBottom: "var(--sp-3)" }}>
        {t("adv.intro")}
      </p>
      <p className="hint" role="status">
        {zablokowane ? t("adv.owner.loading") : edycja
          ? t("adv.owner.preset", { name: edycja.nazwa })
          : t("adv.owner.global")}
      </p>

      {keys.length === 0 ? (
        <Empty icon="search" title={t("adv.empty.title")} text={t("adv.empty.text")} />
      ) : (
        <div className="adv">
          {keys.map((k) => {
            const pole = powiazPoleUstawien(k, app, edycja, zablokowane);
            const v = pole.value;
            const owner = pole.opis?.nieobslugiwane ? t("adv.owner.archived")
              : !pole.opis ? t("adv.owner.unknown")
              : pole.wlasciciel ? t("adv.owner.selected", { name: pole.wlasciciel })
              : t("adv.owner.account");
            const label = <span title={`${k} · ${pole.opis?.dowod ?? "unknown"}`}>{k}<small className="hint" style={{ display: "block" }}>{owner}</small></span>;
            if (pole.zablokowane) {
              return <label key={k} className="advrow" data-setting-owner={owner}>{label}<output>{String(v)}</output></label>;
            }
            if (typeof v === "boolean") {
              return (
                <label key={k} className="advrow" data-setting-owner={owner}>
                  <Checkbox checked={v} onChange={(x) => pole.set(x as Settings[typeof k])} />
                  {label}
                </label>
              );
            }
            if (typeof v === "number") {
              return (
                <label key={k} className="advrow" data-setting-owner={owner}>
                  {label}
                  <NumberInput value={v} onChange={(x) => pole.set(x as Settings[typeof k])} step={0.1} size="sm" />
                </label>
              );
            }
            return (
              <label key={k} className="advrow" data-setting-owner={owner}>
                {label}
                <TextInput value={String(v)} onChange={(x) => pole.set(x as Settings[typeof k])} size="sm" />
              </label>
            );
          })}
        </div>
      )}
    </Card>
  );
}
