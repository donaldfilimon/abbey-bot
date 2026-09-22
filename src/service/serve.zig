//! `abbey-bot-zig serve [--managed-service]`: select the credential, refuse
//! beside the live Rust service, authenticate, run the gateway bot, and (in
//! managed mode) publish v1 readiness every 10 s under
//! `~/.local/share/abbey-bot-zig`. Order is load-bearing: every refusal
//! happens before any network I/O.
const std = @import("std");
const cfg = @import("config.zig");
const readiness = @import("readiness.zig");
const App = @import("../app/app.zig").App;
const bot_mod = @import("../app/bot.zig");
const engine = @import("../engine/engine.zig");
const memory_service = @import("../memory/service.zig");
const episode_config = @import("../episode/config.zig");
const propose = @import("../episode/propose.zig");
const http = @import("../net/http.zig");
const rest_mod = @import("../discord/rest.zig");
const session = @import("../gateway/session.zig");
const Allocator = std.mem.Allocator;

pub const Exit = enum(u8) { ok = 0, config = 1, usage = 2, refused = 3, runtime = 4 };

pub const Mode = enum { foreground, managed };

pub fn parseArgs(args: []const []const u8) ?Mode {
    if (args.len == 0) return .foreground;
    if (args.len == 1 and std.mem.eql(u8, args[0], "--managed-service")) return .managed;
    return null;
}

/// Owner ids from `GET /oauth2/applications/@me`: the team's members when
/// the application belongs to a team, else the owner (poise's rule).
pub fn parseOwners(arena: Allocator, body: []const u8) ![]const u64 {
    const Id = struct { id: []const u8 };
    const Member = struct { user: Id };
    const Team = struct { members: []const Member = &.{} };
    const Info = struct { id: []const u8, owner: ?Id = null, team: ?Team = null };
    const info = try std.json.parseFromSliceLeaky(Info, arena, body, .{ .ignore_unknown_fields = true, .allocate = .alloc_always });
    var out: std.ArrayList(u64) = .empty;
    if (info.team) |t| {
        for (t.members) |m| try out.append(arena, try std.fmt.parseInt(u64, m.user.id, 10));
    } else if (info.owner) |o| try out.append(arena, try std.fmt.parseInt(u64, o.id, 10));
    return out.items;
}

/// Readiness as observed from the bot. `ready` needs the gateway READY and
/// the command registration to have both completed.
pub fn observe(discord: bot_mod.DiscordState, registered: bool, draining: bool, persistence: readiness.Persistence) struct { state: readiness.State, checkpoints: readiness.Checkpoints } {
    const d: readiness.Discord = switch (discord) {
        .connecting => .connecting,
        .ready => .ready,
        .stopped => .stopped,
    };
    const cp: readiness.Checkpoints = .{ .scheduler_running = !draining, .discord_ready = d == .ready, .commands_registered = registered };
    const phase: readiness.Phase = if (draining) .draining else if (cp.complete()) .ready else .starting;
    return .{ .state = .{ .phase = phase, .discord = d, .scheduler = if (draining) .stopped else .running, .last_persistence = persistence }, .checkpoints = cp };
}

var stop_signal: std.atomic.Value(bool) = .init(false);

fn onSignal(_: std.posix.SIG) callconv(.c) void {
    stop_signal.store(true, .release);
}

fn installSignals() void {
    // std/posix.zig: sigaction + sigemptyset; std/c.zig Darwin Sigaction.
    const act: std.posix.Sigaction = .{ .handler = .{ .handler = onSignal }, .mask = std.posix.sigemptyset(), .flags = 0 };
    std.posix.sigaction(.TERM, &act, null);
    std.posix.sigaction(.INT, &act, null);
}

