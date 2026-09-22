//! Zig-native append-only JSONL episodic store for short- and long-term
//! memory (decision 2): every memory mutation is one line
//! `{"v":1,"seq":N,"at":T,"kind":...}` appended and fsynced before the caller
//! acknowledges it, and startup replays the log into a `MemoryBank`.
//!
//! LTM kinds: fact_stored, fact_forgotten, pending_proposed, pending_dropped,
//! receipt_recorded, receipt_dropped, setting. STM kinds: message, summary.
//!
//! Durability rules: the file and its directory are owner-only (0600/0700);
//! the file is opened with an exclusive advisory lock so a second process
//! cannot interleave appends; a final line without its newline (a crash mid
//! write) is truncated away on open; any malformed complete line fails the
//! open instead of silently dropping memory.
const std = @import("std");
const bank_mod = @import("bank.zig");
const Allocator = std.mem.Allocator;
const File = std.Io.File;

pub const file_name = "episodes.jsonl";
pub const version = 1;

pub const Error = error{ CorruptLog, LogLocked, StoreIo } || Allocator.Error;

pub const Event = union(enum) {
    fact_stored: struct { guild: []const u8, user: []const u8, fact: []const u8 },
    fact_forgotten: struct { guild: []const u8, user: []const u8, fact: []const u8 },
    pending_proposed: struct { guild: []const u8, user: []const u8, new_fact: []const u8, old_fact: []const u8 },
    pending_dropped: struct { guild: []const u8, user: []const u8, old_fact: []const u8 },
    receipt_recorded: struct { guild: []const u8, user: []const u8, fact: []const u8, digest: []const u8 },
    receipt_dropped: struct { guild: []const u8, user: []const u8, fact: []const u8 },
    setting: struct { scope: []const u8, key: []const u8, value: []const u8 },
    message: struct { channel: []const u8, author: []const u8, text: []const u8 },
    summary: struct { channel: []const u8, text: []const u8 },
};

/// State rebuilt by replay beyond the bank itself.
pub const Replayed = struct {
    /// receipt key (guild \x1f user \x1f fact) -> digest hex
    receipts: std.StringArrayHashMapUnmanaged([]u8) = .empty,
    /// setting key (scope \x1f key) -> value
    settings: std.StringArrayHashMapUnmanaged([]u8) = .empty,

    pub fn deinit(r: *Replayed, gpa: Allocator) void {
        for (r.receipts.keys(), r.receipts.values()) |k, v| {
            gpa.free(k);
            gpa.free(v);
        }
        r.receipts.deinit(gpa);
        for (r.settings.keys(), r.settings.values()) |k, v| {
            gpa.free(k);
            gpa.free(v);
        }
        r.settings.deinit(gpa);
    }

    fn put(map: *std.StringArrayHashMapUnmanaged([]u8), gpa: Allocator, key: []const u8, value: []const u8) Allocator.Error!void {
        const v = try gpa.dupe(u8, value);
        errdefer gpa.free(v);
        const gop = try map.getOrPut(gpa, key);
        if (gop.found_existing) {
            gpa.free(gop.value_ptr.*);
        } else {
            gop.key_ptr.* = gpa.dupe(u8, key) catch |e| {
                _ = map.orderedRemove(key);
                return e;
            };
        }
        gop.value_ptr.* = v;
    }

    fn remove(map: *std.StringArrayHashMapUnmanaged([]u8), gpa: Allocator, key: []const u8) void {
        const kv = map.fetchOrderedRemove(key) orelse return;
        gpa.free(kv.key);
        gpa.free(kv.value);
    }
};

pub fn receiptKey(buf: []u8, guild: []const u8, user: []const u8, fact: []const u8) error{NoSpaceLeft}![]const u8 {
    return std.fmt.bufPrint(buf, "{s}\x1f{s}\x1f{s}", .{ guild, user, fact });
}

