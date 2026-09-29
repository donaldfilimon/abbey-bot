//! `/modcall` recommendation ladder, transcribed from the oracle's
//! `src/moderation.rs`. It recommends and never acts. Severity outranks
//! history; more history never yields a lighter action; every timeout is
//! clamped at Discord's 28-day ceiling.
const std = @import("std");
const Allocator = std.mem.Allocator;

pub const Severity = enum { minor, serious, severe };
pub const History = struct { warnings: u8 = 0, timeouts: u8 = 0 };
pub const max_timeout_minutes: u32 = 28 * 24 * 60;

pub const Action = union(enum) {
    note,
    warn,
    timeout: u32,
    kick,
    ban,

    pub fn requiredPermission(a: Action) ?[]const u8 {
        return switch (a) {
            .note, .warn => null,
            .timeout => "Moderate Members",
            .kick => "Kick Members",
            .ban => "Ban Members",
        };
    }

    /// Discord permission bit behind `requiredPermission`.
    pub fn requiredBit(a: Action) ?u64 {
        return switch (a) {
            .note, .warn => null,
            .timeout => 1 << 40,
            .kick => 1 << 1,
            .ban => 1 << 2,
        };
    }

    fn rank(a: Action) u8 {
        return switch (a) {
            .note => 0,
            .warn => 1,
            .timeout => 2,
            .kick => 3,
            .ban => 4,
        };
    }

    pub fn format(a: Action, w: *std.Io.Writer) std.Io.Writer.Error!void {
        switch (a) {
            .note => try w.writeAll("Note"),
            .warn => try w.writeAll("Warn"),
            .kick => try w.writeAll("Kick"),
            .ban => try w.writeAll("Ban"),
            .timeout => |m| {
                try w.writeAll("Timeout ");
                if (m % (24 * 60) == 0 and m >= 24 * 60) {
                    const days = m / (24 * 60);
                    try w.print("{d} day{s}", .{ days, if (days == 1) "" else "s" });
                } else if (m % 60 == 0 and m >= 60) {
                    const hours = m / 60;
                    try w.print("{d} hour{s}", .{ hours, if (hours == 1) "" else "s" });
                } else try w.print("{d} minutes", .{m});
            },
        }
    }
};

fn timeout(minutes: u32) Action {
    return .{ .timeout = @min(minutes, max_timeout_minutes) };
}

pub const Recommendation = struct { action: Action, reason: []const u8 };

pub fn recommend(arena: Allocator, severity: Severity, h: History) Allocator.Error!Recommendation {
    return switch (severity) {
        .severe => .{ .action = .ban, .reason = "severe incident; severity decides this regardless of record" },
        .serious => switch (h.timeouts) {
            0 => if (h.warnings == 0)
                .{ .action = timeout(60), .reason = "first serious incident" }
            else
                .{ .action = timeout(60), .reason = try std.fmt.allocPrint(arena, "first serious incident, though {d} prior warning(s) precede it", .{h.warnings}) },
            1 => .{ .action = timeout(24 * 60), .reason = "second serious incident after a prior timeout" },
            2 => .{ .action = timeout(7 * 24 * 60), .reason = "third serious incident; last step before removal" },
            else => |n| .{ .action = .ban, .reason = try std.fmt.allocPrint(arena, "{d} prior timeouts have not changed the behaviour", .{n}) },
        },
        .minor => switch (h.warnings) {
            0 => if (h.timeouts == 0)
                .{ .action = .note, .reason = "first minor incident; nothing on record yet" }
            else
                .{ .action = .warn, .reason = try std.fmt.allocPrint(arena, "minor, but {d} prior timeout(s) are on record", .{h.timeouts}) },
            1 => .{ .action = .warn, .reason = "second minor incident; put it on record" },
            2 => .{ .action = timeout(10), .reason = "warned twice already" },
            3 => .{ .action = timeout(60), .reason = "pattern of minor incidents after a timeout" },
            else => |n| .{ .action = .kick, .reason = try std.fmt.allocPrint(arena, "{d} warnings without change; rejoinable if they want to reset", .{n}) },
        },
    };
}

pub fn hierarchyBlocker(actor_is_owner: bool, actor_top: u16, target_is_owner: bool, target_is_admin: bool, target_top: u16, is_timeout: bool) ?[]const u8 {
    if (target_is_owner) return "The server owner cannot be kicked, banned, or timed out.";
    if (actor_is_owner) return null;
    if (is_timeout and target_is_admin) return "Administrators cannot be timed out \u{2014} Discord refuses it outright.";
    if (actor_top <= target_top) return "Their top role is at or above yours, so Discord will refuse this \u{2014} hand it to someone who outranks them.";
    return null;
}

