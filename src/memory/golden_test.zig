//! Replays contracts/golden/memory.json: context render, grounding sources
//! and fact validation against the oracle.
const std = @import("std");
const golden = @import("../testing/golden.zig");
const bank = @import("bank.zig");
const PersonaContext = @import("context.zig").PersonaContext;

test "memory golden: persona context render, grounding sources and fact validation match the oracle" {
    const gpa = std.testing.allocator;
    var parsed = try golden.parse(gpa, @embedFile("golden_memory"));
    defer parsed.deinit();
    const all_facts = golden.items(parsed.value, "facts");
    const long_facts = golden.items(parsed.value, "long_facts");
    var facts_buf: [16][]const u8 = undefined;
    var rows: usize = 0;
    for (golden.items(parsed.value, "contexts")) |row| {
        const facts_field = golden.field(row, "facts");
        var n: usize = 0;
        if (facts_field == .string) {
            for (long_facts) |f| {
                facts_buf[n] = f.string;
                n += 1;
            }
        } else {
            n = @intCast(facts_field.integer);
            for (all_facts[0..n], 0..) |f, i| facts_buf[i] = f.string;
        }
        const rep_bits: u64 = @bitCast(golden.int(row, "reputation_bits"));
        const ctx: PersonaContext = .{
            .channel_summary = golden.str(row, "summary"),
            .user_facts = facts_buf[0..n],
            .reputation = @bitCast(rep_bits),
        };
        const query = golden.str(row, "query");
        const rendered = try ctx.render(gpa, query);
        defer gpa.free(rendered);
        try std.testing.expectEqualStrings(golden.str(row, "render"), rendered);
        const sources = try ctx.groundingSources(gpa, query);
        defer gpa.free(sources);
        const want = golden.items(row, "grounding_sources");
        try std.testing.expectEqual(want.len, sources.len);
        for (want, sources) |w, s| try std.testing.expectEqualStrings(w.string, s);
        rows += 1;
    }
    try std.testing.expect(rows >= 26);

    for (golden.items(parsed.value, "validated")) |row| {
        const input = golden.str(row, "input");
        const result = golden.field(row, "result");
        if (result.object.get("ok")) |ok| {
            const got = try bank.validatedFact(gpa, input);
            defer gpa.free(got);
            try std.testing.expectEqualStrings(ok.string, got);
        } else {
            const err_text = result.object.get("err").?.string;
            if (bank.validatedFact(gpa, input)) |got| {
                gpa.free(got);
                return error.TestExpectedError;
            } else |err| switch (err) {
                error.OutOfMemory => return err,
                else => |e| try std.testing.expectEqualStrings(err_text, bank.factErrorMessage(e)),
            }
        }
    }
}
