//! CLI tests for `cast keychain` subcommands.

use anvil::NodeConfig;
use foundry_test_utils::util::OutputExt;

/// Anvil test accounts (standard mnemonic).
mod accounts {
    pub const PK1: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    pub const ADDR1: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    pub const ADDR2: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    pub const TOKEN: &str = "0x20C000000000000000000000b9537d11c60E8b50"; // PathUSD
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
