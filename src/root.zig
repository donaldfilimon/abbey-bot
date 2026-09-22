//! abbey-bot-zig library root. Every subsystem is reachable from here so
//! `refAllDecls` compiles and runs every test in the tree.
const std = @import("std");

pub const text = @import("text/text.zig");
pub const decimal = @import("text/decimal.zig");
pub const ryu_style = @import("text/ryu_style.zig");
pub const embedding = @import("memory/embedding.zig");
pub const wdbx_segment = @import("memory/wdbx_segment.zig");
pub const wdbx_golden_test = @import("memory/wdbx_golden_test.zig");
pub const wdbx_bridge = @import("memory/wdbx_bridge.zig");
pub const wdbx_interop = @import("memory/wdbx_interop.zig");
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
pub const catalog = @import("catalog/catalog.zig");
pub const catalog_serialize = @import("catalog/serialize.zig");
pub const json_pretty = @import("json/pretty.zig");
pub const contracts_corpus = @import("contracts/corpus.zig");
pub const catalog_golden_test = @import("catalog/golden_test.zig");
pub const ws = @import("gateway/ws.zig");
pub const gateway_session = @import("gateway/session.zig");
pub const http = @import("net/http.zig");
pub const gateway_conn = @import("gateway/conn.zig");
pub const gateway_probe = @import("gateway/probe.zig");
pub const ratelimit = @import("discord/ratelimit.zig");
pub const rest = @import("discord/rest.zig");
pub const llm = @import("llm/openai.zig");
pub const tls_test = @import("net/tls_test.zig");

pub const version = "0.1.0";

test {
    std.testing.refAllDecls(@This());
}
