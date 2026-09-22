//! Content-free episode writes, transcribed from the oracle's
//! `src/episode_gate.rs` and `episode_gate/edge.rs`: learning-toggle
//! proposals, memory candidates and memory edges, serialized exactly as serde
//! does (field order of the canonical abi-wdbx structs, digests as arrays of
//! 32 integers). Ids are keyed Wyhash digests; no Discord snowflake, message
//! text, channel or timestamp reaches the ledger.
const std = @import("std");
const config = @import("config.zig");
const Allocator = std.mem.Allocator;
const Sha256 = std.crypto.hash.sha2.Sha256;

const principal_seed: u64 = 0x6162_6265_795f_6764; // "abbey_gd"
const operation_seed: u64 = 0x6c65_6172_6e5f_6f70; // "learn_op"
const request_seed: u64 = 0x6c65_6172_6e5f_7271; // "learn_rq"
const memory_operation_seed: u64 = 0x6d65_6d6f_7279_6f70; // "memoryop"
const memory_request_seed: u64 = 0x6d65_6d6f_7279_7271; // "memoryrq"
const edge_operation_seed: u64 = 0x6d65_6d65_6467_6f70; // "memedgop"
const edge_request_seed: u64 = 0x6d65_6d65_6467_7271; // "memedgrq"
const token_cost = 1;

pub const Digest = [32]u8;

pub fn requesterPrincipal(buf: *[23]u8, scoped_guild: []const u8, scoped_user: []const u8) []const u8 {
    var h = std.hash.Wyhash.init(principal_seed);
    h.update(scoped_guild);
    h.update("\x1f");
    h.update(scoped_user);
    return std.fmt.bufPrint(buf, "admin-{x:0>16}", .{h.final()}) catch unreachable;
}

fn keyedId(arena: Allocator, prefix: []const u8, seed: u64, guild_ref: []const u8, now: u64, nonce: u64) Allocator.Error![]const u8 {
    var h = std.hash.Wyhash.init(seed);
    h.update(guild_ref);
    h.update("\x1f");
    var le: [8]u8 = undefined;
    std.mem.writeInt(u64, &le, now, .little);
    h.update(&le);
    std.mem.writeInt(u64, &le, nonce, .little);
    h.update(&le);
    return std.fmt.allocPrint(arena, "{s}-{x:0>16}", .{ prefix, h.final() });
}

pub const ActorKind = enum { human_subject, organization_owner, guild_owner, guild_administrator, guild_manager, service };
pub const Actor = struct { principal_id: []const u8, kind: ActorKind };
pub const MemoryClass = enum { fact, experience, embedding, summary };
pub const Retention = enum { session, operational, durable };
pub const EdgeKind = enum { quarantines, contradicts, resolves };
pub const EdgeReason = enum { source_untrusted, signature_invalid, policy_violation, operator_report, conflicting_observation, superseded_evidence, reviewed_valid, reviewed_invalid };

pub const Event = union(enum) {
    proposal: struct { requested_by: Actor, proposed_by: Actor },
    memory_candidate: struct {
        recorded_by: Actor,
        class: MemoryClass,
        retention: Retention,
        payload_commitment: Digest,
        payload_bytes: u64,
        member_scoped: bool,
        supersedes: ?Digest,
        forgets: ?Digest,
    },
    memory_edge: struct { recorded_by: Actor, kind: EdgeKind, target: Digest, counterpart: ?Digest, reason: EdgeReason },
};

pub const Write = struct {
    request_id: []const u8,
    operation_id: []const u8,
    contract_revision: u64,
    contract_digest: Digest,
    guild_ref: []const u8,
    policy_version: []const u8,
    evidence_level: config.EvidenceLevel,
    event: Event,

    /// Compact JSON, byte-identical to serde's `to_string`.
    pub fn render(wr: *const Write, w: *std.Io.Writer) std.Io.Writer.Error!void {
        try w.writeAll("{\"request_id\":");
        try str(w, wr.request_id);
        try w.writeAll(",\"operation_id\":");
        try str(w, wr.operation_id);
        try w.print(",\"contract_revision\":{d},\"contract_digest\":", .{wr.contract_revision});
        try digest(w, wr.contract_digest);
        try w.writeAll(",\"guild_ref\":");
        try str(w, wr.guild_ref);
        try w.writeAll(",\"consent_epoch\":null,\"source_type\":\"discord_guild\",\"policy_version\":");
        try str(w, wr.policy_version);
        try w.print(",\"evidence_level\":\"{s}\",\"event\":", .{@tagName(wr.evidence_level)});
        switch (wr.event) {
            .proposal => |p| {
                try w.writeAll("{\"kind\":\"proposal\",\"requested_by\":");
                try actor(w, p.requested_by);
                try w.writeAll(",\"proposed_by\":");
                try actor(w, p.proposed_by);
                try w.writeByte('}');
            },
            .memory_candidate => |m| {
                try w.writeAll("{\"kind\":\"memory_candidate\",\"recorded_by\":");
                try actor(w, m.recorded_by);
                try w.print(",\"candidate\":{{\"class\":\"{s}\",\"retention\":\"{s}\",\"payload_commitment\":", .{ @tagName(m.class), @tagName(m.retention) });
                try digest(w, m.payload_commitment);
                try w.print(",\"payload_bytes\":{d},\"dimension\":null,\"embedding_version\":null,\"member_scoped\":{s},\"supersedes\":", .{ m.payload_bytes, if (m.member_scoped) "true" else "false" });
                try optDigest(w, m.supersedes);
                try w.writeAll(",\"forgets\":");
                try optDigest(w, m.forgets);
                try w.writeAll("}}");
            },
            .memory_edge => |e| {
                try w.writeAll("{\"kind\":\"memory_edge\",\"recorded_by\":");
                try actor(w, e.recorded_by);
                try w.print(",\"edge\":{{\"kind\":\"{s}\",\"target\":", .{@tagName(e.kind)});
                try digest(w, e.target);
                try w.writeAll(",\"counterpart\":");
                try optDigest(w, e.counterpart);
                try w.print(",\"reason\":\"{s}\"}}}}", .{@tagName(e.reason)});
            },
        }
        try w.print(",\"token_cost\":{d},\"expected_commitment\":null,\"quiet\":false}}", .{token_cost});
    }
};

