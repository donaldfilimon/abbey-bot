# Adaptive Learning — State Encoding, Rewards, Per-Guild Brains

Extends `brain.md`. That file defines the machinery (`NeuralNetwork`, `DQNAgent`,
`ReplayBuffer`); this file defines what was always missing around it: **what the state
vector actually is, where rewards come from, and how policies stay per-guild and
survive restarts.** Nothing here changes the learning dynamics in `brain.md` — it
completes the loop around them.

## The action space

```swift
enum BotAction: Int, CaseIterable, Sendable {
    case stay = 0       // don't reply — the most important action to learn
    case reply = 1      // reply as the routed persona
    case react = 2      // emoji-react only (low-cost acknowledgment)
}
```

Topology becomes `[18, 64, 32, 3]`. `brain.md`'s example topology was
`[128, 64, 32, 3]` with no encoder ever defined for the 128 — this isn't a revert of a
working design, it's the first concrete definition of the input side. The 18th
dimension is deterministic sentiment, matching the AbbeyBot native-macOS design.

## StateEncoder — 18 dimensions

Deterministic, no learned components, so the same message always produces the same
state — replays stay valid across restarts.

```swift
enum StateEncoder {
    static let dimensions = 18

    /// Layout:
    ///  [0..8]  intent one-hot (9 intents, IntentClassifier.Intent.allCases order)
    ///  [9]     author reputation                       0…1
    ///  [10]    message length, capped at 400 chars     0…1
    ///  [11]    mentions the bot                        0|1
    ///  [12]    is a question                           0|1
    ///  [13]    has image attachment(s)                 0|1
    ///  [14]    hour-of-day sin                         -1…1
    ///  [15]    hour-of-day cos                         -1…1
    ///  [16]    channel heat: messages in last 5 min, capped at 30, 0…1
    ///  [17]    deterministic sentiment                 -1…1
    static func encode(event: SocialEvent, text: String,
                       intent: IntentClassifier.Intent,
                       reputation: Double,
                       channelHeat: Int = 0,
                       mentionsBot: Bool = false) -> [Float] {
        var s = [Float](repeating: 0, count: dimensions)

        if let idx = IntentClassifier.Intent.allCases.firstIndex(of: intent) { s[idx] = 1 }
        s[9]  = Float(reputation)
        s[10] = Float(min(text.count, 400)) / 400
        s[11] = mentionsBot ? 1 : 0
        s[12] = text.hasSuffix("?") ? 1 : 0
        if case .message(_, let atts) = event.kind {
            s[13] = atts.contains(where: \.isImage) ? 1 : 0
        }
        let hour = Double(Calendar.current.component(.hour, from: event.timestamp))
        s[14] = Float(sin(2 * .pi * hour / 24))
        s[15] = Float(cos(2 * .pi * hour / 24))
        s[16] = Float(min(channelHeat, 30)) / 30
        s[17] = Sentiment.score(text)
        return s
    }
}

/// Deterministic lexicon sentiment — the 18th state dimension. Intentionally not an
/// ML model: reproducibility beats accuracy for a reward-shaping feature.
enum Sentiment {
    private static let positive: Set<String> = [
        "love","great","awesome","nice","good","thanks","thank","cool","amazing",
        "best","happy","lol","lmao","haha","w","based","fire","goat","pog","clean",
    ]
    private static let negative: Set<String> = [
        "hate","bad","awful","terrible","worst","sucks","trash","angry","sad",
        "annoying","stupid","dumb","l","mid","cringe","broken","ugh","wtf",
    ]

    /// -1…1: (pos - neg) / tokens, clamped, with light emoji weighting.
    static func score(_ text: String) -> Float {
        let tokens = text.lowercased()
            .components(separatedBy: CharacterSet.alphanumerics.inverted)
            .filter { !$0.isEmpty }
        guard !tokens.isEmpty else { return 0 }
        var score = 0
        for t in tokens {
            if positive.contains(t) { score += 1 }
            if negative.contains(t) { score -= 1 }
        }
        for scalar in text.unicodeScalars {
            switch scalar {
            case "❤", "😂", "🔥", "👍", "😍": score += 1
            case "💀", "👎", "😡", "🤮":      score -= 1
            default: break
            }
        }
        return max(-1, min(1, Float(score) / Float(max(tokens.count, 4))))
    }
}
```

## Reward signal

The DQN's reward is *delayed*: Abbey acts now, the guild reacts over the next couple
of minutes. `RewardCollector` holds pending experiences open for a settlement window,
accumulates evidence, then closes them into the guild's replay buffer.

| Signal | Reward |
|---|---|
| 👍 / ❤️ / 🔥 / positive reaction on Abbey's reply | +1.0 each (capped +3) |
| 👎 / 💀 / negative reaction | −1.0 each |
| A human replies to Abbey's message | +0.5 |
| Abbey's message deleted by a mod | −2.0, settle immediately |
| Replied, then nothing at all for the window | −0.2 (mild spam pressure) |
| Stayed silent | 0.0 (neutral baseline — silence is never punished) |
| Reacted (`.react`) and got a reaction back | +0.5 |

