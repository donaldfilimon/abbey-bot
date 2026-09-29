//! Pure Discord surface policy, transcribed from the oracle's
//! `src/command_catalog{,/availability,/decisions}.rs`: command specs, access
//! and condition rules, availability, and the `/help` section reference.
//! No transport, clock, environment, or live identities.
const std = @import("std");
const Allocator = std.mem.Allocator;
pub const specs = @import("specs.zig");

pub const CommandKind = enum { slash, user_context, message_context };
pub const InteractionContext = enum { guild, bot_dm };

pub const DiscordPermission = enum {
    manage_messages,
    moderate_members,
    manage_webhooks,
    manage_server,
    manage_channels,
    manage_roles,
    move_members,
    administrator,

    /// Discord permission bit for this flag.
    pub fn bit(p: DiscordPermission) u64 {
        return switch (p) {
            .manage_messages => 1 << 13,
            .moderate_members => 1 << 40,
            .manage_webhooks => 1 << 29,
            .manage_server => 1 << 5,
            .manage_channels => 1 << 4,
            .manage_roles => 1 << 28,
            .move_members => 1 << 24,
            .administrator => 1 << 3,
        };
    }

    /// The oracle's `permissions_input`: flags present in a Discord bitfield,
    /// in declaration order.
    pub fn fromBits(bits: u64, out: *std.EnumSet(DiscordPermission)) void {
        out.* = .empty;
        inline for (@typeInfo(DiscordPermission).@"enum".field_names) |field| {
            const p = @field(DiscordPermission, field);
            if (bits & p.bit() != 0) out.insert(p);
        }
    }
};

pub const Capability = enum { generation, tool_generation, vision, ocr, voice_configured, voice_local, voice_open_ai };

pub const Blocker = enum {
    context,
    permission,
    voice_presence,
    subject,
    generation,
    vision,
    ocr,
    vision_policy,
    voice_setup,
    voice_mode,
    target,
    hierarchy,
    unknown,
    busy,
    unavailable,

    pub fn label(b: Blocker) []const u8 {
        return switch (b) {
            .generation => "Needs generation setup",
            .vision => "Needs image-description setup",
            .ocr => "Needs text-extraction setup",
            .vision_policy => "Images disabled by server policy",
            .voice_setup => "Needs voice setup",
            .voice_mode => "Selected voice mode unavailable",
            .target => "Needs a member and action",
            .hierarchy => "Blocked by role hierarchy",
            .unknown => "Readiness unknown; ask a manager to check",
            .busy => "Provider busy; retry shortly",
            .unavailable => "Provider unavailable; retry later",
            .context => "Requires a server",
            .permission => "Requires permission",
            .voice_presence => "Requires presence in the voice channel",
            .subject => "Requires your own subject",
        };
    }

    pub fn message(b: Blocker) []const u8 {
        return switch (b) {
            .context => "Use this command in a server where Abbey is installed.",
            .permission => "You do not have the required permission for this command. Ask a server manager or open /help for your permitted actions.",
            .voice_presence => "Join Abbey's voice channel before using this control. Use /voice status to check the current call.",
            .subject => "In a DM, use this command for your own memory only.",
            .generation => "No eligible generation route is configured for this operation. Ask a manager to check provider setup, then retry.",
            .vision => "Image description is unavailable. Ask a manager to check image-description provider setup, then retry.",
            .ocr => "Text extraction is unavailable. Ask a manager to check OCR provider setup, then retry.",
            .vision_policy => "Images are disabled for this server. Ask a manager to review /admin vision.",
            .voice_setup => "Voice is not configured here. Open /voice status for the current state and ask a manager to check setup.",
            .voice_mode => "The selected voice mode is unavailable. Open /voice status and ask a manager to check /voice mode.",
            .target => "Choose a member and an action, then run /modcall again.",
            .hierarchy => "Discord's role hierarchy prevents that action. Choose a permitted target or ask a manager to review the roles.",
            .unknown => "Current provider readiness is unknown or needs requalification. Ask a manager to check provider diagnostics, then retry.",
            .busy => "The provider is busy. Wait briefly and try again.",
            .unavailable => "The provider is temporarily unavailable. Try again later; if it persists, ask a manager to check provider diagnostics.",
        };
    }
};

