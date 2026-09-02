#!/usr/bin/env python3
"""Fail unless cargo-audit reports Abbey's exact, explicitly non-clean debt."""

from __future__ import annotations

import json
from pathlib import Path
import re
import subprocess
import sys
from typing import Any


ROOT = Path(__file__).resolve().parent.parent
POLICY_PATH = ROOT / "security/rustsec-accepted-debt.json"

POLICY_SCHEMA_VERSION = 1
CARGO_AUDIT_VERSION = "0.22.2"
APPROVED_ADVISORY_IDS = frozenset(
    {
        "RUSTSEC-2026-0049",
        "RUSTSEC-2026-0098",
        "RUSTSEC-2026-0099",
        "RUSTSEC-2026-0104",
    }
)
APPROVED_INFORMATIONAL_WARNINGS = frozenset(
    {"notice", "unmaintained", "unsound"}
)

MAX_JSON_BYTES = 8 * 1024 * 1024
MAX_REPORT_RECORDS = 128
MAX_WARNING_KINDS = 32
MAX_DIAGNOSTIC_ITEMS = 8
MAX_AFFECTED_FINGERPRINT_BYTES = 16 * 1024

_VERSION_OUTPUT = re.compile(r"cargo-audit(?:-audit)? ([0-9]+\.[0-9]+\.[0-9]+)")
_ADVISORY_ID = re.compile(r"RUSTSEC-[0-9]{4}-[0-9]{4}")
_WARNING_KIND = re.compile(r"[a-z0-9][a-z0-9_-]{0,39}")
_CHECKSUM = re.compile(r"[0-9a-f]{64}")

_POLICY_KEYS = {
    "schema_version",
    "cargo_audit_version",
    "audit_state",
    "rationale",
    "review_triggers",
    "accepted_vulnerability_count",
    "accepted_vulnerabilities",
}
_POLICY_RECORD_KEYS = {"id", "advisory", "versions", "affected", "package"}
_POLICY_ADVISORY_KEYS = {
    "package",
    "source",
    "aliases",
    "categories",
    "cvss",
    "informational",
    "withdrawn",
}
_POLICY_VERSION_KEYS = {"patched", "unaffected"}
_POLICY_PACKAGE_KEYS = {"name", "version", "source", "checksum"}
_REPORT_SETTINGS_KEYS = {
    "target_arch",
    "target_os",
    "severity",
    "ignore",
    "informational_warnings",
}


class DebtCheckError(Exception):
    """A bounded, user-safe contract failure."""


def _reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("duplicate JSON key")
        value[key] = item
    return value


def _reject_non_finite_number(_: str) -> None:
    raise ValueError("non-finite JSON number")


def _parse_json(text: str, *, label: str) -> Any:
    if len(text.encode("utf-8")) > MAX_JSON_BYTES:
        raise DebtCheckError(f"{label} exceeds the size limit")
    try:
        return json.loads(
            text,
            object_pairs_hook=_reject_duplicate_keys,
            parse_constant=_reject_non_finite_number,
        )
    except (json.JSONDecodeError, UnicodeError, ValueError):
        raise DebtCheckError(f"{label} is malformed JSON") from None


