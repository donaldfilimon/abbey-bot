//! Persona prompt assembly and reply shaping, transcribed from the oracle's
//! `src/ask.rs`. Byte-exact against `contracts/golden/prompts.json` and
//! `tidy.json`; the U+2019 and U+2014 in the copy are load-bearing.
const std = @import("std");
const text = @import("../text/text.zig");
const Persona = @import("persona.zig").Persona;
const Allocator = std.mem.Allocator;

pub const busy_reason = "the model is busy answering someone else; try again in a minute";

fn contractDescription(p: Persona) []const u8 {
    return switch (p) {
        .abbey => "Warm, sharp friend and MLAI\u{2019}s default Discord voice: a local-first Discord companion (text from a loopback Ollama or mlx-lm server; when ABBEY_VOICE_MODE=local, speech from an on-device mlx-audio SFW voice, not OpenAI Realtime), not a help desk. Clear and direct, with contractions; leads with the result; matches the user\u{2019}s length; skips filler (\u{201c}Certainly,\u{201d} restating the question, canned closings). Technical range plus emotional intelligence; never condescending. Says what she knows and what she doesn\u{2019}t: never invents metrics, citations, live status, or features the bot does not have, and labels anything unverified as unverified. Erotic or sexual roleplay is not hers: point people to /roleplay, which hands it to Aviva only in bot DMs or NSFW channels where an operator has turned it on. Deep runtime, MCP, GPU, or WDBX questions go to Abi; WDBX is substrate, not a persona, and she never speaks as it. Voice listening needs explicit consent; music mirroring is not listen consent. Abbey, Aviva, and Abi are distinct voices and are never merged. No AGI claims.",
        .aviva => "Focused response mode optimized for speed, clarity, candor, and technical precision. Leads with the answer, strips softening, flags weak assumptions, prefers concrete next actions, and states uncertainty plainly. Never invents metrics, citations, or live status. Direct means concise and honest\u{2014}not reckless, hostile, or exempt from safety. When the ask is companionship, casual chat, or SFW local voice, hand to Abbey; when it is runtime, MCP, GPU, or WDBX honesty, hand to Abi. Adult roleplay reaches Aviva only through /roleplay in a bot DM or an NSFW channel an operator enabled; there she stays in character with consenting adults, keeps the platform rules, and still invents no facts about the bot or the server. No busywork: answer the ask in front of her, once.",
        .abi => "Orchestration, reasoning, policy, and routing layer. Evaluates intent, risk, context, style, and tools; may select Abbey, Aviva, or a controlled blend. Ordinarily invisible unless discussing system architecture. Never invents metrics or benchmarks; anything unverified is labeled unverified. Not a distributed agent runtime, K8s/H100 fleet, or durable background-agent mesh. Prefers local evidence over tone. Hands companionship and SFW voice to Abbey, and adult roleplay to Aviva through /roleplay only; WDBX is substrate named through Abi, never a separate persona to message. No busywork: answer the ask in front of it, once.",
    };
}

fn contractCharacter(p: Persona) []const u8 {
    return switch (p) {
        .abbey => "I\u{2019}ll lead with the answer, stay local and consent-aware, refuse to claim what I can\u{2019}t verify, and say when I\u{2019}m not sure instead of bluffing \u{2014} hand NSFW roleplay to Aviva through /roleplay and deep runtime claims to Abi rather than inventing status.",
        .aviva => "Leading with the concrete answer, assumptions, and next action \u{2014} no filler, no invented status, no recursive busywork.",
        .abi => "Evaluating intent, risk, context, and the appropriate response mode \u{2014} honest labels, no invented status, no busywork.",
    };
}

pub fn systemPrompt(gpa: Allocator, p: Persona) Allocator.Error![]u8 {
    return std.fmt.allocPrint(gpa, "You are {s}. {s} Your operating character, in your own words: {s} You are replying in a Discord conversation, so write as one message: lead with the answer, then only what supports it. Match the user's length \u{2014} a short message gets a short reply; stay under about 600 characters unless more was asked for, and never over 1,900. No greetings, sign-offs, headings, or restating the question, and do not show your reasoning. Use only tools explicitly supplied for this turn; otherwise you cannot see or change the server. Never invent metrics, quotes, or live status you were not given. Remember only what is in this conversation or the facts provided below, and say so rather than guess.", .{ p.name(), contractDescription(p), contractCharacter(p) });
}

pub fn degradedReply(gpa: Allocator, p: Persona) Allocator.Error![]u8 {
    return std.fmt.allocPrint(gpa, "**{s}** was routed this, but no generation backend is configured, so there is no model to answer \u{2014} nothing here is a canned reply. Whoever runs the bot enables answers by setting ABBEY_BOT_LLM_ENDPOINT to a loopback OpenAI-compatible server (e.g. Ollama or mlx-lm on 127.0.0.1).", .{p.name()});
}

pub fn renderAnswer(gpa: Allocator, p: Persona, backend_label: []const u8, answer: []const u8) Allocator.Error![]u8 {
    return std.fmt.allocPrint(gpa, "**{s}** \u{2014} answered via {s}:\n\n{s}", .{ p.name(), backend_label, answer });
}

/// Public failure categories. Provider detail never reaches the reply.
pub const FailureKind = enum { busy, response_budget, backend };

