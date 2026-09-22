//! Byte-identical rendering of the registration payload in serde_json's
//! `to_vec_pretty` format (two-space indent, `": "`, `{}`/`[]` when empty,
//! raw UTF-8, trailing LF), so the frozen export can be compared with `cmp`.
const std = @import("std");
const p = @import("payload_types.zig");
const payload = @import("payload.zig");
const pretty_json = @import("../json/pretty.zig");
pub const Pretty = pretty_json.Pretty;
pub const writeString = pretty_json.writeString;
const Writer = std.Io.Writer;

/// serde_json f64: integral values within 2^53 print as `N.0`. The frozen
/// payload holds no other f64, and std.fmt's shortest digits are not proven
/// equal to serde_json's ryu text, so any other value is refused rather than
/// written with unverified bytes.
fn writeFloat(w: *Writer, v: f64) Writer.Error!void {
    if (@floor(v) == v and @abs(v) <= 9007199254740992.0) {
        try w.print("{d}.0", .{@as(i64, @intFromFloat(v))});
    } else return error.WriteFailed;
}

test "payload floats: integral values print as N.0 and anything else is refused" {
    var buf: [32]u8 = undefined;
    var w: Writer = .fixed(&buf);
    try writeFloat(&w, 6000.0);
    try std.testing.expectEqualStrings("6000.0", w.buffered());
    try std.testing.expectError(error.WriteFailed, writeFloat(&w, 0.5));
}

fn writeOptional(j: *Pretty, name: []const u8, comptime T: type, value: ?T) Writer.Error!void {
    try j.key(name);
    if (value) |v| {
        if (T == f64) try writeFloat(j.w, v) else try j.w.print("{d}", .{v});
    } else try j.w.writeAll("null");
}

fn writeOption(j: *Pretty, o: p.Option) Writer.Error!void {
    try j.beginObject();
    try j.key("type");
    try j.w.print("{d}", .{o.kind});
    try j.key("name");
    try writeString(j.w, o.name);
    try j.key("description");
    try writeString(j.w, o.description);
    try j.key("required");
    try j.w.writeAll(if (o.required) "true" else "false");
    try j.key("choices");
    try j.beginArray();
    for (o.choices) |c| {
        try j.item();
        try j.beginObject();
        try j.key("name");
        try writeString(j.w, c.name);
        try j.key("name_localizations");
        try j.emptyObject();
        try j.key("value");
        try j.w.print("{d}", .{c.value});
        try j.endObject();
    }
    try j.endArray();
    try j.key("options");
    try j.beginArray();
    for (o.options) |child| {
        try j.item();
        try writeOption(j, child);
    }
    try j.endArray();
    try j.key("channel_types");
    try j.beginArray();
    try j.endArray();
    try writeOptional(j, "min_value", f64, o.min_value);
    try writeOptional(j, "max_value", f64, o.max_value);
    try writeOptional(j, "min_length", u16, o.min_length);
    try writeOptional(j, "max_length", u16, o.max_length);
    try j.key("autocomplete");
    try j.w.writeAll(if (o.autocomplete) "true" else "false");
    try j.endObject();
}

pub fn writeCommand(j: *Pretty, c: p.Command) Writer.Error!void {
    try j.beginObject();
    try j.key("name");
    try writeString(j.w, c.name);
    try j.key("name_localizations");
    try j.emptyObject();
    if (c.description) |d| {
        try j.key("description");
        try writeString(j.w, d);
    }
    try j.key("description_localizations");
    try j.emptyObject();
    try j.key("options");
    try j.beginArray();
    for (c.options) |o| {
        try j.item();
        try writeOption(j, o);
    }
    try j.endArray();
    if (c.kind) |k| {
        try j.key("type");
        try j.w.print("{d}", .{k});
    }
    if (c.default_member_permissions) |perm| {
        try j.key("default_member_permissions");
        try writeString(j.w, perm);
    }
    try j.key("contexts");
    try j.beginArray();
    for (c.contexts) |ctx| {
        try j.item();
        try j.w.print("{d}", .{ctx});
    }
    try j.endArray();
    try j.key("nsfw");
    try j.w.writeAll("false");
    try j.endObject();
}

/// Render `commands` as the pretty payload with a trailing LF.
pub fn writePayload(w: *Writer, commands: []const p.Command) Writer.Error!void {
    var j: Pretty = .{ .w = w };
    try j.beginArray();
    for (commands) |c| {
        try j.item();
        try writeCommand(&j, c);
    }
    try j.endArray();
    try w.writeByte('\n');
}

/// Compact form for the Discord bulk-overwrite request body.
pub fn writeCompact(w: *Writer, commands: []const p.Command) Writer.Error!void {
    var buf: [256 * 1024]u8 = undefined;
    var pretty: Writer = .fixed(&buf);
    try writePayload(&pretty, commands);
    // The pretty form carries whitespace only between tokens and inside
    // strings; strip the former by tracking string state.
    var in_string = false;
    var escaped = false;
    for (pretty.buffered()) |b| {
        if (in_string) {
            try w.writeByte(b);
            if (escaped) {
                escaped = false;
            } else if (b == '\\') {
                escaped = true;
            } else if (b == '"') in_string = false;
            continue;
        }
        switch (b) {
            ' ', '\n' => {},
            '"' => {
                in_string = true;
                try w.writeByte(b);
            },
            else => try w.writeByte(b),
        }
    }
}

pub fn frozen() []const p.Command {
    return &payload.commands;
}
