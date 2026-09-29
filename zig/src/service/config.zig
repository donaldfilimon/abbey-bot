//! The `serve` environment contract (the oracle's `.env.example`, plus the
//! rewrite's fallback-provider keys), credential selection transcribed from
//! the oracle's `main.rs` (`read_discord_token`), and the refusal to run a
//! real token beside the live managed Rust service.
const std = @import("std");
const llm = @import("../llm/openai.zig");
const text = @import("../text/text.zig");
const Allocator = std.mem.Allocator;

pub const TokenSource = enum {
    primary,
    fallback,

    pub fn name(s: TokenSource) []const u8 {
        return if (s == .primary) "DISCORD_TOKEN" else "DISCORD_BOT_TOKEN";
    }

    pub fn rejectedDiagnostic(s: TokenSource) []const u8 {
        return switch (s) {
            .primary => "DISCORD_TOKEN was rejected by Discord during authentication. Reset the bot token in the Developer Portal, export the new value as DISCORD_TOKEN, and never hardcode it.",
            .fallback => "DISCORD_BOT_TOKEN was rejected by Discord during authentication. Reset the bot token in the Developer Portal, export the new value as DISCORD_BOT_TOKEN, and never hardcode it.",
        };
    }

    pub fn acceptedDiagnostic(s: TokenSource) []const u8 {
        return switch (s) {
            .primary => "Discord authentication preflight accepted the credential from DISCORD_TOKEN.",
            .fallback => "Discord authentication preflight accepted the credential from DISCORD_BOT_TOKEN.",
        };
    }
};

pub const Token = struct { secret: []const u8, source: TokenSource };

/// A present, nonblank DISCORD_TOKEN wins; DISCORD_BOT_TOKEN is consulted
/// only when the primary is absent; a present-but-blank or non-UTF-8 value
/// is an error and never falls through. Diagnostics name the variable only.
pub fn selectToken(env: *const std.process.Environ.Map) union(enum) { ok: Token, err: []const u8 } {
    if (env.get("DISCORD_TOKEN")) |v| {
        if (!std.unicode.utf8ValidateSlice(v)) return .{ .err = "DISCORD_TOKEN is not valid Unicode; refusing to consult DISCORD_BOT_TOKEN." };
        const t = text.trim(v);
        if (t.len == 0) return .{ .err = "DISCORD_TOKEN is present but blank; refusing to consult DISCORD_BOT_TOKEN." };
        return .{ .ok = .{ .secret = t, .source = .primary } };
    }
    if (env.get("DISCORD_BOT_TOKEN")) |v| {
        if (!std.unicode.utf8ValidateSlice(v)) return .{ .err = "DISCORD_BOT_TOKEN is not valid Unicode." };
        const t = text.trim(v);
        if (t.len == 0) return .{ .err = "DISCORD_BOT_TOKEN is present but blank." };
        return .{ .ok = .{ .secret = t, .source = .fallback } };
    }
    return .{ .err = "Neither DISCORD_TOKEN nor DISCORD_BOT_TOKEN is set. Export one bot token; never hardcode it." };
}

fn nonBlank(env: *const std.process.Environ.Map, key: []const u8) ?[]const u8 {
    const v = env.get(key) orelse return null;
    return text.nonBlank(v);
}

pub const Config = struct {
    home_guild: ?u64,
    data_dir: ?[]const u8,
    message_content: bool,
    provider: llm.Provider,
    episode_gate_config: ?[]const u8,
    test_guild_override: bool,
};

pub const ConfigError = union(enum) { ok: Config, err: []const u8 };

/// Parse everything except the token. Endpoint problems are reported with
/// the oracle's wording.
pub fn parse(arena: Allocator, env: *const std.process.Environ.Map) Allocator.Error!ConfigError {
    var provider: llm.Provider = .{};
    if (nonBlank(env, "ABBEY_BOT_LLM_ENDPOINT")) |endpoint| {
        llm.validateEndpoint(endpoint, .primary) catch |e| {
            const buf = try arena.alloc(u8, 160);
            return .{ .err = llm.endpointErrorMessage("ABBEY_BOT_LLM_ENDPOINT", e, buf) };
        };
        provider.primary = .{ .base = endpoint, .model = nonBlank(env, "ABBEY_BOT_LLM_MODEL") orelse llm.default_local_model, .label = llm.primary_label };
    }
    if (nonBlank(env, "ABBEY_BOT_LLM_FALLBACK_ENDPOINT")) |endpoint| {
        llm.validateEndpoint(endpoint, .fallback) catch |e| {
            const buf = try arena.alloc(u8, 160);
            return .{ .err = llm.endpointErrorMessage("ABBEY_BOT_LLM_FALLBACK_ENDPOINT", e, buf) };
        };
        provider.fallback = .{ .base = endpoint, .model = nonBlank(env, "ABBEY_BOT_LLM_FALLBACK_MODEL") orelse llm.default_local_model, .key = nonBlank(env, "ABBEY_BOT_LLM_FALLBACK_KEY"), .label = llm.fallback_label };
    }
    const home_guild: ?u64 = if (nonBlank(env, "ABBEY_GUILD_ID")) |g| std.fmt.parseInt(u64, g, 10) catch return .{ .err = "ABBEY_GUILD_ID must be a Discord guild id" } else null;
    return .{ .ok = .{
        .home_guild = home_guild,
        .data_dir = nonBlank(env, "ABBEY_DATA_DIR"),
        .message_content = if (env.get("ABBEY_MESSAGE_CONTENT")) |v| std.mem.eql(u8, text.trim(v), "1") else false,
        .provider = provider,
        .episode_gate_config = nonBlank(env, "ABBEY_EPISODE_GATE_CONFIG"),
        .test_guild_override = if (env.get("ABBEY_BOT_ZIG_TEST_GUILD")) |v| std.mem.eql(u8, text.trim(v), "1") else false,
    } };
}

