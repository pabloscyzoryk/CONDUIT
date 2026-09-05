/* CONDUIT GRAPH — analiza wykresów z backtestów.
 *
 * ZASADA WYDAJNOŚCI, od której zależy cała reszta:
 * każdy panel rysuje się do WŁASNEGO płótna poza ekranem i tylko wtedy, gdy
 * zmieni się stan. Ruch myszy NIE przerysowuje wykresów — przepisuje gotowy
 * obraz i dokłada celownik. Pierwszy prototyp liczył szczyt kroczący na 11 tys.
 * punktów przy każdym pikselu ruchu i mulił; tutaj szczyt liczy serwer, raz
 * przy wczytaniu pliku.
 *
 * ZASADA DANYCH: krzywa idzie wprost z silnika (`*_dane.json` → serwer).
 * Nic nie jest odczytywane z pikseli, nic nie jest rekonstruowane.
 */
"use strict";

const API = "/api/graph";
const $ = (i) => document.getElementById(i);
const DOBA = 864e5, GODZ = 36e5, MIN = 6e4;
const DOW = ["nd", "pn", "wt", "śr", "cz", "pt", "sb"];

const fmt = (v, d = 2) =>
  (v == null || !isFinite(v)) ? "—"
    : v.toLocaleString("pl-PL", { minimumFractionDigits: d, maximumFractionDigits: d });
const kw = (v) => {
  const a = Math.abs(v);
  return a >= 1e6 ? fmt(v / 1e6, 2) + "M" : a >= 1000 ? fmt(v / 1000, 1) + "k" : fmt(v, a < 10 ? 2 : 0);
};
const zn = (v, d = 2) => (v >= 0 ? "+" : "") + fmt(v, d);
const dwa = (n) => String(n).padStart(2, "0");

/* Czas W UTC — tak jak liczy silnik (`Settings::session_offset()` = 0).
 * Użycie strefy przeglądarki przesunęłoby dobę o dwie godziny i słupek
 * „24 lipca" obejmowałby inny materiał niż wiersz „2026-07-24" z silnika. */
const dataU = (ts) => { const d = new Date(ts);
  return d.getUTCFullYear() + "-" + dwa(d.getUTCMonth() + 1) + "-" + dwa(d.getUTCDate()); };
const czasU = (ts) => { const d = new Date(ts);
  return dwa(d.getUTCHours()) + ":" + dwa(d.getUTCMinutes()) + ":" + dwa(d.getUTCSeconds()); };
const pelnyU = (ts) => dataU(ts) + " " + czasU(ts);
const dowU = (ts) => DOW[new Date(ts).getUTCDay()];

// [klucz, liczba mnoga, liczba pojedyncza] — bez trzeciej kolumny wychodziło
// „jeden prostokąt to tygodni", bo polskiej liczby pojedynczej nie da się
// zrobić przez obcięcie ostatniej litery
const POZIOMY = [
  ["miesiac", "miesiące", "miesiąc"], ["tydzien", "tygodnie", "tydzień"],
  ["dzien", "dni", "dzień"], ["godzina", "godziny", "godzina"],
  ["minuta", "minuty", "minuta"],
];
const NAZWA_POZ = Object.fromEntries(POZIOMY.map(([k, m]) => [k, m]));
const JEDEN_POZ = Object.fromEntries(POZIOMY.map(([k, , p]) => [k, p]));
const GLEBIEJ = { miesiac: "tydzien", tydzien: "dzien", dzien: "godzina", godzina: "minuta", minuta: null };
const CALE_DOBY = { miesiac: 1, tydzien: 1, dzien: 1 };

// ============================================================
//  STAN
// ============================================================

const S = {
  przebiegi: [], filtr: "",
  idA: null, idB: null, metaA: null, metaB: null,
  poziom: "dzien", auto: true,
  t0: 0, t1: 0, od: 0, do: 0,
  serA: null, serB: null, kub: [], kubB: [], kubW: [], trans: [],
  sumaSilnika: 0, obciete: false, krokMs: 0,
  log: true, bezWeek: true, bezPustych: false, znaczniki: true, procenty: false,
  roznica: false, rozjazd: null,
  stos: [], zazn: null, sortK: null, sortD: 1, wybranyKub: null,
};

let pokolenie = 0;                 // odrzucanie spóźnionych odpowiedzi
const pobierz = (u) => fetch(u).then((r) => r.json());

// ============================================================
//  LISTA PRZEBIEGÓW
// ============================================================

async function wczytajListe() {
  S.przebiegi = await pobierz(API + "/przebiegi");
  rysujListe();
  if (!S.idA && S.przebiegi.length) wybierz(S.przebiegi[0].id, false);
  if (!S.przebiegi.length) {
    $("glowny").insertAdjacentHTML("afterbegin",
      '<div class="pusto">Nie znalazłem żadnego przebiegu.<br><br>' +
      'Przebieg powstaje przy <code>btp --out &lt;katalog&gt;</code> jako plik ' +
      '<code>*_dane.json</code>.<br>Wskaż katalog po lewej albo uruchom ' +
      '<code>graph.exe --dane &lt;ścieżka&gt;</code>.</div>');
  }
}

function rysujListe() {
  const f = S.filtr.toLowerCase();
  const l = S.przebiegi.filter((p) =>
    !f || (p.etykieta + " " + p.preset + " " + p.tryb + " " + p.katalog).toLowerCase().includes(f));
  $("lista").innerHTML = l.map((p) => {
    const m = p.metryki || {};
    const kl = p.id === S.idA ? " a" : p.id === S.idB ? " bb" : "";
    const zysk = m.total_profit ?? 0;
    return `<div class="poz${kl}" data-id="${p.id}">
      <div class="nz">${p.etykieta}<em class="zr">${p.zrodlo || "?"}</em>${p.ma_transakcje ? '<em>transakcje</em>' : ""}</div>
      <div class="op">${p.od || "?"} … ${p.do || "?"} · ${p.tryb || "?"} · start ${fmt(p.saldo_start, 0)} $</div>
      <div class="wy"><span class="${zysk >= 0 ? "z" : "c"}">${zn(zysk, 0)} $</span>
        <span class="g"> · o włos ${fmt(m.min_equity ?? 0, 0)} $ · DD ${fmt(m.max_dd_pct ?? 0, 1)} %</span></div>
    </div>`;
  }).join("") || '<div class="szary mini">nic nie pasuje</div>';
  for (const el of document.querySelectorAll("#lista .poz")) {
    el.onclick = (e) => wybierz(el.dataset.id, e.shiftKey);
  }
}

async function wybierz(id, jakoB) {
  if (jakoB) {
    S.idB = (S.idB === id || id === S.idA) ? null : id;
    S.metaB = S.idB ? await pobierz(`${API}/p/${S.idB}`) : null;
    if (!S.idB) { S.roznica = false; S.kubB = []; S.rozjazd = null; }
    $("roznica").disabled = !S.idB;
    $("roznica").classList.toggle("on", S.roznica);
    rysujListe(); legenda(); await odswiez(); return;
  }
  S.idA = id;
  if (S.idB === id) { S.idB = null; S.metaB = null; }
  S.metaA = await pobierz(`${API}/p/${id}`);
  S.t0 = S.metaA.t0; S.t1 = S.metaA.t1 + 1;
  S.od = S.t0; S.do = S.t1;
  S.stos = []; S.zazn = null; S.wybranyKub = null; S.sortK = null;
  S.krokMs = S.metaA.krok_ms;
  S.poziom = S.auto ? poziomAuto(S.od, S.do) : S.poziom;
  $("podpis").textContent =
    `${S.metaA.preset || "?"} @ ${S.metaA.zrodlo || "?"} · ${S.metaA.od} … ${S.metaA.do} · ` +
    `${S.metaA.tryb} · start ${fmt(S.metaA.saldo_start, 0)} $ · ` +
    `${S.metaA.punktow.toLocaleString("pl-PL")} próbek co ${Math.round(S.krokMs / 1000)} s`;
  rysujListe(); legenda(); przyciskiPoziomow();
  await odswiez();
}

