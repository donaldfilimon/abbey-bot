//! Checkpoint gate: "only an admitted checkpoint is persisted", transcribed
//! from the oracle's `src/checkpoint_gate.rs` as pure functions over opaque
//! rows `{snapshot_json, experience_count}` keyed by scoped guild. In the
//! oracle the rows are DQN brain snapshots; this rewrite has no learning loop
//! in phase 1, so nothing produces rows yet and the gate is wired to no
//! producer (claims: Partial).
const std = @import("std");
const propose = @import("propose.zig");
const config = @import("config.zig");
const Allocator = std.mem.Allocator;
const Sha256 = std.crypto.hash.sha2.Sha256;

pub const Row = struct { snapshot_json: []const u8, experience_count: u64 };

pub const Rows = std.StringArrayHashMapUnmanaged(Row);

pub const Admitted = struct { commitment: [32]u8, episode_digest: ?[32]u8, row: Row };

pub const AdmittedMap = std.StringArrayHashMapUnmanaged(Admitted);

pub const Proposal = struct { guild: []const u8, row: Row, payload: []const u8, commitment: [32]u8, supersedes: ?[32]u8 };

/// The canonical payload: serde's `{"snapshot_json":..,"experience_count":..}`.
pub fn payload(arena: Allocator, row: Row) Allocator.Error![]const u8 {
    var out: std.Io.Writer.Allocating = .init(arena);
    std.json.Stringify.value(row, .{}, &out.writer) catch return error.OutOfMemory;
    return out.written();
}

pub fn commitment(bytes: []const u8) [32]u8 {
    var d: [32]u8 = undefined;
    Sha256.hash(bytes, &d, .{});
    return d;
}

fn sortedKeys(arena: Allocator, keys: []const []const u8) Allocator.Error![]const []const u8 {
    const out = try arena.dupe([]const u8, keys);
    std.mem.sort([]const u8, out, {}, struct {
        fn lt(_: void, a: []const u8, b: []const u8) bool {
            return std.mem.lessThan(u8, a, b);
        }
    }.lt);
    return out;
}

/// Rows loaded before any gate decision count as admitted, with no digest.
pub fn seed(arena: Allocator, rows: *const Rows) Allocator.Error!AdmittedMap {
    var out: AdmittedMap = .empty;
    for (rows.keys(), rows.values()) |g, r| try out.put(arena, g, .{ .commitment = commitment(try payload(arena, r)), .episode_digest = null, .row = r });
    return out;
}

/// Covered guilds whose row differs from the last admitted checkpoint, in
/// guild order (the oracle's BTreeMap order).
pub fn plan(arena: Allocator, rows: *const Rows, admitted: *const AdmittedMap, covers: anytype) Allocator.Error![]Proposal {
    var out: std.ArrayList(Proposal) = .empty;
    for (try sortedKeys(arena, rows.keys())) |g| {
        if (!covers.covers(g)) continue;
        const r = rows.get(g).?;
        const p = try payload(arena, r);
        const c = commitment(p);
        const previous = admitted.get(g);
        if (previous) |prev| if (std.mem.eql(u8, &prev.commitment, &c)) continue;
        try out.append(arena, .{ .guild = g, .row = r, .payload = p, .commitment = c, .supersedes = if (previous) |prev| prev.episode_digest else null });
    }
    return out.items;
}

pub const Settlement = struct { admitted: []const []const u8, refused: []const []const u8 };

fn substitute(arena: Allocator, rows: *Rows, admitted: *const AdmittedMap, guild: []const u8) Allocator.Error!void {
    if (admitted.get(guild)) |prev| {
        try rows.put(arena, guild, prev.row);
    } else _ = rows.orderedRemove(guild);
}

pub fn settle(arena: Allocator, rows: *Rows, admitted: *AdmittedMap, proposals: []const Proposal, outcomes: []const propose.Outcome) Allocator.Error!Settlement {
    var ok: std.ArrayList([]const u8) = .empty;
    var refused: std.ArrayList([]const u8) = .empty;
    for (proposals, outcomes) |p, o| switch (o) {
        .appended => |a| {
            try admitted.put(arena, p.guild, .{ .commitment = p.commitment, .episode_digest = config.parseDigest(a.digest_hex), .row = p.row });
            try ok.append(arena, p.guild);
        },
        .rejected, .unavailable => {
            try substitute(arena, rows, admitted, p.guild);
            try refused.append(arena, p.guild);
        },
    };
    return .{ .admitted = ok.items, .refused = refused.items };
}

/// The synchronous persist path proposes nothing: every covered row that is
/// not the admitted checkpoint is substituted.
pub fn restrictToAdmitted(arena: Allocator, rows: *Rows, admitted: *const AdmittedMap, covers: anytype) Allocator.Error![]const []const u8 {
    var changed: std.ArrayList([]const u8) = .empty;
    for (try sortedKeys(arena, rows.keys())) |g| {
        if (!covers.covers(g)) continue;
        const c = commitment(try payload(arena, rows.get(g).?));
        const same = if (admitted.get(g)) |prev| std.mem.eql(u8, &prev.commitment, &c) else false;
        if (!same) try changed.append(arena, g);
    }
    for (changed.items) |g| try substitute(arena, rows, admitted, g);
    return changed.items;
}

