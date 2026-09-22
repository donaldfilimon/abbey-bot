//! abbey-bot-zig library root. Every subsystem is reachable from here so
//! `refAllDecls` compiles and runs every test in the tree.
const std = @import("std");

pub const text = @import("text/text.zig");
pub const decimal = @import("text/decimal.zig");
pub const persona = @import("persona/persona.zig");
pub const signals = @import("persona/signals.zig");
pub const prompts = @import("persona/prompts.zig");
pub const roleplay = @import("persona/roleplay.zig");
pub const persona_golden_test = @import("persona/golden_test.zig");
pub const grounding = @import("grounding/grounding.zig");
pub const grounding_golden_test = @import("grounding/golden_test.zig");
pub const recall = @import("memory/recall.zig");
pub const memory_bank = @import("memory/bank.zig");
pub const memory_context = @import("memory/context.zig");
pub const memory_golden_test = @import("memory/golden_test.zig");
pub const engine = @import("engine/engine.zig");

pub const version = "0.1.0";

test {
    std.testing.refAllDecls(@This());
}
