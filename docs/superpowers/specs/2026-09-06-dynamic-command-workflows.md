# Abbey command and function redesign

Approved by Donald on September 6, 2026 at 07:53 EDT. Implementation is in progress. This extends the existing command-center architecture; approval alone is not deployment evidence.

## Outcome

Abbey should make the next useful action obvious. A person should be able to open `/help`, choose what they want to do, see whether it is available here, and either complete it or receive a specific recovery step. Every accepted interaction must finish with a result, a truthful partial result, or an actionable failure. Silent accepted interactions are defects; deliberate unsolicited silence and voice wake-name/agreement gates remain correct behavior.

The immediate repair is separate: the installed September 4 binary and its registered command list are older than the current canonical source. Existing source fixes include acknowledgement before command checks, member voice status, diagnostics, an admin dashboard, memory browsing, and image menus. Those should be validated and installed before attributing their old failures to the current implementation.

## Approaches considered

| Approach | Benefit | Limitation |
|---|---|---|
| Repair the deployment and improve command descriptions | Smallest change; makes existing fixes available | Leaves people navigating commands and inferring readiness |
| Extend the existing catalog into guided, state-aware workflows | Reuses current policy and handlers; supports slash commands and buttons consistently | Requires typed availability reasons and some handler extraction |
| Build a separate web dashboard or replace the command surface | More layout freedom | Adds another product, authentication boundary, and opportunity for policy drift |

Recommend the second approach, preceded by the deployment repair. Keep existing command names and classic Discord components. A new website is outside this design.

## User experience

`/help` opens a private home view with Conversation, Memory, Images, Voice & Music, and Server tools. Show a small number of relevant actions first, with a way to browse everything the caller is permitted to discover. Recheck permissions and current state on every action; an old button never grants authority.

Each action has a status and a concrete next step:

- **Ready:** the required configuration and recent relevant readiness evidence are present. Actual execution still validates its inputs and dependencies.
- **Needs input:** request the missing question, attachment, member, or choice.
- **Needs setup or agreement:** explain the prerequisite, with a safe route to the corresponding control.
- **Busy:** show that work is pending and offer refresh or cancellation where the operation supports it.
- **Temporarily unavailable:** show the fixed failure category and recovery step.
- **Unknown:** no recent readiness observation exists; offer a bounded check rather than labeling the function healthy.

Do not expose hidden channels, other members' private records, provider secrets, or operator paths in member views. An action the caller cannot discover remains hidden. A discoverable feature that is temporarily unavailable remains visible with an explanation.

## Function-by-function scope

| Area | Proposed flow | Completion evidence |
|---|---|---|
| Conversation and persona | Ask through a modal or the existing slash command; state reply visibility; show pending generation and categorized failures | Answer delivered, explicit cancellation, or actionable failure |
| Summaries | State the available conversation scope and handle an empty history directly | Bounded summary or clear empty-state response |
| Memory, recall, reputation, pending replacements | Open the existing private browser; guide add, inspect, replace, and forget; preserve subject permissions | Confirmed local storage, a receipt where episode gating applies, or truthful pending/refused/partial result; never a premature saved claim |
| Images and OCR | Guide attachment selection and offer the existing message menu; distinguish description, text extraction, and follow-up generation | Result or a specific unsupported-input/provider message |
| Voice | Show destination visibility, presence, processing mode, current phase, caller agreement, and the next allowed action | Consent-complete activation and a human-heard response for live acceptance |
| Music | Label music controls separately from listening; expose sidecar/output readiness and distinguish pause, stop, and resume | Playback state and later audible acceptance; playing music never grants listening agreement |
| Member and permission tools | Use member selection and scoped explanations; distinguish a recommendation from an executed moderation action | The requested read or explicit statement that no mutation was performed |
| Server planning and webhook guidance | Clearly label preview and instructions; show what the bot can actually execute | A generated plan or guidance, without suggesting it was applied |
| Administration | Use the existing private dashboard; put routine settings first, with operational detail accessible to authorized managers | Current state read back or a categorized failure |
| Natural conversation about capabilities | Ground capability answers in a bounded runtime summary and point to the exact supported command | No invented commands, installations, permissions, or completed actions |

