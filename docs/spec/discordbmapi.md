# DiscordBM — API Reference

Verified against the live `DiscordBM/DiscordBM` README (fetched during this skill's
last refresh). Current release: **v1.16.2**. Current Vapor: **4.121.4**. Where this
doc's pattern differs from what you remember writing before, trust this doc — several
calls here were corrected against source, noted inline with `# corrected:`.

## Package.swift

```swift
// swift-tools-version: 6.4
// Swift 6.4 toolchain, per canonical target. DiscordBM's own manifest is tools 6.1 —
// dependency manifests don't constrain the consumer upward, so this is fine.
import PackageDescription

let package = Package(
    name: "AbbeyBot",
    platforms: [.macOS(.v26)],   // canonical Apple Silicon host; deploy fallback is Linux (see bot-architecture.md)
    dependencies: [
        .package(url: "https://github.com/DiscordBM/DiscordBM", from: "1.16.0"),
        .package(url: "https://github.com/vapor/vapor", from: "4.115.0"),
        .package(url: "https://github.com/vapor/fluent", from: "4.9.0"),
        .package(url: "https://github.com/vapor/fluent-postgres-driver", from: "2.9.0"),
    ],
    targets: [
        // libopus for voice (voice.md). brew install opus / apt install libopus-dev.
        .systemLibrary(name: "COpus", pkgConfig: "opus",
                       providers: [.brew(["opus"]), .apt(["libopus-dev"])]),
        .executableTarget(
            name: "App",
            dependencies: [
                .product(name: "DiscordBM", package: "DiscordBM"),
                .product(name: "Vapor", package: "vapor"),
                .product(name: "Fluent", package: "fluent"),
                .product(name: "FluentPostgresDriver", package: "fluent-postgres-driver"),
                "COpus",
            ]
            // corrected: no -strict-concurrency=complete unsafeFlag. That was the
            // pre-Swift-6-GA idiom for opting into strict checking under a 5.x language
            // mode. With swift-tools-version 6.x, targets default to Swift 6 language
            // mode already — the flag is redundant and DiscordBM v1.16.x already
            // requires a 6.0+ toolchain (confirmed: swift-tools-6.0 minimum on
            // Swift Package Index).
        )
    ]
)
```

## Gateway Manager — Init and Event Consumption

```swift
// corrected: DiscordBM's own initializer is `await BotGatewayManager(token:presence:intents:)`
// (or the eventLoopGroup/httpClient overload when wiring into an existing Vapor app).
// It does NOT take an `identifyPayload:` wrapper — token and intents are top-level,
// labeled params, and the initializer itself is `async`.
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
        .guildMessages,
        .messageContent,       // Privileged
        .guildMembers,         // Privileged
        .guildPresences,       // Privileged
        .directMessages,
        .guildVoiceStates,
        .guildMessageReactions,
    ]
)
app.storage[BotStorage.self] = bot   // `BotStorage: StorageKey` — defined in bot-architecture.md § Boot

// corrected: consume via `bot.events` (a property, async sequence), not a
// `makeEventsStream()` method — and DiscordBM's own recommended pattern is a type
// conforming to `GatewayEventHandler`, which gives you one function per event type
// (`onMessageCreate`, `onInteractionCreate`, etc.) instead of a manual switch.
struct EventHandler: GatewayEventHandler {
    let event: Gateway.Event
    let client: any DiscordClient
    let db: Database

    func onMessageCreate(_ payload: Gateway.MessageCreate) async throws {
        await MessageHandler(client: client, db: db).handle(payload)
    }

    func onInteractionCreate(_ interaction: Interaction) async throws {
        await InteractionHandler(client: client, db: db).handle(interaction)
    }

    func onVoiceStateUpdate(_ vs: Gateway.VoiceState) async throws {
        await VoiceStateHandler(client: client, db: db).handle(vs)
    }

    func onGuildMemberAdd(_ member: Gateway.GuildMemberAdd) async throws {
        await MemberHandler(client: client, db: db).handleJoin(member)
    }
}

Task {
    // The event stream never ends — this Task keeps running for the process lifetime.
    for await event in await bot.events {
        await EventHandler(event: event, client: bot.client, db: app.db).handleAsync()
    }
}
try await bot.connect()
```

