//! CLI tests for `cast keychain` subcommands.

use anvil::NodeConfig;
use foundry_test_utils::{TestCommand, util::OutputExt};
use std::{fs, path::Path};

/// Anvil test accounts (standard mnemonic).
mod accounts {
    pub const PK1: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    pub const ADDR1: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    pub const ADDR2: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    pub const TOKEN: &str = "0x20C000000000000000000000b9537d11c60E8b50"; // PathUSD
}

fn create_session(cmd: &mut TestCommand, tempo_home: &Path, chain_id: &str) -> (String, String) {
    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home);
    let create_output = cmd
        .args([
            "--json",
            "wallet",
            "session",
            "create",
            "--root",
            accounts::ADDR1,
            "--chain-id",
            chain_id,
            "--expires",
            "10m",
            "--scope",
            accounts::TOKEN,
            "--private-key",
            accounts::PK1,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    let created: serde_json::Value =
        serde_json::from_str(create_output.trim()).expect("session create emits JSON");
    let session_id = created["session_id"].as_str().expect("session_id").to_string();
    let key_address = created["key_address"].as_str().expect("key_address").to_string();
    (session_id, key_address)
}

fn assert_session_file_status_without_key(tempo_home: &Path, status: &str) {
    let session_file = tempo_home.join("wallet/sessions.toml");
    let contents = fs::read_to_string(&session_file).expect("sessions.toml exists");
    assert!(
        contents.contains(&format!("status = \"{status}\"")),
        "unexpected sessions.toml:\n{contents}"
    );
    assert!(
        !contents.contains("key = \"0x"),
        "{status} session must not retain private key material:\n{contents}"
    );
}

// `cast keychain rl --json` must emit `{"remaining":"<value>"}`, not a bare string.
casttest!(keychain_rl_json_is_object, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();

    let output = cmd
        .args([
            "keychain",
            "rl",
            accounts::ADDR1,
            accounts::ADDR2,
            accounts::TOKEN,
            "--rpc-url",
            &rpc,
            "--json",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    let parsed: serde_json::Value = serde_json::from_str(output.trim())
        .expect("cast keychain rl --json should emit valid JSON");
    assert!(parsed.is_object(), "expected JSON object, got: {output}");
    assert!(
        parsed.get("remaining").is_some(),
        "expected 'remaining' key in JSON output, got: {output}"
    );
    // Must not be a bare string (old bug: `"0"`)
    assert!(!parsed.is_string(), "JSON output must not be a bare string, got: {output}");
});

// `cast keychain authorize --tempo.print-sponsor-hash --json` must emit
// `{"sponsor_hash":"0x..."}`, not a raw hex string.
casttest!(keychain_authorize_sponsor_hash_json_is_object, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();

    let output = cmd
        .args([
            "keychain",
            "authorize",
            accounts::ADDR2, // key to authorize
            "--private-key",
            accounts::PK1,
            "--rpc-url",
            &rpc,
            "--tempo.print-sponsor-hash",
            "--json",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    let parsed: serde_json::Value = serde_json::from_str(output.trim())
        .expect("cast keychain authorize --tempo.print-sponsor-hash --json should emit valid JSON");
    assert!(parsed.is_object(), "expected JSON object, got: {output}");
    let hash = parsed
        .get("sponsor_hash")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("expected 'sponsor_hash' key in JSON output, got: {output}"));
    assert!(hash.starts_with("0x"), "sponsor_hash should be 0x-prefixed, got: {hash}");
    assert_eq!(hash.len(), 66, "sponsor_hash should be 32-byte hex (66 chars), got: {hash}");
});

casttest!(wallet_session_revoke_revokes_provisioned_key_on_chain, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let (session_id, key_address) = create_session(&mut cmd, tempo_home.path(), "31337");

    cmd.cast_fuse()
        .args([
            "keychain",
            "authorize",
            &key_address,
            "--private-key",
            accounts::PK1,
            "--rpc-url",
            &rpc,
        ])
        .assert_success();

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    cmd.args([
        "wallet",
        "session",
        "revoke",
        &session_id,
        "--private-key",
        accounts::PK1,
        "--rpc-url",
        &rpc,
    ])
    .assert_success();

    let check_output = cmd
        .cast_fuse()
        .args(["keychain", "check", accounts::ADDR1, &key_address, "--rpc-url", &rpc, "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let checked: serde_json::Value =
        serde_json::from_str(check_output.trim()).expect("keychain check emits JSON");
    assert_eq!(checked["provisioned"], false);
    assert_eq!(checked["is_revoked"], true);

    assert_session_file_status_without_key(tempo_home.path(), "revoked");
});

casttest!(wallet_session_revoke_marks_unprovisioned_key_revoked_locally, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let (session_id, key_address) = create_session(&mut cmd, tempo_home.path(), "31337");

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    let output = cmd
        .args([
            "--json",
            "wallet",
            "session",
            "revoke",
            &session_id,
            "--private-key",
            accounts::PK1,
            "--rpc-url",
            &rpc,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let revoked: serde_json::Value =
        serde_json::from_str(output.trim()).expect("session revoke emits JSON");
    assert_eq!(revoked["status"], "revoked");
    assert_eq!(revoked["reason"], "not_provisioned");

    let check_output = cmd
        .cast_fuse()
        .args(["keychain", "check", accounts::ADDR1, &key_address, "--rpc-url", &rpc, "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let checked: serde_json::Value =
        serde_json::from_str(check_output.trim()).expect("keychain check emits JSON");
    assert_eq!(checked["provisioned"], false);
    assert_eq!(checked["is_revoked"], false);

    assert_session_file_status_without_key(tempo_home.path(), "revoked");
});

casttest!(wallet_session_revoke_local_cleans_key_without_rpc, async |_prj, cmd| {
    let tempo_home = tempfile::tempdir().unwrap();
    let (session_id, _) = create_session(&mut cmd, tempo_home.path(), "31337");

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    let output = cmd
        .args(["--json", "wallet", "session", "revoke", &session_id, "--local"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let revoked: serde_json::Value =
        serde_json::from_str(output.trim()).expect("session revoke --local emits JSON");
    assert_eq!(revoked["status"], "revoked");
    assert_eq!(revoked["reason"], "local");

    assert_session_file_status_without_key(tempo_home.path(), "revoked");
});

casttest!(wallet_session_revoke_failure_marks_failed_and_clears_key, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let (session_id, _) = create_session(&mut cmd, tempo_home.path(), "31338");

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    cmd.args([
        "wallet",
        "session",
        "revoke",
        &session_id,
        "--private-key",
        accounts::PK1,
        "--rpc-url",
        &rpc,
    ])
    .assert_failure();

    assert_session_file_status_without_key(tempo_home.path(), "failed");
});
