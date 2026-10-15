use chisel::session::ChiselSession;
use foundry_evm::core::evm::EthEvmNetwork;
use foundry_test_utils::{snapbox, str};
use std::{fs, path::Path};

use std::os::unix::fs::PermissionsExt;

fn command(home: &Path) -> snapbox::cmd::Command {
    snapbox::cmd::Command::new(env!("CARGO_BIN_EXE_chisel"))
        .env_clear()
        .env("HOME", home)
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .current_dir(home)
        .args(["--offline", "--no-vm"])
}

#[test]
fn save_omits_environment_and_config_credentials() {
    let home = tempfile::tempdir().unwrap();
    fs::write(
        home.path().join("foundry.toml"),
        r#"
[rpc_endpoints]
private = { endpoint = "https://rpc.invalid/synthetic-token", auth = "Bearer synthetic-auth" }
[etherscan]
mainnet = { key = "synthetic-explorer-key", chain = 1 }
"#,
    )
    .unwrap();

    command(home.path())
        .args(["--use", "0.8.29"])
        .env("ETHERSCAN_API_KEY", "synthetic-api-key")
        .env("FOUNDRY_ETH_RPC_JWT", "synthetic-jwt")
        .env("FOUNDRY_ETH_RPC_HEADERS", r#"["Authorization: Bearer synthetic-header"]"#)
        .args(["eval", "!save credentials"])
        .assert()
        .success()
        .stdout_eq(str![[r#"
Saved session to cache with ID = credentials

"#]])
        .stderr_eq(str![""]);

    let cache = home.path().join(".foundry/cache/chisel");
    let path = cache.join("chisel-credentials.json");
    let session: ChiselSession<EthEvmNetwork> =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let config = session.source.config;
    assert_eq!(config.foundry_config.etherscan_api_key, None);
    assert_eq!(config.foundry_config.eth_rpc_jwt, None);
    assert_eq!(config.foundry_config.eth_rpc_headers, None);
    assert!(config.foundry_config.rpc_endpoints.is_empty());
    assert!(config.foundry_config.etherscan.is_empty());
    assert_eq!(config.evm_opts.rpc_jwt, None);
    assert_eq!(config.evm_opts.rpc_headers, None);
    assert_eq!(fs::metadata(cache).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn view_saved_fork_source_without_endpoint_or_compiler() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir(home.path().join(".svm")).unwrap();
    command(home.path())
        .args(["--use", "0.8.29", "--network", "tempo", "eval", "!save forked"])
        .assert()
        .success()
        .stdout_eq(str![[r#"
Saved session to cache with ID = forked

"#]])
        .stderr_eq(str![""]);
    let path = home.path().join(".foundry/cache/chisel/chisel-forked.json");
    let mut session: ChiselSession<EthEvmNetwork> =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    session.source.config.evm_opts.fork_url = Some("https://rpc.invalid/synthetic-token".into());
    session.source.run_code = "uint256 privateValue = 42;".into();
    fs::write(&path, serde_json::to_vec(&session).unwrap()).unwrap();
    fs::write(home.path().join("prelude.sol"), "invalid Solidity").unwrap();

    for config in
        ["[profile.default]\neth_rpc_url = \"http://127.0.0.1:0\"\n", "[profile.default]\n"]
    {
        fs::write(home.path().join("foundry.toml"), config).unwrap();
        for id in ["forked", "latest"] {
            command(home.path())
                .args(["--prelude", "prelude.sol", "view", id])
                .assert()
                .success()
                .stdout_eq(str![[r#"
Loaded Chisel session! (ID = forked)
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0;

contract REPL {
    /// @notice REPL contract entry point
    function run() public {
        uint256 privateValue = 42;
    }
}


"#]])
                .stderr_eq(str![""]);
        }
    }
    assert_eq!(fs::read_dir(home.path().join(".svm")).unwrap().count(), 0);
}
