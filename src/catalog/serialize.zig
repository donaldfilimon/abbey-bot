//! Byte-identical rendering of the registration payload in serde_json's
//! `to_vec_pretty` format (two-space indent, `": "`, `{}`/`[]` when empty,
//! raw UTF-8, trailing LF), so the frozen export can be compared with `cmp`.
const std = @import("std");
const p = @import("payload_types.zig");
const payload = @import("payload.zig");
const Writer = std.Io.Writer;

/// Minimal pretty JSON emitter mirroring serde_json's PrettyFormatter.
pub const Pretty = struct {
    w: *Writer,
    depth: usize = 0,
    /// Whether the current container already has an element.
    has_value: [64]bool = @splat(false),

    fn indent(j: *Pretty) Writer.Error!void {
        try j.w.writeByte('\n');
        for (0..j.depth) |_| try j.w.writeAll("  ");
    }

    fn element(j: *Pretty) Writer.Error!void {
        if (j.has_value[j.depth]) try j.w.writeByte(',');
        j.has_value[j.depth] = true;
        try j.indent();
    }

    pub fn beginArray(j: *Pretty) Writer.Error!void {
        try j.w.writeByte('[');
        j.depth += 1;
        j.has_value[j.depth] = false;
    }

    pub fn endArray(j: *Pretty) Writer.Error!void {
        const had = j.has_value[j.depth];
        j.depth -= 1;
        if (had) try j.indent();
        try j.w.writeByte(']');
    }

    pub fn beginObject(j: *Pretty) Writer.Error!void {
        try j.w.writeByte('{');
        j.depth += 1;
        j.has_value[j.depth] = false;
    }

    pub fn endObject(j: *Pretty) Writer.Error!void {
        const had = j.has_value[j.depth];
        j.depth -= 1;
        if (had) try j.indent();
        try j.w.writeByte('}');
    }

    /// Start an array element (the caller then writes the value).
    pub fn item(j: *Pretty) Writer.Error!void {
        try j.element();
    }

    pub fn key(j: *Pretty, name: []const u8) Writer.Error!void {
        try j.element();
        try writeString(j.w, name);
        try j.w.writeAll(": ");
    }

    pub fn emptyObject(j: *Pretty) Writer.Error!void {
        try j.w.writeAll("{}");
    }
};

/// serde_json string escaping: `"`, `\\`, and C0 controls; everything else raw.
pub fn writeString(w: *Writer, s: []const u8) Writer.Error!void {
    try w.writeByte('"');
    for (s) |b| {
        switch (b) {
            '"' => try w.writeAll("\\\""),
            '\\' => try w.writeAll("\\\\"),
            '\n' => try w.writeAll("\\n"),
            '\r' => try w.writeAll("\\r"),
            '\t' => try w.writeAll("\\t"),
            0x08 => try w.writeAll("\\b"),
            0x0C => try w.writeAll("\\f"),
            0...0x07, 0x0B, 0x0E...0x1F => try w.print("\\u{x:0>4}", .{b}),
            else => try w.writeByte(b),
        }
    }
    try w.writeByte('"');
}

/// serde_json f64: integral values within 2^53 print as `N.0`.
fn writeFloat(w: *Writer, v: f64) Writer.Error!void {
    if (@floor(v) == v and @abs(v) <= 9007199254740992.0) {
        try w.print("{d}.0", .{@as(i64, @intFromFloat(v))});
    } else {
        // NOTE(SDK): no payload value takes this branch; std.fmt's shortest
        // round-trip digits are not guaranteed to match ryu's text here.
        try w.print("{d}", .{v});
    }
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
