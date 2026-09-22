//! Discord REST rate limiting from response headers (pure; the caller owns
//! the clock and the sleeping).
//!
//! Routes are keyed by method plus path with every snowflake replaced by
//! `:id` except the top-level major parameter (channels, guilds, webhooks and
//! their token), because Discord shares a bucket across a major parameter's
//! sub-resources. `X-RateLimit-Bucket` maps a route to its shared bucket;
//! `X-RateLimit-Remaining` and `X-RateLimit-Reset-After` decide the wait; a
//! 429 with `X-RateLimit-Global` (or `"global": true`) pauses every route.
const std = @import("std");
const Allocator = std.mem.Allocator;

pub const Headers = struct {
    bucket: ?[]const u8 = null,
    remaining: ?u32 = null,
    reset_after_ms: ?u64 = null,
    global: bool = false,
    retry_after_ms: ?u64 = null,
};

/// Parse Discord's decimal-seconds header (`"1.250"`) to whole milliseconds,
/// rounding up so a wait never ends early.
pub fn secondsToMs(text: []const u8) ?u64 {
    const t = std.mem.trim(u8, text, " \t");
    if (t.len == 0) return null;
    var whole: u64 = 0;
    var frac_ms: u64 = 0;
    var i: usize = 0;
    while (i < t.len and std.ascii.isDigit(t[i])) : (i += 1) {
        whole = std.math.mul(u64, whole, 10) catch return null;
        whole = std.math.add(u64, whole, t[i] - '0') catch return null;
    }
    if (i == 0) return null;
    var round_up = false;
    if (i < t.len) {
        if (t[i] != '.') return null;
        i += 1;
        var digits: usize = 0;
        while (i < t.len and std.ascii.isDigit(t[i])) : (i += 1) {
            if (digits < 3) {
                frac_ms = frac_ms * 10 + (t[i] - '0');
            } else if (t[i] != '0') round_up = true;
            digits += 1;
        }
        if (i != t.len) return null;
        while (digits < 3) : (digits += 1) frac_ms *= 10;
    }
    const ms = std.math.add(u64, std.math.mul(u64, whole, 1000) catch return null, frac_ms) catch return null;
    return if (round_up) ms + 1 else ms;
}

/// `METHOD /path` with non-major snowflakes (and interaction tokens) masked.
/// A webhook's token is part of its major parameter and is kept.
pub fn routeKey(gpa: Allocator, method: []const u8, path: []const u8) Allocator.Error![]u8 {
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(gpa);
    try out.appendSlice(gpa, method);
    try out.append(gpa, ' ');
    const query = std.mem.indexOfScalar(u8, path, '?') orelse path.len;
    var it = std.mem.splitScalar(u8, path[0..query], '/');
    var prev: []const u8 = "";
    var index: usize = 0;
    const Pending = enum { none, keep_token, mask_token };
    var pending: Pending = .none;
    while (it.next()) |seg| : (index += 1) {
        if (index > 0) try out.append(gpa, '/');
        const numeric = seg.len > 0 and for (seg) |ch| {
            if (!std.ascii.isDigit(ch)) break false;
        } else true;
        const major = std.mem.eql(u8, prev, "channels") or std.mem.eql(u8, prev, "guilds") or std.mem.eql(u8, prev, "webhooks");
        switch (pending) {
            .keep_token => {
                try out.appendSlice(gpa, seg);
                pending = .none;
            },
            .mask_token => {
                try out.appendSlice(gpa, ":token");
                pending = .none;
            },
            .none => if (numeric and major) {
                try out.appendSlice(gpa, seg);
                if (std.mem.eql(u8, prev, "webhooks")) pending = .keep_token;
            } else if (numeric) {
                try out.appendSlice(gpa, ":id");
                if (std.mem.eql(u8, prev, "interactions")) pending = .mask_token;
            } else try out.appendSlice(gpa, seg),
        }
        prev = seg;
    }
    return out.toOwnedSlice(gpa);
}

const Bucket = struct { remaining: u32 = 1, reset_at_ms: u64 = 0 };

