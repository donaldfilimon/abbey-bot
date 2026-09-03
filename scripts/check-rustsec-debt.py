#!/usr/bin/env python3
"""Fail unless cargo-audit and Cargo's locked graph match Abbey's exact debt."""

from __future__ import annotations

import json
from pathlib import Path
import re
import subprocess
import sys
from typing import Any


ROOT = Path(__file__).resolve().parent.parent
POLICY_PATH = ROOT / "security/rustsec-accepted-debt.json"
LOCKFILE_PATH = ROOT / "Cargo.lock"

POLICY_SCHEMA_VERSION = 2
CARGO_AUDIT_VERSION = "0.22.2"
RESOLUTION_SCOPE = "all-resolved-targets"
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
APPROVED_DEPENDENCY_PATH = (
    ("serenity", "0.12.5"),
    ("tokio-tungstenite", "0.21.0"),
    ("rustls", "0.22.4"),
    ("rustls-webpki", "0.102.8"),
)
APPROVED_DEPENDENCY_EDGES = ("tokio_tungstenite", "rustls", "webpki")

MAX_JSON_BYTES = 8 * 1024 * 1024
MAX_LOCKFILE_BYTES = 8 * 1024 * 1024
MAX_REPORT_RECORDS = 128
MAX_WARNING_KINDS = 32
MAX_DIAGNOSTIC_ITEMS = 8
MAX_AFFECTED_FINGERPRINT_BYTES = 16 * 1024
MAX_METADATA_PACKAGES = 2048
MAX_METADATA_DEPENDENCIES = 512

_VERSION_OUTPUT = re.compile(r"cargo-audit(?:-audit)? ([0-9]+\.[0-9]+\.[0-9]+)")
_ADVISORY_ID = re.compile(r"RUSTSEC-[0-9]{4}-[0-9]{4}")
_WARNING_KIND = re.compile(r"[a-z0-9][a-z0-9_-]{0,39}")
_CHECKSUM = re.compile(r"[0-9a-f]{64}")
_LOCKFILE_STRING_FIELD = re.compile(
    r'(?P<key>name|version|source|checksum|replace) = "(?P<value>[^"\r\n]{1,2048})"'
)
_LOCKFILE_RELEVANT_PREFIXES = tuple(
    f"{field} =" for field in ("name", "version", "source", "checksum", "replace")
)