const Publisher = struct {
    dir: readiness.PrivateDir,
    id: readiness.Identity,
    io: std.Io,

    fn nowMs(p: *Publisher) u64 {
        return @intCast(@max(0, std.Io.Clock.real.now(p.io).toMilliseconds()));
    }

    /// The oracle's `ManagedSink::readiness`: once a `ready` document is
    /// published, the `starting` bootstrap document is removed.
    fn publish(p: *Publisher, state: readiness.State, cp: readiness.Checkpoints) !void {
        var buf: [readiness.readiness_max_bytes]u8 = undefined;
        const bytes = try readiness.encodeReadiness(&buf, p.id, state, p.nowMs(), cp);
        try p.dir.publish(readiness.readiness_file, bytes);
        if (state.phase == .ready) p.dir.remove(readiness.bootstrap_file);
    }

    fn bootstrap(p: *Publisher, phase: readiness.BootstrapPhase, code: readiness.BootstrapCode) void {
        var buf: [readiness.bootstrap_max_bytes]u8 = undefined;
        const bytes = readiness.encodeBootstrap(&buf, p.id, phase, code) catch return;
        p.dir.publish(readiness.bootstrap_file, bytes) catch {};
    }
};

pub fn run(gpa: Allocator, io: std.Io, env: *const std.process.Environ.Map, args: []const []const u8, err: *std.Io.Writer) !Exit {
    var arena_state: std.heap.ArenaAllocator = .init(gpa);
    defer arena_state.deinit();
    const arena = arena_state.allocator();
    const mode = parseArgs(args) orelse {
        try err.writeAll("usage: abbey-bot-zig serve [--managed-service]\n");
        return .usage;
    };
    const token = switch (cfg.selectToken(env)) {
        .ok => |t| t,
        .err => |m| {
            try err.print("{s}\n", .{m});
            return .config;
        },
    };
    const config = switch (try cfg.parse(arena, env)) {
        .ok => |c| c,
        .err => |m| {
            try err.print("{s}\n", .{m});
            return .config;
        },
    };
    if (cfg.refusal(cfg.liveRustServiceLoaded(arena, io), config.test_guild_override) == .live_service_loaded) {
        try err.print("{s}\n", .{cfg.refusal_message});
        return .refused;
    }

    var publisher: ?Publisher = null;
    // The oracle's `ManagedSink::remove`: both documents go on exit.
    defer if (publisher) |*p| {
        p.dir.remove(readiness.readiness_file);
        p.dir.remove(readiness.bootstrap_file);
        p.dir.close();
    };
    if (mode == .managed) {
        const home = env.get("HOME") orelse {
            try err.writeAll("--managed-service needs HOME\n");
            return .config;
        };
        const id = readiness.Identity.current(io, gpa) catch {
            try err.writeAll("--managed-service: run identity unavailable\n");
            return .runtime;
        };
        const dir = readiness.PrivateDir.open(io, home) catch {
            try err.writeAll("--managed-service: ~/.local/share/" ++ readiness.component ++ " is unsafe or unavailable\n");
            return .runtime;
        };
        publisher = .{ .dir = dir, .id = id, .io = io };
        publisher.?.bootstrap(.starting, .none);
        publisher.?.publish(.{}, .{}) catch {
            publisher.?.bootstrap(.failed, .readiness_file);
            try err.writeAll("--managed-service: readiness.json could not be published\n");
            return .runtime;
        };
    }

    var client = http.Client.init(gpa, io);
    defer client.deinit();
    var rest = try rest_mod.Rest.init(gpa, io, &client, token.secret, rest_mod.default_api_base);
    defer rest.deinit();

    // Authentication preflight, and the owner list /help and /modcall use.
    var info = rest.call(.GET, "/oauth2/applications/@me", null) catch |e| {
        try err.print("Discord authentication preflight failed ({s}).\n", .{@errorName(e)});
        return .runtime;
    };
    defer info.deinit();
    if (info.status == 401) {
        try err.print("{s}\n", .{token.source.rejectedDiagnostic()});
        return .config;
    }
    if (info.status != 200) {
        try err.print("Discord authentication preflight returned HTTP {d}.\n", .{info.status});
        return .runtime;
    }
    try err.print("{s}\n", .{token.source.acceptedDiagnostic()});
    const owners = parseOwners(arena, info.body) catch &.{};

    var gate_storage: propose.Gate = undefined;
    var gate: ?*propose.Gate = null;
    if (config.episode_gate_config) |path| {
        const text = std.Io.Dir.cwd().readFileAlloc(io, path, arena, .limited(64 * 1024)) catch {
            try err.writeAll(episode_config.env_name ++ ": cannot read the configuration file\n");
            return .config;
        };
        switch (try episode_config.fromJson(arena, text)) {
            .ok => |c| {
                if (episode_config.checkFiles(io, &c)) |name| {
                    try err.print(episode_config.env_name ++ ": {s} is not a regular file\n", .{name});
                    return .config;
                }
                gate_storage = .{ .config = c };
                gate = &gate_storage;
            },
            .err => |m| {
                try err.print("{s}\n", .{m});
                return .config;
            },
        }
    }

    var app: App = .{
        .gpa = gpa,
        .io = io,
        .engine = engine.Engine.init(gpa),
        .memory = try memory_service.Service.init(gpa, io, config.data_dir),
        .http_client = &client,
        .rest = &rest,
        .provider = config.provider,
        .gate = gate,
        .environ = env,
        .application_owner_ids = owners,
    };
    defer app.deinit();
    var bot = bot_mod.Bot.init(&app, &rest, .{
        .token = token.secret,
        .intents = session.intents.forBot(config.message_content),
        .home_guild = config.home_guild,
    });
    // Honest persistence state: nothing has been written yet. Without a data
    // directory the bot is memory-only by construction.
    const persistence: readiness.Persistence = if (config.data_dir == null) .memory_only else .not_attempted;
    var bot_done: std.atomic.Value(bool) = .init(false);
    installSignals();
    const runner = try std.Thread.spawn(.{}, struct {
        fn f(b: *bot_mod.Bot, done: *std.atomic.Value(bool)) void {
            b.run() catch {};
            done.store(true, .release);
        }
    }.f, .{ &bot, &bot_done });

    var last_publish: u64 = 0;
    while (!bot_done.load(.acquire)) {
        if (stop_signal.load(.acquire)) bot.requestStop();
        if (publisher) |*p| {
            const now = p.nowMs();
            if (now -| last_publish >= readiness.refresh_interval_ms / 2) {
                const o = observe(bot.discord_state.load(.acquire), bot.registration_done.load(.acquire), bot.stop.load(.acquire), persistence);
                p.publish(o.state, o.checkpoints) catch {};
                last_publish = now;
            }
        }
        std.Io.sleep(io, .fromMilliseconds(250), .awake) catch {};
    }
    if (publisher) |*p| {
        const o = observe(.stopped, false, true, persistence);
        p.publish(o.state, o.checkpoints) catch {};
    }
    runner.join();
    bot.deinit();
    if (bot.fatal_close) |code| {
        if (code == 4004) {
            try err.print("{s}\n", .{token.source.rejectedDiagnostic()});
            return .config;
        }
        try err.print("gateway closed with fatal code {d}\n", .{code});
        return .runtime;
    }
    return .ok;
}

