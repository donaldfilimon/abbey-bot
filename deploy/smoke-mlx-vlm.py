#!/usr/bin/env python3
"""Side-effect-free OpenAI text/tool/vision acceptance for Abbey's MLX-VLM."""

from __future__ import annotations

import argparse
import base64
import binascii
import ipaddress
import json
import struct
import sys
import urllib.error
import urllib.parse
import urllib.request
import zlib
from typing import Any, NoReturn

MAX_BODY_BYTES = 4 * 1024 * 1024
MAX_TOOL_CALLS_PER_TURN = 8


def fail(message: str) -> NoReturn:
    raise SystemExit(message)


def read_capped(response: Any) -> bytes:
    body = response.read(MAX_BODY_BYTES + 1)
    if len(body) > MAX_BODY_BYTES:
        fail("MLX-VLM smoke response exceeded 4 MiB")
    return body


class RejectRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args: Any, **_kwargs: Any) -> None:
        return None


def validate_loopback_base_url(raw: str) -> str:
    if raw != raw.strip():
        fail("MLX-VLM smoke base URL must not contain surrounding whitespace")
    parsed = urllib.parse.urlsplit(raw)
    try:
        port = parsed.port
    except ValueError as error:
        fail(f"MLX-VLM smoke base URL has an invalid port: {error}")
    try:
        loopback = parsed.hostname is not None and ipaddress.ip_address(
            parsed.hostname
        ).is_loopback
    except ValueError:
        loopback = False
    if (
        parsed.scheme != "http"
        or not loopback
        or parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
        or parsed.path not in ("", "/")
        or port is None
    ):
        fail(
            "MLX-VLM smoke base URL must be credential-free numeric loopback HTTP "
            "with an explicit port and no path, query, or fragment"
        )
    return raw.rstrip("/")


def merge_streamed_tool_deltas(
    calls: list[dict[str, Any]], tool_deltas: Any
) -> None:
    if type(tool_deltas) is not list:
        fail("MLX-VLM streamed tool_calls was not an array")
    for tool_delta in tool_deltas:
        if type(tool_delta) is not dict:
            fail("MLX-VLM streamed a non-object tool-call delta")
        unexpected = set(tool_delta) - {"index", "id", "type", "function"}
        if unexpected:
            fail("MLX-VLM streamed a tool-call delta with unexpected fields")
        if "index" not in tool_delta:
            fail("MLX-VLM streamed a tool-call delta without an index")
        index = tool_delta["index"]
        if (
            type(index) is not int
            or index < 0
            or index >= MAX_TOOL_CALLS_PER_TURN
        ):
            fail("MLX-VLM streamed an invalid tool-call index")
        while len(calls) <= index:
            calls.append(
                {
                    "id": "",
                    "type": "",
                    "function": {"name": "", "arguments": ""},
                }
            )
        call = calls[index]
        saw_fragment = False
        if "id" in tool_delta:
            call_id = tool_delta["id"]
            if type(call_id) is not str:
                fail("MLX-VLM streamed a non-string tool-call id fragment")
            call["id"] += call_id
            saw_fragment = True
        if "type" in tool_delta:
            call_type = tool_delta["type"]
            if type(call_type) is not str:
                fail("MLX-VLM streamed a non-string tool-call type fragment")
            call["type"] += call_type
            saw_fragment = True
        if "function" in tool_delta:
            function_delta = tool_delta["function"]
            if type(function_delta) is not dict:
                fail("MLX-VLM streamed a non-object tool-call function delta")
            unexpected_function = set(function_delta) - {"name", "arguments"}
            if unexpected_function:
                fail("MLX-VLM streamed a function delta with unexpected fields")
            if not function_delta:
                fail("MLX-VLM streamed an empty tool-call function delta")
            if "name" in function_delta:
                name = function_delta["name"]
                if type(name) is not str:
                    fail("MLX-VLM streamed a non-string tool name fragment")
                call["function"]["name"] += name
            if "arguments" in function_delta:
                arguments = function_delta["arguments"]
                if type(arguments) is not str:
                    fail("MLX-VLM streamed non-string tool arguments")
                call["function"]["arguments"] += arguments
            saw_fragment = True
        if not saw_fragment:
            fail("MLX-VLM streamed an empty tool-call delta")


