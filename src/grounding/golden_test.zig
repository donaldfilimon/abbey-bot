//! Replays contracts/golden/grounding.json: 528 reply/source combinations
//! and the recall selection matrix. Rows where the oracle panicked (a digit
//! run followed by a multi-byte letter, "12\u{e9}") are asserted to complete
//! without error here; see grounding.zig's classify note.
const std = @import("std");
const golden = @import("../testing/golden.zig");
const grounding = @import("grounding.zig");
const recall = @import("../memory/recall.zig");

fn expectSpecifics(want: []std.json.Value, got: []const grounding.Specific, indices: ?[]const usize) !void {
    const n = if (indices) |ix| ix.len else got.len;
    try std.testing.expectEqual(want.len, n);
    for (want, 0..) |w, i| {
        const s = if (indices) |ix| got[ix[i]] else got[i];
        const triple = w.array.items;
        try std.testing.expectEqualStrings(triple[0].string, @tagName(s.kind));
        try std.testing.expectEqualStrings(triple[1].string, s.text);
        try std.testing.expectEqualStrings(triple[2].string, s.key);
    }
}

test "grounding golden: specifics, verdicts and hedged replies match the oracle" {
    const gpa = std.testing.allocator;
    var parsed = try golden.parse(gpa, @embedFile("golden_grounding"));
    defer parsed.deinit();
    var checked: usize = 0;
    var panicked: usize = 0;
    for (golden.items(parsed.value, "rows")) |row| {
        const reply = golden.str(row, "reply");
        const src_values = golden.items(row, "sources");
        const sources = try gpa.alloc([]const u8, src_values.len);
        defer gpa.free(sources);
        for (src_values, sources) |v, *s| s.* = v.string;
        var g = try grounding.fromSources(gpa, sources);
        defer g.deinit(gpa);
        var v = try grounding.check(gpa, reply, &g);
        defer v.deinit(gpa);
        const hedged = try grounding.hedged(gpa, reply, &v);
        defer gpa.free(hedged);
        if (row.object.get("panic") != null) {
            panicked += 1;
            continue;
        }
        expectSpecifics(golden.items(row, "examined"), v.examined.items, null) catch |e| {
            std.debug.print("examined mismatch: reply={f} mode={s}\n", .{ std.json.fmt(reply, .{}), golden.str(row, "mode") });
            return e;
        };
        expectSpecifics(golden.items(row, "ungrounded"), v.examined.items, v.ungrounded.items) catch |e| {
            std.debug.print("ungrounded mismatch: reply={f} mode={s}\n", .{ std.json.fmt(reply, .{}), golden.str(row, "mode") });
            return e;
        };
        try std.testing.expectEqual(golden.boolean(row, "grounding_empty"), v.grounding_empty);
        try std.testing.expectEqualStrings(golden.str(row, "hedged"), hedged);
        checked += 1;
    }
    try std.testing.expect(checked >= 500);
    try std.testing.expectEqual(@as(usize, 6), panicked);
}

test "recall golden: selection order and omitted counts match the oracle" {
    const gpa = std.testing.allocator;
    var parsed = try golden.parse(gpa, @embedFile("golden_grounding"));
    defer parsed.deinit();
    const sets = golden.items(parsed.value, "recall_sets");
    var n: usize = 0;
    for (golden.items(parsed.value, "recall")) |row| {
        const set = sets[@intCast(golden.int(row, "set"))].array.items;
        const facts = try gpa.alloc([]const u8, set.len);
        defer gpa.free(facts);
        for (set, facts) |v, *f| f.* = v.string;
        var sel = try recall.select(gpa, facts, golden.str(row, "query"), @intCast(golden.int(row, "max")), @intCast(golden.int(row, "budget")));
        defer sel.deinit(gpa);
        const want = golden.items(row, "chosen");
        try std.testing.expectEqual(want.len, sel.chosen.items.len);
        for (want, sel.chosen.items) |w, got| try std.testing.expectEqual(@as(usize, @intCast(w.integer)), got);
        try std.testing.expectEqual(@as(usize, @intCast(golden.int(row, "omitted"))), sel.omitted);
        n += 1;
    }
    try std.testing.expect(n >= 190);
}
