use super::{EtherscanOpts, RpcOpts};
use crate::{
    cmd::cast::{
        access_list::AccessListArgs, bind::BindArgs, call::CallArgs, create2::Create2Args,
        estimate::EstimateArgs, find_block::FindBlockArgs, interface::InterfaceArgs,
        logs::LogsArgs, rpc::RpcArgs, run::RunArgs, send::SendTxArgs, storage::StorageArgs,
        wallet::WalletSubcommands,
    },
    utils::parse_u256,
};
use clap::{Parser, Subcommand, ValueHint};
use ethers::{
    abi::ethabi::ethereum_types::BigEndianHash,
    types::{serde_helpers::Numeric, Address, BlockId, NameOrAddress, H256, U256},
};
use std::{path::PathBuf, str::FromStr};

#[derive(Debug, Parser)]
#[clap(name = "cast", version = crate::utils::VERSION_MESSAGE)]
pub struct Opts {
    #[clap(subcommand)]
    pub sub: Subcommands,
}

/// Perform Ethereum RPC calls from the comfort of your command line.
#[derive(Debug, Subcommand)]
#[clap(
    after_help = "Find more information in the book: http://book.getfoundry.sh/reference/cast/cast.html",
    next_display_order = None
)]
pub enum Subcommands {
    /// Prints the maximum value of the given integer type.
    #[clap(visible_aliases = &["--max-int", "maxi"])]
    MaxInt {
        /// The integer type to get the maximum value of.
        #[clap(default_value = "int256")]
        r#type: String,
    },

    /// Prints the minimum value of the given integer type.
    #[clap(visible_aliases = &["--min-int", "mini"])]
    MinInt {
        /// The integer type to get the minimum value of.
        #[clap(default_value = "int256")]
        r#type: String,
    },

    /// Prints the maximum value of the given integer type.
    #[clap(visible_aliases = &["--max-uint", "maxu"])]
    MaxUint {
        /// The unsigned integer type to get the maximum value of.
        #[clap(default_value = "uint256")]
        r#type: String,
    },

    /// Prints the zero address.
    #[clap(visible_aliases = &["--address-zero", "az"])]
    AddressZero,

    /// Prints the zero hash.
    #[clap(visible_aliases = &["--hash-zero", "hz"])]
    HashZero,

