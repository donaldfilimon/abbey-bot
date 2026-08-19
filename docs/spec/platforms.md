# Platform Layer — Multi-Social Abstraction

Abbey speaks more than Discord. This layer normalizes every network into one event
model so the Brain, personas, and memory never know which platform a message came from.
Discord remains first-class (full feature surface via DiscordBM); other platforms map
onto the common subset.

## Design

```
Discord ──DiscordAdapter──┐
Telegram ─TelegramAdapter─┼──▶ SocialEvent ──▶ SocialRouter ──▶ Brain/Personas/Memory
Slack ────SlackAdapter────┘                        │
                                                   ▼
                                            OutboundMessage ──▶ adapter.send(...)
```

- Adapters own transport (gateway WS, long-poll, HTTP) and translate both directions.
- Guild/channel/user IDs are namespaced `"{platform}:{nativeId}"` so Fluent rows,
  SocialBrain keys, and per-guild brains never collide across networks. Discord rows
  keep their raw IDs for backward compatibility (see migration note at the bottom).

## Common Types

```swift
enum SocialNetwork: String, Codable, Sendable, CaseIterable {
    case discord, telegram, slack
}

/// One inbound thing that happened on any network.
struct SocialEvent: Sendable {
    enum Kind: Sendable {
        case message(text: String, attachments: [RemoteAttachment])
        case reaction(emoji: String, targetMessageId: String, added: Bool)
        case memberJoined
        case voiceActivity(joined: Bool)      // Discord-only today; others no-op
    }

    let network: SocialNetwork
    let kind: Kind
    let nativeMessageId: String
    let nativeChannelId: String
    let nativeGuildId: String?                // nil for DMs / networks without guilds
    let nativeUserId: String
    let userDisplayName: String
    let isBot: Bool
    let timestamp: Date

    /// Namespaced keys — what everything downstream stores and looks up by.
    var scopedGuildId: String   { "\(network.rawValue):\(nativeGuildId ?? "dm")" }
    var scopedChannelId: String { "\(network.rawValue):\(nativeChannelId)" }
    var scopedUserId: String    { "\(network.rawValue):\(nativeUserId)" }
}

struct RemoteAttachment: Sendable {
    let url: String
    let filename: String
    let contentType: String?
    var isImage: Bool { contentType?.hasPrefix("image/") ?? false }
    var isAudio: Bool { contentType?.hasPrefix("audio/") ?? false }
}

struct OutboundMessage: Sendable {
    var text: String
    var replyToNativeMessageId: String?
    /// Rich fields degrade per network: Discord renders an embed, Telegram gets
    /// Markdown, Slack gets mrkdwn blocks. Adapters own the degradation.
    var title: String?
    var accentColor: Int?
}

protocol SocialAdapter: Sendable {
    var network: SocialNetwork { get }
    /// Long-lived. Yields events until cancelled; owns its own reconnect loop.
    func events() -> AsyncStream<SocialEvent>
    func send(_ message: OutboundMessage, toNativeChannel channelId: String) async throws
    func setTyping(nativeChannelId: String) async
}
```

## SocialRouter — the single ingest point

Replaces direct `MessageHandler` wiring in `configure.swift`. The Discord
`GatewayEventHandler` path in `discordbm-api.md` still exists — `DiscordAdapter` below
is a thin translation over it.