_POLICY_KEYS = {
    "schema_version",
    "cargo_audit_version",
    "audit_state",
    "rationale",
    "review_triggers",
    "locked_dependency_path",
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
_POLICY_PACKAGE_KEYS = {
    "name",
    "version",
    "source",
    "checksum",
    "dependencies",
    "replace",
}
_AUDIT_DEPENDENCY_KEYS = {"name", "version", "source"}
_DEPENDENCY_PATH_KEYS = {"scope", "nodes"}
_DEPENDENCY_PATH_NODE_KEYS = {
    "name",
    "version",
    "source",
    "checksum",
    "replace",
    "dependency_to_next",
}
_DEPENDENCY_PATH_EDGE_KEYS = {"name", "kinds"}
_DEPENDENCY_KIND_KEYS = {"kind", "target"}
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


def _canonical_json(value: Any, *, label: str) -> str:
    try:
        encoded = json.dumps(value, sort_keys=True, separators=(",", ":"))
    except (TypeError, ValueError):
        raise DebtCheckError(f"{label} is malformed") from None
    if len(encoded.encode("utf-8")) > MAX_AFFECTED_FINGERPRINT_BYTES:
        raise DebtCheckError(f"{label} exceeds the size limit")
    return encoded


def _dependency_kinds(
    value: Any,
    *,
    label: str,
) -> tuple[tuple[str | None, str | None], ...]:
    items = _list(value, label=label)
    if not items or len(items) > MAX_REPORT_RECORDS:
        raise DebtCheckError(f"{label} is malformed")
    kinds: list[tuple[str | None, str | None]] = []
    for index, item in enumerate(items):
        kind = _mapping(item, label=f"{label} entry {index + 1}")
        _exact_keys(
            kind,
            _DEPENDENCY_KIND_KEYS,
            label=f"{label} entry {index + 1}",
        )
        kinds.append(
            (
                _optional_text(
                    _required(kind, "kind", label=label),
                    label=f"{label} kind",
                    maximum=32,
                ),
                _optional_text(
                    _required(kind, "target", label=label),
                    label=f"{label} target",
                    maximum=2048,
                ),
            )
        )
    if len(set(kinds)) != len(kinds):
        raise DebtCheckError(f"{label} contains duplicate entries")
    return tuple(sorted(kinds, key=repr))


def _audit_dependencies(
    value: Any,
    *,
    label: str,
) -> tuple[tuple[str, str, str], ...]:
    items = _list(value, label=label)
    if len(items) > MAX_REPORT_RECORDS:
        raise DebtCheckError(f"{label} has too many entries")
    dependencies: list[tuple[str, str, str]] = []
    for index, item in enumerate(items):
        dependency = _mapping(item, label=f"{label} entry {index + 1}")
        _exact_keys(
            dependency,
            _AUDIT_DEPENDENCY_KEYS,
            label=f"{label} entry {index + 1}",
        )
        dependencies.append(
            (
                _text(
                    _required(dependency, "name", label=label),
                    label=f"{label} name",
                ),
                _text(
                    _required(dependency, "version", label=label),
                    label=f"{label} version",
                ),
                _text(
                    _required(dependency, "source", label=label),
                    label=f"{label} source",
                    maximum=1024,
                ),
            )
        )
    if len(set(dependencies)) != len(dependencies):
        raise DebtCheckError(f"{label} contains duplicate entries")
    return tuple(sorted(dependencies))


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
        "affected": _canonical_json(
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
        "package.dependencies": _audit_dependencies(
            _required(package, "dependencies", label=label),
            label=f"{label} package dependencies",
        ),
        "package.replace": _canonical_json(
            _required(package, "replace", label=label),
            label=f"{label} package replacement state",
        ),
    }
    return advisory_id, fingerprint


def _load_dependency_path_policy(
    value: Any,
) -> tuple[str, tuple[dict[str, Any], ...]]:
    policy = _mapping(value, label="accepted-debt locked dependency path")
    _exact_keys(
        policy,
        _DEPENDENCY_PATH_KEYS,
        label="accepted-debt locked dependency path",
    )
    scope = _text(
        _required(policy, "scope", label="accepted-debt locked dependency path"),
        label="accepted-debt locked dependency scope",
        maximum=128,
    )
    if scope != RESOLUTION_SCOPE:
        raise DebtCheckError("accepted-debt locked dependency scope mismatch")
    records = _list(
        _required(policy, "nodes", label="accepted-debt locked dependency path"),
        label="accepted-debt locked dependency path nodes",
    )
    if len(records) != len(APPROVED_DEPENDENCY_PATH):
        raise DebtCheckError("accepted-debt locked dependency path length mismatch")

    path: list[dict[str, Any]] = []
    for index, value_record in enumerate(records):
        label = f"accepted-debt locked dependency path node {index + 1}"
        record = _mapping(value_record, label=label)
        _exact_keys(record, _DEPENDENCY_PATH_NODE_KEYS, label=label)
        node = {
            "name": _text(
                _required(record, "name", label=label),
                label=f"{label} name",
            ),
            "version": _text(
                _required(record, "version", label=label),
                label=f"{label} version",
            ),
            "source": _text(
                _required(record, "source", label=label),
                label=f"{label} source",
                maximum=1024,
            ),
            "checksum": _text(
                _required(record, "checksum", label=label),
                label=f"{label} checksum",
                maximum=64,
            ),
            "replace": _required(record, "replace", label=label),
        }
        if _CHECKSUM.fullmatch(node["checksum"]) is None:
            raise DebtCheckError(f"{label} checksum is malformed")
        if node["replace"] is not None:
            raise DebtCheckError(
                "accepted-debt locked dependency path must remain unreplaced"
            )

        raw_edge = _required(record, "dependency_to_next", label=label)
        if index == len(records) - 1:
            if raw_edge is not None:
                raise DebtCheckError(
                    "accepted-debt locked dependency path terminal edge mismatch"
                )
            node["dependency_to_next"] = None
        else:
            edge = _mapping(raw_edge, label=f"{label} edge")
            _exact_keys(edge, _DEPENDENCY_PATH_EDGE_KEYS, label=f"{label} edge")
            node["dependency_to_next"] = {
                "name": _text(
                    _required(edge, "name", label=f"{label} edge"),
                    label=f"{label} edge name",
                ),
                "kinds": _dependency_kinds(
                    _required(edge, "kinds", label=f"{label} edge"),
                    label=f"{label} edge kinds",
                ),
            }
        path.append(node)

    identities = tuple((node["name"], node["version"]) for node in path)
    if identities != APPROVED_DEPENDENCY_PATH:
        raise DebtCheckError("accepted-debt locked dependency path identity mismatch")
    edges = tuple(node["dependency_to_next"]["name"] for node in path[:-1])
    if edges != APPROVED_DEPENDENCY_EDGES:
        raise DebtCheckError("accepted-debt locked dependency path edge mismatch")
    return scope, tuple(path)


def load_policy(
    path: Path = POLICY_PATH,
) -> tuple[
    str,
    dict[str, dict[str, Any]],
    str,
    tuple[dict[str, Any], ...],
]:
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
    dependency_scope, dependency_path = _load_dependency_path_policy(
        _required(policy, "locked_dependency_path", label="accepted-debt policy")
    )

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
    return version, fingerprints, dependency_scope, dependency_path


def _verify_lockfile_path(
    lockfile_text: str,
    expected_path: tuple[dict[str, Any], ...],
) -> None:
    if len(lockfile_text.encode("utf-8")) > MAX_LOCKFILE_BYTES:
        raise DebtCheckError("Cargo.lock exceeds the size limit")
    packages: list[dict[str, str]] = []
    current: dict[str, str] | None = None
    format_version_seen = False
    for line in lockfile_text.splitlines():
        if line == "[[package]]":
            if current is not None:
                packages.append(current)
            current = {}
            continue
        if line.startswith("["):
            if current is not None:
                packages.append(current)
                current = None
            continue
        if current is None:
            if line.startswith("version ="):
                if format_version_seen or line != "version = 4":
                    raise DebtCheckError("Cargo.lock format version mismatch")
                format_version_seen = True
            continue
        match = _LOCKFILE_STRING_FIELD.fullmatch(line)
        if match is not None:
            key = match.group("key")
            if key in current:
                raise DebtCheckError("Cargo.lock package contains a duplicate field")
            current[key] = match.group("value")
        elif line.startswith(_LOCKFILE_RELEVANT_PREFIXES):
            raise DebtCheckError("Cargo.lock package field is malformed")
    if current is not None:
        packages.append(current)
    if not format_version_seen:
        raise DebtCheckError("Cargo.lock format version mismatch")
    if len(packages) > MAX_METADATA_PACKAGES:
        raise DebtCheckError("Cargo.lock package inventory exceeds the limit")

    for expected in expected_path:
        matches: list[dict[str, str]] = []
        for package in packages:
            if (
                package.get("name") == expected["name"]
                and package.get("version") == expected["version"]
            ):
                matches.append(package)
        identity = f"{expected['name']} {expected['version']}"
        if len(matches) != 1:
            raise DebtCheckError(
                f"Cargo.lock dependency path package identity mismatch: {identity}"
            )
        package = matches[0]
        source = _text(
            _required(package, "source", label="Cargo.lock package"),
            label="Cargo.lock package source",
            maximum=1024,
        )
        checksum = _text(
            _required(package, "checksum", label="Cargo.lock package"),
            label="Cargo.lock package checksum",
            maximum=64,
        )
        if _CHECKSUM.fullmatch(checksum) is None:
            raise DebtCheckError("Cargo.lock package checksum is malformed")
        replacement = package.get("replace")
        changed = sorted(
            field
            for field, observed in (
                ("source", source),
                ("checksum", checksum),
                ("replace", replacement),
            )
            if observed != expected[field]
        )
        if changed:
            raise DebtCheckError(
                "Cargo.lock dependency path package fingerprint mismatch: "
                f"{identity} ({_bounded(changed)})"
            )


def load_locked_dependency_path(
    expected_path: tuple[dict[str, Any], ...],
    path: Path = LOCKFILE_PATH,
) -> None:
    try:
        lockfile_text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError):
        raise DebtCheckError("Cargo.lock is unavailable") from None
    _verify_lockfile_path(lockfile_text, expected_path)


