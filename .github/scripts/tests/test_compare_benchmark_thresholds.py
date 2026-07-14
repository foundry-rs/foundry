import json
import pathlib
import subprocess
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).parents[1] / "compare-benchmark-thresholds.py"


class CompareBenchmarkThresholdsTests(unittest.TestCase):
    def run_compare(self, base, candidate, rules):
        with tempfile.TemporaryDirectory() as directory:
            directory = pathlib.Path(directory)
            base_path = directory / "base.json"
            candidate_path = directory / "candidate.json"
            base_path.write_text(json.dumps(base))
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

    def test_advisory_missing_and_uncalibrated_never_alert(self):
        advisory = self.run_compare(
            {"coverage": 100}, {"coverage": 120}, {"coverage": self.rule(5, 10, False)}
        )
        self.assertEqual(advisory.returncode, 0)
        self.assertIn("Regression (advisory)", advisory.stdout)

        missing = self.run_compare({"only-base": 1}, {"only-candidate": 1}, {})
        self.assertEqual(missing.returncode, 0)
        self.assertEqual(missing.stdout.count("Inconclusive (missing side)"), 2)

        uncalibrated = self.run_compare({"symbolic": 1}, {"symbolic": 2}, {})
        self.assertEqual(uncalibrated.returncode, 0)
        self.assertIn("Uncalibrated", uncalibrated.stdout)

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
