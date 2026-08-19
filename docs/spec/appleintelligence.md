# Apple Intelligence — Core AI, Foundation Models, and the ABIEngine seam

Closes the `ABIEngine` hole. `bot-architecture.md` declared the inference seam
"out of scope"; personas depended on `complete(system:input:context:)` with nothing
behind it. This file is what's behind it.

## ⚠ Beta-software boundary — read before using any of this

Everything in this file that is marked **[FM26]** comes from WWDC26 (June 2026) and
targets **iOS 27 / iPadOS 27 / macOS 27 / watchOS 27**, which are in developer beta as
of writing. Production is expected fall 2026. Apple changes beta API between seeds.

Verified against the WWDC26 session 241 transcript and its published code samples —
type names, method shapes, and property paths below are quoted from Apple's own
samples, not reconstructed from memory. What is *not* verified: exact module
availability annotations, the PCC entitlement identifier, and whether the open-source
FoundationModels package builds cleanly on Linux today. Those are marked `NOTE(SDK):`.

Do **not** ship Abbey's Linux production path against beta APIs. The architecture below
is explicitly two-tier for that reason.

## The architectural shift

Apple introduced a **`LanguageModel` protocol** — a model abstraction layer where
on-device, server, third-party, and open-source models all back a single
`LanguageModelSession`. **[FM26]**

That collapses Abbey's inference seam into Apple's. `ABIEngine` no longer needs to be a
bespoke provider protocol with three hand-rolled implementations; it becomes a thin
actor over `LanguageModelSession`, and swapping backends is a package-manifest change.

Conforming models available:

| Model | Where it runs | Notes |
|---|---|---|
| `SystemLanguageModel` | On-device, Apple silicon | Rebuilt in the 27 cycle; gained **vision**. Context ~8192 (query `contextSize`) |
| `PrivateCloudComputeLanguageModel` | Apple's servers | 32K context, reasoning levels, no API keys/auth to manage, requires an entitlement |
| `CoreAILanguageModel` | On-device ANE/GPU | Open source. Backed by **Core AI**, the WWDC26 successor to Core ML for generative workloads (`.aimodel` format) |
| `MLXLanguageModel` | On-device GPU | Open source. Runs MLX models |
| Anthropic / Google packages | Their servers | First-party Swift packages conforming to `LanguageModel`; you own OAuth + billing |
| Chat-Completions model | Any OpenAI-compatible endpoint | Ships in the **Foundation Models framework utilities** package |

**The one that matters most for Abbey:** the core FoundationModels framework is being
**open sourced**, explicitly so it works "everywhere Swift runs, including Linux
servers," and the utilities package includes a `LanguageModel` that speaks the Chat
Completions standard. That means the Linux bot and the macOS companion app can share
one inference protocol — the thing the old three-provider seam was faking.

### Two-tier deployment — non-negotiable

`SystemLanguageModel`, `PrivateCloudComputeLanguageModel`, `CoreAILanguageModel`, and
`MLXLanguageModel` require Apple hardware and Apple OSes. **Abbey's Linux gateway
process cannot use any of them.** The split:

```
Linux (Vapor gateway, production)   → FoundationModels + ChatCompletions LanguageModel
                                       → any OpenAI-compatible endpoint
macOS 27 / iPadOS 27 (companion)    → SystemLanguageModel / PCC / CoreAI / MLX
                                       → on-device, private, zero marginal cost
```

Both sides call the same `ABIEngine` API. Only the injected `LanguageModel` differs.

## ABIEngine — the seam, implemented

