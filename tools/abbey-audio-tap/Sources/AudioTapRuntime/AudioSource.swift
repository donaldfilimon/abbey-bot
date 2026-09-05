import AudioTapCore
import Dispatch
import Foundation

/// Source callbacks and all methods run on the queue passed to its factory.
/// A stop completion means the old source cannot still be capturing.
public protocol AudioSource: AnyObject, Sendable {
    func start(pcm: @escaping @Sendable (Data) -> Void, failed: @escaping @Sendable (TapFailure) -> Void)
    func stop(completion: @escaping @Sendable () -> Void)
}

public typealias SourceFactory = @Sendable (DispatchQueue) -> any AudioSource
