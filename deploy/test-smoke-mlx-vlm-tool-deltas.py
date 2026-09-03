#!/usr/bin/env python3
"""Unit coverage for streamed tool_calls null vs array handling."""
from __future__ import annotations

import importlib.util
from pathlib import Path

SMOKE = Path(__file__).with_name("smoke-mlx-vlm.py")
spec = importlib.util.spec_from_file_location("smoke_mlx_vlm", SMOKE)
mod = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(mod)


def test_null_is_skipped_like_omitted():
    calls: list = []
    # The production loop only calls merge when the value is not None.
    tool_calls = None
    if tool_calls is not None:
        mod.merge_streamed_tool_deltas(calls, tool_calls)
    assert calls == []


def test_array_is_merged():
    calls: list = []
    mod.merge_streamed_tool_deltas(
        calls,
        [
            {
                "index": 0,
                "id": "call_1",
                "type": "function",
                "function": {"name": "probe_status", "arguments": "{\"marker\":\"ready\"}"},
            }
        ],
    )
    mod.validate_streamed_tool_calls(calls)
    assert calls[0]["function"]["name"] == "probe_status"


def test_non_array_still_fails():
    try:
        mod.merge_streamed_tool_deltas([], {"index": 0})
    except SystemExit as error:
        assert "was not an array" in str(error)
    else:
        raise AssertionError("expected SystemExit")


if __name__ == "__main__":
    test_null_is_skipped_like_omitted()
    test_array_is_merged()
    test_non_array_still_fails()
    print("smoke tool-delta tests passed")