def _mapping(value: Any, *, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise DebtCheckError(f"{label} is malformed")
    return value


def _list(value: Any, *, label: str) -> list[Any]:
    if not isinstance(value, list):
        raise DebtCheckError(f"{label} is malformed")
    return value


def _required(mapping: dict[str, Any], key: str, *, label: str) -> Any:
    if key not in mapping:
        raise DebtCheckError(f"{label} is malformed")
    return mapping[key]


def _text(value: Any, *, label: str, maximum: int = 512) -> str:
    if not isinstance(value, str) or not value or len(value) > maximum:
        raise DebtCheckError(f"{label} is malformed")
    return value


def _optional_text(value: Any, *, label: str, maximum: int = 512) -> str | None:
    if value is None:
        return None
    return _text(value, label=label, maximum=maximum)


def _integer(
    value: Any,
    *,
    label: str,
    maximum: int = MAX_REPORT_RECORDS,
) -> int:
    if type(value) is not int or value < 0 or value > maximum:
        raise DebtCheckError(f"{label} is malformed")
    return value


def _string_set(value: Any, *, label: str) -> tuple[str, ...]:
    items = _list(value, label=label)
    if len(items) > MAX_REPORT_RECORDS:
        raise DebtCheckError(f"{label} has too many entries")
    strings = tuple(_text(item, label=label) for item in items)
    if len(set(strings)) != len(strings):
        raise DebtCheckError(f"{label} contains duplicate entries")
    return tuple(sorted(strings))


def _canonical_affected(value: Any, *, label: str) -> str:
    try:
        encoded = json.dumps(value, sort_keys=True, separators=(",", ":"))
    except (TypeError, ValueError):
        raise DebtCheckError(f"{label} is malformed") from None
    if len(encoded.encode("utf-8")) > MAX_AFFECTED_FINGERPRINT_BYTES:
        raise DebtCheckError(f"{label} exceeds the size limit")
    return encoded


def _exact_keys(mapping: dict[str, Any], expected: set[str], *, label: str) -> None:
    if set(mapping) != expected:
        raise DebtCheckError(f"{label} schema mismatch")


def _material_record(
    value: Any,
    *,
    label: str,
    policy_record: bool,
) -> tuple[str, dict[str, Any]]:
    record = _mapping(value, label=label)
    if policy_record:
        _exact_keys(record, _POLICY_RECORD_KEYS, label=label)

    advisory = _mapping(_required(record, "advisory", label=label), label=label)
    versions = _mapping(_required(record, "versions", label=label), label=label)
    package = _mapping(_required(record, "package", label=label), label=label)
    if policy_record:
        _exact_keys(advisory, _POLICY_ADVISORY_KEYS, label=f"{label} advisory")
        _exact_keys(versions, _POLICY_VERSION_KEYS, label=f"{label} versions")
        _exact_keys(package, _POLICY_PACKAGE_KEYS, label=f"{label} package")

    raw_id = (
        _required(record, "id", label=label)
        if policy_record
        else _required(advisory, "id", label=label)
    )
    advisory_id = _text(raw_id, label=f"{label} advisory id", maximum=32)
    if _ADVISORY_ID.fullmatch(advisory_id) is None:
        raise DebtCheckError(f"{label} advisory id is malformed")

    advisory_package = _text(
        _required(advisory, "package", label=label),
        label=f"{label} advisory package",
    )
    package_name = _text(
        _required(package, "name", label=label),
        label=f"{label} package name",
    )
    if advisory_package != package_name:
        raise DebtCheckError(f"{label} package identity mismatch")

    checksum = _text(
        _required(package, "checksum", label=label),
        label=f"{label} package checksum",
        maximum=64,
    )
    if _CHECKSUM.fullmatch(checksum) is None:
        raise DebtCheckError(f"{label} package checksum is malformed")

    fingerprint = {
        "advisory.package": advisory_package,
        "advisory.source": _optional_text(
            _required(advisory, "source", label=label),
            label=f"{label} advisory source",
        ),
        "advisory.aliases": _string_set(
            _required(advisory, "aliases", label=label),
            label=f"{label} aliases",
        ),
        "advisory.categories": _string_set(
            _required(advisory, "categories", label=label),
            label=f"{label} categories",
        ),
        "advisory.cvss": _optional_text(
            _required(advisory, "cvss", label=label),
            label=f"{label} CVSS",
        ),
        "advisory.informational": _optional_text(
            _required(advisory, "informational", label=label),
            label=f"{label} informational state",
        ),
        "advisory.withdrawn": _optional_text(
            _required(advisory, "withdrawn", label=label),
            label=f"{label} withdrawal state",
        ),
        "versions.patched": _string_set(
            _required(versions, "patched", label=label),
            label=f"{label} patched ranges",
        ),
        "versions.unaffected": _string_set(
            _required(versions, "unaffected", label=label),
            label=f"{label} unaffected ranges",
        ),
        "affected": _canonical_affected(
            _required(record, "affected", label=label),
            label=f"{label} affected state",
        ),
        "package.name": package_name,
        "package.version": _text(
            _required(package, "version", label=label),
            label=f"{label} package version",
        ),
        "package.source": _text(
            _required(package, "source", label=label),
            label=f"{label} package source",
            maximum=1024,
        ),
        "package.checksum": checksum,
    }
    return advisory_id, fingerprint


def load_policy(path: Path = POLICY_PATH) -> tuple[str, dict[str, dict[str, Any]]]:
    try:
        policy_text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError):
        raise DebtCheckError("accepted-debt policy is unavailable") from None

    policy = _mapping(
        _parse_json(policy_text, label="accepted-debt policy"),
        label="accepted-debt policy",
    )
    _exact_keys(policy, _POLICY_KEYS, label="accepted-debt policy")

    schema_version = _integer(
        _required(policy, "schema_version", label="accepted-debt policy"),
        label="accepted-debt policy schema version",
    )
    if schema_version != POLICY_SCHEMA_VERSION:
        raise DebtCheckError("accepted-debt policy schema version mismatch")
    version = _text(
        _required(policy, "cargo_audit_version", label="accepted-debt policy"),
        label="accepted-debt cargo-audit version",
        maximum=32,
    )
    if version != CARGO_AUDIT_VERSION:
        raise DebtCheckError("accepted-debt cargo-audit version mismatch")
    if _required(policy, "audit_state", label="accepted-debt policy") != "not-clean":
        raise DebtCheckError("accepted-debt policy must remain explicitly not-clean")
    _text(
        _required(policy, "rationale", label="accepted-debt policy"),
        label="accepted-debt rationale",
        maximum=4096,
    )
    review_triggers = _string_set(
        _required(policy, "review_triggers", label="accepted-debt policy"),
        label="accepted-debt review triggers",
    )
    if not review_triggers:
        raise DebtCheckError("accepted-debt review triggers are missing")

    expected_count = _integer(
        _required(policy, "accepted_vulnerability_count", label="accepted-debt policy"),
        label="accepted-debt vulnerability count",
    )
    if expected_count != len(APPROVED_ADVISORY_IDS):
        raise DebtCheckError("accepted-debt vulnerability count is not the approved count")
    records = _list(
        _required(policy, "accepted_vulnerabilities", label="accepted-debt policy"),
        label="accepted-debt vulnerabilities",
    )
    if len(records) != expected_count:
        raise DebtCheckError("accepted-debt policy count mismatch")

    fingerprints: dict[str, dict[str, Any]] = {}
    for index, record in enumerate(records):
        advisory_id, fingerprint = _material_record(
            record,
            label=f"accepted-debt record {index + 1}",
            policy_record=True,
        )
        if advisory_id in fingerprints:
            raise DebtCheckError("accepted-debt policy contains a duplicate advisory")
        fingerprints[advisory_id] = fingerprint
    if set(fingerprints) != APPROVED_ADVISORY_IDS:
        raise DebtCheckError("accepted-debt policy advisory inventory mismatch")
    return version, fingerprints


