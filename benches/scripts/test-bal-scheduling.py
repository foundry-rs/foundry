#!/usr/bin/env python3
"""Exercise campaign scheduling through the real runner and a synthetic Cast child.

Requires a built foundry-cast-run-bench; no Anvil or external RPC is needed.
Invocation counts verify scheduling, not real-provider performance.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


RUNNER = Path(__file__).resolve().parents[2] / "target/debug/foundry-cast-run-bench"
BLOCK_HASH = "0x" + "11" * 32
PARENT_HASH = "0x" + "22" * 32
TRANSACTION_HASH = "0x" + "33" * 32
FAKE_CAST = r'''
import json
from pathlib import Path
import sys
import urllib.error
import urllib.request

if sys.argv[1:] == ['--version']:
    print('cast campaign fixture 1.0')
    raise SystemExit(0)
if sys.argv[1:] == ['run', '--help']:
    print('Options: --no-bal')
    raise SystemExit(0)
settings = json.loads(Path(__file__).with_suffix('.json').read_text())
transaction = sys.argv[2]
failure = settings.get('failure') if transaction == settings.get('failure_transaction') else None
trace = '-vvvvv' in sys.argv
arm = 'replay' if '--no-bal' in sys.argv else 'auto'
if arm == 'auto':
    endpoint = sys.argv[sys.argv.index('--rpc-url') + 1]
    payload = {'jsonrpc': '2.0', 'id': 1,
               'method': 'eth_getBlockAccessListByBlockHash',
               'params': [settings['block_hash']]}
    if failure == 'bal_batch':
        payload = [payload]
    body = json.dumps(payload).encode()
    request = urllib.request.Request(endpoint, body, {'Content-Type': 'application/json'})
    try:
        with urllib.request.urlopen(request, timeout=5) as response:
            if 'error' in json.load(response):
                arm = 'miss'
    except urllib.error.HTTPError as error:
        if failure != 'bal_batch' or error.code != 400:
            raise
        # Cast also falls back to replay when its BAL request fails.
        arm = 'miss'
log = Path(settings['log'])
prior = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
ordinary = sum(event['ref'] == settings['ref'] and event['arm'] == arm
               and event['transaction'] == transaction and not event['trace'] for event in prior)
with log.open('a') as output:
    output.write(json.dumps({'ref': settings['ref'], 'arm': arm, 'trace': trace,
                             'transaction': transaction}) + '\n')
if arm in {'replay', 'miss'}:
    print('Executing previous transactions from the block.', file=sys.stderr)
if failure == 'oracle_failure' and not trace and arm == 'replay' and ordinary == 0:
    raise SystemExit(1)
print('Transaction successfully executed.')
print('Gas used: 21000')
# Trace and ordinary output deliberately differ, as do the two binaries.
print(('Full trace: ' if trace else 'Output: ') + settings['ref'] + transaction)
if failure == 'trace_mismatch' and trace and arm == 'auto':
    print('incorrect trace')
if failure == 'late_mismatch' and not trace and arm == 'auto' and ordinary > 0:
    print('incorrect later output')
'''


class CampaignTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="foundry-bal-campaign-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.log = self.root / "cast.jsonl"
        self.panel = self.root / "panel.json"
        self.panel.write_text(json.dumps({
            "schema_version": 1, "endpoint_label": "synthetic-campaign", "chain_id": 31337,
            "client_version": "fixture", "source_context": {"synthetic": True}, "seed": 0,
            "blocks": [{
                "block_hash": BLOCK_HASH, "block_number": 1, "parent_hash": PARENT_HASH,
                "stratum": "fixture", "targets": [{
                    "id": "fixture", "tx_hash": TRANSACTION_HASH, "index": 0,
                }],
            }],
            "cases": [{
                "id": "fixture", "transaction_hash": TRANSACTION_HASH, "block_hash": BLOCK_HASH,
                "index": 0, "positions": ["first"], "stratum": "fixture",
                "expected_receipt_gas": 21000, "expected_receipt_status": True, "bal_response": [],
            }],
        }))

        panel_path = self.panel

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_POST(self):
                request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                if request["method"] == "eth_chainId":
                    result = "0x7a69"
                elif request["method"] == "eth_getBlockByNumber":
                    targets = json.loads(panel_path.read_text())["blocks"][0]["targets"]
                    result = {"hash": BLOCK_HASH, "parentHash": PARENT_HASH,
                              "transactions": [target["tx_hash"] for target in targets]}
                else:
                    self.send_error(400, "unexpected upstream method")
                    return
                body = json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.stop_server)
        self.cast = {}
        for label in ("base", "candidate"):
            binary = self.root / f"cast-{label}"
            binary.write_text(f"#!{sys.executable}\n" + FAKE_CAST)
            binary.chmod(0o755)
            binary.with_suffix(".json").write_text(json.dumps({
                "ref": label, "block_hash": BLOCK_HASH, "log": str(self.log),
            }))
            self.cast[label] = binary

    def stop_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()

    def fail_candidate(self, failure):
        path = self.cast["candidate"].with_suffix(".json")
        settings = json.loads(path.read_text())
        settings["failure"] = failure
        settings["failure_transaction"] = TRANSACTION_HASH
        path.write_text(json.dumps(settings))

    def run_campaign(self, rounds=10, warmups=2, include_miss=False, baseline=True):
        output = self.root / "results"
        command = [str(RUNNER), "run", "--manifest", str(self.panel),
                   "--cast", str(self.cast["candidate"]), "--output-dir", str(output),
                   "--rounds", str(rounds), "--warmup-rounds", str(warmups),
                   "--rpc-env", "BAL_CAMPAIGN_TEST_RPC", "--timeout-seconds", "5"]
        if baseline:
            command.extend(["--baseline-cast", str(self.cast["base"])])
        if include_miss:
            command.append("--include-miss")
        environment = {**os.environ,
                       "BAL_CAMPAIGN_TEST_RPC": f"http://127.0.0.1:{self.server.server_port}"}
        result = subprocess.run(command, env=environment, capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        return output

    @staticmethod
    def samples(output):
        return [json.loads(line) for line in (output / "samples.jsonl").read_text().splitlines()]

    def test_default_campaign_validates_once_and_alternates_rounds(self):
        output = self.run_campaign()
        events = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual(len(events), 54)
        self.assertEqual([(event["ref"], event["arm"], event["trace"]) for event in events[:6]], [
            (label, arm, trace)
            for label in ("base", "candidate")
            for arm, trace in (("auto", True), ("replay", True), ("replay", False))
        ])
        measured = []
        for label in ("base", "candidate"):
            samples = self.samples(output / label / "aggregate")
            self.assertEqual(len(samples), 27)
            self.assertEqual(sum(sample["phase"] == "validation" for sample in samples), 2)
            self.assertEqual(sum(sample["phase"] == "oracle" for sample in samples), 1)
            self.assertEqual(sum(sample["phase"] == "warmup" for sample in samples), 4)
            measured.extend(sample for sample in samples if sample["phase"] == "measured")
            self.assertTrue(all(sample["correctness"] == "equivalent"
                                for sample in samples if sample["phase"] != "oracle"))
        self.assertEqual(len(measured), 40)
        # Remove setup per ref; remaining child order must alternate refs and arms by round.
        ordinary_seen = set()
        sampled_events = []
        for event in events:
            if event["trace"]:
                continue
            if event["ref"] not in ordinary_seen:
                self.assertEqual(event["arm"], "replay")
                ordinary_seen.add(event["ref"])
            else:
                sampled_events.append((event["ref"], event["arm"]))
        expected = [
            (label, arm)
            for count in (2, 10)
            for round_index in range(count)
            for label in (("base", "candidate") if round_index % 2 == 0 else ("candidate", "base"))
            for arm in (("auto", "replay") if round_index % 2 == 0 else ("replay", "auto"))
        ]
        self.assertEqual(sampled_events, expected)
        schedule = json.loads((output / "schedule.json").read_text())
        self.assertEqual(schedule["execution_order"], [
            {"ref": label, "phase": phase, "round": round_index}
            for phase, count in (("warmup", 2), ("measured", 10))
            for round_index in range(count)
            for label in (("base", "candidate") if round_index % 2 == 0 else ("candidate", "base"))
        ])

    def test_miss_arm_reuses_its_validation(self):
        output = self.run_campaign(include_miss=True)
        self.assertEqual(len(self.log.read_text().splitlines()), 80)
        for label in ("base", "candidate"):
            samples = self.samples(output / label / "aggregate")
            self.assertEqual([sample["arm"] for sample in samples if sample["phase"] == "validation"],
                             ["auto", "miss", "replay"])
            self.assertEqual(sum(sample["phase"] == "measured" for sample in samples), 30)
            self.assertTrue(all(sample["correctness"] == "equivalent"
                                for sample in samples if sample["phase"] != "oracle"))

    def test_failed_validation_does_not_borrow_another_refs_oracle(self):
        self.fail_candidate("trace_mismatch")
        output = self.run_campaign(rounds=2, warmups=0)
        base = self.samples(output / "base/aggregate")
        candidate = self.samples(output / "candidate/aggregate")
        self.assertEqual(sum(sample["phase"] == "oracle" for sample in candidate), 0)
        self.assertTrue(all(sample["correctness"] == "equivalent"
                            for sample in base if sample["phase"] == "measured"))
        self.assertTrue(all(sample["correctness"] == "correctness_blocked" for sample in candidate))
        summary = json.loads((output / "candidate/aggregate/summary.json").read_text())
        self.assertIsNone(summary["comparisons"][0]["speedup"])

    def test_oracles_remain_per_case_and_uncaptured_cases_have_no_children(self):
        self.fail_candidate("oracle_failure")
        panel = json.loads(self.panel.read_text())
        for index, label in enumerate(("healthy", "uncaptured"), start=1):
            transaction = "0x" + f"{index + 3:02x}" * 32
            panel["blocks"][0]["targets"].append({"id": label, "tx_hash": transaction, "index": index})
            case = {**panel["cases"][0], "id": label, "transaction_hash": transaction, "index": index}
            if label == "uncaptured":
                case.update({"capture_error": "fixture receipt unavailable", "expected_receipt_gas": None})
            panel["cases"].append(case)
        self.panel.write_text(json.dumps(panel))
        output = self.run_campaign(rounds=2, warmups=0)
        samples = self.samples(output / "candidate/aggregate")
        self.assertEqual(sum(sample["phase"] == "oracle" for sample in samples), 2)
        for case_id, correctness in (("fixture", "correctness_blocked"), ("healthy", "equivalent"),
                                     ("uncaptured", "capture_incomplete")):
            measured = [sample for sample in samples
                        if sample["phase"] == "measured" and sample["case_id"] == case_id]
            self.assertEqual(len(measured), 4)
            self.assertTrue(all(sample["correctness"] == correctness for sample in measured))
        events = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual(len(events), 28)
        self.assertNotIn(panel["cases"][2]["transaction_hash"], {event["transaction"] for event in events})
        self.assertEqual([(event["ref"], event["transaction"], event["arm"]) for event in events[12:]], [
            (label, case["transaction_hash"], arm)
            for labels, arms in ((("base", "candidate"), ("auto", "replay")),
                                 (("candidate", "base"), ("replay", "auto")))
            for label in labels
            for case in panel["cases"][:2]
            for arm in arms
        ])
        comparisons = {item["case_id"]: item for item in
                       json.loads((output / "candidate/aggregate/summary.json").read_text())["comparisons"]}
        self.assertIsNone(comparisons["fixture"]["speedup"])
        self.assertIsNone(comparisons["uncaptured"]["speedup"])
        self.assertGreater(comparisons["healthy"]["speedup"], 0)

    def test_single_ref_still_checks_later_outputs_against_oracle(self):
        self.fail_candidate("late_mismatch")
        output = self.run_campaign(rounds=2, warmups=0, baseline=False)
        samples = self.samples(output)
        self.assertEqual(len(samples), 7)
        auto = [sample for sample in samples if sample["phase"] == "measured" and sample["arm"] == "auto"]
        self.assertEqual([sample["correctness"] for sample in auto], ["equivalent", "correctness_blocked"])
        summary = json.loads((output / "summary.json").read_text())
        self.assertIsNone(summary["comparisons"][0]["speedup"])

    def test_rejected_bal_batch_cannot_publish_successful_fallback(self):
        self.fail_candidate("bal_batch")
        output = self.run_campaign(rounds=1, warmups=0, baseline=False, include_miss=True)
        samples = self.samples(output)
        rejected = [sample for sample in samples if sample["arm"] in {"auto", "miss"}]
        self.assertEqual(len(rejected), 4)
        for sample in rejected:
            self.assertEqual(sample["exit_code"], 0)
            self.assertEqual(sample["actual_path"], "replay_after_probe")
            self.assertEqual(len(sample["bal_events"]), 1)
            event = sample["bal_events"][0]
            self.assertEqual(event["http_status"], 400)
            self.assertEqual(event["issues"], ["unsupported_bal_batch_injection"])
            self.assertEqual(sample["rpc"]["upstream_http_exchanges"], 0)
            self.assertEqual(sample["rpc"]["injected_responses"], 0)
            self.assertEqual(sample["correctness"], "invalid_path")
        summary = json.loads((output / "summary.json").read_text())
        comparison = summary["comparisons"][0]
        self.assertEqual(comparison["valid_pairs"], 0)
        self.assertEqual(comparison["invalid_pairs"], 1)
        for field in ("speedup", "auto_wall_seconds", "replay_wall_seconds",
                      "paired_wall_delta_seconds", "rpc_delta_per_pair"):
            self.assertIsNone(comparison[field])
        self.assertFalse((output / "common-results.json").exists())


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runner", type=Path, default=RUNNER)
    arguments, remaining = parser.parse_known_args()
    RUNNER = arguments.runner.resolve()
    unittest.main(argv=[sys.argv[0], *remaining])
