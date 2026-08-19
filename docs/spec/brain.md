# Brain — Neural Network + DQN + SocialBrain

Donald's own ML core, not a third-party library — no external API to verify against.
Preserved as designed; flag any behavioral changes to Donald rather than "fixing" the
learning dynamics unilaterally.

## ⚠ Open decision — softmax on Q-values

`forward` previously applied `softmax` to the output layer while `DQNAgent` used the
result as Q-values in a Bellman update. Those are incompatible: softmax normalizes to a
probability simplex (sums to 1, always positive), so it destroys the magnitude and sign
that `reward + γ·max Q(s',a')` depends on. `argmax` still picks the same action, so this
degrades silently rather than crashing — training just never converges.

Per the "don't unilaterally change learning dynamics" rule this is **not** silently
fixed. The output activation is now explicit (`OutputActivation`), and both paths work:

- `.linear` — correct for DQN. **Recommended.**
- `.softmax` — preserves the old behavior; correct if the net is reused as a classifier.

`DQNAgent` below constructs its networks with `.linear`. If Abbey's DQN was tuned
against softmaxed Q-values, changing this will change behavior — Donald's call.

## NeuralNetwork.swift (feed-forward, SIMD-accelerated)

SIMD is applied *along the input axis* — each neuron's dot product is accumulated in
`SIMD8<Float>` lanes and reduced once. The previous `[[SIMD8<Float>]]` packing stored one
`SIMD8` per neuron, which capped every neuron at 8 inputs and could not represent the
declared `[128, 64, 32, 3]` topology.

