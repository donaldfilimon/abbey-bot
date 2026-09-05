import Foundation

public enum PCM {
    public static let sampleRate = 48_000
    public static let channels = 2
    public static let bytesPerFrame = 4
    public static let maximumFrames = 4_800 // 100 ms per callback.
    public static let maximumChunkBytes = maximumFrames * bytesPerFrame

    public static func fresh(presentationTime: Double, now: Double, previous: Double?) -> Bool {
        presentationTime.isFinite && now.isFinite && presentationTime >= 0 &&
            now >= presentationTime && now - presentationTime <= 0.25 &&
            (previous.map { presentationTime > $0 } ?? true)
    }

    public static func valid(_ bytes: Data) -> Bool {
        !bytes.isEmpty && bytes.count <= maximumChunkBytes && bytes.count % bytesPerFrame == 0
    }

    /// No resampler guesses: the capture adapter requests and validates 48 kHz stereo.
    public static func encode(left: [Float], right: [Float]) throws -> Data {
        guard !left.isEmpty, left.count == right.count, left.count <= maximumFrames else {
            throw TapFailure.invalidAudio
        }
        var result = Data(capacity: left.count * bytesPerFrame)
        for index in left.indices {
            for sample in [left[index], right[index]] {
                guard sample.isFinite else { throw TapFailure.invalidAudio }
                let clamped = min(1, max(-1, sample))
                let value = Int16(clamping: Int((clamped * 32768).rounded()))
                let bits = UInt16(bitPattern: value)
                result.append(UInt8(truncatingIfNeeded: bits))
                result.append(UInt8(truncatingIfNeeded: bits >> 8))
            }
        }
        return result
    }
}

public enum TapFailure: String, Error, Sendable {
    case permissionRequired = "permission_required"
    case noEligibleApplications = "no_eligible_applications"
    case captureUnavailable = "capture_unavailable"
    case captureStopped = "capture_stopped"
    case applicationChanged = "application_changed"
    case invalidAudio = "invalid_audio"
    case staleAudio = "stale_audio"
    case startupTimeout = "startup_timeout"
    case captureStalled = "capture_stalled"
    case slowConsumer = "slow_consumer"
}
