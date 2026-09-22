//! `/roleplay` admission, transcribed from the oracle's `src/roleplay_gate.rs`:
//! Aviva only in bot DMs or NSFW guild channels, and only while the durable
//! gate is enabled. Fail-closed: disabled is the default.
const Persona = @import("persona.zig").Persona;
const std = @import("std");

pub const Context = union(enum) {
    bot_dm,
    guild: struct { channel_nsfw: bool },
};

pub const Decision = enum {
    allow_aviva,
    refuse_disabled,
    refuse_guild_sfw,

    pub fn allow(d: Decision) bool {
        return d == .allow_aviva;
    }

    pub fn persona(d: Decision) ?Persona {
        return if (d == .allow_aviva) .aviva else null;
    }

    pub fn message(d: Decision) []const u8 {
        return switch (d) {
            .allow_aviva => "Aviva roleplay is available here. I'll answer as Aviva.",
            .refuse_disabled => "Roleplay is disabled here. An operator can enable it with `/admin nsfw on` in a server, or `/nsfw on` in a DM with me.",
            .refuse_guild_sfw => "Roleplay (Aviva) is only available in NSFW channels or in a DM with me \u{2014} not in SFW server channels. Abbey stays SFW here.",
        };
    }
};

pub fn decide(context: Context, enabled: bool) Decision {
    return switch (context) {
        .bot_dm => if (enabled) .allow_aviva else .refuse_disabled,
        .guild => |g| if (!g.channel_nsfw) .refuse_guild_sfw else if (enabled) .allow_aviva else .refuse_disabled,
    };
}

test "behavior matrix matches the product gate" {
    try std.testing.expectEqual(Decision.allow_aviva, decide(.bot_dm, true));
    try std.testing.expectEqual(Decision.refuse_disabled, decide(.bot_dm, false));
    try std.testing.expectEqual(Decision.allow_aviva, decide(.{ .guild = .{ .channel_nsfw = true } }, true));
    try std.testing.expectEqual(Decision.refuse_disabled, decide(.{ .guild = .{ .channel_nsfw = true } }, false));
    try std.testing.expectEqual(Decision.refuse_guild_sfw, decide(.{ .guild = .{ .channel_nsfw = false } }, true));
    try std.testing.expectEqual(Decision.refuse_guild_sfw, decide(.{ .guild = .{ .channel_nsfw = false } }, false));
    try std.testing.expectEqual(@as(?Persona, .aviva), Decision.allow_aviva.persona());
    try std.testing.expectEqual(@as(?Persona, null), Decision.refuse_disabled.persona());
}
