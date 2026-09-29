//! Conversational signals composed beside canonical routing, transcribed from
//! the oracle's `src/routing_signals.rs`. The canonical router always decides
//! first; a signal may only decide when it landed on the neutral prior.
const std = @import("std");
const text = @import("../text/text.zig");
const persona = @import("persona.zig");
const Persona = persona.Persona;
const Allocator = std.mem.Allocator;

const fire: i32 = 2;
const terse_words: usize = 12;

const Cue = struct { []const u8, i32 };

const distress_phrases = [_]Cue{
    .{ "losing my mind", 3 },         .{ "lost my mind", 3 },         .{ "out of my mind", 3 },
    .{ "at my wits end", 3 },         .{ "wits end", 3 },             .{ "about to cry", 3 },
    .{ "want to cry", 3 },            .{ "about to lose it", 3 },     .{ "give up", 3 },
    .{ "giving up", 3 },              .{ "cant take it anymore", 3 }, .{ "cant take this anymore", 3 },
    .{ "fed up", 3 },                 .{ "burned out", 3 },           .{ "burnt out", 3 },
    .{ "driving me crazy", 3 },       .{ "driving me nuts", 3 },      .{ "driving me insane", 3 },
    .{ "pulling my hair out", 3 },    .{ "tearing my hair out", 3 },  .{ "i hate this", 3 },
    .{ "im done", 3 },                .{ "i am done", 3 },            .{ "so done", 3 },
    .{ "nothing works", 3 },          .{ "nothing is working", 3 },   .{ "nothing i try", 3 },
    .{ "been at this for hours", 3 }, .{ "been stuck", 3 },           .{ "breaking point", 3 },
    .{ "im stuck", 2 },               .{ "i am stuck", 2 },           .{ "im struggling", 2 },
    .{ "i am struggling", 2 },
};

const distress_words = [_]Cue{
    .{ "stuck", 1 },       .{ "overwhelmed", 2 }, .{ "hopeless", 2 },  .{ "exhausted", 2 },
    .{ "demoralized", 2 }, .{ "defeated", 2 },    .{ "desperate", 2 }, .{ "despair", 2 },
    .{ "panicking", 2 },   .{ "panicked", 2 },    .{ "miserable", 2 }, .{ "frustrating", 2 },
    .{ "frustration", 2 }, .{ "infuriating", 2 }, .{ "maddening", 2 }, .{ "agonizing", 2 },
    .{ "dreading", 2 },    .{ "upset", 2 },       .{ "furious", 2 },   .{ "livid", 2 },
    .{ "struggling", 1 },  .{ "suffering", 2 },   .{ "anxious", 2 },   .{ "stressed", 2 },
    .{ "crying", 2 },      .{ "sobbing", 2 },
};

const confusion_phrases = [_]Cue{
    .{ "no idea", 2 },               .{ "not sure why", 2 },       .{ "not sure how", 2 },
    .{ "no clue", 2 },               .{ "cant figure it out", 2 }, .{ "cant figure out", 2 },
    .{ "cant work out", 2 },         .{ "cannot figure out", 2 },  .{ "dont understand", 2 },
    .{ "do not understand", 2 },     .{ "dont get it", 2 },        .{ "dont get why", 2 },
    .{ "makes no sense", 2 },        .{ "doesnt make sense", 2 },  .{ "does not make sense", 2 },
    .{ "what am i doing wrong", 2 }, .{ "im lost", 2 },            .{ "i am lost", 2 },
    .{ "completely lost", 2 },       .{ "totally lost", 2 },       .{ "over my head", 2 },
    .{ "in over my head", 2 },
};

const confusion_words = [_]Cue{
    .{ "confused", 2 },   .{ "confusing", 2 }, .{ "baffled", 2 },  .{ "baffling", 2 },  .{ "puzzled", 2 },
    .{ "bewildered", 2 }, .{ "unclear", 2 },   .{ "clueless", 2 }, .{ "mystified", 2 },
};

const confusion_soft = [_]Cue{
    .{ "why is", 1 }, .{ "why does", 1 }, .{ "why did", 1 }, .{ "why cant", 1 }, .{ "how do i", 1 }, .{ "what does", 1 },
};

const urgency_phrases = [_]Cue{
    .{ "right now", 2 },     .{ "as soon as possible", 2 }, .{ "on fire", 2 }, .{ "is down", 2 },
    .{ "are down", 2 },      .{ "went down", 2 },           .{ "no time", 2 }, .{ "out of time", 2 },
    .{ "need this now", 3 }, .{ "hurry up", 3 },
};

const urgency_words = [_]Cue{
    .{ "asap", 2 },       .{ "urgent", 2 }, .{ "urgently", 2 }, .{ "immediately", 2 }, .{ "emergency", 2 },
    .{ "hurry", 2 },      .{ "outage", 2 }, .{ "downtime", 2 }, .{ "blocker", 2 },     .{ "escalate", 2 },
    .{ "escalating", 2 }, .{ "pager", 2 },  .{ "paging", 2 },   .{ "sev1", 2 },        .{ "p0", 2 },
};