// ============================================================
//  POZIOM I OKRUCHY
// ============================================================

function poziomAuto(od, doo) {
  const d = doo - od;
  if (d > 200 * DOBA) return "miesiac";
  if (d > 60 * DOBA) return "tydzien";
  if (d > 3 * DOBA) return "dzien";
  if (d > 6 * GODZ) return "godzina";
  return "minuta";
}

function przyciskiPoziomow() {
  $("poziomy").innerHTML =
    '<button class="n" data-p="auto">auto</button>' +
    POZIOMY.map(([k, n]) => `<button class="n" data-p="${k}">${n}</button>`).join("");
  for (const b of document.querySelectorAll("#poziomy button")) {
    b.classList.toggle("aktywny", b.dataset.p === "auto" ? S.auto : (!S.auto && b.dataset.p === S.poziom));
    b.onclick = () => {
      if (b.dataset.p === "auto") { S.auto = true; S.poziom = poziomAuto(S.od, S.do); }
      else { S.auto = false; S.poziom = b.dataset.p; }
      przyciskiPoziomow(); odswiez();
    };
  }
}

function okruchy() {
  const cz = [];
  cz.push(`<button data-i="-1">CAŁOŚĆ</button>`);
  S.stos.forEach((s, i) => {
    cz.push('<span class="strz">›</span>');
    cz.push(`<button data-i="${i}">${s.etykieta}</button>`);
  });
  cz.push('<span class="strz">›</span>');
  cz.push(`<span class="teraz">${NAZWA_POZ[S.poziom]} · ${pelnyU(S.od)} … ${pelnyU(S.do)}</span>`);
  $("okruchy").innerHTML = cz.join(" ");
  for (const b of document.querySelectorAll("#okruchy button")) {
    b.onclick = () => wrocDo(+b.dataset.i);
  }
  $("wyzej").disabled = S.stos.length === 0;
}

function wrocDo(i) {
  if (i < 0) {
    S.stos = []; S.od = S.t0; S.do = S.t1;
  } else {
    const s = S.stos[i];
    S.stos = S.stos.slice(0, i);
    S.od = s.od; S.do = s.do;
    if (!S.auto) S.poziom = s.poziom;
  }
  if (S.auto) S.poziom = poziomAuto(S.od, S.do);
  S.zazn = null; S.wybranyKub = null;
  odswiez();
}

function wejdz(k) {
  const gl = GLEBIEJ[S.poziom];
  if (!gl) return;
  S.stos.push({ poziom: S.poziom, od: S.od, do: S.do, etykieta: k.etykieta });
  S.od = k.t; S.do = k.do;
  if (S.auto) S.poziom = poziomAuto(S.od, S.do); else S.poziom = gl;
  S.zazn = null; S.wybranyKub = null; S.sortK = null;
  odswiez();
}

// ============================================================
//  POBRANIE DANYCH DLA OKNA
// ============================================================

let czekaNaOdswiez = null;

function odswiezPozniej() {         // przy przeciąganiu/zoomie: rysuj od razu,
  rysuj();                          // dociągnij dokładniejsze dane po chwili
  clearTimeout(czekaNaOdswiez);
  czekaNaOdswiez = setTimeout(odswiez, 170);
}

async function odswiez() {
  if (!S.idA) return;
  clearTimeout(czekaNaOdswiez);
  const g = ++pokolenie;
  const od = Math.round(S.od), doo = Math.round(S.do);
  const zap = [
    pobierz(`${API}/p/${S.idA}/seria?od=${od}&do=${doo}&cel=4000`),
    pobierz(`${API}/p/${S.idA}/slupki?poziom=${S.poziom}&od=${od}&do=${doo}`),
    (S.metaA && S.metaA.transakcji)
      ? pobierz(`${API}/p/${S.idA}/transakcje?od=${od}&do=${doo}&limit=4000`)
      : Promise.resolve({ transakcje: [] }),
    S.idB ? pobierz(`${API}/p/${S.idB}/seria?od=${od}&do=${doo}&cel=4000`) : Promise.resolve(null),
    S.idB ? pobierz(`${API}/p/${S.idB}/slupki?poziom=${S.poziom}&od=${od}&do=${doo}`) : Promise.resolve(null),
  ];
  const [ser, sl, tr, serB, slB] = await Promise.all(zap);
  if (g !== pokolenie) return;                 // przyszła nowsza odpowiedź
  S.serA = ser; S.serB = serB;
  S.kub = sl.kubelki; S.sumaSilnika = sl.suma_silnika; S.obciete = sl.obciete;
  S.kubB = slB ? slB.kubelki : [];
  S.trans = tr.transakcje || [];
  if (!S.auto && sl.poziom !== S.poziom) S.poziom = sl.poziom;
  zestawZB();
  filtrujKubelki();
  okruchy(); przyciskiPoziomow(); kafle(); tabela(); rysuj(); uwaga();
}

/* ROZJAZD DWÓCH PRZEBIEGÓW.
 *
 * Dowiedzione 03.08: pojedynczy compounding nie jest miarą — przesunięcie
 * znaczników o sekundę zmienia go siedmiokrotnie. Sama para krzywych tego nie
 * tłumaczy; trzeba wiedzieć, KTÓRY kubełek odpowiada za różnicę. Przykład
 * z tego samego dnia: HYPER-X1 przy 200 $ i 300 $ rozchodzi się czterokrotnie,
 * a cała różnica siedzi w JEDNYM dniu (01.07).
 *
 * Dlatego zestawiamy kubełki po znaczniku początku i liczymy `delta`
 * (wynik A − wynik B) oraz `udzial` — ile procent całego rozjazdu przypada
 * na ten jeden kubełek. Liczone raz na odświeżenie, nie przy rysowaniu. */
function zestawZB() {
  S.rozjazd = null;
  if (!S.kubB.length) { for (const k of S.kub) { k.b = null; k.delta = null; k.udzial = null; } return; }
  const mapa = new Map(S.kubB.map((k) => [k.t, k]));
  let sumaAbs = 0;
  for (const k of S.kub) {
    const b = mapa.get(k.t) || null;
    k.b = b ? b.profit : null;
    k.b_zamkniecie = b ? b.zamkniecie : null;
    k.delta = b ? k.profit - b.profit : null;
    if (k.delta != null) sumaAbs += Math.abs(k.delta);
  }
  let naj = null;
  for (const k of S.kub) {
    if (k.delta == null) continue;
    k.udzial = sumaAbs > 0 ? Math.abs(k.delta) / sumaAbs * 100 : 0;
    if (!naj || Math.abs(k.delta) > Math.abs(naj.delta)) naj = k;
  }
  if (naj) S.rozjazd = { kub: naj, sumaAbs };
}

