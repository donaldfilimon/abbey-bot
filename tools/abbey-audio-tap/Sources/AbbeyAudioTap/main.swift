import AudioTapRuntime
import CoreGraphics
import Darwin
import Dispatch
import Foundation

let usage = """
abbey-audio-tap 1
Usage: abbey-audio-tap serve | request-permission | --help | --version

serve binds HTTP only to 127.0.0.1:8182.
GET /health reports service state without capture or a permission prompt.
GET /stream starts a single 48 kHz stereo s16le stream after permission preflight.
Discord, browsers, Abbey and unidentified applications are excluded.
No microphone, screen output, audio files, providers or Discord connection.
Screen & System Audio Recording permission must already be granted by the operator.
request-permission asks macOS for that permission; it starts no listener or capture.
"""

switch Array(CommandLine.arguments.dropFirst()) {
case ["--help"], ["-h"]: print(usage)
case ["--version"]: print("abbey-audio-tap 1")
case ["request-permission"]:
    if CGRequestScreenCaptureAccess() {
        print("abbey-audio-tap: screen capture permission granted; live audio remains unverified")
    } else {
        print("abbey-audio-tap: permission pending; review Screen & System Audio Recording in System Settings, then restart the service")
        exit(EXIT_FAILURE)
    }
case ["serve"]:
    do {
        let server = try LoopbackServer()
        signal(SIGINT, SIG_IGN)
        signal(SIGTERM, SIG_IGN)
        let signals = [SIGINT, SIGTERM].map { number in
            let signal = DispatchSource.makeSignalSource(signal: number, queue: .main)
            signal.setEventHandler { server.stop { exit(EXIT_SUCCESS) } }
            signal.resume()
            return signal
        }
        server.start(ready: { _ in }, failed: {
            // Fixed, content-free diagnostics only, never underlying framework errors.
            FileHandle.standardError.write(Data("abbey-audio-tap: loopback listener unavailable\n".utf8))
            exit(EXIT_FAILURE)
        })
        withExtendedLifetime(signals) { dispatchMain() }
    } catch {
        FileHandle.standardError.write(Data("abbey-audio-tap: loopback listener unavailable\n".utf8))
        exit(EXIT_FAILURE)
    }
default:
    FileHandle.standardError.write(Data((usage + "\n").utf8))
    exit(2)
}
