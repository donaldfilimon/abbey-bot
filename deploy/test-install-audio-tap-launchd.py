#!/usr/bin/env python3
"""Installer transactions in a fake home with no real launchd, HTTP or capture."""

from __future__ import annotations

import json
import os
from pathlib import Path
import plistlib
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
LABEL = "com.donaldfilimon.abbey-audio-tap"
FAKE_COMMAND = r'''#!__PYTHON__
import json
import os
from pathlib import Path
import plistlib
import signal
import sys

root = Path(os.environ["FAKE_ROOT"])
state_path = root / "fake-state.json"
state = json.loads(state_path.read_text())
name = Path(sys.argv[0]).name
args = sys.argv[1:]
with (root / "calls.jsonl").open("a") as log:
    log.write(json.dumps([name, *args]) + "\n")

def save():
    state_path.write_text(json.dumps(state))

def value(flag):
    return args[args.index(flag) + 1]

if name == "uname":
    print(state.get("platform", "Darwin"))
elif name == "sw_vers":
    print(state.get("os_version", "14.0"))
elif name == "swift":
    assert "TOOLCHAINS" not in os.environ, "Swift must use the default toolchain"
    bin_dir = Path(value("--scratch-path")) / "release"
    if "--show-bin-path" in args:
        print(bin_dir)
    else:
        if state.get("build_fails"):
            sys.exit(1)
        bin_dir.mkdir(parents=True)
        product = bin_dir / "abbey-audio-tap"
        product.write_text("#!/bin/sh\n[ \"$1\" = --version ] || exit 99\nprintf 'abbey-audio-tap 0.1.0\\n'\n")
        product.chmod(0o700)
elif name == "plutil":
    with open(args[-1], "rb") as source:
        plistlib.load(source)
elif name == "launchctl":
    action = args[0]
    if action == "print":
        if args[1].count("/") == 1:
            sys.exit(1 if state.get("domain_unavailable") else 0)
        if state.get("service_print_denied"):
            print("Operation not permitted", file=sys.stderr)
            sys.exit(1)
        if state.get("service_print_unknown"):
            print("Unknown service inspection failure", file=sys.stderr)
            sys.exit(113)
        if not state.get("loaded"):
            print('Could not find service "com.donaldfilimon.abbey-audio-tap" in domain for user gui', file=sys.stderr)
            sys.exit(113)
        print("service = {\n\tpid = " + str(state["pid"]) + "\n}")
    elif action == "bootout":
        state["bootouts"] = state.get("bootouts", 0) + 1
        if state.get("bootout_fails") or (
            state.get("candidate_bootout_fails") and state.get("bootstraps", 0) == 1
        ):
            save()
            sys.exit(1)
        state["loaded"] = False
        save()
    elif action == "bootstrap":
        state["bootstraps"] = state.get("bootstraps", 0) + 1
        if state.get("bootstrap_fails") and state["bootstraps"] == 1:
            save()
            sys.exit(1)
        with open(args[2], "rb") as source:
            installed = plistlib.load(source)
        assert installed["Label"] == "com.donaldfilimon.abbey-audio-tap"
        assert installed["ProgramArguments"][1:] == ["serve"]
        state["loaded"] = True
        state["pid"] = 8000 + state["bootstraps"]
        save()
        if state.get("interrupt_bootstrap") and state["bootstraps"] == 1:
            os.kill(os.getppid(), signal.SIGTERM)
    else:
        raise AssertionError("Unexpected launchctl mutation: " + repr(args))
elif name == "lsof":
    pid = value("-p")
    assert args == ["-nP", "-a", "-p", pid, "-iTCP", "-sTCP:LISTEN", "-Fpn"]
    if state.get("wrong_pid") and state.get("bootstraps") == 1:
        pid = "99999"
    address = "127.0.0.1:8182"
    if state.get("wildcard_listener") and state.get("bootstraps") == 1:
        address = "*:8182"
    print("p" + pid + "\nn" + address)
elif name == "curl":
    assert args[0] == "--disable", "curl must ignore per-user configuration before all other options"
    assert args[-1] == "http://127.0.0.1:8182/health"
    assert args[args.index("--noproxy") + 1] == "*"
    health = {
        "service": "abbey-audio-tap", "protocol_version": 1,
        "status": "idle", "ready": False,
        "audio": {"sample_rate": 48000, "channels": 2, "format": "s16le"},
        "stream_path": "/stream",
    }
    if state.get("bad_health") and state.get("bootstraps") == 1:
        health["service"] = "some-other-service"
    if state.get("capture_ready") and state.get("bootstraps") == 1:
        health["status"] = "capturing"
        health["ready"] = True
    if state.get("pid_changes") and state.get("bootstraps") == 1:
        state["pid"] += 1
        save()
    print(json.dumps(health))
else:
    raise AssertionError("Unexpected command: " + name)
'''


