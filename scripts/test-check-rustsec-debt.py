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
    "RUSTSEC-2026-0049": ["CVE-2026-93602", "GHSA-pwjx-qhcg-rvj4"],
    "RUSTSEC-2026-0098": ["CVE-2026-93600", "GHSA-965h-392x-2mh5"],
    "RUSTSEC-2026-0099": ["CVE-2026-93601", "GHSA-xgp8-3hg3-c2mh"],
    "RUSTSEC-2026-0104": ["CVE-2026-93599", "GHSA-82j2-j2ch-gfr8"],
}
RINGBUF_ID = "RUSTSEC-2026-0293"
RINGBUF_CHECKSUM = "fe47b720588c8702e34b5979cb3271a8b1842c7cb6f57408efa70c779363488c"
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
REGISTRY_SOURCE = "registry+https://github.com/rust-lang/crates.io-index"
EXPECTED_AUDIT_DEPENDENCIES = [
    {"name": "ring", "version": "0.17.14", "source": REGISTRY_SOURCE},
    {
        "name": "rustls-pki-types",
        "version": "1.15.1",
        "source": REGISTRY_SOURCE,
    },
    {"name": "untrusted", "version": "0.9.0", "source": REGISTRY_SOURCE},
]
EXPECTED_DEPENDENCY_PATH = (
    (
        "serenity",
        "0.12.5",
        "9bde37f42765dfdc34e2a039e0c84afbf79a3101c1941763b0beb816c2f17541",
        "tokio_tungstenite",
    ),
    (
        "tokio-tungstenite",
        "0.21.0",
        "c83b561d025642014097b66e6c1bb422783339e0909e4429cde4749d1990bc38",
        "rustls",
    ),
    (
        "rustls",
        "0.22.4",
        "bf4ef73721ac7bcd79b2b315da7779d8fc09718c6b3d2d1b2d94850eb8c18432",
        "webpki",
    ),
    ("rustls-webpki", "0.102.8", PACKAGE_CHECKSUM, None),
)
EXPECTED_RINGBUF_PATH = (
    (
        "songbird",
        "0.6.0",
        "5b9bf4ecb5083685b79a8fc60222f12e4f036e2817c5c341cd2b9ff2b429d237",
        "ringbuf",
    ),
    ("ringbuf", "0.4.8", RINGBUF_CHECKSUM, None),
)
EXPECTED_PATHS = (EXPECTED_DEPENDENCY_PATH, EXPECTED_RINGBUF_PATH)


def accepted_policy() -> dict[str, object]:
    return json.loads(POLICY.read_text(encoding="utf-8"))


def exact_report() -> dict[str, object]:
    policy = accepted_policy()
    records = []
    for accepted in policy["accepted_vulnerabilities"]:
        record = copy.deepcopy(accepted)
        advisory_id = record.pop("id")
        record["advisory"] = {"id": advisory_id, **record["advisory"]}
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


def all_path_nodes(policy: dict[str, object]) -> list[dict[str, object]]:
    return [
        node
        for dependency_path in policy["locked_dependency_paths"]
        for node in dependency_path["nodes"]
    ]


