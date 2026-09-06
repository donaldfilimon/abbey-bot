#!/usr/bin/env python3
"""Fixed-output entry point; the complete sibling helper bundle is required."""
import sys
from pathlib import Path

# Select reviewed sibling modules ahead of caller PYTHONPATH and cwd.
sys.path.insert(0, str(Path(__file__).resolve().parent))
try:
    from service_readiness import main
except Exception:
    print("readiness: bundle")
    raise SystemExit(1) from None

if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
