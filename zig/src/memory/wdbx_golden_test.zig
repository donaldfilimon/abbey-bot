//! Replays contracts/golden/wdbx.json: embedding bits and their JSON text,
//! cosine bits, segment renders after remember/forget sequences, per-user
//! recall ranking, reconciliation, and fixture parse/render identity.
const std = @import("std");
const golden = @import("../testing/golden.zig");
const embedding = @import("embedding.zig");
const seg = @import("wdbx_segment.zig");
const ryu_style = @import("../text/ryu_style.zig");

test "wdbx golden: embeddings match the oracle bit for bit and render as serde_json does" {
    const gpa = std.testing.allocator;
    var parsed = try golden.parse(gpa, @embedFile("golden_wdbx"));
    defer parsed.deinit();
    for (golden.items(parsed.value, "embeddings")) |row| {
        const e = embedding.textEmbedding(golden.str(row, "input"));
        for (golden.items(row, "bits"), e) |want, got| try std.testing.expectEqual(@as(u32, @intCast(want.integer)), @as(u32, @bitCast(got)));
        var out: std.Io.Writer.Allocating = .init(gpa);
        defer out.deinit();
        try out.writer.writeByte('[');
        for (e, 0..) |x, i| {
            if (i > 0) try out.writer.writeByte(',');
            try ryu_style.writeF32(&out.writer, x);
        }
        try out.writer.writeByte(']');
        try std.testing.expectEqualStrings(golden.str(row, "json"), out.written());
    }
    for (golden.items(parsed.value, "cosines")) |row| {
        const a = embedding.textEmbedding(golden.str(row, "a"));
        const b = embedding.textEmbedding(golden.str(row, "b"));
        try std.testing.expectEqual(@as(u32, @intCast(golden.int(row, "bits"))), @as(u32, @bitCast(embedding.cosine(&a, &b))));
    }
}

test "wdbx golden: segment renders, recall ranking and reconciliation match the oracle" {
    const gpa = std.testing.allocator;
    var parsed = try golden.parse(gpa, @embedFile("golden_wdbx"));
    defer parsed.deinit();
    const steps = golden.items(parsed.value, "steps");
    var r = seg.Recall.init(gpa);
    defer r.deinit();
    const a = try r.remember("discord:1", "discord:2", "likes rust", 1000);
    const b = try r.remember("discord:1", "discord:3", "likes zig", 1001);
    const c = try r.remember("discord:dm:2", "discord:2", "has a cat named Miso", 1002);
    try std.testing.expectEqual(@as(u64, 1), a);
    try std.testing.expectEqual(@as(u64, 3), c);
    try expectRender(gpa, &r, golden.str(steps[0], "render"));
    try std.testing.expect(r.forget("discord:1", b));
    try expectRender(gpa, &r, golden.str(steps[1], "render"));
    try std.testing.expect(!r.forget("discord:1", b));
    _ = try r.remember("discord:1", "discord:2", "writes \"quoted\" text\nwith newline", 1003);
    try expectRender(gpa, &r, golden.str(steps[2], "render"));

    for (golden.items(parsed.value, "queries")) |q| {
        var arena: std.heap.ArenaAllocator = .init(gpa);
        defer arena.deinit();
        const hits = try r.recallForUser(arena.allocator(), golden.str(q, "guild"), golden.str(q, "user"), golden.str(q, "query"), 5);
        const want = golden.items(q, "hits");
        try std.testing.expectEqual(want.len, hits.len);
        for (want, hits) |w, h| {
            try std.testing.expectEqual(@as(u64, @intCast(golden.int(w, "id"))), h.id);
            try std.testing.expectEqualStrings(golden.str(w, "text"), h.text);
            try std.testing.expectEqual(@as(u32, @intCast(golden.int(w, "score_bits"))), @as(u32, @bitCast(h.score)));
        }
    }

    var rec = seg.Recall.init(gpa);
    defer rec.deinit();
    _ = try rec.remember("discord:1", "discord:2", "stale", 5);
    var arena: std.heap.ArenaAllocator = .init(gpa);
    defer arena.deinit();
    try rec.reconcile(arena.allocator(), &.{
        .{ .guild = "discord:1", .user = "discord:2", .text = "likes rust", .at = 1000 },
        .{ .guild = "discord:1", .user = "discord:2", .text = "second", .at = 1001 },
    });
    try expectRender(gpa, &rec, golden.str(parsed.value, "reconciled_render"));
}

fn expectRender(gpa: std.mem.Allocator, r: *const seg.Recall, want: []const u8) !void {
    const got = try r.store.render(gpa);
    defer gpa.free(got);
    try std.testing.expectEqualStrings(want, got);
}

test "the conformance fixture parses and re-renders byte-identically" {
    const gpa = std.testing.allocator;
    const fixture = @embedFile("golden_wdbx_fixture");
    var store = try seg.Store.parse(gpa, fixture);
    defer store.deinit();
    const out = try store.render(gpa);
    defer gpa.free(out);
    try std.testing.expectEqualStrings(fixture, out);
    try std.testing.expectEqual(@as(u64, 3), store.next_id);
    try std.testing.expectError(error.MissingHeader, seg.Store.parse(gpa, "# not wdbx\n"));
    try std.testing.expectError(error.MalformedLine, seg.Store.parse(gpa, "# ABI-WDBX v1\n{\"type\":\"kv\",\"key\":1}\n"));
    try std.testing.expectError(error.DuplicateVector, seg.Store.parse(gpa, "# ABI-WDBX v1\n{\"type\":\"vector\",\"id\":1,\"values\":[]}\n{\"type\":\"vector\",\"id\":1,\"values\":[]}\n"));
}
