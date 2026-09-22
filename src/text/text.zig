//! Unicode-aware text helpers matching the Rust oracle's `str`/`char` behavior.
//!
//! The oracle (Rust 1.98.0) leans on `char::is_alphanumeric`, `is_alphabetic`,
//! `is_uppercase`, `is_whitespace` and `to_lowercase`. Zig std has no Unicode
//! property tables, so `unicode_tables.zig` is generated from a dump of Rust's
//! own tables over every scalar value (tools/gen_unicode_tables.py). Inputs are
//! UTF-8; an invalid byte decodes as U+FFFD so a malformed payload can never
//! read past a buffer or loop, and the caller-facing boundary (std.json) has
//! already validated the text in practice.
const std = @import("std");
const tables = @import("unicode_tables.zig");
const Allocator = std.mem.Allocator;

pub const replacement: u21 = 0xFFFD;

/// One decoded scalar and the byte length it occupied.
pub const Scalar = struct { cp: u21, len: u3 };

/// Decode the scalar at `s[i..]`. Invalid sequences yield U+FFFD of length 1.
pub fn decodeAt(s: []const u8, i: usize) Scalar {
    // std/unicode.zig: utf8ByteSequenceLength(first_byte) !u3, utf8Decode([]const u8) !u21
    const n = std.unicode.utf8ByteSequenceLength(s[i]) catch return .{ .cp = replacement, .len = 1 };
    if (i + n > s.len) return .{ .cp = replacement, .len = 1 };
    const cp = std.unicode.utf8Decode(s[i .. i + n]) catch return .{ .cp = replacement, .len = 1 };
    return .{ .cp = cp, .len = n };
}

/// Forward scalar iterator over UTF-8 bytes.
pub const Iterator = struct {
    bytes: []const u8,
    index: usize = 0,

    pub fn next(it: *Iterator) ?Scalar {
        if (it.index >= it.bytes.len) return null;
        const scalar = decodeAt(it.bytes, it.index);
        it.index += scalar.len;
        return scalar;
    }
};

pub fn iterate(s: []const u8) Iterator {
    return .{ .bytes = s };
}

fn inRanges(comptime ranges: []const [2]u21, cp: u21) bool {
    var lo: usize = 0;
    var hi: usize = ranges.len;
    while (lo < hi) {
        const mid = lo + (hi - lo) / 2;
        if (cp < ranges[mid][0]) {
            hi = mid;
        } else if (cp > ranges[mid][1]) {
            lo = mid + 1;
        } else return true;
    }
    return false;
}

pub fn isAlphanumeric(cp: u21) bool {
    if (cp < 0x80) return std.ascii.isAlphanumeric(@intCast(cp));
    return inRanges(&tables.alphanumeric, cp);
}

pub fn isAlphabetic(cp: u21) bool {
    if (cp < 0x80) return std.ascii.isAlphabetic(@intCast(cp));
    return inRanges(&tables.alphabetic, cp);
}

pub fn isUppercase(cp: u21) bool {
    if (cp < 0x80) return std.ascii.isUpper(@intCast(cp));
    return inRanges(&tables.uppercase, cp);
}

/// Rust `char::is_whitespace` (White_Space property), not ASCII-only.
pub fn isWhitespace(cp: u21) bool {
    return inRanges(&tables.whitespace, cp);
}

/// Append Rust's `char::to_lowercase` expansion of `cp` to `out`.
pub fn appendLowercase(gpa: Allocator, out: *std.ArrayList(u8), cp: u21) Allocator.Error!void {
    if (cp < 0x80) return out.append(gpa, std.ascii.toLower(@intCast(cp)));
    for (tables.lowercase_multi) |row| {
        if (row.cp == cp) {
            for (row.lower) |l| try appendScalar(gpa, out, l);
            return;
        }
    }
    try appendScalar(gpa, out, lowerSingle(cp));
}

fn lowerSingle(cp: u21) u21 {
    const map = &tables.lowercase_map;
    var lo: usize = 0;
    var hi: usize = map.len;
    while (lo < hi) {
        const mid = lo + (hi - lo) / 2;
        if (cp < map[mid][0]) {
            hi = mid;
        } else if (cp > map[mid][0]) {
            lo = mid + 1;
        } else return map[mid][1];
    }
    return cp;
}

