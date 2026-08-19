# Abbey Bot — Architecture (Swift/Vapor/DiscordBM)

Structural reference. DiscordBM call-site details (gateway init, slash commands,
interactions, permissions) are in `discordbm-api.md`, not repeated here.

## Canonical Architecture

| Layer | Technology |
|---|---|
| Language | Swift 6.4 (via `swift-tools-version: 6.4` — dependency manifests on older tools versions, e.g. DiscordBM's 6.1, don't constrain the consumer) |
| Canonical host | Apple Silicon native (macOS 26+) — required for the Apple-framework seam implementations (Vision, Speech). Linux/Docker remains fully supported via the remote seam implementations |
| HTTP server | Vapor 4.121.x |
| Discord library | DiscordBM v1.16.x (actor-isolated REST + Gateway) — **no Voice**; voice is implemented at protocol level, see `voice.md` |
| Other networks | Telegram Bot API + Slack Events API adapters over raw AsyncHTTPClient — see `platforms.md` |
| ORM / DB | Fluent + PostgreSQL (FluentPostgresDriver) |
| Web dashboard | Vapor routes + React (Vite/TSX) or Leaf |
| ML / self-learning | Custom neural network + per-guild DQN agents (Swift, no external deps) — see `brain.md` + `adaptive-learning.md` |
| Vision | `ImageUnderstanding` seam: Apple Vision (macOS) / remote VLM (everywhere) — see `vision.md` |
| Voice | Discord voice gateway + Opus + AES-GCM; `SpeechTranscribing`/`SpeechSynthesizing` seams — see `voice.md` |
| Intent routing | IntentClassifier → ABI persona dispatcher (per-guild defaults — see `multi-guild.md`) |
| Memory system | Per-channel + per-user context (Fluent-backed, **lexical only** — see gap below) |
| Reputation | SocialBrain: weighted scoring per user per guild |
| Personas | Abbey (default) / Aviva (technical) / Abi (warm) |
| Hosting | Apple Silicon native (canonical) or Docker/systemd on Linux (Ubuntu 22+) |

### ⚠ Gap — no vector memory layer

`UserMemory.facts` is `[String]` and `ChannelContext.summary` is free text. Recall is
exact-match/substring only; there is no embedding or semantic retrieval anywhere in this
implementation. If semantic recall gets added, the canonical store is **WDBX** —
namespace-scoped per guild/channel, consistent with the rest of the MLAI stack. Do **not**
reach for pgvector, `sqlite-vec`, or a third-party vector DB just because Postgres is
already wired up here; that would fork vector state away from WDBX. This is flagged, not
designed — the schema below has no embedding column and shouldn't grow one silently.

## Project Layout

