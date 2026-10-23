# MLAI release baseline execution — 2026-10-22

**Status: Partial; full release not accepted.** Latest: source and candidate qualified
on published main `6eb5cff`; see the final fixed-main section. Earlier pending
entries below are chronological observations. Requirement coverage and external
prerequisites are in the [release matrix](2026-10-22-mlai-release-matrix.md).

## Source and publication

Entry HEAD: `7b1b3f0b4ec8640e6c18c3ecb6d4e291d1d783dd`, clean branch `pr190`.
Remote main observed through `git ls-remote`:
`1e272bf393e14486078c08f407bb1149161e0640`. PR #190 remains the separate
MLX-VLM dependency upgrade. This execution uses `abbey/mlai-release-baseline`;
its parent includes the three unmerged MLX/lock/merge commits. Publication must
preserve that dependency and qualify the final main SHA separately.

The current canonical macOS Gate succeeded for the observed main revision:
[run 39314808196](https://github.com/donaldfilimon/abbey-bot/actions/runs/39314808196).
That run does not qualify this branch or its pending changes.

## Confirmed repair

The first `ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` exited 1 at the
RustSec inventory gate: unexpected RUSTSEC-2026-0381 and RUSTSEC-2026-0382.
Earlier stages passed; downstream stages were not reached.

The exact root and standalone patched-Serenity lockfiles now select
`rustls-webpki 0.103.16` and `webpki-roots 1.0.10`, fixing the newly reported
emSign trust defects. The compatibility roots package remains 0.26.11 and
re-exports the patched 1.x roots. Targeted lock regeneration changed no manifests,
production source or accepted-debt policy. Builds and validation retain `--locked`.

References: [roots advisory](https://rustsec.org/advisories/RUSTSEC-2026-0381.html),
[verification advisory](https://rustsec.org/advisories/RUSTSEC-2026-0382.html).

`python3 scripts/check-rustsec-debt.py` now exits 0: one previously accepted
ringbuf vulnerability remains, plus three informational unmaintained warnings.
This is not a clean audit. `python3 scripts/check-linux-tls-tree.py` exits 0;
Rustls/WebPKI remains selected and native TLS/OpenSSL absent.

README now reflects macOS-only CI and the MLX-VLM 0.7.8 deployment lock;
`.env.example` no longer names the obsolete service version. Public APIs,
commands, provider routing and persistence formats are unchanged.

## Current checks and sibling evidence

- `npm --prefix activity test`: exit 0; 20 passed, zero failed/skipped.
- `python3 scripts/check-pages-liquid.py`: exit 0 before the new release records;
  final documentation checks must cover the final diff.
- Strict repaired source gate: pending; no current complete source pass claimed.
- WDBX current HEAD: `36bc99b699de60be6c07b2838d74580084fb5d4c`.
  Current sibling, Abbey and CI-pinned revision
  `7ddeb3d1389ec0e9fab5ac5397acf0da91a01174` projection fixtures all have
  SHA-256 `a4ec232c6980e009b77936386c9b233b864abb2d6b66b6253624d2f7a474be90`.
  Fixture equality is not qualification of all current sibling source.
- ABI current HEAD: `69910bf61205458dec27dd4b616201e0a5f3898a`.
  Its October 8 ledger names the foundation and aggregate family-isolation
  qualification, published as `5c275dd`; later ABI work is outside that receipt.
  No new full ABI/WDBX source qualification or trained-model claim is made here.

## Managed host diagnosis and external prerequisites

`python3 -I deploy/service-status.py` exits 1 with unavailable/unverified status.
The wrapper imports successfully. No matching Abbey launchd label appeared;
`~/.config/abbey-bot/env` and `~/.local/libexec/abbey-bot/abbey-bot` are absent.
The entry checkout also had no built `target/release/abbey-bot`.
This host lacks the managed installation/configuration evidence; it is not
established that a running service has failed. Do not synthesize credentials,
install a second gateway, or infer readiness from a plist.

Next operator input: identify/provision the intended managed Mac and its private
environment. Provider qualification, candidate/installed comparison and the
transactional deployment depend on that prerequisite and final-main CI.
Live voice V1–V8, 100-label human adjudication, text/ordinary-member journeys,
up-to-three-recipient initiative pilot, independent moderation assessment and
Activity host/TLS/Portal/two-participant witnesses remain open. No deployment,
restart, live Discord message, moderation activation or Portal change occurred.

## Local target and validation correction

Donald selected this Mac as the managed deployment target. No repo `.env`,
owner wrappers or Discord token in the process environment were present.
Ollama responds on 11434; the documented `gemma4:12b` is absent, while
`gemma4:latest` and `gpt-oss:20b` are available. The MLX speech/vision endpoints
on 8181/8282 are unavailable. Availability is not provider qualification.
An owner-only `~/.config/abbey-bot/env.pending` prepares inactive local settings;
its token and exact model remain blank. It is not the service environment.
No paid/cloud model was selected and no service was installed or started.

GitHub reports the repository's only macOS runner offline; the documented local
runner/fleet paths are absent. Exact-head CI remains blocked on runner restoration.

The first repaired gate exited 101 at Clippy under Homebrew Rust 1.99.1, which
bypassed the pinned toolchain. Two new needless-borrow lints were reported in
`src/wdbx.rs`; no source workaround was applied before testing pinned Rust.
Its 1,062 captured file inputs were unchanged, but this wrong-toolchain run
does not qualify the pinned release. During that run the identical source
was committed as `b7794a8226831491a14e4a7d1802bc73e28c8253`; the commit is preserved.
Evidence is retained outside the repository under
`/Users/donald/.codex/verification/abbey-bot-release-20261022/`.

The next complete gate explicitly prepends the Rust 1.98.0 toolchain bin directory
and pins RUSTC/RUSTDOC; version preflight confirms Cargo 1.98.0, Rustc 1.98.0 and
Clippy 0.1.98. Independent review of both lock repairs and complete requirement
coverage found no blocking issues. No full repaired source pass is yet claimed.

## Pinned-run Pages correction

The first pinned Rust 1.98.0 gate exited 1 at the maintained Pages inventory:
after the release records became tracked, its explicit expected set lacked the
two new documents. Added exactly those two paths; the existing 21 selector tests
and Liquid scan pass. The frozen failed run kept all 1,062 inputs unchanged.
No Clippy/build/test pass is claimed from this early exit. A complete pinned
rerun follows this narrow correction.

The independent human-learning worksheet is prepared outside the repository at
`/Users/donald/.codex/verification/abbey-bot-release-20261022/human-learning-review.json`.
All 100 human verdicts are null; agent labels are in a separate comparison file.
The frozen corpus SHA-256 accompanies the worksheet. This preparation does not
constitute independent adjudication or fixed-provider answer qualification.

## Final fixed-main source and candidate qualification

2026-10-23 continuation of the October 22 release execution.

Source **6eb5cff114c22dd7a2d0f78f98c2a1c2df8bf215** passed the complete strict
`ABBEY_REQUIRE_WDBX_CONFORMANCE=1 ./check.sh` under explicitly selected Rust
1.98.0. The detached sibling worktree isolated source from concurrent changes.
Actual exit **0**; elapsed 1121.93 seconds; **1,062 inputs unchanged**.
Manifest SHA-256 `e4ea19d527dab189bc39d4c7659e836752197d33648b47631db1d6ad4ad59238`; full log SHA-256
`907a77e6968834a508c7ef63681c2a46844c1a9277472f0c90a9011123bbca98`. Evidence and saved executable remain outside source
under `/Users/donald/.codex/verification/abbey-bot-release-20261022/final-main/`.

Observed checks: 2,356 Rust tests passed, zero failed, eight ignored; two patched
Serenity transport regressions passed; warnings-denied bot Clippy, Swift 12+16,
strict sibling fixture parity, debt policy, locked release and offline text/learning
startup checks passed. Activity separately passed 20/20 and Pages selectors 21/21.
The remaining accepted ringbuf vulnerability and three informational warnings
are unchanged; the audit is not clean. Existing upstream Serenity warnings remain
visible. These counts describe this snapshot, not the October 19 mixed source.

The existing Rust workflow was found manually disabled and re-enabled as source
validation only. The runner subsequently came online. Exact-source
[Gate (macOS)](https://github.com/donaldfilimon/abbey-bot/actions/runs/39894628930)
completed **success** for this same SHA. PRs #190/#191 are merged. Any earlier
offline/disabled statements above describe observations before that restoration.

The prior canonical run exited 0 but changed Cargo.lock during execution; its
manifest comparison failed. It is retained as mixed-input evidence and does not
supply source qualification. The fixed-main run above replaces it for that claim.

Saved candidate binary SHA-256:
`96ebd720fba80e58db716921c9f9af8d55696ea3824ce9534b442ffac321a9a9`. Its bytes match the earlier saved candidate exactly, so
the successful synthetic provider report applies to this exact artifact: local
Ollama `gemma4:latest`, OS build 26B118, fixture abbey-provider-fixtures-v1; text,
streaming, structured output, tools, vision and OCR all pass. Report exit 0,
17.09 seconds. No PCC/cloud route or live Discord credential was used.

The inactive owner-only `env.pending` now proposes that qualified local
text/tool/vision route; existing nonblank choices and credential contents are
preserved. The actual managed `env` and installed executable remain absent.
This is candidate qualification, not installed identity, live answer quality or
speech qualification. MLX speech endpoints, human voice, six Discord cases,
installed baseline comparisons, held-out human labels/answers and other live
acceptance remain open. No service installation or start occurred.

The current immutable Activity package was prepared and verified with Node
v26.12.0. Package identity `b17bfb0c4bb3ee0c5ee7195d19dcf15e16f4801919c11cb9a416fd3d4534d20a`;
Court digest `b81786c9d780ba01a7d1c757825ae4b898998c44358fc60f5e4a8c9dc1f93530`.
It remains prepared-source-only, ingress proposal-only and activation
operator-required; origin is unset and no prior installed assets establish a
rollback target. No host/TLS/Portal/iframe readiness is inferred.

These qualification-record updates follow the frozen source gate. They receive
focused Pages/Liquid/diff validation; they do not rewrite the qualified manifest
or claim that a later documentation commit already passed exact-head CI.

## Broader provider reliability remains open

The unchanged saved release artifact ran the actual 48-case synthetic workload
against local Gemma: exit **1**, 43.31 seconds, **19 successes / 29 failures /
0 incomplete / 11 no-text**. The model bundle identity remained unchanged.
Failures were 18 fixture mismatches, one response-schema failure and ten later
invalid-request refusals. A minimal synthetic reproduction returned `7.0` for
a fixture requiring exactly `7`; this identifies an exact-format mismatch, not
incorrect arithmetic. No scorer, workload, acceptance limit or production prompt
was weakened to change that result. The tool/protocol failure remains unresolved.

Observed candidate provider-first-text p95 was 5,438 ms across 37 observed samples
out of 48 attempts. This is not Discord first-visible latency or an installed
baseline. The CLI's named-artifact argument pointed to the saved candidate copy;
its report explicitly retains managed-service, declared-model/hardware, delivery,
voice-contention and capability-qualification gaps. No installed identity is
claimed by that pathname or by this measurement. Raw reports, model/hardware
identity and result exits are retained beside the frozen gate evidence.

A bounded alternative probe used existing local `gpt-oss:20b` for text with
separately identified Gemma vision. Exit **1**, 57.84 seconds: text and Gemma
vision/OCR passed; streaming (`stream_protocol`), structured output and tools
(`tool_protocol`) failed. Its metadata reported no remote host/model; no cloud
model, download or production route change was used. A full 48-case run was
not performed for this candidate after required capability failure.

The six passing Gemma capability probes are retained as narrow candidate evidence,
not promoted to release reliability. The proposed Gemma settings remain inactive
and require review of these failed benchmark cases before route selection.
Next provider work is an identity-bound local serving/model qualification repair
or an explicit acceptance decision, followed by the unchanged workload and then
installed/Discord measurements. Private Discord configuration, speech setup and
all human/public acceptance remain open. The full MLAI release is **Partial**.

## 2026-10-23 provider repair investigation

The provider repair keeps the exact saved executable above and leaves the
synthetic prompts, fixtures, protocol validation, scoring and acceptance limits
unchanged. A direct GPT-OSS stream had valid SSE framing but returned a marker
with incorrect capitalization; that observation does not justify relaxing the
stream parser. Qwen's deterministic candidate passed text/streaming and Gemma
vision/OCR, but failed tools/structured output. A direct synthetic Qwen tool
request produced the correct call and nonce after 108.54 seconds, then returned
`ABB_EY_PROVIDER_CONTINUATION_V1` instead of the exact requested marker. Qwen
remains unselected, and its failures remain recorded.

A separate local Ollama profile, `abbey-gptoss20b:t0-20261023`, derives from the
already installed `gpt-oss:20b` with temperature changed from 1 to 0. Its source
model is preserved; no model download, cloud route, production selection or
service action occurred. Parent manifest digest:
`f38aa0c53da5f8c49d08c43a99df24ff53167fe68e24664a7777288e7656fdfe`;
candidate manifest digest:
`8a541710a0caa203c8eb7d43dbcf6bc6d3e0548ece3adbc794d9b2f1d92e4f07`.
Gemma vision remains separately identified by manifest
`eb2177adf7646d15faead94fdad6d3e170dc8015653418a3b7d5d52a6380be5b`.
Resolved parameters, local model metadata, recipe, unchanged-binary identity and
raw reports are retained under
`/Users/donald/.codex/verification/abbey-bot-provider-repair-20261023/`.

All six narrow capability checks passed for that GPT text/Gemma vision
combination: actual exit 0, 29.93 seconds. This is a candidate result, not
installed readiness or a replacement for the broader workload. Independent
review requires a fixed-profile baseline before any same-model latency claim;
a comparison against failed Gemma measurements cannot establish that claim.

Discord MCP discovery inspected the available tool inventory and searched the
plugin catalog. No usable Discord connector was exposed; the search returned
only Vercel. No credential was fetched, displayed or fabricated. Private bot
configuration remains a local-install prerequisite. Additional connectors may
exist outside this session's available inventory.

The unchanged 48-case GPT candidate workload subsequently exited **1** after
253.62 seconds: **40 successes / 8 failures / 0 incomplete / 5 no-text**.
All 24 arithmetic and 12 coding cases passed. Tool cases had three exact-output
mismatches, one response-schema failure and four later admission refusals after
the lane degraded. The manifest identity remained unchanged. Provider-first-text
p95 was 8,909 ms across 43/48 observed samples; completion p95 was 12,375 ms
across 43/48. These measurements do not pass reliability acceptance and do not
establish comparable installed/Discord latency. The profile remains inactive.

A synthetic direct reproduction of case 43 returned the correct first tool call
and nonce, followed by a `stop`/`[DONE]` continuation with empty answer content.
Its reasoning text was neither displayed nor retained. The retained diagnostic
contains only reasoning character counts and content-free protocol frames plus
synthetic answers/tool arguments. Rejecting that empty answer is required; no
parser or fixture change follows from this model-output failure.

A third existing local candidate (`abbey-bonsai2:t0-20261023`, derived from
`bonsai-2:latest` with temperature 0) failed required text, streaming,
structured-output and tool checks: exit 1, 191.01 seconds. Gemma vision/OCR
passed. Its resolved parent/candidate identities remain in the external receipt;
it was not promoted and no broader workload followed failed required probes.

The documented `gemma4:12b` prerequisite was absent: `gemma4:latest` reported
8,074,934,862 parameters and is a different model identity. The documented 12B
model was downloaded separately in 218.07 seconds. Its parent manifest is
`c7597fc90b86aff6965317f37dd5465402284e26382d9b4bf2dfe3ef1491f4b6`.
A separate `abbey-gemma4-12b:t0-20261023` profile sets temperature 0, preserving
the original tag; candidate manifest:
`0a0bda2c9df30bb61ae2183f5d2414be95af7953b3959e00589d7781f9041915`.
All six narrow capabilities passed for this same text/vision profile: exit 0,
85.28 seconds, unchanged model identity and exact saved binary. Its unchanged 48-case
workload subsequently passed: exit 0, 255.38 seconds, 48 successes, zero failures,
zero incomplete and zero no-text; model identity remained unchanged. No source, prompt, parser, scoring
or acceptance change is made by adding a local model artifact.

The qualification-record PR #192 merged as
`890adf0af9b3c27180dbcd1f14fe56557b6e3951` and its exact-main
[Gate (macOS)](https://github.com/donaldfilimon/abbey-bot/actions/runs/39898551292)
completed successfully. The delta from the frozen 6eb5cff source contains only
README, acceptance/matrix records and task-ledger documentation. Provider
receipts above continue to name the actual saved 6eb5cff executable hash rather
than relabeling that artifact as a later build.


The qualified 12B profile's initial candidate provider-first-text p95 is
14,917 ms across 48/48 observations; completion p95 is 15,374 ms across 48/48.
This is the initial fixed-profile measurement, not evidence of <=10% regression
against the different 8B/GPT models, and not installed/Discord latency. The
[committed synthetic receipt](2026-10-23-provider-repair.json) preserves exact
identities, the successful 48-case report and hashes/summaries of failed runs.
Independent review found no recipe/capability promotion blocker and required
these baseline and installation distinctions.

The inactive owner-only `env.pending` now proposes the qualified 12B t0 tag for
text and vision; only the two previously proposed `gemma4:latest` model values
were changed. Credentials and other owner choices were preserved. The actual
managed environment and bot installation remain absent. No bot or speech service
was installed, started or restarted. Speech qualification, private Discord access,
transactional installation and all required live/human witnesses remain open.
The full MLAI release remains **Partial**.
