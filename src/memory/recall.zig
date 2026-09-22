//! Relevance selection over a user's durable facts, transcribed from the
//! oracle's `src/recall.rs`: lexical overlap weighted by rarity across the
//! user's own facts, score descending then newest first, under a count and a
//! character budget. Ranking never deletes.
const std = @import("std");
const text = @import("../text/text.zig");
const Allocator = std.mem.Allocator;

pub const max_context_facts: usize = 8;
pub const fact_context_chars: usize = 1_200;

const stopwords = [_][]const u8{
    "a",     "about", "all",   "an",   "and",    "any",   "are",    "as",    "at",   "be",   "been", "but",  "by",    "can",
    "could", "did",   "do",    "does", "for",    "from",  "had",    "has",   "have", "he",   "her",  "here", "his",   "how",
    "i",     "if",    "in",    "into", "is",     "it",    "its",    "just",  "me",   "my",   "no",   "not",  "of",    "on",
    "or",    "our",   "out",   "over", "please", "she",   "should", "so",    "some", "than", "that", "the",  "their", "them",
    "then",  "there", "these", "they", "this",   "those", "to",     "too",   "us",   "very", "was",  "we",   "were",  "what",
    "when",  "where", "which", "who",  "why",    "will",  "with",   "would", "you",  "your",
};

fn isStopword(word: []const u8) bool {
    for (stopwords) |s| if (std.mem.eql(u8, s, word)) return true;
    return false;
}

/// Owned token list for one text.
const Tokens = struct {
    words: std.ArrayList([]u8) = .empty,

    fn deinit(t: *Tokens, gpa: Allocator) void {
        for (t.words.items) |w| gpa.free(w);
        t.words.deinit(gpa);
    }

    fn contains(t: *const Tokens, word: []const u8) bool {
        for (t.words.items) |w| if (std.mem.eql(u8, w, word)) return true;
        return false;
    }
};

/// Split on every non-alphanumeric scalar, lowercase (`str::to_lowercase`),
/// drop stopwords.
fn tokenize(gpa: Allocator, s: []const u8) Allocator.Error!Tokens {
    var out: Tokens = .{};
    errdefer out.deinit(gpa);
    var it = text.iterate(s);
    var start: usize = 0;
    while (true) {
        const at = it.index;
        const scalar = it.next();
        const boundary = if (scalar) |sc| !text.isAlphanumeric(sc.cp) else true;
        if (boundary) {
            if (at > start) {
                const lowered = try text.toLowercase(gpa, s[start..at]);
                if (isStopword(lowered)) {
                    gpa.free(lowered);
                } else {
                    errdefer gpa.free(lowered);
                    try out.words.append(gpa, lowered);
                }
            }
            start = it.index;
        }
        if (scalar == null) break;
    }
    return out;
}

pub const Selection = struct {
    /// Indices into the caller's fact slice, most relevant first.
    chosen: std.ArrayList(usize) = .empty,
    omitted: usize = 0,

    pub fn deinit(s: *Selection, gpa: Allocator) void {
        s.chosen.deinit(gpa);
    }
};

pub fn select(gpa: Allocator, facts: []const []const u8, query: []const u8, max_facts: usize, char_budget: usize) Allocator.Error!Selection {
    var sel: Selection = .{};
    errdefer sel.deinit(gpa);
    if (facts.len == 0 or max_facts == 0) {
        sel.omitted = facts.len;
        return sel;
    }
    const fact_tokens = try gpa.alloc(Tokens, facts.len);
    var built: usize = 0;
    defer {
        for (fact_tokens[0..built]) |*t| t.deinit(gpa);
        gpa.free(fact_tokens);
    }
    for (facts, 0..) |f, i| {
        fact_tokens[i] = try tokenize(gpa, f);
        built += 1;
    }
    var query_tokens = try tokenize(gpa, query);
    defer query_tokens.deinit(gpa);
    // sort + dedup the query terms, as the oracle does.
    std.mem.sort([]u8, query_tokens.words.items, {}, struct {
        fn lt(_: void, a: []u8, b: []u8) bool {
            return std.mem.lessThan(u8, a, b);
        }
    }.lt);
    var terms: std.ArrayList([]const u8) = .empty;
    defer terms.deinit(gpa);
    for (query_tokens.words.items) |w| {
        if (terms.items.len == 0 or !std.mem.eql(u8, terms.items[terms.items.len - 1], w)) try terms.append(gpa, w);
    }

    const Scored = struct { score: f64, index: usize };
    const scored = try gpa.alloc(Scored, facts.len);
    defer gpa.free(scored);
    for (fact_tokens, 0..) |*ft, index| {
        var score: f64 = 0.0;
        for (terms.items) |term| {
            if (ft.contains(term)) {
                var df: usize = 0;
                for (fact_tokens) |*candidate| {
                    if (candidate.contains(term)) df += 1;
                }
                score += 1.0 / (1.0 + @as(f64, @floatFromInt(df)));
            }
        }
        scored[index] = .{ .score = score, .index = index };
    }
    std.mem.sort(Scored, scored, {}, struct {
        fn lt(_: void, a: Scored, b: Scored) bool {
            if (a.score != b.score) return a.score > b.score;
            return a.index > b.index;
        }
    }.lt);
    var used: usize = 0;
    for (scored) |entry| {
        if (sel.chosen.items.len >= max_facts) break;
        const cost = text.charCount(facts[entry.index]);
        if (used + cost > char_budget) continue;
        used += cost;
        try sel.chosen.append(gpa, entry.index);
    }
    sel.omitted = facts.len - sel.chosen.items.len;
    return sel;
}

test "rare terms dominate and zero-score facts fill newest first" {
    const gpa = std.testing.allocator;
    const facts = [_][]const u8{ "likes rust", "has a cat", "writes rust at night about a cat" };
    var sel = try select(gpa, &facts, "a rust question", 8, 1200);
    defer sel.deinit(gpa);
    try std.testing.expectEqualSlices(usize, &.{ 2, 0, 1 }, sel.chosen.items);
    try std.testing.expectEqual(@as(usize, 0), sel.omitted);
}
