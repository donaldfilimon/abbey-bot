//! RFC 6455 WebSocket client framing and opening handshake over any
//! `std.Io.Reader`/`std.Io.Writer` pair (TLS or plain TCP, or in-memory in
//! tests). Zig 0.17-dev std ships no WebSocket client (std/http/ has
//! ChunkParser, Client, HeadParser, HeaderIterator, Server and no WebSocket),
//! so this is implemented here, stdlib-only.
//!
//! Client rules enforced (RFC 6455 section 5): every client frame is masked
//! with a fresh key; a masked server frame, a nonzero RSV bit, a fragmented or
//! oversized (>125) control frame, an unknown opcode, a continuation without a
//! start, and invalid UTF-8 in a text message are protocol errors.
const std = @import("std");
const Reader = std.Io.Reader;
const Writer = std.Io.Writer;
const Allocator = std.mem.Allocator;

pub const guid = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub const Opcode = enum(u4) {
    continuation = 0x0,
    text = 0x1,
    binary = 0x2,
    close = 0x8,
    ping = 0x9,
    pong = 0xA,
    _,

    pub fn isControl(op: Opcode) bool {
        return @backingInt(op) & 0x8 != 0;
    }
};

pub const Error = error{
    ProtocolError,
    MessageTooLarge,
    InvalidUtf8,
    HandshakeFailed,
    ConnectionClosed,
} || Reader.Error || Writer.Error || Allocator.Error;

pub const Header = struct {
    fin: bool,
    opcode: Opcode,
    masked: bool,
    len: u64,
    mask: [4]u8 = .{ 0, 0, 0, 0 },
};

/// Encode one frame. Client frames pass a mask key; server-side encoding (used
/// only by tests) passes null.
pub fn writeFrame(w: *Writer, fin: bool, opcode: Opcode, payload: []const u8, mask: ?[4]u8) Writer.Error!void {
    try w.writeByte(@as(u8, if (fin) 0x80 else 0) | @as(u8, @backingInt(opcode)));
    const mask_bit: u8 = if (mask != null) 0x80 else 0;
    if (payload.len <= 125) {
        try w.writeByte(mask_bit | @as(u8, @intCast(payload.len)));
    } else if (payload.len <= 0xFFFF) {
        try w.writeByte(mask_bit | 126);
        try w.writeInt(u16, @intCast(payload.len), .big);
    } else {
        try w.writeByte(mask_bit | 127);
        try w.writeInt(u64, payload.len, .big);
    }
    if (mask) |m| {
        try w.writeAll(&m);
        for (payload, 0..) |b, i| try w.writeByte(b ^ m[i % 4]);
    } else try w.writeAll(payload);
}

pub fn readHeader(r: *Reader) (Reader.Error || error{ProtocolError})!Header {
    const b0 = try r.takeByte();
    const b1 = try r.takeByte();
    if (b0 & 0x70 != 0) return error.ProtocolError; // RSV bits: no extension negotiated
    var h: Header = .{
        .fin = b0 & 0x80 != 0,
        .opcode = @fromBackingInt(@intCast(@as(u4, @truncate(b0)))),
        .masked = b1 & 0x80 != 0,
        .len = b1 & 0x7F,
    };
    if (h.len == 126) {
        h.len = try r.takeVarInt(u16, .big, 2);
        if (h.len < 126) return error.ProtocolError; // non-minimal length
    } else if (h.len == 127) {
        h.len = try r.takeVarInt(u64, .big, 8);
        if (h.len >> 63 != 0 or h.len <= 0xFFFF) return error.ProtocolError;
    }
    if (h.masked) h.mask = (try r.takeArray(4)).*;
    switch (h.opcode) {
        .continuation, .text, .binary, .close, .ping, .pong => {},
        _ => return error.ProtocolError,
    }
    if (h.opcode.isControl() and (!h.fin or h.len > 125)) return error.ProtocolError;
    return h;
}

