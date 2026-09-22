//! abbey-bot-zig entry point. Argument dispatch only; behavior lives in the
//! library (`src/root.zig`).
const std = @import("std");
const abbey = @import("abbey");

pub fn main(init: std.process.Init) !void {
    const arena = init.arena.allocator();
    const args = try init.minimal.args.toSlice(arena);
    _ = args;
    std.debug.print("abbey-bot-zig {s}\n", .{abbey.version});
}
