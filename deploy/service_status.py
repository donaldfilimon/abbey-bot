"""Current managed-service observations, never installation acceptance."""
from dataclasses import dataclass
from enum import Enum
import os
from pathlib import Path
import sys
import time

from service_installation import binary_digest
from service_protocol import (MAX_PID, encode_document, fresh, identity,
                              parse_document, process_exists, read_optional_private)
from service_readiness import launchd_pid


class ObservationKind(Enum):
    READY = "ready"
    NOT_READY = "not_ready"
    UNAVAILABLE = "unavailable"
    BOOTSTRAP_FAILED = "bootstrap_failed"


@dataclass(frozen=True)
class ServiceObservation:
    kind: ObservationKind
    discord: str | None = None
    scheduler: str | None = None
    telegram: str | None = None
    slack: str | None = None
    persistence: str | None = None
    bootstrap_code: str | None = None


class LocalHost:
    """Fixed read-only operations; constructor performs no I/O."""
    def __init__(self, home):
        self.home = home

    def monotonic(self):
        return time.monotonic_ns()

    def wall_ms(self):
        return time.time_ns() // 1_000_000

    def service_pid(self, deadline):
        return launchd_pid(min(deadline, self.monotonic() + 2_000_000_000))

    def alive(self, pid):
        return process_exists(pid)

    def digest(self, deadline):
        return binary_digest(self.home, deadline)

    def document(self, kind):
        return read_optional_private(self.home, kind)


def observe(host):
    """Require two fresh matching identity samples, without a transaction floor."""
    unavailable = ServiceObservation(ObservationKind.UNAVAILABLE)
    try:
        deadline = host.monotonic() + 10_000_000_000
        pid = host.service_pid(deadline)
        if type(pid) is not int or not 1 <= pid <= MAX_PID or host.alive(pid) is not True:
            return unavailable
        digest = host.digest(deadline)
        samples = []
        for _ in range(2):
            document = host.document("readiness")
            if document is None:
                return unavailable
            document = parse_document(encode_document(document))
            if (document["pid"] != pid or document["executable_sha256"] != digest
                    or not fresh(document["published_at_unix_ms"], 0, host.wall_ms())):
                return unavailable
            samples.append(document)
            if host.service_pid(deadline) != pid or host.alive(pid) is not True:
                return unavailable
        first, current = samples
        if identity(first) != identity(current):
            return unavailable
        ready = all(sample["phase"] == "ready" and sample["discord"] == "ready"
                    and sample["scheduler"] == "running" for sample in samples)
        kind = ObservationKind.READY if ready else ObservationKind.NOT_READY
        code = None
        if not ready:
            bootstrap = host.document("bootstrap")
            if bootstrap is not None:
                bootstrap = parse_document(encode_document(bootstrap, "bootstrap"), "bootstrap")
                # A fresh readiness identity supplies the nonce comparison that
                # bootstrap's intentionally timestamp-free schema cannot supply.
                if (identity(bootstrap) == identity(current) and bootstrap["phase"] == "failed"
                        and bootstrap["code"] != "none"):
                    kind = ObservationKind.BOOTSTRAP_FAILED
                    code = bootstrap["code"]
            if host.service_pid(deadline) != pid or host.alive(pid) is not True:
                return unavailable
        if host.monotonic() >= deadline:
            return unavailable
        return ServiceObservation(kind, current["discord"], current["scheduler"],
                                  current["telegram"], current["slack"],
                                  current["last_persistence"], code)
    except Exception:
        # Errors may carry private filesystem or process text. None is rendered.
        return unavailable


def render(observation):
    title = {
        ObservationKind.READY: "Abbey service: ready (current observation)",
        ObservationKind.NOT_READY: "Abbey service: not ready (current observation)",
        ObservationKind.UNAVAILABLE: "Abbey service: current state unavailable or unverified.",
        ObservationKind.BOOTSTRAP_FAILED: "Abbey service: startup failed (current observation)",
    }.get(observation.kind, "Abbey service: current state unavailable or unverified.")
    if observation.kind not in (ObservationKind.READY, ObservationKind.NOT_READY,
                                ObservationKind.BOOTSTRAP_FAILED):
        return title
    labels = {
        "connecting": "connecting", "ready": "ready", "stopped": "stopped",
        "starting": "starting", "running": "running", "disabled": "disabled",
        "connected": "connected", "degraded": "degraded", "not_attempted": "not attempted",
        "memory_only": "memory only", "complete": "complete", "partial": "partial", "failed": "failed",
    }
    rows = [title]
    connector_states = ("disabled", "starting", "connected", "degraded", "stopped")
    for label, value, permitted in (
            ("Discord", observation.discord, ("connecting", "ready", "stopped")),
            ("Scheduler", observation.scheduler, ("starting", "running", "stopped")),
            ("Telegram", observation.telegram, connector_states),
            ("Slack", observation.slack, connector_states),
            ("Last completed persistence", observation.persistence,
             ("not_attempted", "memory_only", "complete", "partial", "failed"))):
        rows.append(f"{label}: {labels[value] if type(value) is str and value in permitted else 'unknown'}")
    if observation.kind == ObservationKind.BOOTSTRAP_FAILED:
        rows.append({
            "readiness_file": "Startup could not publish private readiness evidence.",
            "log_directory": "Startup could not validate its private log directory.",
            "log_file": "Startup could not validate its private log file.",
            "log_writer": "Startup could not initialize its operational log writer.",
        }.get(observation.bootstrap_code, "Startup failure details are unavailable."))
    return "\n".join(rows)


def main(argv, *, platform=sys.platform, host_factory=LocalHost, environ=os.environ):
    if argv == ["--help"]:
        print("Usage: service-status.py [--help]\nRead-only current service evidence; no installation acceptance or repair.")
        return 0
    if argv:
        print("Usage: service-status.py [--help]")
        return 2
    if platform != "darwin":
        print("Abbey service status is supported on macOS only.")
        return 2
    home = environ.get("HOME")
    if not home or not Path(home).is_absolute():
        observation = ServiceObservation(ObservationKind.UNAVAILABLE)
    else:
        try:
            observation = observe(host_factory(Path(home)))
        except Exception:
            observation = ServiceObservation(ObservationKind.UNAVAILABLE)
    print(render(observation))
    return 0 if observation.kind == ObservationKind.READY else 1