/* Filtry działają na KUBEŁKACH, nie na osi czasu: oś zostaje ciągła, więc
 * odstęp po weekendzie widać jako przerwę. Kafle i tabela liczą się już
 * z odfiltrowanego zbioru — bo to one odpowiadają na pytanie „ile średnio
 * na dzień handlowy". */
function filtrujKubelki() {
  S.kubW = S.kub.filter((k) => {
    if (S.bezWeek) { const d = new Date(k.t).getUTCDay(); if (d === 0 || d === 6) return false; }
    if (S.bezPustych && !k.transakcje && Math.abs(k.profit) < 1e-9) return false;
    return true;
  });
}

function uwaga() {
  const w = [];
  if (S.obciete) w.push(`Zakres zawiera więcej niż 20 000 ${NAZWA_POZ[S.poziom]} — pokazuję początek. Zawęź okno albo zmień poziom.`);
  if (S.poziom === "minuta" && S.krokMs > 60000)
    w.push(`Ten przebieg ma krzywą próbkowaną co ${Math.round(S.krokMs / 1000)} s, więc większość minut jest pusta. Policz go ponownie: <code>btp … --krzywa-ms 60000</code>.`);
  // Zestawienie DWÓCH niezależnych dróg do tej samej liczby. Rozjazd nie jest
  // błędem sam w sobie (silnik zamyka dobę na pierwszym kursie następnej,
  // krzywa kończy się na ostatniej próbce), ale ma być widoczny, a nie ukryty.
  if (CALE_DOBY[S.poziom] && !dziennyTryb()) {
    const kr = S.kubW.reduce((a, k) => a + (k.profit_krzywa ?? 0), 0);
    if (Math.abs(kr - sumaWidoczna()) > 0.01)
      w.push(`Wynik z silnika ${zn(sumaWidoczna())} $ wobec ${zn(kr)} $ z różnicy equity na krzywej — ` +
        `rozbieżność ${zn(kr - sumaWidoczna())} $ bierze się z próbkowania (silnik zamyka dobę na pierwszym kursie następnej).`);
  }
  if (CALE_DOBY[S.poziom] && dziennyTryb())
    w.push(`Tryb <b>dzień po dniu</b>: o północy silnik zamyka wszystko i przywraca saldo startowe, więc różnica equity liczona przez północ nic nie znaczy. Wyniki dnia i wyżej biorę z <b>dni policzonych przez silnik</b>.`);
  if (S.rozjazd) {
    const k = S.rozjazd.kub;
    w.push(`Największy rozjazd A↔B: <b>${k.etykieta}</b> — ${zn(k.delta)} $ ` +
      `(${fmt(k.udzial, 0)} % całego rozjazdu w oknie; A ${zn(k.profit)} $ wobec B ${zn(k.b)} $) ` +
      `<button class="n" id="rozjazdSkocz">pokaż ten ${JEDEN_POZ[S.poziom]}</button>`);
  }
  if (S.zazn)
    w.push(`Zaznaczono ${pelnyU(S.zazn.a)} … ${pelnyU(S.zazn.b)} — <button class="n" id="zaznZoom">przybliż</button> <button class="n" id="zaznCsv">CSV krzywa</button> <button class="n" id="zaznX">wyczyść</button>`);
  const el = $("uwaga");
  el.innerHTML = w.join("<br>");
  el.classList.toggle("jest", w.length > 0);
  if (S.zazn) {
    $("zaznZoom").onclick = () => { S.od = S.zazn.a; S.do = S.zazn.b; S.zazn = null; if (S.auto) S.poziom = poziomAuto(S.od, S.do); odswiez(); };
    $("zaznCsv").onclick = () => csv("krzywa");
    $("zaznX").onclick = () => { S.zazn = null; uwaga(); rysuj(); };
  }
  if (S.rozjazd) $("rozjazdSkocz").onclick = () => wejdz(S.rozjazd.kub);
}

const sumaWidoczna = () => S.kubW.reduce((a, k) => a + k.profit, 0);
const dziennyTryb = () => !!(S.metaA && S.metaA.tryb === "daily");

// ============================================================
//  KAFLE
// ============================================================

function kafle() {
  const K = S.kubW, zt = K.filter((k) => k.transakcje > 0 || Math.abs(k.profit) > 1e-9);
  const suma = sumaWidoczna();
  const v = zt.map((k) => k.profit).sort((a, b) => a - b);
  const med = v.length ? (v.length % 2 ? v[(v.length - 1) / 2] : (v[v.length / 2 - 1] + v[v.length / 2]) / 2) : 0;
  const trans = K.reduce((a, k) => a + k.transakcje, 0);
  const wygr = K.reduce((a, k) => a + k.wygrane, 0);
  const zreal = K.reduce((a, k) => a + k.zrealizowany, 0);
  let minEq = Infinity, ddMin = 0;
  for (const k of K) { if (k.min < minEq) minEq = k.min; if (k.dd_pct < ddMin) ddMin = k.dd_pct; }
  if (!isFinite(minEq)) minEq = 0;
  const nazwa = NAZWA_POZ[S.poziom].toUpperCase();
  const k = [
    [zn(suma, 0) + " $", "SUMA W OKNIE", suma >= 0 ? "var(--zielony)" : "var(--czerwony)"],
    [fmt(zt.length ? suma / zt.length : 0, 1) + " $", "ŚREDNIA / " + JEDEN_POZ[S.poziom].toUpperCase(), "var(--tekst)"],
    [fmt(med, 1) + " $", "MEDIANA", med >= 0 ? "var(--zielony)" : "var(--czerwony)"],
    [fmt(minEq, 2) + " $", "PUNKT O WŁOS", "var(--zolty)"],
    [fmt(ddMin, 1) + " %", "MAX OD SZCZYTU", "var(--czerwony)"],
    [zt.length + " / " + K.length, "CZYNNE " + nazwa, "var(--tekst)"],
    [fmt(100 * zt.filter((x) => x.profit > 0).length / Math.max(zt.length, 1), 0) + " %",
      nazwa + " NA PLUSIE", "var(--tekst)"],
    [String(trans), "TRANSAKCJI", "var(--tekst)"],
    [fmt(100 * wygr / Math.max(trans, 1), 0) + " %", "WYGRANYCH", "var(--tekst)"],
    [zn(zreal, 0) + " $", "ZREALIZOWANY", zreal >= 0 ? "var(--zielony)" : "var(--czerwony)"],
  ];
  $("kafle").innerHTML = k.map(([w, e, c]) =>
    `<div class="kafel"><div class="w" style="color:${c}">${w}</div><div class="e">${e}</div></div>`).join("");
}

function legenda() {
  const l = [
    '<span><i style="background:#26d9a3"></i>equity <b>' + (S.metaA ? (S.metaA.preset || "A") : "A") + '</b></span>',
    '<span><i style="background:#4a9eff"></i>saldo zaksięgowane</span>',
    '<span><i style="background:#e5c07b"></i>różnica = wynik pływający</span>',
  ];
  if (S.metaB) l.push('<span><i style="background:#c678dd"></i>equity <b>' +
    (S.metaB.preset || "B") + " @ " + (S.metaB.zrodlo || "?") + " · start " +
    fmt(S.metaB.saldo_start, 0) + ' $</b> (porównanie)</span>');
  if (S.trans && S.trans.length) l.push('<span class="szary">▲ wejście · ● wyjście (zielone = zysk, czerwone = strata)</span>');
  $("legenda").innerHTML = l.join("");
}

