"""Read-only validation of fixed managed installation artifacts."""
from __future__ import annotations

import hashlib
import os
from pathlib import Path
import plistlib
import stat
import time
from xml.parsers.expat import ExpatError

from service_protocol import Failure, ProtocolError


LABEL = "com.donaldfilimon.abbey-bot"


def installed_binary(home: Path) -> Path:
    return home / ".local/libexec/abbey-bot/abbey-bot"


def binary_digest(home: Path, deadline_ns: int, *, monotonic=time.monotonic_ns,
                  uid: int | None = None) -> str:
    """Hash an owned fixed binary through no-follow directory descriptors.

    Deadline checks bound cooperative work; they cannot bound a kernel read.
    No runtime data, owner environment or log is opened.
    """
    owner = os.getuid() if uid is None else uid
    descriptors = []
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW

    def check_deadline():
        if type(deadline_ns) is not int or monotonic() >= deadline_ns:
            raise ProtocolError(Failure.UNAVAILABLE)

    try:
        check_deadline()
        current = os.open(home, flags)
        descriptors.append(current)
        for component in (None, ".local", "libexec", "abbey-bot"):
            check_deadline()
            if component is not None:
                current = os.open(component, flags, dir_fd=current)
                descriptors.append(current)
            metadata = os.fstat(current)
            mode = stat.S_IMODE(metadata.st_mode)
            if (metadata.st_uid != owner or not stat.S_ISDIR(metadata.st_mode)
                    or mode & 0o022 or (component == "abbey-bot" and mode != 0o700)):
                raise ProtocolError(Failure.UNSAFE_FILE)
        handle = os.open("abbey-bot", os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                         dir_fd=current)
        descriptors.append(handle)
        before = os.fstat(handle)
        if (before.st_uid != owner or not stat.S_ISREG(before.st_mode)
                or stat.S_IMODE(before.st_mode) != 0o700):
            raise ProtocolError(Failure.UNSAFE_FILE)
        digest = hashlib.sha256()
        while True:
            check_deadline()
            chunk = os.read(handle, 65536)
            if not chunk:
                break
            digest.update(chunk)
        after = os.fstat(handle)
        attributes = ("st_dev", "st_ino", "st_size", "st_mtime_ns", "st_ctime_ns")
        if any(getattr(before, name) != getattr(after, name) for name in attributes):
            raise ProtocolError(Failure.UNAVAILABLE)
        check_deadline()
        result = digest.hexdigest()
    except OSError:
        raise ProtocolError(Failure.UNAVAILABLE) from None
    finally:
        cleanup_failed = False
        for handle in reversed(descriptors):
            try:
                os.close(handle)
            except OSError:
                cleanup_failed = True
    if cleanup_failed:
        raise ProtocolError(Failure.UNAVAILABLE)
    check_deadline()
    return result


def validate_managed_plist(raw: bytes, home: Path) -> None:
    """Validate the executable launch contract before bootstrap, never execute it."""
    if type(raw) is not bytes or len(raw) > 65536:
        raise ProtocolError(Failure.INVALID_DOCUMENT)
    class UniqueDict(dict):
        def __setitem__(self, key, item):
            if key in self:
                raise ProtocolError(Failure.INVALID_DOCUMENT)
            super().__setitem__(key, item)

    try:
        value = plistlib.loads(raw, dict_type=UniqueDict)
    except (ValueError, TypeError, OverflowError, RecursionError, ExpatError):
        raise ProtocolError(Failure.INVALID_DOCUMENT) from None
    if (not isinstance(value, dict) or value.get("Label") != LABEL
            or value.get("ProgramArguments") != [str(installed_binary(home)), "--managed-service"]
            or "Program" in value or "BundleProgram" in value
            or value.get("WorkingDirectory") != str(home / ".local/share/abbey-bot")
            or value.get("StandardOutPath") != "/dev/null"
            or value.get("StandardErrorPath") != "/dev/null"
            or type(value.get("Umask")) is not int or value["Umask"] != 0o077):
        raise ProtocolError(Failure.INVALID_DOCUMENT)
