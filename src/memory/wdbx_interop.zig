//! `abbey-bot-zig wdbx-interop <abi>`: write a Zig-rendered fact segment into
//! a private temp directory, query it through the real `abi` binary, and
//! require abi's `semantic` scores to equal the in-process cosine (to abi's
//! printed precision) and the scoped ranking to match. The child's HOME and
//! TMPDIR are the temp directory, so abi cannot touch the operator's ~/.abi.
const std = @import("std");
const seg = @import("wdbx_segment.zig");
const bridge_mod = @import("wdbx_bridge.zig");
const embedding = @import("embedding.zig");

pub const Report = struct { vectors: usize, compared: usize, max_abs_diff: f64 };

pub fn run(gpa: std.mem.Allocator, io: std.Io, abi_cli: []const u8, tmp_root: []const u8) !Report {
    var arena_state: std.heap.ArenaAllocator = .init(gpa);
    defer arena_state.deinit();
    const arena = arena_state.allocator();

    var rnd: [8]u8 = undefined;
    io.random(&rnd);
    const dir_path = try std.fmt.allocPrint(arena, "{s}/abbey-bot-zig-interop-{x}", .{ std.mem.trimEnd(u8, tmp_root, "/"), std.mem.readInt(u64, &rnd, .little) });
    const cwd = std.Io.Dir.cwd();
    try cwd.createDirPath(io, dir_path);
    defer cwd.deleteTree(io, dir_path) catch {};

    var recall = seg.Recall.init(gpa);
    defer recall.deinit();
    const facts = [_][3][]const u8{
        .{ "discord:1", "discord:2", "likes rust and zig" },
        .{ "discord:1", "discord:2", "has a cat named Miso" },
        .{ "discord:1", "discord:3", "likes rust too" },
        .{ "discord:dm:2", "discord:2", "prefers short answers" },
        .{ "discord:1", "discord:2", "deploys on Fridays" },
    };
    for (facts, 0..) |f, i| _ = try recall.remember(f[0], f[1], f[2], 1000 + i);
    const rendered = try recall.store.render(arena);
    const segment = try std.fmt.allocPrint(arena, "{s}/wdbx.seg.0.jsonl", .{dir_path});
    try cwd.writeFile(io, .{ .sub_path = segment, .data = rendered });

    var env = std.process.Environ.Map.init(arena);
    try env.put("HOME", dir_path);
    try env.put("TMPDIR", dir_path);
    const b: bridge_mod.Bridge = .{ .abi_cli = abi_cli };
    const query = "rust";
    const hits = try b.query(arena, io, &env, segment, query, facts.len);
    if (hits.len != facts.len) return error.InteropVectorCount;
    const q = embedding.textEmbedding(query);
    var max_diff: f64 = 0;
    for (hits) |h| {
        const v = recall.store.vector(h.id) orelse return error.InteropUnknownId;
        const local: f64 = embedding.cosine(&q, v);
        max_diff = @max(max_diff, @abs(local - h.semantic));
    }
    // abi prints six decimals.
    if (max_diff > 0.0000005 + 1e-9) return error.InteropScoreMismatch;
    const scoped = try b.recallForUser(arena, io, &env, &recall, segment, "discord:1", "discord:2", query, 3);
    const in_process = try recall.recallForUser(arena, "discord:1", "discord:2", query, 3);
    if (scoped.len != in_process.len) return error.InteropRankingMismatch;
    for (scoped, in_process) |x, y| if (x.id != y.id) return error.InteropRankingMismatch;
    return .{ .vectors = facts.len, .compared = hits.len, .max_abs_diff = max_diff };
}