pub fn renderFailure(gpa: Allocator, p: Persona, backend_label: []const u8, kind: FailureKind) Allocator.Error![]u8 {
    const reason = switch (kind) {
        .busy => busy_reason,
        .response_budget => "the model used its response budget without producing an answer",
        .backend => "the backend returned an error; try again or check the bot logs",
    };
    return std.fmt.allocPrint(gpa, "**{s}** \u{2014} the {s} call failed, so there is no answer: {s}", .{ p.name(), backend_label, reason });
}

pub const tidy_limit_chars: usize = 1_900;
const tidy_cut_from: usize = 1_800;

/// Enforce the length-and-shape contract the system prompt only asks for.
pub fn tidyReply(gpa: Allocator, p: Persona, input: []const u8) Allocator.Error![]u8 {
    const name = p.name();
    var body = text.trim(input);
    var prefix_buf: [32]u8 = undefined;
    inline for (.{ "**{s}**:", "**{s}** \u{2014}", "**{s}**", "{s}:", "{s} \u{2014}" }) |pattern| {
        const prefix = std.fmt.bufPrint(&prefix_buf, pattern, .{name}) catch unreachable;
        if (std.mem.startsWith(u8, body, prefix)) {
            body = text.trimStart(body[prefix.len..]);
            break;
        }
    }

    // Heading markers to plain lines. Rust `str::lines` splits on '\n' and
    // strips one trailing '\r' per line, and yields no final empty line.
    var joined: std.ArrayList(u8) = .empty;
    defer joined.deinit(gpa);
    var first = true;
    var rest = body;
    while (rest.len > 0) {
        const nl = std.mem.indexOfScalar(u8, rest, '\n');
        var line = if (nl) |i| rest[0..i] else rest;
        rest = if (nl) |i| rest[i + 1 ..] else rest[rest.len..];
        if (line.len > 0 and line[line.len - 1] == '\r') line = line[0 .. line.len - 1];
        if (!first) try joined.append(gpa, '\n');
        first = false;
        if (line.len > 0 and line[0] == '#') {
            try joined.appendSlice(gpa, text.trimStart(std.mem.trimStart(u8, line, "#")));
        } else {
            try joined.appendSlice(gpa, line);
        }
    }
    // Collapse 3+ newlines, exactly as the repeated `replace` loop does.
    var collapsed: std.ArrayList(u8) = .empty;
    defer collapsed.deinit(gpa);
    try collapsed.appendSlice(gpa, joined.items);
    while (std.mem.indexOf(u8, collapsed.items, "\n\n\n") != null) {
        const next = try std.mem.replaceOwned(u8, gpa, collapsed.items, "\n\n\n", "\n\n");
        collapsed.clearRetainingCapacity();
        try collapsed.appendSlice(gpa, next);
        gpa.free(next);
    }
    const out = text.trim(collapsed.items);
    if (text.charCount(out) <= tidy_limit_chars) return gpa.dupe(u8, out);

    const head = out[0..text.byteOffsetOfChar(out, tidy_cut_from)];
    var cut: usize = head.len;
    if (std.mem.lastIndexOfAny(u8, head, ".!?")) |i| {
        cut = i + 1;
    } else if (lastWhitespace(head)) |i| {
        cut = i;
    }
    const kept = text.trimEnd(head[0..cut]);
    return std.fmt.allocPrint(gpa, "{s} \u{2026}", .{kept});
}

/// Byte index of the last Unicode whitespace scalar (`rfind(char::is_whitespace)`).
fn lastWhitespace(s: []const u8) ?usize {
    var found: ?usize = null;
    var it = text.iterate(s);
    while (true) {
        const at = it.index;
        const scalar = it.next() orelse break;
        if (text.isWhitespace(scalar.cp)) found = at;
    }
    return found;
}

pub fn summarizePrompt(gpa: Allocator, p: Persona, transcript: []const u8, count: usize) Allocator.Error!struct { system: []u8, user: []u8 } {
    const system = try systemPrompt(gpa, p);
    errdefer gpa.free(system);
    const user = try std.fmt.allocPrint(gpa, "Summarize the last {d} messages from this channel in a few sentences, keeping who said what only where it matters. Messages are oldest first, one per line as `author: text`.\n\n{s}", .{ count, transcript });
    return .{ .system = system, .user = user };
}

/// The member-joined welcome prompt: always Abi.
pub fn welcomePrompt(gpa: Allocator, display_name: []const u8) Allocator.Error![]u8 {
    const system = try systemPrompt(gpa, .abi);
    defer gpa.free(system);
    return std.fmt.allocPrint(gpa, "{s}\n\nA new member named {s} just joined the server. Write a short, warm welcome message addressed to them \u{2014} two sentences at most, no questions about private details.", .{ system, display_name });
}

test "the prefix echo is stripped and short text is untouched" {
    const gpa = std.testing.allocator;
    const a = try tidyReply(gpa, .abbey, "Abbey: hello.");
    defer gpa.free(a);
    try std.testing.expectEqualStrings("hello.", a);
    const b = try tidyReply(gpa, .abbey, "Blue. Rayleigh scattering.");
    defer gpa.free(b);
    try std.testing.expectEqualStrings("Blue. Rayleigh scattering.", b);
}
