//! Handler tests over real interaction payloads: an in-memory App, a
//! loopback LLM endpoint and a loopback Discord REST server where needed.
const std = @import("std");
const App = @import("app.zig").App;
const handlers = @import("handlers.zig");
const interaction = @import("../discord/interaction.zig");
const engine = @import("../engine/engine.zig");
const memory_service = @import("../memory/service.zig");
const http = @import("../net/http.zig");
const llm = @import("../llm/openai.zig");
const rest_mod = @import("../discord/rest.zig");
const loopback = @import("../testing/loopback.zig");
const prompts = @import("../persona/prompts.zig");
const testing = std.testing;

const Fixture = struct {
    app: App,
    client: http.Client,
    environ: std.process.Environ.Map,
    arena: std.heap.ArenaAllocator,

    fn init(f: *Fixture) !void {
        f.environ = std.process.Environ.Map.init(testing.allocator);
        f.client = http.Client.init(testing.allocator, testing.io);
        f.arena = .init(testing.allocator);
        f.app = .{
            .gpa = testing.allocator,
            .io = testing.io,
            .engine = engine.Engine.init(testing.allocator),
            .memory = try memory_service.Service.init(testing.allocator, testing.io, null),
            .http_client = &f.client,
            .rest = null,
            .provider = .{},
            .environ = &f.environ,
            .fixed_now = 1_000_000,
        };
    }

    fn deinit(f: *Fixture) void {
        f.app.deinit();
        f.client.deinit();
        f.environ.deinit();
        f.arena.deinit();
    }

    fn run(f: *Fixture, raw: []const u8) ![]const u8 {
        const a = f.arena.allocator();
        const v = try std.json.parseFromSliceLeaky(std.json.Value, a, raw, .{});
        const i = try interaction.parse(a, v);
        const r = try handlers.dispatch(&f.app, a, &i);
        return r.content orelse r.embed orelse "";
    }
};

fn dm(comptime data: []const u8) []const u8 {
    return "{\"id\":\"1\",\"application_id\":\"2\",\"type\":2,\"token\":\"t\",\"channel_id\":\"77\",\"user\":{\"id\":\"42\",\"username\":\"dana\"},\"data\":" ++ data ++ "}";
}

fn guild(comptime perms: []const u8, comptime nsfw: []const u8, comptime data: []const u8) []const u8 {
    return "{\"id\":\"1\",\"application_id\":\"2\",\"type\":2,\"token\":\"t\",\"guild_id\":\"500\",\"channel_id\":\"77\",\"channel\":{\"id\":\"77\",\"nsfw\":" ++ nsfw ++ "},\"member\":{\"user\":{\"id\":\"42\",\"username\":\"dana\"},\"permissions\":\"" ++ perms ++ "\"},\"data\":" ++ data ++ "}";
}

test "handlers: /help renders the section reference with owner-bound controls" {
    var f: Fixture = undefined;
    try f.init();
    defer f.deinit();
    const a = f.arena.allocator();
    const v = try std.json.parseFromSliceLeaky(std.json.Value, a, dm("{\"name\":\"help\",\"type\":1,\"options\":[{\"type\":4,\"name\":\"section\",\"value\":2}]}"), .{});
    const i = try interaction.parse(a, v);
    const r = try handlers.dispatch(&f.app, a, &i);
    try testing.expect(std.mem.startsWith(u8, r.embed.?, "**Abbey \u{b7} Memory**\n"));
    try testing.expect(std.mem.indexOf(u8, r.embed.?, "`/remember`") != null);
    try testing.expectEqualStrings("abbey:help:v1:42:1000900:memory", r.select.?.custom_id);
    try testing.expect(r.select.?.options[2].default);
    // The Start section never promises task buttons this build does not have.
    const start = try f.run(dm("{\"name\":\"help\",\"type\":1}"));
    try testing.expect(std.mem.indexOf(u8, start, "Task buttons") == null);
}

