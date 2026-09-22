//! Memory service: the one writer of durable memory, transcribed from the
//! oracle's `src/runtime/memory_service.rs`. Every change is a store event
//! (append + fsync before it is acknowledged, `store.zig`), and after each
//! change the WDBX fact projection is reconciled from the canonical bank and
//! saved atomically so `abi wdbx query` sees exactly the canonical facts.
const std = @import("std");
const bank_mod = @import("bank.zig");
const store_mod = @import("store.zig");
const seg = @import("wdbx_segment.zig");
const context_mod = @import("context.zig");
const Allocator = std.mem.Allocator;

pub const no_match_message = "No remembered fact matches what you asked to replace.";

pub const RememberOutcome = union(enum) {
    stored: []const u8,
    unchanged,
    superseded: struct { stored: []const u8, removed: []const u8 },
    proposed: struct { stored: []const u8, proposed: []const u8 },
};

/// Either an outcome (strings owned by the caller's arena) or the oracle's
/// fixed user-facing refusal.
pub const Result = union(enum) { ok: RememberOutcome, refused: []const u8 };

pub const SupersessionOutcome = union(enum) {
    confirmed: []const u8,
    already_gone: []const u8,
    premise_gone: struct { old_fact: []const u8, new_fact: []const u8 },
    not_pending,
};