pub const Availability = union(enum) {
    ready,
    access_blocked: Blocker,
    blocked: Blocker,

    pub fn message(a: Availability) []const u8 {
        return switch (a) {
            .ready => "Available to attempt; execution checks current readiness.",
            .access_blocked, .blocked => |b| b.message(),
        };
    }
};

pub const AccessRule = union(enum) {
    allow,
    permission: DiscordPermission,
    caller_present_in_voice,
    self_subject,
    application_owner,
    all: []const AccessRule,
    any: []const AccessRule,
};

pub const InputPredicate = enum { follow_up_absent, action_target_resolved };

pub const ConditionRule = union(enum) {
    always,
    available: Capability,
    guild_vision_allowed,
    input: InputPredicate,
    selected_voice_mode_ready,
    hierarchy_allows_action,
    all: []const ConditionRule,
    any: []const ConditionRule,
};

pub const AccessId = enum {
    a0,
    a1,
    a2,
    a3,
    a4,
    a5,
    a6,
    a7,
    a8,
    a9,
    a10,
    a11,

    pub fn rule(id: AccessId) AccessRule {
        return switch (id) {
            .a0 => .allow,
            .a1 => .{ .any = &.{ .self_subject, .{ .permission = .manage_messages }, .{ .permission = .manage_server }, .{ .permission = .administrator } } },
            .a2 => .{ .permission = .moderate_members },
            .a3 => .{ .permission = .manage_webhooks },
            .a4 => .{ .permission = .manage_server },
            .a5 => .{ .all = &.{ .{ .permission = .manage_server }, .caller_present_in_voice } },
            .a6 => .{ .any = &.{ .caller_present_in_voice, .{ .permission = .manage_server } } },
            .a7 => .{ .any = &.{ .application_owner, .{ .permission = .administrator } } },
            .a8 => .{ .permission = .manage_channels },
            .a9 => .{ .permission = .manage_roles },
            .a10 => .{ .permission = .move_members },
            .a11 => .{ .permission = .manage_messages },
        };
    }
};

pub const ConditionId = enum {
    c0,
    c1,
    c2,
    c3,
    c4,
    c5,
    c6,
    c7,
    c8,
    c9,

    pub fn rule(id: ConditionId) ConditionRule {
        return switch (id) {
            .c0 => .always,
            .c1 => .{ .available = .generation },
            .c2 => .{ .all = &.{ .guild_vision_allowed, .{ .available = .vision } } },
            .c3 => .{ .all = &.{ .guild_vision_allowed, .{ .available = .vision }, .{ .any = &.{ .{ .input = .follow_up_absent }, .{ .available = .generation } } } } },
            .c4 => .{ .available = .voice_configured },
            .c5 => .{ .all = &.{ .{ .available = .voice_configured }, .selected_voice_mode_ready } },
            .c6 => .{ .all = &.{ .{ .available = .voice_configured }, .{ .available = .voice_local } } },
            .c7 => .hierarchy_allows_action,
            .c8 => .{ .available = .tool_generation },
            .c9 => .{ .all = &.{ .guild_vision_allowed, .{ .available = .ocr } } },
        };
    }
};

pub const HelpSection = enum {
    start,
    conversation,
    memory,
    images,
    moderation,
    server,
    voice,
    administration,

    pub const all = [_]HelpSection{ .start, .conversation, .memory, .images, .moderation, .server, .voice, .administration };

    pub fn slug(s: HelpSection) []const u8 {
        return @tagName(s);
    }

    pub fn label(s: HelpSection) []const u8 {
        return switch (s) {
            .start => "Start",
            .conversation => "Conversation",
            .memory => "Memory",
            .images => "Images",
            .moderation => "Moderation",
            .server => "Server",
            .voice => "Voice",
            .administration => "Administration",
        };
    }

    pub fn parse(value: []const u8) ?HelpSection {
        for (all) |s| if (std.mem.eql(u8, s.slug(), value)) return s;
        return null;
    }
};