## MessageHandler

```swift
actor MessageHandler {
    let client: any DiscordClient
    let db: Database

    // corrected: actors get no synthesized memberwise init (unlike structs), so the
    // `MessageHandler(client:db:)` call sites elsewhere in this doc could not compile
    // without this. Same fix applies to InteractionHandler and CommandRegistry below.
    init(client: any DiscordClient, db: Database) {
        self.client = client
        self.db = db
    }

    func handle(_ message: Gateway.MessageCreate) async {
        guard message.author?.bot != true,
              let content = message.content, !content.isEmpty else { return }

        let record = GuildMessage(
            discordMessageId: message.id.rawValue,
            channelId: message.channel_id.rawValue,
            guildId: message.guild_id?.rawValue ?? "DM",
            authorId: message.author?.id.rawValue ?? "",
            content: content
        )
        try? await record.save(on: db)

        let intent = IntentClassifier.classify(content)
        // corrected: ABIRouter is now a real actor with instance state, not an actor
        // full of `static var` (which was nonisolated global mutable state and a hard
        // error in Swift 6 language mode). Guild-scoped defaults live in the
        // multi-guild version — see multi-guild.md.
        let scopedGuild = "discord:\(message.guild_id?.rawValue ?? "dm")"
        let persona = await ABIRouter.shared.route(intent: intent, scopedGuildId: scopedGuild)
        let context = await buildContext(message: message)
        let response = await persona.respond(to: content, context: context, db: db)

        await SocialBrain.shared.recordInteraction(
            userId: message.author?.id.rawValue ?? "",
            guildId: message.guild_id?.rawValue ?? "",
            quality: intent.quality,
            db: db
        )

        try? await client.createMessage(
            channelId: message.channel_id,
            payload: .init(
                content: response.text,
                embeds: response.embed.map { [$0] },
                message_reference: .init(message_id: message.id)
            )
        )
    }

    private func buildContext(message: Gateway.MessageCreate) async -> PersonaContext {
        let channelCtx = try? await ChannelContext.query(on: db)
            .filter(\.$channelId == message.channel_id.rawValue)
            .first()
        let userMem = try? await UserMemory.query(on: db)
            .filter(\.$discordUserId == (message.author?.id.rawValue ?? ""))
            .first()
        return PersonaContext(
            channelSummary: channelCtx?.summary ?? "",
            userFacts: userMem?.facts ?? [],
            reputation: userMem?.reputation ?? 0.5
        )
    }
}
```

## Slash Commands

```swift
actor CommandRegistry {
    let client: any DiscordClient

    init(client: any DiscordClient) { self.client = client }

    func registerAll() async throws {
        // corrected: bulk-register in one call — matches DiscordBM's documented
        // pattern (`bulkSetApplicationCommands`), no appId param needed, and it's a
        // full overwrite so stale commands from prior deploys get cleaned up too.
        // Use guild-scoped registration during dev (instant); switch to this
        // (global) for prod once the command set is stable.
        try await client
            .bulkSetApplicationCommands(payload: AbbeyCommands.all)
            .guardSuccess()
    }
}

enum AbbeyCommands {
    // corrected: the payload type is `Payloads.ApplicationCommandCreate`, not
    // `Payloads.CreateGlobalApplicationCommand` (that name doesn't exist in the
    // library — confirmed against the README's own registration example).
    static let all: [Payloads.ApplicationCommandCreate] = [
        .init(name: "ask",      description: "Ask Abbey anything",        options: [questionOption]),
        .init(name: "rep",      description: "Check a user's reputation", options: [userOption]),
        .init(name: "remember", description: "Store a fact about a user", options: [userOption, factOption]),
        .init(name: "forget",   description: "Remove a stored fact",      options: [userOption, factIndexOption]),
        .init(name: "context",  description: "Show channel memory summary"),
        .init(name: "persona",  description: "Switch active persona",     options: [personaOption]),
    ]

    static let questionOption = ApplicationCommand.Option(
        type: .string, name: "question", description: "Your question", required: true,
        autocomplete: true
    )
    static let userOption = ApplicationCommand.Option(
        type: .user, name: "user", description: "Target user", required: true
    )
    static let personaOption = ApplicationCommand.Option(
        type: .string, name: "name", description: "abbey | aviva | abi", required: true,
        choices: [
            .init(name: "Abbey", value: .string("abbey")),
            .init(name: "Aviva", value: .string("aviva")),
            .init(name: "Abi",   value: .string("abi")),
        ]
    )
}
```

