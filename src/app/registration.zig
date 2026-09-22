//! Slash-command registration: a projection of the frozen payload onto the
//! top-level commands that have a phase-1 handler for every leaf. Registering
//! a command with no handler would be a stub; the frozen payload itself stays
//! byte-identical to the oracle (catalog parity tests).
const std = @import("std");
const serialize = @import("../catalog/serialize.zig");
const payload_types = @import("../catalog/payload_types.zig");
const handlers = @import("handlers.zig");
const rest_mod = @import("../discord/rest.zig");
const Allocator = std.mem.Allocator;

pub fn isHandled(name: []const u8) bool {
    for (handlers.handled) |h| if (std.mem.eql(u8, h, name)) return true;
    return false;
}

pub fn registered(arena: Allocator) Allocator.Error![]const payload_types.Command {
    var out: std.ArrayList(payload_types.Command) = .empty;
    for (serialize.frozen()) |c| if (isHandled(c.name)) try out.append(arena, c);
    return out.items;
}

/// Compact JSON body for the bulk overwrite.
pub fn body(arena: Allocator) ![]const u8 {
    var out: std.Io.Writer.Allocating = .init(arena);
    try serialize.writeCompact(&out.writer, try registered(arena));
    return out.written();
}

/// Global registration, then the optional home guild (oracle order); the
/// first failure is returned.
pub fn register(rest: *rest_mod.Rest, arena: Allocator, application_id: u64, home_guild: ?u64) !void {
    const payload = try body(arena);
    var global = try rest.call(.PUT, try std.fmt.allocPrint(arena, "/applications/{d}/commands", .{application_id}), payload);
    defer global.deinit();
    if (global.status < 200 or global.status >= 300) return error.RegistrationRefused;
    if (home_guild) |g| {
        var home = try rest.call(.PUT, try std.fmt.allocPrint(arena, "/applications/{d}/guilds/{d}/commands", .{ application_id, g }), payload);
        defer home.deinit();
        if (home.status < 200 or home.status >= 300) return error.RegistrationRefused;
    }
}

const testing = std.testing;

test "registration is a subset of the frozen surface and the rest is claimed Proposed" {
    var arena: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena.deinit();
    const a = arena.allocator();
    const reg = try registered(a);
    try testing.expectEqual(handlers.handled.len, reg.len);
    // Every registered entry is byte-for-byte a frozen entry.
    for (reg) |c| {
        var found = false;
        for (serialize.frozen()) |f| if (std.mem.eql(u8, f.name, c.name)) {
            found = true;
        };
        try testing.expect(found);
    }
    // Every excluded command is named by a claims row that is not Current.
    const claims = try std.json.parseFromSliceLeaky(std.json.Value, a, @embedFile("golden_claims"), .{});
    const rows = claims.object.get("rows").?.array.items;
    var excluded: usize = 0;
    for (serialize.frozen()) |f| {
        if (isHandled(f.name)) continue;
        excluded += 1;
        const needle = if (f.kind == null) try std.fmt.allocPrint(a, "/{s}", .{f.name}) else f.name;
        var covered = false;
        for (rows) |row| {
            const cap = row.object.get("capability").?.string;
            const status = row.object.get("status").?.string;
            if (std.mem.indexOf(u8, cap, needle) != null and !std.mem.eql(u8, status, "Current")) covered = true;
        }
        if (!covered) {
            std.debug.print("excluded command {s} has no non-Current claims row\n", .{needle});
            return error.TestUnexpectedResult;
        }
    }
    try testing.expectEqual(@as(usize, 26 - handlers.handled.len), excluded);
    const json = try body(a);
    try testing.expect(std.mem.startsWith(u8, json, "[{\"name\":\"help\","));
    try testing.expect(std.mem.indexOf(u8, json, "\"name\":\"voice\"") == null);
}
