//! Running `abi wdbx episode propose <write.json> --json --endpoint ...
//! --token-file ... [--ca-cert ...]` and classifying the result, transcribed
//! from the oracle's `episode_gate.rs` (`propose`, `classify`,
//! `parse_appended`, `first_line`) and `episode_gate/process.rs`. The write
//! file is created owner-only (0600) under TMPDIR and removed afterwards; the
//! bearer token stays in the file the config names and only abi reads it.
const std = @import("std");
const config = @import("config.zig");
const write = @import("write.zig");
const text = @import("../text/text.zig");
const Allocator = std.mem.Allocator;

pub const max_stdout: usize = 64 * 1024;
pub const max_stderr: usize = 16 * 1024;
const max_detail_chars = 240;
pub const allowed_environment = [_][]const u8{ "HOME", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE", "__CF_USER_TEXT_ENCODING" };

pub const Outcome = union(enum) {
    appended: struct { digest_hex: []const u8, sequence: []const u8 },
    rejected: []const u8,
    unavailable: []const u8,

    pub fn summary(o: Outcome, arena: Allocator) Allocator.Error![]const u8 {
        return switch (o) {
            .appended => |a| std.fmt.allocPrint(arena, "appended sequence={s} digest={s}", .{ a.sequence, a.digest_hex }),
            .rejected => |d| std.fmt.allocPrint(arena, "rejected: {s}", .{d}),
            .unavailable => |d| std.fmt.allocPrint(arena, "unavailable: {s}", .{d}),
        };
    }
};

/// First non-blank line of `bytes` (lossy UTF-8), trimmed, at most 240
/// scalars; "no diagnostic" when there is none.
fn firstLine(arena: Allocator, bytes: []const u8) Allocator.Error![]const u8 {
    const lossy = try lossyUtf8(arena, bytes);
    var it = std.mem.splitScalar(u8, lossy, '\n');
    while (it.next()) |raw| {
        const line = text.trim(std.mem.trimEnd(u8, raw, "\r"));
        if (line.len == 0) continue;
        return line[0..text.byteOffsetOfChar(line, max_detail_chars)];
    }
    return "no diagnostic";
}

fn lossyUtf8(arena: Allocator, bytes: []const u8) Allocator.Error![]const u8 {
    if (std.unicode.utf8ValidateSlice(bytes)) return bytes;
    var out: std.ArrayList(u8) = .empty;
    var it = text.iterate(bytes);
    while (it.next()) |s| try text.appendScalar(arena, &out, s.cp);
    return out.items;
}

/// Exit 0 with a JSON object is an append; exit 1 is the gateway's refusal;
/// anything else is unknown. `code` is null when a signal ended the child.
pub fn classify(arena: Allocator, code: ?u8, stdout: []const u8, stderr: []const u8) Allocator.Error!Outcome {
    const c = code orelse return .{ .unavailable = "the abi binary was killed by a signal" };
    return switch (c) {
        0 => parseAppended(arena, stdout),
        1 => .{ .rejected = try firstLine(arena, stderr) },
        else => .{ .unavailable = try std.fmt.allocPrint(arena, "the abi binary exited with status {d}: {s}", .{ c, try firstLine(arena, stderr) }) },
    };
}

fn parseAppended(arena: Allocator, stdout: []const u8) Allocator.Error!Outcome {
    const value = std.json.parseFromSliceLeaky(std.json.Value, arena, stdout, .{}) catch |e| switch (e) {
        error.OutOfMemory => return error.OutOfMemory,
        else => return .{ .unavailable = "the abi binary printed something other than one JSON object" },
    };
    const field = struct {
        fn f(v: std.json.Value, name: []const u8) ?[]const u8 {
            if (v != .object) return null;
            const x = v.object.get(name) orelse return null;
            return if (x == .string) x.string else null;
        }
    }.f;
    const decision = field(value, "decision");
    const digest_hex = field(value, "episode_digest");
    const sequence = field(value, "sequence");
    if (decision == null) return .{ .unavailable = "the abi binary's JSON lacked decision, episode_digest, or sequence" };
    if (digest_hex != null and sequence != null) {
        const lower_hex = for (digest_hex.?) |b| {
            if (!(std.ascii.isDigit(b) or (b >= 'a' and b <= 'f'))) break false;
        } else true;
        const seq_ok = if (std.fmt.parseInt(u64, sequence.?, 10)) |n| blk: {
            var buf: [24]u8 = undefined;
            break :blk std.mem.eql(u8, std.fmt.bufPrint(&buf, "{d}", .{n}) catch unreachable, sequence.?);
        } else |_| false;
        if (std.mem.eql(u8, decision.?, "appended") and config.parseDigest(digest_hex.?) != null and lower_hex and seq_ok) {
            return .{ .appended = .{ .digest_hex = digest_hex.?, .sequence = sequence.? } };
        }
    }
    return .{ .unavailable = try std.fmt.allocPrint(arena, "unexpected decision {s}", .{decision.?}) };
}

pub const Gate = struct {
    config: config.Config,
    nonce: std.atomic.Value(u64) = .init(0),
    appended: std.atomic.Value(u64) = .init(0),
    rejected: std.atomic.Value(u64) = .init(0),
    unavailable: std.atomic.Value(u64) = .init(0),
    ungated_forgets: std.atomic.Value(u64) = .init(0),

    pub fn nextNonce(g: *Gate) u64 {
        return g.nonce.fetchAdd(1, .monotonic);
    }

    fn count(g: *Gate, o: Outcome) void {
        const counter = switch (o) {
            .appended => &g.appended,
            .rejected => &g.rejected,
            .unavailable => &g.unavailable,
        };
        _ = counter.fetchAdd(1, .monotonic);
    }

    /// Serialize, write the owner-only file, run abi, classify, count.
    pub fn propose(g: *Gate, arena: Allocator, io: std.Io, parent_env: *const std.process.Environ.Map, w: *const write.Write) Allocator.Error!Outcome {
        const outcome = g.proposeInner(arena, io, parent_env, w) catch |e| switch (e) {
            error.OutOfMemory => return error.OutOfMemory,
            else => Outcome{ .unavailable = "could not start the abi binary" },
        };
        g.count(outcome);
        return outcome;
    }

    fn proposeInner(g: *Gate, arena: Allocator, io: std.Io, parent_env: *const std.process.Environ.Map, w: *const write.Write) !Outcome {
        var body: std.Io.Writer.Allocating = .init(arena);
        try w.render(&body.writer);
        const tmp_root = parent_env.get("TMPDIR") orelse "/tmp";
        var rnd: [8]u8 = undefined;
        io.random(&rnd);
        const path = try std.fmt.allocPrint(arena, "{s}/abbey-bot-zig-episode-{x}.json", .{ std.mem.trimEnd(u8, tmp_root, "/"), std.mem.readInt(u64, &rnd, .little) });
        const cwd = std.Io.Dir.cwd();
        // std/Io/Dir.zig: CreateFileOptions{ .exclusive, .permissions }
        cwd.writeFile(io, .{ .sub_path = path, .data = body.written(), .flags = .{ .exclusive = true, .permissions = .fromMode(0o600) } }) catch
            return .{ .unavailable = "could not create the write file" };
        defer cwd.deleteFile(io, path) catch {};

        var env = std.process.Environ.Map.init(arena);
        for (allowed_environment) |name| if (parent_env.get(name)) |v| try env.put(name, v);
        var argv: std.ArrayList([]const u8) = .empty;
        try argv.appendSlice(arena, &.{ g.config.abi_cli, "wdbx", "episode", "propose", path, "--json", "--endpoint", g.config.endpoint, "--token-file", g.config.token_file });
        if (g.config.ca_cert) |ca| try argv.appendSlice(arena, &.{ "--ca-cert", ca });
        const result = std.process.run(arena, io, .{
            .argv = argv.items,
            .environ_map = &env,
            .stdout_limit = .limited(max_stdout),
            .stderr_limit = .limited(max_stderr),
            .timeout = .{ .duration = .{ .raw = .fromSeconds(@intCast(g.config.timeout_secs)), .clock = .awake } },
        }) catch |e| return switch (e) {
            error.OutOfMemory => error.OutOfMemory,
            error.StreamTooLong => Outcome{ .unavailable = "the abi binary's output exceeded its limit" },
            error.Timeout => Outcome{ .unavailable = try std.fmt.allocPrint(arena, "the abi binary did not answer within {d}s", .{g.config.timeout_secs}) },
            else => Outcome{ .unavailable = "could not start the abi binary" },
        };
        return classify(arena, if (result.term == .exited) result.term.exited else null, result.stdout, result.stderr);
    }
};

const testing = std.testing;

test "propose runs abi with an owner-only write file, the configured flags and a scrubbed environment" {
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    var arena_state: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena_state.deinit();
    const arena = arena_state.allocator();
    const dir = try tmp.dir.realPathFileAlloc(testing.io, ".", arena);
    // The fake abi records argv, env, the write file's mode and contents,
    // then answers like the gateway.
    const script =
        \\#!/bin/sh
        \\here=$(dirname "$0")
        \\printf '%s\n' "$@" > "$here/argv.txt"
        \\/usr/bin/env > "$here/env.txt"
        \\/usr/bin/stat -f '%Lp' "$4" > "$here/mode.txt"
        \\/bin/cat "$4" > "$here/write.json"
        \\echo '{"decision":"appended","episode_digest":"abababababababababababababababababababababababababababababababab","sequence":"3"}'
        \\
    ;
    try tmp.dir.writeFile(testing.io, .{ .sub_path = "abi", .data = script, .flags = .{ .permissions = .fromMode(0o755) } });
    const abi = try std.fs.path.join(arena, &.{ dir, "abi" });
    const json = try std.fmt.allocPrint(arena, "{{\"abi_cli\":\"{s}\",\"endpoint\":\"http://127.0.0.1:50051\",\"token_file\":\"/opt/token\",\"policy_version\":\"policy_v1\",\"contract_revision\":2,\"contract_digest\":\"{s}\"}}", .{ abi, "0108151d242b323940474e555c636a71787f868d949ba2a9b0b7bec5ccd3dae1" });
    const parsed = try config.fromJson(arena, json);
    var gate: Gate = .{ .config = parsed.ok };
    const built = try write.learningToggle(arena, &gate.config, "discord:1", "discord:2", 5, gate.nextNonce());
    var env = std.process.Environ.Map.init(arena);
    try env.put("TMPDIR", dir);
    try env.put("DISCORD_TOKEN", "must-not-leak");
    const outcome = try gate.propose(arena, testing.io, &env, &built.ok);
    try testing.expectEqualStrings("3", outcome.appended.sequence);
    try testing.expectEqual(@as(u64, 1), gate.appended.load(.monotonic));
    const argv = try tmp.dir.readFileAlloc(testing.io, "argv.txt", arena, .limited(4096));
    try testing.expect(std.mem.startsWith(u8, argv, "wdbx\nepisode\npropose\n"));
    try testing.expect(std.mem.endsWith(u8, argv, "--json\n--endpoint\nhttp://127.0.0.1:50051\n--token-file\n/opt/token\n"));
    try testing.expectEqualStrings("600\n", try tmp.dir.readFileAlloc(testing.io, "mode.txt", arena, .limited(64)));
    var rendered: std.Io.Writer.Allocating = .init(arena);
    try built.ok.render(&rendered.writer);
    try testing.expectEqualStrings(rendered.written(), try tmp.dir.readFileAlloc(testing.io, "write.json", arena, .limited(8192)));
    const envs = try tmp.dir.readFileAlloc(testing.io, "env.txt", arena, .limited(64 * 1024));
    try testing.expect(std.mem.indexOf(u8, envs, "must-not-leak") == null);
    // The write file is removed after the call.
    var it = std.mem.splitScalar(u8, argv, '\n');
    for (0..3) |_| _ = it.next();
    const write_path = it.next().?;
    try testing.expect(std.mem.endsWith(u8, write_path, ".json"));
    try testing.expectError(error.FileNotFound, std.Io.Dir.cwd().statFile(testing.io, write_path, .{}));
}

test "a missing abi binary is unavailable, never an append" {
    var arena_state: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena_state.deinit();
    const arena = arena_state.allocator();
    const parsed = try config.fromJson(arena, "{\"abi_cli\":\"/nonexistent/abi\",\"endpoint\":\"http://127.0.0.1:1\",\"token_file\":\"/t\",\"policy_version\":\"p\",\"contract_revision\":1,\"contract_digest\":\"0108151d242b323940474e555c636a71787f868d949ba2a9b0b7bec5ccd3dae1\"}");
    var gate: Gate = .{ .config = parsed.ok };
    const built = try write.learningToggle(arena, &gate.config, "discord:1", "discord:2", 5, 0);
    var env = std.process.Environ.Map.init(arena);
    const outcome = try gate.propose(arena, testing.io, &env, &built.ok);
    try testing.expect(outcome == .unavailable);
    try testing.expectEqual(@as(u64, 1), gate.unavailable.load(.monotonic));
}