pub const Limiter = struct {
    gpa: Allocator,
    /// route key -> bucket id (owned strings)
    routes: std.StringHashMapUnmanaged([]u8) = .empty,
    /// bucket id (or route key when unknown) -> state
    buckets: std.StringHashMapUnmanaged(Bucket) = .empty,
    global_until_ms: u64 = 0,

    pub fn init(gpa: Allocator) Limiter {
        return .{ .gpa = gpa };
    }

    pub fn deinit(l: *Limiter) void {
        var r = l.routes.iterator();
        while (r.next()) |e| {
            l.gpa.free(e.key_ptr.*);
            l.gpa.free(e.value_ptr.*);
        }
        l.routes.deinit(l.gpa);
        var b = l.buckets.iterator();
        while (b.next()) |e| l.gpa.free(e.key_ptr.*);
        l.buckets.deinit(l.gpa);
    }

    fn bucketKey(l: *const Limiter, route: []const u8) []const u8 {
        return l.routes.get(route) orelse route;
    }

    /// Milliseconds to wait before sending on `route` (0 = go now). A send
    /// consumes one request from the known remaining budget.
    pub fn acquire(l: *Limiter, route: []const u8, now_ms: u64) u64 {
        if (now_ms < l.global_until_ms) return l.global_until_ms - now_ms;
        const b = l.buckets.getPtr(l.bucketKey(route)) orelse return 0;
        if (now_ms >= b.reset_at_ms) return 0;
        if (b.remaining == 0) return b.reset_at_ms - now_ms;
        b.remaining -= 1;
        return 0;
    }

    /// Record a response's headers. A 429 sets the retry deadline from
    /// `retry_after_ms` (header or body) on the bucket or globally.
    pub fn record(l: *Limiter, route: []const u8, h: Headers, status: u16, now_ms: u64) Allocator.Error!void {
        if (status == 429) {
            const wait = h.retry_after_ms orelse h.reset_after_ms orelse 1000;
            if (h.global) {
                l.global_until_ms = now_ms + wait;
                return;
            }
        }
        const id: []const u8 = if (h.bucket) |bucket| blk: {
            const gop = try l.routes.getOrPut(l.gpa, route);
            if (!gop.found_existing) {
                gop.key_ptr.* = l.gpa.dupe(u8, route) catch |e| {
                    _ = l.routes.remove(route);
                    return e;
                };
                gop.value_ptr.* = try l.gpa.dupe(u8, bucket);
            } else if (!std.mem.eql(u8, gop.value_ptr.*, bucket)) {
                const replacement = try l.gpa.dupe(u8, bucket);
                l.gpa.free(gop.value_ptr.*);
                gop.value_ptr.* = replacement;
            }
            break :blk gop.value_ptr.*;
        } else route;
        const gop = try l.buckets.getOrPut(l.gpa, id);
        if (!gop.found_existing) {
            gop.key_ptr.* = l.gpa.dupe(u8, id) catch |e| {
                _ = l.buckets.remove(id);
                return e;
            };
            gop.value_ptr.* = .{};
        }
        if (h.remaining) |r| gop.value_ptr.remaining = r;
        if (status == 429) {
            gop.value_ptr.remaining = 0;
            gop.value_ptr.reset_at_ms = now_ms + (h.retry_after_ms orelse h.reset_after_ms orelse 1000);
        } else if (h.reset_after_ms) |ra| gop.value_ptr.reset_at_ms = now_ms + ra;
    }
};

const testing = std.testing;

test "reset-after seconds parse to milliseconds, rounding up" {
    try testing.expectEqual(@as(?u64, 1250), secondsToMs("1.250"));
    try testing.expectEqual(@as(?u64, 1000), secondsToMs("1"));
    try testing.expectEqual(@as(?u64, 64), secondsToMs("0.0635"));
    try testing.expectEqual(@as(?u64, 500), secondsToMs("0.5"));
    try testing.expect(secondsToMs("") == null);
    try testing.expect(secondsToMs("abc") == null);
    try testing.expect(secondsToMs("1.2x") == null);
}

test "route keys keep major parameters and mask the rest" {
    const gpa = testing.allocator;
    const cases = [_][3][]const u8{
        .{ "POST", "/api/v10/channels/123/messages", "POST /api/v10/channels/123/messages" },
        .{ "PATCH", "/api/v10/channels/123/messages/456", "PATCH /api/v10/channels/123/messages/:id" },
        .{ "GET", "/api/v10/guilds/9/members/77", "GET /api/v10/guilds/9/members/:id" },
        .{ "POST", "/api/v10/interactions/555/tok.en/callback", "POST /api/v10/interactions/:id/:token/callback" },
        .{ "PATCH", "/api/v10/webhooks/1/abc/messages/@original", "PATCH /api/v10/webhooks/1/abc/messages/@original" },
        .{ "PUT", "/api/v10/applications/42/commands?with=x", "PUT /api/v10/applications/:id/commands" },
    };
    for (cases) |c| {
        const got = try routeKey(gpa, c[0], c[1]);
        defer gpa.free(got);
        try testing.expectEqualStrings(c[2], got);
    }
}

test "remaining zero waits until the bucket resets and shared buckets share budgets" {
    var l = Limiter.init(testing.allocator);
    defer l.deinit();
    try testing.expectEqual(@as(u64, 0), l.acquire("A", 0));
    try l.record("A", .{ .bucket = "b1", .remaining = 1, .reset_after_ms = 2000 }, 200, 0);
    try l.record("B", .{ .bucket = "b1", .remaining = 1, .reset_after_ms = 2000 }, 200, 0);
    try testing.expectEqual(@as(u64, 0), l.acquire("A", 10)); // consumes the last one
    try testing.expectEqual(@as(u64, 1990), l.acquire("B", 10)); // same bucket, now empty
    try testing.expectEqual(@as(u64, 0), l.acquire("B", 2000));
}

test "a 429 sets the retry deadline; a global 429 pauses every route" {
    var l = Limiter.init(testing.allocator);
    defer l.deinit();
    try l.record("A", .{ .retry_after_ms = 1500 }, 429, 100);
    try testing.expectEqual(@as(u64, 1500), l.acquire("A", 100));
    try testing.expectEqual(@as(u64, 0), l.acquire("C", 100));
    try l.record("C", .{ .retry_after_ms = 700, .global = true }, 429, 200);
    try testing.expectEqual(@as(u64, 700), l.acquire("Z", 200));
    try testing.expectEqual(@as(u64, 0), l.acquire("Z", 900));
}
