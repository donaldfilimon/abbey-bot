#!/usr/bin/env python3
"""Offline actual-CLI regression: permissive tracing must not escape benchmark JSON."""
from __future__ import annotations

import argparse
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import threading
import unittest


class StartupTests(unittest.TestCase):
    binary: Path

    def test_permissive_logging_produces_one_content_free_report(self) -> None:
        errors: list[str] = []
        calls: list[int] = []

        class Fixture(BaseHTTPRequestHandler):
            def log_message(self, *_args: object) -> None:
                pass

            def do_POST(self) -> None:
                self.connection.settimeout(5)
                try:
                    length = int(self.headers.get("Content-Length", "0"))
                    if not 0 < length <= 65536:
                        raise ValueError("fixture_request_bound")
                    request = json.loads(self.rfile.read(length))
                    if not request.get("stream"):
                        raise ValueError("fixture_requires_stream")
                    messages = request["messages"]
                    text = messages[-1]["content"]
                    if request.get("tools"):
                        nonce = request["tools"][0]["function"]["parameters"]["properties"]["nonce"]["enum"][0]
                        delta = {"tool_calls": [{"index": 0, "id": "fixture-call", "type": "function", "function": {"name": "probe_status", "arguments": json.dumps({"nonce": nonce})}}]}
                        finish = "tool_calls"
                    else:
                        addition = re.search(r"sum of (\d+) and 7", text)
                        exact = re.search(r"Return exactly(?:\:)? (.+)", text)
                        if addition:
                            answer = str(int(addition.group(1)) + 7)
                        elif exact:
                            answer = exact.group(1)
                        else:
                            raise ValueError("unknown_synthetic_fixture")
                        delta = {"content": answer}
                        finish = "stop"
                    events = [
                        {"choices": [{"index": 0, "delta": delta, "finish_reason": None}]},
                        {"choices": [{"index": 0, "delta": {}, "finish_reason": finish}]},
                    ]
                    encoded = ("".join("data: " + json.dumps(event) + "\n\n" for event in events) + "data: [DONE]\n\n").encode()
                    self.send_response(200)
                    self.send_header("Content-Type", "text/event-stream")
                    self.send_header("Content-Length", str(len(encoded)))
                    self.send_header("Connection", "close")
                    self.end_headers()
                    calls.append(1)
                    self.wfile.write(encoded)
                except (ValueError, KeyError, OSError, TypeError):
                    errors.append("synthetic_fixture_failed")
                    self.send_error(500)

        server = HTTPServer(("127.0.0.1", 0), Fixture)
        fixture = threading.Thread(target=server.serve_forever, name="benchmark-loopback-fixture")
        fixture.start()
        child: subprocess.Popen[bytes] | None = None
        try:
            with tempfile.TemporaryDirectory(prefix="abbey-benchmark-cli-") as temporary:
                scratch = Path(temporary)
                endpoint = f"http://127.0.0.1:{server.server_port}"
                environment = {key: os.environ[key] for key in ("PATH", "SystemRoot", "SYSTEMROOT", "WINDIR") if key in os.environ}
                environment.update({
                    "HOME": temporary, "TMPDIR": temporary, "TEMP": temporary, "TMP": temporary,
                    "RUST_LOG": "trace", "ABBEY_BOT_LLM_ENDPOINT": endpoint,
                    "ABBEY_BOT_LLM_MODEL": "ABBEY_BENCHMARK_PRIVATE_MODEL",
                    "ABBEY_BOT_LLM_TIMEOUT_SECS": "10", "ABBEY_BOT_LLM_QUEUE_SECS": "10",
                    "ABBEY_DATA_DIR": str(scratch / "canonical-state"),
                    "ABBEY_PROVIDER_STATE_DIR": str(scratch / "provider-state"),
                })
                child = subprocess.Popen([
                    str(self.binary), "--text-benchmark", "primary", "--installed-artifact", str(self.binary),
                    "--model-sha256", "c" * 64, "--hardware-sha256", "d" * 64, "--json",
                ], cwd=scratch, env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                stdout, stderr = child.communicate(timeout=90)
                self.assertEqual(child.returncode, 0, "synthetic benchmark CLI did not finish successfully")
                self.assertEqual(len(calls), 60)
                self.assertFalse(errors, "synthetic fixture rejected an unexpected request")
                try:
                    report = json.loads(stdout)
                except (ValueError, UnicodeDecodeError):
                    self.fail("benchmark stdout was not exactly one parseable JSON report")
                self.assertEqual(report["summary"]["counts"], {"attempted": 48, "success": 48, "failure": 0, "incomplete": 0, "no_text": 0})
                self.assertEqual(report["receipt"]["measurement_mode"], "primary_streaming")
                for forbidden in (endpoint.encode(), b"ABBEY_BENCHMARK_PRIVATE_MODEL", b"ABBEY_BENCHMARK_DONE_", b"probe_status"):
                    self.assertFalse(forbidden in stdout + stderr, "benchmark output exposed fixture endpoint or request data")
                self.assertFalse((scratch / "canonical-state").exists(), "benchmark constructed canonical state")
                self.assertFalse((scratch / "provider-state").exists(), "benchmark constructed provider persistence")
                print(json.dumps({"measurement_mode": report["receipt"]["measurement_mode"], "summary": report["summary"], "stage_origins": report["stage_origins"], "qualification_gaps": report["qualification_gaps"]}, sort_keys=True))
        finally:
            if child is not None and child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=5)
            server.shutdown()
            server.server_close()
            fixture.join(timeout=5)
            self.assertFalse(fixture.is_alive(), "synthetic fixture owner was not joined")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    arguments = parser.parse_args()
    StartupTests.binary = arguments.binary.resolve(strict=True)
    unittest.main(argv=[__file__])
