//! Managed-service readiness and bootstrap documents in the oracle's v1 wire
//! format (`deploy/service-protocol-v1.json`, `src/readiness.rs`), published
//! under this component's own private directory
//! `~/.local/share/abbey-bot-zig` (0700, files 0600, temp file plus rename).
//! It never writes the live Rust bot's `~/.local/share/abbey-bot`.
const std = @import("std");
const Io = std.Io;
const Allocator = std.mem.Allocator;

pub const component = "abbey-bot-zig";
pub const readiness_file = "readiness.json";
pub const bootstrap_file = "bootstrap-status.json";
pub const readiness_max_bytes = 4096;
pub const bootstrap_max_bytes = 512;
/// The oracle's `REFRESH_INTERVAL`: a ready document older than 30 s is stale.
pub const refresh_interval_ms: u64 = 10_000;

pub const Phase = enum { starting, ready, draining };
pub const Discord = enum { connecting, ready, stopped };
pub const Scheduler = enum { starting, running, stopped };
pub const Connector = enum { disabled, starting, connected, degraded, stopped };
pub const Persistence = enum { not_attempted, memory_only, complete, partial, failed };
pub const BootstrapPhase = enum { starting, failed };
pub const BootstrapCode = enum { none, readiness_file, log_directory, log_file, log_writer };

pub const Identity = struct {
    pid: u32,
    nonce: [64]u8,
    executable_sha256: [64]u8,

    /// Fresh 32-byte nonce plus the SHA-256 of the running executable, with a
    /// before/after size+mtime check (the oracle's `RunIdentity::current`).
    pub fn current(io: Io, gpa: Allocator) !Identity {
        var entropy: [32]u8 = undefined;
        try io.randomSecure(&entropy); // std/Io.zig: randomSecure
        const path = try std.process.executablePathAlloc(io, gpa); // std/process.zig
        defer gpa.free(path);
        var file = try Io.Dir.openFileAbsolute(io, path, .{});
        defer file.close(io);
        const before = try file.stat(io);
        if (before.kind != .file) return error.IdentityUnavailable;
        // std/crypto/sha2.zig: Sha256; std/Io/File.zig: reader, stat;
        // std/Io/Reader.zig: readSliceShort (0 only at end of stream).
        var hash: std.crypto.hash.sha2.Sha256 = .init(.{});
        var buf: [65536]u8 = undefined;
        var reader = file.reader(io, &.{});
        while (true) {
            const n = reader.interface.readSliceShort(&buf) catch return error.IdentityUnavailable;
            if (n == 0) break;
            hash.update(buf[0..n]);
        }
        const after = try file.stat(io);
        if (before.size != after.size or before.mtime.nanoseconds != after.mtime.nanoseconds) return error.IdentityUnavailable;
        var digest: [32]u8 = undefined;
        hash.final(&digest);
        const pid: u32 = @intCast(std.c.getpid()); // std/c.zig: getpid
        return fromParts(pid, entropy, digest);
    }

    pub fn fromParts(pid: u32, nonce: [32]u8, sha: [32]u8) error{IdentityUnavailable}!Identity {
        if (pid == 0 or pid > std.math.maxInt(i32)) return error.IdentityUnavailable;
        // std/fmt.zig: bytesToHex returns a fixed [2N]u8 array.
        return .{ .pid = pid, .nonce = std.fmt.bytesToHex(nonce, .lower), .executable_sha256 = std.fmt.bytesToHex(sha, .lower) };
    }
};

pub const State = struct {
    phase: Phase = .starting,
    discord: Discord = .connecting,
    scheduler: Scheduler = .starting,
    telegram: Connector = .disabled,
    slack: Connector = .disabled,
    last_persistence: Persistence = .not_attempted,
};

/// Explicit startup checkpoints: readiness is never inferred from a PID.
pub const Checkpoints = struct {
    scheduler_running: bool = false,
    discord_ready: bool = false,
    commands_registered: bool = false,

    pub fn complete(c: Checkpoints) bool {
        return c.scheduler_running and c.discord_ready and c.commands_registered;
    }
};

pub const EncodeError = error{ NotReady, TimeOutOfRange, WriteFailed };