pub fn appendScalar(gpa: Allocator, out: *std.ArrayList(u8), cp: u21) Allocator.Error!void {
    var buf: [4]u8 = undefined;
    // std/unicode.zig: utf8Encode(c: u21, out: []u8) !u3; every decoded value is encodable.
    const n = std.unicode.utf8Encode(cp, &buf) catch std.unicode.utf8Encode(replacement, &buf) catch unreachable;
    try out.appendSlice(gpa, buf[0..n]);
}

/// Rust `str::to_lowercase` (scalar-wise; Rust's final-sigma rule is the one
/// context-sensitive case and is applied here too).
pub fn toLowercase(gpa: Allocator, s: []const u8) Allocator.Error![]u8 {
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(gpa);
    var it = iterate(s);
    var prev_index: usize = 0;
    while (it.next()) |scalar| {
        defer prev_index = it.index;
        if (scalar.cp == 0x3A3 and isFinalSigma(s, prev_index, it.index)) {
            try appendScalar(gpa, &out, 0x3C2);
            continue;
        }
        try appendLowercase(gpa, &out, scalar.cp);
    }
    return out.toOwnedSlice(gpa);
}

/// Rust's `str::to_lowercase` maps U+03A3 to U+03C2 when it is word-final:
/// skipping Case_Ignorable scalars, the previous scalar is Cased and the next
/// is not. Both sets come from the oracle's own behavior (see the tables).
fn isFinalSigma(s: []const u8, start: usize, end: usize) bool {
    var i = start;
    const before_cased = while (i > 0) {
        var j = i - 1;
        while (j > 0 and (s[j] & 0xC0) == 0x80) j -= 1;
        const cp = decodeAt(s, j).cp;
        if (!inRanges(&tables.case_ignorable, cp)) break inRanges(&tables.cased, cp);
        i = j;
    } else false;
    if (!before_cased) return false;
    var it = Iterator{ .bytes = s, .index = end };
    const after_cased = while (it.next()) |scalar| {
        if (!inRanges(&tables.case_ignorable, scalar.cp)) break inRanges(&tables.cased, scalar.cp);
    } else false;
    return !after_cased;
}

/// Count Unicode scalar values (`chars().count()`).
pub fn charCount(s: []const u8) usize {
    var n: usize = 0;
    var it = iterate(s);
    while (it.next()) |_| n += 1;
    return n;
}

/// Byte offset of the `n`th scalar, or `s.len` when there are fewer.
pub fn byteOffsetOfChar(s: []const u8, n: usize) usize {
    var it = iterate(s);
    var count: usize = 0;
    while (count < n) : (count += 1) {
        if (it.next() == null) return s.len;
    }
    return it.index;
}

/// Rust `str::trim` (Unicode White_Space at both ends).
pub fn trim(s: []const u8) []const u8 {
    return trimEnd(trimStart(s));
}

pub fn trimStart(s: []const u8) []const u8 {
    var it = iterate(s);
    while (it.next()) |scalar| {
        if (!isWhitespace(scalar.cp)) return s[it.index - scalar.len ..];
    }
    return s[s.len..];
}

pub fn trimEnd(s: []const u8) []const u8 {
    var end = s.len;
    while (end > 0) {
        var start = end - 1;
        while (start > 0 and (s[start] & 0xC0) == 0x80) start -= 1;
        if (!isWhitespace(decodeAt(s, start).cp)) break;
        end = start;
    }
    return s[0..end];
}

/// Rust `str::split_whitespace` joined by single ASCII spaces.
pub fn collapseWhitespace(gpa: Allocator, s: []const u8) Allocator.Error![]u8 {
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(gpa);
    var it = iterate(s);
    var in_word = false;
    var word_start: usize = 0;
    while (true) {
        const at = it.index;
        const scalar = it.next();
        const ws = if (scalar) |sc| isWhitespace(sc.cp) else true;
        if (!ws and !in_word) {
            in_word = true;
            word_start = at;
        } else if (ws and in_word) {
            in_word = false;
            if (out.items.len > 0) try out.append(gpa, ' ');
            try out.appendSlice(gpa, s[word_start..at]);
        }
        if (scalar == null) break;
    }
    return out.toOwnedSlice(gpa);
}

/// `text::normalize`: drop ASCII and U+2019 apostrophes, lowercase every
/// alphanumeric scalar, and collapse every other run into one ASCII space.
pub fn normalize(gpa: Allocator, s: []const u8) Allocator.Error![]u8 {
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(gpa);
    var at_space = true;
    var it = iterate(s);
    while (it.next()) |scalar| {
        const cp = scalar.cp;
        if (cp == '\'' or cp == 0x2019) continue;
        if (isAlphanumeric(cp)) {
            try appendLowercase(gpa, &out, cp);
            at_space = false;
        } else if (!at_space) {
            try out.append(gpa, ' ');
            at_space = true;
        }
    }
    if (at_space and out.items.len > 0 and out.items[out.items.len - 1] == ' ') {
        _ = out.pop();
    }
    return out.toOwnedSlice(gpa);
}