pub const RegistrationPolicy = struct {
    contexts: []const InteractionContext,
    default_member_permissions: ?DiscordPermission,
};

pub const CommandSpec = struct {
    key: specs.CommandKey,
    kind: CommandKind,
    name: []const u8,
    registration: RegistrationPolicy,
    access: AccessId,
    condition: ConditionId,
    section: HelpSection,
    description: []const u8,
    private: bool,
};

pub const both_contexts = [_]InteractionContext{ .guild, .bot_dm };
pub const guild_only = [_]InteractionContext{.guild};

pub fn registered() []const CommandSpec {
    return &specs.registered;
}

pub fn command(key: specs.CommandKey) *const CommandSpec {
    for (&specs.registered) |*spec| if (spec.key == key) return spec;
    unreachable; // exhaustive: specs.zig has one row per key (pinned by test)
}

pub fn commandByName(name: []const u8) ?*const CommandSpec {
    for (&specs.registered) |*spec| if (std.mem.eql(u8, spec.name, name)) return spec;
    return null;
}

pub const SelectedVoiceMode = enum { off, local, open_ai };

pub const Readiness = union(enum) { ready, blocked: Blocker };

pub const EligibilityInput = struct {
    context: InteractionContext,
    permissions: std.EnumSet(DiscordPermission) = .empty,
    self_subject: ?bool = null,
    application_owner: bool = false,
    caller_present_in_voice: ?bool = null,
    selected_voice_mode: SelectedVoiceMode = .off,
    // std/enums.zig: EnumArray.initFill(v)
    readiness: std.EnumArray(Capability, ?Readiness) = .initFill(null),
    vision_allowed: bool = true,
    follow_up_absent: ?bool = null,
    action_target_resolved: bool = false,
    hierarchy_allows_action: ?bool = null,
};

pub const EvaluationMode = enum { invocation, discoverability };

const max_depth = 32;
const Decision = ?Blocker; // null = allowed

fn rank(b: Blocker) u8 {
    return switch (b) {
        .target => 1,
        .permission, .subject => 2,
        else => 0,
    };
}

fn accessEval(rule: AccessRule, input: *const EligibilityInput, depth: usize) Decision {
    if (depth > max_depth) return .unavailable;
    return switch (rule) {
        .allow => null,
        .permission => |p| if (input.permissions.contains(p) or input.permissions.contains(.administrator)) null else .permission,
        .self_subject => if (input.self_subject == true) null else .permission,
        .application_owner => if (input.application_owner) null else .permission,
        .caller_present_in_voice => if (input.caller_present_in_voice == true) null else .voice_presence,
        .all => |rules| {
            if (rules.len == 0) return .unavailable;
            for (rules) |r| if (accessEval(r, input, depth + 1)) |b| return b;
            return null;
        },
        .any => |rules| {
            var failure: ?Blocker = null;
            for (rules) |r| {
                const d = accessEval(r, input, depth + 1) orelse return null;
                if (failure == null or rank(d) < rank(failure.?)) failure = d;
            }
            return failure orelse .unavailable;
        },
    };
}

pub fn accessDecision(rule: AccessRule, input: *const EligibilityInput) Decision {
    return accessEval(rule, input, 0);
}

pub fn accessAllows(rule: AccessRule, input: *const EligibilityInput) bool {
    return accessDecision(rule, input) == null;
}

fn capabilityDecision(cap: Capability, input: *const EligibilityInput) Decision {
    const observation: Readiness = input.readiness.get(cap) orelse .{ .blocked = switch (cap) {
        .generation, .tool_generation => .generation,
        .vision => .vision,
        .ocr => .ocr,
        .voice_configured => .voice_setup,
        .voice_local, .voice_open_ai => .voice_mode,
    } };
    return switch (observation) {
        .ready => null,
        .blocked => |b| b,
    };
}