def validate_streamed_tool_calls(calls: list[dict[str, Any]]) -> None:
    for call in calls:
        if type(call.get("id")) is not str or not call["id"]:
            fail("MLX-VLM streamed an incomplete tool-call id")
        if call.get("type") != "function":
            fail("MLX-VLM streamed an incomplete or invalid tool-call type")
        function = call.get("function")
        if type(function) is not dict:
            fail("MLX-VLM assembled a non-object tool-call function")
        name = function.get("name")
        if type(name) is not str or not name:
            fail("MLX-VLM streamed an incomplete tool name")
        arguments = function.get("arguments")
        if type(arguments) is not str or not arguments:
            fail("MLX-VLM streamed incomplete tool arguments")
        try:
            decoded_arguments = json.loads(arguments)
        except json.JSONDecodeError as error:
            fail(f"MLX-VLM streamed invalid tool arguments JSON: {error}")
        if type(decoded_arguments) is not dict:
            fail("MLX-VLM streamed tool arguments that were not a JSON object")


class Client:
    def __init__(self, base_url: str, timeout: int) -> None:
        self.base_url = validate_loopback_base_url(base_url)
        if timeout <= 0:
            fail("MLX-VLM smoke timeout must be positive")
        self.timeout = timeout
        # The smoke carries generated images and tool schemas. It must neither
        # inherit a proxy nor follow even a server-selected redirect.
        self.opener = urllib.request.build_opener(
            urllib.request.ProxyHandler({}), RejectRedirects()
        )

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
                    try:
                        line = raw_line.decode("utf-8").strip()
                    except UnicodeDecodeError as error:
                        fail(f"MLX-VLM streamed invalid UTF-8: {error}")
                    if not line.startswith("data:"):
                        continue
                    data = line[5:].strip()
                    if data == "[DONE]":
                        saw_done = True
                        break
                    event = json.loads(data)
                    if type(event) is not dict:
                        fail("MLX-VLM streamed a non-object event")
                    choices = event.get("choices")
                    if type(choices) is not list or len(choices) != 1:
                        fail("MLX-VLM streamed an event without exactly one choice")
                    choice = choices[0]
                    if type(choice) is not dict:
                        fail("MLX-VLM streamed a non-object choice")
                    delta = choice.get("delta")
                    if type(delta) is not dict:
                        fail("MLX-VLM streamed a non-object delta")
                    if "content" in delta:
                        content = delta["content"]
                        if content is not None and type(content) is not str:
                            fail("MLX-VLM streamed non-string content")
                        if type(content) is str:
                            chunks.append(content)
                    if "finish_reason" in choice:
                        reason = choice["finish_reason"]
                        if reason is not None and type(reason) is not str:
                            fail("MLX-VLM streamed a non-string finish reason")
                        if type(reason) is str:
                            finish_reason = reason
                    if "tool_calls" in delta:
                        merge_streamed_tool_deltas(calls, delta["tool_calls"])
        except (urllib.error.HTTPError, OSError, json.JSONDecodeError) as error:
            fail(f"MLX-VLM streamed chat failed: {error}")
        if not saw_done:
            fail("MLX-VLM streamed response lacked the [DONE] terminator")
        validate_streamed_tool_calls(calls)
        return "".join(chunks), calls, finish_reason

    def streamed_chat(self, payload: dict[str, Any]) -> str:
        text, calls, finish_reason = self.streamed_turn(payload)
        if not text.strip() or calls:
            fail("MLX-VLM streamed plain chat lacked text or returned a tool call")
        if finish_reason != "stop":
            fail(
                "MLX-VLM streamed plain chat did not finish normally: "
                f"{finish_reason!r}"
            )
        return text


