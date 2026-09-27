#!/usr/bin/env python3
"""Compile an offline Choice recording into local scfuzzbench campaign slices."""

import argparse
import hashlib
import json
import math
from pathlib import Path
import subprocess


POLICIES = ("baseline", "uniform", "round-robin", "greedy", "weighted")


def derive_candidates(depth):
    """Bound the experiment to shorter, original, and longer local sequences."""
    # Halving must stay positive; doubling must fit Foundry's u32 depth.
    if type(depth) is not int or not 2 <= depth <= 2**31 - 1:
        raise ValueError("base depth must be an integer in [2, 2**31 - 1]")
    return {"short": depth // 2, "base": depth, "long": depth * 2}


def compile_plan(recording, seed, slices, seconds, exploration):
    if type(seed) is not int or seed < 0:
        raise ValueError("seed must be a nonnegative integer")
    if type(slices) is not int or slices < 1 or type(seconds) is not int or seconds < 1:
        raise ValueError("slices and seconds must be positive integers")
    if not math.isfinite(exploration) or not 0 < exploration <= 1:
        raise ValueError("exploration must be in (0, 1]")
    candidates = derive_candidates(recording["base_depth"])
    request = recording["request"]
    if set(request["questions"]) != {"campaign"}:
        raise ValueError("expected exactly one campaign question")
    question = request["questions"]["campaign"]
    if question["type"] != "choice" or set(question["criteria"]) != set(candidates):
        raise ValueError("request must describe the complete local candidate set")
    if request["state"]["depths"] != candidates:
        raise ValueError("request depths differ from locally derived candidates")
    answer = recording["response"]["answers"]["campaign"]
    probabilities = answer["probabilities"]
    if answer["type"] != "choice" or set(probabilities) != set(candidates):
        raise ValueError("response must contain every candidate, and no unknown candidates")
    values = list(probabilities.values()) + [answer["confidence"]]
    if any(type(x) not in (int, float) or not math.isfinite(x) or not 0 <= x <= 1 for x in values):
        raise ValueError("probabilities and confidence must be finite values in [0, 1]")
    if not math.isclose(sum(probabilities.values()), 1, abs_tol=1e-6, rel_tol=0):
        raise ValueError("probabilities must sum to one")
    choice = answer["choice"]
    if choice not in candidates or probabilities[choice] != max(probabilities.values()):
        raise ValueError("choice must be a maximum-probability candidate")
    if recording["provenance"] not in ("live", "synthetic"):
        raise ValueError("recording provenance must be live or synthetic")
    if recording["provenance"] == "live":
        for key in ("latency_ms", "cost_usd"):
            value = recording[key]
            if type(value) not in (int, float) or not math.isfinite(value) or value < 0:
                raise ValueError("live recordings require measured latency and cost")
        if not recording["response"].get("model") or not recording["response"].get("usage"):
            raise ValueError("live recordings require model and usage metadata")
        if not request["state"].get("target_repo") or not request["state"].get("target_ref"):
            raise ValueError("live recordings must identify their target revision")

    # Confidence controls trust, not a probability that the policy is correct.
    trust = (1 - exploration) * answer["confidence"]
    weights = {key: (1 - trust) / len(candidates) + trust * probabilities[key]
               for key in candidates}
    schedules = {policy: [] for policy in POLICIES}
    names = list(candidates)
    for index in range(slices):
        # Versioned SHA-256 draws make plans independent of Python's RNG version.
        digest = hashlib.sha256(f"jev-plan-v1:{seed}:{index}".encode()).digest()
        draw = int.from_bytes(digest[:8], "big") / 2**64
        cumulative = 0
        weighted = names[-1]
        for name in names:
            cumulative += weights[name]
            if draw < cumulative:
                weighted = name
                break
        selected = {"baseline": "base", "uniform": names[min(int(draw * len(names)), len(names) - 1)],
                    "round-robin": names[(seed + index) % len(names)], "greedy": choice,
                    "weighted": weighted}
        # All policies get the same engine seed for a given campaign slice.
        engine_seed = "0x" + digest[8:].hex()
        for policy, name in selected.items():
            schedules[policy].append({"candidate": name, "depth": candidates[name],
                                      "seed": engine_seed, "seconds": seconds})
    return {"version": 1, "recording": recording, "seed": seed, "slices": slices,
            "seconds": seconds, "exploration": exploration, "candidates": candidates,
            "weights": weights, "schedules": schedules}


def run_plan(plan, policy, runner, forge, target_repo, target_ref, scfuzzbench_ref, output):
    # Reject edited schedules or stale recordings before starting any campaign.
    expected = compile_plan(plan["recording"], plan["seed"], plan["slices"],
                            plan["seconds"], plan["exploration"])
    if plan != expected:
        raise ValueError("plan differs from its deterministic reconstruction")
    for ref in (target_ref, scfuzzbench_ref):
        if len(ref) != 40 or any(c not in "0123456789abcdef" for c in ref):
            raise ValueError("benchmark and target refs must be full commit SHAs")
    state = plan["recording"]["request"]["state"]
    if plan["recording"]["provenance"] == "live" and (
        state["target_repo"] != target_repo or state["target_ref"] != target_ref
    ):
        raise ValueError("live recording belongs to a different target revision")
    output.mkdir(parents=True, exist_ok=False)
    (output / "plan.json").write_text(json.dumps(plan, indent=2) + "\n")
    for index, item in enumerate(plan["schedules"][policy]):
        # Independent slices intentionally have no cross-slice corpus or failure reuse.
        destination = output / str(index)
        test_args = (f'--fuzz-seed {item["seed"]} --invariant-depth {item["depth"]} '
                     '--invariant-workers 1 --show-progress')
        command = [str(runner), "--foundry-bin", str(forge), "--target-repo", target_repo,
                   "--target-ref", target_ref, "--scfuzzbench-ref", scfuzzbench_ref,
                   "--benchmark-type", "property", "--workers", "1", "--timeout-seconds",
                   str(item["seconds"]), f"--foundry-test-args={test_args}",
                   "--output-dir", str(destination)]
        (output / f"{index}-command.json").write_text(json.dumps(command, indent=2) + "\n")
        with (output / f"{index}-runner.log").open("w") as log:
            subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    build = commands.add_parser("compile")
    build.add_argument("recording", type=Path)
    build.add_argument("output", type=Path)
    build.add_argument("--seed", type=int, required=True)
    build.add_argument("--slices", type=int, required=True)
    build.add_argument("--seconds", type=int, required=True)
    build.add_argument("--exploration", type=float, required=True)
    run = commands.add_parser("run")
    run.add_argument("plan", type=Path)
    run.add_argument("output", type=Path)
    run.add_argument("--policy", choices=POLICIES, required=True)
    run.add_argument("--runner", type=Path, required=True)
    run.add_argument("--forge", type=Path, required=True)
    run.add_argument("--target-repo", required=True)
    run.add_argument("--target-ref", required=True)
    run.add_argument("--scfuzzbench-ref", required=True)
    args = parser.parse_args()
    if args.command == "compile":
        plan = compile_plan(json.loads(args.recording.read_text()), args.seed, args.slices,
                            args.seconds, args.exploration)
        with args.output.open("x") as output:
            output.write(json.dumps(plan, indent=2) + "\n")
    else:
        run_plan(json.loads(args.plan.read_text()), args.policy, args.runner.resolve(),
                 args.forge.resolve(), args.target_repo, args.target_ref,
                 args.scfuzzbench_ref, args.output.resolve())


if __name__ == "__main__":
    main()
