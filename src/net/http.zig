//! HTTP(S) transport over std.http.Client (std/http/Client.zig): TLS 1.2 and
//! 1.3 through std.crypto.tls.Client, CA verification against the system
//! bundle (Certificate.Bundle.rescan), connection reuse through the client's
//! pool. Responses are fully buffered up to a caller-chosen limit, and only
//! the headers a caller asks for are copied out.
const std = @import("std");
const Allocator = std.mem.Allocator;

pub const Method = std.http.Method;

pub const Header = struct { name: []const u8, value: []const u8 };

pub const Response = struct {
    /// The allocator that owns `headers` and `body` (the client's).
    gpa: Allocator,
    status: u16,
    /// Owned copies of the requested headers that were present.
    headers: []Header,
    body: []u8,

    pub fn deinit(r: *Response) void {
        for (r.headers) |h| {
            r.gpa.free(h.name);
            r.gpa.free(h.value);
        }
        r.gpa.free(r.headers);
        r.gpa.free(r.body);
    }

    pub fn header(r: *const Response, name: []const u8) ?[]const u8 {
        for (r.headers) |h| if (std.ascii.eqlIgnoreCase(h.name, name)) return h.value;
        return null;
    }
};

pub const Request = struct {
    method: Method,
    url: []const u8,
    /// Sent as `Authorization`; kept out of every log line by construction.
    authorization: ?[]const u8 = null,
    content_type: ?[]const u8 = null,
    body: ?[]const u8 = null,
    extra_headers: []const std.http.Header = &.{},
    /// Response headers to copy out (case-insensitive).
    keep_headers: []const []const u8 = &.{},
    max_body: usize = 4 * 1024 * 1024,
};

pub const Error = error{
    ResponseTooLarge,
    TransportFailed,
} || Allocator.Error || std.Uri.ParseError;

pub const Client = struct {
    inner: std.http.Client,

    pub fn init(gpa: Allocator, io: std.Io) Client {
        return .{ .inner = .{ .allocator = gpa, .io = io } };
    }

    pub fn deinit(c: *Client) void {
        c.inner.deinit();
    }

    /// Perform one request. Transport-level failures collapse to
    /// `error.TransportFailed` so callers never format a raw error (which may
    /// carry a hostname or path) into user-facing text.
    pub fn send(c: *Client, req: Request) Error!Response {
        const gpa = c.inner.allocator;
        const uri = try std.Uri.parse(req.url);
        var r = c.inner.request(req.method, uri, .{
            .redirect_behavior = .unhandled,
            .keep_alive = true,
            .headers = .{
                .authorization = if (req.authorization) |a| .{ .override = a } else .default,
                .content_type = if (req.content_type) |t| .{ .override = t } else .default,
                .user_agent = .{ .override = "DiscordBot (https://github.com/donaldfilimon/abbey-bot-zig, 0.1.0)" },
                .accept_encoding = .omit,
            },
            .extra_headers = req.extra_headers,
        }) catch |e| return switch (e) {
            error.OutOfMemory => error.OutOfMemory,
            else => error.TransportFailed,
        };
        defer r.deinit();
        if (req.body) |body| {
            r.transfer_encoding = .{ .content_length = body.len };
            var bw = r.sendBodyUnflushed(&.{}) catch return error.TransportFailed;
            bw.writer.writeAll(body) catch return error.TransportFailed;
            bw.end() catch return error.TransportFailed;
            r.connection.?.flush() catch return error.TransportFailed;
        } else {
            r.sendBodiless() catch return error.TransportFailed;
        }
        var redirect: [0]u8 = .{};
        var response = r.receiveHead(&redirect) catch |e| return switch (e) {
            else => error.TransportFailed,
        };
        const status: u16 = @backingInt(response.head.status);

        var kept: std.ArrayList(Header) = .empty;
        errdefer {
            for (kept.items) |h| {
                gpa.free(h.name);
                gpa.free(h.value);
            }
            kept.deinit(gpa);
        }
        var it = response.head.iterateHeaders();
        while (it.next()) |h| {
            for (req.keep_headers) |want| {
                if (!std.ascii.eqlIgnoreCase(h.name, want)) continue;
                const name = try gpa.dupe(u8, h.name);
                errdefer gpa.free(name);
                const value = try gpa.dupe(u8, h.value);
                errdefer gpa.free(value);
                try kept.append(gpa, .{ .name = name, .value = value });
            }
        }

        var transfer: [4096]u8 = undefined;
        const body_reader = response.reader(&transfer);
        const body = body_reader.allocRemaining(gpa, .limited(req.max_body)) catch |e| return switch (e) {
            error.OutOfMemory => error.OutOfMemory,
            error.StreamTooLong => error.ResponseTooLarge,
            else => error.TransportFailed,
        };
        errdefer gpa.free(body);
        const headers = try kept.toOwnedSlice(gpa);
        return .{ .gpa = gpa, .status = status, .headers = headers, .body = body };
    }
};
