//! Replays the oracle goldens for routing, prompts and tidy. Every assertion
//! names the input so a divergence is attributable.
const std = @import("std");
const golden = @import("../testing/golden.zig");
const text = @import("../text/text.zig");
const persona = @import("persona.zig");
const signals = @import("signals.zig");
const prompts = @import("prompts.zig");
const roleplay = @import("roleplay.zig");
const Persona = persona.Persona;

fn personaFrom(name: []const u8) Persona {
    return std.meta.stringToEnum(Persona, name) orelse std.debug.panic("persona {s}", .{name});
}

fn fail(what: []const u8, input: []const u8, want: []const u8, got: []const u8) error{TestExpectedEqual} {
    std.debug.print("golden mismatch [{s}] for input {f}\n want: {f}\n  got: {f}\n", .{
        what, std.json.fmt(input, .{}), std.json.fmt(want, .{}), std.json.fmt(got, .{}),
    });
    return error.TestExpectedEqual;
}

test "routing golden: canonical weights, reasons, signals and describe match the oracle byte for byte" {
    const gpa = std.testing.allocator;
    var parsed = try golden.parse(gpa, @embedFile("golden_routing"));
    defer parsed.deinit();
    var count: usize = 0;
    for (golden.items(parsed.value, "routes")) |row| {
        const input = golden.str(row, "input");
        count += 1;

        const sel = persona.explicitSelector(input);
        const want_sel = golden.field(row, "explicit_selector");
        if (want_sel == .null) {
            if (sel != null) return fail("explicit_selector", input, "null", @tagName(sel.?));
        } else if (sel == null or sel.? != personaFrom(want_sel.string)) {
            return fail("explicit_selector", input, want_sel.string, if (sel) |s| @tagName(s) else "null");
        }

        const w = persona.analyze(input);
        const bits = golden.items(row, "weights_bits");
        const got_bits = [3]u32{ @bitCast(w.abbey), @bitCast(w.aviva), @bitCast(w.abi) };
        for (got_bits, bits) |g, want| {
            if (g != @as(u32, @intCast(want.integer))) {
                std.debug.print("weights for {f}: got {any} want {d},{d},{d}\n", .{ std.json.fmt(input, .{}), got_bits, bits[0].integer, bits[1].integer, bits[2].integer });
                return error.TestExpectedEqual;
            }
        }

        const r = persona.route(input, null);
        if (r.persona != personaFrom(golden.str(row, "persona"))) return fail("persona", input, golden.str(row, "persona"), @tagName(r.persona));
        if (!std.mem.eql(u8, @tagName(r.reason), golden.str(row, "reason"))) return fail("reason", input, golden.str(row, "reason"), @tagName(r.reason));

        const d = try persona.describe(gpa, r);
        defer gpa.free(d);
        if (!std.mem.eql(u8, d, golden.str(row, "describe"))) return fail("describe", input, golden.str(row, "describe"), d);

        const n = try text.normalize(gpa, input);
        defer gpa.free(n);
        if (!std.mem.eql(u8, n, golden.str(row, "normalized"))) return fail("normalize", input, golden.str(row, "normalized"), n);

        const c = try signals.route(gpa, input, null);
        const s = golden.field(row, "signals");
        if (c.signals.distress != golden.int(s, "distress") or c.signals.confusion != golden.int(s, "confusion") or
            c.signals.urgency != golden.int(s, "urgency") or c.signals.words != @as(usize, @intCast(golden.int(s, "words"))) or
            c.signals.emphasis != golden.boolean(s, "emphasis") or !std.mem.eql(u8, c.signals.shape.label(), golden.str(s, "shape")))
        {
            std.debug.print("signals for {f}: got {any}\n", .{ std.json.fmt(input, .{}), c.signals });
            return error.TestExpectedEqual;
        }
        if (!std.mem.eql(u8, @tagName(c.adjustment), golden.str(row, "adjustment"))) return fail("adjustment", input, golden.str(row, "adjustment"), @tagName(c.adjustment));
        if (c.persona != personaFrom(golden.str(row, "composed_persona"))) return fail("composed_persona", input, golden.str(row, "composed_persona"), @tagName(c.persona));
        if (c.isDecisive() != golden.boolean(row, "decisive")) return fail("decisive", input, "", "");
        const cd = try signals.describe(gpa, c);
        defer gpa.free(cd);
        if (!std.mem.eql(u8, cd, golden.str(row, "composed_describe"))) return fail("composed_describe", input, golden.str(row, "composed_describe"), cd);
    }
    try std.testing.expect(count >= 200);

    for (golden.items(parsed.value, "forced")) |row| {
        const forced = personaFrom(golden.str(row, "forced"));
        const c = try signals.route(gpa, "anything at all", forced);
        const cd = try signals.describe(gpa, c);
        defer gpa.free(cd);
        try std.testing.expectEqualStrings(golden.str(row, "describe"), cd);
        try std.testing.expectEqual(golden.boolean(row, "decisive"), c.isDecisive());
    }
}