/// Apply one event to in-memory state (used by replay and by live writes, so
/// the two can never disagree).
pub fn apply(gpa: Allocator, bank: *bank_mod.MemoryBank, extra: *Replayed, event: Event, at: u64) Allocator.Error!void {
    var buf: [2048]u8 = undefined;
    switch (event) {
        .fact_stored => |e| _ = try bank.remember(e.guild, e.user, e.fact, at),
        .fact_forgotten => |e| _ = bank.forget(e.guild, e.user, e.fact),
        .pending_proposed => |e| _ = try bank.proposeSupersession(e.guild, e.user, e.new_fact, e.old_fact, at),
        .pending_dropped => |e| _ = bank.dropSupersession(e.guild, e.user, e.old_fact),
        .receipt_recorded => |e| if (receiptKey(&buf, e.guild, e.user, e.fact)) |k| try Replayed.put(&extra.receipts, gpa, k, e.digest) else |_| {},
        .receipt_dropped => |e| if (receiptKey(&buf, e.guild, e.user, e.fact)) |k| Replayed.remove(&extra.receipts, gpa, k) else |_| {},
        .setting => |e| if (std.fmt.bufPrint(&buf, "{s}\x1f{s}", .{ e.scope, e.key })) |k| try Replayed.put(&extra.settings, gpa, k, e.value) else |_| {},
        .message => |e| try bank.recordMessage(e.channel, e.author, e.text, at),
        .summary => |e| try bank.setSummary(e.channel, e.text),
    }
}

pub fn renderLine(w: *std.Io.Writer, seq: u64, at: u64, event: Event) std.Io.Writer.Error!void {
    var s: std.json.Stringify = .{ .writer = w };
    try s.beginObject();
    try s.objectField("v");
    try s.write(@as(u8, version));
    try s.objectField("seq");
    try s.write(seq);
    try s.objectField("at");
    try s.write(at);
    try s.objectField("kind");
    try s.write(@tagName(event));
    switch (event) {
        inline else => |payload| {
            inline for (@typeInfo(@TypeOf(payload)).@"struct".field_names) |name| {
                try s.objectField(name);
                try s.write(@field(payload, name));
            }
        },
    }
    try s.endObject();
    try w.writeByte('\n');
}

/// Parse one complete line into an event whose strings live in `arena`.
pub fn parseLine(arena: Allocator, line: []const u8) error{CorruptLog}!struct { seq: u64, at: u64, event: Event } {
    const v = std.json.parseFromSliceLeaky(std.json.Value, arena, line, .{ .allocate = .alloc_always }) catch return error.CorruptLog;
    if (v != .object) return error.CorruptLog;
    const o = v.object;
    const get = struct {
        fn s(obj: std.json.ObjectMap, name: []const u8) error{CorruptLog}![]const u8 {
            const x = obj.get(name) orelse return error.CorruptLog;
            return if (x == .string) x.string else error.CorruptLog;
        }
        fn n(obj: std.json.ObjectMap, name: []const u8) error{CorruptLog}!u64 {
            const x = obj.get(name) orelse return error.CorruptLog;
            return if (x == .integer and x.integer >= 0) @intCast(x.integer) else error.CorruptLog;
        }
    };
    if (try get.n(o, "v") != version) return error.CorruptLog;
    const kind = std.meta.stringToEnum(std.meta.Tag(Event), try get.s(o, "kind")) orelse return error.CorruptLog;
    const event: Event = switch (kind) {
        inline else => |tag| blk: {
            const Payload = @FieldType(Event, @tagName(tag));
            var payload: Payload = undefined;
            inline for (@typeInfo(Payload).@"struct".field_names) |name| @field(payload, name) = try get.s(o, name);
            break :blk @unionInit(Event, @tagName(tag), payload);
        },
    };
    return .{ .seq = try get.n(o, "seq"), .at = try get.n(o, "at"), .event = event };
}