const urgency_soft = [_]Cue{
    .{ "now", 1 }, .{ "fast", 1 }, .{ "rush", 1 }, .{ "critical", 1 }, .{ "in prod", 1 }, .{ "in production", 1 },
};

const interrogatives = [_][]const u8{ "what", "why", "how", "when", "where", "who", "whom", "whose", "which" };

const imperatives = [_][]const u8{
    "fix",    "run",    "deploy",  "restart", "reboot", "ship", "push",   "pull",  "revert", "rollback",
    "roll",   "kill",   "stop",    "start",   "check",  "send", "make",   "do",    "get",    "update",
    "merge",  "retry",  "cancel",  "pause",   "resume", "mute", "unmute", "join",  "leave",  "add",
    "remove", "delete", "set",     "give",    "tell",   "show", "open",   "close", "clear",  "reset",
    "bump",   "patch",  "publish", "release", "page",
};

const polite = [_][]const u8{ "please", "pls", "plz", "hey", "hi", "hello", "ok", "okay", "yo", "just", "and" };

pub const Shape = enum {
    question,
    command,
    statement,

    pub fn label(s: Shape) []const u8 {
        return @tagName(s);
    }
};

pub const Signals = struct {
    distress: i32,
    confusion: i32,
    urgency: i32,
    shape: Shape,
    words: usize,
    emphasis: bool,

    pub fn detect(gpa: Allocator, input: []const u8) Allocator.Error!Signals {
        const normalized = try text.normalize(gpa, input);
        defer gpa.free(normalized);
        var words: usize = 0;
        var it = text.asciiWords(normalized);
        while (it.next()) |_| words += 1;
        const shape = shapeOf(input, normalized);
        const emphasis = hasEmphasis(input);

        var distress = score(normalized, &distress_phrases) + score(normalized, &distress_words);
        var confusion = score(normalized, &confusion_phrases) + score(normalized, &confusion_words);
        var urgency = score(normalized, &urgency_phrases) + score(normalized, &urgency_words);
        confusion += score(normalized, &confusion_soft);
        urgency += score(normalized, &urgency_soft);
        if (emphasis) {
            if (distress > 0) distress += 1;
            if (urgency > 0) urgency += 1;
        }
        switch (shape) {
            .question => if (confusion > 0) {
                confusion += 1;
            },
            .command => if (urgency > 0) {
                urgency += 1;
            },
            .statement => {},
        }
        return .{ .distress = distress, .confusion = confusion, .urgency = urgency, .shape = shape, .words = words, .emphasis = emphasis };
    }

    pub fn any(s: Signals) bool {
        return s.distress > 0 or s.confusion > 0 or s.urgency > 0;
    }

    pub fn adjustment(s: Signals) Adjustment {
        if (s.distress >= fire) return .distress;
        if (s.confusion >= fire) return .confusion;
        if (s.urgency >= fire and s.words <= terse_words and s.shape != .question and s.distress == 0 and s.confusion == 0) {
            return .terse_urgency;
        }
        return .none;
    }

    pub fn writeSummary(s: Signals, w: *std.Io.Writer) std.Io.Writer.Error!void {
        try w.print("distress {d} \u{b7} confusion {d} \u{b7} urgency {d} \u{b7} {s} \u{b7} {d} word{s}{s}", .{
            s.distress,
            s.confusion,
            s.urgency,
            s.shape.label(),
            s.words,
            if (s.words == 1) "" else "s",
            if (s.emphasis) " \u{b7} emphatic" else "",
        });
    }
};

pub const Adjustment = enum {
    none,
    distress,
    confusion,
    terse_urgency,

    fn why(a: Adjustment) []const u8 {
        return switch (a) {
            .none => "canonical routing",
            .distress => "distress cues the canonical keyword table cannot see, over the neutral prior",
            .confusion => "confusion cues the canonical keyword table cannot see, over the neutral prior",
            .terse_urgency => "terse urgency cues the canonical keyword table cannot see, over the neutral prior",
        };
    }
};

pub const ComposedRoute = struct {
    persona: Persona,
    base: persona.Route,
    signals: Signals,
    adjustment: Adjustment,

    pub fn adjusted(r: ComposedRoute) bool {
        return r.adjustment != .none;
    }

    /// True when the route stands on its own evidence, so a caller must not
    /// fall back to a session or guild default.
    pub fn isDecisive(r: ComposedRoute) bool {
        return r.base.reason != .default or r.adjusted();
    }
};

pub fn route(gpa: Allocator, request: []const u8, explicit: ?Persona) Allocator.Error!ComposedRoute {
    const base = persona.route(request, explicit);
    const signals = try Signals.detect(gpa, request);
    const adj: Adjustment = if (base.reason == .default) signals.adjustment() else .none;
    const chosen: Persona = switch (adj) {
        .none => base.persona,
        .distress, .confusion => .abbey,
        .terse_urgency => .aviva,
    };
    return .{ .persona = chosen, .base = base, .signals = signals, .adjustment = adj };
}