test "prompt golden: system prompts, honesty copy, failures and roleplay messages are byte-exact" {
    const gpa = std.testing.allocator;
    var parsed = try golden.parse(gpa, @embedFile("golden_prompts"));
    defer parsed.deinit();
    const personas = golden.field(parsed.value, "personas");
    for (Persona.all) |p| {
        const row = golden.field(personas, @tagName(p));
        const sp = try prompts.systemPrompt(gpa, p);
        defer gpa.free(sp);
        try std.testing.expectEqualStrings(golden.str(row, "system_prompt"), sp);
        const dr = try prompts.degradedReply(gpa, p);
        defer gpa.free(dr);
        try std.testing.expectEqualStrings(golden.str(row, "degraded_reply"), dr);
        const ra = try prompts.renderAnswer(gpa, p, "configured endpoint", "the answer");
        defer gpa.free(ra);
        try std.testing.expectEqualStrings(golden.str(row, "render_answer"), ra);
        inline for (.{ .{ "failure_backend", prompts.FailureKind.backend }, .{ "failure_busy", prompts.FailureKind.busy }, .{ "failure_budget", prompts.FailureKind.response_budget } }) |case| {
            const f = try prompts.renderFailure(gpa, p, "configured endpoint", case[1]);
            defer gpa.free(f);
            try std.testing.expectEqualStrings(golden.str(row, case[0]), f);
        }
        const pair = golden.items(row, "summarize");
        const sum = try prompts.summarizePrompt(gpa, p, "a: hi\nb: yo", 2);
        defer gpa.free(sum.system);
        defer gpa.free(sum.user);
        try std.testing.expectEqualStrings(pair[0].string, sum.system);
        try std.testing.expectEqualStrings(pair[1].string, sum.user);
    }
    const rp = golden.field(parsed.value, "roleplay");
    try std.testing.expectEqualStrings(golden.str(rp, "allow"), roleplay.Decision.allow_aviva.message());
    try std.testing.expectEqualStrings(golden.str(rp, "disabled"), roleplay.Decision.refuse_disabled.message());
    try std.testing.expectEqualStrings(golden.str(rp, "guild_sfw"), roleplay.Decision.refuse_guild_sfw.message());
    try std.testing.expectEqualStrings(golden.str(parsed.value, "busy_reason"), prompts.busy_reason);
    const welcome = try prompts.welcomePrompt(gpa, "Dana");
    defer gpa.free(welcome);
    try std.testing.expectEqualStrings(golden.str(parsed.value, "welcome_dana"), welcome);
}

test "tidy golden: echo stripping, headings, blank runs and sentence cuts match the oracle" {
    const gpa = std.testing.allocator;
    var parsed = try golden.parse(gpa, @embedFile("golden_tidy"));
    defer parsed.deinit();
    var count: usize = 0;
    for (parsed.value.array.items) |row| {
        const p = personaFrom(golden.str(row, "persona"));
        const input = golden.str(row, "input");
        const got = try prompts.tidyReply(gpa, p, input);
        defer gpa.free(got);
        if (!std.mem.eql(u8, got, golden.str(row, "output"))) return fail("tidy", input, golden.str(row, "output"), got);
        count += 1;
    }
    try std.testing.expect(count >= 600);
}
