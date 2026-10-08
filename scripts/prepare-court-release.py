#!/usr/bin/env python3
"""Prepare immutable Court source artifacts and inspectable dry-run receipts.

No deployment, launchctl, HTTP, DNS, proxy, TLS or Portal operations exist here.
"""
import argparse
import contextlib
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import selectors
import shutil
import stat
import subprocess
import tempfile
import time
from urllib.parse import urlsplit

ASSETS = ('app.js', 'court.js', 'index.html', 'server/court.mjs')
MAX_ASSET_BYTES = 4 * 1024 * 1024
MAX_MANIFEST_BYTES = 16 * 1024
HEX = re.compile(r'^[0-9a-f]{64}$')
NODE_VERSION = re.compile(r'^v[1-9][0-9]*\.[0-9]+\.[0-9]+$')


class PackageError(Exception):
    """Closed content-free failure reason."""


def canonical(value):
    return (json.dumps(value, sort_keys=True, separators=(',', ':'),
                       allow_nan=False) + '\n').encode('utf-8')


def digest(data):
    return hashlib.sha256(data).hexdigest()


def regular_bytes(path, maximum):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_size > maximum:
        raise PackageError('unsafe-file')
    with path.open('rb') as stream:
        data = stream.read(maximum + 1)
    if len(data) > maximum or len(data) != info.st_size:
        raise PackageError('unstable-file')
    return data


def no_symlink_path(path):
    path = Path(os.path.abspath(path))
    for part in (path, *path.parents):
        if part.is_symlink():
            raise PackageError('symlink-path')
    return path


def read_assets(root):
    root = no_symlink_path(root)
    values = {}
    for name in ASSETS:
        path = no_symlink_path(root / name)
        values[name] = regular_bytes(path, MAX_ASSET_BYTES)
    return values


def asset_rows(values):
    return [{'path': name, 'bytes': len(values[name]),
             'sha256': digest(values[name])} for name in ASSETS]


def court_digest(values):
    result = hashlib.sha256()
    for name in ASSETS:
        value = values[name]
        result.update(f'{name}\0{len(value)}\0'.encode())
        result.update(value)
    return result.hexdigest()


def default_ingress():
    return {
        'schema': 1, 'status': 'proposal-only',
        'backend': {'address': '127.0.0.1', 'port': 8791},
        'tls': {'origin': None, 'termination': 'operator-selected'},
        'limits': {'body_bytes': 1024, 'connections': 256,
                   'request_ms': 5000, 'headers_ms': 5000,
                   'socket_ms': 5000, 'absolute_body_ms': 5000},
        'logging': {'access': False, 'query': False, 'body': False},
        'routes': {'GET': ['/', '/index.html', '/app.js', '/court.js', '/health'],
                   'POST': ['/court', '/.proxy/court']},
        'pass_deployed_digest_header': True,
    }


def validate_ingress(value):
    if not isinstance(value, dict):
        raise PackageError('unsafe-ingress')
    expected = default_ingress()
    try:
        origin = value['tls']['origin']
        if origin is not None:
            if not isinstance(origin, str) or len(origin) > 253:
                raise PackageError('unsafe-ingress')
            parsed = urlsplit(origin)
            host = parsed.hostname or ''
            if (parsed.scheme != 'https' or parsed.netloc != host
                    or parsed.path or parsed.query or parsed.fragment
                    or any(not re.fullmatch(r'[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?', label)
                           for label in host.split('.'))
                    or '.' not in host or host.endswith(('.local', '.localhost'))):
                raise PackageError('unsafe-ingress')
            try:
                ipaddress.ip_address(host)
            except ValueError:
                pass
            else:
                raise PackageError('unsafe-ingress')
        expected['tls']['origin'] = origin
        # Compare canonical bytes to distinguish booleans from integers and
        # reject every unknown key, widened limit, route and logging toggle.
        if canonical(value) != canonical(expected):
            raise PackageError('unsafe-ingress')
    except (KeyError, TypeError, ValueError, RecursionError):
        raise PackageError('unsafe-ingress') from None
    return expected


