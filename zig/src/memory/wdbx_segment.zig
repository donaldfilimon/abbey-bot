//! The `# ABI-WDBX v1` JSONL segment the oracle projects durable facts into
//! (`src/wdbx.rs` `WdbxStore` + `Recall`), byte-compatible with it and with
//! `contracts/fixtures/wdbx_v1_conformance.seg.jsonl` (which the gate also
//! compares with ../wdbx's own golden). Vector retrieval over the segment is
//! delegated to `abi wdbx query` (`wdbx_bridge.zig`); the in-process search
//! here exists to pin the oracle's ranking in tests and as the shape the
//! bridge's results are checked against.
const std = @import("std");
const embedding = @import("embedding.zig");
const ryu_style = @import("../text/ryu_style.zig");
const Allocator = std.mem.Allocator;

pub const header = "# ABI-WDBX v1";
const checksum_prefix = "# checksum:";

pub const ParseError = error{ MissingHeader, MalformedLine, DuplicateVector } || Allocator.Error;

const Vector = struct { id: u64, values: []f32 };

pub const Store = struct {
    gpa: Allocator,
    /// Sorted by key on render (the oracle's BTreeMap).
    kv: std.StringArrayHashMapUnmanaged([]u8) = .empty,
    vectors: std.ArrayList(Vector) = .empty,
    next_id: u64 = 1,
    unknown: std.ArrayList([]u8) = .empty,

    pub fn init(gpa: Allocator) Store {
        return .{ .gpa = gpa };
    }

    pub fn deinit(s: *Store) void {
        for (s.kv.keys(), s.kv.values()) |k, v| {
            s.gpa.free(k);
            s.gpa.free(v);
        }
        s.kv.deinit(s.gpa);
        for (s.vectors.items) |v| s.gpa.free(v.values);
        s.vectors.deinit(s.gpa);
        for (s.unknown.items) |u| s.gpa.free(u);
        s.unknown.deinit(s.gpa);
    }

    pub fn parse(gpa: Allocator, text: []const u8) ParseError!Store {
        var s = Store.init(gpa);
        errdefer s.deinit();
        var lines = std.mem.splitScalar(u8, text, '\n');
        const first = std.mem.trimEnd(u8, lines.first(), "\r");
        if (!std.mem.eql(u8, first, header)) return error.MissingHeader;
        while (lines.next()) |raw| {
            const line = std.mem.trimEnd(u8, raw, "\r");
            if (std.mem.trim(u8, line, " \t").len == 0) continue;
            if (std.mem.startsWith(u8, line, checksum_prefix)) continue;
            if (line[0] == '#') {
                try s.unknown.append(gpa, try gpa.dupe(u8, line));
                continue;
            }
            try s.parseRecord(line);
        }
        return s;
    }

    fn parseRecord(s: *Store, line: []const u8) ParseError!void {
        var parsed = std.json.parseFromSlice(std.json.Value, s.gpa, line, .{}) catch |e| switch (e) {
            error.OutOfMemory => return error.OutOfMemory,
            else => return error.MalformedLine,
        };
        defer parsed.deinit();
        if (parsed.value != .object) return error.MalformedLine;
        const obj = parsed.value.object;
        const kind = obj.get("type") orelse return error.MalformedLine;
        if (kind != .string) return error.MalformedLine;
        if (std.mem.eql(u8, kind.string, "kv")) {
            const key = obj.get("key") orelse return error.MalformedLine;
            const value = obj.get("value") orelse return error.MalformedLine;
            if (key != .string or value != .string) return error.MalformedLine;
            try s.putKv(key.string, value.string);
        } else if (std.mem.eql(u8, kind.string, "vector")) {
            const id_v = obj.get("id") orelse return error.MalformedLine;
            if (id_v != .integer or id_v.integer < 0) return error.MalformedLine;
            const id: u64 = @intCast(id_v.integer);
            const items = obj.get("values") orelse return error.MalformedLine;
            if (items != .array) return error.MalformedLine;
            const values = try s.gpa.alloc(f32, items.array.items.len);
            errdefer s.gpa.free(values);
            for (items.array.items, values) |item, *out| {
                out.* = switch (item) {
                    .integer => |n| @floatFromInt(n),
                    .float => |f| @floatCast(f),
                    .number_string => |t| std.fmt.parseFloat(f32, t) catch return error.MalformedLine,
                    else => return error.MalformedLine,
                };
            }
            for (s.vectors.items) |v| if (v.id == id) return error.DuplicateVector;
            try s.vectors.append(s.gpa, .{ .id = id, .values = values });
            s.next_id = @max(s.next_id, id +| 1);
        } else {
            try s.unknown.append(s.gpa, try s.gpa.dupe(u8, line));
        }
    }

    pub fn putKv(s: *Store, key: []const u8, value: []const u8) Allocator.Error!void {
        const v = try s.gpa.dupe(u8, value);
        errdefer s.gpa.free(v);
        const gop = try s.kv.getOrPut(s.gpa, key);
        if (gop.found_existing) {
            s.gpa.free(gop.value_ptr.*);
        } else {
            gop.key_ptr.* = s.gpa.dupe(u8, key) catch |e| {
                _ = s.kv.orderedRemove(key);
                return e;
            };
        }
        gop.value_ptr.* = v;
    }

    pub fn getKv(s: *const Store, key: []const u8) ?[]const u8 {
        return s.kv.get(key);
    }

    pub fn removeKv(s: *Store, key: []const u8) bool {
        const kv = s.kv.fetchOrderedRemove(key) orelse return false;
        s.gpa.free(kv.key);
        s.gpa.free(kv.value);
        return true;
    }

    pub fn insertVector(s: *Store, values: []const f32) Allocator.Error!u64 {
        const id = s.next_id;
        const owned = try s.gpa.dupe(f32, values);
        errdefer s.gpa.free(owned);
        try s.vectors.append(s.gpa, .{ .id = id, .values = owned });
        s.next_id +|= 1;
        return id;
    }

    pub fn vector(s: *const Store, id: u64) ?[]const f32 {
        for (s.vectors.items) |v| if (v.id == id) return v.values;
        return null;
    }

    pub fn removeVector(s: *Store, id: u64) bool {
        for (s.vectors.items, 0..) |v, i| {
            if (v.id == id) {
                s.gpa.free(v.values);
                _ = s.vectors.orderedRemove(i);
                return true;
            }
        }
        return false;
    }

    fn sortedKeys(s: *const Store, gpa: Allocator) Allocator.Error![][]const u8 {
        const keys = try gpa.dupe([]const u8, s.kv.keys());
        std.mem.sort([]const u8, keys, {}, struct {
            fn lt(_: void, a: []const u8, b: []const u8) bool {
                return std.mem.lessThan(u8, a, b);
            }
        }.lt);
        return keys;
    }

    /// Byte-identical to the oracle's `WdbxStore::try_render`.
    pub fn render(s: *const Store, gpa: Allocator) (error{NonFiniteVector} || Allocator.Error)![]u8 {
        var out: std.Io.Writer.Allocating = .init(gpa);
        errdefer out.deinit();
        const w = &out.writer;
        w.writeAll(header ++ "\n") catch return error.OutOfMemory;
        for (s.vectors.items) |v| {
            for (v.values) |x| if (!std.math.isFinite(x)) return error.NonFiniteVector;
            w.print("{{\"type\":\"vector\",\"id\":{d},\"values\":[", .{v.id}) catch return error.OutOfMemory;
            for (v.values, 0..) |x, i| {
                if (i > 0) w.writeByte(',') catch return error.OutOfMemory;
                ryu_style.writeF32(w, x) catch return error.OutOfMemory;
            }
            w.writeAll("]}\n") catch return error.OutOfMemory;
        }
        const keys = try s.sortedKeys(gpa);
        defer gpa.free(keys);
        for (keys) |k| {
            std.json.Stringify.value(.{ .type = "kv", .key = k, .value = s.kv.get(k).? }, .{}, w) catch return error.OutOfMemory;
            w.writeByte('\n') catch return error.OutOfMemory;
        }
        for (s.unknown.items) |line| {
            w.writeAll(line) catch return error.OutOfMemory;
            w.writeByte('\n') catch return error.OutOfMemory;
        }
        return out.toOwnedSlice();
    }
};

