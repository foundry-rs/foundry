//! CLI tests for bytecode commands.

use super::*;
use alloy_primitives::KECCAK256_EMPTY;

// <https://github.com/foundry-rs/foundry/issues/10945>
// tests `cast code --disassemble`
#[casttest]
fn can_disassemble_contract_code(cmd: _) {
    let rpc = next_http_rpc_endpoint();
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
    .stdout_eq(format!("{KECCAK256_EMPTY}\n"));
}

#[casttest]
fn disassemble_incomplete_sequence(cmd: _) {
    cmd.cast_fuse().args(["disassemble", "60"]).assert_success().stdout_eq("00000000: PUSH1\n\n");
    cmd.cast_fuse()
        .args(["disassemble", "e6"])
        .assert_success()
        .stdout_eq("00000000: DUPN 145\n\n");
    cmd.cast_fuse()
        .args(["disassemble", "6000"])
        .assert_success()
        .stdout_eq("00000000: PUSH1 0x00\n\n");
    cmd.cast_fuse()
        .args(["disassemble", "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"])
        .assert_success()
        .stdout_eq(format!("00000000: PUSH32 0x{}\n\n", "ff".repeat(31)));
    cmd.cast_fuse().args(["disassemble", "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"]).assert_success().stdout_eq("00000000: PUSH32 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\n\n");
}

// EIP-8024 immediates are decoded, and opcode names follow the latest spec.
#[casttest]
fn disassemble_spec_names(cmd: _) {
    cmd.args(["disassemble", "44e600e780e88e00"]).assert_success().stdout_eq(str![[r#"
00000000: PREVRANDAO
00000001: DUPN 145
00000003: SWAPN 17
00000005: EXCHANGE 1, 2
00000007: STOP


"#]]);
}

#[casttest]
fn disassemble_unknown_opcode(cmd: _) {
    cmd.args(["disassemble", "0c"]).assert_success().stdout_eq(str![[r#"
00000000: UNKNOWN(0x0C)


"#]]);
}
