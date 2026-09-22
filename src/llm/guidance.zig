//! Operational capability context appended to every generation's system
//! prompt, transcribed from the oracle's `generation/capability_guidance.rs`.
//! Phase 1 offers no model tools, so every round is the "no callable tools"
//! round.
const std = @import("std");
const Allocator = std.mem.Allocator;

const no_tools = "No callable tools are offered in this round. Answer from the available conversation and evidence; do not claim to have inspected or changed external state.";
const discord_note = "\nYou are integrated with Discord. Prefer `/help` for the private task home. Supported guild mutations go through `/server \u{2026}` with permission-mirroring (both the asking member and Abbey must hold the Discord permission)\u{2014}e.g. create/rename/slowmode/delete-channel, assign/remove-role, move-member, purge with confirm. Voice/music uses `/voice \u{2026}` (listening needs explicit consent; music never grants consent). Memory/persona/admin use `/remember`, `/persona`, `/admin \u{2026}`. Chat tools still do not invent custom slash commands or send unsolicited member DMs. Point people at the exact supported slash next step instead of claiming every Discord action is available or denying that you are a bot. Voice presence is not listening.";
const learning_note = "\nWhen the topic is learning, the guild policy loop, DQN, epsilon, act/budget, or self-improvement: point operators at `/admin act`, `/admin learning`, `/admin brain`, and `/admin budget`. Learning updates the in-process per-guild DQN from settled rewards and does not rewrite Abbey's source code or promise autonomous self-rewrite.";

const learning_keys = [_][]const u8{ "learning", "self-improv", "self improv", "dqn", "epsilon", "replay", "step_count", "step count", "unsolicited", "/admin act", "/admin learning", "/admin brain", "/admin budget", "guild policy", "policy loop" };

fn learningTopic(buf: []u8, user_input: []const u8) bool {
    const n = @min(buf.len, user_input.len);
    const lowered = std.ascii.lowerString(buf[0..n], user_input[0..n]);
    for (learning_keys) |k| if (std.mem.indexOf(u8, lowered, k) != null) return true;
    return false;
}

pub fn systemPrompt(gpa: Allocator, base: []const u8, scope: []const u8, user_input: []const u8) Allocator.Error![]u8 {
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(gpa);
    try out.print(gpa, "{s}\n\nOperational capability context (not conversation evidence):\n{s}\nDo not claim an action completed without its successful result. Pending, refused, partial and uncertain effects are not completed actions. Do not invent commands or promise future work that has not been scheduled.", .{ base, no_tools });
    if (std.mem.startsWith(u8, scope, "discord:")) {
        try out.appendSlice(gpa, discord_note);
        const buf = try gpa.alloc(u8, user_input.len);
        defer gpa.free(buf);
        if (learningTopic(buf, user_input)) try out.appendSlice(gpa, learning_note);
    }
    return out.toOwnedSlice(gpa);
}

test "guidance: no tools, Discord controls, and learning pointers only on topic" {
    const gpa = std.testing.allocator;
    const plain = try systemPrompt(gpa, "Persona", "discord:42", "how do I rename a channel?");
    defer gpa.free(plain);
    try std.testing.expect(std.mem.startsWith(u8, plain, "Persona\n\n"));
    try std.testing.expect(std.mem.indexOf(u8, plain, "No callable tools") != null);
    try std.testing.expect(std.mem.indexOf(u8, plain, "/help") != null);
    try std.testing.expect(std.mem.indexOf(u8, plain, "/admin act") == null);
    const learning = try systemPrompt(gpa, "Persona", "discord:42", "Is Abbey self-improving / learning from the guild DQN?");
    defer gpa.free(learning);
    try std.testing.expect(std.mem.indexOf(u8, learning, "does not rewrite Abbey's source code") != null);
    const slack = try systemPrompt(gpa, "Persona", "slack:42", "tell me about learning");
    defer gpa.free(slack);
    try std.testing.expect(std.mem.indexOf(u8, slack, "/help") == null);
}
