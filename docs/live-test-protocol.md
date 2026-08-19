# Live test protocol — Discord web client (written 2026-08-19, not yet executed)

Operator drives Discord in Chrome as Donald; the bot runs from this clone with
`ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:11434 ABBEY_BOT_LLM_MODEL=gemma4:12b
ABBEY_QUIET=1 ABBEY_DATA_DIR=<dir> RUST_LOG=info,abbey_bot=debug`. Every
pipeline outcome is one `info` line: `message handled … outcome=…` /
`reaction handled … outcome=…`. Deadlines: 5 s for non-model commands, 90 s for
anything through the model. Anything not seen by its deadline is recorded as
**NOT observed** with the last relevant log line.

## A — DM (no MESSAGE_CONTENT needed)
1. DM `hi abbey, what can you do here?` → generated reply-to within 90 s; log `guild=None … outcome=Replied`.
2. DM `and what did I just ask you?` → references turn 1 (per-DM transcript).
3. `/remember fact:"Donald tests Abbey from the web client"` → "Stored about @Donald"; `/recall` lists it.
4. DM `what do you remember about me?` → mentions the fact (WDBX recall folded in).
5. 👍 on Abbey's reply → log `reaction handled … outcome=Rewarded`.
6. `/forget` → autocomplete shows the fact; "Forgotten."; `/recall` → none.
7. `/persona ask` works in the DM (no longer guild-only).

## B — guild slash commands
1. `/stats` → record `messages seen`, brain line, `pending rewards`, backend/vision.
2. `/admin show` → `persona: abbey · learning: on · voice: on · vision: on · cooldown: 20s`.
3. `/admin vision off` (vision falls back to the LLM endpoint and gemma has no vision).
4. `/admin brain` → `ε 0.100 · learn steps 0 · replay buffer 0/10000 · experiences 0`.
5. `/remember` / `/recall` / `/forget` in the guild; the fact must not appear in the DM's `/recall`.
6. `/persona ask question:"In one sentence, what is Discord?"` → `**Abbey** — answered via …`.
7. `/persona ask … as:Aviva` → `**Aviva**` header.
8. `/summarize` before C → honest "not seen any messages"; after C → model summary.

## C — mention → reply → reward → settle
1. `@Abbey hello, say something short` (pick from popup) → typing, reply-to within 90 s; log `mentions_bot=true outcome=Replied`.
2. `/stats` → `pending rewards: 1`, brain loaded.
3. 👍 on the reply → `outcome=Rewarded`.
4. Wait ≥ 150 s (settle tick 30 s). `/admin brain` → `replay buffer 1/10000 · experiences 1`; `/stats` pending back down.
5. `/admin brain epsilon:0.0`; `/admin flush` → files in the data dir; `/summarize`; `/admin reset`; `/admin export` → JSON attachment.

## D — requires MESSAGE_CONTENT (Dev Portal toggle)
Only with `ABBEY_QUIET=1` still on: a plain message → `outcome=Ignored("quiet")` proves content arrived and the guard held. Watching the policy itself on a 58-guild token is deliberately NOT run without a per-guild allow-list.
