//! Discord REST client: bot authentication, JSON bodies, and header-driven
//! rate limiting (`ratelimit.zig`) with bounded 429 retries. The token is held
//! only as the `Authorization` value and never formatted anywhere else.
const std = @import("std");
const http = @import("../net/http.zig");
const ratelimit = @import("ratelimit.zig");
const Allocator = std.mem.Allocator;

pub const default_api_base = "https://discord.com/api/v10";

const rate_headers = [_][]const u8{
    "x-ratelimit-bucket",
    "x-ratelimit-remaining",
    "x-ratelimit-reset-after",
    "x-ratelimit-global",
    "retry-after",
};

pub const Error = error{ RateLimited, RequestFailed } || http.Error;

pub const Rest = struct {
    gpa: Allocator,
    io: std.Io,
    http_client: *http.Client,
    /// "Bot <token>"; owned.
    authorization: []u8,
    api_base: []const u8,
    limiter: ratelimit.Limiter,
    max_retries: u8 = 3,

    pub fn init(gpa: Allocator, io: std.Io, http_client: *http.Client, token: []const u8, api_base: []const u8) Allocator.Error!Rest {
        return .{
            .gpa = gpa,
            .io = io,
            .http_client = http_client,
            .authorization = try std.fmt.allocPrint(gpa, "Bot {s}", .{token}),
            .api_base = api_base,
            .limiter = ratelimit.Limiter.init(gpa),
        };
    }

    pub fn deinit(r: *Rest) void {
        std.crypto.secureZero(u8, r.authorization);
        r.gpa.free(r.authorization);
        r.limiter.deinit();
    }

    fn nowMs(r: *Rest) u64 {
        // std/Io.zig: Clock.awake is monotonic; Timestamp.toMilliseconds
        return @intCast(@max(0, std.Io.Clock.awake.now(r.io).toMilliseconds()));
    }

    fn sleepMs(r: *Rest, ms: u64) void {
        std.Io.sleep(r.io, .fromMilliseconds(@intCast(ms)), .awake) catch {};
    }

    /// `path` starts with `/` and is relative to the API base.
    pub fn call(r: *Rest, method: http.Method, path: []const u8, json_body: ?[]const u8) Error!http.Response {
        const route = try ratelimit.routeKey(r.gpa, @tagName(method), path);
        defer r.gpa.free(route);
        const url = try std.fmt.allocPrint(r.gpa, "{s}{s}", .{ r.api_base, path });
        defer r.gpa.free(url);
        var attempt: u8 = 0;
        while (true) : (attempt += 1) {
            const wait = r.limiter.acquire(route, r.nowMs());
            if (wait > 0) r.sleepMs(wait);
            var response = try r.http_client.send(.{
                .method = method,
                .url = url,
                .authorization = r.authorization,
                .content_type = if (json_body != null) "application/json" else null,
                .body = json_body,
                .keep_headers = &rate_headers,
            });
            const headers = parseHeaders(&response);
            try r.limiter.record(route, headers, response.status, r.nowMs());
            if (response.status != 429) return response;
            response.deinit();
            if (attempt + 1 >= r.max_retries) return error.RateLimited;
        }
    }
};

/// Rate-limit fields from headers, falling back to a 429 body's
/// `retry_after` (seconds, possibly fractional) and `global`.
pub fn parseHeaders(response: *const http.Response) ratelimit.Headers {
    var h: ratelimit.Headers = .{
        .bucket = response.header("x-ratelimit-bucket"),
        .remaining = if (response.header("x-ratelimit-remaining")) |v| std.fmt.parseInt(u32, v, 10) catch null else null,
        .reset_after_ms = if (response.header("x-ratelimit-reset-after")) |v| ratelimit.secondsToMs(v) else null,
        .global = if (response.header("x-ratelimit-global")) |v| std.ascii.eqlIgnoreCase(v, "true") else false,
        .retry_after_ms = if (response.header("retry-after")) |v| ratelimit.secondsToMs(v) else null,
    };
    if (response.status == 429) {
        const Body = struct { retry_after: ?f64 = null, global: ?bool = null };
        if (std.json.parseFromSlice(Body, std.heap.page_allocator, response.body, .{ .ignore_unknown_fields = true })) |parsed| {
            defer parsed.deinit();
            if (parsed.value.retry_after) |ra| {
                if (ra >= 0 and ra < 3600) h.retry_after_ms = @intFromFloat(@ceil(ra * 1000.0));
            }
            if (parsed.value.global == true) h.global = true;
        } else |_| {}
    }
    return h;
}

const testing = std.testing;
const loopback = @import("../testing/loopback.zig");

test "rest client authenticates, follows buckets and retries a 429 after retry_after" {
    var server: loopback.Server = undefined;
    try server.start(testing.allocator, testing.io, &.{
        .{ .status = .too_many_requests, .headers = &.{ .{ .name = "x-ratelimit-bucket", .value = "abc" }, .{ .name = "x-ratelimit-remaining", .value = "0" } }, .body = "{\"message\":\"You are being rate limited.\",\"retry_after\":0.05,\"global\":false}" },
        .{ .status = .ok, .headers = &.{ .{ .name = "x-ratelimit-bucket", .value = "abc" }, .{ .name = "x-ratelimit-remaining", .value = "4" }, .{ .name = "x-ratelimit-reset-after", .value = "1.0" } }, .body = "{\"id\":\"1\"}" },
    });
    defer server.deinit();
    var client = http.Client.init(testing.allocator, testing.io);
    defer client.deinit();
    var base_buf: [64]u8 = undefined;
    const base = try std.fmt.bufPrint(&base_buf, "http://127.0.0.1:{d}/api/v10", .{server.port});
    var rest = try Rest.init(testing.allocator, testing.io, &client, "test-token", base);
    defer rest.deinit();
    const started = std.Io.Clock.awake.now(testing.io);
    var response = try rest.call(.POST, "/channels/123/messages", "{\"content\":\"hi\"}");
    defer response.deinit();
    const elapsed = started.untilNow(testing.io, .awake).toMilliseconds();
    server.join();
    try testing.expect(server.failure == null);
    try testing.expectEqual(@as(u16, 200), response.status);
    try testing.expectEqualStrings("{\"id\":\"1\"}", response.body);
    try testing.expectEqual(@as(usize, 2), server.seen.items.len);
    for (server.seen.items) |seen| {
        try testing.expectEqualStrings("/api/v10/channels/123/messages", seen.target);
        try testing.expectEqualStrings("Bot test-token", seen.authorization.?);
        try testing.expectEqualStrings("{\"content\":\"hi\"}", seen.body);
    }
    try testing.expect(elapsed >= 50); // waited for retry_after before the retry
}

test "persistent 429s stop after the retry budget" {
    var server: loopback.Server = undefined;
    const limited: loopback.Scripted = .{ .status = .too_many_requests, .body = "{\"retry_after\":0.001,\"global\":true}" };
    try server.start(testing.allocator, testing.io, &.{ limited, limited, limited });
    defer server.deinit();
    var client = http.Client.init(testing.allocator, testing.io);
    defer client.deinit();
    var base_buf: [64]u8 = undefined;
    var rest = try Rest.init(testing.allocator, testing.io, &client, "t", try std.fmt.bufPrint(&base_buf, "http://127.0.0.1:{d}/api/v10", .{server.port}));
    defer rest.deinit();
    try testing.expectError(error.RateLimited, rest.call(.GET, "/users/@me", null));
    server.join();
    try testing.expectEqual(@as(usize, 3), server.seen.items.len);
}