pub fn render(arena: Allocator, subject: []const u8, rec: Recommendation, blocked: ?[]const u8) Allocator.Error![]u8 {
    var out: std.Io.Writer.Allocating = .init(arena);
    out.writer.print("**{s}** \u{2014} {f}. {s}", .{ subject, rec.action, rec.reason }) catch return error.OutOfMemory;
    if (blocked) |reason| out.writer.print("\n\n\u{26a0}\u{fe0f} {s}", .{reason}) catch return error.OutOfMemory;
    return out.toOwnedSlice();
}

const testing = std.testing;

test "severity outranks history completely and a first minor incident is only noted" {
    var arena: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena.deinit();
    const a = arena.allocator();
    try testing.expectEqual(Action.ban, (try recommend(a, .severe, .{})).action);
    try testing.expectEqual(Action.ban, (try recommend(a, .severe, .{ .warnings = 9, .timeouts = 9 })).action);
    try testing.expectEqual(Action.note, (try recommend(a, .minor, .{})).action);
    const minor = try recommend(a, .minor, .{ .timeouts = 5 });
    try testing.expect(minor.action != .note);
    try testing.expect(std.mem.indexOf(u8, (try recommend(a, .serious, .{ .warnings = 7 })).reason, "prior warning") != null);
}

test "serious incidents escalate through timeouts then ban, and more history is never lighter" {
    var arena: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena.deinit();
    const a = arena.allocator();
    try testing.expectEqual(Action{ .timeout = 60 }, (try recommend(a, .serious, .{})).action);
    try testing.expectEqual(Action{ .timeout = 24 * 60 }, (try recommend(a, .serious, .{ .timeouts = 1 })).action);
    try testing.expectEqual(Action{ .timeout = 7 * 24 * 60 }, (try recommend(a, .serious, .{ .timeouts = 2 })).action);
    try testing.expectEqual(Action.ban, (try recommend(a, .serious, .{ .timeouts = 3 })).action);
    for ([_]Severity{ .minor, .serious, .severe }) |s| {
        var w: u8 = 0;
        while (w < 12) : (w += 1) {
            var t: u8 = 0;
            while (t < 12) : (t += 1) {
                const base = (try recommend(a, s, .{ .warnings = w, .timeouts = t })).action;
                const more_w = (try recommend(a, s, .{ .warnings = w + 1, .timeouts = t })).action;
                const more_t = (try recommend(a, s, .{ .warnings = w, .timeouts = t + 1 })).action;
                try testing.expect(more_w.rank() >= base.rank());
                try testing.expect(more_t.rank() >= base.rank());
                if (base == .timeout) try testing.expect(base.timeout <= max_timeout_minutes);
            }
        }
    }
}

test "hierarchy refusals and rendering match the oracle's copy" {
    try testing.expectEqualStrings("The server owner cannot be kicked, banned, or timed out.", hierarchyBlocker(false, 9, true, false, 1, false).?);
    try testing.expect(hierarchyBlocker(true, 0, false, true, 9, true) == null);
    try testing.expectEqualStrings("Administrators cannot be timed out \u{2014} Discord refuses it outright.", hierarchyBlocker(false, 9, false, true, 1, true).?);
    try testing.expect(hierarchyBlocker(false, 5, false, false, 5, false) != null);
    try testing.expect(hierarchyBlocker(false, 6, false, false, 5, false) == null);
    var arena: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena.deinit();
    const a = arena.allocator();
    const text = try render(a, "dana", try recommend(a, .serious, .{ .timeouts = 1 }), null);
    try testing.expectEqualStrings("**dana** \u{2014} Timeout 1 day. second serious incident after a prior timeout", text);
    const minutes = try render(a, "x", .{ .action = .{ .timeout = 10 }, .reason = "r" }, "blocked");
    try testing.expectEqualStrings("**x** \u{2014} Timeout 10 minutes. r\n\n\u{26a0}\u{fe0f} blocked", minutes);
    const hours = try render(a, "x", .{ .action = .{ .timeout = 120 }, .reason = "r" }, null);
    try testing.expectEqualStrings("**x** \u{2014} Timeout 2 hours. r", hours);
}
