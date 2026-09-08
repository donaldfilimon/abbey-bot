#!/usr/bin/env python3
"""Read-only Activity Portal URL-map contract + local static asset checks.

Bot tokens cannot set Developer Portal URL mappings. This script encodes the
canonical PREFIX/TARGET contract from docs/activities.md, verifies the local
Pages-served Activity shell files exist, and prints PASS/FAIL lines. It never
claims the Portal mapping is live — Donald must still click Portal and confirm
the discordsays iframe.

Usage (from repo root):

    python3 deploy/check-activity-url-map.py
"""

from __future__ import annotations

import pathlib
import sys
from dataclasses import dataclass


ROOT = pathlib.Path(__file__).resolve().parent.parent

# Canonical Portal URL mapping (Developer Portal → Activities → URL Mappings).
CANONICAL_PREFIX = "/"
CANONICAL_TARGET = "donaldfilimon.github.io/abbey-bot/activity"

# Browser-check URL for GitHub Pages (distinct from the Discord iframe origin).
PUBLIC_PAGES_URL = "https://donaldfilimon.github.io/abbey-bot/activity/"

# Discord embeds the Activity under the application proxy, not PUBLIC_PAGES_URL.
APPLICATION_ID = "1147940171099152464"
DISCORDSAYS_IFRAME_ORIGIN = f"https://{APPLICATION_ID}.discordsays.com/"

# Static files Pages must serve for the Activity shell (tracked paths).
REQUIRED_ACTIVITY_ASSETS = (
    "activity/index.html",
    "activity/app.js",
)

# Local Pages-shell UX markers in activity/app.js (no network; not Portal proof).
REQUIRED_APP_JS_MARKERS = (
    "Pages shell only",
    "No Discord parent (plain browser)",
    "does not prove Developer Portal URL mapping",
    "ready() timeout",
    "claims Portal is done",
)

# Post-Portal verify language that must stay in docs/activities.md (local only;
# never claims Portal is live).
REQUIRED_ACTIVITIES_DOC_MARKERS = (
    "After Donald clicks Portal",
    "discordsays iframe",
    "checker never sees Portal state",
    "operator gate",
    "Pages shell only",
)


@dataclass(frozen=True)
class CheckResult:
    name: str
    ok: bool
    detail: str

    def line(self) -> str:
        status = "PASS" if self.ok else "FAIL"
        return f"{status}: {self.name} — {self.detail}"


def normalize_prefix(value: str) -> str:
    """Strip surrounding whitespace only; PREFIX is otherwise literal."""
    return value.strip()


def normalize_target(value: str) -> str:
    """Strip surrounding whitespace only; TARGET is otherwise literal."""
    return value.strip()


def validate_prefix(value: str) -> list[str]:
    """Return human-readable rejection reasons; empty means PREFIX is canonical."""
    prefix = normalize_prefix(value)
    reasons: list[str] = []
    if prefix != CANONICAL_PREFIX:
        reasons.append(
            f"PREFIX must be exactly {CANONICAL_PREFIX!r} (got {prefix!r})"
        )
    return reasons


def validate_target(value: str) -> list[str]:
    """Return human-readable rejection reasons; empty means TARGET is canonical.

    Rejects common Portal misconfigs documented in activities.md:
    scheme prefix, index.html, wrong path, trailing-slash directory form.
    """
    target = normalize_target(value)
    reasons: list[str] = []
    lowered = target.lower()

    if "://" in target or lowered.startswith("https:") or lowered.startswith("http:"):
        reasons.append("TARGET must not include a URL scheme (no https://)")
    if target.endswith("/") or target.endswith("\\"):
        reasons.append(
            "TARGET must be directory form without a trailing slash "
            f"(canonical: {CANONICAL_TARGET!r})"
        )
    if lowered.endswith("index.html") or "/index.html" in lowered:
        reasons.append("TARGET must be a directory, not index.html")
    if target != CANONICAL_TARGET:
        # Keep a specific wrong-path note when scheme/slash/index already fired,
        # but always surface the canonical expectation when values differ.
        if not reasons:
            reasons.append(
                f"TARGET must be exactly {CANONICAL_TARGET!r} (got {target!r})"
            )
        elif target.rstrip("/") != CANONICAL_TARGET and not (
            lowered.endswith("index.html") or "/index.html" in lowered
        ):
            reasons.append(
                f"TARGET path must be {CANONICAL_TARGET!r} (got {target!r})"
            )
        elif (
            lowered.endswith("index.html") or "/index.html" in lowered
        ) and target.replace("/index.html", "").replace("index.html", "").rstrip(
            "/"
        ) != CANONICAL_TARGET and "://" not in target:
            # index.html on a wrong host/path
            base = target
            for suffix in ("/index.html", "index.html"):
                if base.lower().endswith(suffix):
                    base = base[: -len(suffix)]
                    break
            base = base.rstrip("/")
            if base != CANONICAL_TARGET:
                reasons.append(
                    f"TARGET path must be {CANONICAL_TARGET!r} (got {target!r})"
                )
    return reasons


