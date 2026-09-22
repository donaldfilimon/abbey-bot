//! Phase-1 slash-command handlers. Each is the oracle's command body
//! (`commands.rs`, `commands_brain*.rs`, `commands_help.rs`) with Discord
//! transport replaced by a returned `Reply`. Decision logic stays in the pure
//! modules; this file translates.
const std = @import("std");
const App = @import("app.zig").App;
const reply_mod = @import("reply.zig");
const Reply = reply_mod.Reply;
const interaction_mod = @import("../discord/interaction.zig");
const Interaction = interaction_mod.Interaction;
const catalog = @import("../catalog/catalog.zig");
const help_session = @import("../catalog/help_session.zig");
const persona_mod = @import("../persona/persona.zig");
const Persona = persona_mod.Persona;
const signals = @import("../persona/signals.zig");
const prompts = @import("../persona/prompts.zig");
const roleplay = @import("../persona/roleplay.zig");
const grounding = @import("../grounding/grounding.zig");
const guidance = @import("../llm/guidance.zig");
const bank = @import("../memory/bank.zig");
const memory_gate = @import("../memory/gate.zig");
const moderation = @import("../moderation/moderation.zig");
const decimal = @import("../text/decimal.zig");
const text = @import("../text/text.zig");
const Allocator = std.mem.Allocator;

pub const no_guild = "This one only works inside a server.";
pub const cross_user_memory_denied = "You can manage only your own memory unless Discord currently grants you Manage Messages or Manage Server.";
const recall_k = 3;

/// Top-level commands with a phase-1 handler for every leaf. Registration
/// sends exactly these (see registration.zig).
pub const handled = [_][]const u8{ "help", "persona", "roleplay", "nsfw", "remember", "forget", "recall", "modcall" };

pub fn scopedGuild(arena: Allocator, i: *const Interaction) Allocator.Error![]const u8 {
    return if (i.guild_id) |g| std.fmt.allocPrint(arena, "discord:{d}", .{g}) else std.fmt.allocPrint(arena, "discord:dm:{d}", .{i.user.id});
}

fn scopedUser(arena: Allocator, id: u64) Allocator.Error![]const u8 {
    return std.fmt.allocPrint(arena, "discord:{d}", .{id});
}

fn scopedChannel(arena: Allocator, i: *const Interaction) Allocator.Error![]const u8 {
    return std.fmt.allocPrint(arena, "discord:{d}", .{i.channel_id});
}

fn clamp(arena: Allocator, body: []const u8) Allocator.Error![]const u8 {
    return catalog.clampMessage(arena, body);
}

fn textReply(arena: Allocator, body: []const u8) Allocator.Error!Reply {
    return .{ .content = try clamp(arena, body) };
}

/// The oracle's `runtime_input` + `invocation_input`, from what the
/// interaction carries (member permissions are the channel-resolved bits).
pub fn eligibilityInput(app: *App, i: *const Interaction, vision_allowed: bool) catalog.EligibilityInput {
    var input: catalog.EligibilityInput = .{ .context = if (i.guild_id != null) .guild else .bot_dm };
    input.vision_allowed = vision_allowed;
    const generation: catalog.Readiness = if (app.provider.configured()) .ready else .{ .blocked = .generation };
    input.readiness.set(.generation, generation);
    // Phase 1 offers no model tools, so a tool-generation route is the plain
    // text route (the oracle's ABBEY_BOT_LLM_TOOLS=off behavior).
    input.readiness.set(.tool_generation, generation);
    input.application_owner = app.isOwner(i.user.id);
    input.self_subject = true;
    if (i.option("user")) |v| {
        if (v == .snowflake) input.self_subject = v.snowflake == i.user.id;
    }
    if (i.target_id) |t| input.self_subject = t == i.user.id;
    input.follow_up_absent = i.option("question") == null;
    if (i.member_permissions) |bits| catalog.DiscordPermission.fromBits(bits, &input.permissions);
    return input;
}

