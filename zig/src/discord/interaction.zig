//! Parsed `INTERACTION_CREATE` payloads (application commands and message
//! components). Strings borrow the caller's arena. Only fields phase-1
//! handlers read are extracted; unknown fields are ignored.
const std = @import("std");
const Allocator = std.mem.Allocator;

pub const Kind = enum(u8) { ping = 1, application_command = 2, message_component = 3, autocomplete = 4, modal_submit = 5, _ };

pub const OptionValue = union(enum) {
    string: []const u8,
    integer: i64,
    boolean: bool,
    /// user/channel/role/mentionable ids arrive as snowflake strings.
    snowflake: u64,
    number: f64,
};

pub const Option = struct { name: []const u8, value: OptionValue };

pub const User = struct { id: u64, username: []const u8, bot: bool };

pub const Interaction = struct {
    id: u64,
    application_id: u64,
    kind: Kind,
    token: []const u8,
    guild_id: ?u64,
    channel_id: u64,
    channel_nsfw: bool,
    user: User,
    /// Member permissions in the channel (guild interactions only).
    member_permissions: ?u64,
    /// Space-joined command path, e.g. "persona ask"; empty for components.
    command_path: []const u8,
    command_kind: u8,
    options: []const Option,
    target_id: ?u64,
    custom_id: ?[]const u8,
    values: []const []const u8,
    /// Usernames of resolved users (for /modcall's subject label).
    resolved_users: []const User,

    pub fn option(i: *const Interaction, name: []const u8) ?OptionValue {
        for (i.options) |o| if (std.mem.eql(u8, o.name, name)) return o.value;
        return null;
    }

    pub fn string(i: *const Interaction, name: []const u8) ?[]const u8 {
        const v = i.option(name) orelse return null;
        return if (v == .string) v.string else null;
    }

    pub fn resolvedUser(i: *const Interaction, id: u64) ?User {
        for (i.resolved_users) |u| if (u.id == id) return u;
        return null;
    }
};

pub const ParseError = error{MalformedInteraction} || Allocator.Error;

fn snowflake(v: ?std.json.Value) ?u64 {
    const x = v orelse return null;
    return switch (x) {
        .string => |s| std.fmt.parseInt(u64, s, 10) catch null,
        .integer => |n| if (n >= 0) @intCast(n) else null,
        else => null,
    };
}

fn str(v: ?std.json.Value) ?[]const u8 {
    const x = v orelse return null;
    return if (x == .string) x.string else null;
}

fn obj(v: ?std.json.Value) ?std.json.ObjectMap {
    const x = v orelse return null;
    return if (x == .object) x.object else null;
}

fn parseUser(o: std.json.ObjectMap) ?User {
    return .{
        .id = snowflake(o.get("id")) orelse return null,
        .username = str(o.get("username")) orelse "",
        .bot = if (o.get("bot")) |b| (b == .bool and b.bool) else false,
    };
}

/// Walk nested subcommand options into a path and leaf options.
fn flatten(arena: Allocator, options: ?std.json.Value, path: *std.ArrayList(u8), leaves: *std.ArrayList(Option)) ParseError!void {
    const list = options orelse return;
    if (list != .array) return;
    for (list.array.items) |item| {
        const o = obj(item) orelse return error.MalformedInteraction;
        const name = str(o.get("name")) orelse return error.MalformedInteraction;
        const kind: i64 = if (o.get("type")) |t| (if (t == .integer) t.integer else 0) else 0;
        if (kind == 1 or kind == 2) {
            try path.append(arena, ' ');
            try path.appendSlice(arena, name);
            try flatten(arena, o.get("options"), path, leaves);
            continue;
        }
        const raw = o.get("value") orelse continue;
        const value: OptionValue = switch (kind) {
            6, 7, 8, 9 => .{ .snowflake = snowflake(raw) orelse return error.MalformedInteraction },
            else => switch (raw) {
                .string => |s| .{ .string = s },
                .integer => |n| .{ .integer = n },
                .bool => |b| .{ .boolean = b },
                .float => |f| .{ .number = f },
                else => return error.MalformedInteraction,
            },
        };
        try leaves.append(arena, .{ .name = name, .value = value });
    }
}

