#!/usr/bin/env python3
"""Reject source logging that could serialize Abbey's private request data.

This deliberately scans logging/printing macro expressions, not prose in their
format strings. Fixed messages may describe a "request body" failure; the
unsafe behavior is interpolating or attaching the body itself.
"""

from __future__ import annotations

import ast
import pathlib
import re
import shlex
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE = ROOT / "src"

MACRO = re.compile(
    r"(?:(?:tracing|log)::)?"
    r"(?P<name>trace|debug|info|warn|error|event|span|log|print|println|eprint|"
    r"eprintln|write|writeln|panic|dbg)!\s*"
    r"(?P<delimiter>[({[])"
)
SENSITIVE = re.compile(
    r"(?:^|_)"
    r"(?:authorization|auth_header|api_key|discord_token|telegram_bot_token|"
    r"slack_bot_token|slack_app_token|openai_api_key|anthropic_api_key|token|"
    r"request_body|response_body|body|payload|prompt|system_prompt|transcript|"
    r"data_url|image|image_bytes|vision|ocr|private_context|messages)"
    r"(?:$|_)"
)
SAFE_SUFFIXES = (
    "_bytes_len",
    "_category",
    "_chars",
    "_configured",
    "_count",
    "_hash",
    "_kind",
    "_len",
    "_media_type",
    "_mode",
    "_pass",
    "_present",
    "_redacted",
    "_sha256",
    "_status",
)
FIELD = re.compile(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*=")
CAPTURE = re.compile(r"[?%]\s*&?\s*([A-Za-z_][A-Za-z0-9_]*)")
IDENTIFIER = re.compile(r"\b[A-Za-z_][A-Za-z0-9_]*\b")
INSTRUMENT = re.compile(r"#\s*\[\s*(?:tracing::)?instrument\b([^]]*)\]")
RUST_SAFE_METADATA = re.compile(
    r"\b[A-Za-z_][A-Za-z0-9_]*\s*\.\s*(?:len|is_empty)\s*\(\s*\)"
)
PYTHON_LOG_METHODS = {
    "critical",
    "debug",
    "error",
    "exception",
    "info",
    "log",
    "pp",
    "pprint",
    "warn",
    "warning",
}
RUST_OUTPUT_CALL = re.compile(
    r"(?P<name>std::io::Write::(?:write|write_all|write_fmt|write_vectored)|"
    r"\.(?:write|write_all|write_fmt|write_vectored))\s*\("
)
SHELL_OUTPUT = re.compile(r"\b(?:cat|echo|printf|logger|tee)\b")
SHELL_VARIABLE = re.compile(r"\$(?:\{)?([A-Za-z_][A-Za-z0-9_]*)")
SHELL_HEREDOC = re.compile(r"<<-?\s*(['\"]?)([A-Za-z_][A-Za-z0-9_]*)\1")
SHELL_DUMP_COMMAND = re.compile(
    r"(?:^|(?:&&|\|\||[;&|])\s*)(?:command\s+)?"
    r"(?P<command>env|printenv|set|export)\b(?P<arguments>[^;&|]*)"
)
SHELL_REDIRECTION = re.compile(r"(?:^|\s)(?:\d*>>?|&>)")
SHELL_ASSIGNMENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")


def sensitive(identifier: str) -> bool:
    lowered = identifier.lower()
    return not lowered.endswith(SAFE_SUFFIXES) and SENSITIVE.search(lowered) is not None


def mask_literals_and_comments(source: str) -> str:
    """Return source with strings, character literals, and comments blanked."""

    chars = list(source)
    output = list(source)
    index = 0
    length = len(chars)
    block_depth = 0
    while index < length:
        if block_depth:
            output[index] = "\n" if chars[index] == "\n" else " "
            if index + 1 < length and chars[index] == "/" and chars[index + 1] == "*":
                output[index + 1] = " "
                block_depth += 1
                index += 2
            elif index + 1 < length and chars[index] == "*" and chars[index + 1] == "/":
                output[index + 1] = " "
                block_depth -= 1
                index += 2
            else:
                index += 1
            continue
        if index + 1 < length and chars[index] == "/" and chars[index + 1] == "/":
            while index < length and chars[index] != "\n":
                output[index] = " "
                index += 1
            continue
        if index + 1 < length and chars[index] == "/" and chars[index + 1] == "*":
            output[index] = output[index + 1] = " "
            block_depth = 1
            index += 2
            continue
        raw = re.match(r"r(#+)?\"", source[index:])
        if raw:
            hashes = raw.group(1) or ""
            delimiter = '"' + hashes
            end = source.find(delimiter, index + len(raw.group(0)))
            end = length if end < 0 else end + len(delimiter)
            for position in range(index, end):
                output[position] = "\n" if chars[position] == "\n" else " "
            index = end
            continue
        if chars[index] == "'" and re.match(r"'(?:\\.|[^\\'\n])'", source[index:]) is None:
            # Rust lifetime, not a character literal.
            index += 1
            continue
        if chars[index] in ('"', "'"):
            quote = chars[index]
            output[index] = " "
            index += 1
            escaped = False
            while index < length:
                current = chars[index]
                output[index] = "\n" if current == "\n" else " "
                index += 1
                if escaped:
                    escaped = False
                elif current == "\\":
                    escaped = True
                elif current == quote:
                    break
            continue
        index += 1
    return "".join(output)


def macro_end(masked: str, opening: int) -> int:
    pairs = {"(": ")", "[": "]", "{": "}"}
    stack = [pairs[masked[opening]]]
    index = opening + 1
    while index < len(masked):
        current = masked[index]
        if current in pairs:
            stack.append(pairs[current])
        elif current == stack[-1]:
            stack.pop()
            if not stack:
                return index
        index += 1
    return len(masked)


def scan_source(source: str, label: str) -> list[str]:
    masked = mask_literals_and_comments(source)
    failures: list[str] = []
    for match in MACRO.finditer(masked):
        name = match.group("name")
        start = match.start()
        end = macro_end(masked, match.end() - 1)
        expression = masked[match.end():end]
        line = source.count("\n", 0, start) + 1
        if name == "dbg":
            failures.append(f"{label}:{line}: dbg! is forbidden")
            continue
        identifiers = set(FIELD.findall(expression))
        identifiers.update(CAPTURE.findall(expression))
        # Positional format arguments are valid in every supported logging
        # macro, not only println!/eprintln!. Strings/comments are already
        # blanked, so identifier scanning here sees expressions only.
        positional_expression = RUST_SAFE_METADATA.sub("", expression)
        identifiers.update(IDENTIFIER.findall(positional_expression))
        unsafe = sorted(identifier for identifier in identifiers if sensitive(identifier))
        if unsafe:
            failures.append(
                f"{label}:{line}: {name}! exposes sensitive field/expression "
                + ", ".join(unsafe)
            )
    for match in RUST_OUTPUT_CALL.finditer(masked):
        end = macro_end(masked, match.end() - 1)
        expression = RUST_SAFE_METADATA.sub("", masked[match.end():end])
        unsafe = sorted(
            identifier
            for identifier in set(IDENTIFIER.findall(expression))
            if sensitive(identifier)
        )
        if unsafe:
            line = source.count("\n", 0, match.start()) + 1
            failures.append(
                f"{label}:{line}: {match.group('name')} exposes sensitive expression "
                + ", ".join(unsafe)
            )
    for match in INSTRUMENT.finditer(masked):
        attributes = match.group(1)
        identifiers = set(FIELD.findall(attributes))
        identifiers.update(CAPTURE.findall(attributes))
        unsafe = sorted(identifier for identifier in identifiers if sensitive(identifier))
        line = source.count("\n", 0, match.start()) + 1
        if unsafe:
            failures.append(
                f"{label}:{line}: #[instrument] exposes sensitive explicit field "
                + ", ".join(unsafe)
            )
        if "skip_all" in attributes:
            continue
        failures.append(
            f"{label}:{line}: #[instrument] must use skip_all so arguments cannot leak"
        )
    return failures


def scan(path: pathlib.Path) -> list[str]:
    return scan_source(path.read_text(encoding="utf-8"), str(path.relative_to(ROOT)))


def scan_python_source(source: str, label: str) -> list[str]:
    tree = ast.parse(source, filename=label)
    failures: list[str] = []
    stream_aliases: set[str] = set()
    os_write_aliases: set[str] = set()
    for node in ast.walk(tree):
        if not isinstance(node, ast.ImportFrom):
            continue
        if node.module == "sys":
            stream_aliases.update(
                imported.asname or imported.name
                for imported in node.names
                if imported.name in {"stdout", "stderr", "__stdout__", "__stderr__"}
            )
        elif node.module == "os":
            os_write_aliases.update(
                imported.asname or imported.name
                for imported in node.names
                if imported.name in {"write", "writev"}
            )

    def attribute_path(expression: ast.AST) -> list[str] | None:
        parts: list[str] = []
        while isinstance(expression, ast.Attribute):
            parts.append(expression.attr)
            expression = expression.value
        if not isinstance(expression, ast.Name):
            return None
        parts.append(expression.id)
        return list(reversed(parts))

    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        is_output = isinstance(node.func, ast.Name) and node.func.id in {"print", "pprint"}
        is_log = isinstance(node.func, ast.Attribute) and node.func.attr in PYTHON_LOG_METHODS
        receiver = (
            attribute_path(node.func.value)
            if isinstance(node.func, ast.Attribute)
            else None
        )
        is_stream_write = (
            isinstance(node.func, ast.Attribute)
            and node.func.attr in {"write", "writelines"}
            and (
                (receiver is not None and receiver[:2] in (["sys", "stdout"], ["sys", "stderr"]))
                or (
                    receiver is not None
                    and receiver[:2] in (["sys", "__stdout__"], ["sys", "__stderr__"])
                )
                or (receiver is not None and len(receiver) == 1 and receiver[0] in stream_aliases)
            )
        )
        is_os_write = (
            isinstance(node.func, ast.Attribute)
            and node.func.attr in {"write", "writev"}
            and receiver == ["os"]
        ) or (isinstance(node.func, ast.Name) and node.func.id in os_write_aliases)
        if not (is_output or is_log or is_stream_write or is_os_write):
            continue
        identifiers: set[str] = set()

        def collect(expression: ast.AST) -> None:
            # Length is safe metadata even when computed from private content.
            if (
                isinstance(expression, ast.Call)
                and isinstance(expression.func, ast.Name)
                and expression.func.id == "len"
            ):
                return
            if isinstance(expression, ast.Name):
                identifiers.add(expression.id)
            elif isinstance(expression, ast.Attribute):
                identifiers.add(expression.attr)
            for child in ast.iter_child_nodes(expression):
                collect(child)

        for argument in node.args:
            collect(argument)
        for keyword in node.keywords:
            collect(keyword.value)
        unsafe = sorted(identifier for identifier in identifiers if sensitive(identifier))
        if unsafe:
            failures.append(
                f"{label}:{node.lineno}: output/log call exposes sensitive "
                + ", ".join(unsafe)
            )
    return failures


def scan_python(path: pathlib.Path) -> list[str]:
    return scan_python_source(
        path.read_text(encoding="utf-8"), str(path.relative_to(ROOT))
    )


def shell_dump_command(line: str) -> str | None:
    """Return a direct environment-dump command found in one logical line."""

    for match in SHELL_DUMP_COMMAND.finditer(line):
        command = match.group("command")
        arguments = match.group("arguments")
        before_redirection = SHELL_REDIRECTION.split(arguments, maxsplit=1)[0].strip()
        try:
            words = shlex.split(before_redirection, posix=True)
        except ValueError:
            # `sh -n` reports malformed quoting; do not guess at its tokenization here.
            continue

        if command == "printenv":
            # With or without names, printenv exists only to emit environment values.
            return command
        if command == "set" and not words:
            return command
        if command == "export" and (not words or words[0] == "-p"):
            return command
        if command != "env":
            continue

        # `env` emits the resulting environment when no utility remains after
        # its options and NAME=VALUE operands. Keep the ordinary
        # `env NAME=VALUE utility ...` execution form available.
        index = 0
        while index < len(words):
            word = words[index]
            if SHELL_ASSIGNMENT.match(word):
                index += 1
            elif word in {"-u", "--unset", "-C", "--chdir", "-S", "--split-string"}:
                index += 2
            elif word.startswith(("--unset=", "--chdir=", "--split-string=")):
                index += 1
            elif word.startswith("-"):
                index += 1
            else:
                break
        if index >= len(words):
            return command
    return None


def scan_shell_source(source: str, label: str) -> list[str]:
    failures: list[str] = []
    logical_lines: list[tuple[int, str]] = []
    pending = ""
    start = 1
    for number, physical in enumerate(source.splitlines(), 1):
        if not pending:
            start = number
        stripped = physical.rstrip()
        if stripped.endswith("\\"):
            pending += stripped[:-1] + " "
            continue
        logical_lines.append((start, pending + physical))
        pending = ""
    if pending:
        logical_lines.append((start, pending))

    heredoc_end: str | None = None
    for number, line in logical_lines:
        if heredoc_end is not None:
            if line.strip() == heredoc_end:
                heredoc_end = None
                continue
            unsafe = sorted(
                identifier
                for identifier in SHELL_VARIABLE.findall(line)
                if sensitive(identifier)
            )
            if unsafe:
                failures.append(
                    f"{label}:{number}: shell heredoc exposes sensitive "
                    + ", ".join(unsafe)
                )
            continue
        dump_command = shell_dump_command(line)
        if dump_command is not None:
            failures.append(
                f"{label}:{number}: shell environment dump via {dump_command} is forbidden"
            )
            continue
        if re.search(r"(?:^|[;&|]\s*)set\s+-[^\n#]*x", line):
            failures.append(f"{label}:{number}: shell xtrace is forbidden")
            continue
        if not SHELL_OUTPUT.search(line):
            continue
        heredoc = SHELL_HEREDOC.search(line)
        if heredoc:
            heredoc_end = heredoc.group(2)
        unsafe = sorted(
            identifier for identifier in SHELL_VARIABLE.findall(line) if sensitive(identifier)
        )
        if unsafe:
            failures.append(
                f"{label}:{number}: shell output exposes sensitive "
                + ", ".join(unsafe)
            )
    return failures


def scan_shell(path: pathlib.Path) -> list[str]:
    return scan_shell_source(
        path.read_text(encoding="utf-8"), str(path.relative_to(ROOT))
    )


def self_test() -> None:
    safe = '''
        tracing::warn!(body_status = 413, image_bytes_len = 12, "request body rejected");
        tracing::debug!("image bytes: {}", image_bytes.len());
        std::io::Write::write_vectored(
            &mut std::io::stderr(),
            &[std::io::IoSlice::new(image_bytes_len.as_bytes())],
        );
        std::io::stderr().write_vectored(
            &[std::io::IoSlice::new(response_body_len.as_bytes())],
        );
        #[tracing::instrument(skip_all)]
        fn qualified(system_prompt: &str) {}
    '''
    unsafe = '''
        tracing::warn!(body = %body, ?system_prompt, "provider failed");
        tracing::debug!("request: {:?}", request_body);
        tracing::info!("{}", prompt);
        log::warn!("{:?}", image_bytes);
        tracing::event!(tracing::Level::DEBUG, response_body = %response_body);
        tracing::span!(tracing::Level::DEBUG, "provider", request_body = ?request_body);
        log::log!(log::Level::Debug, "{:?}", response_body);
        tracing::debug! { response_body = %response_body }
        println!["{}", response_body];
        print!("{}", response_body);
        eprint!("{}", image_bytes);
        writeln!(std::io::stderr(), "{}", request_body);
        panic!("{}", response_body);
        std::io::Write::write_all(&mut std::io::stderr(), response_body.as_bytes());
        std::io::stderr().write_all(response_body.as_bytes());
        std::io::Write::write_vectored(
            &mut std::io::stderr(),
            &[std::io::IoSlice::new(response_body.as_bytes())],
        );
        std::io::stderr().write_vectored(
            &[std::io::IoSlice::new(image_bytes.as_slice())],
        );
        println!("transcript: {}", reply_transcript);
        #[instrument]
        fn leaks(api_key: &str) {}
        #[tracing::instrument(skip_all, fields(response_body = %response_body))]
        fn explicit_field_leaks(response_body: &str) {}
    '''
    if scan_source(safe, "safe-fixture"):
        raise SystemExit("privacy logging gate rejected its safe fixture")
    failures = scan_source(unsafe, "unsafe-fixture")
    if len(failures) != 20:
        raise SystemExit(
            f"privacy logging gate missed an unsafe fixture: expected 20 failures, got {failures}"
        )
    python_safe = (
        'print({"vision_chars": len(vision_text), "body_status": 413})\n'
        'sys.__stderr__.write(str(len(response_body)))'
    )
    python_unsafe = (
        'from sys import stderr\n'
        'from os import write\n'
        'print(response_body)\n'
        'logger.debug("provider failed", extra={"prompt": prompt})\n'
        'sys.stderr.write(response_body)\n'
        'sys.stdout.write(prompt)\n'
        'sys.stderr.buffer.write(image_bytes)\n'
        'os.write(2, response_body)\n'
        'stderr.write(response_body)\n'
        'sys.stderr.writelines([response_body])\n'
        'write(2, image_bytes)\n'
        'logger.warn(response_body)\n'
        'warnings.warn(response_body)\n'
        'pprint.pprint(response_body)\n'
        'pprint.pp(response_body)\n'
        'sys.stderr.buffer.raw.write(image_bytes)\n'
        'os.writev(2, [image_bytes])\n'
        'sys.__stderr__.write(response_body)\n'
        'sys.__stdout__.buffer.write(image_bytes)\n'
        'sys.__stderr__.buffer.raw.writelines([private_context])'
    )
    if scan_python_source(python_safe, "safe-python-fixture"):
        raise SystemExit("privacy logging gate rejected its safe Python fixture")
    python_failures = scan_python_source(python_unsafe, "unsafe-python-fixture")
    if len(python_failures) != 18:
        raise SystemExit(
            "privacy logging gate missed an unsafe Python fixture: "
            f"expected 18 failures, got {python_failures}"
        )
    shell_safe = "printf '%s\\n' \\\n+  \"$IMAGE_BYTES_LEN\""
    shell_unsafe = "printf '%s\\n' \\\n+  \"$RESPONSE_BODY\""
    if scan_shell_source(shell_safe, "safe-shell-fixture"):
        raise SystemExit("privacy logging gate rejected its safe shell fixture")
    shell_failures = scan_shell_source(shell_unsafe, "unsafe-shell-fixture")
    if len(shell_failures) != 1:
        raise SystemExit(
            "privacy logging gate missed an unsafe shell fixture: "
            f"expected 1 failure, got {shell_failures}"
        )
    for source in (
        'logger "$RESPONSE_BODY"',
        'tee /tmp/debug <<< "$REQUEST_BODY"',
        'set -x\ncurl -H "Authorization: Bearer $DISCORD_TOKEN" localhost',
        'logger <<EOF\n$RESPONSE_BODY\nEOF',
        'cat <<< "$RESPONSE_BODY"',
        "env",
        "printenv",
        "set",
        "printenv DISCORD_TOKEN",
        "env > /tmp/environment.debug",
        "env -0 > /tmp/environment.debug",
        "env ABBEY_TEST=1 | logger",
        "set | logger",
        "true; set > /tmp/shell.debug",
        "export -p",
        "export -p DISCORD_TOKEN | logger",
    ):
        if not scan_shell_source(source, "unsafe-shell-fixture"):
            raise SystemExit(
                "privacy logging gate missed a standard shell output or tracing path"
            )


def main() -> None:
    self_test()
    failures: list[str] = []
    for path in sorted(SOURCE.rglob("*.rs")):
        failures.extend(scan(path))
    for directory in (ROOT / "deploy", ROOT / "scripts"):
        for path in sorted(directory.glob("*.py")):
            failures.extend(scan_python(path))
        for path in sorted(directory.glob("*.sh")):
            failures.extend(scan_shell(path))
    if failures:
        print("privacy logging gate failed:", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        print(
            "Log fixed categories and safe metadata (*_len, *_count, *_kind, *_status, *_pass, *_hash) only.",
            file=sys.stderr,
        )
        raise SystemExit(1)
    print(
        "privacy logging gate: no credential, prompt, transcript, request/response-body, "
        "or image expressions"
    )


if __name__ == "__main__":
    main()