fn conditionEval(rule: ConditionRule, input: *const EligibilityInput, mode: EvaluationMode, depth: usize) Decision {
    if (depth > max_depth) return .unavailable;
    return switch (rule) {
        .always => null,
        .guild_vision_allowed => if (input.vision_allowed) null else .vision_policy,
        .available => |cap| capabilityDecision(cap, input),
        .input => |pred| switch (pred) {
            .follow_up_absent => if (input.follow_up_absent == true) null else .target,
            .action_target_resolved => if (input.action_target_resolved) null else .target,
        },
        .selected_voice_mode_ready => switch (input.selected_voice_mode) {
            .off => .voice_mode,
            .local => capabilityDecision(.voice_local, input),
            .open_ai => capabilityDecision(.voice_open_ai, input),
        },
        .hierarchy_allows_action => {
            if (mode == .discoverability and !input.action_target_resolved) return null;
            if (conditionEval(.{ .input = .action_target_resolved }, input, mode, depth + 1)) |b| return b;
            return if (input.hierarchy_allows_action == true) null else .hierarchy;
        },
        .all => |rules| {
            if (rules.len == 0) return .unavailable;
            for (rules) |r| if (conditionEval(r, input, mode, depth + 1)) |b| return b;
            return null;
        },
        .any => |rules| {
            var failure: ?Blocker = null;
            for (rules) |r| {
                const d = conditionEval(r, input, mode, depth + 1) orelse return null;
                if (failure == null or rank(d) < rank(failure.?)) failure = d;
            }
            return failure orelse .unavailable;
        },
    };
}

pub fn conditionDecision(rule: ConditionRule, input: *const EligibilityInput, mode: EvaluationMode) Decision {
    return conditionEval(rule, input, mode, 0);
}

fn containsContext(list: []const InteractionContext, c: InteractionContext) bool {
    for (list) |x| if (x == c) return true;
    return false;
}

pub fn availability(spec: *const CommandSpec, input: *const EligibilityInput) Availability {
    if (!containsContext(spec.registration.contexts, input.context)) return .{ .access_blocked = .context };
    if (input.context == .bot_dm and spec.access == .a1 and input.self_subject != true) return .{ .access_blocked = .subject };
    if (accessDecision(spec.access.rule(), input)) |b| return .{ .access_blocked = b };
    if (conditionDecision(spec.condition.rule(), input, .invocation)) |b| return .{ .blocked = b };
    return .ready;
}

pub fn eligible(spec: *const CommandSpec, input: *const EligibilityInput, mode: EvaluationMode) bool {
    const result = availability(spec, input);
    return switch (mode) {
        .invocation => result == .ready,
        .discoverability => result != .access_blocked,
    };
}

pub fn invocationHint(kind: CommandKind) []const u8 {
    return switch (kind) {
        .slash => "slash command",
        .user_context => "member menu",
        .message_context => "message menu",
    };
}

pub fn visibilityHint(private: bool, context: InteractionContext) []const u8 {
    if (private) return "private";
    if (context == .bot_dm) return "reply in this DM";
    return "channel-visible";
}