const testing = std.testing;

test "serve arguments: only the sole --managed-service flag is accepted" {
    try testing.expectEqual(Mode.foreground, parseArgs(&.{}).?);
    try testing.expectEqual(Mode.managed, parseArgs(&.{"--managed-service"}).?);
    try testing.expect(parseArgs(&.{ "--managed-service", "x" }) == null);
    try testing.expect(parseArgs(&.{"--other"}) == null);
}

test "application owners come from the team when present, else the owner" {
    var arena: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena.deinit();
    const a = arena.allocator();
    try testing.expectEqualSlices(u64, &.{7}, try parseOwners(a, "{\"id\":\"1\",\"owner\":{\"id\":\"7\",\"username\":\"d\"}}"));
    try testing.expectEqualSlices(u64, &.{ 8, 9 }, try parseOwners(a, "{\"id\":\"1\",\"owner\":{\"id\":\"7\"},\"team\":{\"members\":[{\"user\":{\"id\":\"8\"}},{\"user\":{\"id\":\"9\"}}]}}"));
}

test "readiness observation reaches ready only after READY and registration" {
    try testing.expectEqual(readiness.Phase.starting, observe(.connecting, false, false, .memory_only).state.phase);
    try testing.expectEqual(readiness.Phase.starting, observe(.ready, false, false, .memory_only).state.phase);
    const ready = observe(.ready, true, false, .complete);
    try testing.expectEqual(readiness.Phase.ready, ready.state.phase);
    var buf: [readiness.readiness_max_bytes]u8 = undefined;
    const id = try readiness.Identity.fromParts(1, @splat(1), @splat(2));
    _ = try readiness.encodeReadiness(&buf, id, ready.state, 5, ready.checkpoints);
    const draining = observe(.ready, true, true, .complete);
    try testing.expectEqual(readiness.Phase.draining, draining.state.phase);
    try testing.expectEqual(readiness.Scheduler.stopped, draining.state.scheduler);
}