def parse_cargo_audit_version(output: str) -> str:
    if len(output) > 128 or "\n" in output.strip("\n"):
        raise DebtCheckError("cargo-audit version output is malformed")
    match = _VERSION_OUTPUT.fullmatch(output.strip())
    if match is None:
        raise DebtCheckError("cargo-audit version output is malformed")
    return match.group(1)


def _parse_report(
    report_text: str,
) -> tuple[dict[str, dict[str, Any]], tuple[tuple[str, int], ...]]:
    report = _mapping(
        _parse_json(report_text, label="cargo-audit report"),
        label="cargo-audit report",
    )
    settings = _mapping(
        _required(report, "settings", label="cargo-audit report"),
        label="cargo-audit settings",
    )
    _exact_keys(settings, _REPORT_SETTINGS_KEYS, label="cargo-audit settings")
    ignored = _string_set(
        _required(settings, "ignore", label="cargo-audit settings"),
        label="cargo-audit advisory ignore list",
    )
    if ignored:
        raise DebtCheckError("cargo-audit advisory ignore list must be empty")
    target_arch = _string_set(
        _required(settings, "target_arch", label="cargo-audit settings"),
        label="cargo-audit target architecture filter",
    )
    if target_arch:
        raise DebtCheckError("cargo-audit target architecture filter must be empty")
    target_os = _string_set(
        _required(settings, "target_os", label="cargo-audit settings"),
        label="cargo-audit target OS filter",
    )
    if target_os:
        raise DebtCheckError("cargo-audit target OS filter must be empty")
    if _required(settings, "severity", label="cargo-audit settings") is not None:
        raise DebtCheckError("cargo-audit severity filter must be unset")
    informational_warnings = frozenset(
        _string_set(
            _required(
                settings,
                "informational_warnings",
                label="cargo-audit settings",
            ),
            label="cargo-audit informational warning settings",
        )
    )
    if informational_warnings != APPROVED_INFORMATIONAL_WARNINGS:
        raise DebtCheckError("cargo-audit informational warning settings mismatch")

    vulnerabilities = _mapping(
        _required(report, "vulnerabilities", label="cargo-audit report"),
        label="cargo-audit vulnerabilities",
    )
    found = _required(vulnerabilities, "found", label="cargo-audit vulnerabilities")
    if type(found) is not bool:
        raise DebtCheckError("cargo-audit vulnerability found state is malformed")
    count = _integer(
        _required(vulnerabilities, "count", label="cargo-audit vulnerabilities"),
        label="cargo-audit vulnerability count",
    )
    records = _list(
        _required(vulnerabilities, "list", label="cargo-audit vulnerabilities"),
        label="cargo-audit vulnerability list",
    )
    if len(records) > MAX_REPORT_RECORDS:
        raise DebtCheckError("cargo-audit vulnerability inventory exceeds the limit")
    if count != len(records):
        raise DebtCheckError(
            f"cargo-audit vulnerability count mismatch (declared {count}, observed {len(records)})"
        )
    if found != (count != 0):
        raise DebtCheckError("cargo-audit vulnerability found/count mismatch")

    fingerprints: dict[str, dict[str, Any]] = {}
    for index, record in enumerate(records):
        advisory_id, fingerprint = _material_record(
            record,
            label=f"cargo-audit vulnerability record {index + 1}",
            policy_record=False,
        )
        if advisory_id in fingerprints:
            raise DebtCheckError("cargo-audit report contains a duplicate advisory")
        fingerprints[advisory_id] = fingerprint

    warnings = _mapping(
        _required(report, "warnings", label="cargo-audit report"),
        label="cargo-audit warnings",
    )
    if len(warnings) > MAX_WARNING_KINDS:
        raise DebtCheckError("cargo-audit warning categories exceed the limit")
    warning_counts: list[tuple[str, int]] = []
    for kind, warning_records in warnings.items():
        if not isinstance(kind, str) or _WARNING_KIND.fullmatch(kind) is None:
            raise DebtCheckError("cargo-audit warning category is malformed")
        entries = _list(warning_records, label="cargo-audit warning records")
        if len(entries) > MAX_REPORT_RECORDS:
            raise DebtCheckError("cargo-audit warning inventory exceeds the limit")
        warning_counts.append((kind, len(entries)))
    return fingerprints, tuple(sorted(warning_counts))


