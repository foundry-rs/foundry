//! Binary-level agent-contract tests for `cast`.
//!
//! Pins the cross-cutting guarantees agents depend on: `--introspect`
//! produces a document conforming to `foundry:introspect@v1`, and under
//! `--machine` the help / parse-error paths emit a `foundry:envelope@v1`
//! envelope on stdout (never raw clap text).

use foundry_test_utils::agent_schema;
use serde_json::{Value, json};

// `cast --introspect` returns valid JSON conforming to
// `foundry:introspect@v1`.
casttest!(introspect_document_matches_v1_schema, |_prj, cmd| {
    let assert = cmd.arg("--introspect").assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let doc: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected single JSON document on stdout: {stdout}: {e}"));
    agent_schema::validate("foundry:introspect@v1", &doc);

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.is_empty(), "stderr must be empty under --introspect, got: {stderr}");
});

// `cast --machine --help` emits a success envelope on stdout (not raw clap
// help text).
casttest!(machine_mode_help_emits_success_envelope, |_prj, cmd| {
    let assert = cmd.args(["--machine", "--help"]).assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected single envelope on stdout: {stdout}: {e}"));
    assert_eq!(envelope["success"], true);
    assert!(envelope["data"]["help"].is_string(), "missing data.help: {envelope}");
    assert_eq!(envelope["errors"], json!([]));
    assert_eq!(envelope["warnings"], json!([]));
    agent_schema::validate("foundry:envelope@v1", &envelope);

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.is_empty(), "stderr must be empty under --machine, got: {stderr}");
});

// `cast --machine abi-encode` emits the encoded bytes inside an envelope.
casttest!(cast_abi_encode_machine_mode_emits_envelope, |_prj, cmd| {
    let assert = cmd
        .args([
            "--machine",
            "abi-encode",
            "transfer(address,uint256)",
            "0x0000000000000000000000000000000000000001",
            "42",
        ])
        .assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value =
        serde_json::from_str(stdout.trim()).expect("stdout is exactly one JSON envelope");

    assert_eq!(envelope["success"], true);
    assert_eq!(envelope["errors"], json!([]));
    assert_eq!(envelope["warnings"], json!([]));
    assert_eq!(
        envelope["data"]["encoded"],
        "0x0000000000000000000000000000000000000000000000000000000000000001\
         000000000000000000000000000000000000000000000000000000000000002a"
    );
    agent_schema::validate_envelope_data(&envelope, "foundry:cast.abi-encode@v1");

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.is_empty(), "stderr must be empty under --machine, got: {stderr}");
});

// `cast --machine abi-encode --packed` emits packed (non-padded) bytes.
casttest!(cast_abi_encode_packed_machine_mode_emits_envelope, |_prj, cmd| {
    let assert = cmd
        .args(["--machine", "abi-encode", "--packed", "f(uint8,uint16)", "1", "2"])
        .assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value =
        serde_json::from_str(stdout.trim()).expect("stdout is exactly one JSON envelope");

    assert_eq!(envelope["success"], true);
    assert_eq!(envelope["data"]["encoded"], "0x010002");
    agent_schema::validate_envelope_data(&envelope, "foundry:cast.abi-encode@v1");

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.is_empty(), "stderr must be empty under --machine, got: {stderr}");
});

// `cast --machine abi-decode` emits the formatted token list.
casttest!(cast_abi_decode_machine_mode_emits_envelope, |_prj, cmd| {
    let assert = cmd
        .args([
            "--machine",
            "abi-decode",
            "balanceOf(address)(uint256)",
            "0x000000000000000000000000000000000000000000000000000000000000002a",
        ])
        .assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value =
        serde_json::from_str(stdout.trim()).expect("stdout is exactly one JSON envelope");

    assert_eq!(envelope["success"], true);
    assert_eq!(envelope["errors"], json!([]));
    assert_eq!(envelope["warnings"], json!([]));
    assert_eq!(envelope["data"]["decoded"], json!(["42"]));
    agent_schema::validate_envelope_data(&envelope, "foundry:cast.abi-decode@v1");

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.is_empty(), "stderr must be empty under --machine, got: {stderr}");
});

// `cast --machine abi-decode --input` decodes against the input types.
casttest!(cast_abi_decode_input_machine_mode_emits_envelope, |_prj, cmd| {
    let assert = cmd
        .args([
            "--machine",
            "abi-decode",
            "--input",
            "transfer(address,uint256)",
            "0x0000000000000000000000000000000000000000000000000000000000000001\
             000000000000000000000000000000000000000000000000000000000000002a",
        ])
        .assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value =
        serde_json::from_str(stdout.trim()).expect("stdout is exactly one JSON envelope");

    assert_eq!(envelope["success"], true);
    assert_eq!(
        envelope["data"]["decoded"],
        json!(["0x0000000000000000000000000000000000000001", "42"])
    );
    agent_schema::validate_envelope_data(&envelope, "foundry:cast.abi-decode@v1");

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.is_empty(), "stderr must be empty under --machine, got: {stderr}");
});

