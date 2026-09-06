#!/usr/bin/env python3
"""Private phase protocol for the shell-owned launchd transaction.

Run with Python -I. Stdout is private transaction state; stderr is closed codes.
Production effects are injected explicitly by the offline fixture, never by env.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import plistlib
import secrets
import select
import stat
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
try:
    from service_installation import binary_digest, validate_managed_plist
    from service_environment import EnvironmentError, validate_environment
    from service_protocol import ProtocolError, read_optional_private
    from service_readiness import (BUDGET_NS, TransactionContext, ReadinessError,
                                   launchd_pid, make_context, wait_ready, parse_pid_record, cleanup_incomplete)
except Exception:
    print("installation: bundle", file=sys.stderr)
    raise SystemExit(1) from None

LABEL = 'com.donaldfilimon.abbey-bot'
STATE = '.local/share/abbey-bot'
BIN = '.local/libexec/abbey-bot/abbey-bot'
PLIST = 'Library/LaunchAgents/' + LABEL + '.plist'
ENV = '.config/abbey-bot/env'
MAX_ARTIFACT = 512 * 1024 * 1024


class TransactionError(Exception):
    def __init__(self, code):
        self.code = code
        super().__init__(code)


class Production:
    monotonic = staticmethod(time.monotonic_ns)
    wall = staticmethod(lambda: time.time_ns() // 1000000)
    sleep = staticmethod(time.sleep)

    def pid(self, deadline):
        return launchd_pid(deadline, monotonic=self.monotonic)

    def alive(self, pid):
        # Unlike readiness, only ESRCH proves a stopped process.
        try:
            os.kill(pid, 0)
            return True
        except ProcessLookupError:
            return False
        except OSError:
            raise TransactionError('process') from None

    def control(self, operation, home, deadline):
        arguments = ['/bin/launchctl', operation]
        arguments += ([f'gui/{os.getuid()}', str(home / PLIST)] if operation == 'bootstrap'
                      else [f'gui/{os.getuid()}/{LABEL}'])
        deadline = min(deadline, self.monotonic() + 2_000_000_000)
        remaining = min(1.5, max(0, deadline - self.monotonic()) / 1e9)
        if remaining <= 0:
            raise TransactionError('timeout')
        child = subprocess.Popen(arguments, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                 stderr=subprocess.DEVNULL, env={'PATH': '/usr/bin:/bin'})
        try:
            result = child.wait(timeout=remaining)
        except subprocess.TimeoutExpired:
            raise TransactionError('timeout') from None
        finally:
            if child.poll() is None:
                try:
                    child.kill()
                    child.wait(timeout=max(0, deadline - self.monotonic()) / 1e9)
                except (OSError, subprocess.TimeoutExpired):
                    _CHILDREN.append(child)
                    raise TransactionError('cleanup') from None
        if self.monotonic() > deadline:
            raise TransactionError('timeout')
        return result

    def status(self, deadline):
        deadline = min(deadline, self.monotonic() + 2_000_000_000)
        query_deadline = deadline - 250_000_000
        child = subprocess.Popen(['/bin/launchctl', 'print', f'gui/{os.getuid()}/{LABEL}'],
                                 stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE, env={'PATH': '/usr/bin:/bin'})
        data = {child.stdout: bytearray(), child.stderr: bytearray()}
        open_pipes = list(data)
        try:
            while open_pipes:
                remaining = query_deadline - self.monotonic()
                if remaining <= 0:
                    raise TransactionError('timeout')
                ready = select.select(open_pipes, [], [], remaining / 1e9)[0]
                if not ready:
                    raise TransactionError('timeout')
                for pipe in ready:
                    chunk = os.read(pipe.fileno(), 4096)
                    if not chunk:
                        open_pipes.remove(pipe)
                    data[pipe].extend(chunk)
                    if sum(map(len, data.values())) > 65536:
                        raise TransactionError('launchd')
            code = child.wait(timeout=max(0, query_deadline-self.monotonic()) / 1e9)
            return code, bytes(data[child.stdout]), bytes(data[child.stderr])
        finally:
            cleanup_failed = False
            if child.poll() is None:
                try:
                    child.kill()
                    child.wait(timeout=max(0, deadline-self.monotonic()) / 1e9)
                except (OSError, subprocess.TimeoutExpired):
                    _CHILDREN.append(child)
                    cleanup_failed = True
            for pipe in (child.stdout, child.stderr):
                try:
                    pipe.close()
                except OSError:
                    cleanup_failed = True
            if cleanup_failed:
                raise TransactionError('cleanup') from None
            if self.monotonic() > deadline:
                raise TransactionError('timeout')

    def absent(self, home, deadline):
        code, out, err = self.status(deadline)
        expected = f'Could not find service "{LABEL}" in domain for user gui: {os.getuid()}'
        if code == 113 and expected in err.decode('utf8', errors='replace').splitlines():
            return True
        if code == 0:
            return False
        raise TransactionError('launchd')

    def optional_pid(self, deadline):
        code, out, _err = self.status(deadline)
        if code != 0:
            raise TransactionError('launchd')
        return parse_pid_record(out, os.getuid(), allow_absent=True)

    def exit78(self, deadline):
        code, out, _err = self.status(deadline)
        if code != 0:
            return False
        lines = out.decode('utf8').splitlines()
        if not lines or lines[0] != f'gui/{os.getuid()}/{LABEL} = {{':
            raise TransactionError('launchd')
        if any(line.startswith('\tpid') for line in lines):
            parse_pid_record(out, os.getuid())
            return False
        matches = [line for line in lines if line.startswith('\tlast exit code = ')]
        if len(matches) > 1:
            raise TransactionError('launchd')
        return matches == ['\tlast exit code = 78']


_CHILDREN = []
SYSTEM = Production()


class PrivateTree:
    def __init__(self, home):
        self.home = home
        self.uid = os.getuid()
        self.root = os.open(home, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            self.check(os.fstat(self.root), True)
        except BaseException:
            os.close(self.root)
            raise

    def close(self):
        os.close(self.root)

    def check(self, info, directory, mode=None):
        if (info.st_uid != self.uid or (not stat.S_ISDIR(info.st_mode) if directory else not stat.S_ISREG(info.st_mode))
                or stat.S_IMODE(info.st_mode) & 0o022 or
                (mode is not None and stat.S_IMODE(info.st_mode) != mode)):
            raise TransactionError('unsafe_file')

    def directory(self, relative, create=False, exact=False):
        parts = relative.split('/') if relative else []
        if any(p in ('', '.', '..') for p in parts):
            raise TransactionError('protocol')
        current = os.dup(self.root)
        try:
            for part in parts:
                if create:
                    try:
                        os.mkdir(part, 0o700, dir_fd=current)
                    except FileExistsError:
                        pass
                following = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=current)
                os.close(current)
                current = following
                self.check(os.fstat(current), True)
            if exact:
                self.check(os.fstat(current), True, 0o700)
            return current
        except BaseException:
            os.close(current)
            raise

    def ensure(self, relative, exact=True):
        fd = self.directory(relative, create=True, exact=exact)
        os.close(fd)

    def read(self, relative, mode=0o600, optional=False, cap=MAX_ARTIFACT):
        parent, name = relative.rsplit('/', 1)
        try:
            directory = self.directory(parent)
        except FileNotFoundError:
            if optional:
                return None
            raise
        try:
            try:
                handle = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
            except FileNotFoundError:
                if optional:
                    return None
                raise
            try:
                before = os.fstat(handle)
                self.check(before, False, mode)
                if before.st_size > cap:
                    raise TransactionError('artifact')
                result = bytearray()
                while len(result) <= cap:
                    chunk = os.read(handle, min(65536, cap + 1 - len(result)))
                    if not chunk:
                        break
                    result.extend(chunk)
                after = os.fstat(handle)
                if len(result) > cap or any(getattr(before, k) != getattr(after, k) for k in
                        ('st_dev', 'st_ino', 'st_size', 'st_mtime_ns', 'st_ctime_ns')):
                    raise TransactionError('artifact')
                return bytes(result)
            finally:
                os.close(handle)
        finally:
            os.close(directory)

    def write(self, relative, raw, mode=0o600, absent=False):
        parent, name = relative.rsplit('/', 1)
        directory = self.directory(parent)
        temporary = '.transaction-' + secrets.token_hex(16)
        try:
            try:
                info = os.stat(name, dir_fd=directory, follow_symlinks=False)
                self.check(info, False, mode)
                if absent:
                    raise TransactionError('exists')
            except FileNotFoundError:
                pass
            handle = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                             mode, dir_fd=directory)
            try:
                view = memoryview(raw)
                while view:
                    count = os.write(handle, view)
                    if count <= 0:
                        raise TransactionError('write')
                    view = view[count:]
                os.fsync(handle)
            finally:
                os.close(handle)
            os.rename(temporary, name, src_dir_fd=directory, dst_dir_fd=directory)
            os.fsync(directory)
        finally:
            try:
                os.unlink(temporary, dir_fd=directory)
            except FileNotFoundError:
                pass
            os.close(directory)

    def remove(self, relative, mode=0o600, expected=None):
        observed = self.read(relative, mode, optional=True)
        if observed is None:
            return
        if expected is not None and observed != expected:
            raise TransactionError('successor')
        parent, name = relative.rsplit('/', 1)
        directory = self.directory(parent)
        try:
            try:
                os.unlink(name, dir_fd=directory)
                os.fsync(directory)
            except FileNotFoundError:
                pass
        finally:
            os.close(directory)


def capture(home):
    document = read_optional_private(home)
    return None if document is None else {key: document[key] for key in ('pid', 'run_nonce')}


def add_nonce(state, baseline):
    if baseline is not None and baseline['run_nonce'] not in state['excluded']:
        state['excluded'].append(baseline['run_nonce'])
    if len(state['excluded']) > 2:
        raise TransactionError('identity')


def verify_lock(tree, state):
    for parent in (STATE, STATE + '/install.lock'):
        descriptor = tree.directory(parent, exact=True)
        os.close(descriptor)
    content = tree.read(STATE + '/install.lock/owner', cap=128)
    if content != state['lock'].encode():
        raise TransactionError('lock_owner')


def stop(home):
    deadline = SYSTEM.monotonic() + 25_000_000_000
    prior = capture(home)
    boot = read_optional_private(home, 'bootstrap')
    known_pids = {value['pid'] for value in (prior, boot) if value is not None}
    unloaded = SYSTEM.absent(home, deadline)
    if not unloaded:
        # A canonical loaded record without a PID is a valid failed-start state.
        # Malformed/ambiguous records remain failures, never an absent process.
        current = SYSTEM.optional_pid(deadline)
        if current is not None:
            known_pids.add(current)
        if SYSTEM.control('bootout', home, deadline) != 0:
            raise TransactionError('stop')
    while True:
        stopped = all(not SYSTEM.alive(pid) for pid in known_pids)
        unloaded = SYSTEM.absent(home, deadline)
        if stopped and unloaded:
            if SYSTEM.monotonic() > deadline:
                raise TransactionError('stop')
            return
        remaining = deadline - SYSTEM.monotonic()
        if remaining <= 0:
            raise TransactionError('stop')
        SYSTEM.sleep(min(.25, remaining / 1e9))


def bootstrap(home, state):
    digest = binary_digest(home, SYSTEM.monotonic() + 5_000_000_000, monotonic=SYSTEM.monotonic)
    # Hash/plist inspection precedes the one candidate bootstrap/readiness budget.
    context = make_context(state['excluded'], wall_ms=SYSTEM.wall, monotonic=SYSTEM.monotonic)
    if SYSTEM.control('bootstrap', home, context.deadline_monotonic_ns) != 0:
        raise TransactionError('bootstrap')
    while SYSTEM.monotonic() < context.deadline_monotonic_ns:
        if SYSTEM.exit78(context.deadline_monotonic_ns):
            raise TransactionError('bootstrap_exit78')
        failure = read_optional_private(home, 'bootstrap')
        if failure is not None and failure['phase'] == 'failed' and failure['executable_sha256'] == digest:
            if failure['run_nonce'] not in state['excluded']:
                raise TransactionError('bootstrap_' + failure['code'])
        try:
            pid = SYSTEM.pid(context.deadline_monotonic_ns)
            break
        except ReadinessError as error:
            if error.code.value == 'cleanup':
                raise
            SYSTEM.sleep(min(.25, max(0, context.deadline_monotonic_ns - SYSTEM.monotonic()) / 1e9))
    else:
        raise TransactionError('timeout')
    def current_document():
        boot = read_optional_private(home, 'bootstrap')
        if (boot is not None and boot['phase'] == 'failed' and boot['pid'] == pid
                and boot['executable_sha256'] == digest and boot['run_nonce'] not in state['excluded']):
            raise TransactionError('bootstrap_' + boot['code'])
        if SYSTEM.exit78(context.deadline_monotonic_ns):
            raise TransactionError('bootstrap_exit78')
        return read_optional_private(home)
    wait_ready(context, pid, digest, entry_ns=context.deadline_monotonic_ns - BUDGET_NS,
               monotonic=SYSTEM.monotonic, wall_ms=SYSTEM.wall, sleep=SYSTEM.sleep,
               current_pid=SYSTEM.pid, alive=SYSTEM.alive,
               read_document=current_document)


def acquire(tree):
    tree.ensure(STATE)
    parent = tree.directory(STATE)
    try:
        os.mkdir('install.lock', 0o700, dir_fd=parent)
    except FileExistsError:
        raise TransactionError('locked') from None
    finally:
        os.close(parent)
    token = secrets.token_hex(32)
    try:
        tree.write(STATE + '/install.lock/owner', token.encode(), absent=True)
    except BaseException:
        parent = tree.directory(STATE)
        try:
            os.rmdir('install.lock', dir_fd=parent)
        finally:
            os.close(parent)
        raise
    return {'lock': token, 'rollback': secrets.token_hex(16), 'excluded': [],
            'baseline': None, 'bootstrap_baseline': None, 'had_bin': False, 'had_plist': False, 'prepared': False}


def phase(operation, home, state, checkout):
    tree = PrivateTree(home)
    try:
        if operation == 'acquire':
            return acquire(tree)
        verify_lock(tree, state)
        backup = STATE + '/rollback/abbey/' + state['rollback']
        if operation == 'prepare':
            for directory in ('.local/libexec/abbey-bot', STATE + '/data', STATE + '/rollback',
                              STATE + '/rollback/abbey', backup):
                tree.ensure(directory)
            tree.ensure('Library/LaunchAgents', exact=False)
            descriptor = tree.directory('.config/abbey-bot', exact=True)
            os.close(descriptor)
            env = tree.read(ENV, cap=65536)
            # Match the managed runtime parser before the shell may stop a prior run.
            try:
                validate_environment(env)
            except EnvironmentError:
                raise TransactionError('environment') from None
            # Existing names-only diagnostics still run separately under the shell.
            old_bin = tree.read(BIN, 0o700, optional=True)
            old_plist = tree.read(PLIST, optional=True, cap=65536)
            state['baseline'] = capture(home)
            boot = read_optional_private(home, 'bootstrap')
            state['bootstrap_baseline'] = None if boot is None else {k: boot[k] for k in ('pid', 'run_nonce')}
            add_nonce(state, state['baseline'])
            add_nonce(state, state['bootstrap_baseline'])
            for name, raw, mode in (('binary', old_bin, 0o700), ('plist', old_plist, 0o600), ('env', env, 0o600)):
                if raw is not None:
                    tree.write(backup + '/' + name, raw, mode, absent=True)
            state['had_bin'], state['had_plist'] = old_bin is not None, old_plist is not None
            build_tree = PrivateTree(checkout)
            try:
                candidate = build_tree.read('target/release/abbey-bot', mode=None)
            finally:
                build_tree.close()
            tree.write(backup + '/candidate-binary', candidate, 0o700, absent=True)
            model = plistlib.loads((checkout / 'deploy' / (LABEL + '.plist')).read_bytes())
            model['ProgramArguments'] = [str(home / BIN), '--managed-service']
            model['WorkingDirectory'] = str(home / STATE)
            raw = plistlib.dumps(model)
            validate_managed_plist(raw, home)
            tree.write(backup + '/candidate-plist', raw, absent=True)
            state['prepared'] = True
        elif operation == 'publish':
            if not state['prepared']:
                raise TransactionError('protocol')
            candidate = tree.read(backup + '/candidate-binary', 0o700)
            raw = tree.read(backup + '/candidate-plist', cap=65536)
            validate_managed_plist(raw, home)
            tree.write(BIN, candidate, 0o700)
            tree.write(PLIST, raw)
            if binary_digest(home, SYSTEM.monotonic() + 5_000_000_000, monotonic=SYSTEM.monotonic) != hashlib.sha256(candidate).hexdigest():
                raise TransactionError('artifact')
        elif operation == 'capture':
            add_nonce(state, capture(home))
            boot = read_optional_private(home, 'bootstrap')
            add_nonce(state, boot)
        elif operation == 'stop':
            stop(home)
        elif operation == 'start':
            validate_managed_plist(tree.read(PLIST, cap=65536), home)
            bootstrap(home, state)
        elif operation == 'restore':
            for target, name, present, mode in ((BIN, 'binary', state['had_bin'], 0o700),
                                               (PLIST, 'plist', state['had_plist'], 0o600)):
                if present:
                    tree.write(target, tree.read(backup + '/' + name, mode), mode)
                else:
                    tree.remove(target, mode)
        elif operation == 'uninstall_prepare':
            state['baseline'] = capture(home)
            # Reject unsafe fixed targets before stopping the service.
            tree.read(PLIST, optional=True, cap=65536)
            boot = read_optional_private(home, 'bootstrap')
            state['bootstrap_baseline'] = None if boot is None else {k: boot[k] for k in ('pid', 'run_nonce')}
        elif operation == 'uninstall':
            for kind, name in (('readiness', 'readiness.json'), ('bootstrap', 'bootstrap-status.json')):
                value = read_optional_private(home, kind)
                prior = state['baseline'] if kind == 'readiness' else state['bootstrap_baseline']
                if value is not None and prior is not None and all(value[k] == prior[k] for k in prior):
                    raw = tree.read(STATE + '/' + name, cap=4096)
                    observed = json.loads(raw)
                    if all(observed.get(k) == prior[k] for k in prior):
                        tree.remove(STATE + '/' + name, expected=raw)
            tree.remove(PLIST)
        elif operation == 'release':
            tree.remove(STATE + '/install.lock/owner')
            directory = tree.directory(STATE)
            try:
                os.rmdir('install.lock', dir_fd=directory)
            except OSError:
                tree.write(STATE + '/install.lock/owner', state['lock'].encode(), absent=True)
                raise TransactionError('lock_release') from None
            finally:
                os.close(directory)
        else:
            raise TransactionError('usage')
        return state
    finally:
        tree.close()


def main(args):
    try:
        if len(args) != 1:
            raise TransactionError('usage')
        home = Path(os.environ['HOME'])
        if not home.is_absolute():
            raise TransactionError('home')
        state = None
        if args[0] != 'acquire':
            raw = sys.stdin.buffer.read(4097)
            if len(raw) > 4096:
                raise TransactionError('protocol')
            def unique_pairs(items):
                value = {}
                for key, item in items:
                    if key in value:
                        raise TransactionError('protocol')
                    value[key] = item
                return value
            state = json.loads(raw, object_pairs_hook=unique_pairs)
            if (type(state) is not dict or set(state) != {'lock', 'rollback', 'excluded', 'baseline', 'bootstrap_baseline', 'had_bin', 'had_plist', 'prepared'}
                    or type(state['lock']) is not str or len(state['lock']) != 64
                    or any(c not in '0123456789abcdef' for c in state['lock'])
                    or type(state['rollback']) is not str or len(state['rollback']) != 32
                    or any(c not in '0123456789abcdef' for c in state['rollback'])):
                raise TransactionError('protocol')
            if (any(type(state[k]) is not bool for k in ('had_bin', 'had_plist', 'prepared'))
                    or type(state['excluded']) is not list):
                raise TransactionError('protocol')
            # Reuse the exact private comparison-context validator for nonce bounds.
            make_context(state['excluded'], wall_ms=lambda: 0, monotonic=lambda: 0)
            for key in ('baseline', 'bootstrap_baseline'):
                value = state[key]
                if value is not None:
                    if (type(value) is not dict or set(value) != {'pid', 'run_nonce'}
                            or type(value['pid']) is not int or not 1 <= value['pid'] <= 2147483647):
                        raise TransactionError('protocol')
                    make_context((value['run_nonce'],), wall_ms=lambda: 0, monotonic=lambda: 0)
        result = phase(args[0], home, state, Path(__file__).resolve().parent.parent)
        if _CHILDREN or cleanup_incomplete():
            raise TransactionError('cleanup')
        print(json.dumps(result, separators=(',', ':')))
        return 0
    except TransactionError as error:
        print('installation: ' + error.code, file=sys.stderr)
        if error.code == 'cleanup':
            return 79
    except ReadinessError as error:
        print('installation: verification', file=sys.stderr)
        if error.code.value == 'cleanup':
            return 79
    except ProtocolError:
        print('installation: verification', file=sys.stderr)
    except Exception:
        print('installation: failure', file=sys.stderr)
    return 79 if _CHILDREN or cleanup_incomplete() else 1


if __name__ == '__main__':
    raise SystemExit(main(sys.argv[1:]))
