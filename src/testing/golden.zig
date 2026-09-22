//! Helpers for replaying oracle goldens (contracts/golden/*.json) in tests.
const std = @import("std");

pub const Parsed = std.json.Parsed(std.json.Value);

pub fn parse(gpa: std.mem.Allocator, bytes: []const u8) !Parsed {
    return std.json.parseFromSlice(std.json.Value, gpa, bytes, .{});
}

pub fn field(v: std.json.Value, name: []const u8) std.json.Value {
    return v.object.get(name) orelse std.debug.panic("golden field missing: {s}", .{name});
}

pub fn str(v: std.json.Value, name: []const u8) []const u8 {
    return field(v, name).string;
}

pub fn int(v: std.json.Value, name: []const u8) i64 {
    return field(v, name).integer;
}

pub fn boolean(v: std.json.Value, name: []const u8) bool {
    return field(v, name).bool;
}

pub fn items(v: std.json.Value, name: []const u8) []std.json.Value {
    return field(v, name).array.items;
}
