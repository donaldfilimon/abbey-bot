import AudioTapCore
import Dispatch
import Foundation
import Network

/// All mutable fields are confined to queue, including source and Network callbacks.
public final class LoopbackServer: @unchecked Sendable {
    private let queue = DispatchQueue(label: "com.donaldfilimon.abbey-audio-tap.http")
    private let factory: SourceFactory
    private let listener: NWListener
    private var clients: [UUID: Client] = [:]
    private var session: Client?
    private var source: (any AudioSource)?
    private var stoppingSource: (any AudioSource)?
    private var stopWaiters: [@Sendable () -> Void] = []
    private var stopping = false
    private var state = StreamState()
    private var timer: DispatchSourceTimer?
    private var authority = HTTP.authority
    private var stopped = false

    // Port injection exists for offline tests only. The executable always uses 8182.
    public init(port: UInt16 = 8182, factory: @escaping SourceFactory = { ScreenCaptureSource(queue: $0) }) throws {
        self.factory = factory
        let tcp = NWProtocolTCP.Options()
        tcp.noDelay = true
        let parameters = NWParameters(tls: nil, tcp: tcp)
        parameters.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: NWEndpoint.Port(rawValue: port)!)
        parameters.allowLocalEndpointReuse = false
        listener = try NWListener(using: parameters)
    }

    public func start(ready: @escaping @Sendable (UInt16) -> Void, failed: @escaping @Sendable () -> Void) {
        queue.async { [self] in
            listener.stateUpdateHandler = { [weak self] status in
                guard let self else { return }
                switch status {
                case .ready:
                    guard let port = self.listener.port else { failed(); return }
                    self.authority = "127.0.0.1:\(port.rawValue)"
                    ready(port.rawValue)
                case .failed: failed()
                default: break
                }
            }
            listener.newConnectionHandler = { [weak self] connection in self?.accept(connection) }
            listener.start(queue: queue)
            let timer = DispatchSource.makeTimerSource(queue: queue)
            timer.schedule(deadline: .now() + .milliseconds(100), repeating: .milliseconds(100))
            timer.setEventHandler { [weak self] in self?.tick() }
            self.timer = timer
            timer.resume()
        }
    }

    public func stop(completion: @escaping @Sendable () -> Void) {
        queue.async { [self] in
            stopped = true
            listener.cancel()
            timer?.cancel()
            timer = nil
            for client in Array(clients.values) { client.close() }
            clients.removeAll()
            session = nil
            state.disconnect()
            stopWaiters.append(completion)
            // Shutdown shares the same retained, asynchronous stop as disconnect.
            // In particular, a source still starting must outlive this queue block.
            if stoppingSource != nil { return }
            if source != nil { stopSource() }
            else { finishStopWaiters() }
        }
    }

    private var now: UInt64 { DispatchTime.now().uptimeNanoseconds }

    private func accept(_ connection: NWConnection) {
        guard !stopped, clients.count < 16 else { connection.cancel(); return }
        let client = Client(connection: connection, queue: queue, server: self, created: now)
        clients[client.id] = client
        client.start()
    }

    fileprivate func request(_ data: Data, client: Client) {
        switch HTTP.route(data, authority: authority) {
        case .health: client.reply(status: 200, body: state.health())
        case .reject(let status): client.reply(status: status)
        case .stream:
            guard session == nil, !stopping else { client.reply(status: 409); return }
            guard let token = state.begin(now: now) else { client.reply(status: 409); return }
            session = client
            client.streaming = true
            let source = factory(queue)
            self.source = source
            source.start(pcm: { [weak self, weak client] data in
                guard let self, let client, self.session === client,
                      self.state.generation == token else { return }
                if self.state.append(data, token: token, now: self.now) { self.drain(client, token: token) }
                else if self.state.status == .failed { self.failSession() }
            }, failed: { [weak self, weak client] failure in
                guard let self, let client, self.session === client,
                      self.state.generation == token else { return }
                self.state.fail(failure)
                self.failSession()
            })
        }
    }

    private func drain(_ client: Client, token: UInt64) {
        guard session === client else { return }
        guard let data = state.next(now: now) else {
            if state.status == .failed { failSession() }
            return
        }
        var packet = Data()
        if !client.sentHeader { packet.append(HTTP.streamHeader); client.sentHeader = true }
        packet.append(HTTP.chunk(data))
        client.send(packet) { [weak self, weak client] okay in
            guard let self, let client, self.session === client, self.state.generation == token else { return }
            guard okay else { self.disconnected(client); return }
            self.state.acknowledge(bytes: data.count, token: token)
            self.drain(client, token: token)
        }
    }

    private func failSession() {
        guard let client = session else { return }
        session = nil
        if client.sentHeader { client.close() }
        else { client.reply(status: 503, body: state.health()) }
        stopSource()
    }

    private func stopSource() {
        guard let source else { return }
        self.source = nil
        stopping = true
        stoppingSource = source
        source.stop { [self] in
            // Retain the server and its stopping source until capture confirms stop.
            stopping = false
            stoppingSource = nil
            finishStopWaiters()
        }
    }

    private func finishStopWaiters() {
        let waiters = stopWaiters
        stopWaiters.removeAll()
        for waiter in waiters { waiter() }
    }

    fileprivate func disconnected(_ client: Client) {
        clients.removeValue(forKey: client.id)
        client.close()
        guard session === client else { return }
        session = nil
        state.disconnect()
        stopSource()
    }

    private func tick() {
        for client in Array(clients.values) where !client.streaming && now - client.created > 5_000_000_000 {
            client.close()
            clients.removeValue(forKey: client.id)
        }
        state.tick(now: now)
        if state.status == .failed { failSession() }
    }
}