/// `text::non_blank`: the trimmed text, or null when nothing remains.
pub fn nonBlank(s: []const u8) ?[]const u8 {
    const t = trim(s);
    return if (t.len == 0) null else t;
}

/// Rust `split_ascii_whitespace` iterator (space, \t, \n, \x0C, \r only).
pub const AsciiWords = struct {
    bytes: []const u8,
    index: usize = 0,

    pub fn next(w: *AsciiWords) ?[]const u8 {
        while (w.index < w.bytes.len and isAsciiWs(w.bytes[w.index])) w.index += 1;
        if (w.index >= w.bytes.len) return null;
        const start = w.index;
        while (w.index < w.bytes.len and !isAsciiWs(w.bytes[w.index])) w.index += 1;
        return w.bytes[start..w.index];
    }
};

pub fn asciiWords(s: []const u8) AsciiWords {
    return .{ .bytes = s };
}

/// Rust `u8::is_ascii_whitespace`: note it excludes U+000B.
pub fn isAsciiWs(b: u8) bool {
    return b == ' ' or b == '\t' or b == '\n' or b == 0x0C or b == '\r';
}

test "normalize matches the oracle's pinned cases" {
    const gpa = std.testing.allocator;
    const cases = [_][2][]const u8{
        .{ "", "" },                                              .{ "   ", "" },
        .{ "Can't\u{2014}stop!", "cant stop" },                   .{ "  Hello   WORLD  ", "hello world" },
        .{ "I don't understand this", "i dont understand this" }, .{ "It\u{2019}s fine", "its fine" },
        .{ "caf\u{e9} na\u{ef}ve", "caf\u{e9} na\u{ef}ve" },      .{ "a  b\tc\n\nd", "a b c d" },
        .{ "!!!", "" },                                           .{ "'hello'", "hello" },
    };
    for (cases) |case| {
        const got = try normalize(gpa, case[0]);
        defer gpa.free(got);
        try std.testing.expectEqualStrings(case[1], got);
    }
}

test "trim and collapse use Unicode whitespace" {
    const gpa = std.testing.allocator;
    try std.testing.expectEqualStrings("x y", trim("\u{a0} x y\u{3000}\n"));
    const got = try collapseWhitespace(gpa, "\u{a0}nb\u{2003}sp\u{3000}");
    defer gpa.free(got);
    try std.testing.expectEqualStrings("nb sp", got);
    try std.testing.expect(nonBlank("\t\n ") == null);
}

test "lowercase covers multi-scalar and final sigma" {
    const gpa = std.testing.allocator;
    const dotted = try toLowercase(gpa, "\u{130}stanbul");
    defer gpa.free(dotted);
    try std.testing.expectEqualStrings("i\u{307}stanbul", dotted);
    const sigma = try toLowercase(gpa, "\u{3A3}\u{39F}\u{3A6}\u{399}\u{391} \u{39F}\u{3A3}");
    defer gpa.free(sigma);
    try std.testing.expectEqualStrings("\u{3C3}\u{3BF}\u{3C6}\u{3B9}\u{3B1} \u{3BF}\u{3C2}", sigma);
}

test "invalid utf-8 decodes as replacement without overrun" {
    var it = iterate("a\xff\xe2\x82");
    try std.testing.expectEqual(@as(u21, 'a'), it.next().?.cp);
    try std.testing.expectEqual(replacement, it.next().?.cp);
    try std.testing.expectEqual(replacement, it.next().?.cp);
    try std.testing.expectEqual(replacement, it.next().?.cp);
    try std.testing.expect(it.next() == null);
}

test "final sigma follows the oracle's Case_Ignorable and Cased sets" {
    const gpa = std.testing.allocator;
    const golden = @import("../testing/golden.zig");
    var parsed = try golden.parse(gpa, @embedFile("golden_grounding"));
    defer parsed.deinit();
    var n: usize = 0;
    for (golden.items(parsed.value, "sigma")) |row| {
        const got = try toLowercase(gpa, golden.str(row, "input"));
        defer gpa.free(got);
        try std.testing.expectEqualStrings(golden.str(row, "lower"), got);
        n += 1;
    }
    try std.testing.expectEqual(@as(usize, 14), n);
}