/// The Sec-WebSocket-Accept value for a key (RFC 6455 section 4.2.2).
pub fn acceptKey(key_b64: []const u8, out: *[28]u8) []const u8 {
    var sha = std.crypto.hash.Sha1.init(.{});
    sha.update(key_b64);
    sha.update(guid);
    var digest: [20]u8 = undefined;
    sha.final(&digest);
    return std.base64.standard.Encoder.encode(out, &digest);
}

/// Flushes every layer under the frame writer (for TLS: the TLS writer and
/// then the socket writer). A bare `Writer.flush` on a TLS writer only moves
/// ciphertext into the socket buffer.
pub const Flusher = struct {
    ctx: *anyopaque,
    func: *const fn (ctx: *anyopaque) Writer.Error!void,

    pub fn call(f: Flusher) Writer.Error!void {
        return f.func(f.ctx);
    }

    fn writerOnly(ctx: *anyopaque) Writer.Error!void {
        const w: *Writer = @ptrCast(@alignCast(ctx));
        return w.flush();
    }

    pub fn of(w: *Writer) Flusher {
        return .{ .ctx = w, .func = writerOnly };
    }
};

/// Send the opening handshake and validate the server's 101 response.
/// `key_raw` is 16 random bytes. Response headers beyond the checked ones are
/// ignored; the head must fit in the reader's buffer.
pub fn handshake(r: *Reader, w: *Writer, flusher: Flusher, host: []const u8, path: []const u8, key_raw: [16]u8) Error!void {
    var key_buf: [24]u8 = undefined;
    const key = std.base64.standard.Encoder.encode(&key_buf, &key_raw);
    try w.print("GET {s} HTTP/1.1\r\nHost: {s}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {s}\r\nSec-WebSocket-Version: 13\r\nUser-Agent: DiscordBot (abbey-bot-zig, 0.1.0)\r\n\r\n", .{ path, host, key });
    try flusher.call();

    var accept_buf: [28]u8 = undefined;
    const want_accept = acceptKey(key, &accept_buf);
    const status_line = r.takeDelimiterInclusive('\n') catch |e| return mapDelimiter(e);
    const trimmed = std.mem.trimEnd(u8, status_line, "\r\n");
    if (!std.mem.startsWith(u8, trimmed, "HTTP/1.1 101")) return error.HandshakeFailed;
    var saw_upgrade = false;
    var saw_connection = false;
    var saw_accept = false;
    while (true) {
        const raw = r.takeDelimiterInclusive('\n') catch |e| return mapDelimiter(e);
        const line = std.mem.trimEnd(u8, raw, "\r\n");
        if (line.len == 0) break;
        const colon = std.mem.indexOfScalar(u8, line, ':') orelse return error.HandshakeFailed;
        const name = std.mem.trim(u8, line[0..colon], " \t");
        const value = std.mem.trim(u8, line[colon + 1 ..], " \t");
        if (std.ascii.eqlIgnoreCase(name, "upgrade")) {
            saw_upgrade = std.ascii.eqlIgnoreCase(value, "websocket");
        } else if (std.ascii.eqlIgnoreCase(name, "connection")) {
            saw_connection = containsTokenIgnoreCase(value, "upgrade");
        } else if (std.ascii.eqlIgnoreCase(name, "sec-websocket-accept")) {
            saw_accept = std.mem.eql(u8, value, want_accept);
        } else if (std.ascii.eqlIgnoreCase(name, "sec-websocket-extensions")) {
            return error.HandshakeFailed; // none requested, none accepted
        }
    }
    if (!(saw_upgrade and saw_connection and saw_accept)) return error.HandshakeFailed;
}

fn mapDelimiter(e: Reader.DelimiterError) Error {
    return switch (e) {
        error.ReadFailed => error.ReadFailed,
        error.EndOfStream => error.EndOfStream,
        error.StreamTooLong => error.HandshakeFailed,
    };
}

fn containsTokenIgnoreCase(list: []const u8, token: []const u8) bool {
    var it = std.mem.splitScalar(u8, list, ',');
    while (it.next()) |part| {
        if (std.ascii.eqlIgnoreCase(std.mem.trim(u8, part, " \t"), token)) return true;
    }
    return false;
}