```swift
actor RewardCollector {
    static let shared = RewardCollector()

    struct Pending {
        let state: [Float]
        let action: Int
        let scopedGuildId: String
        var reward: Float
        var positiveReactions: Int
        let createdAt: Date
        var settleImmediately: Bool
    }

    static let settlementWindow: TimeInterval = 150     // 2.5 min
    private var pending: [String: Pending] = [:]        // key: nativeMessageId Abbey replied TO
    private var sweeper: Task<Void, Never>?

    private static let positiveEmoji: Set<String> = ["👍","❤️","🔥","😂","💯","⭐"]
    private static let negativeEmoji: Set<String> = ["👎","💀","😡","🤮"]

    func startSweeping(db: Database) {
        sweeper?.cancel()
        sweeper = Task {
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(30))
                await settleExpired(db: db)
            }
        }
    }

    func registerReply(state: [Float], action: Int, sentNativeMessageId: String,
                       scopedGuildId: String) {
        pending[sentNativeMessageId] = Pending(
            state: state, action: action, scopedGuildId: scopedGuildId,
            reward: -0.2,                    // starts mildly negative; engagement earns it back
            positiveReactions: 0, createdAt: Date(), settleImmediately: false)
    }

    /// Silence settles instantly at 0 — there's nothing to wait for.
    func registerSilence(state: [Float], scopedGuildId: String) async {
        let exp = Experience(state: state, action: BotAction.stay.rawValue,
                             reward: 0, nextState: state, done: true)
        await BrainRegistry.shared.remember(exp, scopedGuildId: scopedGuildId)
    }

    func reaction(emoji: String, targetNativeMessageId: String,
                  scopedGuildId: String, added: Bool, db: Database) {
        guard var p = pending[targetNativeMessageId], added else { return }
        if Self.positiveEmoji.contains(emoji), p.positiveReactions < 3 {
            p.reward += 1.0
            p.positiveReactions += 1
        } else if Self.negativeEmoji.contains(emoji) {
            p.reward -= 1.0
        }
        pending[targetNativeMessageId] = p
    }

    func humanReplied(toNativeMessageId id: String) {
        pending[id]?.reward += 0.5
    }

    func abbeyMessageDeleted(nativeMessageId: String) {
        pending[nativeMessageId]?.reward = -2.0
        pending[nativeMessageId]?.settleImmediately = true
    }

    private func settleExpired(db: Database) async {
        let now = Date()
        for (key, p) in pending
        where p.settleImmediately || now.timeIntervalSince(p.createdAt) > Self.settlementWindow {
            pending[key] = nil
            // Bandit-style episode: single step, done=true, nextState==state. The
            // gamma term in brain.md's Bellman update zeroes out via `done` — this is
            // deliberate; conversational credit assignment beyond one exchange isn't
            // worth the variance.
            let exp = Experience(state: p.state, action: p.action,
                                 reward: max(-3, min(3, p.reward)),
                                 nextState: p.state, done: true)
            await BrainRegistry.shared.remember(exp, scopedGuildId: p.scopedGuildId)
        }
    }
}
```

Wire the two extra signals in `EventHandler`:

```swift
func onMessageDelete(_ payload: Gateway.MessageDelete) async throws {
    await RewardCollector.shared.abbeyMessageDeleted(nativeMessageId: payload.id.rawValue)
}
// In onMessageCreate, before normal handling:
// if let ref = payload.referenced_message?.id.rawValue {
//     await RewardCollector.shared.humanReplied(toNativeMessageId: ref)
// }
```

## BrainState — per-guild persistence

```swift
final class BrainState: Model, Content, @unchecked Sendable {
    static let schema = "brain_states"
    @ID(key: .id) var id: UUID?
    @Field(key: "scoped_guild_id") var scopedGuildId: String
    @Field(key: "snapshot")        var snapshot: Data      // JSON-encoded BrainSnapshot
    @Field(key: "experiences")     var experienceCount: Int
    @Timestamp(key: "updated_at", on: .update) var updatedAt: Date?

    init() {}
    init(id: UUID? = nil, scopedGuildId: String, snapshot: Data, experienceCount: Int) {
        self.id = id
        self.scopedGuildId = scopedGuildId
        self.snapshot = snapshot
        self.experienceCount = experienceCount
    }
}

struct CreateBrainState: AsyncMigration {
    func prepare(on database: Database) async throws {
        try await database.schema(BrainState.schema)
            .id()
            .field("scoped_guild_id", .string, .required)
            .field("snapshot",        .data,   .required)
            .field("experiences",     .int,    .required)
            .field("updated_at",      .datetime)
            .unique(on: "scoped_guild_id")
            .create()
    }
    func revert(on database: Database) async throws {
        try await database.schema(BrainState.schema).delete()
    }
}
```

## BrainRegistry — one policy per guild