def file_hash(path, maximum=256 * 1024 * 1024):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_size > maximum:
        raise PackageError('unsafe-node')
    result = hashlib.sha256()
    count = 0
    with path.open('rb') as stream:
        while chunk := stream.read(1024 * 1024):
            count += len(chunk)
            if count > maximum:
                raise PackageError('unsafe-node')
            result.update(chunk)
    if count != info.st_size:
        raise PackageError('unstable-node')
    return {'path': str(path), 'bytes': count, 'sha256': result.hexdigest()}


def inspect_node(path):
    path = Path(path).resolve(strict=True)
    before = file_hash(path)
    probe = None
    try:
        probe = subprocess.Popen([str(path), '--version'], stdin=subprocess.DEVNULL,
                                 stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                 env={'PATH': str(path.parent)})
        data = bytearray()
        deadline = time.monotonic() + 3
        # POSIX pipe readiness is bounded. An unavailable platform transport
        # fails closed rather than claiming a Windows runtime was observed.
        with selectors.DefaultSelector() as ready:
            ready.register(probe.stdout, selectors.EVENT_READ)
            while ready.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise PackageError('node-version-unavailable')
                for key, _ in ready.select(min(0.05, remaining)):
                    chunk = os.read(key.fd, 129 - len(data))
                    if not chunk:
                        ready.unregister(key.fileobj)
                    else:
                        data.extend(chunk)
                        if len(data) > 128:
                            raise PackageError('node-version-unavailable')
            code = probe.wait(timeout=max(0.001, deadline - time.monotonic()))
        if code != 0:
            raise PackageError('node-version-unavailable')
    except (subprocess.SubprocessError, OSError):
        raise PackageError('node-version-unavailable') from None
    finally:
        if probe is not None:
            if probe.poll() is None:
                probe.kill()
            probe.wait(timeout=3)
            probe.stdout.close()
    try:
        version = data.decode('ascii').strip()
    except UnicodeError:
        raise PackageError('node-version-unavailable') from None
    if not NODE_VERSION.fullmatch(version) or file_hash(path) != before:
        raise PackageError('unstable-node')
    return {**before, 'version': version}


def validate_node(value, current=True):
    if (not isinstance(value, dict) or set(value) != {'path', 'bytes', 'sha256', 'version'}
            or not isinstance(value['path'], str) or not Path(value['path']).is_absolute()
            or type(value['bytes']) is not int or not 0 < value['bytes'] <= 256 * 1024 * 1024
            or not isinstance(value['sha256'], str) or not HEX.fullmatch(value['sha256'])
            or not isinstance(value['version'], str) or not NODE_VERSION.fullmatch(value['version'])):
        raise PackageError('unsafe-node')
    if current:
        path = no_symlink_path(value['path'])
        observed = file_hash(path)
        if observed != {key: value[key] for key in ('path', 'bytes', 'sha256')}:
            raise PackageError('node-identity-changed')


def manifest_for(values, node, ingress):
    validate_node(node)
    return {
        'schema': 1, 'status': 'prepared-source-only', 'court_protocol': 2,
        'court_digest': court_digest(values), 'assets': asset_rows(values),
        'node': node, 'ingress': validate_ingress(ingress),
        'preparer_sha256': digest(regular_bytes(Path(__file__), 1024 * 1024)),
        'activation': 'operator-required',
    }


def load_json(data):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise PackageError('duplicate-json-key')
            result[key] = value
        return result
    try:
        return json.loads(data, object_pairs_hook=pairs,
                          parse_constant=lambda _: (_ for _ in ()).throw(PackageError('nonfinite-json')))
    except (ValueError, UnicodeError, RecursionError):
        raise PackageError('malformed-json') from None


