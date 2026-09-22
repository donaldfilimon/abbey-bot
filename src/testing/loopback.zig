//! A scripted loopback HTTP/1.1 server for tests (std.http.Server over a
//! 127.0.0.1 listener on an ephemeral port). It answers `responses` in order
//! on one keep-alive connection (or new connections as the client opens
//! them) and records every request it saw.
const std = @import("std");
const Allocator = std.mem.Allocator;

pub const Scripted = struct {
    status: std.http.Status = .ok,
    headers: []const std.http.Header = &.{},
    body: []const u8 = "",
};

pub const Seen = struct {
    method: std.http.Method,
    target: []u8,
    authorization: ?[]u8,
    body: []u8,
};

pub const Server = struct {
    gpa: Allocator,
    io: std.Io,
    listener: std.Io.net.Server,
    port: u16,
    responses: []const Scripted,
    seen: std.ArrayList(Seen) = .empty,
    thread: ?std.Thread = null,
    failure: ?anyerror = null,

    pub fn start(s: *Server, gpa: Allocator, io: std.Io, responses: []const Scripted) !void {
        const address: std.Io.net.IpAddress = .{ .ip4 = .loopback(0) };
        s.* = .{ .gpa = gpa, .io = io, .listener = try address.listen(io, .{ .reuse_address = true }), .port = 0, .responses = responses };
        s.port = s.listener.socket.address.getPort();
        s.thread = try std.Thread.spawn(.{}, serve, .{s});
    }

    pub fn join(s: *Server) void {
        if (s.thread) |t| t.join();
        s.thread = null;
    }

    pub fn deinit(s: *Server) void {
        s.join();
        s.listener.deinit(s.io);
        for (s.seen.items) |r| {
            s.gpa.free(r.target);
            if (r.authorization) |a| s.gpa.free(a);
            s.gpa.free(r.body);
        }
        s.seen.deinit(s.gpa);
    }

    fn serve(s: *Server) void {
        s.serveInner() catch |e| {
            s.failure = e;
        };
    }

    fn serveInner(s: *Server) !void {
        var answered: usize = 0;
        while (answered < s.responses.len) {
            const stream = try s.listener.accept(s.io);
            defer stream.close(s.io);
            var in_buf: [16 * 1024]u8 = undefined;
            var out_buf: [16 * 1024]u8 = undefined;
            var reader = stream.reader(s.io, &in_buf);
            var writer = stream.writer(s.io, &out_buf);
            var http = std.http.Server.init(&reader.interface, &writer.interface);
            while (answered < s.responses.len) {
                var request = http.receiveHead() catch break;
                var auth: ?[]u8 = null;
                var it = request.iterateHeaders();
                while (it.next()) |h| {
                    if (std.ascii.eqlIgnoreCase(h.name, "authorization")) auth = try s.gpa.dupe(u8, h.value);
                }
                errdefer if (auth) |a| s.gpa.free(a);
                const target = try s.gpa.dupe(u8, request.head.target);
                errdefer s.gpa.free(target);
                const method = request.head.method;
                var body_buf: [4096]u8 = undefined;
                const body_reader = request.readerExpectNone(&body_buf);
                const body = try body_reader.allocRemaining(s.gpa, .limited(1 << 20));
                errdefer s.gpa.free(body);
                try s.seen.append(s.gpa, .{ .method = method, .target = target, .authorization = auth, .body = body });
                const r = s.responses[answered];
                answered += 1;
                try request.respond(r.body, .{ .status = r.status, .extra_headers = r.headers, .keep_alive = true });
            }
        }
    }
};
