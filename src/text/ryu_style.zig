//! f32 text in the exact layout serde_json uses (the `ryu` crate's
//! `format32`, read from ~/.cargo/registry ryu-1.0.23 src/pretty/mod.rs):
//! shortest round-trip digits, integral values as `N.0`, `0.000123` down to
//! 1e-5, exponent notation outside [1e-5, 1e13). The shortest digits come from
//! std's own Ryu (std/fmt/float.zig `binaryToDecimal` with f32 bit widths).
const std = @import("std");
const float = std.fmt.float;
const Writer = std.Io.Writer;

pub fn writeF32(w: *Writer, value: f32) Writer.Error!void {
    const bits: u32 = @bitCast(value);
    if (bits >> 31 != 0) try w.writeByte('-');
    if (bits & 0x7FFF_FFFF == 0) return w.writeAll("0.0");
    if (!std.math.isFinite(value)) return w.writeAll(if (std.math.isNan(value)) "NaN" else "inf");
    const d = float.binaryToDecimal(u64, bits, 23, 8, false, &float.Backend64_TablesFull);
    var digits_buf: [20]u8 = undefined;
    const digits = std.fmt.bufPrint(&digits_buf, "{d}", .{d.mantissa}) catch unreachable;
    const length: i32 = @intCast(digits.len);
    const k: i32 = d.exponent;
    const kk = length + k;
    if (0 <= k and kk <= 13) {
        try w.writeAll(digits);
        var i: i32 = length;
        while (i < kk) : (i += 1) try w.writeByte('0');
        try w.writeAll(".0");
    } else if (0 < kk and kk <= 13) {
        const split: usize = @intCast(kk);
        try w.writeAll(digits[0..split]);
        try w.writeByte('.');
        try w.writeAll(digits[split..]);
    } else if (-6 < kk and kk <= 0) {
        try w.writeAll("0.");
        var i: i32 = 0;
        while (i < -kk) : (i += 1) try w.writeByte('0');
        try w.writeAll(digits);
    } else if (length == 1) {
        try w.print("{s}e{d}", .{ digits, kk - 1 });
    } else {
        try w.print("{c}.{s}e{d}", .{ digits[0], digits[1..], kk - 1 });
    }
}

fn expectF32(want: []const u8, v: f32) !void {
    var buf: [48]u8 = undefined;
    var w: Writer = .fixed(&buf);
    try writeF32(&w, v);
    try std.testing.expectEqualStrings(want, w.buffered());
}

test "f32 text matches serde_json's ryu layout" {
    try expectF32("0.5", 0.5);
    try expectF32("-0.25", -0.25);
    try expectF32("0.125", 0.125);
    try expectF32("-1.0", -1.0);
    try expectF32("0.0", 0.0);
    try expectF32("-0.0", -0.0);
    try expectF32("1.0", 1.0);
    try expectF32("0.36927447", 0.36927447);
    try expectF32("-0.09365858", -0.09365858);
    try expectF32("100.0", 100.0);
    try expectF32("0.00001", 0.00001);
    try expectF32("0.000001", 0.000001);
    try expectF32("1e-7", 0.0000001);
    try expectF32("1.5e-7", 0.00000015);
    try expectF32("1e13", 1e13);
    try expectF32("1234567.0", 1234567.0);
}