```swift
import FoundationModels

/// Backends are injected, not branched on. Every conforming model — on-device,
/// PCC, CoreAI, MLX, Anthropic, Google, or a Chat-Completions endpoint — is a
/// `LanguageModel`, so this actor is written once and never learns about vendors.
actor ABIEngine {
    static let shared = ABIEngine()

    private var model: any LanguageModel
    private var sessions: [String: LanguageModelSession] = [:]   // key: scoped channel ID

    private init(model: (any LanguageModel)? = nil) {
        self.model = model ?? ABIEngine.defaultModel()
    }

    /// Compile-time platform split. Linux has no Apple models to fall back to, so the
    /// Chat-Completions backend is not optional there — it is the only path.
    private static func defaultModel() -> any LanguageModel {
        #if canImport(Darwin)
        return SystemLanguageModel()
        #else
        // NOTE(SDK): from the FoundationModels utilities package. Verify the exact
        // initializer label set against the open-source package before building —
        // the session described it as "a language model that can interface with
        // servers using the Chat Completions standard" without showing its init.
        return ChatCompletionsLanguageModel(
            endpoint: URL(string: Environment.get("ABBEY_LLM_ENDPOINT") ?? "https://api.openai.com/v1")!,
            model: Environment.get("ABBEY_LLM_MODEL") ?? "gpt-4o-mini",
            apiKey: Environment.get("ABBEY_LLM_KEY") ?? ""
        )
        #endif
    }

    func setModel(_ newModel: any LanguageModel) {
        model = newModel
        sessions.removeAll()      // transcripts are model-scoped; don't carry them across
    }

    /// One session per channel so multi-turn context is per-conversation, not global.
    /// This is what the old stateless `complete()` could not express.
    private func session(for scopeKey: String, instructions: String) -> LanguageModelSession {
        if let existing = sessions[scopeKey] { return existing }
        let created = LanguageModelSession(model: model, instructions: instructions)
        sessions[scopeKey] = created
        return created
    }

    /// The call personas already make. Signature preserved so no persona changes.
    func complete(system: String, input: String, context: PersonaContext,
                  scopeKey: String = "global") async -> String {
        let s = session(for: scopeKey, instructions: system + "\n\n" + render(context))
        do {
            let response = try await s.respond(to: input)
            await MetricsCollector.shared.record(usage: response.usage, scope: scopeKey)
            return response.content
        } catch {
            return "I hit an error thinking about that."
        }
    }

    /// Multimodal path — the on-device model gained vision in the 27 cycle. **[FM26]**
    /// Attachments accept UIImage, NSImage, CGImage, Core Image types, CoreVideo pixel
    /// buffers, and file URLs, at any size or aspect ratio (larger costs more tokens).
    #if canImport(Darwin)
    func completeWithImage(system: String, input: String, imageURL: URL,
                           scopeKey: String = "global") async -> String {
        let s = session(for: scopeKey, instructions: system)
        do {
            let response = try await s.respond {
                input
                Attachment(imageURL)
            }
            return response.content
        } catch {
            return "I couldn't read that image."
        }
    }
    #endif

    /// Reasoning is a PCC capability, surfaced per-request rather than per-session.
    func completeReasoning(system: String, input: String, depth: ReasoningLevel,
                           scopeKey: String = "global") async -> String {
        let s = session(for: scopeKey, instructions: system)
        do {
            let response = try await s.respond(
                to: input,
                contextOptions: ContextOptions(reasoningLevel: depth)
            )
            return response.content
        } catch {
            return "I hit an error thinking about that."
        }
    }

    /// Context-window pressure check. `contextSize` and `tokenCount(for:)` landed in
    /// iOS 26.4 — use them rather than guessing a budget per model.
    func willOverflow(_ prompt: String) async -> Bool {
        guard let counted = try? await model.tokenCount(for: prompt) else { return false }
        return counted > (model.contextSize * 3 / 4)
    }

    private func render(_ c: PersonaContext) -> String {
        var out = ""
        if !c.channelSummary.isEmpty { out += "Recent channel context: \(c.channelSummary)\n" }
        if !c.userFacts.isEmpty { out += "Known about this user: \(c.userFacts.joined(separator: "; "))\n" }
        out += "User standing: \(String(format: "%.2f", c.reputation))"
        return out
    }
}
```

`response.usage` exposes `input.totalTokenCount`, `input.cachedTokenCount`,
`output.totalTokenCount`, and `output.reasoningTokenCount` — real per-request cost
accounting, wired into `MetricsCollector` above. With third-party server models you are
billed per token, so this is not optional instrumentation.

## Personas as Dynamic Profiles — replacing ABIRouter's manual switch

`multi-guild.md` routes intent → persona by constructing a new persona object per
message. **Dynamic Profiles** **[FM26]** express that declaratively: one session, whose
instructions, tools, model, and reasoning level all resolve from app state, while the
conversation transcript survives the switch.

That last part is the real win. The old `ABIRouter` discarded context on every persona
change, so switching Abbey → Aviva mid-conversation lost the thread.

```swift
@Observable
final class AbbeyStates {
    var persona: PersonaKind = .abbey
    var guildID: String = ""
}

/// A DynamicProfile resolves to exactly one active Profile at any moment. Conditionals
/// pick the branch; the framework handles the transition and preserves history.
struct AbbeyProfile: LanguageModelSession.DynamicProfile {
    let states: AbbeyStates

    var body: some DynamicProfile {
        switch states.persona {
        case .abbey:
            Profile {
                Instructions {
                    """
                    You are Abbey. Direct, street-smart, reads people fast. Terse — \
                    no pleasantries, no sign-offs. Match the user's energy and length.
                    """
                }
                RememberFactTool(guildID: states.guildID)
                SwitchPersonaTool(states: states)
            }

        case .aviva:
            Profile {
                Instructions {
                    """
                    You are Aviva. Analytical, structured, precise. Lead with the \
                    mechanism. State tradeoffs explicitly. No hedging, no filler.
                    """
                }
                RememberFactTool(guildID: states.guildID)
                ReputationLookupTool(guildID: states.guildID)
                SwitchPersonaTool(states: states)
            }
            // Aviva handles the technical/moderation load — give her the bigger model
            // and deep reasoning. Modifiers apply per-branch, not per-session.
            .model(PrivateCloudComputeLanguageModel())
            .reasoningLevel(.deep)

        case .abi:
            Profile {
                Instructions {
                    """
                    You are Abi. Warm, adaptive, rapport-first. Build comfort before \
                    information. De-escalate rather than match heat.
                    """
                }
                SwitchPersonaTool(states: states)
            }
        }
    }
}

let session = LanguageModelSession(profile: AbbeyProfile(states: states))
```

