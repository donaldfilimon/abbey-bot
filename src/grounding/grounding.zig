//! Reply grounding, transcribed from the oracle's `src/grounding.rs`.
//!
//! A reply's checkable specifics (versions, dates, years, percentages,
//! statistics, quotations) are compared with a pre-candidate snapshot of what
//! the user said and the facts selected for the turn. Anything unsupported is
//! hedged, never silently removed. The scan works over decoded scalars, like
//! the oracle's `Vec<char>`.
const std = @import("std");
const text = @import("../text/text.zig");
const prompts = @import("../persona/prompts.zig");
const Allocator = std.mem.Allocator;

pub const SpecificKind = enum(u3) { version, date, year, percentage, statistic, quotation };

pub const Specific = struct {
    kind: SpecificKind,
    text: []u8,
    key: []u8,

    fn deinit(s: Specific, gpa: Allocator) void {
        gpa.free(s.text);
        gpa.free(s.key);
    }
};

const SpecificList = std.ArrayList(Specific);

fn freeList(gpa: Allocator, list: *SpecificList) void {
    for (list.items) |s| s.deinit(gpa);
    list.deinit(gpa);
}

/// Immutable-once-built snapshot. Owns its keys and haystack.
pub const Grounding = struct {
    keys: std.StringHashMapUnmanaged(void) = .empty,
    haystack: std.ArrayList(u8) = .empty,

    pub fn deinit(g: *Grounding, gpa: Allocator) void {
        var it = g.keys.keyIterator();
        while (it.next()) |k| gpa.free(k.*);
        g.keys.deinit(gpa);
        g.haystack.deinit(gpa);
    }

    pub fn clone(g: *const Grounding, gpa: Allocator) Allocator.Error!Grounding {
        var out: Grounding = .{};
        errdefer out.deinit(gpa);
        var it = g.keys.keyIterator();
        while (it.next()) |k| try out.insertKey(gpa, k.*);
        try out.haystack.appendSlice(gpa, g.haystack.items);
        return out;
    }

    pub fn isEmpty(g: *const Grounding) bool {
        return g.haystack.items.len == 0;
    }

    fn insertKey(g: *Grounding, gpa: Allocator, key: []const u8) Allocator.Error!void {
        if (g.keys.contains(key)) return;
        const owned = try gpa.dupe(u8, key);
        errdefer gpa.free(owned);
        try g.keys.put(gpa, owned, {});
    }

    pub fn pushSource(g: *Grounding, gpa: Allocator, source: []const u8) Allocator.Error!void {
        const chars = try decode(gpa, source);
        defer gpa.free(chars);
        var found: SpecificList = .empty;
        defer freeList(gpa, &found);
        try scanInto(gpa, chars, true, true, &found);
        for (found.items) |s| try g.expandKey(gpa, s);
        if (g.haystack.items.len > 0) try g.haystack.append(gpa, '\n');
        const lowered = try text.toLowercase(gpa, source);
        defer gpa.free(lowered);
        const collapsed = try text.collapseWhitespace(gpa, lowered);
        defer gpa.free(collapsed);
        try g.haystack.appendSlice(gpa, collapsed);
    }

    fn expandKey(g: *Grounding, gpa: Allocator, s: Specific) Allocator.Error!void {
        try g.insertKey(gpa, s.key);
        switch (s.kind) {
            .date => {
                var parts = std.mem.splitScalar(u8, s.key, '-');
                try g.insertKey(gpa, parts.first());
            },
            .version => {
                var count: usize = 1;
                for (s.key) |b| {
                    if (b == '.') count += 1;
                }
                var take: usize = 1;
                while (take < count) : (take += 1) {
                    var dots: usize = 0;
                    var end: usize = 0;
                    while (end < s.key.len) : (end += 1) {
                        if (s.key[end] == '.') {
                            dots += 1;
                            if (dots == take) break;
                        }
                    }
                    try g.insertKey(gpa, s.key[0..end]);
                }
            },
            .percentage => if (std.mem.endsWith(u8, s.key, "%")) {
                try g.insertKey(gpa, s.key[0 .. s.key.len - 1]);
            },
            else => {},
        }
    }

    fn grounds(g: *const Grounding, gpa: Allocator, s: Specific) Allocator.Error!bool {
        if (g.keys.contains(s.key)) return true;
        if (s.kind == .quotation) return std.mem.indexOf(u8, g.haystack.items, s.key) != null;
        if (boundaryContains(g.haystack.items, s.key)) return true;
        const lowered = try text.toLowercase(gpa, s.text);
        defer gpa.free(lowered);
        return boundaryContains(g.haystack.items, lowered);
    }
};

