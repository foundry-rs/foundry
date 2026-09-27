import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from plan import compile_plan, run_plan


class PlanTests(unittest.TestCase):
    def setUp(self):
        self.recording = json.loads(Path(__file__).with_name("synthetic-recording.json").read_text())

    def test_replay_and_matched_inputs(self):
        first = compile_plan(self.recording, 1009, 40, 30, 0.2)
        self.assertEqual(first, compile_plan(copy.deepcopy(self.recording), 1009, 40, 30, 0.2))
        for key, expected in {"short": 0.2746666666666667, "base": 0.3066666666666667,
                              "long": 0.4186666666666667}.items():
            self.assertAlmostEqual(first["weights"][key], expected)
        schedules = first["schedules"]
        self.assertEqual(schedules["uniform"][0], {
            "candidate": "short", "depth": 50,
            "seed": "0xa33dc98eeddd72e64abea6436a96c9553f099a3c600cc81d", "seconds": 30,
        })
        for schedule in schedules.values():
            self.assertEqual([x["seed"] for x in schedule], [x["seed"] for x in schedules["uniform"]])
            self.assertEqual(sum(x["seconds"] for x in schedule), 1200)
        self.assertEqual({x["candidate"] for x in schedules["greedy"]}, {"long"})
        self.assertEqual({x["candidate"] for x in schedules["baseline"]}, {"base"})
        self.assertEqual({x["candidate"] for x in schedules["weighted"]}, {"short", "base", "long"})

    def test_confidence_and_exploration(self):
        self.recording["response"]["answers"]["campaign"]["confidence"] = 0
        plan = compile_plan(self.recording, 7, 40, 1, 0.2)
        self.assertEqual(plan["schedules"]["uniform"], plan["schedules"]["weighted"])
        answer = self.recording["response"]["answers"]["campaign"]
        answer["confidence"] = 1
        answer["probabilities"] = {"short": 0, "base": 0, "long": 1}
        plan = compile_plan(self.recording, 7, 40, 1, 0.3)
        self.assertAlmostEqual(plan["weights"]["short"], 0.1)
        self.assertAlmostEqual(sum(plan["weights"].values()), 1)

    def test_reject_invalid_response(self):
        for probabilities in ({"short": 1}, {"short": 0, "base": 0, "long": 0},
                              {"short": -0.1, "base": 0.5, "long": 0.6},
                              {"short": float("nan"), "base": 0, "long": 1}):
            with self.subTest(probabilities=probabilities):
                recording = copy.deepcopy(self.recording)
                recording["response"]["answers"]["campaign"]["probabilities"] = probabilities
                with self.assertRaises(ValueError):
                    compile_plan(recording, 1, 1, 1, 0.2)
        self.recording["request"]["state"]["depths"]["long"] = 201
        with self.assertRaises(ValueError):
            compile_plan(self.recording, 1, 1, 1, 0.2)

    def test_live_provenance_and_target_binding(self):
        self.recording["provenance"] = "live"
        with self.assertRaises(ValueError):
            compile_plan(self.recording, 1, 1, 1, 0.2)
        self.recording.update(latency_ms=123, cost_usd=0.001)
        self.recording["response"].update(model="recorded-version", usage={"input_tokens": 10})
        self.recording["request"]["state"].update(target_repo="repo", target_ref="a" * 40)
        plan = compile_plan(self.recording, 1, 1, 1, 0.2)
        with tempfile.TemporaryDirectory() as directory, patch("plan.subprocess.run") as run:
            with self.assertRaises(ValueError):
                run_plan(plan, "weighted", Path("runner"), Path("forge"), "other-repo",
                         "a" * 40, "b" * 40, Path(directory) / "run")
            run.assert_not_called()

    def test_runner_uses_existing_pipeline_and_fresh_slices(self):
        plan = compile_plan(self.recording, 1, 2, 15, 0.2)
        with tempfile.TemporaryDirectory() as directory, patch("plan.subprocess.run") as run:
            output = Path(directory) / "run"
            run_plan(plan, "weighted", Path("runner"), Path("forge"), "repo",
                     "a" * 40, "b" * 40, output)
            self.assertEqual(run.call_count, 2)
            for index, call in enumerate(run.call_args_list):
                argv = call.args[0]
                self.assertEqual(argv[0], "runner")
                self.assertEqual(argv[-2:], ["--output-dir", str(output / str(index))])
                self.assertEqual(argv[argv.index("--workers") + 1], "1")
                self.assertEqual(argv[-3], "--foundry-test-args=" +
                                 f'--fuzz-seed {plan["schedules"]["weighted"][index]["seed"]} ' +
                                 f'--invariant-depth {plan["schedules"]["weighted"][index]["depth"]} ' +
                                 "--invariant-workers 1 --show-progress")
            with self.assertRaises(FileExistsError):
                run_plan(plan, "weighted", Path("runner"), Path("forge"), "repo",
                         "a" * 40, "b" * 40, output)

    def test_no_execution_for_modified_plan(self):
        plan = compile_plan(self.recording, 1, 2, 1, 0.2)
        plan["schedules"]["weighted"][0]["depth"] = 999
        with tempfile.TemporaryDirectory() as directory, patch("plan.subprocess.run") as run:
            with self.assertRaises(ValueError):
                run_plan(plan, "weighted", Path("runner"), Path("forge"), "repo",
                         "a" * 40, "b" * 40, Path(directory) / "run")
            run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
