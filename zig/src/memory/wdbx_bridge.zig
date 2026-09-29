//! Vector retrieval through the `abi` binary (decision 1: abi is consumed
//! only as a subprocess). `abi wdbx query <segment> <text> --limit N --json`
//! embeds the query with the same 32-dimension embedding and scores every
//! vector; this module keeps only the caller's guild+user scope and orders by
//! the reported `semantic` score, then id, the oracle's cosine ranking.
//!
//! The child gets an allowlisted environment (the oracle's episode-gate
//! ALLOWED_ENVIRONMENT: HOME, TMPDIR, LANG, LC_ALL, LC_CTYPE,
//! __CF_USER_TEXT_ENCODING), never the bot's credentials; stdout is capped and
//! the call is bounded by a timeout (std.process.run `timeout`).
const std = @import("std");
const seg = @import("wdbx_segment.zig");
const Allocator = std.mem.Allocator;

pub const allowed_environment = [_][]const u8{ "HOME", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE", "__CF_USER_TEXT_ENCODING" };
pub const max_stdout: usize = 256 * 1024;

pub const Error = error{ AbiUnavailable, AbiFailed, AbiOutputInvalid } || Allocator.Error;

pub const Bridge = struct {
    abi_cli: []const u8,
    timeout_ms: u64 = 15_000,

    fn childEnv(gpa: Allocator, parent: *const std.process.Environ.Map) Allocator.Error!std.process.Environ.Map {
        var env = std.process.Environ.Map.init(gpa);
        errdefer env.deinit();
        for (allowed_environment) |name| {
            if (parent.get(name)) |v| try env.put(name, v);
        }
        return env;
    }

    pub const Hit = struct { id: u64, semantic: f64 };

    fn pickJson(stdout: []const u8, stderr: []const u8) ?[]const u8 {
        for ([_][]const u8{ stdout, stderr }) |candidate| {
            const trimmed = std.mem.trim(u8, candidate, " \t\r\n");
            if (trimmed.len > 1 and trimmed[0] == '{') return trimmed;
        }
        return null;
    }

    /// Raw scored hits for `text` over every vector in the segment.
    pub fn query(b: Bridge, arena: Allocator, io: std.Io, parent_env: *const std.process.Environ.Map, segment_path: []const u8, text: []const u8, limit: usize) Error![]Hit {
        var env = try childEnv(arena, parent_env);
        defer env.deinit();
        var limit_buf: [24]u8 = undefined;
        const limit_text = std.fmt.bufPrint(&limit_buf, "{d}", .{@max(limit, 1)}) catch unreachable;
        // std/process.zig: run(gpa, io, RunOptions{ argv, environ_map, stdout_limit, timeout })
        const result = std.process.run(arena, io, .{
            .argv = &.{ b.abi_cli, "wdbx", "query", segment_path, text, "--limit", limit_text, "--json" },
            .environ_map = &env,
            .stdout_limit = .limited(max_stdout),
            .stderr_limit = .limited(16 * 1024),
            .timeout = .{ .duration = .{ .raw = .fromMilliseconds(@intCast(b.timeout_ms)), .clock = .awake } },
        }) catch |e| return switch (e) {
            error.OutOfMemory => error.OutOfMemory,
            else => error.AbiUnavailable,
        };
        if (result.term != .exited or result.term.exited != 0) return error.AbiFailed;
        const Reply = struct {
            results: []const struct { id: u64, semantic: f64 },
        };
        // Measured 2026-09-22 with the installed abi: `wdbx query --json`
        // writes its JSON object to stderr when stdout is not a terminal, so
        // take whichever stream carries the object.
        const payload = pickJson(result.stdout, result.stderr) orelse return error.AbiOutputInvalid;
        const parsed = std.json.parseFromSliceLeaky(Reply, arena, payload, .{ .ignore_unknown_fields = true, .allocate = .alloc_always }) catch return error.AbiOutputInvalid;
        const hits = try arena.alloc(Hit, parsed.results.len);
        for (parsed.results, hits) |r, *h| h.* = .{ .id = r.id, .semantic = r.semantic };
        return hits;
    }

    /// Scoped recall: facts in `guild` belonging to `user`, best first.
    pub fn recallForUser(b: Bridge, arena: Allocator, io: std.Io, parent_env: *const std.process.Environ.Map, recall: *const seg.Recall, segment_path: []const u8, guild: []const u8, user: []const u8, text: []const u8, k: usize) Error![]seg.Recalled {
        const total = recall.store.vectors.items.len;
        if (total == 0 or k == 0) return &.{};
        const hits = try b.query(arena, io, parent_env, segment_path, text, total);
        var out: std.ArrayList(seg.Recalled) = .empty;
        for (hits) |h| {
            const f = recall.fact(arena, guild, h.id, @floatCast(h.semantic)) orelse continue; // other guilds' keys never match
            if (!std.mem.eql(u8, f.user, user)) continue;
            try out.append(arena, f);
        }
        std.mem.sort(seg.Recalled, out.items, {}, struct {
            fn lt(_: void, x: seg.Recalled, y: seg.Recalled) bool {
                if (x.score != y.score) return x.score > y.score;
                return x.id < y.id;
            }
        }.lt);
        return out.items[0..@min(k, out.items.len)];
    }
};

const testing = std.testing;

/// Writes an executable fake `abi` that records its argv and environment and
/// prints a canned query reply.
fn writeFakeAbi(dir: std.Io.Dir, reply: []const u8) !void {
    var script: std.ArrayList(u8) = .empty;
    defer script.deinit(testing.allocator);
    try script.appendSlice(testing.allocator, "#!/bin/sh\nhere=$(dirname \"$0\")\nprintf '%s\\n' \"$@\" > \"$here/argv.txt\"\nenv > \"$here/env.txt\"\ncat <<'JSON'\n");
    try script.appendSlice(testing.allocator, reply);
    try script.appendSlice(testing.allocator, "\nJSON\n");
    try dir.writeFile(testing.io, .{ .sub_path = "abi", .data = script.items, .flags = .{ .permissions = .fromMode(0o755) } });
}

test "bridge runs abi wdbx query with a scrubbed environment and keeps only the caller's scope" {
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    var arena_state: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena_state.deinit();
    const arena = arena_state.allocator();
    var recall = seg.Recall.init(testing.allocator);
    defer recall.deinit();
    _ = try recall.remember("discord:1", "discord:2", "likes rust", 1); // id 1
    _ = try recall.remember("discord:1", "discord:3", "likes rust too", 2); // id 2, other user
    _ = try recall.remember("discord:9", "discord:2", "rust in another guild", 3); // id 3, other guild
    _ = try recall.remember("discord:1", "discord:2", "has a cat", 4); // id 4
    try writeFakeAbi(tmp.dir, "{\"results\":[{\"id\":2,\"score\":0.9,\"semantic\":0.9},{\"id\":3,\"semantic\":0.8},{\"id\":4,\"semantic\":0.1},{\"id\":1,\"semantic\":0.65}]}");
    const dir_path = try tmp.dir.realPathFileAlloc(testing.io, ".", arena);
    const abi_path = try std.fs.path.join(arena, &.{ dir_path, "abi" });
    var parent = std.process.Environ.Map.init(arena);
    try parent.put("HOME", "/tmp/home-for-test");
    try parent.put("DISCORD_TOKEN", "must-not-leak");
    try parent.put("OPENAI_API_KEY", "must-not-leak");
    const bridge: Bridge = .{ .abi_cli = abi_path };
    const segment = try std.fs.path.join(arena, &.{ dir_path, "wdbx.seg.0.jsonl" });
    const hits = try bridge.recallForUser(arena, testing.io, &parent, &recall, segment, "discord:1", "discord:2", "rust", 5);
    // Other users and other guilds never surface, whatever abi scored them.
    try testing.expectEqual(@as(usize, 2), hits.len);
    try testing.expectEqual(@as(u64, 1), hits[0].id);
    try testing.expectEqualStrings("likes rust", hits[0].text);
    try testing.expectEqual(@as(u64, 4), hits[1].id);
    // argv: no shell, the segment path and query text as separate arguments.
    const argv = try tmp.dir.readFileAlloc(testing.io, "argv.txt", arena, .limited(4096));
    try testing.expectEqualStrings(try std.fmt.allocPrint(arena, "wdbx\nquery\n{s}\nrust\n--limit\n4\n--json\n", .{segment}), argv);
    // env: only the allowlist reaches the child.
    const env = try tmp.dir.readFileAlloc(testing.io, "env.txt", arena, .limited(64 * 1024));
    try testing.expect(std.mem.indexOf(u8, env, "HOME=/tmp/home-for-test") != null);
    try testing.expect(std.mem.indexOf(u8, env, "must-not-leak") == null);
}
