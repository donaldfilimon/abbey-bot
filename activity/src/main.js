/**
 * Preferred rebuild path using @discord/embedded-app-sdk.
 *
 * Pages currently serves the hand-maintained ../app.js (no bundler / no CDN
 * PREFIX). Keep both in sync for P2 behavior:
 *  - ready()
 *  - pre-auth channel/guild from SDK URL context
 *  - authorize + server token exchange + authenticate when endpoint is live
 *  - getChannel, participants subscribe, truthful setActivity
 *
 * Rebuild later with esbuild/vite into ../app.js when you want the official
 * SDK package in production. Never put DISCORD_CLIENT_SECRET in client code.
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

const discordSdk = new DiscordSDK(CLIENT_ID);
const params = new URLSearchParams(window.location.search);
const forceOauth = params.get('oauth') === '1';

function formatChannel(channelId, name) {
  if (!channelId) return 'Not in an embedded Activity frame';
  if (name) return `${name} (${channelId})`;
  const known = KNOWN_CHANNELS[channelId];
  return known ? `${known} (${channelId})` : channelId;
}

function formatGuild(guildId) {
  if (!guildId) return discordSdk.channelId ? 'DM / group DM (no guild)' : '—';
  const known = KNOWN_GUILDS[guildId];
  return known ? `${known} (${guildId})` : guildId;
}

function renderPreAuth() {
  setText(els.channel, formatChannel(discordSdk.channelId, null));
  setText(els.guild, formatGuild(discordSdk.guildId));
  setText(
    els.participants,
    'Subscribe after OAuth token exchange (see docs/activities.md § P2)',
  );
  setText(els.auth, 'READY — pre-auth context');
  setMode(discordSdk.channelId ? 'idle' : 'waiting');
}

async function tokenEndpointReady() {
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

async function setupAuthenticated(tokenPath) {
  setText(els.auth, 'Attempting authorize…');
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

  setText(els.auth, 'Exchanging code (server-side secret)…');
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
  setText(els.guild, formatGuild(discordSdk.guildId));

  const updateParticipants = (participants) => {
    if (!participants?.length) {
      setText(els.participants, 'No other participants yet');
      return;
    }
    setText(
      els.participants,
      participants
        .map((p) => p.global_name || p.username || p.id)
        .join(', '),
    );
  };

  try {
    const { participants } =
      await discordSdk.commands.getInstanceConnectedParticipants();
    updateParticipants(participants);
  } catch {
    setText(els.participants, 'Participants unavailable');
  }

  try {
    await discordSdk.subscribe(
      'ACTIVITY_INSTANCE_PARTICIPANTS_UPDATE',
      ({ participants }) => updateParticipants(participants),
    );
  } catch {
    /* optional */
  }

  // Truthful mode only — never invent a bot Go Live / stream.
  const mode = discordSdk.channelId ? 'idle' : 'waiting';
  setMode(mode);
  try {
    await discordSdk.commands.setActivity({
      activity: {
        type: 0,
        details: 'Abbey · Intelligence Without Limits',
        state: mode === 'waiting' ? 'Waiting' : 'Idle in voice Activity',
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
      'Abbey — open from the Discord rocket in a voice channel (plain browser tabs have no Embedded App parent).',
    );
    setText(els.auth, 'No Discord parent');
    setMode('waiting');
    return;
  }

  await discordSdk.ready();
  setText(els.status, 'Abbey is ready in this voice Activity.');
  renderPreAuth();

  const tokenPath = await tokenEndpointReady();
  if (!tokenPath) {
    setText(
      els.hint,
      'Pre-auth channel/guild context is live. Map a token exchange host and open with ?oauth=1 (or serve /api/token/health) to enable authorize, participants, and setActivity. Secret stays in operator env — see activity/server/token-exchange.example.mjs.',
    );
    return;
  }

  try {
    await setupAuthenticated(tokenPath);
  } catch (err) {
    renderPreAuth();
    setText(
      els.auth,
      'Pre-auth context only — OAuth/token exchange failed',
    );
    setText(
      els.hint,
      `Token path ${tokenPath} was reachable but auth failed: ${
        err?.message || String(err)
      }`,
    );
  }
}

main();
