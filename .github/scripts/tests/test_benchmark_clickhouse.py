import importlib.util
import json
import pathlib
import tempfile
import unittest
from unittest import mock


SCRIPT = pathlib.Path(__file__).parents[1] / "benchmark_clickhouse.py"
SPEC = importlib.util.spec_from_file_location("benchmark_clickhouse", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


BASE_SHA = "1" * 40
CANDIDATE_SHA = "2" * 40
WORKLOAD_SHA = "3" * 40


def result(mean):
    return {
        "mean": mean,
        "stddev": 0.1,
        "median": mean,
        "user": mean / 2,
        "system": mean / 4,
        "min": mean - 0.1,
        "max": mean + 0.1,
        "times": [mean - 0.1, mean, mean + 0.1],
    }


def manifest():
    def case(benchmark):
        return {
            "id": f"{benchmark}/example-project",
            "benchmark": benchmark,
            "workload_name": "example-project",
            "workload_repository": "example/project",
            "workload_requested_ref": "v1.0.0",
            "workload_commit": WORKLOAD_SHA,
            "arguments": None,
            "versions": ["master", "local"],
        }

    suites = [
        {
            "schema": MODULE.SUITE_SCHEMA,
            "suite": suite,
            "cases": [case(benchmark) for benchmark in sorted(benchmarks)],
        }
        for suite, benchmarks in MODULE.EXPECTED_SUITES.items()
    ]
    expected = sorted(case["id"] for suite in suites for case in suite["cases"])
    return {
        "schema": MODULE.RUN_SCHEMA,
        "project": "foundry",
        "run": {
            "repository": "foundry-rs/foundry",
            "workflow": "Foundry Benchmarks",
            "workflow_file": ".github/workflows/benchmarks.yml",
            "run_id": 123,
            "run_attempt": 1,
            "run_url": "https://github.com/foundry-rs/foundry/actions/runs/123",
            "event": "workflow_dispatch",
            "branch": "feature",
        },
        "comparison": {
            "kind": "source_vs_master",
            "baseline_ref": "master",
            "baseline_commit": BASE_SHA,
            "candidate_ref": "feature",
            "candidate_commit": CANDIDATE_SHA,
        },
        "environment": {
            "build_profile": "profiling",
            "runner_os": "Linux",
            "runner_arch": "X64",
            "cpu_model": "Example CPU",
            "cpu_count": 32,
            "rustc": "rustc 1.0.0",
            "hyperfine": "hyperfine 1.0.0",
        },
        "expected_result_keys": expected,
        "suites": suites,
    }


TRUSTED = {
    "repository": "foundry-rs/foundry",
    "workflow_file": ".github/workflows/benchmarks.yml",
    "default_branch": "master",
    "run_id": 123,
    "run_attempt": 1,
    "run_url": "https://github.com/foundry-rs/foundry/actions/runs/123",
    "head_sha": CANDIDATE_SHA,
    "branch": "feature",
    "started_at": "2026-07-15 12:00:00.000",
}


class BenchmarkClickHouseTest(unittest.TestCase):
    def artifact_dir(self, base=None, candidate=None, run_manifest=None):
        temp = tempfile.TemporaryDirectory()
        root = pathlib.Path(temp.name)
        run_manifest = run_manifest or manifest()
        default_base = {key: result(1.0) for key in run_manifest["expected_result_keys"]}
        default_candidate = {key: result(1.1) for key in run_manifest["expected_result_keys"]}
        values = {
            "benchmark-manifest.json": run_manifest,
            "base-summary.json": default_base if base is None else base,
            "candidate-summary.json": default_candidate if candidate is None else candidate,
        }
        for name, value in values.items():
            (root / name).write_text(json.dumps(value), encoding="utf-8")
        return temp, root

    def test_builds_versioned_run_manifest(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        fragment = pathlib.Path(temp.name) / "suite.json"
        suite = manifest()["suites"][0]
        fragment.write_text(json.dumps(suite), encoding="utf-8")
        environment = {
            "GITHUB_REPOSITORY": "foundry-rs/foundry",
            "GITHUB_RUN_ID": "123",
            "GITHUB_RUN_ATTEMPT": "1",
            "GITHUB_REF_NAME": "feature",
            "GITHUB_EVENT_NAME": "workflow_dispatch",
        }
        with (
            mock.patch.dict(MODULE.os.environ, environment, clear=True),
            mock.patch.object(MODULE, "_cpu_model", return_value="Example CPU"),
            mock.patch.object(MODULE, "_command_version", return_value="tool 1.0"),
        ):
            run_manifest = MODULE.build_manifest([fragment], BASE_SHA, CANDIDATE_SHA)
        self.assertEqual(run_manifest["schema"], MODULE.RUN_SCHEMA)
        self.assertEqual(run_manifest["comparison"]["candidate_commit"], CANDIDATE_SHA)
        self.assertEqual(
            run_manifest["suites"][0]["cases"][0]["workload_commit"], WORKLOAD_SHA
        )

    def test_flattens_both_comparison_sides(self):
        temp, root = self.artifact_dir()
        self.addCleanup(temp.cleanup)
        rows = MODULE.build_rows(root, TRUSTED)
        self.assertEqual({row["side_role"] for row in rows}, {"baseline", "candidate"})
        self.assertEqual(rows[0]["mean_seconds"], 1.0)
        self.assertEqual(rows[-1]["mean_seconds"], 1.1)
        self.assertNotEqual(rows[0]["result_id"], rows[-1]["result_id"])
        self.assertEqual(rows[0]["workload_commit"], WORKLOAD_SHA)

    def test_result_ids_are_stable_but_attempts_are_distinct(self):
        temp, root = self.artifact_dir()
        self.addCleanup(temp.cleanup)
        first = MODULE.build_rows(root, TRUSTED)
        self.assertEqual(first, MODULE.build_rows(root, TRUSTED))

        rerun_manifest = manifest()
        rerun_manifest["run"]["run_attempt"] = 2
        (root / "benchmark-manifest.json").write_text(json.dumps(rerun_manifest), encoding="utf-8")
        rerun_trusted = {**TRUSTED, "run_attempt": 2}
        rerun = MODULE.build_rows(root, rerun_trusted)
        self.assertNotEqual(first[0]["result_id"], rerun[0]["result_id"])

    def test_rejects_incomplete_comparison(self):
        temp, root = self.artifact_dir(candidate={"unexpected": result(1.1)})
        self.addCleanup(temp.cleanup)
        with self.assertRaisesRegex(ValueError, "exactly match"):
            MODULE.build_rows(root, TRUSTED)

    def test_rejects_non_finite_timings(self):
        bad = result(1.0)
        bad["mean"] = float("nan")
        base = {key: result(1.0) for key in manifest()["expected_result_keys"]}
        base["forge_test/example-project"] = bad
        temp, root = self.artifact_dir(base=base)
        self.addCleanup(temp.cleanup)
        with self.assertRaisesRegex(ValueError, "non-finite"):
            MODULE.build_rows(root, TRUSTED)

    def test_preserves_symbolic_counters_as_integers(self):
        symbolic = result(1.0)
        symbolic["symbolic"] = {"solver_queries": 42, "solver_time_ms": 123}
        base = {key: result(1.0) for key in manifest()["expected_result_keys"]}
        base["forge_test/example-project"] = symbolic
        temp, root = self.artifact_dir(base=base)
        self.addCleanup(temp.cleanup)
        rows = MODULE.build_rows(root, TRUSTED)
        row = next(row for row in rows if row["benchmark_case_id"] == "forge_test/example-project")
        self.assertEqual(row["counters_u64"]["solver_queries"], 42)

    def test_rejects_unsafe_clickhouse_table_name(self):
        environment = {
            "CLICKHOUSE_HOST": "clickhouse.example",
            "CLICKHOUSE_USER": "writer",
            "CLICKHOUSE_PASSWORD": "secret",
            "CLICKHOUSE_TABLE": "results; DROP TABLE results",
        }
        with mock.patch.dict(MODULE.os.environ, environment, clear=True):
            with self.assertRaisesRegex(ValueError, "identifier"):
                MODULE.upload_rows([])

    def test_uploads_to_reth_style_clickhouse_host(self):
        environment = {
            "CLICKHOUSE_HOST": "clickhouse.example",
            "CLICKHOUSE_USER": "writer",
            "CLICKHOUSE_PASSWORD": "secret",
        }
        response = mock.MagicMock()
        response.__enter__.return_value.read.return_value = b""
        with mock.patch.dict(MODULE.os.environ, environment, clear=True):
            with mock.patch.object(MODULE.urllib.request, "urlopen", return_value=response) as urlopen:
                MODULE.upload_rows([{"result_id": "example"}])

        request = urlopen.call_args.args[0]
        self.assertTrue(request.full_url.startswith("https://clickhouse.example:8443/"))
        self.assertIn("database=default", request.full_url)
        self.assertIn("INSERT+INTO+benchmark_results+FORMAT+JSONEachRow", request.full_url)
        self.assertEqual(request.data, b'{"result_id":"example"}\n')


if __name__ == "__main__":
    unittest.main()
