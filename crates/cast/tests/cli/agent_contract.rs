//! Binary-level agent-contract tests for `cast`.
//!
//! Pins the cross-cutting guarantees agents depend on: `--introspect`
//! produces a document conforming to `foundry:introspect@v1`, and under
//! `--machine` the help / parse-error paths emit a `foundry:envelope@v1`
//! envelope on stdout (never raw clap text).

use foundry_test_utils::agent_schema;
use serde_json::Value;

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
    assert_eq!(envelope["errors"], serde_json::json!([]));
    assert_eq!(envelope["warnings"], serde_json::json!([]));
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
    assert_eq!(envelope["warnings"], serde_json::json!([]));
    agent_schema::validate("foundry:envelope@v1", &envelope);

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.is_empty(), "stderr must be empty under --machine, got: {stderr}");
});
