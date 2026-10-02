//! CLI tests for bytecode commands.

use super::*;

// <https://github.com/foundry-rs/foundry/issues/10945>
// tests `cast code --disassemble`
#[casttest]
fn can_disassemble_contract_code(cmd: _) {
    let rpc = next_rpc_endpoint(NamedChain::Mainnet);
    cmd.args([
        "code",
        "--disassemble",
        "--rpc-url",
        rpc.as_str(),
        "0x1F573D6Fb3F13d689FF844B4cE37794d79a7FF1C",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
00000000: PUSH1 0x60
00000002: PUSH1 0x40
00000004: MSTORE
00000005: CALLDATASIZE
00000006: ISZERO
00000007: PUSH2 0x010f
0000000a: JUMPI
0000000b: PUSH4 0xffffffff
00000010: PUSH29 0x0100000000000000000000000000000000000000000000000000000000
0000002e: PUSH1 0x00
...
"#]]);
}

#[casttest]
async fn code_empty(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    cmd.args([
        "code",
        "0x0000000000000000000000000000000000000000",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq("0x\n");
}

#[casttest]
async fn codesize_empty(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    cmd.args([
        "codesize",
        "0x0000000000000000000000000000000000000000",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq("0\n");
}

#[casttest]
async fn codehash_empty(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    cmd.args([
        "codehash",
        "0x0000000000000000000000000000000000000000",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq(format!("{}\n", keccak256([])));
}

#[casttest]
fn disassemble_incomplete_sequence(cmd: _) {
    cmd.cast_fuse().args(["disassemble", "60"]).assert_success().stdout_eq("00000000: PUSH1\n\n");
    cmd.cast_fuse()
        .args(["disassemble", "6000"])
        .assert_success()
        .stdout_eq("00000000: PUSH1 0x00\n\n");
    cmd.cast_fuse()
        .args(["disassemble", "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"])
        .assert_success()
        .stdout_eq("00000000: PUSH32\n\n");
    cmd.cast_fuse().args(["disassemble", "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"]).assert_success().stdout_eq("00000000: PUSH32 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\n\n");
}
