/**
 * Preferred rebuild path using @discord/embedded-app-sdk.
 *
 * Pages currently serves the hand-maintained ../app.js (no bundler / no CDN
 * PREFIX). Keep both in sync for P2 behavior:
 *  - ready()
 *  - pre-auth channel/guild from SDK URL context
 *  - getInstanceConnectedParticipants + ACTIVITY_INSTANCE_PARTICIPANTS_UPDATE
 *    after ready (no OAuth scopes required)
 *  - authorize + server token exchange + authenticate when endpoint is live
 *  - getChannel + truthful setActivity after authenticate
 *
 * Rebuild with a bundler into ../app.js when you want the official SDK package
 * in production. Never put DISCORD_CLIENT_SECRET in client code.
 */
import { DiscordSDK } from '@discord/embedded-app-sdk';

const CLIENT_ID = '1147940171099152464';
const KNOWN_CHANNELS = {
  '1495755277859815595': 'Office Hours',
};
const KNOWN_GUILDS = {
  '1275617641620443146': 'MLAI Community',
};
const TOKEN_PATHS = ['/.proxy/api/token', './api/token', '/api/token'];

const els = {
  status: document.getElementById('status'),
  mode: document.getElementById('mode'),
  channel: document.getElementById('channel'),
  guild: document.getElementById('guild'),
  participants: document.getElementById('participants'),
  auth: document.getElementById('auth'),
  hint: document.getElementById('hint'),
};

function setText(el, text) {
  if (el) el.textContent = text;
}

function setMode(mode) {
  setText(els.mode, mode);
  if (els.mode) {
    els.mode.classList.remove('mode-idle', 'mode-waiting');
    els.mode.classList.add(mode === 'idle' ? 'mode-idle' : 'mode-waiting');
  }
}

function formatChannel(channelId, name) {
  if (!channelId) return 'Not in an embedded Activity frame';
  if (name) return `${name} (${channelId})`;
  const known = KNOWN_CHANNELS[channelId];
  return known ? `${known} (${channelId})` : channelId;
}

function formatGuild(guildId, channelId) {
  if (!guildId) return channelId ? 'DM / group DM (no guild)' : '\u2014';
  const known = KNOWN_GUILDS[guildId];
  return known ? `${known} (${guildId})` : guildId;
}

function renderParticipants(participants) {
  const list = participants || [];
  const n = list.length;
  if (!n) {
    setText(els.participants, '0 participants');
    return;
  }
  const names = list.map((p) => p.global_name || p.username || p.id || 'member');
  setText(els.participants, `${n} \u2014 ${names.join(', ')}`);
}

async function tokenEndpointReady(forceOauth) {
  if (forceOauth) return TOKEN_PATHS[0];
  for (const path of TOKEN_PATHS) {
    try {
      const res = await fetch(path.replace(/\/token$/, '/token/health'), {
        method: 'GET',
        headers: { Accept: 'application/json' },
      });
      if (res.ok) {
        const body = await res.json().catch(() => ({}));
        if (body && body.ok) return path;
      }
    } catch {
      /* try next */
    }
  }
  return null;
}

async function exchangeCode(code, tokenPath) {
  const res = await fetch(tokenPath, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ code }),
  });
  if (!res.ok) throw new Error(`token HTTP ${res.status}`);
  const body = await res.json();
  if (!body.access_token) throw new Error('no access_token');
  return body.access_token;
}

async function syncParticipants(discordSdk) {
  try {
    const { participants } =
      await discordSdk.commands.getInstanceConnectedParticipants();
    renderParticipants(participants);
  } catch (err) {
    setText(
      els.participants,
      `Participants unavailable (${err?.message || 'rpc'})`,
    );
  }
  try {
    await discordSdk.subscribe(
      'ACTIVITY_INSTANCE_PARTICIPANTS_UPDATE',
      ({ participants }) => renderParticipants(participants),
    );
  } catch {
    /* optional */
  }
}

async function setupAuthenticated(discordSdk, tokenPath) {
  setText(els.auth, 'Attempting authorize\u2026');
  const { code } = await discordSdk.commands.authorize({
    client_id: CLIENT_ID,
    response_type: 'code',
    state: '',
    prompt: 'none',
    scope: [
      'identify',
      'guilds',
      'applications.commands',
      'rpc.activities.write',
    ],
  });

  setText(els.auth, 'Exchanging code (server-side secret)\u2026');
  const access_token = await exchangeCode(code, tokenPath);
  const auth = await discordSdk.commands.authenticate({ access_token });
  setText(
    els.auth,
    auth?.user?.username
      ? `Authenticated as ${auth.user.username}`
      : 'Authenticated',
  );

  let channelName = null;
  if (discordSdk.channelId != null && discordSdk.guildId != null) {
    try {
      const channel = await discordSdk.commands.getChannel({
        channel_id: discordSdk.channelId,
      });
      channelName = channel?.name ?? null;
    } catch {
      /* keep pre-auth label */
    }
  }
  setText(els.channel, formatChannel(discordSdk.channelId, channelName));
  setText(els.guild, formatGuild(discordSdk.guildId, discordSdk.channelId));

  const mode = discordSdk.channelId ? 'idle' : 'waiting';
  setMode(mode);
  // Truthful mode only — never invent a bot Go Live / stream.
  try {
    await discordSdk.commands.setActivity({
      activity: {
        type: 0,
        details: 'Abbey \u00b7 Intelligence Without Limits',
        state:
          mode === 'waiting'
            ? 'Waiting for voice context'
            : 'Idle in voice Activity',
      },
    });
  } catch {
    /* needs rpc.activities.write */
  }
}

async function main() {
  if (window.parent === window) {
    setText(
      els.status,
      'Abbey \u2014 open from the Discord rocket in a voice channel (plain browser tabs have no Embedded App parent).',
    );
    setText(els.auth, 'No Discord parent');
    setMode('waiting');
    return;
  }

  const params = new URLSearchParams(window.location.search);
  const forceOauth = params.get('oauth') === '1';

  let discordSdk;
  try {
    discordSdk = new DiscordSDK(CLIENT_ID);
  } catch (err) {
    setText(
      els.status,
      'Abbey \u2014 open from the Discord rocket (SDK needs frame_id / instance_id / platform).',
    );
    setText(els.auth, err?.message || 'SDK init failed');
    setMode('waiting');
    return;
  }

  await discordSdk.ready();
  setText(els.status, 'Abbey is ready in this voice Activity.');
  setText(els.channel, formatChannel(discordSdk.channelId, null));
  setText(els.guild, formatGuild(discordSdk.guildId, discordSdk.channelId));
  setMode(discordSdk.channelId ? 'idle' : 'waiting');
  setText(els.auth, 'READY \u2014 pre-auth context');

  await syncParticipants(discordSdk);

  const tokenPath = await tokenEndpointReady(forceOauth);
  if (!tokenPath) {
    setText(
      els.hint,
      'Channel/guild + participant count are live without OAuth. Map a token exchange host and open with ?oauth=1 (or serve /api/token/health) to enable authorize, getChannel name, and setActivity. Secret stays in operator env \u2014 see activity/server/token-exchange.example.mjs.',
    );
    return;
  }

  try {
    await setupAuthenticated(discordSdk, tokenPath);
  } catch (err) {
    setText(els.auth, 'Pre-auth context only \u2014 OAuth/token exchange failed');
    setText(
      els.hint,
      `Token path ${tokenPath} was reachable but auth failed: ${
        err?.message || String(err)
      }`,
    );
  }
}

main();
