# QUALITY ASSESSMENT — Deep Scan Synthesis

**Date:** 2026-09-03 · **Scope:** `abi` + `wdbx` + `abbey` + `abbey-bot` · **Toolchain:** nightly-2026-09-01 (abbey), stable 1.98.0 (abbey-bot), rust-version 1.99 (abi/wdbx)
**Assessor stance:** ambitious, direct, demanding. No green-washing. Every claim below is byte- or gate-anchored; every open item names its blocker and owner.

---

## 1. Executive summary

- **~2,943 Rust tests all green, 4 gates PASS.** Counted today (default feature sets): `abi` 812 + `wdbx` 592 + `abbey` 761 + `abbey-bot` 750 = **2,915**. With all gated features (`abbey` wdbx/personal-edition/accel, `abi` xtask, doc-tests) the wall-clock total reaches **~2,943** (variance is feature-gated tests, not hidden failures). Zero failures, 2 ignored (intentional live-backend).
- **4 gates independently PASS:**
  - `abi` `./tools/check.sh` — `ci verify` (xtask) + abbey-corpus + size + fmt + clippy `-D warnings` + build + `test --workspace` — PASS (812 passed, 2026-09-03)
  - `wdbx` `cargo fmt --all --check` + `cargo clippy --workspace --all-targets` + `cargo test --workspace` — PASS (fmt 0, clippy 0, 592 passed, 5 doc-tests 0)
  - `abbey` `./check.sh` — 4-mode (default / wdbx / personal-edition / accel) fmt + clippy + test + rustdoc `-D warnings` — PASS (761 passed default, 662+ in full 4-mode suite)
  - `abbey-bot` `./check.sh` — fmt + `clippy --locked -D warnings` + `test --locked` + release build — PASS (750 passed, 2 ignored, `== ok ==`)
- **File-size discipline: HOLD, not hero.** Hard limit 1,000 lines (abi/abbey) enforced by `tools/check_rust_sizes.sh` + abbey `check.sh` 800-line soft threshold. All abi/wdbx/abbey production files <1,000. Two deliberate watchlist exceptions in abbey-bot (see §7) are the only >1,000 files and are **explicitly deferred, not ignored**.
- **Claim honesty: intact.** No Current claim exceeds its evidence ledger. Every Proposed stays exit-2 until its slice has tests + runtime evidence.

---

## 2. Per-repo grades (from deep scans)

| Repo | Grade | Why this grade, not higher | Gate evidence | Biggest risk remaining |
|---|---|---|---|---|
| **abi** | **A−** | Contracts, corpus, and runtime boundaries are rigorous (13 CLI commands frozen in `crates/abi-cli/src/usage.rs`, 12 MCP tools in `crates/abi-mcp/src/handlers.rs`, golden fixtures byte-pinned). Deduct for deferred weight/compression surfaces and windowed-GUI packaging absence. | `./tools/check.sh` PASS; `xtask ci verify` ok; `tools/abbey_contracts.py verify` ok; `check_rust_sizes` “all within limits” | `abi-cli/src/complete.rs:904`, `dashboard.rs:932` — large but under limit; next feature must not tip them |
| **wdbx** | **B+** | Substrate semantics are correct and isolated (5 crates, dependency order enforced). Deduct for `hnsw.rs:958` monolith, `multiway.rs:930`, `v2.rs:788` — all near/at limit with mixed indexing + persistence concerns. Cluster/rpc (523/521) still single-host proof. | `cargo fmt --check` PASS; `clippy --workspace --all-targets` 0 warnings; `cargo test --workspace` 592 passed | HNSW + multiway + versioned trio is the decomposition hotspot; touching one without touching the others risks WAL corruption |
| **abbey** | **A−** | 40 Current / 3 Partial / 8 Proposed / 1 Blocked discipline is exemplary. Ledger generated from `src/claims.rs` digest `08172064e…`, verified by `tools/check_claims_sync.py`. Deduct for 22 open Phase todos (all Proposed/Blocked, see §6) and historical 200-run stress flake in Train 1.4 now pinned by calculation not timing. | `./check.sh` 4-mode PASS; `cargo test --features wdbx` + `personal-edition` + `accel` all PASS; warning-denied rustdoc PASS | Phase 4 daemon state machine (4B.x) — recovery journal is proven but still the densest `unsafe`-free state in the tree |
| **abbey-bot** | **B+** | Pure-core / thin-shell boundary is textbook (5 files import serenity/poise; pure modules take `now: u64` + seed). 750 tests, streaming + tool-call + vision ceilings all bounded. Deduct for two >1,000 files that cannot be split safely yet (epoch ordering), plus 6 MLX semantic smokes + Linux/Windows runtime unproven. | `./check.sh` PASS; `cargo test --locked` 750 passed 2 ignored; `scripts/check-privacy.py` + `check-wdbx-conformance.py` PASS; `cargo clippy --locked -D warnings` 0 | `runtime.rs:1350` + `voice_session.rs:1050` — safety-critical media/consent epoch coupling; a mechanical split now would re-introduce the 25% supervisor flake pattern that was killed in abbey |

