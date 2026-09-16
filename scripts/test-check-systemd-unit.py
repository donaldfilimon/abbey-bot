#!/usr/bin/env python3
"""Regressions for the systemd unit hardening pin."""
import importlib.util
import pathlib
import unittest

SPEC = importlib.util.spec_from_file_location("systemd_unit", pathlib.Path(__file__).with_name("check-systemd-unit.py"))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def unit(**overrides):
    service = dict(MODULE.REQUIRED_SERVICE)
    service.update({"ExecStart": "/usr/local/bin/abbey-bot", "RestartSec": "30"})
    extra = []
    for key, value in overrides.items():
        if value is None:
            service.pop(key, None)
        elif isinstance(value, list):
            service.pop(key, None)
            extra.extend(f"{key}={item}" for item in value)
        else:
            service[key] = value
    lines = ["[Unit]", "Description=test", "", "[Service]"]
    lines += [f"{key}={value}" for key, value in service.items()] + extra
    lines += ["", "[Install]", "WantedBy=multi-user.target", ""]
    return "\n".join(lines)


class SystemdUnitTests(unittest.TestCase):
    def test_checked_in_unit_passes(self):
        self.assertEqual(MODULE.problems(MODULE.UNIT.read_text(encoding="utf-8")), [])

    def test_complete_synthetic_unit_passes(self):
        self.assertEqual(MODULE.problems(unit()), [])

    def test_missing_or_weakened_hardening_fails(self):
        self.assertTrue(MODULE.problems(unit(UMask=None)))
        self.assertTrue(MODULE.problems(unit(UMask="0022")))
        self.assertTrue(MODULE.problems(unit(ProtectSystem="full")))
        self.assertTrue(MODULE.problems(unit(CapabilityBoundingSet="CAP_NET_ADMIN")))

    def test_repeated_key_is_not_silently_merged(self):
        self.assertTrue(MODULE.problems(unit(SystemCallFilter=["@system-service", "@privileged"])))

    def test_restart_backoff_floor(self):
        self.assertTrue(MODULE.problems(unit(RestartSec="5")))
        self.assertTrue(MODULE.problems(unit(RestartSec="5s")))
        self.assertEqual(MODULE.problems(unit(RestartSec="60")), [])

    def test_relative_exec_start_fails(self):
        self.assertTrue(MODULE.problems(unit(ExecStart="abbey-bot")))

    def test_inline_secret_fails_but_plain_environment_passes(self):
        self.assertTrue(MODULE.problems(unit(Environment=["DISCORD_TOKEN=x"])))
        self.assertTrue(MODULE.problems(unit(Environment=["OPENAI_API_KEY=x"])))
        self.assertEqual(MODULE.problems(unit(Environment=["RUST_LOG=info"])), [])

    def test_missing_section_and_stray_line_fail(self):
        self.assertTrue(MODULE.problems(unit().replace("[Install]\nWantedBy=multi-user.target", "")))
        self.assertTrue(MODULE.problems("stray\n" + unit()))


if __name__ == "__main__":
    unittest.main()
