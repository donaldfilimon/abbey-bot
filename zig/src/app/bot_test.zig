//! Offline end-to-end run of the gateway runtime: a fake Discord gateway
//! (std.http.Server's own WebSocket server, an independent implementation of
//! RFC 6455 that also rejects unmasked client frames), a scripted fake REST
//! API and a fake local model. The bot Identifies, heartbeats, registers the
//! phase-1 commands, and answers `/help` and one persona reply.
const std = @import("std");
const App = @import("app.zig").App;
const bot_mod = @import("bot.zig");
const engine = @import("../engine/engine.zig");
const memory_service = @import("../memory/service.zig");
const http = @import("../net/http.zig");
const llm = @import("../llm/openai.zig");
const rest_mod = @import("../discord/rest.zig");
const session = @import("../gateway/session.zig");
const loopback = @import("../testing/loopback.zig");
const testing = std.testing;

const FakeGateway = struct {
    gpa: std.mem.Allocator,
    io: std.Io,
    listener: std.Io.net.Server,
    port: u16,
    dispatches: []const []const u8,
    thread: ?std.Thread = null,
    identify: std.ArrayList(u8) = .empty,
    heartbeats: std.atomic.Value(u32) = .init(0),
    failure: ?anyerror = null,

    fn start(g: *FakeGateway, gpa: std.mem.Allocator, io: std.Io, dispatches: []const []const u8) !void {
        const address: std.Io.net.IpAddress = .{ .ip4 = .loopback(0) };
        g.* = .{ .gpa = gpa, .io = io, .listener = try address.listen(io, .{ .reuse_address = true }), .port = 0, .dispatches = dispatches };
        g.port = g.listener.socket.address.getPort();
        g.thread = try std.Thread.spawn(.{}, serve, .{g});
    }

    fn deinit(g: *FakeGateway) void {
        g.listener.deinit(g.io);
        if (g.thread) |t| t.join();
        g.identify.deinit(g.gpa);
    }

    fn serve(g: *FakeGateway) void {
        g.serveInner() catch |e| {
            if (e != error.ConnectionClose and e != error.EndOfStream and e != error.ReadFailed) g.failure = e;
        };
    }

    fn serveInner(g: *FakeGateway) !void {
        const stream = try g.listener.accept(g.io);
        defer stream.close(g.io);
        var in_buf: [64 * 1024]u8 = undefined;
        var out_buf: [64 * 1024]u8 = undefined;
        var reader = stream.reader(g.io, &in_buf);
        var writer = stream.writer(g.io, &out_buf);
        var server = std.http.Server.init(&reader.interface, &writer.interface);
        var request = try server.receiveHead();
        if (!std.mem.eql(u8, request.head.target, session.gateway_query)) return error.WrongGatewayQuery;
        const key = switch (request.upgradeRequested()) {
            .websocket => |k| k orelse return error.NoKey,
            else => return error.NotWebSocket,
        };
        var ws = try request.respondWebSocket(.{ .key = key });
        try ws.flush();
        try ws.writeMessage("{\"op\":10,\"d\":{\"heartbeat_interval\":250}}", .text);
        var seq: u64 = 0;
        while (true) {
            const msg = try ws.readSmallMessage();
            const v = try std.json.parseFromSlice(std.json.Value, g.gpa, msg.data, .{});
            defer v.deinit();
            const op = v.value.object.get("op").?.integer;
            if (op == 2) {
                try g.identify.appendSlice(g.gpa, msg.data);
                seq += 1;
                var buf: [512]u8 = undefined;
                const ready = try std.fmt.bufPrint(&buf, "{{\"op\":0,\"s\":{d},\"t\":\"READY\",\"d\":{{\"v\":10,\"session_id\":\"sess-1\",\"resume_gateway_url\":\"ws://127.0.0.1:{d}\",\"application\":{{\"id\":\"2\"}},\"user\":{{\"id\":\"2\",\"username\":\"abbey\"}}}}}}", .{ seq, g.port });
                try ws.writeMessage(ready, .text);
                for (g.dispatches) |d| {
                    seq += 1;
                    var dbuf: [4096]u8 = undefined;
                    try ws.writeMessage(try std.fmt.bufPrint(&dbuf, "{{\"op\":0,\"s\":{d},\"t\":\"INTERACTION_CREATE\",\"d\":{s}}}", .{ seq, d }), .text);
                }
            } else if (op == 1) {
                _ = g.heartbeats.fetchAdd(1, .release);
                try ws.writeMessage("{\"op\":11}", .text);
            }
        }
    }
};

