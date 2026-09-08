#!/usr/bin/env python3
"""Validate exact-version-only cooldown policy, independent of TOML layout."""

import re
import subprocess
import sys
import tomllib
from pathlib import Path


def validate_policy(policy):
    if set(policy) != {"allow"} or not isinstance(policy["allow"], dict):
        raise ValueError("cooldown.toml permits only exact-version allow rules")
    allow = policy["allow"]
    if set(allow) != {"exact"} or not isinstance(allow["exact"], list):
        raise ValueError("allow must contain only an exact array")
    seen = set()
    for rule in allow["exact"]:
        if not isinstance(rule, dict) or set(rule) != {"crate", "version"}:
            raise ValueError("each exact rule must contain only crate and version")
        crate, version = rule["crate"], rule["version"]
        if not isinstance(crate, str) or not re.fullmatch(r"[A-Za-z0-9_-]+", crate):
            raise ValueError("crate must be an exact package name")
        if not isinstance(version, str) or not re.fullmatch(r"[A-Za-z0-9.+_-]+", version):
            raise ValueError("version must be an exact version")
        key = (crate, version)
        if key in seen:
            raise ValueError(f"duplicate exact rule for {crate}@{version}")
        seen.add(key)


def main():
    path = Path("cooldown.toml")
    if path.is_symlink() or not path.is_file():
        raise ValueError("cooldown.toml must be a regular file")
    subprocess.run(["git", "ls-files", "--error-unmatch", str(path)], check=True,
                   stdout=subprocess.DEVNULL)
    subprocess.run(["git", "diff", "--exit-code", "HEAD", "--", str(path)], check=True)
    with path.open("rb") as config:
        validate_policy(tomllib.load(config))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"::error file=cooldown.toml::{error}", file=sys.stderr)
        sys.exit(1)
