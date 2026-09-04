/**
 * Abbey Embedded Activity client (same-origin Pages bundle).
 *
 * Source of truth for a full SDK rebuild: ./src/main.js
 * (@discord/embedded-app-sdk). This file ships without a bundler so the
 * Activity iframe needs no extra CDN / PREFIX mapping.
 *
 * P2:
 *  - ready() handshake (opcode 0)
 *  - channel + guild from Discord-injected query params (pre-auth)
 *  - GET_ACTIVITY_INSTANCE_CONNECTED_PARTICIPANTS +
 *    ACTIVITY_INSTANCE_PARTICIPANTS_UPDATE after ready (no OAuth scopes)
 *  - truthful local mode: idle | waiting (never invents Go Live)
 *  - optional authorize -> token exchange -> authenticate when a server-side
 *    exchange is mapped (health check or ?oauth=1). See docs/activities.md.
 *  - getChannel + setActivity after successful authenticate
 *
 * No Client Secret here. Application ID is public.
 */
(function () {
  var CLIENT_ID = '1147940171099152464';
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
    if (!guildId) return channelId ? 'DM / group DM (no guild)' : '\u2014';
    var known = KNOWN_GUILDS[guildId];
    return known ? known + ' (' + guildId + ')' : guildId;
  }

  function renderContext() {
    setText(els.channel, formatChannel());
    setText(els.guild, formatGuild());
  }

  function renderParticipants() {
    var n = state.participants ? state.participants.length : 0;
    if (!n) {
      setText(els.participants, '0 participants');
      return;
    }
    var names = state.participants.map(function (p) {
      return p.global_name || p.username || p.id || 'member';
    });
    setText(els.participants, n + ' \u2014 ' + names.join(', '));
  }

  function nextNonce() {
    state.nonce += 1;
    return 'abbey-' + state.nonce;
  }

  function sendCommand(cmd, args, evt) {
    return new Promise(function (resolve, reject) {
      if (!source) {
        reject(new Error('No Discord parent frame'));
        return;
      }
      var nonce = nextNonce();
      state.pending[nonce] = { resolve: resolve, reject: reject };
      var frame = { cmd: cmd, args: args || {}, nonce: nonce };
      if (evt) frame.evt = evt;
      source.postMessage([1, frame], '*');
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
    if (
      payload.evt === 'ACTIVITY_INSTANCE_PARTICIPANTS_UPDATE' &&
      payload.data
    ) {
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

  async function syncParticipants() {
    try {
      var part = await sendCommand(
        'GET_ACTIVITY_INSTANCE_CONNECTED_PARTICIPANTS',
        {}
      );
      state.participants = (part && part.participants) || [];
      renderParticipants();
    } catch (err) {
      setText(
        els.participants,
        'Participants unavailable (' +
          ((err && err.message) || 'rpc') +
          ')'
      );
    }
    try {
      await sendCommand(
        'SUBSCRIBE',
        {},
        'ACTIVITY_INSTANCE_PARTICIPANTS_UPDATE'
      );
    } catch (_) {
      /* optional */
    }
  }

  async function applySetActivity() {
    try {
      await sendCommand('SET_ACTIVITY', {
        activity: {
          type: 0,
          details: 'Abbey \u00b7 Intelligence Without Limits',
          state:
            state.mode === 'waiting'
              ? 'Waiting for voice context'
              : 'Idle in voice Activity',
        },
      });
    } catch (_) {
      /* needs rpc.activities.write after authorize */
    }
  }

  async function tryAuthorizeFlow(tokenPath) {
    setText(els.auth, 'Attempting authorize\u2026');
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

      setText(els.auth, 'Exchanging code (server-side secret)\u2026');
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

      await applySetActivity();
      setText(els.status, 'Abbey is ready in this voice Activity.');
    } catch (err) {
      state.authenticated = false;
      setText(
        els.auth,
        'Pre-auth context only \u2014 OAuth/token exchange failed'
      );
      setText(
        els.hint,
        'Token path was selected but auth failed: ' +
          ((err && err.message) || String(err)) +
          '. Channel/guild IDs and participant count still come from the Activity iframe (no OAuth required for those).'
      );
    }
  }

  async function onReady() {
    if (state.ready) return;
    state.ready = true;
    setText(els.status, 'Abbey is ready in this voice Activity.');
    renderContext();
    setMode(channelId ? 'idle' : 'waiting');
    setText(els.auth, 'READY \u2014 pre-auth context');

    // Participants require no OAuth scopes (Embedded App SDK docs).
    await syncParticipants();

    var tokenPath = await tokenEndpointReady();
    if (!tokenPath) {
      setText(
        els.hint,
        'Channel/guild + participant count are live without OAuth. Map a token exchange host and open with ?oauth=1 (or serve /api/token/health) to enable authorize, getChannel name, and setActivity. Secret stays in operator env \u2014 see activity/server/token-exchange.example.mjs.'
      );
      return;
    }
    await tryAuthorizeFlow(tokenPath);
  }

  renderContext();
  setMode('waiting');
  setText(els.auth, 'Connecting\u2026');
  setText(els.participants, '\u2014');

  if (!source) {
    setText(
      els.status,
      'Abbey \u2014 open from the Discord rocket in a voice channel (plain browser tabs have no Embedded App parent).'
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
