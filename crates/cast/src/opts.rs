use crate::cmd::{
    access_list::AccessListArgs, artifact::ArtifactArgs, bind::BindArgs, call::CallArgs,
    constructor_args::ConstructorArgsArgs, create2::Create2Args, creation_code::CreationCodeArgs,
    da_estimate::DAEstimateArgs, estimate::EstimateArgs, find_block::FindBlockArgs,
    interface::InterfaceArgs, logs::LogsArgs, mktx::MakeTxArgs, rpc::RpcArgs, run::RunArgs,
    send::SendTxArgs, storage::StorageArgs, txpool::TxPoolSubcommands, wallet::WalletSubcommands,
};
use alloy_ens::NameOrAddress;
use alloy_primitives::{Address, B256, Selector, U256};
use alloy_rpc_types::BlockId;
use clap::{Parser, Subcommand, ValueHint};
use eyre::Result;
use foundry_cli::opts::{EtherscanOpts, GlobalArgs, RpcOpts};
use foundry_common::version::{LONG_VERSION, SHORT_VERSION};
use std::{path::PathBuf, str::FromStr};

/// A Swiss Army knife for interacting with Ethereum applications from the command line.
#[derive(Parser)]
#[command(
    name = "cast",
    version = SHORT_VERSION,
    long_version = LONG_VERSION,
    after_help = "Find more information in the book: https://getfoundry.sh/cast/overview",
    next_display_order = None,
)]
pub struct Cast {
    /// Include the global arguments.
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub cmd: CastSubcommand,
}

#[derive(Subcommand)]
pub enum CastSubcommand {
    /// Prints the maximum value of the given integer type.
    #[command(visible_aliases = &["--max-int", "maxi"])]
    MaxInt {
        /// The integer type to get the maximum value of.
        #[arg(default_value = "int256")]
        r#type: String,
    },

    /// Prints the minimum value of the given integer type.
    #[command(visible_aliases = &["--min-int", "mini"])]
    MinInt {
        /// The integer type to get the minimum value of.
        #[arg(default_value = "int256")]
        r#type: String,
    },

    /// Prints the maximum value of the given integer type.
    #[command(visible_aliases = &["--max-uint", "maxu"])]
    MaxUint {
        /// The unsigned integer type to get the maximum value of.
        #[arg(default_value = "uint256")]
        r#type: String,
    },

    /// Prints the zero address.
    #[command(visible_aliases = &["--address-zero", "az"])]
    AddressZero,

    /// Prints the zero hash.
    #[command(visible_aliases = &["--hash-zero", "hz"])]
    HashZero,

