/**
 * Abbey Embedded Activity client (same-origin Pages bundle).
 *
 * Source of truth for a full SDK rebuild: ./src/main.js
 * (@discord/embedded-app-sdk). This file ships without a bundler so the
 * Activity iframe needs no extra CDN / PREFIX mapping.
 *
 * P2 slice:
 *  - ready() handshake (opcode 0)
 *  - channel + guild context from Discord-injected query params (pre-auth)
 *  - truthful local mode: idle | waiting (never invents Go Live)
 *  - optional authorize → token exchange → authenticate when a server-side
 *    exchange is mapped (health check or ?oauth=1). See docs/activities.md.
 *  - participants + setActivity after successful authenticate
 *
 * No Client Secret here. Application ID is public.
 */
(function () {
  var CLIENT_ID = '1147940171099152464';
  /** Public guild channel labels we already document — display only. */
  var KNOWN_CHANNELS = {
    '1495755277859815595': 'Office Hours',
  };
  var KNOWN_GUILDS = {
    '1275617641620443146': 'MLAI Community',
  };

  var TOKEN_PATHS = [
    '/.proxy/api/token',
    './api/token',
    '/api/token',
  ];

  var els = {
    status: document.getElementById('status'),
    mode: document.getElementById('mode'),
    channel: document.getElementById('channel'),
    guild: document.getElementById('guild'),
    participants: document.getElementById('participants'),
    auth: document.getElementById('auth'),
    hint: document.getElementById('hint'),
  };

  var params = new URLSearchParams(window.location.search);
  var frameId = params.get('frame_id') || '';
  var channelId = params.get('channel_id') || '';
  var guildId = params.get('guild_id') || '';
  var instanceId = params.get('instance_id') || '';
  var forceOauth = params.get('oauth') === '1';
  var source = window.parent === window ? null : window.parent;

  var state = {
    mode: 'waiting',
    ready: false,
    authenticated: false,
    channelName: null,
    participants: [],
    nonce: 0,
    pending: Object.create(null),
  };

  function setText(el, text) {
    if (el) el.textContent = text;
  }

  function setMode(mode) {
    state.mode = mode;
    setText(els.mode, mode);
    if (els.mode) {
      els.mode.classList.remove('mode-idle', 'mode-waiting');
      els.mode.classList.add(mode === 'idle' ? 'mode-idle' : 'mode-waiting');
    }
  }

  function formatChannel() {
    if (!channelId) return 'Not in an embedded Activity frame';
    if (state.channelName) return state.channelName + ' (' + channelId + ')';
    var known = KNOWN_CHANNELS[channelId];
    if (known) return known + ' (' + channelId + ')';
    return channelId;
  }

  function formatGuild() {
    if (!guildId) return channelId ? 'DM / group DM (no guild)' : '—';
    var known = KNOWN_GUILDS[guildId];
    return known ? known + ' (' + guildId + ')' : guildId;
  }

  function renderContext() {
    setText(els.channel, formatChannel());
    setText(els.guild, formatGuild());
  }

  function renderParticipants() {
    if (!state.authenticated) {
      setText(
        els.participants,
        'Subscribe after OAuth token exchange (see docs/activities.md § P2)'
      );
      return;
    }
    if (!state.participants.length) {
      setText(els.participants, 'No other participants yet');
      return;
    }
    var names = state.participants.map(function (p) {
      return p.global_name || p.username || p.id || 'member';
    });
    setText(els.participants, names.join(', '));
  }

  function nextNonce() {
    state.nonce += 1;
    return 'abbey-' + state.nonce;
  }

  function sendCommand(cmd, args) {
    return new Promise(function (resolve, reject) {
      if (!source) {
        reject(new Error('No Discord parent frame'));
        return;
      }
      var nonce = nextNonce();
      state.pending[nonce] = { resolve: resolve, reject: reject };
      source.postMessage(
        [1, { cmd: cmd, args: args || {}, nonce: nonce }],
        '*'
      );
      setTimeout(function () {
        if (state.pending[nonce]) {
          delete state.pending[nonce];
          reject(new Error('RPC timeout: ' + cmd));
        }
      }, 8000);
    });
  }

  function handleFrameMessage(payload) {
    if (!payload || typeof payload !== 'object') return;

    if (payload.nonce && state.pending[payload.nonce]) {
      var pending = state.pending[payload.nonce];
      delete state.pending[payload.nonce];
      if (payload.evt === 'ERROR') {
        pending.reject(
          new Error((payload.data && payload.data.message) || 'RPC error')
        );
        return;
      }
      pending.resolve(payload.data);
      return;
    }

    if (payload.evt === 'READY') {
      onReady();
      return;
    }
    if (payload.evt === 'ACTIVITY_INSTANCE_PARTICIPANTS_UPDATE' && payload.data) {
      state.participants = payload.data.participants || [];
      renderParticipants();
    }
  }

  function onMessage(event) {
    var data = event.data;
    var payload = Array.isArray(data) ? data[1] : data;
    handleFrameMessage(payload);
  }

  function healthPathFor(tokenPath) {
    if (tokenPath.slice(-6) === '/token') {
      return tokenPath.slice(0, -6) + '/token/health';
    }
    return tokenPath + '/health';
  }

  async function tokenEndpointReady() {
    if (forceOauth) return TOKEN_PATHS[0];
    for (var i = 0; i < TOKEN_PATHS.length; i++) {
      var path = TOKEN_PATHS[i];
      try {
        var res = await fetch(healthPathFor(path), {
          method: 'GET',
          headers: { Accept: 'application/json' },
        });
        if (!res.ok) continue;
        var body = await res.json().catch(function () {
          return {};
        });
        if (body && body.ok) return path;
      } catch (_) {
        /* try next */
      }
    }
    return null;
  }

  async function tryTokenExchange(code, tokenPath) {
    var res = await fetch(tokenPath, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ code: code }),
    });
    if (!res.ok) throw new Error('token HTTP ' + res.status + ' at ' + tokenPath);
    var body = await res.json();
    if (!body || !body.access_token) throw new Error('no access_token');
    return body.access_token;
  }

  async function tryAuthorizeFlow(tokenPath) {
    setText(els.auth, 'Attempting authorize…');
    try {
      var authz = await sendCommand('AUTHORIZE', {
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
      var code = authz && authz.code;
      if (!code) throw new Error('authorize returned no code');

      setText(els.auth, 'Exchanging code (server-side secret)…');
      var accessToken = await tryTokenExchange(code, tokenPath);

      var auth = await sendCommand('AUTHENTICATE', {
        access_token: accessToken,
      });
      state.authenticated = true;
      setText(
        els.auth,
        'Authenticated' +
          (auth && auth.user && auth.user.username
            ? ' as ' + auth.user.username
            : '')
      );

      await enrichAfterAuth();
    } catch (err) {
      state.authenticated = false;
      setText(
        els.auth,
        'Pre-auth context only — OAuth/token exchange failed'
      );
      setText(
        els.hint,
        'Token path was selected but auth failed: ' +
          ((err && err.message) || String(err)) +
          '. Channel/guild IDs still come from the Activity iframe.'
      );
      renderParticipants();
      setMode(channelId ? 'idle' : 'waiting');
    }
  }

  async function enrichAfterAuth() {
    if (channelId && guildId) {
      try {
        var channel = await sendCommand('GET_CHANNEL', {
          channel_id: channelId,
        });
        if (channel && channel.name) {
          state.channelName = channel.name;
          renderContext();
        }
      } catch (_) {
        /* keep pre-auth label */
      }
    }

    try {
      var part = await sendCommand('GET_INSTANCE_CONNECTED_PARTICIPANTS', {});
      state.participants = (part && part.participants) || [];
      renderParticipants();
    } catch (_) {
      renderParticipants();
    }

    try {
      await sendCommand('SUBSCRIBE', {
        evt: 'ACTIVITY_INSTANCE_PARTICIPANTS_UPDATE',
      });
    } catch (_) {
      /* optional */
    }

    try {
      await sendCommand('SET_ACTIVITY', {
        activity: {
          type: 0,
          details: 'Abbey · Intelligence Without Limits',
          state:
            state.mode === 'waiting' ? 'Waiting' : 'Idle in voice Activity',
        },
      });
    } catch (_) {
      /* rpc.activities.write may be missing until re-consent */
    }

    setMode('idle');
    setText(els.status, 'Abbey is ready in this voice Activity.');
  }

  async function onReady() {
    if (state.ready) return;
    state.ready = true;
    setText(els.status, 'Abbey is ready in this voice Activity.');
    renderContext();
    setMode(channelId ? 'idle' : 'waiting');
    setText(els.auth, 'READY — pre-auth context');
    renderParticipants();

    var tokenPath = await tokenEndpointReady();
    if (!tokenPath) {
      setText(
        els.hint,
        'Pre-auth channel/guild context is live. Map a token exchange host and open with ?oauth=1 (or serve /api/token/health) to enable authorize, participants, and setActivity. Secret stays in operator env — see activity/server/token-exchange.example.mjs.'
      );
      return;
    }
    await tryAuthorizeFlow(tokenPath);
  }

  // --- boot ---
  renderContext();
  setMode('waiting');
  setText(els.auth, 'Connecting…');
  renderParticipants();

  if (!source) {
    setText(
      els.status,
      'Abbey — open from the Discord rocket in a voice channel (plain browser tabs have no Embedded App parent).'
    );
    setText(els.auth, 'No Discord parent');
    setMode('waiting');
    return;
  }

  window.addEventListener('message', onMessage);
  source.postMessage(
    [
      0,
      {
        v: 1,
        encoding: 'json',
        client_id: CLIENT_ID,
        frame_id: frameId,
      },
    ],
    '*'
  );

  window.__abbeyActivity = {
    clientId: CLIENT_ID,
    channelId: channelId,
    guildId: guildId,
    instanceId: instanceId,
    getMode: function () {
      return state.mode;
    },
    isAuthenticated: function () {
      return state.authenticated;
    },
  };
})();