fn str(w: *std.Io.Writer, s: []const u8) std.Io.Writer.Error!void {
    try std.json.Stringify.value(s, .{}, w);
}

fn digest(w: *std.Io.Writer, d: Digest) std.Io.Writer.Error!void {
    try w.writeByte('[');
    for (d, 0..) |b, i| {
        if (i > 0) try w.writeByte(',');
        try w.print("{d}", .{b});
    }
    try w.writeByte(']');
}

fn optDigest(w: *std.Io.Writer, d: ?Digest) std.Io.Writer.Error!void {
    if (d) |v| try digest(w, v) else try w.writeAll("null");
}

fn actor(w: *std.Io.Writer, a: Actor) std.Io.Writer.Error!void {
    try w.writeAll("{\"principal_id\":");
    try str(w, a.principal_id);
    try w.print(",\"kind\":\"{s}\"}}", .{@tagName(a.kind)});
}

pub const BuildResult = union(enum) { ok: Write, err: []const u8 };

pub fn learningToggle(arena: Allocator, c: *const config.Config, scoped_guild: []const u8, scoped_user: []const u8, now: u64, nonce: u64) Allocator.Error!BuildResult {
    var ref_buf: [256]u8 = undefined;
    const guild_ref = try arena.dupe(u8, config.guildRefFor(&ref_buf, scoped_guild) orelse return .{ .err = "scoped guild id does not map to a ledger guild reference" });
    var p_buf: [23]u8 = undefined;
    const requester = try arena.dupe(u8, requesterPrincipal(&p_buf, scoped_guild, scoped_user));
    if (std.mem.eql(u8, requester, c.service_principal)) return .{ .err = "requester principal collides with the service principal" };
    return .{ .ok = .{
        .request_id = try keyedId(arena, "req", request_seed, guild_ref, now, nonce),
        .operation_id = try keyedId(arena, "learning-toggle", operation_seed, guild_ref, now, nonce),
        .contract_revision = c.contract_revision,
        .contract_digest = c.contract_digest,
        .guild_ref = guild_ref,
        .policy_version = c.policy_version,
        .evidence_level = c.evidence_level,
        .event = .{ .proposal = .{
            .requested_by = .{ .principal_id = requester, .kind = .guild_administrator },
            .proposed_by = .{ .principal_id = c.service_principal, .kind = .service },
        } },
    } };
}

pub const MemoryCandidateRequest = struct {
    scoped_guild: []const u8,
    class: MemoryClass,
    retention: Retention,
    payload: []const u8,
    member_scoped: bool,
    supersedes: ?Digest = null,
    forgets: ?Digest = null,
    now: u64,
    nonce: u64,
};