```
AbbeyBot/
├── Package.swift
├── Sources/
│   └── App/
│       ├── configure.swift              # Vapor + DiscordBM boot
│       ├── entrypoint.swift             # @main, Application lifecycle
│       │
│       ├── Models/                      # Fluent DB models + migrations
│       │   ├── GuildMessage.swift       # Persisted message log
│       │   ├── UserMemory.swift         # Per-user fact store + reputation
│       │   ├── ChannelContext.swift     # Rolling per-channel summary
│       │   ├── ReputationEvent.swift    # Audit trail for rep changes
│       │   ├── InteractionLog.swift     # Slash command usage analytics
│       │   └── Migrations/
│       │       └── CreateAll.swift
│       │
│       ├── Handlers/                    # Event → dispatch (GatewayEventHandler)
│       │   ├── EventRouter.swift        # Conforms to GatewayEventHandler
│       │   ├── MessageHandler.swift     # onMessageCreate
│       │   ├── InteractionHandler.swift # onInteractionCreate (slash/button/select/modal)
│       │   ├── VoiceStateHandler.swift  # onVoiceStateUpdate
│       │   └── MemberHandler.swift      # onGuildMemberAdd / onGuildMemberRemove
│       │
│       ├── Commands/                    # Slash command definitions + registrar
│       │   ├── CommandRegistry.swift
│       │   ├── AskCommand.swift
│       │   ├── ReputationCommand.swift
│       │   ├── RememberCommand.swift
│       │   ├── ForgetCommand.swift
│       │   └── AdminCommand.swift
│       │
│       ├── Components/                  # Button / select / modal builders
│       │   ├── ButtonBuilder.swift
│       │   ├── SelectMenuBuilder.swift
│       │   └── ModalBuilder.swift
│       │
│       ├── Platforms/                   # Multi-social layer — see platforms.md
│       │   ├── SocialAdapter.swift      # Protocol + SocialEvent/OutboundMessage
│       │   ├── SocialRouter.swift       # Cross-platform ingest pipeline
│       │   ├── DiscordAdapter.swift
│       │   ├── TelegramAdapter.swift
│       │   └── SlackAdapter.swift
│       │
│       ├── Voice/                       # Protocol-level Discord voice — see voice.md
│       │   ├── VoiceGateway.swift       # Voice WSS handshake + heartbeats
│       │   ├── VoiceUDP.swift           # RTP + AES-GCM(rtpsize) + IP discovery
│       │   ├── OpusCodec.swift          # libopus wrapper (COpus system target)
│       │   ├── VoiceSession.swift       # Per-guild session, silence segmentation
│       │   └── Speech.swift             # SpeechTranscribing/SpeechSynthesizing seams
│       │
│       ├── Vision/                      # Image understanding — see vision.md
│       │   ├── ImageUnderstanding.swift # Protocol + ImageFetcher
│       │   ├── AppleVisionAnalyzer.swift
│       │   └── RemoteVisionAnalyzer.swift
│       │
│       ├── Guilds/                      # Multi-guild — see multi-guild.md
│       │   ├── GuildConfig.swift        # Fluent model + migration
│       │   ├── GuildRegistry.swift      # Settings cache
│       │   └── ReplyCooldown.swift
│       │
│       ├── Brain/                       # ML core — see brain.md + adaptive-learning.md
│       │   ├── NeuralNetwork.swift
│       │   ├── DQNAgent.swift
│       │   ├── ReplayBuffer.swift
│       │   ├── StateEncoder.swift       # 18-dim state, deterministic sentiment
│       │   ├── RewardCollector.swift    # Delayed reaction-based rewards
│       │   ├── BrainRegistry.swift      # Per-guild agents + BrainState persistence
│       │   ├── AbbeyScheduler.swift     # learn/flush/persist heartbeat
│       │   ├── IntentClassifier.swift
│       │   └── SocialBrain.swift
│       │
│       ├── Personas/                    # ABI persona system
│       │   ├── PersonaProtocol.swift    # `Persona` protocol + context type
│       │   ├── ABIRouter.swift          # Intent → persona dispatch
│       │   ├── AbbeyPersona.swift       # Street-smart, direct
│       │   ├── AvivaPersona.swift       # Analytical, structured
│       │   └── AbiPersona.swift         # Warm, rapport-first
│       │
│       └── Routes/                      # Vapor HTTP routes
│           ├── DashboardRoutes.swift    # Web dashboard API
│           ├── WebhookRoutes.swift      # Incoming interaction endpoint
│           └── APIRoutes.swift          # Internal REST for dashboard
│
├── Resources/
│   └── Views/                           # Leaf templates (if not React)
└── Public/                              # Static assets / React build output
```

## Persona System

```swift
struct PersonaContext: Sendable {
    var channelSummary: String
    var userFacts: [String]
    var reputation: Double
    static let empty = PersonaContext(channelSummary: "", userFacts: [], reputation: 0.5)
}

struct PersonaResponse: Sendable {
    var text: String
    var embed: Embed?
}

protocol Persona: Sendable {
    var name: String { get }
    func respond(to input: String, context: PersonaContext, db: Database) async -> PersonaResponse
}
```

### ABIRouter