### InteractionHandler — Slash + Button + Select + Modal

```swift
actor InteractionHandler {
    let client: any DiscordClient
    let db: Database

    init(client: any DiscordClient, db: Database) {
        self.client = client
        self.db = db
    }

    func handle(_ interaction: Interaction) async {
        switch interaction.data {
        case .applicationCommand(let data):
            await handleCommand(interaction, data: data)
        case .messageComponent(let data):
            await handleComponent(interaction, data: data)
        case .modalSubmit(let data):
            await handleModal(interaction, data: data)
        case .applicationCommandAutocomplete(let data):
            await handleAutocomplete(interaction, data: data)
        default: break
        }
    }

    // --- Slash Command ---
    private func handleCommand(_ i: Interaction, data: Interaction.ApplicationCommand) async {
        // corrected: param labels are `id:`/`token:`, not `interactionId:`/`interactionToken:`.
        try? await client.createInteractionResponse(
            id: i.id, token: i.token,
            payload: .deferredChannelMessageWithSource()
        ).guardSuccess()

        // corrected: `.option(named:)` is a documented convenience helper on the
        // options array — prefer it over manually filtering `.first(where:)`.
        let result: String
        switch data.name {
        case "ask":
            let question = data.option(named: "question")?.value?.asString ?? ""
            let scoped = i.guild_id.map { "discord:\($0.rawValue)" }
            let persona = await ABIRouter.shared.activePersona(scopedGuildId: scoped)
            result = await persona.respond(to: question, context: .empty, db: db).text
        case "rep":
            let userId = data.option(named: "user")?.value?.asString ?? ""
            let mem = try? await UserMemory.query(on: db).filter(\.$discordUserId == userId).first()
            result = "Reputation: \(String(format: "%.2f", mem?.reputation ?? 0.5))"
        default:
            result = "Unknown command."
        }

        // corrected: DiscordBM's documented pattern for finishing a deferred
        // response is `updateOriginalInteractionResponse(token:payload:)`, not a
        // separate "createFollowupMessage(appId:...)" call — use follow-ups only
        // when you need to send *additional* messages beyond the original response.
        try? await client.updateOriginalInteractionResponse(
            token: i.token,
            payload: .init(content: result)
        ).guardSuccess()
    }

    // --- Button / Select ---
    private func handleComponent(_ i: Interaction, data: Interaction.MessageComponent) async {
        switch data.custom_id {
        case let id where id.hasPrefix("confirm_"):
            let action = String(id.dropFirst("confirm_".count))
            await executeConfirmedAction(action, interaction: i)
        case "persona_select":
            let selected = data.values?.first ?? "abbey"
            let scoped = i.guild_id.map { "discord:\($0.rawValue)" }
            let applied = await ABIRouter.shared.setPersona(selected, scopedGuildId: scoped)
            try? await client.updateMessage(
                id: i.id, token: i.token,
                payload: .init(content: "Switched to **\(applied)**")
            ).guardSuccess()
        default: break
        }
    }

    /// Terminal step of the confirm-button flow. Re-checks permission at execution
    /// time — the button may have been clicked by someone other than the invoker, and
    /// a component interaction carries the *clicker's* member, not the original one.
    private func executeConfirmedAction(_ action: String, interaction i: Interaction) async {
        guard let guildId = i.guild_id,
              let member = i.member,
              let actorId = member.user?.id else { return }

        let parts = action.split(separator: ":").map(String.init)   // e.g. "ban:12345"
        guard parts.count == 2, let verb = parts.first, let targetId = parts.last else { return }

        let permitted = member.permissions?.contains(verb == "ban" ? .banMembers : .moderateMembers) ?? false
        guard permitted else {
            try? await client.createInteractionResponse(
                id: i.id, token: i.token,
                payload: .channelMessageWithSource(.init(content: "You don't have permission for that.",
                                                         flags: [.ephemeral]))
            ).guardSuccess()
            return
        }

        let outcome: String
        switch verb {
        case "ban":
            let ok = (try? await client.createGuildBan(
                guildId: guildId, userId: .init(targetId),
                payload: .init(delete_message_seconds: 0)
            ).guardSuccess()) != nil
            outcome = ok ? "Banned <@\(targetId)>." : "Ban failed."
            if ok {
                await SocialBrain.shared.penalize(userId: targetId, guildId: guildId.rawValue,
                                                  reason: "ban by \(actorId.rawValue)", db: db)
            }
        case "timeout":
            let until = Date().addingTimeInterval(300)
            let ok = (try? await client.modifyGuildMember(
                guildId: guildId, userId: .init(targetId),
                payload: .init(communication_disabled_until: .init(until))
            ).guardSuccess()) != nil
            outcome = ok ? "Timed out <@\(targetId)> for 5m." : "Timeout failed."
        default:
            outcome = "Unknown action."
        }

        // Replace the confirm prompt so the buttons can't be re-clicked.
        try? await client.updateMessage(
            id: i.id, token: i.token,
            payload: .init(content: outcome, components: [])
        ).guardSuccess()
    }

    // --- Modal Submit ---
    private func handleModal(_ i: Interaction, data: Interaction.ModalSubmit) async {
        guard data.custom_id == "remember_modal" else { return }
        let fact = data.components.first?.components?.first?.value ?? ""
        let userId = i.member?.user?.id.rawValue ?? ""
        if let mem = try? await UserMemory.query(on: db).filter(\.$discordUserId == userId).first() {
            mem.facts.append(fact)
            try? await mem.save(on: db)
        }
        try? await client.createInteractionResponse(
            id: i.id, token: i.token,
            payload: .channelMessageWithSource(.init(content: "Remembered ✅", flags: [.ephemeral]))
        ).guardSuccess()
    }

    // --- Autocomplete ---
    private func handleAutocomplete(_ i: Interaction, data: Interaction.ApplicationCommand) async {
        let partial = data.options?.first(where: { $0.name == "question" && $0.focused == true })?.value?.asString ?? ""
        let suggestions = IntentClassifier.suggestCompletions(for: partial)
        let choices = suggestions.prefix(25).map { ApplicationCommand.Option.Choice(name: $0, value: .string($0)) }
        try? await client.createInteractionResponse(
            id: i.id, token: i.token,
            payload: .autocompleteResult(.init(choices: Array(choices)))
        ).guardSuccess()
    }
}
```