def exact_metadata() -> dict[str, object]:
    policy = accepted_policy()
    path = all_path_nodes(policy)
    package_ids = [
        f"{node['source']}#{node['name']}@{node['version']}" for node in path
    ]
    first_ids = {
        dependency_path["nodes"][0]["name"]: (
            f"{dependency_path['nodes'][0]['source']}#"
            f"{dependency_path['nodes'][0]['name']}@"
            f"{dependency_path['nodes'][0]['version']}"
        )
        for dependency_path in policy["locked_dependency_paths"]
    }
    root_id = "path+file:///fixture/abbey-bot#0.1.0"
    packages = [
        {
            "id": root_id,
            "name": "abbey-bot",
            "version": "0.1.0",
            "source": None,
        },
        *[
            {
                "id": package_id,
                "name": node["name"],
                "version": node["version"],
                "source": node["source"],
            }
            for package_id, node in zip(package_ids, path, strict=True)
        ],
    ]
    nodes = [
        {
            "id": root_id,
            "deps": [
                {
                    "name": name,
                    "pkg": package_id,
                    "dep_kinds": [{"kind": None, "target": None}],
                }
                for name, package_id in first_ids.items()
            ],
        }
    ]
    for index, (package_id, path_node) in enumerate(
        zip(package_ids, path, strict=True)
    ):
        dependencies = []
        edge = path_node["dependency_to_next"]
        if edge is not None:
            dependencies.append(
                {
                    "name": edge["name"],
                    "pkg": package_ids[index + 1],
                    "dep_kinds": copy.deepcopy(edge["kinds"]),
                }
            )
        nodes.append({"id": package_id, "deps": dependencies})
    return {"packages": packages, "resolve": {"root": root_id, "nodes": nodes}}


def lockfile_for_path(path: list[dict[str, object]]) -> str:
    lines = ["version = 4", ""]
    for node in path:
        lines.extend(
            [
                "[[package]]",
                f"name = {json.dumps(node['name'])}",
                f"version = {json.dumps(node['version'])}",
                f"source = {json.dumps(node['source'])}",
                f"checksum = {json.dumps(node['checksum'])}",
            ]
        )
        if node["replace"] is not None:
            lines.append(f"replace = {json.dumps(node['replace'])}")
        lines.append("")
    return "\n".join(lines)