/// The oracle's `catalog_check`: refuse with the availability message unless
/// the spec is Ready. `/help` and `/modcall` do their own checks.
pub fn guard(app: *App, i: *const Interaction, spec: *const catalog.CommandSpec) ?[]const u8 {
    if (spec.key == .help or spec.key == .modcall) return null;
    const input = eligibilityInput(app, i, true);
    const a = catalog.availability(spec, &input);
    return if (a == .ready) null else a.message();
}

pub fn dispatch(app: *App, arena: Allocator, i: *const Interaction) !Reply {
    const path = i.command_path;
    if (std.mem.eql(u8, path, "help")) return help(app, arena, i);
    if (std.mem.eql(u8, path, "persona route")) return personaRoute(arena, i);
    if (std.mem.eql(u8, path, "persona ask")) return personaAsk(app, arena, i);
    if (std.mem.eql(u8, path, "roleplay")) return roleplayCommand(app, arena, i);
    if (std.mem.eql(u8, path, "nsfw")) return nsfw(app, arena, i);
    if (std.mem.eql(u8, path, "remember")) return remember(app, arena, i);
    if (std.mem.eql(u8, path, "forget")) return forget(app, arena, i);
    if (std.mem.eql(u8, path, "recall")) return recall(app, arena, i);
    if (std.mem.eql(u8, path, "modcall")) return @import("modcall.zig").modcall(app, arena, i);
    return error.UnhandledCommand;
}

// ---------------------------------------------------------------------------
// /help
// ---------------------------------------------------------------------------

/// The Start section advertises task buttons; the five private workflows are
/// not ported in phase 1, so that sentence is dropped rather than promised.
const workflow_sentence = "\nTask buttons open private workflows. Use the section menu for the command reference.\n";

fn helpReply(app: *App, arena: Allocator, i: *const Interaction, session: help_session.Session) !Reply {
    var input = eligibilityInput(app, i, true);
    input.follow_up_absent = true;
    const raw = try catalog.renderHelp(arena, session.section, &input);
    const body = try std.mem.replaceOwned(u8, arena, raw, workflow_sentence, "");
    const options = try arena.alloc(reply_mod.SelectOption, catalog.HelpSection.all.len);
    for (catalog.HelpSection.all, options) |s, *o| o.* = .{ .label = s.label(), .value = s.slug(), .default = s == session.section };
    const id_buf = try arena.alloc(u8, 100);
    return .{ .embed = try clamp(arena, body), .select = .{ .custom_id = session.customId(id_buf), .placeholder = "Choose a help section", .options = options } };
}

fn help(app: *App, arena: Allocator, i: *const Interaction) !Reply {
    const section: catalog.HelpSection = if (i.option("section")) |v| switch (v) {
        .integer => |n| if (n >= 0 and n < catalog.HelpSection.all.len) catalog.HelpSection.all[@intCast(n)] else .start,
        else => .start,
    } else .start;
    const session = help_session.Session.new(i.user.id, app.now(), section) orelse return .{ .embed = help_session.stale };
    return helpReply(app, arena, i, session);
}

/// A section select on a help message (component interaction).
pub fn helpComponent(app: *App, arena: Allocator, i: *const Interaction) !Reply {
    const id = i.custom_id orelse return .{ .embed = help_session.stale };
    const session = switch (help_session.validate(id, i.user.id, app.now())) {
        .ok => |s| s,
        .rejected => |r| return .{ .embed = r.message() },
    };
    if (i.user.bot or i.values.len != 1) return .{ .embed = help_session.stale };
    const section = catalog.HelpSection.parse(i.values[0]) orelse return .{ .embed = help_session.stale };
    return helpReply(app, arena, i, session.navigate(section));
}

// ---------------------------------------------------------------------------
// /persona
// ---------------------------------------------------------------------------

fn forcedPersona(i: *const Interaction) ?Persona {
    const v = i.option("as") orelse return null;
    return switch (v) {
        .integer => |n| if (n >= 0 and n < 3) Persona.all[@intCast(n)] else null,
        else => null,
    };
}

