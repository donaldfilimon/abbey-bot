//! Exact fixed-point float formatting with Rust's `{:.N}` semantics.
//!
//! Rust formats `{:.2}` from the exact binary value with ties to even. Zig's
//! `std.fmt` (std/fmt/float.zig `round`) rounds the shortest round-trip digits
//! half-up, a double rounding that differs on values such as 0.125 or 0.845.
//! This formatter computes round(value * 10^N) exactly with u128 arithmetic.
const std = @import("std");

pub const max_digits = 6;

/// Write `value` with exactly `digits` fractional digits (digits <= 6).
/// Magnitudes of 2^63 or more and non-finite values are outside every caller's
/// domain (weights and standings are in [0, 1]); they are written through
/// std.fmt and pinned as such by test rather than silently mis-rounded.
pub fn writeFixed(w: *std.Io.Writer, value: f64, comptime digits: u3) std.Io.Writer.Error!void {
    comptime std.debug.assert(digits <= max_digits);
    if (!std.math.isFinite(value) or @abs(value) >= 9.223372036854775808e18) {
        return w.print("{d}", .{value});
    }
    const bits: u64 = @bitCast(value);
    const negative = (bits >> 63) != 0;
    const exp_bits: u11 = @truncate(bits >> 52);
    const frac: u64 = bits & ((@as(u64, 1) << 52) - 1);
    var mantissa: u128 = frac;
    var exponent: i32 = undefined;
    if (exp_bits == 0) {
        exponent = -1074;
    } else {
        mantissa |= @as(u128, 1) << 52;
        exponent = @as(i32, exp_bits) - 1075;
    }
    const scale: u128 = comptime std.math.pow(u128, 10, digits);
    var scaled: u128 = undefined;
    if (exponent >= 0) {
        scaled = (mantissa << @intCast(exponent)) * scale;
    } else {
        const shift: u32 = @intCast(-exponent);
        const numerator = mantissa * scale;
        if (shift >= 127) {
            scaled = 0; // numerator < 2^73 < half of 2^shift: rounds to zero, never a tie
        } else {
            const s: u7 = @intCast(shift);
            const quotient = numerator >> s;
            const remainder = numerator & ((@as(u128, 1) << s) - 1);
            const half = @as(u128, 1) << (s - 1);
            scaled = quotient;
            if (remainder > half or (remainder == half and (quotient & 1) == 1)) scaled += 1;
        }
    }
    if (negative) try w.writeByte('-');
    const whole = scaled / scale;
    const fraction = scaled % scale;
    try w.print("{d}", .{whole});
    if (digits > 0) {
        try w.writeByte('.');
        var buf: [max_digits]u8 = undefined;
        var rest = fraction;
        var i: usize = digits;
        while (i > 0) {
            i -= 1;
            buf[i] = '0' + @as(u8, @intCast(rest % 10));
            rest /= 10;
        }
        try w.writeAll(buf[0..digits]);
    }
}

fn expectFixed(expected: []const u8, value: f64) !void {
    var buf: [64]u8 = undefined;
    var w: std.Io.Writer = .fixed(&buf);
    try writeFixed(&w, value, 2);
    try std.testing.expectEqualStrings(expected, w.buffered());
}

test "fixed two digits rounds the exact value half to even" {
    try expectFixed("0.12", 0.125); // exact tie, even
    try expectFixed("0.38", 0.375); // exact tie, even
    try expectFixed("0.50", 0.5);
    try expectFixed("0.12", 0.123456);
    try expectFixed("1.00", 0.999999);
    try expectFixed("0.00", 0.0);
    try expectFixed("-0.00", -0.0);
    try expectFixed("0.00", 5e-324);
    try expectFixed("12345.67", 12345.675); // binary value is below the tie (rustc 1.98.0 agrees)
}

test "f32 widened exactly keeps its own digits" {
    const x: f32 = 0.405405405;
    var buf: [64]u8 = undefined;
    var w: std.Io.Writer = .fixed(&buf);
    try writeFixed(&w, x, 2);
    try std.testing.expectEqualStrings("0.41", w.buffered());
}