test "handlers: /persona route explains the composed route; /persona ask without a backend is the honesty copy" {
    var f: Fixture = undefined;
    try f.init();
    defer f.deinit();
    const route = try f.run(dm("{\"name\":\"persona\",\"type\":1,\"options\":[{\"type\":1,\"name\":\"route\",\"options\":[{\"type\":3,\"name\":\"request\",\"value\":\"execute deploy run the build quickly\"}]}]}"));
    try testing.expect(std.mem.startsWith(u8, route, "**Aviva** \u{2014} concise direct expert"));
    const ask = try f.run(dm("{\"name\":\"persona\",\"type\":1,\"options\":[{\"type\":1,\"name\":\"ask\",\"options\":[{\"type\":3,\"name\":\"question\",\"value\":\"hello there\"}]}]}"));
    const want = try prompts.degradedReply(f.arena.allocator(), .abbey);
    try testing.expectEqualStrings(want, ask);
    // The 30-second cooldown applies per user.
    const again = try f.run(dm("{\"name\":\"persona\",\"type\":1,\"options\":[{\"type\":1,\"name\":\"ask\",\"options\":[{\"type\":3,\"name\":\"question\",\"value\":\"hi again\"}]}]}"));
    try testing.expectEqualStrings("You can ask again 30 seconds after your last accepted question.", again);
}

test "handlers: /persona ask answers through the local endpoint, tidies, grounds and commits" {
    var server: loopback.Server = undefined;
    try server.start(testing.allocator, testing.io, &.{
        .{ .body = "{\"choices\":[{\"message\":{\"content\":\"Abbey: Rayleigh scattering. It was measured in 2019.\"},\"finish_reason\":\"stop\"}]}" },
    });
    defer server.deinit();
    var f: Fixture = undefined;
    try f.init();
    defer f.deinit();
    const base = try std.fmt.allocPrint(f.arena.allocator(), "http://127.0.0.1:{d}", .{server.port});
    f.app.provider = .{ .primary = .{ .base = base, .model = llm.default_local_model, .label = llm.primary_label } };
    const out = try f.run(dm("{\"name\":\"persona\",\"type\":1,\"options\":[{\"type\":1,\"name\":\"ask\",\"options\":[{\"type\":3,\"name\":\"question\",\"value\":\"why is the sky blue?\"}]}]}"));
    server.join();
    try testing.expect(std.mem.startsWith(u8, out, "**Abbey** \u{2014} answered via configured OpenAI-compatible endpoint:\n\nRayleigh scattering."));
    try testing.expect(std.mem.indexOf(u8, out, "Heads up \u{2014} treat these as unsupported: `2019`") != null);
    try testing.expectEqual(@as(usize, 2), f.app.engine.sessionLen("discord:77"));
    try testing.expect(std.mem.indexOf(u8, server.seen.items[0].body, "Operational capability context") != null);
}

test "handlers: /roleplay refuses in SFW channels, sticks Aviva when empty in an enabled DM" {
    var f: Fixture = undefined;
    try f.init();
    defer f.deinit();
    const sfw = try f.run(guild("0", "false", "{\"name\":\"roleplay\",\"type\":1}"));
    try testing.expect(std.mem.indexOf(u8, sfw, "Abbey stays SFW here") != null);
    const disabled = try f.run(dm("{\"name\":\"roleplay\",\"type\":1}"));
    try testing.expect(std.mem.startsWith(u8, disabled, "Roleplay is disabled here."));
    const on = try f.run(dm("{\"name\":\"nsfw\",\"type\":1,\"options\":[{\"type\":4,\"name\":\"state\",\"value\":0}]}"));
    try testing.expectEqualStrings("nsfw roleplay is now **on** in this DM.", on);
    const stick = try f.run(dm("{\"name\":\"roleplay\",\"type\":1}"));
    try testing.expectEqualStrings("Aviva roleplay is available here. I'll answer as Aviva.", stick);
    try testing.expectEqual(@as(?@import("../persona/persona.zig").Persona, .aviva), f.app.engine.sessionPersona("discord:77"));
    try testing.expectEqual(@as(usize, 0), f.app.engine.sessionLen("discord:77"));
    const in_guild = try f.run(guild("0", "true", "{\"name\":\"nsfw\",\"type\":1,\"options\":[{\"type\":4,\"name\":\"state\",\"value\":0}]}"));
    try testing.expectEqualStrings("In a server, use `/admin nsfw on|off` instead.", in_guild);
}