/// The `/help` section body.
pub fn renderHelp(gpa: Allocator, section: HelpSection, input: *const EligibilityInput) Allocator.Error![]u8 {
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(gpa);
    try out.print(gpa, "**Abbey \u{b7} {s}**\nChoose a section. Commands below are permitted here; execution rechecks readiness.\n\n", .{section.label()});
    var count: usize = 0;
    var member_menu = false;
    var message_menu = false;
    for (&specs.registered) |*spec| {
        if (spec.section != section or !eligible(spec, input, .discoverability)) continue;
        const prefix: []const u8 = if (spec.kind == .slash) "/" else "";
        const what = switch (availability(spec, input)) {
            .blocked => |b| b.label(),
            else => spec.description,
        };
        try out.print(gpa, "`{s}{s}` ({s}; {s}) \u{b7} {s}\n", .{ prefix, spec.name, invocationHint(spec.kind), visibilityHint(spec.private, input.context), what });
        member_menu = member_menu or spec.kind == .user_context;
        message_menu = message_menu or spec.kind == .message_context;
        count += 1;
    }
    if (count == 0) try out.appendSlice(gpa, "No commands in this section are currently available to you.\n");
    if (section == .start) try out.appendSlice(gpa, "\nTask buttons open private workflows. Use the section menu for the command reference.\n");
    if (member_menu and message_menu) {
        try out.appendSlice(gpa, "\nOpen the member or message menu, then Apps, for its listed actions.\n");
    } else if (member_menu) {
        try out.appendSlice(gpa, "\nOpen the member menu, then Apps, for its listed actions.\n");
    } else if (message_menu) {
        try out.appendSlice(gpa, "\nOpen the message menu, then Apps, for its listed actions.\n");
    }
    try out.appendSlice(gpa, "\nProvider health is checked when you run a command. Controls expire 15 minutes after opening; `/help` starts a new private session.");
    return out.toOwnedSlice(gpa);
}

/// The README command table the oracle generates (`render_readme`).
pub fn renderReadme(gpa: Allocator) Allocator.Error![]u8 {
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(gpa);
    try out.appendSlice(gpa, "<!-- BEGIN GENERATED COMMAND CATALOG -->\n| Command | Context | Response | What it does |\n|---|---|---|---|\n");
    for (&specs.registered) |*spec| {
        const prefix: []const u8 = if (spec.kind == .slash) "/" else "";
        const context: []const u8 = if (spec.registration.contexts.len == 2) "guild, bot DM" else "guild";
        const visibility: []const u8 = if (spec.private) "private" else "public";
        try out.print(gpa, "| `{s}{s}` | {s} | {s} | {s} |\n", .{ prefix, spec.name, context, visibility, spec.description });
    }
    try out.appendSlice(gpa, "\nThe member voice status, typed voice-mode choices, manager diagnostics, and classic administration dashboard are registered surfaces.\n<!-- END GENERATED COMMAND CATALOG -->");
    return out.toOwnedSlice(gpa);
}

/// Discord's 2,000-character message cap with the oracle's truncation marker.
pub const discord_message_cap: usize = 2000;

pub fn clampMessage(gpa: Allocator, body: []const u8) Allocator.Error![]u8 {
    const text = @import("../text/text.zig");
    const marker = "\n\u{2026} (truncated to fit Discord's 2,000-character limit)";
    if (text.charCount(body) <= discord_message_cap) return gpa.dupe(u8, body);
    const keep = discord_message_cap - text.charCount(marker);
    return std.fmt.allocPrint(gpa, "{s}{s}", .{ body[0..text.byteOffsetOfChar(body, keep)], marker });
}

test "nested alternatives prefer operational recovery over missing input" {
    const input: EligibilityInput = .{ .context = .guild };
    const rule: ConditionRule = .{ .any = &.{ .{ .all = &.{.{ .input = .action_target_resolved }} }, .{ .available = .generation } } };
    try std.testing.expectEqual(@as(Decision, .generation), conditionDecision(rule, &input, .invocation));
}

test "clamp keeps 2,000 scalars and appends the marker" {
    const gpa = std.testing.allocator;
    const text = @import("../text/text.zig");
    var long: std.ArrayList(u8) = .empty;
    defer long.deinit(gpa);
    for (0..2500) |_| try long.appendSlice(gpa, "\u{e9}");
    const out = try clampMessage(gpa, long.items);
    defer gpa.free(out);
    try std.testing.expect(text.charCount(out) <= discord_message_cap);
    try std.testing.expect(std.mem.endsWith(u8, out, "limit)"));
}