// ============================================================
//  PŁÓTNA
// ============================================================

const M = { l: 78, r: 14, t: 10, b: 22 };
const P = ["cSlupki", "cKapital", "cDd"].map((id) => {
  const cv = $(id);
  return { cv, g: cv.getContext("2d"), off: document.createElement("canvas"), w: 0, h: 0 };
});
const [PS, PK, PD] = P;
const dym = $("dymek");
let L = null;                       // ostatnio policzony układ (do celownika)

function przygotuj(p) {
  const r = p.cv.getBoundingClientRect();
  const dpr = devicePixelRatio || 1;
  p.w = r.width; p.h = r.height;
  for (const c of [p.cv, p.off]) {
    if (c.width !== Math.round(r.width * dpr)) c.width = Math.round(r.width * dpr);
    if (c.height !== Math.round(r.height * dpr)) c.height = Math.round(r.height * dpr);
  }
  const g = p.off.getContext("2d");
  g.setTransform(dpr, 0, 0, dpr, 0, 0);
  g.clearRect(0, 0, r.width, r.height);
  return g;
}

function blit(p) {
  const dpr = devicePixelRatio || 1;
  p.g.setTransform(1, 0, 0, 1, 0, 0);
  p.g.clearRect(0, 0, p.cv.width, p.cv.height);
  p.g.drawImage(p.off, 0, 0);
  p.g.setTransform(dpr, 0, 0, dpr, 0, 0);
}

function siatkaCzasu(g, p, fx, podpisy) {
  const ph = p.h - M.t - M.b, zakres = S.do - S.od;
  let krok, form;
  if (zakres > 300 * DOBA) { krok = 30 * DOBA; form = (t) => dataU(t).slice(0, 7); }
  else if (zakres > 40 * DOBA) { krok = 7 * DOBA; form = (t) => dataU(t).slice(5); }
  else if (zakres > 6 * DOBA) { krok = DOBA; form = (t) => dataU(t).slice(5); }
  else if (zakres > 12 * GODZ) { krok = 6 * GODZ; form = (t) => czasU(t).slice(0, 5); }
  else if (zakres > 2 * GODZ) { krok = GODZ; form = (t) => czasU(t).slice(0, 5); }
  else if (zakres > 20 * MIN) { krok = 10 * MIN; form = (t) => czasU(t).slice(0, 5); }
  else { krok = MIN; form = (t) => czasU(t).slice(0, 5); }
  g.font = "10px ui-monospace,Consolas,monospace";
  g.textAlign = "center"; g.textBaseline = "top";
  for (let t = Math.ceil(S.od / krok) * krok; t <= S.do; t += krok) {
    const x = fx(t);
    if (x < M.l || x > p.w - M.r) continue;
    g.strokeStyle = "#151b28"; g.beginPath(); g.moveTo(x, M.t); g.lineTo(x, M.t + ph); g.stroke();
    if (podpisy) { g.fillStyle = "#6b7688"; g.fillText(form(t), x, M.t + ph + 5); }
  }
}

function osY(g, p, lo, hi, fy, sufiks) {
  const ph = p.h - M.t - M.b;
  g.font = "10px ui-monospace,Consolas,monospace";
  g.textAlign = "right"; g.textBaseline = "middle";
  for (let i = 0; i <= 5; i++) {
    const v = lo + (hi - lo) * i / 5, y = fy(v);
    g.strokeStyle = "#1a2130"; g.lineWidth = 1;
    g.beginPath(); g.moveTo(M.l, y); g.lineTo(p.w - M.r, y); g.stroke();
    g.fillStyle = "#7c879e"; g.fillText(sufiks === "%" ? fmt(v, 1) + "%" : kw(v), M.l - 7, y);
  }
  void ph;
}

// przelicznik wartości dla panelu kapitału: albo dolary, albo % od startu
function przelicznik(meta) {
  if (!S.procenty || !meta || !meta.saldo_start) return (v) => v;
  const s = meta.saldo_start;
  return (v) => (v / s - 1) * 100;
}

