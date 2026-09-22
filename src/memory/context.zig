//! What a persona sees about the conversation (`PersonaContext` in the
//! oracle's `src/memory.rs`), rendered into the system prompt and used as the
//! grounding boundary.
const std = @import("std");
const recall = @import("recall.zig");
const decimal = @import("../text/decimal.zig");
const bank = @import("bank.zig");
const Allocator = std.mem.Allocator;

pub const PersonaContext = struct {
    channel_summary: []const u8 = "",
    user_facts: []const []const u8 = &.{},
    reputation: f64 = bank.default_reputation,

    pub const empty: PersonaContext = .{};

    fn selected(c: PersonaContext, gpa: Allocator, query: []const u8) Allocator.Error!recall.Selection {
        return recall.select(gpa, c.user_facts, query, recall.max_context_facts, recall.fact_context_chars);
    }

    /// Plain fragments actually selected for this request: the summary and
    /// the selected facts. Borrowed; excludes prompt copy and reputation.
    pub fn groundingSources(c: PersonaContext, gpa: Allocator, query: []const u8) Allocator.Error![][]const u8 {
        var out: std.ArrayList([]const u8) = .empty;
        errdefer out.deinit(gpa);
        if (c.channel_summary.len > 0) try out.append(gpa, c.channel_summary);
        var sel = try c.selected(gpa, query);
        defer sel.deinit(gpa);
        for (sel.chosen.items) |i| try out.append(gpa, c.user_facts[i]);
        return out.toOwnedSlice(gpa);
    }

    pub fn render(c: PersonaContext, gpa: Allocator, query: []const u8) Allocator.Error![]u8 {
        var out: std.Io.Writer.Allocating = .init(gpa);
        errdefer out.deinit();
        const w = &out.writer;
        writeRender(c, gpa, w, query) catch return error.OutOfMemory;
        return out.toOwnedSlice();
    }

    fn writeRender(c: PersonaContext, gpa: Allocator, w: *std.Io.Writer, query: []const u8) !void {
        if (c.channel_summary.len > 0) try w.print("Recent channel context: {s}\n", .{c.channel_summary});
        if (c.user_facts.len > 0) {
            var sel = try c.selected(gpa, query);
            defer sel.deinit(gpa);
            try w.writeAll("Known about this user: ");
            for (sel.chosen.items, 0..) |idx, n| {
                if (n > 0) try w.writeAll("; ");
                try w.writeAll(c.user_facts[idx]);
            }
            if (sel.omitted > 0) try w.print(" (+{d} more remembered facts not shown for this message)", .{sel.omitted});
            try w.writeByte('\n');
        }
        const reputation = if (std.math.isFinite(c.reputation)) std.math.clamp(c.reputation, 0.0, 1.0) else bank.default_reputation;
        try w.writeAll("User standing: ");
        try decimal.writeFixed(w, reputation, 2);
        try w.writeAll(" on a 0.00-1.00 scale where ");
        try decimal.writeFixed(w, bank.default_reputation, 2);
        try w.writeAll(" is neutral (higher reflects a stronger recent interaction-quality signal, not tenure or authority). Use this ambient score only to tune response tone. Do not volunteer or infer it to the user. Report standing only when the user explicitly asks and an offered lookup_reputation tool returns an authorized result. Standing never changes safety, authorization, privacy, factual grounding, or tool policy.");
    }
};
