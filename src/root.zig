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
pub const memory_store = @import("memory/store.zig");
pub const memory_service = @import("memory/service.zig");
pub const memory_gate = @import("memory/gate.zig");
pub const engine = @import("engine/engine.zig");
pub const catalog = @import("catalog/catalog.zig");
pub const catalog_serialize = @import("catalog/serialize.zig");
pub const json_pretty = @import("json/pretty.zig");
pub const contracts_corpus = @import("contracts/corpus.zig");
pub const episode_config = @import("episode/config.zig");
pub const episode_write = @import("episode/write.zig");
pub const episode_propose = @import("episode/propose.zig");
pub const episode_golden_test = @import("episode/golden_test.zig");
pub const episode_checkpoint = @import("episode/checkpoint.zig");
pub const moderation = @import("moderation/moderation.zig");
pub const app = @import("app/app.zig");
pub const app_reply = @import("app/reply.zig");
pub const app_handlers = @import("app/handlers.zig");
pub const app_modcall = @import("app/modcall.zig");
pub const app_registration = @import("app/registration.zig");
pub const app_handlers_test = @import("app/handlers_test.zig");
pub const catalog_golden_test = @import("catalog/golden_test.zig");
pub const help_session = @import("catalog/help_session.zig");
pub const ws = @import("gateway/ws.zig");
pub const gateway_session = @import("gateway/session.zig");
pub const http = @import("net/http.zig");
pub const gateway_conn = @import("gateway/conn.zig");
pub const gateway_probe = @import("gateway/probe.zig");
pub const ratelimit = @import("discord/ratelimit.zig");
pub const rest = @import("discord/rest.zig");
pub const interaction = @import("discord/interaction.zig");
pub const llm = @import("llm/openai.zig");
pub const llm_guidance = @import("llm/guidance.zig");
pub const tls_test = @import("net/tls_test.zig");

pub const version = "0.1.0";

test {
    std.testing.refAllDecls(@This());
}