function rysuj() {
  if (!S.serA) return;
  const pw0 = PS.w - M.l - M.r;
  const fx = (t) => M.l + (t - S.od) / (S.do - S.od) * pw0;
  L = { fx };

  // ---------- panel 1: słupki ----------
  {
    const g = przygotuj(PS), p = PS;
    const pw = p.w - M.l - M.r, ph = p.h - M.t - M.b;
    const fxx = (t) => M.l + (t - S.od) / (S.do - S.od) * pw;
    const K = S.kubW;
    // tryb różnicowy: jeden słupek to A − B, czyli wprost wkład tego kubełka
    // w rozjazd dwóch przebiegów
    const roz = S.roznica && S.kubB.length > 0;
    const wart = (k) => roz ? (k.delta ?? 0) : k.profit;
    let lo = 0, hi = 0;
    for (const k of K) { const v = wart(k); if (v < lo) lo = v; if (v > hi) hi = v; }
    if (!roz) for (const k of K) if (S.kubB.length && k.b != null) {
      if (k.b < lo) lo = k.b; if (k.b > hi) hi = k.b;
    }
    const m = (hi - lo) * .08 || 1; hi += m; lo -= m;
    const fy = (v) => M.t + ph - (v - lo) / (hi - lo) * ph;
    osY(g, p, lo, hi, fy, "$");
    siatkaCzasu(g, p, fxx, false);
    const y0 = fy(0);
    g.strokeStyle = "#2a3346"; g.beginPath(); g.moveTo(M.l, y0); g.lineTo(p.w - M.r, y0); g.stroke();
    const pozycje = [];
    const najT = S.rozjazd ? S.rozjazd.kub.t : null;
    for (const k of K) {
      const xa = fxx(k.t), xb = fxx(k.do);
      if (xb < M.l - 2 || xa > p.w - M.r + 2) continue;
      const sz = Math.max(1.5, Math.min(30, (xb - xa) * .78));
      const xc = (xa + xb) / 2, v = wart(k), y = fy(v);
      g.fillStyle = roz ? (v >= 0 ? "#26d9a3" : "#c678dd")
        : (k.punktow === 0 ? "#232b3c" : (v >= 0 ? "#26d9a3" : "#e06c75"));
      const gora = Math.min(y, y0), wys = Math.max(Math.abs(y - y0), k.punktow === 0 ? 2 : 1);
      g.fillRect(xc - sz / 2, gora, sz, wys);
      // przebieg B jako wąski słupek obok — widać parę bez przełączania trybu
      if (!roz && k.b != null) {
        const yb = fy(k.b);
        g.fillStyle = "#c678dd";
        g.fillRect(xc + sz / 2 - Math.max(1, sz * .3), Math.min(yb, y0),
          Math.max(1, sz * .3), Math.max(Math.abs(yb - y0), 1));
      }
      pozycje.push({ k, xa: xc - sz / 2 - 1, xb: xc + sz / 2 + 1 });
      if (S.wybranyKub === k.t) {
        g.strokeStyle = "#4a9eff"; g.lineWidth = 1.5;
        g.strokeRect(xc - sz / 2 - 1.5, Math.min(y, y0) - 1.5, sz + 3, Math.abs(y - y0) + 3);
      }
      // kubełek odpowiadający za największą część rozjazdu — obwódka, żeby
      // dało się go znaleźć wzrokiem, a nie przez czytanie tabeli
      if (najT === k.t && S.kubB.length) {
        g.strokeStyle = "#e5c07b"; g.lineWidth = 1.5;
        g.strokeRect(xc - sz / 2 - 3, M.t + 1, sz + 6, ph - 2);
      }
    }
    L.slupki = { pozycje, fy, fx: fxx };
    blit(p);
  }

  // ---------- panel 2: kapitał ----------
  {
    const g = przygotuj(PK), p = PK;
    const pw = p.w - M.l - M.r, ph = p.h - M.t - M.b;
    const fxx = (t) => M.l + (t - S.od) / (S.do - S.od) * pw;
    const tA = przelicznik(S.metaA), tB = przelicznik(S.metaB);
    const A = S.serA;
    let lo = Infinity, hi = -Infinity;
    const zbadaj = (arr, tr) => { for (const v of arr) { const x = tr(v); if (x < lo) lo = x; if (x > hi) hi = x; } };
    zbadaj(A.eq, tA); zbadaj(A.sal, tA);
    if (S.serB) zbadaj(S.serB.eq, tB);
    if (!isFinite(lo)) { lo = 0; hi = 1; }
    const uLog = S.log && !S.procenty && lo > 0;
    const mm = (hi - lo) * .06 || 1;
    hi += mm; lo = uLog ? Math.max(lo * .93, .01) : lo - mm;
    const Lg = Math.log10(Math.max(lo, 1e-9)), Hg = Math.log10(Math.max(hi, 1e-9));
    const fy = (v) => uLog
      ? M.t + ph - (Math.log10(Math.max(v, lo)) - Lg) / (Hg - Lg) * ph
      : M.t + ph - (v - lo) / (hi - lo) * ph;

    g.font = "10px ui-monospace,Consolas,monospace";
    g.textAlign = "right"; g.textBaseline = "middle";
    for (let i = 0; i <= 5; i++) {
      const v = uLog ? Math.pow(10, Lg + (Hg - Lg) * i / 5) : lo + (hi - lo) * i / 5;
      const y = fy(v);
      g.strokeStyle = "#1a2130"; g.beginPath(); g.moveTo(M.l, y); g.lineTo(p.w - M.r, y); g.stroke();
      g.fillStyle = "#7c879e"; g.fillText(S.procenty ? fmt(v, 1) + "%" : kw(v), M.l - 7, y);
    }
    siatkaCzasu(g, p, fxx, false);

    // wypełnienie równe wynikowi pływającemu
    if (!S.procenty && A.sal.length === A.eq.length) {
      g.beginPath();
      for (let i = 0; i < A.t.length; i++) { const x = fxx(A.t[i]), y = fy(A.eq[i]); i ? g.lineTo(x, y) : g.moveTo(x, y); }
      for (let i = A.t.length - 1; i >= 0; i--) g.lineTo(fxx(A.t[i]), fy(A.sal[i]));
      g.closePath(); g.fillStyle = "rgba(229,192,123,.13)"; g.fill();
      g.beginPath();
      for (let i = 0; i < A.t.length; i++) { const x = fxx(A.t[i]), y = fy(A.sal[i]); i ? g.lineTo(x, y) : g.moveTo(x, y); }
      g.strokeStyle = "#4a9eff"; g.lineWidth = 1.2; g.stroke();
    }
    if (S.serB) {
      const B = S.serB;
      g.beginPath();
      for (let i = 0; i < B.t.length; i++) { const x = fxx(B.t[i]), y = fy(tB(B.eq[i])); i ? g.lineTo(x, y) : g.moveTo(x, y); }
      g.strokeStyle = "#c678dd"; g.lineWidth = 1.4; g.stroke();
    }
    g.beginPath();
    for (let i = 0; i < A.t.length; i++) { const x = fxx(A.t[i]), y = fy(tA(A.eq[i])); i ? g.lineTo(x, y) : g.moveTo(x, y); }
    g.strokeStyle = "#26d9a3"; g.lineWidth = 1.6; g.lineJoin = "round"; g.stroke();

    // próg „punkt o włos" — 40 $ ma sens tylko w dolarach
    if (!S.procenty && lo < 40 && hi > 40) {
      const y = fy(40);
      g.strokeStyle = "#e5c07b88"; g.setLineDash([4, 4]);
      g.beginPath(); g.moveTo(M.l, y); g.lineTo(p.w - M.r, y); g.stroke(); g.setLineDash([]);
      g.fillStyle = "#e5c07b"; g.textAlign = "left"; g.fillText("40 $", M.l + 5, y - 8);
    }

    // znaczniki transakcji — pozycje liczone RAZ, tu; celownik ich nie przelicza
    const zna = [];
    if (S.znaczniki && S.trans.length && S.trans.length <= 4000) {
      for (const tr of S.trans) {
        if (tr.open_ts >= S.od && tr.open_ts <= S.do) {
          const x = fxx(tr.open_ts), y = fy(tA(eqW(tr.open_ts)));
          g.fillStyle = tr.side === "Buy" ? "#4a9eff" : "#e59f5b";
          g.beginPath(); g.moveTo(x, y - 5); g.lineTo(x - 3.6, y + 1.6); g.lineTo(x + 3.6, y + 1.6);
          g.closePath(); g.fill();
          zna.push({ x, y, tr, typ: "wejście" });
        }
        if (tr.close_ts >= S.od && tr.close_ts <= S.do) {
          const x = fxx(tr.close_ts), y = fy(tA(eqW(tr.close_ts)));
          g.fillStyle = tr.profit >= 0 ? "#26d9a3" : "#e06c75";
          g.beginPath(); g.arc(x, y, 2.9, 0, 7); g.fill();
          if (tr.reason === "Sl" || tr.reason === "VirtualSl") {
            g.strokeStyle = "#e06c75"; g.lineWidth = 1;
            g.beginPath(); g.moveTo(x - 4, y - 4); g.lineTo(x + 4, y + 4);
            g.moveTo(x + 4, y - 4); g.lineTo(x - 4, y + 4); g.stroke();
          }
          zna.push({ x, y, tr, typ: "wyjście" });
        }
      }
    }
    // Znaczniki posortowane po X, żeby celownik szukał ich POŁOWIENIEM,
    // a nie przeglądaniem wszystkich 908 na każdy piksel ruchu myszy.
    zna.sort((a, b) => a.x - b.x);
    L.kapital = { fy, fx: fxx, tA, zna, znaX: Float64Array.from(zna, (z) => z.x) };
    blit(p);
  }

  // ---------- panel 3: obsunięcie ----------
  {
    const g = przygotuj(PD), p = PD;
    const pw = p.w - M.l - M.r, ph = p.h - M.t - M.b;
    const fxx = (t) => M.l + (t - S.od) / (S.do - S.od) * pw;
    const A = S.serA;
    let lo = 0; for (const v of A.dd) if (v < lo) lo = v;
    lo = Math.min(lo, -.5);
    const fy = (v) => M.t + ph - (v - lo) / (0 - lo) * ph;
    osY(g, p, lo, 0, fy, "%");
    siatkaCzasu(g, p, fxx, true);
    g.beginPath();
    for (let i = 0; i < A.t.length; i++) { const x = fxx(A.t[i]), y = fy(A.dd[i]); i ? g.lineTo(x, y) : g.moveTo(x, y); }
    g.strokeStyle = "#e06c75"; g.lineWidth = 1.3; g.stroke();
    if (A.t.length) {
      g.lineTo(fxx(A.t[A.t.length - 1]), fy(0)); g.lineTo(fxx(A.t[0]), fy(0)); g.closePath();
      g.fillStyle = "rgba(224,108,117,.18)"; g.fill();
    }
    L.dd = { fy, fx: fxx };
    blit(p);
  }

  if (S.zazn) zaznaczenie();
}

