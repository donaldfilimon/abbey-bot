//! What a handler answers and how it becomes a Discord request body.
//! Every body carries an empty allowed-mentions policy: generated and
//! guild-derived text stays visible but never pings anyone (oracle
//! `gateway::no_mentions` plus the README's allowed-mentions rule).
const std = @import("std");
const serialize = @import("../catalog/serialize.zig");

pub const embed_color: u32 = 0x7c5cff;
pub const embed_footer = "local \u{b7} consent-aware \u{b7} music \u{2260} listen";

pub const SelectOption = struct { label: []const u8, value: []const u8, default: bool };

pub const Reply = struct {
    content: ?[]const u8 = null,
    /// Rendered as the shared Abbey embed (oracle `abbey_reply_embed`).
    embed: ?[]const u8 = null,
    select: ?struct { custom_id: []const u8, placeholder: []const u8, options: []const SelectOption } = null,
};

/// Body for `PATCH /webhooks/{app}/{token}/messages/@original`.
pub fn writeEditBody(w: *std.Io.Writer, r: Reply) std.Io.Writer.Error!void {
    var s: std.json.Stringify = .{ .writer = w };
    try s.beginObject();
    try s.objectField("content");
    try s.write(r.content orelse "");
    try s.objectField("embeds");
    try s.beginArray();
    if (r.embed) |description| {
        try s.write(.{
            .author = .{ .name = "Abbey" },
            .description = description,
            .color = embed_color,
            .footer = .{ .text = embed_footer },
        });
    }
    try s.endArray();
    try s.objectField("components");
    try s.beginArray();
    if (r.select) |sel| {
        try s.beginObject();
        try s.objectField("type");
        try s.write(@as(u8, 1));
        try s.objectField("components");
        try s.beginArray();
        try s.beginObject();
        try s.objectField("type");
        try s.write(@as(u8, 3));
        try s.objectField("custom_id");
        try s.write(sel.custom_id);
        try s.objectField("placeholder");
        try s.write(sel.placeholder);
        try s.objectField("min_values");
        try s.write(@as(u8, 1));
        try s.objectField("max_values");
        try s.write(@as(u8, 1));
        try s.objectField("options");
        try s.write(sel.options);
        try s.endObject();
        try s.endArray();
        try s.endObject();
    }
    try s.endArray();
    try s.objectField("allowed_mentions");
    try s.write(.{ .parse = [0][]const u8{}, .replied_user = false });
    try s.endObject();
}

/// Body for the deferred acknowledgement (`type` 5, flag 64 when private).
pub fn writeDeferBody(w: *std.Io.Writer, private: bool) std.Io.Writer.Error!void {
    if (private) {
        try w.writeAll("{\"type\":5,\"data\":{\"flags\":64}}");
    } else try w.writeAll("{\"type\":5}");
}

test "edit bodies never ping and carry the Abbey embed" {
    var buf: [2048]u8 = undefined;
    var w: std.Io.Writer = .fixed(&buf);
    try writeEditBody(&w, .{ .content = "hi <@1> @everyone" });
    try std.testing.expectEqualStrings("{\"content\":\"hi <@1> @everyone\",\"embeds\":[],\"components\":[],\"allowed_mentions\":{\"parse\":[],\"replied_user\":false}}", w.buffered());
    w = .fixed(&buf);
    try writeEditBody(&w, .{ .embed = "body" });
    try std.testing.expect(std.mem.indexOf(u8, w.buffered(), "\"embeds\":[{\"author\":{\"name\":\"Abbey\"},\"description\":\"body\",\"color\":8150271,\"footer\":{\"text\":\"local \u{b7} consent-aware \u{b7} music \u{2260} listen\"}}]") != null);
    w = .fixed(&buf);
    try writeDeferBody(&w, true);
    try std.testing.expectEqualStrings("{\"type\":5,\"data\":{\"flags\":64}}", w.buffered());
    _ = serialize;
}