def validate_mapping(prefix: str, target: str) -> list[str]:
    """Validate a PREFIX+TARGET pair against the canonical contract."""
    return validate_prefix(prefix) + validate_target(target)


def is_canonical_mapping(prefix: str, target: str) -> bool:
    return not validate_mapping(prefix, target)


def public_pages_url() -> str:
    """GitHub Pages browser-check URL (trailing slash; includes https://)."""
    return PUBLIC_PAGES_URL


def discordsays_iframe_origin() -> str:
    """Origin Discord uses for the Activity iframe after Portal mapping."""
    return DISCORDSAYS_IFRAME_ORIGIN


def check_required_assets(root: pathlib.Path) -> list[CheckResult]:
    """Verify Activity client files exist on disk under root."""
    results: list[CheckResult] = []
    activity_dir = root / "activity"
    results.append(
        CheckResult(
            name="activity directory",
            ok=activity_dir.is_dir(),
            detail=str(activity_dir.relative_to(root))
            if activity_dir.is_dir()
            else "missing activity/",
        )
    )
    for relative in REQUIRED_ACTIVITY_ASSETS:
        path = root.joinpath(*relative.split("/"))
        ok = path.is_file() and path.stat().st_size > 0
        results.append(
            CheckResult(
                name=f"asset {relative}",
                ok=ok,
                detail="present" if ok else "missing or empty",
            )
        )
    return results


def check_pages_markdown_inventory(root: pathlib.Path) -> CheckResult:
    """Ensure Activity README markdown that Pages may index remains present.

    The Liquid gate scans publishable Markdown. activity/README.md documents the
    public Pages URL and must stay tracked so operators and Pages inventory stay
    aligned. This does not prove Portal mapping.
    """
    relative = "activity/README.md"
    path = root.joinpath(*relative.split("/"))
    ok = path.is_file() and path.stat().st_size > 0
    return CheckResult(
        name="pages markdown inventory",
        ok=ok,
        detail=f"{relative} present" if ok else f"{relative} missing or empty",
    )


def check_contract_constants() -> list[CheckResult]:
    """Self-check that module constants match the documented contract."""
    results = [
        CheckResult(
            name="canonical PREFIX constant",
            ok=CANONICAL_PREFIX == "/",
            detail=repr(CANONICAL_PREFIX),
        ),
        CheckResult(
            name="canonical TARGET constant",
            ok=CANONICAL_TARGET == "donaldfilimon.github.io/abbey-bot/activity",
            detail=repr(CANONICAL_TARGET),
        ),
        CheckResult(
            name="public Pages URL constant",
            ok=PUBLIC_PAGES_URL
            == "https://donaldfilimon.github.io/abbey-bot/activity/",
            detail=PUBLIC_PAGES_URL,
        ),
        CheckResult(
            name="mapping self-check",
            ok=is_canonical_mapping(CANONICAL_PREFIX, CANONICAL_TARGET),
            detail="PREFIX+TARGET validate cleanly",
        ),
        CheckResult(
            name="Pages URL distinct from discordsays iframe",
            ok=PUBLIC_PAGES_URL != DISCORDSAYS_IFRAME_ORIGIN
            and DISCORDSAYS_IFRAME_ORIGIN.startswith("https://")
            and APPLICATION_ID in DISCORDSAYS_IFRAME_ORIGIN,
            detail=(
                f"browser={PUBLIC_PAGES_URL} iframe={DISCORDSAYS_IFRAME_ORIGIN}"
            ),
        ),
    ]
    return results


