#!/usr/bin/env python3
"""ONE-SHOT BOOTSTRAP, run once on 2026-09-21. Not part of the gate.

Rendered src/catalog/specs.zig and src/catalog/payload.zig from the oracle's
own exports (contracts/golden/catalog.json and
contracts/catalog/command-payload.json). From that commit on, the Zig files
are the source of truth and the parity tests pin them to the frozen exports;
re-running this to make a failing parity test pass would erase the evidence of
a surface change, which is a decision for Donald, not a fix.
"""
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def snake(name):
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


def zstr(s):
    return json.dumps(s, ensure_ascii=False)


def fmt(src):
    return subprocess.run(["zig", "fmt", "--stdin"], input=src.encode(), capture_output=True, check=True).stdout.decode()


def specs():
    data = json.loads((ROOT / "contracts/golden/catalog.json").read_text())
    keys = [snake(s["key"]) for s in data["specs"]]
    out = ["//! Registered command specs (68), transcribed from the oracle's",
           "//! `src/command_catalog/data.rs` via its own export (contracts/golden/catalog.json).",
           "//! Order is load-bearing: `/help` and the README table render in this order.",
           'const c = @import("catalog.zig");', "",
           "pub const CommandKey = enum {"] + [f"    {k}," for k in keys] + ["};", "",
           "pub const registered = [_]c.CommandSpec{"]
    kinds = {"Slash": ".slash", "UserContext": ".user_context", "MessageContext": ".message_context"}
    for s, k in zip(data["specs"], keys):
        ctx = "&c.both_contexts" if s["contexts"] == ["Guild", "BotDm"] else "&c.guild_only"
        assert s["contexts"] in (["Guild", "BotDm"], ["Guild"]), s["contexts"]
        perm = "null" if s["default_member_permissions"] is None else "." + snake(s["default_member_permissions"])
        out.append(f"    .{{ .key = .{k}, .kind = {kinds[s['kind']]}, .name = {zstr(s['name'])}, .registration = .{{ .contexts = {ctx}, .default_member_permissions = {perm} }}, .access = .{s['access'].lower()}, .condition = .{s['condition'].lower()}, .section = .{s['section']}, .description = {zstr(s['description'])}, .private = {str(s['private']).lower()} }},")
    out.append("};")
    assert data["planned"] == [], data["planned"]
    return fmt("\n".join(out) + "\n")


def option(o, depth):
    fields = [f".kind = {o['type']}", f".name = {zstr(o['name'])}", f".description = {zstr(o['description'])}"]
    if o["required"]:
        fields.append(".required = true")
    if o["choices"]:
        fields.append(".choices = &.{" + " ".join(f".{{ .name = {zstr(ch['name'])}, .value = {ch['value']} }}," for ch in o["choices"]) + "}")
    if o["options"]:
        fields.append(".options = &.{" + "".join(option(x, depth + 1) for x in o["options"]) + "}")
    assert o["channel_types"] == [], o
    for key in ("min_value", "max_value"):
        if o[key] is not None:
            fields.append(f".{key} = {o[key]!r}")
    for key in ("min_length", "max_length"):
        if o[key] is not None:
            fields.append(f".{key} = {o[key]}")
    if o["autocomplete"]:
        fields.append(".autocomplete = true")
    return ".{ " + ", ".join(fields) + " },\n"


def payload():
    data = json.loads((ROOT / "contracts/catalog/command-payload.json").read_text())
    out = ["//! The frozen registration payload: every top-level command with its",
           "//! options, transcribed from the oracle's own export",
           "//! (contracts/catalog/command-payload.json, poise create_application_commands).",
           "//! `serialize.zig` renders it byte-identically; a parity test pins it.",
           'const p = @import("payload_types.zig");', "",
           "pub const commands = [_]p.Command{"]
    for cmd in data:
        assert cmd["name_localizations"] == {} and cmd["description_localizations"] == {} and cmd["nsfw"] is False
        fields = [f".name = {zstr(cmd['name'])}"]
        if "description" in cmd:
            fields.append(f".description = {zstr(cmd['description'])}")
        if "type" in cmd:
            fields.append(f".kind = {cmd['type']}")
        if "default_member_permissions" in cmd:
            fields.append(f".default_member_permissions = {zstr(cmd['default_member_permissions'])}")
        fields.append(".contexts = " + ("&.{ 0, 1 }" if cmd["contexts"] == [0, 1] else "&.{0}"))
        if cmd["options"]:
            fields.append(".options = &.{" + "".join(option(o, 1) for o in cmd["options"]) + "}")
        out.append("    .{ " + ", ".join(fields) + " },")
    out.append("};")
    return fmt("\n".join(out) + "\n")


if __name__ == "__main__":
    (ROOT / "src/catalog/specs.zig").write_text(specs())
    (ROOT / "src/catalog/payload.zig").write_text(payload())
    print("wrote src/catalog/specs.zig and src/catalog/payload.zig")
