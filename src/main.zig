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
    \\  gateway-probe  Token-free live check: HTTPS /gateway, WSS Hello, close
    \\  wdbx-interop <abi>  Query a Zig-written fact segment through the real abi binary
    \\  version        Print the version
    \\  help           Show this help
    \\
;

const Command = enum { @"catalog-json", @"gateway-probe", @"wdbx-interop", version, help };

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
        .@"wdbx-interop" => {
            if (args.len < 3) {
                try out.writeAll("usage: abbey-bot-zig wdbx-interop <absolute path to abi>\n");
                return 2;
            }
            const tmp_root = init.environ_map.get("TMPDIR") orelse "/tmp";
            const report = abbey.wdbx_interop.run(init.gpa, io, args[2], tmp_root) catch |err| {
                try out.print("wdbx-interop: FAILED ({s})\n", .{@errorName(err)});
                return 1;
            };
            try out.print("wdbx-interop: ok ({d} vectors scored by abi, max |abi - zig| = {e:.3})\n", .{ report.compared, report.max_abs_diff });
        },
        .@"catalog-json" => try abbey.catalog_serialize.writePayload(out, abbey.catalog_serialize.frozen()),
        .@"gateway-probe" => {
            var client = abbey.http.Client.init(init.gpa, io);
            defer client.deinit();
            const report = abbey.gateway_probe.run(init.gpa, &client) catch |err| {
                try out.print("gateway-probe: FAILED ({s})\n", .{@errorName(err)});
                return 1;
            };
            try out.print("gateway-probe: ok (gateway url wss: {}, hello heartbeat_interval {d} ms)\n", .{ report.gateway_url_ok, report.heartbeat_interval_ms });
        },
    }
    return 0;
}

test "usage names every command" {
    inline for (@typeInfo(Command).@"enum".field_names) |field| {
        try std.testing.expect(std.mem.indexOf(u8, usage, "  " ++ field) != null);
    }
}