pub fn fromSources(gpa: Allocator, sources: []const []const u8) Allocator.Error!Grounding {
    var g: Grounding = .{};
    errdefer g.deinit(gpa);
    for (sources) |s| try g.pushSource(gpa, s);
    return g;
}

pub const Verdict = struct {
    examined: SpecificList = .empty,
    /// Indices into `examined`.
    ungrounded: std.ArrayList(usize) = .empty,
    grounding_empty: bool = false,

    pub fn deinit(v: *Verdict, gpa: Allocator) void {
        freeList(gpa, &v.examined);
        v.ungrounded.deinit(gpa);
    }

    pub fn isGrounded(v: *const Verdict) bool {
        return v.ungrounded.items.len == 0;
    }

    pub fn shouldHedge(v: *const Verdict) bool {
        return !v.grounding_empty and !v.isGrounded();
    }
};

pub fn check(gpa: Allocator, reply: []const u8, g: *const Grounding) Allocator.Error!Verdict {
    var v: Verdict = .{ .grounding_empty = g.isEmpty() };
    errdefer v.deinit(gpa);
    const chars = try decode(gpa, reply);
    defer gpa.free(chars);
    try scanInto(gpa, chars, false, true, &v.examined);
    // dedup by (kind, key), keeping first occurrences in order.
    var i: usize = 0;
    while (i < v.examined.items.len) {
        const cur = v.examined.items[i];
        var dup = false;
        for (v.examined.items[0..i]) |prev| {
            if (prev.kind == cur.kind and std.mem.eql(u8, prev.key, cur.key)) {
                dup = true;
                break;
            }
        }
        if (dup) {
            cur.deinit(gpa);
            _ = v.examined.orderedRemove(i);
        } else i += 1;
    }
    for (v.examined.items, 0..) |s, idx| {
        if (!try g.grounds(gpa, s)) try v.ungrounded.append(gpa, idx);
    }
    return v;
}

const hedge_prefix = "Heads up \u{2014} treat these as unsupported: ";
const hedge_suffix = ". Nothing in this conversation or the facts I was given contains them, and I have no source for them here.";
const hedge_max_listed = 3;
const hedge_item_chars = 40;

/// The hedge note, or null for pass-through (`Action::PassThrough`).
pub fn hedgeNote(gpa: Allocator, v: *const Verdict) Allocator.Error!?[]u8 {
    if (!v.shouldHedge()) return null;
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(gpa);
    try out.appendSlice(gpa, hedge_prefix);
    const listed = @min(v.ungrounded.items.len, hedge_max_listed);
    for (v.ungrounded.items[0..listed], 0..) |idx, n| {
        if (n > 0) try out.appendSlice(gpa, ", ");
        const flat = try text.collapseWhitespace(gpa, v.examined.items[idx].text);
        defer gpa.free(flat);
        const no_ticks = try std.mem.replaceOwned(u8, gpa, flat, "`", "");
        defer gpa.free(no_ticks);
        try out.append(gpa, '`');
        try appendClipped(gpa, &out, no_ticks, hedge_item_chars);
        try out.append(gpa, '`');
    }
    const extra = v.ungrounded.items.len - listed;
    if (extra > 0) try out.print(gpa, " and {d} more", .{extra});
    try out.appendSlice(gpa, hedge_suffix);
    return try out.toOwnedSlice(gpa);
}

/// Reply text after grounding: trimmed pass-through, or the reply cut to fit
/// the 1,900-character budget with the note appended.
pub fn hedged(gpa: Allocator, reply: []const u8, v: *const Verdict) Allocator.Error![]u8 {
    const note = try hedgeNote(gpa, v) orelse return gpa.dupe(u8, text.trim(reply));
    defer gpa.free(note);
    const sep = "\n\n";
    const used = text.charCount(note) + 2;
    const budget = if (prompts.tidy_limit_chars > used) prompts.tidy_limit_chars - used else 0;
    const body = try trimToWords(gpa, text.trim(reply), budget);
    defer gpa.free(body);
    return std.fmt.allocPrint(gpa, "{s}{s}{s}", .{ body, sep, note });
}

fn appendClipped(gpa: Allocator, out: *std.ArrayList(u8), s: []const u8, max: usize) Allocator.Error!void {
    if (text.charCount(s) <= max) return out.appendSlice(gpa, s);
    const head = s[0..text.byteOffsetOfChar(s, max -| 1)];
    try out.appendSlice(gpa, text.trimEnd(head));
    try out.appendSlice(gpa, "\u{2026}");
}