/* equity w danej chwili — dwudzielne szukanie w przerzedzonej serii.
 * Znaczniki transakcji trafiają dzięki temu NA krzywą, a nie obok niej. */
function eqW(t) {
  const A = S.serA;
  if (!A || !A.t.length) return 0;
  let i = 0, j = A.t.length - 1;
  if (t <= A.t[0]) return A.eq[0];
  if (t >= A.t[j]) return A.eq[j];
  while (j - i > 1) { const m = (i + j) >> 1; if (A.t[m] < t) i = m; else j = m; }
  const d = A.t[j] - A.t[i];
  return d > 0 ? A.eq[i] + (A.eq[j] - A.eq[i]) * (t - A.t[i]) / d : A.eq[i];
}

/* Kubełek zawierający chwilę `t`. Przy 20 000 kubełków (sufit serwera)
 * przeglądanie po kolei kosztowałoby tyle, co całe rysowanie. */
function kubelekW(t) {
  const K = S.kubW;
  if (!K.length) return null;
  let i = 0, j = K.length;
  while (i < j) { const m = (i + j) >> 1; if (K[m].t <= t) i = m + 1; else j = m; }
  const k = K[i - 1];
  return k && t < k.do ? k : null;
}

function indeksCzasu(t) {
  const A = S.serA;
  let i = 0, j = A.t.length - 1;
  while (j - i > 1) { const m = (i + j) >> 1; if (A.t[m] < t) i = m; else j = m; }
  return Math.abs(A.t[i] - t) <= Math.abs(A.t[j] - t) ? i : j;
}

function zaznaczenie() {
  for (const p of P) {
    const g = p.g, pw = p.w - M.l - M.r;
    const fx = (t) => M.l + (t - S.od) / (S.do - S.od) * pw;
    const xa = Math.max(fx(S.zazn.a), M.l), xb = Math.min(fx(S.zazn.b), p.w - M.r);
    g.fillStyle = "rgba(74,158,255,.13)";
    g.fillRect(xa, M.t, xb - xa, p.h - M.t - M.b);
    g.strokeStyle = "#4a9eff88";
    g.beginPath(); g.moveTo(xa, M.t); g.lineTo(xa, p.h - M.b);
    g.moveTo(xb, M.t); g.lineTo(xb, p.h - M.b); g.stroke();
  }
}

// ============================================================
//  CELOWNIK — jedyna rzecz licząca się przy ruchu myszy
// ============================================================

/* Mysz potrafi zgłosić kilkaset zdarzeń na sekundę, a ekran odświeża się
 * sześćdziesiąt razy. Bez sklejenia do klatki rysowalibyśmy celownik kilka
 * razy na jedno malowanie — czyli robili robotę, której nikt nie zobaczy. */
let klatka = 0, ostatnieZdarzenie = null;
function celownikPozniej(e, p) {
  ostatnieZdarzenie = { clientX: e.clientX, clientY: e.clientY, p };
  if (klatka) return;
  klatka = requestAnimationFrame(() => {
    klatka = 0;
    const z = ostatnieZdarzenie;
    if (z) celownik(z, z.p);
  });
}

function celownik(e, p) {
  if (!S.serA || !L) return;
  const r = p.cv.getBoundingClientRect();
  const pw = p.w - M.l - M.r;
  const f = ((e.clientX - r.left) - M.l) / pw;
  if (f < 0 || f > 1) { dym.style.opacity = 0; for (const q of P) blit(q); if (S.zazn) zaznaczenie(); return; }
  const t = S.od + (S.do - S.od) * f;

  for (const q of P) blit(q);
  if (S.zazn) zaznaczenie();

  const idx = indeksCzasu(t);
  const A = S.serA, tp = A.t[idx];

  for (const q of P) {
    const g = q.g, x = M.l + (tp - S.od) / (S.do - S.od) * (q.w - M.l - M.r);
    g.strokeStyle = "#4a9eff55"; g.setLineDash([3, 3]);
    g.beginPath(); g.moveTo(x, M.t); g.lineTo(x, q.h - M.b); g.stroke(); g.setLineDash([]);
  }
  const K = L.kapital, xk = K.fx(tp);
  K && (() => {
    const g = PK.g;
    g.fillStyle = "#26d9a3"; g.beginPath(); g.arc(xk, K.fy(K.tA(A.eq[idx])), 3.3, 0, 7); g.fill();
    if (!S.procenty) { g.fillStyle = "#4a9eff"; g.beginPath(); g.arc(xk, K.fy(A.sal[idx]), 2.9, 0, 7); g.fill(); }
  })();

  // Najbliższy znacznik transakcji pod kursorem (tylko na panelu kapitału).
  // Połowienie po X + przegląd wyłącznie sąsiedztwa ±11 px: koszt nie zależy
  // od tego, czy transakcji jest 40 czy 4 000.
  let bliski = null;
  if (p === PK && K.zna.length) {
    const mx = e.clientX - r.left, my = e.clientY - r.top;
    const X = K.znaX;
    let i = 0, j = X.length;
    while (i < j) { const m = (i + j) >> 1; if (X[m] < mx - 11) i = m + 1; else j = m; }
    let best = 11 * 11;
    for (let k = i; k < X.length && X[k] <= mx + 11; k++) {
      const z = K.zna[k], dx = z.x - mx, dy = z.y - my, d = dx * dx + dy * dy;
      if (d < best) { best = d; bliski = z; }
    }
    if (bliski) {
      const g = PK.g;
      g.strokeStyle = "#e8ecf5"; g.lineWidth = 1.4;
      g.beginPath(); g.arc(bliski.x, bliski.y, 7, 0, 7); g.stroke();
    }
  }

  // kubełek pod kursorem — też połowieniem, `kubW` jest posortowane po czasie
  const kub = kubelekW(t);

  let txt;
  if (bliski) {
    const x = bliski.tr;
    txt = `TRANSAKCJA #${x.ticket}  ${bliski.typ}\n` +
      `${x.side === "Buy" ? "KUPNO " : "SPRZEDAŻ"}  ${fmt(x.volume, 2)} lota` +
      (x.basket != null ? `   koszyk ${x.basket}` : "") +
      `\notwarcie    ${pelnyU(x.open_ts)}  @ ${fmt(x.open_price, 2)}` +
      `\nzamknięcie  ${pelnyU(x.close_ts)}  @ ${fmt(x.close_price, 2)}` +
      `\ntrwała      ${trwanie(x.close_ts - x.open_ts)}` +
      `\npowód       ${x.reason}` +
      `\nwynik       ${zn(x.profit)} $` +
      (x.swap ? `   swap ${fmt(x.swap)} $` : "") +
      (x.commission ? `   prowizja ${fmt(x.commission)} $` : "");
  } else {
    txt = pelnyU(tp) + "  " + dowU(tp) +
      `\nequity      ${fmt(A.eq[idx])} $` +
      `\nsaldo       ${fmt(A.sal[idx])} $` +
      `\npływające   ${zn(A.eq[idx] - A.sal[idx])} $` +
      `\nod szczytu  ${fmt(A.dd[idx], 2)} %`;
    if (S.serB && S.serB.t.length) {
      const B = S.serB;
      let i = 0, j = B.t.length - 1;
      while (j - i > 1) { const m = (i + j) >> 1; if (B.t[m] < tp) i = m; else j = m; }
      txt += `\nporównanie  ${fmt(B.eq[i])} $`;
    }
    if (kub) {
      txt += `\n\n${NAZWA_POZ[S.poziom].toUpperCase()} ${kub.etykieta}` +
        `\notwarcie    ${fmt(kub.otwarcie)} $` +
        `\nzamknięcie  ${fmt(kub.zamkniecie)} $` +
        `\nwynik       ${zn(kub.profit)} $` +
        `\nzakres      ${fmt(kub.min)} … ${fmt(kub.max)} $` +
        `\nobsunięcie  ${fmt(kub.obsuniecie)} $  (${fmt(kub.dd_pct, 2)} % od szczytu)` +
        `\ntransakcji  ${kub.transakcje}  (wygranych ${kub.wygrane}, zrealizowano ${zn(kub.zrealizowany)} $)` +
        `\notwarto     ${kub.otwarc}` +
        (kub.delta != null
          ? `\n\nporównanie B ${zn(kub.b)} $` +
            `\nΔ A−B       ${zn(kub.delta)} $  (${fmt(kub.udzial, 0)} % rozjazdu w oknie)`
          : "") +
        (kub.sygnaly != null ? `\nsygnałów    ${kub.sygnaly}` : "") +
        `\nźródło      ${kub.zrodlo_wyniku === "silnik" ? "dni z silnika" : "różnica equity"}` +
        (kub.zrodlo_wyniku === "silnik" && Math.abs(kub.profit_krzywa - kub.profit) > 0.005
          ? `\nz krzywej   ${zn(kub.profit_krzywa)} $` : "") +
        (kub.punktow === 0 ? `\n⚠ brak próbek krzywej w tym oknie` : "") +
        (GLEBIEJ[S.poziom] ? `\n\nklik = wejdź w ${NAZWA_POZ[GLEBIEJ[S.poziom]]}` : "");
    }
  }
  dym.textContent = txt;
  const dr = dym.getBoundingClientRect();
  const owij = $("owijka").getBoundingClientRect();
  dym.style.left = Math.max(4, Math.min(e.clientX - owij.left + 16, owij.width - dr.width - 8)) + "px";
  dym.style.top = Math.max(4, e.clientY - owij.top - dr.height - 12) + "px";
  dym.style.opacity = 1;
}

