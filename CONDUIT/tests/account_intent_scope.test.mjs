import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";

const read = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
const transportSource = read("../src/store/transport.ts");
const transportTree = ts.createSourceFile("transport.ts", transportSource, ts.ScriptTarget.Latest, true);
const printer = ts.createPrinter();
const selected = transportTree.statements.filter((n) =>
  (ts.isFunctionDeclaration(n) && ["bindCommandToAccount", "wsUrl"].includes(n.name?.text)) ||
  (ts.isClassDeclaration(n) && n.name?.text === "Transport"));
assert.equal(selected.length, 3, "execute actual transport and binder");
const js = ts.transpileModule(selected.map((n) => printer.printNode(ts.EmitHint.Unspecified, n, transportTree)).join("\n"), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText;

class FakeSocket {
  static OPEN = 1;
  static latest;
  readyState = 0;
  sent = [];
  constructor() { FakeSocket.latest = this; }
  send(wire) { this.sent.push(wire); }
  close() { this.readyState = 3; this.onclose?.(); }
  open() { this.readyState = 1; this.onopen?.(); }
}
const moduleExports = {};
new Function("exports", "WebSocket", "t", "backendBase", "MAX_KOLEJKA", "ACK_TIMEOUT_MS", js)(
  moduleExports, FakeSocket, (key) => key, () => "http://synthetic.invalid", 100, 10000);
const { Transport, bindCommandToAccount } = moduleExports;

test("old intent and undo tokens cannot be replaced with the current account token", () => {
  const old = { cmd: "closePosition", ticket: 7, accountSession: "B-2" };
  assert.equal(bindCommandToAccount(old, "A-3"), old);
  assert.equal(bindCommandToAccount({ cmd: "closePosition", ticket: 7, accountSession: "" }, "A-3").accountSession, "");
  assert.equal(bindCommandToAccount({ cmd: "closePosition", ticket: 7, accountSession: null }, "A-3").accountSession, null);
  assert.equal(bindCommandToAccount({ cmd: "closePosition", ticket: 7 }, "A-3").accountSession, "A-3");
});

test("disconnected queue serializes original B token and flushes it unchanged after A reconnect", async () => {
  const t = new Transport({}, "http://synthetic.invalid");
  const pending = t.send({ cmd: "closePosition", ticket: 7 }, "B-2");
  assert.equal(t.kolejka.length, 1);
  const original = t.kolejka[0].wiadomosc;
  assert.equal(JSON.parse(original).accountSession, "B-2");
  t.connect();
  const ws = FakeSocket.latest;
  // Snapshot changes before flushing; transport must never attach this token.
  ws.onmessage?.({ data: JSON.stringify({ type: "snapshot", state: { connection: { accountSession: "A-3" } } }) });
  ws.open();
  assert.equal(ws.sent[0], original);
  const command = JSON.parse(ws.sent[0]);
  ws.onmessage({ data: JSON.stringify({ type: "ack", reqId: command.reqId, ok: false, error: "accountSession" }) });
  assert.deepEqual(await pending, { ok: false, error: "accountSession" });
  t.close();
});

test("legacy unscoped wire format remains without accountSession", async () => {
  const t = new Transport({}, "http://synthetic.invalid");
  const pending = t.send({ cmd: "closePosition", ticket: 7 });
  assert.deepEqual(JSON.parse(t.kolejka[0].wiadomosc), { type: "command", reqId: 1, cmd: "closePosition", ticket: 7 });
  t.close();
  assert.equal((await pending).ok, false);
});

// Execute the ACTUAL hook callback declarations with a trivial useCallback.
// The old closure models a still-pending modal confirmation after re-render.
const hookSource = read("../src/store/useBackend.ts");
const hookTree = ts.createSourceFile("useBackend.ts", hookSource, ts.ScriptTarget.Latest, true);
const hook = hookTree.statements.find((n) => ts.isFunctionDeclaration(n) && n.name?.text === "useBackend");
const vars = hook.body.statements.filter((n) => ts.isVariableStatement(n) && n.declarationList.declarations.some(
  (d) => ts.isIdentifier(d.name) && ["renderedAccountSession", "send"].includes(d.name.text)));
assert.equal(vars.length, 2);
const callbackJs = ts.transpileModule(vars.map((n) => printer.printNode(ts.EmitHint.Unspecified, n, hookTree)).join("\n"), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, removeComments: true },
}).outputText;
assert.ok(!callbackJs.includes("stan.current"), "never look up newest snapshot when invoking old intent");
const renderSender = new Function("snapshot", "transport", "useCallback", "tSlownik", `${callbackJs};return send;`);