fn trimToWords(gpa: Allocator, s: []const u8, max: usize) Allocator.Error![]u8 {
    if (text.charCount(s) <= max) return gpa.dupe(u8, s);
    if (max <= 2) return gpa.dupe(u8, "");
    const head = s[0..text.byteOffsetOfChar(s, max - 2)];
    var cut: usize = text.trimEnd(head).len;
    var it = text.iterate(head);
    var last_ws: ?usize = null;
    while (true) {
        const at = it.index;
        const scalar = it.next() orelse break;
        if (text.isWhitespace(scalar.cp)) last_ws = at;
    }
    if (last_ws) |w| cut = w;
    return std.fmt.allocPrint(gpa, "{s} \u{2026}", .{text.trimEnd(head[0..cut])});
}

// ---------------------------------------------------------------------------
// Scanning.
// ---------------------------------------------------------------------------

const min_quote_chars = 6;

fn decode(gpa: Allocator, s: []const u8) Allocator.Error![]u21 {
    var out: std.ArrayList(u21) = .empty;
    errdefer out.deinit(gpa);
    var it = text.iterate(s);
    while (it.next()) |scalar| try out.append(gpa, scalar.cp);
    return out.toOwnedSlice(gpa);
}

fn encode(gpa: Allocator, chars: []const u21) Allocator.Error![]u8 {
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(gpa);
    for (chars) |c| try text.appendScalar(gpa, &out, c);
    return out.toOwnedSlice(gpa);
}

fn isConnector(c: u21) bool {
    return c == '.' or c == ',' or c == ':' or c == '-' or c == '/' or c == '_';
}

fn scanInto(gpa: Allocator, chars: []const u21, bare_integers: bool, quotes: bool, out: *SpecificList) Allocator.Error!void {
    const n = chars.len;
    var i: usize = 0;
    while (i < n) {
        const c = chars[i];
        if (quotes and (c == '"' or c == 0x201C)) {
            var close: ?usize = null;
            var j = i + 1;
            while (j < n) : (j += 1) {
                if (chars[j] == '"' or chars[j] == 0x201D) {
                    close = j;
                    break;
                }
            }
            if (close) |end| {
                const inner = chars[i + 1 .. end];
                const content = try encode(gpa, inner);
                defer gpa.free(content);
                const trimmed = text.trim(content);
                var any_alpha = false;
                var it = text.iterate(trimmed);
                while (it.next()) |scalar| {
                    if (text.isAlphabetic(scalar.cp)) any_alpha = true;
                }
                if (text.charCount(trimmed) >= min_quote_chars and any_alpha) {
                    const lowered = try text.toLowercase(gpa, trimmed);
                    defer gpa.free(lowered);
                    const key = try text.collapseWhitespace(gpa, lowered);
                    errdefer gpa.free(key);
                    const owned = try gpa.dupe(u8, trimmed);
                    errdefer gpa.free(owned);
                    try out.append(gpa, .{ .kind = .quotation, .text = owned, .key = key });
                }
                try scanInto(gpa, inner, bare_integers, false, out);
                i = end + 1;
                continue;
            }
            i += 1;
            continue;
        }
        if (text.isAlphanumeric(c)) {
            var j = i;
            while (j < n and (text.isAlphanumeric(chars[j]) or (isConnector(chars[j]) and j + 1 < n and text.isAlphanumeric(chars[j + 1])))) j += 1;
            var token_chars: std.ArrayList(u21) = .empty;
            defer token_chars.deinit(gpa);
            try token_chars.appendSlice(gpa, chars[i..j]);
            var end = j;
            if (end < n and chars[end] == '%') {
                try token_chars.append(gpa, '%');
                end += 1;
            }
            const token = try encode(gpa, token_chars.items);
            defer gpa.free(token);
            const unit = try nextWord(gpa, chars, end);
            defer if (unit) |u| gpa.free(u.word);
            if (try classify(gpa, token, if (unit) |u| u.word else null, bare_integers)) |cls| {
                errdefer gpa.free(cls.key);
                const consumes = cls.consumes_unit and unit != null;
                const specific_text = if (consumes)
                    try std.fmt.allocPrint(gpa, "{s} {s}", .{ token, unit.?.word })
                else
                    try gpa.dupe(u8, token);
                errdefer gpa.free(specific_text);
                try out.append(gpa, .{ .kind = cls.kind, .text = specific_text, .key = cls.key });
                if (consumes) {
                    i = unit.?.after;
                    continue;
                }
            }
            i = end;
            continue;
        }
        i += 1;
    }
}

