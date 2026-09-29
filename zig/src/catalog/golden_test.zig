//! Catalog parity: the Zig registration payload serializes byte-for-byte to
//! the oracle's export, the spec table matches the oracle's specs, every
//! registered leaf has a payload entry with matching contexts and default
//! permission, and availability and /help agree with the oracle.
const std = @import("std");
const golden = @import("../testing/golden.zig");
const catalog = @import("catalog.zig");
const serialize = @import("serialize.zig");
const p = @import("payload_types.zig");

test "catalog parity: the Zig registration payload is byte-identical to the oracle export" {
    const gpa = std.testing.allocator;
    var out: std.Io.Writer.Allocating = .init(gpa);
    defer out.deinit();
    try serialize.writePayload(&out.writer, serialize.frozen());
    const want = @embedFile("golden_payload");
    if (!std.mem.eql(u8, want, out.written())) {
        const n = std.mem.indexOfDiff(u8, want, out.written()) orelse 0;
        const lo = n -| 80;
        std.debug.print("payload differs at byte {d}\n want: {f}\n  got: {f}\n", .{ n, std.json.fmt(want[lo..@min(want.len, n + 80)], .{}), std.json.fmt(out.written()[lo..@min(out.written().len, n + 80)], .{}) });
        return error.TestExpectedEqual;
    }
    try std.testing.expectEqual(@as(usize, 26), serialize.frozen().len);
}

test "catalog parity: the compact request body parses to the same document" {
    const gpa = std.testing.allocator;
    var out: std.Io.Writer.Allocating = .init(gpa);
    defer out.deinit();
    try serialize.writeCompact(&out.writer, serialize.frozen());
    try std.testing.expect(std.mem.indexOfScalar(u8, out.written(), '\n') == null);
    var a = try golden.parse(gpa, out.written());
    defer a.deinit();
    var b = try golden.parse(gpa, @embedFile("golden_payload"));
    defer b.deinit();
    try std.testing.expectEqual(b.value.array.items.len, a.value.array.items.len);
    for (a.value.array.items, b.value.array.items) |x, y| try std.testing.expectEqualStrings(golden.str(y, "name"), golden.str(x, "name"));
    // Descriptions keep their raw UTF-8 (em dash in the persona choices).
    try std.testing.expect(std.mem.indexOf(u8, out.written(), "abbey \u{2014} warm sharp friend and default") != null);
}

test "catalog specs match the oracle's 68 registered commands in order" {
    const gpa = std.testing.allocator;
    var parsed = try golden.parse(gpa, @embedFile("golden_catalog"));
    defer parsed.deinit();
    const want = golden.items(parsed.value, "specs");
    try std.testing.expectEqual(@as(usize, 68), want.len);
    try std.testing.expectEqual(want.len, catalog.registered().len);
    for (want, catalog.registered()) |w, spec| {
        try std.testing.expectEqualStrings(golden.str(w, "name"), spec.name);
        try std.testing.expectEqualStrings(golden.str(w, "description"), spec.description);
        try std.testing.expectEqualStrings(golden.str(w, "section"), spec.section.slug());
        try std.testing.expectEqual(golden.boolean(w, "private"), spec.private);
        try std.testing.expectEqual(golden.items(w, "contexts").len, spec.registration.contexts.len);
    }
}

fn findOption(options: []const p.Option, name: []const u8) ?p.Option {
    for (options) |o| if (std.mem.eql(u8, o.name, name)) return o;
    return null;
}

test "every registered leaf maps to the payload with its contexts and default permission" {
    for (catalog.registered()) |*spec| {
        var parts = std.mem.splitScalar(u8, spec.name, ' ');
        const top_name = if (spec.kind == .slash) parts.first() else spec.name;
        var top: ?p.Command = null;
        for (serialize.frozen()) |c| if (std.mem.eql(u8, c.name, top_name)) {
            top = c;
        };
        const cmd = top orelse {
            std.debug.print("no payload command for {s}\n", .{spec.name});
            return error.TestUnexpectedResult;
        };
        if (spec.kind == .slash) {
            var options = cmd.options;
            while (parts.next()) |sub| {
                const o = findOption(options, sub) orelse {
                    std.debug.print("no subcommand {s} in {s}\n", .{ sub, spec.name });
                    return error.TestUnexpectedResult;
                };
                try std.testing.expect(o.kind == 1 or o.kind == 2);
                options = o.options;
            }
        } else {
            const want_kind: u8 = if (spec.kind == .user_context) 2 else 3;
            try std.testing.expectEqual(@as(?u8, want_kind), cmd.kind);
        }
        // A top-level command is guild-only exactly when all its leaves are.
        if (spec.registration.contexts.len == 2) try std.testing.expectEqual(@as(usize, 2), cmd.contexts.len);
        // A leaf default permission appears on the top level only when every
        // sibling shares it (poise reduces with AND); check leaves alone.
        const single_leaf = spec.kind != .slash or std.mem.indexOfScalar(u8, spec.name, ' ') == null;
        if (single_leaf) {
            if (spec.registration.default_member_permissions) |perm| {
                var buf: [24]u8 = undefined;
                const s = try std.fmt.bufPrint(&buf, "{d}", .{perm.bit()});
                try std.testing.expectEqualStrings(s, cmd.default_member_permissions.?);
            } else try std.testing.expect(cmd.default_member_permissions == null);
        }
    }
}

