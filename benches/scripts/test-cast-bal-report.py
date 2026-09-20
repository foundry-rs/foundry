#!/usr/bin/env python3
"""Exercise Markdown reporting with complete, failed and untrusted campaign evidence."""

import copy
import json
from pathlib import Path
import tempfile
import unittest

from cast_bal_report import MARKER, main, render


BASE = "1" * 40
HEAD = "2" * 40


class ReportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="foundry-bal-report-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.case = {"id": "b100-t2", "block_hash": "0x" + "3" * 64,
                     "index": 2, "transaction_hash": "0x" + "4" * 64,
                     "expected_receipt_gas": 21000, "expected_receipt_status": True}
        self.block = {"block_number": 100, "block_hash": self.case["block_hash"]}
        self.panel = {"schema_version": 1, "chain_id": 1,
                      "cases": [self.case], "blocks": [self.block]}
        self.config = {"rounds": 2, "warmup_rounds": 1, "timeout_seconds": 120, "worker_count": 1}
        self.summary = {
            "schema_version": 1, "status": "complete", "refs": {"base": BASE, "head": HEAD},
            "configuration": self.config,
            "files": {"base_cast": {"sha256": "a" * 64}, "head_cast": {"sha256": "b" * 64}},
            "blocks": [{"block_number": 100, "status": "pass", "fixture_barrier_closed": True,
                        "barrier_attempts": 0, "fixture_failures": 0,
                        "verification": {"issues": [], "cross_ref_receipt_gas_and_status_match": True}}],
        }
        self.samples = {}
        self.runs = {}
        for label in ("base", "candidate"):
            binary = self.summary["files"]["base_cast" if label == "base" else "head_cast"]
            self.runs[label] = {"schema_version": 1, **self.config, "include_miss": False,
                                "panel": self.panel, "binary": binary,
                                "build": {"source_sha": BASE if label == "base" else HEAD,
                                          "cast": binary}}
            samples = []
            for phase, rounds, arms in (("oracle", 1, ("replay",)),
                                        ("validation", 1, ("auto", "replay")),
                                        ("warmup", 1, ("auto", "replay")),
                                        ("measured", 2, ("auto", "replay"))):
                for index in range(rounds):
                    for arm in arms:
                        wall = (1 + 2 * index) if arm == "auto" else (10 + 4 * index)
                        if label == "candidate":
                            wall *= 2 if arm == "auto" else .5
                        samples.append({
                            "schema_version": 1, "id": f"{phase}-{index}-{arm}", "case_id": self.case["id"],
                            "phase": phase, "round": index, "arm": arm,
                            "actual_path": "bal_hit" if arm == "auto" else "replay_no_probe",
                            "exit_code": 0, "timed_out": False, "wall_time_seconds": wall,
                            "observed_duration_seconds": wall, "stdout_sha256": "c" * 64,
                            "local_gas": 21000, "execution_success": True,
                            "correctness": "unchecked" if phase == "oracle" else "equivalent",
                            "fault": None, "synthetic": False,
                            "rpc": {"client_requests_by_method": {"eth_getCode": 4 if label == "base" else 2},
                                    "upstream_requests_by_method": {"eth_getCode": 4 if label == "base" else 2},
                                    "client_response_body_bytes": 2048},
                        })
            self.samples[label] = samples
        self.write_fixture()

    def write_fixture(self):
        (self.root / "campaign.json").write_text(json.dumps(self.summary))
        (self.root / "manifest.json").write_text(json.dumps(self.panel))
        for label in self.samples:
            root = self.root / f"block-100/results/{label}/aggregate"
            root.mkdir(parents=True, exist_ok=True)
            (root / "manifest.json").write_text(json.dumps(self.runs[label]))
            (root / "samples.jsonl").write_text("".join(json.dumps(sample) + "\n" for sample in self.samples[label]))

    def report(self, **kwargs):
        return render(self.root, BASE, HEAD, **kwargs)

    def assert_withheld(self, report):
        self.assertIn("**No valid PR performance comparison.**", report)
        self.assertNotIn("| Time ratio |", report)
        self.assertNotIn("2.000×", report)
        self.assertNotIn("| 2000.000 /", report)

    def test_complete_report_compares_each_mode_across_revisions(self):
        report = self.report(run_url="https://github.com/foundry-rs/foundry/actions/runs/123")
        self.assertTrue(report.startswith(MARKER + "\n"))
        self.assertIn("**Complete:** 1 cases, 2 measured rounds per revision/mode", report)
        self.assertIn("| b100-t2 | BAL-accelerated | 2000.000 → 4000.000 | 2.000× | 4 → 2 | 0.500× |", report)
        self.assertIn("| b100-t2 | Full replay | 12000.000 → 6000.000 | 0.500× | 4 → 2 | 0.500× |", report)
        self.assertIn("| Base / BAL-accelerated | 2 / 0 / 0 | 2000.000 / 1000.000 / 1000.000 / 3000.000 | 4 / 4 | 2.0 |", report)
        self.assertIn("<summary>Per-case measurements and correctness</summary>", report)
        self.assertIn("not public-RPC latency", report)
        self.assertIn("no overall speedup or statistical significance", report)
        self.assertEqual(report, self.report(run_url="https://github.com/foundry-rs/foundry/actions/runs/123"))

    def test_missing_duplicate_and_unexpected_attempts_suppress_all_comparisons(self):
        original = copy.deepcopy(self.samples["base"])
        for mutation in ("missing", "duplicate", "unexpected", "missing-validation", "missing-oracle"):
            with self.subTest(mutation=mutation):
                self.samples["base"] = copy.deepcopy(original)
                if mutation == "missing":
                    self.samples["base"].pop()
                elif mutation == "duplicate":
                    self.samples["base"].append(copy.deepcopy(original[-1]))
                elif mutation == "unexpected":
                    self.samples["base"][-1]["round"] = 30
                elif mutation == "missing-validation":
                    self.samples["base"].pop(1)
                else:
                    self.samples["base"].pop(0)
                self.write_fixture()
                report = self.report()
                self.assert_withheld(report)
                self.assertIn("Missing, duplicate or unexpected scheduled attempts", report)

    def test_invalid_measurements_and_receipts_are_never_rendered_as_performance(self):
        original = copy.deepcopy(self.samples["candidate"][-1])
        for field, value in (("timed_out", True), ("exit_code", 1), ("wall_time_seconds", float("nan")),
                             ("wall_time_seconds", -1), ("wall_time_seconds", 0), ("wall_time_seconds", None),
                             ("execution_success", False), ("local_gas", 21001),
                             ("actual_path", "replay_after_probe"), ("synthetic", True),
                             ("correctness", "correctness_blocked"), ("stdout_sha256", "d" * 64)):
            with self.subTest(field=field, value=value):
                self.samples["candidate"][-1] = {**original, field: value}
                self.write_fixture()
                self.assert_withheld(self.report())

    def test_negative_rpc_counts_are_invalid(self):
        self.samples["base"][-1]["rpc"]["client_requests_by_method"]["eth_getCode"] = -1
        self.write_fixture()
        self.assert_withheld(self.report())
        self.assertIn("Invalid RPC request counters", self.report())

    def test_failed_campaign_keeps_attempt_and_timeout_diagnostics(self):
        self.summary.update(status="failed", error="runner timed out")
        self.samples["candidate"][-1].update(timed_out=True, exit_code=None, wall_time_seconds=None)
        self.write_fixture()
        report = self.report()
        self.assert_withheld(report)
        self.assertIn("Campaign failed: runner timed out", report)
        self.assertIn("| PR / Full replay | 2 / 1 / 1 | withheld | withheld | withheld |", report)

    def test_missing_block_and_open_barrier_are_invalid(self):
        self.summary["blocks"] = []
        self.write_fixture()
        self.assert_withheld(self.report())
        self.assertIn("Block 100: not completed", self.report())
        self.summary["blocks"] = [{"block_number": 100, "status": "pass", "fixture_barrier_closed": False}]
        self.write_fixture()
        self.assert_withheld(self.report())

    def test_expected_shas_override_untrusted_mismatched_evidence(self):
        self.summary["refs"]["head"] = "9" * 40
        self.write_fixture()
        report = self.report()
        self.assert_withheld(report)
        self.assertIn(f"Base `{BASE}` → PR `{HEAD}`", report)
        self.assertIn("Campaign revisions differ", report)
        self.assertNotIn("9" * 40, report)

    def test_binary_and_build_provenance_must_match(self):
        self.runs["candidate"]["binary"] = {"sha256": "f" * 64}
        self.write_fixture()
        self.assert_withheld(self.report())
        self.runs["candidate"]["binary"] = self.summary["files"]["head_cast"]
        self.runs["candidate"]["build"]["source_sha"] = BASE
        self.write_fixture()
        self.assert_withheld(self.report())

    def test_validated_controls_show_measurements_without_pr_comparisons(self):
        for identity in ("binary", "revision"):
            with self.subTest(identity=identity):
                head = BASE if identity == "revision" else HEAD
                self.summary["refs"]["head"] = head
                self.runs["candidate"]["build"]["source_sha"] = head
                self.summary["files"]["head_cast"]["sha256"] = ("a" if identity == "binary" else "b") * 64
                self.write_fixture()
                report = render(self.root, BASE, head)
                self.assertIn("**Validated control:** 1 cases, 2 measured rounds per revision/mode", report)
                self.assertIn("All 8/8 measured attempts passed", report)
                self.assertIn("passed all receipt, replay-equivalence and preparation-barrier checks", report)
                self.assertNotIn("did not pass all checks", report)
                self.assertNotIn("| Time ratio |", report)
                self.assertNotIn("| RPC ratio |", report)
                self.assertNotIn("2.000×", report)
                self.assertIn("same revision or binary", report)
                self.assertIn("not PR performance evidence", report)
                self.assertIn("| Base / BAL-accelerated | 2 / 0 / 0 | 2000.000 / 1000.000 / 1000.000 / 3000.000 | 4 / 4 | 2.0 |", report)
                self.assertIn("| Base / Full replay | 2 / 0 / 0 | 12000.000 / 2000.000 / 10000.000 / 14000.000 | 4 / 4 | 2.0 |", report)
                self.assertIn("| PR / BAL-accelerated | 2 / 0 / 0 | 4000.000 / 2000.000 / 2000.000 / 6000.000 | 2 / 2 | 2.0 |", report)
                self.assertIn("| PR / Full replay | 2 / 0 / 0 | 6000.000 / 1000.000 / 5000.000 / 7000.000 | 2 / 2 | 2.0 |", report)

    def test_invalid_control_counters_withhold_all_measurements(self):
        self.summary["files"]["head_cast"]["sha256"] = "a" * 64
        self.samples["candidate"][-1]["rpc"]["client_response_body_bytes"] = -1
        self.write_fixture()
        report = self.report()
        self.assert_withheld(report)
        self.assertIn("Invalid RPC byte counter", report)
        self.assertIn("| Base / BAL-accelerated | 2 / 0 / 0 | withheld | withheld | withheld |", report)
        self.assertIn("| PR / Full replay | 2 / 0 / 0 | withheld | withheld | withheld |", report)

    def test_failed_control_is_not_reported_as_validated(self):
        self.summary["files"]["head_cast"]["sha256"] = "a" * 64
        self.samples["candidate"][-1].update(timed_out=True, wall_time_seconds=None)
        self.write_fixture()
        report = self.report()
        self.assert_withheld(report)
        self.assertNotIn("**Validated control:**", report)
        self.assertIn("Failed or timed-out attempt", report)
        self.assertIn("| Base / BAL-accelerated | 2 / 0 / 0 | withheld | withheld | withheld |", report)
        self.assertIn("| PR / Full replay | 2 / 1 / 0 | withheld | withheld | withheld |", report)

    def test_failed_build_cli_still_writes_postable_comment(self):
        output = self.root / "comment.md"
        self.assertEqual(main(["--campaign-dir", str(self.root / "absent"), "--output", str(output),
                               "--base-sha", BASE, "--head-sha", HEAD, "--failure", "Build failed"]), 0)
        report = output.read_text()
        self.assert_withheld(report)
        self.assertTrue(report.startswith(MARKER))
        self.assertIn("Build failed", report)

    def test_run_url_supports_attempts_and_rejects_markdown_injection(self):
        url = "https://github.com/foundry-rs/foundry/actions/runs/123/attempts/2"
        self.assertIn(f"[Benchmark run]({url})", self.report(run_url=url))
        for invalid in (url + ")evil", "https://evil.invalid/123", url + "/extra"):
            with self.subTest(invalid=invalid), self.assertRaisesRegex(ValueError, "Invalid GitHub Actions"):
                self.report(run_url=invalid)

    def test_invalid_json_and_nonobject_campaign_are_postable_failures(self):
        for contents in ("{", "[]", "null", '{"schema_version":1,"refs":[]}'):
            with self.subTest(contents=contents):
                (self.root / "campaign.json").write_text(contents)
                self.assert_withheld(self.report())

    def test_errors_are_escaped_and_cannot_ping_users(self):
        self.summary.update(status="failed", error="<script>alert(1)</script> @everyone [click](evil) | row")
        self.write_fixture()
        report = self.report()
        self.assertNotIn("<script>", report)
        self.assertNotIn("@everyone", report)
        self.assertNotIn("[click]", report)
        self.assertIn("&lt;script&gt;", report)

    def test_symlink_cannot_read_evidence_outside_campaign(self):
        path = self.root / "block-100/results/base/aggregate/samples.jsonl"
        path.unlink()
        path.symlink_to(self.root.parent / "not-campaign-evidence")
        report = self.report()
        self.assert_withheld(report)
        self.assertIn("Evidence path escapes campaign directory", report)


if __name__ == "__main__":
    unittest.main()
