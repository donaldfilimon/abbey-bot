//! In-process memory state, transcribed from the oracle's `src/memory.rs`:
//! per-user facts, standing and pending supersessions, and per-channel recent
//! context. Pure: every mutation takes `now` from the caller. Durability is
//! the episodic store's job (`store.zig` replays into a bank).
const std = @import("std");
const text = @import("../text/text.zig");
const Allocator = std.mem.Allocator;

pub const recent_cap: usize = 50;
pub const max_facts: usize = 100;
pub const max_pending_supersessions: usize = 16;
pub const max_fact_chars: usize = 300;
pub const default_reputation: f64 = 0.5;
pub const autocomplete_max_choices: usize = 25;
pub const autocomplete_max_chars: usize = 100;
pub const summary_every_messages: u64 = 30;
/// Unambiguous separator between scoped guild and scoped user in map keys.
pub const user_key_separator = "\x1f";

pub const FactError = error{ EmptyFact, FactTooLong };

/// Fixed user-facing copy for a rejected fact.
pub fn factErrorMessage(err: FactError) []const u8 {
    return switch (err) {
        error.EmptyFact => "The fact must contain some text.",
        error.FactTooLong => "Keep one remembered fact to 300 characters or fewer.",
    };
}

/// Normalize (collapse Unicode whitespace) and validate one durable fact.
/// Caller owns the returned slice.
pub fn validatedFact(gpa: Allocator, fact: []const u8) (FactError || Allocator.Error)![]u8 {
    const normalized = try text.collapseWhitespace(gpa, fact);
    errdefer gpa.free(normalized);
    if (normalized.len == 0) return error.EmptyFact;
    if (text.charCount(normalized) > max_fact_chars) return error.FactTooLong;
    return normalized;
}

pub const PendingSupersession = struct {
    new_fact: []u8,
    old_fact: []u8,
    at: u64,
};

pub const UserMemory = struct {
    facts: std.ArrayList([]u8) = .empty,
    reputation: f64 = default_reputation,
    interaction_count: u64 = 0,
    updated_at: u64 = 0,
    pending: std.ArrayList(PendingSupersession) = .empty,

    fn deinit(m: *UserMemory, gpa: Allocator) void {
        for (m.facts.items) |f| gpa.free(f);
        m.facts.deinit(gpa);
        for (m.pending.items) |p| {
            gpa.free(p.new_fact);
            gpa.free(p.old_fact);
        }
        m.pending.deinit(gpa);
    }

    pub fn hasFact(m: *const UserMemory, fact: []const u8) bool {
        for (m.facts.items) |f| if (std.mem.eql(u8, f, fact)) return true;
        return false;
    }
};

pub const RecentMessage = struct { author: []u8, text: []u8, at: u64 };

pub const ChannelContext = struct {
    summary: []u8 = &.{},
    message_count: u64 = 0,
    recent: std.ArrayList(RecentMessage) = .empty,
    updated_at: u64 = 0,
    summarized_at_count: u64 = 0,

    fn deinit(c: *ChannelContext, gpa: Allocator) void {
        gpa.free(c.summary);
        for (c.recent.items) |m| {
            gpa.free(m.author);
            gpa.free(m.text);
        }
        c.recent.deinit(gpa);
    }

    pub fn summaryDue(c: *const ChannelContext) bool {
        return c.message_count >= c.summarized_at_count + summary_every_messages and c.recent.items.len > 0;
    }

    pub fn pushRecent(c: *ChannelContext, gpa: Allocator, author: []const u8, body: []const u8, now: u64) Allocator.Error!void {
        const a = try gpa.dupe(u8, author);
        errdefer gpa.free(a);
        const t = try gpa.dupe(u8, body);
        errdefer gpa.free(t);
        try c.recent.append(gpa, .{ .author = a, .text = t, .at = now });
        while (c.recent.items.len > recent_cap) {
            const old = c.recent.orderedRemove(0);
            gpa.free(old.author);
            gpa.free(old.text);
        }
        c.message_count += 1;
        c.updated_at = now;
    }

    /// The last `limit` messages, oldest first, one `author: text` line each.
    pub fn renderRecent(c: *const ChannelContext, gpa: Allocator, limit: usize) Allocator.Error![]u8 {
        var out: std.ArrayList(u8) = .empty;
        errdefer out.deinit(gpa);
        const skip = c.recent.items.len -| limit;
        for (c.recent.items[skip..], 0..) |m, i| {
            if (i > 0) try out.append(gpa, '\n');
            try out.print(gpa, "{s}: {s}", .{ m.author, m.text });
        }
        return out.toOwnedSlice(gpa);
    }
};

/// Why a remember did nothing (the oracle returns `false` for both).
pub const RememberResult = enum { stored, duplicate, full };