/// Canonical bytes: serde field order, compact separators, trailing LF.
pub fn encodeReadiness(buf: *[readiness_max_bytes]u8, id: Identity, s: State, at_ms: u64, cp: Checkpoints) EncodeError![]const u8 {
    if (at_ms > std.math.maxInt(i64)) return error.TimeOutOfRange;
    if (s.phase == .ready and (!cp.complete() or s.discord != .ready or s.scheduler != .running)) return error.NotReady;
    var w: Io.Writer = .fixed(buf);
    w.print(
        "{{\"schema_version\":1,\"pid\":{d},\"run_nonce\":\"{s}\",\"executable_sha256\":\"{s}\",\"phase\":\"{t}\",\"published_at_unix_ms\":{d},\"discord\":\"{t}\",\"scheduler\":\"{t}\",\"telegram\":\"{t}\",\"slack\":\"{t}\",\"last_persistence\":\"{t}\"}}\n",
        .{ id.pid, &id.nonce, &id.executable_sha256, s.phase, at_ms, s.discord, s.scheduler, s.telegram, s.slack, s.last_persistence },
    ) catch return error.WriteFailed;
    return w.buffered();
}

pub fn encodeBootstrap(buf: *[bootstrap_max_bytes]u8, id: Identity, phase: BootstrapPhase, code: BootstrapCode) EncodeError![]const u8 {
    var w: Io.Writer = .fixed(buf);
    w.print(
        "{{\"schema_version\":1,\"pid\":{d},\"run_nonce\":\"{s}\",\"executable_sha256\":\"{s}\",\"phase\":\"{t}\",\"code\":\"{t}\"}}\n",
        .{ id.pid, &id.nonce, &id.executable_sha256, phase, code },
    ) catch return error.WriteFailed;
    return w.buffered();
}

pub const DirError = error{ UnsafeDirectory, Unavailable };

fn ownedMode(handle: std.posix.fd_t) DirError!struct { uid: std.c.uid_t, mode: u32, dir: bool, reg: bool } {
    // std/c.zig: fstat + the Darwin `Stat` (uid, mode). std.Io.File.Stat has no owner.
    var st: std.c.Stat = undefined;
    if (std.c.fstat(handle, &st) != 0) return error.Unavailable;
    const mode: u32 = @intCast(st.mode);
    return .{ .uid = st.uid, .mode = mode & 0o7777, .dir = mode & std.c.S.IFMT == std.c.S.IFDIR, .reg = mode & std.c.S.IFMT == std.c.S.IFREG };
}

/// `<home>/.local/share/abbey-bot-zig`, opened component by component
/// without following symlinks. Every ancestor must be owned by this user
/// and not group/other writable; the leaf must be exactly 0700.
pub const PrivateDir = struct {
    dir: Io.Dir,
    io: Io,

    pub fn open(io: Io, home: []const u8) DirError!PrivateDir {
        const uid = std.c.getuid();
        var current = Io.Dir.openDirAbsolute(io, home, .{ .follow_symlinks = false }) catch return error.Unavailable;
        errdefer current.close(io);
        const parts = [_][]const u8{ ".local", "share", component };
        try checkAncestor(current, uid);
        for (parts, 0..) |part, i| {
            const leaf = i + 1 == parts.len;
            current.createDir(io, part, .fromMode(if (leaf) 0o700 else 0o755)) catch |e| switch (e) {
                error.PathAlreadyExists => {},
                else => return error.Unavailable,
            };
            const next = current.openDir(io, part, .{ .follow_symlinks = false }) catch |e| switch (e) {
                error.SymLinkLoop, error.NotDir => return error.UnsafeDirectory,
                else => return error.Unavailable,
            };
            current.close(io);
            current = next;
            try checkAncestor(current, uid);
            if (leaf) {
                const m = try ownedMode(current.handle);
                if (m.mode != 0o700) return error.UnsafeDirectory;
            }
        }
        return .{ .dir = current, .io = io };
    }

    fn checkAncestor(d: Io.Dir, uid: std.c.uid_t) DirError!void {
        const m = try ownedMode(d.handle);
        if (m.uid != uid or !m.dir or m.mode & 0o022 != 0) return error.UnsafeDirectory;
    }

    pub fn close(p: *PrivateDir) void {
        p.dir.close(p.io);
    }

    /// Write `name.tmp-<pid>` at 0600, sync, rename over `name`. A failure
    /// removes only the temp file and leaves the previous document intact.
    pub fn publish(p: *PrivateDir, name: []const u8, bytes: []const u8) !void {
        var tmp_buf: [64]u8 = undefined;
        const tmp = try std.fmt.bufPrint(&tmp_buf, ".{s}.tmp-{d}", .{ name, std.c.getpid() });
        {
            var f = try p.dir.createFile(p.io, tmp, .{ .exclusive = true, .permissions = .fromMode(0o600) });
            defer f.close(p.io);
            errdefer p.dir.deleteFile(p.io, tmp) catch {};
            const m = try ownedMode(f.handle);
            if (m.mode != 0o600 or !m.reg) return error.UnsafeDirectory;
            // std/Io/File.zig: writeStreamingAll, sync; std/Io/Dir.zig: rename.
            try f.writeStreamingAll(p.io, bytes);
            try f.sync(p.io);
        }
        errdefer p.dir.deleteFile(p.io, tmp) catch {};
        try p.dir.rename(tmp, p.dir, name, p.io);
    }

    pub fn remove(p: *PrivateDir, name: []const u8) void {
        p.dir.deleteFile(p.io, name) catch {};
    }
};