def png_chunk(kind: bytes, payload: bytes) -> bytes:
    checksum = binascii.crc32(kind + payload) & 0xFFFFFFFF
    return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", checksum)


def png_data_url(width: int, height: int, pixels: bytes) -> str:
    if len(pixels) != width * height * 3:
        fail("generated PNG fixture has the wrong pixel count")
    rows = []
    stride = width * 3
    for row in range(height):
        rows.append(b"\x00" + pixels[row * stride : (row + 1) * stride])
    png = (
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + png_chunk(b"IDAT", zlib.compress(b"".join(rows), level=9))
        + png_chunk(b"IEND", b"")
    )
    return "data:image/png;base64," + base64.b64encode(png).decode("ascii")


def shape_fixture_data_url() -> str:
    """A red square at left and a blue circle at right on white."""
    width, height = 160, 80
    image = bytearray(b"\xff\xff\xff" * width * height)

    def set_pixel(x: int, y: int, color: bytes) -> None:
        offset = (y * width + x) * 3
        image[offset : offset + 3] = color

    for y in range(18, 63):
        for x in range(18, 63):
            set_pixel(x, y, b"\xff\x00\x00")
    center_x, center_y, radius = 119, 40, 24
    for y in range(height):
        for x in range(80, width):
            if (x - center_x) ** 2 + (y - center_y) ** 2 <= radius**2:
                set_pixel(x, y, b"\x00\x00\xff")
    return png_data_url(width, height, bytes(image))


GLYPHS: dict[str, tuple[str, ...]] = {
    "A": ("01110", "10001", "10001", "11111", "10001", "10001", "10001"),
    "B": ("11110", "10001", "10001", "11110", "10001", "10001", "11110"),
    "E": ("11111", "10000", "10000", "11110", "10000", "10000", "11111"),
    "Y": ("10001", "10001", "01010", "00100", "00100", "00100", "00100"),
    "0": ("01110", "10001", "10011", "10101", "11001", "10001", "01110"),
    "2": ("01110", "10001", "00001", "00010", "00100", "01000", "11111"),
    "4": ("00010", "00110", "01010", "10010", "11111", "00010", "00010"),
    "7": ("11111", "00001", "00010", "00100", "01000", "01000", "01000"),
    "9": ("01110", "10001", "10001", "01111", "00001", "10001", "01110"),
}


OCR_TEXT = "ABBEY 4729"


def ocr_fixture_data_url() -> str:
    scale = 7
    margin = 12
    glyph_width = 5 * scale
    spacing = 2 * scale
    width = margin * 2 + len(OCR_TEXT) * (glyph_width + spacing) - spacing
    height = margin * 2 + 7 * scale
    image = bytearray(b"\xff\xff\xff" * width * height)
    cursor = margin
    for character in OCR_TEXT:
        if character == " ":
            cursor += glyph_width + spacing
            continue
        glyph = GLYPHS[character]
        for row, bits in enumerate(glyph):
            for column, bit in enumerate(bits):
                if bit != "1":
                    continue
                for dy in range(scale):
                    for dx in range(scale):
                        x = cursor + column * scale + dx
                        y = margin + row * scale + dy
                        offset = (y * width + x) * 3
                        image[offset : offset + 3] = b"\x00\x00\x00"
        cursor += glyph_width + spacing
    return png_data_url(width, height, bytes(image))


