#!/usr/bin/env python3
"""Exercise Markdown reporting with complete, failed and untrusted campaign evidence."""

import copy
import json
from pathlib import Path
import tempfile
import unittest

from cast_bal_report import MARKER, main, median_interval, render


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

    def set_measured_values(self, label, arm, values):
        template = next(sample for sample in self.samples[label]
                        if sample["phase"] == "measured" and sample["arm"] == arm)
        self.samples[label] = [sample for sample in self.samples[label]
                               if sample["phase"] != "measured" or sample["arm"] != arm]
        for index, value in enumerate(values):
            sample = copy.deepcopy(template)
            sample.update(id=f"measured-{index}-{arm}", round=index,
                          wall_time_seconds=value, observed_duration_seconds=value)
            self.samples[label].append(sample)
        self.config["rounds"] = len(values)
        for run in self.runs.values():
            run["rounds"] = len(values)

    def set_wall_comparison(self, base, candidate):
        self.set_mode_comparison(base, base, candidate, candidate)

    def set_mode_comparison(self, base_bal, base_replay, candidate_bal, candidate_replay):
        self.assertEqual(len({len(values) for values in
                              (base_bal, base_replay, candidate_bal, candidate_replay)}), 1)
        for label, modes in (("base", (base_bal, base_replay)),
                             ("candidate", (candidate_bal, candidate_replay))):
            for arm, values in zip(("auto", "replay"), modes):
                self.set_measured_values(label, arm, values)
        self.write_fixture()

    def assert_withheld(self, report):
        self.assertIn("**No valid PR performance comparison.**", report)
        self.assertNotIn("| Case | Base speedup |", report)
        self.assertNotIn("| Case | Run 1 speedup |", report)
        self.assertNotIn("BAL benefit: ", report)
        self.assertNotIn("| Wall ms (Base → PR) |", report)
        self.assertNotIn("Improved (", report)
        self.assertNotIn("Regressed (", report)
        self.assertNotIn("wall time: Improved", report)
        self.assertNotIn("wall time: Regressed", report)
        self.assertNotIn("| Time ratio |", report)
        self.assertNotIn("2.000×", report)
        self.assertNotIn("| 2000.000 /", report)

    def test_complete_report_compares_bal_benefit_and_folds_raw_measurements(self):
        report = self.report(run_url="https://github.com/foundry-rs/foundry/actions/runs/123")
        self.assertTrue(report.startswith(MARKER + "\n"))
        self.assertIn("**Complete:** 1 cases, 2 measured rounds per revision/mode", report)
        headline, details = report.split("<details>", 1)
        self.assertIn("| Case | Base speedup | PR speedup | Speedup change | BAL benefit |", headline)
        self.assertIn("| b100-t2 | 6.00× | 1.50× | 75.0% lower | Inconclusive |", headline)
        self.assertNotIn("IQR", headline)
        self.assertNotIn("min / max", headline)
        self.assertNotIn("Median intervals", headline)
        self.assertNotIn("Speedup intervals", headline)
        self.assertNotIn("Too few measured rounds", headline)
        self.assertEqual(report.count("<details>"), 1)
        self.assertIn("median / IQR / min / max", details)
        self.assertIn("Too few measured rounds (at least 7 per revision required).", details)
        self.assertIn("Too few measured rounds (at least 8 per revision/mode required).", details)
        self.assertIn("**BAL-accelerated wall time: Inconclusive (100.0% higher).**", details)
        self.assertIn("**Full replay wall time: Inconclusive (50.0% lower).**", details)
        self.assertIn("| Base / BAL-accelerated | 2 / 0 / 0 | 2000.000 / 1000.000 / 1000.000 / 3000.000 | 4 / 4 | 2.0 |", report)
        self.assertIn("<summary>Per-case measurements and correctness</summary>", report)
        self.assertIn("not public-RPC latency", report)
        self.assertEqual(report, self.report(run_url="https://github.com/foundry-rs/foundry/actions/runs/123"))

    def test_clear_timing_direction_is_reported_for_each_mode(self):
        self.set_wall_comparison([2] * 10, [1] * 10)
        self.set_measured_values("base", "replay", [1] * 10)
        self.set_measured_values("candidate", "replay", [2] * 10)
        self.write_fixture()
        headline, details = self.report().split("<details>", 1)
        self.assertIn("| b100-t2 | 0.50× | 2.00× | 300.0% higher | Improved |", headline)
        self.assertIn("**BAL-accelerated wall time: Improved (50.0% lower).** Median intervals: Base 2000.000–2000.000 ms; PR 1000.000–1000.000 ms. PR interval is entirely lower.", details)
        self.assertIn("**Full replay wall time: Regressed (100.0% higher).** Median intervals: Base 1000.000–1000.000 ms; PR 2000.000–2000.000 ms. PR interval is entirely higher.", details)
        self.assertNotIn("Median intervals", headline)

    def test_faster_time_and_higher_resource_use_have_separate_outcomes(self):
        self.set_mode_comparison([2] * 10, [4] * 10, [1] * 10, [4] * 10)
        for sample in self.samples["candidate"]:
            if sample["phase"] == "measured":
                sample["rpc"]["client_requests_by_method"]["eth_getCode"] = 6
                sample["rpc"]["upstream_requests_by_method"]["eth_getCode"] = 6
                sample["rpc"]["client_response_body_bytes"] = 4096
        self.write_fixture()
        headline, details = self.report().split("<details>", 1)
        self.assertIn("| b100-t2 | 2.00× | 4.00× | 100.0% higher | Improved |", headline)
        self.assertIn("**BAL-accelerated wall time: Improved (50.0% lower).**", details)
        self.assertIn("RPC: 50.0% higher; Response: 100.0% higher.", details)
        self.assertNotIn("Regressed", headline)
        self.assertIn("6 / 6 | 4.0", details)

    def test_overlapping_or_touching_intervals_are_inconclusive(self):
        comparisons = (
            ([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], [value + .5 for value in range(1, 11)]),
            ([1] + [2] * 8 + [3], [2] * 2 + [3] * 8),
            ([2] * 10, [2] * 10),
        )
        for base, candidate in comparisons:
            with self.subTest(base=base, candidate=candidate):
                self.set_wall_comparison(base, candidate)
                headline, details = self.report().split("<details>", 1)
                self.assertIn("| Inconclusive |", headline)
                self.assertNotIn("| Improved |", headline)
                self.assertNotIn("| Regressed |", headline)
                self.assertNotIn("Intervals overlap or touch", headline)
                self.assertIn("Intervals overlap or touch; direction is unresolved.", details)
                self.assertIn("Median intervals: Base", details)

    def test_fewer_than_seven_rounds_cannot_claim_timing_direction(self):
        self.set_wall_comparison([10] * 6, [1] * 6)
        headline, details = self.report().split("<details>", 1)
        self.assertIn("| b100-t2 | 1.00× | 1.00× | unchanged | Inconclusive |", headline)
        self.assertIn("**BAL-accelerated wall time: Inconclusive (90.0% lower).**", details)
        self.assertNotIn("Improved (", headline)
        self.assertIn("Too few measured rounds (at least 7 per revision required).", details)

    def test_median_intervals_use_exact_conservative_order_statistics(self):
        for count in (0, 1, 6):
            with self.subTest(count=count):
                self.assertIsNone(median_interval(list(range(count))))
        self.assertEqual(median_interval(list(range(7))), (0, 6))
        self.assertEqual(median_interval(list(range(10))), (1, 8))
        self.assertEqual(median_interval(list(reversed(range(10)))), (1, 8))
        self.assertEqual(median_interval([2] * 10), (2, 2))
        self.assertIsNone(median_interval(list(range(7)), groups=4))
        self.assertEqual(median_interval(list(range(8)), groups=4), (0, 7))
        self.assertEqual(median_interval(list(range(10)), groups=4), (0, 9))

    def test_sample_and_round_order_does_not_change_report(self):
        self.set_mode_comparison(list(range(11, 21)), list(range(21, 31)),
                                 list(range(1, 11)), list(range(31, 41)))
        expected = self.report()
        for samples in self.samples.values():
            samples.reverse()
            for sample in samples:
                if sample["phase"] == "measured":
                    sample["round"] = 9 - sample["round"]
        self.write_fixture()
        self.assertEqual(self.report(), expected)

    def test_zero_resource_baselines_show_counts_without_dividing_by_zero(self):
        self.set_wall_comparison([2] * 10, [1] * 10)
        for sample in self.samples["base"]:
            if sample["phase"] == "measured":
                sample["rpc"]["client_requests_by_method"] = {}
                sample["rpc"]["upstream_requests_by_method"] = {}
                sample["rpc"]["client_response_body_bytes"] = 0
        self.write_fixture()
        details = self.report().split("<details>", 1)[1]
        self.assertIn("**BAL-accelerated wall time: Improved (50.0% lower).**", details)
        self.assertIn("RPC: 0 → 2; Response: 0 → 2048 bytes.", details)

    def test_small_speedup_and_timing_changes_do_not_round_to_misleading_zero(self):
        for value, change, benefit, timing in ((1.0001, "<0.1% higher", "Improved", "Regressed"),
                                               (.9999, "<0.1% lower", "Regressed", "Improved")):
            with self.subTest(value=value):
                self.set_mode_comparison([1] * 10, [1] * 10, [1] * 10, [value] * 10)
                headline, details = self.report().split("<details>", 1)
                self.assertIn(f"| b100-t2 | 1.00× | 1.00× | {change} | {benefit} |", headline)
                self.assertIn(f"**Full replay wall time: {timing} ({change}).**", details)
                self.assertNotIn("| 0.0%", headline)
                self.assertNotIn("(0.0%", details)

    def test_bal_benefit_direction_uses_each_revisions_full_replay(self):
        for bal, speedup, change, status in ((1, "4.00×", "100.0% higher", "Improved"),
                                            (4, "1.00×", "50.0% lower", "Regressed")):
            with self.subTest(bal=bal):
                self.set_mode_comparison([2] * 10, [4] * 10, [bal] * 10, [4] * 10)
                headline, details = self.report().split("<details>", 1)
                self.assertIn(f"| b100-t2 | 2.00× | {speedup} | {change} | {status} |", headline)
                self.assertIn(f"**BAL benefit: {status}.** Speedup intervals: Base 2.000–2.000×;", details)

    def test_speedup_uses_ratio_of_medians_without_pairing_rounds(self):
        self.set_mode_comparison([1, 100, 100] * 3, [10, 10, 1000] * 3,
                                 [2, 200, 200] * 3, [20, 20, 2000] * 3)
        headline = self.report().split("<details>", 1)[0]
        self.assertIn("| b100-t2 | 0.10× | 0.10× | unchanged | Inconclusive |", headline)
        self.assertNotIn("| 10.00× |", headline)

    def test_both_modes_faster_does_not_imply_larger_bal_benefit(self):
        self.set_mode_comparison([2] * 10, [4] * 10, [1] * 10, [2] * 10)
        headline, details = self.report().split("<details>", 1)
        self.assertIn("| b100-t2 | 2.00× | 2.00× | unchanged | Inconclusive |", headline)
        self.assertIn("**BAL-accelerated wall time: Improved (50.0% lower).**", details)
        self.assertIn("**Full replay wall time: Improved (50.0% lower).**", details)
        self.assertIn("**BAL benefit: Inconclusive.** Speedup intervals: Base 2.000–2.000×; PR 2.000–2.000×. Intervals overlap or touch; direction is unresolved.", details)

    def test_larger_bal_benefit_can_coexist_with_slower_absolute_bal_time(self):
        self.set_mode_comparison([2] * 10, [4] * 10, [3] * 10, [12] * 10)
        headline, details = self.report().split("<details>", 1)
        self.assertIn("| b100-t2 | 2.00× | 4.00× | 100.0% higher | Improved |", headline)
        self.assertIn("**BAL-accelerated wall time: Regressed (50.0% higher).**", details)
        self.assertIn("**Full replay wall time: Regressed (200.0% higher).**", details)

    def test_overlapping_and_touching_speedup_intervals_are_inconclusive(self):
        comparisons = (
            ([1] * 5 + [3] * 5, [2] * 5 + [4] * 5,
             "2.00× | 3.00× | 50.0% higher", "Base 1.000–3.000×; PR 2.000–4.000×."),
            ([1] * 5 + [2] * 5, [2] * 5 + [3] * 5,
             "1.50× | 2.50× | 66.7% higher", "Base 1.000–2.000×; PR 2.000–3.000×."),
        )
        for base, candidate, values, intervals in comparisons:
            with self.subTest(intervals=intervals):
                self.set_mode_comparison([1] * 10, base, [1] * 10, candidate)
                headline, details = self.report().split("<details>", 1)
                self.assertIn(f"| b100-t2 | {values} | Inconclusive |", headline)
                self.assertIn(f"**BAL benefit: Inconclusive.** Speedup intervals: {intervals} Intervals overlap or touch; direction is unresolved.", details)

    def test_bal_benefit_requires_eight_rounds_while_wall_direction_requires_seven(self):
        for rounds, status in ((7, "Inconclusive"), (8, "Improved")):
            with self.subTest(rounds=rounds):
                self.set_mode_comparison([2] * rounds, [4] * rounds,
                                         [1] * rounds, [4] * rounds)
                headline, details = self.report().split("<details>", 1)
                self.assertIn(f"| b100-t2 | 2.00× | 4.00× | 100.0% higher | {status} |", headline)
                self.assertIn("**BAL-accelerated wall time: Improved (50.0% lower).**", details)
                if rounds == 7:
                    self.assertIn("**BAL benefit: Inconclusive.** Too few measured rounds (at least 8 per revision/mode required).", details)
                else:
                    self.assertIn("**BAL benefit: Improved.** Speedup intervals: Base 2.000–2.000×; PR 4.000–4.000×.", details)
                    self.assertNotIn("Too few measured rounds", details)

    def test_below_one_speedup_is_visible_as_bal_slower_than_replay(self):
        self.set_mode_comparison([4] * 10, [2] * 10, [8] * 10, [2] * 10)
        headline, details = self.report().split("<details>", 1)
        self.assertIn("| b100-t2 | 0.50× | 0.25× | 50.0% lower | Regressed |", headline)
        self.assertIn("below 1× means BAL is slower", headline)
        self.assertIn("PR has a smaller BAL benefit relative to its own full replay.", details)

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
                headline = report.split("<details>", 1)[0]
                self.assertIn("| Case | Run 1 speedup | Run 2 speedup |", headline)
                self.assertIn("| b100-t2 | 6.00× | 1.50× |", headline)
                self.assertNotIn("Not compared", report)
                self.assertNotIn("Speedup change", report)
                self.assertNotIn("| BAL benefit |", report)
                self.assertNotIn("BAL benefit:", report)
                self.assertNotIn("Improved", report)
                self.assertNotIn("Regressed", report)
                self.assertNotIn("wall time: Improved", report)
                self.assertNotIn("wall time: Regressed", report)
                self.assertNotIn("% higher", report)
                self.assertNotIn("% lower", report)
                self.assertNotIn("IQR", headline)
                self.assertNotIn("min / max", headline)
                self.assertIn("same revision or binary", report)
                self.assertIn("not PR performance evidence", report)
                self.assertIn("| Base / BAL-accelerated | 2 / 0 / 0 | 2000.000 / 1000.000 / 1000.000 / 3000.000 | 4 / 4 | 2.0 |", report)
                self.assertIn("| Base / Full replay | 2 / 0 / 0 | 12000.000 / 2000.000 / 10000.000 / 14000.000 | 4 / 4 | 2.0 |", report)
                self.assertIn("| PR / BAL-accelerated | 2 / 0 / 0 | 4000.000 / 2000.000 / 2000.000 / 6000.000 | 2 / 2 | 2.0 |", report)
                self.assertIn("| PR / Full replay | 2 / 0 / 0 | 6000.000 / 1000.000 / 5000.000 / 7000.000 | 2 / 2 | 2.0 |", report)

    def test_validated_control_does_not_judge_separated_speedup_intervals(self):
        self.set_mode_comparison([2] * 10, [4] * 10, [1] * 10, [4] * 10)
        self.summary["files"]["head_cast"]["sha256"] = "a" * 64
        self.write_fixture()
        report = self.report()
        headline = report.split("<details>", 1)[0]
        self.assertIn("| b100-t2 | 2.00× | 4.00× |", headline)
        self.assertNotIn("Improved", report)
        self.assertNotIn("Regressed", report)
        self.assertNotIn("Speedup change", report)
        self.assertNotIn("% higher", report)
        self.assertNotIn("% lower", report)

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
