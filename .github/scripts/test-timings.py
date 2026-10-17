#!/usr/bin/env python3
"""Summarize nextest JUnit timings, including time spent in failed attempts."""

import argparse
import html
from pathlib import Path
import xml.etree.ElementTree as ET


def slowest_tests(report):
    rows = []
    for suite in report.iter("testsuite"):
        for case in suite.findall("testcase"):
            if case.find("skipped") is not None:
                continue
            retries = [
                child
                for child in case
                if child.tag in {"flakyFailure", "flakyError", "rerunFailure", "rerunError"}
            ]
            elapsed = float(case.get("time", "0")) + sum(
                float(retry.get("time", "0")) for retry in retries
            )
            failed = case.find("failure") is not None or case.find("error") is not None
            status = "failed" if failed else "flaky" if retries else "passed"
            rows.append(
                (elapsed, suite.get("name", ""), case.get("name", ""), len(retries) + 1, status)
            )
    return sorted(rows, key=lambda row: (-row[0], row[1], row[2]))


def table_cell(value):
    return html.escape(value).replace("|", "&#124;").replace("\n", " ").replace("\r", " ")


def summarize(report, limit=20):
    rows = slowest_tests(report)
    lines = [
        "### Slowest tests",
        "",
        "Durations include failed attempts, but exclude retry delays and compilation. "
        "Tests run concurrently; these durations do not add up to job wall time.",
        "",
        "| Time (s) | Binary | Test | Attempts | Result |",
        "| ---: | --- | --- | ---: | --- |",
    ]
    for elapsed, binary, name, attempts, status in rows[:limit]:
        lines.append(
            f"| {elapsed:.3f} | {table_cell(binary)} | {table_cell(name)} | {attempts} | {status} |"
        )
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    args = parser.parse_args()
    print(summarize(ET.parse(args.report).getroot()), end="")


if __name__ == "__main__":
    main()
