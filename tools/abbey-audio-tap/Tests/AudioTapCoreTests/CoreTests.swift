import Foundation
import Testing
@testable import AudioTapCore

private func app(_ bundle: String = "com.apple.Music", name: String = "Music", pid: Int32 = 42,
                 executable: String = "/Applications/Music.app/Contents/MacOS/Music", launched: Double = 1) -> ApplicationIdentity {
    ApplicationIdentity(pid: pid, bundleID: bundle, name: name, executable: executable, launchTime: launched)
}

@Test func permittedMusicAndSpotify() {
    #expect(ApplicationPolicy.permits(app(), ownPID: 9))
    #expect(ApplicationPolicy.permits(app("com.spotify.client", name: "Spotify"), ownPID: 9))
}

@Test(arguments: ["com.hnc.Discord", "com.hnc.DiscordPTB", "com.hnc.DiscordCanary.helper",
                  "com.hnc.DiscordDevelopment", "dev.vencord.Vesktop", "com.legcord.Legcord",
                  "com.apple.Safari", "com.google.Chrome.helper", "org.mozilla.firefox",
                  "company.thebrowser.Browser.helper", "com.openai.atlas.helper", "ai.perplexity.comet.helper",
                  "app.zen-browser.zen", "com.microsoft.edgemac.helper", "com.donaldfilimon.abbey-bot",
                  "com.apple.Terminal", "com.googlecode.iterm2", "com.mitchellh.ghostty", "dev.warp.Warp-Stable"])
func forbiddenApplicationBundles(bundle: String) {
    #expect(!ApplicationPolicy.permits(app(bundle, name: "Helper"), ownPID: 9))
}

@Test func processAncestryAndUnknownIdentityFailClosed() {
    #expect(!ApplicationPolicy.permits(app("vendor.helper", name: "Helper", executable: "/Applications/Discord Canary.app/Contents/Frameworks/Helper"), ownPID: 9))
    #expect(!ApplicationPolicy.permits(app("vendor.helper", name: "Atlas Helper", executable: "/Applications/Atlas.app/Contents/Frameworks/Helper"), ownPID: 9))
    #expect(!ApplicationPolicy.permits(app(""), ownPID: 9))
    #expect(!ApplicationPolicy.permits(app(launched: .nan), ownPID: 9))
    #expect(!ApplicationPolicy.permits(app(), ownPID: 42))
}

@Test func applicationIdentityChangeNeverAdmitsReusedPID() {
    let initial = [app()]
    #expect(ApplicationPolicy.identitiesStillMatch(initial) { _ in app() })
    #expect(!ApplicationPolicy.identitiesStillMatch(initial) { _ in app(launched: 2) })
    #expect(!ApplicationPolicy.identitiesStillMatch(initial) { _ in app("com.hnc.Discord") })
    #expect(!ApplicationPolicy.identitiesStillMatch(initial) { _ in nil })
    // A newly launched process is absent from the selected filter and is never added.
    #expect(!ApplicationPolicy.identitiesStillMatch([]) { _ in app() })
}

@Test func pcmInterleavesClampsAndUsesLittleEndian() throws {
    let bytes = try PCM.encode(left: [-1, 0, 1, 0.5], right: [1, -0.5, -2, 0])
    #expect(Array(bytes) == [0,128,255,127, 0,0,0,192, 255,127,0,128, 0,64,0,0])
}

@Test func pcmRejectsMalformedSource() {
    #expect(throws: TapFailure.invalidAudio) { try PCM.encode(left: [.nan], right: [0]) }
    #expect(throws: TapFailure.invalidAudio) { try PCM.encode(left: [0], right: []) }
    #expect(throws: TapFailure.invalidAudio) { try PCM.encode(left: [], right: []) }
    #expect(throws: TapFailure.invalidAudio) {
        try PCM.encode(left: Array(repeating: 0, count: 4_801), right: Array(repeating: 0, count: 4_801))
    }
}

@Test func captureTimestampsRejectOldDuplicatedAndFutureSamples() {
    #expect(PCM.fresh(presentationTime: 10, now: 10.1, previous: 9.9))
    #expect(!PCM.fresh(presentationTime: 10, now: 10.3, previous: nil))
    #expect(!PCM.fresh(presentationTime: 10, now: 10.1, previous: 10))
    #expect(!PCM.fresh(presentationTime: 10, now: 9.9, previous: nil))
    #expect(!PCM.fresh(presentationTime: .nan, now: 10, previous: nil))
}

private func request(_ path: String, headers: String = "") -> Data {
    Data("GET \(path) HTTP/1.1\r\nHost: 127.0.0.1:8182\r\n\(headers)\r\n".utf8)
}

@Test func httpRoutesAndChunkFraming() {
    #expect(HTTP.route(request("/health")) == .health)
    #expect(HTTP.route(request("/stream")) == .stream)
    #expect(HTTP.route(request("/other")) == .reject(404))
    #expect(HTTP.chunk(Data([0,1,2,3])) == Data([52,13,10,0,1,2,3,13,10]))
}

