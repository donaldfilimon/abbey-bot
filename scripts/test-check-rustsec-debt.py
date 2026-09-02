#!/usr/bin/env python3
"""Offline regressions for Abbey's exact, explicitly non-clean RustSec debt."""

from __future__ import annotations

from contextlib import redirect_stderr, redirect_stdout
import copy
import importlib.util
from io import StringIO
import json
from pathlib import Path
import subprocess
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/check-rustsec-debt.py"
POLICY = ROOT / "security/rustsec-accepted-debt.json"

SPEC = importlib.util.spec_from_file_location("check_rustsec_debt", SCRIPT)
if SPEC is None or SPEC.loader is None:  # pragma: no cover - import machinery guard
    raise RuntimeError(f"unable to load {SCRIPT}")
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)

EXPECTED_IDS = {
    "RUSTSEC-2026-0049": "GHSA-pwjx-qhcg-rvj4",
    "RUSTSEC-2026-0098": "GHSA-965h-392x-2mh5",
    "RUSTSEC-2026-0099": "GHSA-xgp8-3hg3-c2mh",
    "RUSTSEC-2026-0104": "GHSA-82j2-j2ch-gfr8",
}
EXPECTED_RANGES_AND_CATEGORIES = {
    "RUSTSEC-2026-0049": {
        "patched": [">=0.103.10"],
        "unaffected": ["<0.102.0-alpha.0"],
        "categories": ["privilege-escalation"],
    },
    "RUSTSEC-2026-0098": {
        "patched": [">=0.103.12, <0.104.0-alpha.1", ">=0.104.0-alpha.6"],
        "unaffected": [],
        "categories": [],
    },
    "RUSTSEC-2026-0099": {
        "patched": [">=0.103.12, <0.104.0-alpha.1", ">=0.104.0-alpha.6"],
        "unaffected": [],
        "categories": [],
    },
    "RUSTSEC-2026-0104": {
        "patched": [">=0.103.13, <0.104.0-alpha.1", ">=0.104.0-alpha.7"],
        "unaffected": [],
        "categories": ["denial-of-service"],
    },
}
PACKAGE_CHECKSUM = "64ca1bc8749bd4cf37b5ce386cc146580777b4e8572c7b97baf22c83f444bee9"


def accepted_policy() -> dict[str, object]:
    return json.loads(POLICY.read_text(encoding="utf-8"))


def exact_report() -> dict[str, object]:
    policy = accepted_policy()
    records = []
    for accepted in policy["accepted_vulnerabilities"]:
        record = copy.deepcopy(accepted)
        advisory_id = record.pop("id")
        record["advisory"] = {"id": advisory_id, **record["advisory"]}
        record["package"] = {
            **record["package"],
            "dependencies": [],
            "replace": None,
        }
        records.append(record)
    return {
        "database": {
            "advisory-count": 1239,
            "last-commit": None,
            "last-updated": None,
        },
        "lockfile": {"dependency-count": 505},
        "settings": {
            "target_arch": [],
            "target_os": [],
            "severity": None,
            "ignore": [],
            "informational_warnings": ["unmaintained", "unsound", "notice"],
        },
        "vulnerabilities": {
            "found": True,
            "count": len(records),
            "list": records,
        },
        "warnings": {
            "unmaintained": [{"advisory": {}}, {"advisory": {}}, {"advisory": {}}]
        },
    }


def synchronize_count(report: dict[str, object]) -> None:
    vulnerabilities = report["vulnerabilities"]
    records = vulnerabilities["list"]
    vulnerabilities["count"] = len(records)
    vulnerabilities["found"] = bool(records)


def set_nested(record: dict[str, object], path: tuple[str, ...], value: object) -> None:
    target = record
    for part in path[:-1]:
        target = target[part]
    target[path[-1]] = value


