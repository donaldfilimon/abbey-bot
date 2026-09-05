import AudioTapCore
import AudioTapRuntime
import Dispatch
import Foundation
import Network
import Testing

private final class FakeControl: @unchecked Sendable {
    private let lock = NSLock()
    private var created = 0
    private var stopped = 0
    private var current: FakeSource?
    let audio: Data?
    let failure: TapFailure?

    init(audio: Data? = nil, failure: TapFailure? = nil) { self.audio = audio; self.failure = failure }
    var counts: (created: Int, stopped: Int) { lock.withLock { (created, stopped) } }
    func make(_ queue: DispatchQueue) -> any AudioSource {
        let source = FakeSource(queue: queue, control: self)
        lock.withLock { created += 1; current = source }
        return source
    }
    func didStop() { lock.withLock { stopped += 1; current = nil } }
    func fail() { lock.withLock { current }?.injectFailure() }
    func flood() { lock.withLock { current }?.flood() }
}

private final class FakeSource: AudioSource, @unchecked Sendable {
    let queue: DispatchQueue
    let control: FakeControl
    private var pcm: (@Sendable (Data) -> Void)?
    private var failed: (@Sendable (TapFailure) -> Void)?
    init(queue: DispatchQueue, control: FakeControl) { self.queue = queue; self.control = control }
    func start(pcm: @escaping @Sendable (Data) -> Void, failed: @escaping @Sendable (TapFailure) -> Void) {
        self.pcm = pcm
        self.failed = failed
        if let failure = control.failure { failed(failure) }
        else if let audio = control.audio { pcm(audio) }
    }
    func stop(completion: @escaping @Sendable () -> Void) {
        pcm = nil
        failed = nil
        control.didStop()
        completion()
    }
    func injectFailure() { queue.async { [self] in failed?(.captureStopped) } }
    func flood() {
        queue.async { [self] in
            // Faster than the transport can acknowledge: deterministically exercises the
            // production bounded queue and its connection/source teardown, with fake PCM.
            for _ in 0..<8 { pcm?(Data(repeating: 1, count: PCM.maximumChunkBytes)) }
        }
    }
}

private enum ProbeError: Error { case failed, timeout }

private final class Probe: @unchecked Sendable {
    let connection: NWConnection
    let queue = DispatchQueue(label: "abbey-audio-tap.offline-client")

    init(port: UInt16) {
        connection = NWConnection(host: "127.0.0.1", port: NWEndpoint.Port(rawValue: port)!, using: .tcp)
        connection.start(queue: queue)
        // This is only a failed-test bound; tests never touch a production capture source.
        queue.asyncAfter(deadline: .now() + 8) { [weak self] in self?.connection.cancel() }
    }
    func send(_ text: String) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, any Error>) in
            connection.send(content: Data(text.utf8), completion: .contentProcessed { error in
                if let error { continuation.resume(throwing: error) }
                else { continuation.resume() }
            })
        }
    }
    func receive() async throws -> (Data, Bool) {
        try await withCheckedThrowingContinuation { continuation in
            connection.receive(minimumIncompleteLength: 1, maximumLength: 65_536) { data, _, complete, error in
                if let error { continuation.resume(throwing: error) }
                else { continuation.resume(returning: (data ?? Data(), complete)) }
            }
        }
    }
    func readToEOF() async throws -> Data {
        var data = Data()
        while true {
            let (part, complete) = try await receive()
            data.append(part)
            if complete { return data }
            guard data.count < 100_000 else { throw ProbeError.failed }
        }
    }
    func close() { connection.cancel() }
}

private func fixture(_ fake: FakeControl) async throws -> (LoopbackServer, UInt16) {
    try await fixture(factory: { fake.make($0) })
}

private func fixture(factory: @escaping SourceFactory) async throws -> (LoopbackServer, UInt16) {
    let server = try LoopbackServer(port: 0, factory: factory)
    let port = try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<UInt16, any Error>) in
        server.start(ready: { continuation.resume(returning: $0) }, failed: { continuation.resume(throwing: ProbeError.failed) })
    }
    #expect(port != 8182)
    return (server, port)
}