/// The live Rust bot's launchd label. `serve` never runs a real token while
/// it is loaded unless the operator explicitly marks this a test-guild run.
pub const live_rust_label = "com.donaldfilimon.abbey-bot";
pub const this_label = "com.donaldfilimon.abbey-bot-zig";

pub const Refusal = enum { none, live_service_loaded };

pub fn refusal(live_rust_loaded: bool, test_guild_override: bool) Refusal {
    return if (live_rust_loaded and !test_guild_override) .live_service_loaded else .none;
}

pub const refusal_message = "Refusing to start: the managed Rust bot (" ++ live_rust_label ++ ") is loaded. Running a second bot on a production token would fight it for the same session. Use a separate test-guild bot token and set ABBEY_BOT_ZIG_TEST_GUILD=1, or stop here.";

/// Exact-label launchd query: `launchctl print gui/<uid>/<label>` exits 0
/// only when that one job is loaded (never a substring match).
pub fn liveRustServiceLoaded(arena: Allocator, io: std.Io) bool {
    const uid = std.posix.system.getuid();
    const target = std.fmt.allocPrint(arena, "gui/{d}/" ++ live_rust_label, .{uid}) catch return true;
    const result = std.process.run(arena, io, .{
        .argv = &.{ "/bin/launchctl", "print", target },
        .stdout_limit = .limited(1 << 20),
        .stderr_limit = .limited(64 * 1024),
        .timeout = .{ .duration = .{ .raw = .fromSeconds(10), .clock = .awake } },
    }) catch return true; // cannot tell: fail closed
    return result.term == .exited and result.term.exited == 0;
}

const testing = std.testing;

test "token selection is source-aware and fail-closed" {
    var env = std.process.Environ.Map.init(testing.allocator);
    defer env.deinit();
    try testing.expectEqualStrings("Neither DISCORD_TOKEN nor DISCORD_BOT_TOKEN is set. Export one bot token; never hardcode it.", selectToken(&env).err);
    try env.put("DISCORD_BOT_TOKEN", " fallback ");
    try testing.expectEqualStrings("fallback", selectToken(&env).ok.secret);
    try testing.expectEqual(TokenSource.fallback, selectToken(&env).ok.source);
    try env.put("DISCORD_TOKEN", "   ");
    try testing.expectEqualStrings("DISCORD_TOKEN is present but blank; refusing to consult DISCORD_BOT_TOKEN.", selectToken(&env).err);
    try env.put("DISCORD_TOKEN", "primary");
    try testing.expectEqual(TokenSource.primary, selectToken(&env).ok.source);
    try env.put("DISCORD_TOKEN", "\xff");
    try testing.expectEqualStrings("DISCORD_TOKEN is not valid Unicode; refusing to consult DISCORD_BOT_TOKEN.", selectToken(&env).err);
    _ = env.swapRemove("DISCORD_TOKEN");
    try env.put("DISCORD_BOT_TOKEN", "");
    try testing.expectEqualStrings("DISCORD_BOT_TOKEN is present but blank.", selectToken(&env).err);
}

test "serve refuses beside the live Rust service unless this is an explicit test-guild run" {
    try testing.expectEqual(Refusal.live_service_loaded, refusal(true, false));
    try testing.expectEqual(Refusal.none, refusal(true, true));
    try testing.expectEqual(Refusal.none, refusal(false, false));
    try testing.expect(!std.mem.eql(u8, this_label, live_rust_label));
}

test "environment contract: blank values are unset and remote primaries are refused" {
    var arena: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena.deinit();
    var env = std.process.Environ.Map.init(testing.allocator);
    defer env.deinit();
    try env.put("ABBEY_BOT_LLM_ENDPOINT", "");
    try env.put("ABBEY_GUILD_ID", "123");
    try env.put("ABBEY_MESSAGE_CONTENT", "1");
    const c = (try parse(arena.allocator(), &env)).ok;
    try testing.expect(!c.provider.configured());
    try testing.expectEqual(@as(?u64, 123), c.home_guild);
    try testing.expect(c.message_content);
    try env.put("ABBEY_BOT_LLM_ENDPOINT", "http://10.0.0.1:11434");
    try testing.expectEqualStrings("ABBEY_BOT_LLM_ENDPOINT must target loopback (127.0.0.1 / localhost / ::1); remote hosts are refused", (try parse(arena.allocator(), &env)).err);
    try env.put("ABBEY_BOT_LLM_ENDPOINT", "http://127.0.0.1:11434");
    const ok = (try parse(arena.allocator(), &env)).ok;
    try testing.expectEqualStrings("gemma4:12b", ok.provider.primary.?.model);
}