```swift
actor SocialRouter {
    private let adapters: [any SocialAdapter]
    private let db: Database
    private let vision: any ImageUnderstanding          // vision.md
    private var pumps: [Task<Void, Never>] = []

    init(adapters: [any SocialAdapter], db: Database, vision: any ImageUnderstanding) {
        self.adapters = adapters
        self.db = db
        self.vision = vision
    }

    func start() {
        for adapter in adapters {
            let task = Task {
                for await event in adapter.events() {
                    await self.handle(event, via: adapter)
                }
            }
            pumps.append(task)
        }
    }

    func shutdown() {
        for p in pumps { p.cancel() }
        pumps.removeAll()
    }

    private func handle(_ event: SocialEvent, via adapter: any SocialAdapter) async {
        guard !event.isBot else { return }
        guard await GuildRegistry.shared.config(for: event.scopedGuildId, db: db).enabled else { return }

        switch event.kind {
        case .message(let text, let attachments):
            await handleMessage(event, text: text, attachments: attachments, via: adapter)
        case .reaction(let emoji, let targetId, let added):
            // Reactions are the reward channel — adaptive-learning.md
            await RewardCollector.shared.reaction(
                emoji: emoji, targetNativeMessageId: targetId,
                scopedGuildId: event.scopedGuildId, added: added, db: db)
        case .memberJoined:
            let persona = AbiPersona()   // welcome is always warm
            let response = await persona.respond(
                to: "__member_joined__ \(event.userDisplayName)", context: .empty, db: db)
            try? await adapter.send(.init(text: response.text), toNativeChannel: event.nativeChannelId)
        case .voiceActivity:
            break                        // handled by VoiceSessionManager (voice.md)
        }
    }

    private func handleMessage(_ event: SocialEvent, text: String,
                               attachments: [RemoteAttachment],
                               via adapter: any SocialAdapter) async {
        let record = GuildMessage(
            discordMessageId: "\(event.network.rawValue):\(event.nativeMessageId)",
            channelId: event.scopedChannelId,
            guildId: event.scopedGuildId,
            authorId: event.scopedUserId,
            content: text)
        try? await record.save(on: db)

        // Vision: describe image attachments and fold into the input (vision.md).
        var enriched = text
        for att in attachments.filter(\.isImage).prefix(3) {
            if let described = try? await vision.describe(imageURL: att.url) {
                enriched += "\n[image \(att.filename): \(described)]"
            }
        }

        let intent = IntentClassifier.classify(enriched)
        let state = StateEncoder.encode(event: event, text: enriched, intent: intent,
                                        reputation: await SocialBrain.shared.reputation(
                                            userId: event.scopedUserId,
                                            guildId: event.scopedGuildId, db: db))
        let brain = await BrainRegistry.shared.brain(for: event.scopedGuildId, db: db)
        let action = await brain.selectAction(state: state)

        // Action 0 = stay silent. Learned per guild — lurker channels train it high.
        guard action != BotAction.stay.rawValue else {
            await RewardCollector.shared.registerSilence(state: state, scopedGuildId: event.scopedGuildId)
            return
        }

        await adapter.setTyping(nativeChannelId: event.nativeChannelId)

        let persona = await ABIRouter.shared.route(intent: intent)
        let context = await MemoryAssembler.context(for: event, db: db)
        let response = await persona.respond(to: enriched, context: context, db: db)

        try? await adapter.send(
            .init(text: response.text,
                  replyToNativeMessageId: event.nativeMessageId,
                  title: persona.name),
            toNativeChannel: event.nativeChannelId)

        await RewardCollector.shared.registerReply(
            state: state, action: action,
            sentNativeMessageId: event.nativeMessageId,
            scopedGuildId: event.scopedGuildId)

        await SocialBrain.shared.recordInteraction(
            userId: event.scopedUserId, guildId: event.scopedGuildId,
            quality: intent.quality, db: db)
    }
}

/// Context assembly, shared across platforms.
enum MemoryAssembler {
    static func context(for event: SocialEvent, db: Database) async -> PersonaContext {
        let channelCtx = try? await ChannelContext.query(on: db)
            .filter(\.$channelId == event.scopedChannelId).first()
        let userMem = try? await UserMemory.query(on: db)
            .filter(\.$discordUserId == event.scopedUserId)
            .filter(\.$guildId == event.scopedGuildId).first()
        return PersonaContext(
            channelSummary: channelCtx?.summary ?? "",
            userFacts: userMem?.facts ?? [],
            reputation: userMem?.reputation ?? 0.5)
    }
}
```

## DiscordAdapter

Wraps the existing DiscordBM event stream. All the richer Discord-only handling
(interactions, components, modals) stays on the `GatewayEventHandler` path from
`discordbm-api.md` — this adapter only feeds the cross-platform pipeline.