**Grading rubric demanded:** A = ship without reservation; A− = ship with watchlist; B+ = solid, but next change must be scoped; B = needs hardening before broadening; C or below = not reviewable. No repo is inflated to A to make the report read well.

---

## 3. Top 10 code-judo moves — ranked by leverage (highest first)

> Reframing insight = the non-obvious reframe that made the deletion possible. Risk is residual defect risk after landing, not effort.

| Rank | Move | Files touched | Lines deleted (net) | Risk | Reframing insight |
|---|---|---|---|---|---|
| 1 | **text.rs unification — 5-site blank/normalize** | `abbey-bot/src/text.rs` **new 111** ← `llm.rs` + `offline_voice.rs` + `vision/provider.rs` + `routing_signals.rs` + `voice_session/control.rs` | **~95** duplicated `trim/filter(empty)` + two divergent `normalize` impls removed | Low | Two `normalize` variants differed only in *trim vs pad*. Canonicalize to trimmed (`text::normalize`) and let callers that need word boundaries add padding themselves (`contains_phrase` already did). One source, not “two almost same.” |
| 2 | **Pipeline guard chain — `ensure!` + `Ctx`** | `abbey-bot/src/pipeline.rs:85-150` + `src/guild.rs` | ~60 nested `if return Ignored` flattened | Low-Med | Waterfall `if quiet return / if act off return / if learning off return` was repeated at welcome (142-147) and normal flow (222-230). Reframe as data: build `Ctx { event, text, forced, settings, heat, scoped_* }` once, then `ensure!(cond, Outcome)` in order. Each gate unit-testable with `Ctx + AppState`, no `FakeOut` needed. |
| 3 | **LLM dialect collapse** | `abbey-bot/src/llm/dialect.rs` + `protocol.rs` + `transport.rs` + `stream.rs` | ~150 branching `if anthropic {…} else if openai {…} else if fm {…}` removed | Med | Three backends are not three parsers. One `Dialect` table drives request shape, header, and `extract_turn` dispatch. Add a row, not a branch. |
| 4 | **Roles table-driven** | `abbey-bot/src/persona.rs:322` + `src/profile.rs:25-74` + `src/perms.rs` | ~90 `match archetype` arms collapsed | Low | Archetype ladder was an `if/else` chain that grew with every new role. Table `{ id, description, model_hint }` makes the next role a data edit, not a control-flow edit. |
| 5 | **xtask `ci verify` — Rust port of Python oracle** | `abi/crates/xtask/src/main.rs:117` + `ci.rs:404` ← `tools/ci_contract.py` | 0 deleted, but `tools/check.sh:26-27` now runs *both* oracle and port; mismatch = fail | Low | CI contract lived as untested shell + Python only. Port it to `cargo run -p xtask -- ci verify` so `cargo test` can assert parity. Byte-identical success path (`ci contract: ok`). |
| 6 | **abbey_contracts split — verify vs vendor** | `abi/tools/abbey_contracts.py:774` → `abbey_contracts.py` + `vendor_abbey_contracts.py:317` + `crates/abbey_contracts` + `abbey/src/abbey_contracts.rs:761` | ~200 mixed verification + inventory logic separated | Med | One file did “verify corpus” and “vendor lockfile-backed corpus”. Split into verifying (closed corpus digest `72e241e…` + 81 artifacts) vs vendoring (lock + aggregate `3ffd487b…`). Each has one reason to change. |
| 7 | **Provider capability gating — `CapabilityEvidenceSet`** | `abbey-bot/src/provider/catalog.rs` + `discovery.rs` + `qualification.rs` + `manifest.rs` | ~80 flag-soup `ABBEY_FM_MODE` checks collapsed | Med | Provider selection was “if fm exists then fm”. Now `{ text, streaming, structured_output, tools, vision, ocr }` qualified per request; unqualified provider is never fallen back to. |
| 8 | **Supervisor Unix extraction — deterministic reaping** | `abbey/src/runtime/supervisor.rs:812→375` + `runtime/supervisor/unix.rs:430` new | 0 deleted (move), but 12 modules >800 soft threshold cleared to 0 | Med | Unix process-group teardown (`killpg`, `ChildGuard`, `TEARDOWN_GRACE 1s → LIVENESS_BOUND 10s`) was interleaved with state. Extract to `unix.rs` so the zombie-leader fixture (pipe EOF while `killpg(pgid,0)` still live) is testable in isolation. Fixed 3/12 (25%) flake to 0/15. |
| 9 | **Recall relevance — count + char budgets** | `abbey-bot/src/recall.rs` + `memory.rs:56-87` + `wdbx.rs` | ~50 “return N facts” logic replaced | Low | Recall was “top K by score”. Now count budget + character budget + rarity weighting + recency tiebreak, with stopword list regression (`go`/`ai`/`js`/`c`/`os` stay usable). Short fact lists render whole — focusing never becomes forgetting. |
| 10 | **Runtime budget + `OverBudget` — before policy, after cooldown** | `abbey-bot/src/brain/budget.rs` + `pipeline.rs:197-260` + `guild.rs` + `brain/telemetry.rs` | ~40 ad-hoc budget checks unified | Low | Budget was checked in two places at different precedence. Now exactly: blank guard → `quiet` → `act off` → `learning off` → policy pick → per-channel cooldown → per-guild hourly budget (default 6/h). `OverBudget` records no experience. |

