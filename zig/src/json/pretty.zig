//! Two-space pretty JSON with the exact layout serde_json's `to_vec_pretty`
//! and Python's `json.dumps(..., indent=2)` both produce: `{}`/`[]` when
//! empty, `": "` after a key, raw UTF-8, and no trailing newline. Both the
//! frozen command payload and the Abbey corpus aggregate digest depend on it.
const std = @import("std");
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

/// Serialize a parsed `std.json.Value` in this layout, preserving key order
/// (std.json objects are insertion-ordered), for digests over re-rendered
/// documents. Integers print as integers; floats are rejected because every
/// caller's documents are integer-only and a float would need ryu/repr rules.
pub fn writeValue(j: *Pretty, v: std.json.Value) (Writer.Error || error{UnsupportedFloat})!void {
    switch (v) {
        .null => try j.w.writeAll("null"),
        .bool => |b| try j.w.writeAll(if (b) "true" else "false"),
        .integer => |n| try j.w.print("{d}", .{n}),
        .number_string => |t| try j.w.writeAll(t),
        .float => return error.UnsupportedFloat,
        .string => |s| try writeString(j.w, s),
        .array => |a| {
            try j.beginArray();
            for (a.items) |item| {
                try j.item();
                try writeValue(j, item);
            }
            try j.endArray();
        },
        .object => |o| {
            try j.beginObject();
            var it = o.iterator();
            while (it.next()) |entry| {
                try j.key(entry.key_ptr.*);
                try writeValue(j, entry.value_ptr.*);
            }
            try j.endObject();
        },
    }
}

test "layout matches serde_json and python indent=2" {
    var buf: [256]u8 = undefined;
    var w: Writer = .fixed(&buf);
    var j: Pretty = .{ .w = &w };
    try j.beginObject();
    try j.key("a");
    try j.beginArray();
    try j.item();
    try writeString(&w, "x\ny");
    try j.item();
    try j.beginObject();
    try j.endObject();
    try j.endArray();
    try j.key("b");
    try j.emptyObject();
    try j.endObject();
    try std.testing.expectEqualStrings("{\n  \"a\": [\n    \"x\\ny\",\n    {}\n  ],\n  \"b\": {}\n}", w.buffered());
}
