// Anonymous, content-free multiplayer state; no bot credentials or OAuth.
import { createHash, randomUUID } from 'node:crypto';
import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';

export function createCourtServer({ now = Date.now, newEpoch = randomUUID } = {}) {
  const rooms = new Map();
  const ttl = 30 * 60 * 1000;
  const valid = value => typeof value === 'string' && /^[A-Za-z0-9_-]{8,128}$/.test(value);
  const assets = new Map([
    ['/', ['index.html', 'text/html']], ['/index.html', ['index.html', 'text/html']],
    ['/app.js', ['app.js', 'text/javascript']], ['/court.js', ['court.js', 'text/javascript']],
  ]);
  const root = fileURLToPath(new URL('../', import.meta.url));
  // Snapshot immutable assets once, so identity describes the bytes served.
  const release = new Map();
  const hash = createHash('sha256');
  for (const file of ['app.js', 'court.js', 'index.html', 'server/court.mjs']) {
    const bytes = readFileSync(resolve(root, file));
    hash.update(`${file}\0${bytes.length}\0`); hash.update(bytes);
    release.set(file, bytes);
  }
  const digest = hash.digest('hex');
  const server = createServer({ connectionsCheckingInterval: 1000 }, async (req, res) => {
    // Inactivity and Node's periodic request checks do not bound a trickling body.
    const deadline = setTimeout(() => req.destroy(), 5000);
    deadline.unref();
    res.once('close', () => clearTimeout(deadline));
    const reply = (status, body) => {
      res.writeHead(status, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff', 'X-Abbey-Deployed-Digest': digest });
      res.end(JSON.stringify(body));
    };
    try {
      const path = new URL(req.url, 'http://localhost').pathname.replace(/^\/\.proxy/, '');
      if (req.method === 'GET' && path === '/health') {
        reply(200, { service: 'abbey-court', protocol: 2, status: 'ready', digest }); return;
      }
      if (req.method === 'GET' && assets.has(path)) {
        const [file, type] = assets.get(path);
        const data = release.get(file);
        res.writeHead(200, { 'Content-Type': type + '; charset=utf-8', 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff', 'X-Abbey-Deployed-Digest': digest });
        res.end(data); return;
      }
      if (path !== '/court' || req.method !== 'POST') { reply(404, { error: 'not_found' }); return; }
      if (!(req.headers['content-type'] || '').startsWith('application/json')) { reply(415, { error: 'json_required' }); return; }
      let bytes = 0; const chunks = [];
      for await (const chunk of req) {
        bytes += chunk.length;
        if (bytes > 1024) { reply(413, { error: 'too_large' }); return; }
        chunks.push(chunk);
      }
      let input;
      try { input = JSON.parse(Buffer.concat(chunks)); } catch { reply(400, { error: 'invalid_json' }); return; }
      if (!input || !valid(input.room) || !valid(input.player) || !['read', 'vote', 'next'].includes(input.action)
        || (input.action === 'vote' && !['yes', 'no'].includes(input.vote))) {
        reply(400, { error: 'invalid_request' }); return;
      }
      const time = now();
      for (const [id, room] of rooms) if (time - room.touched > ttl) rooms.delete(id);
      let room = rooms.get(input.room);
      if (!room) {
        if (rooms.size >= 256) { reply(503, { error: 'court_full' }); return; }
        const epoch = newEpoch();
        if (typeof epoch !== 'string' || !/^[A-Za-z0-9_-]{16,64}$/.test(epoch)) {
          reply(503, { error: 'epoch_unavailable' }); return;
        }
        room = { epoch, revision: 0, round: 0, votes: new Map(), touched: time, advanced: -Infinity };
        rooms.set(input.room, room);
      }
      room.touched = time;
      if (input.action !== 'read' && (input.epoch !== room.epoch || input.round !== room.round)) { reply(409, { error: 'stale_round' }); return; }
      if (input.action === 'next') {
        if (time - room.advanced < 3000) { reply(429, { error: 'slow_down' }); return; }
        room.round++; room.votes.clear(); room.advanced = time;
      }
      if (input.action === 'vote') {
        if (!room.votes.has(input.player) && room.votes.size >= 100) { reply(429, { error: 'jury_full' }); return; }
        room.votes.set(input.player, input.vote);
      }
      if (input.action !== 'read') room.revision++;
      const votes = [...room.votes.values()];
      reply(200, { protocol: 2, epoch: room.epoch, revision: room.revision, round: room.round, yes: votes.filter(v => v === 'yes').length,
        no: votes.filter(v => v === 'no').length, vote: room.votes.get(input.player) ?? null });
    } catch { if (!res.headersSent) reply(500, { error: 'court_unavailable' }); else res.end(); }
  });
  server.requestTimeout = 5000;
  server.headersTimeout = 5000;
  server.timeout = 5000;
  server.maxConnections = 256;
  return server;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const port = Number(process.env.ABBEY_COURT_PORT || 8790);
  if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new Error('Invalid court port');
  const server = createCourtServer();
  server.requestTimeout = 5000;
  server.listen(port, '127.0.0.1', () => console.log(`Abbey court listening on http://127.0.0.1:${port}`));
  for (const signal of ['SIGTERM', 'SIGINT']) process.on(signal, () => server.close());
}
