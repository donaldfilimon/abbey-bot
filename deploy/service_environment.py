"""Pure validation for the fixed managed owner environment; never evaluates code.

The installer supplies already descriptor-validated private bytes. The Rust
runtime consumes the same literal fixture corpus. Errors contain categories only.
"""
from __future__ import annotations

import re

MAX_ENV_BYTES = 64 * 1024
# Rust char::is_whitespace (Unicode White_Space), deliberately not Python's
# broader str.strip()/splitlines() set, which additionally treats C0 separators.
WHITESPACE = '\u0009\u000a\u000b\u000c\u000d\u0020\u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000'
KEY = re.compile(r'[A-Za-z_][A-Za-z0-9_]*', re.ASCII)


class EnvironmentError(Exception):
    def __init__(self, code: str):
        super().__init__(code)


def validate_environment(document: bytes) -> None:
    """Validate supported assignment syntax and required nonblank values.

    Values are transient data and are never returned, logged, expanded, sourced,
    or used as paths. HOME/data overrides are ignored by the Rust runtime.
    """
    if type(document) is not bytes or len(document) > MAX_ENV_BYTES or b'\0' in document:
        raise EnvironmentError('syntax')
    try:
        text = document.decode('utf-8', errors='strict')
    except UnicodeError:
        raise EnvironmentError('syntax') from None
    values = {}
    for line in text.split('\n'):
        line = line.strip(WHITESPACE)
        if not line or line.startswith('#'):
            continue
        if line.startswith('export') and len(line) > 6 and line[6] in WHITESPACE:
            line = line[6:].lstrip(WHITESPACE)
        if '=' not in line:
            raise EnvironmentError('syntax')
        key, value = line.split('=', 1)
        if KEY.fullmatch(key) is None or key in values:
            raise EnvironmentError('syntax')
        value = value.strip(WHITESPACE)
        if value.startswith(('"', "'")):
            if len(value) < 2 or value[-1] != value[0]:
                raise EnvironmentError('syntax')
            value = value[1:-1]
        values[key] = value
    present = lambda key: bool(values.get(key, '').strip(WHITESPACE))
    if (not present('DISCORD_TOKEN')
            or present('ABBEY_VOICE_GUILD_ID') != present('ABBEY_VOICE_CHANNEL_ID')
            or (present('ABBEY_VOICE_GUILD_ID') and not present('ABBEY_BOT_LLM_ENDPOINT'))):
        raise EnvironmentError('required_configuration')
