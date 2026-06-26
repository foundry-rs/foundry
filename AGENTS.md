# Local Agent Instructions

- In tests, prefer `snapbox` assertions for command output, JSON, and snapshot-like data. Do not add ad hoc `.contains(...)` assertions when `snapbox` can express the check.
