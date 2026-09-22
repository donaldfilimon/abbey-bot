//! A scripted loopback HTTP/1.1 server for tests (std.http.Server over a
//! 127.0.0.1 listener on an ephemeral port). Responses are handed out in
//! arrival order across any number of concurrent keep-alive connections
//! (one thread per connection), and every request is recorded.
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
    mutex: std.Io.Mutex = .init,
    seen: std.ArrayList(Seen) = .empty,
    answered: std.atomic.Value(usize) = .init(0),
    accept_thread: ?std.Thread = null,
    handlers: std.ArrayList(std.Thread) = .empty,
    stopping: std.atomic.Value(bool) = .init(false),
    failure: ?anyerror = null,

    pub fn start(s: *Server, gpa: Allocator, io: std.Io, responses: []const Scripted) !void {
        const address: std.Io.net.IpAddress = .{ .ip4 = .loopback(0) };
        s.* = .{ .gpa = gpa, .io = io, .listener = try address.listen(io, .{ .reuse_address = true }), .port = 0, .responses = responses };
        s.port = s.listener.socket.address.getPort();
        s.accept_thread = try std.Thread.spawn(.{}, acceptLoop, .{s});
    }

    /// Wait (up to 30 s) until every scripted response has been sent.
    pub fn join(s: *Server) void {
        var waited: usize = 0;
        while (s.answered.load(.acquire) < s.responses.len and waited < 3000) : (waited += 1) {
            std.Io.sleep(s.io, .fromMilliseconds(10), .awake) catch {};
        }
    }

    pub fn deinit(s: *Server) void {
        s.stopping.store(true, .release);
        // Unblock accept by closing the listener, then collect every thread.
        s.listener.deinit(s.io);
        if (s.accept_thread) |t| t.join();
        for (s.handlers.items) |t| t.join();
        s.handlers.deinit(s.gpa);
        for (s.seen.items) |r| {
            s.gpa.free(r.target);
            if (r.authorization) |a| s.gpa.free(a);
            s.gpa.free(r.body);
        }
        s.seen.deinit(s.gpa);
    }

    fn acceptLoop(s: *Server) void {
        while (!s.stopping.load(.acquire)) {
            const stream = s.listener.accept(s.io) catch return;
            const t = std.Thread.spawn(.{}, connection, .{ s, stream }) catch {
                stream.close(s.io);
                continue;
            };
            s.mutex.lockUncancelable(s.io);
            s.handlers.append(s.gpa, t) catch {};
            s.mutex.unlock(s.io);
        }
    }

    fn connection(s: *Server, stream: std.Io.net.Stream) void {
        defer stream.close(s.io);
        s.serveConnection(stream) catch |e| {
            s.mutex.lockUncancelable(s.io);
            s.failure = e;
            s.mutex.unlock(s.io);
        };
    }

    fn serveConnection(s: *Server, stream: std.Io.net.Stream) !void {
        var in_buf: [16 * 1024]u8 = undefined;
        var out_buf: [16 * 1024]u8 = undefined;
        var reader = stream.reader(s.io, &in_buf);
        var writer = stream.writer(s.io, &out_buf);
        var http = std.http.Server.init(&reader.interface, &writer.interface);
        while (true) {
            var request = http.receiveHead() catch return;
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
            s.mutex.lockUncancelable(s.io);
            const index = s.seen.items.len;
            s.seen.append(s.gpa, .{ .method = method, .target = target, .authorization = auth, .body = body }) catch |e| {
                s.mutex.unlock(s.io);
                return e;
            };
            s.mutex.unlock(s.io);
            const r = if (index < s.responses.len) s.responses[index] else Scripted{ .status = .service_unavailable, .body = "unscripted" };
            try request.respond(r.body, .{ .status = r.status, .extra_headers = r.headers, .keep_alive = true });
            _ = s.answered.fetchAdd(1, .release);
        }
    }

    /// Copy of what was seen so far (under the lock), for assertions.
    pub fn snapshot(s: *Server) []const Seen {
        s.mutex.lockUncancelable(s.io);
        defer s.mutex.unlock(s.io);
        return s.seen.items;
    }
};
