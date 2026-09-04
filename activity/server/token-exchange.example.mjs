#!/usr/bin/env node
/**
 * Example OAuth2 token exchange for Abbey Embedded Activity (P2).
 *
 * NOT deployed by default. GitHub Pages cannot hold DISCORD_CLIENT_SECRET.
 * Run this (or an equivalent route on your bot host) behind operator env, then
 * add a Developer Portal URL mapping PREFIX so the Activity iframe can reach
 * it via Discord's `/.proxy/...` path — or serve Activity static + API same-origin.
 *
 * Required env (never commit):
 *   DISCORD_CLIENT_ID       default 1147940171099152464 (public)
 *   DISCORD_CLIENT_SECRET   Application Client Secret from Developer Portal
 *
 * Endpoints:
 *   GET  /api/token/health  → { ok: true }  (client probes this before authorize)
 *   POST /api/token         body: { code }  → { access_token }
 *
 * Discord token URL: https://discord.com/api/oauth2/token
 * Docs: docs/activities.md § P2, docs/discord-application-api-roadmap.md § P2
 */
import http from 'node:http';
import { URL } from 'node:url';

const PORT = Number(process.env.PORT || 8787);
const CLIENT_ID = process.env.DISCORD_CLIENT_ID || '1147940171099152464';
const CLIENT_SECRET = process.env.DISCORD_CLIENT_SECRET || '';

function send(res, status, body) {
  const json = JSON.stringify(body);
  res.writeHead(status, {
    'Content-Type': 'application/json',
    'Cache-Control': 'no-store',
  });
  res.end(json);
}

function readJson(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => {
      try {
        const raw = Buffer.concat(chunks).toString('utf8') || '{}';
        resolve(JSON.parse(raw));
      } catch (err) {
        reject(err);
      }
    });
    req.on('error', reject);
  });
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url || '/', `http://${req.headers.host || 'localhost'}`);

  if (req.method === 'GET' && url.pathname === '/api/token/health') {
    return send(res, 200, {
      ok: Boolean(CLIENT_SECRET),
      client_id_configured: Boolean(CLIENT_ID),
      secret_configured: Boolean(CLIENT_SECRET),
    });
  }

  if (req.method === 'POST' && url.pathname === '/api/token') {
    if (!CLIENT_SECRET) {
      return send(res, 503, {
        error: 'DISCORD_CLIENT_SECRET not set in operator env',
      });
    }
    let body;
    try {
      body = await readJson(req);
    } catch {
      return send(res, 400, { error: 'invalid JSON' });
    }
    const code = body && body.code;
    if (!code || typeof code !== 'string') {
      return send(res, 400, { error: 'missing code' });
    }

    const form = new URLSearchParams({
      client_id: CLIENT_ID,
      client_secret: CLIENT_SECRET,
      grant_type: 'authorization_code',
      code,
    });

    try {
      const tokenRes = await fetch('https://discord.com/api/oauth2/token', {
        method: 'POST',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body: form.toString(),
      });
      const tokenBody = await tokenRes.json();
      if (!tokenRes.ok) {
        return send(res, 502, {
          error: 'discord_token_exchange_failed',
          status: tokenRes.status,
          details: tokenBody,
        });
      }
      // Return only what the Activity client needs. Do not log the secret or code.
      return send(res, 200, {
        access_token: tokenBody.access_token,
        expires_in: tokenBody.expires_in,
        token_type: tokenBody.token_type,
        scope: tokenBody.scope,
      });
    } catch (err) {
      return send(res, 502, {
        error: 'token_exchange_network_error',
        message: err && err.message ? err.message : String(err),
      });
    }
  }

  send(res, 404, { error: 'not_found' });
});

server.listen(PORT, () => {
  console.log(
    `[abbey-token-exchange] listening on :${PORT} (secret_configured=${Boolean(
      CLIENT_SECRET,
    )})`,
  );
});
