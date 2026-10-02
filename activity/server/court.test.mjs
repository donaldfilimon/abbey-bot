import test from 'node:test';
import { connect } from 'node:net';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { createCourtServer } from './court.mjs';

test('two anonymous clients share votes, replacements and round transitions', async () => {
  let time = 10000;
  const server = createCourtServer({ now: () => time });
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const url = `http://127.0.0.1:${server.address().port}`;
  const call = async (body) => {
    const response = await fetch(url + '/court', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
    return { status: response.status, body: await response.json() };
  };
  const boot = await call({ room: 'room-123456', player: 'player-111', action: 'read' });
  const a = { epoch: boot.body.epoch, room: 'room-123456', player: 'player-111', action: 'vote', round: 0, vote: 'yes' };
  try {
    assert.equal((await call(a)).body.yes, 1);
    assert.equal((await call({ ...a, player: 'player-222', vote: 'no' })).body.no, 1);
    assert.equal((await call({ ...a, vote: 'no' })).body.no, 2);
    assert.equal((await call({ ...a, room: 'other-room', action: 'read' })).body.no, 0);
    assert.equal((await call({ ...a, action: 'next' })).body.round, 1);
    assert.equal((await call(a)).status, 409);
    assert.equal((await call({ ...a, action: 'next', round: 1 })).status, 429);
    assert.equal((await call({ ...a, action: 'read' })).body.yes, 0);
    assert.equal((await call({ ...a, room: '../secret' })).status, 400);
    assert.equal((await call({ ...a, vote: 'arbitrary content' })).status, 400);
    assert.equal((await call({ ...a, extra: 'x'.repeat(2000) })).status, 413);
    const staticResponse = await fetch(url + '/');
    assert.match(await staticResponse.text(), /Bad Idea Court/);
    assert.equal((await fetch(url + '/server/court.mjs')).status, 404);
    time += 31 * 60 * 1000;
    assert.equal((await call({ ...a, action: 'read' })).body.round, 0);
  } finally { await new Promise(r => server.close(r)); }
});

async function withCourt(run) {
  let time = 10000, epoch = 0;
  const server = createCourtServer({ now: () => time, newEpoch: () => `test-room-epoch-${++epoch}` });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const call = async (patch = {}) => {
    const response = await fetch(`http://127.0.0.1:${server.address().port}/court`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ room: 'room-123456', player: 'player-111', action: 'read', ...patch }),
    });
    return { status: response.status, body: await response.json() };
  };
  try { await run(call, amount => { time += amount; }, server); }
  finally { await new Promise(resolve => server.close(resolve)); }
}

test('v2 mutations require current epoch and increment revision', async () => {
  await withCourt(async call => {
    const boot = (await call()).body;
    assert.equal(boot.protocol, 2);
    assert.equal(boot.revision, 0);
    assert.match(boot.epoch, /^[A-Za-z0-9_-]{16,64}$/);
    assert.equal((await call({ action: 'vote', round: 0, vote: 'yes' })).status, 409);
    const voted = await call({ action: 'vote', epoch: boot.epoch, round: 0, vote: 'yes' });
    assert.equal(voted.body.revision, 1);
    assert.equal(voted.body.yes, 1);
    assert.equal((await call({ action: 'next', epoch: 'stale-epoch-12345', round: 0 })).status, 409);
    assert.equal((await call()).body.revision, 1);
    assert.equal((await call({ action: 'next', epoch: boot.epoch, round: 0 })).body.revision, 2);
  });
});

test('expired room gets a new epoch and cannot accept an old vote', async () => {
  await withCourt(async (call, advance) => {
    const before = (await call()).body;
    await call({ action: 'vote', epoch: before.epoch, round: 0, vote: 'yes' });
    advance(1800001);
    const after = (await call()).body;
    assert.notEqual(after.epoch, before.epoch);
    assert.equal(after.revision, 0);
    assert.equal(after.yes, 0);
    assert.equal((await call({ action: 'vote', epoch: before.epoch, round: 0, vote: 'yes' })).status, 409);
    assert.equal((await call()).body.yes, 0);
  });
});

test('player and room capacities refuse excess without changing ballots', async () => {
  await withCourt(async call => {
    const boot = (await call()).body;
    for (let i = 0; i < 100; i++) {
      assert.equal((await call({ action: 'vote', epoch: boot.epoch, round: 0, vote: 'yes', player: `player-${i.toString().padStart(4, '0')}` })).status, 200);
    }
    assert.equal((await call({ action: 'vote', epoch: boot.epoch, round: 0, vote: 'no', player: 'extra-player' })).status, 429);
    assert.equal((await call()).body.yes, 100);
    for (let i = 1; i < 256; i++) assert.equal((await call({ room: `room-${i.toString().padStart(4, '0')}` })).status, 200);
    assert.equal((await call({ room: 'extra-room' })).status, 503);
  });
});


test('health and asset headers bind to the served release bytes', async () => {
  await withCourt(async (_call, _advance, server) => {
    const origin = `http://127.0.0.1:${server.address().port}`;
    const response = await fetch(origin + '/health');
    assert.equal(response.status, 200);
    const health = await response.json();
    assert.equal(health.service, 'abbey-court');
    assert.equal(health.protocol, 2);
    const hash = createHash('sha256');
    for (const path of ['app.js', 'court.js', 'index.html', 'server/court.mjs']) {
      const bytes = await readFile(new URL(`../${path}`, import.meta.url));
      hash.update(`${path}\0${bytes.length}\0`); hash.update(bytes);
    }
    assert.equal(health.digest, hash.digest('hex'));
    assert.equal(response.headers.get('x-abbey-deployed-digest'), health.digest);
    const asset = await fetch(origin + '/');
    assert.equal(asset.headers.get('x-abbey-deployed-digest'), health.digest);
  });
});


test('stalled request body is closed without creating ballot state', { timeout: 9000 }, async () => {
  await withCourt(async (call, _advance, server) => {
    const socket = connect(server.address().port, '127.0.0.1');
    try {
      await new Promise((resolve, reject) => { socket.once('connect', resolve); socket.once('error', reject); });
      const closed = new Promise(resolve => socket.once('close', resolve));
      socket.on('error', () => {});
      socket.write('POST /court HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{');
      await closed;
      const state = (await call()).body;
      assert.equal(state.revision, 0);
      assert.equal(state.yes, 0);
    } finally { socket.destroy(); }
  });
});


test('trickling body cannot extend the absolute five-second request deadline', { timeout: 9000 }, async () => {
  await withCourt(async (_call, _advance, server) => {
    const socket = connect(server.address().port, '127.0.0.1');
    let interval, watchdog, exceededDeadline = false;
    try {
      await new Promise((resolve, reject) => { socket.once('connect', resolve); socket.once('error', reject); });
      const closed = new Promise(resolve => socket.once('close', resolve));
      socket.on('error', () => {});
      socket.write('POST /court HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 1000\r\n\r\n{');
      interval = setInterval(() => socket.write(' '), 200);
      watchdog = setTimeout(() => { exceededDeadline = true; socket.destroy(); }, 6500);
      await closed;
      assert.equal(exceededDeadline, false, 'trickling bytes must not keep a request alive');
    } finally { clearInterval(interval); clearTimeout(watchdog); socket.destroy(); }
  });
});
