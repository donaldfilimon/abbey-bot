# Multi-Guild — Isolation, Config, Registries, Sharding

One Abbey process, many communities, zero bleed-through. The isolation invariant:
**everything learned, remembered, or configured is keyed by `scopedGuildId`**
(`"{platform}:{nativeGuildId}"` — see `platforms.md`). Reputation earned in one guild,
facts stored in another, and a DQN policy trained in a third never interact.

What was already per-guild: `UserMemory` (unique on `discord_user_id, guild_id`),
`ReputationEvent`, `SocialBrain` keys. What this file adds: per-guild **config**,
per-guild **brains**, per-guild **rate limiting**, and the sharding story.

## GuildConfig — Fluent model

```swift
final class GuildConfig: Model, Content, @unchecked Sendable {
    static let schema = "guild_configs"
    @ID(key: .id) var id: UUID?
    @Field(key: "scoped_guild_id")  var scopedGuildId: String
    @Field(key: "enabled")          var enabled: Bool
    @Field(key: "default_persona")  var defaultPersona: String      // "abbey" | "aviva" | "abi"
    @Field(key: "learning_enabled") var learningEnabled: Bool       // DQN on/off per guild
    @Field(key: "voice_enabled")    var voiceEnabled: Bool
    @Field(key: "vision_enabled")   var visionEnabled: Bool
    @Field(key: "reply_cooldown_s") var replyCooldownSeconds: Int   // min gap between unsolicited replies
    @Field(key: "epsilon_override") var epsilonOverride: Double?    // nil = global schedule
    @Field(key: "locale")           var locale: String              // reply language hint
    @Timestamp(key: "updated_at", on: .update) var updatedAt: Date?

    init() {}
    init(id: UUID? = nil, scopedGuildId: String, enabled: Bool = true,
         defaultPersona: String = "abbey", learningEnabled: Bool = true,
         voiceEnabled: Bool = true, visionEnabled: Bool = true,
         replyCooldownSeconds: Int = 20, epsilonOverride: Double? = nil,
         locale: String = "en") {
        self.id = id
        self.scopedGuildId = scopedGuildId
        self.enabled = enabled
        self.defaultPersona = defaultPersona
        self.learningEnabled = learningEnabled
        self.voiceEnabled = voiceEnabled
        self.visionEnabled = visionEnabled
        self.replyCooldownSeconds = replyCooldownSeconds
        self.epsilonOverride = epsilonOverride
        self.locale = locale
    }
}

/// Snapshot value handed to hot paths — never hand out the Fluent object itself.
struct GuildSettings: Sendable {
    var enabled: Bool
    var defaultPersona: String
    var learningEnabled: Bool
    var voiceEnabled: Bool
    var visionEnabled: Bool
    var replyCooldownSeconds: Int
    var epsilonOverride: Double?
    var locale: String

    static let defaults = GuildSettings(
        enabled: true, defaultPersona: "abbey", learningEnabled: true,
        voiceEnabled: true, visionEnabled: true, replyCooldownSeconds: 20,
        epsilonOverride: nil, locale: "en")

    init(enabled: Bool, defaultPersona: String, learningEnabled: Bool,
         voiceEnabled: Bool, visionEnabled: Bool, replyCooldownSeconds: Int,
         epsilonOverride: Double?, locale: String) {
        self.enabled = enabled; self.defaultPersona = defaultPersona
        self.learningEnabled = learningEnabled; self.voiceEnabled = voiceEnabled
        self.visionEnabled = visionEnabled; self.replyCooldownSeconds = replyCooldownSeconds
        self.epsilonOverride = epsilonOverride; self.locale = locale
    }

    init(_ model: GuildConfig) {
        self.init(enabled: model.enabled, defaultPersona: model.defaultPersona,
                  learningEnabled: model.learningEnabled, voiceEnabled: model.voiceEnabled,
                  visionEnabled: model.visionEnabled,
                  replyCooldownSeconds: model.replyCooldownSeconds,
                  epsilonOverride: model.epsilonOverride, locale: model.locale)
    }
}
```

Migration (append to the migration list in `configure.swift`):

```swift
struct CreateGuildConfig: AsyncMigration {
    func prepare(on database: Database) async throws {
        try await database.schema(GuildConfig.schema)
            .id()
            .field("scoped_guild_id",  .string, .required)
            .field("enabled",          .bool,   .required)
            .field("default_persona",  .string, .required)
            .field("learning_enabled", .bool,   .required)
            .field("voice_enabled",    .bool,   .required)
            .field("vision_enabled",   .bool,   .required)
            .field("reply_cooldown_s", .int,    .required)
            .field("epsilon_override", .double)
            .field("locale",           .string, .required)
            .field("updated_at",       .datetime)
            .unique(on: "scoped_guild_id")
            .create()
    }
    func revert(on database: Database) async throws {
        try await database.schema(GuildConfig.schema).delete()
    }
}
```