const testing = std.testing;
const All = struct {
    fn covers(_: All, _: []const u8) bool {
        return true;
    }
};
const Only = struct {
    guild: []const u8,
    fn covers(o: Only, g: []const u8) bool {
        return std.mem.eql(u8, o.guild, g);
    }
};

fn rowsOf(arena: Allocator, entries: []const struct { []const u8, Row }) !Rows {
    var rows: Rows = .empty;
    for (entries) |e| try rows.put(arena, e[0], e[1]);
    return rows;
}

fn expectNames(want: []const []const u8, got: anytype) !void {
    try testing.expectEqual(want.len, got.len);
    for (want, got) |w, g| try testing.expectEqualStrings(w, if (@TypeOf(g) == Proposal) g.guild else g);
}

test "checkpoints: unchanged rows are not re-proposed and changed rows supersede their receipt" {
    var arena_state: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena_state.deinit();
    const a = arena_state.allocator();
    const loaded = try rowsOf(a, &.{ .{ "g1", .{ .snapshot_json = "{\"a\":1}", .experience_count = 1 } }, .{ "g2", .{ .snapshot_json = "{\"b\":2}", .experience_count = 2 } } });
    var admitted = try seed(a, &loaded);
    try testing.expectEqual(@as(usize, 0), (try plan(a, &loaded, &admitted, All{})).len);
    var next = try loaded.clone(a);
    try next.put(a, "g1", .{ .snapshot_json = "{\"a\":2}", .experience_count = 3 });
    try next.put(a, "g3", .{ .snapshot_json = "{\"c\":1}", .experience_count = 1 });
    const proposals = try plan(a, &next, &admitted, All{});
    try expectNames(&.{ "g1", "g3" }, proposals);
    try testing.expect(proposals[0].supersedes == null);
    try testing.expectEqualStrings("{\"snapshot_json\":\"{\\\"a\\\":2}\",\"experience_count\":3}", proposals[0].payload);
    const digest = "abababababababababababababababababababababababababababababababab";
    const outcomes = [_]propose.Outcome{ .{ .appended = .{ .digest_hex = digest, .sequence = "1" } }, .{ .appended = .{ .digest_hex = digest, .sequence = "2" } } };
    const s = try settle(a, &next, &admitted, proposals, &outcomes);
    try expectNames(&.{ "g1", "g3" }, s.admitted);
    try testing.expectEqual(@as(?[32]u8, @splat(0xab)), admitted.get("g1").?.episode_digest);
    try next.put(a, "g1", .{ .snapshot_json = "{\"a\":3}", .experience_count = 4 });
    const again = try plan(a, &next, &admitted, All{});
    try testing.expectEqual(@as(usize, 1), again.len);
    try testing.expectEqual(@as(?[32]u8, @splat(0xab)), again[0].supersedes);
}

test "checkpoints: a refused checkpoint falls back to the last admitted row or is dropped" {
    var arena_state: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena_state.deinit();
    const a = arena_state.allocator();
    const loaded = try rowsOf(a, &.{.{ "g1", .{ .snapshot_json = "{\"a\":1}", .experience_count = 1 } }});
    var admitted = try seed(a, &loaded);
    var next = try loaded.clone(a);
    try next.put(a, "g1", .{ .snapshot_json = "{\"a\":2}", .experience_count = 2 });
    try next.put(a, "new", .{ .snapshot_json = "{\"n\":1}", .experience_count = 1 });
    const proposals = try plan(a, &next, &admitted, All{});
    const outcomes = [_]propose.Outcome{ .{ .rejected = "FailedPrecondition: episode_storage_budget_exhausted" }, .{ .unavailable = "timed out" } };
    const s = try settle(a, &next, &admitted, proposals, &outcomes);
    try expectNames(&.{ "g1", "new" }, s.refused);
    try testing.expectEqualStrings("{\"a\":1}", next.get("g1").?.snapshot_json);
    try testing.expect(next.get("new") == null);
    try testing.expect(admitted.get("new") == null);
}

test "checkpoints: the synchronous path persists only admitted rows and uncovered guilds are untouched" {
    var arena_state: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena_state.deinit();
    const a = arena_state.allocator();
    const loaded = try rowsOf(a, &.{ .{ "g1", .{ .snapshot_json = "{\"a\":1}", .experience_count = 1 } }, .{ "g2", .{ .snapshot_json = "{\"b\":1}", .experience_count = 1 } } });
    const admitted = try seed(a, &loaded);
    var next = try loaded.clone(a);
    try next.put(a, "g1", .{ .snapshot_json = "{\"a\":2}", .experience_count = 2 });
    try next.put(a, "g2", .{ .snapshot_json = "{\"b\":2}", .experience_count = 2 });
    try next.put(a, "g3", .{ .snapshot_json = "{\"c\":1}", .experience_count = 1 });
    try expectNames(&.{"g1"}, try plan(a, &next, &admitted, Only{ .guild = "g1" }));
    try expectNames(&.{"g1"}, try restrictToAdmitted(a, &next, &admitted, Only{ .guild = "g1" }));
    try testing.expectEqualStrings("{\"a\":1}", next.get("g1").?.snapshot_json);
    try testing.expectEqualStrings("{\"b\":2}", next.get("g2").?.snapshot_json);
    try testing.expect(next.get("g3") != null);
    try expectNames(&.{ "g2", "g3" }, try restrictToAdmitted(a, &next, &admitted, All{}));
    try testing.expect(next.get("g3") == null);
}