def exact_lockfile() -> str:
    policy = accepted_policy()
    return lockfile_for_path(all_path_nodes(policy))


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
        metadata: dict[str, object] | None = None,
        metadata_text: str | None = None,
        metadata_returncode: int = 0,
        metadata_stderr: str = "",
        metadata_decode_error: UnicodeDecodeError | None = None,
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
        if metadata_text is None:
            metadata_text = json.dumps(
                metadata if metadata is not None else exact_metadata()
            )
        metadata_result = subprocess.CompletedProcess(
            args=["cargo", "metadata"],
            returncode=metadata_returncode,
            stdout=metadata_text,
            stderr=metadata_stderr,
        )
        metadata_outcome: subprocess.CompletedProcess[str] | UnicodeDecodeError = (
            metadata_decode_error
            if metadata_decode_error is not None
            else metadata_result
        )
        stdout = StringIO()
        stderr = StringIO()
        with mock.patch.object(
            CHECKER.subprocess,
            "run",
            side_effect=[version_result, report_result, metadata_outcome],
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

    def test_policy_pins_the_exact_five_vulnerabilities(self) -> None:
        policy = accepted_policy()

        self.assertEqual(policy["schema_version"], 3)
        self.assertEqual(policy["cargo_audit_version"], "0.22.2")
        self.assertEqual(policy["audit_state"], "not-clean")
        self.assertEqual(policy["accepted_vulnerability_count"], 5)
        records = {
            record["id"]: record for record in policy["accepted_vulnerabilities"]
        }
        self.assertEqual(set(records), {*EXPECTED_IDS, RINGBUF_ID})
        for advisory_id, aliases in EXPECTED_IDS.items():
            with self.subTest(advisory_id=advisory_id):
                record = records[advisory_id]
                self.assertEqual(record["advisory"]["aliases"], aliases)
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
                self.assertEqual(record["package"]["source"], REGISTRY_SOURCE)
                self.assertEqual(record["package"]["checksum"], PACKAGE_CHECKSUM)
                self.assertEqual(
                    record["package"]["dependencies"],
                    EXPECTED_AUDIT_DEPENDENCIES,
                )
                self.assertEqual(record["package"]["replace"], None)

        ringbuf = records[RINGBUF_ID]
        self.assertEqual(ringbuf["advisory"]["package"], "ringbuf")
        self.assertEqual(ringbuf["advisory"]["aliases"], [])
        self.assertEqual(ringbuf["advisory"]["categories"], ["memory-corruption"])
        self.assertEqual(ringbuf["versions"], {"patched": [">=0.5.2"], "unaffected": []})
        self.assertEqual(
            ringbuf["affected"]["functions"],
            {
                "ringbuf::traits::consumer::Consumer::clear": ["<0.5.2"],
                "ringbuf::traits::consumer::Consumer::skip": ["<0.5.2"],
            },
        )
        self.assertEqual(ringbuf["package"]["version"], "0.4.8")
        self.assertEqual(ringbuf["package"]["checksum"], RINGBUF_CHECKSUM)

        dependency_paths = policy["locked_dependency_paths"]
        self.assertEqual(
            [set(item["advisory_ids"]) for item in dependency_paths],
            [set(EXPECTED_IDS), {RINGBUF_ID}],
        )
        for dependency_path, expected_path in zip(
            dependency_paths, EXPECTED_PATHS, strict=True
        ):
            self.assertEqual(dependency_path["scope"], "all-resolved-targets")
            self.assertTrue(dependency_path["rationale"])
            self.assertTrue(dependency_path["review_triggers"])
            nodes = dependency_path["nodes"]
            self.assertEqual(len(nodes), len(expected_path))
            for node, (name, version, checksum, edge_name) in zip(
                nodes,
                expected_path,
                strict=True,
            ):
                self.assertEqual(node["name"], name)
                self.assertEqual(node["version"], version)
                self.assertEqual(node["source"], REGISTRY_SOURCE)
                self.assertEqual(node["checksum"], checksum)
                self.assertEqual(node["replace"], None)
                if edge_name is None:
                    self.assertEqual(node["dependency_to_next"], None)
                else:
                    self.assertEqual(node["dependency_to_next"]["name"], edge_name)
                    self.assertEqual(
                        node["dependency_to_next"]["kinds"],
                        [{"kind": None, "target": None}],
                    )

    def test_policy_pins_the_unfiltered_metadata_scope(self) -> None:
        for index, approved in enumerate(CHECKER.APPROVED_DEPENDENCY_PATHS):
            with self.subTest(path=index):
                dependency_policy = copy.deepcopy(
                    accepted_policy()["locked_dependency_paths"][index]
                )
                dependency_policy["scope"] = "x86_64-unknown-linux-gnu-only"

                with self.assertRaisesRegex(
                    CHECKER.DebtCheckError,
                    "locked dependency scope mismatch",
                ):
                    CHECKER._load_dependency_path_policy(dependency_policy, approved)

    def load_policy_from(self, policy: dict[str, object]) -> None:
        with mock.patch.object(Path, "read_text", return_value=json.dumps(policy)):
            CHECKER.load_policy(Path("/fixture/policy.json"))

    def test_advisory_bound_to_the_wrong_path_is_rejected(self) -> None:
        policy = accepted_policy()
        first, second = policy["locked_dependency_paths"]
        first["advisory_ids"].append(RINGBUF_ID)
        second["advisory_ids"].remove(RINGBUF_ID)

        with self.assertRaisesRegex(
            CHECKER.DebtCheckError,
            "advisory binding mismatch",
        ):
            self.load_policy_from(policy)

    def test_advisory_listed_twice_in_a_path_is_rejected(self) -> None:
        policy = accepted_policy()
        policy["locked_dependency_paths"][1]["advisory_ids"].append(RINGBUF_ID)

        with self.assertRaisesRegex(
            CHECKER.DebtCheckError,
            "lists an advisory twice",
        ):
            self.load_policy_from(policy)

    def test_advisory_package_must_be_its_path_terminal(self) -> None:
        policy = accepted_policy()
        for record in policy["accepted_vulnerabilities"]:
            if record["id"] == RINGBUF_ID:
                record["package"]["checksum"] = PACKAGE_CHECKSUM

        with self.assertRaisesRegex(
            CHECKER.DebtCheckError,
            "not its path's terminal package: RUSTSEC-2026-0293",
        ):
            self.load_policy_from(policy)

    def test_missing_or_extra_dependency_path_is_rejected(self) -> None:
        for mutation in ("missing", "extra"):
            with self.subTest(mutation=mutation):
                policy = accepted_policy()
                paths = policy["locked_dependency_paths"]
                if mutation == "missing":
                    paths.pop()
                else:
                    paths.append(copy.deepcopy(paths[-1]))

                with self.assertRaisesRegex(
                    CHECKER.DebtCheckError,
                    "locked dependency path count mismatch",
                ):
                    self.load_policy_from(policy)

    def test_path_without_rationale_or_triggers_is_rejected(self) -> None:
        for field, fragment in (
            ("rationale", "rationale"),
            ("review_triggers", "review triggers are missing"),
        ):
            with self.subTest(field=field):
                policy = accepted_policy()
                policy["locked_dependency_paths"][1][field] = (
                    "" if field == "rationale" else []
                )

                with self.assertRaisesRegex(CHECKER.DebtCheckError, fragment):
                    self.load_policy_from(policy)

    def test_cargo_lock_v4_package_fingerprints_are_bound_for_every_path_node(
        self,
    ) -> None:
        _, _, _, expected_path = CHECKER.load_policy()
        CHECKER._verify_lockfile_path(exact_lockfile(), expected_path)
        mutations = (
            ("name", "changed-package"),
            ("version", "999.0.0"),
            ("source", "registry+https://example.invalid/index"),
            ("checksum", "0" * 64),
            (
                "replace",
                "replacement 1.0.0 (registry+https://example.invalid/index)",
            ),
        )
        original_nodes = all_path_nodes(accepted_policy())
        for index, original in enumerate(original_nodes):
            for field, value in mutations:
                with self.subTest(node=original["name"], field=field):
                    nodes = copy.deepcopy(original_nodes)
                    nodes[index][field] = value

                    with self.assertRaises(CHECKER.DebtCheckError):
                        CHECKER._verify_lockfile_path(
                            lockfile_for_path(nodes),
                            expected_path,
                        )

    def test_cargo_lock_format_version_is_explicitly_v4(self) -> None:
        _, _, _, expected_path = CHECKER.load_policy()

        with self.assertRaisesRegex(
            CHECKER.DebtCheckError,
            "Cargo.lock format version mismatch",
        ):
            CHECKER._verify_lockfile_path(
                exact_lockfile().replace("version = 4", "version = 3", 1),
                expected_path,
            )

    def test_metadata_binds_every_path_node_identity_and_source(self) -> None:
        _, _, _, expected_path = CHECKER.load_policy()
        flattened = [node for path in expected_path for node in path]
        for index, expected in enumerate(flattened):
            for field, value in (
                ("name", "changed-package"),
                ("version", "999.0.0"),
                ("source", "registry+https://example.invalid/index"),
            ):
                with self.subTest(node=expected["name"], field=field):
                    metadata = exact_metadata()
                    metadata["packages"][index + 1][field] = value

                    with self.assertRaises(CHECKER.DebtCheckError):
                        CHECKER._verify_metadata_path(
                            json.dumps(metadata),
                            expected_path,
                        )

    def edge_node_indexes(self) -> list[int]:
        nodes = exact_metadata()["resolve"]["nodes"]
        return [index for index in range(1, len(nodes)) if nodes[index]["deps"]]

    def test_every_metadata_path_edge_fingerprint_is_bound(self) -> None:
        cases = ("name", "kind", "target")
        self.assertEqual(len(self.edge_node_indexes()), 4)
        for edge_index in self.edge_node_indexes():
            for field in cases:
                with self.subTest(edge=edge_index, field=field):
                    metadata = exact_metadata()
                    dependency = metadata["resolve"]["nodes"][edge_index]["deps"][0]
                    if field == "name":
                        dependency["name"] = "changed_edge"
                    elif field == "kind":
                        dependency["dep_kinds"][0]["kind"] = "build"
                    else:
                        dependency["dep_kinds"][0]["target"] = "cfg(windows)"

                    result = self.run_check(metadata=metadata)

                    self.assert_failure(result, "edge fingerprint mismatch")

    def test_missing_metadata_path_edge_is_rejected(self) -> None:
        for edge_index in self.edge_node_indexes():
            with self.subTest(edge=edge_index):
                metadata = exact_metadata()
                metadata["resolve"]["nodes"][edge_index]["deps"] = []

                result = self.run_check(metadata=metadata)

                self.assert_failure(result, "dependency path edge mismatch")

    def test_added_bypass_route_for_any_kind_or_target_is_rejected(self) -> None:
        cases = (
            (None, None),
            ("build", None),
            ("dev", None),
            (None, "cfg(windows)"),
        )
        terminals = (
            ("rustls-webpki", "alternate route bypassing serenity 0.12.5"),
            ("ringbuf", "alternate route bypassing songbird 0.6.0"),
        )
        for terminal, fragment in terminals:
            for kind, target in cases:
                with self.subTest(terminal=terminal, kind=kind, target=target):
                    metadata = exact_metadata()
                    vulnerable_id = next(
                        package["id"]
                        for package in metadata["packages"]
                        if package["name"] == terminal
                    )
                    metadata["resolve"]["nodes"][0]["deps"].append(
                        {
                            "name": f"bypass_{terminal}",
                            "pkg": vulnerable_id,
                            "dep_kinds": [{"kind": kind, "target": target}],
                        }
                    )

                    result = self.run_check(metadata=metadata)

                    self.assert_failure(result, fragment)

    def test_exact_exit_one_report_passes_and_says_not_clean(self) -> None:
        status, stdout, stderr, run = self.run_check()

        self.assertEqual(status, 0)
        self.assertEqual(stderr, "")
        self.assertEqual(
            stdout,
            "rustsec-debt-check: accepted temporary debt matches: "
            "5 vulnerabilities remain; audit is NOT clean\n"
            "rustsec-debt-check: informational warnings "
            "(not accepted vulnerability debt): 3 (unmaintained=3)\n",
        )
        self.assertIn("audit is NOT clean", stdout)
        self.assertNotIn("audit clean", stdout.lower())
        commands = [call.args[0] for call in run.call_args_list]
        self.assertEqual(
            commands,
            [
                ["cargo", "audit", "--version"],
                ["cargo", "audit", "--json"],
                [
                    "cargo",
                    "metadata",
                    "--locked",
                    "--format-version",
                    "1",
                ],
            ],
        )
        self.assertFalse(any("--ignore" in argument for command in commands for argument in command))
        for call in run.call_args_list:
            self.assertTrue(call.kwargs["text"])
            self.assertEqual(call.kwargs["encoding"], "utf-8")
            self.assertEqual(call.kwargs["errors"], "strict")

    def test_exact_exit_zero_report_is_also_accepted(self) -> None:
        status, stdout, stderr, _ = self.run_check(report_returncode=0)

        self.assertEqual(status, 0)
        self.assertEqual(stderr, "")
        self.assertIn("5 vulnerabilities remain; audit is NOT clean", stdout)

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
        self.assertIn("5 vulnerabilities remain", stdout)

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

    def test_changed_missing_or_added_cargo_audit_dependency_is_rejected(self) -> None:
        for mutation in ("missing", "added", "changed"):
            with self.subTest(mutation=mutation):
                report = exact_report()
                dependencies = report["vulnerabilities"]["list"][0]["package"][
                    "dependencies"
                ]
                if mutation == "missing":
                    dependencies.pop()
                elif mutation == "added":
                    dependencies.append(
                        {
                            "name": "new-dependency",
                            "version": "1.0.0",
                            "source": REGISTRY_SOURCE,
                        }
                    )
                else:
                    dependencies[0]["version"] = "999.0.0"

                result = self.run_check(report)

                self.assert_failure(result, "vulnerability fingerprint mismatch")

    def test_cargo_audit_dependency_order_is_canonicalized(self) -> None:
        report = exact_report()
        report["vulnerabilities"]["list"][0]["package"][
            "dependencies"
        ].reverse()

        status, stdout, stderr, _ = self.run_check(report)

        self.assertEqual(status, 0)
        self.assertEqual(stderr, "")
        self.assertIn("5 vulnerabilities remain; audit is NOT clean", stdout)

    def test_duplicate_cargo_audit_dependency_is_rejected(self) -> None:
        report = exact_report()
        dependencies = report["vulnerabilities"]["list"][0]["package"][
            "dependencies"
        ]
        dependencies.append(copy.deepcopy(dependencies[0]))

        result = self.run_check(report)

        self.assert_failure(result, "package dependencies contains duplicate entries")

    def test_missing_cargo_audit_dependencies_field_is_rejected(self) -> None:
        report = exact_report()
        del report["vulnerabilities"]["list"][0]["package"]["dependencies"]

        result = self.run_check(report)

        self.assert_failure(result, "package schema mismatch")

    def test_non_null_or_missing_cargo_audit_replace_is_rejected(self) -> None:
        for mutation in ("non-null", "missing"):
            with self.subTest(mutation=mutation):
                report = exact_report()
                package = report["vulnerabilities"]["list"][0]["package"]
                if mutation == "non-null":
                    package["replace"] = {
                        "name": "rustls-webpki",
                        "version": "0.102.9",
                        "source": REGISTRY_SOURCE,
                    }
                else:
                    del package["replace"]

                result = self.run_check(report)

                expected = (
                    "vulnerability fingerprint mismatch"
                    if mutation == "non-null"
                    else "package schema mismatch"
                )
                self.assert_failure(result, expected)

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
        report["vulnerabilities"]["count"] = 6

        result = self.run_check(report)

        self.assert_failure(
            result,
            "vulnerability count mismatch (declared 6, observed 5)",
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

    def test_metadata_command_failure_fails_without_echo(self) -> None:
        sentinel = "DO-NOT-ECHO-CARGO-METADATA-STDERR"
        result = self.run_check(
            metadata_returncode=101,
            metadata_stderr=sentinel,
        )

        self.assert_failure(result, "cargo metadata command failed with exit 101")
        self.assertNotIn(sentinel, result[2])

    def test_metadata_decode_failures_are_redacted_and_fail_closed(self) -> None:
        decode_errors = (
            UnicodeDecodeError(
                "charmap",
                b"\x9dDO-NOT-ECHO-CARGO-METADATA",
                0,
                1,
                "character maps to <undefined>",
            ),
            UnicodeDecodeError(
                "utf-8",
                b"\xffDO-NOT-ECHO-CARGO-METADATA",
                0,
                1,
                "invalid start byte",
            ),
        )
        for decode_error in decode_errors:
            with self.subTest(encoding=decode_error.encoding):
                status, stdout, stderr, run = self.run_check(
                    metadata_decode_error=decode_error
                )

                self.assertEqual(status, 1)
                self.assertEqual(stdout, "")
                self.assertEqual(
                    stderr,
                    "rustsec-debt-check: FAIL: "
                    "Cargo tooling output is not valid UTF-8\n",
                )
                self.assertNotIn("DO-NOT-ECHO-CARGO-METADATA", stderr)
                self.assertNotIn("Traceback", stderr)
                self.assertEqual(run.call_count, 3)
                metadata_call = run.call_args_list[-1]
                self.assertEqual(metadata_call.kwargs["encoding"], "utf-8")
                self.assertEqual(metadata_call.kwargs["errors"], "strict")

    def test_malformed_metadata_fails_closed_without_echo(self) -> None:
        sentinel = "DO-NOT-ECHO-METADATA-CONTENT"
        result = self.run_check(metadata_text="{" + sentinel)

        self.assert_failure(result, "cargo metadata output is malformed JSON")
        self.assertNotIn(sentinel, result[2])


if __name__ == "__main__":
    unittest.main()