pub const Store = struct {
    gpa: Allocator,
    io: std.Io,
    dir: std.Io.Dir,
    file: File,
    length: u64,
    next_seq: u64,

    /// Open (creating if needed) `<dir_path>/episodes.jsonl`, replay it into
    /// `bank` and `extra`, and hold it locked for appends.
    pub fn open(gpa: Allocator, io: std.Io, dir_path: []const u8, bank: *bank_mod.MemoryBank, extra: *Replayed) Error!Store {
        const cwd = std.Io.Dir.cwd();
        cwd.createDirPath(io, dir_path) catch return error.StoreIo;
        var dir = cwd.openDir(io, dir_path, .{}) catch return error.StoreIo;
        errdefer dir.close(io);
        dir.setPermissions(io, .fromMode(0o700)) catch return error.StoreIo;
        const file = dir.createFile(io, file_name, .{ .read = true, .truncate = false, .lock = .exclusive, .lock_nonblocking = true, .permissions = .fromMode(0o600) }) catch |e| return switch (e) {
            error.WouldBlock => error.LogLocked,
            else => error.StoreIo,
        };
        errdefer file.close(io);
        file.setPermissions(io, .fromMode(0o600)) catch return error.StoreIo;
        const len = file.length(io) catch return error.StoreIo;
        const bytes = gpa.alloc(u8, @intCast(len)) catch return error.OutOfMemory;
        defer gpa.free(bytes);
        const n = file.readPositionalAll(io, bytes, 0) catch return error.StoreIo;
        if (n != bytes.len) return error.StoreIo;

        var arena_state: std.heap.ArenaAllocator = .init(gpa);
        defer arena_state.deinit();
        var complete_end: usize = 0;
        var next_seq: u64 = 1;
        var start: usize = 0;
        while (std.mem.indexOfScalarPos(u8, bytes, start, '\n')) |nl| {
            const line = bytes[start..nl];
            if (line.len > 0) {
                _ = arena_state.reset(.retain_capacity);
                const rec = try parseLine(arena_state.allocator(), line);
                if (rec.seq != next_seq) return error.CorruptLog;
                try apply(gpa, bank, extra, rec.event, rec.at);
                next_seq += 1;
            }
            start = nl + 1;
            complete_end = start;
        }
        // A torn final line (no newline) is a crash artifact: drop it.
        if (complete_end < bytes.len) file.setLength(io, complete_end) catch return error.StoreIo;
        return .{ .gpa = gpa, .io = io, .dir = dir, .file = file, .length = complete_end, .next_seq = next_seq };
    }

    pub fn close(s: *Store) void {
        s.file.close(s.io);
        s.dir.close(s.io);
    }

    /// Append one event durably (write + fsync), then apply it in memory.
    pub fn append(s: *Store, bank: *bank_mod.MemoryBank, extra: *Replayed, event: Event, at: u64) Error!void {
        var line: std.Io.Writer.Allocating = .init(s.gpa);
        defer line.deinit();
        renderLine(&line.writer, s.next_seq, at, event) catch return error.OutOfMemory;
        s.file.writePositionalAll(s.io, line.written(), s.length) catch return error.StoreIo;
        s.file.sync(s.io) catch return error.StoreIo;
        s.length += line.written().len;
        s.next_seq += 1;
        try apply(s.gpa, bank, extra, event, at);
    }
};

const testing = std.testing;

