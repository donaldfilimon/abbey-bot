# AbbeyCompanion — macOS 27 / iPadOS 27 SwiftUI app

The control surface for the bot. **Additive** — it does not replace the Linux Vapor
gateway, and it is not a second bot backend. It observes, configures, and provides
on-device inference the Linux host cannot.

## ⚠ Same beta boundary as `apple-intelligence.md`

macOS 27 and iPadOS 27 are in developer beta as of writing (production expected fall
2026). Items marked **[OS27]** target that cycle. Ship the app against macOS 26 /
iPadOS 26 and gate 27-only surfaces behind `if #available`, or you have an app that
cannot be released until fall.

## Why it exists

Three things the Linux process structurally cannot do:

1. **On-device inference.** `SystemLanguageModel`, PCC, Core AI, MLX are Apple-only.
   The companion can answer from the on-device model at zero marginal cost and full
   privacy, then hand results back to the gateway.
2. **A real operator UI.** The web dashboard in `bot-architecture.md` is a thin JSON
   API. Live transcripts, per-guild config, reputation inspection, and the
   confirmation gate for destructive moderation want a native app.
3. **Human-in-the-loop approval.** Destructive actions (ban, purge, role changes)
   route to the device for explicit confirmation rather than firing autonomously.

## Package layout

```
AbbeyCompanion/
├── Package.swift                    # swift-tools-version: 6.4
├── Sources/AbbeyCompanion/
│   ├── AbbeyCompanionApp.swift      # @main App
│   ├── Engine/
│   │   ├── AbbeyEngine.swift        # central orchestrator, @Observable
│   │   ├── GatewayLink.swift        # WebSocket to the Vapor host
│   │   ├── EventBus.swift           # AsyncStream broadcast
│   │   ├── ConfirmationGate.swift   # human-in-the-loop actor
│   │   └── MetricsCollector.swift   # token usage + latency
│   ├── Models/                      # SwiftData mirrors of the Fluent models
│   │   ├── SDGuild.swift
│   │   ├── SDUserMemory.swift
│   │   ├── SDMessage.swift
│   │   └── SDInteractionLog.swift
│   ├── Intelligence/
│   │   ├── LocalInference.swift     # ABIEngine bound to on-device models
│   │   └── PersonaProfile.swift     # DynamicProfile definitions
│   └── Views/
│       ├── RootView.swift           # NavigationSplitView shell
│       ├── GuildListView.swift
│       ├── LiveFeedView.swift
│       ├── MemoryInspectorView.swift
│       ├── ReputationView.swift
│       ├── PersonaTunerView.swift
│       ├── ConfirmationQueueView.swift
│       └── MetricsView.swift
└── Tests/AbbeyCompanionTests/
```

```swift
// Package.swift
// swift-tools-version: 6.4
import PackageDescription

let package = Package(
    name: "AbbeyCompanion",
    // Ship-safe floor. Raise to .v27/.v27 only when those are GM and you accept
    // dropping every user still on 26.
    platforms: [.macOS(.v26), .iOS(.v26)],
    products: [.executable(name: "AbbeyCompanion", targets: ["AbbeyCompanion"])],
    targets: [
        .executableTarget(
            name: "AbbeyCompanion",
            // No @main + main.swift together — SwiftPM treats main.swift as
            // top-level code and the two entry points collide.
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
        .testTarget(name: "AbbeyCompanionTests", dependencies: ["AbbeyCompanion"]),
    ]
)
```

## App entry + SwiftData

```swift
import SwiftUI
import SwiftData

@main
struct AbbeyCompanionApp: App {
    @State private var engine = AbbeyEngine()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(engine)
        }
        .modelContainer(for: [SDGuild.self, SDUserMemory.self,
                              SDMessage.self, SDInteractionLog.self])
        #if os(macOS)
        .commands { AbbeyCommands(engine: engine) }

        Settings { SettingsView().environment(engine) }
        #endif
    }
}
```

SwiftData models **mirror** the Fluent schema; they are a local cache fed by the
gateway link, not the source of truth. Postgres on the Linux host remains canonical.
Conflict resolution is last-write-wins from the server — the app never pushes memory
rows directly, only commands.

```swift
import SwiftData

@Model
final class SDUserMemory {
    #Unique<SDUserMemory>([\.discordUserID, \.guildID])

    var discordUserID: String = ""
    var guildID: String = ""
    var facts: [String] = []
    var reputation: Double = 0.5
    var interactionCount: Int = 0
    var updatedAt: Date = Date.now

    init(discordUserID: String, guildID: String, facts: [String] = [],
         reputation: Double = 0.5, interactionCount: Int = 0) {
        self.discordUserID = discordUserID
        self.guildID = guildID
        self.facts = facts
        self.reputation = reputation
        self.interactionCount = interactionCount
    }
}
```

