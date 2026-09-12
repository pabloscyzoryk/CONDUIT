import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync, readdirSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import ts from 'typescript';

const root = resolve(import.meta.dirname, '../src');
const modules = new Map(), saved = [], patches = [];
const react = {
  createContext: () => ({}), useCallback: f => f, useContext: () => null,
  useEffect: () => {}, useSyncExternalStore: (_, get) => get(),
};
const jsx = { jsx: (type, props) => ({ type, props }), jsxs: (type, props) => ({ type, props }) };
function load(name, parent = root + '/index.ts') {
  if (name === 'react') return react;
  if (name === 'react/jsx-runtime') return jsx;
  if (name === '@/store/storage') return { load: (_, value) => value, save: (...args) => saved.push(args) };
  if (name === '@/store/transport') return { api: { patchSettings: async value => patches.push(value) } };
  let file = name.startsWith('@/') ? resolve(root, name.slice(2)) : resolve(dirname(parent), name);
  if (!existsSync(file) || !/\.tsx?$/.test(file)) file = [file + '.ts', file + '.tsx', file + '/index.tsx'].find(existsSync);
  assert.ok(file, name);
  file = resolve(file); // Windows aliases and relative imports share one module.
  if (modules.has(file)) return modules.get(file);
  const exports = {}; modules.set(file, exports);
  const source = readFileSync(file, 'utf8');
  const compiled = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX } }).outputText;
  new Function('exports', 'require', compiled)(exports, child => load(child, file));
  return exports;
}
const i18n = load('@/i18n');
const { EN } = load('@/i18n/en'), { PL } = load('@/i18n/pl');
const { SETTINGS_SCHEMA } = load('@/data/settingsSchema');
const { SCHEMA_EN } = load('@/i18n/settingsSchema.en');
const { przetlumaczGrupy } = load('@/i18n/schema');
const { ENGINE_TEMPLATES } = load('@/i18n/engineTemplates');
const { presentEngineText } = load('@/i18n/enginePresentation');
const { tSilnik } = load('@/i18n/silnik');

test('lot growth and causal context holds preserve exact reason codes in PL/EN, observations are not rejections', () => {
  const sources = ['../rust/crates/core/src/engine.rs', '../rust/crates/core/src/engine/lot_context_execution.rs']
    .map(p => readFileSync(resolve(root, p), 'utf8')).join('\n');
  for (const title of ['LOT GROWTH HOLD', 'LOT CONTEXT HOLD']) {
    assert.ok(sources.includes(`${title}: {reason:?}; new order withheld`));
    const reason = 'MissingOrInvalidInput { axis: 4, field: "stop_distance" }';
    const raw = `${title}: ${reason}; new order withheld`;
    i18n.setLanguage('pl'); const pl = tSilnik(raw);
    assert.notEqual(pl, raw); assert.ok(pl.includes(reason)); assert.ok(pl.endsWith('nowe zlecenie wstrzymane'));
    i18n.setLanguage('en'); assert.equal(tSilnik(pl), raw);
  }
  const observation = 'LOT CONTEXT: causal pre-send sizing observation';
  assert.ok(sources.includes(observation));
  i18n.setLanguage('pl'); const pl = tSilnik(observation);
  assert.equal(pl, 'KONTEKST LOTA: obserwacja wielkości pozycji przed wysłaniem zlecenia');
  assert.doesNotMatch(pl, /BLOKADA|odrzucone|wstrzymane/);
  i18n.setLanguage('en'); assert.equal(tSilnik(pl), observation);
});