def _verify_metadata_path(
    metadata_text: str,
    expected_path: tuple[dict[str, Any], ...],
) -> None:
    metadata = _mapping(
        _parse_json(metadata_text, label="cargo metadata output"),
        label="cargo metadata output",
    )
    packages = _list(
        _required(metadata, "packages", label="cargo metadata output"),
        label="cargo metadata packages",
    )
    if len(packages) > MAX_METADATA_PACKAGES:
        raise DebtCheckError("cargo metadata package inventory exceeds the limit")

    selected: list[tuple[str, dict[str, Any]]] = []
    for expected in expected_path:
        matches: list[dict[str, Any]] = []
        for value in packages:
            package = _mapping(value, label="cargo metadata package")
            if (
                package.get("name") == expected["name"]
                and package.get("version") == expected["version"]
            ):
                matches.append(package)
        identity = f"{expected['name']} {expected['version']}"
        if len(matches) != 1:
            raise DebtCheckError(
                f"cargo metadata dependency path package identity mismatch: {identity}"
            )
        package = matches[0]
        package_id = _text(
            _required(package, "id", label="cargo metadata package"),
            label="cargo metadata package id",
            maximum=2048,
        )
        source = _optional_text(
            _required(package, "source", label="cargo metadata package"),
            label="cargo metadata package source",
            maximum=1024,
        )
        if source != expected["source"]:
            raise DebtCheckError(
                f"cargo metadata dependency path package source mismatch: {identity}"
            )
        selected.append((package_id, expected))
    if len({package_id for package_id, _ in selected}) != len(selected):
        raise DebtCheckError("cargo metadata dependency path package ids are not unique")

    resolve = _mapping(
        _required(metadata, "resolve", label="cargo metadata output"),
        label="cargo metadata resolve",
    )
    root_id = _text(
        _required(resolve, "root", label="cargo metadata resolve"),
        label="cargo metadata resolve root",
        maximum=2048,
    )
    nodes = _list(
        _required(resolve, "nodes", label="cargo metadata resolve"),
        label="cargo metadata resolve nodes",
    )
    if len(nodes) > MAX_METADATA_PACKAGES:
        raise DebtCheckError("cargo metadata resolve node inventory exceeds the limit")
    node_map: dict[str, dict[str, Any]] = {}
    for value in nodes:
        node = _mapping(value, label="cargo metadata resolve node")
        node_id = _text(
            _required(node, "id", label="cargo metadata resolve node"),
            label="cargo metadata resolve node id",
            maximum=2048,
        )
        if node_id in node_map:
            raise DebtCheckError("cargo metadata contains a duplicate resolve node")
        node_map[node_id] = node
    if root_id not in node_map:
        raise DebtCheckError("cargo metadata resolve root is missing")

    for index, (from_id, expected) in enumerate(selected):
        node = node_map.get(from_id)
        if node is None:
            raise DebtCheckError(
                "cargo metadata dependency path resolve node is missing: "
                f"{expected['name']} {expected['version']}"
            )
        dependencies = _list(
            _required(node, "deps", label="cargo metadata resolve node"),
            label="cargo metadata resolve dependencies",
        )
        if len(dependencies) > MAX_METADATA_DEPENDENCIES:
            raise DebtCheckError("cargo metadata dependency inventory exceeds the limit")
        edge = expected["dependency_to_next"]
        if edge is None:
            continue
        to_id = selected[index + 1][0]
        matches: list[dict[str, Any]] = []
        for value in dependencies:
            dependency = _mapping(value, label="cargo metadata dependency")
            if dependency.get("pkg") == to_id:
                matches.append(dependency)
        if len(matches) != 1:
            raise DebtCheckError(
                "cargo metadata dependency path edge mismatch: "
                f"{expected['name']} -> {selected[index + 1][1]['name']}"
            )
        dependency = matches[0]
        observed_name = _text(
            _required(dependency, "name", label="cargo metadata dependency"),
            label="cargo metadata dependency name",
        )
        observed_kinds = _dependency_kinds(
            _required(dependency, "dep_kinds", label="cargo metadata dependency"),
            label="cargo metadata dependency kinds",
        )
        if observed_name != edge["name"] or observed_kinds != edge["kinds"]:
            raise DebtCheckError(
                "cargo metadata dependency path edge fingerprint mismatch: "
                f"{expected['name']} -> {selected[index + 1][1]['name']}"
            )

    graph: dict[str, set[str]] = {}
    for node_id, node in node_map.items():
        dependencies = _list(
            _required(node, "deps", label="cargo metadata resolve node"),
            label="cargo metadata resolve dependencies",
        )
        if len(dependencies) > MAX_METADATA_DEPENDENCIES:
            raise DebtCheckError("cargo metadata dependency inventory exceeds the limit")
        successors: set[str] = set()
        for value in dependencies:
            dependency = _mapping(value, label="cargo metadata dependency")
            package_id = _text(
                _required(dependency, "pkg", label="cargo metadata dependency"),
                label="cargo metadata dependency package id",
                maximum=2048,
            )
            _dependency_kinds(
                _required(dependency, "dep_kinds", label="cargo metadata dependency"),
                label="cargo metadata dependency kinds",
            )
            if package_id not in node_map:
                raise DebtCheckError(
                    "cargo metadata dependency references a missing resolve node"
                )
            successors.add(package_id)
        graph[node_id] = successors

    vulnerable_id = selected[-1][0]
    if not _is_reachable(graph, root_id, vulnerable_id):
        raise DebtCheckError(
            "cargo metadata vulnerable package is unreachable from the workspace root"
        )
    for required_id, required in selected[:-1]:
        if _is_reachable(graph, root_id, vulnerable_id, blocked=required_id):
            raise DebtCheckError(
                "cargo metadata dependency path has an alternate route bypassing "
                f"{required['name']} {required['version']}"
            )


def _is_reachable(
    graph: dict[str, set[str]],
    start: str,
    target: str,
    *,
    blocked: str | None = None,
) -> bool:
    if start == blocked:
        return False
    pending = [start]
    visited: set[str] = set()
    while pending:
        node = pending.pop()
        if node == blocked or node in visited:
            continue
        if node == target:
            return True
        visited.add(node)
        pending.extend(graph.get(node, ()))
    return False


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
        raise DebtCheckError("Cargo tooling is unavailable") from None


def main() -> int:
    try:
        expected_version, expected, _dependency_scope, expected_path = load_policy()
        load_locked_dependency_path(expected_path)

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

        metadata_result = _run(
            ["cargo", "metadata", "--locked", "--format-version", "1"]
        )
        if metadata_result.returncode != 0:
            raise DebtCheckError(
                f"cargo metadata command failed with exit {metadata_result.returncode}"
            )
        _verify_metadata_path(metadata_result.stdout, expected_path)
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