```swift
struct DiscordAdapter: SocialAdapter {
    let network = SocialNetwork.discord
    let bot: BotGatewayManager
    let client: any DiscordClient

    func events() -> AsyncStream<SocialEvent> {
        AsyncStream { continuation in
            let task = Task {
                for await event in await bot.events {
                    if let translated = Self.translate(event) {
                        continuation.yield(translated)
                    }
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    static func translate(_ event: Gateway.Event) -> SocialEvent? {
        switch event.data {
        case .messageCreate(let m):
            guard let author = m.author else { return nil }
            return SocialEvent(
                network: .discord,
                kind: .message(
                    text: m.content ?? "",
                    attachments: (m.attachments ?? []).map {
                        RemoteAttachment(url: $0.url, filename: $0.filename, contentType: $0.content_type)
                    }),
                nativeMessageId: m.id.rawValue,
                nativeChannelId: m.channel_id.rawValue,
                nativeGuildId: m.guild_id?.rawValue,
                nativeUserId: author.id.rawValue,
                userDisplayName: author.global_name ?? author.username,
                isBot: author.bot ?? false,
                timestamp: Date())
        case .messageReactionAdd(let r):
            return SocialEvent(
                network: .discord,
                kind: .reaction(emoji: r.emoji.name ?? "", targetMessageId: r.message_id.rawValue, added: true),
                nativeMessageId: r.message_id.rawValue,
                nativeChannelId: r.channel_id.rawValue,
                nativeGuildId: r.guild_id?.rawValue,
                nativeUserId: r.user_id.rawValue,
                userDisplayName: "", isBot: false, timestamp: Date())
        case .messageReactionRemove(let r):
            return SocialEvent(
                network: .discord,
                kind: .reaction(emoji: r.emoji.name ?? "", targetMessageId: r.message_id.rawValue, added: false),
                nativeMessageId: r.message_id.rawValue,
                nativeChannelId: r.channel_id.rawValue,
                nativeGuildId: r.guild_id?.rawValue,
                nativeUserId: r.user_id.rawValue,
                userDisplayName: "", isBot: false, timestamp: Date())
        case .guildMemberAdd(let m):
            guard let user = m.user else { return nil }
            return SocialEvent(
                network: .discord, kind: .memberJoined,
                nativeMessageId: "", nativeChannelId: "",
                nativeGuildId: m.guild_id.rawValue,
                nativeUserId: user.id.rawValue,
                userDisplayName: user.global_name ?? user.username,
                isBot: user.bot ?? false, timestamp: Date())
        default:
            return nil
        }
        // NOTE(SDK): payload case names above (`.messageReactionAdd` etc.) follow
        // DiscordBM's Gateway.Event.data enum conventions seen for messageCreate;
        // verify exact case/field names against DiscordModels when wiring up.
    }

    func send(_ message: OutboundMessage, toNativeChannel channelId: String) async throws {
        try await client.createMessage(
            channelId: .init(channelId),
            payload: .init(
                content: message.title == nil ? message.text : nil,
                embeds: message.title.map { [Embed(
                    title: $0, description: message.text,
                    color: message.accentColor.map { .init(value: $0) })] },
                message_reference: message.replyToNativeMessageId.map {
                    .init(message_id: .init($0))
                }))
        .guardSuccess()
    }

    func setTyping(nativeChannelId: String) async {
        try? await client.triggerTypingIndicator(channelId: .init(nativeChannelId)).guardSuccess()
    }
}
```

## TelegramAdapter (Bot API, long-poll — no external SDK)

Telegram's Bot API is plain HTTPS JSON; long-polling `getUpdates` is the documented
transport. Raw `AsyncHTTPClient`, zero new dependencies.

