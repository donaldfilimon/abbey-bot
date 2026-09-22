//! Owner-bound private help controls, transcribed from the oracle's
//! `src/help_center.rs`: `abbey:help:v1:<owner>:<expiry>:<section>` custom
//! ids that expire 15 minutes after issue and fail closed on any deviation.
const std = @import("std");
const catalog = @import("catalog.zig");

pub const lifetime_secs: u64 = 15 * 60;
pub const stale = "That control is stale or invalid. Open /help for fresh private controls.";
pub const not_owner = "These private help controls belong to someone else. Open /help for your own.";
pub const expired = "This help session has expired. Open /help for fresh private controls.";

pub const Session = struct {
    owner: u64,
    expiry: u64,
    section: catalog.HelpSection,

    pub fn new(owner: u64, now: u64, section: catalog.HelpSection) ?Session {
        if (owner == 0) return null;
        const expiry = std.math.add(u64, now, lifetime_secs) catch return null;
        return .{ .owner = owner, .expiry = expiry, .section = section };
    }

    pub fn customId(s: Session, buf: []u8) []const u8 {
        return std.fmt.bufPrint(buf, "abbey:help:v1:{d}:{d}:{s}", .{ s.owner, s.expiry, s.section.slug() }) catch unreachable;
    }

    pub fn navigate(s: Session, section: catalog.HelpSection) Session {
        return .{ .owner = s.owner, .expiry = s.expiry, .section = section };
    }
};

pub const Rejection = enum {
    stale,
    not_owner,
    expired,

    pub fn message(r: Rejection) []const u8 {
        return switch (r) {
            .stale => stale,
            .not_owner => not_owner,
            .expired => expired,
        };
    }
};

fn decimal(value: []const u8) ?u64 {
    if (value.len == 0 or (value.len > 1 and value[0] == '0')) return null;
    for (value) |b| if (!std.ascii.isDigit(b)) return null;
    return std.fmt.parseInt(u64, value, 10) catch null;
}

pub fn validate(custom_id: []const u8, actor: u64, now: u64) union(enum) { ok: Session, rejected: Rejection } {
    if (custom_id.len > 100) return .{ .rejected = .stale };
    for (custom_id) |b| if (b >= 0x80) return .{ .rejected = .stale };
    var it = std.mem.splitScalar(u8, custom_id, ':');
    if (!std.mem.eql(u8, it.next() orelse "", "abbey") or !std.mem.eql(u8, it.next() orelse "", "help") or !std.mem.eql(u8, it.next() orelse "", "v1")) return .{ .rejected = .stale };
    const owner = decimal(it.next() orelse "") orelse return .{ .rejected = .stale };
    if (owner == 0) return .{ .rejected = .stale };
    const expiry = decimal(it.next() orelse "") orelse return .{ .rejected = .stale };
    const section = catalog.HelpSection.parse(it.next() orelse "") orelse return .{ .rejected = .stale };
    if (it.next() != null) return .{ .rejected = .stale };
    if (owner != actor) return .{ .rejected = .not_owner };
    if (now >= expiry) return .{ .rejected = .expired };
    if (expiry - now > lifetime_secs) return .{ .rejected = .stale };
    return .{ .ok = .{ .owner = owner, .expiry = expiry, .section = section } };
}

const testing = std.testing;

test "help controls: strict protocol and fixed expiry" {
    const max = std.math.maxInt(u64);
    const s = Session.new(max, 1000, .start).?;
    try testing.expectEqual(@as(u64, 1900), s.expiry);
    var buf: [128]u8 = undefined;
    for (catalog.HelpSection.all) |section| {
        const next = s.navigate(section);
        const id = next.customId(&buf);
        try testing.expect(id.len <= 100);
        try testing.expectEqual(section, validate(id, max, 1899).ok.section);
        try testing.expectEqual(Rejection.expired, validate(id, max, 1900).rejected);
    }
    try testing.expectEqual(Rejection.not_owner, validate(s.customId(&buf), 2, 1000).rejected);
    try testing.expect(Session.new(0, 1000, .start) == null);
    try testing.expect(Session.new(1, max, .start) == null);
}

test "help controls: malformed, unknown, overlong and future controls fail closed" {
    const cases = [_][]const u8{
        "abbey:help:v2:1:1000:start",     "abbey:admin:v1:1:1000:start",
        "abbey:help:v1:0:1000:start",     "abbey:help:v1:+1:1000:start",
        "abbey:help:v1:01:1000:start",    "abbey:help:v1:1:01000:start",
        "abbey:help:v1:1:1000:Start",     "abbey:help:v1:1:1000:start:extra",
        "abbey:help:v1:1:1000:\u{1f980}", "abbey:help:v1:18446744073709551616:1000:start",
        "abbey:help:v1:1:1001:start",
    };
    for (cases) |id| try testing.expectEqual(Rejection.stale, validate(id, 1, 100).rejected);
    const long: [101]u8 = @splat('a');
    try testing.expectEqual(Rejection.stale, validate(&long, 1, 100).rejected);
}