test "handlers: /remember, /recall and /forget keep memory self-scoped unless moderators act" {
    var f: Fixture = undefined;
    try f.init();
    defer f.deinit();
    const stored = try f.run(guild("0", "false", "{\"name\":\"remember\",\"type\":1,\"options\":[{\"type\":3,\"name\":\"fact\",\"value\":\" likes   rust \"}]}"));
    try testing.expectEqualStrings("Stored about <@42>: likes rust", stored);
    const dup = try f.run(guild("0", "false", "{\"name\":\"remember\",\"type\":1,\"options\":[{\"type\":3,\"name\":\"fact\",\"value\":\"likes rust\"}]}"));
    try testing.expectEqualStrings("Already on record (or the fact list is full).", dup);
    const card = try f.run(guild("0", "false", "{\"name\":\"recall\",\"type\":1}"));
    try testing.expectEqualStrings("**<@42>** \u{2014} standing 0.50 (0 = poor, 1 = excellent)\nFacts:\n\u{2022} likes rust\nPending replacements:\n\u{2022} None.", card);
    const other = try f.run(guild("0", "false", "{\"name\":\"remember\",\"type\":1,\"options\":[{\"type\":3,\"name\":\"fact\",\"value\":\"x\"},{\"type\":6,\"name\":\"user\",\"value\":\"99\"}]}"));
    try testing.expectEqualStrings(handlers.cross_user_memory_denied, other);
    // Manage Messages (1 << 13) may act for another member.
    const moderated = try f.run(guild("8192", "false", "{\"name\":\"remember\",\"type\":1,\"options\":[{\"type\":3,\"name\":\"fact\",\"value\":\"is new here\"},{\"type\":6,\"name\":\"user\",\"value\":\"99\"}]}"));
    try testing.expectEqualStrings("Stored about <@99>: is new here", moderated);
    try testing.expectEqualStrings("Forgotten.", try f.run(guild("0", "false", "{\"name\":\"forget\",\"type\":1,\"options\":[{\"type\":3,\"name\":\"fact\",\"value\":\"likes  rust\"}]}")));
    try testing.expectEqualStrings("Nothing by that wording was on record.", try f.run(guild("0", "false", "{\"name\":\"forget\",\"type\":1,\"options\":[{\"type\":3,\"name\":\"fact\",\"value\":\"likes rust\"}]}")));
    try testing.expectEqualStrings("The fact must contain some text.", try f.run(guild("0", "false", "{\"name\":\"remember\",\"type\":1,\"options\":[{\"type\":3,\"name\":\"fact\",\"value\":\"  \"}]}")));
}

test "handlers: /modcall recommends from live guild data and reports hierarchy blockers" {
    var server: loopback.Server = undefined;
    const guild_json = "{\"id\":\"500\",\"owner_id\":\"1\",\"roles\":[{\"id\":\"500\",\"permissions\":\"0\",\"position\":0},{\"id\":\"600\",\"permissions\":\"1099511627776\",\"position\":2},{\"id\":\"700\",\"permissions\":\"0\",\"position\":5}]}";
    try server.start(testing.allocator, testing.io, &.{
        .{ .body = guild_json },
        .{ .body = "{\"user\":{\"id\":\"42\"},\"roles\":[\"600\"]}" },
        .{ .body = "{\"user\":{\"id\":\"99\"},\"roles\":[\"700\"]}" },
    });
    defer server.deinit();
    var f: Fixture = undefined;
    try f.init();
    defer f.deinit();
    const base = try std.fmt.allocPrint(f.arena.allocator(), "http://127.0.0.1:{d}/api/v10", .{server.port});
    var rest = try rest_mod.Rest.init(testing.allocator, testing.io, &f.client, "tok", base);
    defer rest.deinit();
    f.app.rest = &rest;
    const out = try f.run("{\"id\":\"1\",\"application_id\":\"2\",\"type\":2,\"token\":\"t\",\"guild_id\":\"500\",\"channel_id\":\"77\",\"member\":{\"user\":{\"id\":\"42\",\"username\":\"mod\"},\"permissions\":\"1099511627776\"},\"data\":{\"name\":\"modcall\",\"type\":1,\"options\":[{\"type\":6,\"name\":\"user\",\"value\":\"99\"},{\"type\":4,\"name\":\"severity\",\"value\":1}],\"resolved\":{\"users\":{\"99\":{\"id\":\"99\",\"username\":\"trouble\"}}}}}");
    server.join();
    try testing.expectEqualStrings("**trouble** \u{2014} Timeout 1 hour. first serious incident\n\n\u{26a0}\u{fe0f} Their top role is at or above yours, so Discord will refuse this \u{2014} hand it to someone who outranks them.", out);
    try testing.expectEqualStrings("/api/v10/guilds/500/members/99", server.seen.items[2].target);
}