fn personaRoute(arena: Allocator, i: *const Interaction) !Reply {
    const request = i.string("request") orelse "";
    const route = try signals.route(arena, request, forcedPersona(i));
    return textReply(arena, try signals.describe(arena, route));
}

fn personaAsk(app: *App, arena: Allocator, i: *const Interaction) !Reply {
    return textReply(arena, try answerQuestion(app, arena, i, i.string("question") orelse "", forcedPersona(i), true));
}

/// Shared generation for `/persona ask` and `/roleplay` (oracle
/// `answer_question_with_memory`): one routing decision, one cooldown, one
/// transcript scope. The lock is released for the network call.
pub fn answerQuestion(app: *App, arena: Allocator, i: *const Interaction, question: []const u8, forced: ?Persona, commit: bool) ![]const u8 {
    const scope = try scopedChannel(arena, i);
    const route = try signals.route(arena, question, forced);
    const guild = try scopedGuild(arena, i);
    const user = try scopedUser(arena, i.user.id);
    const now = app.now();
    app.lock();
    const routed = app.engine.routedPersona(route, scope);
    if (!try app.reserveAsk(user, now)) {
        app.unlock();
        return @import("app.zig").ask_cooldown_reply;
    }
    const label = app.provider.label() orelse {
        app.unlock();
        return prompts.degradedReply(arena, routed);
    };
    const context = try app.memory.contextFor(arena, guild, user, scope, question, recall_k, bank.default_reputation);
    var prepared = if (commit)
        try app.engine.prepare(arena, scope, routed, context, question, now)
    else
        try app.engine.prepareEphemeral(arena, scope, routed, context, question);
    app.unlock();
    const system = try guidance.systemPrompt(arena, prepared.system_prompt, scope, question);
    const generated = try app.provider.generate(arena, app.http_client, system, prepared.turns) orelse return prompts.degradedReply(arena, routed);
    switch (generated.outcome) {
        .failure => |kind| return prompts.renderFailure(arena, routed, generated.label, kind),
        .answer => |raw| {
            const tidied = try prompts.tidyReply(arena, routed, raw);
            var verdict = try grounding.check(arena, tidied, &prepared.grounding);
            const final = try grounding.hedged(arena, tidied, &verdict);
            if (commit) {
                app.lock();
                defer app.unlock();
                try app.engine.commit(scope, question, final, app.now());
            }
            _ = label;
            return prompts.renderAnswer(arena, routed, generated.label, final);
        },
    }
}

// ---------------------------------------------------------------------------
// /roleplay and /nsfw
// ---------------------------------------------------------------------------

const nsfw_key = "nsfw_roleplay_enabled";

fn roleplayCommand(app: *App, arena: Allocator, i: *const Interaction) !Reply {
    const guild = try scopedGuild(arena, i);
    app.lock();
    const enabled = if (app.memory.setting(guild, nsfw_key)) |v| std.mem.eql(u8, v, "true") else false;
    app.unlock();
    const context: roleplay.Context = if (i.guild_id != null) .{ .guild = .{ .channel_nsfw = i.channel_nsfw } } else .bot_dm;
    const decision = roleplay.decide(context, enabled);
    const persona = decision.persona();
    if (!decision.allow() or persona == null) return textReply(arena, decision.message());
    const prompt_text = i.string("prompt");
    if (prompt_text == null or text.trim(prompt_text.?).len == 0) {
        // Explicit persona stick on this channel; no transcript turn.
        const scope = try scopedChannel(arena, i);
        app.lock();
        defer app.unlock();
        try app.engine.setSessionPersona(scope, persona.?, app.now());
        return textReply(arena, decision.message());
    }
    return textReply(arena, try answerQuestion(app, arena, i, prompt_text.?, persona, true));
}

fn onOff(i: *const Interaction) ?bool {
    const v = i.option("state") orelse return null;
    return switch (v) {
        .integer => |n| n == 0, // choices: on = 0, off = 1
        else => null,
    };
}