## AbbeyEngine — orchestrator

```swift
import Observation

@Observable
@MainActor
final class AbbeyEngine {
    private(set) var guilds: [GuildSummary] = []
    private(set) var liveEvents: [LiveEvent] = []
    private(set) var pendingConfirmations: [PendingAction] = []
    private(set) var connectionState: ConnectionState = .disconnected

    var selectedGuildID: String?

    private let link = GatewayLink()
    private let gate = ConfirmationGate()

    func connect(host: URL, token: String) {
        Task {
            connectionState = .connecting
            do {
                try await link.connect(to: host, token: token)
                connectionState = .connected
                for await event in await link.events { await handle(event) }
            } catch {
                connectionState = .failed(error.localizedDescription)
            }
        }
    }

    private func handle(_ event: GatewayLink.InboundEvent) async {
        switch event {
        case .message(let m):
            liveEvents.insert(.message(m), at: 0)
            if liveEvents.count > 500 { liveEvents.removeLast() }   // bounded

        case .confirmationRequest(let action):
            pendingConfirmations.append(action)
            #if os(macOS) || os(iOS)
            await NotificationPresenter.shared.alert(
                title: "Abbey needs approval",
                body: action.humanDescription)
            #endif

        case .guildSnapshot(let summaries):
            guilds = summaries
        }
    }

    /// Nothing destructive executes without passing through here.
    func resolve(_ action: PendingAction, approved: Bool) {
        Task {
            await gate.resolve(action.id, approved: approved)
            try? await link.send(.confirmationResult(id: action.id, approved: approved))
            pendingConfirmations.removeAll { $0.id == action.id }
        }
    }
}
```

## ConfirmationGate — the human-in-the-loop actor

```swift
/// Suspends a destructive action until a human answers, or the timeout denies it.
/// Default-deny on timeout is deliberate: an unattended device must not become an
/// implicit approval channel.
actor ConfirmationGate {
    private var waiting: [UUID: CheckedContinuation<Bool, Never>] = [:]

    func request(_ action: PendingAction, timeout: Duration = .seconds(120)) async -> Bool {
        await withTaskGroup(of: Bool.self) { group in
            group.addTask { await self.suspend(action.id) }
            group.addTask {
                try? await Task.sleep(for: timeout)
                await self.resolve(action.id, approved: false)
                return false
            }
            let result = await group.next() ?? false
            group.cancelAll()
            return result
        }
    }

    private func suspend(_ id: UUID) async -> Bool {
        await withCheckedContinuation { continuation in
            waiting[id] = continuation
        }
    }

    func resolve(_ id: UUID, approved: Bool) {
        // Removal before resume — a double-resolve (human answers as the timeout
        // fires) would otherwise resume the same continuation twice and trap.
        guard let continuation = waiting.removeValue(forKey: id) else { return }
        continuation.resume(returning: approved)
    }
}
```

## Views — adaptive across macOS and iPadOS

```swift
struct RootView: View {
    @Environment(AbbeyEngine.self) private var engine

    var body: some View {
        @Bindable var engine = engine

        NavigationSplitView {
            GuildListView(selection: $engine.selectedGuildID)
                .navigationSplitViewColumnWidth(min: 200, ideal: 240)
        } content: {
            if let guildID = engine.selectedGuildID {
                LiveFeedView(guildID: guildID)
            } else {
                ContentUnavailableView("Select a server",
                                       systemImage: "server.rack")
            }
        } detail: {
            InspectorTabs()
        }
        .overlay(alignment: .top) {
            if !engine.pendingConfirmations.isEmpty {
                ConfirmationBanner()
                    .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .animation(.snappy, value: engine.pendingConfirmations.count)
    }
}
```

`NavigationSplitView` gives a three-column Mac layout and collapses correctly to
iPad's slide-over/compact widths without a separate iPad view tree.

### iPadOS-specific **[OS27]**

- **Pointer + keyboard**: `.onHover`, `.hoverEffect(.highlight)` on feed rows;
  `.keyboardShortcut` on approve/deny in the confirmation queue (`⌘⏎` / `⌘⌫`).
- **Multiple windows / Stage Manager**: `WindowGroup(for: String.self)` keyed by guild
  ID so an operator can pin two servers side by side.
- **External display**: the Mac-like windowing in the 27 cycle means the same split
  view is usable on a connected monitor without layout work.
- **Drag and drop**: `.dropDestination(for: URL.self)` on the feed to push an image
  through the vision path.

## Local inference — on-device, gateway-assisting

