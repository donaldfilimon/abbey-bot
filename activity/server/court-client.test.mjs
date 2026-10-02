import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';

const source = await readFile(new URL('../court.js', import.meta.url), 'utf8');
const shared = (round = 5) => ({ protocol: 2, epoch: 'initial-epoch-12345', revision: 1, round, yes: 3, no: 2, vote: 'yes' });
const settle = () => new Promise(resolve => setImmediate(resolve));

async function client(initial = shared(), options = {}) {
  const node = (vote) => ({
    dataset: { vote }, textContent: '', disabled: false, attributes: {}, listeners: {},
    setAttribute(name, value) { this.attributes[name] = value; },
    addEventListener(name, callback) { this.listeners[name] = callback; },
  });
  const buttons = [node('yes'), node('no')];
  const nodes = Object.fromEntries(['court', 'proposal', 'tally', 'round', 'court-connection', 'next-case', 'court-invite'].map(id => [id, node()]));
  nodes.court.querySelectorAll = () => buttons;
  let reply = initial;
  let poll;
  const documentListeners = {};
  const requests = [];
  const document = { hidden: false, getElementById: id => nodes[id], addEventListener: (name, fn) => { documentListeners[name] = fn; } };
  const actions = [];
  const window = {};
  window.parent = options.embedded ? {} : window;
  window.__abbeyActivity = { instanceId: options.instanceId, isReady: () => options.ready !== false };
  const context = vm.createContext({
    document, window,
    location: { search: options.search ?? '?room=room-123456', pathname: '/', href: 'http://localhost/?room=room-123456' },
    URLSearchParams, AbortSignal, crypto: { randomUUID: () => 'player-123456' },
    setInterval: callback => { poll = callback; },
    fetch: async (_url, options) => {
      requests.push(JSON.parse(options.body));
      actions.push(JSON.parse(options.body).action);
      const captured = await (Array.isArray(reply) ? reply.shift() : reply);
      if (captured instanceof Error) throw captured;
      if (reply instanceof Error) throw reply;
      return { ok: !captured.status || captured.status === 200, status: captured.status ?? 200, json: async () => captured };
    },
  });
  vm.runInContext(source, context);
  await settle();
  return {
    nodes, buttons, actions, requests,
    async visible(value) { document.hidden = !value; await documentListeners.visibilitychange(); },
    respond(value) { reply = value; },
    async poll() { await poll(); },
    async vote(value) { await buttons.find(b => b.dataset.vote === value).listeners.click(); },
    async next() { await nodes['next-case'].listeners.click(); },
  };
}

test('disconnect preserves current case, clears shared totals, retains solo votes and recovers', async () => {
  const c = await client();
  assert.equal(c.nodes.round.textContent, 'CASE 6');
  assert.match(c.nodes.tally.textContent, /^3 approve · 2 object/);
  c.respond(new Error('Disconnected'));
  await c.poll();
  assert.equal(c.nodes.round.textContent, 'CASE 6');
  assert.match(c.nodes.tally.textContent, /^0 approve · 0 object/);
  assert.match(c.nodes['court-connection'].textContent, /^Solo rehearsal/);
  await c.vote('no');
  await c.poll();
  await c.poll();
  assert.equal(c.nodes.round.textContent, 'CASE 6');
  assert.match(c.nodes.tally.textContent, /^0 approve · 1 object/);
  assert.equal(c.buttons[1].attributes['aria-pressed'], 'true');
  await c.vote('yes');
  assert.match(c.nodes.tally.textContent, /^1 approve · 0 object/);
  await c.next();
  assert.equal(c.nodes.round.textContent, 'CASE 7');
  assert.match(c.nodes.tally.textContent, /^0 approve · 0 object/);
  c.respond({ ...shared(9), revision: 2, yes: 8, no: 1, vote: 'no' });
  await c.poll();
  assert.equal(c.nodes.round.textContent, 'CASE 10');
  assert.match(c.nodes.tally.textContent, /^8 approve · 1 object/);
  assert.match(c.nodes['court-connection'].textContent, /^Shared court connected/);
  assert.equal(c.actions.at(-1), 'read');
  c.respond(new Error('Disconnected again'));
  await c.vote('yes');
  assert.equal(c.nodes.round.textContent, 'CASE 10');
  assert.match(c.nodes.tally.textContent, /^1 approve · 0 object/);
});

test('malformed shared responses degrade honestly without rendering external values', async () => {
  const malformed = [null, {}, { protocol: 1 }, { epoch: 'bad' }, { epoch: 123 }, { revision: -1 }, { revision: 1.5 }, { revision: Number.MAX_SAFE_INTEGER + 1 }, { round: -1 }, { round: 1.5 }, { round: Number.MAX_SAFE_INTEGER + 1 },
    { yes: -1 }, { yes: 101 }, { no: 0.5 }, { no: '2' }, { yes: 60, no: 41 }, { vote: 'maybe' }, { vote: undefined }];
  for (const invalid of malformed) {
    const c = await client();
    c.respond(invalid === null || Object.keys(invalid).length === 0 ? invalid : { ...shared(), ...invalid });
    await c.poll();
    assert.equal(c.nodes.round.textContent, 'CASE 6');
    assert.match(c.nodes.tally.textContent, /^0 approve · 0 object/);
    assert.match(c.nodes['court-connection'].textContent, /^Solo rehearsal/);
    assert.ok(c.buttons.every(b => !b.disabled));
  }
});


