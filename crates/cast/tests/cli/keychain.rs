//! CLI tests for `cast keychain` subcommands.

use anvil::NodeConfig;
use foundry_evm::core::tempo::PATH_USD_ADDRESS;
use foundry_test_utils::{TestCommand, util::OutputExt};
use path_slash::PathExt;
use std::{fs, path::Path};
use tempo_contracts::precompiles::TIP20_FACTORY_ADDRESS;

/// Anvil test accounts (standard mnemonic).
mod accounts {
    pub const PK1: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    pub const PK2: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
    pub const ADDR1: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    pub const ADDR2: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    pub const ADDR3: &str = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";
}

fn path_usd() -> String {
    PATH_USD_ADDRESS.to_string()
}

const ADDRESS_REGISTRY: &str = "0xFDC0000000000000000000000000000000000000";

fn tip20_factory() -> String {
    TIP20_FACTORY_ADDRESS.to_string()
}

const MISSING_SESSION_ID: &str =
    "0x5555555555555555555555555555555555555555555555555555555555555555";

fn batch_send_transfer_call(path_usd: &str) -> String {
    format!("{path_usd}::transfer(address,uint256):{},0", accounts::ADDR3)
}

const PRECOMPUTED_VADDR_SALT_FOR_ADDR1: &str =
    "0x00000000000000000000000000000000000000000000000000000000abf52baf";

fn assert_wrong_chain_error(stderr: &str) {
    assert!(stderr.contains("is for chain 31338"), "unexpected stderr:\n{stderr}");
    assert!(stderr.contains("command is using chain 31337"), "unexpected stderr:\n{stderr}");
}

fn cast_send_session_script(path_usd: &str) -> String {
    format!(
        r#"#!/bin/sh
set -eu
test -n "${{TEMPO_SESSION_ID:-}}"
"${{CAST_BIN}}" send "{path_usd}" 'transfer(address,uint256)' "{recipient}" 0 --rpc-url "${{RPC_URL}}" --tempo.fee-token "{path_usd}" --async
"#,
        recipient = accounts::ADDR3,
    )
}

fn create_session(cmd: &mut TestCommand, tempo_home: &Path, chain_id: &str) -> (String, String) {
    create_session_with_scope(cmd, tempo_home, chain_id, &path_usd())
}

