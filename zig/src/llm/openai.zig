//! OpenAI-compatible chat completions over std.http.Client, local first.
//!
//! Primary: `ABBEY_BOT_LLM_ENDPOINT` + `ABBEY_BOT_LLM_MODEL` (default
//! gemma4:12b), loopback only, exactly as the oracle's
//! `validate_remote_endpoint` requires. The rewrite scope adds a second tier,
//! `ABBEY_BOT_LLM_FALLBACK_ENDPOINT` / `_MODEL` / `_KEY`: any host, HTTPS
//! required unless loopback, used only when the primary is unset or its call
//! failed at the transport or HTTP level. Phase 1 turns are read-only (no
//! tools), so a retry on the second tier can never repeat a side effect.
//!
//! Request and extraction rules transcribe the oracle's `llm/dialect.rs` and
//! `llm/protocol.rs`: POST `<base>/v1/chat/completions`, `max_tokens` 4096,
//! system message first; a terminal `finish_reason` of `stop` with no tool
//! calls is required; an empty answer whose `reasoning` is non-empty is a
//! response-budget failure. Provider error text never reaches a user.
const std = @import("std");
const http = @import("../net/http.zig");
const engine = @import("../engine/engine.zig");
const prompts = @import("../persona/prompts.zig");
const Allocator = std.mem.Allocator;

pub const default_local_model = "gemma4:12b";
pub const local_max_tokens = 4096;
pub const primary_label = "configured OpenAI-compatible endpoint";
pub const fallback_label = "configured fallback OpenAI-compatible endpoint";

pub const EndpointError = error{
    InvalidUrl,
    CredentialsInUrl,
    QueryOrFragment,
    NotLoopback,
    InsecureRemote,
    UnsupportedScheme,
};

pub fn endpointErrorMessage(name: []const u8, e: EndpointError, buf: []u8) []const u8 {
    const what = switch (e) {
        error.InvalidUrl => "must be a valid absolute URL",
        error.CredentialsInUrl => "must not contain credentials",
        error.QueryOrFragment => "must not contain a query or fragment",
        error.NotLoopback => "must target loopback (127.0.0.1 / localhost / ::1); remote hosts are refused",
        error.InsecureRemote => "must use HTTPS for a non-loopback host",
        error.UnsupportedScheme => "must use HTTP or HTTPS",
    };
    return std.fmt.bufPrint(buf, "{s} {s}", .{ name, what }) catch what;
}

pub fn isLoopbackHost(host: []const u8) bool {
    const h = std.mem.trim(u8, host, "[]");
    if (std.ascii.eqlIgnoreCase(h, "localhost") or std.mem.eql(u8, h, "::1")) return true;
    // Any 127.0.0.0/8 address is loopback (Rust IpAddr::is_loopback).
    const ip4 = std.Io.net.Ip4Address.parse(h, 0) catch return false;
    return ip4.bytes[0] == 127;
}

pub const Tier = enum { primary, fallback };

/// Validate a base URL. The primary tier must be loopback; the fallback tier
/// may be remote but then must be HTTPS.
pub fn validateEndpoint(url: []const u8, tier: Tier) EndpointError!void {
    const uri = std.Uri.parse(url) catch return error.InvalidUrl;
    if (uri.user != null or uri.password != null) return error.CredentialsInUrl;
    if (uri.query != null or uri.fragment != null) return error.QueryOrFragment;
    const host_component = uri.host orelse return error.InvalidUrl;
    var host_buf: [256]u8 = undefined;
    const host = host_component.toRaw(&host_buf) catch return error.InvalidUrl;
    const https = std.ascii.eqlIgnoreCase(uri.scheme, "https");
    const plain = std.ascii.eqlIgnoreCase(uri.scheme, "http");
    if (!https and !plain) return error.UnsupportedScheme;
    const is_loopback = isLoopbackHost(host);
    switch (tier) {
        .primary => if (!is_loopback) return error.NotLoopback,
        .fallback => if (!is_loopback and !https) return error.InsecureRemote,
    }
}