**Superseded for multi-guild:** the version below is the single-guild form kept for
the actor-isolation correction history. The current implementation is the guild-scoped
`ABIRouter` in `multi-guild.md` (same actor shape, `perGuild` dimension added) — use
that one.

**Corrected.** The previous version declared `actor ABIRouter` but made every member
`static`. `static` members are *not* actor-isolated — they're nonisolated global state,
so `static var current: any Persona` is a hard error under Swift 6 language mode
("not concurrency-safe because it is nonisolated global shared mutable state"), and the
`actor` keyword bought nothing. Isolation now comes from real instance state on the
actor, reached through a `shared` singleton.

Callers change: `ABIRouter.setPersona(x)` → `await ABIRouter.shared.setPersona(x)`, and
`ABIRouter.route(intent:)` / `ABIRouter.current` likewise become `await`-ed instance
calls. Call sites in `discordbm-api.md` are updated to match.

```swift
actor ABIRouter {
    static let shared = ABIRouter()

    private var current: any Persona = AbbeyPersona()

    /// Persona for this specific intent, falling back to the sticky current persona.
    func route(intent: IntentClassifier.Intent) -> any Persona {
        switch intent {
        case .modRequest, .command: return AvivaPersona()
        case .greeting, .smallTalk: return AbiPersona()
        default: return current
        }
    }

    func activePersona() -> any Persona { current }

    @discardableResult
    func setPersona(_ name: String) -> String {
        switch name.lowercased() {
        case "aviva": current = AvivaPersona()
        case "abi":   current = AbiPersona()
        default:      current = AbbeyPersona()
        }
        return current.name
    }
}
```

### Persona Conformances

Each persona owns its register; the shared context assembly is identical, so only the
system prompt and embed accent differ.

```swift
struct AbbeyPersona: Persona {
    let name = "Abbey"
    private let accent = 0x34d399

    func respond(to input: String, context: PersonaContext, db: Database) async -> PersonaResponse {
        let text = await ABIEngine.shared.complete(
            system: """
            You are Abbey. Direct, street-smart, reads people fast. Terse — no pleasantries, \
            no sign-offs. Match the user's energy and length.
            """,
            input: input, context: context)
        return PersonaResponse(text: text, embed: nil)
    }
}

struct AvivaPersona: Persona {
    let name = "Aviva"
    private let accent = 0xa855f7

    func respond(to input: String, context: PersonaContext, db: Database) async -> PersonaResponse {
        let text = await ABIEngine.shared.complete(
            system: """
            You are Aviva. Analytical, structured, precise. Lead with the mechanism. \
            State tradeoffs explicitly. No hedging, no filler.
            """,
            input: input, context: context)
        return PersonaResponse(text: text, embed: nil)
    }
}

struct AbiPersona: Persona {
    let name = "Abi"
    private let accent = 0x22d3ee

    func respond(to input: String, context: PersonaContext, db: Database) async -> PersonaResponse {
        let text = await ABIEngine.shared.complete(
            system: """
            You are Abi. Warm, adaptive, rapport-first. Build comfort before information. \
            De-escalate rather than match heat.
            """,
            input: input, context: context)
        return PersonaResponse(text: text, embed: nil)
    }
}
```

`ABIEngine` is the inference seam. **It is implemented in `apple-intelligence.md`** —
this was previously declared out of scope, which left personas depending on a call with
nothing behind it. It is now an actor over Apple's `LanguageModel` protocol, so the
backing model (on-device `SystemLanguageModel`, `PrivateCloudComputeLanguageModel`,
`CoreAILanguageModel`, `MLXLanguageModel`, an Anthropic/Google package, or an
OpenAI-compatible Chat Completions endpoint on Linux) is injected rather than branched
on. Personas still depend only on `complete(system:input:context:)`.

Note the persona structs below construct a fresh instance per message and therefore
discard conversation transcript on every persona switch. `apple-intelligence.md`
supersedes this with **Dynamic Profiles** — one session whose instructions, tools,
model, and reasoning level resolve from state while history survives the switch. Treat
the structs here as the portable/Linux-safe form and the profile form as canonical on
Apple platforms.