/// Queue confinement matches the owning server. There is one outstanding receive and send.
private final class Client: @unchecked Sendable {
    let id = UUID()
    let created: UInt64
    var streaming = false
    var sentHeader = false
    private let connection: NWConnection
    private let queue: DispatchQueue
    private weak var server: LoopbackServer?
    private var request = Data()
    private var handled = false
    private var closed = false

    init(connection: NWConnection, queue: DispatchQueue, server: LoopbackServer, created: UInt64) {
        self.connection = connection
        self.queue = queue
        self.server = server
        self.created = created
    }

    func start() {
        connection.stateUpdateHandler = { [weak self] state in
            guard let self else { return }
            switch state {
            case .ready: self.receive()
            case .failed, .cancelled: self.server?.disconnected(self)
            default: break
            }
        }
        connection.start(queue: queue)
    }

    private func receive() {
        guard !closed else { return }
        connection.receive(minimumIncompleteLength: 1, maximumLength: HTTP.maximumRequestBytes + 1) { [weak self] data, _, complete, error in
            guard let self, !self.closed else { return }
            if let data, !data.isEmpty {
                if self.handled { self.server?.disconnected(self); return }
                self.request.append(data)
                if self.request.count > HTTP.maximumRequestBytes { self.reply(status: 431); return }
                if self.request.range(of: Data("\r\n\r\n".utf8)) != nil {
                    self.handled = true
                    self.server?.request(self.request, client: self)
                    self.request.removeAll(keepingCapacity: false)
                }
            }
            if complete || error != nil { self.server?.disconnected(self); return }
            self.receive() // Detect a peer leaving even when the audio source is silent.
        }
    }

    func send(_ data: Data, completion: @escaping @Sendable (Bool) -> Void) {
        guard !closed else { completion(false); return }
        connection.send(content: data, completion: .contentProcessed { error in completion(error == nil) })
    }

    func reply(status: Int, body: Data = Data("{}\n".utf8)) {
        send(HTTP.response(status: status, body: body)) { [weak self] _ in
            guard let self else { return }
            self.server?.disconnected(self)
        }
    }

    func close() {
        guard !closed else { return }
        closed = true
        connection.cancel()
    }
}