def check_activity_client_copy_markers(root: pathlib.Path) -> list[CheckResult]:
    """Require truthful plain-browser / ready()-timeout copy in activity/app.js.

    Operators open the Pages URL as a smoke check. The client must distinguish
    that smoke from Discord iframe ready(), and must never claim Portal mapping
    is done from a plain browser tab. This is a local string contract only.
    """
    relative = "activity/app.js"
    path = root.joinpath(*relative.split("/"))
    results: list[CheckResult] = []
    if not path.is_file():
        results.append(
            CheckResult(
                name="activity client copy markers",
                ok=False,
                detail=f"{relative} missing",
            )
        )
        return results

    try:
        body = path.read_text(encoding="utf-8")
    except OSError as exc:
        results.append(
            CheckResult(
                name="activity client copy markers",
                ok=False,
                detail=f"unreadable: {exc}",
            )
        )
        return results

    missing = [marker for marker in REQUIRED_APP_JS_MARKERS if marker not in body]
    # Guard against accidental "Portal is done" / "mapping complete" claims.
    forbidden = (
        "Portal is done",
        "Portal mapping is live",
        "URL mapping complete",
        "Portal confirmed",
    )
    # The required marker includes the negative phrase "claims Portal is done";
    # only flag bare positive claims that are not part of that negation.
    hits = []
    for phrase in forbidden:
        if phrase == "Portal is done":
            # allow the required negation "...claims Portal is done"
            if "claims Portal is done" in body:
                # strip that occurrence for a crude positive-claim scan
                scan = body.replace("claims Portal is done", "")
            else:
                scan = body
            if phrase in scan:
                hits.append(phrase)
        elif phrase in body:
            hits.append(phrase)

    if missing:
        results.append(
            CheckResult(
                name="activity client copy markers",
                ok=False,
                detail="missing: " + ", ".join(missing),
            )
        )
    elif hits:
        results.append(
            CheckResult(
                name="activity client copy markers",
                ok=False,
                detail="forbidden Portal-done claim: " + ", ".join(hits),
            )
        )
    else:
        results.append(
            CheckResult(
                name="activity client copy markers",
                ok=True,
                detail=(
                    f"{len(REQUIRED_APP_JS_MARKERS)} plain-browser/ready-timeout "
                    "markers present; no Portal-done claim"
                ),
            )
        )
    return results



def check_activities_docs_verify_markers(root: pathlib.Path) -> list[CheckResult]:
    """Require post-Portal verify checklist language in docs/activities.md.

    Keeps operator-gated truth: these markers document how Donald confirms the
    iframe. The checker still cannot see Portal state.
    """
    relative = "docs/activities.md"
    path = root.joinpath(*relative.split("/"))
    results: list[CheckResult] = []
    if not path.is_file():
        results.append(
            CheckResult(
                name="activities docs verify markers",
                ok=False,
                detail=f"{relative} missing",
            )
        )
        return results
    try:
        body = path.read_text(encoding="utf-8")
    except OSError as exc:
        results.append(
            CheckResult(
                name="activities docs verify markers",
                ok=False,
                detail=f"unreadable: {exc}",
            )
        )
        return results
    missing = [m for m in REQUIRED_ACTIVITIES_DOC_MARKERS if m not in body]
    if missing:
        results.append(
            CheckResult(
                name="activities docs verify markers",
                ok=False,
                detail="missing: " + ", ".join(missing),
            )
        )
    else:
        results.append(
            CheckResult(
                name="activities docs verify markers",
                ok=True,
                detail=(
                    f"{len(REQUIRED_ACTIVITIES_DOC_MARKERS)} post-Portal verify "
                    "markers present; still operator-gated"
                ),
            )
        )
    return results

def run_checks(root: pathlib.Path | None = None) -> list[CheckResult]:
    base = ROOT if root is None else root
    results: list[CheckResult] = []
    results.extend(check_contract_constants())
    results.extend(check_required_assets(base))
    results.append(check_pages_markdown_inventory(base))
    results.extend(check_activity_client_copy_markers(base))
    results.extend(check_activities_docs_verify_markers(base))
    return results


def main(argv: list[str] | None = None) -> int:
    args = list(sys.argv[1:] if argv is None else argv)
    if args in (["-h"], ["--help"]):
        print(__doc__.strip())
        print()
        print("Canonical mapping (operator must enter in Portal):")
        print(f"  PREFIX: {CANONICAL_PREFIX}")
        print(f"  TARGET: {CANONICAL_TARGET}")
        print(f"  Pages browser check: {PUBLIC_PAGES_URL}")
        print(f"  Discord iframe origin: {DISCORDSAYS_IFRAME_ORIGIN}")
        print()
        print(
            "This checker cannot verify Portal state. After Donald saves the "
            "mapping, confirm ready() inside the discordsays iframe (not only "
            "in a plain browser tab)."
        )
        return 0

    if args:
        print("usage: python3 deploy/check-activity-url-map.py", file=sys.stderr)
        return 2

    results = run_checks()
    failed = 0
    for result in results:
        print(result.line())
        if not result.ok:
            failed += 1

    print()
    print(
        "NOTE: PASS lines only cover local assets + documented mapping "
        "constants. Portal URL Mappings remain operator-gated until Donald "
        f"confirms the iframe at {DISCORDSAYS_IFRAME_ORIGIN} shows Abbey ready."
    )
    if failed:
        print(f"check-activity-url-map: {failed} FAIL", file=sys.stderr)
        return 1
    print("check-activity-url-map: all local contract checks PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