pub const Service = struct {
    gpa: Allocator,
    io: std.Io,
    bank: bank_mod.MemoryBank,
    extra: store_mod.Replayed = .{},
    store: ?store_mod.Store = null,
    recall: seg.Recall,
    /// `<data>/wdbx.seg.0.jsonl` when persistent; owned.
    segment_path: ?[]u8 = null,

    /// In-memory when `data_dir` is null, otherwise replay the store.
    pub fn init(gpa: Allocator, io: std.Io, data_dir: ?[]const u8) !Service {
        var s: Service = .{ .gpa = gpa, .io = io, .bank = bank_mod.MemoryBank.init(gpa), .recall = seg.Recall.init(gpa) };
        errdefer s.deinit();
        if (data_dir) |dir| {
            s.store = try store_mod.Store.open(gpa, io, dir, &s.bank, &s.extra);
            s.segment_path = try std.fs.path.join(gpa, &.{ dir, "wdbx.seg.0.jsonl" });
        }
        try s.reconcile();
        return s;
    }

    pub fn deinit(s: *Service) void {
        if (s.store) |*st| st.close();
        s.bank.deinit();
        s.extra.deinit(s.gpa);
        s.recall.deinit();
        if (s.segment_path) |p| s.gpa.free(p);
    }

    fn commit(s: *Service, event: store_mod.Event, at: u64) !void {
        if (s.store) |*st| return st.append(&s.bank, &s.extra, event, at);
        return store_mod.apply(s.gpa, &s.bank, &s.extra, event, at);
    }

    /// Rebuild the projection from canonical facts (oracle `fact_records`:
    /// user rows sorted by key, facts in order, `at` = the row's updated_at)
    /// and persist the segment with write-to-temp plus rename.
    fn reconcile(s: *Service) !void {
        var arena_state: std.heap.ArenaAllocator = .init(s.gpa);
        defer arena_state.deinit();
        const arena = arena_state.allocator();
        const keys = try arena.dupe([]const u8, s.bank.users.keys());
        std.mem.sort([]const u8, keys, {}, struct {
            fn lt(_: void, a: []const u8, b: []const u8) bool {
                return std.mem.lessThan(u8, a, b);
            }
        }.lt);
        var wanted: std.ArrayList(seg.Recall.Wanted) = .empty;
        for (keys) |key| {
            const sep = std.mem.indexOf(u8, key, bank_mod.user_key_separator) orelse continue;
            const m = s.bank.users.getPtr(key).?;
            for (m.facts.items) |f| try wanted.append(arena, .{ .guild = key[0..sep], .user = key[sep + 1 ..], .text = f, .at = m.updated_at });
        }
        try s.recall.reconcile(arena, wanted.items);
        if (s.segment_path) |path| {
            const rendered = try s.recall.store.render(arena);
            const tmp = try std.fmt.allocPrint(arena, "{s}.tmp", .{path});
            const cwd = std.Io.Dir.cwd();
            try cwd.writeFile(s.io, .{ .sub_path = tmp, .data = rendered, .flags = .{ .permissions = .fromMode(0o600) } });
            try cwd.rename(tmp, cwd, path, s.io);
        }
    }

    fn validated(arena: Allocator, fact: []const u8) !union(enum) { ok: []u8, refused: []const u8 } {
        return if (bank_mod.validatedFact(arena, fact)) |v| .{ .ok = v } else |e| switch (e) {
            error.OutOfMemory => error.OutOfMemory,
            else => |fe| .{ .refused = bank_mod.factErrorMessage(fe) },
        };
    }

    fn canStore(s: *Service, guild: []const u8, user: []const u8, fact: []const u8) bool {
        const facts = s.bank.facts(guild, user);
        for (facts) |f| if (std.mem.eql(u8, f, fact)) return false;
        return facts.len < bank_mod.max_facts;
    }

    /// Exact text first, then the whitespace-normalized text.
    pub fn resolveFact(s: *Service, arena: Allocator, guild: []const u8, user: []const u8, requested: []const u8) !?[]const u8 {
        const facts = s.bank.facts(guild, user);
        for (facts) |f| if (std.mem.eql(u8, f, requested)) return try arena.dupe(u8, f);
        const normalized = try @import("../text/text.zig").collapseWhitespace(arena, requested);
        for (facts) |f| if (std.mem.eql(u8, f, normalized)) return try arena.dupe(u8, f);
        return null;
    }

    pub fn rememberBlocked(s: *Service, guild: []const u8, user: []const u8, fact: []const u8) ?[]const u8 {
        const facts = s.bank.facts(guild, user);
        for (facts) |f| if (std.mem.eql(u8, f, fact)) return "already on record";
        if (facts.len >= bank_mod.max_facts) return "the fact list is full";
        return null;
    }

    pub fn remember(s: *Service, arena: Allocator, guild: []const u8, user: []const u8, fact: []const u8, now: u64) !Result {
        const v = switch (try validated(arena, fact)) {
            .ok => |x| x,
            .refused => |m| return .{ .refused = m },
        };
        if (!s.canStore(guild, user, v)) return .{ .ok = .unchanged };
        try s.commit(.{ .fact_stored = .{ .guild = guild, .user = user, .fact = v } }, now);
        try s.reconcile();
        return .{ .ok = .{ .stored = v } };
    }

    pub fn rememberReplacing(s: *Service, arena: Allocator, guild: []const u8, user: []const u8, fact: []const u8, replaces: []const u8, now: u64) !Result {
        const v = switch (try validated(arena, fact)) {
            .ok => |x| x,
            .refused => |m| return .{ .refused = m },
        };
        const selected = try s.resolveFact(arena, guild, user, replaces) orelse return .{ .refused = no_match_message };
        if (std.mem.eql(u8, selected, v)) return .{ .ok = .unchanged };
        try s.commit(.{ .fact_forgotten = .{ .guild = guild, .user = user, .fact = selected } }, now);
        if (!s.canStore(guild, user, v)) {
            // Restore the old fact (it returns at the end, as in the oracle).
            try s.commit(.{ .fact_stored = .{ .guild = guild, .user = user, .fact = selected } }, now);
            try s.reconcile();
            return .{ .ok = .unchanged };
        }
        try s.commit(.{ .fact_stored = .{ .guild = guild, .user = user, .fact = v } }, now);
        try s.commit(.{ .pending_dropped = .{ .guild = guild, .user = user, .old_fact = selected } }, now);
        try s.reconcile();
        return .{ .ok = .{ .superseded = .{ .stored = v, .removed = selected } } };
    }

    /// Store an admitted fact, optionally proposing (never applying) a
    /// supersession, and key it by its ledger receipt.
    pub fn rememberAdmitted(s: *Service, arena: Allocator, guild: []const u8, user: []const u8, fact: []const u8, supersedes: ?[]const u8, now: u64, receipt_hex: ?[]const u8) !Result {
        const v = switch (try validated(arena, fact)) {
            .ok => |x| x,
            .refused => |m| return .{ .refused = m },
        };
        if (!s.canStore(guild, user, v)) return .{ .ok = .unchanged };
        try s.commit(.{ .fact_stored = .{ .guild = guild, .user = user, .fact = v } }, now);
        var proposed: ?[]const u8 = null;
        if (supersedes) |old| {
            if (try s.resolveFact(arena, guild, user, old)) |candidate| {
                if (!std.mem.eql(u8, candidate, v)) proposed = candidate;
            }
        }
        if (proposed) |old| try s.commit(.{ .pending_proposed = .{ .guild = guild, .user = user, .new_fact = v, .old_fact = old } }, now);
        if (receipt_hex) |r| try s.commit(.{ .receipt_recorded = .{ .guild = guild, .user = user, .fact = v, .digest = r } }, now);
        try s.reconcile();
        return .{ .ok = if (proposed) |old| .{ .proposed = .{ .stored = v, .proposed = old } } else .{ .stored = v } };
    }

    pub fn forget(s: *Service, arena: Allocator, guild: []const u8, user: []const u8, requested: []const u8, now: u64) !bool {
        const selected = try s.resolveFact(arena, guild, user, requested) orelse return false;
        try s.commit(.{ .fact_forgotten = .{ .guild = guild, .user = user, .fact = selected } }, now);
        try s.reconcile();
        return true;
    }

    pub fn confirmSupersession(s: *Service, arena: Allocator, guild: []const u8, user: []const u8, old_fact: []const u8, now: u64) !SupersessionOutcome {
        const pending = for (s.bank.pending(guild, user)) |p| {
            if (std.mem.eql(u8, p.old_fact, old_fact)) break p;
        } else return .not_pending;
        const old = try arena.dupe(u8, pending.old_fact);
        const new = try arena.dupe(u8, pending.new_fact);
        const facts = s.bank.facts(guild, user);
        const has_new = for (facts) |f| {
            if (std.mem.eql(u8, f, new)) break true;
        } else false;
        if (!has_new) {
            try s.commit(.{ .pending_dropped = .{ .guild = guild, .user = user, .old_fact = old } }, now);
            try s.reconcile();
            return .{ .premise_gone = .{ .old_fact = old, .new_fact = new } };
        }
        const has_old = for (facts) |f| {
            if (std.mem.eql(u8, f, old)) break true;
        } else false;
        if (has_old) try s.commit(.{ .fact_forgotten = .{ .guild = guild, .user = user, .fact = old } }, now);
        try s.commit(.{ .pending_dropped = .{ .guild = guild, .user = user, .old_fact = old } }, now);
        try s.reconcile();
        return if (has_old) .{ .confirmed = old } else .{ .already_gone = old };
    }

    pub fn dismissSupersession(s: *Service, guild: []const u8, user: []const u8, old_fact: []const u8, now: u64) !bool {
        const present = for (s.bank.pending(guild, user)) |p| {
            if (std.mem.eql(u8, p.old_fact, old_fact)) break true;
        } else false;
        if (!present) return false;
        try s.commit(.{ .pending_dropped = .{ .guild = guild, .user = user, .old_fact = old_fact } }, now);
        return true;
    }

    pub fn receipt(s: *Service, guild: []const u8, user: []const u8, fact: []const u8) ?[]const u8 {
        var buf: [2048]u8 = undefined;
        const key = store_mod.receiptKey(&buf, guild, user, fact) catch return null;
        return s.extra.receipts.get(key);
    }

    pub fn recordReceipt(s: *Service, guild: []const u8, user: []const u8, fact: []const u8, digest_hex: []const u8, now: u64) !void {
        try s.commit(.{ .receipt_recorded = .{ .guild = guild, .user = user, .fact = fact, .digest = digest_hex } }, now);
    }

    pub fn dropReceipt(s: *Service, guild: []const u8, user: []const u8, fact: []const u8, now: u64) !void {
        if (s.receipt(guild, user, fact) == null) return;
        try s.commit(.{ .receipt_dropped = .{ .guild = guild, .user = user, .fact = fact } }, now);
    }

    pub fn setting(s: *Service, scope: []const u8, key: []const u8) ?[]const u8 {
        var buf: [512]u8 = undefined;
        const k = std.fmt.bufPrint(&buf, "{s}\x1f{s}", .{ scope, key }) catch return null;
        return s.extra.settings.get(k);
    }

    pub fn setSetting(s: *Service, scope: []const u8, key: []const u8, value: []const u8, now: u64) !void {
        try s.commit(.{ .setting = .{ .scope = scope, .key = key, .value = value } }, now);
    }

    pub fn recordMessage(s: *Service, channel: []const u8, author: []const u8, body: []const u8, now: u64) !void {
        try s.commit(.{ .message = .{ .channel = channel, .author = author, .text = body } }, now);
    }

    /// Canonical context plus in-process projection recall for `query`
    /// (oracle `context_for`); strings borrow the bank or `arena`.
    pub fn contextFor(s: *Service, arena: Allocator, guild: []const u8, user: []const u8, channel: []const u8, query: []const u8, recall_limit: usize, reputation: f64) !context_mod.PersonaContext {
        var ctx = s.bank.contextFor(guild, user, channel);
        ctx.reputation = reputation;
        var facts: std.ArrayList([]const u8) = .empty;
        try facts.appendSlice(arena, ctx.user_facts);
        const hits = try s.recall.recallForUser(arena, guild, user, query, recall_limit);
        for (hits) |h| {
            const dup = for (facts.items) |f| {
                if (std.mem.eql(u8, f, h.text)) break true;
            } else false;
            if (!dup) try facts.append(arena, h.text);
        }
        ctx.user_facts = facts.items;
        return ctx;
    }
};

