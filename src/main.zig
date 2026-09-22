//! abbey-bot-zig entry point: argument dispatch only. Behavior lives in the
//! library (`src/root.zig`); every subcommand is one call into it.
const std = @import("std");
const abbey = @import("abbey");

const usage =
    \\abbey-bot-zig - Abbey Discord bot (Zig rewrite, phase 1)
    \\
    \\USAGE:
    \\  abbey-bot-zig <command>
    \\
    \\COMMANDS:
    \\  catalog-json   Print the frozen slash-command registration payload
    \\  version        Print the version
    \\  help           Show this help
    \\
;

const Command = enum { @"catalog-json", version, help };

pub fn main(init: std.process.Init) !u8 {
    const arena = init.arena.allocator();
    const args = try init.minimal.args.toSlice(arena);
    const io = init.io;
    var buf: [64 * 1024]u8 = undefined;
    // std/Io/File.zig: File.stdout() and File.writer(io, buffer) -> File.Writer{ .interface }
    var stdout = std.Io.File.stdout().writer(io, &buf);
    const out = &stdout.interface;
    defer out.flush() catch {};

    const name = if (args.len >= 2) args[1] else "help";
    const command = std.meta.stringToEnum(Command, name) orelse {
        try out.print("unknown command: {s}\n\n{s}", .{ name, usage });
        return 2;
    };
    switch (command) {
        .help => try out.writeAll(usage),
        .version => try out.print("abbey-bot-zig {s}\n", .{abbey.version}),
        .@"catalog-json" => try abbey.catalog_serialize.writePayload(out, abbey.catalog_serialize.frozen()),
    }
    return 0;
}

test "usage names every command" {
    inline for (@typeInfo(Command).@"enum".field_names) |field| {
        try std.testing.expect(std.mem.indexOf(u8, usage, "  " ++ field) != null);
    }
}
