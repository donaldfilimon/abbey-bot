#!/usr/bin/env python3
"""Invoke with python3 -I to isolate interpreter startup and the helper bundle."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
try:
    from service_status import main
except Exception:
    print("Abbey service: current state unavailable or unverified.")
    raise SystemExit(1) from None

if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