### Component Builders

```swift
func confirmButton(action: String) -> Payloads.CreateMessage {
    .init(
        content: "Are you sure?",
        components: [[
            .button(.init(custom_id: "confirm_\(action)", label: "Confirm", style: .danger)),
            .button(.init(custom_id: "cancel", label: "Cancel", style: .secondary)),
        ]]
    )
}

func personaSelector() -> Payloads.CreateMessage {
    .init(
        content: "Select a persona:",
        components: [[
            .stringSelect(.init(
                custom_id: "persona_select",
                options: [
                    .init(label: "Abbey", value: "abbey", description: "Direct & street-smart", emoji: .init(name: "🖤")),
                    .init(label: "Aviva", value: "aviva", description: "Analytical & structured", emoji: .init(name: "🔷")),
                    .init(label: "Abi",   value: "abi",   description: "Warm & rapport-first",   emoji: .init(name: "🌸")),
                ]
            ))
        ]]
    )
}

func rememberModal(userId: String) -> Payloads.InteractionResponse {
    .modal(.init(
        custom_id: "remember_modal",
        title: "Store a Fact",
        components: [[
            .textInput(.init(
                custom_id: "fact_input",
                label: "Fact to remember",
                style: .paragraph,
                placeholder: "This user always...",
                required: true,
                max_length: 500
            ))
        ]]
    ))
}
```

## Incoming Interactions Webhook (Signature Verification)

For HTTP-mode interactions (no persistent Gateway connection):

