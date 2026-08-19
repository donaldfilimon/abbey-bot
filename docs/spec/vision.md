# Vision — Image Understanding

Abbey reads images the same way she reads text: attachments get described, the
description is folded into the message before intent classification, so the Brain and
personas see one enriched string. Same seam pattern as `ABIEngine` and the speech
protocols — one Apple-framework implementation for the Apple Silicon host, one remote
OpenAI-compatible implementation that works everywhere including the Linux container.
Selected at boot via `ABBEY_VISION=apple|remote`.

```swift
protocol ImageUnderstanding: Sendable {
    /// Natural-language description of the image at `imageURL`, ≤2 sentences,
    /// suitable for inline folding into a chat message.
    func describe(imageURL: String) async throws -> String
    /// OCR only — verbatim text found in the image.
    func extractText(imageURL: String) async throws -> String
}
```

## Shared fetch

```swift
enum ImageFetcher {
    static let maxBytes = 10 << 20   // 10MB cap — attachments are attacker-controlled

    static func fetch(_ url: String, http: HTTPClient) async throws -> Data {
        // tgfile:// pseudo-URLs come from TelegramAdapter; resolve first.
        var resolved = url
        if url.hasPrefix("tgfile://"), let adapter = await PlatformRegistry.shared.telegram {
            resolved = try await adapter.resolveFileURL(fileId: String(url.dropFirst("tgfile://".count)))
        }
        var req = HTTPClientRequest(url: resolved)
        req.method = .GET
        // Slack private files need the bot token.
        if resolved.contains("files.slack.com"), let token = await PlatformRegistry.shared.slackToken {
            req.headers.add(name: "Authorization", value: "Bearer \(token)")
        }
        let res = try await http.execute(req, timeout: .seconds(30))
        return Data(buffer: try await res.body.collect(upTo: maxBytes))
    }
}

/// Boot-time registry so cross-cutting services (vision, voice) can reach adapters
/// without threading them through every call.
actor PlatformRegistryStore {
    private(set) var telegram: TelegramAdapter?
    private(set) var slackToken: String?
    func setTelegram(_ adapter: TelegramAdapter) { telegram = adapter }
    func setSlackToken(_ token: String) { slackToken = token }
}
enum PlatformRegistry {
    static let shared = PlatformRegistryStore()
}
```

## RemoteVisionAnalyzer — OpenAI-compatible, all platforms

Chat-completions with image content parts. Works against OpenAI, or any local
OpenAI-compatible server that supports vision models.

```swift
struct RemoteVisionAnalyzer: ImageUnderstanding {
    let baseURL: String
    let apiKey: String
    let model: String            // e.g. "gpt-4o-mini"
    let http: HTTPClient

    func describe(imageURL: String) async throws -> String {
        try await ask(imageURL: imageURL,
                      prompt: "Describe this image in at most two short sentences. Factual, no preamble.")
    }

    func extractText(imageURL: String) async throws -> String {
        try await ask(imageURL: imageURL,
                      prompt: "Transcribe all text visible in this image verbatim. Output only the text.")
    }

    private func ask(imageURL: String, prompt: String) async throws -> String {
        // Fetch + base64 rather than passing the URL through: Discord CDN links are
        // signed and expire, and Telegram/Slack URLs need auth the remote end lacks.
        let data = try await ImageFetcher.fetch(imageURL, http: http)
        let mime = Self.sniffMime(data)
        let b64 = data.base64EncodedString()

        struct ChatRequest: Encodable {
            struct Message: Encodable {
                let role: String
                let content: [Content]
            }
            enum Content: Encodable {
                case text(String), image(String)
                func encode(to encoder: Encoder) throws {
                    var c = encoder.container(keyedBy: Keys.self)
                    switch self {
                    case .text(let t):
                        try c.encode("text", forKey: .type)
                        try c.encode(t, forKey: .text)
                    case .image(let url):
                        try c.encode("image_url", forKey: .type)
                        try c.encode(["url": url], forKey: .image_url)
                    }
                }
                enum Keys: String, CodingKey { case type, text, image_url }
            }
            let model: String
            let messages: [Message]
            let max_tokens: Int
        }

        let payload = ChatRequest(
            model: model,
            messages: [.init(role: "user", content: [
                .text(prompt),
                .image("data:\(mime);base64,\(b64)"),
            ])],
            max_tokens: 200)

        var req = HTTPClientRequest(url: "\(baseURL)/chat/completions")
        req.method = .POST
        req.headers.add(name: "Authorization", value: "Bearer \(apiKey)")
        req.headers.add(name: "Content-Type", value: "application/json")
        req.body = .bytes(try JSONEncoder().encode(payload))
        let res = try await http.execute(req, timeout: .seconds(60))
        let body = try await res.body.collect(upTo: 1 << 20)

        struct ChatResponse: Decodable {
            struct Choice: Decodable {
                struct Msg: Decodable { let content: String }
                let message: Msg
            }
            let choices: [Choice]
        }
        return try JSONDecoder().decode(ChatResponse.self, from: body)
            .choices.first?.message.content ?? ""
    }

    static func sniffMime(_ data: Data) -> String {
        if data.starts(with: [0xFF, 0xD8, 0xFF]) { return "image/jpeg" }
        if data.starts(with: [0x89, 0x50, 0x4E, 0x47]) { return "image/png" }
        if data.starts(with: [0x47, 0x49, 0x46]) { return "image/gif" }
        if data.count > 12, data[8...11] == Data("WEBP".utf8) { return "image/webp" }
        return "application/octet-stream"
    }
}
```