def message(response: Any) -> dict[str, Any]:
    try:
        choice = response["choices"][0]
        value = choice["message"]
    except (KeyError, IndexError, TypeError):
        fail("MLX-VLM response carried no choices[0].message")
    if choice.get("finish_reason") != "stop":
        fail(
            "MLX-VLM non-streamed response did not finish normally: "
            f"{choice.get('finish_reason')!r}"
        )
    if not isinstance(value, dict):
        fail("MLX-VLM message was not an object")
    return value


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--expected-kv-size", type=int, required=True)
    parser.add_argument("--timeout", type=int, default=600)
    args = parser.parse_args()
    if args.expected_kv_size <= 0:
        fail("MLX-VLM expected KV size must be positive")
    client = Client(args.base_url, args.timeout)

    health = client.request("GET", "/health")
    models = client.request("GET", "/v1/models")
    if not isinstance(health, dict) or health.get("status") != "healthy":
        fail("MLX-VLM health response was not healthy")
    if health.get("loaded_model") != args.model:
        fail("MLX-VLM health response did not identify the exact pinned snapshot")
    if health.get("configured_context_limit") != args.expected_kv_size:
        fail(
            "MLX-VLM configured context limit was "
            f"{health.get('configured_context_limit')!r}, expected {args.expected_kv_size}"
        )
    if health.get("effective_context_limit") != args.expected_kv_size:
        fail("MLX-VLM effective context limit did not equal the configured ceiling")
    if not isinstance(models, dict) or not isinstance(models.get("data"), list):
        fail("MLX-VLM model listing was not an OpenAI model list")
    model_ids = {
        item.get("id") for item in models["data"] if isinstance(item, dict)
    }
    if args.model not in model_ids:
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
    if streamed != "MLX_READY":
        fail(f"MLX-VLM streamed marker was not exact: {streamed[:200]!r}")

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
                    "additionalProperties": False,
                },
            },
        }
    ]
    first_payload = {
        "model": args.model,
        "messages": [
                {
                    "role": "system",
                    "content": (
                        "Use the offered tool exactly when instructed. After its result, "
                        "reply with exactly TOOL_CONTINUATION_READY."
                    ),
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
    call_id = call.get("id")
    if not isinstance(call_id, str) or not call_id:
        fail("MLX-VLM streamed tool call had no id")
    if call.get("type") != "function":
        fail("MLX-VLM streamed tool call had the wrong type")
    function = call.get("function", {})
    if function.get("name") != "probe_status":
        fail("MLX-VLM returned the wrong tool name")
    arguments = function.get("arguments")
    if isinstance(arguments, str):
        arguments = json.loads(arguments)
    if arguments != {"marker": "ready"}:
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
        "tool_call_id": call_id,
        "content": '{"marker":"ready"}',
    }
    final_text, final_calls, final_finish_reason = client.streamed_turn(
        {
            "model": args.model,
            "messages": first_payload["messages"] + [assistant_turn, tool_result],
            "tools": tools,
            "max_tokens": 256,
            "stream": True,
        }
    )
    if (
        final_text != "TOOL_CONTINUATION_READY"
        or final_calls
        or final_finish_reason != "stop"
    ):
        fail(
            "MLX-VLM streamed tool-result continuation marker was not exact: "
            f"{final_text[:200]!r}"
        )

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
                            "text": (
                                "Identify the colored shapes from left to right. Return only a "
                                "lowercase comma-separated list formatted as "
                                "color shape, color shape."
                            ),
                        },
                        {
                            "type": "image_url",
                            "image_url": {"url": shape_fixture_data_url()},
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
    if vision_text.strip().casefold() != "red square, blue circle":
        fail(f"MLX-VLM colored-shape marker was not exact: {vision_text[:200]!r}")

    ocr = client.request(
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
                            "text": (
                                "Transcribe the text in this image exactly. Return only the "
                                "characters from the image, with no punctuation or explanation."
                            ),
                        },
                        {
                            "type": "image_url",
                            "image_url": {"url": ocr_fixture_data_url()},
                        },
                    ],
                }
            ],
            "max_tokens": 128,
            "stream": False,
        },
    )
    ocr_text = message(ocr).get("content")
    if not isinstance(ocr_text, str) or ocr_text.strip() != OCR_TEXT:
        fail(f"MLX-VLM OCR mismatch: expected {OCR_TEXT!r}, got {ocr_text!r}")

    print(
        json.dumps(
            {
                "health": health,
                "model": args.model,
                "configured_context_limit": args.expected_kv_size,
                "stream_chars": len(streamed),
                "tool": "probe_status",
                "tool_round_trip_chars": len(final_text),
                "vision": vision_text.strip()[:200],
                "ocr": ocr_text.strip(),
            },
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
