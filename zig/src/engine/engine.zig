//! Conversation engine, transcribed from the oracle's `src/engine.rs`: one
//! session per scope key (normally `discord:<channel id>`), persona switches
//! keep the transcript, trimming drops the oldest turns by count and by
//! character budget, and `prepare` never records a turn (only `commit` does,
//! after the backend answered).
const std = @import("std");
const text = @import("../text/text.zig");
const persona_mod = @import("../persona/persona.zig");
const signals = @import("../persona/signals.zig");
const prompts = @import("../persona/prompts.zig");
const grounding = @import("../grounding/grounding.zig");
const PersonaContext = @import("../memory/context.zig").PersonaContext;
const Persona = persona_mod.Persona;
const Allocator = std.mem.Allocator;

pub const max_turns: usize = 20;
pub const context_budget_chars: usize = 6000;

pub const Role = enum { user, assistant };

pub const ChatTurn = struct {
    role: Role,
    text: []const u8,
};

const Session = struct {
    persona: Persona,
    turns: std.ArrayList(ChatTurn) = .empty,
    last_used: u64,

    fn deinit(s: *Session, gpa: Allocator) void {
        for (s.turns.items) |t| gpa.free(t.text);
        s.turns.deinit(gpa);
    }

    fn chars(s: *const Session) usize {
        var n: usize = 0;
        for (s.turns.items) |t| n += text.charCount(t.text);
        return n;
    }

    fn popFront(s: *Session, gpa: Allocator) void {
        const t = s.turns.orderedRemove(0);
        gpa.free(t.text);
    }

    /// Drop oldest turns until both caps hold, then never start on an
    /// assistant turn.
    fn trim(s: *Session, gpa: Allocator) void {
        while ((s.turns.items.len > max_turns or s.chars() > context_budget_chars) and s.turns.items.len > 0) s.popFront(gpa);
        while (s.turns.items.len > 0 and s.turns.items[0].role == .assistant) s.popFront(gpa);
    }
};

/// What the command layer sends to a backend. Everything is owned by the
/// allocator passed to `prepare` (arena-per-request is the intended use).
pub const PreparedTurn = struct {
    system_prompt: []u8,
    turns: []ChatTurn,
    grounding: grounding.Grounding,

    pub fn deinit(p: *PreparedTurn, gpa: Allocator) void {
        gpa.free(p.system_prompt);
        for (p.turns) |t| gpa.free(t.text);
        gpa.free(p.turns);
        p.grounding.deinit(gpa);
    }

    pub fn chars(p: *const PreparedTurn) usize {
        var n = text.charCount(p.system_prompt);
        for (p.turns) |t| n += text.charCount(t.text);
        return n;
    }

    pub fn willOverflow(p: *const PreparedTurn, budget_chars: usize) bool {
        return p.chars() > budget_chars;
    }
};