```swift
import FoundationModels

/// The companion runs inference the Linux host can't. Results are posted back to the
/// gateway, which owns delivery to Discord — the app never talks to Discord directly.
actor LocalInference {
    private var session: LanguageModelSession?
    private let states: AbbeyStates

    init(states: AbbeyStates) { self.states = states }

    /// Availability is a real branch, not a formality: the model is unavailable on
    /// ineligible hardware, with Apple Intelligence off, or while assets download.
    func isReady() -> Bool {
        switch SystemLanguageModel.default.availability {
        case .available: return true
        case .unavailable: return false
        }
    }

    func unavailabilityReason() -> String? {
        guard case .unavailable(let reason) = SystemLanguageModel.default.availability
        else { return nil }
        switch reason {
        case .deviceNotEligible:          return "This device can't run on-device models."
        case .appleIntelligenceNotEnabled: return "Turn on Apple Intelligence in Settings."
        case .modelNotReady:               return "Model assets are still downloading."
        @unknown default:                  return "On-device model unavailable."
        }
    }

    func respond(to prompt: String) async throws -> String {
        let s = session ?? LanguageModelSession(profile: AbbeyProfile(states: states))
        session = s
        return try await s.respond(to: prompt).content
    }
}
```

`@unknown default` matters here — `UnavailableReason` is a frozen-looking enum in a
beta framework, and Apple has added cases mid-cycle before.

## Full slash-command surface

Registered by `CommandRegistry` on the gateway (see `discordbm-api.md`). The companion
mirrors these as UI affordances; it does not register commands itself.

| Command | Options | Persona/system | Gated |
|---|---|---|---|
| `/ask` | `question` (str, req), `persona` (choice, opt) | ABIEngine | — |
| `/remember` | `fact` (str, req), `user` (user, opt) | MemoryStore | — |
| `/forget` | `fact` (str, autocomplete) | MemoryStore | mod |
| `/recall` | `user` (user, opt) | MemoryStore | — |
| `/reputation` | `user` (user, opt) | SocialBrain | — |
| `/persona` | `name` (choice: abbey/aviva/abi) | ABIRouter | mod |
| `/see` | `image` (attachment, req), `question` (str, opt) | vision | — |
| `/ocr` | `image` (attachment, req) | vision / `OCRTool` | — |
| `/join` | `channel` (channel, opt) | VoiceSessionManager | — |
| `/leave` | — | VoiceSessionManager | — |
| `/say` | `text` (str, req) | TTS → voice | — |
| `/transcribe` | `enabled` (bool, req) | STT | mod |
| `/summarize` | `count` (int 10–200, opt) | ABIEngine | — |
| `/config` | subcommands: `view`, `set`, `reset` | GuildRegistry | admin |
| `/admin` | subcommands: `brain`, `flush`, `export`, `reload` | multiple | admin |
| `/stats` | `scope` (choice: guild/global) | MetricsCollector | — |

`/admin brain` exposes DQN state (ε, step count, buffer fill) — the observability the
adaptive loop needs to be debuggable rather than a black box.

### Autocomplete — the piece most bots skip

`/forget` over a free-text fact list is unusable without it.

```swift
case .applicationCommandAutocomplete:
    guard let focused = data.options?.first(where: { $0.focused == true }),
          let partial = focused.value?.asString else { break }

    let facts = await MemoryStore.shared.facts(userID: invokerID, guildID: guildID)
    let matches = facts
        .filter { $0.localizedCaseInsensitiveContains(partial) }
        .prefix(25)                                   // Discord's hard cap
        .map { Interaction.ApplicationCommand.Choice(name: String($0.prefix(100)),
                                                     value: .string($0)) }

    try? await client.createInteractionResponse(
        id: interaction.id, token: interaction.token,
        payload: .autocompleteResult(.init(choices: Array(matches)))
    ).guardSuccess()
```

Both caps are Discord-enforced: 25 choices max, 100 characters per choice name. Exceed
either and the interaction fails rather than truncating.

## Gateway link security

The app authenticates to the Vapor host with the same bearer secret as the dashboard
(`ABBEY_DASHBOARD_SECRET`), over `wss://` only. Store it in the Keychain, never in
`UserDefaults` or the app bundle:

```swift
func storeToken(_ token: String) throws {
    let query: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: "com.abbey.companion",
        kSecAttrAccount as String: "gateway",
        kSecValueData as String: Data(token.utf8),
        // Device-only, requires an unlocked device — this token can moderate a server.
        kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
    ]
    SecItemDelete(query as CFDictionary)
    let status = SecItemAdd(query as CFDictionary, nil)
    guard status == errSecSuccess else { throw KeychainError.store(status) }
}
```

If third-party `LanguageModel` packages (Anthropic, Google) are used from the app, the
same rule applies to their keys — fetch via OAuth, store in Keychain, never embed in
the binary. Apple's guidance on this is explicit and the token-usage API exists
precisely because you are billed per token.