**Availability caveat:** `.model(PrivateCloudComputeLanguageModel())` only resolves on
Apple platforms. On Linux that branch must fall back to the injected default model —
gate the modifier behind `#if canImport(Darwin)` or the profile won't compile.

## Tools — where slash commands and the model converge

Apple's `Tool` protocol lets the model call into Abbey's own systems. This is the piece
that makes the bot genuinely agentic rather than a prompt wrapper: the model decides to
store a fact or check reputation, instead of an intent classifier guessing.

```swift
/// Lets the model persist a durable fact about a user — the self-learning write path,
/// now model-initiated rather than keyword-triggered by IntentClassifier.
struct RememberFactTool: Tool {
    let description = "Store a durable fact about a user for future conversations."
    let guildID: String

    @Generable
    struct Arguments {
        @Guide(description: "Discord user ID the fact is about")
        let userID: String
        @Guide(description: "A single concise fact, stated in third person")
        let fact: String
    }

    func call(arguments: Arguments) async throws -> some PromptRepresentable {
        try await MemoryStore.shared.appendFact(
            userID: arguments.userID, guildID: guildID, fact: arguments.fact)
        return "Stored: \(arguments.fact)"
    }
}

struct ReputationLookupTool: Tool {
    let description = "Look up a user's standing in this server before acting on them."
    let guildID: String

    @Generable
    struct Arguments {
        @Guide(description: "Discord user ID to look up")
        let userID: String
    }

    func call(arguments: Arguments) async throws -> some PromptRepresentable {
        let score = await SocialBrain.shared.reputation(
            userId: arguments.userID, guildId: guildID, db: Abbey.db)
        return "Reputation \(String(format: "%.2f", score)) (0 = poor, 1 = excellent)."
    }
}

/// The model can hand off to another persona mid-conversation without losing transcript.
struct SwitchPersonaTool: Tool {
    let description = "Switch to a persona better suited to the conversation."
    let states: AbbeyStates

    @Generable
    struct Arguments {
        @Guide(description: "One of: abbey, aviva, abi")
        let persona: PersonaKind
    }

    func call(arguments: Arguments) async throws -> some PromptRepresentable {
        states.persona = arguments.persona
        return "Switched to \(arguments.persona)."
    }
}
```

### System tools — free capability

Two Vision-backed tools ship built in **[FM26]**: `OCRTool` (structured text out of
images) and `BarcodeReaderTool`. Adding `OCRTool()` to a Profile makes the `/ocr` slash
command in `vision.md` largely redundant on Apple platforms — the model calls it itself
when a user posts a screenshot. Keep the explicit command for the Linux path.

A **Spotlight-powered search tool** enables fully local RAG. On the companion app that
is a real answer to the open WDBX question *for locally indexed content* — but it
indexes the Mac's Spotlight database, not Abbey's guild memory. It does **not** resolve
open decision #3; guild-scoped semantic recall still needs WDBX.

## Core AI — custom models

Core AI is the WWDC26 successor to Core ML for generative workloads: `.aimodel` format,
`coreai-torch` for PyTorch conversion, pre-optimized open models (Qwen, Mistral, SAM3)
in Apple's `coreai-models` repo, Xcode integration for inspecting model metadata and
function signatures.

For Abbey this is the escape hatch when neither the system model nor a hosted endpoint
fits — a fine-tuned persona model running on the Neural Engine, reached through
`CoreAILanguageModel` so it plugs into the same seam:

```swift
#if canImport(Darwin)
// NOTE(SDK): CoreAILanguageModel is open source and new; confirm the initializer
// against the shipped package. The .aimodel bundle is produced by coreai-torch.
let custom = CoreAILanguageModel(modelURL: Bundle.main.url(
    forResource: "abbey-persona", withExtension: "aimodel")!)
await ABIEngine.shared.setModel(custom)
#endif
```

Core ML is **not** deprecated — it remains correct for classification, object detection,
and smaller non-generative models. Abbey's own `NeuralNetwork`/`DQNAgent` in `brain.md`
stay hand-rolled Swift; they are tiny, and Core AI targets a different scale entirely.
Do not port the DQN to Core AI.

## Evaluations — measuring whether the learning loop works

The **Evaluations framework** **[FM26]** quantifies output quality as prompts change.
This is the missing counterpart to `adaptive-learning.md`: the DQN optimizes a reward
signal, but nothing currently verifies persona prompts got *better*. Wire an eval suite
over a fixed set of representative guild messages before tuning persona instructions,
otherwise reward-shaping and prompt edits are indistinguishable.

## fm CLI — operations

macOS 27 ships an `fm` command-line tool with on-device and PCC access **[FM26]**.
Useful for Abbey ops without a build: `fm chat` to iterate on persona instructions
interactively, or piping transcript exports through `fm` to summarize an incident.
Not part of the running bot — a development and operations convenience.
