#!/usr/bin/env python3
"""Unit coverage for MLX-VLM Gemma tool-encoding patches."""
from __future__ import annotations

import importlib.util
from pathlib import Path

PATCH = Path(__file__).with_name("patch-mlx-vlm-tool-encoding.py")
spec = importlib.util.spec_from_file_location("patch_mlx_vlm_tool_encoding", PATCH)
mod = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(mod)


OPENAI_FIXTURE = '''
            if message.tool_calls is not None:
                msg["tool_calls"] = message.tool_calls
            if message.tool_call_id is not None:
                msg["tool_call_id"] = message.tool_call_id
'''

PROMPT_FIXTURE = '''
def _normalize_tool_message(message):
    normalized = dict(message)
    tool_calls = normalized.get("tool_calls")
    if tool_calls is None:
        return normalized
'''

TEMPLATE_FIXTURE = '''
            {%- elif message.get('tool_calls') -%}
                {%- for k in range(loop.index0 + 1, loop_messages | length) -%}
                    {%- else -%}
                        {%- set follow = loop_messages[k] -%}
                        {#- Handle content as string or content-parts array -#}
                        {%- set tool_body = follow.get('content') -%}
                        {%- if tool_body is string -%}
                            {{- format_tool_response_block(ns_tname.name, tool_body) -}}
                        {%- elif tool_body is sequence and tool_body is not string -%}
                            {{- format_tool_response_block(ns_tname.name, ns_txt.s) -}}
                        {%- endif -%}
                    {%- endif -%}
                {%- endfor -%}
            {%- endif -%}
'''


def test_parse_object_and_array():
    assert mod.parse_tool_json_content('{"marker":"ready","ok":true}') == {
        "marker": "ready",
        "ok": True,
    }
    assert mod.parse_tool_json_content("[1, 2]") == [1, 2]


def test_parse_leaves_plain_text():
    assert mod.parse_tool_json_content("ready") == "ready"
    assert mod.parse_tool_json_content("{not json") == "{not json"
    assert mod.parse_tool_json_content(None) is None
    assert mod.parse_tool_json_content({"already": True}) == {"already": True}


def test_openai_insert_is_idempotent():
    once = mod.patch_openai_source(OPENAI_FIXTURE)
    assert OPENAI_FIXTURE != once
    assert "json.loads(raw)" in once
    assert once.count("if message.role == \"tool\"") == 1
    twice = mod.patch_openai_source(once)
    assert twice == once


def test_prompt_utils_insert_is_idempotent():
    once = mod.patch_prompt_utils_source(PROMPT_FIXTURE)
    assert "normalized.get(\"role\") == \"tool\"" in once
    twice = mod.patch_prompt_utils_source(once)
    assert twice == once


def test_template_mapping_before_sequence():
    once = mod.patch_chat_template_source(TEMPLATE_FIXTURE)
    mapping_at = once.find("tool_body is mapping")
    string_at = once.find("tool_body is string")
    sequence_at = once.find("tool_body is sequence")
    assert mapping_at != -1
    assert mapping_at < string_at < sequence_at
    assert "Abbey: mappings before sequence" in once
    twice = mod.patch_chat_template_source(once)
    assert twice == once


def test_missing_openai_anchor_fails():
    try:
        mod.patch_openai_source("no anchors here")
    except SystemExit as error:
        assert "openai.py" in str(error)
    else:
        raise AssertionError("expected SystemExit")


if __name__ == "__main__":
    test_parse_object_and_array()
    test_parse_leaves_plain_text()
    test_openai_insert_is_idempotent()
    test_prompt_utils_insert_is_idempotent()
    test_template_mapping_before_sequence()
    test_missing_openai_anchor_fails()
    print("mlx-vlm tool-encoding patch tests passed")
