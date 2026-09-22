//! Token-free live probe: GET /api/v10/gateway over HTTPS, open the gateway
//! WebSocket over TLS, read Hello (op 10), close with 1000. It never sends
//! Identify, so it needs no bot token and cannot touch any bot's session.
const std = @import("std");
const http = @import("../net/http.zig");
const conn = @import("conn.zig");

pub const Report = struct { gateway_url_ok: bool, heartbeat_interval_ms: u64 };

pub fn run(gpa: std.mem.Allocator, client: *http.Client) !Report {
    var response = try client.send(.{ .method = .GET, .url = "https://discord.com/api/v10/gateway" });
    defer response.deinit(gpa);
    if (response.status != 200) return error.UnexpectedStatus;
    const Body = struct { url: []const u8 };
    var parsed = try std.json.parseFromSlice(Body, gpa, response.body, .{ .ignore_unknown_fields = true });
    defer parsed.deinit();
    var c = try conn.Conn.open(gpa, &client.inner, parsed.value.url, 1 << 20);
    defer c.close(1000);
    const message = try c.socket.readMessage();
    if (message != .text) return error.UnexpectedFrame;
    const Hello = struct { op: u8, d: struct { heartbeat_interval: u64 } };
    var hello = try std.json.parseFromSlice(Hello, gpa, message.text, .{ .ignore_unknown_fields = true });
    defer hello.deinit();
    if (hello.value.op != 10) return error.UnexpectedOpcode;
    return .{ .gateway_url_ok = std.mem.startsWith(u8, parsed.value.url, "wss://"), .heartbeat_interval_ms = hello.value.d.heartbeat_interval };
}