pub const Endpoint = struct {
    base: []const u8,
    model: []const u8,
    /// Bearer key for the fallback tier only.
    key: ?[]const u8 = null,
    label: []const u8,
};

pub fn writeBody(w: *std.Io.Writer, model: []const u8, system_prompt: []const u8, turns: []const engine.ChatTurn) std.Io.Writer.Error!void {
    var s: std.json.Stringify = .{ .writer = w };
    try s.beginObject();
    try s.objectField("model");
    try s.write(model);
    try s.objectField("max_tokens");
    try s.write(@as(u32, local_max_tokens));
    try s.objectField("messages");
    try s.beginArray();
    try s.write(.{ .role = "system", .content = system_prompt });
    for (turns) |t| try s.write(.{ .role = @tagName(t.role), .content = t.text });
    try s.endArray();
    try s.endObject();
}

pub const Outcome = union(enum) {
    /// Owned answer text.
    answer: []u8,
    failure: prompts.FailureKind,
};

/// Extract the answer from a completions body (oracle `extract_text`).
pub fn extractText(gpa: Allocator, raw: []const u8) Allocator.Error!Outcome {
    const Parsed = std.json.Parsed(std.json.Value);
    var parsed: Parsed = std.json.parseFromSlice(std.json.Value, gpa, raw, .{}) catch |e| switch (e) {
        error.OutOfMemory => return error.OutOfMemory,
        else => return .{ .failure = .backend },
    };
    defer parsed.deinit();
    const choice = firstChoice(parsed.value) orelse return .{ .failure = .backend };
    const message = objectField(choice, "message") orelse return .{ .failure = .backend };
    if (objectField(message, "tool_calls")) |calls| switch (calls) {
        .null => {},
        .array => |a| if (a.items.len > 0) return .{ .failure = .backend }, // unrequested tool calls
        else => return .{ .failure = .backend },
    };
    const finish = objectField(choice, "finish_reason") orelse return .{ .failure = .backend };
    if (finish != .string or !std.mem.eql(u8, finish.string, "stop")) return .{ .failure = .backend };
    const content: []const u8 = if (objectField(message, "content")) |c| (if (c == .string) c.string else "") else "";
    if (std.mem.trim(u8, content, " \t\r\n").len > 0) return .{ .answer = try gpa.dupe(u8, content) };
    if (objectField(message, "reasoning")) |r| {
        if (r == .string and r.string.len > 0) return .{ .failure = .response_budget };
    }
    return .{ .failure = .backend };
}

fn objectField(v: std.json.Value, name: []const u8) ?std.json.Value {
    return switch (v) {
        .object => |o| o.get(name),
        else => null,
    };
}

fn firstChoice(v: std.json.Value) ?std.json.Value {
    const choices = objectField(v, "choices") orelse return null;
    if (choices != .array or choices.array.items.len == 0) return null;
    return choices.array.items[0];
}

pub const Generated = struct {
    outcome: Outcome,
    /// Which tier produced the outcome (for the honest "answered via" label).
    label: []const u8,
};