```swift
enum OutputActivation: Sendable { case linear, softmax }

struct DenseLayer: Sendable {
    var weights: [Float]        // row-major, [outputCount * inputCount]
    var biases: [Float]         // [outputCount]
    let inputCount: Int
    let outputCount: Int

    init(inputCount: Int, outputCount: Int) {
        self.inputCount = inputCount
        self.outputCount = outputCount
        // He initialization — correct pairing for ReLU hidden layers.
        let scale = (2.0 / Float(inputCount)).squareRoot()
        self.weights = (0..<(inputCount * outputCount)).map { _ in .random(in: -scale...scale) }
        self.biases = [Float](repeating: 0, count: outputCount)
    }

    /// Dot product of `weights[row]` and `input`, SIMD8-accumulated along the input axis.
    func dot(row: Int, _ input: [Float]) -> Float {
        let base = row * inputCount
        var acc = SIMD8<Float>(repeating: 0)
        var i = 0
        while i + 8 <= inputCount {
            var w = SIMD8<Float>(), x = SIMD8<Float>()
            for lane in 0..<8 {
                w[lane] = weights[base + i + lane]
                x[lane] = input[i + lane]
            }
            acc += w * x                       // fused multiply-accumulate across 8 lanes
            i += 8
        }
        var total = acc.sum()
        while i < inputCount {                 // scalar tail for non-multiple-of-8 widths
            total += weights[base + i] * input[i]
            i += 1
        }
        return total
    }
}

struct NeuralNetwork: Sendable {
    var layers: [DenseLayer]
    let topology: [Int]                        // e.g. [128, 64, 32, 3]
    let outputActivation: OutputActivation

    init(topology: [Int], outputActivation: OutputActivation = .linear) {
        precondition(topology.count >= 2, "need at least an input and an output layer")
        self.topology = topology
        self.outputActivation = outputActivation
        self.layers = (0..<(topology.count - 1)).map {
            DenseLayer(inputCount: topology[$0], outputCount: topology[$0 + 1])
        }
    }

    /// Non-mutating: inference must be callable from `let` bindings (e.g. the target net).
    func forward(_ input: [Float]) -> [Float] {
        forwardRetainingActivations(input).output
    }

    /// Returns every layer's pre- and post-activation values — backprop needs both.
    func forwardRetainingActivations(_ input: [Float]) -> (output: [Float], pre: [[Float]], post: [[Float]]) {
        var pre: [[Float]] = []
        var post: [[Float]] = [input]
        var activation = input

        for (idx, layer) in layers.enumerated() {
            var z = [Float](repeating: 0, count: layer.outputCount)
            for row in 0..<layer.outputCount {
                z[row] = layer.dot(row: row, activation) + layer.biases[row]
            }
            pre.append(z)
            let isOutput = (idx == layers.count - 1)
            activation = isOutput ? applyOutputActivation(z) : z.map(Self.relu)
            post.append(activation)
        }
        return (activation, pre, post)
    }

    private func applyOutputActivation(_ z: [Float]) -> [Float] {
        switch outputActivation {
        case .linear:  return z
        case .softmax: return Self.softmax(z)
        }
    }

    /// One SGD step. Gradients clipped at ±1.0 (unchanged from the original design).
    /// `target` is a full output-width vector; for DQN only the taken action's slot
    /// differs from the current prediction (see `makeTarget`), so the other slots
    /// contribute zero error and the update is effectively single-action.
    mutating func train(input: [Float], target: [Float], lr: Float = 0.001) {
        let (output, pre, post) = forwardRetainingActivations(input)
        precondition(target.count == output.count, "target width must match output width")

        // Output-layer error. For .linear + squared error, and for .softmax +
        // cross-entropy, dL/dz reduces to the same (output - target) — the activation
        // derivative cancels in both pairings.
        var delta = zip(output, target).map { $0 - $1 }

        for idx in stride(from: layers.count - 1, through: 0, by: -1) {
            let layer = layers[idx]
            let inputToLayer = post[idx]
            var nextDelta = [Float](repeating: 0, count: layer.inputCount)

            for row in 0..<layer.outputCount {
                let d = Self.clip(delta[row])
                guard d != 0 else { continue }
                let base = row * layer.inputCount
                for col in 0..<layer.inputCount {
                    // Propagate before the weight is overwritten.
                    nextDelta[col] += layers[idx].weights[base + col] * d
                    layers[idx].weights[base + col] -= lr * d * inputToLayer[col]
                }
                layers[idx].biases[row] -= lr * d
            }

            if idx > 0 {
                // ReLU derivative: pass the gradient only where the unit fired.
                let z = pre[idx - 1]
                for col in 0..<nextDelta.count {
                    nextDelta[col] = z[col] > 0 ? nextDelta[col] : 0
                }
            }
            delta = nextDelta
        }
    }

    static func relu(_ x: Float) -> Float { max(0, x) }
    static func clip(_ g: Float) -> Float { min(max(g, -1.0), 1.0) }

    /// Max-subtracted for numerical stability — raw `exp` overflows on large logits.
    static func softmax(_ z: [Float]) -> [Float] {
        guard let maxZ = z.max() else { return z }
        let exps = z.map { Foundation.exp($0 - maxZ) }
        let sum = exps.reduce(0, +)
        return sum > 0 ? exps.map { $0 / sum } : z
    }
}
```

## ReplayBuffer.swift

Fixed-capacity circular buffer. Overwrites oldest on wrap — no unbounded growth across
a long-running gateway session.

```swift
struct Experience: Sendable {
    let state: [Float]
    let action: Int
    let reward: Float
    let nextState: [Float]
    let done: Bool
}

struct ReplayBuffer: Sendable {
    private var storage: [Experience] = []
    private var writeIndex = 0
    let capacity: Int

    init(capacity: Int = 10_000) {
        self.capacity = capacity
        storage.reserveCapacity(capacity)
    }

    var count: Int { storage.count }
    var isEmpty: Bool { storage.isEmpty }

    mutating func append(_ experience: Experience) {
        if storage.count < capacity {
            storage.append(experience)
        } else {
            storage[writeIndex] = experience          // circular overwrite
            writeIndex = (writeIndex + 1) % capacity
        }
    }

    /// Uniform sample with replacement. Returns [] if under-filled — callers gate on
    /// `count` before calling, but this stays total rather than trapping.
    func sample(size: Int) -> [Experience] {
        guard !storage.isEmpty else { return [] }
        return (0..<size).map { _ in storage[Int.random(in: 0..<storage.count)] }
    }

    mutating func removeAll() {
        storage.removeAll(keepingCapacity: true)
        writeIndex = 0
    }
}
```

