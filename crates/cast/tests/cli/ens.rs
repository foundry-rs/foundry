//! CLI tests for ens commands.

use super::*;

casttest!(ens_namehash, |_prj, cmd| {
    cmd.args(["namehash", "emo.eth"]).assert_success().stdout_eq(str![[r#"
0x0a21aaf2f6414aa664deb341d1114351fdb023cad07bf53b28e57c26db681910

"#]]);
});

casttest!(ens_lookup, |_prj, cmd| {
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
});

casttest!(ens_resolve, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args(["resolve-name", "emo.eth", "--rpc-url", &eth_rpc_url, "--verify"])
        .assert_success()
        .stdout_eq(str![[r#"
0x28679A1a632125fbBf7A68d850E50623194A709E

"#]]);
});

casttest!(ens_resolve_no_dot_eth, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args(["resolve-name", "emo", "--rpc-url", &eth_rpc_url, "--verify"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: Failed to resolve ENS name: emo

Context:
- [..]
"#]]);
});
