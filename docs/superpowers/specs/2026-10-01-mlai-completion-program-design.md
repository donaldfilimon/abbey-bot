# MLAI and Abbey Completion Program Design

**Status:** Proposed planning package. Donald explicitly requested brainstorming, writing-plans and continuation until the plan is done. This authorizes producing the full reviewable package; implementation, publication, new dependencies, commits and operator actions retain their separate gates.

**Date:** 2026-10-01. **Checkout:** canonical Rust abbey-bot. **Purpose:** make Abbey reliable and useful in text, voice and Activities, then improve bounded personality learning, continuity and initiative. Preserve server history/access and individual consent. Do not substitute a new bot stack.

## Decision and decomposition

Recommended: complete existing architecture in independently reviewable vertical slices. A central-service rewrite adds migration risk across already-working authority/persistence. Broad feature-first changes delay attributable acceptance. Keep one runtime, one provider execution authority, one canonical store and existing Work scheduling/delivery ownership.

The eight sub-projects below each have a specification and implementation plan. A ninth, separately approved member-engagement workstream is linked in the master plan and supplies the sole optional outreach consent/budget authority. Larger learning/continuity/initiative changes are independently gated; no single giant implementation starts from this overview alone. Proposed numeric limits and interfaces are decisions for review, not observed Current behavior.

## Verified baseline and limits

Source gate observed exit0: 1,568 Rust passed, zero failed, five ignored; Swift16 offline tests/release; warnings-denied Clippy; locked Rust release. Built SHA256 `18b1a1794d54001e9c51c83743575664a7e4f2fdc9e566726b6c2b7680f18914`; installed SHA256 `b965ed9b11c9aa2f0ac85dd6213661c781716c5eb80a0dac11d27fba254fb5a9`. Live read-only service status ready. New source is not installed.

Court local three tests/browser proof; public API/new iframe unknown. Synthetic voice100% word recall; H2 absent/no valid Local receipt at recorded observation; human conversation not proven. Learning is action policy plus bounded fixed-template style, not LLM training or factual-confidence proof. Current voice supervision already calls while-present auto-listen; do not duplicate it. Work delivery already preserves uncertain sends as ReviewRequired; reuse it.

Authoritative dated evidence: [MLAI-LIVE-ACCEPTANCE](../../MLAI-LIVE-ACCEPTANCE.md). Prior dates/user reports are not current runtime qualification. Telegram/Slack remain disabled, public hosting/publication not authorized by this planning request, and other guild opt-outs remain intact.

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

## Program acceptance

Implementation complete means every sub-project requirement has its own correct proof layer and no missing required evidence. Planning complete means all eight designs/plans and this roadmap are written, reviewed, internally consistent, linked, and cover requirements; execution review remains separate. Do not mark product acceptance from plan approval.

## Sub-projects and ordering

1. [MLAI Activity Production Launch](2026-10-01-mlai-activity-launch-design.md) — Launch the new shared Court inside Discord with honest recovery and bounded anonymous room state.
2. [MLAI Voice Completion](2026-10-01-mlai-voice-completion-design.md) — Prove reliable consented server voice and eliminate lifecycle ambiguity without weakening consent.
3. [MLAI Text Latency and Reliability](2026-10-01-mlai-text-reliability-design.md) — Measure and improve text delivery while preserving bounded ownership, cancellation and effect-aware fallback.
4. [MLAI Bounded Learning Quality](2026-10-01-mlai-learning-quality-design.md) — Make personality adaptation and learning rewards reliable, inspectable and deletable within honest limits.
5. [MLAI Opt-in Conversation Continuity](2026-10-01-mlai-continuity-design.md) — Resume user-confirmed work across restart without persisting raw conversation or widening access.
6. [MLAI Appropriate Initiative and Follow-ups](2026-10-01-mlai-initiative-design.md) — Deliver useful explicitly opted-in task follow-ups with inspectable suppression and no repeated or cross-scope outreach.
7. [MLAI Member Workflows and Server Acceptance](2026-10-01-mlai-member-workflows-design.md) — Make the redesigned server and bot discoverable and usable by ordinary members across complete task flows.
8. [MLAI Release and Completion Qualification](2026-10-01-mlai-release-qualification-design.md) — Prove source, installed artifact and live capability acceptance across the entire completion program.

## Scope exclusions and deferred decisions

No new stack, general autonomous code rewriting, automatic moderation/contradiction, mass role grants, new guild installations, unbounded outreach, raw voice retention, new provider credentials or unimplemented WDBX encryption/COSE/key-destruction claims. Hosted API target is selected in the Activity publication task from verified operator-controlled resources; a static-only origin is rejected. Any requested Worker/state port gets a separate design, not an implicit package conversion.

Human voice consent/witness and human Portal mapping are real external actions. Plans prepare everything possible, identify exact required actor/action and keep those acceptance rows open. No unanswered approval is treated as authorization.

## Approved engagement dependency

Read [member engagement design](2026-10-01-member-engagement-design.md) and its [ten-task plan](../plans/2026-10-01-member-engagement.md). Initiative depends on its Tasks1–4 and uses EngagementStore for optional proactive delivery. Required configured Work reminders preserve their current policy. The master plan maps all approved engagement sections; external documents remain owned by their authoring chat. Source/install observations above are dated snapshots and require fresh verification after shared-tree changes.

