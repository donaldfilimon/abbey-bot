import Foundation

/// A positive application filter prevents new processes entering the capture by default.
/// Values are used in memory only and never enter health responses or diagnostics.
public struct ApplicationIdentity: Equatable, Sendable {
    public let pid: Int32
    public let bundleID: String
    public let name: String
    public let executable: String
    public let launchTime: TimeInterval

    public init(pid: Int32, bundleID: String, name: String, executable: String, launchTime: TimeInterval) {
        self.pid = pid
        self.bundleID = bundleID
        self.name = name
        self.executable = executable
        self.launchTime = launchTime
    }
}

public enum ApplicationPolicy {
    // Browser audio is application-wide; a Discord tab cannot be removed separately.
    private static let deniedNames = [
        "discord", "vesktop", "vencord", "armcord", "legcord", "webcord", "equibop",
        "abbeybot", "abbeyaudiotap", "safari", "chrome", "chromium", "firefox",
        "brave", "vivaldi", "microsoftedge", "opera", "orion", "librewolf", "waterfox",
        "zenbrowser", "ladybird", "perplexitycomet", "chatgptatlas", "thebrowser",
        "terminal", "iterm", "ghostty", "warp", "alacritty", "kitty", "wezterm",
        "tabby", "hyper", "rioapp",
    ]

    private static func normalized(_ value: String) -> String {
        value.lowercased().filter { $0.isLetter || $0.isNumber }
    }

    public static func permits(_ app: ApplicationIdentity, ownPID: Int32) -> Bool {
        guard app.pid > 0, app.pid != ownPID, app.launchTime.isFinite,
              app.launchTime > 0, !app.bundleID.trimmingCharacters(in: .whitespaces).isEmpty,
              app.executable.hasPrefix("/") else { return false }
        let identity = normalized(app.bundleID + " " + app.name + " " + app.executable)
        if deniedNames.contains(where: identity.contains) { return false }
        let bundle = app.bundleID.lowercased()
        let browserPrefixes = ["com.openai.atlas", "com.perplexity.comet", "company.thebrowser.",
                               "company.thebrowser", "app.zen-browser.", "app.zen-browser", "org.zen."]
        if browserPrefixes.contains(where: bundle.hasPrefix) { return false }
        let components = app.executable.lowercased().split(separator: "/")
        if components.contains(where: { ["arc.app", "dia.app", "comet.app", "atlas.app", "zen.app", "edge.app"].contains(String($0)) }) {
            return false
        }
        let name = normalized(app.name)
        // Short names need exact matching, while the vendor ID catches their helpers.
        return !["arc", "dia", "comet", "atlas", "zen", "edge"].contains(name)
    }

    public static func identitiesStillMatch(
        _ selected: [ApplicationIdentity], current: (Int32) -> ApplicationIdentity?
    ) -> Bool {
        !selected.isEmpty && selected.allSatisfy { current($0.pid) == $0 }
    }
}