test('equity floor wording allows old pending fills and conditional recovery; percent lot is not SL risk', () => {
  const source = readFileSync(resolve(root, '../rust/crates/core/src/settings.rs'), 'utf8');
  const literal = source.match(/"(`equity_floor_pct = \{\}`[^"\n]*(?:\\\r?\n[^"\n]*)*)"/);
  assert.ok(literal, 'read the actual production settings warning');
  const polish = literal[1].replace(/\\\r?\n\s*/g, '').replace('{}', '35');
  i18n.setLanguage('en');
  const english = tSilnik(polish);
  assert.ok(english.startsWith('`equity_floor_pct = 35` blocks new entries checked by the account entry gate'));
  for (const phrase of ['equal to or below', 'orders can still fill', 'may resume automatically', 'other rules', 'not a maximum loss limit'])
    assert.ok(english.includes(phrase), phrase);
  i18n.setLanguage('pl'); assert.equal(tSilnik(english), polish);
  for (const language of ['pl', 'en']) {
    i18n.setLanguage(language);
    const fields = przetlumaczGrupy(SETTINGS_SCHEMA, language).flatMap(g => g.fields);
    const floor = fields.find(f => f.key === 'equity_floor_pct');
    assert.ok(floor);
    const hint = floor.hint;
    assert.equal(typeof hint, 'string');
    assert.ok(hint.includes(language === 'pl' ? 'zlecenia te nadal mogą się wykonać' : 'orders can still fill'));
    assert.ok(hint.includes(language === 'pl' ? 'mogą wznowić się samoczynnie' : 'may resume automatically'));
    assert.ok(hint.includes(language === 'pl' ? 'inne reguły' : 'other rules'));
    assert.ok(hint.includes('0 ='));
    const lot = i18n.t('lot.pct.hint');
    assert.ok(lot.includes(language === 'pl' ? 'nie ryzyko straty do SL' : 'not the risk of loss at SL'));
    for (const value of ['1%', '0.01', '$100', '0.10', '$1000']) assert.ok(lot.includes(value));
  }
});

test('mode namespaces and passive source quarantine retain counts, history and account scope in PL/EN', () => {
  const live = readFileSync(resolve(root, '../rust/crates/app/src/live.rs'), 'utf8');
  for (const literal of ["T-100 source context", "contexts quarantined after an observation gap; pending history retained, new confirmed signals remain usable", "events; affected old contexts require a new complete source version", "Cannot restore the account's active strategy mode:"])
    assert.ok(live.includes(literal), 'review actual production diagnostic: ' + literal);
  const pairs = [
    ['Kontekst źródła T-100', 'T-100 source context'],
    ['T-100: liczba kontekstów w kwarantannie po luce w obserwacji: 0; historia oczekujących zdarzeń zachowana, nowe potwierdzone sygnały nadal mogą być używane', 'T-100: 0 contexts quarantined after an observation gap; pending history retained, new confirmed signals remain usable'],
    ['T-100: bierna kolejka kontekstu utraciła 8193 zdarzeń; dotknięte luką stare konteksty wymagają nowej pełnej wersji źródła', 'T-100: passive context queue lost 8193 events; affected old contexts require a new complete source version'],
    ['Nie można przywrócić aktywnego trybu strategii rachunku: SYNTHETIC_STORE_FAILURE_7', "Cannot restore the account's active strategy mode: SYNTHETIC_STORE_FAILURE_7"],
  ];
  for (const [pl, en] of pairs) {
    i18n.setLanguage('pl'); assert.equal(tSilnik(en), pl);
    i18n.setLanguage('en'); assert.equal(tSilnik(pl), en);
  }
  const policy = readFileSync(resolve(root, '../rust/crates/core/src/t100/policy.rs'), 'utf8');
  for (const reason of ['invalid_account_day_anchors', 'stale_account_day_anchors']) {
    assert.ok(policy.includes('"' + reason + '"'), reason);
    const pl = presentEngineText(reason, 'pl');
    assert.notEqual(pl, reason); assert.ok(pl.includes(reason), 'machine code remains identifiable');
    assert.equal(presentEngineText(pl, 'en'), reason);
    const wrapped = `T-100 REVIEW: ${reason}; new exposure blocked, protection remains active`;
    const translated = presentEngineText(wrapped, 'pl');
    assert.ok(translated.includes(pl)); assert.equal(presentEngineText(translated, 'en'), wrapped);
  }
});

test('native terminal discovery diagnostics all have reversible presentation translations', () => {
  // Read production literals without importing Python or inspecting any terminal.
  const source = readFileSync(resolve(root, '../rust/crates/mt5/sidecar/terminal_discovery.py'), 'utf8');
  const diagnostics = [...new Set([...source.matchAll(/raise TerminalDiscoveryError\('([^']+)'\)/g)].map(match => match[1]))];
  assert.equal(diagnostics.length, 11, 'review added or removed native discovery failure modes');
  for (const raw of diagnostics) {
    const english = presentEngineText(raw, 'en');
    assert.notEqual(english, raw, raw);
    assert.ok(!/[ąćęłńóśźż]/i.test(english), english);
    assert.equal(presentEngineText(english, 'pl'), raw);
    i18n.setLanguage('en');
    assert.equal(tSilnik(raw), english);
    i18n.setLanguage('pl');
    assert.equal(tSilnik(english), raw);
  }
  assert.equal(presentEngineText('MT5 nie jest uruchomiony w tej sesji Windows; połączenie zostanie ponowione', 'en'),
    'MT5 is not running in this Windows session; the connection will be retried');
  assert.equal(presentEngineText('działa więcej niż jeden terminal MT5; wskaż jednoznaczną ścieżkę albo zamknij dodatkową instancję', 'en'),
    'more than one MT5 terminal is running; specify an unambiguous path or close the extra instance');
});

test('MT5 retry notification translates the actual nested startup cause while retaining stage and code', () => {
  const pl = 'Próba 7 nieudana: MT5: start nieudany [terminal_discovery], kod -10001: '
    + 'nie można odczytać ścieżki terminala w tej sesji Windows; sprawdź uprawnienia MT5 i bota'
    + '\n\nNajczęstsze przyczyny:\n'
    + '• terminal MetaTrader 5 nie jest uruchomiony albo nie jest zalogowany,\n'
    + '• w Pythonie brakuje pakietu MetaTrader5 (pip install MetaTrader5),\n'
    + '• pole „interpreter Pythona” wskazuje inny Python niż ten z pakietem,\n'
    + '• w terminalu wyłączony jest handel algorytmiczny.';
  const en = 'Attempt 7 failed: MT5: startup failed [terminal_discovery], code -10001: '
    + 'cannot read the terminal path in this Windows session; check MT5 and bot permissions'
    + '\n\nCommon causes:\n'
    + '• the MetaTrader 5 terminal is not running or is not logged in,\n'
    + '• Python is missing the MetaTrader5 package (pip install MetaTrader5),\n'
    + '• the Python interpreter setting points to a different Python installation than the one containing the package,\n'
    + '• algorithmic trading is disabled in the terminal.';
  assert.equal(presentEngineText(pl, 'en'), en);
  assert.equal(presentEngineText(en, 'pl'), pl);
  i18n.setLanguage('en'); assert.equal(tSilnik(pl), en);
  i18n.setLanguage('pl'); assert.equal(tSilnik(en), pl);
});

test('startup presentation preserves vendor details, paths, symbols, exception types and protection requirements', () => {
  const vendor = String.raw`IPC initialize failed (os error 5); C:\Synthetic Terminal\terminal64.exe`;
  for (const [pl, en] of [
    [`MT5: start nieudany [initialize], kod -10001: initialize() nieudane: -10005 ${vendor}`,
      `MT5: startup failed [initialize], code -10001: initialize() failed: -10005 ${vendor}`],
    [`Próba 2 nieudana: MT5: start nieudany [python_spawn], kod -10001: nie udało się uruchomić procesu Pythona: ${vendor}`,
      `Attempt 2 failed: MT5: startup failed [python_spawn], code -10001: could not start the Python process: ${vendor}`],
    ['MT5: start nieudany [protocol], kod -10001: wersja protokołu sidecara 3 zamiast 4',
      'MT5: startup failed [protocol], code -10001: sidecar protocol version 3 instead of 4'],
    ['MT5: start nieudany [symbol], kod -10001: symbol XAUUSD.s niedostępny w Podglądzie rynku',
      'MT5: startup failed [symbol], code -10001: symbol XAUUSD.s is unavailable in Market Watch'],
    ['BŁĄD STARTU [history_seed]: błąd inicjalizacji sidecara: OSError',
      'STARTUP ERROR [history_seed]: sidecar initialization error: OSError'],
    ['follow: terminal jest na rachunku REAL/CONTEST; handel zablokowany. DEMO jest domyślne; REAL wymaga jawnego mt5_allow_real_account=true',
      'follow: the terminal is on a REAL/CONTEST account; trading is blocked. DEMO is the default; REAL requires explicit mt5_allow_real_account=true'],
    ['nie udało się pobrać parametrów XAUUSD.s: terminal nie ma potwierdzonego zalogowanego konta',
      'could not retrieve parameters for XAUUSD.s: the terminal has no confirmed logged-in account'],
  ]) {
    assert.equal(presentEngineText(pl, 'en'), en);
    assert.equal(presentEngineText(en, 'pl'), pl);
  }
  const unknown = 'MT5_VENDOR_FACT "XAUUSD.s" ticket=9007199254740993 code=-10005';
  assert.equal(presentEngineText(`MT5: start nieudany [vendor_stage], kod -10005: ${unknown}`, 'en'),
    `MT5: startup failed [vendor_stage], code -10005: ${unknown}`);
  const body = 'MT5 nie jest uruchomiony w tej sesji Windows; połączenie zostanie ponowione';
  assert.equal(presentEngineText(`Sygnał odrzucony\nTREŚĆ:\n${body}`, 'en'),
    `Signal rejected\nSOURCE MESSAGE:\n${body}`, 'source messages must not be translated as system diagnostics');
});

test('early Python exit keeps the bounded stderr traceback while translating the known package warning', () => {
  const traceback = String.raw` | Traceback (most recent call last): | File "C:\Synthetic Runtime\mt5_sidecar.py", line 69 | ModuleNotFoundError: No module named 'MetaTrader5'`;
  const pl = 'Próba 3 nieudana: MT5: start nieudany [python_exit], kod 1: '
    + 'proces Pythona zakończył się przed połączeniem z botem; diagnostyka: '
    + 'BRAK PAKIETU MetaTrader5. Zainstaluj: pip install MetaTrader5 | '
    + 'Uwaga: pakiet jest 64-bitowy i TYLKO pod Windows.' + traceback;
  const en = 'Attempt 3 failed: MT5: startup failed [python_exit], code 1: '
    + 'the Python process exited before connecting to the bot; diagnostics: '
    + 'MetaTrader5 PACKAGE MISSING. Install: pip install MetaTrader5 | '
    + 'Note: the package is 64-bit and Windows-only.' + traceback;
  assert.equal(presentEngineText(pl, 'en'), en);
  assert.equal(presentEngineText(en, 'pl'), pl);
  for (const language of ['en', 'pl']) {
    i18n.setLanguage(language);
    assert.equal(tSilnik(pl), language === 'en' ? en : pl);
  }
  const missingStderr = 'MT5: start nieudany [python_exit], kod 1: proces Pythona zakończył się przed połączeniem z botem; diagnostyka: ';
  assert.equal(presentEngineText(missingStderr, 'en'),
    'MT5: startup failed [python_exit], code 1: the Python process exited before connecting to the bot; diagnostics: ',
    'an empty stderr does not invent an import failure');
});

test('startup transport failure literals are covered in the shared presentation catalog', () => {
  const source = readFileSync(resolve(root, '../rust/crates/mt5/src/transport.rs'), 'utf8').split('#[cfg(test)]')[0];
  const messages = [...source.matchAll(/msg:\s*(?:format!\()?"([^"\n]+)"/g)].map(match => match[1]);
  assert.ok(messages.length >= 8, 'exercise production startup failure details');
  for (const message of messages) {
    const raw = message.replace(/\{[^{}]*\}/g, '7301');
    const english = presentEngineText(raw, 'en');
    assert.notEqual(english, raw, raw);
    assert.equal(presentEngineText(english, 'pl'), raw);
  }
});

test('terminal history category and progress change PL/EN without raw keys', () => {
  i18n.setLanguage('en');
  assert.equal(i18n.t('logs.group.broker'), 'Broker account');
  assert.equal(i18n.t('logs.source.broker_history.label'), 'terminal account history');
  assert.equal(tSilnik('Historia rachunku z terminala'), 'Terminal account history');
  i18n.setLanguage('pl');
  assert.equal(i18n.t('logs.group.broker'), 'Rachunek brokera');
  assert.equal(i18n.t('logs.source.broker_history.label'), 'historia rachunku z terminala');
  assert.equal(tSilnik('Terminal account history'), 'Historia rachunku z terminala');
});

test('execution confirmation messages distinguish temporary waiting from review in PL/EN', () => {
  for (const [pl, en] of [
    ['Oczekiwanie na potwierdzenie wykonania', 'Waiting for execution confirmation'],
    ['Potwierdzenia są w trakcie uzgadniania; nowe wejścia czekają.', 'Execution confirmations are being reconciled; new entries are waiting.'],
    ['Potwierdzenia wymagają sprawdzenia; nowe wejścia pozostają zablokowane.', 'Execution confirmations require review; new entries remain blocked.'],
    ['Kontrolna odbudowa połączenia po ciszy kwotowań', 'Connection recovery check after quote silence'],
    ['Terminal potwierdził brak połączenia z brokerem — odbudowuję połączenie.', 'The terminal confirmed that the broker connection is down — reconnecting.'],
    ['Kontrola połączenia sidecara nie powiodła się: timeout 10060', 'The sidecar connection check failed: timeout 10060'],
    ['Brak rozstrzygającego potwierdzenia operacji; wymagane uzgodnienie stanu.', 'No conclusive operation confirmation; state reconciliation is required.'],
    ['Nieudane operacje handlowe: 3', 'Unsuccessful trading operations: 3'],
    ['operacja nie uzyskała potwierdzenia wykonania', 'the operation has no execution confirmation'],
  ]) {
    assert.equal(presentEngineText(pl, 'en'), en);
    assert.equal(presentEngineText(en, 'pl'), pl);
  }
});

test('unknown broker outcomes preserve command, retcode and transport detail in both languages', () => {
  const source = readFileSync(resolve(root, '../rust/crates/mt5/src/bridge.rs'), 'utf8').split('#[cfg(test)]')[0];
  const literals = [...source.matchAll(/receipt_fault\(format!\("(nieznany wynik[^"\n]+)"/g)].map(m => m[1]);
  assert.equal(literals.length, 3, 'cover all current production unknown-outcome variants');
  for (const command of ['place_pending', 'close_position', 'close_partial', 'cancel_pending']) {
    for (const [suffix, english] of [
      ['retcode=10011; wymagane potwierdzenie wykonania przed nowymi wejściami',
        'retcode=10011; execution confirmation required before new entries'],
      ['timeout po wysłaniu; odczyt stanu nie zastępuje potwierdzenia wykonania',
        'timeout after dispatch; a state read does not replace execution confirmation'],
      ['bridge disconnected: EOF (10031); wymagane potwierdzenie wykonania przed nowymi wejściami',
        'bridge disconnected: EOF (10031); execution confirmation required before new entries'],
    ]) {
      const pl = `nieznany wynik ${command}: ${suffix}`;
      const en = `unknown outcome of ${command}: ${english}`;
      assert.equal(presentEngineText(pl, 'en'), en);
      assert.equal(presentEngineText(en, 'pl'), pl);
    }
  }
  for (const literal of literals) {
    const raw = literal.replace('{cmd}', 'close_partial').replace('{}', '10012').replace('{e}', 'EOF');
    const english = presentEngineText(raw, 'en');
    assert.ok(english.startsWith('unknown outcome of close_partial: '), raw);
    assert.equal(presentEngineText(english, 'pl'), raw);
  }
});

test('PL ↔ EN changes every dictionary entry without exposing a raw key', () => {
  for (const language of ['pl', 'en', 'pl', 'en']) {
    i18n.setLanguage(language);
    for (const [key, value] of Object.entries(language === 'en' ? EN : PL)) {
      assert.ok(value.trim(), key);
      assert.equal(i18n.t(key), value, key);
      assert.notEqual(i18n.t(key), key, key);
    }
  }
  assert.ok(saved.some(([, lang]) => lang === 'pl'));
  assert.ok(patches.some(value => value.language === 'en'));
  const count = patches.length;
  i18n.applyServerLanguage('pl');
  assert.equal(patches.length, count, 'server language update must not send a write back');
  i18n.cycleLanguage(); assert.equal(i18n.getLanguage(), 'en');
});

test('every schema label, hint, enum and warning has an English presentation with unchanged machine values', () => {
  const translated = przetlumaczGrupy(SETTINGS_SCHEMA, 'en');
  for (let i = 0; i < SETTINGS_SCHEMA.length; i++) {
    const group = SETTINGS_SCHEMA[i], english = SCHEMA_EN[group.id];
    assert.ok(english?.title, group.id);
    if (group.desc) assert.notEqual(english.desc, undefined, group.id + '.desc');
    for (let j = 0; j < group.fields.length; j++) {
      const field = group.fields[j], layer = english.fields?.[field.key];
      assert.ok(layer?.label, field.key);
      if (field.hint) assert.notEqual(layer.hint, undefined, field.key + '.hint');
      if (field.warn) assert.equal(typeof layer.warn, 'function', field.key + '.warn');
      for (const option of field.options ?? []) {
        // ISO currency codes and their native symbols are language-neutral.
        if (field.key === 'display_currency' && !['OFF', 'MT5', 'PLN'].includes(option.value)) continue;
        assert.ok(layer.options?.[option.value], `${field.key}:${option.value}`);
      }
      assert.equal(translated[i].fields[j].key, field.key);
      assert.ok(!['pkt', 'dni'].includes(translated[i].fields[j].unit), field.key + '.unit');
      assert.deepEqual(translated[i].fields[j].options?.map(o => o.value), field.options?.map(o => o.value));
    }
  }
});

test('visible static JSX copy and accessibility labels do not regress to untranslated Polish', () => {
  const files = directory => readdirSync(directory, { withFileTypes: true }).flatMap(entry =>
    entry.isDirectory() ? files(resolve(directory, entry.name)) : entry.name.endsWith('.tsx') ? [resolve(directory, entry.name)] : []);
  const polish = /[ąćęłńóśźż]|\b(?:Anuluj|Wykresy|ulubiony|nowy poziom|instrument bota|Reset widoku|Szukaj instrumentu|Wybierz ten katalog|pusto)\b/i;
  for (const file of files(root)) {
    const tree = ts.createSourceFile(file, readFileSync(file, 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
    const visit = node => {
      if (ts.isJsxText(node)) assert.ok(!polish.test(node.text), `${file}: ${node.text.trim()}`);
      if (ts.isJsxAttribute(node) && ['title', 'placeholder', 'aria-label'].includes(node.name.text) && node.initializer && ts.isStringLiteral(node.initializer))
        assert.ok(!polish.test(node.initializer.text), `${file}: ${node.initializer.text}`);
      ts.forEachChild(node, visit);
    };
    visit(tree);
  }
});

test('engine translations preserve variable values, symbols and account identifiers', () => {
  const shared = [...ENGINE_TEMPLATES, ...Object.keys(EN).filter(key => key.startsWith('eng.')).map(key => [PL[key], EN[key]])];
  assert.deepEqual(JSON.parse(readFileSync(resolve(root, '../rust/crates/server/src/engine_translations.json'), 'utf8')), shared, 'email and UI must use the same generated dictionary');
  const interpolate = text => text.replace(/\{([^{}]*)\}/g, () => '7301');
  const mismatches = ENGINE_TEMPLATES.map(([source, english]) => ({ source, actual: presentEngineText(interpolate(source)), expected: interpolate(english) })).filter(row => row.actual !== row.expected);
  assert.deepEqual(mismatches, []);
  assert.equal(presentEngineText('Preset MY-PRESET 42 przeładowany w locie'), 'Preset MY-PRESET 42 reloaded during operation');
  assert.equal(presentEngineText('Sygnał ODROCZONY — nie wykonany'), 'DEFERRED signal — not executed');
  assert.equal(presentEngineText('koszyk B15: broker odrzucił zmianę SL — INVALID_STOPS'), 'basket B15: broker rejected the SL change — INVALID_STOPS');
  assert.equal(presentEngineText('UNKNOWN_BROKER_FACT <value>'), 'UNKNOWN_BROKER_FACT <value>');
  assert.equal(presentEngineText('BUY GOLD 2400\nSL 2390\nTP 2420'), 'BUY GOLD 2400\nSL 2390\nTP 2420');
  assert.equal(presentEngineText('Sygnał odrzucony\nTREŚĆ:\nSygnał odrzucony'), 'Signal rejected\nSOURCE MESSAGE:\nSygnał odrzucony');
});

test('existing dynamic notifications also follow the current display language', () => {
  i18n.setLanguage('en');
  const message = i18n.t('toast.basketCreated', { id: 27 });
  i18n.setLanguage('pl');
  assert.equal(load('./index', resolve(root, 'i18n/silnik.ts')), i18n);
  assert.equal(i18n.getLanguage(), 'pl');
  assert.equal(presentEngineText(message, 'pl'), i18n.t('toast.basketCreated', { id: 27 }));
  assert.equal(tSilnik(message), i18n.t('toast.basketCreated', { id: 27 }));
  i18n.setLanguage('en');
  assert.equal(tSilnik('Brak połączenia z MT5'), 'No connection to MT5');
  i18n.setLanguage('pl');
  assert.equal(tSilnik('No connection to MT5'), 'Brak połączenia z MT5');
  assert.equal(tSilnik('MT5: local order sequence unavailable; tied legacy fills retain observed order'),
    'MT5: kolejność lokalnych zleceń niedostępna; remisy starych filli zachowują kolejność obserwacji');
  assert.equal(tSilnik('FAST ADDON: TP 3994.00 is beyond the market; no order sent and capacity remains available'),
    'DOKŁADKA TEMPOWA: TP 3994.00 jest już za rynkiem; zlecenie nie zostało wysłane, slot pozostaje dostępny');
  assert.equal(tSilnik('ENTRY EDIT: first complete protected source evaluated at receive time'),
    'EDYCJA WEJŚCIA: pierwszy pełny chroniony sygnał oceniony w chwili odbioru');
  assert.equal(tSilnik('ENTRY EDIT: source 7301 has no basket; recovery requires the preset policy and a complete protected entry'),
    'EDYCJA WEJŚCIA: źródło 7301 nie ma koszyka; przyjęcie wymaga zgody presetu i pełnego chronionego wejścia');
  assert.equal(tSilnik('ENTRY SOURCE: publisher withdrawal prevents reactivation'),
    'ŹRÓDŁO WEJŚCIA: anulowanie przez nadawcę blokuje ponowną aktywację');
  assert.equal(tSilnik('ENTRY SOURCE: late NEW cannot replace an already received entry edit'),
    'ŹRÓDŁO WEJŚCIA: spóźniony NEW nie może zastąpić wcześniej odebranej edycji wejścia');
  assert.equal(tSilnik('ENTRY SOURCE: bound CANCEL saved before its source entry; no unrelated basket selected'),
    'ŹRÓDŁO WEJŚCIA: powiązany CANCEL zapisany przed jego sygnałem wejścia; nie wybrano obcego koszyka');
  assert.equal(tSilnik('RISK BUDGET: MissingStop; new order withheld'),
    'BUDŻET RYZYKA: MissingStop; nowe zlecenie wstrzymane');
  assert.equal(tSilnik('RISK BUDGET: manual order rejected; maximum allowed volume: 0.07000000; requested volume is unchanged'),
    'BUDŻET RYZYKA: zlecenie ręczne odrzucone; maksymalny dozwolony wolumen: 0.07000000; żądany wolumen pozostaje bez zmian');
  assert.ok(!presentEngineText('RISK FREE @ 2400 · closed 2 positions (5 $, entry VWAP 2400) · runners: 1, SL 2400', 'pl').includes('{'));
});

test('strategy summaries retain their accounting scope and signed values in both languages', () => {
  const cases = [
    ['Podsumowanie strategii: -123.45 $ dzisiaj', 'Strategy summary: -123.45 $ today'],
    ['Dzisiaj · wynik strategii +0.00 $ · obsunięcie dnia 12.30 $ · transakcji 7', 'Today · strategy result +0.00 $ · daily drawdown 12.30 $ · trades 7'],
    ['Skuteczność strategii od startu · 3 z 7 (43 %) · profit factor 0.75', 'Strategy win rate since startup · 3 of 7 (43 %) · profit factor 0.75'],
  ];
  for (const [polish, english] of cases) {
    i18n.setLanguage('en');
    assert.equal(tSilnik(polish), english);
    i18n.setLanguage('pl');
    assert.equal(tSilnik(english), polish);
  }
  assert.equal(i18n.t('aim.feat.bk_realized'), 'Zrealizowany wynik strategii koszyka');
  i18n.setLanguage('en');
  assert.equal(i18n.t('aim.feat.bk_realized'), 'Basket strategy realised result');
  // Legacy logs remain translatable without relabelling their source facts.
  assert.equal(tSilnik('Podsumowanie: +10.00 $ dzisiaj'), 'Summary: +10.00 $ today');
});

test('strategy accounting holds retain the concrete verification reason and diagnostic enum', () => {
  const cases = [
    ['WYNIK STRATEGII HOLD: wymagane potwierdzone XAUUSD, rachunek w USD i kontrakt 100 jednostek', 'STRATEGY P/L HOLD: verified XAUUSD, USD account and 100-unit contract required'],
    ['WYNIK STRATEGII HOLD: wymagane potwierdzone parametry wejścia i wyjścia oraz przypisany swap', 'STRATEGY P/L HOLD: confirmed entry/exit geometry and allocated swap required'],
    ['WYNIK STRATEGII HOLD: zapisany wynik zarządzania ma niezweryfikowaną podstawę', 'STRATEGY P/L HOLD: saved management result has an unverified basis'],
    ...['MissingGeometry', 'InvalidValue', 'CanonicalReceipt'].map(reason => [
      `WYNIK STRATEGII: niezweryfikowana zamknięta transza (${reason})`,
      `STRATEGY P/L: unverified closed tranche (${reason})`,
    ]),
  ];
  for (const [polish, english] of cases) {
    i18n.setLanguage('pl'); assert.equal(tSilnik(english), polish);
    i18n.setLanguage('en'); assert.equal(tSilnik(polish), english);
  }
});

test('RichT preserves dictionary markup but escapes untrusted interpolation', () => {
  i18n.setLanguage('en');
  const rendered = i18n.RichT({ k: 'path.newName' });
  assert.equal(rendered.props.dangerouslySetInnerHTML.__html, EN['path.newName']);
  const injected = i18n.RichT({ k: 'wiz.models', vars: { n: '<img src=x onerror=alert(1)>', m: 'A&B' } });
  const html = injected.props.dangerouslySetInnerHTML.__html;
  assert.ok(!html.includes('<img'));
  assert.ok(html.includes('&lt;img'));
  assert.ok(html.includes('A&amp;B'));
});


test('entry source telemetry preserves unknown historical counts and is distinct from closed trades', () => {
  const { entrySourceCounts, optionalCount } = load('./lib/labMetrics', resolve(root, 'index.ts'));
  for (const missing of [undefined, null, NaN, Infinity, -1]) assert.equal(optionalCount(missing), '—');
  assert.equal(optionalCount(0), '0');
  const rows = entrySourceCounts({ knownEntrySources: 7, knownFullEntrySources: 4, entrySourcesFirstSeenAsEdit: 3, trades: 31 });
  assert.deepEqual(rows.map(row => row.value), [7, 4, 3]);
  assert.ok(entrySourceCounts({}).every(row => optionalCount(row.value) === '—'));
  for (const language of ['en', 'pl']) {
    i18n.setLanguage(language);
    for (const row of rows) assert.notEqual(i18n.t(row.label), row.label);
  }
});


test('T100 review, confirmed entry and M1 wait retain exact reasons and identifiers in both languages', () => {
  const source = readFileSync(resolve(root, '../rust/crates/core/src/engine/t100_execution.rs'), 'utf8');
  const reasons = [...new Set([...source.matchAll(/(?:t100_hold|ok_or)\("([^"{}]+)"\)/g)].map(match => match[1]))];
  assert.ok(reasons.length >= 10, 'exercise actual production hold reasons');
  for (const reason of reasons) {
    const raw = `T-100 REVIEW: ${reason}; new exposure blocked, protection remains active`;
    const polish = presentEngineText(raw, 'pl');
    assert.ok(polish.startsWith('T-100 WERYFIKACJA: '), polish);
    assert.ok(!polish.includes(reason), reason);
    assert.equal(presentEngineText(polish, 'en'), raw);
  }
  const pairs = [
    ["Konfiguracja T-100 jest nieprawidłowa", "T-100 configuration is invalid"],
    ["Zmiana konfiguracji T-100 wymaga potwierdzonego braku ekspozycji na rachunku i uzgodnionego stanu decyzji", "T-100 configuration change requires confirmed flat account and reconciled decision state"],
    ["T-100 HOLD: wymagany tryb AUTO-EA; AUTO nie może użyć tego presetu", "T-100 HOLD: AUTO-EA mode is required; AUTO cannot use this preset"],
    ['T-100 WEJŚCIE: decyzja 17 potwierdzona w koszyku 4294967000', 'T-100 ENTRY: decision 17 confirmed in basket 4294967000'],
    ['T-100 HOLD: wysłane wejście oczekuje na dokładne potwierdzenie brokera', 'T-100 HOLD: submitted entry awaits exact broker confirmation'],
    ['T-100 WERYFIKACJA: niepełny stan decyzji; przed nowymi wejściami przywróć zgodny zapis stanu', 'T-100 REVIEW: incomplete decision state; restore the matching checkpoint before new exposure'],
    ['T-100 M1: SYNTHETIC_FEED_WAIT; nowe wejścia czekają na pełne świece', 'T-100 M1: SYNTHETIC_FEED_WAIT; new entries await complete candles'],
    ['Ustawienia T-100: nieznane pole signalX', 'T-100 settings: unknown field signalX'],
  ];
  for (const [polish, english] of pairs) {
    i18n.setLanguage('en'); assert.equal(tSilnik(polish), english);
    i18n.setLanguage('pl'); assert.equal(tSilnik(english), polish);
  }
  const unknown = 'T-100 REVIEW: opaque_vendor_reason_19; new exposure blocked, protection remains active';
  assert.equal(presentEngineText(unknown, 'pl'), 'T-100 WERYFIKACJA: opaque_vendor_reason_19; nowe wejścia zablokowane, ochrona pozostaje aktywna');
});

test('actual deferred mode and complete M1 diagnostics preserve protection and session scope', () => {
  const live = readFileSync(resolve(root, '../rust/crates/app/src/live.rs'), 'utf8');
  const mode = [...new Set([...live.matchAll(/"((?:Zmiana trybu (?:wymaga|czeka|pochodzi)|Tryb zmienił)[^"{}]+)"/g)].map(m => m[1]))];
  assert.equal(mode.length, 5);
  for (const raw of mode) {
    const translated = presentEngineText(raw, 'en');
    assert.notEqual(translated, raw);
    assert.equal(presentEngineText(translated, 'pl'), raw);
  }
  const files = ['complete_m1.rs', 'bridge.rs', 'transport.rs'];
  const reasons = new Set(files.flatMap(file => {
    const source = readFileSync(resolve(root, '../rust/crates/mt5/src', file), 'utf8');
    return [...source.matchAll(/"(M1 [^"{}]+)"/g)].map(m => m[1]);
  }));
  assert.ok(reasons.size >= 10);
  for (const reason of reasons) {
    const en = `T-100 M1: ${reason}; new entries await complete candles`;
    const pl = presentEngineText(en, 'pl');
    assert.ok(!pl.includes(reason), reason);
    assert.equal(presentEngineText(pl, 'en'), en);
  }
  // Python machine codes remain verbatim, including codes unknown to this UI.
  assert.equal(presentEngineText('T-100 M1: history_unavailable; nowe wejścia czekają na pełne świece', 'en'),
    'T-100 M1: history_unavailable; new entries await complete candles');
});
