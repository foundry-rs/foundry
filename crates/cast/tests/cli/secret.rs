//! CLI tests for `cast secret` and `${NAME}` resolution from the secret store.

use super::*;
use foundry_test_utils::TestCommand;

const PASSWORD: &str = "test-password";

/// Resets `cmd` to `cast` using the secret store in `dir`, unlocked with `password` if given.
fn cast<'a>(cmd: &'a mut TestCommand, dir: &Path, password: Option<&str>) -> &'a mut TestCommand {
    cmd.cast_fuse();
    // Run in the project, whose `foundry.toml` defines the RPC aliases.
    cmd.current_dir(dir.parent().unwrap());
    cmd.env("FOUNDRY_SECRETS_DIR", dir);
    if let Some(password) = password {
        cmd.env("FOUNDRY_SECRETS_PASSWORD", password);
    }
    cmd
}

#[casttest]
fn secret_set_list_get_remove(prj: _, cmd: _) {
    let dir = prj.root().join("secrets");

    cast(&mut cmd, &dir, Some(PASSWORD))
        .args(["secret", "set", "SEPOLIA_RPC_URL"])
        .stdin("https://rpc.example/v2/key123\n")
        .assert_success()
        .stdout_eq("")
        .stderr_eq(str![[r#"
Stored secret `SEPOLIA_RPC_URL` in [..]

"#]]);
    // The file holds no plaintext.
    let raw = fs::read_to_string(dir.join("SEPOLIA_RPC_URL")).unwrap();
    assert!(!raw.contains("key123"));

    cast(&mut cmd, &dir, None).args(["secret", "list"]).assert_success().stdout_eq(str![[r#"
SEPOLIA_RPC_URL

"#]]);
    cast(&mut cmd, &dir, Some(PASSWORD))
        .args(["secret", "get", "SEPOLIA_RPC_URL"])
        .assert_success()
        .stdout_eq(str![[r#"
https://rpc.example/v2/key123

"#]]);
    cast(&mut cmd, &dir, None).args(["secret", "remove", "SEPOLIA_RPC_URL"]).assert_success();
    cast(&mut cmd, &dir, None).args(["secret", "list"]).assert_success().stdout_eq("");
}

#[casttest]
fn secret_store_has_one_password(prj: _, cmd: _) {
    let dir = prj.root().join("secrets");
    cast(&mut cmd, &dir, Some(PASSWORD)).args(["secret", "set", "A"]).stdin("1\n").assert_success();

    cast(&mut cmd, &dir, Some("another-password"))
        .args(["secret", "set", "B"])
        .stdin("2\n")
        .assert_failure()
        .stderr_eq(str![[r#"
Error: wrong password for the secret store

"#]]);
    cast(&mut cmd, &dir, Some(PASSWORD))
        .args(["secret", "set", "../B"])
        .stdin("2\n")
        .assert_failure()
        .stderr_eq(str![[r#"
Error: invalid secret name `../B`: use letters, digits and `_`, and do not start with a digit

"#]]);
}

// `${NAME}` in `[rpc_endpoints]` resolves from the store when `NAME` is not in the environment.
#[casttest]
async fn rpc_alias_resolves_from_secret_store(prj: _, cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let dir = prj.root().join("secrets");
    fs::write(
        prj.root().join("foundry.toml"),
        "[rpc_endpoints]\nlocal = \"${FOUNDRY_TEST_LOCAL_RPC}\"\n",
    )
    .unwrap();

    cast(&mut cmd, &dir, Some(PASSWORD))
        .args(["secret", "set", "FOUNDRY_TEST_LOCAL_RPC"])
        .stdin(format!("{}\n", handle.http_endpoint()))
        .assert_success();

    cast(&mut cmd, &dir, Some(PASSWORD))
        .args(["chain-id", "--rpc-url", "local"])
        .assert_success()
        .stdout_eq(str![[r#"
31337

"#]]);

    // The environment wins over the store.
    cast(&mut cmd, &dir, Some(PASSWORD));
    cmd.env("FOUNDRY_TEST_LOCAL_RPC", "http://127.0.0.1:1");
    cmd.args(["chain-id", "--rpc-url", "local"]).assert_failure();

    // Without a password or a terminal, the store stays locked and says so.
    cast(&mut cmd, &dir, None).args(["chain-id", "--rpc-url", "local"]).assert_failure().stderr_eq(
        str![[r#"
Error: could not read secret `FOUNDRY_TEST_LOCAL_RPC`: `FOUNDRY_TEST_LOCAL_RPC` is in the secret store, which is locked: set FOUNDRY_SECRETS_PASSWORD or run in a terminal
...
"#]],
    );
}

// `ETH_RPC_URL` is read from the store when it is not set in the environment, and the store is
// not touched when `--rpc-url` is given.
#[casttest]
async fn eth_rpc_url_from_secret_store(prj: _, cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let dir = prj.root().join("secrets");
    fs::write(prj.root().join("foundry.toml"), "[rpc_endpoints]\nunused = \"${ETH_RPC_URL}\"\n")
        .unwrap();

    cast(&mut cmd, &dir, Some(PASSWORD))
        .args(["secret", "set", "ETH_RPC_URL"])
        .stdin(format!("{}\n", handle.http_endpoint()))
        .assert_success();

    cast(&mut cmd, &dir, Some(PASSWORD));
    cmd.unset_env("ETH_RPC_URL");
    cmd.args(["chain-id"]).assert_success().stdout_eq(str![[r#"
31337

"#]]);

    // A locked store is not asked for anything when `--rpc-url` is given.
    cast(&mut cmd, &dir, None);
    cmd.unset_env("ETH_RPC_URL");
    cmd.args(["chain-id", "--rpc-url", &handle.http_endpoint()]).assert_success().stderr_eq("");
}

#[casttest]
async fn inaccessible_stored_rpc_never_uses_fallback(prj: _, cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let dir = prj.root().join("secrets");
    fs::write(
        prj.root().join("foundry.toml"),
        format!("[profile.default]\neth_rpc_url = \"{}\"\n", handle.http_endpoint()),
    )
    .unwrap();
    cast(&mut cmd, &dir, Some(PASSWORD))
        .args(["secret", "set", "ETH_RPC_URL"])
        .stdin("http://127.0.0.1:1\n")
        .assert_success();

    cast(&mut cmd, &dir, Some("wrong"));
    cmd.unset_env("ETH_RPC_URL");
    cmd.args(["chain-id"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: failed to extract foundry config:
foundry config error: wrong password for the secret store

"#]]);

    cast(&mut cmd, &dir, None);
    cmd.unset_env("ETH_RPC_URL");
    cmd.args(["chain-id"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: failed to extract foundry config:
foundry config error: `ETH_RPC_URL` is in the secret store, which is locked: set FOUNDRY_SECRETS_PASSWORD or run in a terminal

"#]]);

    fs::write(dir.join("ETH_RPC_URL"), "invalid json").unwrap();
    cast(&mut cmd, &dir, Some(PASSWORD));
    cmd.unset_env("ETH_RPC_URL");
    cmd.args(["chain-id"]).assert_failure().stdout_eq("").stderr_eq(str![[r#"
Error: failed to extract foundry config:
foundry config error: secret store: serde-json: expected value at line 1 column 1

"#]]);
}

#[cfg(unix)]
#[casttest]
fn interrupted_secret_update_preserves_previous_value(prj: _, cmd: _) {
    let dir = prj.root().join("secrets");
    cast(&mut cmd, &dir, Some(PASSWORD))
        .args(["secret", "set", "A"])
        .stdin("original\n")
        .assert_success();
    let original = fs::read(dir.join("A")).unwrap();

    // Force a partial write and SIGXFSZ in the child, not in the test runner.
    let mut limited = Command::new("sh");
    limited
        .args(["-c", "ulimit -f 1; exec \"$@\"", "sh"])
        .arg(prj.ensure_foundry_bin("cast"))
        .args(["secret", "set", "A"])
        .env("FOUNDRY_SECRETS_DIR", &dir)
        .env("FOUNDRY_SECRETS_PASSWORD", PASSWORD)
        .current_dir(prj.root());
    cmd.set_cmd(limited).stdin("replacement".repeat(1024)).assert_failure();

    assert_eq!(fs::read(dir.join("A")).unwrap(), original);
    cast(&mut cmd, &dir, Some(PASSWORD))
        .args(["secret", "get", "A"])
        .assert_success()
        .stdout_eq("original\n");
}
