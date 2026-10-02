# MLAI Bounded Learning Quality Design

**Status:** Proposed. Planning authorized; implementation/publication/operator actions are not authorized by this file. **Goal:** Make personality adaptation and learning rewards reliable, inspectable and deletable within honest limits.

**Parent:** [2026-10-01-mlai-completion-program-design.md](2026-10-01-mlai-completion-program-design.md). **Dependencies:** Text failure categorization first; continuity and initiative consume this audit/erasure contract.

## Current evidence

Per-guild DQN chooses stay/reply/react; style addenda use five net signals, three supporters, seven-day window/fourteen-day TTL. Quotation safeguard and aggregate pending inspection passed focused tests and full gate. Learning does not train LLM weights or establish truth. Pending rewards currently include raw ask/asker/scope.

## Architecture

Retain one per-guild brain and template-only style ledger. Improve attribution before expanding reward volume. Minimize reward inputs, add private aggregate explanations, and define member-linked erasure separately from aggregate trained influence.

## Requirements

LEARNING-QUALITY-R1: Deduplicate reactions by scoped (message,reactor,emoji) active state; bot reactions and unidentified reactors cannot become attributed human reward. One active supported reaction per member/emoji contributes once; removal reverses only that recorded contribution once.
LEARNING-QUALITY-R2: Exact reply-to attribution has priority. Same-channel attribution is refused when multiple open turns plausibly match; record closed reason Ambiguous instead of rewarding the newest turn by default. Keep 150-second settlement and ±3 clamp unchanged.
LEARNING-QUALITY-R3: At most 4,096 active dedup entries across the collector, expiring no later than 300 seconds after their turn closes; eviction cannot permit replay into an already-settled turn. Persist only bounded native identity/hash/state needed for recovery and erasure, never into managed JSONL.
LEARNING-QUALITY-R4: Introduce AskSignature of at most 32 domain-separated hashed lexical tokens plus closed marker flags; use it for overlap instead of persisted raw original ask. Migrate legacy pending ask records once, then omit raw ask on publication; maintain covered episode-gate checkpoint rules.
LEARNING-QUALITY-R5: Private brain diagnostics show accepted/duplicate/ambiguous/expired outcomes, attribution sources, pending age and guard reason. Q-values remain action values; reputation never grants permission or becomes factual confidence.
LEARNING-QUALITY-R6: Corrections can repair an exactly attributed answer by re-reading authorized evidence or admitting insufficient evidence. Quoted examples and ambiguous standalone no are not repairs. Repair never writes facts or administrative memory edges automatically.
LEARNING-QUALITY-R7: Member erasure removes linkable pending rewards, reactions, reputation/events/cache and style observations, plus continuity/initiative records once those plans land. Aggregate DQN weights and consumed addenda support are not individually attributable; full influence removal requires a separate guild-manager-confirmed scoped reset, preserving canonical facts.
LEARNING-QUALITY-R8: Preserve opt-in learning, unchanged fixed style templates, existing quorum/TTL and budgets. Add a 100-case held-out quality set: 20 supported, 20 unsupported, 20 contradictory-source, 20 correction and 20 empty-retrieval cases; report counts/false positives/false negatives without invented probabilities.

## Interfaces and data ownership

Proposed `ReactionKey { scope, message, reactor_hash, emoji }` with closed active state. `FeedbackAttribution::{ExactReply,UniqueScoped,Duplicate,Ambiguous,Expired,Unsupported}` is an enum. Proposed `AskSignature { token_hashes: Vec<u64>, markers: AskMarkers }` has at most 32 hashes. `LearningAudit` contains aggregate counters only. `CorrectionDecision::{Ignore,Repair { turn_id }}` is transport-free. `LearningEraseReport` separately lists removed linkable records and whether an operator-authorized aggregate reset occurred.

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

- Replayed add/remove reaction across restart contributes at most once. Test owner: implementation Task 1.
- Two competing conversations must not assign feedback to the newest turn arbitrarily. Test owner: implementation Task 1.
- A legacy raw-ask record must disappear after migrated publication. Test owner: implementation Task 2.
- A quoted correction must not trigger repair or durable fact mutation. Test owner: implementation Task 3.
- Erasing a member must not resurrect state from caches, projection or late settlement. Test owner: implementation Task 4.

Each requirement needs source and any appropriate installed/live/human evidence. A focused filter must match nonzero tests. The final source slice gate is ./check.sh; deployment/live acceptance are separate. No measured performance claim is inferred from a design or fake.