class RustsecDebtCheckTests(unittest.TestCase):
    def run_check(
        self,
        report: dict[str, object] | None = None,
        *,
        report_text: str | None = None,
        report_returncode: int = 1,
        version_output: str = "cargo-audit-audit 0.22.2\n",
        version_returncode: int = 0,
        version_stderr: str = "",
        report_stderr: str = "",
    ) -> tuple[int, str, str, mock.Mock]:
        version_result = subprocess.CompletedProcess(
            args=["cargo", "audit", "--version"],
            returncode=version_returncode,
            stdout=version_output,
            stderr=version_stderr,
        )
        if report_text is None:
            report_text = json.dumps(report if report is not None else exact_report())
        report_result = subprocess.CompletedProcess(
            args=["cargo", "audit", "--json"],
            returncode=report_returncode,
            stdout=report_text,
            stderr=report_stderr,
        )
        stdout = StringIO()
        stderr = StringIO()
        with mock.patch.object(
            CHECKER.subprocess,
            "run",
            side_effect=[version_result, report_result],
        ) as run:
            with redirect_stdout(stdout), redirect_stderr(stderr):
                status = CHECKER.main()
        return status, stdout.getvalue(), stderr.getvalue(), run

    def assert_failure(
        self,
        result: tuple[int, str, str, mock.Mock],
        fragment: str,
    ) -> None:
        status, stdout, stderr, _ = result
        self.assertEqual(status, 1)
        self.assertEqual(stdout, "")
        self.assertIn(fragment, stderr)
        self.assertLessEqual(len(stderr), 1024)

    def test_policy_pins_the_exact_four_vulnerabilities(self) -> None:
        policy = accepted_policy()

        self.assertEqual(policy["cargo_audit_version"], "0.22.2")
        self.assertEqual(policy["audit_state"], "not-clean")
        self.assertEqual(policy["accepted_vulnerability_count"], 4)
        records = {
            record["id"]: record for record in policy["accepted_vulnerabilities"]
        }
        self.assertEqual(set(records), set(EXPECTED_IDS))
        for advisory_id, alias in EXPECTED_IDS.items():
            with self.subTest(advisory_id=advisory_id):
                record = records[advisory_id]
                self.assertEqual(record["advisory"]["aliases"], [alias])
                self.assertEqual(record["advisory"]["source"], None)
                self.assertEqual(
                    record["advisory"]["categories"],
                    EXPECTED_RANGES_AND_CATEGORIES[advisory_id]["categories"],
                )
                self.assertEqual(record["advisory"]["cvss"], None)
                self.assertEqual(record["advisory"]["informational"], None)
                self.assertEqual(record["advisory"]["withdrawn"], None)
                self.assertEqual(
                    record["versions"]["patched"],
                    EXPECTED_RANGES_AND_CATEGORIES[advisory_id]["patched"],
                )
                self.assertEqual(
                    record["versions"]["unaffected"],
                    EXPECTED_RANGES_AND_CATEGORIES[advisory_id]["unaffected"],
                )
                self.assertEqual(record["affected"], None)
                self.assertEqual(record["package"]["name"], "rustls-webpki")
                self.assertEqual(record["package"]["version"], "0.102.8")
                self.assertEqual(
                    record["package"]["source"],
                    "registry+https://github.com/rust-lang/crates.io-index",
                )
                self.assertEqual(record["package"]["checksum"], PACKAGE_CHECKSUM)

    def test_exact_exit_one_report_passes_and_says_not_clean(self) -> None:
        status, stdout, stderr, run = self.run_check()

        self.assertEqual(status, 0)
        self.assertEqual(stderr, "")
        self.assertEqual(
            stdout,
            "rustsec-debt-check: accepted temporary debt matches: "
            "4 vulnerabilities remain; audit is NOT clean\n"
            "rustsec-debt-check: informational warnings "
            "(not accepted vulnerability debt): 3 (unmaintained=3)\n",
        )
        self.assertIn("audit is NOT clean", stdout)
        self.assertNotIn("audit clean", stdout.lower())
        commands = [call.args[0] for call in run.call_args_list]
        self.assertEqual(
            commands,
            [["cargo", "audit", "--version"], ["cargo", "audit", "--json"]],
        )
        self.assertFalse(any("--ignore" in argument for command in commands for argument in command))

    def test_exact_exit_zero_report_is_also_accepted(self) -> None:
        status, stdout, stderr, _ = self.run_check(report_returncode=0)

        self.assertEqual(status, 0)
        self.assertEqual(stderr, "")
        self.assertIn("4 vulnerabilities remain; audit is NOT clean", stdout)

    def test_advisory_database_metadata_is_not_part_of_the_fingerprint(self) -> None:
        report = exact_report()
        report["database"] = {
            "advisory-count": 9_999_999,
            "last-commit": "database-metadata-is-not-accepted-debt",
            "last-updated": "2099-01-01",
        }

        status, stdout, stderr, _ = self.run_check(report)

        self.assertEqual(status, 0)
        self.assertEqual(stderr, "")
        self.assertIn("audit is NOT clean", stdout)

    def test_each_non_default_filter_or_ignore_setting_is_rejected(self) -> None:
        cases = (
            (
                "ignore",
                ["RUSTSEC-2026-0104"],
                "advisory ignore list must be empty",
            ),
            (
                "target_arch",
                ["x86_64"],
                "target architecture filter must be empty",
            ),
            (
                "target_os",
                ["linux"],
                "target OS filter must be empty",
            ),
            ("severity", "high", "severity filter must be unset"),
        )
        for setting, value, failure in cases:
            with self.subTest(setting=setting):
                report = exact_report()
                report["settings"][setting] = value

                result = self.run_check(report)

                self.assert_failure(result, failure)

    def test_every_approved_informational_warning_class_is_required(self) -> None:
        approved = ("unmaintained", "unsound", "notice")
        for omitted in approved:
            with self.subTest(omitted=omitted):
                report = exact_report()
                report["settings"]["informational_warnings"] = [
                    warning for warning in approved if warning != omitted
                ]

                result = self.run_check(report)

                self.assert_failure(
                    result,
                    "informational warning settings mismatch",
                )

    def test_extra_informational_warning_setting_is_rejected(self) -> None:
        report = exact_report()
        report["settings"]["informational_warnings"].append("yanked")

        result = self.run_check(report)

        self.assert_failure(result, "informational warning settings mismatch")

    def test_informational_warning_setting_order_is_not_a_filter(self) -> None:
        report = exact_report()
        report["settings"]["informational_warnings"].reverse()

        status, stdout, stderr, _ = self.run_check(report)

        self.assertEqual(status, 0)
        self.assertEqual(stderr, "")
        self.assertIn("audit is NOT clean", stdout)

    def test_missing_or_unknown_report_setting_fails_closed(self) -> None:
        for mutation in ("missing", "unknown"):
            with self.subTest(mutation=mutation):
                report = exact_report()
                if mutation == "missing":
                    del report["settings"]["target_os"]
                else:
                    report["settings"]["future_suppression"] = True

                result = self.run_check(report)

                self.assert_failure(result, "cargo-audit settings schema mismatch")

    def test_informational_warnings_are_reported_but_not_accepted_as_vulnerabilities(self) -> None:
        report = exact_report()
        report["warnings"]["unsound"] = [{}, {}]

        status, stdout, stderr, _ = self.run_check(report)

        self.assertEqual(status, 0)
        self.assertEqual(stderr, "")
        self.assertIn(
            "informational warnings (not accepted vulnerability debt): "
            "5 (unmaintained=3, unsound=2)",
            stdout,
        )
        self.assertIn("4 vulnerabilities remain", stdout)

    def test_missing_advisory_is_rejected(self) -> None:
        report = exact_report()
        removed = report["vulnerabilities"]["list"].pop()
        synchronize_count(report)

        result = self.run_check(report)

        self.assert_failure(result, f"missing {removed['advisory']['id']}")

    def test_extra_advisory_is_rejected(self) -> None:
        report = exact_report()
        extra = copy.deepcopy(report["vulnerabilities"]["list"][0])
        extra["advisory"]["id"] = "RUSTSEC-2026-9999"
        report["vulnerabilities"]["list"].append(extra)
        synchronize_count(report)

        result = self.run_check(report)

        self.assert_failure(result, "unexpected RUSTSEC-2026-9999")

    def test_extra_advisory_diagnostics_are_bounded(self) -> None:
        report = exact_report()
        template = report["vulnerabilities"]["list"][0]
        for suffix in range(9000, 9020):
            extra = copy.deepcopy(template)
            extra["advisory"]["id"] = f"RUSTSEC-2026-{suffix}"
            report["vulnerabilities"]["list"].append(extra)
        synchronize_count(report)

        result = self.run_check(report)

        self.assert_failure(result, "+12 more")
        self.assertNotIn("RUSTSEC-2026-9019", result[2])

    def test_every_material_fingerprint_field_is_bound(self) -> None:
        mutations = (
            ("advisory source", ("advisory", "source"), "registry:changed"),
            ("aliases", ("advisory", "aliases"), ["GHSA-0000-0000-0000"]),
            ("categories", ("advisory", "categories"), ["code-execution"]),
            ("cvss", ("advisory", "cvss"), "CVSS:3.1/AV:N/AC:L"),
            ("informational", ("advisory", "informational"), "unmaintained"),
            ("withdrawn", ("advisory", "withdrawn"), "2026-09-02"),
            ("patched", ("versions", "patched"), [">=999.0.0"]),
            ("unaffected", ("versions", "unaffected"), ["<0.1.0"]),
            ("affected", ("affected",), {"os": ["linux"]}),
            ("package version", ("package", "version"), "0.102.9"),
            (
                "package source",
                ("package", "source"),
                "registry+https://example.invalid/index",
            ),
            ("package checksum", ("package", "checksum"), "0" * 64),
        )
        for name, path, value in mutations:
            with self.subTest(field=name):
                report = exact_report()
                set_nested(report["vulnerabilities"]["list"][0], path, value)

                result = self.run_check(report)

                self.assert_failure(result, "vulnerability fingerprint mismatch")

    def test_changed_package_identity_is_rejected(self) -> None:
        report = exact_report()
        record = report["vulnerabilities"]["list"][0]
        record["advisory"]["package"] = "rustls-webpki-changed"
        record["package"]["name"] = "rustls-webpki-changed"

        result = self.run_check(report)

        self.assert_failure(result, "vulnerability fingerprint mismatch")

    def test_malformed_report_json_fails_closed_without_echo(self) -> None:
        sentinel = "DO-NOT-ECHO-REPORT-CONTENT"

        result = self.run_check(report_text="{" + sentinel)

        self.assert_failure(result, "cargo-audit report is malformed JSON")
        self.assertNotIn(sentinel, result[2])

    def test_declared_vulnerability_count_must_match_the_list(self) -> None:
        report = exact_report()
        report["vulnerabilities"]["count"] = 5

        result = self.run_check(report)

        self.assert_failure(
            result,
            "vulnerability count mismatch (declared 5, observed 4)",
        )

    def test_found_state_must_match_the_count(self) -> None:
        report = exact_report()
        report["vulnerabilities"]["found"] = False

        result = self.run_check(report)

        self.assert_failure(result, "vulnerability found/count mismatch")

    def test_malformed_version_output_fails_without_running_the_report(self) -> None:
        status, stdout, stderr, run = self.run_check(
            version_output="cargo audit version unknown\n"
        )

        self.assertEqual(status, 1)
        self.assertEqual(stdout, "")
        self.assertIn("version output is malformed", stderr)
        self.assertEqual(run.call_count, 1)

    def test_wrong_cargo_audit_tooling_version_fails_separately(self) -> None:
        status, stdout, stderr, run = self.run_check(
            version_output="cargo-audit-audit 0.23.0\n"
        )

        self.assertEqual(status, 1)
        self.assertEqual(stdout, "")
        self.assertIn(
            "tooling version mismatch (expected 0.22.2, observed 0.23.0)",
            stderr,
        )
        self.assertEqual(run.call_count, 1)

    def test_version_command_failure_does_not_echo_tool_output(self) -> None:
        sentinel = "DO-NOT-ECHO-CARGO-AUDIT-STDERR"
        result = self.run_check(
            version_returncode=2,
            version_stderr=sentinel,
        )

        self.assert_failure(result, "cargo-audit version check failed")
        self.assertNotIn(sentinel, result[2])

    def test_report_exit_other_than_zero_or_one_fails_without_echo(self) -> None:
        sentinel = "DO-NOT-ECHO-CARGO-AUDIT-REPORT-STDERR"
        result = self.run_check(
            report_returncode=2,
            report_stderr=sentinel,
        )

        self.assert_failure(result, "report command failed with exit 2")
        self.assertNotIn(sentinel, result[2])


if __name__ == "__main__":
    unittest.main()
