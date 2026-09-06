"""Identity-bound readiness polling; all production effects have injectable seams."""
from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
import json
import os
from pathlib import Path
import re
import select
import subprocess
import time

from service_protocol import (MAX_PID, MAX_TIME, Failure, ProtocolError,
                              read_optional_private, process_exists, validate_ready)

BUDGET_NS = 30_000_000_000
STABILITY_NS = 5_000_000_000
POLL_NS = 250_000_000
CONTEXT_CAP = 512
STATUS_CAP = 65536
HEX = re.compile(r"[0-9a-f]{64}", re.ASCII)
DECIMAL = re.compile(r"0|[1-9][0-9]*", re.ASCII)
# A failed cleanup retains ownership until the terminal process boundary.
_OUTSTANDING_CHILDREN = []


class FailureCode(str, Enum):
    USAGE = "usage"
    CONTEXT = "context"
    TIMEOUT = "timeout"
    LAUNCHD = "launchd"
    DOCUMENT = "document"
    IDENTITY = "identity"
    NOT_READY = "not_ready"
    CLEANUP = "cleanup"
    INTERNAL = "internal"


class ReadinessError(Exception):
    def __init__(self, code):
        self.code = code
        super().__init__(code.value)


@dataclass(frozen=True, repr=False)
class TransactionContext:
    transaction_start_ms: int
    deadline_monotonic_ns: int
    excluded_nonces: tuple[str, ...]

    def __repr__(self):
        return "TransactionContext(<private>)"


def _integer(value, maximum=MAX_TIME):
    return type(value) is int and 0 <= value <= maximum


def _pairs(items):
    result = {}
    for key, value in items:
        if key in result:
            raise ReadinessError(FailureCode.CONTEXT)
        result[key] = value
    return result


def parse_context(raw, start_ms):
    try:
        if type(raw) is not bytes or len(raw) > CONTEXT_CAP or not raw.endswith(b"\n"):
            raise ValueError
        value = json.loads(raw, object_pairs_hook=_pairs)
        if type(value) is not dict or set(value) != {
                "schema_version", "transaction_start_ms", "deadline_monotonic_ns", "excluded_nonces"}:
            raise ValueError
        if type(value["schema_version"]) is not int or value["schema_version"] != 1:
            raise ValueError
        if not _integer(start_ms) or value["transaction_start_ms"] != start_ms:
            raise ValueError
        if not all(_integer(value[k]) for k in ("transaction_start_ms", "deadline_monotonic_ns")):
            raise ValueError
        nonces = value["excluded_nonces"]
        if (type(nonces) is not list or len(nonces) > 2
                or any(type(n) is not str or not HEX.fullmatch(n) for n in nonces)
                or len(set(nonces)) != len(nonces)):
            raise ValueError
        # Require the documented compact object plus exactly one LF.
        if json.dumps(value, separators=(",", ":")).encode() + b"\n" != raw:
            raise ValueError
        return TransactionContext(start_ms, value["deadline_monotonic_ns"], tuple(nonces))
    except (ValueError, TypeError, UnicodeError, RecursionError):
        raise ReadinessError(FailureCode.CONTEXT) from None


def capture_prior_nonces(home, *, reader=read_optional_private):
    document = reader(home)
    return () if document is None else (document["run_nonce"],)


def encode_context(context):
    raw = (json.dumps({"schema_version": 1,
                       "transaction_start_ms": context.transaction_start_ms,
                       "deadline_monotonic_ns": context.deadline_monotonic_ns,
                       "excluded_nonces": list(context.excluded_nonces)},
                      separators=(",", ":")) + "\n").encode()
    parse_context(raw, context.transaction_start_ms)
    return raw


def make_context(excluded_nonces, *, wall_ms=lambda: time.time_ns() // 1_000_000,
                 monotonic=time.monotonic_ns):
    context = TransactionContext(wall_ms(), monotonic() + BUDGET_NS, tuple(excluded_nonces))
    encode_context(context)
    return context


