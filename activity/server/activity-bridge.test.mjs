import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';

const source = await readFile(new URL('../app.js', import.meta.url), 'utf8');
function bridge() {
  const listeners = {};
  const parent = { postMessage() {} };
  const nodes = new Map();
  const window = {
    parent, location: { search: '?frame_id=frame-12345&instance_id=instance-12345', origin: 'https://1147940171099152464.discordsays.com' },
    addEventListener: (name, fn) => { listeners[name] = fn; },
  };
  const context = vm.createContext({ window, URLSearchParams,
    document: { getElementById: id => {
      if (!nodes.has(id)) nodes.set(id, { textContent: '', classList: { add() {}, remove() {} } });
      return nodes.get(id);
    } },
    setTimeout() {}, fetch: async () => ({ ok: false }),
  });
  vm.runInContext(source, context);
  return { window, parent, send: event => listeners.message(event) };
}
const ready = [1, { cmd: 'DISPATCH', evt: 'READY', data: {} }];

test('READY requires the Discord RPC parent, allowed origin and frame envelope', () => {
  for (const patch of [
    { source: {} }, { origin: 'https://discord.com.evil.example' },
    { origin: 'http://discord.com' }, { origin: 'https://untrusted.example' },
    { data: ready[1] }, { data: [99, ready[1]] }, { data: null },
  ]) {
    const b = bridge();
    b.send({ source: b.parent, origin: 'https://discord.com', data: ready, ...patch });
    assert.equal(b.window.__abbeyActivity.isReady(), false);
  }
});

test('verified Discord parent can establish READY on stable, preview and native origins', () => {
  for (const origin of ['https://discord.com', 'https://ptb.discord.com', 'https://canary.discord.com', 'null']) {
    const b = bridge();
    b.send({ source: b.parent, origin, data: ready });
    assert.equal(b.window.__abbeyActivity.isReady(), true);
    assert.equal(b.window.__abbeyActivity.instanceId, 'instance-12345');
  }
});
