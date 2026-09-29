//! Memory gate: propose before writing, write only on `appended`, fail
//! closed and visibly (oracle `src/memory_gate.rs`, amendment 2026-09-06).
//! With no gate configured, or a scope the config does not cover, every
//! function is a no-op and memory behaves exactly as ungated.
const std = @import("std");
const service_mod = @import("service.zig");
const episode_config = @import("../episode/config.zig");
const episode_write = @import("../episode/write.zig");
const propose = @import("../episode/propose.zig");
const Allocator = std.mem.Allocator;

pub const Decision = enum {
    stored,
    proposed,
    rejected,
    unknown,
    cancelled,
    unobserved,
    local_refused,

    pub fn message(d: Decision) []const u8 {
        return switch (d) {
            .stored => "Memory: stored locally after the gate appended its receipt.",
            .proposed => "Memory: stored locally after the gate appended its receipt. The proposed replacement still needs your confirmation; the old fact is unchanged.",
            .rejected => "Memory: the gate rejected the proposal. Nothing was stored locally.",
            .unknown => "Memory: admission is unknown because the gate did not return a valid receipt. Nothing was stored locally.",
            .cancelled => "Memory: the request was cancelled before admission. Nothing was stored locally. Try `/remember` when Abbey is available.",
            .unobserved => "Memory: the final local outcome could not be observed. Check `/facts` before trying again.",
            .local_refused => "Memory: the gate admitted the proposal, but the local fact was not added because local state changed. No replacement was applied.",
        };
    }
};

pub const forget_rejected = "Not stored: the constitutional memory gate refused it. Nothing was stored locally.";
pub const forget_unknown = "Not stored locally: admission is unknown because the constitutional memory gate did not return a valid receipt.";

pub const Admission = union(enum) {
    /// No gate for this scope: write freely.
    ungated,
    /// The ledger appended; key the stored fact by this digest.
    receipt: []const u8,
    refused: Decision,
};

pub const Env = struct {
    gate: *propose.Gate,
    io: std.Io,
    environ: *const std.process.Environ.Map,
};

fn gateFor(env: ?Env, scoped_guild: []const u8) ?Env {
    const e = env orelse return null;
    return if (e.gate.config.covers(scoped_guild)) e else null;
}

pub fn admitFact(svc: *service_mod.Service, env_opt: ?Env, arena: Allocator, scoped_guild: []const u8, scoped_user: []const u8, fact: []const u8, replaces: ?[]const u8, now: u64) !Admission {
    const env = gateFor(env_opt, scoped_guild) orelse return .ungated;
    var supersedes: ?episode_write.Digest = null;
    if (replaces) |old_text| {
        if (try svc.resolveFact(arena, scoped_guild, scoped_user, old_text)) |old| {
            if (svc.receipt(scoped_guild, scoped_user, old)) |hex| {
                supersedes = episode_config.parseDigest(hex);
            } else _ = env.gate.ungated_forgets.fetchAdd(1, .monotonic);
        }
    }
    const built = try episode_write.memoryCandidate(arena, &env.gate.config, .{
        .scoped_guild = scoped_guild,
        .class = .fact,
        .retention = .durable,
        .payload = fact,
        .member_scoped = true,
        .supersedes = supersedes,
        .now = now,
        .nonce = env.gate.nextNonce(),
    });
    const w = switch (built) {
        .ok => |w| w,
        .err => return .{ .refused = .unknown },
    };
    return switch (try env.gate.propose(arena, env.io, env.environ, &w)) {
        .appended => |a| .{ .receipt = try arena.dupe(u8, a.digest_hex) },
        .rejected => .{ .refused = .rejected },
        .unavailable => .{ .refused = .unknown },
    };
}

/// `null` means the forget may proceed; otherwise the refusal to show.
pub fn admitForget(svc: *service_mod.Service, env_opt: ?Env, arena: Allocator, scoped_guild: []const u8, scoped_user: []const u8, fact: []const u8, now: u64) !?[]const u8 {
    const env = gateFor(env_opt, scoped_guild) orelse return null;
    const target = if (svc.receipt(scoped_guild, scoped_user, fact)) |hex| episode_config.parseDigest(hex) else null;
    if (target == null) {
        _ = env.gate.ungated_forgets.fetchAdd(1, .monotonic);
        return null;
    }
    const built = try episode_write.memoryCandidate(arena, &env.gate.config, .{
        .scoped_guild = scoped_guild,
        .class = .fact,
        .retention = .durable,
        .payload = "",
        .member_scoped = true,
        .forgets = target,
        .now = now,
        .nonce = env.gate.nextNonce(),
    });
    const w = switch (built) {
        .ok => |w| w,
        .err => return forget_unknown,
    };
    return switch (try env.gate.propose(arena, env.io, env.environ, &w)) {
        .appended => null,
        .rejected => forget_rejected,
        .unavailable => forget_unknown,
    };
}