def read_context(fd, start_ms, entry_ns, *, monotonic=time.monotonic_ns,
                 wait=select.select, read=os.read):
    """Read bounded pipe input, parsing LF immediately to enforce its deadline."""
    deadline = entry_ns + BUDGET_NS
    raw = bytearray()
    context = None
    while True:
        remaining = deadline - monotonic()
        if remaining <= 0:
            raise ReadinessError(FailureCode.TIMEOUT)
        if not wait([fd], [], [], remaining / 1e9)[0]:
            raise ReadinessError(FailureCode.TIMEOUT)
        chunk = read(fd, CONTEXT_CAP + 1 - len(raw))
        if not chunk:
            if context is None:
                context = parse_context(bytes(raw), start_ms)
            if monotonic() >= deadline:
                raise ReadinessError(FailureCode.TIMEOUT)
            return context
        raw.extend(chunk)
        if len(raw) > CONTEXT_CAP:
            raise ReadinessError(FailureCode.CONTEXT)
        if b"\n" in raw:
            context = parse_context(bytes(raw), start_ms)
            deadline = min(deadline, context.deadline_monotonic_ns)


def parse_pid_record(raw, uid):
    """Accept one exact service root and one direct child PID; nested PIDs do not count."""
    try:
        if type(raw) is not bytes or len(raw) > STATUS_CAP:
            raise ValueError
        lines = raw.decode("utf-8").splitlines()
        if not lines or lines[0] != f"gui/{uid}/com.donaldfilimon.abbey-bot = {{":
            raise ValueError
        depth = 1
        pids = []
        closed = False
        for line in lines[1:]:
            stripped = line.strip()
            if closed:
                if stripped:
                    raise ValueError
                continue
            if depth == 1 and re.match(r"pid\s*=", stripped):
                match = re.fullmatch(r"\tpid = ([1-9][0-9]*)", line)
                if match is None or len(match[1]) > 10:
                    raise ValueError
                pids.append(int(match[1]))
            # launchctl print uses brace-delimited dictionaries/arrays.
            if stripped.endswith("{"):
                depth += 1
            if stripped == "}":
                depth -= 1
                closed = depth == 0
            if depth < 0:
                raise ValueError
        if not closed or len(pids) != 1 or not 1 <= pids[0] <= MAX_PID:
            raise ValueError
        return pids[0]
    except (ValueError, UnicodeError):
        raise ReadinessError(FailureCode.LAUNCHD) from None


def launchd_pid(deadline_ns, *, monotonic=time.monotonic_ns, uid=None):
    owner = os.getuid() if uid is None else uid
    # Reserve cleanup time inside the caller's allowance; bound each query too.
    query_deadline = min(deadline_ns - 250_000_000, monotonic() + 1_000_000_000)
    if query_deadline <= monotonic():
        raise ReadinessError(FailureCode.TIMEOUT)
    child = None
    try:
        child = subprocess.Popen(["/bin/launchctl", "print",
                                  f"gui/{owner}/com.donaldfilimon.abbey-bot"],
                                 stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                 stderr=subprocess.DEVNULL, env={"PATH": "/usr/bin:/bin"})
        result = bytearray()
        while True:
            remaining = query_deadline - monotonic()
            if remaining <= 0:
                raise ReadinessError(FailureCode.TIMEOUT)
            if not select.select([child.stdout], [], [], remaining / 1e9)[0]:
                raise ReadinessError(FailureCode.TIMEOUT)
            chunk = os.read(child.stdout.fileno(), min(4096, STATUS_CAP + 1 - len(result)))
            if not chunk:
                break
            result.extend(chunk)
            if len(result) > STATUS_CAP:
                raise ReadinessError(FailureCode.LAUNCHD)
        remaining = max(0, query_deadline - monotonic()) / 1e9
        if child.wait(timeout=remaining) != 0:
            raise ReadinessError(FailureCode.LAUNCHD)
        return parse_pid_record(bytes(result), owner)
    except (OSError, subprocess.SubprocessError):
        raise ReadinessError(FailureCode.LAUNCHD) from None
    finally:
        if child is not None:
            if child.poll() is None:
                try:
                    child.kill()
                    child.wait(timeout=max(0, deadline_ns - monotonic()) / 1e9)
                except (OSError, subprocess.SubprocessError):
                    _OUTSTANDING_CHILDREN.append(child)
                    if child.stdout is not None:
                        child.stdout.close()
                    raise ReadinessError(FailureCode.CLEANUP) from None
            if child.stdout is not None:
                child.stdout.close()