fn nsfw(app: *App, arena: Allocator, i: *const Interaction) !Reply {
    if (i.guild_id != null) return textReply(arena, "In a server, use `/admin nsfw on|off` instead.");
    const on = onOff(i) orelse return textReply(arena, "Choose on or off.");
    const guild = try scopedGuild(arena, i);
    app.lock();
    defer app.unlock();
    try app.memory.setSetting(guild, nsfw_key, if (on) "true" else "false", app.now());
    if (!on) _ = app.engine.reset(try scopedChannel(arena, i));
    return textReply(arena, try std.fmt.allocPrint(arena, "nsfw roleplay is now **{s}** in this DM.", .{if (on) "on" else "off"}));
}

// ---------------------------------------------------------------------------
// Memory: /remember, /forget, /recall
// ---------------------------------------------------------------------------

/// The oracle's `memory_subject_authorized` (access rule A1).
fn subjectAuthorized(i: *const Interaction, subject: u64) bool {
    var input: catalog.EligibilityInput = .{ .context = .guild, .self_subject = subject == i.user.id };
    if (subject != i.user.id) if (i.member_permissions) |bits| catalog.DiscordPermission.fromBits(bits, &input.permissions);
    return catalog.accessAllows(catalog.AccessId.a1.rule(), &input);
}

fn subjectOf(i: *const Interaction) u64 {
    if (i.option("user")) |v| if (v == .snowflake) return v.snowflake;
    return i.user.id;
}

fn remember(app: *App, arena: Allocator, i: *const Interaction) !Reply {
    const guild = try scopedGuild(arena, i);
    const subject = subjectOf(i);
    if (!subjectAuthorized(i, subject)) return textReply(arena, cross_user_memory_denied);
    const user = try scopedUser(arena, subject);
    const fact = bank.validatedFact(arena, i.string("fact") orelse "") catch |e| switch (e) {
        error.OutOfMemory => return error.OutOfMemory,
        else => |fe| return textReply(arena, bank.factErrorMessage(fe)),
    };
    const replaces = i.string("replaces");
    app.lock();
    const gated = app.gateEnv() != null and app.gate.?.config.covers(guild);
    if (gated) {
        if (replaces) |old| {
            const selected = try app.memory.resolveFact(arena, guild, user, old);
            if (selected == null) {
                app.unlock();
                return textReply(arena, "No remembered fact matches what you asked to replace.");
            }
            if (std.mem.eql(u8, selected.?, fact)) {
                app.unlock();
                return textReply(arena, "Already on record (or the fact list is full).");
            }
        } else if (app.memory.rememberBlocked(guild, user, fact) != null) {
            app.unlock();
            return textReply(arena, "Already on record (or the fact list is full).");
        }
    }
    app.unlock();
    // The gate call is a subprocess; never under the lock.
    const admission = try memory_gate.admitFact(&app.memory, app.gateEnv(), arena, guild, user, fact, replaces, app.now());
    const receipt: ?[]const u8 = switch (admission) {
        .ungated => null,
        .receipt => |r| r,
        .refused => |d| return textReply(arena, d.message()),
    };
    app.lock();
    defer app.unlock();
    const now = app.now();
    const result = if (replaces) |old| try app.memory.rememberReplacing(arena, guild, user, fact, old, now) else try app.memory.remember(arena, guild, user, fact, now);
    const outcome = switch (result) {
        .refused => |m| return textReply(arena, m),
        .ok => |o| o,
    };
    if (receipt) |digest| switch (outcome) {
        .stored => |stored| try app.memory.recordReceipt(guild, user, stored, digest, now),
        .superseded => |s| {
            try app.memory.dropReceipt(guild, user, s.removed, now);
            try app.memory.recordReceipt(guild, user, s.stored, digest, now);
        },
        else => {},
    };
    const body = switch (outcome) {
        .stored => |stored| try std.fmt.allocPrint(arena, "Stored about <@{d}>: {s}", .{ subject, stored }),
        .superseded => |s| try std.fmt.allocPrint(arena, "Stored about <@{d}>: {s}\nReplaced: {s}", .{ subject, s.stored, s.removed }),
        .proposed => |p| try std.fmt.allocPrint(arena, "Stored about <@{d}>: {s}\nProposed to replace: {s} \u{2014} nothing was removed. Run /pending confirm to apply it.", .{ subject, p.stored, p.proposed }),
        .unchanged => "Already on record (or the fact list is full).",
    };
    return textReply(arena, body);
}