## Fluent Models — Complete Set

```swift
// GuildMessage.swift
final class GuildMessage: Model, Content, @unchecked Sendable {
    static let schema = "guild_messages"
    @ID(key: .id) var id: UUID?
    @Field(key: "discord_message_id") var discordMessageId: String
    @Field(key: "channel_id")  var channelId: String
    @Field(key: "guild_id")    var guildId: String
    @Field(key: "author_id")   var authorId: String
    @Field(key: "content")     var content: String
    @Timestamp(key: "created_at", on: .create) var createdAt: Date?

    init() {}
    init(id: UUID? = nil, discordMessageId: String, channelId: String,
         guildId: String, authorId: String, content: String) {
        self.id = id
        self.discordMessageId = discordMessageId
        self.channelId = channelId
        self.guildId = guildId
        self.authorId = authorId
        self.content = content
    }
}

// UserMemory.swift
final class UserMemory: Model, Content, @unchecked Sendable {
    static let schema = "user_memories"
    @ID(key: .id) var id: UUID?
    @Field(key: "discord_user_id") var discordUserId: String
    @Field(key: "guild_id")        var guildId: String
    @Field(key: "facts")           var facts: [String]
    @Field(key: "reputation")      var reputation: Double  // 0.0 – 1.0
    @Field(key: "interaction_count") var interactionCount: Int
    @Timestamp(key: "updated_at", on: .update) var updatedAt: Date?

    init() {}
    init(id: UUID? = nil, discordUserId: String, guildId: String,
         facts: [String] = [], reputation: Double = 0.5, interactionCount: Int = 0) {
        self.id = id
        self.discordUserId = discordUserId
        self.guildId = guildId
        self.facts = facts
        self.reputation = reputation
        self.interactionCount = interactionCount
    }
}

// ChannelContext.swift — rolling 2k-token summary of recent messages
final class ChannelContext: Model, Content, @unchecked Sendable {
    static let schema = "channel_contexts"
    @ID(key: .id) var id: UUID?
    @Field(key: "channel_id") var channelId: String
    @Field(key: "guild_id")   var guildId: String
    @Field(key: "summary")    var summary: String   // compressed via ABI
    @Field(key: "message_count") var messageCount: Int
    @Timestamp(key: "updated_at", on: .update) var updatedAt: Date?

    init() {}
    init(id: UUID? = nil, channelId: String, guildId: String,
         summary: String = "", messageCount: Int = 0) {
        self.id = id
        self.channelId = channelId
        self.guildId = guildId
        self.summary = summary
        self.messageCount = messageCount
    }
}

// ReputationEvent.swift — append-only audit trail
final class ReputationEvent: Model, Content, @unchecked Sendable {
    static let schema = "reputation_events"
    @ID(key: .id) var id: UUID?
    @Field(key: "user_id")   var userId: String
    @Field(key: "guild_id")  var guildId: String
    @Field(key: "delta")     var delta: Double
    @Field(key: "reason")    var reason: String
    @Timestamp(key: "created_at", on: .create) var createdAt: Date?

    init() {}
    init(id: UUID? = nil, userId: String, guildId: String, delta: Double, reason: String) {
        self.id = id
        self.userId = userId
        self.guildId = guildId
        self.delta = delta
        self.reason = reason
    }
}

// InteractionLog.swift — slash command usage analytics
// (listed in the layout but previously missing from this "complete set")
final class InteractionLog: Model, Content, @unchecked Sendable {
    static let schema = "interaction_logs"
    @ID(key: .id) var id: UUID?
    @Field(key: "command_name")  var commandName: String
    @Field(key: "user_id")       var userId: String
    @Field(key: "guild_id")      var guildId: String
    @Field(key: "channel_id")    var channelId: String
    @Field(key: "succeeded")     var succeeded: Bool
    @OptionalField(key: "error_message") var errorMessage: String?
    @Field(key: "duration_ms")   var durationMs: Int
    @Timestamp(key: "created_at", on: .create) var createdAt: Date?

    init() {}
    init(id: UUID? = nil, commandName: String, userId: String, guildId: String,
         channelId: String, succeeded: Bool, errorMessage: String? = nil, durationMs: Int) {
        self.id = id
        self.commandName = commandName
        self.userId = userId
        self.guildId = guildId
        self.channelId = channelId
        self.succeeded = succeeded
        self.errorMessage = errorMessage
        self.durationMs = durationMs
    }
}
```

