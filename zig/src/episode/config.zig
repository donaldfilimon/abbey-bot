//! Episode gate configuration (`ABBEY_EPISODE_GATE_CONFIG`), transcribed from
//! the oracle's `src/episode_gate/config.rs`. Default off: an unset or blank
//! variable means no gate. Validation messages are the oracle's words; a
//! JSON-level failure keeps its prefix and reports std.json's own position.
const std = @import("std");
const Allocator = std.mem.Allocator;

pub const env_name = "ABBEY_EPISODE_GATE_CONFIG";
pub const default_timeout_secs: u64 = 15;
pub const max_timeout_secs: u64 = 120;
pub const max_config_bytes: usize = 64 * 1024;
pub const default_service_principal = "abbey-service";
pub const max_identifier_len = 64;
pub const max_guild_ref_len = 128;

pub const EvidenceLevel = enum { C0, C1, C2, C3 };

pub const Config = struct {
    abi_cli: []const u8,
    endpoint: []const u8,
    token_file: []const u8,
    ca_cert: ?[]const u8,
    policy_version: []const u8,
    contract_revision: u64,
    contract_digest: [32]u8,
    service_principal: []const u8,
    evidence_level: EvidenceLevel,
    timeout_secs: u64,
    /// null covers every scope.
    guilds: ?[]const []const u8,

    pub fn covers(c: *const Config, scoped_guild: []const u8) bool {
        const list = c.guilds orelse return true;
        for (list) |g| if (std.mem.eql(u8, g, scoped_guild)) return true;
        return false;
    }

    pub fn coverage(c: *const Config) ?usize {
        return if (c.guilds) |g| g.len else null;
    }
};

/// Parse outcome: a config (arena-owned) or the oracle's message.
pub const Parsed = union(enum) { ok: Config, err: []const u8 };

pub fn parseDigest(text: []const u8) ?[32]u8 {
    if (text.len != 64) return null;
    var out: [32]u8 = undefined;
    _ = std.fmt.hexToBytes(&out, text) catch return null;
    if (std.mem.allEqual(u8, &out, 0)) return null;
    return out;
}

pub fn boundedIdentifier(value: []const u8, max: usize) bool {
    if (value.len == 0 or value.len > max) return false;
    for (value) |b| {
        if (!(std.ascii.isLower(b) or std.ascii.isDigit(b) or b == '_' or b == '-' or b == '.')) return false;
    }
    return true;
}

/// `discord:123` -> `discord-123` when it is a valid ledger reference.
pub fn guildRefFor(buf: []u8, scoped_guild: []const u8) ?[]const u8 {
    if (scoped_guild.len > buf.len) return null;
    for (scoped_guild, 0..) |b, i| buf[i] = if (b == ':') '-' else std.ascii.toLower(b);
    const out = buf[0..scoped_guild.len];
    return if (boundedIdentifier(out, max_guild_ref_len)) out else null;
}

fn absolutePath(p: []const u8) bool {
    if (p.len == 0 or p[0] != '/') return false;
    var it = std.mem.splitScalar(u8, p, '/');
    while (it.next()) |part| if (std.mem.eql(u8, part, "..")) return false;
    return true;
}

fn checkEndpoint(endpoint: []const u8, has_ca: bool) ?[]const u8 {
    const sep = std.mem.indexOf(u8, endpoint, "://") orelse return "endpoint must start with http:// or https://";
    const scheme = endpoint[0..sep];
    const rest = endpoint[sep + 3 ..];
    const auth_end = std.mem.indexOfAny(u8, rest, "/?#") orelse rest.len;
    const authority = rest[0..auth_end];
    const host = if (authority.len > 0 and authority[0] == '[')
        (if (std.mem.indexOfScalar(u8, authority[1..], ']')) |i| authority[1 .. 1 + i] else authority[1..])
    else if (std.mem.lastIndexOfScalar(u8, authority, ':')) |i| authority[0..i] else authority;
    const loopback = std.mem.eql(u8, host, "127.0.0.1") or std.mem.eql(u8, host, "::1") or std.mem.eql(u8, host, "localhost");
    if (std.mem.eql(u8, scheme, "http")) return if (loopback) null else "endpoint: non-loopback endpoints require https and ca_cert";
    if (std.mem.eql(u8, scheme, "https")) return if (loopback or has_ca) null else "endpoint: ca_cert is required for a non-loopback https endpoint";
    return "endpoint must start with http:// or https://";
}

const Raw = struct {
    abi_cli: []const u8,
    endpoint: []const u8,
    token_file: []const u8,
    ca_cert: ?[]const u8 = null,
    policy_version: []const u8,
    contract_revision: u64,
    contract_digest: []const u8,
    service_principal: ?[]const u8 = null,
    evidence_level: ?[]const u8 = null,
    timeout_secs: ?u64 = null,
    guilds: ?[]const []const u8 = null,
};

