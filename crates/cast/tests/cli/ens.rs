//! CLI tests for ens commands.

use super::*;

#[casttest]
fn ens_namehash(cmd: _) {
    cmd.args(["namehash", "emo.eth"]).assert_success().stdout_eq(str![[r#"
0x0a21aaf2f6414aa664deb341d1114351fdb023cad07bf53b28e57c26db681910

"#]]);
}

#[casttest]
fn ens_lookup(cmd: _) {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args([
        "lookup-address",
        "0x28679A1a632125fbBf7A68d850E50623194A709E",
        "--rpc-url",
        &eth_rpc_url,
        "--verify",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
emo.eth

"#]]);
}

#[casttest]
fn ens_resolve(cmd: _) {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args(["resolve-name", "emo.eth", "--rpc-url", &eth_rpc_url, "--verify"])
        .assert_success()
        .stdout_eq(str![[r#"
0x28679A1a632125fbBf7A68d850E50623194A709E

"#]]);
}

#[casttest]
fn ens_resolve_no_dot_eth(cmd: _) {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args(["resolve-name", "emo", "--rpc-url", &eth_rpc_url, "--verify"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: Failed to resolve ENS name: emo

Context:
- [..]
"#]]);
}
