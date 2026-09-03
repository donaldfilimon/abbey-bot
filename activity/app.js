/**
 * Same-origin Activity bootstrap.
 *
 * Source of truth for a bundled rebuild is ./src/main.js using
 * @discord/embedded-app-sdk (see package.json). This file implements the
 * SDK's ready() handshake only — no authorize(), no client secret.
 *
 * Protocol: postMessage [opcode, payload] to the Discord parent, opcode 0 =
 * HANDSHAKE, then wait for evt READY. Client ID is the public application id.
 */
(function () {
  var CLIENT_ID = '1147940171099152464';
  var statusEl = document.getElementById('status');
  var params = new URLSearchParams(window.location.search);
  var frameId = params.get('frame_id') || '';
  var source = window.parent === window ? null : window.parent;

  function setStatus(text) {
    if (statusEl) statusEl.textContent = text;
  }

  setStatus('Abbey');

  if (!source) {
    setStatus('Abbey — open this page from the Discord rocket in a voice channel (plain browser tabs have no Embedded App parent).');
    return;
  }

  function onMessage(event) {
    var data = event.data;
    var payload = data;
    if (Array.isArray(data)) {
      payload = data[1];
    }
    if (!payload || typeof payload !== 'object') {
      return;
    }
    var evt = payload.evt || (payload.cmd === 'DISPATCH' ? payload.evt : null);
    if (evt === 'READY') {
      window.removeEventListener('message', onMessage);
      setStatus('Abbey is ready in this voice Activity.');
    }
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
})();