const trwanie = (ms) => {
  const s = Math.round(ms / 1000);
  if (s < 60) return s + " s";
  if (s < 3600) return Math.floor(s / 60) + " min " + (s % 60) + " s";
  if (s < 86400) return Math.floor(s / 3600) + " h " + Math.floor((s % 3600) / 60) + " min";
  return Math.floor(s / 86400) + " dni " + Math.floor((s % 86400) / 3600) + " h";
};

// ============================================================
//  INTERAKCJA
// ============================================================

let ciag = null;
for (const p of P) {
  p.cv.addEventListener("wheel", (e) => {
    e.preventDefault();
    if (!S.serA) return;
    const r = p.cv.getBoundingClientRect();
    const f = Math.min(Math.max(((e.clientX - r.left) - M.l) / (r.width - M.l - M.r), 0), 1);
    const t = S.od + (S.do - S.od) * f, k = e.deltaY < 0 ? .8 : 1 / .8;
    let na = t - (t - S.od) * k, nb = t + (S.do - t) * k;
    if (nb - na < 2 * MIN) { const s = (na + nb) / 2; na = s - MIN; nb = s + MIN; }
    S.od = Math.max(na, S.t0); S.do = Math.min(nb, S.t1);
    if (S.do <= S.od) S.do = S.od + MIN;
    if (S.auto) S.poziom = poziomAuto(S.od, S.do);
    odswiezPozniej();
  }, { passive: false });

  p.cv.addEventListener("pointerdown", (e) => {
    if (!S.serA) return;
    const r = p.cv.getBoundingClientRect();
    const f = ((e.clientX - r.left) - M.l) / (r.width - M.l - M.r);
    ciag = { p, x: e.clientX, od: S.od, do: S.do, zazn: e.shiftKey, t0: S.od + (S.do - S.od) * f, ruch: false };
    // przechwycenie wskaźnika bywa odrzucane (zdarzenie syntetyczne, pióro,
    // wskaźnik już zwolniony) — to nie może przerwać obsługi kliknięcia
    try { p.cv.setPointerCapture(e.pointerId); } catch (_) { /* nieistotne */ }
    p.cv.style.cursor = e.shiftKey ? "col-resize" : "grabbing";
  });

  p.cv.addEventListener("pointerup", (e) => {
    p.cv.style.cursor = "crosshair";
    if (!ciag) return;
    const byl = ciag;
    ciag = null;
    if (byl.zazn) { uwaga(); rysuj(); return; }
    if (byl.ruch) { odswiez(); return; }
    // klik bez przeciągania — wejście w głąb
    if (p === PS && L && L.slupki) {
      const r = p.cv.getBoundingClientRect(), mx = e.clientX - r.left;
      const trafiony = L.slupki.pozycje.find((z) => mx >= z.xa && mx <= z.xb);
      if (trafiony) { wejdz(trafiony.k); return; }
    }
  });

  p.cv.addEventListener("pointerleave", () => { dym.style.opacity = 0; });
  p.cv.addEventListener("dblclick", () => { wrocDo(-1); });

  p.cv.addEventListener("pointermove", (e) => {
    if (ciag) {
      const r = ciag.p.cv.getBoundingClientRect(), pw = r.width - M.l - M.r;
      if (Math.abs(e.clientX - ciag.x) > 2) ciag.ruch = true;
      if (ciag.zazn) {
        const f = ((e.clientX - r.left) - M.l) / pw;
        const t = S.od + (S.do - S.od) * f;
        S.zazn = { a: Math.min(ciag.t0, t), b: Math.max(ciag.t0, t) };
        rysuj();
        return;
      }
      const d = (e.clientX - ciag.x) / pw * (ciag.do - ciag.od);
      let na = ciag.od - d, nb = ciag.do - d;
      if (na < S.t0) { nb += S.t0 - na; na = S.t0; }
      if (nb > S.t1) { na -= nb - S.t1; nb = S.t1; }
      S.od = Math.max(na, S.t0); S.do = Math.min(nb, S.t1);
      odswiezPozniej();
      return;
    }
    celownikPozniej(e, p);
  });
}

