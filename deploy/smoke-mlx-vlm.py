#!/usr/bin/env python3
"""Side-effect-free OpenAI text/tool/vision acceptance for Abbey's MLX-VLM."""

from __future__ import annotations

import argparse
import base64
import binascii
import json
import struct
import sys
import urllib.error
import urllib.request
import zlib
from typing import Any, NoReturn

MAX_BODY_BYTES = 4 * 1024 * 1024


def fail(message: str) -> NoReturn:
    raise SystemExit(message)


def read_capped(response: Any) -> bytes:
    body = response.read(MAX_BODY_BYTES + 1)
    if len(body) > MAX_BODY_BYTES:
        fail("MLX-VLM smoke response exceeded 4 MiB")
    return body


class Client:
    def __init__(self, base_url: str, timeout: int) -> None:
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout
        # The smoke carries a generated image and tool schema. Loopback traffic
        # must not be redirected through an inherited process-wide proxy.
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def request(self, method: str, path: str, payload: dict[str, Any] | None = None) -> Any:
        data = None if payload is None else json.dumps(payload).encode("utf-8")
        request = urllib.request.Request(
            f"{self.base_url}{path}",
            data=data,
            method=method,
            headers={"Content-Type": "application/json"},
        )
        try:
            with self.opener.open(request, timeout=self.timeout) as response:
                raw = read_capped(response)
        except urllib.error.HTTPError as error:
            detail = error.read(2048).decode("utf-8", "replace")
            fail(f"MLX-VLM {path} returned HTTP {error.code}: {detail}")
        except OSError as error:
            fail(f"MLX-VLM {path} request failed: {error}")
        try:
            return json.loads(raw)
        except json.JSONDecodeError as error:
            fail(f"MLX-VLM {path} returned invalid JSON: {error}")

    def streamed_turn(
        self, payload: dict[str, Any]
    ) -> tuple[str, list[dict[str, Any]], str | None]:
        request = urllib.request.Request(
            f"{self.base_url}/v1/chat/completions",
            data=json.dumps(payload).encode("utf-8"),
            method="POST",
            headers={"Content-Type": "application/json"},
        )
        total = 0
        chunks: list[str] = []
        calls: list[dict[str, Any]] = []
        finish_reason: str | None = None
        saw_done = False
        try:
            with self.opener.open(request, timeout=self.timeout) as response:
                for raw_line in response:
                    total += len(raw_line)
                    if total > MAX_BODY_BYTES:
                        fail("MLX-VLM streamed smoke response exceeded 4 MiB")
                    line = raw_line.decode("utf-8", "replace").strip()
                    if not line.startswith("data:"):
                        continue
                    data = line[5:].strip()
                    if data == "[DONE]":
                        saw_done = True
                        break
                    event = json.loads(data)
                    choice = event.get("choices", [{}])[0]
                    delta = choice.get("delta", {})
                    content = delta.get("content")
                    if isinstance(content, str):
                        chunks.append(content)
                    reason = choice.get("finish_reason")
                    if isinstance(reason, str):
                        finish_reason = reason
                    tool_deltas = delta.get("tool_calls")
                    if isinstance(tool_deltas, list):
                        for tool_delta in tool_deltas:
                            if not isinstance(tool_delta, dict):
                                continue
                            index = tool_delta.get("index", 0)
                            if not isinstance(index, int) or index < 0:
                                fail("MLX-VLM streamed an invalid tool-call index")
                            while len(calls) <= index:
                                calls.append(
                                    {
                                        "id": "",
                                        "type": "function",
                                        "function": {"name": "", "arguments": ""},
                                    }
                                )
                            call = calls[index]
                            call_id = tool_delta.get("id")
                            if isinstance(call_id, str):
                                call["id"] = call_id
                            function_delta = tool_delta.get("function")
                            if isinstance(function_delta, dict):
                                name = function_delta.get("name")
                                if isinstance(name, str):
                                    call["function"]["name"] = name
                                arguments = function_delta.get("arguments")
                                if isinstance(arguments, str):
                                    call["function"]["arguments"] += arguments
        except (urllib.error.HTTPError, OSError, json.JSONDecodeError) as error:
            fail(f"MLX-VLM streamed chat failed: {error}")
        if not saw_done:
            fail("MLX-VLM streamed response lacked the [DONE] terminator")
        return "".join(chunks).strip(), calls, finish_reason

    def streamed_chat(self, payload: dict[str, Any]) -> str:
        text, calls, _ = self.streamed_turn(payload)
        if not text or calls:
            fail("MLX-VLM streamed plain chat lacked text or returned a tool call")
        return text