    /// Convert UTF8 text to hex.
    #[command(
        visible_aliases = &[
        "--from-ascii",
        "--from-utf8",
        "from-ascii",
        "fu",
        "fa"]
    )]
    FromUtf8 {
        /// The text to convert.
        text: Option<String>,
    },

    /// Concatenate hex strings.
    #[command(visible_aliases = &["--concat-hex", "ch"])]
    ConcatHex {
        /// The data to concatenate.
        data: Vec<String>,
    },

    /// Convert binary data into hex data.
    #[command(visible_aliases = &["--from-bin", "from-binx", "fb"])]
    FromBin,

    /// Normalize the input to lowercase, 0x-prefixed hex.
    ///
    /// The input can be:
    /// - mixed case hex with or without 0x prefix
    /// - 0x prefixed hex, concatenated with a ':'
    /// - an absolute path to file
    /// - @tag, where the tag is defined in an environment variable
    #[command(visible_aliases = &["--to-hexdata", "thd", "2hd"])]
    ToHexdata {
        /// The input to normalize.
        input: Option<String>,
    },

    /// Convert an address to a checksummed format (EIP-55).
    #[command(
        visible_aliases = &["--to-checksum-address",
        "--to-checksum",
        "to-checksum",
        "ta",
        "2a"]
    )]
    ToCheckSumAddress {
        /// The address to convert.
        address: Option<Address>,
        /// EIP-155 chain ID to encode the address using EIP-1191.
        chain_id: Option<u64>,
    },

    /// Convert hex data to an ASCII string.
    #[command(visible_aliases = &["--to-ascii", "tas", "2as"])]
    ToAscii {
        /// The hex data to convert.
        hexdata: Option<String>,
    },

    /// Convert hex data to a utf-8 string.
    #[command(visible_aliases = &["--to-utf8", "tu8", "2u8"])]
    ToUtf8 {
        /// The hex data to convert.
        hexdata: Option<String>,
    },

    /// Convert a fixed point number into an integer.
    #[command(visible_aliases = &["--from-fix", "ff"])]
    FromFixedPoint {
        /// The number of decimals to use.
        decimals: Option<String>,

        /// The value to convert.
        #[arg(allow_hyphen_values = true)]
        value: Option<String>,
    },

    /// Right-pads hex data to 32 bytes.
    #[command(visible_aliases = &["--to-bytes32", "tb", "2b"])]
    ToBytes32 {
        /// The hex data to convert.
        bytes: Option<String>,
    },

    /// Pads hex data to a specified length.
    #[command(visible_aliases = &["pd"])]
    Pad {
        /// The hex data to pad.
        data: Option<String>,

        /// Right-pad the data (instead of left-pad).
        #[arg(long)]
        right: bool,

        /// Left-pad the data (default).
        #[arg(long, conflicts_with = "right")]
        left: bool,

        /// Target length in bytes (default: 32).
        #[arg(long, default_value = "32")]
        len: usize,
    },

    /// Convert an integer into a fixed point number.
    #[command(visible_aliases = &["--to-fix", "tf", "2f"])]
    ToFixedPoint {
        /// The number of decimals to use.
        decimals: Option<String>,

        /// The value to convert.
        #[arg(allow_hyphen_values = true)]
        value: Option<String>,
    },

    /// Convert a number to a hex-encoded uint256.
    #[command(name = "to-uint256", visible_aliases = &["--to-uint256", "tu", "2u"])]
    ToUint256 {
        /// The value to convert.
        value: Option<String>,
    },

    /// Convert a number to a hex-encoded int256.
    #[command(name = "to-int256", visible_aliases = &["--to-int256", "ti", "2i"])]
    ToInt256 {
        /// The value to convert.
        value: Option<String>,
    },

    /// Perform a left shifting operation
    #[command(name = "shl")]
    LeftShift {
        /// The value to shift.
        value: String,

        /// The number of bits to shift.
        bits: String,

        /// The input base.
        #[arg(long)]
        base_in: Option<String>,

        /// The output base.
        #[arg(long, default_value = "16")]
        base_out: String,
    },

    /// Perform a right shifting operation
    #[command(name = "shr")]
    RightShift {
        /// The value to shift.
        value: String,

        /// The number of bits to shift.
        bits: String,

        /// The input base,
        #[arg(long)]
        base_in: Option<String>,

        /// The output base,
        #[arg(long, default_value = "16")]
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
    #[command(visible_aliases = &["--to-unit", "tun", "2un"])]
    ToUnit {
        /// The value to convert.
        value: Option<String>,

        /// The unit to convert to (ether, gwei, wei).
        #[arg(default_value = "wei")]
        unit: String,
    },

    /// Convert a number from decimal to smallest unit with arbitrary decimals.
    ///
    /// Examples:
    /// - 1.0 6    (for USDC, result: 1000000)
    /// - 2.5 12   (for 12 decimals token, result: 2500000000000)
    /// - 1.23 3   (for 3 decimals token, result: 1230)
    #[command(visible_aliases = &["--parse-units", "pun"])]
    ParseUnits {
        /// The value to convert.
        value: Option<String>,

        /// The unit to convert to.
        #[arg(default_value = "18")]
        unit: u8,
    },

    /// Format a number from smallest unit to decimal with arbitrary decimals.
    ///
    /// Examples:
    /// - 1000000 6       (for USDC, result: 1.0)
    /// - 2500000000000 12 (for 12 decimals, result: 2.5)
    /// - 1230 3          (for 3 decimals, result: 1.23)
    #[command(visible_aliases = &["--format-units", "fun"])]
    FormatUnits {
        /// The value to format.
        value: Option<String>,

        /// The unit to format to.
        #[arg(default_value = "18")]
        unit: u8,
    },

    /// Convert an ETH amount to wei.
    ///
    /// Consider using --to-unit.
    #[command(visible_aliases = &["--to-wei", "tw", "2w"])]
    ToWei {
        /// The value to convert.
        #[arg(allow_hyphen_values = true)]
        value: Option<String>,

        /// The unit to convert from (ether, gwei, wei).
        #[arg(default_value = "eth")]
        unit: String,
    },

    /// Convert wei into an ETH amount.
    ///
    /// Consider using --to-unit.
    #[command(visible_aliases = &["--from-wei", "fw"])]
    FromWei {
        /// The value to convert.
        #[arg(allow_hyphen_values = true)]
        value: Option<String>,

        /// The unit to convert from (ether, gwei, wei).
        #[arg(default_value = "eth")]
        unit: String,
    },

    /// RLP encodes hex data, or an array of hex data.
    ///
    /// Accepts a hex-encoded string, or an array of hex-encoded strings.
    /// Can be arbitrarily recursive.
    ///
    /// Examples:
    /// - `cast to-rlp "[]"` -> `0xc0`
    /// - `cast to-rlp "0x22"` -> `0x22`
    /// - `cast to-rlp "[\"0x61\"]"` -> `0xc161`
    /// - `cast to-rlp "[\"0xf1\", \"f2\"]"` -> `0xc481f181f2`
    #[command(visible_aliases = &["--to-rlp"])]
    ToRlp {
        /// The value to convert.
        ///
        /// This is a hex-encoded string, or an array of hex-encoded strings.
        /// Can be arbitrarily recursive.
        value: Option<String>,
    },

    /// Decodes RLP hex-encoded data.
    #[command(visible_aliases = &["--from-rlp"])]
    FromRlp {
        /// The RLP hex-encoded data.
        value: Option<String>,

        /// Decode the RLP data as int
        #[arg(long, alias = "int")]
        as_int: bool,
    },

    /// Converts a number of one base to another
    #[command(visible_aliases = &["--to-hex", "th", "2h"])]
    ToHex(ToBaseArgs),

    /// Converts a number of one base to decimal
    #[command(visible_aliases = &["--to-dec", "td", "2d"])]
    ToDec(ToBaseArgs),

    /// Converts a number of one base to another
    #[command(
        visible_aliases = &["--to-base",
        "--to-radix",
        "to-radix",
        "tr",
        "2r"]
    )]
    ToBase {
        #[command(flatten)]
        base: ToBaseArgs,

        /// The output base.
        #[arg(value_name = "BASE")]
        base_out: Option<String>,
    },
    /// Create an access list for a transaction.
    #[command(visible_aliases = &["ac", "acl"])]
    AccessList(AccessListArgs),
    /// Get logs by signature or topic.
    #[command(visible_alias = "l")]
    Logs(LogsArgs),
    /// Get information about a block.
    #[command(visible_alias = "bl")]
    Block {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        block: Option<BlockId>,

        /// If specified, only get the given field of the block.
        #[arg(long, short)]
        field: Option<String>,

        /// Print the raw RLP encoded block header.
        #[arg(long, conflicts_with = "field")]
        raw: bool,

        #[arg(long, env = "CAST_FULL_BLOCK")]
        full: bool,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Get the latest block number.
    #[command(visible_alias = "bn")]
    BlockNumber {
        /// The hash or tag to query. If not specified, the latest number is returned.
        block: Option<BlockId>,
        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Perform a call on an account without publishing a transaction.
    #[command(visible_alias = "c")]
    Call(CallArgs),

    /// ABI-encode a function with arguments.
    #[command(name = "calldata", visible_alias = "cd")]
    CalldataEncode {
        /// The function signature in the format `<name>(<in-types>)(<out-types>)`
        sig: String,

        /// The arguments to encode.
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,

        // Path to file containing arguments to encode.
        #[arg(long, value_name = "PATH")]
        file: Option<PathBuf>,
    },

    /// Get the symbolic name of the current chain.
    Chain {
        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Get the Ethereum chain ID.
    #[command(visible_aliases = &["ci", "cid"])]
    ChainId {
        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Get the current client version.
    #[command(visible_alias = "cl")]
    Client {
        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Compute the contract address from a given nonce and deployer address.
    #[command(visible_alias = "ca")]
    ComputeAddress {
        /// The deployer address.
        address: Option<Address>,

        /// The nonce of the deployer address.
        #[arg(
            long,
            conflicts_with = "salt",
            conflicts_with = "init_code",
            conflicts_with = "init_code_hash"
        )]
        nonce: Option<u64>,

        /// The salt for CREATE2 address computation.
        #[arg(long, conflicts_with = "nonce")]
        salt: Option<B256>,

        /// The init code for CREATE2 address computation.
        #[arg(
            long,
            requires = "salt",
            conflicts_with = "init_code_hash",
            conflicts_with = "nonce"
        )]
        init_code: Option<String>,

        /// The init code hash for CREATE2 address computation.
        #[arg(long, requires = "salt", conflicts_with = "init_code", conflicts_with = "nonce")]
        init_code_hash: Option<B256>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Disassembles a hex-encoded bytecode into a human-readable representation.
    #[command(visible_alias = "da")]
    Disassemble {
        /// The hex-encoded bytecode.
        bytecode: Option<String>,
    },

    /// Build and sign a transaction.
    #[command(name = "mktx", visible_alias = "m")]
    MakeTx(MakeTxArgs),

    /// Calculate the ENS namehash of a name.
    #[command(visible_aliases = &["na", "nh"])]
    Namehash { name: Option<String> },

    /// Get information about a transaction.
    #[command(visible_alias = "t")]
    Tx {
        /// The transaction hash.
        tx_hash: Option<String>,

        /// The sender of the transaction.
        #[arg(long, value_parser = NameOrAddress::from_str)]
        from: Option<NameOrAddress>,

        /// Nonce of the transaction.
        #[arg(long)]
        nonce: Option<u64>,

        /// If specified, only get the given field of the transaction. If "raw", the RLP encoded
        /// transaction will be printed.
        field: Option<String>,

        /// Print the raw RLP encoded transaction.
        #[arg(long, conflicts_with = "field")]
        raw: bool,

        #[command(flatten)]
        rpc: RpcOpts,

        /// If specified, the transaction will be converted to a TransactionRequest JSON format.
        #[arg(long)]
        to_request: bool,
    },

    /// Get the transaction receipt for a transaction.
    #[command(visible_alias = "re")]
    Receipt {
        /// The transaction hash.
        tx_hash: String,

        /// If specified, only get the given field of the transaction.
        field: Option<String>,

        /// The number of confirmations until the receipt is fetched
        #[arg(long, default_value = "1")]
        confirmations: u64,

        /// Exit immediately if the transaction was not found.
        #[arg(id = "async", long = "async", env = "CAST_ASYNC", alias = "cast-async")]
        cast_async: bool,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Sign and publish a transaction.
    #[command(name = "send", visible_alias = "s")]
    SendTx(SendTxArgs),

    /// Publish a raw transaction to the network.
    #[command(name = "publish", visible_alias = "p")]
    PublishTx {
        /// The raw transaction
        raw_tx: String,

        /// Only print the transaction hash and exit immediately.
        #[arg(id = "async", long = "async", env = "CAST_ASYNC", alias = "cast-async")]
        cast_async: bool,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Estimate the gas cost of a transaction.
    #[command(visible_alias = "e")]
    Estimate(EstimateArgs),

    /// Decode ABI-encoded input data.
    ///
    /// Similar to `abi-decode --input`, but function selector MUST be prefixed in `calldata`
    /// string
    #[command(visible_aliases = &["calldata-decode", "--calldata-decode", "cdd"])]
    DecodeCalldata {
        /// The function signature in the format `<name>(<in-types>)(<out-types>)`.
        sig: String,

        /// The ABI-encoded calldata.
        #[arg(required_unless_present = "file", index = 2)]
        calldata: Option<String>,

        /// Load ABI-encoded calldata from a file instead.
        #[arg(long = "file", short = 'f', conflicts_with = "calldata")]
        file: Option<PathBuf>,
    },

    /// Decode ABI-encoded string.
    ///
    /// Similar to `calldata-decode --input`, but the function argument is a `string`
    #[command(visible_aliases = &["string-decode", "--string-decode", "sd"])]
    DecodeString {
        /// The ABI-encoded string.
        data: String,
    },

    /// Decode event data.
    #[command(visible_aliases = &["event-decode", "--event-decode", "ed"])]
    DecodeEvent {
        /// The event signature. If none provided then tries to decode from local cache or <https://api.openchain.xyz>.
        #[arg(long, visible_alias = "event-sig")]
        sig: Option<String>,
        /// The event data to decode.
        data: String,
    },

    /// Decode custom error data.
    #[command(visible_aliases = &["error-decode", "--error-decode", "erd"])]
    DecodeError {
        /// The error signature. If none provided then tries to decode from local cache or <https://api.openchain.xyz>.
        #[arg(long, visible_alias = "error-sig")]
        sig: Option<String>,
        /// The error data to decode.
        data: String,
    },

    /// Decode ABI-encoded input or output data.
    ///
    /// Defaults to decoding output data. To decode input data pass --input.
    ///
    /// When passing `--input`, function selector must NOT be prefixed in `calldata` string
    #[command(name = "decode-abi", visible_aliases = &["abi-decode", "--abi-decode", "ad"])]
    DecodeAbi {
        /// The function signature in the format `<name>(<in-types>)(<out-types>)`.
        sig: String,

        /// The ABI-encoded calldata.
        calldata: String,

        /// Whether to decode the input or output data.
        #[arg(long, short, help_heading = "Decode input data instead of output data")]
        input: bool,
    },

    /// ABI encode the given function argument, excluding the selector.
    #[command(visible_alias = "ae")]
    AbiEncode {
        /// The function signature.
        sig: String,

        /// Whether to use packed encoding.
        #[arg(long)]
        packed: bool,

        /// The arguments of the function.
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Compute the storage slot for an entry in a mapping.
    #[command(visible_alias = "in")]
    Index {
        /// The mapping key type.
        key_type: String,

        /// The mapping key.
        key: String,

        /// The storage slot of the mapping.
        slot_number: String,
    },

    /// Compute storage slots as specified by `ERC-7201: Namespaced Storage Layout`.
    #[command(name = "index-erc7201", alias = "index-erc-7201", visible_aliases = &["index7201", "in7201"])]
    IndexErc7201 {
        /// The arbitrary identifier.
        id: Option<String>,
        /// The formula ID. Currently the only supported formula is `erc7201`.
        #[arg(long, default_value = "erc7201")]
        formula_id: String,
    },

    /// Fetch the EIP-1967 implementation for a contract
    /// Can read from the implementation slot or the beacon slot.
    #[command(visible_alias = "impl")]
    Implementation {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        /// Fetch the implementation from the beacon slot.
        ///
        /// If not specified, the implementation slot is used.
        #[arg(long)]
        beacon: bool,

        /// The address for which the implementation will be fetched.
        #[arg(value_parser = NameOrAddress::from_str)]
        who: NameOrAddress,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Fetch the EIP-1967 admin account
    #[command(visible_alias = "adm")]
    Admin {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        /// The address from which the admin account will be fetched.
        #[arg(value_parser = NameOrAddress::from_str)]
        who: NameOrAddress,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Get the function signatures for the given selector from <https://openchain.xyz>.
    #[command(name = "4byte", visible_aliases = &["4", "4b"])]
    FourByte {
        /// The function selector.
        selector: Option<Selector>,
    },

    /// Decode ABI-encoded calldata using <https://openchain.xyz>.
    #[command(name = "4byte-calldata", aliases = &["4byte-decode", "4d", "4bd"], visible_aliases = &["4c", "4bc"])]
    FourByteCalldata {
        /// The ABI-encoded calldata.
        calldata: Option<String>,
    },

    /// Get the event signature for a given topic 0 from <https://openchain.xyz>.
    #[command(name = "4byte-event", visible_aliases = &["4e", "4be", "topic0-event", "t0e"])]
    FourByteEvent {
        /// Topic 0
        #[arg(value_name = "TOPIC_0")]
        topic: Option<B256>,
    },

    /// Upload the given signatures to <https://openchain.xyz>.
    ///
    /// Example inputs:
    /// - "transfer(address,uint256)"
    /// - "function transfer(address,uint256)"
    /// - "function transfer(address,uint256)" "event Transfer(address,address,uint256)"
    /// - "./out/Contract.sol/Contract.json"
    #[command(visible_aliases = &["ups"])]
    UploadSignature {
        /// The signatures to upload.
        ///
        /// Prefix with 'function', 'event', or 'error'. Defaults to function if no prefix given.
        /// Can also take paths to contract artifact JSON.
        signatures: Vec<String>,
    },

    /// Pretty print calldata.
    ///
    /// Tries to decode the calldata using <https://openchain.xyz> unless --offline is passed.
    #[command(visible_alias = "pc")]
    PrettyCalldata {
        /// The calldata.
        calldata: Option<String>,

        /// Skip the <https://openchain.xyz> lookup.
        #[arg(long, short)]
        offline: bool,
    },

    /// Get the timestamp of a block.
    #[command(visible_alias = "a")]
    Age {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Get the balance of an account in wei.
    #[command(visible_alias = "b")]
    Balance {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        /// The account to query.
        #[arg(value_parser = NameOrAddress::from_str)]
        who: NameOrAddress,

        /// Format the balance in ether.
        #[arg(long, short)]
        ether: bool,

        #[command(flatten)]
        rpc: RpcOpts,

        /// erc20 address to query, with the method `balanceOf(address) return (uint256)`, alias
        /// with '--erc721'
        #[arg(long, alias = "erc721")]
        erc20: Option<Address>,
    },

    /// Get the basefee of a block.
    #[command(visible_aliases = &["ba", "fee", "basefee"])]
    BaseFee {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Get the runtime bytecode of a contract.
    #[command(visible_alias = "co")]
    Code {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        /// The contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        who: NameOrAddress,

        /// Disassemble bytecodes.
        #[arg(long, short)]
        disassemble: bool,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Get the runtime bytecode size of a contract.
    #[command(visible_alias = "cs")]
    Codesize {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        /// The contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        who: NameOrAddress,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Get the current gas price.
    #[command(visible_alias = "g")]
    GasPrice {
        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Generate event signatures from event string.
    #[command(visible_alias = "se")]
    SigEvent {
        /// The event string.
        event_string: Option<String>,
    },

    /// Hash arbitrary data using Keccak-256.
    #[command(visible_aliases = &["k", "keccak256"])]
    Keccak {
        /// The data to hash.
        data: Option<String>,
    },

    /// Hash a message according to EIP-191.
    #[command(visible_aliases = &["--hash-message", "hm"])]
    HashMessage {
        /// The message to hash.
        message: Option<String>,
    },

    /// Perform an ENS lookup.
    #[command(visible_alias = "rn")]
    ResolveName {
        /// The name to lookup.
        who: Option<String>,

        /// Perform a reverse lookup to verify that the name is correct.
        #[arg(long)]
        verify: bool,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Perform an ENS reverse lookup.
    #[command(visible_alias = "la")]
    LookupAddress {
        /// The account to perform the lookup for.
        who: Option<Address>,

        /// Perform a normal lookup to verify that the address is correct.
        #[arg(long)]
        verify: bool,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Get the raw value of a contract's storage slot.
    #[command(visible_alias = "st")]
    Storage(StorageArgs),

    /// Generate a storage proof for a given storage slot.
    #[command(visible_alias = "pr")]
    Proof {
        /// The contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        address: NameOrAddress,

        /// The storage slot numbers (hex or decimal).
        #[arg(value_parser = parse_slot)]
        slots: Vec<B256>,

        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Get the nonce for an account.
    #[command(visible_alias = "n")]
    Nonce {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        /// The address to get the nonce for.
        #[arg(value_parser = NameOrAddress::from_str)]
        who: NameOrAddress,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Get the codehash for an account.
    #[command()]
    Codehash {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        /// The address to get the codehash for.
        #[arg(value_parser = NameOrAddress::from_str)]
        who: NameOrAddress,

        /// The storage slot numbers (hex or decimal).
        #[arg(value_parser = parse_slot)]
        slots: Vec<B256>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Get the storage root for an account.
    #[command(visible_alias = "sr")]
    StorageRoot {
        /// The block height to query at.
        ///
        /// Can also be the tags earliest, finalized, safe, latest, or pending.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        /// The address to get the storage root for.
        #[arg(value_parser = NameOrAddress::from_str)]
        who: NameOrAddress,

        /// The storage slot numbers (hex or decimal).
        #[arg(value_parser = parse_slot)]
        slots: Vec<B256>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Get the source code of a contract from a block explorer.
    #[command(visible_aliases = &["et", "src"])]
    Source {
        /// The contract's address.
        address: String,

        /// Whether to flatten the source code.
        #[arg(long, short)]
        flatten: bool,

        /// The output directory/file to expand source tree into.
        #[arg(short, value_hint = ValueHint::DirPath, alias = "path")]
        directory: Option<PathBuf>,

        #[command(flatten)]
        etherscan: EtherscanOpts,

        /// Alternative explorer API URL to use that adheres to the Etherscan API. If not provided,
        /// defaults to Etherscan.
        #[arg(long, env = "EXPLORER_API_URL")]
        explorer_api_url: Option<String>,

        /// Alternative explorer browser URL.
        #[arg(long, env = "EXPLORER_URL")]
        explorer_url: Option<String>,
    },

    /// Wallet management utilities.
    #[command(visible_alias = "w")]
    Wallet {
        #[command(subcommand)]
        command: WalletSubcommands,
    },

    /// Download a contract creation code from Etherscan and RPC.
    #[command(visible_alias = "cc")]
    CreationCode(CreationCodeArgs),

    /// Generate an artifact file, that can be used to deploy a contract locally.
    #[command(visible_alias = "ar")]
    Artifact(ArtifactArgs),

    /// Display constructor arguments used for the contract initialization.
    #[command(visible_alias = "cra")]
    ConstructorArgs(ConstructorArgsArgs),

    /// Generate a Solidity interface from a given ABI.
    ///
    /// Currently does not support ABI encoder v2.
    #[command(visible_alias = "i")]
    Interface(InterfaceArgs),

    /// Generate a rust binding from a given ABI.
    #[command(visible_alias = "bi")]
    Bind(BindArgs),

    /// Get the selector for a function.
    #[command(visible_alias = "si")]
    Sig {
        /// The function signature, e.g. transfer(address,uint256).
        sig: Option<String>,

        /// Optimize signature to contain provided amount of leading zeroes in selector.
        optimize: Option<usize>,
    },

    /// Generate a deterministic contract address using CREATE2.
    #[command(visible_alias = "c2")]
    Create2(Create2Args),

    /// Get the block number closest to the provided timestamp.
    #[command(visible_alias = "f")]
    FindBlock(FindBlockArgs),

    /// Generate shell completions script.
    #[command(visible_alias = "com")]
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Generate Fig autocompletion spec.
    #[command(visible_alias = "fig")]
    GenerateFigSpec,

    /// Runs a published transaction in a local environment and prints the trace.
    #[command(visible_alias = "r")]
    Run(RunArgs),

    /// Perform a raw JSON-RPC request.
    #[command(visible_alias = "rp")]
    Rpc(RpcArgs),

    /// Formats a string into bytes32 encoding.
    #[command(name = "format-bytes32-string", visible_aliases = &["--format-bytes32-string"])]
    FormatBytes32String {
        /// The string to format.
        string: Option<String>,
    },

    /// Parses a string from bytes32 encoding.
    #[command(name = "parse-bytes32-string", visible_aliases = &["--parse-bytes32-string"])]
    ParseBytes32String {
        /// The string to parse.
        bytes: Option<String>,
    },
    #[command(name = "parse-bytes32-address", visible_aliases = &["--parse-bytes32-address"])]
    #[command(about = "Parses a checksummed address from bytes32 encoding.")]
    ParseBytes32Address {
        #[arg(value_name = "BYTES")]
        bytes: Option<String>,
    },

    /// Decodes a raw signed EIP 2718 typed transaction
    #[command(visible_aliases = &["dt", "decode-tx"])]
    DecodeTransaction { tx: Option<String> },

    /// Recovery an EIP-7702 authority from a Authorization JSON string.
    #[command(visible_aliases = &["decode-auth"])]
    RecoverAuthority { auth: String },

    /// Extracts function selectors and arguments from bytecode
    #[command(visible_alias = "sel")]
    Selectors {
        /// The hex-encoded bytecode.
        bytecode: Option<String>,

        /// Resolve the function signatures for the extracted selectors using <https://openchain.xyz>
        #[arg(long, short)]
        resolve: bool,
    },

    /// Inspect the TxPool of a node.
    #[command(visible_alias = "tp")]
    TxPool {
        #[command(subcommand)]
        command: TxPoolSubcommands,
    },
    /// Estimates the data availability size of a given opstack block.
    #[command(name = "da-estimate")]
    DAEstimate(DAEstimateArgs),
}

/// CLI arguments for `cast --to-base`.
#[derive(Debug, Parser)]
pub struct ToBaseArgs {
    /// The value to convert.
    #[arg(allow_hyphen_values = true)]
    pub value: Option<String>,

    /// The input base.
    #[arg(long, short = 'i')]
    pub base_in: Option<String>,
}

pub fn parse_slot(s: &str) -> Result<B256> {
    let slot = U256::from_str(s).map_err(|e| eyre::eyre!("Could not parse slot number: {e}"))?;
    Ok(B256::from(slot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SimpleCast;
    use alloy_rpc_types::{BlockNumberOrTag, RpcBlockHash};
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

                let selector = SimpleCast::get_selector(&sig, 0).unwrap();
                assert_eq!(selector.0, "0x23b872dd".to_string());
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
                expect: BlockId::Number(BlockNumberOrTag::Number(0u64)),
            },
            TestCase {
                input: "0x56462c47c03df160f66819f0a79ea07def1569f8aac0fe91bb3a081159b61b4a"
                    .to_string(),
                expect: BlockId::Hash(RpcBlockHash::from_hash(
                    "0x56462c47c03df160f66819f0a79ea07def1569f8aac0fe91bb3a081159b61b4a"
                        .parse()
                        .unwrap(),
                    None,
                )),
            },
            TestCase {
                input: "latest".to_string(),
                expect: BlockId::Number(BlockNumberOrTag::Latest),
            },
            TestCase {
                input: "earliest".to_string(),
                expect: BlockId::Number(BlockNumberOrTag::Earliest),
            },
            TestCase {
                input: "pending".to_string(),
                expect: BlockId::Number(BlockNumberOrTag::Pending),
            },
            TestCase { input: "safe".to_string(), expect: BlockId::Number(BlockNumberOrTag::Safe) },
            TestCase {
                input: "finalized".to_string(),
                expect: BlockId::Number(BlockNumberOrTag::Finalized),
            },
        ];

        for test in test_cases {
            let result: BlockId = test.input.parse().unwrap();
            assert_eq!(result, test.expect);
        }
    }
}