pub const Engine = struct {
    gpa: Allocator,
    sessions: std.StringHashMapUnmanaged(Session) = .empty,

    pub fn init(gpa: Allocator) Engine {
        return .{ .gpa = gpa };
    }

    pub fn deinit(e: *Engine) void {
        var it = e.sessions.iterator();
        while (it.next()) |kv| {
            e.gpa.free(kv.key_ptr.*);
            kv.value_ptr.deinit(e.gpa);
        }
        e.sessions.deinit(e.gpa);
    }

    fn sessionEntry(e: *Engine, scope: []const u8, persona: Persona, now: u64) Allocator.Error!*Session {
        const gop = try e.sessions.getOrPut(e.gpa, scope);
        if (!gop.found_existing) {
            gop.key_ptr.* = e.gpa.dupe(u8, scope) catch |err| {
                _ = e.sessions.remove(scope);
                return err;
            };
            gop.value_ptr.* = .{ .persona = persona, .last_used = now };
        }
        return gop.value_ptr;
    }

    /// Create-or-update: insert with `persona` on first use, then switch the
    /// persona and refresh the timestamp. The transcript is untouched.
    fn touchSession(e: *Engine, scope: []const u8, persona: Persona, now: u64) Allocator.Error!void {
        const s = try e.sessionEntry(scope, persona, now);
        s.persona = persona;
        s.last_used = now;
    }

    pub fn prepare(e: *Engine, out_gpa: Allocator, scope: []const u8, persona: Persona, context: PersonaContext, user_input: []const u8, now: u64) Allocator.Error!PreparedTurn {
        try e.touchSession(scope, persona, now);
        return e.prepareEphemeral(out_gpa, scope, persona, context, user_input);
    }

    /// Read shared history for a private request without creating, changing,
    /// or refreshing a session.
    pub fn prepareEphemeral(e: *const Engine, out_gpa: Allocator, scope: []const u8, persona: Persona, context: PersonaContext, user_input: []const u8) Allocator.Error!PreparedTurn {
        var turns: std.ArrayList(ChatTurn) = .empty;
        errdefer {
            for (turns.items) |t| out_gpa.free(t.text);
            turns.deinit(out_gpa);
        }
        if (e.sessions.getPtr(scope)) |s| {
            for (s.turns.items) |t| {
                const copy = try out_gpa.dupe(u8, t.text);
                errdefer out_gpa.free(copy);
                try turns.append(out_gpa, .{ .role = t.role, .text = copy });
            }
        }
        {
            const copy = try out_gpa.dupe(u8, user_input);
            errdefer out_gpa.free(copy);
            try turns.append(out_gpa, .{ .role = .user, .text = copy });
        }
        var g: grounding.Grounding = .{};
        errdefer g.deinit(out_gpa);
        for (turns.items) |t| {
            if (t.role == .user) try g.pushSource(out_gpa, t.text);
        }
        const sources = try context.groundingSources(out_gpa, user_input);
        defer out_gpa.free(sources);
        for (sources) |src| try g.pushSource(out_gpa, src);

        const base = try prompts.systemPrompt(out_gpa, persona);
        defer out_gpa.free(base);
        const rendered = try context.render(out_gpa, user_input);
        defer out_gpa.free(rendered);
        const system_prompt = try std.fmt.allocPrint(out_gpa, "{s}\n\n{s}", .{ base, rendered });
        errdefer out_gpa.free(system_prompt);
        const owned_turns = try turns.toOwnedSlice(out_gpa);
        return .{ .system_prompt = system_prompt, .turns = owned_turns, .grounding = g };
    }

    /// Record one exchange and trim to the caps. A scope first seen here is
    /// created as Abbey, exactly as the oracle does.
    pub fn commit(e: *Engine, scope: []const u8, user_input: []const u8, assistant_reply: []const u8, now: u64) Allocator.Error!void {
        const s = try e.sessionEntry(scope, .abbey, now);
        const u = try e.gpa.dupe(u8, user_input);
        errdefer e.gpa.free(u);
        const a = try e.gpa.dupe(u8, assistant_reply);
        errdefer e.gpa.free(a);
        try s.turns.ensureUnusedCapacity(e.gpa, 2);
        s.turns.appendAssumeCapacity(.{ .role = .user, .text = u });
        s.turns.appendAssumeCapacity(.{ .role = .assistant, .text = a });
        s.last_used = now;
        s.trim(e.gpa);
    }

    pub fn reset(e: *Engine, scope: []const u8) bool {
        const kv = e.sessions.fetchRemove(scope) orelse return false;
        e.gpa.free(kv.key);
        var s = kv.value;
        s.deinit(e.gpa);
        return true;
    }

    pub fn evictIdle(e: *Engine, now: u64, max_idle_secs: u64) usize {
        var dropped: usize = 0;
        var again = true;
        while (again) {
            again = false;
            var it = e.sessions.iterator();
            while (it.next()) |kv| {
                if (now -| kv.value_ptr.last_used > max_idle_secs) {
                    const key = kv.key_ptr.*;
                    kv.value_ptr.deinit(e.gpa);
                    e.sessions.removeByPtr(kv.key_ptr);
                    e.gpa.free(key);
                    dropped += 1;
                    again = true;
                    break;
                }
            }
        }
        return dropped;
    }

    pub fn sessionLen(e: *const Engine, scope: []const u8) usize {
        const s = e.sessions.getPtr(scope) orelse return 0;
        return s.turns.items.len;
    }

    /// Stick `persona` on `scope` without recording a turn (the empty
    /// `/roleplay` arm).
    pub fn setSessionPersona(e: *Engine, scope: []const u8, persona: Persona, now: u64) Allocator.Error!void {
        try e.touchSession(scope, persona, now);
    }

    pub fn sessionPersona(e: *const Engine, scope: []const u8) ?Persona {
        const s = e.sessions.getPtr(scope) orelse return null;
        return s.persona;
    }

    /// The persona that answers `question` in `scope`: a decisive composed
    /// route wins; otherwise session stickiness, then the route's own choice.
    /// This is `answer_question_with_memory`'s rule in the oracle's
    /// `commands.rs`.
    pub fn routedPersona(e: *const Engine, route: signals.ComposedRoute, scope: []const u8) Persona {
        if (route.isDecisive()) return route.persona;
        return e.sessionPersona(scope) orelse route.persona;
    }
};

