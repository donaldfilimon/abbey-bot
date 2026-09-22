//! The gateway runtime: connect, Identify or Resume, heartbeat on a timer
//! thread, dispatch interactions to worker threads, reconnect by the session
//! state machine's rules (`gateway/session.zig`). Interaction workers defer
//! first (Discord invalidates an interaction token after 3 seconds), then
//! compute the reply and edit the original response.
const std = @import("std");
const App = @import("app.zig").App;
const handlers = @import("handlers.zig");
const registration = @import("registration.zig");
const reply_mod = @import("reply.zig");
const session_mod = @import("../gateway/session.zig");
const conn_mod = @import("../gateway/conn.zig");
const interaction_mod = @import("../discord/interaction.zig");
const rest_mod = @import("../discord/rest.zig");
const catalog = @import("../catalog/catalog.zig");
const Allocator = std.mem.Allocator;

pub const DiscordState = enum(u8) { connecting, ready, stopped };

pub const Options = struct {
    token: []const u8,
    intents: u64,
    /// null: ask `GET /gateway/bot`.
    gateway_url: ?[]const u8 = null,
    home_guild: ?u64 = null,
    /// Tests stop after this many completed connections.
    max_connections: ?usize = null,
};

pub const Bot = struct {
    app: *App,
    rest: *rest_mod.Rest,
    opts: Options,
    session: session_mod.Session,
    session_lock: std.Io.Mutex = .init,
    write_lock: std.Io.Mutex = .init,
    stop: std.atomic.Value(bool) = .init(false),
    workers: std.atomic.Value(usize) = .init(0),
    registered: std.atomic.Value(bool) = .init(false),
    /// Observed by serve's readiness writer.
    discord_state: std.atomic.Value(DiscordState) = .init(.connecting),
    registration_done: std.atomic.Value(bool) = .init(false),
    application_id: std.atomic.Value(u64) = .init(0),
    /// Set when the loop ends on a fatal close code.
    fatal_close: ?u16 = null,
    heartbeats_sent: std.atomic.Value(u64) = .init(0),

    pub fn init(app: *App, rest: *rest_mod.Rest, opts: Options) Bot {
        return .{ .app = app, .rest = rest, .opts = opts, .session = session_mod.Session.init(app.gpa) };
    }

    pub fn deinit(b: *Bot) void {
        b.waitForWorkers();
        b.session.deinit();
    }

    pub fn requestStop(b: *Bot) void {
        b.stop.store(true, .release);
    }

    pub fn waitForWorkers(b: *Bot) void {
        while (b.workers.load(.acquire) > 0) std.Io.sleep(b.app.io, .fromMilliseconds(5), .awake) catch {};
    }

    fn monoMs(b: *Bot) u64 {
        return @intCast(@max(0, std.Io.Clock.awake.now(b.app.io).toMilliseconds()));
    }

    fn jitter(b: *Bot) f64 {
        var r: [8]u8 = undefined;
        b.app.io.random(&r);
        return @as(f64, @floatFromInt(std.mem.readInt(u64, &r, .little) >> 11)) / 9007199254740992.0;
    }

    fn gatewayUrl(b: *Bot, arena: Allocator) ![]const u8 {
        if (b.opts.gateway_url) |u| return u;
        var response = try b.rest.call(.GET, "/gateway/bot", null);
        defer response.deinit();
        if (response.status != 200) return error.GatewayUrlUnavailable;
        const Body = struct { url: []const u8 };
        const parsed = try std.json.parseFromSliceLeaky(Body, arena, response.body, .{ .ignore_unknown_fields = true, .allocate = .alloc_always });
        return parsed.url;
    }

    /// Run until stopped or a fatal close. Returns the fatal close code, if any.
    pub fn run(b: *Bot) !void {
        var connections: usize = 0;
        var backoff_ms: u64 = 1000;
        var kind: session_mod.ReconnectKind = .reidentify;
        while (!b.stop.load(.acquire)) {
            if (b.opts.max_connections) |max| if (connections >= max) break;
            var arena_state: std.heap.ArenaAllocator = .init(b.app.gpa);
            defer arena_state.deinit();
            const arena = arena_state.allocator();
            b.discord_state.store(.connecting, .release);
            const url = blk: {
                b.session_lock.lockUncancelable(b.app.io);
                defer b.session_lock.unlock(b.app.io);
                if (kind == .resume_session) if (b.session.resume_url) |r| break :blk try arena.dupe(u8, r);
                break :blk null;
            } orelse (b.gatewayUrl(arena) catch {
                b.sleepMs(backoff_ms);
                backoff_ms = @min(backoff_ms * 2, 60_000);
                continue;
            });
            const full = try std.fmt.allocPrint(arena, "{s}{s}", .{ std.mem.trimEnd(u8, url, "/"), session_mod.gateway_query });
            var conn = conn_mod.Conn.open(b.app.gpa, &b.app.http_client.inner, full, 16 * 1024 * 1024) catch {
                b.sleepMs(backoff_ms);
                backoff_ms = @min(backoff_ms * 2, 60_000);
                continue;
            };
            conn.socket.write_lock = &b.write_lock;
            connections += 1;
            {
                b.session_lock.lockUncancelable(b.app.io);
                defer b.session_lock.unlock(b.app.io);
                b.session.connected(kind);
            }
            var hb_stop: std.atomic.Value(bool) = .init(false);
            const hb = try std.Thread.spawn(.{}, heartbeatLoop, .{ b, &conn, &hb_stop });
            const action = b.readLoop(&conn);
            hb_stop.store(true, .release);
            hb.join();
            conn.close(1000);
            b.discord_state.store(.stopped, .release);
            switch (action) {
                .stop => |code| {
                    b.fatal_close = code;
                    return;
                },
                .reconnect => |k| {
                    kind = k;
                    backoff_ms = 1000;
                },
                else => kind = .resume_session,
            }
        }
    }

    fn sleepMs(b: *Bot, ms: u64) void {
        var left = ms;
        while (left > 0 and !b.stop.load(.acquire)) {
            const step = @min(left, 100);
            std.Io.sleep(b.app.io, .fromMilliseconds(@intCast(step)), .awake) catch {};
            left -= step;
        }
    }

    fn sendJson(_: *Bot, conn: *conn_mod.Conn, payload: []const u8) void {
        conn.sendText(payload) catch {};
    }

    fn heartbeatLoop(b: *Bot, conn: *conn_mod.Conn, hb_stop: *std.atomic.Value(bool)) void {
        while (!hb_stop.load(.acquire)) {
            std.Io.sleep(b.app.io, .fromMilliseconds(50), .awake) catch {};
            if (b.stop.load(.acquire)) {
                // Wake the reader so run() can exit.
                conn.connection.stream_reader.stream.shutdown(b.app.io, .both) catch {};
                return;
            }
            var buf: [64]u8 = undefined;
            var w: std.Io.Writer = .fixed(&buf);
            const act = blk: {
                b.session_lock.lockUncancelable(b.app.io);
                defer b.session_lock.unlock(b.app.io);
                const a = b.session.tick(b.monoMs());
                if (a == .send_heartbeat_now) {
                    session_mod.writeHeartbeat(&w, b.session.seq) catch {};
                    b.session.heartbeatSent(b.monoMs());
                }
                break :blk a;
            };
            switch (act) {
                .send_heartbeat_now => {
                    b.sendJson(conn, w.buffered());
                    _ = b.heartbeats_sent.fetchAdd(1, .monotonic);
                },
                .reconnect => {
                    // Zombie connection: no ACK since the last heartbeat.
                    conn.connection.stream_reader.stream.shutdown(b.app.io, .both) catch {};
                    return;
                },
                else => {},
            }
        }
    }

    fn readLoop(b: *Bot, conn: *conn_mod.Conn) session_mod.Action {
        while (true) {
            const message = conn.socket.readMessage() catch {
                if (b.stop.load(.acquire)) return .{ .stop = 1000 };
                b.session_lock.lockUncancelable(b.app.io);
                defer b.session_lock.unlock(b.app.io);
                return b.session.onTransportError();
            };
            switch (message) {
                .close => |c| {
                    if (b.stop.load(.acquire)) return .{ .stop = 1000 };
                    b.session_lock.lockUncancelable(b.app.io);
                    defer b.session_lock.unlock(b.app.io);
                    return b.session.onClose(c.code);
                },
                .binary => continue, // compression is off; nothing binary is expected
                .text => |payload| {
                    const act = b.handlePayload(conn, payload) catch continue;
                    switch (act) {
                        .reconnect, .stop => return act,
                        else => {},
                    }
                },
            }
        }
    }

    fn handlePayload(b: *Bot, conn: *conn_mod.Conn, payload: []const u8) !session_mod.Action {
        var arena_state: std.heap.ArenaAllocator = .init(b.app.gpa);
        defer arena_state.deinit();
        const arena = arena_state.allocator();
        const v = try std.json.parseFromSliceLeaky(std.json.Value, arena, payload, .{});
        if (v != .object) return .none;
        const op_v = v.object.get("op") orelse return .none;
        if (op_v != .integer) return .none;
        const op: session_mod.Op = @fromBackingInt(@intCast(@as(u8, @intCast(std.math.clamp(op_v.integer, 0, 255)))));
        const d = v.object.get("d") orelse .null;
        switch (op) {
            .hello => {
                const interval: u64 = if (d == .object) (if (d.object.get("heartbeat_interval")) |h| (if (h == .integer and h.integer > 0) @intCast(h.integer) else 41_250) else 41_250) else 41_250;
                var out: std.Io.Writer.Allocating = .init(arena);
                const act = blk: {
                    b.session_lock.lockUncancelable(b.app.io);
                    defer b.session_lock.unlock(b.app.io);
                    const a = b.session.onHello(interval, b.monoMs(), b.jitter());
                    switch (a) {
                        .send_identify => try session_mod.writeIdentify(&out.writer, b.opts.token, b.opts.intents, .{}),
                        .send_resume => try session_mod.writeResume(&out.writer, b.opts.token, b.session.session_id orelse "", b.session.seq),
                        else => {},
                    }
                    break :blk a;
                };
                if (out.written().len > 0) try conn.sendText(out.written());
                return act;
            },
            .heartbeat_ack => {
                b.session_lock.lockUncancelable(b.app.io);
                defer b.session_lock.unlock(b.app.io);
                b.session.onHeartbeatAck(b.monoMs());
                return .none;
            },
            .heartbeat => {
                var buf: [64]u8 = undefined;
                var w: std.Io.Writer = .fixed(&buf);
                {
                    b.session_lock.lockUncancelable(b.app.io);
                    defer b.session_lock.unlock(b.app.io);
                    try session_mod.writeHeartbeat(&w, b.session.seq);
                    b.session.heartbeatSent(b.monoMs());
                }
                try conn.sendText(w.buffered());
                return .none;
            },
            .reconnect => {
                b.session_lock.lockUncancelable(b.app.io);
                defer b.session_lock.unlock(b.app.io);
                return b.session.onReconnectRequest();
            },
            .invalid_session => {
                const resumable = d == .bool and d.bool;
                // Discord asks for a 1-5 s pause before identifying again.
                b.sleepMs(1000 + @as(u64, @intFromFloat(b.jitter() * 4000)));
                b.session_lock.lockUncancelable(b.app.io);
                defer b.session_lock.unlock(b.app.io);
                return b.session.onInvalidSession(resumable);
            },
            .dispatch => {
                const seq: ?u64 = if (v.object.get("s")) |s| (if (s == .integer and s.integer >= 0) @intCast(s.integer) else null) else null;
                const t = if (v.object.get("t")) |x| (if (x == .string) x.string else "") else "";
                b.session_lock.lockUncancelable(b.app.io);
                b.session.onDispatchSeq(seq);
                if (std.mem.eql(u8, t, "READY") and d == .object) {
                    const sid = if (d.object.get("session_id")) |x| (if (x == .string) x.string else "") else "";
                    const rurl = if (d.object.get("resume_gateway_url")) |x| (if (x == .string) x.string else "") else "";
                    b.session.onReady(sid, rurl) catch {};
                    b.session_lock.unlock(b.app.io);
                    if (d.object.get("application")) |app_v| if (app_v == .object) if (app_v.object.get("id")) |id_v| if (id_v == .string) {
                        b.application_id.store(std.fmt.parseInt(u64, id_v.string, 10) catch 0, .release);
                    };
                    b.discord_state.store(.ready, .release);
                    if (!b.registered.swap(true, .acq_rel)) try b.spawnWorker(registerWorker, "");
                    return .none;
                }
                if (std.mem.eql(u8, t, "RESUMED")) b.session.onResumed();
                b.session_lock.unlock(b.app.io);
                if (std.mem.eql(u8, t, "RESUMED")) b.discord_state.store(.ready, .release);
                if (std.mem.eql(u8, t, "INTERACTION_CREATE")) try b.spawnWorker(interactionWorker, payload);
                return .none;
            },
            else => return .none,
        }
    }

    fn spawnWorker(b: *Bot, comptime f: fn (*Bot, []u8) void, payload: []const u8) !void {
        const owned = try b.app.gpa.dupe(u8, payload);
        errdefer b.app.gpa.free(owned);
        _ = b.workers.fetchAdd(1, .acq_rel);
        const t = std.Thread.spawn(.{}, workerMain, .{ b, f, owned }) catch |e| {
            _ = b.workers.fetchSub(1, .acq_rel);
            return e;
        };
        t.detach();
    }

    fn workerMain(b: *Bot, comptime f: fn (*Bot, []u8) void, payload: []u8) void {
        defer _ = b.workers.fetchSub(1, .acq_rel);
        defer b.app.gpa.free(payload);
        f(b, payload);
    }

    fn registerWorker(b: *Bot, _: []u8) void {
        var arena_state: std.heap.ArenaAllocator = .init(b.app.gpa);
        defer arena_state.deinit();
        const app_id = b.application_id.load(.acquire);
        if (app_id == 0) return;
        registration.register(b.rest, arena_state.allocator(), app_id, b.opts.home_guild) catch return;
        b.registration_done.store(true, .release);
    }

    fn interactionWorker(b: *Bot, payload: []u8) void {
        var arena_state: std.heap.ArenaAllocator = .init(b.app.gpa);
        defer arena_state.deinit();
        b.handleInteraction(arena_state.allocator(), payload) catch {};
    }

    fn handleInteraction(b: *Bot, arena: Allocator, payload: []const u8) !void {
        const v = try std.json.parseFromSliceLeaky(std.json.Value, arena, payload, .{});
        const d = v.object.get("d") orelse return;
        const i = try interaction_mod.parse(arena, d);
        var private = true;
        var reply: reply_mod.Reply = undefined;
        switch (i.kind) {
            .application_command => {
                const spec = catalog.commandByName(i.command_path) orelse return;
                private = spec.private;
                var top_it = std.mem.splitScalar(u8, i.command_path, ' ');
                if (!registration.isHandled(top_it.first())) return; // never registered here
                try b.acknowledge(arena, &i, private);
                reply = if (handlers.guard(b.app, &i, spec)) |message|
                    .{ .content = message }
                else
                    handlers.dispatch(b.app, arena, &i) catch |e| switch (e) {
                        error.OutOfMemory => return e,
                        else => reply_mod.Reply{ .content = "Something went wrong while answering. Please try again." },
                    };
            },
            .message_component => {
                const id = i.custom_id orelse return;
                if (!std.mem.startsWith(u8, id, "abbey:help:")) return;
                try b.acknowledge(arena, &i, true);
                reply = try handlers.helpComponent(b.app, arena, &i);
            },
            else => return,
        }
        var body: std.Io.Writer.Allocating = .init(arena);
        try reply_mod.writeEditBody(&body.writer, reply);
        var edited = try b.rest.call(.PATCH, try std.fmt.allocPrint(arena, "/webhooks/{d}/{s}/messages/@original", .{ i.application_id, i.token }), body.written());
        edited.deinit();
    }

    fn acknowledge(b: *Bot, arena: Allocator, i: *const interaction_mod.Interaction, private: bool) !void {
        var body: std.Io.Writer.Allocating = .init(arena);
        try reply_mod.writeDeferBody(&body.writer, private);
        var ack = try b.rest.call(.POST, try std.fmt.allocPrint(arena, "/interactions/{d}/{s}/callback", .{ i.id, i.token }), body.written());
        ack.deinit();
    }
};