```swift
actor TelegramAdapter: SocialAdapter {
    nonisolated let network = SocialNetwork.telegram
    private let token: String
    private let http: HTTPClient
    private var offset: Int64 = 0
    private var base: String { "https://api.telegram.org/bot\(token)" }

    init(token: String, http: HTTPClient) {
        self.token = token
        self.http = http
    }

    nonisolated func events() -> AsyncStream<SocialEvent> {
        AsyncStream { continuation in
            let task = Task {
                while !Task.isCancelled {
                    do {
                        for update in try await self.poll() {
                            if let e = await self.translate(update) { continuation.yield(e) }
                        }
                    } catch {
                        try? await Task.sleep(for: .seconds(5))   // backoff, then re-poll
                    }
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    private func poll() async throws -> [TGUpdate] {
        var req = HTTPClientRequest(url: "\(base)/getUpdates?timeout=50&offset=\(offset)")
        req.method = .GET
        let res = try await http.execute(req, timeout: .seconds(60))
        let body = try await res.body.collect(upTo: 4 << 20)
        let decoded = try JSONDecoder().decode(TGResponse<[TGUpdate]>.self, from: body)
        let updates = decoded.result ?? []
        if let maxId = updates.map(\.update_id).max() { offset = maxId + 1 }
        return updates
    }

    private func translate(_ u: TGUpdate) -> SocialEvent? {
        guard let m = u.message, let from = m.from else { return nil }
        var attachments: [RemoteAttachment] = []
        if let photos = m.photo, let largest = photos.max(by: { $0.width * $0.height < $1.width * $1.height }) {
            // file_id → URL requires a getFile round trip; defer to send-time fetch.
            attachments.append(.init(url: "tgfile://\(largest.file_id)",
                                     filename: "photo.jpg", contentType: "image/jpeg"))
        }
        return SocialEvent(
            network: .telegram,
            kind: .message(text: m.text ?? m.caption ?? "", attachments: attachments),
            nativeMessageId: String(m.message_id),
            nativeChannelId: String(m.chat.id),
            nativeGuildId: m.chat.type == "private" ? nil : String(m.chat.id),
            nativeUserId: String(from.id),
            userDisplayName: [from.first_name, from.last_name].compactMap { $0 }.joined(separator: " "),
            isBot: from.is_bot,
            timestamp: Date(timeIntervalSince1970: TimeInterval(m.date)))
    }

    /// Resolves a `tgfile://` pseudo-URL into a real download URL for the vision layer.
    func resolveFileURL(fileId: String) async throws -> String {
        var req = HTTPClientRequest(url: "\(base)/getFile?file_id=\(fileId)")
        req.method = .GET
        let res = try await http.execute(req, timeout: .seconds(15))
        let body = try await res.body.collect(upTo: 1 << 20)
        let decoded = try JSONDecoder().decode(TGResponse<TGFile>.self, from: body)
        guard let path = decoded.result?.file_path else { throw Abort(.notFound) }
        return "https://api.telegram.org/file/bot\(token)/\(path)"
    }

    nonisolated func send(_ message: OutboundMessage, toNativeChannel channelId: String) async throws {
        struct SendMessage: Encodable {
            let chat_id: String
            let text: String
            let parse_mode: String
            let reply_to_message_id: Int?
        }
        let text = message.title.map { "*\($0)*\n\(message.text)" } ?? message.text
        let payload = SendMessage(
            chat_id: channelId, text: text, parse_mode: "Markdown",
            reply_to_message_id: message.replyToNativeMessageId.flatMap(Int.init))
        var req = HTTPClientRequest(url: "https://api.telegram.org/bot\(token)/sendMessage")
        req.method = .POST
        req.headers.add(name: "Content-Type", value: "application/json")
        req.body = .bytes(try JSONEncoder().encode(payload))
        _ = try await http.execute(req, timeout: .seconds(15))
    }

    nonisolated func setTyping(nativeChannelId: String) async {
        var req = HTTPClientRequest(
            url: "https://api.telegram.org/bot\(token)/sendChatAction?chat_id=\(nativeChannelId)&action=typing")
        req.method = .GET
        _ = try? await http.execute(req, timeout: .seconds(10))
    }
}

