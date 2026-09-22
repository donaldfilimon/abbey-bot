//! TLS verification against a loopback TLS server started by the test.
//!
//! Zig std has no TLS server (std/crypto/tls/ holds only Client.zig), so the
//! server is `openssl s_server`, with certificates generated per test run.
//! The client under test is the production path: std.http.Client with its CA
//! bundle holding only the test certificate, so chain verification, hostname
//! checks, and TLS 1.2 and 1.3 handshakes are all exercised. A missing
//! openssl binary fails these tests rather than skipping them.
const std = @import("std");
const http = @import("http.zig");
const testing = std.testing;

const Server = struct {
    child: std.process.Child,
    port: u16,

    fn stop(s: *Server) void {
        s.child.kill(testing.io);
    }
};

fn openssl(dir: std.Io.Dir, argv: []const []const u8) !void {
    var full: std.ArrayList([]const u8) = .empty;
    defer full.deinit(testing.allocator);
    try full.append(testing.allocator, "openssl");
    try full.appendSlice(testing.allocator, argv);
    const result = try std.process.run(testing.allocator, testing.io, .{ .argv = full.items, .cwd = .{ .dir = dir } });
    defer testing.allocator.free(result.stdout);
    defer testing.allocator.free(result.stderr);
    if (result.term != .exited or result.term.exited != 0) {
        std.debug.print("openssl {s} failed: {s}\n", .{ argv[0], result.stderr });
        return error.OpensslFailed;
    }
}

fn makeCert(dir: std.Io.Dir, key_alg: []const []const u8, cn: []const u8, prefix: []const u8) !void {
    var subj_buf: [64]u8 = undefined;
    var san_buf: [64]u8 = undefined;
    var key_buf: [32]u8 = undefined;
    var cert_buf: [32]u8 = undefined;
    var argv: std.ArrayList([]const u8) = .empty;
    defer argv.deinit(testing.allocator);
    try argv.appendSlice(testing.allocator, &.{ "req", "-x509", "-nodes", "-days", "2" });
    try argv.appendSlice(testing.allocator, key_alg);
    try argv.appendSlice(testing.allocator, &.{
        "-keyout", try std.fmt.bufPrint(&key_buf, "{s}-key.pem", .{prefix}),
        "-out",    try std.fmt.bufPrint(&cert_buf, "{s}-cert.pem", .{prefix}),
        "-subj",   try std.fmt.bufPrint(&subj_buf, "/CN={s}", .{cn}),
        "-addext", try std.fmt.bufPrint(&san_buf, "subjectAltName=DNS:{s}", .{cn}),
    });
    try openssl(dir, argv.items);
}

fn startServer(dir: std.Io.Dir, prefix: []const u8, version_flag: []const u8) !Server {
    var attempt: usize = 0;
    while (attempt < 8) : (attempt += 1) {
        var rnd: [2]u8 = undefined;
        testing.io.random(&rnd);
        const port: u16 = 40000 + (std.mem.readInt(u16, &rnd, .little) % 20000);
        var accept_buf: [32]u8 = undefined;
        var key_buf: [32]u8 = undefined;
        var cert_buf: [32]u8 = undefined;
        var child = try std.process.spawn(testing.io, .{
            .argv = &.{
                "openssl",                                                   "s_server",                                               "-accept",
                try std.fmt.bufPrint(&accept_buf, "127.0.0.1:{d}", .{port}), "-cert",                                                  try std.fmt.bufPrint(&cert_buf, "{s}-cert.pem", .{prefix}),
                "-key",                                                      try std.fmt.bufPrint(&key_buf, "{s}-key.pem", .{prefix}), "-www",
                "-naccept",                                                  "1",                                                      version_flag,
            },
            .cwd = .{ .dir = dir },
            .stdin = .ignore,
            .stdout = .pipe,
            .stderr = .ignore,
        });
        // s_server prints ACCEPT once it is listening (after a DH note).
        var buf: [256]u8 = undefined;
        var reader = child.stdout.?.reader(testing.io, &buf);
        var lines: usize = 0;
        while (lines < 4) : (lines += 1) {
            const line = reader.interface.takeDelimiterInclusive('\n') catch break;
            if (std.mem.indexOf(u8, line, "ACCEPT") != null) return .{ .child = child, .port = port };
        }
        child.kill(testing.io);
    }
    return error.NoFreePort;
}

fn clientTrusting(dir: std.Io.Dir, prefix: []const u8) !http.Client {
    var client = http.Client.init(testing.allocator, testing.io);
    errdefer client.deinit();
    const now = std.Io.Clock.real.now(testing.io);
    var cert_buf: [32]u8 = undefined;
    // std/crypto/Certificate/Bundle.zig: addCertsFromFilePath(gpa, io, now, dir, sub_path)
    try client.inner.ca_bundle.addCertsFromFilePath(testing.allocator, testing.io, now, dir, try std.fmt.bufPrint(&cert_buf, "{s}-cert.pem", .{prefix}));
    client.inner.now = now; // bundle is loaded; std/http/Client.zig request() skips rescan when now != null
    return client;
}

fn fetchStatusPage(dir: std.Io.Dir, prefix: []const u8, version_flag: []const u8, host: []const u8) !http.Response {
    var server = try startServer(dir, prefix, version_flag);
    defer server.stop();
    var client = try clientTrusting(dir, prefix);
    defer client.deinit();
    var url_buf: [64]u8 = undefined;
    return client.send(.{ .method = .GET, .url = try std.fmt.bufPrint(&url_buf, "https://{s}:{d}/", .{ host, server.port }) });
}

test "tls loopback: TLS 1.3 with an ECDSA P-256 certificate verifies and carries HTTP" {
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    try makeCert(tmp.dir, &.{ "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1" }, "localhost", "ec");
    var response = try fetchStatusPage(tmp.dir, "ec", "-tls1_3", "localhost");
    defer response.deinit(testing.allocator);
    try testing.expectEqual(@as(u16, 200), response.status);
    try testing.expect(std.mem.indexOf(u8, response.body, "TLSv1.3") != null);
}

test "tls loopback: TLS 1.2 with an RSA-2048 certificate verifies and carries HTTP" {
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    try makeCert(tmp.dir, &.{ "-newkey", "rsa:2048" }, "localhost", "rsa");
    var response = try fetchStatusPage(tmp.dir, "rsa", "-tls1_2", "localhost");
    defer response.deinit(testing.allocator);
    try testing.expectEqual(@as(u16, 200), response.status);
    try testing.expect(std.mem.indexOf(u8, response.body, "TLSv1.2") != null);
}

test "tls loopback: a certificate for another host is refused" {
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    try makeCert(tmp.dir, &.{ "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1" }, "not-localhost.example", "wrong");
    try testing.expectError(error.TransportFailed, fetchStatusPage(tmp.dir, "wrong", "-tls1_3", "localhost"));
}
