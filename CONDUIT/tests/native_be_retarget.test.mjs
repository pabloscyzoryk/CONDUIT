import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// Execute the ACTUAL arithmetic/boolean MQL helper bodies after removing only
// primitive type syntax. This is offline cross-language verification, not an
// MT5 execution/fill test. Broker effects and the MQL VM need a native gate.
const source = readFileSync(new URL('../mql5/CONDUIT_XT.mq5', import.meta.url), 'utf8');
function body(name) {
  const declaration = new RegExp('\\b(?:bool|double|int|void)\\s+' + name + '\\s*\\(([^)]*)\\)\\s*\\{').exec(source);
  assert.ok(declaration, name);
  const start = declaration.index + declaration[0].length;
  let level = 1, end = start;
  for (; level && end < source.length; end++) {
    if (source[end] === '{') level++;
    else if (source[end] === '}') level--;
  }
  assert.equal(level, 0, name);
  return { args: declaration[1], text: source.slice(start, end - 1) };
}
function compile(name, env = {}) {
  const actual = body(name);
  const args = actual.args.split(',').map(x => x.trim().replace(/^(?:int|double|bool)\s+/, ''));
  assert.ok(args.every(x => /^[a-z_][a-z0-9_]*$/i.test(x)));
  const code = actual.text.replace(/\b(?:int|double|bool)\s+/g, 'let ');
  return new Function(...Object.keys(env), `return function(${args.join(',')}) {${code}}`)(...Object.values(env));
}
const env = { SideBetter: compile('SideBetter'), SideSign: compile('SideSign') };
const mayBe = compile('BeReplacementAllowed', env);
const retarget = compile('RetargetPrice', env);

test('actual native BE decision preserves better Buy/Sell stops and retains legacy OFF/equal/no-stop behavior', () => {
  for (const side of [0, 1]) {
    const better = side === 0 ? 4006 : 3994;
    const worse = side === 0 ? 3994 : 4006;
    assert.equal(mayBe(side, better, 4000, true), false);
    assert.equal(mayBe(side, better, 4000, false), true);
    assert.equal(mayBe(side, worse, 4000, true), true);
    assert.equal(mayBe(side, 0, 4000, true), true);
    assert.equal(mayBe(side, 4000, 4000, true), true);
  }
});

test('actual native retarget helper preserves final target including one offset across repeated stages', () => {
  for (const side of [0, 1]) {
    const s = side === 0 ? 1 : -1;
    const last = 4000 + s * 30, final = last + s * 12;
    let legacy = last, fixed = last;
    for (const stage of [1, 2, 3, 4]) {
      const hasNext = stage < 3;
      const next = 4000 + s * (stage + 1) * 10;
      legacy = retarget(side, false, 0, legacy, hasNext, next, last, 4, false, 12);
      fixed = retarget(side, true, final, fixed, hasNext, next, last, 4, false, 12);
      assert.equal(fixed, final);
      assert.equal(legacy, 4000 + s * [0, 20, 30, 42, 54][stage]);
    }
    assert.equal(retarget(side, false, 0, final, false, 0, last, 4, true, 12), last);
    assert.equal(retarget(side, false, 0, final, true, 4000+s*20, last, 0, false, 12), last);
  }
});

test('actual trade paths use the helpers with exactly the Rust scopes and retain TP-less runners', () => {
  const be = body('MoveBasketToBe').text;
  const rf = body('HandleRiskFree').text;
  const target = body('Retarget').text;
  assert.match(be, /BeReplacementAllowed\(g_b\[bi\]\.side, cur_sl, be, In_TrailSrEnabled \|\| In_BeNeverLoosen\)/);
  assert.match(rf, /BeReplacementAllowed\(side, cur_sl, be, In_BeNeverLoosen\)/);
  assert.doesNotMatch(rf, /In_TrailSrEnabled/);
  assert.match(rf, /else if\(newtp != cur_tp\) ModyfikujPozycje\(t, cur_sl/);
  assert.match(target, /keep_final = In_RetargetRespectsFinal && In_CeleNaOstatnim/);
  assert.ok(target.indexOf('if(cur_tp == 0.0) continue') < target.indexOf('TargetForEx(bi, 0, 1, final_target)'));
  assert.match(target, /RetargetPrice\(g_b\[bi\]\.side, keep_final, final_target/);
  assert.match(body('OnInit').text, /if\(!TestBeRetargetContract\(\)\) return INIT_FAILED/);
  assert.match(source, /input bool\s+In_BeNeverLoosen\s*= false/);
  assert.match(source, /input bool\s+In_RetargetRespectsFinal\s*= false/);
});
