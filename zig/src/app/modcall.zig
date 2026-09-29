//! `/modcall`: recommend a moderation action and say whether the invoking
//! moderator can carry it out (oracle `commands.rs` `modcall`). It never
//! acts. Guild-level permissions and top-role positions are computed from
//! `GET /guilds/{id}` and `GET /guilds/{id}/members/{user}` exactly as
//! serenity's `member_permissions` does: @everyone plus every held role,
//! everything for the owner or an Administrator.
const std = @import("std");
const App = @import("app.zig").App;
const Reply = @import("reply.zig").Reply;
const Interaction = @import("../discord/interaction.zig").Interaction;
const catalog = @import("../catalog/catalog.zig");
const moderation = @import("../moderation/moderation.zig");
const handlers = @import("handlers.zig");
const Allocator = std.mem.Allocator;

pub const moderate_members_required = "Discord must currently grant you Moderate Members to use this command.";
const administrator: u64 = 1 << 3;
const all_permissions: u64 = std.math.maxInt(u64);

pub const Role = struct { id: u64, permissions: u64, position: u16 };
pub const Guild = struct { id: u64, owner_id: u64, roles: []const Role };
pub const Member = struct { user_id: u64, roles: []const u64 };

pub fn memberPermissions(g: Guild, m: Member) u64 {
    if (m.user_id == g.owner_id) return all_permissions;
    var bits: u64 = 0;
    for (g.roles) |r| {
        if (r.id == g.id) bits |= r.permissions; // @everyone
        for (m.roles) |held| if (held == r.id) {
            bits |= r.permissions;
        };
    }
    return if (bits & administrator != 0) all_permissions else bits;
}

pub fn topRolePosition(g: Guild, m: Member) u16 {
    var top: u16 = 0;
    for (m.roles) |held| for (g.roles) |r| if (r.id == held and r.position > top) {
        top = r.position;
    };
    return top;
}

fn snowflake(v: ?std.json.Value) ?u64 {
    const x = v orelse return null;
    return switch (x) {
        .string => |s| std.fmt.parseInt(u64, s, 10) catch null,
        .integer => |n| if (n >= 0) @intCast(n) else null,
        else => null,
    };
}

pub fn parseGuild(arena: Allocator, raw: []const u8) !Guild {
    const v = try std.json.parseFromSliceLeaky(std.json.Value, arena, raw, .{ .allocate = .alloc_always });
    if (v != .object) return error.MalformedGuild;
    const roles_v = v.object.get("roles") orelse return error.MalformedGuild;
    if (roles_v != .array) return error.MalformedGuild;
    var roles: std.ArrayList(Role) = .empty;
    for (roles_v.array.items) |r| {
        if (r != .object) return error.MalformedGuild;
        const perm_text = if (r.object.get("permissions")) |p| (if (p == .string) p.string else "0") else "0";
        const pos = if (r.object.get("position")) |p| (if (p == .integer and p.integer >= 0) @as(u16, @intCast(@min(p.integer, 65535))) else 0) else 0;
        try roles.append(arena, .{ .id = snowflake(r.object.get("id")) orelse return error.MalformedGuild, .permissions = std.fmt.parseInt(u64, perm_text, 10) catch 0, .position = pos });
    }
    return .{ .id = snowflake(v.object.get("id")) orelse return error.MalformedGuild, .owner_id = snowflake(v.object.get("owner_id")) orelse return error.MalformedGuild, .roles = roles.items };
}

pub fn parseMember(arena: Allocator, raw: []const u8) !Member {
    const v = try std.json.parseFromSliceLeaky(std.json.Value, arena, raw, .{ .allocate = .alloc_always });
    if (v != .object) return error.MalformedMember;
    const user = v.object.get("user") orelse return error.MalformedMember;
    if (user != .object) return error.MalformedMember;
    var roles: std.ArrayList(u64) = .empty;
    if (v.object.get("roles")) |rs| if (rs == .array) for (rs.array.items) |r| if (snowflake(r)) |id| try roles.append(arena, id);
    return .{ .user_id = snowflake(user.object.get("id")) orelse return error.MalformedMember, .roles = roles.items };
}

