//! Discord gateway session state machine (pure; no sockets, no clock reads).
//!
//! Decisions follow the oracle's transport, serenity 0.12.5
//! `src/gateway/shard.rs` (`handle_event`, `handle_gateway_closed`,
//! `do_heartbeat`, `reconnection_type`), read from the local cargo registry:
//! resume when a session id exists and the close was not 4004; forget the
//! session on 4006/4009; reset the sequence on 4007; stop on 4003, 4004, 4010,
//! 4011, 4013 and 4014; reconnect on a missed heartbeat ACK.
//!
//! One deliberate difference, required by the rewrite scope: the first
//! heartbeat after Hello waits `interval * jitter` (jitter uniform in [0, 1)),
//! as Discord's gateway documentation asks; serenity sends it immediately.
const std = @import("std");
const Allocator = std.mem.Allocator;

pub const api_version = 10;

pub const Op = enum(u8) {
    dispatch = 0,
    heartbeat = 1,
    identify = 2,
    presence_update = 3,
    voice_state_update = 4,
    resume_session = 6,
    reconnect = 7,
    request_guild_members = 8,
    invalid_session = 9,
    hello = 10,
    heartbeat_ack = 11,
    _,
};

/// Gateway intent bits (serenity 0.12.5 `model/gateway.rs`).
pub const intents = struct {
    pub const guilds: u64 = 1 << 0;
    pub const guild_members: u64 = 1 << 1;
    pub const guild_voice_states: u64 = 1 << 7;
    pub const guild_presences: u64 = 1 << 8;
    pub const guild_messages: u64 = 1 << 9;
    pub const direct_messages: u64 = 1 << 12;
    pub const message_content: u64 = 1 << 15;
    /// Every defined flag except GUILD_MEMBERS, GUILD_PRESENCES and
    /// MESSAGE_CONTENT (`GatewayIntents::non_privileged()`).
    pub const non_privileged: u64 = blk: {
        var v: u64 = 0;
        for ([_]u6{ 0, 2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 13, 14, 16, 20, 21, 24, 25 }) |b| v |= @as(u64, 1) << b;
        break :blk v;
    };

    /// The oracle's `startup.rs`: non_privileged | GUILD_VOICE_STATES, plus
    /// MESSAGE_CONTENT only when ABBEY_MESSAGE_CONTENT=1.
    pub fn forBot(with_message_content: bool) u64 {
        var v = non_privileged | guild_voice_states;
        if (with_message_content) v |= message_content;
        return v;
    }
};

pub const Stage = enum { disconnected, handshake, identifying, resuming, connected };

pub const ReconnectKind = enum { resume_session, reidentify };

pub const Action = union(enum) {
    none,
    send_identify,
    send_resume,
    send_heartbeat_now,
    reconnect: ReconnectKind,
    /// A close the oracle treats as fatal; carries the close code.
    stop: u16,
};

