#!/usr/bin/env python3
"""Exit 0 when two JSON files are equal as ordered documents (key order included)."""
import json
import sys
from collections import OrderedDict


def load(path):
    with open(path, "rb") as handle:
        return json.loads(handle.read().decode("utf-8"), object_pairs_hook=lambda pairs: list(pairs))


def main():
    if len(sys.argv) != 3:
        print("usage: json_equal.py A B", file=sys.stderr)
        return 2
    a, b = load(sys.argv[1]), load(sys.argv[2])
    if a != b:
        print(f"JSON documents differ: {sys.argv[1]} vs {sys.argv[2]}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
