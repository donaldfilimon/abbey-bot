//! Types for the frozen registration payload (`payload.zig`).
pub const Choice = struct { name: []const u8, value: i64 };

pub const Option = struct {
    /// Discord ApplicationCommandOptionType (1 subcommand, 2 group, 3 string,
    /// 4 integer, 5 boolean, 6 user, 7 channel, 8 role, 10 number, 11 attachment).
    kind: u8,
    name: []const u8,
    description: []const u8,
    required: bool = false,
    choices: []const Choice = &.{},
    options: []const Option = &.{},
    min_value: ?f64 = null,
    max_value: ?f64 = null,
    min_length: ?u16 = null,
    max_length: ?u16 = null,
    autocomplete: bool = false,
};

pub const Command = struct {
    name: []const u8,
    /// Absent for context-menu commands (their `type` is 2 or 3).
    description: ?[]const u8 = null,
    /// Absent (chat input) or 2 (user) / 3 (message).
    kind: ?u8 = null,
    default_member_permissions: ?[]const u8 = null,
    contexts: []const u8,
    options: []const Option = &.{},
};