pub fn memoryCandidate(arena: Allocator, c: *const config.Config, r: MemoryCandidateRequest) Allocator.Error!BuildResult {
    var ref_buf: [256]u8 = undefined;
    const guild_ref = try arena.dupe(u8, config.guildRefFor(&ref_buf, r.scoped_guild) orelse return .{ .err = "scoped guild id does not map to a ledger guild reference" });
    if (r.class == .embedding) return .{ .err = "the bot never proposes embedding candidates" };
    var commitment: Digest = @splat(0);
    var bytes: u64 = 0;
    if (r.forgets != null) {
        if (r.supersedes != null or r.payload.len != 0) return .{ .err = "a forget candidate carries no payload and no supersedes edge" };
    } else {
        if (r.payload.len == 0) return .{ .err = "a memory candidate needs payload bytes to commit to" };
        Sha256.hash(r.payload, &commitment, .{});
        bytes = r.payload.len;
    }
    const op_prefix = try std.fmt.allocPrint(arena, "memory-{s}", .{@tagName(r.class)});
    return .{ .ok = .{
        .request_id = try keyedId(arena, "req", memory_request_seed, guild_ref, r.now, r.nonce),
        .operation_id = try keyedId(arena, op_prefix, memory_operation_seed, guild_ref, r.now, r.nonce),
        .contract_revision = c.contract_revision,
        .contract_digest = c.contract_digest,
        .guild_ref = guild_ref,
        .policy_version = c.policy_version,
        .evidence_level = c.evidence_level,
        .event = .{ .memory_candidate = .{
            .recorded_by = .{ .principal_id = c.service_principal, .kind = .service },
            .class = r.class,
            .retention = r.retention,
            .payload_commitment = commitment,
            .payload_bytes = bytes,
            .member_scoped = r.member_scoped,
            .supersedes = r.supersedes,
            .forgets = r.forgets,
        } },
    } };
}

pub const Reviewer = enum { owner, administrator, manager };

pub const EdgeRequest = union(enum) {
    quarantine: struct { scoped_guild: []const u8, target: Digest, reason: EdgeReason, now: u64, nonce: u64 },
    contradict: struct { scoped_guild: []const u8, target: Digest, counterpart: Digest, now: u64, nonce: u64 },
    resolve: struct { scoped_guild: []const u8, scoped_user: []const u8, reviewer: Reviewer, edge: Digest, valid: bool, now: u64, nonce: u64 },
};

pub fn memoryEdge(arena: Allocator, c: *const config.Config, request: EdgeRequest) Allocator.Error!BuildResult {
    const zero: Digest = @splat(0);
    var scoped_guild: []const u8 = undefined;
    var recorded_by: Actor = .{ .principal_id = c.service_principal, .kind = .service };
    var kind: EdgeKind = undefined;
    var target: Digest = undefined;
    var counterpart: ?Digest = null;
    var reason: EdgeReason = undefined;
    var now: u64 = undefined;
    var nonce: u64 = undefined;
    switch (request) {
        .quarantine => |q| {
            switch (q.reason) {
                .source_untrusted, .signature_invalid, .policy_violation, .operator_report, .superseded_evidence => {},
                else => return .{ .err = "that reason does not describe a quarantine" },
            }
            scoped_guild = q.scoped_guild;
            kind = .quarantines;
            target = q.target;
            reason = q.reason;
            now = q.now;
            nonce = q.nonce;
        },
        .contradict => |x| {
            if (std.mem.eql(u8, &x.target, &x.counterpart)) return .{ .err = "a fact cannot contradict itself" };
            if (std.mem.eql(u8, &x.counterpart, &zero)) return .{ .err = "an edge must name a nonzero digest" };
            const lt = std.mem.order(u8, &x.target, &x.counterpart) == .lt;
            scoped_guild = x.scoped_guild;
            kind = .contradicts;
            target = if (lt) x.target else x.counterpart;
            counterpart = if (lt) x.counterpart else x.target;
            reason = .conflicting_observation;
            now = x.now;
            nonce = x.nonce;
        },
        .resolve => |r| {
            var p_buf: [23]u8 = undefined;
            const principal = try arena.dupe(u8, requesterPrincipal(&p_buf, r.scoped_guild, r.scoped_user));
            if (std.mem.eql(u8, principal, c.service_principal)) return .{ .err = "reviewer principal collides with the service principal" };
            recorded_by = .{ .principal_id = principal, .kind = switch (r.reviewer) {
                .owner => .guild_owner,
                .administrator => .guild_administrator,
                .manager => .guild_manager,
            } };
            scoped_guild = r.scoped_guild;
            kind = .resolves;
            target = r.edge;
            reason = if (r.valid) .reviewed_valid else .reviewed_invalid;
            now = r.now;
            nonce = r.nonce;
        },
    }
    if (std.mem.eql(u8, &target, &zero)) return .{ .err = "an edge must name a nonzero digest" };
    var ref_buf: [256]u8 = undefined;
    const guild_ref = try arena.dupe(u8, config.guildRefFor(&ref_buf, scoped_guild) orelse return .{ .err = "scoped guild id does not map to a ledger guild reference" });
    const op_prefix = try std.fmt.allocPrint(arena, "memory-edge-{s}", .{@tagName(kind)});
    return .{ .ok = .{
        .request_id = try keyedId(arena, "req", edge_request_seed, guild_ref, now, nonce),
        .operation_id = try keyedId(arena, op_prefix, edge_operation_seed, guild_ref, now, nonce),
        .contract_revision = c.contract_revision,
        .contract_digest = c.contract_digest,
        .guild_ref = guild_ref,
        .policy_version = c.policy_version,
        .evidence_level = c.evidence_level,
        .event = .{ .memory_edge = .{ .recorded_by = recorded_by, .kind = kind, .target = target, .counterpart = counterpart, .reason = reason } },
    } };
}
