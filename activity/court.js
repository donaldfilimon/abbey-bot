const proposals = [
  'Replace every meeting with a dramatic courtroom objection.',
  'Give the compiler a tiny judge wig. Errors become legally binding.',
  'Rename production to probably-fine and see if morale improves.',
  'Make the office printer CEO. It already refuses most requests.',
  'Require rubber ducks to sign an NDA before debugging.',
  'Replace the loading spinner with a raccoon doing its best.',
  'Ship a feature that only works when someone says please.',
  'Pay technical debt in arcade tickets.',
];
const verdicts = ['Approved. The court has misplaced its common sense.', 'Rejected. Even the raccoon had concerns.', 'Hung jury. Please consult a second rubber duck.'];
const panel = document.getElementById('court');
const prompt = document.getElementById('proposal');
const tally = document.getElementById('tally');
const connection = document.getElementById('court-connection');
const buttons = [...panel.querySelectorAll('[data-vote]')];
const params = new URLSearchParams(location.search);
const embedded = window.parent !== window;
// Discord's instance identifier isolates a shared Activity. Plain browsers use
// an explicit invite code. These are anonymous games, never identity evidence.
function deriveCourtRoom({ instanceId, previewRoom }) {
  const key = embedded ? instanceId : previewRoom;
  if (typeof key !== 'string' || !/^[A-Za-z0-9_-]{8,120}$/.test(key)) return null;
  return `${embedded ? 'discord' : 'browser'}-${key}`;
}
let previewRoom = params.get('room');
if (!previewRoom && !embedded) {
  previewRoom = crypto.randomUUID();
  params.set('room', previewRoom);
  history.replaceState(null, '', `${location.pathname}?${params}`);
}
let room = deriveCourtRoom({ instanceId: window.__abbeyActivity?.instanceId, previewRoom });
let player = crypto.randomUUID();
let current;
let pollPending = false;
const pending = new Set();
let generation = 0;
let authority = null;
let restarted = false;
const nextButton = document.getElementById('next-case');
let localRound = 0;
let offline = false;
const api = embedded ? '/.proxy/court' : './court';
function render(state) {
  current = state;
  prompt.textContent = proposals[state.round % proposals.length];
  document.getElementById('round').textContent = `CASE ${state.round + 1}`;
  const total = state.yes + state.no;
  const verdict = !total ? 'The court awaits your questionable judgment.' : verdicts[state.yes === state.no ? 2 : state.yes > state.no ? 0 : 1];
  tally.textContent = `${state.yes} approve · ${state.no} object. ${verdict}`;
  buttons.forEach(b => { b.disabled = pending.has('vote'); b.setAttribute('aria-pressed', String(state.vote === b.dataset.vote)); });
}
function validateSnapshot(state) {
  if (!state || state.protocol !== 2 || typeof state.epoch !== 'string'
      || !/^[A-Za-z0-9_-]{16,64}$/.test(state.epoch)
      || !Number.isSafeInteger(state.revision) || state.revision < 0
      || !Number.isSafeInteger(state.round) || state.round < 0
      || !Number.isInteger(state.yes) || state.yes < 0 || state.yes > 100
      || !Number.isInteger(state.no) || state.no < 0 || state.no > 100
      || state.yes + state.no > 100 || ![null, 'yes', 'no'].includes(state.vote)) return null;
  return state;
}
function adoptSnapshot(state, requestGeneration) {
  if (requestGeneration !== generation) return false;
  if (authority?.epoch === state.epoch && state.revision < authority.revision) return false;
  if (authority && authority.epoch !== state.epoch) {
    generation++;
    restarted = true;
  }
  authority = state;
  render(state);
  offline = false;
  connection.textContent = `${restarted ? 'Room restarted · ' : ''}Shared court connected · anonymous browser ballots · no chat or audio captured`;
  return true;
}
async function request(action, vote) {
  const response = await fetch(api, {
    signal: AbortSignal.timeout(5000),
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ room, player, action, epoch: authority?.epoch, round: authority?.round, vote }),
  });
  // Refresh authority without ever replaying a stale or rate-limited mutation.
  if ((response.status === 409 || response.status === 429) && action !== 'read') return request('read');
  if (!response.ok) throw new Error('Court unavailable');
  const state = validateSnapshot(await response.json());
  if (!state) throw new Error('Invalid shared court state');
  return state;
}
function rehearse(action, vote) {
  if (!offline) {
    localRound = current?.round ?? 0;
    render({ round: localRound, yes: 0, no: 0, vote: null });
  }
  offline = true;
  connection.textContent = 'Solo rehearsal · shared court server unavailable. Votes are only on this screen.';
  if (action === 'read') return;
  if (action === 'next') localRound = localRound === Number.MAX_SAFE_INTEGER ? 0 : localRound + 1;
  const state = { round: localRound, yes: 0, no: 0, vote: null };
  if (action === 'vote') { state[vote] = 1; state.vote = vote; }
  render(state);
}
async function update(action = 'read', vote) {
  if (action === 'read' ? pollPending || document.hidden : pending.has(action)) return;
  if (embedded && !window.__abbeyActivity?.isReady()) {
    connection.textContent = 'Waiting for Discord’s Activity connection. Shared ballots start after READY.';
    buttons.forEach(b => { b.disabled = true; });
    nextButton.disabled = true;
    return;
  }
  if (!room || !/^[A-Za-z0-9_-]{8,128}$/.test(room)) {
    connection.textContent = 'Discord did not provide a valid shared instance. Relaunch the Activity.';
    buttons.forEach(b => { b.disabled = true; });
    nextButton.disabled = true;
    return;
  }
  if (action === 'read') pollPending = true;
  else pending.add(action);
  buttons.forEach(b => { b.disabled = pending.has('vote'); });
  nextButton.disabled = pending.has('next');
  const requestGeneration = generation;
  try {
    if (action !== 'read' && offline) {
      if (adoptSnapshot(await request('read'), requestGeneration)) {
        connection.textContent += ' · Reconnected: choose your ballot or next case again.';
      }
      return;
    }
    if (action !== 'read' && !authority) { rehearse(action, vote); return; }
    adoptSnapshot(await request(action, vote), requestGeneration);
  } catch {
    if (requestGeneration === generation) rehearse(action, vote);
  } finally {
    if (action === 'read') pollPending = false;
    else pending.delete(action);
    buttons.forEach(b => { b.disabled = pending.has('vote'); });
    nextButton.disabled = pending.has('next');
  }
}
buttons.forEach(b => b.addEventListener('click', () => update('vote', b.dataset.vote)));
document.getElementById('next-case').addEventListener('click', () => update('next'));
document.getElementById('court-invite').addEventListener('click', async () => {
  if (embedded) {
    connection.textContent = 'Invite friends using Discord’s Activity invitation controls.';
    return;
  }
  try { await navigator.clipboard.writeText(location.href); connection.textContent = 'Court link copied. Open it in another browser to join the same room.'; }
  catch { connection.textContent = 'Copy the address from your browser to invite another player.'; }
});
render({ round: 0, yes: 0, no: 0, vote: null });
update();
setInterval(() => update(), 2500);
document.addEventListener('visibilitychange', () => { if (!document.hidden) return update(); });