def verify(release, current_node=True):
    release = no_symlink_path(release)
    info = release.lstat()
    if not stat.S_ISDIR(info.st_mode):
        raise PackageError('unsafe-release')
    if os.name == 'posix' and info.st_mode & 0o222:
        raise PackageError('writable-release')
    data = regular_bytes(release / 'release-manifest.json', MAX_MANIFEST_BYTES)
    value = load_json(data)
    required = {'schema', 'status', 'court_protocol', 'court_digest', 'assets',
                'node', 'ingress', 'preparer_sha256', 'activation'}
    if (not isinstance(value, dict) or set(value) != required or type(value['schema']) is not int
            or value['schema'] != 1 or type(value['court_protocol']) is not int
            or value['court_protocol'] != 2 or value['status'] != 'prepared-source-only'
            or value['activation'] != 'operator-required'
            or not isinstance(value['preparer_sha256'], str)
            or not HEX.fullmatch(value['preparer_sha256'])
            or data != canonical(value) or release.name != 'release-' + digest(data)):
        raise PackageError('manifest-identity-mismatch')
    validate_ingress(value['ingress'])
    validate_node(value['node'], current_node)
    expected_files = {*ASSETS, 'release-manifest.json'}
    seen_files = set()
    seen_dirs = set()
    for root, dirs, files in os.walk(release, followlinks=False):
        seen_dirs.add(Path(root).relative_to(release).as_posix())
        for name in (*dirs, *files):
            child = Path(root) / name
            no_symlink_path(child)
            if os.name == 'posix' and child.stat().st_mode & 0o222:
                raise PackageError('writable-release')
        for name in files:
            seen_files.add((Path(root) / name).relative_to(release).as_posix())
    if seen_files != expected_files or seen_dirs != {'.', 'server'}:
        raise PackageError('release-inventory-mismatch')
    values = read_assets(release)
    if canonical(value['assets']) != canonical(asset_rows(values)) or value['court_digest'] != court_digest(values):
        raise PackageError('asset-identity-mismatch')
    return value


def output_root(path):
    path = no_symlink_path(path)
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = path.stat()
    if (not stat.S_ISDIR(info.st_mode) or (os.name == 'posix'
            and (info.st_uid != os.getuid() or info.st_mode & 0o077))):
        raise PackageError('unsafe-output')
    return path


@contextlib.contextmanager
def writer_lock(output):
    lock = output / '.prepare.lock'
    try:
        fd = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    except FileExistsError:
        raise PackageError('output-busy') from None
    try:
        identity = os.fstat(fd)
        os.close(fd)
        yield
    finally:
        try:
            observed = lock.lstat()
        except FileNotFoundError:
            raise PackageError('output-lock-changed') from None
        if observed.st_dev != identity.st_dev or observed.st_ino != identity.st_ino:
            raise PackageError('output-lock-changed')
        lock.unlink()


def clean_stage(path):
    if path is None:
        return
    for root, dirs, files in os.walk(path):
        Path(root).chmod(0o700)
        for name in files:
            (Path(root) / name).chmod(0o600)
    shutil.rmtree(path)


def sync_directory(path):
    if os.name == 'posix':
        fd = os.open(path, os.O_RDONLY | getattr(os, 'O_DIRECTORY', 0))
        try:
            os.fsync(fd)
        finally:
            os.close(fd)


def prepare(source, output, node_path, ingress=None):
    source = no_symlink_path(source)
    requested_output = no_symlink_path(output)
    if requested_output.is_relative_to(source):
        raise PackageError('output-inside-source')
    values = read_assets(source)
    node = inspect_node(node_path)
    manifest = manifest_for(values, node, default_ingress() if ingress is None else ingress)
    encoded = canonical(manifest)
    output = output_root(requested_output)
    target = output / ('release-' + digest(encoded))
    with writer_lock(output):
        if target.exists() or target.is_symlink():
            if verify(target) != manifest:
                raise PackageError('existing-release-mismatch')
            if read_assets(source) != values:
                raise PackageError('source-changed')
            return target
        stage = Path(tempfile.mkdtemp(prefix='.stage-', dir=output))
        try:
            (stage / 'server').mkdir(mode=0o700)
            for name, value in values.items():
                with (stage / name).open('xb') as stream:
                    stream.write(value)
                    stream.flush()
                    os.fsync(stream.fileno())
                (stage / name).chmod(0o444)
            with (stage / 'release-manifest.json').open('xb') as stream:
                stream.write(encoded)
                stream.flush()
                os.fsync(stream.fileno())
            (stage / 'release-manifest.json').chmod(0o444)
            if read_assets(source) != values:
                raise PackageError('source-changed')
            validate_node(node)
            (stage / 'server').chmod(0o555)
            stage.chmod(0o555)
            sync_directory(stage / 'server')
            sync_directory(stage)
            stage.rename(target)
            stage = None
            sync_directory(output)
        finally:
            clean_stage(stage)
    verify(target)
    return target


