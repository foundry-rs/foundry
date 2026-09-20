#!/usr/bin/env python3
"""Offline correctness checks for captured BAL campaigns; no Foundry build required."""

from contextlib import redirect_stderr
import gzip
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import cast_bal_campaign as campaign
from cast_bal_transport import Journal, LocalClient, RecordedFixture, STATE_METHODS, request_key, start_server, write_json


FIXTURES = Path(__file__).resolve().parents[1] / "fixtures/cast-bal"
BLOCK_HASH = "0x" + "11" * 32
PARENT_HASH = "0x" + "22" * 32
TRANSACTION_HASH = "0x" + "33" * 32
ADDRESS = "0x" + "44" * 20


class CampaignTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="foundry-cast-bal-campaign-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.case = {
            "id": "historical-t0", "transaction_hash": TRANSACTION_HASH,
            "block_hash": BLOCK_HASH, "index": 0,
            "expected_receipt_gas": 21000, "expected_receipt_status": True,
        }
        self.frozen = {"block_number": 2, "block_hash": BLOCK_HASH, "parent_hash": PARENT_HASH}
        self.panel = {"schema_version": 1, "blocks": [self.frozen], "cases": [self.case]}
        self.block = {
            "hash": BLOCK_HASH, "number": "0x2", "parentHash": PARENT_HASH,
            "transactions": [{"hash": TRANSACTION_HASH}],
        }
        self.parent = {"hash": PARENT_HASH, "number": "0x1", "transactions": []}
        self.receipt = {
            "transactionHash": TRANSACTION_HASH, "blockHash": BLOCK_HASH,
            "transactionIndex": "0x0", "gasUsed": "0x5208", "status": "0x1",
        }
        self.records = [
            {"method": "eth_getBlockByHash", "params": [BLOCK_HASH, True], "result": self.block},
            {"method": "eth_getBlockByNumber", "params": ["0x1", True], "result": self.parent},
            {"method": "eth_getBlockAccessListByBlockHash", "params": [BLOCK_HASH],
             "result": [{"address": ADDRESS, "storageReads": [], "storageChanges": []}]},
            {"method": "eth_getTransactionReceipt", "params": [TRANSACTION_HASH], "result": self.receipt},
            {"method": "eth_getBalance", "params": [ADDRESS, PARENT_HASH], "result": "0x10"},
            {"method": "eth_getStorageAt", "params": [ADDRESS, "0x01", PARENT_HASH], "result": "0xff"},
        ]
        self.fixture_path = self.root / "block-2.json.gz"
        self.write_fixture()
        self.write_panel()

    def write_fixture(self):
        with gzip.open(self.fixture_path, "wt") as stream:
            json.dump({"schema_version": 1, "block_number": 2, "parent_hash": PARENT_HASH,
                       "records": self.records}, stream)

    def write_panel(self):
        write_json(self.root / "panel.json", self.panel)
        write_json(self.root / "manifest.json", {
            "schema_version": 1, "panel_sha256": campaign.sha256(self.root / "panel.json"),
            "blocks": [{"block_number": 2, "path": self.fixture_path.name,
                        "sha256": campaign.sha256(self.fixture_path), "records": len(self.records)}],
        })

    def fixture(self):
        return RecordedFixture(self.fixture_path, Journal(self.root))

    def payload(self, method="eth_getBalance", params=None):
        return {"jsonrpc": "2.0", "id": 7, "method": method,
                "params": [ADDRESS, PARENT_HASH] if params is None else params}

    def test_load_panel_verifies_checked_in_fixtures_and_authentic_parent_state(self):
        panel = campaign.load_panel(FIXTURES)
        manifest = json.loads((FIXTURES / "manifest.json").read_text())
        self.assertEqual(len(panel["cases"]), 6)
        self.assertEqual(len(panel["blocks"]), len(manifest["blocks"]))
        for frozen, recorded in zip(panel["blocks"], manifest["blocks"]):
            fixture = RecordedFixture(FIXTURES / recorded["path"], Journal(self.root))
            with gzip.open(FIXTURES / recorded["path"], "rt") as stream:
                raw = json.load(stream)
            cases = [case for case in panel["cases"] if case["block_hash"] == frozen["block_hash"]]
            block, parent, bal, receipts = campaign.validate_metadata(fixture, frozen, cases)
            self.assertEqual(fixture.block_number, frozen["block_number"])
            self.assertEqual(len(raw["records"]), recorded["records"])
            self.assertEqual(parent["hash"], frozen["parent_hash"])
            self.assertEqual(len(receipts), 3)
            self.assertTrue(block["transactions"] and bal)
            state_reads = 0
            for record in raw["records"]:
                if record["method"] in STATE_METHODS:
                    self.assertEqual(record["params"][-1], parent["hash"])
                    state_reads += 1
            self.assertGreater(state_reads, 0)

    def test_load_panel_rejects_modified_panel_or_fixture(self):
        for path in (self.root / "panel.json", self.fixture_path):
            with self.subTest(path=path.name):
                original = path.read_bytes()
                path.write_bytes(original + b"\n")
                with self.assertRaisesRegex(ValueError, "integrity"):
                    campaign.load_panel(self.root)
                path.write_bytes(original)

    def test_load_panel_rejects_escaping_fixture_path(self):
        manifest_path = self.root / "manifest.json"
        manifest = json.loads(manifest_path.read_text())
        manifest["blocks"][0]["path"] = "../block-2.json.gz"
        write_json(manifest_path, manifest)
        with self.assertRaises(ValueError):
            campaign.load_panel(self.root)

    def test_load_panel_rejects_unsafe_or_duplicate_case_ids(self):
        for identifier in ("../escape", "two/parts", "", "bad\nname"):
            with self.subTest(identifier=identifier):
                self.case["id"] = identifier
                self.write_panel()
                with self.assertRaisesRegex(ValueError, "case identifier"):
                    campaign.load_panel(self.root)
        self.case["id"] = "safe"
        self.panel["cases"].append(dict(self.case))
        self.write_panel()
        with self.assertRaisesRegex(ValueError, "case identifier"):
            campaign.load_panel(self.root)

    def test_load_panel_rejects_unknown_case_block(self):
        self.case["block_hash"] = PARENT_HASH
        self.write_panel()
        with self.assertRaisesRegex(ValueError, "Case block absent"):
            campaign.load_panel(self.root)

    def test_fixture_normalizes_hex_case_and_storage_key_padding(self):
        fixture = self.fixture()
        self.assertEqual(fixture.get("eth_getStorageAt", [ADDRESS.upper().replace("0X", "0x"),
                                                        "0x0001", PARENT_HASH]), "0xff")

    def test_fixture_rejects_conflicting_recordings(self):
        self.records.append(dict(self.records[-1], params=[ADDRESS, "0x0001", PARENT_HASH], result="0x00"))
        self.write_fixture()
        with self.assertRaisesRegex(ValueError, "Conflicting recorded response"):
            self.fixture()

    def test_account_info_projects_authentic_balance_code_and_nonce(self):
        self.records.append({
            "method": "eth_getAccountInfo", "params": [ADDRESS, PARENT_HASH],
            "result": {"balance": "0x10", "code": "0x6000", "nonce": "0x3"},
        })
        self.write_fixture()
        fixture = self.fixture()
        for method, expected in (("eth_getBalance", "0x10"), ("eth_getCode", "0x6000"),
                                 ("eth_getTransactionCount", "0x3")):
            with self.subTest(method=method):
                self.assertEqual(fixture.dispatch(self.payload(method, [ADDRESS, "0x1"]))["result"], expected)
        self.records[-1]["result"]["balance"] = "0xff"
        self.write_fixture()
        with self.assertRaisesRegex(ValueError, "Conflicting captured account field"):
            self.fixture()

    def test_account_info_reconstruction_requires_all_captured_fields(self):
        self.records.extend([
            {"method": "eth_getCode", "params": [ADDRESS, PARENT_HASH], "result": "0x"},
            {"method": "eth_getTransactionCount", "params": [ADDRESS, PARENT_HASH], "result": "0x0"},
        ])
        self.write_fixture()
        fixture = self.fixture()
        expected = {"balance": "0x10", "code": "0x", "nonce": "0x0"}
        self.assertEqual(fixture.get("eth_getAccountInfo", [ADDRESS, "0x1"]), expected)
        self.assertEqual(fixture.get("eth_getAccountInfo", [ADDRESS, {"blockHash": PARENT_HASH}]), expected)
        for method in ("eth_getBalance", "eth_getCode", "eth_getTransactionCount"):
            with self.subTest(missing=method):
                incomplete = self.fixture()
                del incomplete.records[request_key(method, [ADDRESS, PARENT_HASH])]
                response = incomplete.dispatch(self.payload("eth_getAccountInfo"))
                self.assertIn("Missing captured RPC response", response["error"]["message"])
                self.assertNotIn("result", response)
                self.assertEqual(len(incomplete.failures), 1)
        with self.assertRaisesRegex(ValueError, "Missing captured RPC response"):
            fixture.get("eth_getAccountInfo", [ADDRESS, BLOCK_HASH])

    def test_missing_response_fails_without_remote_fallback(self):
        fixture = self.fixture()
        response = fixture.dispatch(self.payload("eth_chainId", []))
        self.assertEqual(response["id"], 7)
        self.assertIn("Missing captured RPC response", response["error"]["message"])
        self.assertEqual(len(fixture.failures), 1)
        self.assertEqual(fixture.failures[0]["source"], "missing-captured-response")

    def test_parent_guard_rejects_recorded_current_state(self):
        self.records.append({"method": "eth_getBalance", "params": [ADDRESS, BLOCK_HASH], "result": "0xff"})
        self.write_fixture()
        fixture = self.fixture()
        response = fixture.dispatch(self.payload(params=[ADDRESS, BLOCK_HASH]))
        self.assertIn("error", response)
        self.assertNotIn("result", response)
        self.assertEqual(len(fixture.failures), 1)
        self.assertEqual(fixture.state_requests, [{"method": "eth_getBalance", "params": [ADDRESS, BLOCK_HASH]}])

    def test_parent_guard_allows_parent_number_hash_and_eip1898(self):
        tags = ["0x1", PARENT_HASH, {"blockHash": PARENT_HASH, "requireCanonical": True}]
        self.records.extend({"method": "eth_getBalance", "params": [ADDRESS, tag], "result": "0x10"}
                            for tag in tags)
        self.write_fixture()
        fixture = self.fixture()
        for tag in tags:
            with self.subTest(tag=tag):
                self.assertEqual(fixture.dispatch(self.payload(params=[ADDRESS, tag]))["result"], "0x10")
        self.assertEqual(fixture.failures, [])

    def test_offline_barrier_rejects_previously_available_responses(self):
        fixture = self.fixture()
        self.assertEqual(fixture.dispatch(self.payload())["result"], "0x10")
        fixture.close_barrier()
        fixture.close_barrier()
        response = fixture.dispatch(self.payload())
        self.assertIn("after preparation", response["error"]["message"])
        self.assertEqual(fixture.barrier_counts, {"eth_getBalance": 1})
        self.assertEqual(fixture.failures[0]["source"], "offline-barrier")
        self.assertEqual(fixture.counts, {"eth_getBalance": 2})

    def test_loopback_batch_round_trip_preserves_success_and_error_ids(self):
        fixture = self.fixture()
        server = start_server(fixture.dispatch, fixture.journal)
        client = LocalClient()
        try:
            response = client.request(server.server_port, [self.payload(), self.payload("missing", [])])
            self.assertEqual(response[0]["result"], "0x10")
            self.assertEqual(response[1]["id"], 7)
            self.assertIn("error", response[1])
        finally:
            client.close()
            server.shutdown()
            server.server_close()

    def test_failed_setup_preserves_campaign_identity_and_nonzero_status(self):
        output = self.root / "output"
        argv = ["--fixture-dir", str(self.root / "missing"), "--output-dir", str(output),
                "--base-sha", "a" * 40, "--head-sha", "b" * 40,
                "--anvil", "/missing/anvil", "--runner", "/missing/runner",
                "--base-cast", "/missing/base", "--head-cast", "/missing/head"]
        with redirect_stderr(io.StringIO()):
            self.assertEqual(campaign.main(argv), 1)
        result = json.loads((output / "campaign.json").read_text())
        self.assertEqual(result["status"], "failed")
        self.assertEqual(result["refs"], {"base": "a" * 40, "head": "b" * 40})
        self.assertIn("finished_at", result)

    def test_metadata_rejects_wrong_ancestry_receipt_and_empty_bal(self):
        mutations = (
            ("eth_getBlockByHash", [BLOCK_HASH, True], "parentHash", BLOCK_HASH),
            ("eth_getBlockByNumber", ["0x1", True], "number", "0x2"),
            ("eth_getTransactionReceipt", [TRANSACTION_HASH], "gasUsed", "0x0"),
            ("eth_getTransactionReceipt", [TRANSACTION_HASH], "status", "0x0"),
            ("eth_getTransactionReceipt", [TRANSACTION_HASH], "transactionIndex", "0x1"),
        )
        for method, params, field, value in mutations:
            with self.subTest(field=field):
                fixture = self.fixture()
                fixture.records[request_key(method, params)][field] = value
                with self.assertRaises(ValueError):
                    campaign.validate_metadata(fixture, self.frozen, [self.case])
        fixture = self.fixture()
        fixture.records[request_key("eth_getBlockAccessListByBlockHash", [BLOCK_HASH])] = []
        with self.assertRaisesRegex(ValueError, "BAL is absent or empty"):
            campaign.validate_metadata(fixture, self.frozen, [self.case])

    def prepare(self, outputs):
        fixture = self.fixture()
        results = iter(outputs)

        def run_process(_argv, root, _env, label, _timeout):
            stdout, stderr = next(results)
            (root / f"{label}.stdout").write_bytes(stdout)
            (root / f"{label}.stderr").write_bytes(stderr)
            return {"exit_code": 0, "timed_out": False, "elapsed_seconds": 0.01}

        with patch.object(campaign, "run_process", side_effect=run_process):
            campaign.prepare_cast(Path("cast"), "base", self.case, 1234, self.root, {}, 1, fixture)

    def test_preparation_requires_receipt_gas_status_and_identical_trace(self):
        good = b"[trace]\nTransaction successfully executed.\nGas used: 21000\n"
        self.prepare([(good, b""), (good, b"")])
        for stdout, stderr in ((good.replace(b"21000", b"21001"), b""),
                               (good, b"Error: Transaction failed.\n")):
            with self.subTest(stdout=stdout, stderr=stderr):
                with self.assertRaisesRegex(RuntimeError, "receipt checks"):
                    self.prepare([(stdout, stderr)])
        with self.assertRaisesRegex(RuntimeError, "traces differ"):
            self.prepare([(good, b""), (good.replace(b"[trace]", b"[different trace]"), b"")])

    def samples(self):
        samples = [{
            "id": f"historical-t0-{phase}-{arm}-{round_}", "case_id": self.case["id"],
            "phase": phase, "arm": arm, "round": round_, "correctness": "equivalent",
            "timed_out": False, "exit_code": 0,
            "actual_path": "bal_hit" if arm == "auto" else "replay_no_probe",
            "local_gas": 21000, "execution_success": True, "wall_time_seconds": 0.01,
            "stdout_sha256": "a" * 64,
        } for phase, count in (("validation", 1), ("warmup", 1), ("measured", 2))
           for arm in ("auto", "replay") for round_ in range(count)]
        samples.insert(0, dict(samples[1], id="historical-t0-oracle-replay-0", phase="oracle",
                               arm="replay", correctness="unchecked"))
        return samples

    def verify_samples(self, samples):
        for label in ("base", "candidate"):
            path = self.root / label / "aggregate/samples.jsonl"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("".join(json.dumps(sample) + "\n" for sample in samples))
        return campaign.verify_samples(self.root, [self.case], 2, 1)

    def test_samples_require_complete_schedule(self):
        samples = self.samples()
        valid = self.verify_samples(samples)
        self.assertEqual(valid["issues"], [])
        self.assertEqual(valid["samples"]["base"], {"all": 9, "measured": 4, "timeouts": 0})
        for invalid in (samples[:-1], samples + [samples[-1]],
                        samples[:-1] + [dict(samples[-1], round=0)]):
            with self.subTest(samples=invalid):
                self.assertTrue(self.verify_samples(invalid)["issues"])

    def test_samples_reject_wrong_path_receipt_status_and_timeout(self):
        mutations = (
            {"actual_path": "replay_miss"}, {"local_gas": 1}, {"execution_success": False},
            {"timed_out": True}, {"exit_code": 1}, {"correctness": "different"},
            {"wall_time_seconds": float("nan")}, {"wall_time_seconds": float("inf")},
            {"wall_time_seconds": -1},
        )
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                samples = self.samples()
                samples[-1].update(mutation)
                self.assertTrue(self.verify_samples(samples)["issues"])

    def test_samples_reject_unexpected_case_phase_or_arm(self):
        for mutation in ({"case_id": "unknown"}, {"phase": "ignored"}, {"arm": "miss"}):
            with self.subTest(mutation=mutation):
                samples = self.samples()
                samples.append(dict(samples[-1], **mutation))
                self.assertTrue(self.verify_samples(samples)["issues"])

    def test_trace_format_may_differ_between_individually_equivalent_revisions(self):
        samples = self.samples()
        self.verify_samples(samples)
        for sample in samples:
            sample["stdout_sha256"] = "b" * 64
        path = self.root / "candidate/aggregate/samples.jsonl"
        path.write_text(
            "".join(json.dumps(sample) + "\n" for sample in samples))
        self.assertEqual(campaign.verify_samples(self.root, [self.case], 2, 1)["issues"], [])
        samples[-1]["correctness"] = "mismatch"
        path.write_text("".join(json.dumps(sample) + "\n" for sample in samples))
        self.assertEqual(campaign.verify_samples(self.root, [self.case], 2, 1)["issues"],
                         [f"Invalid attempt: candidate/{samples[-1]['id']}"])

    @unittest.skipUnless(os.name == "posix", "Campaign subprocess groups require POSIX")
    def test_process_timeout_terminates_child_and_preserves_evidence(self):
        argv = [sys.executable, "-c", "import time; print('started', flush=True); time.sleep(30)"]
        result = campaign.run_process(argv, self.root, os.environ.copy(), "deadline", 0.2)
        self.assertTrue(result["timed_out"])
        self.assertLess(result["exit_code"], 0)
        self.assertLess(result["elapsed_seconds"], 10)
        self.assertEqual((self.root / "deadline.stdout").read_text(), "started\n")
        self.assertEqual((self.root / "deadline.stderr").read_text(), "")


if __name__ == "__main__":
    unittest.main()
