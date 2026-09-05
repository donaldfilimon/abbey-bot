@preconcurrency import AppKit
import AudioTapCore
import CoreGraphics
import CoreMedia
import Dispatch
import Foundation
@preconcurrency import ScreenCaptureKit

/// All state and audio work run on the server's serial queue. Only framework delegate
/// failure callbacks hop onto that queue; PCM is processed synchronously, without tasks.
public final class ScreenCaptureSource: NSObject, AudioSource, SCStreamOutput, SCStreamDelegate, @unchecked Sendable {
    private let queue: DispatchQueue
    private var stream: SCStream?
    private var active = false
    private var starting = false
    private var stopping = false
    private var stopCallbacks: [@Sendable () -> Void] = []
    private var selected: [ApplicationIdentity] = []
    private var previousPresentationTime: Double?
    private var pcm: (@Sendable (Data) -> Void)?
    private var failed: (@Sendable (TapFailure) -> Void)?

    public init(queue: DispatchQueue) { self.queue = queue }

    public func start(pcm: @escaping @Sendable (Data) -> Void, failed: @escaping @Sendable (TapFailure) -> Void) {
        self.pcm = pcm
        self.failed = failed
        // Never request permission from the service or an HTTP request. No TCC API is
        // called at all by /health; even this preflight only runs for a stream request.
        guard CGPreflightScreenCaptureAccess() else { failed(.permissionRequired); return }
        active = true
        SCShareableContent.getExcludingDesktopWindows(false, onScreenWindowsOnly: false) { [weak self] content, error in
            guard let self else { return }
            self.queue.async {
                guard self.active else { return }
                guard error == nil, let content else { self.report(.captureUnavailable); return }
                self.configure(content)
            }
        }
    }

    private static func identity(_ pid: Int32) -> ApplicationIdentity? {
        guard let app = NSRunningApplication(processIdentifier: pid), !app.isTerminated,
              let bundle = app.bundleIdentifier, let executable = app.executableURL,
              let launched = app.launchDate else { return nil }
        return ApplicationIdentity(pid: pid, bundleID: bundle, name: app.localizedName ?? "",
                                   executable: executable.path, launchTime: launched.timeIntervalSince1970)
    }

    private func configure(_ content: SCShareableContent) {
        guard let display = content.displays.min(by: { $0.displayID < $1.displayID }) else {
            report(.captureUnavailable); return
        }
        var applications: [SCRunningApplication] = []
        var identities: [ApplicationIdentity] = []
        for app in content.applications {
            guard let identity = Self.identity(app.processID), identity.bundleID == app.bundleIdentifier,
                  ApplicationPolicy.permits(identity, ownPID: ProcessInfo.processInfo.processIdentifier) else { continue }
            applications.append(app)
            identities.append(identity)
        }
        guard !applications.isEmpty,
              ApplicationPolicy.identitiesStillMatch(identities, current: Self.identity) else {
            report(.noEligibleApplications); return
        }
        selected = identities
        // A deny-list snapshot would capture a newly launched Discord process until
        // refresh. An inclusion filter admits no newly discovered application at all.
        let filter = SCContentFilter(display: display, including: applications, exceptingWindows: [])
        let config = SCStreamConfiguration()
        config.capturesAudio = true
        config.excludesCurrentProcessAudio = true
        config.sampleRate = PCM.sampleRate
        config.channelCount = PCM.channels
        if #available(macOS 15.0, *) { config.captureMicrophone = false }
        config.width = 2
        config.height = 2
        config.minimumFrameInterval = CMTime(seconds: 1, preferredTimescale: 1)
        config.queueDepth = 3
        config.showsCursor = false
        let stream = SCStream(filter: filter, configuration: config, delegate: self)
        do {
            // There is no screen, microphone, recording, or screenshot output.
            try stream.addStreamOutput(self, type: .audio, sampleHandlerQueue: queue)
        } catch { report(.captureUnavailable); return }
        self.stream = stream
        starting = true
        stream.startCapture { [weak self] error in
            guard let self else { return }
            self.queue.async {
                self.starting = false
                if error != nil {
                    self.stream = nil
                    if self.active { self.report(.captureUnavailable) }
                    self.finishStop()
                } else if !self.active { self.stopStream() }
            }
        }
    }

    public func stop(completion: @escaping @Sendable () -> Void) {
        active = false
        pcm = nil
        failed = nil
        selected.removeAll(keepingCapacity: false)
        previousPresentationTime = nil
        stopCallbacks.append(completion)
        if !starting { stopStream() }
    }

    private func stopStream() {
        guard !stopping else { return }
        guard let stream else { finishStop(); return }
        stopping = true
        stream.stopCapture { [self] error in
            queue.async {
                // If stopping failed, keep the source closed and retain the stop gate.
                // No subsequent consumer may start another capture in this process.
                guard error == nil else { return }
                self.stream = nil
                self.stopping = false
                self.finishStop()
            }
        }
    }

