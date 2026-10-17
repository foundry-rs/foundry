import importlib.util
from pathlib import Path
import unittest
import xml.etree.ElementTree as ET


SCRIPT = Path(__file__).parents[1] / "test-timings.py"
SPEC = importlib.util.spec_from_file_location("test_timings", SCRIPT)
TIMINGS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(TIMINGS)


class TestTimingsTests(unittest.TestCase):
    def test_ranks_all_attempts_and_excludes_skipped_tests(self):
        report = ET.fromstring('''
            <testsuites><testsuite name="forge::cli">
                <testcase name="fast" time="0.5"/>
                <testcase name="slow" time="3"/>
                <testcase name="flaky" time="2">
                    <flakyFailure time="4"/><flakyError time="1"/>
                </testcase>
                <testcase name="failed" time="1">
                    <failure/><rerunFailure time="2"/><rerunError time="3"/>
                </testcase>
                <testcase name="skipped" time="100"><skipped/></testcase>
            </testsuite></testsuites>
        ''')
        self.assertEqual(TIMINGS.slowest_tests(report), [
            (7.0, "forge::cli", "flaky", 3, "flaky"),
            (6.0, "forge::cli", "failed", 3, "failed"),
            (3.0, "forge::cli", "slow", 1, "passed"),
            (0.5, "forge::cli", "fast", 1, "passed"),
        ])

    def test_formats_limits_and_escapes_the_table(self):
        report = ET.fromstring('''
            <testsuites><testsuite name="forge|cli">
                <testcase name="&lt;slow&gt;" time="2.125"/>
                <testcase name="fast"/>
            </testsuite></testsuites>
        ''')
        self.assertEqual(TIMINGS.summarize(report, limit=1), """### Slowest tests

Durations include failed attempts, but exclude retry delays and compilation. Tests run concurrently; these durations do not add up to job wall time.

| Time (s) | Binary | Test | Attempts | Result |
| ---: | --- | --- | ---: | --- |
| 2.125 | forge&#124;cli | &lt;slow&gt; | 1 | passed |
""")

    def test_handles_empty_reports_missing_times_and_errors(self):
        self.assertEqual(TIMINGS.slowest_tests(ET.fromstring("<testsuites/>")), [])
        report = ET.fromstring('''
            <testsuite name="tests"><testcase name="error"><error/></testcase></testsuite>
        ''')
        self.assertEqual(TIMINGS.slowest_tests(report), [(0.0, "tests", "error", 1, "failed")])


if __name__ == "__main__":
    unittest.main()
