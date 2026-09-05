import Foundation

public enum Route: Equatable {
    case health, stream, reject(Int)
}

public enum HTTP {
    public static let authority = "127.0.0.1:8182"
    public static let maximumRequestBytes = 4_096

    /// Only a single, body-free HTTP/1.1 request from a native loopback client.
    public static func route(_ data: Data, authority: String = authority) -> Route {
        guard data.count <= maximumRequestBytes, let text = String(data: data, encoding: .utf8),
              text.hasSuffix("\r\n\r\n"), !text.contains("\0") else { return .reject(400) }
        // CRLF is one Swift Character; HTTP terminators are four bytes, not four Characters.
        let lines = String(decoding: data.dropLast(4), as: UTF8.self).components(separatedBy: "\r\n")
        guard let request = lines.first else { return .reject(400) }
        let parts = request.split(separator: " ", omittingEmptySubsequences: false)
        guard parts.count == 3, parts[2] == "HTTP/1.1" else { return .reject(400) }
        guard parts[0] == "GET" else { return .reject(405) }
        var headers: [String: String] = [:]
        for line in lines.dropFirst() {
            guard let colon = line.firstIndex(of: ":"), line.first != " ", line.first != "\t" else {
                return .reject(400)
            }
            let name = String(line[..<colon]).lowercased()
            guard !name.isEmpty, name.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-") }),
                  headers[name] == nil else { return .reject(400) }
            headers[name] = String(line[line.index(after: colon)...]).trimmingCharacters(in: .whitespaces)
        }
        guard headers["host"] == authority else { return .reject(403) }
        guard headers["origin"] == nil, !headers.keys.contains(where: { $0.hasPrefix("sec-fetch-") }),
              headers["transfer-encoding"] == nil, headers["content-length"] == nil,
              headers["upgrade"] == nil else { return .reject(403) }
        switch parts[1] {
        case "/health": return .health
        case "/stream": return .stream
        default: return .reject(404)
        }
    }

    public static func response(status: Int, body: Data) -> Data {
        let reasons = [200: "OK", 400: "Bad Request", 403: "Forbidden", 404: "Not Found",
                       405: "Method Not Allowed", 408: "Request Timeout", 409: "Conflict",
                       431: "Request Header Fields Too Large", 503: "Service Unavailable"]
        var data = Data("HTTP/1.1 \(status) \(reasons[status] ?? "Error")\r\nContent-Type: application/json\r\nContent-Length: \(body.count)\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n\r\n".utf8)
        data.append(body)
        return data
    }

    public static let streamHeader = Data("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nX-Audio-Format: s16le\r\nX-Audio-Sample-Rate: 48000\r\nX-Audio-Channels: 2\r\n\r\n".utf8)

    public static func chunk(_ pcm: Data) -> Data {
        var data = Data("\(String(pcm.count, radix: 16))\r\n".utf8)
        data.append(pcm)
        data.append(Data("\r\n".utf8))
        return data
    }
}
