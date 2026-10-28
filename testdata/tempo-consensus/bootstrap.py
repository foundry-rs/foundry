#!/usr/bin/env python3
"""Fetch immutable upstream Solidity sources, retaining their notices and checking SHA-256."""

import argparse
import hashlib
import json
from pathlib import Path
import urllib.request


ROOT = Path(__file__).resolve().parent


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--offline", action="store_true", help="verify the existing cache without network access")
    args = parser.parse_args()
    sources = json.loads((ROOT / "sources.lock.json").read_text())
    for source in sources:
        destination = ROOT / source["path"]
        if destination.exists():
            data = destination.read_bytes()
        elif args.offline:
            raise SystemExit(f"missing cached source: {destination}")
        else:
            with urllib.request.urlopen(source["url"], timeout=60) as response:
                data = response.read()
        digest = hashlib.sha256(data).hexdigest()
        if digest != source["sha256"]:
            raise SystemExit(f"source checksum mismatch: {source['path']}")
        if not destination.exists():
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
    print(f"verified {len(sources)} immutable upstream sources")