## DQNAgent.swift

```swift
actor DQNAgent {
    private var online: NeuralNetwork
    private var target: NeuralNetwork
    private var buffer: ReplayBuffer

    private let gamma: Float = 0.99
    private var epsilon: Float = 0.1            // ε-greedy exploration
    private let epsilonMin: Float = 0.01
    private let epsilonDecay: Float = 0.995
    private let batchSize = 64
    private let targetSyncInterval = 100
    private var stepCount = 0

    var actionCount: Int { online.topology.last! }

    init(topology: [Int] = [128, 64, 32, 3], bufferCapacity: Int = 10_000) {
        // .linear — Q-values must keep magnitude and sign. See the open decision above.
        self.online = NeuralNetwork(topology: topology, outputActivation: .linear)
        self.target = self.online              // target starts as an exact copy
        self.buffer = ReplayBuffer(capacity: bufferCapacity)
    }

    /// `forward` is sync and non-mutating — no `await` here (the previous version
    /// awaited a non-async call, which does not compile).
    func selectAction(state: [Float]) -> Int {
        if Float.random(in: 0...1) < epsilon {
            return Int.random(in: 0..<actionCount)
        }
        let qValues = online.forward(state)
        return qValues.enumerated().max(by: { $0.element < $1.element })!.offset
    }

    func remember(_ experience: Experience) {
        buffer.append(experience)
    }

    func learn() {
        guard buffer.count >= batchSize else { return }

        for exp in buffer.sample(size: batchSize) {
            // Terminal states bootstrap nothing — their future value is 0 by definition.
            let futureQ = exp.done ? 0 : (target.forward(exp.nextState).max() ?? 0)
            let targetQ = exp.reward + gamma * futureQ
            let predicted = online.forward(exp.state)
            online.train(input: exp.state, target: makeTarget(predicted, action: exp.action, value: targetQ))
        }

        stepCount += 1
        if stepCount % targetSyncInterval == 0 {
            target = online                     // hard sync of the target network
        }
        epsilon = max(epsilonMin, epsilon * epsilonDecay)
    }

    /// Copy the prediction, then overwrite only the taken action's slot. Untouched
    /// slots yield zero error, so no gradient flows for actions that weren't taken.
    private func makeTarget(_ predicted: [Float], action: Int, value: Float) -> [Float] {
        var t = predicted
        t[action] = value
        return t
    }

    // --- Persistence: without this the agent relearns from scratch on every restart ---

    func exportWeights() -> BrainSnapshot {
        BrainSnapshot(topology: online.topology,
                      layers: online.layers.map { .init(weights: $0.weights, biases: $0.biases) },
                      epsilon: epsilon,
                      stepCount: stepCount)
    }

    func importWeights(_ snapshot: BrainSnapshot) {
        guard snapshot.topology == online.topology else { return }   // reject shape drift
        for (idx, saved) in snapshot.layers.enumerated() where idx < online.layers.count {
            online.layers[idx].weights = saved.weights
            online.layers[idx].biases = saved.biases
        }
        target = online
        epsilon = snapshot.epsilon
        stepCount = snapshot.stepCount
    }
}

struct BrainSnapshot: Codable, Sendable {
    struct Layer: Codable, Sendable {
        var weights: [Float]
        var biases: [Float]
    }
    var topology: [Int]
    var layers: [Layer]
    var epsilon: Float
    var stepCount: Int
}
```

## IntentClassifier.swift