const testing = std.testing;

test "prepare appends context and records nothing" {
    var e = Engine.init(testing.allocator);
    defer e.deinit();
    const ctx: PersonaContext = .{ .channel_summary = "deploy talk", .user_facts = &.{"likes rust"}, .reputation = 0.5 };
    var p = try e.prepare(testing.allocator, "c", .aviva, ctx, "hi", 1);
    defer p.deinit(testing.allocator);
    const base = try prompts.systemPrompt(testing.allocator, .aviva);
    defer testing.allocator.free(base);
    const rendered = try ctx.render(testing.allocator, "hi");
    defer testing.allocator.free(rendered);
    const want = try std.fmt.allocPrint(testing.allocator, "{s}\n\n{s}", .{ base, rendered });
    defer testing.allocator.free(want);
    try testing.expectEqualStrings(want, p.system_prompt);
    try testing.expect(std.mem.indexOf(u8, p.system_prompt, "Recent channel context: deploy talk") != null);
    try testing.expectEqual(@as(usize, 1), p.turns.len);
    try testing.expectEqual(@as(usize, 0), e.sessionLen("c"));
}

test "trimming by turn count keeps the most recent" {
    var e = Engine.init(testing.allocator);
    defer e.deinit();
    var i: u64 = 0;
    while (i < 15) : (i += 1) {
        var qb: [8]u8 = undefined;
        var ab: [8]u8 = undefined;
        try e.commit("c", try std.fmt.bufPrint(&qb, "q{d}", .{i}), try std.fmt.bufPrint(&ab, "a{d}", .{i}), i);
    }
    try testing.expectEqual(max_turns, e.sessionLen("c"));
    var p = try e.prepare(testing.allocator, "c", .abbey, .empty, "next", 99);
    defer p.deinit(testing.allocator);
    try testing.expectEqualStrings("q5", p.turns[0].text);
    try testing.expectEqualStrings("a14", p.turns[max_turns - 1].text);
    try testing.expectEqualStrings("next", p.turns[max_turns].text);
}

test "trimming by char budget starts on a user turn" {
    var e = Engine.init(testing.allocator);
    defer e.deinit();
    const big_array: [2500]u8 = @splat('x');
    const big: []const u8 = &big_array;
    try e.commit("c", "q0", big, 1);
    try e.commit("c", "q1", big, 2);
    try e.commit("c", "q2", big, 3);
    var p = try e.prepare(testing.allocator, "c", .abbey, .empty, "next", 4);
    defer p.deinit(testing.allocator);
    try testing.expectEqualStrings("q1", p.turns[0].text);
    try testing.expectEqual(Role.user, p.turns[0].role);
    try testing.expectEqual(@as(usize, 5), p.turns.len);
}