    /// Convert UTF8 text to hex.
    #[clap(
        visible_aliases = &[
        "--from-ascii",
        "from-ascii",
        "fu",
        "fa"]
    )]
    FromUtf8 {
        /// The text to convert.
        text: Option<String>,
    },

    /// Concatenate hex strings.
    #[clap(visible_aliases = &["--concat-hex", "ch"])]
    ConcatHex {
        /// The data to concatenate.
        data: Vec<String>,
    },

    /// "Convert binary data into hex data."
    #[clap(visible_aliases = &["--from-bin", "from-binx", "fb"])]
    FromBin,

    /// Normalize the input to lowercase, 0x-prefixed hex.
    ///
    /// The input can be:
    /// - mixed case hex with or without 0x prefix
    /// - 0x prefixed hex, concatenated with a ':'
    /// - an absolute path to file
    /// - @tag, where the tag is defined in an environment variable
    #[clap(visible_aliases = &["--to-hexdata", "thd", "2hd"])]
    ToHexdata {
        /// The input to normalize.
        input: Option<String>,
    },

    /// Convert an address to a checksummed format (EIP-55).
    #[clap(
        visible_aliases = &["--to-checksum-address",
        "--to-checksum",
        "to-checksum",
        "ta",
        "2a"]
    )]
    ToCheckSumAddress {
        /// The address to convert.
        address: Option<Address>,
    },

    /// Convert hex data to an ASCII string.
    #[clap(visible_aliases = &["--to-ascii", "tas", "2as"])]
    ToAscii {
        /// The hex data to convert.
        hexdata: Option<String>,
    },

    /// Convert a fixed point number into an integer.
    #[clap(visible_aliases = &["--from-fix", "ff"])]
    FromFixedPoint {
        /// The number of decimals to use.
        decimals: Option<String>,

        /// The value to convert.
        #[clap(allow_hyphen_values = true)]
        value: Option<String>,
    },

    /// Right-pads hex data to 32 bytes.
    #[clap(visible_aliases = &["--to-bytes32", "tb", "2b"])]
    ToBytes32 {
        /// The hex data to convert.
        bytes: Option<String>,
    },

    /// Convert an integer into a fixed point number.
    #[clap(visible_aliases = &["--to-fix", "tf", "2f"])]
    ToFixedPoint {
        /// The number of decimals to use.
        decimals: Option<String>,

        /// The value to convert.
        #[clap(allow_hyphen_values = true)]
        value: Option<String>,
    },

    /// Convert a number to a hex-encoded uint256.
    #[clap(name = "to-uint256", visible_aliases = &["--to-uint256", "tu", "2u"])]
    ToUint256 {
        /// The value to convert.
        value: Option<String>,
    },

    /// Convert a number to a hex-encoded int256.
    #[clap(name = "to-int256", visible_aliases = &["--to-int256", "ti", "2i"])]
    ToInt256 {
        /// The value to convert.
        value: Option<String>,
    },

    /// Perform a left shifting operation
    #[clap(name = "shl")]
    LeftShift {
        /// The value to shift.
        value: String,

        /// The number of bits to shift.
        bits: String,

        /// The input base.
        #[clap(long)]
        base_in: Option<String>,

        /// The output base.
        #[clap(long, default_value = "16")]
        base_out: String,
    },

    /// Perform a right shifting operation
    #[clap(name = "shr")]
    RightShift {
        /// The value to shift.
        value: String,

        /// The number of bits to shift.
        bits: String,

        /// The input base,
        #[clap(long)]
        base_in: Option<String>,

        /// The output base,
        #[clap(long, default_value = "16")]
        base_out: String,
    },

    /// Convert an ETH amount into another unit (ether, gwei or wei).
    ///
    /// Examples:
    /// - 1ether wei
    /// - "1 ether" wei
    /// - 1ether
    /// - 1 gwei
    /// - 1gwei ether
    #[clap(visible_aliases = &["--to-unit", "tun", "2un"])]
    ToUnit {
        /// The value to convert.
        value: Option<String>,

        /// The unit to convert to (ether, gwei, wei).
        #[clap(default_value = "wei")]
        unit: String,
    },

    /// Convert an ETH amount to wei.
    ///
    /// Consider using --to-unit.
    #[clap(visible_aliases = &["--to-wei", "tw", "2w"])]
    ToWei {
        /// The value to convert.
        #[clap(allow_hyphen_values = true)]
        value: Option<String>,

        /// The unit to convert from (ether, gwei, wei).
        #[clap(default_value = "eth")]
        unit: String,
    },

    /// Convert wei into an ETH amount.
    ///
    /// Consider using --to-unit.
    #[clap(visible_aliases = &["--from-wei", "fw"])]
    FromWei {
        /// The value to convert.
        #[clap(allow_hyphen_values = true)]
        value: Option<String>,

        /// The unit to convert from (ether, gwei, wei).
        #[clap(default_value = "eth")]
        unit: String,
    },

    /// RLP encodes hex data, or an array of hex data
    #[clap(visible_aliases = &["--to-rlp"])]
    ToRlp {
        /// The value to convert.
        value: Option<String>,
    },

    /// Decodes RLP encoded data.
    ///
    /// Input must be hexadecimal.
    #[clap(visible_aliases = &["--from-rlp"])]
    FromRlp {
        /// The value to convert.
        value: Option<String>,
    },

    /// Converts a number of one base to another
    #[clap(visible_aliases = &["--to-hex", "th", "2h"])]
    ToHex(ToBaseArgs),

    /// Converts a number of one base to decimal
    #[clap(visible_aliases = &["--to-dec", "td", "2d"])]
    ToDec(ToBaseArgs),

    /// Converts a number of one base to another
    #[clap(
        visible_aliases = &["--to-base",
        "--to-radix",
        "to-radix",
        "tr",
        "2r"]
    )]
    ToBase {
        #[clap(flatten)]
        base: ToBaseArgs,

        /// The output base.
        #[clap(value_name = "BASE")]
        base_out: Option<String>,
    },
    /// Create an access list for a transaction.
    #[clap(visible_aliases = &["ac", "acl"])]
    AccessList(AccessListArgs),
    /// Get logs by signature or topic.
    #[clap(visible_alias = "l")]
    Logs(LogsArgs),
    /// Get information about a block.
    #[clap(visible_alias = "bl")]
    Block {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        block: Option<BlockId>,

        /// If specified, only get the given field of the block.
        #[clap(long, short)]
        field: Option<String>,

        #[clap(long, env = "CAST_FULL_BLOCK")]
        full: bool,

        /// Print the block as JSON.
        #[clap(long, short, help_heading = "Display options")]
        json: bool,

        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Get the latest block number.
    #[clap(visible_alias = "bn")]
    BlockNumber {
        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Perform a call on an account without publishing a transaction.
    #[clap(visible_alias = "c")]
    Call(CallArgs),

    /// ABI-encode a function with arguments.
    #[clap(name = "calldata", visible_alias = "cd")]
    CalldataEncode {
        /// The function signature in the form <name>(<types...>)
        sig: String,

        /// The arguments to encode.
        #[clap(allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Get the symbolic name of the current chain.
    Chain {
        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Get the Ethereum chain ID.
    #[clap(visible_aliases = &["ci", "cid"])]
    ChainId {
        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Get the current client version.
    #[clap(visible_alias = "cl")]
    Client {
        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Compute the contract address from a given nonce and deployer address.
    #[clap(visible_alias = "ca")]
    ComputeAddress {
        /// The deployer address.
        address: Option<String>,

        /// The nonce of the deployer address.
        #[clap(long, value_parser = parse_u256)]
        nonce: Option<U256>,

        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Disassembles hex encoded bytecode into individual / human readable opcodes
    #[clap(visible_alias = "da")]
    Disassemble {
        /// The hex encoded bytecode.
        bytecode: String,
    },

    /// Calculate the ENS namehash of a name.
    #[clap(visible_aliases = &["na", "nh"])]
    Namehash { name: Option<String> },

    /// Get information about a transaction.
    #[clap(visible_alias = "t")]
    Tx {
        /// The transaction hash.
        tx_hash: String,

        /// If specified, only get the given field of the transaction. If "raw", the RLP encoded
        /// transaction will be printed.
        field: Option<String>,

        /// Print the raw RLP encoded transaction.
        #[clap(long)]
        raw: bool,

        /// Print as JSON.
        #[clap(long, short, help_heading = "Display options")]
        json: bool,

        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Get the transaction receipt for a transaction.
    #[clap(visible_alias = "re")]
    Receipt {
        /// The transaction hash.
        tx_hash: String,

        /// If specified, only get the given field of the transaction.
        field: Option<String>,

        /// The number of confirmations until the receipt is fetched
        #[clap(long, default_value = "1")]
        confirmations: usize,

        /// Exit immediately if the transaction was not found.
        #[clap(long = "async", env = "CAST_ASYNC", name = "async", alias = "cast-async")]
        cast_async: bool,

        /// Print as JSON.
        #[clap(long, short, help_heading = "Display options")]
        json: bool,

        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Sign and publish a transaction.
    #[clap(name = "send", visible_alias = "s")]
    SendTx(SendTxArgs),

    /// Publish a raw transaction to the network.
    #[clap(name = "publish", visible_alias = "p")]
    PublishTx {
        /// The raw transaction
        raw_tx: String,

        /// Only print the transaction hash and exit immediately.
        #[clap(long = "async", env = "CAST_ASYNC", name = "async", alias = "cast-async")]
        cast_async: bool,

        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Estimate the gas cost of a transaction.
    #[clap(visible_alias = "e")]
    Estimate(EstimateArgs),

    /// Decode ABI-encoded input data.
    ///
    /// Similar to `abi-decode --input`, but function selector MUST be prefixed in `calldata`
    /// string
    #[clap(visible_aliases = &["--calldata-decode","cdd"])]
    CalldataDecode {
        /// The function signature in the format `<name>(<in-types>)(<out-types>)`.
        sig: String,

        /// The ABI-encoded calldata.
        calldata: String,
    },

    /// Decode ABI-encoded input or output data.
    ///
    /// Defaults to decoding output data. To decode input data pass --input.
    ///
    /// When passing `--input`, function selector must NOT be prefixed in `calldata` string
    #[clap(name = "abi-decode", visible_aliases = &["ad", "--abi-decode"])]
    AbiDecode {
        /// The function signature in the format `<name>(<in-types>)(<out-types>)`.
        sig: String,

        /// The ABI-encoded calldata.
        calldata: String,

        /// Whether to decode the input or output data.
        #[clap(long, short, help_heading = "Decode input data instead of output data")]
        input: bool,
    },

    /// ABI encode the given function argument, excluding the selector.
    #[clap(visible_alias = "ae")]
    AbiEncode {
        /// The function signature.
        sig: String,

        /// The arguments of the function.
        #[clap(allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Compute the storage slot for an entry in a mapping.
    #[clap(visible_alias = "in")]
    Index {
        /// The mapping key type.
        key_type: String,

        /// The mapping key.
        key: String,

        /// The storage slot of the mapping.
        slot_number: String,
    },

    /// Fetch the EIP-1967 implementation account
    #[clap(visible_alias = "impl")]
    Implementation {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        #[clap(long, short = 'B')]
        block: Option<BlockId>,

        /// The address to get the nonce for.
        #[clap(value_parser = NameOrAddress::from_str)]
        who: NameOrAddress,

        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Fetch the EIP-1967 admin account
    #[clap(visible_alias = "adm")]
    Admin {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        #[clap(long, short = 'B')]
        block: Option<BlockId>,

        /// The address to get the nonce for.
        #[clap(value_parser = NameOrAddress::from_str)]
        who: NameOrAddress,

        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Get the function signatures for the given selector from https://openchain.xyz.
    #[clap(name = "4byte", visible_aliases = &["4", "4b"])]
    FourByte {
        /// The function selector.
        selector: Option<String>,
    },

    /// Decode ABI-encoded calldata using https://openchain.xyz.
    #[clap(name = "4byte-decode", visible_aliases = &["4d", "4bd"])]
    FourByteDecode {
        /// The ABI-encoded calldata.
        calldata: Option<String>,
    },

    /// Get the event signature for a given topic 0 from https://openchain.xyz.
    #[clap(name = "4byte-event", visible_aliases = &["4e", "4be"])]
    FourByteEvent {
        /// Topic 0
        #[clap(value_name = "TOPIC_0")]
        topic: Option<String>,
    },

    /// Upload the given signatures to https://openchain.xyz.
    ///
    /// Example inputs:
    /// - "transfer(address,uint256)"
    /// - "function transfer(address,uint256)"
    /// - "function transfer(address,uint256)" "event Transfer(address,address,uint256)"
    /// - "./out/Contract.sol/Contract.json"
    #[clap(visible_aliases = &["ups"])]
    UploadSignature {
        /// The signatures to upload.
        ///
        /// Prefix with 'function', 'event', or 'error'. Defaults to function if no prefix given.
        /// Can also take paths to contract artifact JSON.
        signatures: Vec<String>,
    },

    /// Pretty print calldata.
    ///
    /// Tries to decode the calldata using https://openchain.xyz unless --offline is passed.
    #[clap(visible_alias = "pc")]
    PrettyCalldata {
        /// The calldata.
        calldata: Option<String>,

        /// Skip the https://openchain.xyz lookup.
        #[clap(long, short)]
        offline: bool,
    },

    /// Get the timestamp of a block.
    #[clap(visible_alias = "a")]
    Age {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        block: Option<BlockId>,

        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Get the balance of an account in wei.
    #[clap(visible_alias = "b")]
    Balance {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        #[clap(long, short = 'B')]
        block: Option<BlockId>,

        /// The account to query.
        #[clap(value_parser = NameOrAddress::from_str)]
        who: NameOrAddress,

        /// Format the balance in ether.
        #[clap(long, short)]
        ether: bool,

        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Get the basefee of a block.
    #[clap(visible_aliases = &["ba", "fee", "basefee"])]
    BaseFee {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        block: Option<BlockId>,

        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Get the runtime bytecode of a contract.
    #[clap(visible_alias = "co")]
    Code {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        #[clap(long, short = 'B')]
        block: Option<BlockId>,

        /// The contract address.
        #[clap(value_parser = NameOrAddress::from_str)]
        who: NameOrAddress,

        /// Disassemble bytecodes into individual opcodes.
        #[clap(long, short)]
        disassemble: bool,

        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Get the runtime bytecode size of a contract.
    #[clap(visible_alias = "cs")]
    Codesize {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        #[clap(long, short = 'B')]
        block: Option<BlockId>,

        /// The contract address.
        #[clap(value_parser = NameOrAddress::from_str)]
        who: NameOrAddress,

        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Get the current gas price.
    #[clap(visible_alias = "g")]
    GasPrice {
        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Generate event signatures from event string.
    #[clap(visible_alias = "se")]
    SigEvent {
        /// The event string.
        event_string: Option<String>,
    },

    /// Hash arbitrary data using Keccak-256.
    #[clap(visible_alias = "k")]
    Keccak {
        /// The data to hash.
        data: Option<String>,
    },

    /// Perform an ENS lookup.
    #[clap(visible_alias = "rn")]
    ResolveName {
        /// The name to lookup.
        who: Option<String>,

        /// Perform a reverse lookup to verify that the name is correct.
        #[clap(long, short)]
        verify: bool,

        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Perform an ENS reverse lookup.
    #[clap(visible_alias = "la")]
    LookupAddress {
        /// The account to perform the lookup for.
        who: Option<Address>,

        /// Perform a normal lookup to verify that the address is correct.
        #[clap(long, short)]
        verify: bool,

        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Get the raw value of a contract's storage slot.
    #[clap(visible_alias = "st")]
    Storage(StorageArgs),

    /// Generate a storage proof for a given storage slot.
    #[clap(visible_alias = "pr")]
    Proof {
        /// The contract address.
        #[clap(value_parser = NameOrAddress::from_str)]
        address: NameOrAddress,

        /// The storage slot numbers (hex or decimal).
        #[clap(value_parser = parse_slot)]
        slots: Vec<H256>,

        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        #[clap(long, short = 'B')]
        block: Option<BlockId>,

        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Get the nonce for an account.
    #[clap(visible_alias = "n")]
    Nonce {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        #[clap(long, short = 'B')]
        block: Option<BlockId>,

        /// The address to get the nonce for.
        #[clap(value_parser = NameOrAddress::from_str)]
        who: NameOrAddress,

        #[clap(flatten)]
        rpc: RpcOpts,
    },

    /// Get the source code of a contract from Etherscan.
    #[clap(visible_aliases = &["et", "src"])]
    EtherscanSource {
        /// The contract's address.
        address: String,

        /// The output directory to expand source tree into.
        #[clap(short, value_hint = ValueHint::DirPath)]
        directory: Option<PathBuf>,

        #[clap(flatten)]
        etherscan: EtherscanOpts,
    },

    /// Wallet management utilities.
    #[clap(visible_alias = "w")]
    Wallet {
        #[clap(subcommand)]
        command: WalletSubcommands,
    },

    /// Generate a Solidity interface from a given ABI.
    ///
    /// Currently does not support ABI encoder v2.
    #[clap(visible_alias = "i")]
    Interface(InterfaceArgs),

    /// Generate a rust binding from a given ABI.
    #[clap(visible_alias = "bi")]
    Bind(BindArgs),

    /// Get the selector for a function.
    #[clap(visible_alias = "si")]
    Sig {
        /// The function signature, e.g. transfer(address,uint256).
        sig: Option<String>,

        /// Optimize signature to contain provided amount of leading zeroes in selector.
        optimize: Option<usize>,
    },

    /// Generate a deterministic contract address using CREATE2.
    #[clap(visible_alias = "c2")]
    Create2(Create2Args),

    /// Get the block number closest to the provided timestamp.
    #[clap(visible_alias = "f")]
    FindBlock(FindBlockArgs),

    /// Generate shell completions script.
    #[clap(visible_alias = "com")]
    Completions {
        #[clap(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Generate Fig autocompletion spec.
    #[clap(visible_alias = "fig")]
    GenerateFigSpec,

    /// Runs a published transaction in a local environment and prints the trace.
    #[clap(visible_alias = "r")]
    Run(RunArgs),

    /// Perform a raw JSON-RPC request.
    #[clap(visible_alias = "rp")]
    Rpc(RpcArgs),

    /// Formats a string into bytes32 encoding.
    #[clap(name = "format-bytes32-string", visible_aliases = &["--format-bytes32-string"])]
    FormatBytes32String {
        /// The string to format.
        string: Option<String>,
    },

    /// Parses a string from bytes32 encoding.
    #[clap(name = "parse-bytes32-string", visible_aliases = &["--parse-bytes32-string"])]
    ParseBytes32String {
        /// The string to parse.
        bytes: Option<String>,
    },
    #[clap(name = "parse-bytes32-address", visible_aliases = &["--parse-bytes32-address"])]
    #[clap(about = "Parses a checksummed address from bytes32 encoding.")]
    ParseBytes32Address {
        #[clap(value_name = "BYTES")]
        bytes: Option<String>,
    },
}

/// CLI arguments for `cast --to-base`.
#[derive(Debug, Parser)]
pub struct ToBaseArgs {
    /// The value to convert.
    #[clap(allow_hyphen_values = true)]
    pub value: Option<String>,

    /// The input base.
    #[clap(long, short = 'i')]
    pub base_in: Option<String>,
}

pub fn parse_slot(s: &str) -> eyre::Result<H256> {
    Numeric::from_str(s)
        .map_err(|e| eyre::eyre!("Could not parse slot number: {e}"))
        .map(|n| H256::from_uint(&n.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::types::BlockNumber;

    #[test]
    fn parse_call_data() {
        let args: Opts = Opts::parse_from([
            "foundry-cli",
            "calldata",
            "f()",
            "5c9d55b78febcc2061715ba4f57ecf8ea2711f2c",
            "2",
        ]);
        match args.sub {
            Subcommands::CalldataEncode { args, .. } => {
                assert_eq!(
                    args,
                    vec!["5c9d55b78febcc2061715ba4f57ecf8ea2711f2c".to_string(), "2".to_string()]
                )
            }
            _ => unreachable!(),
        };
    }

    #[test]
    fn parse_block_ids() {
        struct TestCase {
            input: String,
            expect: BlockId,
        }

        let test_cases = [
            TestCase {
                input: "0".to_string(),
                expect: BlockId::Number(BlockNumber::Number(0u64.into())),
            },
            TestCase {
                input: "0x56462c47c03df160f66819f0a79ea07def1569f8aac0fe91bb3a081159b61b4a"
                    .to_string(),
                expect: BlockId::Hash(
                    "0x56462c47c03df160f66819f0a79ea07def1569f8aac0fe91bb3a081159b61b4a"
                        .parse()
                        .unwrap(),
                ),
            },
            TestCase { input: "latest".to_string(), expect: BlockId::Number(BlockNumber::Latest) },
            TestCase {
                input: "earliest".to_string(),
                expect: BlockId::Number(BlockNumber::Earliest),
            },
            TestCase {
                input: "pending".to_string(),
                expect: BlockId::Number(BlockNumber::Pending),
            },
            TestCase { input: "safe".to_string(), expect: BlockId::Number(BlockNumber::Safe) },
            TestCase {
                input: "finalized".to_string(),
                expect: BlockId::Number(BlockNumber::Finalized),
            },
        ];

        for test in test_cases {
            let result: BlockId = test.input.parse().unwrap();
            assert_eq!(result, test.expect);
        }
    }
}
