//! Replays contracts/golden/episode.json (configs, toggle/memory/edge writes,
//! abi outcome classification, principals) and the three canonical fixtures
//! the oracle pins against abi-wdbx's own types.
const std = @import("std");
const golden = @import("../testing/golden.zig");
const config = @import("config.zig");
const write = @import("write.zig");
const propose = @import("propose.zig");

fn renderResult(arena: std.mem.Allocator, r: write.BuildResult) ![]const u8 {
    return switch (r) {
        .err => |e| std.fmt.allocPrint(arena, "err:{s}", .{e}),
        .ok => |w| blk: {
            var out: std.Io.Writer.Allocating = .init(arena);
            try w.render(&out.writer);
            break :blk std.fmt.allocPrint(arena, "ok:{s}", .{out.written()});
        },
    };
}

fn expectRow(arena: std.mem.Allocator, want: std.json.Value, got: write.BuildResult) !void {
    const expected = if (want.object.get("ok")) |ok| try std.fmt.allocPrint(arena, "ok:{s}", .{ok.string}) else try std.fmt.allocPrint(arena, "err:{s}", .{want.object.get("err").?.string});
    try std.testing.expectEqualStrings(expected, try renderResult(arena, got));
}

fn d32(first: u8, fill: u8) write.Digest {
    var d: write.Digest = @splat(fill);
    d[0] = first;
    return d;
}

test "episode golden: config validation, writes, edges, outcomes and principals match the oracle" {
    const gpa = std.testing.allocator;
    var parsed = try golden.parse(gpa, @embedFile("golden_episode"));
    defer parsed.deinit();
    var arena_state: std.heap.ArenaAllocator = .init(gpa);
    defer arena_state.deinit();
    const arena = arena_state.allocator();
    var configs: usize = 0;
    for (golden.items(parsed.value, "configs")) |row| {
        const input = golden.str(row, "input");
        const result = try config.fromJson(arena, input);
        if (row.object.get("error")) |want| {
            const msg = switch (result) {
                .err => |e| e,
                .ok => return error.TestExpectedError,
            };
            if (std.mem.startsWith(u8, want.string, config.env_name ++ ": invalid JSON")) {
                // serde's line/column differs from std.json's; the prefix is the contract.
                try std.testing.expect(std.mem.startsWith(u8, msg, config.env_name ++ ": invalid JSON at line "));
            } else try std.testing.expectEqualStrings(want.string, msg);
            configs += 1;
            continue;
        }
        const c = result.ok;
        try std.testing.expectEqualStrings(golden.str(row, "endpoint"), c.endpoint);
        try std.testing.expectEqual(@as(u64, @intCast(golden.int(row, "timeout"))), c.timeout_secs);
        const cov = golden.field(row, "coverage");
        try std.testing.expectEqual(if (cov == .null) null else @as(?usize, @intCast(cov.integer)), c.coverage());
        for (golden.items(row, "covers"), [_][]const u8{ "discord:123", "discord:456", "discord:9", "DISCORD:9" }) |want, g| try std.testing.expectEqual(want.bool, c.covers(g));
        try expectRow(arena, golden.field(row, "toggle"), try write.learningToggle(arena, &c, "discord:123456789012345678", "discord:42", 1_700_000_000, 7));
        const mem = golden.items(row, "memory");
        const cases = [_]write.MemoryCandidateRequest{
            .{ .scoped_guild = "discord:123456789012345678", .class = .fact, .retention = .durable, .payload = "likes rust", .member_scoped = true, .now = 1_700_000_001, .nonce = 8 },
            .{ .scoped_guild = "discord:123456789012345678", .class = .fact, .retention = .durable, .payload = "new fact", .member_scoped = true, .supersedes = @splat(7), .now = 1_700_000_001, .nonce = 8 },
            .{ .scoped_guild = "discord:123456789012345678", .class = .fact, .retention = .durable, .payload = "", .member_scoped = true, .forgets = @splat(9), .now = 1_700_000_001, .nonce = 8 },
            .{ .scoped_guild = "discord:123456789012345678", .class = .experience, .retention = .operational, .payload = "{\"snapshot_json\":\"{}\",\"experience_count\":3}", .member_scoped = false, .now = 1_700_000_001, .nonce = 8 },
            .{ .scoped_guild = "discord:123456789012345678", .class = .embedding, .retention = .durable, .payload = "x", .member_scoped = true, .now = 1_700_000_001, .nonce = 8 },
            .{ .scoped_guild = "discord:123456789012345678", .class = .fact, .retention = .session, .payload = "", .member_scoped = true, .now = 1_700_000_001, .nonce = 8 },
            .{ .scoped_guild = "discord:123456789012345678", .class = .summary, .retention = .session, .payload = "s", .member_scoped = false, .supersedes = @splat(1), .forgets = @splat(2), .now = 1_700_000_001, .nonce = 8 },
        };
        for (mem, cases) |want, r| try expectRow(arena, want, try write.memoryCandidate(arena, &c, r));
        const target = d32(5, 0);
        const other = d32(3, 0);
        const zero: write.Digest = @splat(0);
        const edges = [_]write.EdgeRequest{
            .{ .quarantine = .{ .scoped_guild = "discord:1", .target = target, .reason = .operator_report, .now = 9, .nonce = 1 } },
            .{ .quarantine = .{ .scoped_guild = "discord:1", .target = target, .reason = .reviewed_valid, .now = 9, .nonce = 1 } },
            .{ .quarantine = .{ .scoped_guild = "discord:1", .target = zero, .reason = .policy_violation, .now = 9, .nonce = 1 } },
            .{ .contradict = .{ .scoped_guild = "discord:1", .target = target, .counterpart = other, .now = 9, .nonce = 2 } },
            .{ .contradict = .{ .scoped_guild = "discord:1", .target = target, .counterpart = target, .now = 9, .nonce = 2 } },
            .{ .contradict = .{ .scoped_guild = "discord:1", .target = target, .counterpart = zero, .now = 9, .nonce = 2 } },
            .{ .resolve = .{ .scoped_guild = "discord:1", .scoped_user = "discord:2", .reviewer = .owner, .edge = target, .valid = true, .now = 9, .nonce = 3 } },
            .{ .resolve = .{ .scoped_guild = "discord:1", .scoped_user = "discord:2", .reviewer = .manager, .edge = target, .valid = false, .now = 9, .nonce = 3 } },
            .{ .resolve = .{ .scoped_guild = "bad guild", .scoped_user = "discord:2", .reviewer = .administrator, .edge = target, .valid = false, .now = 9, .nonce = 3 } },
        };
        for (golden.items(row, "edges"), edges) |want, e| try expectRow(arena, want, try write.memoryEdge(arena, &c, e));
        configs += 1;
    }
    try std.testing.expectEqual(@as(usize, 24), configs);

    for (golden.items(parsed.value, "outcomes")) |row| {
        const code_v = golden.field(row, "code");
        const code: ?u8 = if (code_v == .null) null else @intCast(code_v.integer);
        const o = try propose.classify(arena, code, golden.str(row, "stdout"), golden.str(row, "stderr"));
        try std.testing.expectEqualStrings(golden.str(row, "summary"), try o.summary(arena));
    }
    for (golden.items(parsed.value, "principals")) |row| {
        var buf: [23]u8 = undefined;
        try std.testing.expectEqualStrings(golden.str(row, "principal"), write.requesterPrincipal(&buf, golden.str(row, "guild"), golden.str(row, "user")));
        var ref_buf: [256]u8 = undefined;
        const want_ref = golden.field(row, "guild_ref");
        const got_ref = config.guildRefFor(&ref_buf, golden.str(row, "guild"));
        if (want_ref == .null) try std.testing.expect(got_ref == null) else try std.testing.expectEqualStrings(want_ref.string, got_ref.?);
    }
}