`@unchecked Sendable` is still required on Fluent models as of Fluent 4.9.x — the
property wrappers (`@ID`, `@Field`, `@Timestamp`) aren't themselves `Sendable`. This
wasn't independently re-verified this pass (low risk, hasn't changed across Fluent 4.x);
flag it for a recheck if a Fluent major version ships.

Explicit `init()` is mandatory, not stylistic: Fluent requires the empty init to
materialize rows from query results, and declaring any custom init suppresses the
compiler-synthesized memberwise one — which is why the call sites throughout
`discordbm-api.md` need these to exist.

## Migrations

```swift
// Models/Migrations/CreateAll.swift
struct CreateAll: AsyncMigration {
    func prepare(on database: Database) async throws {
        try await database.schema(GuildMessage.schema)
            .id()
            .field("discord_message_id", .string, .required)
            .field("channel_id", .string, .required)
            .field("guild_id",   .string, .required)
            .field("author_id",  .string, .required)
            .field("content",    .string, .required)
            .field("created_at", .datetime)
            .unique(on: "discord_message_id")     // gateway redelivers on RESUME
            .create()

        try await database.schema(UserMemory.schema)
            .id()
            .field("discord_user_id", .string, .required)
            .field("guild_id",        .string, .required)
            .field("facts",           .array(of: .string), .required)
            .field("reputation",      .double, .required)
            .field("interaction_count", .int, .required)
            .field("updated_at",      .datetime)
            .unique(on: "discord_user_id", "guild_id")   // reputation is per-guild
            .create()

        try await database.schema(ChannelContext.schema)
            .id()
            .field("channel_id", .string, .required)
            .field("guild_id",   .string, .required)
            .field("summary",    .string, .required)
            .field("message_count", .int, .required)
            .field("updated_at", .datetime)
            .unique(on: "channel_id")
            .create()

        try await database.schema(ReputationEvent.schema)
            .id()
            .field("user_id",  .string, .required)
            .field("guild_id", .string, .required)
            .field("delta",    .double, .required)
            .field("reason",   .string, .required)
            .field("created_at", .datetime)
            .create()

        try await database.schema(InteractionLog.schema)
            .id()
            .field("command_name", .string, .required)
            .field("user_id",      .string, .required)
            .field("guild_id",     .string, .required)
            .field("channel_id",   .string, .required)
            .field("succeeded",    .bool, .required)
            .field("error_message", .string)
            .field("duration_ms",  .int, .required)
            .field("created_at",   .datetime)
            .create()
    }

    func revert(on database: Database) async throws {
        // Reverse creation order.
        try await database.schema(InteractionLog.schema).delete()
        try await database.schema(ReputationEvent.schema).delete()
        try await database.schema(ChannelContext.schema).delete()
        try await database.schema(UserMemory.schema).delete()
        try await database.schema(GuildMessage.schema).delete()
    }
}

/// Hot-path indexes. Separate migration so it can be reverted independently.
struct CreateIndexes: AsyncMigration {
    func prepare(on database: Database) async throws {
        guard let sql = database as? SQLDatabase else { return }
        try await sql.raw("CREATE INDEX IF NOT EXISTS idx_msg_channel ON guild_messages (channel_id, created_at DESC)").run()
        try await sql.raw("CREATE INDEX IF NOT EXISTS idx_rep_user ON reputation_events (guild_id, user_id, created_at DESC)").run()
        try await sql.raw("CREATE INDEX IF NOT EXISTS idx_mem_rep ON user_memories (guild_id, reputation DESC)").run()
    }

    func revert(on database: Database) async throws {
        guard let sql = database as? SQLDatabase else { return }
        for name in ["idx_msg_channel", "idx_rep_user", "idx_mem_rep"] {
            try await sql.raw("DROP INDEX IF EXISTS \(raw: name)").run()
        }
    }
}
```

