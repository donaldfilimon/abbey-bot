# MLAI Voice Completion Design

**Status:** Proposed. Planning authorized; implementation/publication/operator actions are not authorized by this file. **Goal:** Prove reliable consented server voice and eliminate lifecycle ambiguity without weakening consent.

**Parent:** [2026-10-01-mlai-completion-program-design.md](2026-10-01-mlai-completion-program-design.md). **Dependencies:** Release baseline first; coordinate text telemetry semantics with text-reliability; human acceptance needs willing participants.

## Current evidence

Local synthetic Kokoro→Whisper→generation→Kokoro passed with 100% word recall. Abbey was muted/deafened in Office Hours; H2 was absent and had no Local receipt. Current source already invokes try_auto_listen_while_present from supervision on a human join. The external watcher separately restarts the bot for startup retries.

## Architecture

Use the existing consent/media epoch runtime and voice supervision. Make every preparation/listening owner observable by the existing service registry. Validate all-current roster/receipts/permissions before Decode and before unmute; keep Pass/self-deafen for presence and output-only reconnect.

## Requirements

VOICE-COMPLETION-R1: Reuse valid durable Local policy-1 receipts across visits/restarts; do not demand a new receipt solely for each media epoch. Membership, owner assertions, music and silence never grant consent.
VOICE-COMPLETION-R2: While-present activation must use the existing join hook, not a duplicate watcher. Evaluate upgrade only for PresenceOnly, explicit Local auto-listen opt-in, complete live roster coverage and current permissions; all other phases are Noop.
VOICE-COMPLETION-R3: New unattested arrival, withdrawal, permission loss or adverse current bot voice state closes admission/media before any later processing. Destroy Decode before publishing teardown. Retired session events cannot revoke a new epoch.
VOICE-COMPLETION-R4: Every preparation/upgrade worker has a retained join owner. Shutdown closes admission, cancels and observes joins before final freeze; no detached tokio::spawn remains as untracked lifecycle work.
VOICE-COMPLETION-R5: Synthetic probes write fresh operator-generated WAV only. Live verification disables conversational commits and retains no raw human audio/transcripts.
VOICE-COMPLETION-R6: Voice timing uses closed stage codes for recognition, generation and synthesis duration; no utterances or member/channel IDs enter managed JSONL. No speed claim is made before installed baseline measurement.
VOICE-COMPLETION-R7: The restart-based Office Hours watcher may be retired only after the join path and failure/reconnect matrix pass and the operator authorizes that service change. No restart loop is used to overcome missing consent.
VOICE-COMPLETION-R8: Human acceptance: participant personally agrees; current manager join/resume when required; audible wake/reply, interruption, new-arrival pause, withdrawal, reconnect, and leave are witnessed on the exact installed binary.

## Interfaces and data ownership

Preserve existing VoicePhase and runtime start-generation/media-epoch checks. Proposed pure `UpgradeFacts { presence_only, local_mode, auto_listen, nonempty_roster, all_consented, current_permissions }` feeds `decide_upgrade(UpgradeFacts) -> UpgradeDecision::{Noop,Eligible}`. Existing runtime remains the only actor that prepares, revokes and publishes phase transitions; this pure function owns no task/network.

## Global constraints

- Rust 1.98.0 stable, edition 2024; retain --locked on every Cargo command.
- Canonical abbey-bot checkout; preserve unrelated dirty work; no checkout, clone, worktree, commit, push or PR without separate authorization.
- No new production dependency, credential rotation, permission escalation or deployment configuration change without the required operator decision.
- Pure policy receives time and seeds from callers; no Discord, network, randomness or wall-clock reads in pure modules.
- Production and external test-only Rust modules remain below 1,000 lines; production modules over 800 lines require review.
- Keep explicit ToolScope; voice, unsolicited generation and summaries remain read-only.
- Every DM uses network:dm:<user_id>; scoped persistence keys use U+001F; never widen a private scope into a guild or another user.
- Canonical state is atomic abbey-state.json; wdbx.seg.0.jsonl remains a rebuildable projection; no ABI/WDBX crate dependency.
- No automatic quarantine, contradiction or resolve edges; those remain explicit administrative actions.
- Service owners close admission, join observed work, freeze and attempt final persistence at most once; cancellation request is not cleanup.
- Managed JSONL contains only closed content-free events; no prompts, replies, raw feedback, recipient IDs, channel IDs, URLs, credentials or dynamic errors.
- Voice stores no raw audio or transcripts; personal durable Local policy-1 receipts, current roster, permissions and media epoch gate processing.
- Activity Portal URL mapping and human-witnessed acceptance remain operator-gated; no second Discord gateway beside the managed service.
- Preserve pipeline guard, cooldown and hourly-budget ordering; no fallback replay after visible output or mutating/uncertain effects.
- Defer interactions before network work, clamp rendered replies, preserve classic Action Rows; Components V2 remains crate-blocked.
- Run ./check.sh for each completed source slice, python3 scripts/check-pages-liquid.py for docs/Activity, and npm --prefix activity test for Court.
- Source tests, installed hash, provider qualification, Discord behavior and human acceptance are separate evidence layers; unknown evidence is not completion.

## Failures and acceptance

- Receipt persists across restart but changed policy/version fails coverage. Test owner: implementation Task 1.
- Delayed preparation completes after withdrawal or shutdown and must not open media. Test owner: implementation Task 2.
- Retired bot session event arrives after a new session and must be ignored. Test owner: implementation Task 2.
- Uncovered new arrival pauses a currently speaking call immediately. Test owner: implementation Task 3.
- Synthetic audition or muted presence must never tick human audible acceptance. Test owner: implementation Task 4.

Each requirement needs source and any appropriate installed/live/human evidence. A focused filter must match nonzero tests. The final source slice gate is ./check.sh; deployment/live acceptance are separate. No measured performance claim is inferred from a design or fake.

## Execution boundary

Use the companion [2026-10-01-mlai-voice-completion.md](../plans/2026-10-01-mlai-voice-completion.md) only after design/plan review and execution-method selection. Follow the parent constraints; do not commit this design without explicit user instruction.