@Test(arguments: ["Origin: https://example.test\r\n", "Sec-Fetch-Site: cross-site\r\n",
                  "sec-fetch-mode: no-cors\r\n", "Content-Length: 0\r\n", "Transfer-Encoding: chunked\r\n",
                  "Upgrade: websocket\r\n"])
func browserAndBodyRequestsRejected(headers: String) {
    #expect(HTTP.route(request("/stream", headers: headers)) == .reject(403))
}

@Test func hostRebindingAndMalformedHTTPFailClosed() {
    #expect(HTTP.route(Data("GET /stream HTTP/1.1\r\nHost: evil.test:8182\r\n\r\n".utf8)) == .reject(403))
    #expect(HTTP.route(request("/stream", headers: "Host: 127.0.0.1:8182\r\n")) == .reject(400))
    #expect(HTTP.route(request("/stream") + request("/health")) == .reject(400))
    #expect(HTTP.route(Data(repeating: 65, count: 4_097)) == .reject(400))
}

@Test func noAudioReplayAcrossConsumerOrFailure() throws {
    var state = StreamState()
    let observed82 = (state.begin(now: 0))
    let old = try #require(observed82)
    let observed83 = (state.append(Data([1,2,3,4]), token: old, now: 1))
    #expect(observed83)
    state.disconnect()
    let observed85 = (state.begin(now: 2))
    let new = try #require(observed85)
    #expect(new != old)
    let observed87 = (state.next(now: 3) == nil)
    #expect(observed87)
    let observed88 = (!state.append(Data([1,2,3,4]), token: old, now: 3))
    #expect(observed88)
    let observed89 = (state.append(Data([5,6,7,8]), token: new, now: 4))
    #expect(observed89)
    state.fail(.captureStopped)
    #expect(state.bufferedBytes == 0)
    let observed92 = (state.next(now: 5) == nil)
    #expect(observed92)
    let observed93 = (!state.append(Data([5,6,7,8]), token: new, now: 6))
    #expect(observed93)
}

@Test func streamBoundIncludesInflightAndClosesSlowReader() throws {
    var state = StreamState()
    let observed98 = (state.begin(now: 0))
    let token = try #require(observed98)
    let bytes = Data(repeating: 0, count: PCM.maximumChunkBytes)
    let observed100 = (state.append(bytes, token: token, now: 1))
    #expect(observed100)
    let observed101 = (state.next(now: 1) == bytes)
    #expect(observed101)
    #expect(state.bufferedBytes == bytes.count)
    let observed103 = (state.append(bytes, token: token, now: 2))
    #expect(observed103)
    let observed104 = (!state.append(bytes, token: token, now: 3))
    #expect(observed104)
    #expect(state.failure == .slowConsumer)
    #expect(state.bufferedBytes == 0)
}

@Test func writeAndQueuedAgeDeadlinesDiscardAudio() throws {
    var state = StreamState()
    let observed111 = (state.begin(now: 0))
    let token = try #require(observed111)
    let observed112 = (state.append(Data([1,2,3,4]), token: token, now: 1))
    #expect(observed112)
    let observed113 = (state.next(now: 2) != nil)
    #expect(observed113)
    state.tick(now: StreamState.maximumLatency + 3)
    #expect(state.failure == .slowConsumer)
    let observed116 = (state.begin(now: 1_000_000_000))
    let retry = try #require(observed116)
    let observed117 = (state.append(Data([1,2,3,4]), token: retry, now: 1_000_000_001))
    #expect(observed117)
    let observed118 = (state.next(now: 1_000_000_002 + StreamState.maximumLatency) == nil)
    #expect(observed118)
    #expect(state.failure == .slowConsumer)
}

@Test func startupAndCaptureWatchdogsFailClosed() throws {
    var state = StreamState()
    let observed124 = (state.begin(now: 0))
    _ = try #require(observed124)
    let observed125 = (state.begin(now: 1) == nil)
    #expect(observed125)
    state.tick(now: StreamState.startupLimit)
    #expect(state.failure == .startupTimeout)
    let observed128 = (state.begin(now: 0))
    let token = try #require(observed128)
    let observed129 = (state.append(Data([1,2,3,4]), token: token, now: 1))
    #expect(observed129)
    _ = state.next(now: 1)
    state.acknowledge(bytes: 4, token: token)
    state.tick(now: StreamState.stallLimit + 1)
    #expect(state.failure == .captureStalled)
}

@Test func healthIsStableAndContainsNoMedia() throws {
    let json = try #require(JSONSerialization.jsonObject(with: StreamState().health()) as? [String: Any])
    #expect(json["service"] as? String == "abbey-audio-tap")
    #expect(json["protocol_version"] as? Int == 1)
    #expect(json["status"] as? String == "idle")
    #expect(json["ready"] as? Bool == false)
    #expect(Set(json.keys) == ["service", "protocol_version", "status", "ready", "audio", "stream_path", "error"])
}
