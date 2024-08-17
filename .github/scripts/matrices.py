#!/usr/bin/env python3

import json
import os


# A runner target
class Target:
    # GHA runner
    runner_label: str
    # Rust target triple
    target: str
    # SVM Solc target
    svm_target_platform: str

    def __init__(self, runner_label: str, target: str, svm_target_platform: str):
        self.runner_label = runner_label
        self.target = target
        self.svm_target_platform = svm_target_platform


# A single test suite to run.
class Case:
    # Name of the test suite.
    name: str
    # Nextest filter expression.
    filter: str
    # Number of partitions to split the test suite into.
    n_partitions: int
    # Whether to run on non-Linux platforms for PRs. All platforms and tests are run on pushes.
    pr_cross_platform: bool

    def __init__(
        self, name: str, filter: str, n_partitions: int, pr_cross_platform: bool
    ):
        self.name = name
        self.filter = filter
        self.n_partitions = n_partitions
        self.pr_cross_platform = pr_cross_platform


# GHA matrix entry
class Expanded:
    name: str
    runner_label: str
    target: str
    svm_target_platform: str
    flags: str
    partition: int

    def __init__(
        self,
        name: str,
        runner_label: str,
        target: str,
        svm_target_platform: str,
        flags: str,
        partition: int,
    ):
        self.name = name
        self.runner_label = runner_label
        self.target = target
        self.svm_target_platform = svm_target_platform
        self.flags = flags
        self.partition = partition


profile = os.environ.get("PROFILE")
is_pr = os.environ.get("EVENT_NAME") == "pull_request"
t_linux_x86 = Target("ubuntu-latest", "x86_64-unknown-linux-gnu", "linux-amd64")
# TODO: Figure out how to make this work
# t_linux_arm = Target("ubuntu-latest", "aarch64-unknown-linux-gnu", "linux-aarch64")
t_macos = Target("macos-latest", "aarch64-apple-darwin", "macosx-aarch64")
t_windows = Target("windows-latest", "x86_64-pc-windows-msvc", "windows-amd64")
targets = [t_linux_x86, t_windows] if is_pr else [t_linux_x86, t_macos, t_windows]

config = [
    Case(
        name="unit",
        filter="!kind(test)",
        n_partitions=1,
        pr_cross_platform=True,
    ),
    Case(
        name="integration",
        filter="kind(test) & !test(/issue|forge_std|ext_integration/)",
        n_partitions=3,
        pr_cross_platform=True,
    ),
    Case(
        name="integration / issue-repros",
        filter="package(=forge) & test(~issue)",
        n_partitions=2,
        pr_cross_platform=False,
    ),
    Case(
        name="integration / external",
        filter="package(=forge) & test(~ext_integration)",
        n_partitions=2,
        pr_cross_platform=False,
    ),
]


def main():
    expanded = []
    for target in targets:
        for case in config:
            if is_pr and (not case.pr_cross_platform and target != t_linux_x86):
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
                
                if profile == "isolate":
                    flags += " --features=isolate-by-default"
                name += os_str

                obj = Expanded(
                    name=name,
                    runner_label=target.runner_label,
                    target=target.target,
                    svm_target_platform=target.svm_target_platform,
                    flags=flags,
                    partition=partition,
                )
                expanded.append(vars(obj))

    print_json({"include": expanded})


def print_json(obj):
    print(json.dumps(obj), end="", flush=True)


if __name__ == "__main__":
    main()