pub const Message = union(enum) {
    text: []const u8,
    binary: []const u8,
    /// The peer's close frame. `code` is null for an empty close body.
    close: struct { code: ?u16, reason: []const u8 },
};

/// A connected client. Owns a message buffer; `Message` slices borrow it and
/// are valid until the next `readMessage`.
pub const Client = struct {
    gpa: Allocator,
    io: std.Io,
    reader: *Reader,
    writer: *Writer,
    flusher: Flusher,
    max_message: usize,
    message: std.ArrayList(u8) = .empty,
    close_sent: bool = false,

    pub fn init(gpa: Allocator, io: std.Io, reader: *Reader, writer: *Writer, flusher: Flusher, max_message: usize) Client {
        return .{ .gpa = gpa, .io = io, .reader = reader, .writer = writer, .flusher = flusher, .max_message = max_message };
    }

    pub fn deinit(c: *Client) void {
        c.message.deinit(c.gpa);
    }

    fn maskKey(c: *Client) [4]u8 {
        var m: [4]u8 = undefined;
        // std/Io.zig: random(io, buffer)
        c.io.random(&m);
        return m;
    }

    pub fn send(c: *Client, opcode: Opcode, payload: []const u8) Error!void {
        try writeFrame(c.writer, true, opcode, payload, c.maskKey());
        try c.flusher.call();
    }

    pub fn sendText(c: *Client, payload: []const u8) Error!void {
        return c.send(.text, payload);
    }

    /// Send a close frame once. Later calls are no-ops.
    pub fn sendClose(c: *Client, code: u16, reason: []const u8) Error!void {
        if (c.close_sent) return;
        c.close_sent = true;
        var body: [125]u8 = undefined;
        std.mem.writeInt(u16, body[0..2], code, .big);
        const n = @min(reason.len, body.len - 2);
        @memcpy(body[2..][0..n], reason[0..n]);
        try c.send(.close, body[0 .. 2 + n]);
    }

    fn readPayload(c: *Client, h: Header, dest: *std.ArrayList(u8)) Error!void {
        if (h.masked) return error.ProtocolError; // servers never mask (RFC 6455 5.1)
        if (dest.items.len + h.len > c.max_message) return error.MessageTooLarge;
        const start = dest.items.len;
        try dest.resize(c.gpa, start + @as(usize, @intCast(h.len)));
        try c.reader.readSliceAll(dest.items[start..]);
    }

    /// Read the next data message or close. Pings are answered with pongs and
    /// pongs are consumed transparently.
    pub fn readMessage(c: *Client) Error!Message {
        c.message.clearRetainingCapacity();
        var kind: ?Opcode = null;
        var control: std.ArrayList(u8) = .empty;
        defer control.deinit(c.gpa);
        while (true) {
            const h = try readHeader(c.reader);
            if (h.opcode.isControl()) {
                control.clearRetainingCapacity();
                try c.readPayload(h, &control);
                switch (h.opcode) {
                    .ping => try c.send(.pong, control.items),
                    .pong => {},
                    .close => {
                        const body = control.items;
                        if (body.len == 1) return error.ProtocolError;
                        const code: ?u16 = if (body.len >= 2) std.mem.readInt(u16, body[0..2], .big) else null;
                        const reason = if (body.len > 2) body[2..] else "";
                        if (!std.unicode.utf8ValidateSlice(reason)) return error.InvalidUtf8;
                        try c.message.appendSlice(c.gpa, reason);
                        // Echo the close (RFC 6455 5.5.1) unless we initiated it.
                        c.sendClose(code orelse 1000, "") catch {};
                        return .{ .close = .{ .code = code, .reason = c.message.items } };
                    },
                    else => unreachable,
                }
                continue;
            }
            switch (h.opcode) {
                .text, .binary => {
                    if (kind != null) return error.ProtocolError; // new message inside a fragmented one
                    kind = h.opcode;
                },
                .continuation => if (kind == null) return error.ProtocolError,
                else => unreachable,
            }
            try c.readPayload(h, &c.message);
            if (!h.fin) continue;
            if (kind.? == .text) {
                if (!std.unicode.utf8ValidateSlice(c.message.items)) return error.InvalidUtf8;
                return .{ .text = c.message.items };
            }
            return .{ .binary = c.message.items };
        }
    }
};