fn create_session_with_scope(
    cmd: &mut TestCommand,
    tempo_home: &Path,
    chain_id: &str,
    scope: &str,
) -> (String, String) {
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
            scope,
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

fn assert_session_file_status_with_key(tempo_home: &Path, status: &str) {
    let session_file = tempo_home.join("wallet/sessions.toml");
    let contents = fs::read_to_string(&session_file).expect("sessions.toml exists");
    assert!(
        contents.contains(&format!("status = \"{status}\"")),
        "unexpected sessions.toml:\n{contents}"
    );
    assert!(
        contents.contains("key = \"0x"),
        "{status} session should retain private key material:\n{contents}"
    );
}

fn assert_async_tx_hash(stdout: &str, command: &str) {
    assert!(
        stdout.trim().starts_with("0x"),
        "expected {command} --async to print a tx hash, got:\n{stdout}"
    );
}

fn assert_contains_tx_hash(stdout: &str, command: &str) {
    assert!(
        stdout.lines().any(|line| {
            let line = line.trim();
            line.len() == 66
                && line.starts_with("0x")
                && line[2..].chars().all(|c| c.is_ascii_hexdigit())
        }),
        "expected {command} to print a tx hash, got:\n{stdout}"
    );
}

fn assert_session_cleanup_failure(stderr: &str) {
    assert!(
        stderr.contains("failed to clean up Tempo session after inner command"),
        "unexpected stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("session key is not provisioned on-chain yet"),
        "unexpected stderr:\n{stderr}"
    );
}

// `cast keychain rl --json` must emit `{"remaining":"<value>"}`, not a bare string.
casttest!(keychain_rl_json_is_object, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let path_usd = path_usd();

    let output = cmd
        .args([
            "keychain",
            "rl",
            accounts::ADDR1,
            accounts::ADDR2,
            &path_usd,
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

// TODO: remove this check once browser supports T5 KeyAuthorization fields
casttest!(key_authorization_sign_rejects_browser_witness_before_browser_run, |_prj, cmd| {
    let stderr = cmd
        .args([
            "key-authorization",
            "sign",
            accounts::ADDR2,
            "--chain-id",
            "31337",
            "--witness",
            "0x5353535353535353535353535353535353535353535353535353535353535353",
            "--browser",
            "--browser-disable-open",
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(
        stderr
            .contains("browser key authorization signing does not support T5 fields yet: witness"),
        "unexpected stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("Waiting for browser wallet connection"),
        "browser flow should not start before rejecting --browser --witness:\n{stderr}"
    );
});

casttest!(keychain_doctor_json_keeps_report_schema_version, async |_prj, cmd| {
    let output = cmd
        .args([
            "keychain",
            "doctor",
            accounts::ADDR2,
            "--root-account",
            accounts::ADDR1,
            "--rpc-url",
            "http://127.0.0.1:1",
            "--json",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    let parsed: serde_json::Value = serde_json::from_str(output.trim())
        .expect("cast keychain doctor --json should emit valid JSON");
    assert_eq!(parsed["schema_version"], 1);
});

casttest!(keychain_show_json_no_match_returns_empty_array, |prj, cmd| {
    let tempo_home = prj.root().join("tempo-home");
    fs::create_dir_all(tempo_home.join("wallet")).expect("create Tempo wallet dir");
    fs::write(tempo_home.join("wallet/keys.toml"), "keys = []\n").expect("write keys.toml");

    cmd.env("TEMPO_HOME", &tempo_home);

    let output = cmd
        .args(["keychain", "show", accounts::ADDR1, "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    let parsed: serde_json::Value = serde_json::from_str(output.trim())
        .expect("cast keychain show --json should emit valid JSON");
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(
        parsed["data"].as_array().expect("keychain show --json data should be an array").len(),
        0
    );
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
    let checked = &checked["data"];
    assert_eq!(checked["provisioned"], false);
    assert_eq!(checked["is_revoked"], true);

    assert_session_file_status_without_key(tempo_home.path(), "revoked");
});

casttest!(wallet_session_revoke_sponsor_hash_does_not_mark_revoked, async |_prj, cmd| {
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
    let output = cmd
        .args([
            "wallet",
            "session",
            "revoke",
            &session_id,
            "--private-key",
            accounts::PK1,
            "--rpc-url",
            &rpc,
            "--tempo.print-sponsor-hash",
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(
        output.contains("--tempo.print-sponsor-hash only prints a sponsor hash"),
        "unexpected stderr:\n{output}"
    );

    let check_output = cmd
        .cast_fuse()
        .args(["keychain", "check", accounts::ADDR1, &key_address, "--rpc-url", &rpc, "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let checked: serde_json::Value =
        serde_json::from_str(check_output.trim()).expect("keychain check emits JSON");
    let checked = &checked["data"];
    assert_eq!(checked["provisioned"], true);
    assert_eq!(checked["is_revoked"], false);

    assert_session_file_status_with_key(tempo_home.path(), "active");
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
    let checked = &checked["data"];
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

casttest!(wallet_session_revoke_wrong_chain_preserves_local_key, async |_prj, cmd| {
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

    assert_session_file_status_with_key(tempo_home.path(), "active");
});

casttest!(
    wallet_session_run_for_without_key_use_fails_closed_and_cleans_key_material,
    async |_prj, cmd| {
        let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
        let rpc = handle.http_endpoint();
        let tempo_home = tempfile::tempdir().unwrap();
        let path_usd = path_usd();
        let child_dir = tempfile::tempdir().unwrap();
        let child_script = child_dir.path().join("session-child.sh");
        let child_session_out = child_dir.path().join("child-session-id.txt");
        fs::write(
            &child_script,
            r#"#!/bin/sh
set -eu
test -n "${TEMPO_SESSION_ID:-}"
session_file="${TEMPO_HOME}/wallet/sessions.toml"
grep -q 'status = "active"' "${session_file}"
grep -q 'key = "0x' "${session_file}"
grep -q 'key_authorization = "0x' "${session_file}"
printf '%s\n' "${TEMPO_SESSION_ID}" > "$1"
"#,
        )
        .expect("write child script");

        // Git Bash (the `sh` on Windows) treats backslashes as escapes, so embed the script
        // paths with forward slashes; `to_slash_lossy` is a no-op on Unix.
        let for_command =
            format!("sh {} {}", child_script.to_slash_lossy(), child_session_out.to_slash_lossy());

        cmd.cast_fuse();
        cmd.env("TEMPO_HOME", tempo_home.path());
        let stderr = cmd
            .args([
                "wallet",
                "session",
                "--root",
                accounts::ADDR1,
                "--expires",
                "10m",
                "--scope",
                &path_usd,
                "--spend-limit",
                "PathUSD=0",
                "--for",
                &for_command,
                "--private-key",
                accounts::PK1,
                "--rpc-url",
                &rpc,
            ])
            .assert_failure()
            .get_output()
            .stderr_lossy();
        assert_session_cleanup_failure(&stderr);

        let child_session_id =
            fs::read_to_string(&child_session_out).expect("child wrote TEMPO_SESSION_ID");
        assert!(
            child_session_id.trim().starts_with("0x"),
            "unexpected child session id: {child_session_id}"
        );
        assert_session_file_status_without_key(tempo_home.path(), "revoking");
    }
);

casttest!(wallet_session_run_for_cast_send_submits_with_session_key, async |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let child_dir = tempfile::tempdir().unwrap();
    let child_script = child_dir.path().join("session-cast-send.sh");
    let path_usd = path_usd();
    fs::write(&child_script, cast_send_session_script(&path_usd)).expect("write child script");

    let for_command = format!("sh {}", child_script.to_slash_lossy());

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    cmd.env("CAST_BIN", prj.foundry_bin_path("cast"));
    cmd.env("RPC_URL", &rpc);
    let assertion = cmd
        .args([
            "wallet",
            "session",
            "--root",
            accounts::ADDR1,
            "--expires",
            "10m",
            "--scope",
            &path_usd,
            "--spend-limit",
            "PathUSD=1000000",
            "--for",
            &for_command,
            "--private-key",
            accounts::PK1,
            "--rpc-url",
            &rpc,
        ])
        .assert_failure();
    let stdout = assertion.get_output().stdout_lossy();
    let stderr = assertion.get_output().stderr_lossy();

    assert_async_tx_hash(&stdout, "child cast send");
    assert_session_cleanup_failure(&stderr);
    assert_session_file_status_without_key(tempo_home.path(), "revoking");
});

casttest!(wallet_session_run_for_batch_send_submits_with_session_key, async |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let child_dir = tempfile::tempdir().unwrap();
    let child_script = child_dir.path().join("session-batch-send.sh");
    let path_usd = path_usd();
    let call = batch_send_transfer_call(&path_usd);
    fs::write(
        &child_script,
        format!(
            r#"#!/bin/sh
set -eu
test -n "${{TEMPO_SESSION_ID:-}}"
"${{CAST_BIN}}" batch-send --call "{call}" --rpc-url "${{RPC_URL}}" --tempo.fee-token "{path_usd}" --async
"#,
        ),
    )
    .expect("write child script");

    let for_command = format!("sh {}", child_script.to_slash_lossy());

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    cmd.env("CAST_BIN", prj.foundry_bin_path("cast"));
    cmd.env("RPC_URL", &rpc);
    let assertion = cmd
        .args([
            "wallet",
            "session",
            "--root",
            accounts::ADDR1,
            "--expires",
            "10m",
            "--scope",
            &path_usd,
            "--spend-limit",
            "PathUSD=1000000",
            "--for",
            &for_command,
            "--private-key",
            accounts::PK1,
            "--rpc-url",
            &rpc,
        ])
        .assert_failure();
    let stdout = assertion.get_output().stdout_lossy();
    let stderr = assertion.get_output().stderr_lossy();

    assert_async_tx_hash(&stdout, "child cast batch-send");
    assert_session_cleanup_failure(&stderr);
    assert_session_file_status_without_key(tempo_home.path(), "revoking");
});

casttest!(wallet_session_run_for_forge_script_submits_with_session_key, async |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let path_usd = path_usd();

    foundry_test_utils::util::initialize(prj.root());
    let script = prj.add_script(
        "SessionForgeScript.s.sol",
        &format!(
            r#"
import "forge-std/Script.sol";

interface PathUsdLike {{
    function transfer(address to, uint256 amount) external returns (bool);
}}

contract SessionForgeScript is Script {{
    function run() external {{
        vm.startBroadcast();
        PathUsdLike({path_usd}).transfer({recipient}, 0);
        vm.stopBroadcast();
    }}
}}
"#,
            recipient = accounts::ADDR3,
        ),
    );

    let for_command = format!(
        "{} script {} --tc SessionForgeScript --broadcast --timeout 1 --rpc-url {} --root {}",
        prj.ensure_foundry_bin("forge").to_slash_lossy(),
        script.to_slash_lossy(),
        rpc,
        prj.root().to_slash_lossy(),
    );

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    let assertion = cmd
        .args([
            "wallet",
            "session",
            "--root",
            accounts::ADDR1,
            "--expires",
            "10m",
            "--target",
            &path_usd,
            "--selector",
            "transfer(address,uint256)",
            "--spend-limit",
            "PathUSD=0",
            "--for",
            &for_command,
            "--private-key",
            accounts::PK1,
            "--rpc-url",
            &rpc,
        ])
        .assert_failure();
    let stdout = assertion.get_output().stdout_lossy();
    let stderr = assertion.get_output().stderr_lossy();

    assert!(
        stdout.contains("ONCHAIN EXECUTION COMPLETE & SUCCESSFUL."),
        "expected forge script broadcast completion\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let run_latest = foundry_common::fs::json_files(&prj.root().join("broadcast"))
        .find(|path| path.ends_with("run-latest.json"))
        .unwrap_or_else(|| {
            panic!("expected forge broadcast artifact\nstdout:\n{stdout}\nstderr:\n{stderr}")
        });
    let broadcast = fs::read_to_string(&run_latest).expect("read forge broadcast artifact");
    let broadcast: serde_json::Value =
        serde_json::from_str(&broadcast).expect("forge broadcast artifact is valid JSON");
    let tx = &broadcast["transactions"][0];

    assert_eq!(tx["transactionType"], "CALL", "unexpected forge broadcast tx: {tx}");
    assert_eq!(tx["function"], "transfer(address,uint256)", "unexpected forge broadcast tx: {tx}");
    assert_eq!(
        tx["contractAddress"].as_str().map(str::to_ascii_lowercase),
        Some(path_usd.to_ascii_lowercase()),
        "unexpected forge broadcast tx: {tx}"
    );
    assert!(
        tx["hash"].as_str().is_some_and(|hash| hash.starts_with("0x")),
        "forge broadcast tx should have a submitted hash: {tx}"
    );
    assert_session_cleanup_failure(&stderr);
    assert_session_file_status_without_key(tempo_home.path(), "revoking");
});

casttest!(batch_send_uses_tempo_session_id_env, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let path_usd = path_usd();
    let call = batch_send_transfer_call(&path_usd);
    let (session_id, _) = create_session(&mut cmd, tempo_home.path(), "31337");

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    cmd.env("TEMPO_SESSION_ID", &session_id);
    let stdout = cmd
        .args([
            "batch-send",
            "--call",
            &call,
            "--rpc-url",
            &rpc,
            "--tempo.fee-token",
            &path_usd,
            "--async",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_async_tx_hash(&stdout, "cast batch-send");
});

casttest!(batch_mktx_uses_tempo_session_id_env, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let path_usd = path_usd();
    let call = batch_send_transfer_call(&path_usd);
    let (session_id, _) = create_session(&mut cmd, tempo_home.path(), "31337");

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    cmd.env("TEMPO_SESSION_ID", &session_id);
    let stdout = cmd
        .args(["batch-mktx", "--call", &call, "--rpc-url", &rpc, "--tempo.fee-token", &path_usd])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(
        stdout.trim().starts_with("0x"),
        "expected cast batch-mktx to print raw tx hex, got:\n{stdout}"
    );
});

casttest!(batch_mktx_raw_unsigned_resolves_tempo_access_key_metadata, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let path_usd = path_usd();
    let call = batch_send_transfer_call(&path_usd);

    cmd.cast_fuse();
    let stderr = cmd
        .args([
            "batch-mktx",
            "--call",
            &call,
            "--raw-unsigned",
            "--from",
            accounts::ADDR1,
            "--nonce",
            "0",
            "--gas-price",
            "1",
            "--rpc-url",
            &rpc,
            "--tempo.fee-token",
            &path_usd,
            "--tempo.access-key",
            accounts::PK2,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(
        stderr.contains("--tempo.root-account is required when --tempo.access-key is set"),
        "raw unsigned must still resolve Tempo access-key metadata, got:\n{stderr}"
    );
});

casttest!(vaddr_create_uses_tempo_session_id_env, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let (session_id, _) =
        create_session_with_scope(&mut cmd, tempo_home.path(), "31337", ADDRESS_REGISTRY);

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    cmd.env("TEMPO_SESSION_ID", &session_id);
    let stdout = cmd
        .args([
            "--json",
            "vaddr",
            "create",
            "--owner",
            accounts::ADDR1,
            "--salt",
            PRECOMPUTED_VADDR_SALT_FOR_ADDR1,
            "--rpc-url",
            &rpc,
            "--async",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    let envelope: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("vaddr create emits JSON");
    assert!(
        envelope["data"]["registration_tx_hash"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("0x")),
        "expected vaddr create --json to include registration tx hash, got:\n{stdout}"
    );
});

casttest!(tip20_create_uses_tempo_session_id_env, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let path_usd = path_usd();
    let (session_id, _) =
        create_session_with_scope(&mut cmd, tempo_home.path(), "31337", &tip20_factory());

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    cmd.env("TEMPO_SESSION_ID", &session_id);
    let stdout = cmd
        .args([
            "tip20",
            "create",
            "Session Dollar",
            "SESS",
            "USD",
            &path_usd,
            accounts::ADDR1,
            "0x0000000000000000000000000000000000000000000000000000000000005151",
            "--rpc-url",
            &rpc,
            "--tempo.fee-token",
            &path_usd,
            "--async",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_async_tx_hash(&stdout, "cast tip20 create");
});

casttest!(tip20_mine_register_uses_tempo_session_id_env, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let (session_id, _) =
        create_session_with_scope(&mut cmd, tempo_home.path(), "31337", ADDRESS_REGISTRY);

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    cmd.env("TEMPO_SESSION_ID", &session_id);
    let stdout = cmd
        .args([
            "tip20",
            "mine",
            accounts::ADDR1,
            "--salt",
            PRECOMPUTED_VADDR_SALT_FOR_ADDR1,
            "--register",
            "--rpc-url",
            &rpc,
            "--async",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_contains_tx_hash(&stdout, "cast tip20 mine --register");
});

casttest!(erc20_transfer_uses_tempo_session_id_env, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let path_usd = path_usd();
    let (session_id, _) = create_session(&mut cmd, tempo_home.path(), "31337");

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    cmd.env("TEMPO_SESSION_ID", &session_id);
    let stdout = cmd
        .args([
            "erc20",
            "transfer",
            &path_usd,
            accounts::ADDR3,
            "0",
            "--rpc-url",
            &rpc,
            "--tempo.fee-token",
            &path_usd,
            "--async",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_async_tx_hash(&stdout, "cast erc20 transfer");
});

casttest!(batch_mktx_rejects_session_with_explicit_signer, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let path_usd = path_usd();
    let call = batch_send_transfer_call(&path_usd);

    cmd.cast_fuse();
    let stderr = cmd
        .args([
            "batch-mktx",
            "--call",
            &call,
            "--tempo.session",
            MISSING_SESSION_ID,
            "--private-key",
            accounts::PK1,
            "--rpc-url",
            &rpc,
            "--tempo.fee-token",
            &path_usd,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("explicit wallet signer"), "unexpected stderr:\n{stderr}");
});

casttest!(erc20_rejects_session_with_explicit_signer, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let path_usd = path_usd();

    cmd.cast_fuse();
    let stderr = cmd
        .args([
            "erc20",
            "transfer",
            &path_usd,
            accounts::ADDR3,
            "0",
            "--tempo.session",
            MISSING_SESSION_ID,
            "--private-key",
            accounts::PK1,
            "--rpc-url",
            &rpc,
            "--tempo.fee-token",
            &path_usd,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("explicit wallet signer"), "unexpected stderr:\n{stderr}");
});

casttest!(batch_mktx_rejects_session_with_ethsign, |_prj, cmd| {
    let path_usd = path_usd();
    let call = batch_send_transfer_call(&path_usd);

    cmd.cast_fuse();
    let stderr = cmd
        .args([
            "batch-mktx",
            "--call",
            &call,
            "--tempo.session",
            MISSING_SESSION_ID,
            "--ethsign",
            "--from",
            accounts::ADDR1,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("cannot be combined with --ethsign"), "unexpected stderr:\n{stderr}");
});

casttest!(batch_mktx_rejects_session_with_raw_unsigned, |_prj, cmd| {
    let path_usd = path_usd();
    let call = batch_send_transfer_call(&path_usd);

    cmd.cast_fuse();
    let stderr = cmd
        .args([
            "batch-mktx",
            "--call",
            &call,
            "--tempo.session",
            MISSING_SESSION_ID,
            "--raw-unsigned",
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(
        stderr.contains("cannot be combined with --raw-unsigned"),
        "unexpected stderr:\n{stderr}"
    );
});

casttest!(batch_mktx_rejects_session_on_wrong_chain, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let path_usd = path_usd();
    let call = batch_send_transfer_call(&path_usd);
    let (session_id, _) = create_session(&mut cmd, tempo_home.path(), "31338");

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    let stderr = cmd
        .args([
            "batch-mktx",
            "--call",
            &call,
            "--tempo.session",
            &session_id,
            "--rpc-url",
            &rpc,
            "--tempo.fee-token",
            &path_usd,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert_wrong_chain_error(&stderr);
});

casttest!(vaddr_create_rejects_session_with_explicit_signer, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();

    cmd.cast_fuse();
    let stderr = cmd
        .args([
            "vaddr",
            "create",
            "--owner",
            accounts::ADDR1,
            "--salt",
            PRECOMPUTED_VADDR_SALT_FOR_ADDR1,
            "--tempo.session",
            MISSING_SESSION_ID,
            "--private-key",
            accounts::PK1,
            "--rpc-url",
            &rpc,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("explicit wallet signer"), "unexpected stderr:\n{stderr}");
});

casttest!(vaddr_create_rejects_session_with_browser, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();

    cmd.cast_fuse();
    let stderr = cmd
        .args([
            "vaddr",
            "create",
            "--owner",
            accounts::ADDR1,
            "--salt",
            PRECOMPUTED_VADDR_SALT_FOR_ADDR1,
            "--tempo.session",
            MISSING_SESSION_ID,
            "--browser",
            "--rpc-url",
            &rpc,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("cannot be combined with --browser"), "unexpected stderr:\n{stderr}");
});

casttest!(vaddr_create_rejects_session_on_wrong_chain, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let (session_id, _) =
        create_session_with_scope(&mut cmd, tempo_home.path(), "31338", ADDRESS_REGISTRY);

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    let stderr = cmd
        .args([
            "vaddr",
            "create",
            "--owner",
            accounts::ADDR1,
            "--salt",
            PRECOMPUTED_VADDR_SALT_FOR_ADDR1,
            "--tempo.session",
            &session_id,
            "--rpc-url",
            &rpc,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert_wrong_chain_error(&stderr);
});

casttest!(tip20_create_rejects_session_with_explicit_signer, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let path_usd = path_usd();

    cmd.cast_fuse();
    let stderr = cmd
        .args([
            "tip20",
            "create",
            "Session Dollar",
            "SESS",
            "USD",
            &path_usd,
            accounts::ADDR1,
            "0x0000000000000000000000000000000000000000000000000000000000005152",
            "--tempo.session",
            MISSING_SESSION_ID,
            "--private-key",
            accounts::PK1,
            "--rpc-url",
            &rpc,
            "--tempo.fee-token",
            &path_usd,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("explicit wallet signer"), "unexpected stderr:\n{stderr}");
});

casttest!(tip20_create_rejects_session_with_browser, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let path_usd = path_usd();

    cmd.cast_fuse();
    let stderr = cmd
        .args([
            "tip20",
            "create",
            "Session Dollar",
            "SESS",
            "USD",
            &path_usd,
            accounts::ADDR1,
            "0x0000000000000000000000000000000000000000000000000000000000005153",
            "--tempo.session",
            MISSING_SESSION_ID,
            "--browser",
            "--rpc-url",
            &rpc,
            "--tempo.fee-token",
            &path_usd,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("cannot be combined with --browser"), "unexpected stderr:\n{stderr}");
});

casttest!(tip20_create_rejects_session_on_wrong_chain, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let path_usd = path_usd();
    let (session_id, _) =
        create_session_with_scope(&mut cmd, tempo_home.path(), "31338", &tip20_factory());

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    let stderr = cmd
        .args([
            "tip20",
            "create",
            "Session Dollar",
            "SESS",
            "USD",
            &path_usd,
            accounts::ADDR1,
            "0x0000000000000000000000000000000000000000000000000000000000005154",
            "--tempo.session",
            &session_id,
            "--rpc-url",
            &rpc,
            "--tempo.fee-token",
            &path_usd,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert_wrong_chain_error(&stderr);
});

casttest!(tip20_mine_register_rejects_session_with_explicit_signer, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();

    cmd.cast_fuse();
    let stderr = cmd
        .args([
            "tip20",
            "mine",
            accounts::ADDR1,
            "--salt",
            PRECOMPUTED_VADDR_SALT_FOR_ADDR1,
            "--register",
            "--tempo.session",
            MISSING_SESSION_ID,
            "--private-key",
            accounts::PK1,
            "--rpc-url",
            &rpc,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("explicit wallet signer"), "unexpected stderr:\n{stderr}");
});

casttest!(tip20_mine_register_rejects_session_with_browser, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();

    cmd.cast_fuse();
    let stderr = cmd
        .args([
            "tip20",
            "mine",
            accounts::ADDR1,
            "--salt",
            PRECOMPUTED_VADDR_SALT_FOR_ADDR1,
            "--register",
            "--tempo.session",
            MISSING_SESSION_ID,
            "--browser",
            "--rpc-url",
            &rpc,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("cannot be combined with --browser"), "unexpected stderr:\n{stderr}");
});

casttest!(tip20_mine_register_rejects_session_on_wrong_chain, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let (session_id, _) =
        create_session_with_scope(&mut cmd, tempo_home.path(), "31338", ADDRESS_REGISTRY);

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    let stderr = cmd
        .args([
            "tip20",
            "mine",
            accounts::ADDR1,
            "--salt",
            PRECOMPUTED_VADDR_SALT_FOR_ADDR1,
            "--register",
            "--tempo.session",
            &session_id,
            "--rpc-url",
            &rpc,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert_wrong_chain_error(&stderr);
});

casttest!(erc20_rejects_session_with_browser, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let path_usd = path_usd();

    cmd.cast_fuse();
    let stderr = cmd
        .args([
            "erc20",
            "transfer",
            &path_usd,
            accounts::ADDR3,
            "0",
            "--tempo.session",
            MISSING_SESSION_ID,
            "--browser",
            "--rpc-url",
            &rpc,
            "--tempo.fee-token",
            &path_usd,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("cannot be combined with --browser"), "unexpected stderr:\n{stderr}");
});

casttest!(erc20_rejects_session_on_wrong_chain, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let path_usd = path_usd();
    let (session_id, _) = create_session(&mut cmd, tempo_home.path(), "31338");

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    let stderr = cmd
        .args([
            "erc20",
            "transfer",
            &path_usd,
            accounts::ADDR3,
            "0",
            "--tempo.session",
            &session_id,
            "--rpc-url",
            &rpc,
            "--tempo.fee-token",
            &path_usd,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert_wrong_chain_error(&stderr);
});

casttest!(wallet_session_run_for_grandchild_cast_send_inherits_session_key, async |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let child_dir = tempfile::tempdir().unwrap();
    let child_script = child_dir.path().join("session-child.sh");
    let grandchild_script = child_dir.path().join("session-grandchild-cast-send.sh");
    let path_usd = path_usd();

    fs::write(
        &child_script,
        r#"#!/bin/sh
set -eu
test -n "${TEMPO_SESSION_ID:-}"
sh "$1"
"#,
    )
    .expect("write child script");

    fs::write(&grandchild_script, cast_send_session_script(&path_usd))
        .expect("write grandchild script");

    let for_command =
        format!("sh {} {}", child_script.to_slash_lossy(), grandchild_script.to_slash_lossy());

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    cmd.env("CAST_BIN", prj.foundry_bin_path("cast"));
    cmd.env("RPC_URL", &rpc);
    let assertion = cmd
        .args([
            "wallet",
            "session",
            "--root",
            accounts::ADDR1,
            "--expires",
            "10m",
            "--scope",
            &path_usd,
            "--spend-limit",
            "PathUSD=1000000",
            "--for",
            &for_command,
            "--private-key",
            accounts::PK1,
            "--rpc-url",
            &rpc,
        ])
        .assert_failure();
    let stdout = assertion.get_output().stdout_lossy();
    let stderr = assertion.get_output().stderr_lossy();

    assert_async_tx_hash(&stdout, "grandchild cast send");
    assert_session_cleanup_failure(&stderr);
    assert_session_file_status_without_key(tempo_home.path(), "revoking");
});

casttest!(cast_send_rejects_session_with_explicit_signer, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();

    cmd.cast_fuse();
    let stderr = cmd
        .args([
            "send",
            accounts::ADDR3,
            "--value",
            "1",
            "--tempo.session",
            MISSING_SESSION_ID,
            "--private-key",
            accounts::PK1,
            "--rpc-url",
            &rpc,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("explicit wallet signer"), "unexpected stderr:\n{stderr}");
});

casttest!(batch_send_rejects_session_with_explicit_signer, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let path_usd = path_usd();
    let call = batch_send_transfer_call(&path_usd);

    cmd.cast_fuse();
    let stderr = cmd
        .args([
            "batch-send",
            "--call",
            &call,
            "--tempo.session",
            MISSING_SESSION_ID,
            "--private-key",
            accounts::PK1,
            "--rpc-url",
            &rpc,
            "--tempo.fee-token",
            &path_usd,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("explicit wallet signer"), "unexpected stderr:\n{stderr}");
});

casttest!(batch_send_rejects_session_with_unlocked, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let path_usd = path_usd();
    let call = batch_send_transfer_call(&path_usd);

    cmd.cast_fuse();
    let stderr = cmd
        .args([
            "batch-send",
            "--call",
            &call,
            "--tempo.session",
            MISSING_SESSION_ID,
            "--unlocked",
            "--from",
            accounts::ADDR1,
            "--rpc-url",
            &rpc,
            "--tempo.fee-token",
            &path_usd,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("cannot be combined with --unlocked"), "unexpected stderr:\n{stderr}");
});

casttest!(batch_send_rejects_session_on_wrong_chain, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let path_usd = path_usd();
    let call = batch_send_transfer_call(&path_usd);
    let (session_id, _) = create_session(&mut cmd, tempo_home.path(), "31338");

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    let stderr = cmd
        .args([
            "batch-send",
            "--call",
            &call,
            "--tempo.session",
            &session_id,
            "--rpc-url",
            &rpc,
            "--tempo.fee-token",
            &path_usd,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("is for chain 31338"), "unexpected stderr:\n{stderr}");
    assert!(stderr.contains("command is using chain 31337"), "unexpected stderr:\n{stderr}");
});

casttest!(wallet_session_run_for_cleans_key_material_when_child_fails, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let tempo_home = tempfile::tempdir().unwrap();
    let path_usd = path_usd();
    let child_dir = tempfile::tempdir().unwrap();
    let child_script = child_dir.path().join("failing-session-child.sh");
    fs::write(
        &child_script,
        r#"#!/bin/sh
set -eu
test -n "${TEMPO_SESSION_ID:-}"
session_file="${TEMPO_HOME}/wallet/sessions.toml"
grep -q 'status = "active"' "${session_file}"
grep -q 'key = "0x' "${session_file}"
exit 7
"#,
    )
    .expect("write child script");

    let for_command = format!("sh {}", child_script.to_slash_lossy());

    cmd.cast_fuse();
    cmd.env("TEMPO_HOME", tempo_home.path());
    let stderr = cmd
        .args([
            "wallet",
            "session",
            "--root",
            accounts::ADDR1,
            "--expires",
            "10m",
            "--scope",
            &path_usd,
            "--for",
            &for_command,
            "--private-key",
            accounts::PK1,
            "--rpc-url",
            &rpc,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(stderr.contains("exited with code 7"), "unexpected stderr:\n{stderr}");
    assert_session_file_status_without_key(tempo_home.path(), "failed");
});

casttest!(
    wallet_session_run_for_retires_local_key_when_revoke_preflight_fails,
    async |_prj, cmd| {
        let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
        let rpc = handle.http_endpoint();
        let tempo_home = tempfile::tempdir().unwrap();
        let path_usd = path_usd();
        let child_dir = tempfile::tempdir().unwrap();
        let child_script = child_dir.path().join("session-child.sh");
        fs::write(
            &child_script,
            r#"#!/bin/sh
set -eu
test -n "${TEMPO_SESSION_ID:-}"
"#,
        )
        .expect("write child script");

        let for_command = format!("sh {}", child_script.to_slash_lossy());

        cmd.cast_fuse();
        cmd.env("TEMPO_HOME", tempo_home.path());
        let stderr = cmd
            .args([
                "wallet",
                "session",
                "--root",
                accounts::ADDR1,
                "--chain-id",
                "31338",
                "--expires",
                "10m",
                "--scope",
                &path_usd,
                "--for",
                &for_command,
                "--private-key",
                accounts::PK1,
                "--rpc-url",
                &rpc,
            ])
            .assert_failure()
            .get_output()
            .stderr_lossy();

        assert!(stderr.contains("created for chain 31338"), "unexpected stderr:\n{stderr}");
        assert_session_file_status_without_key(tempo_home.path(), "revoking");
    }
);