def artifact_ref(path, value):
    return {'directory': str(no_symlink_path(path)),
            'manifest_sha256': digest(canonical(value)),
            'court_digest': value['court_digest'], 'node_sha256': value['node']['sha256']}


def rollback_plan(candidate, previous, receipt):
    current = verify(candidate)
    prior = verify(previous)
    same = canonical(current) == canonical(prior)
    value = {
        'schema': 1, 'kind': 'court-rollback-dry-run',
        'status': 'no-change' if same else 'operator-review-required',
        'candidate': artifact_ref(candidate, current),
        'rollback': artifact_ref(previous, prior),
        'service_label': 'com.donaldfilimon.abbey-court',
        'port': 8791, 'actions_executed': [],
        'preconditions': [
            'Retain exact original plist and installed four assets before activation.',
            'Observe the existing process stop to terminal before retargeting its plist.',
            'Reverify both packages and exact Node bytes immediately before operator execution.',
            'Room state is in memory; restart resets ballots with a new epoch.',
            'Source package metadata is not active HTTPS or Discord iframe qualification.',
        ],
        'proposed_steps': [] if same else [
            {'action': 'operator-retarget-existing-plist', 'working_directory': str(no_symlink_path(previous)),
             'node': prior['node']['path'], 'script': str(no_symlink_path(previous) / 'server/court.mjs'),
             'port': 8791},
            {'action': 'operator-start-existing-label', 'label': 'com.donaldfilimon.abbey-court'},
            {'action': 'operator-observe-health', 'url': 'http://127.0.0.1:8791/health',
             'protocol': 2, 'expected_digest': prior['court_digest']},
        ],
    }
    receipt = no_symlink_path(receipt)
    output = output_root(receipt.parent)
    encoded = canonical(value)
    with writer_lock(output):
        if receipt.exists() or receipt.is_symlink():
            if regular_bytes(receipt, MAX_MANIFEST_BYTES) != encoded:
                raise PackageError('existing-receipt-mismatch')
            return value
        fd, temp_name = tempfile.mkstemp(prefix='.receipt-', dir=output)
        staged = Path(temp_name)
        try:
            with os.fdopen(fd, 'wb') as stream:
                stream.write(encoded)
                stream.flush()
                os.fsync(stream.fileno())
            staged.chmod(0o400)
            # Exclusive hard-link publication refuses a concurrently created
            # target and never replaces an existing receipt.
            os.link(staged, receipt)
            sync_directory(output)
        finally:
            staged.unlink()
    return value


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    prepare_parser = commands.add_parser('prepare')
    prepare_parser.add_argument('--source', type=Path, required=True)
    prepare_parser.add_argument('--output', type=Path, required=True)
    prepare_parser.add_argument('--node', type=Path, required=True)
    prepare_parser.add_argument('--ingress', type=Path)
    verify_parser = commands.add_parser('verify')
    verify_parser.add_argument('release', type=Path)
    rollback_parser = commands.add_parser('rollback-plan')
    rollback_parser.add_argument('--candidate', type=Path, required=True)
    rollback_parser.add_argument('--previous', type=Path, required=True)
    rollback_parser.add_argument('--receipt', type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == 'prepare':
            ingress = load_json(regular_bytes(args.ingress, MAX_MANIFEST_BYTES)) if args.ingress else None
            result = {'release': str(prepare(args.source, args.output, args.node, ingress)),
                      'status': 'prepared-source-only'}
        elif args.command == 'verify':
            result = verify(args.release)
        else:
            result = rollback_plan(args.candidate, args.previous, args.receipt)
        print(canonical(result).decode(), end='')
        return 0
    except PackageError as error:
        print(str(error))
        return 1
    except (OSError, ValueError, TypeError, RecursionError):
        print('package-input-or-io-unavailable')
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