pub const Provider = struct {
    primary: ?Endpoint = null,
    fallback: ?Endpoint = null,

    pub fn configured(p: Provider) bool {
        return p.primary != null or p.fallback != null;
    }

    /// The label the degraded/failed copy names before a call is made.
    pub fn label(p: Provider) ?[]const u8 {
        if (p.primary) |e| return e.label;
        if (p.fallback) |e| return e.label;
        return null;
    }

    fn call(gpa: Allocator, client: *http.Client, e: Endpoint, system_prompt: []const u8, turns: []const engine.ChatTurn) !Outcome {
        var body: std.Io.Writer.Allocating = .init(gpa);
        defer body.deinit();
        writeBody(&body.writer, e.model, system_prompt, turns) catch return error.OutOfMemory;
        const url = try std.fmt.allocPrint(gpa, "{s}/v1/chat/completions", .{std.mem.trimEnd(u8, e.base, "/")});
        defer gpa.free(url);
        var auth_buf: [512]u8 = undefined;
        const auth: ?[]const u8 = if (e.key) |k| std.fmt.bufPrint(&auth_buf, "Bearer {s}", .{k}) catch return error.KeyTooLong else null;
        var response = try client.send(.{ .method = .POST, .url = url, .authorization = auth, .content_type = "application/json", .body = body.written() });
        defer response.deinit();
        if (response.status < 200 or response.status >= 300) return error.HttpStatus;
        return extractText(gpa, response.body);
    }

    /// Local first; the fallback tier is tried only after a transport or HTTP
    /// failure (never after a well-formed answer or budget failure).
    pub fn generate(p: Provider, gpa: Allocator, client: *http.Client, system_prompt: []const u8, turns: []const engine.ChatTurn) Allocator.Error!?Generated {
        if (p.primary) |e| {
            if (call(gpa, client, e, system_prompt, turns)) |outcome| {
                return .{ .outcome = outcome, .label = e.label };
            } else |err| switch (err) {
                error.OutOfMemory => return error.OutOfMemory,
                else => if (p.fallback == null) return .{ .outcome = .{ .failure = .backend }, .label = e.label },
            }
        }
        if (p.fallback) |e| {
            if (call(gpa, client, e, system_prompt, turns)) |outcome| {
                return .{ .outcome = outcome, .label = e.label };
            } else |err| switch (err) {
                error.OutOfMemory => return error.OutOfMemory,
                else => return .{ .outcome = .{ .failure = .backend }, .label = e.label },
            }
        }
        return null;
    }
};

const testing = std.testing;
const loopback = @import("../testing/loopback.zig");

test "endpoint validation mirrors the oracle: loopback-only primary, HTTPS-only remote fallback" {
    try validateEndpoint("http://127.0.0.1:11434", .primary);
    try validateEndpoint("http://localhost:8080/", .primary);
    try validateEndpoint("https://[::1]:8443", .primary);
    try validateEndpoint("http://127.1.2.3:1", .primary);
    try testing.expectError(error.NotLoopback, validateEndpoint("http://10.0.0.1:11434", .primary));
    try testing.expectError(error.NotLoopback, validateEndpoint("https://api.example.com", .primary));
    try testing.expectError(error.CredentialsInUrl, validateEndpoint("http://u:p@127.0.0.1:1", .primary));
    try testing.expectError(error.QueryOrFragment, validateEndpoint("http://127.0.0.1:1/?x=1", .primary));
    try testing.expectError(error.UnsupportedScheme, validateEndpoint("ftp://127.0.0.1", .primary));
    try validateEndpoint("https://api.example.com/v1", .fallback);
    try testing.expectError(error.InsecureRemote, validateEndpoint("http://api.example.com", .fallback));
}

test "extraction: stop with content answers; reasoning-only is a budget failure; others are backend failures" {
    const gpa = testing.allocator;
    const ok = try extractText(gpa, "{\"choices\":[{\"message\":{\"role\":\"assistant\",\"content\":\"Blue.\"},\"finish_reason\":\"stop\"}]}");
    defer gpa.free(ok.answer);
    try testing.expectEqualStrings("Blue.", ok.answer);
    const budget = try extractText(gpa, "{\"choices\":[{\"message\":{\"content\":\"\",\"reasoning\":\"thinking...\"},\"finish_reason\":\"stop\"}]}");
    try testing.expectEqual(Outcome{ .failure = .response_budget }, budget);
    const cases = [_][]const u8{
        "not json",
        "{\"choices\":[]}",
        "{\"choices\":[{\"message\":{\"content\":\"x\"},\"finish_reason\":\"length\"}]}",
        "{\"choices\":[{\"message\":{\"content\":\"x\"}}]}",
        "{\"choices\":[{\"message\":{\"content\":\"x\",\"tool_calls\":[{\"id\":\"1\"}]},\"finish_reason\":\"stop\"}]}",
        "{\"choices\":[{\"message\":{\"content\":\"   \"},\"finish_reason\":\"stop\"}]}",
    };
    for (cases) |raw| try testing.expectEqual(Outcome{ .failure = .backend }, try extractText(gpa, raw));
}

