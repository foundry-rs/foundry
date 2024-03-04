use alloy_primitives::Address;
use alloy_rpc_types::BlockId;
use clap::{Parser, ValueHint};
use std::path::PathBuf;
/// CLI arguments for `forge verify-bytecode`.
#[derive(Clone, Debug, Parser)]
pub struct VerifyBytecodeArgs {
    /// The address of the contract to verify.
    pub address: Address,

    /// The block at which the bytecode should be verified.
    #[clap(long, value_name = "BLOCK")]
    pub block: Option<BlockId>,

    /// The constructor args to generate the creation code.
    #[clap(
        long,
        conflicts_with = "constructor_args_path",
        value_name = "ARGS",
        visible_alias = "encoded-constructor-args"
    )]
    pub constructor_args: Option<String>,

    /// The path to a file containing the constructor arguments.
    #[clap(long, value_hint = ValueHint::FilePath, value_name = "PATH")]
    pub constructor_args_path: Option<PathBuf>,

    /// The rpc url to use for verification.
    #[clap(long, value_name = "RPC_URL")]
    pub rpc_url: Option<String>,

    /// Verfication Type: `full` or `partial`. Ref: https://docs.sourcify.dev/docs/full-vs-partial-match/
    #[clap(long, default_value = "full", value_name = "TYPE")]
    pub verification_type: String,
}