test "persona switch keeps the transcript" {
    var e = Engine.init(testing.allocator);
    defer e.deinit();
    var first = try e.prepare(testing.allocator, "c", .abbey, .empty, "q1", 1);
    first.deinit(testing.allocator);
    try e.commit("c", "q1", "a1", 1);
    var p = try e.prepare(testing.allocator, "c", .aviva, .empty, "q2", 2);
    defer p.deinit(testing.allocator);
    try testing.expectEqual(@as(?Persona, .aviva), e.sessionPersona("c"));
    try testing.expect(std.mem.startsWith(u8, p.system_prompt, "You are Aviva. "));
    try testing.expectEqual(@as(usize, 3), p.turns.len);
}

test "empty roleplay stick creates the session as Aviva with no turn and follow-ups read it" {
    // Oracle 281ee3b: the empty `/roleplay` arm calls set_session_persona with
    // the gate's persona on scope discord:<channel id>; touch_session inserts
    // with that persona, so the session is never created as Abbey.
    var e = Engine.init(testing.allocator);
    defer e.deinit();
    try e.setSessionPersona("discord:1", .aviva, 1);
    try testing.expectEqual(@as(?Persona, .aviva), e.sessionPersona("discord:1"));
    try testing.expectEqual(@as(usize, 0), e.sessionLen("discord:1"));
    // A neutral follow-up in the same channel answers as Aviva ...
    const neutral = try signals.route(testing.allocator, "hello there", null);
    try testing.expect(!neutral.isDecisive());
    try testing.expectEqual(Persona.aviva, e.routedPersona(neutral, "discord:1"));
    // ... while decisive text still routes on its own evidence ...
    const decisive = try signals.route(testing.allocator, "Abi: orchestrate this", null);
    try testing.expectEqual(Persona.abi, e.routedPersona(decisive, "discord:1"));
    // ... and another channel is unaffected.
    try testing.expectEqual(Persona.abbey, e.routedPersona(neutral, "discord:2"));
}

test "prepared grounding is a pre-candidate snapshot" {
    var e = Engine.init(testing.allocator);
    defer e.deinit();
    try e.commit("c", "The lockfile says 1.2.3. Which line should ship?", "Ship 4.2.1.", 1);
    var p = try e.prepare(testing.allocator, "c", .abbey, .empty, "what should ship?", 2);
    defer p.deinit(testing.allocator);
    var keep = try grounding.check(testing.allocator, "Keep 1.2.3.", &p.grounding);
    defer keep.deinit(testing.allocator);
    try testing.expect(keep.isGrounded());
    var echo = try grounding.check(testing.allocator, "Keep 4.2.1.", &p.grounding);
    defer echo.deinit(testing.allocator);
    try testing.expect(echo.shouldHedge());
    try e.commit("c", "what should ship?", "Ship 4.2.1 in 2019.", 2);
    var after = try grounding.check(testing.allocator, "Ship 4.2.1 in 2019.", &p.grounding);
    defer after.deinit(testing.allocator);
    try testing.expect(after.shouldHedge());
}

test "reset forgets one scope and evict_idle drops only stale sessions" {
    var e = Engine.init(testing.allocator);
    defer e.deinit();
    try e.commit("c1", "q", "a", 1);
    try e.commit("c2", "q", "a", 1);
    try testing.expect(e.reset("c1"));
    try testing.expect(!e.reset("c1"));
    try testing.expectEqual(@as(usize, 2), e.sessionLen("c2"));
    try e.commit("old", "q", "a", 100);
    try e.commit("fresh", "q", "a", 900);
    try testing.expectEqual(@as(usize, 2), e.evictIdle(1000, 300)); // c2 (1) and old (100)
    try testing.expectEqual(@as(usize, 0), e.evictIdle(1200, 300));
    try testing.expectEqual(@as(usize, 1), e.evictIdle(1201, 300));
}

test "will_overflow measures prompt plus turns" {
    var e = Engine.init(testing.allocator);
    defer e.deinit();
    var p = try e.prepare(testing.allocator, "c", .abbey, .empty, "hello", 1);
    defer p.deinit(testing.allocator);
    const size = p.chars();
    try testing.expect(!p.willOverflow(size));
    try testing.expect(p.willOverflow(size - 1));
}