## Boot

```swift
// entrypoint.swift
import Vapor

@main
enum Entrypoint {
    static func main() async throws {
        var env = try Environment.detect()
        try LoggingSystem.bootstrap(from: &env)

        let app = try await Application.make(env)
        do {
            try await configure(app)
            try await app.execute()
        } catch {
            app.logger.report(error: error)
            try? await app.asyncShutdown()
            throw error
        }
        try await app.asyncShutdown()
    }
}
```

```swift
// configure.swift
import Vapor
import Fluent
import FluentPostgresDriver
import DiscordBM

/// Typed storage key so the gateway manager is reachable from routes.
struct BotStorage: StorageKey {
    typealias Value = BotGatewayManager
}

func configure(_ app: Application) async throws {
    // --- Database ---
    guard let dbURL = Environment.get("DATABASE_URL") else {
        throw Abort(.internalServerError, reason: "DATABASE_URL not set")
    }
    try app.databases.use(.postgres(url: dbURL), as: .psql)
    app.migrations.add(CreateAll())
    app.migrations.add(CreateIndexes())
    app.migrations.add(CreateGuildConfig())    // multi-guild.md
    app.migrations.add(CreateBrainState())     // adaptive-learning.md
    try await app.autoMigrate()

    // --- Discord gateway ---
    let bot = await BotGatewayManager(
        eventLoopGroup: app.eventLoopGroup,
        httpClient: app.http.client.shared,
        token: Environment.get("DISCORD_BOT_TOKEN")!,
        presence: .init(
            activities: [.init(name: "for questions", type: .listening)],
            status: .online,
            afk: false
        ),
        intents: [
            .guildMessages, .messageContent, .guildMembers, .guildPresences,
            .directMessages, .guildVoiceStates, .guildMessageReactions,
        ]
    )
    app.storage[BotStorage.self] = bot

    try await CommandRegistry(client: bot.client).registerAll()

    // Discord-only surface (interactions/components/modals) stays on the
    // GatewayEventHandler path; the cross-platform pipeline runs through SocialRouter.
    Task {
        for await event in await bot.events {
            await EventHandler(event: event, client: bot.client, db: app.db).handleAsync()
        }
    }
    Task { await bot.connect() }

    // --- Multi-social router (platforms.md) ---
    var adapters: [any SocialAdapter] = [DiscordAdapter(bot: bot, client: bot.client)]
    if let tgToken = Environment.get("TELEGRAM_BOT_TOKEN") {
        let tg = TelegramAdapter(token: tgToken, http: HTTPClient.shared)
        adapters.append(tg)
        await PlatformRegistry.shared.setTelegram(tg)
    }
    if let slackToken = Environment.get("SLACK_BOT_TOKEN"),
       let slackSecret = Environment.get("SLACK_SIGNING_SECRET") {
        let slack = SlackAdapter(botToken: slackToken, signingSecret: slackSecret,
                                 http: HTTPClient.shared)
        adapters.append(slack)
        await PlatformRegistry.shared.setSlackToken(slackToken)
        app.post("slack", "events") { try await slack.receive($0) }
    }
    let router = SocialRouter(adapters: adapters, db: app.db, vision: makeVision(app: app))
    await router.start()

    // --- Background heartbeat: learn / flush / persist (adaptive-learning.md) ---
    await AbbeyScheduler.shared.start(db: app.db)

    try routes(app)
}
```

## Web Dashboard

