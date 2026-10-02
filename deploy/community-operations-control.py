#!/usr/bin/env python3
"""Owner-local policy controls; never sends a Discord request or exposes credentials."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import tempfile


def read_private_snapshot(path):
    metadata = path.lstat()
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > 8 * 1024 * 1024:
        raise ValueError('unsafe operations file')
    if os.name == 'posix' and (metadata.st_uid != os.geteuid() or metadata.st_mode & 0o077):
        raise ValueError('operations file must be owner-only')
    raw = path.read_bytes()
    return json.loads(raw), hashlib.sha256(raw).hexdigest()


def read_private(path):
    return read_private_snapshot(path)[0]


def publish(path, value, expected_digest):
    raw = json.dumps(value, indent=2).encode() + b'\n'
    handle, temporary = tempfile.mkstemp(prefix='.community-policy-', dir=path.parent)
    try:
        with os.fdopen(handle, 'wb') as file:
            file.write(raw)
            file.flush()
            os.fsync(file.fileno())
        if read_private_snapshot(path)[1] != expected_digest:
            raise ValueError('policy changed during publication')
        os.replace(temporary, path)
        if os.name == 'posix':
            directory = os.open(path.parent, os.O_RDONLY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def control(path, mode=None):
    lock = path.with_suffix('.mode-lock')
    handle = None
    try:
        if mode is not None:
            handle = os.open(lock, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        policy, digest = read_private_snapshot(path)
        if policy.get('version') != 1 or policy.get('mode') not in ('stopped', 'propose', 'apply'):
            raise ValueError('invalid operations policy')
        if mode is not None:
            policy['mode'] = mode
            publish(path, policy, digest)
            confirmed = read_private(path)
            if confirmed != policy:
                raise ValueError('policy changed during readback')
        return {'version': policy['version'], 'mode': policy['mode'],
                'daily_limit': policy.get('daily_limit'),
                'daily_creations': policy.get('daily_creations'),
                'planned_actions': len(policy.get('actions', []))}
    finally:
        if handle is not None:
            os.close(handle)
            lock.unlink()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--policy', type=Path, required=True)
    parser.add_argument('--mode', choices=('stopped', 'propose', 'apply'))
    arguments = parser.parse_args()
    if not arguments.policy.is_absolute():
        parser.error('policy path must be absolute')
    try:
        print(json.dumps(control(arguments.policy, arguments.mode), sort_keys=True))
    except (OSError, ValueError, TypeError):
        parser.exit(1, 'operations control failed; no Discord action was attempted\n')


if __name__ == '__main__':
    main()
