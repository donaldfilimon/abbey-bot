//! One gateway WebSocket connection over a TLS connection from
//! std.http.Client (connectTcp with .tls: std/http/Client.zig), so the WSS
//! path uses the same TLS client and CA bundle as REST.
const std = @import("std");
const ws = @import("ws.zig");
const session = @import("session.zig");
const HttpClient = std.http.Client;
const Allocator = std.mem.Allocator;

pub const Error = error{ InvalidGatewayUrl, TransportFailed, CertificateBundleLoadFailure } || ws.Error;

/// Load the system CA bundle once, the way std.http.Client.request does.
pub fn ensureBundle(client: *HttpClient) error{ CertificateBundleLoadFailure, Canceled, OutOfMemory }!void {
    const io = client.io;
    {
        try client.ca_bundle_lock.lockShared(io);
        defer client.ca_bundle_lock.unlockShared(io);
        if (client.now != null) return;
    }
    var bundle: std.crypto.Certificate.Bundle = .empty;
    defer bundle.deinit(client.allocator);
    const now = std.Io.Clock.real.now(io);
    bundle.rescan(client.allocator, io, now) catch |err| switch (err) {
        error.Canceled => |e| return e,
        else => return error.CertificateBundleLoadFailure,
    };
    try client.ca_bundle_lock.lock(io);
    defer client.ca_bundle_lock.unlock(io);
    client.now = now;
    std.mem.swap(std.crypto.Certificate.Bundle, &client.ca_bundle, &bundle);
}

/// Split `wss://host[:port][/...]` into host and port. Discord hands out
/// `wss://gateway.discord.gg` and resume URLs of the same form.
pub fn parseGatewayUrl(url: []const u8) Error!struct { host: []const u8, port: u16 } {
    const rest = if (std.mem.startsWith(u8, url, "wss://")) url["wss://".len..] else return error.InvalidGatewayUrl;
    const end = std.mem.indexOfAny(u8, rest, "/?#") orelse rest.len;
    const authority = rest[0..end];
    if (authority.len == 0) return error.InvalidGatewayUrl;
    if (std.mem.lastIndexOfScalar(u8, authority, ':')) |colon| {
        const port = std.fmt.parseInt(u16, authority[colon + 1 ..], 10) catch return error.InvalidGatewayUrl;
        return .{ .host = authority[0..colon], .port = port };
    }
    return .{ .host = authority, .port = 443 };
}

/// std/http/Client.zig Connection.flush: TLS writer, then socket writer.
fn flushConnection(ctx: *anyopaque) std.Io.Writer.Error!void {
    const connection: *HttpClient.Connection = @ptrCast(@alignCast(ctx));
    return connection.flush();
}

pub const Conn = struct {
    client: *HttpClient,
    connection: *HttpClient.Connection,
    socket: ws.Client,

    /// Connect, complete TLS and the WebSocket opening handshake.
    pub fn open(gpa: Allocator, client: *HttpClient, url: []const u8, max_message: usize) Error!Conn {
        const target = try parseGatewayUrl(url);
        ensureBundle(client) catch |e| return switch (e) {
            error.OutOfMemory => error.OutOfMemory,
            else => error.CertificateBundleLoadFailure,
        };
        const host = std.Io.net.HostName.init(target.host) catch return error.InvalidGatewayUrl;
        const connection = client.connectTcp(host, target.port, .tls) catch return error.TransportFailed;
        errdefer {
            connection.closing = true;
            client.connection_pool.release(connection, client.io);
        }
        var key: [16]u8 = undefined;
        client.io.random(&key);
        const flusher: ws.Flusher = .{ .ctx = connection, .func = flushConnection };
        try ws.handshake(connection.reader(), connection.writer(), flusher, target.host, session.gateway_query, key);
        return .{
            .client = client,
            .connection = connection,
            .socket = ws.Client.init(gpa, client.io, connection.reader(), connection.writer(), flusher, max_message),
        };
    }

    pub fn sendText(c: *Conn, payload: []const u8) Error!void {
        try c.socket.sendText(payload);
    }

    pub fn close(c: *Conn, code: u16) void {
        c.socket.sendClose(code, "") catch {};
        c.socket.deinit();
        c.connection.closing = true;
        c.client.connection_pool.release(c.connection, c.client.io);
    }
};

test "gateway URLs parse to host and port" {
    const a = try parseGatewayUrl("wss://gateway.discord.gg");
    try std.testing.expectEqualStrings("gateway.discord.gg", a.host);
    try std.testing.expectEqual(@as(u16, 443), a.port);
    const b = try parseGatewayUrl("wss://gateway-us-east1-b.discord.gg:8443/?v=10");
    try std.testing.expectEqualStrings("gateway-us-east1-b.discord.gg", b.host);
    try std.testing.expectEqual(@as(u16, 8443), b.port);
    try std.testing.expectError(error.InvalidGatewayUrl, parseGatewayUrl("https://discord.com"));
    try std.testing.expectError(error.InvalidGatewayUrl, parseGatewayUrl("wss://"));
}