## GuildRegistry — config cache

Read on every inbound event, so it must not hit Postgres per message. Write-through
cache with lazy hydrate; auto-provisions defaults the first time a guild is seen.

```swift
actor GuildRegistry {
    static let shared = GuildRegistry()

    private var cache: [String: GuildSettings] = [:]

    func config(for scopedGuildId: String, db: Database) async -> GuildSettings {
        if let cached = cache[scopedGuildId] { return cached }

        if let row = try? await GuildConfig.query(on: db)
            .filter(\.$scopedGuildId == scopedGuildId).first() {
            let settings = GuildSettings(row)
            cache[scopedGuildId] = settings
            return settings
        }

        // First contact with this guild — provision defaults.
        let row = GuildConfig(scopedGuildId: scopedGuildId)
        try? await row.save(on: db)
        cache[scopedGuildId] = .defaults
        return .defaults
    }

    func update(scopedGuildId: String, db: Database,
                _ mutate: @Sendable (inout GuildSettings) -> Void) async throws {
        var settings = await config(for: scopedGuildId, db: db)
        mutate(&settings)
        cache[scopedGuildId] = settings

        guard let row = try await GuildConfig.query(on: db)
            .filter(\.$scopedGuildId == scopedGuildId).first() else { return }
        row.enabled = settings.enabled
        row.defaultPersona = settings.defaultPersona
        row.learningEnabled = settings.learningEnabled
        row.voiceEnabled = settings.voiceEnabled
        row.visionEnabled = settings.visionEnabled
        row.replyCooldownSeconds = settings.replyCooldownSeconds
        row.epsilonOverride = settings.epsilonOverride
        row.locale = settings.locale
        try await row.save(on: db)
    }

    func evict(scopedGuildId: String) { cache[scopedGuildId] = nil }
}
```

## Per-guild persona default

`ABIRouter` (bot-architecture.md) held a single global `current` persona — wrong for
multi-guild: `/persona aviva` in one server must not flip every server. Guild-scoped
override with global fallback; call sites pass the scoped guild id.

```swift
actor ABIRouter {
    static let shared = ABIRouter()

    private var globalDefault: any Persona = AbbeyPersona()
    private var perGuild: [String: any Persona] = [:]

    func route(intent: IntentClassifier.Intent, scopedGuildId: String? = nil) -> any Persona {
        switch intent {
        case .modRequest, .command: return AvivaPersona()
        case .greeting, .smallTalk: return AbiPersona()
        default:
            if let g = scopedGuildId, let p = perGuild[g] { return p }
            return globalDefault
        }
    }

    func activePersona(scopedGuildId: String? = nil) -> any Persona {
        if let g = scopedGuildId, let p = perGuild[g] { return p }
        return globalDefault
    }

    @discardableResult
    func setPersona(_ name: String, scopedGuildId: String? = nil) -> String {
        let persona: any Persona
        switch name.lowercased() {
        case "aviva": persona = AvivaPersona()
        case "abi":   persona = AbiPersona()
        default:      persona = AbbeyPersona()
        }
        if let g = scopedGuildId { perGuild[g] = persona } else { globalDefault = persona }
        return persona.name
    }
}
```

This supersedes the `ABIRouter` in `bot-architecture.md` — same actor shape, guild
dimension added. The dashboard's `POST /api/persona` should now take an optional
`scopedGuildId` in `PersonaSwitchRequest` and pass it through.

## Per-guild reply cooldown

Guards against Abbey dominating an active channel. Checked in `SocialRouter` before
the brain is even consulted for unsolicited replies (direct mentions and slash
commands bypass it).

```swift
actor ReplyCooldown {
    static let shared = ReplyCooldown()
    private var lastReplyAt: [String: Date] = [:]     // scopedChannelId → last unsolicited reply

    func permitted(scopedChannelId: String, cooldownSeconds: Int) -> Bool {
        guard let last = lastReplyAt[scopedChannelId] else { return true }
        return Date().timeIntervalSince(last) >= Double(cooldownSeconds)
    }

    func recordReply(scopedChannelId: String) {
        lastReplyAt[scopedChannelId] = Date()
    }
}
```

## BrainRegistry — one DQN per guild

Defined in `adaptive-learning.md` (it owns the learning loop); referenced here for
the isolation contract: **policies are per-guild**. A meme-dense gaming server trains
a chattier policy than a professional one, and neither pollutes the other. Snapshots
persist per guild in the `brain_states` table; eviction after inactivity keeps memory
bounded when guild count grows.

## Guild lifecycle events