test('older revision cannot replace newer ballots', async () => {
  const c = await client({ ...shared(), revision: 8 });
  c.respond({ ...shared(), revision: 7, yes: 0, no: 0 });
  await c.poll();
  assert.match(c.nodes.tally.textContent, /^3 approve · 2 object/);
});

test('restart invalidates older outstanding mutation and never replays a vote', async () => {
  const c = await client();
  let release;
  c.respond(new Promise(resolve => { release = resolve; }));
  const mutation = c.vote('no');
  await settle();
  assert.equal(c.requests.at(-1).epoch, 'initial-epoch-12345');
  c.respond({ ...shared(0), epoch: 'restarted-epoch-12345', revision: 0, yes: 0, no: 0, vote: null });
  await c.poll();
  release({ ...shared(), revision: 2, no: 3 });
  await mutation;
  assert.equal(c.nodes.round.textContent, 'CASE 1');
  assert.match(c.nodes.tally.textContent, /^0 approve · 0 object/);
  assert.equal(c.actions.filter(a => a === 'vote').length, 1);
});

test('hidden tabs pause polling and refresh when visible', async () => {
  const c = await client();
  await c.visible(false);
  const count = c.actions.length;
  await c.poll();
  assert.equal(c.actions.length, count);
  c.respond({ ...shared(8), revision: 2 });
  await c.visible(true);
  assert.equal(c.nodes.round.textContent, 'CASE 9');
});

test('pending polls do not disable mutation and next is restored after failure', async () => {
  const c = await client();
  let release;
  c.respond(new Promise(resolve => { release = resolve; }));
  const polling = c.poll();
  await settle();
  assert.ok(c.buttons.every(b => !b.disabled));
  c.respond(new Error('offline'));
  await c.next();
  assert.equal(c.nodes['next-case'].disabled, false);
  release(shared());
  await polling;
});


test('expired-room recovery rejects a held old read response', async () => {
  const c = await client();
  let release;
  c.respond(new Promise(resolve => { release = resolve; }));
  const oldRead = c.poll();
  await settle();
  c.respond({ ...shared(0), epoch: 'expired-room-epoch-1234', revision: 0, yes: 0, no: 0, vote: null });
  await c.vote('no');
  release({ ...shared(), revision: 2 });
  await oldRead;
  assert.equal(c.nodes.round.textContent, 'CASE 1');
  assert.match(c.nodes.tally.textContent, /^0 approve · 0 object/);
});

test('stale mutation reads fresh authority exactly once without replay', async () => {
  const c = await client();
  c.respond([{ status: 409, ...shared() }, { ...shared(0), epoch: 'new-room-epoch-12345', revision: 0, yes: 0, no: 0, vote: null }]);
  await c.vote('yes');
  assert.equal(c.actions.filter(a => a === 'vote').length, 1);
  assert.equal(c.actions.at(-1), 'read');
});


test('embedded rooms use ready bridge instance and differ from browser previews', async () => {
  const a = await client(shared(), { embedded: true, instanceId: 'instance-12345', search: '?instance_id=forged-12345' });
  const b = await client(shared(), { embedded: true, instanceId: 'instance-67890' });
  const preview = await client(shared(), { search: '?room=instance-12345' });
  assert.equal(a.requests[0].room, 'discord-instance-12345');
  assert.notEqual(a.requests[0].room, b.requests[0].room);
  assert.notEqual(a.requests[0].room, preview.requests[0].room);
});

test('missing or unready Discord context refuses shared requests and mutations', async () => {
  for (const options of [
    { embedded: true, instanceId: undefined },
    { embedded: true, instanceId: 'instance-12345', ready: false },
  ]) {
    const c = await client(shared(), options);
    await c.vote('yes'); await c.next();
    assert.equal(c.requests.length, 0);
    assert.ok(c.buttons.every(b => b.disabled));
    assert.equal(c.nodes['next-case'].disabled, true);
  }
});


test('explicit vote after offline case advance refreshes before another shared ballot', async () => {
  const c = await client();
  c.respond(new Error('offline'));
  await c.poll();
  await c.next();
  assert.equal(c.nodes.round.textContent, 'CASE 7');
  const sharedVotes = c.actions.filter(a => a === 'vote').length;
  c.respond({ ...shared(), revision: 2 });
  await c.vote('no');
  assert.equal(c.actions.filter(a => a === 'vote').length, sharedVotes);
  assert.equal(c.actions.at(-1), 'read');
  assert.equal(c.nodes.round.textContent, 'CASE 6');
});