@unittest.skipUnless(os.name == "posix", "installer requires POSIX permissions and shell")
class AudioTapInstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="abbey-audio-tap-install-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        # Exercise XML and shell metacharacters as literal path characters.
        self.home = self.root / "owner's home & $(touch DO_NOT_CREATE)"
        self.home.mkdir(mode=0o700)
        self.repo = self.root / "repo"
        (self.repo / "deploy").mkdir(parents=True)
        (self.repo / "tools/abbey-audio-tap").mkdir(parents=True)
        for name in ("install-audio-tap-launchd.sh", f"{LABEL}.plist"):
            shutil.copyfile(ROOT / "deploy" / name, self.repo / "deploy" / name)
        self.fake_bin = self.root / "fake-bin"
        self.fake_bin.mkdir()
        for name in ("uname", "sw_vers", "swift", "plutil", "launchctl", "lsof", "curl"):
            script = self.fake_bin / name
            script.write_text(FAKE_COMMAND.replace("__PYTHON__", sys.executable))
            script.chmod(0o700)
        (self.fake_bin / "python3").symlink_to(sys.executable)
        self.state_path = self.root / "fake-state.json"
        self.set_state(loaded=False)
        self.state_dir = self.home / ".local/share/abbey-bot/audio-tap"
        self.binary = self.home / ".local/libexec/abbey-bot/audio-tap/abbey-audio-tap"
        self.plist = self.home / "Library/LaunchAgents" / f"{LABEL}.plist"
        self.log = self.home / "Library/Logs/abbey-bot/audio-tap/service.log"
        self.lock = self.state_dir / "install.lock"

    def set_state(self, **values):
        state = json.loads(self.state_path.read_text()) if self.state_path.exists() else {}
        state.update(values)
        self.state_path.write_text(json.dumps(state))

    def state(self):
        return json.loads(self.state_path.read_text())

    def calls(self):
        return [json.loads(line) for line in (self.root / "calls.jsonl").read_text().splitlines()]

    def run_installer(self, *args, succeeds=True):
        env = dict(os.environ)
        env.update(HOME=str(self.home), FAKE_ROOT=str(self.root),
                   PATH=f"{self.fake_bin}:/usr/bin:/bin", TOOLCHAINS="do-not-use")
        result = subprocess.run(
            ["/bin/sh", str(self.repo / "deploy/install-audio-tap-launchd.sh"), *args],
            cwd=self.repo, env=env, capture_output=True, text=True, timeout=45,
        )
        if succeeds:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse((self.repo / "DO_NOT_CREATE").exists())
        for call in self.calls():
            if call[0] == "launchctl" and call[1] in ("bootstrap", "bootout"):
                self.assertIn(LABEL, call[-1])
            if call[0] == "curl":
                self.assertEqual(call[-1], "http://127.0.0.1:8182/health")
        return result

    def previous_install(self, *, loaded=True):
        self.binary.parent.mkdir(parents=True, mode=0o700)
        self.binary.write_text("old binary bytes")
        self.binary.chmod(0o700)
        self.plist.parent.mkdir(parents=True, mode=0o700)
        self.old_plist = plistlib.dumps({
            "Label": LABEL, "ProgramArguments": [str(self.binary), "serve"]
        })
        self.plist.write_bytes(self.old_plist)
        self.plist.chmod(0o600)
        self.set_state(loaded=loaded, pid=7000)

    def assert_previous_restored(self, *, loaded=True):
        self.assertEqual(self.binary.read_text(), "old binary bytes")
        self.assertEqual(self.plist.read_bytes(), self.old_plist)
        self.assertEqual(self.state()["loaded"], loaded)
        self.assertFalse(self.lock.exists())

    def test_install_starts_only_idle_fixed_loopback_service(self):
        unrelated = self.home / ".config/abbey-bot/env"
        unrelated.parent.mkdir(parents=True)
        unrelated.write_text("DISCORD_TOKEN=secret-must-not-leak\n$(touch ENV_WAS_SOURCED)\n")
        result = self.run_installer()
        config = plistlib.loads(self.plist.read_bytes())
        self.assertEqual(config["ProgramArguments"], [str(self.binary), "serve"])
        self.assertEqual(config["WorkingDirectory"], str(self.state_dir))
        self.assertEqual(config["StandardErrorPath"], str(self.log))
        self.assertEqual(config["Umask"], 0o077)
        self.assertEqual(config["KeepAlive"], {"SuccessfulExit": False})
        for target, mode in ((self.binary, 0o700), (self.plist, 0o600), (self.log, 0o600),
                             (self.binary.parent, 0o700), (self.state_dir, 0o700),
                             (self.log.parent, 0o700)):
            self.assertEqual(stat.S_IMODE(target.stat().st_mode), mode, str(target))
        self.assertFalse(self.lock.exists())
        self.assertTrue(self.state()["loaded"])
        self.assertIn("No audio capture", result.stdout)
        self.assertNotIn("secret-must-not-leak", result.stdout + result.stderr)
        self.assertFalse((self.repo / "ENV_WAS_SOURCED").exists())
        self.assertEqual(unrelated.read_text(), "DISCORD_TOKEN=secret-must-not-leak\n$(touch ENV_WAS_SOURCED)\n")

    def test_build_failure_leaves_previous_loaded_service_untouched(self):
        self.previous_install()
        self.set_state(build_fails=True)
        self.run_installer(succeeds=False)
        self.assert_previous_restored()
        self.assertFalse(any(call[1] == "bootout" for call in self.calls()))

    def test_health_probe_disables_default_curl_configuration_before_options(self):
        unrelated = self.home / "preserve-me.txt"
        unrelated.write_text("preserve existing contents")
        config = self.home / ".curlrc"
        config.write_text('url = "http://127.0.0.1:8182/stream"\n'
                          f'output = "{unrelated}"\n')
        original = config.read_text()
        self.run_installer()
        probes = [call for call in self.calls() if call[0] == "curl"]
        self.assertTrue(probes)
        for probe in probes:
            self.assertEqual(probe[1], "--disable")
            self.assertEqual([arg for arg in probe if arg.startswith("http")],
                             ["http://127.0.0.1:8182/health"])
        self.assertEqual(unrelated.read_text(), "preserve existing contents")
        self.assertEqual(config.read_text(), original)

    def test_successful_upgrade_retains_previous_artifacts(self):
        self.previous_install()
        self.run_installer()
        self.assertNotEqual(self.binary.read_text(), "old binary bytes")
        backups = list(self.state_dir.glob("install-*/previous-abbey-audio-tap"))
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0].read_text(), "old binary bytes")
        self.assertFalse((backups[0].parent / "build").exists())
        self.assertEqual(self.state()["bootouts"], 1)

    def test_bootstrap_failure_restores_previous_files_and_service(self):
        self.previous_install()
        self.set_state(bootstrap_fails=True)
        self.run_installer(succeeds=False)
        self.assert_previous_restored()

    def test_health_identity_failure_restores_previous_service(self):
        self.previous_install()
        self.set_state(bad_health=True)
        self.run_installer(succeeds=False)
        self.assert_previous_restored()

    def test_capture_ready_is_not_accepted_as_idle_install(self):
        self.previous_install()
        self.set_state(capture_ready=True)
        self.run_installer(succeeds=False)
        self.assert_previous_restored()

    def test_changed_launchd_pid_fails_health_identity(self):
        self.previous_install()
        self.set_state(pid_changes=True)
        self.run_installer(succeeds=False)
        self.assert_previous_restored()

    def test_wildcard_listener_is_rejected(self):
        self.previous_install()
        self.set_state(wildcard_listener=True)
        self.run_installer(succeeds=False)
        self.assert_previous_restored()
        self.assertEqual(self.state()["bootstraps"], 2)

    def test_listener_owned_by_another_pid_is_rejected(self):
        self.previous_install()
        self.set_state(wrong_pid=True)
        self.run_installer(succeeds=False)
        self.assert_previous_restored()

    def test_signal_during_bootstrap_rolls_back(self):
        self.previous_install()
        self.set_state(interrupt_bootstrap=True)
        self.run_installer(succeeds=False)
        self.assert_previous_restored()

    def test_failed_first_install_leaves_no_job_or_launch_files(self):
        self.set_state(bad_health=True)
        self.run_installer(succeeds=False)
        self.assertFalse(self.state()["loaded"])
        self.assertFalse(self.binary.exists())
        self.assertFalse(self.plist.exists())
        self.assertFalse(self.lock.exists())

    def test_unloaded_previous_install_stays_unloaded_on_failure(self):
        self.previous_install(loaded=False)
        self.set_state(bad_health=True)
        self.run_installer(succeeds=False)
        self.assert_previous_restored(loaded=False)
        self.assertEqual(self.state()["bootstraps"], 1)

    def test_refuses_rollback_when_candidate_cannot_unload(self):
        self.previous_install()
        self.set_state(bad_health=True, candidate_bootout_fails=True)
        result = self.run_installer(succeeds=False)
        self.assertNotEqual(self.binary.read_text(), "old binary bytes")
        self.assertTrue(self.lock.is_dir())
        self.assertTrue(self.state()["loaded"])
        self.assertEqual(self.state()["bootstraps"], 1)
        self.assertIn("Recovery files retained", result.stderr)
        backup = next(self.state_dir.glob("install-*/previous-abbey-audio-tap"))
        self.assertEqual(backup.read_text(), "old binary bytes")

    def test_uninstall_preserves_logs_and_unrelated_files(self):
        self.previous_install()
        self.log.parent.mkdir(parents=True, mode=0o700)
        self.log.write_text("content-free service diagnostic")
        self.log.chmod(0o600)
        unrelated = self.binary.parent.parent / "abbey-bot"
        unrelated.write_text("existing Discord binary")
        self.run_installer("--uninstall")
        self.assertFalse(self.binary.exists())
        self.assertFalse(self.plist.exists())
        self.assertFalse(self.state()["loaded"])
        self.assertEqual(self.log.read_text(), "content-free service diagnostic")
        self.assertEqual(unrelated.read_text(), "existing Discord binary")
        self.assertFalse(any(call[0] == "swift" for call in self.calls()))

    def test_uninstall_refuses_file_changes_when_job_cannot_unload(self):
        self.previous_install()
        self.set_state(bootout_fails=True)
        self.run_installer("--uninstall", succeeds=False)
        self.assertEqual(self.binary.read_text(), "old binary bytes")
        self.assertEqual(self.plist.read_bytes(), self.old_plist)
        self.assertTrue(self.state()["loaded"])

    def test_existing_lock_blocks_service_mutations(self):
        self.lock.mkdir(parents=True)
        (self.lock / "pid").write_text("12345\n")
        self.run_installer(succeeds=False)
        self.assertEqual((self.lock / "pid").read_text(), "12345\n")
        self.assertFalse(any(call[1] in ("bootstrap", "bootout") for call in self.calls()))

    def test_symlink_ancestor_rejected_without_mutating_target(self):
        outside = self.root / "unrelated"
        outside.mkdir()
        (self.home / ".local").symlink_to(outside, target_is_directory=True)
        self.run_installer(succeeds=False)
        self.assertEqual(list(outside.iterdir()), [])

    def test_symlink_log_rejected_without_changing_target(self):
        self.log.parent.mkdir(parents=True, mode=0o700)
        outside = self.root / "unrelated-log"
        outside.write_text("preserve")
        self.log.symlink_to(outside)
        self.run_installer(succeeds=False)
        self.assertEqual(outside.read_text(), "preserve")
        self.assertTrue(self.log.is_symlink())
        self.assertFalse(self.lock.exists())

    def test_multiply_linked_binary_is_rejected(self):
        self.previous_install()
        os.link(self.binary, self.root / "other-binary-link")
        self.run_installer(succeeds=False)
        self.assertEqual(self.binary.read_text(), "old binary bytes")
        self.assertFalse(any(call[1] == "bootout" for call in self.calls()))

    def test_denied_service_print_never_means_unloaded(self):
        self.previous_install()
        self.set_state(service_print_denied=True)
        self.run_installer(succeeds=False)
        self.assert_previous_restored()
        self.assertFalse(any(call[1] in ("bootstrap", "bootout") for call in self.calls()))

    def test_unknown_service_print_error_never_means_unloaded(self):
        self.previous_install()
        self.set_state(service_print_unknown=True)
        self.run_installer(succeeds=False)
        self.assert_previous_restored()
        self.assertFalse(any(call[1] in ("bootstrap", "bootout") for call in self.calls()))

    def test_public_install_directory_is_rejected_without_chmod(self):
        self.binary.parent.mkdir(parents=True, mode=0o755)
        self.run_installer(succeeds=False)
        self.assertEqual(stat.S_IMODE(self.binary.parent.stat().st_mode), 0o755)

    def test_non_macos_refused_before_launchd_or_home_changes(self):
        self.set_state(platform="Linux")
        self.run_installer(succeeds=False)
        self.assertEqual(list(self.home.iterdir()), [])
        self.assertEqual(self.calls(), [["uname", "-s"]])

    def test_old_macos_refused_before_launchd_or_home_changes(self):
        self.set_state(os_version="13.6")
        self.run_installer(succeeds=False)
        self.assertEqual(list(self.home.iterdir()), [])
        self.assertFalse(any(call[0] == "launchctl" for call in self.calls()))


if __name__ == "__main__":
    unittest.main()
