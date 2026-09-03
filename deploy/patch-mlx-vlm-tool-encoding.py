#!/usr/bin/env python3
"""Patch mlx-vlm + Gemma 4 chat_template for OpenAI-style tool JSON bodies.

Gemma 4's template formats tool results via format_tool_response_block:
a mapping becomes response:name{k:v}, a string becomes
response:name{value:"..."}. OpenAI clients send JSON *strings*, which
then look like value:"{...}" to the model. Dicts that skip the mapping
test can satisfy Jinja `is sequence` and then crash on part.get('type')
because the iterated keys are strings.

These encoding fixes are necessary. They are not sufficient to stop the
4-bit Gemma checkpoint from looping <|channel>thought into content after
a tool result — the installer must still fail closed on
TOOL_CONTINUATION_READY. Ollama remains the reasoner until a later
checkpoint actually continues.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


OPENAI_MARKER = "Abbey patch: Gemma 4 chat template formats tool bodies"
OPENAI_ANCHOR = "            if message.tool_call_id is not None:"
OPENAI_INSERT = """            # Abbey patch: Gemma 4 chat template formats tool bodies via
            # format_tool_response_block. A JSON *string* becomes
            # response:name{value:"{...}"}; a mapping becomes response:name{k:v}.
            # OpenAI clients send string content — parse object/array JSON.
            if message.role == "tool" and isinstance(msg.get("content"), str):
                raw = msg["content"].strip()
                if raw[:1] in "{[":
                    try:
                        parsed = json.loads(raw)
                        if isinstance(parsed, (dict, list)):
                            msg["content"] = parsed
                    except (json.JSONDecodeError, TypeError):
                        pass

"""

PROMPT_MARKER = "Abbey patch: parse JSON object/array tool content"
PROMPT_ANCHOR = "    tool_calls = normalized.get(\"tool_calls\")"
PROMPT_INSERT = """    # Abbey patch: parse JSON object/array tool content for Gemma templates.
    if normalized.get("role") == "tool" and isinstance(normalized.get("content"), str):
        raw = normalized["content"].strip()
        if raw[:1] in "{[":
            try:
                parsed = json.loads(raw)
                if isinstance(parsed, (dict, list)):
                    normalized["content"] = parsed
            except (json.JSONDecodeError, TypeError):
                pass

"""

TEMPLATE_MARKER = "Abbey: mappings before sequence"
TEMPLATE_SET = "{%- set tool_body = follow.get('content') -%}"
TEMPLATE_STRING_IF = "                        {%- if tool_body is string -%}"
TEMPLATE_MAPPING_IF = """                        {%- if tool_body is mapping -%}
                            {{- format_tool_response_block(ns_tname.name, tool_body) -}}
                        {%- elif tool_body is string -%}"""


def parse_tool_json_content(content: object) -> object:
    """Parse object/array JSON tool bodies; leave other values untouched."""
    if not isinstance(content, str):
        return content
    raw = content.strip()
    if raw[:1] not in "{[":
        return content
    try:
        parsed = json.loads(raw)
    except (json.JSONDecodeError, TypeError):
        return content
    if isinstance(parsed, (dict, list)):
        return parsed
    return content


def _insert_once(text: str, *, marker: str, anchor: str, insert: str, label: str) -> str:
    if marker in text:
        return text
    if text.count(anchor) != 1:
        raise SystemExit(f"{label} no longer matches expected {anchor!r} (count={text.count(anchor)})")
    return text.replace(anchor, insert + anchor, 1)


def patch_openai_source(text: str) -> str:
    return _insert_once(
        text,
        marker=OPENAI_MARKER,
        anchor=OPENAI_ANCHOR,
        insert=OPENAI_INSERT,
        label="mlx_vlm/server/openai.py",
    )


def patch_prompt_utils_source(text: str) -> str:
    return _insert_once(
        text,
        marker=PROMPT_MARKER,
        anchor=PROMPT_ANCHOR,
        insert=PROMPT_INSERT,
        label="mlx_vlm/prompt_utils.py",
    )


def patch_chat_template_source(text: str) -> str:
    if TEMPLATE_MARKER in text or (
        "tool_body is mapping" in text and TEMPLATE_SET in text
    ):
        return text
    set_idx = text.find(TEMPLATE_SET)
    if set_idx < 0:
        raise SystemExit("chat_template.jinja is missing the tool_body assignment")
    if_idx = text.find(TEMPLATE_STRING_IF, set_idx)
    if if_idx < 0:
        raise SystemExit("chat_template.jinja is missing the tool_body string branch")
    updated = text[:if_idx] + TEMPLATE_MAPPING_IF + text[if_idx + len(TEMPLATE_STRING_IF) :]
    updated = updated.replace(
        "{#- Handle content as string or content-parts array -#}",
        "{#- Handle content as mapping/string/content-parts (Abbey: mappings before sequence) -#}",
        1,
    )
    if TEMPLATE_MARKER not in updated and "tool_body is mapping" not in updated:
        raise SystemExit("chat_template.jinja mapping branch did not apply")
    return updated


def _write_if_changed(path: Path, original: str, updated: str) -> str:
    if updated == original:
        return f"already patched {path}"
    path.write_text(updated, encoding="utf-8")
    return f"patched {path}"


def site_packages_root(prefix: Path | None = None) -> Path:
    root = Path(prefix or sys.prefix)
    version = f"python{sys.version_info.major}.{sys.version_info.minor}"
    return root / "lib" / version / "site-packages"


def apply_installed(*, prefix: Path | None = None) -> list[str]:
    sp = site_packages_root(prefix)
    openai = sp / "mlx_vlm" / "server" / "openai.py"
    prompt = sp / "mlx_vlm" / "prompt_utils.py"
    reports = []
    for path, patcher in (
        (openai, patch_openai_source),
        (prompt, patch_prompt_utils_source),
    ):
        if not path.is_file():
            raise SystemExit(f"missing {path}")
        original = path.read_text(encoding="utf-8")
        reports.append(_write_if_changed(path, original, patcher(original)))
    return reports


def apply_chat_template(path: Path) -> str:
    if not path.is_file():
        raise SystemExit(f"missing chat template: {path}")
    original = path.read_text(encoding="utf-8")
    return _write_if_changed(path, original, patch_chat_template_source(original))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--apply-installed",
        action="store_true",
        help="Patch mlx_vlm in sys.prefix site-packages (run with the staged venv python).",
    )
    parser.add_argument(
        "--chat-template",
        type=Path,
        help="Patch Gemma 4 chat_template.jinja so mappings are tested before sequences.",
    )
    args = parser.parse_args(argv)
    if not args.apply_installed and args.chat_template is None:
        parser.error("pass --apply-installed and/or --chat-template")
    if args.apply_installed:
        for line in apply_installed():
            print(line)
    if args.chat_template is not None:
        print(apply_chat_template(args.chat_template))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
