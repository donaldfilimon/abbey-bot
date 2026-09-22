//! Process-wide bot state shared by the gateway loop and interaction
//! workers. One `std.Io.Mutex` guards the engine, memory service and
//! cooldowns; it is never held across network I/O.
const std = @import("std");
const engine_mod = @import("../engine/engine.zig");
const memory_service = @import("../memory/service.zig");
const memory_gate = @import("../memory/gate.zig");
const llm = @import("../llm/openai.zig");
const http = @import("../net/http.zig");
const rest_mod = @import("../discord/rest.zig");
const propose = @import("../episode/propose.zig");
const Allocator = std.mem.Allocator;

pub const ask_cooldown_seconds: u64 = 30;
pub const ask_cooldown_reply = "You can ask again 30 seconds after your last accepted question.";

pub const App = struct {
    gpa: Allocator,
    io: std.Io,
    mutex: std.Io.Mutex = .init,
    engine: engine_mod.Engine,
    memory: memory_service.Service,
    http_client: *http.Client,
    rest: ?*rest_mod.Rest,
    provider: llm.Provider,
    gate: ?*propose.Gate = null,
    environ: *const std.process.Environ.Map,
    application_owner_ids: []const u64 = &.{},
    ask_cooldown: std.StringHashMapUnmanaged(u64) = .empty,
    /// Tests pin the clock; production reads the wall clock.
    fixed_now: ?u64 = null,

    pub fn deinit(app: *App) void {
        var it = app.ask_cooldown.keyIterator();
        while (it.next()) |k| app.gpa.free(k.*);
        app.ask_cooldown.deinit(app.gpa);
        app.engine.deinit();
        app.memory.deinit();
    }

    pub fn now(app: *const App) u64 {
        if (app.fixed_now) |t| return t;
        // std/Io.zig: Clock.real.now(io) -> Timestamp; toSeconds
        return @intCast(@max(0, std.Io.Clock.real.now(app.io).toSeconds()));
    }

    pub fn lock(app: *App) void {
        app.mutex.lockUncancelable(app.io);
    }

    pub fn unlock(app: *App) void {
        app.mutex.unlock(app.io);
    }

    pub fn gateEnv(app: *App) ?memory_gate.Env {
        const g = app.gate orelse return null;
        return .{ .gate = g, .io = app.io, .environ = app.environ };
    }

    /// Reserve one accepted question per scoped user per 30 seconds (the
    /// oracle's `try_reserve`). Caller holds the lock.
    pub fn reserveAsk(app: *App, scoped_user: []const u8, at: u64) Allocator.Error!bool {
        if (app.ask_cooldown.get(scoped_user)) |last| {
            if (at < last + ask_cooldown_seconds) return false;
        }
        const gop = try app.ask_cooldown.getOrPut(app.gpa, scoped_user);
        if (!gop.found_existing) {
            gop.key_ptr.* = app.gpa.dupe(u8, scoped_user) catch |e| {
                _ = app.ask_cooldown.remove(scoped_user);
                return e;
            };
        }
        gop.value_ptr.* = at;
        return true;
    }

    pub fn isOwner(app: *const App, user_id: u64) bool {
        for (app.application_owner_ids) |id| if (id == user_id) return true;
        return false;
    }
};
