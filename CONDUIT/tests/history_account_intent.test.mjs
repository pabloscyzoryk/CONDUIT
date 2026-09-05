import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";

// Run the actual production module with deterministic React/storage doubles.
// No browser, MT5, websocket or broker process is used by these tests.
const source = readFileSync(new URL("../src/store/historiaOperacji.ts", import.meta.url), "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText;

const context = (sessionToken, follow = true) => ({ follow, sessionToken });
const intent = () => ({
  rodzaj: "position", ticket: 123, zmiany: [{ pole: "sl", z: 100, na: 101 }],
  komenda: { cmd: "modifyPosition", ticket: 123, sl: 101 },
  odwrotna: { cmd: "modifyPosition", ticket: 123, sl: 100 },
});
const row = (extra = {}) => ({ ...intent(), id: 1, czas: Date.now(), cofniety: false, ...extra });
const snapshot = { positions: [{ ticket: 123, openPrice: 102, sl: 101, tp: 110, volume: 0.01 }], pendings: [] };

function harness(initial = []) {
  let memory = JSON.parse(JSON.stringify(initial));
  let state;
  let initialized = false;
  let dirty = false;
  let effects = [];
  const calls = [];
  const storage = {
    load: () => JSON.parse(JSON.stringify(memory)),
    save: (_key, value) => { memory = JSON.parse(JSON.stringify(value)); },
  };
  const react = {
    useState(init) {
      if (!initialized) { state = typeof init === "function" ? init() : init; initialized = true; }
      return [state, (next) => {
        const value = typeof next === "function" ? next(state) : next;
        dirty ||= value !== state;
        state = value;
      }];
    },
    useMemo: (fn) => fn(),
    useCallback: (fn) => fn,
    useEffect: (fn) => effects.push(fn),
  };
  const exports = {};
  new Function("require", "exports", compiled)((name) => {
    if (name === "react") return react;
    if (name === "@/store/storage") return storage;
    throw new Error(`Unexpected runtime dependency ${name}`);
  }, exports);
  const execute = (...args) => calls.push(args);
  return {
    api: exports, calls,
    saved: () => JSON.parse(JSON.stringify(memory)),
    render(ctx, flush = true) {
      let view;
      for (let iteration = 0; iteration < 10; iteration++) {
        dirty = false;
        effects = [];
        view = exports.useHistoriaOperacji(execute, snapshot, ctx);
        if (!flush) return view;
        for (const effect of effects) effect();
        if (!dirty) return view;
      }
      throw new Error("React test fixture did not settle");
    },
  };
}

test("new follow intent records its creation session, not a later send session", () => {
  const { api } = harness();
  const input = intent();
  const bound = api.powiazWpisZRachunkiem(input, context("A:epoch1"));
  assert.equal(bound.accountSession, "A:epoch1");
  assert.equal(input.accountSession, undefined, "does not mutate caller");
  assert.equal(api.powiazWpisZRachunkiem(bound, context("B:epoch2")).accountSession, "A:epoch1");
});

test("unknown or explicitly empty intent never acquires a later account", () => {
  const { api } = harness();
  const missing = api.powiazWpisZRachunkiem(intent(), context(null));
  assert.equal(missing.accountSession, "");
  assert.equal(api.powiazWpisZRachunkiem(missing, context("A:epoch1")).accountSession, "");
  assert.equal(api.powiazWpisZRachunkiem({ ...intent(), accountSession: null }, context("A:epoch1")).accountSession, "");
});

test("same token is valid; same ticket/values on another account are blocked immediately", () => {
  const h = harness([row({ accountSession: "A:epoch1" })]);
  const original = h.render(context("A:epoch1"));
  assert.equal(h.api.przeszkodaCofniecia(original.doCofniecia, snapshot), null);
  const changed = h.render(context("B:epoch2"), false); // before persistence effect
  assert.equal(changed.doCofniecia.accountInvalidated, true);
  assert.equal(h.api.przeszkodaCofniecia(changed.doCofniecia, snapshot), "undo.reason.accountChanged");
  changed.cofnij();
  changed.ponow();
  assert.equal(h.calls.length, 0);
});

test("invalidation survives storage reload and A→B→A, without rebasing audit origin", () => {
  const h = harness([row({ accountSession: "A:epoch1", cofniety: true })]);
  h.render(context("B:epoch2"));
  const stored = h.saved();
  assert.equal(stored[0].accountSession, "A:epoch1");
  assert.equal(stored[0].accountInvalidated, true);
  const restarted = harness(stored);
  const returned = restarted.render(context("A:epoch1"));
  assert.equal(returned.doPonowienia, null);
  returned.ponow();
  assert.equal(restarted.calls.length, 0);
  assert.equal(returned.wpisy.length, 1, "audit is retained");
});

test("a new epoch of the same account invalidates old undo and redo", () => {
  for (const cofniety of [false, true]) {
    const h = harness([row({ accountSession: "A:epoch1", cofniety })]);
    const view = h.render(context("A:epoch3"));
    view.cofnij(); view.ponow();
    assert.equal(h.calls.length, 0);
    assert.equal(view.wpisy[0].accountInvalidated, true);
  }
});

test("legacy localStorage entries without provenance fail closed only in follow mode", () => {
  const h = harness([row()]);
  const view = h.render(context("A:epoch1"));
  assert.equal(view.wpisy[0].accountInvalidated, true);
  view.cofnij();
  assert.equal(h.calls.length, 0);
  const off = harness([row()]);
  off.render(context(null, false)).cofnij();
  assert.equal(off.calls.length, 1);
  assert.equal(off.calls[0][2], undefined);
});

test("current-account undo AND redo send the stored original token", () => {
  const h = harness();
  h.render(context("A:epoch1")).zapisz(intent());
  let view = h.render(context("A:epoch1"));
  assert.equal(h.saved()[0].accountSession, "A:epoch1");
  view.cofnij();
  view = h.render(context("A:epoch1"));
  assert.equal(view.doPonowienia.accountSession, "A:epoch1");
  view.ponow();
  assert.equal(h.calls.length, 2);
  assert.equal(h.calls[0][0].sl, 100);
  assert.equal(h.calls[1][0].sl, 101);
  assert.equal(h.calls[0][2], "A:epoch1");
  assert.equal(h.calls[1][2], "A:epoch1");
});

test("stale modal callback carries old provenance for backend rejection, never current token", () => {
  const h = harness([row({ accountSession: "A:epoch1" })]);
  const staleUndo = h.render(context("A:epoch1")).cofnij;
  h.render(context("B:epoch2"));
  staleUndo();
  assert.equal(h.calls[0][2], "A:epoch1", "server must reject this token on B");
  const view = h.render(context("B:epoch2"));
  assert.equal(view.doPonowienia, null);
});

test("delayed history creation retains old or unknown context across an account switch", () => {
  for (const original of ["A:epoch1", null]) {
    const h = harness();
    const delayedSave = h.render(context(original)).zapisz;
    h.render(context("B:epoch2"));
    delayedSave(intent());
    const view = h.render(context("B:epoch2"));
    assert.equal(view.wpisy[0].accountSession, original ?? "");
    assert.equal(view.wpisy[0].accountInvalidated, true);
  }
});

test("follow OFF preserves legacy document shape and old history behaviour", () => {
  const h = harness();
  const input = intent();
  assert.equal(h.api.powiazWpisZRachunkiem(input, context(null, false)), input);
  const rows = [row()];
  assert.equal(h.api.uniewaznijHistorieRachunku(rows, context(null, false)), rows);
  h.render(context(null, false)).zapisz(input);
  const saved = h.saved()[0];
  assert.equal(Object.hasOwn(saved, "accountSession"), false);
  assert.equal(Object.hasOwn(saved, "accountInvalidated"), false);
  h.render(context(null, false)).cofnij();
  h.render(context(null, false)).ponow();
  assert.equal(h.calls.length, 2);
  assert.equal(h.calls[0][2], undefined);
  assert.equal(h.calls[1][2], undefined);
});

test("persisted invalidation is not a consent that toggling follow OFF can restore", () => {
  const h = harness([row({ accountSession: "A:epoch1", accountInvalidated: true })]);
  const view = h.render(context(null, false));
  view.cofnij(); view.ponow();
  assert.equal(h.calls.length, 0);
});

test("repeated invalidation is idempotent and does not rewrite storage or tokens", () => {
  const { api } = harness();
  const first = api.uniewaznijHistorieRachunku([row({ accountSession: "A:epoch1" })], context(null));
  assert.equal(api.uniewaznijHistorieRachunku(first, context("B:epoch2")), first);
  assert.equal(first[0].accountSession, "A:epoch1");
});
