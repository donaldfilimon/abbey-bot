# Residual MLAI / Abbey Ops Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (or subagent-driven-development) to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status (2026-09-03 ~21:50 ET):** Unblocked code path closed through #62. Abbey live on `main` @ `15c0f15`, binary SHA `931f0186…` (PID **26416**, connected as Abbey; do not kickstart). Activity client on GitHub Pages; **Portal URL map still Donald**. MLX-Audio `:8181` 200; Ollama `:11434` reasoner; `:8282` unpublished (Task 6). Dependabot enabled with safe ignores — `rustls-webpki` alerts still open until serenity+poise crates.io bump (serenity `next` breaks poise 0.6.2). Task 3 Discord structure OK (no mass grant). Task 4 quesar still Hostinger parking (`byte`/`pixel.dns-parking.com`) until confirmed GCP `project_id` + LB IP + Cloudflare (no tofu apply / NS change without Donald).


**Goal:** Close remaining unblocked Abbey/MLAI work after IWL, personality, Discord gap-fill, and MLX-Audio came up; leave human-gated DNS and live voice as explicit Donald steps.

**Architecture:** Prefer landing the already-open voice/sidecar PR (#47), which supersedes the conflicting launchd-only PR (#42). Keep Discord API-first. Do not swap Hostinger NS without LB IP. Brand freeze: IWL on Abbey only.

**Tech Stack:** Rust abbey-bot, launchd, Discord Bot API, OpenTofu/Cloud Run/Cloudflare for quesar (Donald-gated), GitHub PRs.

## Global Constraints

- Never paste Discord/API tokens into chat, PRs, or logs.
- Never force-push `main`. Never commit `.env`.
- `ABBEY_BOT_LLM_ENDPOINT` is host-only `http://127.0.0.1:11434` (`dialect.rs` appends `/v1/chat/completions`). Vision keeps `/v1`.
- MLX-Audio readiness: prefer `GET /v1/models` (and `/` / `/health` if present). Installer on main uses webrtcvad → `importlib.metadata`.
- Gap-fill Discord; do not wipe roles/channels. No mass Member grant without Donald.
- Quesar never gets IWL copy.
- Group chat: no AskQuestion widgets; proceed on continue.

---

### Task 1: Land voice/sidecar operator PR (supersedes #42)

**Files:**
- Review/merge: `donaldfilimon/abbey-bot` PR #47 (`fix/voice-operator-sidecar-path`)
- Close: PR #42 as superseded

**Interfaces:**
- Consumes: current `origin/main` @ `15f3fa9` (personality + IWL docs already merged)
- Produces: merged main with `deploy/check-launchd-env.sh`, fail-closed join without 10-minute hang when LLM missing, operator-facing sidecar errors

- [x] **Step 1: Check mergeability of #47**

```bash
gh pr view 47 --repo donaldfilimon/abbey-bot --json mergeable,mergeStateStatus,statusCheckRollup,url
```

Expected: see CONFLICTING or MERGEABLE. If CONFLICTING, rebase onto `origin/main`.

- [x] **Step 2: If conflicting, rebase on Mac checkout**

```bash
cd ~/dev/active/abbey-bot
git fetch origin
git checkout fix/voice-operator-sidecar-path
git rebase origin/main
# resolve conflicts favoring: personality strings from main + voice fail-closed from branch
python3 deploy/test-check-launchd-env.py
python3 scripts/check-privacy.py
cargo test --locked connect_failures_are_operator_not_listening_copy timeouts_tell_the_operator_to_retry_status operator_env_presence_withholds_values
git push --force-with-lease
```

- [x] **Step 3: Merge #47** (#42 already merged earlier as e3e2e06)

```bash
gh pr merge 47 --repo donaldfilimon/abbey-bot --squash --delete-branch
gh pr close 42 --repo donaldfilimon/abbey-bot --comment "Superseded by #47 (includes launchd env checker + voice/sidecar fail-closed)."
```

- [x] **Step 4: Rebuild and reinstall live binary**

```bash
cd ~/dev/active/abbey-bot && git checkout main && git pull --ff-only
cargo build --release
install -m 755 target/release/abbey-bot "$HOME/.local/libexec/abbey-bot/abbey-bot"
launchctl kickstart -k "gui/$(id -u)/com.donaldfilimon.abbey-bot"
# confirm connected + generation backend; strings include warm, sharp friend
```

---

### Task 2: Sync Mac checkouts after IWL merges

**Files:**
- Modify (local only): `~/dev/active/mlai`, `~/dev/active/abbey`, `~/dev/active/abi` if present

- [x] **Step 1: Fast-forward each checkout**

```bash
for d in mlai abbey abi abbey-bot; do
  [ -d ~/dev/active/$d/.git ] || continue
  git -C ~/dev/active/$d fetch origin
  git -C ~/dev/active/$d checkout main
  git -C ~/dev/active/$d pull --ff-only origin main || echo "FF blocked: $d"
done
```

Expected: mlai has IWL from WWW #42; abbey/abi brand docs present.

---

### Task 3: Discord residual policy note (no mass grant)

**Files:**
- Optional post: `#bot-ops` only if documenting policy, not mutating structure

- [x] **Step 1: API snapshot confirm no unsafe gaps** (2026-09-03: Member/#help/#bot-ops present; no mutations)

```bash
# Bot API GET roles/channels; token from .env never echoed
# Confirm Member-gated #help; onboarding send channels; staff locked
```

- [ ] **Step 2: Leave for Donald (do not auto-apply)**

Document blockers only:
1. Broader Member grants (~59 humans) so `#help` serves community
2. Whether Admin gets Administrator (Land Lord currently holds it)
3. Join Office Hours for live `/voice` 8/8

---

### Task 4: quesar.cloud DNS cutover (Donald-gated checklist)

**Files:**
- Ops checklist: `/workspace/mlai-redesign-docs/DNS-CUTOVER.md`
- Infra: `~/dev/active/mlai/apps/web/infra`

- [x] **Step 1: Re-verify parking (read-only)**

```bash
dig +short NS quesar.cloud
# expect byte.dns-parking.com + pixel.dns-parking.com
```

- [ ] **Step 2: Do NOT swap NS** until Donald completes:

1. `gcloud auth application-default login` + GCS tofu state bucket
2. `tofu apply` → record `load_balancer_ip`
3. GitHub Actions `WIF_PROVIDER` / deployer vars (Deploy to Cloud Run currently fails OIDC)
4. Cloudflare zone: proxied A → LB IP; mail DNS-only copy
5. Then Hostinger NS → Cloudflare NS

---

### Task 5: Plan document commit (optional)

- [x] **Step 1: Commit this plan on a docs branch**

```bash
cd ~/dev/active/abbey-bot
git checkout -B docs/residual-ops-plan origin/main
git add docs/superpowers/plans/2026-09-03-residual-ops.md
git commit -m "docs: residual ops plan after IWL/personality/MLX"
git push -u origin HEAD
gh pr create --title "docs: residual ops plan 2026-09-03" --body "Tracks unblocked merge of #47, checkout sync, and Donald-gated DNS/voice."
```

---


---


### Task 7a: Safe Dependabot config (landed)

**Files:** `.github/dependabot.yml`

- [x] **Step 1:** Enable weekly cargo + Actions + deploy pip.
- [x] **Step 2:** Ignore serenity/poise majors and lone `rustls-webpki` bumps (breaks poise 0.6.2).

### Task 7: Dependabot rustls-webpki (blocked on serenity+poise)

**Files:** `Cargo.toml` / `Cargo.lock` (serenity 0.12.5 → tokio-tungstenite 0.21 → rustls-webpki 0.102.8)

- [x] **Step 1: Root cause** — alert #4 high (and #1–#3) from serenity's tungstenite pin; parallel tree already has webpki 0.103.15.
- [x] **Step 2: Probe serenity `next`** — `[patch]` to `serenity-rs/serenity@next` fails resolve: poise 0.6.2 needs feature `client` removed/renamed on next. Do not ship.
- [ ] **Step 3: Revisit** when crates.io serenity (≥ tungstenite 0.28) + matching poise land together. Abbey does not use CRL verification today.

---
### Task 6: MLX-VLM tool-continuation (blocked on 4-bit Gemma)

**Files:**
- `deploy/patch-mlx-vlm-tool-encoding.py` (install-time openai.py / prompt_utils.py JSON parse + chat_template mapping-before-sequence)
- `deploy/install-mlx-vlm-launchd.sh` (runs the patcher; still fail-closes on `TOOL_CONTINUATION_READY`)
- `docs/MLAI-LIVE-ACCEPTANCE.md`

- [x] **Step 1: Confirm encoding vs model**

JSON tool-body→mapping and `tool_body is mapping` before `is sequence` are required (string bodies become `value:"{…}"`; dicts 500 on Jinja sequence). They do **not** stop the `<|channel>thought` loop after a tool result. `--enable-thinking` is not sufficient.

- [ ] **Step 2: Do not publish `:8282`**

Keep `ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:11434`. Re-run `deploy/install-mlx-vlm-launchd.sh` only after a later snapshot/checkpoint actually returns exact `TOOL_CONTINUATION_READY`.

## Self-Review

1. Spec coverage: unblocked code = Task 1–2; Discord policy = Task 3; DNS = Task 4; plan persistence = Task 5; VLM continuation = Task 6 (blocked).
2. No placeholders for code tasks; Donald gates are explicit stop points.
3. #47 supersedes #42 — do not try to merge both.
