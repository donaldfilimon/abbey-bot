//! The deterministic 32-dimension text embedding WDBX expects, transcribed
//! from the oracle's `src/embedding.rs` ("pinned to abi's own vectors"):
//! weighted 1/2/3-byte n-grams, ASCII-lowercased, hashed with Wyhash (seed =
//! n), signed by the hash's top bit, then L2-normalized. std.hash.Wyhash is
//! the same function abi and the oracle's wyhash.rs transcription compute
//! (contracts/fixtures/wyhash_zig_refs.txt pins it).
const std = @import("std");

pub const dim = 32;

const grams = [_]struct { n: usize, weight: f32 }{ .{ .n = 1, .weight = 0.5 }, .{ .n = 2, .weight = 1.0 }, .{ .n = 3, .weight = 1.5 } };

pub fn textEmbedding(input: []const u8) [dim]f32 {
    var out: [dim]f32 = @splat(0.0);
    if (input.len == 0) {
        out[0] = 1.0;
        return out;
    }
    var window: [3]u8 = undefined;
    for (grams) |g| {
        var i: usize = 0;
        while (i + g.n <= input.len) : (i += 1) {
            for (0..g.n) |k| window[k] = std.ascii.toLower(input[i + k]);
            // std/hash/wyhash.zig: Wyhash.hash(seed: u64, input: []const u8) u64
            const h = std.hash.Wyhash.hash(g.n, window[0..g.n]);
            const bucket: usize = @intCast(h % dim);
            out[bucket] += if ((h >> 63) & 1 == 0) g.weight else -g.weight;
        }
    }
    var norm: f32 = 0.0;
    for (out) |v| norm += v * v;
    if (norm == 0.0) {
        out[0] = 1.0;
        return out;
    }
    const scale = @sqrt(norm);
    for (&out) |*v| v.* /= scale;
    return out;
}

pub fn cosine(a: []const f32, b: []const f32) f32 {
    var dot: f32 = 0.0;
    for (a, b) |x, y| dot += x * y;
    var na: f32 = 0.0;
    for (a) |v| na += v * v;
    var nb: f32 = 0.0;
    for (b) |v| nb += v * v;
    na = @sqrt(na);
    nb = @sqrt(nb);
    if (na == 0.0 or nb == 0.0) return 0.0;
    return dot / (na * nb);
}

test "std Wyhash reproduces the pinned Zig reference vectors" {
    const refs = @embedFile("golden_wyhash_refs");
    var lines = std.mem.splitScalar(u8, refs, '\n');
    var checked: usize = 0;
    var buf: [512]u8 = undefined;
    for (&buf, 0..) |*b, i| b.* = @truncate(i * 31 + 7);
    while (lines.next()) |line| {
        if (line.len == 0 or line[0] == '#') continue;
        var fields = std.mem.tokenizeScalar(u8, line, ' ');
        const seed = try std.fmt.parseInt(u64, fields.next().?, 10);
        const len = try std.fmt.parseInt(usize, fields.next().?, 10);
        const want = try std.fmt.parseInt(u64, fields.next().?, 10);
        try std.testing.expectEqual(want, std.hash.Wyhash.hash(seed, buf[0..len]));
        checked += 1;
    }
    try std.testing.expect(checked >= 20);
}