test "provider posts the persona prompt and transcript to the local endpoint and returns its answer" {
    var server: loopback.Server = undefined;
    try server.start(testing.allocator, testing.io, &.{
        .{ .body = "{\"choices\":[{\"message\":{\"role\":\"assistant\",\"content\":\"Rayleigh scattering.\"},\"finish_reason\":\"stop\"}]}" },
    });
    defer server.deinit();
    var client = http.Client.init(testing.allocator, testing.io);
    defer client.deinit();
    var base_buf: [64]u8 = undefined;
    const base = try std.fmt.bufPrint(&base_buf, "http://127.0.0.1:{d}", .{server.port});
    try validateEndpoint(base, .primary);
    const provider: Provider = .{ .primary = .{ .base = base, .model = default_local_model, .label = primary_label } };
    const turns = [_]engine.ChatTurn{ .{ .role = .user, .text = "q1" }, .{ .role = .assistant, .text = "a1" }, .{ .role = .user, .text = "why is the sky blue?" } };
    const got = (try provider.generate(testing.allocator, &client, "You are Abbey.", &turns)).?;
    defer testing.allocator.free(got.outcome.answer);
    server.join();
    try testing.expectEqualStrings("Rayleigh scattering.", got.outcome.answer);
    try testing.expectEqualStrings(primary_label, got.label);
    const seen = server.seen.items[0];
    try testing.expectEqualStrings("/v1/chat/completions", seen.target);
    try testing.expect(seen.authorization == null);
    try testing.expectEqualStrings("{\"model\":\"gemma4:12b\",\"max_tokens\":4096,\"messages\":[{\"role\":\"system\",\"content\":\"You are Abbey.\"},{\"role\":\"user\",\"content\":\"q1\"},{\"role\":\"assistant\",\"content\":\"a1\"},{\"role\":\"user\",\"content\":\"why is the sky blue?\"}]}", seen.body);
}

test "a failed local endpoint falls through to the fallback tier with its bearer key" {
    var server: loopback.Server = undefined;
    try server.start(testing.allocator, testing.io, &.{
        .{ .status = .internal_server_error, .body = "provider-internal detail" },
        .{ .body = "{\"choices\":[{\"message\":{\"content\":\"from fallback\"},\"finish_reason\":\"stop\"}]}" },
    });
    defer server.deinit();
    var client = http.Client.init(testing.allocator, testing.io);
    defer client.deinit();
    var base_buf: [64]u8 = undefined;
    const base = try std.fmt.bufPrint(&base_buf, "http://127.0.0.1:{d}", .{server.port});
    const provider: Provider = .{
        .primary = .{ .base = base, .model = "m1", .label = primary_label },
        .fallback = .{ .base = base, .model = "m2", .key = "sk-test", .label = fallback_label },
    };
    const got = (try provider.generate(testing.allocator, &client, "sys", &.{.{ .role = .user, .text = "hi" }})).?;
    defer testing.allocator.free(got.outcome.answer);
    server.join();
    try testing.expectEqualStrings("from fallback", got.outcome.answer);
    try testing.expectEqualStrings(fallback_label, got.label);
    try testing.expectEqualStrings("Bearer sk-test", server.seen.items[1].authorization.?);
    try testing.expect(server.seen.items[0].authorization == null);
}

test "no configured tier yields null and a lone failing tier yields a backend failure" {
    var client = http.Client.init(testing.allocator, testing.io);
    defer client.deinit();
    try testing.expect((try (Provider{}).generate(testing.allocator, &client, "s", &.{})) == null);
    // Port 1 on loopback refuses connections.
    const provider: Provider = .{ .primary = .{ .base = "http://127.0.0.1:1", .model = "m", .label = primary_label } };
    const got = (try provider.generate(testing.allocator, &client, "s", &.{.{ .role = .user, .text = "hi" }})).?;
    try testing.expectEqual(Outcome{ .failure = .backend }, got.outcome);
}