pub const Recalled = struct { id: u64, user: []const u8, text: []const u8, at: u64, score: f32 };

const FactRecord = struct { user: []const u8, text: []const u8, at: u64 };

/// Fact projection (`Recall` in the oracle). Keys are `mem:<scoped guild>:<id>`.
pub const Recall = struct {
    store: Store,

    pub fn init(gpa: Allocator) Recall {
        return .{ .store = Store.init(gpa) };
    }

    pub fn deinit(r: *Recall) void {
        r.store.deinit();
    }

    fn factKey(buf: []u8, guild: []const u8, id: u64) error{NoSpaceLeft}![]const u8 {
        return std.fmt.bufPrint(buf, "mem:{s}:{d}", .{ guild, id });
    }

    pub fn remember(r: *Recall, guild: []const u8, user: []const u8, text: []const u8, at: u64) !u64 {
        const e = embedding.textEmbedding(text);
        const id = try r.store.insertVector(&e);
        var value: std.Io.Writer.Allocating = .init(r.store.gpa);
        defer value.deinit();
        try std.json.Stringify.value(FactRecord{ .user = user, .text = text, .at = at }, .{}, &value.writer);
        const key = try std.fmt.allocPrint(r.store.gpa, "mem:{s}:{d}", .{ guild, id });
        defer r.store.gpa.free(key);
        try r.store.putKv(key, value.written());
        return id;
    }

    pub fn forget(r: *Recall, guild: []const u8, id: u64) bool {
        var buf: [512]u8 = undefined;
        const key = factKey(&buf, guild, id) catch return false;
        if (!r.store.removeKv(key)) return false;
        _ = r.store.removeVector(id);
        return true;
    }

    /// Decode one stored fact (strings borrowed from `arena`).
    pub fn fact(r: *const Recall, arena: Allocator, guild: []const u8, id: u64, score: f32) ?Recalled {
        var buf: [512]u8 = undefined;
        const key = factKey(&buf, guild, id) catch return null;
        const raw = r.store.getKv(key) orelse return null;
        const parsed = std.json.parseFromSliceLeaky(FactRecord, arena, raw, .{ .allocate = .alloc_always }) catch return null;
        return .{ .id = id, .user = parsed.user, .text = parsed.text, .at = parsed.at, .score = score };
    }

    /// Ids stored for `guild`, ascending.
    pub fn guildIds(r: *const Recall, gpa: Allocator, guild: []const u8) Allocator.Error![]u64 {
        var ids: std.ArrayList(u64) = .empty;
        errdefer ids.deinit(gpa);
        var prefix_buf: [512]u8 = undefined;
        const prefix = std.fmt.bufPrint(&prefix_buf, "mem:{s}:", .{guild}) catch return ids.toOwnedSlice(gpa);
        for (r.store.kv.keys()) |k| {
            if (!std.mem.startsWith(u8, k, prefix)) continue;
            const id = std.fmt.parseInt(u64, k[prefix.len..], 10) catch continue;
            try ids.append(gpa, id);
        }
        std.mem.sort(u64, ids.items, {}, std.sort.asc(u64));
        return ids.toOwnedSlice(gpa);
    }

    /// In-process cosine ranking for one user's facts (oracle
    /// `recall_for_user`): score descending, then id ascending.
    pub fn recallForUser(r: *const Recall, arena: Allocator, guild: []const u8, user: []const u8, query: []const u8, k: usize) Allocator.Error![]Recalled {
        const ids = try r.guildIds(arena, guild);
        const q = embedding.textEmbedding(query);
        var hits: std.ArrayList(Recalled) = .empty;
        for (ids) |id| {
            const f = r.fact(arena, guild, id, 0) orelse continue;
            if (!std.mem.eql(u8, f.user, user)) continue;
            const v = r.store.vector(id) orelse continue;
            var scored = f;
            scored.score = embedding.cosine(&q, v);
            try hits.append(arena, scored);
        }
        std.mem.sort(Recalled, hits.items, {}, struct {
            fn lt(_: void, a: Recalled, b: Recalled) bool {
                if (a.score != b.score) return a.score > b.score;
                return a.id < b.id;
            }
        }.lt);
        return hits.items[0..@min(k, hits.items.len)];
    }

    /// Replace every `mem:` row with exactly `wanted`, in order (oracle
    /// `reconcile_memory_facts`: a no-op when already equal).
    pub const Wanted = struct { guild: []const u8, user: []const u8, text: []const u8, at: u64 };

    /// Every stored fact with its guild, ordered by id (oracle `all_memory_facts`).
    pub fn allFacts(r: *const Recall, arena: Allocator) Allocator.Error![]Wanted {
        const Loc = struct { id: u64, guild: []const u8 };
        var locs: std.ArrayList(Loc) = .empty;
        for (r.store.kv.keys()) |k| {
            if (!std.mem.startsWith(u8, k, "mem:")) continue;
            const rest = k["mem:".len..];
            const colon = std.mem.lastIndexOfScalar(u8, rest, ':') orelse continue;
            const id = std.fmt.parseInt(u64, rest[colon + 1 ..], 10) catch continue;
            try locs.append(arena, .{ .id = id, .guild = rest[0..colon] });
        }
        std.mem.sort(Loc, locs.items, {}, struct {
            fn lt(_: void, a: Loc, b: Loc) bool {
                if (a.id != b.id) return a.id < b.id;
                return std.mem.lessThan(u8, a.guild, b.guild);
            }
        }.lt);
        var out: std.ArrayList(Wanted) = .empty;
        for (locs.items) |l| {
            const f = r.fact(arena, l.guild, l.id, 1.0) orelse continue;
            try out.append(arena, .{ .guild = l.guild, .user = f.user, .text = f.text, .at = f.at });
        }
        return out.items;
    }

    pub fn reconcile(r: *Recall, arena: Allocator, wanted: []const Wanted) !void {
        const current = try r.allFacts(arena);
        if (current.len == wanted.len) {
            const same = for (current, wanted) |c, w| {
                if (!std.mem.eql(u8, c.guild, w.guild) or !std.mem.eql(u8, c.user, w.user) or !std.mem.eql(u8, c.text, w.text) or c.at != w.at) break false;
            } else true;
            if (same) return;
        }
        var keys: std.ArrayList([]const u8) = .empty;
        for (r.store.kv.keys()) |k| if (std.mem.startsWith(u8, k, "mem:")) try keys.append(arena, try arena.dupe(u8, k));
        for (keys.items) |k| {
            const colon = std.mem.lastIndexOfScalar(u8, k, ':').?;
            const id = std.fmt.parseInt(u64, k[colon + 1 ..], 10) catch null;
            _ = r.store.removeKv(k);
            if (id) |i| _ = r.store.removeVector(i);
        }
        for (wanted) |w| _ = try r.remember(w.guild, w.user, w.text, w.at);
    }
};