private final class DeferredControl: @unchecked Sendable {
    private let lock = NSLock()
    private weak var source: DeferredSource?
    private var queue: DispatchQueue?
    private var pending = false
    private var completed = false
    var alive: Bool { lock.withLock { source != nil } }
    var stopPending: Bool { lock.withLock { pending } }
    var shutdownCompleted: Bool { lock.withLock { completed } }
    func make(_ queue: DispatchQueue) -> any AudioSource {
        let source = DeferredSource(control: self)
        lock.withLock { self.source = source; self.queue = queue }
        return source
    }
    func didRequestStop() { lock.withLock { pending = true } }
    func didShutdown() { lock.withLock { completed = true } }
    func finish() {
        let (queue, source) = lock.withLock { (queue, source) }
        queue?.async { [weak source] in source?.finish() }
    }
}

private final class DeferredSource: AudioSource, @unchecked Sendable {
    let control: DeferredControl
    private var completion: (@Sendable () -> Void)?
    init(control: DeferredControl) { self.control = control }
    func start(pcm: @escaping @Sendable (Data) -> Void, failed: @escaping @Sendable (TapFailure) -> Void) {
        pcm(Data([1,2,3,4]))
    }
    func stop(completion: @escaping @Sendable () -> Void) {
        self.completion = completion
        control.didRequestStop()
    }
    func finish() {
        let callback = completion
        completion = nil
        callback?()
    }
}

private func stop(_ server: LoopbackServer) async {
    await withCheckedContinuation { continuation in server.stop { continuation.resume() } }
}

private func request(_ path: String, port: UInt16, headers: String = "") -> String {
    "GET \(path) HTTP/1.1\r\nHost: 127.0.0.1:\(port)\r\n\(headers)\r\n"
}

private func response(_ path: String, port: UInt16, headers: String = "") async throws -> Data {
    let client = Probe(port: port)
    defer { client.close() }
    try await client.send(request(path, port: port, headers: headers))
    return try await client.readToEOF()
}

private func waitUntil(_ condition: @escaping @Sendable () -> Bool) async throws {
    for _ in 0..<100 {
        if condition() { return }
        try await Task.sleep(for: .milliseconds(10))
    }
    throw ProbeError.timeout
}

@Test func healthNeverConstructsSourceAndBrowserCannotStartCapture() async throws {
    let fake = FakeControl()
    let (server, port) = try await fixture(fake)
    let health = String(decoding: try await response("/health", port: port), as: UTF8.self)
    #expect(health.hasPrefix("HTTP/1.1 200 OK\r\n"))
    #expect(health.contains("\"ready\":false"))
    #expect(health.contains("\"service\":\"abbey-audio-tap\""))
    let forbidden = String(decoding: try await response("/stream", port: port, headers: "Sec-Fetch-Site: cross-site\r\n"), as: UTF8.self)
    #expect(forbidden.hasPrefix("HTTP/1.1 403 Forbidden\r\n"))
    #expect(fake.counts.created == 0)
    await stop(server)
}

@Test func actualHTTPPreservesSyntheticPCMAndDisconnectStopsSource() async throws {
    let pcm = Data([0, 128, 255, 127, 0, 0, 0, 64])
    let fake = FakeControl(audio: pcm)
    let (server, port) = try await fixture(fake)
    let client = Probe(port: port)
    try await client.send(request("/stream", port: port))
    var received = Data()
    let expected = HTTP.streamHeader + HTTP.chunk(pcm)
    while received.count < expected.count {
        let (part, complete) = try await client.receive()
        received.append(part)
        #expect(!complete)
    }
    #expect(received == expected)
    let busy = String(decoding: try await response("/stream", port: port), as: UTF8.self)
    #expect(busy.hasPrefix("HTTP/1.1 409 Conflict\r\n"))
    #expect(fake.counts.created == 1)
    client.close()
    try await waitUntil { fake.counts.stopped == 1 }
    let health = String(decoding: try await response("/health", port: port), as: UTF8.self)
    #expect(health.contains("\"status\":\"idle\""))
    await stop(server)
}