def wait_ready(context, expected_pid, expected_sha256, *, entry_ns,
               monotonic=time.monotonic_ns, wall_ms=lambda: time.time_ns() // 1_000_000,
               sleep=time.sleep, current_pid=launchd_pid, read_document,
               alive=process_exists):
    deadline = min(context.deadline_monotonic_ns, entry_ns + BUDGET_NS)
    encode_context(context)
    if monotonic() >= deadline:
        raise ReadinessError(FailureCode.TIMEOUT)
    first = None
    pinned = None
    while True:
        if monotonic() > deadline:
            raise ReadinessError(FailureCode.TIMEOUT)
        if current_pid(deadline) != expected_pid:
            raise ReadinessError(FailureCode.IDENTITY)
        try:
            present = alive(expected_pid)
        except OSError:
            present = False
        if present is not True:
            raise ReadinessError(FailureCode.DOCUMENT)
        validation_error = None
        candidate = None
        try:
            document = read_document()
            if document is None:
                raise ProtocolError(Failure.NOT_READY)
            candidate = validate_ready(document, transaction_start_ms=context.transaction_start_ms,
                                       now_ms=wall_ms(), launchd_pid=expected_pid,
                                       expected_sha256=expected_sha256, alive=lambda _: present)
            if candidate.nonce in context.excluded_nonces or (pinned and candidate != pinned):
                raise ReadinessError(FailureCode.IDENTITY)
        except ProtocolError as error:
            validation_error = error.category
        if current_pid(deadline) != expected_pid:
            raise ReadinessError(FailureCode.IDENTITY)
        if validation_error is not None:
            if first is not None or validation_error not in (Failure.NOT_READY, Failure.STALE):
                raise ReadinessError(FailureCode.DOCUMENT) from None
        else:
            now = monotonic()
            if now > deadline:
                raise ReadinessError(FailureCode.TIMEOUT)
            if first is None:
                first, pinned = now, candidate
            if now - first >= STABILITY_NS:
                return
        now = monotonic()
        if now >= deadline:
            raise ReadinessError(FailureCode.TIMEOUT)
        sleep(min(POLL_NS, deadline - now) / 1e9)


def parse_arguments(args):
    names = ("--transaction-start-ms", "--expected-sha256", "--launchd-pid")
    if len(args) != 6 or set(args[::2]) != set(names):
        raise ReadinessError(FailureCode.USAGE)
    values = dict(zip(args[::2], args[1::2]))
    start = values[names[0]]
    digest = values[names[1]]
    pid = values[names[2]]
    if (len(start) > 19 or not DECIMAL.fullmatch(start) or int(start) > MAX_TIME
            or len(pid) > 10 or not DECIMAL.fullmatch(pid) or not 1 <= int(pid) <= MAX_PID
            or not HEX.fullmatch(digest)):
        raise ReadinessError(FailureCode.USAGE)
    return int(start), digest, int(pid)


def main(args):
    entry = time.monotonic_ns()
    try:
        start, digest, pid = parse_arguments(args)
        context = read_context(0, start, entry)
        home = os.environ.get("HOME")
        if not home or not Path(home).is_absolute():
            raise ReadinessError(FailureCode.CONTEXT)
        wait_ready(context, pid, digest, entry_ns=entry,
                   read_document=lambda: read_optional_private(Path(home)))
        print("readiness: ready")
        return 0
    except ReadinessError as error:
        print(f"readiness: {error.code.value}")
        return 2 if error.code == FailureCode.USAGE else 1
    except Exception:
        print("readiness: internal")
        return 1