## Execution boundary

Use the companion [2026-10-01-mlai-learning-quality.md](../plans/2026-10-01-mlai-learning-quality.md) only after design/plan review and execution-method selection. Follow the parent constraints; do not commit this design without explicit user instruction.

## Reviewed learning interfaces and evaluator

Task1 owns canonical persistence/default migration of the bounded reaction ledger and settled reward markers in `src/persist.rs`/`src/persist/tests.rs`. Persist these together with pending outcomes before exposing completion; restart tests reload the serialized state before replaying add/remove. Never promise idempotence from transient caches alone.

Task2 owns `LearningAudit { exact: u64, unique: u64, duplicate: u64, ambiguous: u64, expired: u64, unsupported: u64 }` and `LearningAudit::record(&mut self, attribution: FeedbackAttribution) -> ()`, `snapshot(&self) -> LearningAudit` in `src/brain/telemetry.rs`; values are counters only. `AskSignature::from_text(text: &str) -> AskSignature` uses the existing pinned hash seam and at most32 distinct normalized token hashes; tests pin deterministic normalization, bounds and absence of raw text after migrated publication.

Task3 owns `evaluate_correction(attribution: FeedbackAttribution, quoted: bool, source_turn: Option<u64>, authorized: bool) -> CorrectionDecision` in `src/brain/correction.rs`: only exact/unique authorized nonquoted attribution with a current source turn may Repair; everything else Ignore. It supplies no fact mutation authority.

Task4 owns `LearningEraseReport { pending: usize, reactions: usize, social: usize, style: usize, continuity: usize, engagement: usize, aggregate_reset: bool }`. `erase_member(state: &mut LearningEraseState<'_>, scope: &str, member: u64, now: u64) -> LearningEraseReport` receives mutable references to canonical pending/reaction/social/style stores, plus optional continuity and engagement stores as they land. `reset_scope(state: &mut LearningEraseState<'_>, scope: &str, confirmed_manager: bool, now: u64) -> Result<LearningEraseReport, WorkError>` refuses unconfirmed/nonmanager callers and preserves canonical facts. LearningEraseState is owned in erasure.rs; runtime alone builds it under documented lock order. Tombstones cover300seconds after erasure, reject late attribution, and persist with the removal transaction. Do not claim individual aggregate-weight unlearning.

Task5 creates `src/brain/quality_evaluation.rs` and registers it in `src/brain/mod.rs`. Fixture schema1 has100 distinct cases,20 each of supported, unsupported, contradictory-source, correction and empty-retrieval. Each case has id, class, supplied source records `{id, revision, current, text}`, proposed claim `{text, cited_source_ids}`, and human-curated expected `Accept|Abstain` plus rationale. Fixture text is synthetic. `evaluate_case(case: &QualityCase) -> QualityDecision` calls the explicitly bounded lexical and structural checks defined below on only supplied records; no provider/network. `evaluate_corpus(cases: &[QualityCase]) -> QualityReport` counts TP/FP/TN/FN against curated labels, separately by class, rejecting missing/duplicate IDs and unknown source references. Acceptance on the frozen synthetic corpus requires zero false-positive accepted claims against its curated labels; this is not a general semantic-support guarantee; false negatives are reported, not hidden. This proves deterministic grounding policy, not provider answer quality. A separate operator-reviewed fixed-provider run on the same corpus records model outputs and manually adjudicated support; required before tuning, with no automatic model judge or confidence claim. Add fixtures for an invented source, stale source and contradictory source and assert Abstain. Own tests live in quality_evaluation.rs, fixtures only in tests/. Record report alongside source identity; DQN seed/import tests are additional checks, not the evaluator.


Task5 evaluator's exact implementation: construct `Grounding::from_sources` from current cited source records only; call `grounding::check(&case.claim.text, &grounding)`. A claim is Accept only if citations exist, are current, have no fixture-declared contradiction, and Verdict::is_grounded is true; otherwise Abstain. Correction cases also pass through evaluate_correction and must match their curated expected repair/ignore label. This is a lexical/specificity policy check with explicitly curated contradiction labels, not semantic truth inference. QualityCase fields include `contradictory: bool`, optional correction facts and `expected_correction: Option<Repair|Ignore>`. All boolean labels are synthetic fixture inputs, never inferred model fact edits. Human adjudication remains required for provider answer quality.

