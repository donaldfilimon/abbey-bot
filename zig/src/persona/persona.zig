//! Canonical ABI persona routing, transcribed from the Rust oracle's
//! `src/persona.rs` (itself a transcription of abi-ai identity/keywords/router).
//!
//! Arithmetic is f32 with the oracle's exact operation order: `Reason.default`
//! depends on exact equality between the normalized weights and the prior, so a
//! reordered sum would silently change routing. `contracts/golden/routing.json`
//! pins the f32 bit patterns for every corpus input.
const std = @import("std");
const text = @import("../text/text.zig");
const Allocator = std.mem.Allocator;

pub const Persona = enum {
    abbey,
    aviva,
    abi,

    pub const all = [_]Persona{ .abbey, .aviva, .abi };

    pub fn name(p: Persona) []const u8 {
        return switch (p) {
            .abbey => "Abbey",
            .aviva => "Aviva",
            .abi => "Abi",
        };
    }

    pub fn register(p: Persona) []const u8 {
        return switch (p) {
            .abbey => "warm, sharp friend: result-first, clear and direct, honest about uncertainty",
            .aviva => "concise direct expert: answer, assumptions, next action",
            .abi => "orchestration, reasoning, policy, risk, and routing",
        };
    }

    pub fn handles(p: Persona) []const u8 {
        return switch (p) {
            .abbey => "everyday help, teaching, ops, coding, and anything that needs warmth plus depth",
            .aviva => "urgent execution, precise fixes, and deliberately terse answers",
            .abi => "explicit system orchestration, governance, routing, and risk review",
        };
    }
};

pub const ProfileWeights = struct {
    abbey: f32,
    aviva: f32,
    abi: f32,

    pub const prior: ProfileWeights = .{ .abbey = 0.40, .aviva = 0.30, .abi = 0.30 };

    pub fn only(p: Persona) ProfileWeights {
        return switch (p) {
            .abbey => .{ .abbey = 1.0, .aviva = 0.0, .abi = 0.0 },
            .aviva => .{ .abbey = 0.0, .aviva = 1.0, .abi = 0.0 },
            .abi => .{ .abbey = 0.0, .aviva = 0.0, .abi = 1.0 },
        };
    }

    fn normalize(w: *ProfileWeights) void {
        const total = w.abbey + w.aviva + w.abi;
        if (total > 0.0) {
            w.abbey /= total;
            w.aviva /= total;
            w.abi /= total;
        }
    }

    pub fn eql(a: ProfileWeights, b: ProfileWeights) bool {
        return a.abbey == b.abbey and a.aviva == b.aviva and a.abi == b.abi;
    }
};

const Keyword = struct { word: []const u8, abbey: f32, aviva: f32, abi: f32 };

fn kw(word: []const u8, abbey: f32, aviva: f32, abi: f32) Keyword {
    return .{ .word = word, .abbey = abbey, .aviva = aviva, .abi = abi };
}

const keywords = [29]Keyword{
    kw("analyze", 0.9, 0.5, 0.2),  kw("structure", 0.9, 0.6, 0.2),   kw("logical", 0.85, 0.7, 0.2),
    kw("compare", 0.8, 0.7, 0.2),  kw("explain", 0.95, 0.4, 0.2),    kw("creative", 0.95, 0.3, 0.1),
    kw("imagine", 0.95, 0.3, 0.1), kw("explore", 0.9, 0.4, 0.2),     kw("brainstorm", 0.95, 0.3, 0.1),
    kw("help", 0.95, 0.4, 0.2),    kw("learn", 0.95, 0.3, 0.2),      kw("frustrated", 0.95, 0.2, 0.1),
    kw("run", 0.3, 0.95, 0.2),     kw("execute", 0.3, 0.95, 0.2),    kw("deploy", 0.3, 0.95, 0.2),
    kw("build", 0.5, 0.9, 0.2),    kw("fix", 0.5, 0.95, 0.2),        kw("quick", 0.3, 0.95, 0.1),
    kw("direct", 0.3, 0.95, 0.1),  kw("concise", 0.3, 0.95, 0.1),    kw("orchestrate", 0.2, 0.2, 0.95),
    kw("routing", 0.2, 0.2, 0.95), kw("governance", 0.3, 0.3, 0.95), kw("policy", 0.3, 0.4, 0.9),
    kw("profile", 0.3, 0.3, 0.9),  kw("safe", 0.8, 0.5, 0.5),        kw("risk", 0.8, 0.6, 0.6),
    kw("design", 0.9, 0.5, 0.3),   kw("pattern", 0.85, 0.5, 0.3),
};

pub const Reason = union(enum) {
    explicit,
    weighted: ProfileWeights,
    default,
};

pub const Route = struct {
    persona: Persona,
    reason: Reason,
};

/// Rust `str::trim_matches([' ', '\t', '\r', '\n'])`.
fn trimSelectorSpace(s: []const u8) []const u8 {
    return std.mem.trim(u8, s, " \t\r\n");
}