fn parseInput(label: []const u8) catalog.EligibilityInput {
    // Mirrors the dump's construction: "<Context>/p<perm>/r<ready>/f<flags>".
    var it = std.mem.splitScalar(u8, label, '/');
    const context: catalog.InteractionContext = if (std.mem.eql(u8, it.next().?, "Guild")) .guild else .bot_dm;
    const perm_mode = it.next().?[1] - '0';
    const ready = it.next().?[1] - '0';
    const flags = it.next().?[1] - '0';
    var input: catalog.EligibilityInput = .{ .context = context };
    switch (perm_mode) {
        0 => {},
        1 => {
            input.permissions.insert(.manage_messages);
            input.permissions.insert(.manage_channels);
        },
        else => input.permissions = .full,
    }
    for ([_]catalog.Capability{ .generation, .tool_generation, .vision, .ocr, .voice_configured, .voice_local }) |cap| {
        switch (ready) {
            0 => {},
            1 => input.readiness.set(cap, .ready),
            else => input.readiness.set(cap, .{ .blocked = .unavailable }),
        }
    }
    input.self_subject = flags & 1 == 1;
    input.application_owner = flags & 2 == 2;
    input.caller_present_in_voice = if (flags & 1 == 1) true else null;
    input.vision_allowed = flags != 3;
    input.follow_up_absent = if (flags & 2 == 2) true else null;
    input.action_target_resolved = flags & 1 == 1;
    input.hierarchy_allows_action = if (flags == 0) null else flags & 1 == 1;
    input.selected_voice_mode = switch (flags) {
        0 => .off,
        1 => .local,
        else => .open_ai,
    };
    return input;
}

fn camel(b: catalog.Blocker, buf: []u8) []const u8 {
    const name = @tagName(b);
    var n: usize = 0;
    var upper = true;
    for (name) |ch| {
        if (ch == '_') {
            upper = true;
            continue;
        }
        buf[n] = if (upper) std.ascii.toUpper(ch) else ch;
        upper = false;
        n += 1;
    }
    return buf[0..n];
}

test "availability golden: 72 inputs x 68 commands match the oracle" {
    const gpa = std.testing.allocator;
    var parsed = try golden.parse(gpa, @embedFile("golden_availability"));
    defer parsed.deinit();
    var cells: usize = 0;
    for (golden.items(parsed.value, "matrix")) |row| {
        const input = parseInput(golden.str(row, "input"));
        for (golden.items(row, "availability"), catalog.registered()) |want, *spec| {
            var buf: [48]u8 = undefined;
            var cb: [32]u8 = undefined;
            const got = switch (catalog.availability(spec, &input)) {
                .ready => "ready",
                .access_blocked => |b| try std.fmt.bufPrint(&buf, "access:{s}", .{camel(b, &cb)}),
                .blocked => |b| try std.fmt.bufPrint(&buf, "blocked:{s}", .{camel(b, &cb)}),
            };
            if (!std.mem.eql(u8, want.string, got)) {
                std.debug.print("{s} / {s}: want {s} got {s}\n", .{ golden.str(row, "input"), spec.name, want.string, got });
                return error.TestExpectedEqual;
            }
            cells += 1;
        }
    }
    try std.testing.expectEqual(@as(usize, 72 * 68), cells);
    for (golden.items(parsed.value, "blockers")) |row| {
        var cb: [32]u8 = undefined;
        inline for (@typeInfo(catalog.Blocker).@"enum".field_names) |field| {
            const b = @field(catalog.Blocker, field);
            if (std.mem.eql(u8, camel(b, &cb), golden.str(row, "blocker"))) {
                try std.testing.expectEqualStrings(golden.str(row, "label"), b.label());
                try std.testing.expectEqualStrings(golden.str(row, "message"), b.message());
            }
        }
    }
    try std.testing.expectEqualStrings(golden.str(parsed.value, "ready_message"), (catalog.Availability{ .ready = {} }).message());
}

fn helpInput(label: []const u8) catalog.EligibilityInput {
    if (std.mem.eql(u8, label, "guild_all")) {
        var input: catalog.EligibilityInput = .{ .context = .guild, .application_owner = true, .caller_present_in_voice = true, .self_subject = true };
        input.permissions = .full;
        for ([_]catalog.Capability{ .generation, .tool_generation, .vision, .ocr }) |cap| input.readiness.set(cap, .ready);
        return input;
    }
    if (std.mem.eql(u8, label, "guild_none")) return .{ .context = .guild };
    if (std.mem.eql(u8, label, "dm_none")) return .{ .context = .bot_dm };
    if (std.mem.eql(u8, label, "dm_gen")) {
        var input: catalog.EligibilityInput = .{ .context = .bot_dm, .self_subject = true };
        input.readiness.set(.generation, .ready);
        return input;
    }
    var input: catalog.EligibilityInput = .{ .context = .guild, .vision_allowed = false };
    input.readiness.set(.generation, .{ .blocked = .busy });
    return input;
}

test "help golden: every section renders byte-identically for five permission shapes" {
    const gpa = std.testing.allocator;
    var parsed = try golden.parse(gpa, @embedFile("golden_catalog"));
    defer parsed.deinit();
    var n: usize = 0;
    for (golden.items(parsed.value, "help")) |row| {
        const input = helpInput(golden.str(row, "input"));
        const section = catalog.HelpSection.parse(golden.str(row, "section")).?;
        const got = try catalog.renderHelp(gpa, section, &input);
        defer gpa.free(got);
        try std.testing.expectEqualStrings(golden.str(row, "text"), got);
        n += 1;
    }
    try std.testing.expectEqual(@as(usize, 40), n);
    const readme = try catalog.renderReadme(gpa);
    defer gpa.free(readme);
    try std.testing.expectEqualStrings(golden.str(parsed.value, "readme"), readme);
}
