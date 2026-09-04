# MLX-VLM tool-result continuation: root cause

Status: **diagnosis only.** Nothing here has been run against the model. The
candidate fixes are ranked and each carries the exact command that would test
it. No sidecar was started and no 12B checkpoint was loaded to produce this.

## The failure

`deploy/smoke-mlx-vlm.py` gates publication of the `:8282` sidecar. Two probes
pass and the third fails:

| Probe | Result |
|---|---|
| Streamed plain chat returns exactly `MLX_READY`, `finish_reason=stop` | pass |
| Forced `probe_status` tool call, exact args, `finish_reason=tool_calls` | pass |
| **Tool-result continuation returns exactly `TOOL_CONTINUATION_READY`, `finish_reason=stop`** | **fail** |

The recorded signature (`docs/MLAI-LIVE-ACCEPTANCE.md`) is that the 4-bit Gemma
checkpoint "loops `<|channel>thought` into content until `finish_reason=length`".
The gate is `deploy/smoke-mlx-vlm.py:494-499`; the installer fail-closes on it,
and Ollama remains the reasoner.

The ledger attributed this to the checkpoint. That is not the whole story, and
the useful half is fixable.

## Root cause: two defects that compose

### 1. The chat template never emits the thought-suppressor after a tool response

In the pinned snapshot's `chat_template.jinja` (revision
`73bcf09092aa277861d5a191b989b666f7f32e8f`):

{% raw %}
```jinja
358  {%- if add_generation_prompt -%}
359      {%- if ns.prev_message_type != 'tool_response' and ns.prev_message_type != 'tool_call' -%}
360          {{- '<|turn>model\n' -}}
361          {%- if not enable_thinking | default(false) -%}
362              {{- '<|channel>thought\n<channel|>' -}}
363          {%- endif -%}
364      {%- endif -%}
365  {%- endif -%}
```
{% endraw %}

Line 362 is a **pre-filled, already-closed empty thought block**. It is what
stops the model opening a thought channel of its own, and it is why the plain
`MLX_READY` probe passes.

It is never emitted on the continuation turn. The reason is a guard placement:

{% raw %}
```jinja
216  {%- for message in loop_messages -%}
217      {%- if message['role'] != 'tool' -%}
218      {%- set ns.prev_message_type = None -%}
```
{% endraw %}

The reset on line 218 sits **inside** the `role != 'tool'` guard on line 217, so
a `tool` message never clears `ns.prev_message_type`. It stays `'tool_response'`
(set at line 268 or 313), line 359's condition is false, and the whole block —
including the suppressor — is skipped. The prompt ends at `<tool_response|>`
with nothing suppressing reasoning, and the model opens `<|channel>thought`
unprompted.

Note that line 359 also suppresses `<|turn>model`, which appears deliberate: a
tool response continues the model's turn rather than starting a new one. **A fix
must emit the suppressor without also emitting a duplicate turn marker.**

### 2. The server's thought-splitter is a one-shot latch

`mlx_vlm/server/responses_state.py` (`ThinkingStreamState`) treats
`("<|channel>thought", "<channel|>")` as always-active markers, independent of
`enable_thinking`. The first block is absorbed as reasoning and latches
`thinking_done = True`. Every block after that is emitted as **content**, marker
text included, because the remaining strip handles only `<|START_TEXT|>` /
`<|END_TEXT|>`.

So "loops into content" is a splitter-latch artifact, not purely a checkpoint
quirk: defect 1 causes the model to open a thought channel it should never have
opened, and defect 2 is why the repeats land in `content` rather than being
absorbed.

## Ruled out — do not spend time here

- **A client `stop` sequence cannot work.** The chat-completions request model
  has no `stop` field, and its base config sets `extra="allow"`, so a `stop`
  array is **silently ignored, not rejected**. It would look applied and do
  nothing — the worst kind of dead end.
- **`enable_thinking` / thinking-budget cannot help while thinking is off.** The
  budget logits-processor is gated on `enable_thinking` being truthy, so with
  thinking disabled there is nothing forcing the channel closed. Consistent with
  the failed experiments already recorded.
- **Rust-side marker stripping is not the fix.** The model would still burn its
  full output budget and return `length`, which `src/llm/dialect.rs` correctly
  rejects. Worth doing as defence in depth (below), not as the remedy.

## Candidate fixes, ranked

### 1. Emit the suppressor in the `tool_response` case (primary)

Extend `patch_chat_template_source` in
`deploy/patch-mlx-vlm-tool-encoding.py` with a fourth anchored, idempotent
insertion so line 359's branch emits `<|channel>thought\n<channel|>` for
`tool_response` too, without a duplicate `<|turn>model`.

The existing patcher already follows this shape — anchored `_insert_once` with
marker-based idempotency and a `SystemExit` when an anchor drifts — so this is
an extension, not a new mechanism. `install-mlx-vlm-launchd.sh` already invokes
the patcher against the model template, so no installer change is needed.

**Open question that must be resolved first.** `docs/MLAI-LIVE-ACCEPTANCE.md`
records that "generation-prompt experiments also failed". It is **unknown**
whether that meant exactly this prefill-after-tool-response, or something else
such as passing `add_generation_prompt=True` on the continuation call. Confirm
before treating this as untried; if it was already tried in this exact form,
candidate 2 becomes primary.

Test, model-free:
```
python3 deploy/test-patch-mlx-vlm-tool-encoding.py
```
Test, live (yours):
```
python3 deploy/smoke-mlx-vlm.py --base-url http://127.0.0.1:<ephemeral> --model <snapshot path>
```
`chat_template.jinja.bak-pre-mapping` sits beside the live template as an
untouched baseline for reverting.

### 2. Reset the splitter latch on a re-opened marker

If candidate 1 does not converge, the complementary lever is to make
`ThinkingStreamState` re-enter the thinking state when a marker re-opens, rather
than latching after the first block. Same patcher mechanism, different upstream
file. This does not stop the model reasoning; it stops the markers reaching
`content`.

### 3. `logit_bias` on the channel token (last resort)

The request model does accept `logit_bias`. Biasing the `<|channel>` token id
strongly negative would suppress reopening. The token id is **unknown** and
would come from `tokenizer.json` in the snapshot. This treats the symptom and
would need the field added in both the smoke and `src/llm/dialect.rs`.

## Defence in depth, worth doing regardless

**No Rust code anywhere strips channel markers.** Today `finish_reason=length`
fails closed, so nothing leaks. But a checkpoint that finished with `stop` while
emitting markers would render them into Discord verbatim. A strip in the
streamed and non-streamed response paths, with a unit test, closes that
independently of whether MLX-VLM is ever published.

## Two related gaps found while diagnosing

- **The smoke fixture is not production-representative.** Abbey's tool results
  are prose (`src/tools.rs`), but the encoding patch only fires on strings
  beginning `{` or `[`. Production tool results take the `is string` path that
  the smoke's `{"marker":"ready"}` fixture never exercises. A checkpoint could
  pass the smoke and still mis-render real tool results.
- **The cutover gate is weaker than the install gate.** The exact-marker
  fail-close lives only in `deploy/smoke-mlx-vlm.py`. `configure-mlx-primary.py`
  gates on a manifest's self-declared `tools: pass`, so a manifest generated
  from a tool-*call*-only probe would let cutover proceed with continuation
  still broken. Moot today because no manifest exists — worth hardening before
  one does.

## What would close `tasks/todo.md`'s continuation item

Only a live `deploy/smoke-mlx-vlm.py` run passing the exact
`TOOL_CONTINUATION_READY` assertion. A template patch plus a green unit test is
evidence that the patch applies, not that the model continues.