// ---------------------------------------------------------------------------
// Tests: RFC 6455 section 5.7 examples and section 1.3 handshake example.
// ---------------------------------------------------------------------------

const testing = std.testing;

test "RFC 6455 1.3: the sample key yields the sample accept value" {
    var out: [28]u8 = undefined;
    try testing.expectEqualStrings("s3pPLMBiTxaQ9kYGzzhZRbK+xOo=", acceptKey("dGhlIHNhbXBsZSBub25jZQ==", &out));
}

test "RFC 6455 5.7: single-frame masked and unmasked text encode to the sample bytes" {
    var buf: [64]u8 = undefined;
    var w: Writer = .fixed(&buf);
    try writeFrame(&w, true, .text, "Hello", null);
    try testing.expectEqualSlices(u8, &.{ 0x81, 0x05, 0x48, 0x65, 0x6c, 0x6c, 0x6f }, w.buffered());
    w = .fixed(&buf);
    try writeFrame(&w, true, .text, "Hello", .{ 0x37, 0xfa, 0x21, 0x3d });
    try testing.expectEqualSlices(u8, &.{ 0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58 }, w.buffered());
    w = .fixed(&buf);
    try writeFrame(&w, true, .pong, "Hello", .{ 0x37, 0xfa, 0x21, 0x3d });
    try testing.expectEqualSlices(u8, &.{ 0x8a, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58 }, w.buffered());
}

test "RFC 6455 5.7: 256-byte and 64 KiB binary frames use 16- and 64-bit lengths" {
    var payload: [65536]u8 = @splat(0xAB);
    var big: [65536 + 16]u8 = undefined;
    var bw: Writer = .fixed(&big);
    try writeFrame(&bw, true, .binary, payload[0..256], null);
    try testing.expectEqualSlices(u8, &.{ 0x82, 0x7E, 0x01, 0x00 }, bw.buffered()[0..4]);
    bw = .fixed(&big);
    try writeFrame(&bw, true, .binary, &payload, null);
    try testing.expectEqualSlices(u8, &.{ 0x82, 0x7F, 0, 0, 0, 0, 0, 1, 0, 0 }, bw.buffered()[0..10]);
}

/// Serves `server_bytes` to a client and captures what it writes.
const Harness = struct {
    in: Reader,
    out_buf: [4096]u8 = undefined,
    out: Writer = undefined,

    fn init(h: *Harness, server_bytes: []const u8) Client {
        h.in = .fixed(server_bytes);
        h.out = .fixed(&h.out_buf);
        return Client.init(testing.allocator, testing.io, &h.in, &h.out, .of(&h.out), 1 << 20);
    }
};

test "RFC 6455 5.7: a fragmented unmasked text message reassembles to Hello" {
    var h: Harness = undefined;
    var c = h.init(&.{ 0x01, 0x03, 0x48, 0x65, 0x6c, 0x80, 0x02, 0x6c, 0x6f });
    defer c.deinit();
    const m = try c.readMessage();
    try testing.expectEqualStrings("Hello", m.text);
}

test "a ping between fragments is answered with a masked pong carrying the same payload" {
    var h: Harness = undefined;
    var c = h.init(&.{ 0x01, 0x03, 0x48, 0x65, 0x6c, 0x89, 0x05, 0x48, 0x65, 0x6c, 0x6c, 0x6f, 0x80, 0x02, 0x6c, 0x6f });
    defer c.deinit();
    const m = try c.readMessage();
    try testing.expectEqualStrings("Hello", m.text);
    const sent = h.out.buffered();
    try testing.expectEqual(@as(u8, 0x8a), sent[0]);
    try testing.expectEqual(@as(u8, 0x85), sent[1]);
    const mask = sent[2..6];
    var unmasked: [5]u8 = undefined;
    for (sent[6..11], 0..) |b, i| unmasked[i] = b ^ mask[i % 4];
    try testing.expectEqualStrings("Hello", &unmasked);
}