fn forget(app: *App, arena: Allocator, i: *const Interaction) !Reply {
    const guild = try scopedGuild(arena, i);
    const subject = subjectOf(i);
    if (!subjectAuthorized(i, subject)) return textReply(arena, cross_user_memory_denied);
    const user = try scopedUser(arena, subject);
    app.lock();
    const selected = try app.memory.resolveFact(arena, guild, user, i.string("fact") orelse "");
    app.unlock();
    const fact = selected orelse return textReply(arena, "Nothing by that wording was on record.");
    if (try memory_gate.admitForget(&app.memory, app.gateEnv(), arena, guild, user, fact, app.now())) |refusal| return textReply(arena, refusal);
    app.lock();
    defer app.unlock();
    const removed = try app.memory.forget(arena, guild, user, fact, app.now());
    if (removed) try app.memory.dropReceipt(guild, user, fact, app.now());
    return textReply(arena, if (removed) "Forgotten." else "Nothing by that wording was on record.");
}

const max_card_facts = 6;
const max_card_pending = 2;
const max_card_item_chars = 160;

fn cardItem(w: *std.Io.Writer, s: []const u8) !void {
    const end = text.byteOffsetOfChar(s, max_card_item_chars);
    try w.writeAll(s[0..end]);
    if (end < s.len) try w.writeAll("\u{2026}");
}

/// The oracle's `memory_card::render`. Standing is the neutral default in
/// phase 1: reputation signals come from the Phase-2 learning pipeline.
pub fn renderCard(arena: Allocator, subject_id: u64, facts: []const []const u8, pending: []const bank.PendingSupersession, standing: f64) ![]u8 {
    var out: std.Io.Writer.Allocating = .init(arena);
    const w = &out.writer;
    try w.print("**<@{d}>** \u{2014} standing ", .{subject_id});
    try decimal.writeFixed(w, std.math.clamp(standing, 0.0, 1.0), 2);
    try w.writeAll(" (0 = poor, 1 = excellent)\nFacts:\n");
    if (facts.len == 0) {
        try w.writeAll("\u{2022} No facts on record.\n");
    } else {
        for (facts[0..@min(facts.len, max_card_facts)]) |f| {
            try w.writeAll("\u{2022} ");
            try cardItem(w, f);
            try w.writeByte('\n');
        }
        if (facts.len > max_card_facts) try w.print("\u{2022} \u{2026}and {d} more.\n", .{facts.len - max_card_facts});
    }
    try w.writeAll("Pending replacements:\n");
    if (pending.len == 0) {
        try w.writeAll("\u{2022} None.");
    } else {
        for (pending[0..@min(pending.len, max_card_pending)]) |p| {
            try w.writeAll("\u{2022} ");
            try cardItem(w, p.old_fact);
            try w.writeAll(" \u{2192} ");
            try cardItem(w, p.new_fact);
            try w.writeByte('\n');
        }
        if (pending.len > max_card_pending) try w.print("\u{2022} \u{2026}and {d} more. Use `/pending list` to review them.", .{pending.len - max_card_pending});
    }
    return out.toOwnedSlice();
}

fn recall(app: *App, arena: Allocator, i: *const Interaction) !Reply {
    const guild = try scopedGuild(arena, i);
    const subject = subjectOf(i);
    if (!subjectAuthorized(i, subject)) return textReply(arena, cross_user_memory_denied);
    const user = try scopedUser(arena, subject);
    app.lock();
    defer app.unlock();
    const facts = try arena.dupe([]const u8, app.memory.bank.facts(guild, user));
    for (facts) |*f| f.* = try arena.dupe(u8, f.*);
    const pending = try arena.dupe(bank.PendingSupersession, app.memory.bank.pending(guild, user));
    return textReply(arena, try renderCard(arena, subject, facts, pending, bank.default_reputation));
}
