import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

const source = readFileSync(new URL('../src/components/panels/ChatPanel.tsx', import.meta.url), 'utf8');
const tree = ts.createSourceFile('ChatPanel.tsx', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const fn = tree.statements.find(n => ts.isFunctionDeclaration(n) && n.name?.text === 'Message');
const compiled = ts.transpileModule(fn.getText(tree), { compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React } }).outputText;
const flat = node => Array.isArray(node) ? node.flatMap(flat) : node && typeof node === 'object' ? [node, ...flat(node.props?.children)] : [];
const strings = node => flat(node).flatMap(n => n.props?.children ?? []).filter(x => typeof x === 'string');
const calls = [];
const render = new Function('React', 'useApp', 'useT', 'SIGNAL_TONE', 'TONE_MAP', 'SIGNAL_LABEL', 'time', 'Badge', 'Button', 'Icon', `${compiled};return Message;`)(
  { createElement: (type, props, ...children) => ({ type, props: { ...props, children } }) },
  () => ({ executeMessage: id => calls.push(['execute', id]), dismissMessage: id => calls.push(['dismiss', id]) }),
  () => key => key, { ENTRY: 'info' }, { info: 'info' }, { ENTRY: 'entry' }, () => 'TIME', 'Badge', 'Button', 'Icon');
const message = pendingAction => ({ id: 'M1', channelId: 1, channelName: 'Synthetic', types: ['ENTRY'], parsed: [], basketId: null, text: 'ENTRY', pendingAction });

test('actual deferred message has a neutral historical status, no executed claim and no manual execute button', () => {
  const result = render({ m: message('deferred') });
  assert.ok(strings(result).includes('chat.deferred'));
  assert.equal(strings(result).includes('chat.executed'), false);
  assert.equal(strings(result).includes('chat.dismissed'), false);
  assert.equal(flat(result).some(n => n.type === 'Button'), false);
  const badge = flat(result).find(n => n.props?.title === 'chat.deferred.hint');
  assert.equal(badge.props.className, 'msg__status flat');
  assert.deepEqual(calls, []);
});

test('legacy await and confirmed executed remain distinct from deferred', () => {
  const awaiting = render({ m: message('await') });
  assert.equal(flat(awaiting).filter(n => n.type === 'Button').length, 2);
  assert.equal(strings(awaiting).includes('chat.deferred'), false);
  const executed = render({ m: message('executed') });
  assert.ok(strings(executed).includes('chat.executed'));
  assert.equal(strings(executed).includes('chat.deferred'), false);
  assert.match(source, /if \(k === "pending"\) return m\.pendingAction === "await"/);
});