The existing seven-tool model vocabulary remains unchanged in this scope. Natural-language requests for arbitrary Discord administration do not become executable merely because the bot can discuss them. Extending that vocabulary requires a separate contract decision.

## Review findings to fix first

The independent review covers the 51 catalog leaves and their shared command surfaces. It identifies five current-source defects or operational UX gaps and two product improvements. Detailed triggers and source locations are in `abbey-command-review.md`.

1. Replace the generic boolean rejection with typed reasons. Keep `/voice status` useful when voice is unconfigured; currently its own explanatory branch is blocked by the catalog guard.
2. Match readiness to the exact operation: tool-enabled conversation is different from read-only text, and OCR is different from image description.
3. Handle component response failures explicitly. A successful setting change followed by a failed response must not look like a no-op or cause the setting mutation to be replayed.
4. Replace generic framework error rendering with bounded, categorized responses and content-free failure records.
5. Show requested policy and effective behavior together. For example, unsolicited replies may be enabled yet blocked by learning being off; provider vision support may exist while guild vision is disabled.
6. Turn clearly labeled task controls into actual guided entry points, while retaining a separate command reference.
7. In active voice status, tell the person how to address Abbey with a wake-name example, alongside the stop control.

## Architecture

Extend `command_catalog` with a pure availability result rather than creating a second permission system. Its input combines the existing eligibility facts with typed readiness observations. Access authorization, discoverability, readiness, and input completeness remain distinguishable so that an unavailable provider does not erase the route to help.

The projection must use the exact provider request class and tools policy used by execution. It must combine provider capability with guild policy and voice lifecycle state; a broad "generation available" or "vision available" boolean is insufficient.

Use shared typed operations beneath slash and component adapters. A guided action must invoke the same validated operation as its slash counterpart; it must not manufacture a slash interaction, bypass the command guard, or duplicate business logic. Keep transport acknowledgement and response rendering in the Discord adapters.

Add one bounded command outcome path: accepted, awaiting input, running, succeeded, partially completed, cancelled, or failed. Every accepted action has exactly one terminal outcome. Partial writes and uncertain effects must remain explicit. Automatic retry is permitted only when the operation is proven safe to repeat.

Readiness observations are evidence with an age, not a permanent configuration flag. The first implementation should reuse existing provider and service observations and a small bounded refresh mechanism. Expensive model qualification remains an operator action; opening help must not load models, capture audio, call remote providers, or trigger a full qualification suite.

Preserve owner, guild/channel scope, and expiration on private controls. Revalidate after input collection and immediately before mutation. Expired controls explain how to reopen the relevant view. Voice stop and agreement withdrawal retain their immediate media shutdown semantics.

## Implementation sequence

1. Verify and deploy the existing source fixes; compare installed artifact identity and Discord's registered command list.
2. Incorporate the independent command review into a complete command/function matrix. Classify each finding as fixed in source, deployment-only, remaining defect, or proposed UX change.
3. Fix review findings 1–5, then introduce typed availability and recovery reasons with pure tests covering authorized discovery, missing input, stale readiness, busy providers, and blocked voice prerequisites. Include operation-success/response-failure tests.
4. Update the help home and section views. Keep old commands operational while guided actions are added in small vertical slices.
5. Add shared operations and guided inputs for conversation, memory, images, then voice/music and administration. Each slice includes its own terminal outcome and component authorization tests.
6. Add bounded capability explanations to conversational context while preserving the frozen tool vocabulary.
7. Run the strict repository gate and deployment checks. Qualify the installed artifact, then perform user-authorized Discord checks and a currently consented voice session.

## Acceptance

Source tests must cover every registered command key, acknowledgement before remote work, visible rejection, cancellation, partial effects, expired controls, changed permissions, provider outages, empty input/history, and cross-guild isolation. Button and slash entry points must agree on authority and resulting behavior.

Deployment acceptance must compare the actual installed binary and current command payload with the validated build. A registered command is not proof that a person can complete it.

Human Discord acceptance should exercise each workflow family with one ordinary member and an authorized manager. Voice additionally needs current participant agreement, an audible answer, interruption, participant-change pause, resume, and leave. Synthetic speech proves the local pipeline only.

The redesign is complete only after the selected workflow families have both source evidence and live acceptance; unsupported functions must be labeled as unsupported, not counted as passing.