fn fetch(app: *App, arena: Allocator, path: []const u8) ![]const u8 {
    const rest = app.rest orelse return error.NoRestClient;
    var response = try rest.call(.GET, path, null);
    defer response.deinit();
    if (response.status != 200) return error.UnexpectedStatus;
    return arena.dupe(u8, response.body);
}

fn severityOf(i: *const Interaction) moderation.Severity {
    const v = i.option("severity") orelse return .minor;
    return switch (v) {
        .integer => |n| switch (n) {
            1 => .serious,
            2 => .severe,
            else => .minor,
        },
        else => .minor,
    };
}

fn countOf(i: *const Interaction, name: []const u8) u8 {
    const v = i.option(name) orelse return 0;
    return switch (v) {
        .integer => |n| @intCast(std.math.clamp(n, 0, 255)),
        else => 0,
    };
}

pub fn modcall(app: *App, arena: Allocator, i: *const Interaction) !Reply {
    const guild_id = i.guild_id orelse return .{ .content = handlers.no_guild };
    const subject_id = if (i.option("user")) |v| (if (v == .snowflake) v.snowflake else 0) else 0;
    const guild = try parseGuild(arena, try fetch(app, arena, try std.fmt.allocPrint(arena, "/guilds/{d}", .{guild_id})));
    const moderator = try parseMember(arena, try fetch(app, arena, try std.fmt.allocPrint(arena, "/guilds/{d}/members/{d}", .{ guild_id, i.user.id })));
    const held = memberPermissions(guild, moderator);
    var access: catalog.EligibilityInput = .{ .context = .guild };
    catalog.DiscordPermission.fromBits(held, &access.permissions);
    const spec = catalog.command(.modcall);
    if (!catalog.accessAllows(spec.access.rule(), &access)) return .{ .content = moderate_members_required };
    const rec = try moderation.recommend(arena, severityOf(i), .{ .warnings = countOf(i, "warnings"), .timeouts = countOf(i, "timeouts") });
    const target = try parseMember(arena, try fetch(app, arena, try std.fmt.allocPrint(arena, "/guilds/{d}/members/{d}", .{ guild_id, subject_id })));
    const blocker: ?[]const u8 = if (rec.action.requiredBit()) |bit|
        (if (held & bit == 0)
            try std.fmt.allocPrint(arena, "You do not have **{s}**, so you cannot carry this out \u{2014} hand it to someone who does.", .{rec.action.requiredPermission().?})
        else
            moderation.hierarchyBlocker(i.user.id == guild.owner_id, topRolePosition(guild, moderator), subject_id == guild.owner_id, memberPermissions(guild, target) & administrator != 0, topRolePosition(guild, target), rec.action == .timeout))
    else
        null;
    var resolved = access;
    resolved.action_target_resolved = true;
    resolved.hierarchy_allows_action = blocker == null;
    const name = if (i.resolvedUser(subject_id)) |u| u.username else "that member";
    if (!catalog.eligible(spec, &resolved, .invocation)) {
        return .{ .content = try catalog.clampMessage(arena, try moderation.render(arena, name, rec, blocker orelse moderate_members_required)) };
    }
    return .{ .content = try catalog.clampMessage(arena, try moderation.render(arena, name, rec, blocker)) };
}

const testing = std.testing;

test "member permissions follow serenity: @everyone plus roles, all for owner or Administrator" {
    const g: Guild = .{ .id = 1, .owner_id = 9, .roles = &.{
        .{ .id = 1, .permissions = 1 << 10, .position = 0 },
        .{ .id = 2, .permissions = 1 << 40, .position = 3 },
        .{ .id = 3, .permissions = administrator, .position = 5 },
    } };
    try testing.expectEqual(@as(u64, (1 << 10) | (1 << 40)), memberPermissions(g, .{ .user_id = 5, .roles = &.{2} }));
    try testing.expectEqual(all_permissions, memberPermissions(g, .{ .user_id = 5, .roles = &.{3} }));
    try testing.expectEqual(all_permissions, memberPermissions(g, .{ .user_id = 9, .roles = &.{} }));
    try testing.expectEqual(@as(u16, 5), topRolePosition(g, .{ .user_id = 5, .roles = &.{ 2, 3 } }));
    try testing.expectEqual(@as(u16, 0), topRolePosition(g, .{ .user_id = 5, .roles = &.{} }));
}