test "offline gateway run: identify, heartbeat, register, answer /help and a persona reply" {
    const gpa = testing.allocator;
    var llm_server: loopback.Server = undefined;
    try llm_server.start(gpa, testing.io, &.{.{ .body = "{\"choices\":[{\"message\":{\"content\":\"Rayleigh scattering.\"},\"finish_reason\":\"stop\"}]}" }});
    defer llm_server.deinit();
    var rest_server: loopback.Server = undefined;
    const ok: loopback.Scripted = .{ .body = "{}" };
    try rest_server.start(gpa, testing.io, &.{ ok, ok, ok, ok, ok });
    defer rest_server.deinit();
    const help_d = "{\"id\":\"901\",\"application_id\":\"2\",\"type\":2,\"token\":\"tok-help\",\"channel_id\":\"77\",\"user\":{\"id\":\"42\",\"username\":\"dana\"},\"data\":{\"id\":\"1\",\"name\":\"help\",\"type\":1}}";
    const ask_d = "{\"id\":\"902\",\"application_id\":\"2\",\"type\":2,\"token\":\"tok-ask\",\"channel_id\":\"77\",\"user\":{\"id\":\"43\",\"username\":\"eli\"},\"data\":{\"id\":\"2\",\"name\":\"persona\",\"type\":1,\"options\":[{\"type\":1,\"name\":\"ask\",\"options\":[{\"type\":3,\"name\":\"question\",\"value\":\"why is the sky blue?\"}]}]}}";
    var gateway: FakeGateway = undefined;
    try gateway.start(gpa, testing.io, &.{ help_d, ask_d });
    defer gateway.deinit();

    var client = http.Client.init(gpa, testing.io);
    defer client.deinit();
    var environ = std.process.Environ.Map.init(gpa);
    defer environ.deinit();
    var arena: std.heap.ArenaAllocator = .init(gpa);
    defer arena.deinit();
    const a = arena.allocator();
    var rest = try rest_mod.Rest.init(gpa, testing.io, &client, "test-token", try std.fmt.allocPrint(a, "http://127.0.0.1:{d}/api/v10", .{rest_server.port}));
    defer rest.deinit();
    var app: App = .{
        .gpa = gpa,
        .io = testing.io,
        .engine = engine.Engine.init(gpa),
        .memory = try memory_service.Service.init(gpa, testing.io, null),
        .http_client = &client,
        .rest = &rest,
        .provider = .{ .primary = .{ .base = try std.fmt.allocPrint(a, "http://127.0.0.1:{d}", .{llm_server.port}), .model = llm.default_local_model, .label = llm.primary_label } },
        .environ = &environ,
    };
    defer app.deinit();
    var bot = bot_mod.Bot.init(&app, &rest, .{
        .token = "test-token",
        .intents = session.intents.forBot(false),
        .gateway_url = try std.fmt.allocPrint(a, "ws://127.0.0.1:{d}", .{gateway.port}),
        .max_connections = 1,
    });
    const runner = try std.Thread.spawn(.{}, struct {
        fn f(b: *bot_mod.Bot) void {
            b.run() catch {};
        }
    }.f, .{&bot});
    rest_server.join();
    var waited: usize = 0;
    while (gateway.heartbeats.load(.acquire) < 2 and waited < 400) : (waited += 1) std.Io.sleep(testing.io, .fromMilliseconds(10), .awake) catch {};
    bot.requestStop();
    runner.join();
    bot.deinit();

    try testing.expect(gateway.failure == null);
    try testing.expect(rest_server.failure == null);
    try testing.expect(gateway.heartbeats.load(.acquire) >= 2);
    try testing.expect(std.mem.indexOf(u8, gateway.identify.items, "\"token\":\"test-token\",\"intents\":53575421") != null);
    try testing.expectEqual(bot_mod.DiscordState.stopped, bot.discord_state.load(.acquire));

    const seen = rest_server.snapshot();
    try testing.expectEqual(@as(usize, 5), seen.len);
    var register_body: ?[]const u8 = null;
    var help_ack: ?[]const u8 = null;
    var ask_ack: ?[]const u8 = null;
    var help_edit: ?[]const u8 = null;
    var ask_edit: ?[]const u8 = null;
    for (seen) |s| {
        try testing.expectEqualStrings("Bot test-token", s.authorization.?);
        if (std.mem.eql(u8, s.target, "/api/v10/applications/2/commands")) register_body = s.body;
        if (std.mem.eql(u8, s.target, "/api/v10/interactions/901/tok-help/callback")) help_ack = s.body;
        if (std.mem.eql(u8, s.target, "/api/v10/interactions/902/tok-ask/callback")) ask_ack = s.body;
        if (std.mem.eql(u8, s.target, "/api/v10/webhooks/2/tok-help/messages/@original")) help_edit = s.body;
        if (std.mem.eql(u8, s.target, "/api/v10/webhooks/2/tok-ask/messages/@original")) ask_edit = s.body;
    }
    try testing.expect(std.mem.indexOf(u8, register_body.?, "\"name\":\"persona\"") != null);
    try testing.expect(std.mem.indexOf(u8, register_body.?, "\"name\":\"voice\"") == null);
    try testing.expectEqualStrings("{\"type\":5,\"data\":{\"flags\":64}}", help_ack.?);
    try testing.expectEqualStrings("{\"type\":5}", ask_ack.?);
    try testing.expect(std.mem.indexOf(u8, help_edit.?, "\"description\":\"**Abbey \u{b7} Start**") != null);
    try testing.expect(std.mem.indexOf(u8, help_edit.?, "\"custom_id\":\"abbey:help:v1:42:") != null);
    try testing.expect(std.mem.indexOf(u8, ask_edit.?, "\"content\":\"**Abbey** \u{2014} answered via configured OpenAI-compatible endpoint:\\n\\nRayleigh scattering.\"") != null);
    for ([_][]const u8{ help_edit.?, ask_edit.? }) |body| try testing.expect(std.mem.indexOf(u8, body, "\"allowed_mentions\":{\"parse\":[],\"replied_user\":false}") != null);
}