test "serve refuses a blank primary token before touching the network" {
    var env = std.process.Environ.Map.init(testing.allocator);
    defer env.deinit();
    try env.put("DISCORD_TOKEN", " ");
    try env.put("DISCORD_BOT_TOKEN", "would-be-fallback");
    var out: std.Io.Writer.Allocating = .init(testing.allocator);
    defer out.deinit();
    try testing.expectEqual(Exit.config, try run(testing.allocator, testing.io, &env, &.{}, &out.writer));
    try testing.expectEqualStrings("DISCORD_TOKEN is present but blank; refusing to consult DISCORD_BOT_TOKEN.\n", out.written());
}

/// Gate hook: publish one `starting` readiness and bootstrap document with
/// this process's real identity under `<home>/.local/share/abbey-bot-zig`,
/// so the copied Python `service_protocol.read_private` can validate the
/// bytes, modes and path. Never pointed at the real HOME by the gate.
pub fn publishSample(gpa: Allocator, io: std.Io, home: []const u8) !void {
    var p: Publisher = .{ .dir = try readiness.PrivateDir.open(io, home), .id = try readiness.Identity.current(io, gpa), .io = io };
    defer p.dir.close();
    p.bootstrap(.starting, .none);
    const o = observe(.connecting, false, false, .memory_only);
    try p.publish(o.state, o.checkpoints);
}

test "the launchd plist runs this binary with the sole --managed-service argument under its own label" {
    const plist = @embedFile("deploy_plist");
    try testing.expect(std.mem.indexOf(u8, plist, "<string>" ++ cfg.this_label ++ "</string>") != null);
    try testing.expect(std.mem.indexOf(u8, plist, "<string>" ++ cfg.live_rust_label ++ "</string>") == null);
    try testing.expect(std.mem.indexOf(u8, plist, "<string>__HOME__/.local/libexec/" ++ readiness.component ++ "/" ++ readiness.component ++ "</string>\n\t\t<string>--managed-service</string>\n\t</array>") != null);
    try testing.expect(std.mem.indexOf(u8, plist, "<string>__HOME__/.local/share/" ++ readiness.component ++ "</string>") != null);
    try testing.expectEqual(Mode.managed, parseArgs(&.{"--managed-service"}).?);
}

test "a ready publication removes the starting bootstrap document" {
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    const home = try tmp.dir.realPathFileAlloc(testing.io, ".", testing.allocator);
    defer testing.allocator.free(home);
    var p: Publisher = .{ .dir = try readiness.PrivateDir.open(testing.io, home), .id = try readiness.Identity.fromParts(9, @splat(3), @splat(4)), .io = testing.io };
    defer p.dir.close();
    p.bootstrap(.starting, .none);
    const starting = observe(.ready, false, false, .not_attempted);
    try p.publish(starting.state, starting.checkpoints);
    _ = try p.dir.dir.statFile(testing.io, readiness.bootstrap_file, .{});
    const ready = observe(.ready, true, false, .not_attempted);
    try p.publish(ready.state, ready.checkpoints);
    try testing.expectError(error.FileNotFound, p.dir.dir.statFile(testing.io, readiness.bootstrap_file, .{}));
    var buf: [readiness.readiness_max_bytes]u8 = undefined;
    const doc = try p.dir.dir.readFile(testing.io, readiness.readiness_file, &buf);
    try testing.expect(std.mem.indexOf(u8, doc, "\"phase\":\"ready\"") != null);
    try testing.expect(std.mem.indexOf(u8, doc, "\"last_persistence\":\"not_attempted\"") != null);
}
