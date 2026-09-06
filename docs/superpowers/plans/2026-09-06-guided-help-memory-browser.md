# Guided Help and Private Memory Browser Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make existing Discord actions discoverable and every stored fact privately readable.

**Architecture:** Reuse catalog eligibility and the central component dispatcher. Help remains navigation-only; fact browsing reads fresh canonical snapshots after current A1 authorization. Pure models own labels, pagination and strict envelopes; thin Discord adapters own acknowledgement, permissions and delivery.

**Tech Stack:** Rust 1.98, Serenity 0.12, Poise, Tokio and existing in-memory/WDBX authorities.

**Spec:** `docs/superpowers/specs/2026-09-06-guided-ux-operator-status-design.md`, sections 1, 2 and applicable acceptance requirements.

## Global Constraints

- Follow all existing AGENTS.md rules and binding 2026-09-04 command-center/privacy/voice contracts.
- Four complete facts per page, at most 100 facts of 300 characters, pages 0 through 24.
- Use exact `abbey:mem:v1:<owner>:<subject>:<scope>:<expiry>:<page>` ASCII grammar; maximum 100 characters; original 15-minute expiry never extends.
- Guild scope is a nonzero canonical decimal ID; bot-DM scope is literal `d` and owner equals subject.
- Acknowledge before permission refresh and snapshot reads. Revalidate current permission on each page and read nothing on denial.
- Replies are private, clamped, and use empty allowed mentions. No UI payload or custom ID is logged or persisted.
- No new memory mutations, transcript, reward, learning, voice, registration or tool-vocabulary behavior.
- Execute after reviewed modernization Task 6, and finish before modernization Task 12 final checks/integration.
- Root owns plans and ledger. Workers stage exact owned paths; coordinate compilation and commits with other active agents.

## File map

- `src/help_center.rs`: existing help session plus pure shortcut and presentation helpers.
- `src/command_catalog.rs`: existing authoritative eligible command rendering; delegate presentation without changing policy.
- `src/commands_help.rs`: existing central help adapter; add Button navigation and delegate memory protocol.
- `src/memory_browser.rs`: new pure scope/session/parser/pager/renderer.
- `src/commands_memory_browser.rs`: new thin Discord component adapter and summary reply button helper.
- `src/commands_brain.rs`: shared /recall and USER memory-menu attachment of Browse facts controls.
- `src/main.rs`: module registration only; preserve Task 6 central dispatch ownership.
- Focused sibling test modules, existing `commands_help/dispatch_tests.rs`, and README usage prose.

### Task 1: Guided Help Navigation

**Files:** Modify `src/help_center.rs`, `src/command_catalog.rs`, `src/commands_help.rs`, their focused tests, `src/commands_help/dispatch_tests.rs`, and README help prose.

**Interfaces:** Consume `eligible(&CommandSpec, &EligibilityInput, EvaluationMode) -> bool`, `HelpSession::navigate(HelpSection) -> HelpSession`, `HelpSession::custom_id() -> String`, and current dispatcher. Produce:

```rust
pub struct HelpShortcut {
    pub section: HelpSection,
    pub label: &'static str,
}
pub fn help_shortcuts(input: &EligibilityInput) -> Vec<HelpShortcut>;
pub fn invocation_hint(kind: CommandKind) -> &'static str;
pub fn visibility_hint(private: bool, context: InteractionContext) -> &'static str;
```

- [ ] Add pure tests for invocation/visibility language and conditional shortcut visibility before changing rendering. Use the actual catalog evaluator for each referenced command, setting self-subject for the current help input. No duplicate eligibility table.

```rust
assert_eq!(invocation_hint(CommandKind::UserContext), "member menu");
assert_eq!(invocation_hint(CommandKind::MessageContext), "message menu");
assert_eq!(visibility_hint(false, InteractionContext::BotDm), "reply in this DM");
assert_eq!(visibility_hint(true, InteractionContext::Guild), "private");
```

- [ ] Run `cargo test --locked help_center::` and confirm new assertions fail before implementation.
- [ ] Implement fixed invocation/visibility mapping and shortcuts: eligible persona ask -> Conversation; eligible self recall -> Memory; eligible OCR or describe-image menu -> Images. Render exact names and useful guidance only for eligible entries. Preserve every command in the section before clamping; reduce new prose if any full permitted section exceeds 2,000 codepoints.

```rust
pub fn visibility_hint(private: bool, context: InteractionContext) -> &'static str {
    if private { "private" }
    else if context == InteractionContext::BotDm { "reply in this DM" }
    else { "channel-visible" }
}
```