```swift
actor BrainRegistry {
    static let shared = BrainRegistry()

    private var brains: [String: DQNAgent] = [:]
    private var experienceCounts: [String: Int] = [:]
    private var lastTouched: [String: Date] = [:]
    private let evictAfter: TimeInterval = 6 * 3600      // idle guilds unload; snapshot persists

    func brain(for scopedGuildId: String, db: Database) async -> DQNAgent {
        lastTouched[scopedGuildId] = Date()
        if let existing = brains[scopedGuildId] { return existing }

        let agent = DQNAgent(topology: [StateEncoder.dimensions, 64, 32, BotAction.allCases.count])
        if let row = try? await BrainState.query(on: db)
            .filter(\.$scopedGuildId == scopedGuildId).first(),
           let snapshot = try? JSONDecoder().decode(BrainSnapshot.self, from: row.snapshot) {
            await agent.importWeights(snapshot)
            experienceCounts[scopedGuildId] = row.experienceCount
        }
        brains[scopedGuildId] = agent
        return agent
    }

    func remember(_ exp: Experience, scopedGuildId: String) async {
        guard let agent = brains[scopedGuildId] else { return }
        await agent.remember(exp)
        experienceCounts[scopedGuildId, default: 0] += 1
    }

    /// AbbeyScheduler tick: learn on every loaded brain whose guild has learning on.
    func learnAll(db: Database) async {
        for (guildId, agent) in brains {
            guard await GuildRegistry.shared.config(for: guildId, db: db).learningEnabled else { continue }
            await agent.learn()
        }
    }

    /// AbbeyScheduler tick (less frequent): snapshot every loaded brain.
    func persistAll(db: Database) async {
        for (guildId, agent) in brains {
            await persist(guildId: guildId, agent: agent, db: db)
        }
        // Evict idle brains after persisting them.
        let cutoff = Date().addingTimeInterval(-evictAfter)
        for (guildId, touched) in lastTouched where touched < cutoff {
            brains[guildId] = nil
            lastTouched[guildId] = nil
        }
    }

    func persistAndEvict(scopedGuildId: String, db: Database) async {
        if let agent = brains[scopedGuildId] {
            await persist(guildId: scopedGuildId, agent: agent, db: db)
        }
        brains[scopedGuildId] = nil
        lastTouched[scopedGuildId] = nil
    }

    private func persist(guildId: String, agent: DQNAgent, db: Database) async {
        let snapshot = await agent.exportWeights()
        guard let data = try? JSONEncoder().encode(snapshot) else { return }
        let count = experienceCounts[guildId] ?? 0
        if let row = try? await BrainState.query(on: db)
            .filter(\.$scopedGuildId == guildId).first() {
            row.snapshot = data
            row.experienceCount = count
            try? await row.save(on: db)
        } else {
            try? await BrainState(scopedGuildId: guildId, snapshot: data,
                                  experienceCount: count).save(on: db)
        }
    }
}
```

## AbbeyScheduler — the background heartbeat

Replaces the loose `Task { while … }` loops in `configure.swift` with one owned actor;
graceful shutdown flushes everything.

```swift
actor AbbeyScheduler {
    static let shared = AbbeyScheduler()
    private var tasks: [Task<Void, Never>] = []

    func start(db: Database) {
        schedule(every: .seconds(30))  { await BrainRegistry.shared.learnAll(db: db) }
        schedule(every: .seconds(60))  { await SocialBrain.shared.flush(db: db) }
        schedule(every: .seconds(300)) { await BrainRegistry.shared.persistAll(db: db) }
        Task { await RewardCollector.shared.startSweeping(db: db) }
    }

    private func schedule(every interval: Duration, _ body: @escaping @Sendable () async -> Void) {
        tasks.append(Task {
            while !Task.isCancelled {
                try? await Task.sleep(for: interval)
                await body()
            }
        })
    }

    /// Call from the Vapor shutdown lifecycle handler.
    func shutdown(db: Database) async {
        for t in tasks { t.cancel() }
        tasks.removeAll()
        await SocialBrain.shared.flush(db: db)
        await BrainRegistry.shared.persistAll(db: db)
    }
}
```

`configure.swift` change: replace the inline flush `Task` with
`await AbbeyScheduler.shared.start(db: app.db)` and register shutdown via
`app.lifecycle.use(SchedulerLifecycle())` where `SchedulerLifecycle.shutdownAsync`
calls `AbbeyScheduler.shared.shutdown(db:)`.

## Cold-start behavior

A fresh guild's brain is random weights + ε=0.1 exploration over a 3-action space —
it will occasionally reply to things it shouldn't for the first few hundred
experiences. Two dampeners are already in place: the reply cooldown
(`multi-guild.md`) hard-caps unsolicited output regardless of policy, and `stay`
settling at 0 while a bad `reply` settles negative means the silent policy dominates
early. If a guild wants zero learning-phase noise, `/admin learning off` pins Abbey to
mention-and-command-only responses.
