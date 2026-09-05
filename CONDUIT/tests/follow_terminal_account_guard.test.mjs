import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";

// Execute the ACTUAL AppStore helper, not a second implementation. React and
// the browser are deliberately excluded from this pure settings contract test.
const source = readFileSync(new URL("../src/store/AppStore.tsx", import.meta.url), "utf8");
const tree = ts.createSourceFile("AppStore.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const fn = tree.statements.find((n) => ts.isFunctionDeclaration(n) && n.name?.text === "preserveFollowTerminalAccount");
assert.ok(fn, "production account guard exists");
const generated = readFileSync(new URL("../src/store/polaRachunku.generated.ts", import.meta.url), "utf8");
const uiBlock = generated.match(/export const POLA_RACHUNKU = \[([\s\S]*?)\] as const/)[1];
const uiKeys = [...uiBlock.matchAll(/"([a-z0-9_]+)"/g)].map((m) => m[1]);
const rust = readFileSync(new URL("../rust/crates/core/src/wielosilnik.rs", import.meta.url), "utf8");
const coreBlock = rust.match(/pub const POLA_RACHUNKU: &\[&str\] = &\[([\s\S]*?)\n\];/)[1];
const coreKeys = [...coreBlock.matchAll(/^\s*"([a-z0-9_]+)",/gm)].map((m) => m[1]);
const printed = ts.createPrinter().printNode(ts.EmitHint.Unspecified, fn, tree);
const js = ts.transpileModule(printed, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
const guard = new Function("POLA_RACHUNKU", `${js}; return preserveFollowTerminalAccount;`)(uiKeys);

test("follow ON preserves every canonical account field, all UI aliases and runtime identity", () => {
  const keys = [...new Set([...coreKeys, ...uiKeys,
    "mt5_follow_terminal_account", "mt5_allow_real_account", "mt5_login", "mt5_server",
    "mt5_password", "mt5_terminal_path", "mt5_python", "mt5_symbol", "mt5_magic", "mt5_deviation_points",
  ])];
  const before = Object.fromEntries(keys.map((key) => [key, `before-${key}`]));
  before.mt5_follow_terminal_account = true;
  before.mt5_allow_real_account = false;
  before.mt5_autostart = false;
  before.mt5_watchdog = false;
  before.credit_balance_separate = true;
  before.close_receipt_reconcile = true;
  before.closed_profit_net_costs = true;
  before.order_volume_contract_v2 = true;
  const incoming = Object.fromEntries(keys.map((key) => [key, `preset-${key}`]));
  incoming.entry_units = 7;
  const output = guard(before, incoming);
  for (const key of keys) assert.deepEqual(output[key], before[key], key);
  assert.equal(output.entry_units, 7, "strategy fields may change");
  assert.equal(incoming.mt5_allow_real_account, "preset-mt5_allow_real_account", "input is not mutated");
});

test("missing authorization and credentials are not imported by a preset or reset", () => {
  const output = guard({ mt5_follow_terminal_account: true }, {
    mt5_follow_terminal_account: false, mt5_allow_real_account: true, mt5_login: 999,
    mt5_password: "synthetic", mt5_watchdog: true, credit_balance_separate: false,
    close_receipt_reconcile: true,
    closed_profit_net_costs: true,
    order_volume_contract_v2: true,
  });
  assert.deepEqual(output, { mt5_follow_terminal_account: true });
});

test("follow OFF leaves the legacy incoming document unchanged", () => {
  const incoming = { mt5_allow_real_account: true, entry_units: 3 };
  assert.equal(guard({ mt5_follow_terminal_account: false }, incoming), incoming);
  assert.equal(guard({}, incoming), incoming);
});