pub const Session = struct {
    gpa: Allocator,
    stage: Stage = .disconnected,
    session_id: ?[]u8 = null,
    resume_url: ?[]u8 = null,
    seq: ?u64 = null,
    heartbeat_interval_ms: ?u64 = null,
    /// Monotonic ms of the next heartbeat.
    next_heartbeat_ms: ?u64 = null,
    acked: bool = true,
    last_sent_ms: ?u64 = null,
    last_ack_ms: ?u64 = null,

    pub fn init(gpa: Allocator) Session {
        return .{ .gpa = gpa };
    }

    pub fn deinit(s: *Session) void {
        if (s.session_id) |v| s.gpa.free(v);
        if (s.resume_url) |v| s.gpa.free(v);
    }

    fn forgetSession(s: *Session) void {
        if (s.session_id) |v| s.gpa.free(v);
        s.session_id = null;
        if (s.resume_url) |v| s.gpa.free(v);
        s.resume_url = null;
    }

    pub fn reconnectionType(s: *const Session) ReconnectKind {
        return if (s.session_id != null) .resume_session else .reidentify;
    }

    /// A fresh socket is open and the WebSocket handshake completed.
    pub fn connected(s: *Session, kind: ReconnectKind) void {
        s.stage = if (kind == .resume_session and s.session_id != null) .resuming else .handshake;
        s.heartbeat_interval_ms = null;
        s.next_heartbeat_ms = null;
        s.acked = true;
    }

    /// op 10. `jitter` is uniform in [0, 1).
    pub fn onHello(s: *Session, interval_ms: u64, now_ms: u64, jitter: f64) Action {
        s.heartbeat_interval_ms = interval_ms;
        const first: u64 = @intFromFloat(@as(f64, @floatFromInt(interval_ms)) * std.math.clamp(jitter, 0.0, 1.0));
        s.next_heartbeat_ms = now_ms + first;
        s.acked = true;
        return switch (s.stage) {
            .handshake => blk: {
                s.stage = .identifying;
                break :blk .send_identify;
            },
            .resuming => .send_resume,
            else => .{ .reconnect = s.reconnectionType() }, // late Hello
        };
    }

    /// Returns `send_heartbeat_now` when due, `reconnect` when the previous
    /// heartbeat was never acknowledged (zombie connection), else `none`.
    pub fn tick(s: *Session, now_ms: u64) Action {
        const due = s.next_heartbeat_ms orelse return .none;
        if (now_ms < due) return .none;
        if (!s.acked) return .{ .reconnect = s.reconnectionType() };
        return .send_heartbeat_now;
    }

    /// Record that a heartbeat went out.
    pub fn heartbeatSent(s: *Session, now_ms: u64) void {
        s.acked = false;
        s.last_sent_ms = now_ms;
        s.next_heartbeat_ms = now_ms + (s.heartbeat_interval_ms orelse 41_250);
    }

    pub fn onHeartbeatAck(s: *Session, now_ms: u64) void {
        s.acked = true;
        s.last_ack_ms = now_ms;
    }

    /// op 1 from the server: heartbeat immediately.
    pub fn onHeartbeatRequest(_: *Session) Action {
        return .send_heartbeat_now;
    }

    pub fn onDispatchSeq(s: *Session, seq: ?u64) void {
        if (seq) |v| s.seq = v;
    }

    pub fn onReady(s: *Session, session_id: []const u8, resume_url: []const u8) Allocator.Error!void {
        const id = try s.gpa.dupe(u8, session_id);
        errdefer s.gpa.free(id);
        const url = try s.gpa.dupe(u8, resume_url);
        s.forgetSession();
        s.session_id = id;
        s.resume_url = url;
        s.stage = .connected;
    }

    pub fn onResumed(s: *Session) void {
        s.stage = .connected;
    }

    /// op 9.
    pub fn onInvalidSession(s: *Session, resumable: bool) Action {
        if (!resumable) {
            s.forgetSession();
            s.seq = null;
            return .{ .reconnect = .reidentify };
        }
        return .{ .reconnect = .resume_session };
    }

    /// op 7.
    pub fn onReconnectRequest(_: *Session) Action {
        return .{ .reconnect = .resume_session };
    }

    /// A transport error (socket reset, TLS failure, malformed frame).
    pub fn onTransportError(s: *Session) Action {
        return .{ .reconnect = s.reconnectionType() };
    }

    /// The server's close frame (`code` null for an empty close body).
    pub fn onClose(s: *Session, code: ?u16) Action {
        if (code) |c| switch (c) {
            4003, 4004, 4010, 4011, 4013, 4014 => return .{ .stop = c },
            4007 => s.seq = null,
            4006, 4009 => s.forgetSession(),
            else => {},
        };
        const can_resume = if (code) |c| c != 4004 and s.session_id != null else true;
        return .{ .reconnect = if (can_resume and s.session_id != null) .resume_session else .reidentify };
    }
};

// ---------------------------------------------------------------------------
// Payloads. Built with std.json.Stringify so tokens and ids are escaped.
// ---------------------------------------------------------------------------

pub fn writeHeartbeat(w: *std.Io.Writer, seq: ?u64) std.Io.Writer.Error!void {
    // std/json/Stringify.zig: Stringify.value(value, options, writer)
    try std.json.Stringify.value(.{ .op = @as(u8, 1), .d = seq }, .{}, w);
}

pub const Presence = struct {
    activity_name: []const u8 = "for questions",
    /// Discord ActivityType 2 = Listening (serenity `ActivityData::listening`).
    activity_type: u8 = 2,
    status: []const u8 = "online",
};

pub fn writeIdentify(w: *std.Io.Writer, token: []const u8, intent_bits: u64, presence: Presence) std.Io.Writer.Error!void {
    try std.json.Stringify.value(.{
        .op = @as(u8, 2),
        .d = .{
            .token = token,
            .intents = intent_bits,
            .properties = .{ .os = "macos", .browser = "abbey-bot-zig", .device = "abbey-bot-zig" },
            .large_threshold = @as(u8, 250),
            .presence = .{
                .since = @as(?u64, null),
                .activities = .{.{ .name = presence.activity_name, .type = presence.activity_type }},
                .status = presence.status,
                .afk = false,
            },
        },
    }, .{}, w);
}

pub fn writeResume(w: *std.Io.Writer, token: []const u8, session_id: []const u8, seq: ?u64) std.Io.Writer.Error!void {
    try std.json.Stringify.value(.{ .op = @as(u8, 6), .d = .{ .token = token, .session_id = session_id, .seq = seq orelse 0 } }, .{}, w);
}