/// Validate JSON text (the oracle's `from_json`); file existence is checked by
/// `checkFiles` (the oracle's `from_path`).
pub fn fromJson(arena: Allocator, text: []const u8) Allocator.Error!Parsed {
    var diagnostics: std.json.Diagnostics = .{};
    var scanner = std.json.Scanner.initCompleteInput(arena, text);
    scanner.enableDiagnostics(&diagnostics);
    const raw = std.json.parseFromTokenSourceLeaky(Raw, arena, &scanner, .{ .allocate = .alloc_always }) catch |e| switch (e) {
        error.OutOfMemory => return error.OutOfMemory,
        else => return .{ .err = try std.fmt.allocPrint(arena, env_name ++ ": invalid JSON at line {d} column {d}", .{ diagnostics.getLine(), diagnostics.getColumn() }) },
    };
    const trim = struct {
        fn f(s: []const u8) []const u8 {
            return std.mem.trim(u8, s, " \t\r\n");
        }
    }.f;
    const abi_cli = trim(raw.abi_cli);
    if (!absolutePath(abi_cli)) return .{ .err = "abi_cli must be an absolute path without `..`" };
    const token_file = trim(raw.token_file);
    if (!absolutePath(token_file)) return .{ .err = "token_file must be an absolute path without `..`" };
    var ca_cert: ?[]const u8 = null;
    if (raw.ca_cert) |c| {
        const t = trim(c);
        if (t.len > 0) {
            if (!absolutePath(t)) return .{ .err = "ca_cert must be an absolute path without `..`" };
            ca_cert = t;
        }
    }
    const endpoint = trim(raw.endpoint);
    if (checkEndpoint(endpoint, ca_cert != null)) |msg| return .{ .err = msg };
    const policy_version = trim(raw.policy_version);
    if (!boundedIdentifier(policy_version, max_identifier_len)) return .{ .err = "policy_version must be 1-64 chars of [a-z0-9_.-]" };
    if (raw.contract_revision == 0) return .{ .err = "contract_revision must be greater than zero" };
    const digest = parseDigest(trim(raw.contract_digest)) orelse return .{ .err = "contract_digest must be 64 hexadecimal characters and not all zero" };
    var principal: []const u8 = default_service_principal;
    if (raw.service_principal) |p| {
        const t = trim(p);
        if (t.len > 0) principal = t;
    }
    if (!boundedIdentifier(principal, max_identifier_len)) return .{ .err = "service_principal must be 1-64 chars of [a-z0-9_.-]" };
    const level: EvidenceLevel = blk: {
        const t = if (raw.evidence_level) |e| trim(e) else "";
        if (t.len == 0 or std.mem.eql(u8, t, "c0") or std.mem.eql(u8, t, "C0")) break :blk .C0;
        if (std.mem.eql(u8, t, "c1") or std.mem.eql(u8, t, "C1")) break :blk .C1;
        if (std.mem.eql(u8, t, "c2") or std.mem.eql(u8, t, "C2")) break :blk .C2;
        if (std.mem.eql(u8, t, "c3") or std.mem.eql(u8, t, "C3")) break :blk .C3;
        return .{ .err = "evidence_level must be one of c0, c1, c2, c3" };
    };
    const timeout = raw.timeout_secs orelse default_timeout_secs;
    if (timeout < 1 or timeout > max_timeout_secs) return .{ .err = "timeout_secs must be 1-120" };
    var guilds: ?[]const []const u8 = null;
    if (raw.guilds) |entries| {
        var covered: std.ArrayList([]const u8) = .empty;
        for (entries) |entry| {
            const g = trim(entry);
            var buf: [256]u8 = undefined;
            if (g.len == 0 or guildRefFor(&buf, g) == null) return .{ .err = "guilds entries must be scoped guild ids that map to a ledger guild reference (for example discord:123)" };
            var dup = false;
            for (covered.items) |c| if (std.mem.eql(u8, c, g)) {
                dup = true;
            };
            if (!dup) try covered.append(arena, g);
        }
        if (covered.items.len == 0) return .{ .err = "guilds must name at least one scoped guild id or be omitted" };
        guilds = covered.items;
    }
    return .{ .ok = .{
        .abi_cli = abi_cli,
        .endpoint = endpoint,
        .token_file = token_file,
        .ca_cert = ca_cert,
        .policy_version = policy_version,
        .contract_revision = raw.contract_revision,
        .contract_digest = digest,
        .service_principal = principal,
        .evidence_level = level,
        .timeout_secs = timeout,
        .guilds = guilds,
    } };
}

/// The oracle's `from_path` file checks: each named path must be a regular file.
pub fn checkFiles(io: std.Io, c: *const Config) ?[]const u8 {
    const checks = [_]struct { path: ?[]const u8, name: []const u8 }{
        .{ .path = c.abi_cli, .name = "abi_cli" },
        .{ .path = c.token_file, .name = "token_file" },
        .{ .path = c.ca_cert, .name = "ca_cert" },
    };
    for (checks) |ch| {
        const p = ch.path orelse continue;
        const st = std.Io.Dir.cwd().statFile(io, p, .{}) catch return ch.name;
        if (st.kind != .file) return ch.name;
    }
    return null;
}