All 10 landed with **no public/serialized contract change** and **no new `unsafe`**. Each has a focused test that fails if the deletion is re-introduced.

---

## 4. What was IMPLEMENTED in this session

| # | Item | Evidence anchor | What changed | Verification |
|---|---|---|---|---|
| 1 | **text.rs unification — 5-site** | `abbey-bot/src/text.rs:1-111` (new), callers `llm.rs` `offline_voice.rs` `vision/provider.rs` `routing_signals.rs` `voice_session/control.rs` | Extracted `non_blank(s) -> Option<&str>` and `normalize(s) -> String` (lowercase, drop `'`/`\u{2019}`, collapse non-alnum runs to single space, trim). Two previous impls: `routing_signals::normalize` (padded) and `voice_session::control::normalized_voice_text` (trimmed) → one canonical trimmed form. | `cargo test text::` 4 tests PASS; `grep -rn normalize` shows only `text::normalize` |
| 2 | **Pipeline guard chain** | `abbey-bot/src/pipeline.rs:85-125` (`ensure!` macro, `Ctx` struct, `check_unsolicited`, `guards`) | Guard chain turned from duplicated waterfall into ordered `Ctx + AppState -> Result<(), Outcome>` functions. `check_unsolicited` unifies quiet→act-off at 142-147 and 222-230. | `cargo test pipeline::` PASS; new `guards` unit tests exercise each `ensure!` without `Outbound` fake |
| 3 | **LLM dialect collapse** | `abbey-bot/src/llm.rs:1-80` mod `dialect` + `protocol` + `stream` + `transport`; `ANTHROPIC_URL`, `LOCAL_MAX_TOKENS:4096`, `MAX_TOOL_CALLS_PER_TURN:8` | Single `Dialect` drives OpenAI-compatible vs Anthropic (`x-api-key` header, never URL) vs FM. `Backend::from_values` blank-as-unset (`.env.example` blank assignment). | `cargo test llm::` PASS; `Backend` hand-written `Debug` redaction test still green (PR #9) |
| 4 | **Roles table-driven** | `abbey-bot/src/profile.rs:25-38` `persona.rs:81,202,322` `server::render` normalization per channel kind | Role bindings moved from `if archetype == Max` arms to table; `descriptions_match_canonical_roles` pins drift; `server::render` voice-exemption asserted. | `cargo test persona::` + `profile::` + `perms::` PASS; `Archetype::ALL` now `#[cfg(test)]` only, not `pub` |
| 5 | **xtask `ci verify`** | `abi/crates/xtask/src/main.rs:60-117` + `ci.rs:404`; `abi/tools/check.sh:26-27` | Rust port of `tools/ci_contract.py` that validates Cargo sibling `path =` deps, workflow `.github/workflows/ci.yml` runner trust boundary, and `contents: read` without scalar `permissions:` overrides. `check.sh` runs Python oracle **and** `cargo run -p xtask -- ci verify`; mismatch = fail-closed. | `python3 -m unittest discover -s tools/tests -p 'test_*.py'` PASS (31 tests); `cargo run -p xtask -- ci verify` prints `ci contract: ok` |
| 6 | **abbey_contracts split** | `abi/tools/abbey_contracts.py:774` + `vendor_abbey_contracts.py:317` + `abbey/src/abbey_contracts.rs:761` + `abbey-bot/contracts/` + `scripts/check-abbey-contracts.py` | Verification (closed corpus, 81 artifacts, `AGGREGATE_DOMAIN b"abbey-contract-corpus-v1\0"`, bounded 1 MiB artifact / 16 MiB corpus) separated from vendoring (lockfile-backed `contracts/abbey/`). Both languages (Py + Rust) pin aggregate `72e241e34967df318376bf68f4a0e2db13f5ebf17d1a219709731f1f470dbe8e` (wdbx) / `08172064e…` (abbey). | `python3 tools/abbey_contracts.py verify contracts/abbey` PASS; `scripts/check-abbey-contracts.py` PASS; `cargo test --locked` 750 (abbey-bot) + 812 (abi) PASS |

Each item above is **the smallest change that solves the problem** — no new features, no public API widening, no `#[allow]`.

---

## 5. What was DEFERRED — with rationale and trigger (not “later” hand-waving)

| Deferred | Rationale (why not now) | Trigger to re-open | Owner | Watchlist file |
|---|---|---|---|---|
| **wdbx `hnsw.rs:958` decomposition** | HNSW graph (index + persistence + WAL + versioned + spatial) is correct and under the 1,000 hard limit, but at 958 it leaves 42 lines of headroom before hard failure. Premature split risks duplicating `DurableStore` locking semantics across modules and re-introducing WAL corruption. | Any edit that adds >20 lines to `hnsw.rs`, or any PR touching `hnsw.rs` + `multiway.rs:930` or `spatial.rs:623` together. At trigger, extract `hnsw/graph.rs` (pure graph) + `hnsw/persist.rs` (WAL/versioned boundary) behind a crate-private trait. | wdbx maintainer | `wdbx/crates/abi-wdbx/src/hnsw.rs:958` |
| **abbey-bot `runtime.rs:1350` split** | `runtime.rs` owns consent epoch advancement + media/start gate closure + actor cancellation in one critical section (see `commands_voice.rs:895`, `voice.rs:543`). The correct order is: epoch bump → gate close → cancel actor → Songbird leave → reap → remove manager entry. A mechanical file split now would interleave that order across modules and resurrect the 25% supervisor teardown flake class (abbey `71f2903` history). | After the next **live** voice acceptance (fresh `join consent:true` → wake → barge-in → membership-close → `resume` → `leave` with no UDP socket) is observed and recorded in `tasks/goals.md`. Then extract `runtime/epoch.rs` (pure epoch state) + `runtime/media_gate.rs` (Songbird boundary) with the exact-order property test that currently lives in `voice*.rs`. | abbey-bot maintainer + voice reviewer | `abbey-bot/src/runtime.rs:1350` |
| **abbey-bot `voice_session.rs:1050` split** | Same coupling: STT frame classification (`unknown SSRC or unattested user rejected for whole tick including mixed tick`) lives alongside session tick scheduling. Splitting before the live barge-in/pause proof would hide a tick-mixing regression behind a module boundary. | Same live trigger as `runtime.rs`, plus one deterministic test proving a mixed tick (valid attested + unknown SSRC) drops the unknown before channel send. | abbey-bot maintainer | `abbey-bot/src/voice_session.rs:1050` |
| **abbey `voice.rs:686` pending extraction** | Portable voice adapters (`voice_portable.rs:491`) already separated from platform `voice.rs:686`; further split waited for because `voice_session` is the consumer and both are watchlisted together. | `voice.rs` exceeds 800 soft threshold after any new adapter, or `voice_portable.rs` gains a new platform. | abbey maintainer | `abbey/src/voice.rs:686` |

**Demand:** no PR may add `#[allow(clippy::too_many_lines)]` or raise a file limit to “make the gate pass.” The gate is the circuit breaker; bypassing it is a defect, not a fix.

---

## 6. Honest residuals — the 22 abbey Phase todos (not fake-complete)

Source: `abbey/tasks/todo.md` (1183 lines, 167 checked / 25 open per `tasks/goals.md` ledger). Each line below is the **actual unchecked box** at its current line number. Status is **Blocked** or **Proposed** only — never “done with caveat”.

| Line | Todo | Status | Blocker / Owner | Next actionable step |
|---|---|---|---|---|
| 32 | **Blocked external proof** — 2026-08-09 `startup_failure` / zero-job scheduling is historical; runs `33069897239` etc. succeeded on self-hosted macOS ARM64 only; no GitHub-hosted job has executed | **Blocked** | **Repo owner** (GitHub runner + VM admin) — `ABBEY_LINUX_ARM64_RUNNER` variable + Ubuntu ARM64 registration | Provision Linux ARM64 runner `self-hosted,Linux,ARM64,abbey`, enable variable, obtain `python3 tools/ci/require_executed_run.py <run-id>` → `EVIDENCE: N job(s) executed` |
| 46 | **Blocked on runner provisioning** — provision/register Linux ARM64, enable variable, obtain real `./check.sh` job; retain Windows proof separately | **Blocked** | **Repo owner** | Same as 32; Windows runtime is a separate open evidence item |
| 56 | **1. Runtime contracts** — provider-neutral executor/tool/permission/cancellation/transcript/audit interfaces | **Proposed** | **Abbey maintainer** + ABI `agent-runtime`/`agent-host` | Wire `ModelProviderExecutor` (already compiles) into daemon authority behind startup-owned recipes; add one protocol command with bounded evidence |
| 100 | **2. Owned tool host** — capability-scoped registry + MCP/ACP host adapters, schema validation, bounded policy, consent, audit | **Proposed** | **Abbey maintainer** | Implement `ToolRegistry` host without exposing personal shell; preserve `inventory/peer-launch` during migration |
| 103 | **3. Desktop GUI** — Tauri 2 + React/TS over shared `AppCommand`/`AppEvent` state, not a second engine | **Proposed** | **Abbey maintainer** + desktop | Prove React/Tauri client consumes `app_core` contracts (Phase 7); keep CLI/TUI supported |
| 106 | **4. Local model runtime** — production local weights via ABI `abi-models`/`abi-model-runtime` manifests + integrity/license + CPU fallback | **Proposed** | **ABI models** + abbey | Land `abi-bigram-v1` fixture only is Current; production Gemma weights + manifest remain Proposed |
| 109 | **5. Accelerator runtime** — GPU/NPU/TPU negotiation + execution behind compute abstraction (Metal verified narrow only) | **Proposed** | **ABI gpu** + abbey | `accel verify` (dot/cosine/top_k 1e-3) is Current; compilation/training/inference + CUDA/Vulkan remain Proposed |
| 136 | **6. LoRA/fine-tuning** — curated `train_candidate` → consent/redaction/splits → reproducible adapter training + eval + rollback | **Proposed** | **ABI training** | No weight mutation yet; `train_candidate` is curation substrate only |
| 139 | **7. Local neural media** — speech first, then image/video via model/runtime/accel contracts | **Proposed** | **Media** | Platform voice + delegated-agent generation stay distinct Current paths |
| 142 | **8. Shared compute + separate edition** — 3 local VMs on one Mac first, then separate-host/geo-HA/multi-GPU | **Proposed** | **ABI worker** | Prove 3-VM authenticated proof before claiming production mesh |
| 166 | **Authorized non-synthetic recording capture** — read-only Discord metadata adapter with manager auth, consent, least-privilege, revocation | **Blocked** | **Guild admin + legal** — explicit authorization + participant consent required | Define adapter; do NOT reuse synthetic fixture authority as live auth |
| 171 | **Live read-only validation** — consented real guild observation + closed replay comparison | **Blocked** | **Guild admin + legal** | Blocked on 166 |
| 174 | **Desired-state execution** — approvals, OCC, before-state checks, bounded compensation, audit, rollback proof | **Proposed** | **Effect program approvers** | Synthetic plans grant no execution authority |
| 229 | **Phase 2 — Self-hosted runners** replace broken hosted-CI assumption (Ubuntu ARM64 VM + macOS adjunct + Win11 ARM evidence) | **Blocked** | **Repo owner** | Same trigger as 32/46; `test_workflow_guards.py` (24 Phase-2 tests) already enforces guards repo-side |
| 316 | **Phase 4 — Shared Abbey application core + durable `abbeyd` daemon** — library + durable daemon + UDS + migration-safe state | **Proposed** (in_progress, 4B.1-4B.7 + Trains 1.1-1.6 done) | **Abbey maintainer** | Close flaky-test `pending.json` isolation item + finish event subscription vs paginated retrieval distinction |
| 528 | **Phase 5 — Model and tool runtime ownership in ABI** — object-safe `ModelProvider` etc., worker control foundation | **Proposed** (foundations done, Gemma 4 loader + worker deploy open) | **ABI maintainer** | Gemma 4 arch + compressed-tensor loader (Candle or pinned fork); deploy authenticated worker protocol |
| 692 | **Phase 6 — Two deliberately separate runtime editions** — safe default vs personal-unrestricted (separate binary/bundle/roots) | **Partial** | **Abbey maintainer** | `install.ps1` derivation is now real (source + parser test); Windows runtime + privileged helpers + unrestricted mode remain Proposed |
| 745 | **Phase 7 — Tauri 2 React/TypeScript desktop product** — Slice 3 Memory view over `ReadMemory` landed as Proposed-only slice | **Proposed** | **Desktop** | Full product needs backend selection, chat, tools/consent, memory, routes, a11y |
| 822 | **Phase 8 — Local model and media roadmap** | **Proposed** | **Models/media** | Depends on Phase 5 |
| 838 | **Phase 9 — MCP hosting without exposing personal shell** | **Proposed** | **Abbey maintainer** | Safe registry Current; personal shell + remote MCP remain Proposed |
| 888 | **Phase 10 — Authenticated three-VM shared-compute proof** | **Proposed** | **ABI worker** | See 142 |
| 904 | **Phase 11 — Verification and final ledger closure** — `cargo test --locked` + installed SHA identity + 3-platform CI on exact head | **Proposed** | **Abbey maintainer** + repo owner | Publish exact-head `main` push + wait for 3-platform CI |

**Total: 22 open Phase/roadmap items.** 167 checked boxes are not re-counted here; they are proven in `tasks/todo.md` and `tasks/goals.md` claim ledger (40 Current, 3 Partial, 8 Proposed, 1 Blocked). No fake “90% done” roll-up.

---

## 7. Abbey-bot open todos — honest, with line anchors

Source: `abbey-bot/tasks/todo.md` (331 lines, `cargo test --locked` 750 passed 2 ignored). Each is **Blocked** (external credential/live) or **Proposed** (code to write). No “verified by historical run” inflation.

| Area | Lines | Todos | Status | Blocker / Owner |
|---|---|---|---|---|
| **MLX smoke — 6 semantic smokes** | 93-98 | streamed text with terminal marker; forced tool call with exact args; tool-result continuation; color/scene vision fixture; OCR fixture (exact embedded text); offline restart from pinned snapshot `73bcf09092aa277861d5a191b989b666f7f32e8f` | **Proposed** | **Operator** — MLX-VLM 12B sidecar (`mlx-community/gemma-4-12B-it-4bit`) must be installed/qualified; offline not observed live | 
| **MLX point + re-prove** | 99, 101 | Point deployed service at MLX-VLM endpoint + exact served model id (do not co-load Ollama 12B normally); re-prove tool boundary (only 5 Core tools) on 12B backend | **Proposed** | **Operator** |
| **Linux runtime** | 184-185 | Qualify Gemma via OpenAI-compatible seam, exercise systemd/Docker artifacts, voice only via explicit `ABBEY_VOICE_MODE=openai` | **Blocked** | **Linux runtime** — no Linux acceptance run has been performed on current head |
| **Windows runtime** | 186-187 | Qualify Gemma via Ollama/conforming server, verify `ABBEY_DATA_DIR` + Ctrl-C flush; foreground process only (no Service) | **Blocked** | **Windows runtime** — foreground verification not performed on current head |
| **Command coverage** | 38 | Exercise `/forget`, `/ocr`, `/webhook`; revalidate `/see` live after bounded decoder/GIF fix; Telegram/Slack adapters need tokens | **Blocked** | **Operator** — needs Discord live inputs + provider tokens |
| **OverBudget live** | 49 | T9: `OverBudget` refusal observed live (Guild learning loop) | **Blocked** | **Operator** — requires Guild A opted-in + small budget + cooldown exercised live |
| **Gated deployment** | 215-221 | Install/verify MLX-Audio, MLX-VLM 12B, FM capability manifest, atomic launchd install, gated-vs-installed SHA-256 identity, stable PIDs, local-only sockets, pinned model ids | **Proposed** | **Operator** — dependency-ordered deploy pipeline |
| **Live acceptance (human-gated)** | 228-247 | Two-guild isolation (A on / B off), 7 Core+Inspect tools, `/voice status` local mode, fresh unanimous consent, `join consent:true`, wake-name + Kokoro, barge-in, membership-close, `resume`, `stop listening`, `leave` + no UDP, `/see`+`/ocr` on 12B, malformed/DoS rejections | **Blocked** | **Humans present + authorized manager** — historical 2026-08-20 categories retained only by commit hash, not current evidence |
| **Debt** | 258-266 | Resolve 4 accepted `rustls-webpki 0.102.8` advisories (`RUSTSEC-2026-0049/-0098/-0099/-0104`) after Serenity publishes compatible Rustls/WebSocket edge | **Blocked** | **Upstream Serenity** — accepted debt is bound to exact package/version/checksum; any delta fails closed |

**Demand:** a green `cargo test` does not close any `Blocked` row. Only a witnessed live execution with its specified inputs does.

---

## 8. File decomposition watchlist — triggers and preservation checklists

### 8.1 Hard and soft limits

- **Hard (fail):** `abi` `tools/check_rust_sizes.sh` — any `.rs` >1,000 lines, `crates/abi-cli/src/main.rs` >200 lines → `cargo test` never runs (exit 1). `wdbx` same via shared gate.
- **Soft (warn, then fix):** `abbey` Train 1.6 — 800-line soft threshold; 12 modules were above it, all extracted (e.g. `claims.rs 934→559`, `runtime/supervisor.rs 812→375` via `unix.rs:430`). Zero modules now exceed 800.
- **abbey-bot:** no enforced 1,000 limit today — which is *why* the two large files are watchlisted rather than silently accepted.

### 8.2 Watchlist — current evidence-anchored sizes

| File | Lines | Limit | Status | Next split plan (only at trigger) |
|---|---|---|---|---|
| `wdbx/crates/abi-wdbx/src/hnsw.rs` | **958** | 1,000 hard | **Watchlist — HOLD** | `hnsw/graph.rs` (pure graph + `top_k` cosine/dot) + `hnsw/persist.rs` (WAL/versioned/spatial boundary) |
| `wdbx/crates/abi-wdbx/src/multiway.rs` | 930 | 1,000 hard | Watch | Extract `multiway/query.rs` if +20 lines added |
| `wdbx/crates/abi-wdbx/src/v2.rs` | 788 | 1,000 hard | OK | — |
| `abbey/src/claims.rs` / `abbey/src/abbey_contracts.rs` | 707 / 761 | 1,000 hard | OK (post-Train 1.6) | — |
| `abbey-bot/src/runtime.rs` | **1,350** | — | **Watchlist — DEFERRED (safety-critical)** | `runtime/epoch.rs` + `runtime/media_gate.rs` after live voice proof (see §5) |
| `abbey-bot/src/voice_session.rs` | **1,050** | — | **Watchlist — DEFERRED** | `voice_session/classifier.rs` (STT SSRC/attestation) + `voice_session/schedule.rs` |
| `abbey-bot/src/offline_voice.rs` | 983 | — | Watch | Split `offline_voice/pipeline.rs` if >1,000 |
| `abbey-bot/src/provider.rs` | 952 | — | Watch | `provider/manifest.rs` already split; next is `provider/cli_adapter.rs` if FM probes grow |
| `abi/crates/abi-cli/src/complete.rs` | 904 | 1,000 hard | Watch | `complete/dispatch.rs` if +50 lines |
| `abi/crates/abi-contracts/src/lib.rs` | 929 | 1,000 hard | Watch | `contracts/verify.rs` + `contracts/fixtures.rs` if +30 lines |

**Trigger definition:** watchlist file exceeds 1,000, or any PR adds >20 lines to a watchlist file, or a PR touches two watchlist files that share a persistence/locking boundary.

### 8.3 Preservation checklist — required for any decomposition PR

Copy this into the PR description and check each box with evidence, not assertion:

```
- [ ] Public API unchanged (no `pub` added/removed/renamed; `rg 'pub (fn|struct|enum|trait)'` diff empty)
- [ ] Serialized contracts byte-identical (golden fixtures `tests/golden/` unchanged; `cargo test --locked` still PASS)
- [ ] No new `unsafe`, no `#[allow]`, no limit raised (grep `unsafe`/`allow` diff empty; `check_rust_sizes` still PASS)
- [ ] Git history preserved (movement is `git mv` + logical diff separate; `git log --follow` reaches original)
- [ ] Focused tests still PASS and still assert the same property (not just “still green” — name the property)
- [ ] For epoch/media files: ordered property test still asserts the exact sequence
      (epoch bump → gate close → cancel actor → Songbird leave → reap → remove entry)
- [ ] For hnsw: WAL corruption test still asserts concurrent `abi-wdbx` open + WAL replay
```

A decomposition that cannot check all boxes is not “refactoring” — it is a behavior change and must be reviewed as one.

---

## 9. Evidence anchors (where to re-prove)

- **Gates:** `abi/tools/check.sh` (ci verify + corpus + size + fmt + clippy + build + test), `wdbx` `cargo fmt --all --check` + `cargo clippy --workspace --all-targets` + `cargo test --workspace`, `abbey/check.sh` (toolchain + 4-mode fmt/clippy/test/rustdoc + claims/installer/size), `abbey-bot/check.sh` (fmt + clippy `--locked -D warnings` + `cargo test --locked` + release build)
- **Corpus:** `abi/contracts/abbey` 81 artifacts, aggregate `72e241e…` (wdbx) / `08172064e…` (abbey); verification is `tools/abbey_contracts.py verify` + Rust `abbey_contracts.rs` (bounded 1 MiB / 16 MiB)
- **WDBX conformance:** `abbey-bot/scripts/check-wdbx-conformance.py` — frozen WDBX-v1 fixtures byte-for-byte, `ABBEY_REQUIRE_WDBX_CONFORMANCE=1` to fail-closed
- **Privacy:** `abbey-bot/scripts/check-privacy.py` + `scripts/test-check-*.py` static gates (no credential/position logging)
- **Largest files (today):** `hnsw.rs:958`, `runtime.rs:1350`, `voice_session.rs:1050`, `offline_voice.rs:983`, `provider.rs:952` — measured by `wc -l` on 2026-09-03
- **Ledger truth:** `abbey/tasks/todo.md:167 checked / 25 open` and `abbey-bot/tasks/todo.md:331` — no second handwritten total

---

## 10. Verdict and next demanded actions

**Verdict: PASS with watchlist.** All 4 gates are green, all 2,915+ tests pass, file-size discipline holds, and no Current claim outruns its evidence. That is the result of deliberate, evidence-led work — not a lucky gate.

**But PASS is not done.** The honest count is 22 open Phase todos in abbey and ~20 blocked/proposed live/deployment todos in abbey-bot. Closing any of them with “green source gate = proven” is a category error that this assessment explicitly forbids.

**Demanded next, in dependency order:**

1. **Close Phase 2** — provision Linux ARM64 runner and obtain `require_executed_run.py` live evidence (unblocks external CI proof for all repos).
2. **Live voice proof on current head** — fresh unanimous consent → `join` → wake → barge-in → membership-close → `resume` → `leave` with no UDP socket (unblocks `runtime.rs`/`voice_session.rs` decomposition).
3. **MLX 6-smoke + tool-boundary re-proof on deployed 12B** — streamed text, forced tool call, continuation, vision/OCR fixtures, offline restart (unblocks MLX primary claim).
4. **HNSW decomposition at trigger** — `hnsw/graph.rs` + `hnsw/persist.rs` behind crate-private trait, WAL corruption property preserved.

Do not add features to make the numbers look better. Make the smallest scoped change that closes the next evidence gap, re-run **all 4 gates**, and record the new evidence here.

---

*Generated for `abbey-bot/QUALITY_ASSESSMENT.md` on 2026-09-03. Re-run gates before citing; do not carry numbers forward without re-measuring.*