pub const MemoryBank = struct {
    gpa: Allocator,
    users: std.StringArrayHashMapUnmanaged(UserMemory) = .empty,
    channels: std.StringArrayHashMapUnmanaged(ChannelContext) = .empty,
    messages_seen: u64 = 0,

    pub fn init(gpa: Allocator) MemoryBank {
        return .{ .gpa = gpa };
    }

    pub fn deinit(b: *MemoryBank) void {
        for (b.users.keys(), b.users.values()) |k, *v| {
            b.gpa.free(k);
            v.deinit(b.gpa);
        }
        b.users.deinit(b.gpa);
        for (b.channels.keys(), b.channels.values()) |k, *v| {
            b.gpa.free(k);
            v.deinit(b.gpa);
        }
        b.channels.deinit(b.gpa);
    }

    fn userKey(gpa: Allocator, guild: []const u8, user_id: []const u8) Allocator.Error![]u8 {
        return std.fmt.allocPrint(gpa, "{s}" ++ user_key_separator ++ "{s}", .{ guild, user_id });
    }

    pub fn user(b: *const MemoryBank, guild: []const u8, user_id: []const u8) ?*UserMemory {
        var buf: [512]u8 = undefined;
        const key = std.fmt.bufPrint(&buf, "{s}" ++ user_key_separator ++ "{s}", .{ guild, user_id }) catch {
            const owned = userKey(b.gpa, guild, user_id) catch return null;
            defer b.gpa.free(owned);
            return b.users.getPtr(owned);
        };
        return b.users.getPtr(key);
    }

    pub fn userMut(b: *MemoryBank, guild: []const u8, user_id: []const u8) Allocator.Error!*UserMemory {
        const key = try userKey(b.gpa, guild, user_id);
        const gop = b.users.getOrPut(b.gpa, key) catch |e| {
            b.gpa.free(key);
            return e;
        };
        if (gop.found_existing) {
            b.gpa.free(key);
        } else {
            gop.value_ptr.* = .{};
        }
        return gop.value_ptr;
    }

    pub fn remember(b: *MemoryBank, guild: []const u8, user_id: []const u8, fact: []const u8, now: u64) Allocator.Error!RememberResult {
        const m = try b.userMut(guild, user_id);
        if (m.hasFact(fact)) return .duplicate;
        if (m.facts.items.len >= max_facts) return .full;
        const owned = try b.gpa.dupe(u8, fact);
        errdefer b.gpa.free(owned);
        try m.facts.append(b.gpa, owned);
        m.updated_at = now;
        return .stored;
    }

    pub fn forget(b: *MemoryBank, guild: []const u8, user_id: []const u8, fact: []const u8) bool {
        const m = b.user(guild, user_id) orelse return false;
        var removed = false;
        var i: usize = 0;
        while (i < m.facts.items.len) {
            if (std.mem.eql(u8, m.facts.items[i], fact)) {
                b.gpa.free(m.facts.orderedRemove(i));
                removed = true;
            } else i += 1;
        }
        return removed;
    }

    pub fn facts(b: *const MemoryBank, guild: []const u8, user_id: []const u8) []const []const u8 {
        const m = b.user(guild, user_id) orelse return &.{};
        return m.facts.items;
    }

    pub fn proposeSupersession(b: *MemoryBank, guild: []const u8, user_id: []const u8, new_fact: []const u8, old_fact: []const u8, now: u64) Allocator.Error!bool {
        const m = try b.userMut(guild, user_id);
        for (m.pending.items) |p| {
            if (std.mem.eql(u8, p.new_fact, new_fact) and std.mem.eql(u8, p.old_fact, old_fact)) return false;
        }
        while (m.pending.items.len >= max_pending_supersessions) {
            const old = m.pending.orderedRemove(0);
            b.gpa.free(old.new_fact);
            b.gpa.free(old.old_fact);
        }
        const n = try b.gpa.dupe(u8, new_fact);
        errdefer b.gpa.free(n);
        const o = try b.gpa.dupe(u8, old_fact);
        errdefer b.gpa.free(o);
        try m.pending.append(b.gpa, .{ .new_fact = n, .old_fact = o, .at = now });
        m.updated_at = now;
        return true;
    }

    pub fn dropSupersession(b: *MemoryBank, guild: []const u8, user_id: []const u8, old_fact: []const u8) bool {
        const m = b.user(guild, user_id) orelse return false;
        var removed = false;
        var i: usize = 0;
        while (i < m.pending.items.len) {
            if (std.mem.eql(u8, m.pending.items[i].old_fact, old_fact)) {
                const p = m.pending.orderedRemove(i);
                b.gpa.free(p.new_fact);
                b.gpa.free(p.old_fact);
                removed = true;
            } else i += 1;
        }
        return removed;
    }

    pub fn pending(b: *const MemoryBank, guild: []const u8, user_id: []const u8) []const PendingSupersession {
        const m = b.user(guild, user_id) orelse return &.{};
        return m.pending.items;
    }

    pub fn channelMut(b: *MemoryBank, scoped_channel: []const u8) Allocator.Error!*ChannelContext {
        const gop = try b.channels.getOrPut(b.gpa, scoped_channel);
        if (!gop.found_existing) {
            gop.key_ptr.* = b.gpa.dupe(u8, scoped_channel) catch |e| {
                b.channels.swapRemoveAt(gop.index);
                return e;
            };
            gop.value_ptr.* = .{};
        }
        return gop.value_ptr;
    }

    pub fn recordMessage(b: *MemoryBank, scoped_channel: []const u8, author: []const u8, body: []const u8, now: u64) Allocator.Error!void {
        const c = try b.channelMut(scoped_channel);
        try c.pushRecent(b.gpa, author, body, now);
        b.messages_seen += 1;
    }

    pub fn setSummary(b: *MemoryBank, scoped_channel: []const u8, summary: []const u8) Allocator.Error!void {
        const c = try b.channelMut(scoped_channel);
        const owned = try b.gpa.dupe(u8, summary);
        b.gpa.free(c.summary);
        c.summary = owned;
    }

    /// Facts, summary and standing a persona sees. Borrowed from the bank:
    /// valid until the next mutation.
    pub fn contextFor(b: *const MemoryBank, guild: []const u8, user_id: []const u8, scoped_channel: []const u8) @import("context.zig").PersonaContext {
        const m = b.user(guild, user_id);
        const summary: []const u8 = if (b.channels.getPtr(scoped_channel)) |c| c.summary else "";
        return .{
            .channel_summary = summary,
            .user_facts = if (m) |mm| mm.facts.items else &.{},
            .reputation = if (m) |mm| mm.reputation else default_reputation,
        };
    }
};