const Word = struct { word: []u8, after: usize };

fn nextWord(gpa: Allocator, chars: []const u21, from: usize) Allocator.Error!?Word {
    var k = from;
    while (k < chars.len and chars[k] == ' ') k += 1;
    if (k == from) return null;
    const start = k;
    while (k < chars.len and text.isAlphabetic(chars[k])) k += 1;
    if (k == start) return null;
    const raw = try encode(gpa, chars[start..k]);
    defer gpa.free(raw);
    return .{ .word = try text.toLowercase(gpa, raw), .after = k };
}

const Classified = struct { kind: SpecificKind, key: []u8, consumes_unit: bool };

fn allDigits(s: []const u8) bool {
    for (s) |b| if (!std.ascii.isDigit(b)) return false;
    return true;
}

fn startsWithDigit(s: []const u8) bool {
    return s.len > 0 and std.ascii.isDigit(s[0]);
}

fn classify(gpa: Allocator, token: []const u8, unit: ?[]const u8, bare_integers: bool) Allocator.Error!?Classified {
    const has_percent = std.mem.endsWith(u8, token, "%");
    const core = if (has_percent) token[0 .. token.len - 1] else token;
    var vprefix = false;
    var rest = core;
    if (core.len > 0 and (core[0] == 'v' or core[0] == 'V') and startsWithDigit(core[1..])) {
        vprefix = true;
        rest = core[1..];
    }
    if (!startsWithDigit(rest)) return null;

    if (has_percent and !vprefix) {
        if (try plainNumber(gpa, rest)) |num| {
            defer gpa.free(num);
            return .{ .kind = .percentage, .key = try std.fmt.allocPrint(gpa, "{s}%", .{num}), .consumes_unit = false };
        }
    }
    if (!has_percent and !vprefix and unit != null and (eq(unit.?, "percent") or eq(unit.?, "percentage") or eq(unit.?, "pct"))) {
        if (try plainNumber(gpa, rest)) |num| {
            defer gpa.free(num);
            return .{ .kind = .percentage, .key = try std.fmt.allocPrint(gpa, "{s}%", .{num}), .consumes_unit = true };
        }
    }
    if (has_percent) return null;

    var seg_count: usize = 1;
    for (rest) |b| {
        if (b == '.') seg_count += 1;
    }
    var segs = std.mem.splitScalar(u8, rest, '.');
    const seg0 = segs.first();
    const seg1 = segs.next();
    const numeric_lead = seg_count >= 2 and seg0.len > 0 and allDigits(seg0) and startsWithDigit(seg1.?);

    if (vprefix and (numeric_lead or allDigits(rest))) {
        return .{ .kind = .version, .key = try std.ascii.allocLowerString(gpa, rest), .consumes_unit = false };
    }
    if (numeric_lead and seg_count >= 3) {
        return .{ .kind = .version, .key = try std.ascii.allocLowerString(gpa, rest), .consumes_unit = false };
    }
    if (vprefix) return null;

    if (isoDate(rest)) return .{ .kind = .date, .key = try gpa.dupe(u8, rest), .consumes_unit = false };

    if (rest.len == 4 and allDigits(rest)) {
        const year = std.fmt.parseInt(u32, rest, 10) catch 0;
        if (year >= 1900 and year <= 2099) return .{ .kind = .year, .key = try gpa.dupe(u8, rest), .consumes_unit = false };
    }

    if (std.mem.indexOfScalar(u8, rest, ',') != null) {
        if (try plainNumber(gpa, rest)) |num| return .{ .kind = .statistic, .key = num, .consumes_unit = false };
    }

    if (seg_count == 2 and seg0.len > 0 and seg1.?.len > 0 and allDigits(seg0) and allDigits(seg1.?)) {
        return .{ .kind = .statistic, .key = try gpa.dupe(u8, rest), .consumes_unit = false };
    }

    // The oracle's `rest.split_at(rest.len() - 1)` panics when the final scalar
    // is multi-byte ("12\u{e9}"); such a tail is never k/m/b/t, so Zig returns
    // "not a suffixed statistic" instead of crashing (pinned by test).
    if (rest.len >= 2) {
        const tail = rest[rest.len - 1];
        if (allDigits(rest[0 .. rest.len - 1]) and std.mem.indexOfScalar(u8, "kKmMbBtT", tail) != null) {
            return .{ .kind = .statistic, .key = try std.ascii.allocLowerString(gpa, rest), .consumes_unit = false };
        }
    }

    const is_integer = allDigits(rest);
    if (is_integer and unit != null and (eq(unit.?, "thousand") or eq(unit.?, "million") or eq(unit.?, "billion") or eq(unit.?, "trillion"))) {
        return .{ .kind = .statistic, .key = try std.fmt.allocPrint(gpa, "{s} {s}", .{ rest, unit.? }), .consumes_unit = true };
    }
    if (bare_integers and is_integer) return .{ .kind = .statistic, .key = try gpa.dupe(u8, rest), .consumes_unit = false };
    return null;
}