    private func finishStop() {
        let callbacks = stopCallbacks
        stopCallbacks.removeAll()
        for callback in callbacks { callback() }
    }

    private func report(_ error: TapFailure) {
        guard active else { return }
        let callback = failed
        active = false
        pcm = nil
        failed = nil
        callback?(error)
    }

    public func stream(_ stream: SCStream, didStopWithError error: any Error) {
        queue.async { [weak self] in
            guard let self, self.stream === stream else { return }
            self.stream = nil
            self.starting = false
            self.stopping = false
            if self.active { self.report(.captureStopped) }
            self.finishStop()
        }
    }

    public func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard active, self.stream === stream, type == .audio else { return }
        // Do not trust stale PIDs. NSRunningApplication provides native process identity,
        // without a subprocess, whole-machine process scan, or per-buffer async backlog.
        guard ApplicationPolicy.identitiesStillMatch(selected, current: Self.identity) else {
            report(.applicationChanged); return
        }
        guard let clock = stream.synchronizationClock else { report(.staleAudio); return }
        let timestamp = CMSampleBufferGetPresentationTimeStamp(sampleBuffer).seconds
        guard PCM.fresh(presentationTime: timestamp, now: CMClockGetTime(clock).seconds,
                        previous: previousPresentationTime) else { report(.staleAudio); return }
        previousPresentationTime = timestamp
        do { pcm?(try Self.convert(sampleBuffer)) }
        catch { report(.invalidAudio) }
    }

    static func convert(_ sample: CMSampleBuffer) throws -> Data {
        guard sample.isValid, CMSampleBufferDataIsReady(sample),
              let description = CMSampleBufferGetFormatDescription(sample),
              let format = CMAudioFormatDescriptionGetStreamBasicDescription(description)?.pointee,
              format.mFormatID == kAudioFormatLinearPCM,
              format.mSampleRate == Double(PCM.sampleRate), format.mChannelsPerFrame == UInt32(PCM.channels),
              format.mFormatFlags & kAudioFormatFlagIsBigEndian == 0 else { throw TapFailure.invalidAudio }
        let frames = CMSampleBufferGetNumSamples(sample)
        guard frames > 0, frames <= PCM.maximumFrames else { throw TapFailure.invalidAudio }
        let planar = format.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0
        let float = format.mFormatFlags & kAudioFormatFlagIsFloat != 0
        let signed = format.mFormatFlags & kAudioFormatFlagIsSignedInteger != 0
        guard (float && format.mBitsPerChannel == 32) || (!float && signed && format.mBitsPerChannel == 16) else {
            throw TapFailure.invalidAudio
        }
        let sampleBytes = Int(format.mBitsPerChannel / 8)
        let stride = sampleBytes * (planar ? 1 : 2)
        guard format.mBytesPerFrame == stride else { throw TapFailure.invalidAudio }
        let buffers = AudioBufferList.allocate(maximumBuffers: planar ? 2 : 1)
        defer { buffers.unsafeMutablePointer.deallocate() }
        var retainedBlock: CMBlockBuffer?
        let status = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
            sample, bufferListSizeNeededOut: nil,
            bufferListOut: buffers.unsafeMutablePointer,
            bufferListSize: MemoryLayout<AudioBufferList>.size + (planar ? MemoryLayout<AudioBuffer>.stride : 0),
            blockBufferAllocator: nil, blockBufferMemoryAllocator: nil,
            flags: UInt32(kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment),
            blockBufferOut: &retainedBlock
        )
        guard status == noErr, buffers.count == (planar ? 2 : 1) else { throw TapFailure.invalidAudio }
        // Keep the backing CMBlockBuffer alive through every pointer read.
        return try withExtendedLifetime(retainedBlock) {
            var channels = [[Float](), [Float]()]
            for channel in 0..<2 {
                let buffer = buffers[planar ? channel : 0]
                guard buffer.mNumberChannels == (planar ? 1 : 2), let pointer = buffer.mData,
                      Int(buffer.mDataByteSize) == frames * stride else { throw TapFailure.invalidAudio }
                channels[channel].reserveCapacity(frames)
                for frame in 0..<frames {
                    let offset = frame * stride + (planar ? 0 : channel * sampleBytes)
                    let value: Float
                    if float { value = pointer.loadUnaligned(fromByteOffset: offset, as: Float.self) }
                    else { value = Float(pointer.loadUnaligned(fromByteOffset: offset, as: Int16.self)) / 32768 }
                    channels[channel].append(value)
                }
            }
            return try PCM.encode(left: channels[0], right: channels[1])
        }
    }
}