```swift
// EventHandler additions (discordbm-api.md)
func onGuildCreate(_ guild: Gateway.GuildCreate) async throws {
    // Fires on connect for every existing guild AND on new-guild join.
    _ = await GuildRegistry.shared.config(for: "discord:\(guild.id.rawValue)", db: db)
}

func onGuildDelete(_ payload: Gateway.GuildDelete) async throws {
    let scoped = "discord:\(payload.id.rawValue)"
    await GuildRegistry.shared.evict(scopedGuildId: scoped)
    await BrainRegistry.shared.persistAndEvict(scopedGuildId: scoped, db: db)
    await VoiceSessionManager.shared.leave(scopedGuildId: scoped)
    // Rows stay in Postgres — kicked-and-reinvited guilds resume where they left off.
    // Hard data deletion is an explicit admin action, never automatic.
}
```

## /admin command — per-guild config surface

Requires `manageGuild` permission (checked via `interaction.member?.permissions`).

```swift
.init(name: "admin", description: "Configure Abbey for this server", options: [
    ApplicationCommand.Option(type: .subCommand, name: "show",
        description: "Show current settings"),
    ApplicationCommand.Option(type: .subCommand, name: "persona",
        description: "Set default persona", options: [personaOption]),
    ApplicationCommand.Option(type: .subCommand, name: "learning",
        description: "Toggle adaptive learning", options: [onOffOption]),
    ApplicationCommand.Option(type: .subCommand, name: "voice",
        description: "Toggle voice features", options: [onOffOption]),
    ApplicationCommand.Option(type: .subCommand, name: "vision",
        description: "Toggle image understanding", options: [onOffOption]),
    ApplicationCommand.Option(type: .subCommand, name: "cooldown",
        description: "Unsolicited reply cooldown seconds", options: [
            ApplicationCommand.Option(type: .integer, name: "seconds",
                description: "0–600", required: true)
        ]),
]),

static let onOffOption = ApplicationCommand.Option(
    type: .string, name: "state", description: "on | off", required: true,
    choices: [.init(name: "on", value: .string("on")),
              .init(name: "off", value: .string("off"))])
```

Handler shape (in `InteractionHandler.handleCommand`, `case "admin"`):

```swift
case "admin":
    guard i.member?.permissions?.contains(.manageGuild) == true else {
        result = "Requires **Manage Server**."
        break
    }
    let scoped = "discord:\(i.guild_id?.rawValue ?? "")"
    guard let sub = data.options?.first else { result = "Missing subcommand."; break }
    switch sub.name {
    case "show":
        let s = await GuildRegistry.shared.config(for: scoped, db: db)
        result = """
        **Abbey — \(scoped)**
        persona: \(s.defaultPersona) · learning: \(s.learningEnabled ? "on" : "off") \
        · voice: \(s.voiceEnabled ? "on" : "off") · vision: \(s.visionEnabled ? "on" : "off") \
        · cooldown: \(s.replyCooldownSeconds)s
        """
    case "persona":
        let name = sub.options?.first?.value?.asString ?? "abbey"
        try? await GuildRegistry.shared.update(scopedGuildId: scoped, db: db) { $0.defaultPersona = name }
        _ = await ABIRouter.shared.setPersona(name, scopedGuildId: scoped)
        result = "Default persona for this server: **\(name)**"
    case "learning", "voice", "vision":
        let on = sub.options?.first?.value?.asString == "on"
        try? await GuildRegistry.shared.update(scopedGuildId: scoped, db: db) {
            switch sub.name {
            case "learning": $0.learningEnabled = on
            case "voice":    $0.voiceEnabled = on
            default:         $0.visionEnabled = on
            }
        }
        result = "\(sub.name) is now **\(on ? "on" : "off")** for this server."
    case "cooldown":
        let secs = min(600, max(0, sub.options?.first?.value?.asInt ?? 20))
        try? await GuildRegistry.shared.update(scopedGuildId: scoped, db: db) { $0.replyCooldownSeconds = secs }
        result = "Reply cooldown: **\(secs)s**"
    default:
        result = "Unknown subcommand."
    }
```

## Sharding

Discord mandates sharding at 2,500 guilds. DiscordBM's `ShardingGatewayManager` is a
drop-in for `BotGatewayManager` (same init shape; the library manages shard count).
Everything in this architecture is already shard-safe because no state lives on the
gateway connection:

- Registries (`GuildRegistry`, `BrainRegistry`, `SocialBrain`) key by scopedGuildId —
  a guild only ever lives on one shard, so no cross-shard contention exists.
- Postgres is the single source of truth; shards share one pool.
- Voice sessions attach to the guild, not the shard.

**Multi-process sharding** (shards split across machines) is the one thing this design
does *not* cover: the in-process actor caches assume one process. Crossing that line
means moving `SocialBrain`/`GuildRegistry` caches to a shared store — flag it when
guild count approaches four digits, don't pre-build it.