- [ ] Build one optional row of at most three classic text buttons on Start, alongside the existing selector. Encode the destination using `session.navigate(shortcut.section).custom_id()`. Extend the actual help dispatcher to accept Button navigation while retaining StringSelect validation. Preserve original expiry and refresh eligibility after envelope checks.
- [ ] Extend actual dispatcher fixtures to invoke both component kinds, hold acknowledgement before permission I/O, revoke permission between pages, and reject malformed/expired/foreign-owner controls without lookup. Compare canonical stores, pending rewards and voice state before/after navigation. Render all eight sections over member/moderator/manager/admin/DM and capability combinations; assert eligible command coverage and row/ID/message bounds.
- [ ] Read representative rendered output and update the README help-use paragraph. Run `cargo test --locked help_center::`, `cargo test --locked command_catalog::`, `cargo test --locked commands_help::`, formatting and all-targets Clippy. Commit only this task's owned files and obtain independent review.

### Task 2: Complete Private Fact Browsing

**Files:** Create `src/memory_browser.rs`, `src/commands_memory_browser.rs` and separate focused tests; modify `src/commands_brain.rs`, `src/commands_help.rs`, `src/main.rs`, relevant dispatcher fixtures and README memory-use prose.

**Interfaces:** Consume `memory_card::subject_authorized(u64, u64, &[DiscordPermission]) -> bool` and `MemoryService::subject_snapshot`. Produce:

```rust
pub enum MemoryScope { Guild(u64), BotDm }
pub struct MemorySession {
    pub owner: u64,
    pub subject: u64,
    pub scope: MemoryScope,
    pub expiry: u64,
    pub page: u8,
}
pub struct FactPage<'a> {
    pub index: u8,
    pub total_pages: u8,
    pub total_facts: usize,
    pub facts: &'a [String],
}
pub enum BrowserRejection { Stale, NotOwner, WrongScope, Expired }
pub fn page(facts: &[String], requested: u8) -> FactPage<'_>;
pub fn validate(id: &str, actor: u64, scope: &MemoryScope, now: u64)
    -> Result<MemorySession, BrowserRejection>;
pub fn render(subject: u64, page: &FactPage<'_>) -> String;
```

Session new/navigate/custom_id methods use checked `now + 900`, preserve owner/subject/scope/expiry, and change only a validated page. Do not derive Debug for identity-bearing sessions. The shell uses the existing Serenity context/Data/component signature and returns a handled boolean for its protocol branch.

- [ ] Add pure tests for all facts reachable without truncation, empty/shrinking data, Unicode, and strict envelopes. Pin maximum-width IDs at 99 characters. Reject zero/noncanonical/overflow IDs, extra fields, unknown versions, page >24, wrong actor/scope, DM cross-subject, expiry at now and expiry over 900 seconds away.

```rust
let facts: Vec<String> = (0..100).map(|n| format!("{n}:{}", "x".repeat(296))).collect();
let seen: Vec<&String> = (0..25).flat_map(|n| page(&facts, n).facts.iter()).collect();
assert_eq!(seen, facts.iter().collect::<Vec<_>>());
let shortened = page(&facts[..5], 24);
assert_eq!(shortened.index, 1);
assert_eq!(shortened.facts, &facts[4..5]);
```

- [ ] Run `cargo test --locked memory_browser::` to establish failing requirements, then implement canonical parser, snapshot pager and bounded full-text renderer. Empty data has a stable page-one-of-one presentation and no enabled navigation. Use at most four full facts per page; canonical store bounds remain the authority.
- [ ] Add Browse facts to both shared summary reply paths when facts exist, preserving their identical card body. Use separate previous/next controls whose IDs encode destination pages. After acknowledgement, validate bot-authored message and local envelope, fetch current guild permissions when needed, apply shared A1, and only then fetch the fresh subject snapshot. Map DM to the caller's existing one-person memory scope; never trust a serialized scope to select a different current conversation.
- [ ] Add actual registered menu/slash and component tests against fake HTTP and a recording snapshot seam. Prove no snapshot on denied/expired/foreign-context requests, permission loss on navigation, full text/private/no-mentions delivery, fixed expiry, current pagination after deletions elsewhere, and zero memory/transcript/reward/learning/voice mutations by browsing.
- [ ] Update README with Browse facts and its read-only scope. Run `cargo test --locked memory_browser::`, `cargo test --locked commands_memory_browser::`, `cargo test --locked commands_help::`, `cargo test --locked commands_brain::tests::`, format and all-targets Clippy. Commit exact owned paths and obtain independent review.

## Plan self-review

All guided-help and private-browsing design requirements map to Tasks 1/2, including actual adapter enforcement and full fact visibility. Pure types are defined above; existing authorities retain their signatures. No mutation or live-operation task is introduced. The separate operator plan owns design sections 3/4, and modernization Task 12 remains the final integrated gate.
