#!/usr/bin/env python3

import json
import os


class Target:
    # GitHub runner OS
    os_id: str
    # Rust target triple
    target: str

    def __init__(self, os_id: str, target: str):
        self.os_id = os_id
        self.target = target


class Case:
    name: str
    filter: str
    n_partitions: int
    xplatform: bool

    def __init__(self, name: str, filter: str, n_partitions: int, xplatform: bool):
        self.name = name
        self.filter = filter
        self.n_partitions = n_partitions
        self.xplatform = xplatform


class Expanded:
    os: str
    target: str
    name: str
    flags: str
    partition: int

    def __init__(self, os: str, target: str, name: str, flags: str, partition: int):
        self.os = os
        self.target = target
        self.name = name
        self.flags = flags
        self.partition = partition


default_target = Target("ubuntu-latest", "x86_64-unknown-linux-gnu")
if os.environ.get("EVENT_NAME") == "pull_request":
    targets = [
        default_target,
        Target("windows-latest", "x86_64-pc-windows-msvc"),
    ]
else:
    targets = [
        default_target,
        Target("ubuntu-latest", "aarch64-unknown-linux-gnu"),
        Target("macos-latest", "x86_64-apple-darwin"),
        Target("macos-latest", "aarch64-apple-darwin"),
        Target("windows-latest", "x86_64-pc-windows-msvc"),
    ]

config = [
    Case(
        name="unit",
        filter="kind(lib) | kind(bench) | kind(proc-macro)",
        n_partitions=1,
        xplatform=True,
    ),
    Case(
        name="integration",
        filter="kind(test) & !test(/issue|forge_std|ext_integration/)",
        n_partitions=3,
        xplatform=True,
    ),
    Case(
        name="integration/issue-repros",
        filter="package(=forge) & test(~issue)",
        n_partitions=2,
        xplatform=False,
    ),
    Case(
        name="integration/forge-std",
        filter="package(=forge) & test(~forge_std)",
        n_partitions=1,
        xplatform=False,
    ),
    Case(
        name="integration/external",
        filter="package(=forge) & test(~ext_integration)",
        n_partitions=2,
        xplatform=False,
    ),
]


def build_matrix():
    expanded = []
    for target in targets:
        expanded.append({"os": target.os_id, "target": target.target})
    print_json({"include": expanded})


def test_matrix():
    expanded = []
    for target in targets:
        for case in config:
            if not case.xplatform and target != default_target:
                continue

            for partition in range(1, case.n_partitions + 1):
                os_str = ""
                if len(targets) > 1:
                    os_str = f" ({target.target})"

                name = case.name
                flags = f"-E '{case.filter}'"
                if case.n_partitions > 1:
                    s = f"{partition}/{case.n_partitions}"
                    name += f" ({s})"
                    flags += f" --partition count:{s}"
                name += os_str

                obj = Expanded(
                    os=target.os_id,
                    target=target.target,
                    name=name,
                    flags=flags,
                    partition=partition,
                )
                expanded.append(vars(obj))

    print_json({"include": expanded})


def print_json(obj):
    print(json.dumps(obj), end="", flush=True)


if __name__ == "__main__":
    if int(os.environ.get("TEST", "0")) == 0:
        build_matrix()
    else:
        test_matrix()