const testing = std.testing;

fn fixedIdentity() Identity {
    return Identity.fromParts(4242, @splat(0xab), @splat(0x01)) catch unreachable;
}

test "readiness bytes are the oracle's canonical v1 encoding" {
    var buf: [readiness_max_bytes]u8 = undefined;
    const got = try encodeReadiness(&buf, fixedIdentity(), .{}, 1_700_000_000_000, .{});
    const id = fixedIdentity();
    const want = try std.fmt.allocPrint(testing.allocator, "{{\"schema_version\":1,\"pid\":4242,\"run_nonce\":\"{s}\",\"executable_sha256\":\"{s}\",\"phase\":\"starting\",\"published_at_unix_ms\":1700000000000,\"discord\":\"connecting\",\"scheduler\":\"starting\",\"telegram\":\"disabled\",\"slack\":\"disabled\",\"last_persistence\":\"not_attempted\"}}\n", .{ &id.nonce, &id.executable_sha256 });
    defer testing.allocator.free(want);
    try testing.expectEqualStrings(want, got);
    try testing.expectEqualStrings("abab", id.nonce[0..4]);
    try testing.expectEqualStrings("0101", id.executable_sha256[60..64]);
    var bbuf: [bootstrap_max_bytes]u8 = undefined;
    const boot = try encodeBootstrap(&bbuf, fixedIdentity(), .starting, .none);
    try testing.expect(std.mem.endsWith(u8, boot, "\"phase\":\"starting\",\"code\":\"none\"}\n"));
}

test "ready is refused until every checkpoint and state agrees" {
    var buf: [readiness_max_bytes]u8 = undefined;
    const ready: State = .{ .phase = .ready, .discord = .ready, .scheduler = .running };
    try testing.expectError(error.NotReady, encodeReadiness(&buf, fixedIdentity(), ready, 1, .{ .scheduler_running = true, .discord_ready = true }));
    try testing.expectError(error.NotReady, encodeReadiness(&buf, fixedIdentity(), .{ .phase = .ready, .discord = .connecting, .scheduler = .running }, 1, .{ .scheduler_running = true, .discord_ready = true, .commands_registered = true }));
    _ = try encodeReadiness(&buf, fixedIdentity(), ready, 1, .{ .scheduler_running = true, .discord_ready = true, .commands_registered = true });
    try testing.expectError(error.TimeOutOfRange, encodeReadiness(&buf, fixedIdentity(), .{}, std.math.maxInt(u64), .{}));
    try testing.expectError(error.IdentityUnavailable, Identity.fromParts(0, @splat(0), @splat(0)));
}

test "private directory is created 0700 and documents are published 0600 by rename" {
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    const home = try tmp.dir.realPathFileAlloc(testing.io, ".", testing.allocator);
    defer testing.allocator.free(home);
    var p = try PrivateDir.open(testing.io, home);
    defer p.close();
    try p.publish(readiness_file, "one\n");
    try p.publish(readiness_file, "two\n");
    const st = try p.dir.statFile(testing.io, readiness_file, .{});
    try testing.expectEqual(@as(u32, 0o600), @as(u32, @intCast(st.permissions.toMode() & 0o7777)));
    var rbuf: [16]u8 = undefined;
    try testing.expectEqualStrings("two\n", try p.dir.readFile(testing.io, readiness_file, &rbuf));
    const dst = (try ownedMode(p.dir.handle)).mode;
    try testing.expectEqual(@as(u32, 0o700), dst);
    // A group-writable ancestor is refused.
    var bad = try tmp.dir.createDirPathOpen(testing.io, "loose", .{});
    defer bad.close(testing.io);
    try bad.setPermissions(testing.io, .fromMode(0o777));
    const bad_home = try tmp.dir.realPathFileAlloc(testing.io, "loose", testing.allocator);
    defer testing.allocator.free(bad_home);
    try testing.expectError(error.UnsafeDirectory, PrivateDir.open(testing.io, bad_home));
}

test "identity of the running test binary hashes the executable" {
    const id = try Identity.current(testing.io, testing.allocator);
    try testing.expect(id.pid > 0);
    for (id.executable_sha256) |c| try testing.expect(std.ascii.isHex(c) and !std.ascii.isUpper(c));
}
