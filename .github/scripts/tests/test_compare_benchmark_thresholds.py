import json
import pathlib
import subprocess
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).parents[1] / "compare-benchmark-thresholds.py"


class CompareBenchmarkThresholdsTests(unittest.TestCase):
    def run_compare(self, base, candidate, rules, candidate_exists=True):
        with tempfile.TemporaryDirectory() as directory:
            directory = pathlib.Path(directory)
            base_path = directory / "base.json"
            candidate_path = directory / "candidate.json"
            base_path.write_text(json.dumps(base))
            if candidate_exists:
                candidate_path.write_text(json.dumps(candidate))
            config_path = directory / "thresholds.json"
            config_path.write_text(json.dumps({
                "schema_version": 1,
                "kind": "foundry-nightly-benchmark-thresholds",
                "benchmarks": rules,
            }))
            return subprocess.run(
                ["python3", str(SCRIPT), str(base_path), str(candidate_path), str(config_path)],
                text=True,
                capture_output=True,
                check=False,
            )

    @staticmethod
    def rule(warning, regression, alert=True):
        return {"warning_percent": warning, "regression_percent": regression, "alert": alert}

    def test_keys_use_different_thresholds_and_warning_does_not_alert(self):
        result = self.run_compare(
            {"strict": 100, "loose": 100}, {"strict": 103, "loose": 103},
            {"strict": self.rule(1, 2), "loose": self.rule(5, 10)},
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("+3.00% ❌ Regression", result.stdout)
        self.assertIn("+3.00% ⚪ Within threshold", result.stdout)

        warning = self.run_compare({"key": 100}, {"key": 106}, {"key": self.rule(5, 10)})
        self.assertEqual(warning.returncode, 0)
        self.assertIn("Warning", warning.stdout)

    def test_advisory_and_uncalibrated_never_alert(self):
        advisory = self.run_compare(
            {"coverage": 100}, {"coverage": 120}, {"coverage": self.rule(5, 10, False)}
        )
        self.assertEqual(advisory.returncode, 0)
        self.assertIn("Regression (advisory)", advisory.stdout)

        uncalibrated = self.run_compare({"symbolic": 1}, {"symbolic": 2}, {})
        self.assertEqual(uncalibrated.returncode, 0)
        self.assertIn("Uncalibrated", uncalibrated.stdout)

    def test_missing_nightly_alerts_but_missing_baseline_does_not(self):
        rules = {
            "alerting": self.rule(5, 10),
            "advisory": self.rule(5, 10, False),
        }
        missing_nightly = self.run_compare({"alerting": 1, "advisory": 1}, {}, rules)
        self.assertEqual(missing_nightly.returncode, 1)
        self.assertIn("❌ Missing nightly result", missing_nightly.stdout)
        self.assertIn("⚠️ Inconclusive (missing side)", missing_nightly.stdout)

        missing_nightly_file = self.run_compare({"alerting": 1}, {}, rules, False)
        self.assertEqual(missing_nightly_file.returncode, 1)
        self.assertIn("❌ Missing nightly result", missing_nightly_file.stdout)

        missing_baseline = self.run_compare({}, {"alerting": 1}, rules)
        self.assertEqual(missing_baseline.returncode, 0)
        self.assertIn("⚠️ Inconclusive (missing side)", missing_baseline.stdout)

    def test_malformed_config_and_input_exit_two(self):
        config = self.run_compare({"key": 1}, {"key": 2}, {"key": self.rule(10, 5)})
        self.assertEqual(config.returncode, 2)
        rules = {"key": self.rule(1, 2)}
        bad_mean = self.run_compare({"key": {"mean": 0}}, {"key": 2}, rules)
        self.assertEqual(bad_mean.returncode, 2)
        null_candidate = self.run_compare({"key": 1}, {"key": None}, rules)
        self.assertEqual(null_candidate.returncode, 2)
        null_baseline = self.run_compare({"key": None}, {"key": 1}, rules)
        self.assertEqual(null_baseline.returncode, 2)

if __name__ == "__main__":
    unittest.main()