```swift
struct IntentClassifier {
    enum Intent: String, CaseIterable, Sendable {
        case question, greeting, modRequest, memoryStore, personaSwitch,
             repQuery, smallTalk, command, unknown

        var quality: Double {
            switch self {
            case .question, .modRequest, .memoryStore: return 0.8
            case .greeting, .smallTalk: return 0.5
            case .unknown: return 0.2
            default: return 0.6
            }
        }
    }

    static func classify(_ text: String) -> Intent {
        let lower = text.lowercased()
        if lower.hasPrefix("!") || lower.hasPrefix("/") { return .command }
        if lower.contains("remember") || lower.contains("note that") { return .memoryStore }
        if lower.contains("rep") || lower.contains("reputation") { return .repQuery }
        if lower.contains("switch") || lower.contains("be aviva") { return .personaSwitch }
        if lower.hasSuffix("?") || lower.hasPrefix("what") || lower.hasPrefix("how") { return .question }
        if ["hi","hey","yo","sup","hello"].contains(where: { lower.hasPrefix($0) }) { return .greeting }
        return .smallTalk
    }

    static func suggestCompletions(for partial: String) -> [String] {
        let corpus = ["what do you think about", "how does", "can you help me with",
                      "remind me", "what is the reputation of"]
        return corpus.filter { $0.hasPrefix(partial.lowercased()) }
    }
}
```

**Note — `.unknown` is unreachable.** `classify` returns `.smallTalk` as its fallthrough,
so nothing ever produces `.unknown` despite it carrying the distinct `quality: 0.2`
penalty. Either the fallthrough should be `.unknown` (and `.smallTalk` reserved for a
positive match), or `.unknown` should be dropped from the enum. Behavior change either
way — Donald's call, left as-is.

## SocialBrain.swift — Reputation Engine

The original kept `scores` purely in memory with a comment claiming periodic DB flush.
No flush existed, and nothing loaded from `ReputationEvent` on boot — so every restart
silently reset the entire guild's reputation to 0.5 while the audit trail kept
accumulating rows that nothing ever read. Hydrate-on-read + write-through below.

```swift
actor SocialBrain {
    static let shared = SocialBrain()

    private var scores: [String: Double] = [:]      // key: "\(guildId):\(userId)"
    private var dirty: Set<String> = []

    private func key(_ guildId: String, _ userId: String) -> String { "\(guildId):\(userId)" }

    /// Reads through to UserMemory on a cache miss, so a restart no longer wipes standing.
    func reputation(userId: String, guildId: String, db: Database) async -> Double {
        let k = key(guildId, userId)
        if let cached = scores[k] { return cached }

        let stored = try? await UserMemory.query(on: db)
            .filter(\.$discordUserId == userId)
            .filter(\.$guildId == guildId)
            .first()
        let value = stored?.reputation ?? 0.5
        scores[k] = value
        return value
    }

    func recordInteraction(userId: String, guildId: String, quality: Double, db: Database) async {
        let k = key(guildId, userId)
        let current = await reputation(userId: userId, guildId: guildId, db: db)
        // Exponential moving average — slow decay, fast reward (unchanged).
        let updated = current * 0.95 + quality * 0.05
        scores[k] = updated
        dirty.insert(k)

        let event = ReputationEvent(userId: userId, guildId: guildId,
                                    delta: updated - current, reason: "interaction")
        try? await event.save(on: db)
    }

    func penalize(userId: String, guildId: String, reason: String, db: Database) async {
        let k = key(guildId, userId)
        let current = await reputation(userId: userId, guildId: guildId, db: db)
        let updated = max(0, current - 0.1)
        scores[k] = updated
        dirty.insert(k)

        let event = ReputationEvent(userId: userId, guildId: guildId, delta: -0.1, reason: reason)
        try? await event.save(on: db)
    }

    /// Write-back of dirty scores onto UserMemory. Call from AbbeyScheduler on an
    /// interval and once during graceful shutdown — ReputationEvent is an append-only
    /// audit trail, UserMemory.reputation is the queryable current value.
    func flush(db: Database) async {
        for k in dirty {
            let parts = k.split(separator: ":", maxSplits: 1).map(String.init)
            guard parts.count == 2, let value = scores[k] else { continue }
            let (guildId, userId) = (parts[0], parts[1])

            if let mem = try? await UserMemory.query(on: db)
                .filter(\.$discordUserId == userId)
                .filter(\.$guildId == guildId)
                .first() {
                mem.reputation = value
                mem.interactionCount += 1
                try? await mem.save(on: db)
            } else {
                let mem = UserMemory(discordUserId: userId, guildId: guildId,
                                     facts: [], reputation: value, interactionCount: 1)
                try? await mem.save(on: db)
            }
        }
        dirty.removeAll()
    }
}
```