fn eq(a: []const u8, b: []const u8) bool {
    return std.mem.eql(u8, a, b);
}

fn isoDate(s: []const u8) bool {
    var parts = std.mem.splitScalar(u8, s, '-');
    const y = parts.next() orelse return false;
    const m = parts.next() orelse return false;
    const d = parts.next() orelse return false;
    if (parts.next() != null) return false;
    if (y.len != 4 or m.len != 2 or d.len != 2) return false;
    if (!allDigits(y) or !allDigits(m) or !allDigits(d)) return false;
    const month = std.fmt.parseInt(u32, m, 10) catch return false;
    const day = std.fmt.parseInt(u32, d, 10) catch return false;
    return month >= 1 and month <= 12 and day >= 1 and day <= 31;
}

fn plainNumber(gpa: Allocator, s: []const u8) Allocator.Error!?[]u8 {
    if (s.len == 0) return null;
    var dots: usize = 0;
    var it = text.iterate(s);
    while (it.next()) |scalar| {
        switch (scalar.cp) {
            '0'...'9', ',' => {},
            '.' => dots += 1,
            else => return null,
        }
    }
    if (dots > 1) return null;
    return try std.mem.replaceOwned(u8, gpa, s, ",", "");
}

fn prevScalar(s: []const u8, end: usize) ?u21 {
    if (end == 0) return null;
    var j = end - 1;
    while (j > 0 and (s[j] & 0xC0) == 0x80) j -= 1;
    return text.decodeAt(s, j).cp;
}

fn boundaryContains(haystack: []const u8, needle: []const u8) bool {
    if (needle.len == 0) return false;
    var start: usize = 0;
    while (std.mem.indexOfPos(u8, haystack, start, needle)) |idx| {
        start = idx + needle.len;
        const before_ok = if (prevScalar(haystack, idx)) |c|
            !(text.isAlphanumeric(c) or c == '_' or c == '-' or c == '/' or c == '.' or c == ',')
        else
            true;
        var after_it = text.Iterator{ .bytes = haystack, .index = idx + needle.len };
        const after_ok = if (after_it.next()) |sc| blk: {
            const c = sc.cp;
            if (text.isAlphanumeric(c) or c == '_' or c == '-' or c == '/') break :blk false;
            if (c == '.' or c == ',' or c == ':') {
                const d = after_it.next();
                break :blk !(d != null and d.?.cp < 0x80 and std.ascii.isDigit(@intCast(d.?.cp)));
            }
            break :blk true;
        } else true;
        if (before_ok and after_ok) return true;
    }
    return false;
}

test "prior user input grounds, assistant output does not" {
    const gpa = std.testing.allocator;
    var g = try fromSources(gpa, &.{"The lockfile says 1.2.3. Which line should ship?"});
    defer g.deinit(gpa);
    var ok = try check(gpa, "Keep 1.2.3.", &g);
    defer ok.deinit(gpa);
    try std.testing.expect(ok.isGrounded());
    var bad = try check(gpa, "Ship 4.2.1 in 2019.", &g);
    defer bad.deinit(gpa);
    try std.testing.expect(bad.shouldHedge());
    try std.testing.expectEqual(@as(usize, 2), bad.ungrounded.items.len);
}

test "a multi-byte tail after digits is not a statistic and never crashes" {
    const gpa = std.testing.allocator;
    var g = try fromSources(gpa, &.{"nothing numeric"});
    defer g.deinit(gpa);
    var v = try check(gpa, "12\u{e9} and 3k", &g);
    defer v.deinit(gpa);
    try std.testing.expectEqual(@as(usize, 1), v.examined.items.len);
    try std.testing.expectEqualStrings("3k", v.examined.items[0].key);
}