## AppleVisionAnalyzer — macOS host

Vision framework: classification + OCR are on-device and free. Full scene *captioning*
at the quality of a VLM is Foundation Models territory on the macOS 26+ host —
classification labels + OCR composite shown here is complete and useful on its own;
the FM upgrade slots behind the same protocol.

```swift
#if canImport(Vision) && canImport(CoreImage)
import Vision
import CoreImage

struct AppleVisionAnalyzer: ImageUnderstanding {
    let http: HTTPClient

    func describe(imageURL: String) async throws -> String {
        let data = try await ImageFetcher.fetch(imageURL, http: http)
        guard let ci = CIImage(data: data) else { throw Abort(.unprocessableEntity) }

        let classify = VNClassifyImageRequest()
        let animals = VNRecognizeAnimalsRequest()
        let faces = VNDetectFaceRectanglesRequest()
        let handler = VNImageRequestHandler(ciImage: ci)
        try handler.perform([classify, animals, faces])

        var parts: [String] = []
        let labels = (classify.results ?? [])
            .filter { $0.confidence > 0.35 }
            .prefix(4)
            .map(\.identifier)
        if !labels.isEmpty { parts.append(labels.joined(separator: ", ")) }
        if let count = animals.results?.count, count > 0 { parts.append("\(count) animal(s)") }
        if let count = faces.results?.count, count > 0 { parts.append("\(count) face(s)") }

        let ocr = try? await extractText(data: data)
        if let ocr, !ocr.isEmpty { parts.append("text: \"\(ocr.prefix(120))\"") }

        return parts.isEmpty ? "an image (no confident classification)"
                             : parts.joined(separator: "; ")
    }

    func extractText(imageURL: String) async throws -> String {
        let data = try await ImageFetcher.fetch(imageURL, http: http)
        return try await extractText(data: data)
    }

    private func extractText(data: Data) async throws -> String {
        guard let ci = CIImage(data: data) else { return "" }
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = true
        try VNImageRequestHandler(ciImage: ci).perform([request])
        return (request.results ?? [])
            .compactMap { $0.topCandidates(1).first?.string }
            .joined(separator: "\n")
    }
}
#endif
```

## Boot selection

```swift
func makeVision(app: Application) -> any ImageUnderstanding {
    switch Environment.get("ABBEY_VISION") ?? "remote" {
    #if canImport(Vision)
    case "apple":
        return AppleVisionAnalyzer(http: app.http.client.shared)
    #endif
    default:
        return RemoteVisionAnalyzer(
            baseURL: Environment.get("ABBEY_VISION_BASE_URL") ?? "https://api.openai.com/v1",
            apiKey: Environment.get("ABBEY_VISION_API_KEY") ?? "",
            model: Environment.get("ABBEY_VISION_MODEL") ?? "gpt-4o-mini",
            http: app.http.client.shared)
    }
}
```

The `#if canImport` guard means `ABBEY_VISION=apple` inside the Linux container falls
through to remote at compile time rather than crashing at runtime — the canonical
Apple Silicon host gets the on-device path, the Docker fallback gets the remote path,
and neither is a stub.

## Slash command surface

```swift
.init(name: "see", description: "Describe the last image posted in this channel"),
.init(name: "ocr", description: "Extract text from the last image in this channel"),
```

Both resolve "last image" via `client.getChannelMessages(channelId:limit:)`, scanning
for the newest message with an image attachment, then call the active
`ImageUnderstanding` and reply with the result — deferred first, since fetch+inference
routinely exceeds the 3-second interaction window.