```swift
struct DashboardStats: Content {
    let messages: Int
    let users: Int
}

struct PersonaSwitchRequest: Content {
    let name: String
}

func routes(_ app: Application) throws {
    let protected = app.grouped(DashboardAuthMiddleware())

    app.get("**") { _ in app.fileio.collectFile(at: "Public/index.html") }

    let api = protected.grouped("api")
    api.get("stats") { req async throws -> DashboardStats in
        let msgCount = try await GuildMessage.query(on: req.db).count()
        let userCount = try await UserMemory.query(on: req.db).count()
        return DashboardStats(messages: msgCount, users: userCount)
    }
    api.get("users") { req async throws -> [UserMemory] in
        try await UserMemory.query(on: req.db).sort(\.$reputation, .descending).all()
    }
    api.get("messages", ":channelId") { req async throws -> [GuildMessage] in
        guard let cid = req.parameters.get("channelId") else { throw Abort(.badRequest) }
        return try await GuildMessage.query(on: req.db)
            .filter(\.$channelId == cid)
            .sort(\.$createdAt, .descending)
            .limit(100)
            .all()
    }
    api.post("persona") { req async throws -> HTTPStatus in
        let body = try req.content.decode(PersonaSwitchRequest.self)
        await ABIRouter.shared.setPersona(body.name)
        return .ok
    }
}

struct DashboardAuthMiddleware: AsyncMiddleware {
    func respond(to req: Request, chainingTo next: AsyncResponder) async throws -> Response {
        guard let token = req.headers.bearerAuthorization?.token,
              let secret = Environment.get("ABBEY_DASHBOARD_SECRET"),
              // Constant-time compare — `==` on String short-circuits and leaks
              // position of first mismatch to a timing attack.
              constantTimeEquals(token, secret) else {
            throw Abort(.unauthorized)
        }
        return try await next.respond(to: req)
    }
}

func constantTimeEquals(_ a: String, _ b: String) -> Bool {
    let x = Array(a.utf8), y = Array(b.utf8)
    guard x.count == y.count else { return false }
    var diff: UInt8 = 0
    for i in 0..<x.count { diff |= x[i] ^ y[i] }
    return diff == 0
}
```

The `app.get("**")` SPA catch-all is registered *before* the `api` group above. Route
matching is registration-ordered, so as written the wildcard can shadow `/api/*` — keep
the catch-all last, or scope it to exclude the API prefix.

## Deployment

### systemd Service (Linux)
```ini
# /etc/systemd/system/abbey-bot.service
[Unit]
Description=Abbey Discord Bot
After=network.target postgresql.service

[Service]
Type=exec
User=abbey
WorkingDirectory=/opt/abbey-bot
ExecStart=/opt/abbey-bot/.build/release/App serve --env production
Restart=on-failure
RestartSec=5
EnvironmentFile=/opt/abbey-bot/.env

[Install]
WantedBy=multi-user.target
```

### .env (never commit)
```
DISCORD_BOT_TOKEN=
DISCORD_APP_ID=
DISCORD_PUBLIC_KEY=
DATABASE_URL=postgresql://abbey:pass@localhost:5432/abbey
ABBEY_DASHBOARD_SECRET=
LOG_LEVEL=info
```

### Dockerfile
```dockerfile
# corrected: swift:5.10-jammy was the base image here before — that's Swift 5.10,
# not Swift 6, which contradicts every code sample in this skill using Swift 6
# language mode. Current official tags (docker-library/official-images, checked this
# pass): 6.3.2-jammy / 6.3-jammy (full) and 6.3.2-jammy-slim / 6.3-jammy-slim (slim,
# no compiler — runtime stage only).
FROM swift:6.3-jammy AS build
WORKDIR /app
COPY . .
RUN swift build -c release

FROM swift:6.3-jammy-slim
WORKDIR /app
COPY --from=build /app/.build/release/App .
COPY --from=build /app/Resources ./Resources
COPY --from=build /app/Public ./Public
CMD ["./App", "serve", "--env", "production", "--hostname", "0.0.0.0"]
```

### Build & Run
```bash
# Dev
swift run

# Release
swift build -c release
.build/release/App serve --env production

# DB migrations only
.build/release/App migrate

# Xcode
open Package.swift   # then Product → Run
```