```swift
app.post("interactions") { req async throws -> Response in
    let sig = req.headers["X-Signature-Ed25519"].first ?? ""
    let ts  = req.headers["X-Signature-Timestamp"].first ?? ""
    let body = req.body.data.map { Data($0.readableBytesView) } ?? Data()
    guard DiscordSignatureVerifier.verify(
        publicKey: Environment.get("DISCORD_PUBLIC_KEY")!,
        signature: sig, timestamp: ts, body: body
    ) else { throw Abort(.unauthorized) }

    let interaction = try req.content.decode(Interaction.self)

    // Discord PINGs this endpoint to validate it before it will accept the URL —
    // and re-PINGs periodically. Must reply PONG or the endpoint gets disabled.
    if interaction.type == .ping {
        return try await Interaction.Response.pong.encodeResponse(for: req)
    }

    // Dispatch to InteractionHandler...
    return Response(status: .ok)
}
```

`DiscordSignatureVerifier` was referenced above but never defined. Ed25519 over
`timestamp + rawBody`, via swift-crypto (already in the tree transitively through Vapor):

```swift
import Crypto
import Foundation

enum DiscordSignatureVerifier {
    /// Verifies Discord's Ed25519 interaction signature.
    /// The signed payload is the timestamp header concatenated with the *raw* body —
    /// re-encoding the decoded JSON changes the bytes and fails verification, so this
    /// must run before any decode.
    static func verify(publicKey: String, signature: String, timestamp: String, body: Data) -> Bool {
        guard let keyBytes = Data(hexString: publicKey),
              let sigBytes = Data(hexString: signature),
              let key = try? Curve25519.Signing.PublicKey(rawRepresentation: keyBytes)
        else { return false }

        // Reject stale timestamps — signatures are otherwise replayable forever.
        if let sent = Double(timestamp), abs(Date().timeIntervalSince1970 - sent) > 300 {
            return false
        }

        var signed = Data(timestamp.utf8)
        signed.append(body)
        return key.isValidSignature(sigBytes, for: signed)
    }
}

extension Data {
    /// Discord sends the public key and signature as lowercase hex, not base64.
    init?(hexString: String) {
        let chars = Array(hexString.utf8)
        guard chars.count % 2 == 0 else { return nil }
        var bytes = [UInt8]()
        bytes.reserveCapacity(chars.count / 2)
        for i in stride(from: 0, to: chars.count, by: 2) {
            guard let hi = Self.nibble(chars[i]), let lo = Self.nibble(chars[i + 1]) else { return nil }
            bytes.append(hi << 4 | lo)
        }
        self.init(bytes)
    }

    private static func nibble(_ c: UInt8) -> UInt8? {
        switch c {
        case 0x30...0x39: return c - 0x30          // 0-9
        case 0x61...0x66: return c - 0x61 + 10     // a-f
        case 0x41...0x46: return c - 0x41 + 10     // A-F
        default: return nil
        }
    }
}
```

Getting the raw body requires disabling Vapor's streaming collation for this route, or
reading `req.body.data` before any `req.content.decode` — decoding first and
re-serializing will not round-trip byte-identically.

## Common Operations — Quick Paste

```swift
// Send embed
try await client.createMessage(channelId: channelId, payload: .init(embeds: [
    .init(
        title: "Abbey",
        description: responseText,
        color: .init(value: 0x7c3aed),
        footer: .init(text: "ABI v2 · \(persona)"),
        timestamp: Date()
    )
]))

// Ephemeral reply (only sender sees it)
try? await client.createInteractionResponse(
    id: i.id, token: i.token,
    payload: .channelMessageWithSource(.init(content: "Only you can see this", flags: [.ephemeral]))
)

// Add / remove reaction
try await client.addReaction(channelId: cid, messageId: mid, emoji: .unicodeEmoji("✅"))
try await client.deleteOwnReaction(channelId: cid, messageId: mid, emoji: .unicodeEmoji("⏳"))

// Timeout member (5 min)
try await client.modifyGuildMember(
    guildId: gid, userId: uid,
    payload: .init(communication_disabled_until: .init(Date().addingTimeInterval(300)))
)

// Bulk delete messages (≤14 days old, ≤100 at once)
try await client.bulkDeleteMessages(channelId: cid, payload: .init(messages: messageIds))

// Move member to a different VC
try await client.modifyGuildMember(guildId: gid, userId: uid, payload: .init(channel_id: targetVoiceChannelId))

// Create invite
let invite = try await client.createChannelInvite(
    channelId: cid, payload: .init(max_age: 86400, max_uses: 1, unique: true)
).decode()
print("https://discord.gg/\(invite.code)")

// Fetch audit log
let log = try await client.getGuildAuditLog(guildId: gid, params: .init(limit: 50, action_type: .memberBan)).decode()

// Pin / thread
try await client.pinMessage(channelId: cid, messageId: mid)
try await client.createThreadFromMessage(
    channelId: cid, messageId: mid, payload: .init(name: "Discussion", auto_archive_duration: .oneDay)
)

// Mention / timestamp helpers (DiscordUtils — real, documented, worth using instead
// of hand-rolling `<@id>` / `<t:...>` formatting strings)
let userMention = DiscordUtils.mention(id: someUserId)          // "<@id>" → renders as @Name
let roleMention = DiscordUtils.mention(id: someRoleId)
let ts = DiscordUtils.timestamp(date: Date())                    // localized per-viewer timestamp
let relTs = DiscordUtils.timestamp(unixTimestamp: someEpoch, style: .relativeTime)
let safe = DiscordUtils.escapingSpecialCharacters(userSuppliedText)  // strip markdown injection
```