pub fn describe(gpa: Allocator, r: ComposedRoute) Allocator.Error![]u8 {
    if (!r.adjusted()) {
        const base = try persona.describe(gpa, r.base);
        if (!r.signals.any()) return base;
        defer gpa.free(base);
        var out: std.Io.Writer.Allocating = .init(gpa);
        errdefer out.deinit();
        const w = &out.writer;
        w.writeAll(base) catch return error.OutOfMemory;
        w.writeAll("\nSignals: ") catch return error.OutOfMemory;
        r.signals.writeSummary(w) catch return error.OutOfMemory;
        w.writeAll(" (canonical routing kept)") catch return error.OutOfMemory;
        return out.toOwnedSlice();
    }
    var out: std.Io.Writer.Allocating = .init(gpa);
    errdefer out.deinit();
    const w = &out.writer;
    w.print("**{s}** \u{2014} {s}\nHandles: {s}\nWhy: {s}\nSignals: ", .{
        r.persona.name(), r.persona.register(), r.persona.handles(), r.adjustment.why(),
    }) catch return error.OutOfMemory;
    r.signals.writeSummary(w) catch return error.OutOfMemory;
    return out.toOwnedSlice();
}

/// Whole-word `contains` over normalized text (space-delimited). Walks the
/// same non-overlapping occurrences Rust's `match_indices` yields.
fn containsPhrase(haystack: []const u8, phrase: []const u8) bool {
    var start: usize = 0;
    while (std.mem.indexOfPos(u8, haystack, start, phrase)) |at| {
        const end = at + phrase.len;
        const before = at == 0 or haystack[at - 1] == ' ';
        const after = end == haystack.len or haystack[end] == ' ';
        if (before and after) return true;
        start = end;
    }
    return false;
}

fn score(normalized: []const u8, table: []const Cue) i32 {
    var total: i32 = 0;
    for (table) |cue| {
        if (containsPhrase(normalized, cue[0])) total += cue[1];
    }
    return total;
}

fn hasEmphasis(input: []const u8) bool {
    if (input.len >= 2) {
        for (input[0 .. input.len - 1], input[1..]) |a, b| {
            if ((a == '!' or a == '?') and (b == '!' or b == '?')) return true;
        }
    }
    var upper: usize = 0;
    var letters: usize = 0;
    var it = text.iterate(input);
    while (it.next()) |scalar| {
        if (text.isAlphabetic(scalar.cp)) {
            letters += 1;
            if (text.isUppercase(scalar.cp)) upper += 1;
        }
    }
    return letters >= 6 and upper * 5 >= letters * 3;
}

fn inList(list: []const []const u8, token: []const u8) bool {
    for (list) |item| if (std.mem.eql(u8, item, token)) return true;
    return false;
}

/// First token that carries shape, skipping politeness and "could you".
/// Token count is bounded by the input, so a fixed buffer is not needed: the
/// iterator is re-walked instead of collecting into a Vec.
fn leadingToken(normalized: []const u8) ?[]const u8 {
    var it = text.asciiWords(normalized);
    var current = it.next();
    while (true) {
        while (current) |t| {
            if (!inList(&polite, t)) break;
            current = it.next();
        }
        const t = current orelse return null;
        if (std.mem.eql(u8, t, "can") or std.mem.eql(u8, t, "could") or std.mem.eql(u8, t, "would") or std.mem.eql(u8, t, "will")) {
            var peek = it;
            if (peek.next()) |n| {
                if (std.mem.eql(u8, n, "you") or std.mem.eql(u8, n, "u")) {
                    it = peek;
                    current = it.next();
                    continue;
                }
            }
        }
        return t;
    }
}

fn shapeOf(original: []const u8, normalized: []const u8) Shape {
    const leading = leadingToken(normalized);
    const trimmed = text.trimEnd(original);
    if ((trimmed.len > 0 and trimmed[trimmed.len - 1] == '?') or (leading != null and inList(&interrogatives, leading.?))) {
        return .question;
    }
    if (leading != null and inList(&imperatives, leading.?)) return .command;
    return .statement;
}

test "signals only decide over the neutral prior" {
    const gpa = std.testing.allocator;
    const distressed = try route(gpa, "I'm losing my mind with this", null);
    try std.testing.expectEqual(Adjustment.distress, distressed.adjustment);
    try std.testing.expectEqual(Persona.abbey, distressed.persona);
    const keyed = try route(gpa, "deploy it, I'm losing my mind", null);
    try std.testing.expectEqual(Adjustment.none, keyed.adjustment);
    const terse = try route(gpa, "restart it now!!", null);
    try std.testing.expectEqual(Persona.aviva, terse.persona);
}
