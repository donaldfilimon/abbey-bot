//! The frozen registration payload: every top-level command with its
//! options, transcribed from the oracle's own export
//! (contracts/catalog/command-payload.json, poise create_application_commands).
//! `serialize.zig` renders it byte-identically; a parity test pins it.
const p = @import("payload_types.zig");

pub const commands = [_]p.Command{
    .{ .name = "help", .description = "Browse commands available here using private, owner-bound controls.", .contexts = &.{ 0, 1 }, .options = &.{
        .{ .kind = 4, .name = "section", .description = "Which commands to browse", .choices = &.{
            .{ .name = "Start", .value = 0 },
            .{ .name = "Conversation", .value = 1 },
            .{ .name = "Memory", .value = 2 },
            .{ .name = "Images", .value = 3 },
            .{ .name = "Moderation", .value = 4 },
            .{ .name = "Server", .value = 5 },
            .{ .name = "Voice", .value = 6 },
            .{ .name = "Administration", .value = 7 },
        } },
    } },
    .{ .name = "persona", .description = "Abbey's persona surface: `/persona route` and `/persona ask`.", .contexts = &.{ 0, 1 }, .options = &.{
        .{ .kind = 1, .name = "route", .description = "Show which persona takes a request, and why.", .options = &.{
            .{ .kind = 3, .name = "request", .description = "What you want help with", .required = true },
            .{ .kind = 4, .name = "as", .description = "Force a persona instead of routing", .choices = &.{
                .{ .name = "abbey — warm sharp friend and default", .value = 0 },
                .{ .name = "aviva — concise direct expert", .value = 1 },
                .{ .name = "abi — orchestration and governance", .value = 2 },
            } },
        } },
        .{ .kind = 1, .name = "ask", .description = "A slash command", .options = &.{
            .{ .kind = 3, .name = "question", .description = "What you want to know", .required = true, .max_length = 2000, .autocomplete = true },
            .{ .kind = 4, .name = "as", .description = "Force a persona instead of routing", .choices = &.{
                .{ .name = "abbey — warm sharp friend and default", .value = 0 },
                .{ .name = "aviva — concise direct expert", .value = 1 },
                .{ .name = "abi — orchestration and governance", .value = 2 },
            } },
        } },
    } },
    .{ .name = "whois", .description = "Read a member's profile.", .contexts = &.{0}, .options = &.{
        .{ .kind = 6, .name = "user", .description = "Who to read", .required = true },
    } },
    .{ .name = "Abbey: profile", .kind = 2, .contexts = &.{0} },
    .{ .name = "Ask Abbey", .kind = 3, .contexts = &.{ 0, 1 } },
    .{ .name = "Abbey: memory", .kind = 2, .contexts = &.{0} },
    .{ .name = "Abbey: describe image", .kind = 3, .contexts = &.{ 0, 1 } },
    .{ .name = "Abbey: read image text", .kind = 3, .contexts = &.{ 0, 1 } },
    .{ .name = "perms", .description = "Walk through how a channel's permission overwrites resolve for a member.", .contexts = &.{0}, .options = &.{
        .{ .kind = 7, .name = "channel", .description = "Which channel", .required = true },
        .{ .kind = 6, .name = "user", .description = "Which member", .required = true },
    } },
    .{ .name = "modcall", .description = "Recommend a moderation action, and say whether you can actually take it.", .default_member_permissions = "1099511627776", .contexts = &.{0}, .options = &.{
        .{ .kind = 6, .name = "user", .description = "Who the incident is about", .required = true },
        .{ .kind = 4, .name = "severity", .description = "How bad it is", .required = true, .choices = &.{
            .{ .name = "minor — rudeness, mild spam, derailing", .value = 0 },
            .{ .name = "serious — harassment, slurs, deliberate disruption", .value = 1 },
            .{ .name = "severe — threats, doxxing, raiding; bans on the first offence", .value = 2 },
        } },
        .{ .kind = 4, .name = "warnings", .description = "Prior warnings on record (default 0)", .min_value = 0.0, .max_value = 255.0 },
        .{ .kind = 4, .name = "timeouts", .description = "Prior timeouts on record (default 0)", .min_value = 0.0, .max_value = 255.0 },
    } },
    .{ .name = "server", .description = "Parent — Discord forces a subcommand; body is unreachable wiring.", .contexts = &.{ 0, 1 }, .options = &.{
        .{ .kind = 1, .name = "blueprint", .description = "Produce a server blueprint: role hierarchy, channel structure, numbered steps.", .options = &.{
            .{ .kind = 4, .name = "kind", .description = "What kind of server", .required = true, .choices = &.{
                .{ .name = "community — public and open-join, rules gate, moderation depth", .value = 0 },
                .{ .name = "gaming — voice-first group", .value = 1 },
                .{ .name = "project — work server, structured and low noise", .value = 2 },
                .{ .name = "friend group — small and deliberately flat", .value = 3 },
            } },
        } },
        .{ .kind = 1, .name = "create-channel", .description = "Create a text channel (hidden from @everyone until you set overwrites yourself).", .options = &.{
            .{ .kind = 3, .name = "name", .description = "Channel name", .required = true },
            .{ .kind = 7, .name = "category", .description = "Optional category to place it under" },
            .{ .kind = 3, .name = "topic", .description = "Optional topic" },
        } },
        .{ .kind = 1, .name = "rename-channel", .description = "Rename a channel.", .options = &.{
            .{ .kind = 7, .name = "channel", .description = "Channel to rename", .required = true },
            .{ .kind = 3, .name = "name", .description = "New name", .required = true },
        } },
        .{ .kind = 1, .name = "slowmode", .description = "Set text-channel slowmode (0–21600 seconds).", .options = &.{
            .{ .kind = 7, .name = "channel", .description = "Text channel", .required = true },
            .{ .kind = 4, .name = "seconds", .description = "Seconds (0 disables)", .required = true, .min_value = -9007199254740991.0, .max_value = 9007199254740991.0 },
        } },
        .{ .kind = 1, .name = "delete-channel", .description = "Delete a channel. Requires `confirm:true` (non-destructive by default).", .options = &.{
            .{ .kind = 7, .name = "channel", .description = "Channel to delete", .required = true },
            .{ .kind = 5, .name = "confirm", .description = "Must be true to proceed", .required = true },
        } },
        .{ .kind = 1, .name = "assign-role", .description = "Assign a role to a member.", .options = &.{
            .{ .kind = 6, .name = "user", .description = "Member", .required = true },
            .{ .kind = 8, .name = "role", .description = "Role to add", .required = true },
        } },
        .{ .kind = 1, .name = "remove-role", .description = "Remove a role from a member.", .options = &.{
            .{ .kind = 6, .name = "user", .description = "Member", .required = true },
            .{ .kind = 8, .name = "role", .description = "Role to remove", .required = true },
        } },
        .{ .kind = 1, .name = "move-member", .description = "Move a member to another voice channel (both need Move Members).", .options = &.{
            .{ .kind = 6, .name = "user", .description = "Member in voice", .required = true },
            .{ .kind = 7, .name = "channel", .description = "Destination voice channel", .required = true },
        } },
        .{ .kind = 1, .name = "purge", .description = "Bulk-delete recent messages. Requires `confirm:true` and Manage Messages.", .options = &.{
            .{ .kind = 7, .name = "channel", .description = "Channel to purge", .required = true },
            .{ .kind = 4, .name = "count", .description = "How many messages (2–100)", .required = true, .min_value = -9007199254740991.0, .max_value = 9007199254740991.0 },
            .{ .kind = 5, .name = "confirm", .description = "Must be true to proceed", .required = true },
        } },
    } },
    .{ .name = "webhook", .description = "Emit the incoming-webhook setup guide for a channel.", .default_member_permissions = "536870912", .contexts = &.{0}, .options = &.{
        .{ .kind = 7, .name = "channel", .description = "Where the webhook should post", .required = true },
    } },
    .{ .name = "forum", .description = "Parent — Discord forces a subcommand; this body is unreachable wiring.", .contexts = &.{0}, .options = &.{
        .{ .kind = 1, .name = "draft", .description = "Ephemeral tag suggestions + first-post template (no channel mutate).", .options = &.{
            .{ .kind = 3, .name = "title", .description = "Draft title for tag matching", .required = true },
            .{ .kind = 7, .name = "channel", .description = "Forum channel (defaults to #help)" },
            .{ .kind = 3, .name = "details", .description = "Optional details for the template body" },
            .{ .kind = 4, .name = "template", .description = "First-post template", .choices = &.{
                .{ .name = "Question", .value = 0 },
                .{ .name = "Bug", .value = 1 },
                .{ .name = "Build", .value = 2 },
                .{ .name = "General", .value = 3 },
            } },
        } },
        .{ .kind = 1, .name = "post", .description = "Create a `#help` (or chosen forum) post via Discord's forum-thread API.", .options = &.{
            .{ .kind = 3, .name = "title", .description = "Post title (2–100 characters)", .required = true },
            .{ .kind = 7, .name = "channel", .description = "Forum channel (defaults to #help)" },
            .{ .kind = 3, .name = "details", .description = "Details for the first post" },
            .{ .kind = 4, .name = "template", .description = "First-post template", .choices = &.{
                .{ .name = "Question", .value = 0 },
                .{ .name = "Bug", .value = 1 },
                .{ .name = "Build", .value = 2 },
                .{ .name = "General", .value = 3 },
            } },
            .{ .kind = 3, .name = "tags", .description = "Comma-separated tag names (default: auto-suggest)" },
        } },
        .{ .kind = 1, .name = "perms", .description = "Snapshot the bot's forum overwrite, then gap-fill missing required bits only.", .options = &.{
            .{ .kind = 7, .name = "channel", .description = "Forum channel (defaults to #help)" },
            .{ .kind = 5, .name = "apply", .description = "When true, PUT the gap-filled bot member overwrite" },
        } },
    } },
    .{ .name = "remember", .description = "Store a durable fact about a member (yourself by default).", .contexts = &.{ 0, 1 }, .options = &.{
        .{ .kind = 3, .name = "fact", .description = "A single concise fact, stated in third person", .required = true, .max_length = 300 },
        .{ .kind = 6, .name = "user", .description = "Who it is about (default: you; moderators may choose another member)" },
        .{ .kind = 3, .name = "replaces", .description = "An existing fact this replaces — it is removed only because you said so", .max_length = 300, .autocomplete = true },
    } },
    .{ .name = "forget", .description = "Forget one of your stored facts.", .contexts = &.{ 0, 1 }, .options = &.{
        .{ .kind = 3, .name = "fact", .description = "The fact to remove", .required = true, .autocomplete = true },
        .{ .kind = 6, .name = "user", .description = "Who it is about (default: you; moderators may choose another member)" },
    } },
    .{ .name = "pending", .description = "Review or resolve supersessions the model proposed but never applied.", .contexts = &.{ 0, 1 }, .options = &.{
        .{ .kind = 1, .name = "list", .description = "Show supersessions proposed for a member, with nothing removed yet.", .options = &.{
            .{ .kind = 6, .name = "user", .description = "Who to review (default: you; moderators may choose another member)" },
        } },
        .{ .kind = 1, .name = "confirm", .description = "Apply one proposed supersession, removing the old fact.", .options = &.{
            .{ .kind = 3, .name = "old_fact", .description = "The old fact to remove", .required = true, .autocomplete = true },
            .{ .kind = 6, .name = "user", .description = "Who it is about (default: you; moderators may choose another member)" },
        } },
        .{ .kind = 1, .name = "dismiss", .description = "Drop one proposed supersession, keeping both facts.", .options = &.{
            .{ .kind = 3, .name = "old_fact", .description = "The old fact to keep", .required = true, .autocomplete = true },
            .{ .kind = 6, .name = "user", .description = "Who it is about (default: you; moderators may choose another member)" },
        } },
    } },
    .{ .name = "recall", .description = "What Abbey remembers about a member, and their standing.", .contexts = &.{ 0, 1 }, .options = &.{
        .{ .kind = 6, .name = "user", .description = "Who to look up (default: you; moderators may choose another member)" },
    } },
    .{ .name = "reputation", .description = "Your standing privately, or another member when authorized.", .contexts = &.{ 0, 1 }, .options = &.{
        .{ .kind = 6, .name = "user", .description = "Who to look up (default: you)" },
    } },
    .{ .name = "summarize", .description = "Summarize the recent messages Abbey has seen in this channel.", .contexts = &.{ 0, 1 }, .options = &.{
        .{ .kind = 4, .name = "count", .description = "How many recent messages (10–200, default 50)", .min_value = 10.0, .max_value = 200.0 },
        .{ .kind = 4, .name = "as", .description = "Force a persona", .choices = &.{
            .{ .name = "abbey — warm sharp friend and default", .value = 0 },
            .{ .name = "aviva — concise direct expert", .value = 1 },
            .{ .name = "abi — orchestration and governance", .value = 2 },
        } },
    } },
    .{ .name = "see", .description = "Describe an image — and answer a question about it if you ask one.", .contexts = &.{ 0, 1 }, .options = &.{
        .{ .kind = 11, .name = "image", .description = "The image", .required = true },
        .{ .kind = 3, .name = "question", .description = "Something to ask about it" },
    } },
    .{ .name = "ocr", .description = "Transcribe the text in an image.", .contexts = &.{ 0, 1 }, .options = &.{
        .{ .kind = 11, .name = "image", .description = "The image", .required = true },
    } },
    .{ .name = "stats", .description = "Learning and reply-budget statistics for this server or your DM.", .contexts = &.{ 0, 1 } },
    .{ .name = "admin", .description = "Configure Abbey for this server.", .default_member_permissions = "32", .contexts = &.{0}, .options = &.{
        .{ .kind = 1, .name = "show", .description = "Show current settings with a classic page select into the admin dashboard." },
        .{ .kind = 1, .name = "persona", .description = "Set the default persona for this server.", .options = &.{
            .{ .kind = 4, .name = "name", .description = "Who answers by default", .required = true, .choices = &.{
                .{ .name = "abbey — warm sharp friend and default", .value = 0 },
                .{ .name = "aviva — concise direct expert", .value = 1 },
                .{ .name = "abi — orchestration and governance", .value = 2 },
            } },
        } },
        .{ .kind = 1, .name = "learning", .description = "Toggle adaptive learning (the DQN) for this server.", .options = &.{
            .{ .kind = 4, .name = "state", .description = "on | off", .required = true, .choices = &.{
                .{ .name = "on", .value = 0 },
                .{ .name = "off", .value = 1 },
            } },
        } },
        .{ .kind = 1, .name = "nsfw", .description = "Toggle Aviva `/roleplay` for this server (NSFW channels only when on).", .options = &.{
            .{ .kind = 4, .name = "state", .description = "on | off", .required = true, .choices = &.{
                .{ .name = "on", .value = 0 },
                .{ .name = "off", .value = 1 },
            } },
        } },
        .{ .kind = 1, .name = "vision", .description = "Toggle image understanding for this server.", .options = &.{
            .{ .kind = 4, .name = "state", .description = "on | off", .required = true, .choices = &.{
                .{ .name = "on", .value = 0 },
                .{ .name = "off", .value = 1 },
            } },
        } },
        .{ .kind = 1, .name = "cooldown", .description = "Minimum seconds between unsolicited replies in a channel (0–600).", .options = &.{
            .{ .kind = 4, .name = "seconds", .description = "0–600", .required = true, .min_value = -9007199254740991.0, .max_value = 9007199254740991.0 },
        } },
        .{ .kind = 1, .name = "act", .description = "Opt Abbey into unsolicited replies in this server (not guild mutations — use `/server …` for those).", .options = &.{
            .{ .kind = 4, .name = "state", .description = "on | off", .required = true, .choices = &.{
                .{ .name = "on", .value = 0 },
                .{ .name = "off", .value = 1 },
            } },
        } },
        .{ .kind = 1, .name = "budget", .description = "Unsolicited actions allowed per hour in this server (1–60).", .options = &.{
            .{ .kind = 4, .name = "per_hour", .description = "1–60", .required = true, .min_value = -9007199254740991.0, .max_value = 9007199254740991.0 },
        } },
        .{ .kind = 1, .name = "brain", .description = "Inspect this server's policy: ε, steps, buffer fill, experiences.", .options = &.{
            .{ .kind = 10, .name = "epsilon", .description = "Override exploration ε (0–1); omit to show" },
        } },
        .{ .kind = 1, .name = "flush", .description = "A slash command" },
        .{ .kind = 1, .name = "export", .description = "Export this server's brain snapshot as JSON." },
        .{ .kind = 1, .name = "reset", .description = "Reset this channel's conversation memory (the multi-turn transcript)." },
        .{ .kind = 1, .name = "dashboard", .description = "Open the owner- and guild-bound classic administration dashboard." },
        .{ .kind = 1, .name = "quarantine", .description = "Mark a member's stored fact as suspect in the ledger. It stays visible.", .options = &.{
            .{ .kind = 6, .name = "member", .description = "Whose fact it is", .required = true },
            .{ .kind = 3, .name = "fact", .description = "The fact, as stored", .required = true, .max_length = 300 },
            .{ .kind = 4, .name = "reason", .description = "Why it is suspect", .required = true, .choices = &.{
                .{ .name = "operator report", .value = 0 },
                .{ .name = "source untrusted", .value = 1 },
                .{ .name = "policy violation", .value = 2 },
                .{ .name = "superseded by newer evidence", .value = 3 },
            } },
        } },
        .{ .kind = 1, .name = "contradict", .description = "Record that two of a member's stored facts contradict each other. Both stay.", .options = &.{
            .{ .kind = 6, .name = "member", .description = "Whose facts they are", .required = true },
            .{ .kind = 3, .name = "fact", .description = "One fact, as stored", .required = true, .max_length = 300 },
            .{ .kind = 3, .name = "counterpart", .description = "The fact it contradicts, as stored", .required = true, .max_length = 300 },
        } },
        .{ .kind = 1, .name = "resolve", .description = "Close an open memory review with a verdict. Deletes nothing.", .options = &.{
            .{ .kind = 3, .name = "edge", .description = "The edge digest from the quarantine reply", .required = true, .min_length = 64, .max_length = 64 },
            .{ .kind = 4, .name = "verdict", .description = "Was the fact valid?", .required = true, .choices = &.{
                .{ .name = "valid", .value = 0 },
                .{ .name = "invalid", .value = 1 },
            } },
        } },
    } },
    .{ .name = "nsfw", .description = "Toggle Aviva `/roleplay` in this DM (personal one-person guild settings).", .contexts = &.{ 0, 1 }, .options = &.{
        .{ .kind = 4, .name = "state", .description = "on | off", .required = true, .choices = &.{
            .{ .name = "on", .value = 0 },
            .{ .name = "off", .value = 1 },
        } },
    } },
    .{ .name = "roleplay", .description = "Roleplay as Aviva when the NSFW gate allows it (bot DM or NSFW guild channel).", .contexts = &.{ 0, 1 }, .options = &.{
        .{ .kind = 3, .name = "prompt", .description = "What you want from Aviva", .max_length = 2000 },
    } },
    .{ .name = "voice", .description = "Voice listen (consent), status, and macOS music mirror (`play` / `pause` / `stop-music`).", .contexts = &.{0}, .options = &.{
        .{ .kind = 1, .name = "play", .description = "Mirror host Spotify/Music into Abbey's voice channel (macOS tap). Does not grant listen consent.", .options = &.{
            .{ .kind = 3, .name = "query", .description = "Spotify URI or Music search; omit to mirror what is already playing" },
            .{ .kind = 4, .name = "player", .description = "Host player (default Spotify)", .choices = &.{
                .{ .name = "Spotify", .value = 0 },
                .{ .name = "Music", .value = 1 },
            } },
        } },
        .{ .kind = 1, .name = "pause", .description = "Pause mirrored music and close host capture. Listening consent is unchanged." },
        .{ .kind = 1, .name = "resume-music", .description = "Resume mirrored music only. Never renews listen consent." },
        .{ .kind = 1, .name = "stop-music", .description = "Stop host capture and mirrored playback. Listening consent is unchanged." },
        .{ .kind = 1, .name = "volume", .description = "Set mirrored music volume (0–100). Abbey speech ducks music to one quarter.", .options = &.{
            .{ .kind = 4, .name = "level", .description = "Music level from 0 to 100", .required = true, .min_value = 0.0, .max_value = 100.0 },
        } },
        .{ .kind = 1, .name = "join", .description = "Start the configured voice backend after everyone present was notified.", .options = &.{
            .{ .kind = 5, .name = "consent", .description = "Confirm everyone present was notified and consented", .required = true },
        } },
        .{ .kind = 1, .name = "resume", .description = "Resume after a new participant was notified and consented.", .options = &.{
            .{ .kind = 5, .name = "consent", .description = "Confirm everyone now present was notified and consented", .required = true },
        } },
        .{ .kind = 1, .name = "leave", .description = "Stop processing synchronously and leave Discord voice." },
        .{ .kind = 1, .name = "status", .description = "Show the member-safe voice state without operational diagnostics." },
        .{ .kind = 1, .name = "diagnostics", .description = "Show content-free operational detail to current server managers." },
        .{ .kind = 2, .name = "verify", .description = "Arm or read a content-free live acceptance run.", .options = &.{
            .{ .kind = 1, .name = "start", .description = "Start one local, content-free acceptance run before the consented join." },
            .{ .kind = 1, .name = "report", .description = "Render the current redacted acceptance report without ending the run." },
        } },
        .{ .kind = 1, .name = "mode", .description = "Show or change the voice backend in force. Requires MANAGE_GUILD.", .options = &.{
            .{ .kind = 4, .name = "mode", .description = "Off or Local. Omit to show the current mode.", .choices = &.{
                .{ .name = "Off", .value = 0 },
                .{ .name = "Local", .value = 1 },
            } },
        } },
        .{ .kind = 1, .name = "consent", .description = "Review your saved voice choice and agree or withdraw privately." },
        .{ .kind = 1, .name = "notice", .description = "Post the current member choice notice in the configured voice channel." },
    } },
};