test "store appends durably, replays in order and never shares scopes" {
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    var arena_state: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena_state.deinit();
    const dir = try tmp.dir.realPathFileAlloc(testing.io, ".", arena_state.allocator());
    const data = try std.fs.path.join(arena_state.allocator(), &.{ dir, "data" });
    {
        var bank = bank_mod.MemoryBank.init(testing.allocator);
        defer bank.deinit();
        var extra: Replayed = .{};
        defer extra.deinit(testing.allocator);
        var store = try Store.open(testing.allocator, testing.io, data, &bank, &extra);
        defer store.close();
        try store.append(&bank, &extra, .{ .fact_stored = .{ .guild = "discord:1", .user = "discord:2", .fact = "likes rust" } }, 10);
        try store.append(&bank, &extra, .{ .fact_stored = .{ .guild = "discord:1", .user = "discord:2", .fact = "has a \"cat\"\nnamed Miso" } }, 11);
        try store.append(&bank, &extra, .{ .fact_forgotten = .{ .guild = "discord:1", .user = "discord:2", .fact = "likes rust" } }, 12);
        try store.append(&bank, &extra, .{ .setting = .{ .scope = "discord:dm:2", .key = "nsfw_roleplay_enabled", .value = "true" } }, 13);
        try store.append(&bank, &extra, .{ .receipt_recorded = .{ .guild = "discord:1", .user = "discord:2", .fact = "x", .digest = "ab" } }, 14);
        try store.append(&bank, &extra, .{ .message = .{ .channel = "discord:5", .author = "dana", .text = "hi" } }, 15);
        // A second opener is refused while this one holds the lock.
        var bank2 = bank_mod.MemoryBank.init(testing.allocator);
        defer bank2.deinit();
        var extra2: Replayed = .{};
        defer extra2.deinit(testing.allocator);
        try testing.expectError(error.LogLocked, Store.open(testing.allocator, testing.io, data, &bank2, &extra2));
    }
    var bank = bank_mod.MemoryBank.init(testing.allocator);
    defer bank.deinit();
    var extra: Replayed = .{};
    defer extra.deinit(testing.allocator);
    var store = try Store.open(testing.allocator, testing.io, data, &bank, &extra);
    defer store.close();
    try testing.expectEqual(@as(u64, 7), store.next_seq);
    const facts = bank.facts("discord:1", "discord:2");
    try testing.expectEqual(@as(usize, 1), facts.len);
    try testing.expectEqualStrings("has a \"cat\"\nnamed Miso", facts[0]);
    try testing.expectEqual(@as(usize, 0), bank.facts("discord:9", "discord:2").len);
    try testing.expectEqualStrings("true", extra.settings.get("discord:dm:2\x1fnsfw_roleplay_enabled").?);
    try testing.expectEqual(@as(u64, 1), bank.channels.getPtr("discord:5").?.message_count);
    const stat = try tmp.dir.statFile(testing.io, "data/episodes.jsonl", .{});
    try testing.expectEqual(@as(u32, 0o600), @as(u32, @intCast(stat.permissions.toMode() & 0o777)));
}

test "a torn final line is dropped and a corrupt middle line fails closed" {
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    var arena_state: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena_state.deinit();
    const a = arena_state.allocator();
    try tmp.dir.createDirPath(testing.io, "torn");
    const good = "{\"v\":1,\"seq\":1,\"at\":5,\"kind\":\"fact_stored\",\"guild\":\"g\",\"user\":\"u\",\"fact\":\"kept\"}\n";
    try tmp.dir.writeFile(testing.io, .{ .sub_path = "torn/episodes.jsonl", .data = good ++ "{\"v\":1,\"seq\":2,\"at\":6,\"kind\":\"fact_st" });
    const dir = try tmp.dir.realPathFileAlloc(testing.io, ".", a);
    {
        var bank = bank_mod.MemoryBank.init(testing.allocator);
        defer bank.deinit();
        var extra: Replayed = .{};
        defer extra.deinit(testing.allocator);
        var store = try Store.open(testing.allocator, testing.io, try std.fs.path.join(a, &.{ dir, "torn" }), &bank, &extra);
        defer store.close();
        try testing.expectEqual(@as(usize, 1), bank.facts("g", "u").len);
        try testing.expectEqual(@as(u64, good.len), store.length);
    }
    try tmp.dir.createDirPath(testing.io, "corrupt");
    try tmp.dir.writeFile(testing.io, .{ .sub_path = "corrupt/episodes.jsonl", .data = "not json\n" ++ good });
    var bank = bank_mod.MemoryBank.init(testing.allocator);
    defer bank.deinit();
    var extra: Replayed = .{};
    defer extra.deinit(testing.allocator);
    try testing.expectError(error.CorruptLog, Store.open(testing.allocator, testing.io, try std.fs.path.join(a, &.{ dir, "corrupt" }), &bank, &extra));
}