@Test func captureDeathAbortsChunkedResponseAndClearsReadiness() async throws {
    let fake = FakeControl(audio: Data([1,2,3,4]))
    let (server, port) = try await fixture(fake)
    let client = Probe(port: port)
    try await client.send(request("/stream", port: port))
    let (first, _) = try await client.receive()
    #expect(first.starts(with: Data("HTTP/1.1 200 OK".utf8)))
    fake.fail()
    let remainder = try await client.readToEOF()
    #expect(!remainder.suffix(5).elementsEqual(Data("0\r\n\r\n".utf8)))
    #expect(fake.counts.stopped == 1)
    let health = String(decoding: try await response("/health", port: port), as: UTF8.self)
    #expect(health.contains("\"status\":\"failed\""))
    #expect(health.contains("\"error\":\"capture_stopped\""))
    await stop(server)
}

@Test func deniedPermissionDoesNotSendPCMHeaders() async throws {
    let fake = FakeControl(failure: .permissionRequired)
    let (server, port) = try await fixture(fake)
    let data = String(decoding: try await response("/stream", port: port), as: UTF8.self)
    #expect(data.hasPrefix("HTTP/1.1 503 Service Unavailable\r\n"))
    #expect(data.contains("permission_required"))
    #expect(!data.contains("Transfer-Encoding"))
    #expect(fake.counts.stopped == 1)
    await stop(server)
}

@Test func boundedSourceFloodStopsCaptureAndConnection() async throws {
    let fake = FakeControl(audio: Data([1,2,3,4]))
    let (server, port) = try await fixture(fake)
    let client = Probe(port: port)
    try await client.send(request("/stream", port: port))
    _ = try await client.receive()
    fake.flood()
    // Cancellation can be observed as EOF or a TCP reset after pending writes.
    _ = try? await client.readToEOF()
    try await waitUntil { fake.counts.stopped == 1 }
    let health = String(decoding: try await response("/health", port: port), as: UTF8.self)
    #expect(health.contains("\"error\":\"slow_consumer\""))
    await stop(server)
}

@Test func missingFirstFrameTriggersStartupWatchdog() async throws {
    let fake = FakeControl()
    let (server, port) = try await fixture(fake)
    let data = String(decoding: try await response("/stream", port: port), as: UTF8.self)
    #expect(data.hasPrefix("HTTP/1.1 503 Service Unavailable\r\n"))
    #expect(data.contains("startup_timeout"))
    #expect(fake.counts.stopped == 1)
    await stop(server)
}

@Test func deferredStopRetainsSourceAndBlocksReconnectUntilCompletion() async throws {
    let control = DeferredControl()
    let (server, port) = try await fixture(factory: { control.make($0) })
    let client = Probe(port: port)
    try await client.send(request("/stream", port: port))
    _ = try await client.receive()
    client.close()
    try await waitUntil { control.stopPending }
    #expect(control.alive)
    let busy = String(decoding: try await response("/stream", port: port), as: UTF8.self)
    #expect(busy.hasPrefix("HTTP/1.1 409 Conflict\r\n"))
    control.finish()
    try await waitUntil { !control.alive }
    await stop(server)
}

@Test func serverShutdownRetainsSourceAndAwaitsNativeStop() async throws {
    let control = DeferredControl()
    let (server, port) = try await fixture(factory: { control.make($0) })
    let client = Probe(port: port)
    try await client.send(request("/stream", port: port))
    _ = try await client.receive()
    server.stop { control.didShutdown() }
    try await waitUntil { control.stopPending }
    #expect(control.alive)
    #expect(!control.shutdownCompleted)
    control.finish()
    try await waitUntil { control.shutdownCompleted && !control.alive }
    client.close()
}