addEventListener("resize", () => { if (S.serA) rysuj(); });
addEventListener("keydown", (e) => {
  if (e.target.tagName === "INPUT") return;
  if (e.key === "Backspace" || e.key === "Escape") { e.preventDefault(); if (S.stos.length) wrocDo(S.stos.length - 1); }
  if (e.key === "Home") wrocDo(-1);
  const szer = S.do - S.od;
  if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
    e.preventDefault();
    const d = szer * .25 * (e.key === "ArrowLeft" ? -1 : 1);
    let na = S.od + d, nb = S.do + d;
    if (na < S.t0) { nb += S.t0 - na; na = S.t0; }
    if (nb > S.t1) { na -= nb - S.t1; nb = S.t1; }
    S.od = na; S.do = nb; odswiezPozniej();
  }
});

// ============================================================
//  TABELA
// ============================================================

function kolumny() {
  const k = [
    ["etykieta", JEDEN_POZ[S.poziom]],
    ["dow", "dzień"],
    ["otwarcie", "equity otw."],
    ["zamkniecie", "equity zamk."],
    ["profit", "wynik $"],
    ["pr", "wynik %"],
    ["min", "dno"],
    ["obsuniecie", "obsunięcie $"],
    ["dd_pct", "od szczytu %"],
    ["transakcje", "trans."],
    ["wygrane", "wygr."],
    ["zrealizowany", "zrealizowany $"],
    ["otwarc", "otwarto"],
  ];
  if (S.kubB.length) k.push(["b", "B wynik $"], ["delta", "Δ A−B $"], ["udzial", "udział w rozjeździe %"]);
  if (CALE_DOBY[S.poziom]) k.push(["profit_krzywa", "z krzywej $"], ["sygnaly", "sygnałów"]);
  k.push(["punktow", "próbek"]);
  return k;
}

function tabela() {
  const kol = kolumny();
  $("glowa").innerHTML = kol.map(([k, n]) => `<th data-k="${k}">${n}</th>`).join("");
  for (const th of document.querySelectorAll("#glowa th")) {
    th.onclick = () => { const k = th.dataset.k; S.sortD = (S.sortK === k) ? -S.sortD : 1; S.sortK = k; tabela(); };
  }
  let w = S.kubW.map((k) => ({
    ...k,
    pr: k.otwarcie ? k.profit / k.otwarcie * 100 : 0,
    dow: dowU(k.t),
  }));
  if (S.sortK) w.sort((a, b) => {
    const x = a[S.sortK], y = b[S.sortK];
    if (x == null) return 1; if (y == null) return -1;
    return (x > y ? 1 : x < y ? -1 : 0) * S.sortD;
  });
  const kl = (v) => v > 0 ? "z" : v < 0 ? "c" : "g";
  const tb = document.querySelector("#t tbody");
  tb.innerHTML = "";
  const glebiej = GLEBIEJ[S.poziom];
  const frag = document.createDocumentFragment();
  for (const d of w) {
    const tr = document.createElement("tr");
    tr.innerHTML = kol.map(([k]) => {
      const v = d[k];
      if (k === "etykieta") return `<td>${v}</td>`;
      if (k === "dow") return `<td class="g">${v}</td>`;
      if (v == null) return `<td class="g">—</td>`;
      if (k === "profit" || k === "profit_krzywa" || k === "zrealizowany" || k === "b" || k === "delta")
        return `<td class="${kl(v)}">${zn(v)}</td>`;
      if (k === "udzial") return `<td class="${v > 20 ? "" : "g"}">${fmt(v, 1)}</td>`;
      if (k === "pr") return `<td class="${kl(v)}">${zn(v, 1)}%</td>`;
      if (k === "dd_pct") return `<td class="${v < 0 ? "c" : "g"}">${fmt(v, 2)}</td>`;
      if (k === "obsuniecie") return `<td class="${v > 0 ? "c" : "g"}">${fmt(v)}</td>`;
      if (["transakcje", "wygrane", "otwarc", "sygnaly", "punktow"].includes(k))
        return `<td class="${v ? "" : "g"}">${v}</td>`;
      return `<td>${fmt(v)}</td>`;
    }).join("");
    tr.onclick = () => {
      S.wybranyKub = d.t;
      for (const x of tb.children) x.classList.remove("pod");
      tr.classList.add("pod");
      if (glebiej) wejdz(d); else rysuj();
    };
    frag.appendChild(tr);
  }
  tb.appendChild(frag);
  $("tytulTabeli").textContent =
    `TABELA — ${NAZWA_POZ[S.poziom]} (${w.length})` +
    (glebiej ? ` · klik w wiersz wchodzi w ${NAZWA_POZ[glebiej]}` : "") +
    " · klik w nagłówek sortuje";
  $("tytulSlupki").textContent =
    `WYNIK OKRESU — jeden prostokąt to ${JEDEN_POZ[S.poziom]}` +
    (CALE_DOBY[S.poziom] ? " · wynik z DNI POLICZONYCH PRZEZ SILNIK" : " · wynik z różnicy equity na krzywej") +
    (glebiej ? " · klik wchodzi głębiej" : " · to jest maksymalna precyzja");
}

// ============================================================
//  CSV
// ============================================================

function csv(co) {
  if (!S.idA) return;
  const a = Math.round(S.zazn ? S.zazn.a : S.od);
  const b = Math.round(S.zazn ? S.zazn.b : S.do);
  const u = `${API}/p/${S.idA}/csv?co=${co}&poziom=${S.poziom}&od=${a}&do=${b}`;
  const el = document.createElement("a");
  el.href = u; el.download = ""; document.body.appendChild(el); el.click(); el.remove();
}

// ============================================================
//  PRZYCISKI
// ============================================================

const przel = (id, pole, poFiltrze) => {
  $(id).onclick = () => {
    S[pole] = !S[pole];
    $(id).classList.toggle("on", S[pole]);
    if (poFiltrze) { filtrujKubelki(); kafle(); tabela(); }
    rysuj(); legenda(); uwaga();
  };
};
przel("log", "log", false);
przel("weekend", "bezWeek", true);
przel("puste", "bezPustych", true);
przel("znaczniki", "znaczniki", false);
przel("procenty", "procenty", false);
przel("roznica", "roznica", false);
$("roznica").disabled = true;
$("reset").onclick = () => wrocDo(-1);
$("wyzej").onclick = () => { if (S.stos.length) wrocDo(S.stos.length - 1); };
$("csvKrzywa").onclick = () => csv("krzywa");
$("csvSlupki").onclick = () => csv("slupki");
$("csvTrans").onclick = () => csv("transakcje");
$("bocznyPrzel").onclick = () => { $("boczny").classList.toggle("zwiniety"); setTimeout(rysuj, 30); };
$("filtr").oninput = (e) => { S.filtr = e.target.value; rysujListe(); };
$("dodajKatalog").onclick = async () => {
  const s = $("nowyKatalog").value.trim();
  if (!s) return;
  const r = await pobierz(`${API}/dodaj?sciezka=${encodeURIComponent(s)}`);
  $("skanInfo").textContent = r.error ? r.error : `dodano ${r.dodano}, razem ${r.razem}`;
  if (!r.error) { $("nowyKatalog").value = ""; await wczytajListe(); }
};

// ============================================================
//  START
// ============================================================

(async () => {
  const i = await pobierz(API + "/info");
  $("wersja").textContent = `v${i.wersja} · ${i.liczba} przebiegów`;
  przyciskiPoziomow();
  await wczytajListe();
})();