test "server close frames surface their code and are echoed once" {
    var h: Harness = undefined;
    var c = h.init(&.{ 0x88, 0x06, 0x0F, 0xA4, 'b', 'y', 'e', '!' }); // 4004 "bye!"
    defer c.deinit();
    const m = try c.readMessage();
    try testing.expectEqual(@as(?u16, 4004), m.close.code);
    try testing.expectEqualStrings("bye!", m.close.reason);
    try testing.expectEqual(@as(u8, 0x88), h.out.buffered()[0]);
    try c.sendClose(1000, "");
    try testing.expectEqual(@as(usize, 8), h.out.buffered().len); // header 2 + mask 4 + code 2, sent once
}

test "protocol violations are rejected" {
    const cases = [_][]const u8{
        &.{ 0x81, 0x85, 1, 2, 3, 4, 'a', 'b', 'c', 'd', 'e' }, // masked server frame
        &.{ 0xC1, 0x01, 'a' }, // RSV1 set
        &.{ 0x83, 0x00 }, // reserved opcode
        &.{ 0x09, 0x00 }, // fragmented control frame
        &.{ 0x80, 0x01, 'a' }, // continuation without a start
        &.{ 0x01, 0x01, 'a', 0x81, 0x01, 'b' }, // new message inside a fragmented one
        &.{ 0x81, 0x7E, 0x00, 0x05, 'a', 'b', 'c', 'd', 'e' }, // non-minimal 16-bit length
        &.{ 0x88, 0x01, 0x03 }, // one-byte close body
    };
    for (cases) |bytes| {
        var h: Harness = undefined;
        var c = h.init(bytes);
        defer c.deinit();
        try testing.expectError(error.ProtocolError, c.readMessage());
    }
    var h: Harness = undefined;
    var c = h.init(&.{ 0x81, 0x02, 0xC3, 0x28 }); // invalid UTF-8
    defer c.deinit();
    try testing.expectError(error.InvalidUtf8, c.readMessage());
}

test "messages over the configured maximum are refused before allocation" {
    var in: Reader = .fixed(&.{ 0x82, 0x7F, 0, 0, 0, 0, 0x7F, 0xFF, 0xFF, 0xFF });
    var out_buf: [64]u8 = undefined;
    var out: Writer = .fixed(&out_buf);
    var c = Client.init(testing.allocator, testing.io, &in, &out, .of(&out), 1024);
    defer c.deinit();
    try testing.expectError(error.MessageTooLarge, c.readMessage());
}

test "handshake sends the upgrade request and validates the 101 response" {
    const key_raw = [16]u8{ 't', 'h', 'e', ' ', 's', 'a', 'm', 'p', 'l', 'e', ' ', 'n', 'o', 'n', 'c', 'e' };
    const good = "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n";
    var in: Reader = .fixed(good);
    var out_buf: [512]u8 = undefined;
    var out: Writer = .fixed(&out_buf);
    try handshake(&in, &out, .of(&out), "gateway.discord.gg", "/?v=10&encoding=json", key_raw);
    try testing.expect(std.mem.indexOf(u8, out.buffered(), "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n") != null);
    try testing.expect(std.mem.startsWith(u8, out.buffered(), "GET /?v=10&encoding=json HTTP/1.1\r\nHost: gateway.discord.gg\r\n"));
    const bad = [_][]const u8{
        "HTTP/1.1 200 OK\r\n\r\n",
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: wrong\r\n\r\n",
        "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n",
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\nSec-WebSocket-Extensions: permessage-deflate\r\n\r\n",
    };
    for (bad) |response| {
        var bin: Reader = .fixed(response);
        var bout: Writer = .fixed(&out_buf);
        try testing.expectError(error.HandshakeFailed, handshake(&bin, &bout, .of(&bout), "h", "/", key_raw));
    }
}