## Permission Checking

Two layers — raw bitfields for reference, `DiscordCache` for actually checking:

```swift
// Preferred: DiscordCache gives best-effort permission/role checks without you
// hand-rolling bitfield math. Requires a DiscordCache with `.guilds` + `.guildMembers`
// intents and `requestAllMembers: .enabled`.
let cache = await DiscordCache(
    gatewayManager: bot,
    intents: [.guilds, .guildMembers],
    requestAllMembers: .enabled,
    messageCachingPolicy: .saveEditHistoryAndDeleted
)

guard let guild = await cache.guilds[guildId] else { return }

let canViewChannel = guild.userHasPermissions(
    userId: userId, channelId: channelId, permissions: [.viewChannel, .readMessageHistory]
)
let canBan = guild.userHasGuildPermission(userId: userId, permission: .banMembers)
let hasModRole = guild.userHasRole(userId: userId, roleId: modRoleId)

// In an interaction, the member's resolved permissions are already attached —
// no cache lookup needed: `interaction.member?.permissions`
```

```
// Reference — common permission bit values, if you need to reason about raw bits:
// .administrator        = 0x8
// .manageMessages       = 0x2000
// .moderateMembers      = 0x10000000  (timeout)
// .kickMembers          = 0x2
// .banMembers           = 0x4
// .manageChannels       = 0x10
// .viewAuditLog         = 0x80
// .manageRoles          = 0x10000000
// Evaluation order: @everyone → role (highest) → member override.
// Allow bits = granted; deny bits = blocked.
```

## Rate Limits & Error Handling

```swift
// DiscordBM's HTTPRateLimiter tracks x-ratelimit headers and, combined with
// ClientConfiguration's RetryPolicy, auto-retries 429s within the configured window.
// This is on by default — you generally don't need to hand-roll retry logic.

// Guard success on non-throwing-by-default endpoints
try await client.deleteMessage(channelId: cid, messageId: mid).guardSuccess()

// Decode with error context
let guild = try await client.getGuild(guildId: gid).decode()

// If you do want manual retry control on top of the built-in behavior:
func withRetry<T>(attempts: Int = 3, _ block: () async throws -> T) async throws -> T {
    var last: Error?
    for attempt in 0..<attempts {
        do { return try await block() }
        catch {
            last = error
            try await Task.sleep(for: .seconds(Double(attempt + 1) * 2))
        }
    }
    throw last!
}
```

## Gateway Lifecycle

- DiscordBM handles HELLO → IDENTIFY → HEARTBEAT automatically
- On disconnect: auto-reconnect with RESUME if the session ID is still valid
- `bot.connect()` is safe to call from a `Task`
- `for await event in await bot.events` never terminates — that's what keeps the process alive
- Large guilds (>2500 members): needs `requestAllMembers` (via `DiscordCache`, or the
  raw gateway `requestGuildMembers` op) to lazy-load the full member list
- Discord requires sharding at 2500+ guilds — swap `BotGatewayManager` for
  `ShardingGatewayManager` (same call shape, DiscordBM manages shard count for you)