/// Recognize only an exact leading Abbey, Aviva, or ABI token, optionally
/// prefixed with `@` and followed by whitespace, comma, or colon.
pub fn explicitSelector(input: []const u8) ?Persona {
    var remaining = trimSelectorSpace(input);
    if (remaining.len > 0 and remaining[0] == '@') remaining = remaining[1..];
    var end: usize = 0;
    while (end < remaining.len and std.ascii.isAlphabetic(remaining[end])) end += 1;
    if (end == 0) return null;
    if (end < remaining.len) {
        switch (remaining[end]) {
            ' ', '\t', '\n', '\r', 0x0B, 0x0C, ',', ':' => {},
            else => return null,
        }
    }
    const word = remaining[0..end];
    if (std.ascii.eqlIgnoreCase(word, "abbey")) return .abbey;
    if (std.ascii.eqlIgnoreCase(word, "aviva")) return .aviva;
    if (std.ascii.eqlIgnoreCase(word, "abi")) return .abi;
    return null;
}

/// Rust `trim_end_matches(['.', ',', '!', '?', ':', ';', '"', '\''])`.
fn trimKeywordPunct(word: []const u8) []const u8 {
    return std.mem.trimEnd(u8, word, ".,!?:;\"'");
}

/// `haystack.get(..needle.len())` is `None` when the cut is not on a char
/// boundary, so a multi-byte scalar straddling the cut never matches.
fn startsWithIgnoreCase(haystack: []const u8, needle: []const u8) bool {
    if (haystack.len < needle.len) return false;
    if (needle.len < haystack.len and (haystack[needle.len] & 0xC0) == 0x80) return false;
    return std.ascii.eqlIgnoreCase(haystack[0..needle.len], needle);
}

pub fn analyze(input: []const u8) ProfileWeights {
    if (explicitSelector(input)) |p| return ProfileWeights.only(p);
    var weights = ProfileWeights.prior;
    var words = text.asciiWords(input);
    while (words.next()) |word| {
        const trimmed = trimKeywordPunct(word);
        for (keywords) |entry| {
            if (startsWithIgnoreCase(trimmed, entry.word)) {
                weights.abbey += entry.abbey * 0.1;
                weights.aviva += entry.aviva * 0.1;
                weights.abi += entry.abi * 0.1;
            }
        }
    }
    weights.normalize();
    return weights;
}

fn select(w: ProfileWeights) Persona {
    if (w.abbey >= w.aviva and w.abbey >= w.abi) return .abbey;
    if (w.aviva >= w.abi) return .aviva;
    return .abi;
}

pub fn route(request: []const u8, explicit: ?Persona) Route {
    if (explicit) |p| return .{ .persona = p, .reason = .explicit };
    if (explicitSelector(request)) |p| return .{ .persona = p, .reason = .explicit };
    const weights = analyze(request);
    const persona = select(weights);
    const reason: Reason = if (weights.eql(ProfileWeights.prior)) .default else .{ .weighted = weights };
    return .{ .persona = persona, .reason = reason };
}

/// Rust `format!("{:.2}", f32)`: exact decimal expansion, ties to even.
pub fn writeFixed2(w: *std.Io.Writer, value: f64) std.Io.Writer.Error!void {
    try @import("../text/decimal.zig").writeFixed(w, value, 2);
}

pub fn describe(gpa: Allocator, r: Route) Allocator.Error![]u8 {
    var out: std.Io.Writer.Allocating = .init(gpa);
    errdefer out.deinit();
    const w = &out.writer;
    w.print("**{s}** \u{2014} {s}\nHandles: {s}\nWhy: ", .{ r.persona.name(), r.persona.register(), r.persona.handles() }) catch return error.OutOfMemory;
    switch (r.reason) {
        .explicit => w.writeAll("explicit leading name or command choice") catch return error.OutOfMemory,
        .default => w.writeAll("neutral ABI prior favors Abbey (0.40 / 0.30 / 0.30)") catch return error.OutOfMemory,
        .weighted => |x| {
            w.writeAll("ABI weights: Abbey ") catch return error.OutOfMemory;
            writeFixed2(w, x.abbey) catch return error.OutOfMemory;
            w.writeAll(" \u{b7} Aviva ") catch return error.OutOfMemory;
            writeFixed2(w, x.aviva) catch return error.OutOfMemory;
            w.writeAll(" \u{b7} ABI ") catch return error.OutOfMemory;
            writeFixed2(w, x.abi) catch return error.OutOfMemory;
        },
    }
    return out.toOwnedSlice();
}

test "neutral prior and ties favor Abbey" {
    const r = route("hello world", null);
    try std.testing.expectEqual(Persona.abbey, r.persona);
    try std.testing.expect(r.reason == .default);
    try std.testing.expectEqual(Persona.abbey, select(.{ .abbey = 1, .aviva = 1, .abi = 1 }));
}

test "leading names are exact overrides" {
    try std.testing.expectEqual(Persona.aviva, route("Aviva, be direct", null).persona);
    try std.testing.expectEqual(Persona.abi, route("@ABI: orchestrate", null).persona);
    try std.testing.expectEqual(Persona.abbey, route("Abbey help", null).persona);
    try std.testing.expectEqual(Persona.abbey, route("avivacious prose", null).persona);
    try std.testing.expectEqual(Persona.abbey, route("Please ask Aviva", null).persona);
    try std.testing.expectEqual(Persona.abi, route("Aviva execute quickly", .abi).persona);
}

test "stems match only token prefixes" {
    try std.testing.expect(analyze("running").aviva > ProfileWeights.prior.aviva);
    try std.testing.expect(analyze("overrun").eql(ProfileWeights.prior));
    try std.testing.expect(route("unsafe", null).reason == .default);
    try std.testing.expectEqual(Persona.abbey, route("\u{1F642}\u{1F642} please help", null).persona);
}