/// `/forget` autocomplete: case-insensitive (`to_lowercase`) substring
/// matches, at most 25, each cut to 100 scalars. Returned slices borrow `facts`.
pub fn autocompleteFacts(gpa: Allocator, facts: []const []const u8, partial: []const u8) Allocator.Error![][]const u8 {
    var out: std.ArrayList([]const u8) = .empty;
    errdefer out.deinit(gpa);
    const needle = try text.toLowercase(gpa, partial);
    defer gpa.free(needle);
    for (facts) |f| {
        if (out.items.len >= autocomplete_max_choices) break;
        const lowered = try text.toLowercase(gpa, f);
        defer gpa.free(lowered);
        if (std.mem.indexOf(u8, lowered, needle) == null) continue;
        try out.append(gpa, f[0..text.byteOffsetOfChar(f, autocomplete_max_chars)]);
    }
    return out.toOwnedSlice(gpa);
}

test "remember rejects duplicates and the cap; forget removes by exact text" {
    var bank = MemoryBank.init(std.testing.allocator);
    defer bank.deinit();
    try std.testing.expectEqual(RememberResult.stored, try bank.remember("discord:1", "discord:2", "likes rust", 5));
    try std.testing.expectEqual(RememberResult.duplicate, try bank.remember("discord:1", "discord:2", "likes rust", 6));
    var i: usize = 1;
    while (i < max_facts) : (i += 1) {
        var buf: [16]u8 = undefined;
        _ = try bank.remember("discord:1", "discord:2", try std.fmt.bufPrint(&buf, "f{d}", .{i}), 7);
    }
    try std.testing.expectEqual(RememberResult.full, try bank.remember("discord:1", "discord:2", "one more", 8));
    try std.testing.expect(bank.forget("discord:1", "discord:2", "likes rust"));
    try std.testing.expect(!bank.forget("discord:1", "discord:2", "likes rust"));
    try std.testing.expectEqual(@as(usize, max_facts - 1), bank.facts("discord:1", "discord:2").len);
    // Scopes never share: another guild sees nothing.
    try std.testing.expectEqual(@as(usize, 0), bank.facts("discord:9", "discord:2").len);
}

test "pending supersessions are bounded and never touch facts" {
    var bank = MemoryBank.init(std.testing.allocator);
    defer bank.deinit();
    _ = try bank.remember("g", "u", "old", 1);
    try std.testing.expect(try bank.proposeSupersession("g", "u", "new", "old", 2));
    try std.testing.expect(!try bank.proposeSupersession("g", "u", "new", "old", 3));
    var i: usize = 0;
    while (i < 20) : (i += 1) {
        var buf: [16]u8 = undefined;
        _ = try bank.proposeSupersession("g", "u", "n", try std.fmt.bufPrint(&buf, "o{d}", .{i}), 4);
    }
    try std.testing.expectEqual(max_pending_supersessions, bank.pending("g", "u").len);
    try std.testing.expect(bank.dropSupersession("g", "u", "o19"));
    try std.testing.expectEqual(@as(usize, 1), bank.facts("g", "u").len);
}

test "channel recent window is capped at 50 and counts every message" {
    var bank = MemoryBank.init(std.testing.allocator);
    defer bank.deinit();
    var i: u64 = 0;
    while (i < 60) : (i += 1) try bank.recordMessage("discord:5", "a", "m", i);
    const c = bank.channels.getPtr("discord:5").?;
    try std.testing.expectEqual(recent_cap, c.recent.items.len);
    try std.testing.expectEqual(@as(u64, 60), c.message_count);
    try std.testing.expect(c.summaryDue());
    const last = try c.renderRecent(std.testing.allocator, 2);
    defer std.testing.allocator.free(last);
    try std.testing.expectEqualStrings("a: m\na: m", last);
}