test("old modal callback stays B-2 after rendering A-3 with the same ticket", async () => {
  const calls = [];
  const ref = { current: { send: async (cmd, token) => { calls.push({ cmd, token }); return { ok: true }; } } };
  const oldModalConfirm = renderSender({ connection: { accountSession: "B-2" } }, ref, (f) => f, (s) => s);
  const freshButton = renderSender({ connection: { accountSession: "A-3" } }, ref, (f) => f, (s) => s);
  await oldModalConfirm({ cmd: "closePosition", ticket: 7 });
  await freshButton({ cmd: "closePosition", ticket: 7 });
  assert.deepEqual(calls.map((x) => x.token), ["B-2", "A-3"]);
});

test("account-session keyed UI boundary and callback dependencies are present", () => {
  const app = read("../src/store/AppStore.tsx");
  assert.match(app, /<Fragment key=\{accountIntentScope === undefined/);
  assert.match(app, /\[sendCmd, toast\]/);
  assert.match(hookSource, /\[renderedAccountSession\]/);
  assert.match(app, /accountSession: originAccountSession/);
});

test("manual REST submit preserves the rendered token through asynchronous response", async () => {
  // Execute the actual REST helper and actual RecznySygnal submit closure.
  const apiDeclaration = transportTree.statements.find((n) => ts.isVariableStatement(n) && n.declarationList.declarations.some(
    (d) => ts.isIdentifier(d.name) && d.name.text === "api"));
  const apiObject = apiDeclaration.declarationList.declarations.find((d) => d.name.text === "api").initializer;
  const signal = apiObject.properties.find((p) => p.name?.getText(transportTree) === "demoSignal");
  assert.ok(signal && ts.isPropertyAssignment(signal));
  const signalJs = ts.transpileModule(`const call = ${signal.initializer.getText(transportTree)};`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const requests = [];
  const releases = [];
  const call = new Function("json", `${signalJs};return call;`)((url, init) => {
    requests.push({ url, ...JSON.parse(init.body) });
    return new Promise((resolve) => releases.push(() => resolve({ ok: true, target: "engine", parsed: [] })));
  });
  const demoSource = read("../src/views/DemoView.tsx");
  const demoTree = ts.createSourceFile("DemoView.tsx", demoSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const component = demoTree.statements.find((n) => ts.isFunctionDeclaration(n) && n.name?.text === "RecznySygnal");
  const submit = component.body.statements.find((n) => ts.isVariableStatement(n) && n.declarationList.declarations.some(
    (d) => ts.isIdentifier(d.name) && d.name.text === "wyslij"));
  const submitJs = ts.transpileModule(printer.printNode(ts.EmitHint.Unspecified, submit, demoTree), {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const renderSubmit = new Function("app", "api", "text", "setWysyla", "setText", "t", `${submitJs};return wyslij;`);
  const api = { demoSignal: call };
  const appB = { connection: { accountSession: "B-2" }, toast() {} };
  const oldSubmit = renderSubmit(appB, api, "BUY GOLD @ 4000/3995 TP 4010 SL 3990", () => {}, () => {}, (s) => s);
  const freshSubmit = renderSubmit({ connection: { accountSession: "A-3" }, toast() {} }, api, "TP1 HIT", () => {}, () => {}, (s) => s);
  // Re-render/switch happened before an old confirmation callback runs.
  await Promise.resolve();
  const oldPending = oldSubmit();
  const freshPending = freshSubmit();
  assert.deepEqual(requests.map((r) => r.accountSession), ["B-2", "A-3"]);
  for (const resolve of releases) resolve();
  await Promise.all([oldPending, freshPending]);
  assert.deepEqual(requests.map((r) => r.accountSession), ["B-2", "A-3"]);
});