// Minimal Telegram wire types — only the fields Abbey reads.
struct TGResponse<T: Decodable>: Decodable { let ok: Bool; let result: T? }
struct TGUpdate: Decodable { let update_id: Int64; let message: TGMessage? }
struct TGMessage: Decodable {
    let message_id: Int64
    let from: TGUser?
    let chat: TGChat
    let date: Int64
    let text: String?
    let caption: String?
    let photo: [TGPhotoSize]?
}
struct TGUser: Decodable { let id: Int64; let is_bot: Bool; let first_name: String; let last_name: String? }
struct TGChat: Decodable { let id: Int64; let type: String }
struct TGPhotoSize: Decodable { let file_id: String; let width: Int; let height: Int }
struct TGFile: Decodable { let file_path: String? }
```

## SlackAdapter (Events API over HTTP + Web API)

Slack pushes events to a Vapor route (challenge-verified), sends go out via
`chat.postMessage`. Socket Mode is the alternative when inbound HTTP isn't exposable —
same event JSON, different transport; not duplicated here.

```swift
actor SlackAdapter: SocialAdapter {
    nonisolated let network = SocialNetwork.slack
    private let botToken: String            // xoxb-…
    private let signingSecret: String
    private let http: HTTPClient
    private var continuation: AsyncStream<SocialEvent>.Continuation?

    init(botToken: String, signingSecret: String, http: HTTPClient) {
        self.botToken = botToken
        self.signingSecret = signingSecret
        self.http = http
    }

    nonisolated func events() -> AsyncStream<SocialEvent> {
        AsyncStream { continuation in
            Task { await self.store(continuation) }
        }
    }
    private func store(_ c: AsyncStream<SocialEvent>.Continuation) { continuation = c }

    /// Mount on Vapor: `app.post("slack", "events") { try await slackAdapter.receive($0) }`
    func receive(_ req: Request) async throws -> Response {
        guard verifySignature(req) else { throw Abort(.unauthorized) }
        let envelope = try req.content.decode(SlackEnvelope.self)

        if let challenge = envelope.challenge {        // one-time URL verification
            return Response(status: .ok, body: .init(string: challenge))
        }
        if let event = envelope.event, event.type == "message",
           event.bot_id == nil, let user = event.user {
            continuation?.yield(SocialEvent(
                network: .slack,
                kind: .message(text: event.text ?? "", attachments: (event.files ?? []).map {
                    RemoteAttachment(url: $0.url_private, filename: $0.name, contentType: $0.mimetype)
                }),
                nativeMessageId: event.ts ?? "",
                nativeChannelId: event.channel ?? "",
                nativeGuildId: envelope.team_id,
                nativeUserId: user,
                userDisplayName: user,                 // resolved lazily via users.info if needed
                isBot: false,
                timestamp: Date()))
        }
        if let event = envelope.event, event.type == "reaction_added",
           let user = event.user, let item = event.item {
            continuation?.yield(SocialEvent(
                network: .slack,
                kind: .reaction(emoji: event.reaction ?? "", targetMessageId: item.ts, added: true),
                nativeMessageId: item.ts, nativeChannelId: item.channel,
                nativeGuildId: envelope.team_id, nativeUserId: user,
                userDisplayName: user, isBot: false, timestamp: Date()))
        }
        return Response(status: .ok)                   // Slack requires 200 within 3s
    }

    /// v0 signature: HMAC-SHA256 over "v0:{ts}:{rawBody}" with the signing secret.
    private func verifySignature(_ req: Request) -> Bool {
        guard let ts = req.headers["X-Slack-Request-Timestamp"].first,
              let sig = req.headers["X-Slack-Signature"].first,
              let body = req.body.data.map({ Data($0.readableBytesView) }),
              let sent = Double(ts), abs(Date().timeIntervalSince1970 - sent) < 300
        else { return false }
        let base = Data("v0:\(ts):".utf8) + body
        let mac = HMAC<SHA256>.authenticationCode(for: base, using: .init(data: Data(signingSecret.utf8)))
        let computed = "v0=" + mac.map { String(format: "%02x", $0) }.joined()
        return constantTimeEquals(computed, sig)
    }

    nonisolated func send(_ message: OutboundMessage, toNativeChannel channelId: String) async throws {
        struct PostMessage: Encodable {
            let channel: String
            let text: String
            let thread_ts: String?
        }
        let text = message.title.map { "*\($0)*\n\(message.text)" } ?? message.text
        var req = HTTPClientRequest(url: "https://slack.com/api/chat.postMessage")
        req.method = .POST
        req.headers.add(name: "Authorization", value: "Bearer \(botToken)")
        req.headers.add(name: "Content-Type", value: "application/json; charset=utf-8")
        req.body = .bytes(try JSONEncoder().encode(PostMessage(
            channel: channelId, text: text, thread_ts: message.replyToNativeMessageId)))
        _ = try await http.execute(req, timeout: .seconds(15))
    }

    nonisolated func setTyping(nativeChannelId: String) async {
        // Slack has no bot typing indicator over the Web API (RTM-only, deprecated). No-op.
    }
}

struct SlackEnvelope: Decodable {
    let challenge: String?
    let team_id: String?
    let event: SlackEvent?
}
struct SlackEvent: Decodable {
    let type: String
    let user: String?
    let text: String?
    let ts: String?
    let channel: String?
    let bot_id: String?
    let reaction: String?
    let item: SlackItem?
    let files: [SlackFile]?
}
struct SlackItem: Decodable { let channel: String; let ts: String }
struct SlackFile: Decodable { let name: String; let mimetype: String?; let url_private: String }
```

## Migration note — scoped IDs vs existing Discord rows

Existing rows store raw Discord snowflakes; the router writes `discord:{id}` going
forward. One-time backfill (idempotent):

```sql
UPDATE guild_messages    SET guild_id = 'discord:' || guild_id,  channel_id = 'discord:' || channel_id, author_id = 'discord:' || author_id WHERE guild_id NOT LIKE '%:%';
UPDATE user_memories     SET guild_id = 'discord:' || guild_id,  discord_user_id = 'discord:' || discord_user_id WHERE guild_id NOT LIKE '%:%';
UPDATE channel_contexts  SET guild_id = 'discord:' || guild_id,  channel_id = 'discord:' || channel_id WHERE guild_id NOT LIKE '%:%';
UPDATE reputation_events SET guild_id = 'discord:' || guild_id,  user_id = 'discord:' || user_id WHERE guild_id NOT LIKE '%:%';
```

Column names (`discord_message_id`, `discord_user_id`) are now historical misnomers —
they hold scoped cross-platform IDs. Renaming them is a schema churn decision for
Donald, not something to do silently.
