import Foundation

/// Queue-confined state. The shell supplies monotonic time; this type performs no I/O.
public struct StreamState {
    public enum Status: String { case idle, starting, capturing, failed }
    public private(set) var status: Status = .idle
    public private(set) var failure: TapFailure?
    public private(set) var generation: UInt64 = 0
    public private(set) var bufferedBytes = 0
    private var pending: [(data: Data, time: UInt64)] = []
    private var started: UInt64 = 0
    private var lastAudio: UInt64 = 0
    private var sendingSince: UInt64?
    public static let maximumBufferedBytes = 48_000 // 250 ms, including the in-flight write.
    public static let maximumLatency: UInt64 = 250_000_000
    public static let startupLimit: UInt64 = 5_000_000_000
    public static let stallLimit: UInt64 = 2_000_000_000

    public init() {}

    public mutating func begin(now: UInt64) -> UInt64? {
        guard status == .idle || status == .failed else { return nil }
        clear()
        generation &+= 1
        status = .starting
        failure = nil
        started = now
        lastAudio = now
        return generation
    }

    @discardableResult
    public mutating func append(_ data: Data, token: UInt64, now: UInt64) -> Bool {
        guard token == generation, status == .starting || status == .capturing else { return false }
        guard PCM.valid(data) else { fail(.invalidAudio); return false }
        guard bufferedBytes + data.count <= Self.maximumBufferedBytes else {
            fail(.slowConsumer); return false
        }
        status = .capturing
        lastAudio = now
        pending.append((data, now))
        bufferedBytes += data.count
        return true
    }

    public mutating func next(now: UInt64) -> Data? {
        guard status == .capturing, sendingSince == nil, !pending.isEmpty else { return nil }
        guard now >= pending[0].time, now - pending[0].time <= Self.maximumLatency else {
            fail(.slowConsumer); return nil
        }
        let next = pending.removeFirst()
        sendingSince = now
        return next.data
    }

    public mutating func acknowledge(bytes: Int, token: UInt64) {
        guard token == generation, status == .capturing, sendingSince != nil else { return }
        bufferedBytes -= bytes
        sendingSince = nil
    }

    public mutating func tick(now: UInt64) {
        if status == .starting, now >= started, now - started >= Self.startupLimit { fail(.startupTimeout) }
        if status == .capturing, now >= lastAudio, now - lastAudio >= Self.stallLimit { fail(.captureStalled) }
        if let sent = sendingSince, now >= sent, now - sent > Self.maximumLatency { fail(.slowConsumer) }
        if let first = pending.first, now >= first.time, now - first.time > Self.maximumLatency { fail(.slowConsumer) }
    }

    public mutating func fail(_ reason: TapFailure) {
        status = .failed
        failure = reason
        clear()
    }

    public mutating func disconnect() {
        generation &+= 1
        status = .idle
        failure = nil
        clear()
    }

    private mutating func clear() {
        pending.removeAll(keepingCapacity: false)
        bufferedBytes = 0
        sendingSince = nil
    }

    public func health() -> Data {
        // Fixed schema and redacted reason codes; never include app names, PIDs or content.
        let reason = failure.map { "\"\($0.rawValue)\"" } ?? "null"
        return Data("{\"service\":\"abbey-audio-tap\",\"protocol_version\":1,\"status\":\"\(status.rawValue)\",\"ready\":\(status == .capturing),\"audio\":{\"sample_rate\":48000,\"channels\":2,\"format\":\"s16le\"},\"stream_path\":\"/stream\",\"error\":\(reason)}\n".utf8)
    }
}
