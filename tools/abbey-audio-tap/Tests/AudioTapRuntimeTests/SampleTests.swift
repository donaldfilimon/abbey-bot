import AudioTapCore
import CoreMedia
import Foundation
import Testing
@testable import AudioTapRuntime

private func sample(bytes: Data, frames: Int, planar: Bool, float: Bool, sampleRate: Double = 48_000) throws -> CMSampleBuffer {
    let sampleBytes = float ? 4 : 2
    let stride = sampleBytes * (planar ? 1 : 2)
    var format = AudioStreamBasicDescription(
        mSampleRate: sampleRate, mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: (float ? kAudioFormatFlagIsFloat : kAudioFormatFlagIsSignedInteger) |
            kAudioFormatFlagIsPacked | (planar ? kAudioFormatFlagIsNonInterleaved : 0),
        mBytesPerPacket: UInt32(stride), mFramesPerPacket: 1, mBytesPerFrame: UInt32(stride),
        mChannelsPerFrame: 2, mBitsPerChannel: UInt32(sampleBytes * 8), mReserved: 0
    )
    var description: CMAudioFormatDescription?
    let descriptionStatus = CMAudioFormatDescriptionCreate(allocator: kCFAllocatorDefault,
        asbd: &format, layoutSize: 0, layout: nil, magicCookieSize: 0, magicCookie: nil,
        extensions: nil, formatDescriptionOut: &description)
    #expect(descriptionStatus == noErr)
    let audioDescription = try #require(description)
    var block: CMBlockBuffer?
    let blockStatus = CMBlockBufferCreateWithMemoryBlock(allocator: kCFAllocatorDefault,
        memoryBlock: nil, blockLength: bytes.count, blockAllocator: kCFAllocatorDefault,
        customBlockSource: nil, offsetToData: 0, dataLength: bytes.count, flags: 0,
        blockBufferOut: &block)
    #expect(blockStatus == noErr)
    let dataBlock = try #require(block)
    let copied = bytes.withUnsafeBytes { pointer in
        CMBlockBufferReplaceDataBytes(with: pointer.baseAddress!, blockBuffer: dataBlock,
                                     offsetIntoDestination: 0, dataLength: bytes.count)
    }
    #expect(copied == noErr)
    var sample: CMSampleBuffer?
    let sampleStatus = CMAudioSampleBufferCreateReadyWithPacketDescriptions(allocator: kCFAllocatorDefault,
        dataBuffer: dataBlock, formatDescription: audioDescription, sampleCount: frames,
        presentationTimeStamp: .zero, packetDescriptions: nil, sampleBufferOut: &sample)
    #expect(sampleStatus == noErr)
    return try #require(sample)
}

private func floatData(_ values: [Float]) -> Data { values.withUnsafeBytes { Data($0) } }

@Test func realCoreMediaPlanarFloatConversionPreservesChannelOrder() throws {
    let input = try sample(bytes: floatData([-1, 0.5, 1, -0.5]), frames: 2, planar: true, float: true)
    let output = try ScreenCaptureSource.convert(input)
    #expect(output == Data([0,128,255,127, 0,64,0,192]))
}

@Test func realCoreMediaInterleavedFloatConversionPreservesChannelOrder() throws {
    let input = try sample(bytes: floatData([-1, 1, 0.5, -0.5]), frames: 2, planar: false, float: true)
    let output = try ScreenCaptureSource.convert(input)
    #expect(output == Data([0,128,255,127, 0,64,0,192]))
}

@Test func realCoreMediaSignedPCMIsByteExact() throws {
    let bytes = Data([0,128,255,127, 0,64,0,192])
    let input = try sample(bytes: bytes, frames: 2, planar: false, float: false)
    #expect(try ScreenCaptureSource.convert(input) == bytes)
}

@Test func realCoreMediaUnexpectedRateAndNonfiniteSamplesFail() throws {
    let rate = try sample(bytes: floatData([0, 0]), frames: 1, planar: false, float: true, sampleRate: 44_100)
    #expect(throws: TapFailure.invalidAudio) { try ScreenCaptureSource.convert(rate) }
    let invalid = try sample(bytes: floatData([.infinity, 0]), frames: 1, planar: false, float: true)
    #expect(throws: TapFailure.invalidAudio) { try ScreenCaptureSource.convert(invalid) }
}