pub fn parse(arena: Allocator, d: std.json.Value) ParseError!Interaction {
    const o = obj(d) orelse return error.MalformedInteraction;
    const kind: Kind = @fromBackingInt(@intCast(@as(u8, @intCast(if (o.get("type")) |t| (if (t == .integer and t.integer >= 0 and t.integer < 256) t.integer else 0) else 0))));
    const member = obj(o.get("member"));
    const user = if (member) |m| (if (obj(m.get("user"))) |u| parseUser(u) else null) else if (obj(o.get("user"))) |u| parseUser(u) else null;
    const data = obj(o.get("data"));
    var path: std.ArrayList(u8) = .empty;
    var leaves: std.ArrayList(Option) = .empty;
    var command_kind: u8 = 0;
    var values: std.ArrayList([]const u8) = .empty;
    var resolved: std.ArrayList(User) = .empty;
    if (data) |dd| {
        if (str(dd.get("name"))) |n| try path.appendSlice(arena, n);
        if (dd.get("type")) |t| if (t == .integer and t.integer >= 0 and t.integer < 256) {
            command_kind = @intCast(t.integer);
        };
        try flatten(arena, dd.get("options"), &path, &leaves);
        if (dd.get("values")) |vs| if (vs == .array) for (vs.array.items) |v| if (v == .string) try values.append(arena, v.string);
        if (obj(dd.get("resolved"))) |r| if (obj(r.get("users"))) |users| {
            var it = users.iterator();
            while (it.next()) |e| if (obj(e.value_ptr.*)) |u| if (parseUser(u)) |pu| try resolved.append(arena, pu);
        };
    }
    const channel = obj(o.get("channel"));
    return .{
        .id = snowflake(o.get("id")) orelse return error.MalformedInteraction,
        .application_id = snowflake(o.get("application_id")) orelse return error.MalformedInteraction,
        .kind = kind,
        .token = str(o.get("token")) orelse return error.MalformedInteraction,
        .guild_id = snowflake(o.get("guild_id")),
        .channel_id = snowflake(o.get("channel_id")) orelse (if (channel) |c| snowflake(c.get("id")) else null) orelse return error.MalformedInteraction,
        .channel_nsfw = if (channel) |c| (if (c.get("nsfw")) |n| n == .bool and n.bool else false) else false,
        .user = user orelse return error.MalformedInteraction,
        .member_permissions = if (member) |m| (if (str(m.get("permissions"))) |p| std.fmt.parseInt(u64, p, 10) catch null else null) else null,
        .command_path = path.items,
        .command_kind = command_kind,
        .options = leaves.items,
        .target_id = if (data) |dd| snowflake(dd.get("target_id")) else null,
        .custom_id = if (data) |dd| str(dd.get("custom_id")) else null,
        .values = values.items,
        .resolved_users = resolved.items,
    };
}

const testing = std.testing;

test "application command interactions flatten subcommands and read member permissions" {
    var arena: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena.deinit();
    const raw =
        \\{"id":"11","application_id":"22","type":2,"token":"tok","guild_id":"33","channel_id":"44",
        \\ "channel":{"id":"44","type":0,"nsfw":true},
        \\ "member":{"user":{"id":"55","username":"dana"},"permissions":"8"},
        \\ "data":{"id":"66","name":"persona","type":1,"options":[{"type":1,"name":"ask","options":[
        \\   {"type":3,"name":"question","value":"why is the sky blue?"},{"type":4,"name":"as","value":1}]}]}}
    ;
    const v = try std.json.parseFromSliceLeaky(std.json.Value, arena.allocator(), raw, .{});
    const i = try parse(arena.allocator(), v);
    try testing.expectEqual(Kind.application_command, i.kind);
    try testing.expectEqualStrings("persona ask", i.command_path);
    try testing.expectEqualStrings("why is the sky blue?", i.string("question").?);
    try testing.expectEqual(@as(i64, 1), i.option("as").?.integer);
    try testing.expectEqual(@as(?u64, 33), i.guild_id);
    try testing.expect(i.channel_nsfw);
    try testing.expectEqual(@as(u64, 55), i.user.id);
    try testing.expectEqual(@as(?u64, 8), i.member_permissions);
}

test "DM interactions carry the top-level user and components carry custom ids and values" {
    var arena: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena.deinit();
    const raw =
        \\{"id":"1","application_id":"2","type":3,"token":"t","channel_id":"3","user":{"id":"4","username":"u"},
        \\ "data":{"custom_id":"abbey:help:v1:4:1900:start","component_type":3,"values":["memory"]}}
    ;
    const v = try std.json.parseFromSliceLeaky(std.json.Value, arena.allocator(), raw, .{});
    const i = try parse(arena.allocator(), v);
    try testing.expect(i.guild_id == null);
    try testing.expect(i.member_permissions == null);
    try testing.expectEqualStrings("abbey:help:v1:4:1900:start", i.custom_id.?);
    try testing.expectEqualStrings("memory", i.values[0]);
    const bad = try std.json.parseFromSliceLeaky(std.json.Value, arena.allocator(), "{\"id\":\"1\"}", .{});
    try testing.expectError(error.MalformedInteraction, parse(arena.allocator(), bad));
}