const testing = std.testing;

fn gateWithAbi(arena: Allocator, abi_path: []const u8, guilds_json: []const u8) !propose.Gate {
    const json = try std.fmt.allocPrint(arena, "{{\"abi_cli\":\"{s}\",\"endpoint\":\"http://127.0.0.1:50051\",\"token_file\":\"/opt/token\",\"policy_version\":\"policy_v1\",\"contract_revision\":2,\"contract_digest\":\"0108151d242b323940474e555c636a71787f868d949ba2a9b0b7bec5ccd3dae1\"{s}}}", .{ abi_path, guilds_json });
    return .{ .config = (try episode_config.fromJson(arena, json)).ok };
}

test "memory gate: an unreachable gate stores nothing and says so; an uncovered scope is ungated" {
    var arena_state: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena_state.deinit();
    const a = arena_state.allocator();
    var svc = try service_mod.Service.init(testing.allocator, testing.io, null);
    defer svc.deinit();
    var gate = try gateWithAbi(a, "/nonexistent/abi", ",\"guilds\":[\"discord:1\"]");
    var environ = std.process.Environ.Map.init(a);
    const env: Env = .{ .gate = &gate, .io = testing.io, .environ = &environ };
    const admission = try admitFact(&svc, env, a, "discord:1", "discord:2", "likes rust", null, 5);
    try testing.expectEqual(Decision.unknown, admission.refused);
    try testing.expectEqualStrings("Memory: admission is unknown because the gate did not return a valid receipt. Nothing was stored locally.", admission.refused.message());
    try testing.expectEqual(@as(usize, 0), svc.bank.facts("discord:1", "discord:2").len);
    try testing.expect((try admitFact(&svc, env, a, "discord:9", "discord:2", "likes rust", null, 5)) == .ungated);
    try testing.expect((try admitFact(&svc, null, a, "discord:1", "discord:2", "likes rust", null, 5)) == .ungated);
}

test "memory gate: an appended candidate yields its receipt, and forget proposes a tombstone for it" {
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    var arena_state: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena_state.deinit();
    const a = arena_state.allocator();
    const script =
        \\#!/bin/sh
        \\/bin/cat "$4" >> "$(dirname "$0")/writes.jsonl"
        \\echo >> "$(dirname "$0")/writes.jsonl"
        \\echo '{"decision":"appended","episode_digest":"cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd","sequence":"1"}'
        \\
    ;
    try tmp.dir.writeFile(testing.io, .{ .sub_path = "abi", .data = script, .flags = .{ .permissions = .fromMode(0o755) } });
    const abi = try std.fs.path.join(a, &.{ try tmp.dir.realPathFileAlloc(testing.io, ".", a), "abi" });
    var gate = try gateWithAbi(a, abi, "");
    var environ = std.process.Environ.Map.init(a);
    const env: Env = .{ .gate = &gate, .io = testing.io, .environ = &environ };
    var svc = try service_mod.Service.init(testing.allocator, testing.io, null);
    defer svc.deinit();
    const admission = try admitFact(&svc, env, a, "discord:1", "discord:2", "likes rust", null, 5);
    const digest = admission.receipt;
    _ = try svc.rememberAdmitted(a, "discord:1", "discord:2", "likes rust", null, 5, digest);
    try testing.expectEqualStrings(digest, svc.receipt("discord:1", "discord:2", "likes rust").?);
    try testing.expect((try admitForget(&svc, env, a, "discord:1", "discord:2", "likes rust", 6)) == null);
    const writes = try tmp.dir.readFileAlloc(testing.io, "writes.jsonl", a, .limited(1 << 16));
    try testing.expect(std.mem.indexOf(u8, writes, "\"payload_bytes\":10") != null);
    try testing.expect(std.mem.indexOf(u8, writes, "\"payload_bytes\":0,\"dimension\":null,\"embedding_version\":null,\"member_scoped\":true,\"supersedes\":null,\"forgets\":[205,") != null);
    try testing.expect(std.mem.indexOf(u8, writes, "likes rust") == null); // content-free
}
