//! Tests for Cast command-line argument parsing.

use super::*;
use clap::CommandFactory;

#[test]
fn verify_cli() {
    Cast::command().debug_assert();
}

#[test]
fn parse_proof_slot() {
    let args: Cast = Cast::parse_from([
        "foundry-cli",
        "proof",
        "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
        "0",
        "1",
        "0x0000000000000000000000000000000000000000000000000000000000000000",
        "0x1",
        "0x01",
    ]);
    match args.cmd {
        CastSubcommand::Proof { slots, .. } => {
            assert_eq!(
                slots,
                vec![
                    B256::ZERO,
                    U256::from(1).into(),
                    B256::ZERO,
                    U256::from(1).into(),
                    U256::from(1).into()
                ]
            );
        }
        _ => unreachable!(),
    };
}

#[test]
fn parse_call_data() {
    let args: Cast = Cast::parse_from([
        "foundry-cli",
        "calldata",
        "f()",
        "5c9d55b78febcc2061715ba4f57ecf8ea2711f2c",
        "2",
    ]);
    match args.cmd {
        CastSubcommand::CalldataEncode { args, .. } => {
            assert_eq!(
                args,
                vec!["5c9d55b78febcc2061715ba4f57ecf8ea2711f2c".to_string(), "2".to_string()]
            )
        }
        _ => unreachable!(),
    };
}

#[test]
fn parse_call_data_with_file() {
    let args: Cast = Cast::parse_from(["foundry-cli", "calldata", "f()", "--file", "test.txt"]);
    match args.cmd {
        CastSubcommand::CalldataEncode { sig, file, args } => {
            assert_eq!(sig, "f()".to_string());
            assert_eq!(file, Some(PathBuf::from("test.txt")));
            assert!(args.is_empty());
        }
        _ => unreachable!(),
    };
}

// <https://github.com/foundry-rs/book/issues/1019>
#[test]
fn parse_signature() {
    let args: Cast = Cast::parse_from([
        "foundry-cli",
        "sig",
        "__$_$__$$$$$__$$_$$$_$$__$$___$$(address,address,uint256)",
    ]);
    match args.cmd {
        CastSubcommand::Sig { sig, .. } => {
            let sig = sig.unwrap();
            assert_eq!(
                sig,
                "__$_$__$$$$$__$$_$$$_$$__$$___$$(address,address,uint256)".to_string()
            );

            let selector = foundry_common::abi::get_func(&sig).unwrap().selector();
            assert_eq!(selector.to_string(), "0x23b872dd");
        }
        _ => unreachable!(),
    };
}