def _bounded(values: list[str]) -> str:
    shown = values[:MAX_DIAGNOSTIC_ITEMS]
    rendered = ", ".join(shown)
    remaining = len(values) - len(shown)
    if remaining:
        rendered += f", +{remaining} more"
    return rendered


def compare_report(
    expected: dict[str, dict[str, Any]],
    observed: dict[str, dict[str, Any]],
) -> None:
    missing = sorted(set(expected) - set(observed))
    unexpected = sorted(set(observed) - set(expected))
    inventory_problems: list[str] = []
    if missing:
        inventory_problems.append("missing " + _bounded(missing))
    if unexpected:
        inventory_problems.append("unexpected " + _bounded(unexpected))
    if inventory_problems:
        raise DebtCheckError(
            "cargo-audit vulnerability inventory mismatch: "
            + "; ".join(inventory_problems)
        )

    changed: list[str] = []
    for advisory_id in sorted(expected):
        fields = sorted(
            field
            for field, expected_value in expected[advisory_id].items()
            if observed[advisory_id].get(field) != expected_value
        )
        if fields:
            changed.append(f"{advisory_id} ({_bounded(fields)})")
    if changed:
        raise DebtCheckError(
            "cargo-audit vulnerability fingerprint mismatch: " + _bounded(changed)
        )


def _run(command: list[str]) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            command,
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError:
        raise DebtCheckError("cargo-audit tooling is unavailable") from None


def main() -> int:
    try:
        expected_version, expected = load_policy()

        version_result = _run(["cargo", "audit", "--version"])
        if version_result.returncode != 0:
            raise DebtCheckError("cargo-audit version check failed")
        observed_version = parse_cargo_audit_version(version_result.stdout)
        if observed_version != expected_version:
            raise DebtCheckError(
                "cargo-audit tooling version mismatch "
                f"(expected {expected_version}, observed {observed_version})"
            )

        report_result = _run(["cargo", "audit", "--json"])
        if report_result.returncode not in (0, 1):
            raise DebtCheckError(
                f"cargo-audit report command failed with exit {report_result.returncode}"
            )
        observed, warning_counts = _parse_report(report_result.stdout)
        compare_report(expected, observed)
    except DebtCheckError as error:
        print(f"rustsec-debt-check: FAIL: {error}", file=sys.stderr)
        return 1

    warning_total = sum(count for _, count in warning_counts)
    warning_detail = (
        ", ".join(f"{kind}={count}" for kind, count in warning_counts)
        if warning_counts
        else "none"
    )
    print(
        "rustsec-debt-check: accepted temporary debt matches: "
        f"{len(observed)} vulnerabilities remain; audit is NOT clean"
    )
    print(
        "rustsec-debt-check: informational warnings "
        f"(not accepted vulnerability debt): {warning_total} ({warning_detail})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