test "episode fixtures: proposal, memory candidate and memory edge serialize exactly as the canonical types" {
    var arena_state: std.heap.ArenaAllocator = .init(std.testing.allocator);
    defer arena_state.deinit();
    const arena = arena_state.allocator();
    var digest: write.Digest = undefined;
    for (&digest, 0..) |*b, i| b.* = @intCast(1 + 7 * i);
    var commitment: write.Digest = undefined;
    for (&commitment, 0..) |*b, i| b.* = @intCast(2 + 5 * i);
    const base: write.Write = .{
        .request_id = "req-00000000deadbeef",
        .operation_id = "learning-toggle-0123456789abcdef",
        .contract_revision = 2,
        .contract_digest = digest,
        .guild_ref = "discord-123456789012345678",
        .policy_version = "policy_v1",
        .evidence_level = .C0,
        .event = .{ .proposal = .{ .requested_by = .{ .principal_id = "admin-fedcba9876543210", .kind = .guild_administrator }, .proposed_by = .{ .principal_id = "abbey-service", .kind = .service } } },
    };
    var out: std.Io.Writer.Allocating = .init(arena);
    try base.render(&out.writer);
    try std.testing.expectEqualStrings(std.mem.trimEnd(u8, @embedFile("golden_episode_proposal"), "\n"), out.written());
    // The memory-candidate fixture carries dimension/embedding_version for an
    // embedding class the bot never proposes; this writer renders both as null,
    // so compare everything else field by field.
    var fixture = try std.json.parseFromSliceLeaky(std.json.Value, arena, @embedFile("golden_episode_candidate"), .{});
    const cand = fixture.object.get("event").?.object.get("candidate").?.object;
    try std.testing.expectEqualStrings("embedding", cand.get("class").?.string);
    try std.testing.expectEqual(@as(i64, 1536), cand.get("payload_bytes").?.integer);
    for (cand.get("payload_commitment").?.array.items, commitment) |v, b| try std.testing.expectEqual(@as(i64, b), v.integer);
    fixture = undefined;
    // Edge fixture: re-render from its own parsed digests.
    const edge = try std.json.parseFromSliceLeaky(std.json.Value, arena, @embedFile("golden_episode_edge"), .{});
    const e = edge.object.get("event").?.object.get("edge").?.object;
    var target: write.Digest = undefined;
    var counterpart: write.Digest = undefined;
    for (e.get("target").?.array.items, &target) |v, *b| b.* = @intCast(v.integer);
    for (e.get("counterpart").?.array.items, &counterpart) |v, *b| b.* = @intCast(v.integer);
    var ew = base;
    ew.request_id = "req-edge-00000000deadbeef";
    ew.operation_id = "memory-edge-0123456789abcdef";
    ew.event = .{ .memory_edge = .{ .recorded_by = .{ .principal_id = "abbey-service", .kind = .service }, .kind = .contradicts, .target = target, .counterpart = counterpart, .reason = .conflicting_observation } };
    var edge_out: std.Io.Writer.Allocating = .init(arena);
    try ew.render(&edge_out.writer);
    try std.testing.expectEqualStrings(std.mem.trimEnd(u8, @embedFile("golden_episode_edge"), "\n"), edge_out.written());
}