const testing = std.testing;

fn fresh() !Service {
    return Service.init(testing.allocator, testing.io, null);
}

test "explicit replaces supersedes atomically and an absent target is refused without storing" {
    var s = try fresh();
    defer s.deinit();
    var arena: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena.deinit();
    const a = arena.allocator();
    _ = try s.remember(a, "g", "u", "likes tea", 1);
    const r = try s.rememberReplacing(a, "g", "u", "likes coffee", "likes  tea", 2);
    try testing.expectEqualStrings("likes coffee", r.ok.superseded.stored);
    try testing.expectEqualStrings("likes tea", r.ok.superseded.removed);
    try testing.expectEqual(@as(usize, 1), s.bank.facts("g", "u").len);
    const refused = try s.rememberReplacing(a, "g", "u", "x", "never stored", 3);
    try testing.expectEqualStrings(no_match_message, refused.refused);
    try testing.expectEqual(@as(usize, 1), s.bank.facts("g", "u").len);
}

test "a model proposal stores without removing anything; confirm and dismiss behave like the oracle" {
    var s = try fresh();
    defer s.deinit();
    var arena: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena.deinit();
    const a = arena.allocator();
    _ = try s.remember(a, "g", "u", "old", 1);
    const p = try s.rememberAdmitted(a, "g", "u", "new", "old", 2, null);
    try testing.expectEqualStrings("old", p.ok.proposed.proposed);
    try testing.expectEqual(@as(usize, 2), s.bank.facts("g", "u").len);
    try testing.expectEqualStrings("old", (try s.confirmSupersession(a, "g", "u", "old", 3)).confirmed);
    try testing.expectEqual(@as(usize, 1), s.bank.facts("g", "u").len);
    try testing.expect((try s.confirmSupersession(a, "g", "u", "old", 4)) == .not_pending);
    _ = try s.remember(a, "g", "u", "keep1", 5);
    _ = try s.rememberAdmitted(a, "g", "u", "keep2", "keep1", 6, null);
    try testing.expect(try s.dismissSupersession("g", "u", "keep1", 7));
    try testing.expectEqual(@as(usize, 3), s.bank.facts("g", "u").len);
}

