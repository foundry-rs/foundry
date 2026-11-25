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
bb0:
  0000  PUSH1 0x60
  0002  PUSH1 0x40
  0004  MSTORE
  0005  CALLDATASIZE
  0006  ISZERO
  0007  PUSH2 0x010f
  000a  JUMPI  ; -> bb21, else bb1
bb1:
  000b  PUSH4 0xffffffff
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
    cmd.cast_fuse().args(["disassemble", "60"]).assert_success().stdout_eq(str![[r#"
bb0:
  0000  PUSH1

"#]]);
    cmd.cast_fuse().args(["disassemble", "6000"]).assert_success().stdout_eq(str![[r#"
bb0:
  0000  PUSH1 0x00

"#]]);
    cmd.cast_fuse()
        .args(["disassemble", "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"])
        .assert_success()
        .stdout_eq(str![[r#"
bb0:
  0000  PUSH32 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff

"#]]);
    cmd.cast_fuse()
        .args(["disassemble", "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"])
        .assert_success()
        .stdout_eq(str![[r#"
bb0:
  0000  PUSH32 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff

"#]]);
}

// An internal function called from two places returns to both call sites.
#[casttest]
fn disassemble_dynamic_jump(cmd: _) {
    cmd.args(["disassemble", "6005600d565b600b600d565b005b56"]).assert_success().stdout_eq(str![[
        r#"
bb0:
  0000  PUSH1 0x05
  0002  PUSH1 0x0d
  0004  JUMP  ; -> bb3
bb1:
  0005  JUMPDEST
  0006  PUSH1 0x0b
  0008  PUSH1 0x0d
  000a  JUMP  ; -> bb3
bb2:
  000b  JUMPDEST
  000c  STOP
bb3:
  000d  JUMPDEST
  000e  JUMP  ; -> bb1, bb2

"#
    ]]);
}

#[casttest]
fn disassemble_unknown_opcode(cmd: _) {
    cmd.args(["disassemble", "0c"]).assert_success().stdout_eq(str![[r#"
bb0:
  0000  UNKNOWN(0x0c)

"#]]);
}