def png_chunk(kind: bytes, payload: bytes) -> bytes:
    checksum = binascii.crc32(kind + payload) & 0xFFFFFFFF
    return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", checksum)


def red_blue_png_data_url() -> str:
    width = height = 64
    rows = []
    for _ in range(height):
        pixels = b"\xff\x00\x00" * (width // 2) + b"\x00\x00\xff" * (width // 2)
        rows.append(b"\x00" + pixels)
    png = (
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + png_chunk(b"IDAT", zlib.compress(b"".join(rows), level=9))
        + png_chunk(b"IEND", b"")
    )
    return "data:image/png;base64," + base64.b64encode(png).decode("ascii")


def message(response: Any) -> dict[str, Any]:
    try:
        value = response["choices"][0]["message"]
    except (KeyError, IndexError, TypeError):
        fail("MLX-VLM response carried no choices[0].message")
    if not isinstance(value, dict):
        fail("MLX-VLM message was not an object")
    return value


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--timeout", type=int, default=600)
    args = parser.parse_args()
    client = Client(args.base_url, args.timeout)

    health = client.request("GET", "/health")
    models = client.request("GET", "/v1/models")
    if args.model not in json.dumps(models, sort_keys=True):
        fail("MLX-VLM model listing did not identify the pinned model")

    streamed = client.streamed_chat(
        {
            "model": args.model,
            "messages": [
                {
                    "role": "system",
                    "content": "You are a concise local readiness probe.",
                },
                {"role": "user", "content": "Reply with MLX_READY."},
            ],
            "max_tokens": 256,
            "stream": True,
        }
    )

    tools = [
        {
            "type": "function",
            "function": {
                "name": "probe_status",
                "description": "Return the synthetic readiness marker requested by the user.",
                "parameters": {
                    "type": "object",
                    "properties": {"marker": {"type": "string", "enum": ["ready"]}},
                    "required": ["marker"],
                },
            },
        }
    ]
    first_payload = {
        "model": args.model,
        "messages": [
            {
                "role": "system",
                "content": "Use the offered tool exactly when instructed.",
            },
            {"role": "user", "content": "Call probe_status with marker ready."}
        ],
        "tools": tools,
        "tool_choice": {"type": "function", "function": {"name": "probe_status"}},
        "max_tokens": 256,
        "stream": True,
    }
    first_text, calls, first_finish_reason = client.streamed_turn(first_payload)
    if first_text or len(calls) != 1:
        fail("MLX-VLM did not stream exactly one OpenAI tool call")
    call = calls[0]
    function = call.get("function", {})
    if function.get("name") != "probe_status":
        fail("MLX-VLM returned the wrong tool name")
    arguments = function.get("arguments")
    if isinstance(arguments, str):
        arguments = json.loads(arguments)
    if not isinstance(arguments, dict) or arguments.get("marker") != "ready":
        fail("MLX-VLM returned invalid probe_status arguments")
    if first_finish_reason != "tool_calls":
        fail("MLX-VLM streamed tool call did not finish with tool_calls")

    assistant_turn = {
        "role": "assistant",
        "content": "",
        "tool_calls": calls,
    }
    tool_result = {
        "role": "tool",
        "tool_call_id": call.get("id", "probe_0"),
        "content": '{"marker":"ready"}',
    }
    final_text, final_calls, _ = client.streamed_turn(
        {
            "model": args.model,
            "messages": first_payload["messages"] + [assistant_turn, tool_result],
            "tools": tools,
            "max_tokens": 256,
            "stream": True,
        }
    )
    if not final_text or final_calls:
        fail("MLX-VLM streamed tool-result round trip produced no final text")

    vision = client.request(
        "POST",
        "/v1/chat/completions",
        {
            "model": args.model,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "Name the two colors from left to right, separated by a comma.",
                        },
                        {
                            "type": "image_url",
                            "image_url": {"url": red_blue_png_data_url()},
                        },
                    ],
                }
            ],
            "max_tokens": 256,
            "stream": False,
        },
    )
    vision_text = message(vision).get("content")
    if not isinstance(vision_text, str) or not vision_text.strip():
        fail("MLX-VLM vision response was empty")
    lowered = vision_text.casefold()
    if "red" not in lowered or "blue" not in lowered or lowered.index("red") > lowered.index("blue"):
        fail(f"MLX-VLM did not identify the red/blue image: {vision_text[:200]}")

    print(
        json.dumps(
            {
                "health": health,
                "model": args.model,
                "stream_chars": len(streamed),
                "tool": "probe_status",
                "tool_round_trip_chars": len(final_text),
                "vision": vision_text.strip()[:200],
            },
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