test "remember validates once, updates both representations and the projection never crosses scopes" {
    var s = try fresh();
    defer s.deinit();
    var arena: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena.deinit();
    const a = arena.allocator();
    try testing.expectEqualStrings("The fact must contain some text.", (try s.remember(a, "g", "u", "   ", 1)).refused);
    try testing.expectEqualStrings("likes rust", (try s.remember(a, "g", "u", " likes \t rust ", 1)).ok.stored);
    try testing.expect((try s.remember(a, "g", "u", "likes rust", 2)).ok == .unchanged);
    _ = try s.remember(a, "g", "other", "likes rust too", 3);
    const hits = try s.recall.recallForUser(a, "g", "u", "rust", 5);
    try testing.expectEqual(@as(usize, 1), hits.len);
    try testing.expectEqualStrings("likes rust", hits[0].text);
    try testing.expect(try s.forget(a, "g", "u", "likes rust", 4));
    try testing.expectEqual(@as(usize, 0), (try s.recall.recallForUser(a, "g", "u", "rust", 5)).len);
}

test "a persistent service replays facts, settings and the projection after restart" {
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    var arena: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena.deinit();
    const a = arena.allocator();
    const data = try std.fs.path.join(a, &.{ try tmp.dir.realPathFileAlloc(testing.io, ".", a), "data" });
    {
        var s = try Service.init(testing.allocator, testing.io, data);
        defer s.deinit();
        _ = try s.remember(a, "discord:1", "discord:2", "likes rust", 10);
        try s.setSetting("discord:dm:2", "nsfw_roleplay_enabled", "true", 11);
    }
    var s = try Service.init(testing.allocator, testing.io, data);
    defer s.deinit();
    try testing.expectEqual(@as(usize, 1), s.bank.facts("discord:1", "discord:2").len);
    try testing.expectEqualStrings("true", s.setting("discord:dm:2", "nsfw_roleplay_enabled").?);
    const segment = try tmp.dir.readFileAlloc(testing.io, "data/wdbx.seg.0.jsonl", a, .limited(1 << 20));
    try testing.expect(std.mem.startsWith(u8, segment, "# ABI-WDBX v1\n{\"type\":\"vector\",\"id\":1,"));
    try testing.expect(std.mem.indexOf(u8, segment, "\"key\":\"mem:discord:1:1\"") != null);
}