// `cast --machine keccak <arg>` emits the 0x-prefixed hash.
casttest!(cast_keccak_machine_mode_emits_envelope, |_prj, cmd| {
    let assert = cmd.args(["--machine", "keccak", "foundry"]).assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value =
        serde_json::from_str(stdout.trim()).expect("stdout is exactly one JSON envelope");

    assert_eq!(envelope["success"], true);
    assert_eq!(envelope["errors"], json!([]));
    assert_eq!(envelope["warnings"], json!([]));
    assert_eq!(
        envelope["data"]["hash"],
        "0x4eb2f10301a3ed7f2c31091074ca429f73cb8c51539e1a0005e132f70b8bb74a"
    );
    agent_schema::validate_envelope_data(&envelope, "foundry:cast.keccak@v1");

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.is_empty(), "stderr must be empty under --machine, got: {stderr}");
});

// `cast --machine keccak` with no argument hashes stdin bytes.
casttest!(cast_keccak_machine_mode_reads_stdin, |_prj, cmd| {
    let assert = cmd.args(["--machine", "keccak"]).stdin("foundry").assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value =
        serde_json::from_str(stdout.trim()).expect("stdout is exactly one JSON envelope");

    assert_eq!(envelope["success"], true);
    assert_eq!(
        envelope["data"]["hash"],
        "0x4eb2f10301a3ed7f2c31091074ca429f73cb8c51539e1a0005e132f70b8bb74a"
    );
    agent_schema::validate_envelope_data(&envelope, "foundry:cast.keccak@v1");

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.is_empty(), "stderr must be empty under --machine, got: {stderr}");
});

// `cast --machine 4byte` echoes the selector with openchain.xyz signatures.
casttest!(flaky_cast_4byte_machine_mode_emits_envelope, |_prj, cmd| {
    let assert = cmd.args(["--machine", "4byte", "0xa9059cbb"]).assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value =
        serde_json::from_str(stdout.trim()).expect("stdout is exactly one JSON envelope");

    assert_eq!(envelope["success"], true);
    assert_eq!(envelope["errors"], json!([]));
    assert_eq!(envelope["warnings"], json!([]));
    assert_eq!(envelope["data"]["selector"], "0xa9059cbb");
    let signatures = envelope["data"]["signatures"].as_array().expect("signatures is an array");
    assert!(
        signatures.iter().any(|s| s.as_str() == Some("transfer(address,uint256)")),
        "expected `transfer(address,uint256)` in signatures: {envelope}"
    );
    agent_schema::validate_envelope_data(&envelope, "foundry:cast.4byte@v1");

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.is_empty(), "stderr must be empty under --machine, got: {stderr}");
});

// `cast --machine 4byte` without a positional selector exits with
// `cli.usage.invalid` (the stdin fallback is disabled under `--machine`).
casttest!(cast_4byte_machine_mode_rejects_stdin, |_prj, cmd| {
    let assert = cmd.args(["--machine", "4byte"]).stdin("0xa9059cbb").assert_failure();
    assert_eq!(assert.get_output().status.code(), Some(2));
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value = serde_json::from_str(stdout.trim()).expect("error envelope on stdout");

    assert_eq!(envelope["success"], false);
    assert!(envelope["data"].is_null(), "data must be null on failure: {envelope}");
    assert_eq!(envelope["warnings"], json!([]));
    assert_eq!(envelope["errors"][0]["code"], "cli.usage.invalid");
    agent_schema::validate("foundry:envelope@v1", &envelope);

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.is_empty(), "stderr must be empty under --machine, got: {stderr}");
});

// `cast --machine <bad-flag>` emits a typed `cli.usage.invalid` error
// envelope and exits with `ExitCode::Usage` (2) — never raw clap text.
casttest!(machine_mode_unknown_flag_emits_typed_envelope, |_prj, cmd| {
    let assert = cmd.args(["--machine", "--this-flag-does-not-exist"]).assert_failure();
    assert_eq!(assert.get_output().status.code(), Some(2));
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected single envelope on stdout: {stdout}: {e}"));
    assert_eq!(envelope["success"], false);
    assert!(envelope["data"].is_null(), "data must be null on failure: {envelope}");
    assert_eq!(envelope["errors"].as_array().map(Vec::len), Some(1));
    assert_eq!(envelope["errors"][0]["code"], "cli.usage.invalid");
    assert_eq!(envelope["warnings"], json!([]));
    agent_schema::validate("foundry:envelope@v1", &envelope);

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.is_empty(), "stderr must be empty under --machine, got: {stderr}");
});