/// The gateway path with API version and JSON encoding, no compression.
pub const gateway_query = "/?v=10&encoding=json";

const testing = std.testing;

test "identify after Hello, heartbeat after interval times jitter, then every interval" {
    var s = Session.init(testing.allocator);
    defer s.deinit();
    s.connected(.reidentify);
    try testing.expectEqual(Action.send_identify, s.onHello(41_250, 1_000, 0.5));
    try testing.expectEqual(@as(?u64, 1_000 + 20_625), s.next_heartbeat_ms);
    try testing.expectEqual(Action.none, s.tick(21_624));
    try testing.expectEqual(Action.send_heartbeat_now, s.tick(21_625));
    s.heartbeatSent(21_625);
    s.onHeartbeatAck(21_700);
    try testing.expectEqual(Action.none, s.tick(21_625 + 41_249));
    try testing.expectEqual(Action.send_heartbeat_now, s.tick(21_625 + 41_250));
}

test "a missed heartbeat ACK reconnects, resuming when a session exists" {
    var s = Session.init(testing.allocator);
    defer s.deinit();
    s.connected(.reidentify);
    _ = s.onHello(1_000, 0, 0.0);
    try s.onReady("abc", "wss://resume.discord.gg");
    s.heartbeatSent(0);
    try testing.expectEqual(Action{ .reconnect = .resume_session }, s.tick(1_000));
}

test "close codes follow the oracle transport's table" {
    var s = Session.init(testing.allocator);
    defer s.deinit();
    for ([_]u16{ 4003, 4004, 4010, 4011, 4013, 4014 }) |c| try testing.expectEqual(Action{ .stop = c }, s.onClose(c));
    try testing.expectEqual(Action{ .reconnect = .reidentify }, s.onClose(4000)); // no session yet
    try s.onReady("abc", "wss://r");
    s.onDispatchSeq(42);
    try testing.expectEqual(Action{ .reconnect = .resume_session }, s.onClose(4000));
    try testing.expectEqual(Action{ .reconnect = .resume_session }, s.onClose(4008));
    try testing.expectEqual(Action{ .reconnect = .resume_session }, s.onClose(null));
    try testing.expectEqual(Action{ .reconnect = .resume_session }, s.onClose(4007));
    try testing.expectEqual(@as(?u64, null), s.seq);
    try testing.expectEqual(Action{ .reconnect = .reidentify }, s.onClose(4009));
    try testing.expect(s.session_id == null);
}

test "resume after reconnect sends Resume on Hello; invalid session decides the path" {
    var s = Session.init(testing.allocator);
    defer s.deinit();
    s.connected(.reidentify);
    _ = s.onHello(1_000, 0, 0.1);
    try s.onReady("sess", "wss://r");
    s.onDispatchSeq(7);
    try testing.expectEqual(Action{ .reconnect = .resume_session }, s.onReconnectRequest());
    s.connected(.resume_session);
    try testing.expectEqual(Stage.resuming, s.stage);
    try testing.expectEqual(Action.send_resume, s.onHello(1_000, 5, 0.1));
    try testing.expectEqual(Action{ .reconnect = .resume_session }, s.onInvalidSession(true));
    try testing.expectEqual(Action{ .reconnect = .reidentify }, s.onInvalidSession(false));
    try testing.expect(s.session_id == null and s.seq == null);
}

test "payloads are the documented JSON shapes" {
    var buf: [1024]u8 = undefined;
    var w: std.Io.Writer = .fixed(&buf);
    try writeHeartbeat(&w, null);
    try testing.expectEqualStrings("{\"op\":1,\"d\":null}", w.buffered());
    w = .fixed(&buf);
    try writeHeartbeat(&w, 42);
    try testing.expectEqualStrings("{\"op\":1,\"d\":42}", w.buffered());
    w = .fixed(&buf);
    try writeResume(&w, "t\"k", "s", 9);
    try testing.expectEqualStrings("{\"op\":6,\"d\":{\"token\":\"t\\\"k\",\"session_id\":\"s\",\"seq\":9}}", w.buffered());
    w = .fixed(&buf);
    try writeIdentify(&w, "tok", intents.forBot(false), .{});
    try testing.expect(std.mem.startsWith(u8, w.buffered(), "{\"op\":2,\"d\":{\"token\":\"tok\",\"intents\":53575421,"));
    try testing.expect(std.mem.indexOf(u8, w.buffered(), "\"activities\":[{\"name\":\"for questions\",\"type\":2}]") != null);
    try testing.expectEqual(@as(u64, 53575421 | (1 << 15)), intents.forBot(true));
}
