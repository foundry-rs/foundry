use super::{ClapChain, EthereumOpts};
use crate::{
    cmd::cast::{
        bind::BindArgs, call::CallArgs, create2::Create2Args, estimate::EstimateArgs,
        find_block::FindBlockArgs, interface::InterfaceArgs, rpc::RpcArgs, run::RunArgs,
        send::SendTxArgs, storage::StorageArgs, wallet::WalletSubcommands,
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

#[derive(Debug, Subcommand)]
#[clap(
    about = "Perform Ethereum RPC calls from the comfort of your command line.",
    after_help = "Find more information in the book: http://book.getfoundry.sh/reference/cast/cast.html",
    next_display_order = None
)]
pub enum Subcommands {
    #[clap(name = "--max-int")]
    #[clap(visible_aliases = &["max-int", "maxi"])]
    #[clap(about = "Get the maximum value of the given integer type.")]
    MaxInt {
        #[clap(default_value = "int256")]
        r#type: String,
    },
    #[clap(name = "--min-int")]
    #[clap(visible_aliases = &["min-int", "mini"])]
    #[clap(about = "Get the minimum value of the given integer type.")]
    MinInt {
        #[clap(default_value = "int256")]
        r#type: String,
    },
    #[clap(name = "--max-uint")]
    #[clap(visible_aliases = &["max-uint", "maxu"])]
    #[clap(about = "Get the maximum value of the given integer type.")]
    MaxUint {
        #[clap(default_value = "uint256")]
        r#type: String,
    },
    #[clap(name = "--address-zero", about = "Get zero address")]
    #[clap(visible_aliases = &["address-zero", "az"])]
    AddressZero,
    #[clap(name = "--hash-zero", about = "Get zero hash")]
    #[clap(visible_aliases = &["hash-zero", "hz"])]
    HashZero,

    #[clap(name = "--from-utf8")]
    #[clap(visible_aliases = &["from-utf8", "--from-ascii", "from-ascii", "fu", "fa"])]
    #[clap(about = "Convert UTF8 text to hex.")]
    FromUtf8 {
        #[clap(value_name = "TEXT")]
        text: Option<String>,
    },
    #[clap(name = "--concat-hex")]
    #[clap(visible_aliases = &["concat-hex", "ch"])]
    #[clap(about = "Concatenate hex strings.")]
    ConcatHex {
        #[clap(value_name = "DATA")]
        data: Vec<String>,
    },
    #[clap(name = "--from-bin")]
    #[clap(visible_aliases = &["from-bin", "fb"])]
    #[clap(about = "Convert binary data into hex data.")]
    FromBin,
    #[clap(name = "--to-hexdata")]
    #[clap(visible_aliases = &["to-hexdata", "thd", "2hd"])]
    #[clap(
        about = "Normalize the input to lowercase, 0x-prefixed hex. See --help for more info.",
        long_about = r#"Normalize the input to lowercase, 0x-prefixed hex.

The input can be:
- mixed case hex with or without 0x prefix
- 0x prefixed hex, concatenated with a ':'
- an absolute path to file
- @tag, where the tag is defined in an environment variable"#
    )]
    ToHexdata {
        #[clap(value_name = "INPUT")]
        input: Option<String>,
    },
    #[clap(name = "--to-checksum-address")]
    #[clap(visible_aliases = &["to-checksum-address", "--to-checksum", "to-checksum", "ta", "2a"])] // Compatibility with dapptools' cast
    #[clap(about = "Convert an address to a checksummed format (EIP-55).")]
    ToCheckSumAddress {
        #[clap(value_name = "ADDRESS")]
        address: Option<Address>,
    },
    #[clap(name = "--to-ascii")]
    #[clap(visible_aliases = &["to-ascii", "tas", "2as"])]
    #[clap(about = "Convert hex data to an ASCII string.")]
    ToAscii {
        #[clap(value_name = "HEXDATA")]
        hexdata: Option<String>,
    },
    #[clap(name = "--from-fix")]
    #[clap(visible_aliases = &["from-fix", "ff"])]
    #[clap(about = "Convert a fixed point number into an integer.")]
    FromFixedPoint {
        #[clap(value_name = "DECIMALS")]
        decimals: Option<String>,
        #[clap(allow_hyphen_values = true, value_name = "VALUE")]
        // negative values not yet supported internally
        value: Option<String>,
    },
    #[clap(name = "--to-bytes32")]
    #[clap(visible_aliases = &["to-bytes32", "tb", "2b"])]
    #[clap(about = "Right-pads hex data to 32 bytes.")]
    ToBytes32 {
        #[clap(value_name = "BYTES")]
        bytes: Option<String>,
    },
    #[clap(name = "--to-fix")]
    #[clap(visible_aliases = &["to-fix", "tf", "2f"])]
    #[clap(about = "Convert an integer into a fixed point number.")]
    ToFixedPoint {
        #[clap(value_name = "DECIMALS")]
        decimals: Option<String>,
        #[clap(allow_hyphen_values = true, value_name = "VALUE")]
        value: Option<String>,
    },
    #[clap(name = "--to-uint256")]
    #[clap(visible_aliases = &["to-uint256", "tu", "2u"])]
    #[clap(about = "Convert a number to a hex-encoded uint256.")]
    ToUint256 {
        #[clap(value_name = "VALUE")]
        value: Option<String>,
    },
    #[clap(name = "--to-int256")]
    #[clap(visible_aliases = &["to-int256", "ti", "2i"])]
    #[clap(about = "Convert a number to a hex-encoded int256.")]
    ToInt256 {
        #[clap(value_name = "VALUE")]
        value: Option<String>,
    },
    #[clap(name = "shl")]
    #[clap(about = "Perform a left shifting operation")]
    LeftShift {
        #[clap(value_name = "VALUE")]
        value: String,
        #[clap(value_name = "BITS")]
        bits: String,
        #[clap(long = "base-in", help = "The input base")]
        base_in: Option<String>,
        #[clap(long = "base-out", help = "The output base", default_value = "16")]
        base_out: String,
    },
    #[clap(name = "shr")]
    #[clap(about = "Perform a right shifting operation")]
    RightShift {
        #[clap(value_name = "VALUE")]
        value: String,
        #[clap(value_name = "BITS")]
        bits: String,
        #[clap(long = "base-in", help = "The input base")]
        base_in: Option<String>,
        #[clap(long = "base-out", help = "The output base", default_value = "16")]
        base_out: String,
    },
    #[clap(name = "--to-unit")]
    #[clap(visible_aliases = &["to-unit", "tun", "2un"])]
    #[clap(
        about = "Convert an ETH amount into another unit (ether, gwei or wei).",
        long_about = r#"Convert an ETH amount into another unit (ether, gwei or wei).\

Examples:
- 1ether wei
- "1 ether" wei
- 1ether
- 1 gwei
- 1gwei ether"#
    )]
    ToUnit {
        #[clap(value_name = "VALUE")]
        // negative values not yet supported internally
        value: Option<String>,
        #[clap(
            help = "The unit to convert to (ether, gwei, wei).",
            default_value = "wei",
            value_name = "UNIT"
        )]
        unit: String,
    },
    #[clap(name = "--to-wei")]
    #[clap(visible_aliases = &["to-wei", "tw", "2w"])]
    #[clap(about = "Convert an ETH amount to wei. Consider using --to-unit.")]
    ToWei {
        #[clap(allow_hyphen_values = true, value_name = "VALUE")]
        // negative values not yet supported internally
        value: Option<String>,
        #[clap(value_name = "UNIT", default_value = "eth")]
        unit: String,
    },
    #[clap(name = "--from-wei")]
    #[clap(visible_aliases = &["from-wei", "fw"])]
    #[clap(about = "Convert wei into an ETH amount. Consider using --to-unit.")]
    FromWei {
        #[clap(allow_hyphen_values = true, value_name = "VALUE")]
        // negative values not yet supported internally
        value: Option<String>,
        #[clap(value_name = "UNIT", default_value = "eth")]
        unit: String,
    },
    #[clap(name = "--to-rlp")]
    #[clap(about = "RLP encodes hex data, or an array of hex data")]
    ToRlp { value: Option<String> },
    #[clap(name = "--from-rlp")]
    #[clap(about = "Decodes RLP encoded data. Input must be hexadecimal.")]
    FromRlp { value: Option<String> },
    #[clap(name = "--to-hex")]
    #[clap(visible_aliases = &["to-hex", "th", "2h"])]
    #[clap(about = "Converts a number of one base to another")]
    ToHex(ToBaseArgs),
    #[clap(name = "--to-dec")]
    #[clap(visible_aliases = &["to-dec", "td", "2d"])]
    #[clap(about = "Converts a number of one base to decimal")]
    ToDec(ToBaseArgs),
    #[clap(name = "--to-base")]
    #[clap(visible_aliases = &["to-base", "--to-radix", "to-radix", "tr", "2r"])]
    #[clap(about = "Converts a number of one base to another")]
    ToBase {
        #[clap(flatten)]
        base: ToBaseArgs,
        #[clap(value_name = "BASE", help = "The output base")]
        base_out: Option<String>,
    },
    #[clap(name = "access-list")]
    #[clap(visible_aliases = &["ac", "acl"])]
    #[clap(about = "Create an access list for a transaction.")]
    AccessList {
        #[clap(
            help = "The destination of the transaction.",
            value_parser = NameOrAddress::from_str,
            value_name = "ADDRESS"
        )]
        address: NameOrAddress,
        #[clap(help = "The signature of the function to call.", value_name = "SIG")]
        sig: String,
        #[clap(help = "The arguments of the function to call.", value_name = "ARGS")]
        args: Vec<String>,
        #[clap(
            long,
            short = 'B',
            help = "The block height you want to query at.",
            long_help = "The block height you want to query at. Can also be the tags earliest, latest, or pending.",
            value_name = "BLOCK"
        )]
        block: Option<BlockId>,
        #[clap(flatten)]
        // TODO: We only need RPC URL + etherscan stuff from this struct
        eth: EthereumOpts,
        #[clap(long = "json", short = 'j', help_heading = "Display options")]
        to_json: bool,
    },
    #[clap(name = "block")]
    #[clap(visible_alias = "bl")]
    #[clap(about = "Get information about a block.")]
    Block {
        #[clap(
            help = "The block height you want to query at.",
            long_help = "The block height you want to query at. Can also be the tags earliest, latest, or pending.",
            value_name = "BLOCK"
        )]
        block: BlockId,
        #[clap(
            help = "If specified, only get the given field of the block.",
            value_name = "FIELD"
        )]
        field: Option<String>,
        #[clap(long, env = "CAST_FULL_BLOCK")]
        full: bool,
        #[clap(long = "json", short = 'j', help_heading = "Display options")]
        to_json: bool,
        #[clap(long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
    },
    #[clap(name = "block-number")]
    #[clap(visible_alias = "bn")]
    #[clap(about = "Get the latest block number.")]
    BlockNumber {
        #[clap(long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
    },
    #[clap(name = "call")]
    #[clap(visible_alias = "c")]
    #[clap(about = "Perform a call on an account without publishing a transaction.")]
    Call(CallArgs),
    #[clap(name = "calldata")]
    #[clap(visible_alias = "cd")]
    #[clap(about = "ABI-encode a function with arguments.")]
    CalldataEncode {
        #[clap(
            help = "The function signature.",
            long_help = "The function signature in the form <name>(<types...>)",
            value_name = "SIG"
        )]
        sig: String,
        #[clap(allow_hyphen_values = true, value_name = "ARGS")]
        args: Vec<String>,
    },
    #[clap(name = "chain")]
    #[clap(about = "Get the symbolic name of the current chain.")]
    Chain {
        #[clap(long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
    },
    #[clap(name = "chain-id")]
    #[clap(visible_aliases = &["ci", "cid"])]
    #[clap(about = "Get the Ethereum chain ID.")]
    ChainId {
        #[clap(long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
    },
    #[clap(name = "client")]
    #[clap(visible_alias = "cl")]
    #[clap(about = "Get the current client version.")]
    Client {
        #[clap(long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
    },
    #[clap(name = "compute-address")]
    #[clap(visible_alias = "ca")]
    #[clap(about = "Compute the contract address from a given nonce and deployer address.")]
    ComputeAddress {
        #[clap(help = "The deployer address.", value_name = "ADDRESS")]
        address: Option<String>,
        #[clap(long, help = "The nonce of the deployer address.", value_parser = parse_u256, value_name = "NONCE")]
        nonce: Option<U256>,
        #[clap(long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
    },
    #[clap(name = "namehash")]
    #[clap(visible_aliases = &["na", "nh"])]
    #[clap(about = "Calculate the ENS namehash of a name.")]
    Namehash {
        #[clap(value_name = "NAME")]
        name: Option<String>,
    },
    #[clap(name = "tx")]
    #[clap(visible_alias = "t")]
    #[clap(about = "Get information about a transaction.")]
    Tx {
        #[clap(value_name = "TX_HASH")]
        tx_hash: String,
        #[clap(value_name = "FIELD")]
        field: Option<String>,
        #[clap(long = "json", short = 'j', help_heading = "Display options")]
        to_json: bool,
        #[clap(long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
    },
    #[clap(name = "receipt")]
    #[clap(visible_alias = "re")]
    #[clap(about = "Get the transaction receipt for a transaction.")]
    Receipt {
        #[clap(value_name = "TX_HASH")]
        tx_hash: String,
        #[clap(value_name = "FIELD")]
        field: Option<String>,
        #[clap(
            short,
            long,
            help = "The number of confirmations until the receipt is fetched",
            default_value = "1",
            value_name = "CONFIRMATIONS"
        )]
        confirmations: usize,
        #[clap(
            long = "async",
            env = "CAST_ASYNC",
            name = "async",
            alias = "cast-async",
            help = "Exit immediately if the transaction was not found."
        )]
        cast_async: bool,
        #[clap(long = "json", short = 'j', help_heading = "Display options")]
        to_json: bool,
        #[clap(long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
    },
    #[clap(name = "send")]
    #[clap(visible_alias = "s")]
    #[clap(about = "Sign and publish a transaction.")]
    SendTx(SendTxArgs),
    #[clap(name = "publish")]
    #[clap(visible_alias = "p")]
    #[clap(about = "Publish a raw transaction to the network.")]
    PublishTx {
        #[clap(help = "The raw transaction", value_name = "RAW_TX")]
        raw_tx: String,
        #[clap(
            long = "async",
            env = "CAST_ASYNC",
            name = "async",
            alias = "cast-async",
            help = "Only print the transaction hash and exit immediately."
        )]
        cast_async: bool,
        // FIXME: We only need the RPC URL and `--flashbots` options from this.
        #[clap(flatten)]
        eth: EthereumOpts,
    },
    #[clap(name = "estimate")]
    #[clap(visible_alias = "e")]
    #[clap(about = "Estimate the gas cost of a transaction.")]
    Estimate(EstimateArgs),
    #[clap(name = "--calldata-decode")]
    #[clap(visible_alias = "cdd")]
    #[clap(about = "Decode ABI-encoded input data.")]
    CalldataDecode {
        #[clap(
            help = "The function signature in the format `<name>(<in-types>)(<out-types>)`.",
            value_name = "SIG"
        )]
        sig: String,
        #[clap(help = "The ABI-encoded calldata.", value_name = "CALLDATA")]
        calldata: String,
    },
    #[clap(name = "--abi-decode")]
    #[clap(visible_alias = "ad")]
    #[clap(
        about = "Decode ABI-encoded input or output data",
        long_about = r#"Decode ABI-encoded input or output data.

Defaults to decoding output data. To decode input data pass --input or use cast --calldata-decode."#
    )]
    AbiDecode {
        #[clap(
            help = "The function signature in the format `<name>(<in-types>)(<out-types>)`.",
            value_name = "SIG"
        )]
        sig: String,
        #[clap(help = "The ABI-encoded calldata.", value_name = "CALLDATA")]
        calldata: String,
        #[clap(long, short, help = "Decode input data.")]
        input: bool,
    },
    #[clap(name = "abi-encode")]
    #[clap(visible_alias = "ae")]
    #[clap(about = "ABI encode the given function argument, excluding the selector.")]
    AbiEncode {
        #[clap(help = "The function signature.", value_name = "SIG")]
        sig: String,
        #[clap(help = "The arguments of the function.", value_name = "ARGS")]
        #[clap(allow_hyphen_values = true)]
        args: Vec<String>,
    },
    #[clap(name = "index")]
    #[clap(visible_alias = "in")]
    #[clap(about = "Compute the storage slot for an entry in a mapping.")]
    Index {
        #[clap(help = "The mapping key type.", value_name = "KEY_TYPE")]
        key_type: String,
        #[clap(help = "The mapping key.", value_name = "KEY")]
        key: String,
        #[clap(help = "The storage slot of the mapping.", value_name = "SLOT_NUMBER")]
        slot_number: String,
    },
    #[clap(name = "implementation")]
    #[clap(visible_alias = "impl")]
    #[clap(about = "Fetch the EIP-1967 implementation account")]
    Implementation {
        #[clap(
            long,
            short = 'B',
            help = "The block height you want to query at.",
            long_help = "The block height you want to query at. Can also be the tags earliest, latest, or pending.",
            value_name = "BLOCK"
        )]
        block: Option<BlockId>,
        #[clap(help = "The address you want to get the nonce for.", value_parser = NameOrAddress::from_str, value_name = "WHO")]
        who: NameOrAddress,
        #[clap(short, long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
    },
    #[clap(name = "admin")]
    #[clap(visible_alias = "adm")]
    #[clap(about = "Fetch the EIP-1967 admin account")]
    Admin {
        #[clap(
            long,
            short = 'B',
            help = "The block height you want to query at.",
            long_help = "The block height you want to query at. Can also be the tags earliest, latest, or pending.",
            value_name = "BLOCK"
        )]
        block: Option<BlockId>,
        #[clap(help = "The address you want to get the nonce for.", value_parser = NameOrAddress::from_str, value_name = "WHO")]
        who: NameOrAddress,
        #[clap(short, long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
    },
    #[clap(name = "4byte")]
    #[clap(visible_aliases = &["4", "4b"])]
    #[clap(
        about = "Get the function signatures for the given selector from https://sig.eth.samczsun.com."
    )]
    FourByte {
        #[clap(help = "The function selector.", value_name = "SELECTOR")]
        selector: Option<String>,
    },
    #[clap(name = "4byte-decode")]
    #[clap(visible_aliases = &["4d", "4bd"])]
    #[clap(about = "Decode ABI-encoded calldata using https://sig.eth.samczsun.com.")]
    FourByteDecode {
        #[clap(help = "The ABI-encoded calldata.", value_name = "CALLDATA")]
        calldata: Option<String>,
    },
    #[clap(name = "4byte-event")]
    #[clap(visible_aliases = &["4e", "4be"])]
    #[clap(
        about = "Get the event signature for a given topic 0 from https://sig.eth.samczsun.com."
    )]
    FourByteEvent {
        #[clap(help = "Topic 0", value_name = "TOPIC_0")]
        topic: Option<String>,
    },
    #[clap(name = "upload-signature")]
    #[clap(visible_aliases = &["ups"])]
    #[clap(
        about = "Upload the given signatures to https://sig.eth.samczsun.com.",
        long_about = r#"Upload the given signatures to https://sig.eth.samczsun.com.

Examples:
- cast upload-signature "transfer(address,uint256)"
- cast upload-signature "function transfer(address,uint256)"
- cast upload-signature "function transfer(address,uint256)" "event Transfer(address,address,uint256)"
- cast upload-signature ./out/Contract.sol/Contract.json""#
    )]
    UploadSignature {
        #[clap(
            help = "The signatures to upload. Prefix with 'function', 'event', or 'error'. Defaults to function if no prefix given. Can also take paths to contract artifact JSON."
        )]
        signatures: Vec<String>,
    },
    #[clap(name = "pretty-calldata")]
    #[clap(visible_alias = "pc")]
    #[clap(
        about = "Pretty print calldata.",
        long_about = r#"Pretty print calldata.

Tries to decode the calldata using https://sig.eth.samczsun.com unless --offline is passed."#
    )]
    PrettyCalldata {
        #[clap(help = "The calldata.", value_name = "CALLDATA")]
        calldata: Option<String>,
        #[clap(long, short, help = "Skip the https://sig.eth.samczsun.com lookup.")]
        offline: bool,
    },
    #[clap(name = "age")]
    #[clap(visible_alias = "a")]
    #[clap(about = "Get the timestamp of a block.")]
    Age {
        #[clap(
            long,
            short = 'B',
            help = "The block height you want to query at.",
            long_help = "The block height you want to query at. Can also be the tags earliest, latest, or pending.",
            value_name = "BLOCK"
        )]
        block: Option<BlockId>,
        #[clap(short, long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
    },
    #[clap(name = "balance")]
    #[clap(visible_alias = "b")]
    #[clap(about = "Get the balance of an account in wei.")]
    Balance {
        #[clap(
            long,
            short = 'B',
            help = "The block height you want to query at.",
            long_help = "The block height you want to query at. Can also be the tags earliest, latest, or pending.",
            value_name = "BLOCK"
        )]
        block: Option<BlockId>,
        #[clap(
            help = "The account you want to query",
            value_parser = NameOrAddress::from_str,
            value_name = "WHO"
        )]
        who: NameOrAddress,
        #[clap(short, long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
        #[clap(long = "ether", short = 'e', help_heading = "format to ether")]
        to_ether: bool,
    },
    #[clap(name = "basefee")]
    #[clap(visible_aliases = &["ba", "fee"])]
    #[clap(about = "Get the basefee of a block.")]
    BaseFee {
        #[clap(
            long,
            short = 'B',
            help = "The block height you want to query at.",
            long_help = "The block height you want to query at. Can also be the tags earliest, latest, or pending.",
            value_name = "BLOCK"
        )]
        block: Option<BlockId>,
        #[clap(short, long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
    },
    #[clap(name = "code")]
    #[clap(visible_alias = "co")]
    #[clap(about = "Get the bytecode of a contract.")]
    Code {
        #[clap(
            long,
            short = 'B',
            help = "The block height you want to query at.",
            long_help = "The block height you want to query at. Can also be the tags earliest, latest, or pending.",
            value_name = "BLOCK"
        )]
        block: Option<BlockId>,
        #[clap(help = "The contract address.", value_parser = NameOrAddress::from_str, value_name = "WHO")]
        who: NameOrAddress,
        #[clap(short, long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
    },
    #[clap(name = "gas-price")]
    #[clap(visible_alias = "g")]
    #[clap(about = "Get the current gas price.")]
    GasPrice {
        #[clap(short, long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
    },
    #[clap(name = "sig-event")]
    #[clap(visible_alias = "se")]
    #[clap(about = "Generate event signatures from event string.")]
    SigEvent {
        #[clap(value_name = "EVENT_STRING")]
        event_string: Option<String>,
    },
    #[clap(name = "keccak")]
    #[clap(visible_alias = "k")]
    #[clap(about = "Hash arbitrary data using Keccak-256.")]
    Keccak {
        #[clap(value_name = "DATA")]
        data: Option<String>,
    },
    #[clap(name = "resolve-name")]
    #[clap(visible_alias = "rn")]
    #[clap(about = "Perform an ENS lookup.")]
    ResolveName {
        #[clap(help = "The name to lookup.", value_name = "WHO")]
        who: Option<String>,
        #[clap(short, long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
        #[clap(long, short, help = "Perform a reverse lookup to verify that the name is correct.")]
        verify: bool,
    },
    #[clap(name = "lookup-address")]
    #[clap(visible_alias = "l")]
    #[clap(about = "Perform an ENS reverse lookup.")]
    LookupAddress {
        #[clap(help = "The account to perform the lookup for.", value_name = "WHO")]
        who: Option<Address>,
        #[clap(short, long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
        #[clap(
            long,
            short,
            help = "Perform a normal lookup to verify that the address is correct."
        )]
        verify: bool,
    },
    #[clap(
        name = "storage",
        visible_alias = "st",
        about = "Get the raw value of a contract's storage slot."
    )]
    Storage(StorageArgs),
    #[clap(
        name = "proof",
        visible_alias = "pr",
        about = "Generate a storage proof for a given storage slot."
    )]
    Proof {
        #[clap(help = "The contract address.", value_parser = NameOrAddress::from_str, value_name = "ADDRESS")]
        address: NameOrAddress,
        #[clap(help = "The storage slot numbers (hex or decimal).",  value_parser = parse_slot, value_name = "SLOTS")]
        slots: Vec<H256>,
        #[clap(short, long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
        #[clap(
            long,
            short = 'B',
            help = "The block height you want to query at.",
            long_help = "The block height you want to query at. Can also be the tags earliest, latest, or pending.",
            value_name = "BLOCK"
        )]
        block: Option<BlockId>,
    },
    #[clap(name = "nonce")]
    #[clap(visible_alias = "n")]
    #[clap(about = "Get the nonce for an account.")]
    Nonce {
        #[clap(
            long,
            short = 'B',
            help = "The block height you want to query at.",
            long_help = "The block height you want to query at. Can also be the tags earliest, latest, or pending.",
            value_name = "BLOCK"
        )]
        block: Option<BlockId>,
        #[clap(help = "The address you want to get the nonce for.", value_parser = NameOrAddress::from_str, value_name = "WHO")]
        who: NameOrAddress,
        #[clap(short, long, env = "ETH_RPC_URL", value_name = "URL")]
        rpc_url: Option<String>,
    },
    #[clap(name = "etherscan-source")]
    #[clap(visible_aliases = &["et", "src"])]
    #[clap(about = "Get the source code of a contract from Etherscan.")]
    EtherscanSource {
        #[clap(flatten)]
        chain: ClapChain,
        #[clap(help = "The contract's address.", value_name = "ADDRESS")]
        address: String,
        #[clap(short, help = "The output directory to expand source tree into.", value_hint = ValueHint::DirPath, value_name = "DIRECTORY")]
        directory: Option<PathBuf>,
        #[clap(long, short, env = "ETHERSCAN_API_KEY", value_name = "KEY")]
        etherscan_api_key: Option<String>,
    },
    #[clap(name = "wallet", visible_alias = "w", about = "Wallet management utilities.")]
    Wallet {
        #[clap(subcommand)]
        command: WalletSubcommands,
    },
    #[clap(
        name = "interface",
        visible_alias = "i",
        about = "Generate a Solidity interface from a given ABI.",
        long_about = "Generate a Solidity interface from a given ABI. Currently does not support ABI encoder v2."
    )]
    Interface(InterfaceArgs),
    #[clap(
        name = "bind",
        visible_alias = "bi",
        about = "Generate a rust binding from a given ABI.",
        long_about = "Generate a rust binding from a given ABI. Currently does not support ABI encoder v2."
    )]
    Bind(BindArgs),

    #[clap(name = "sig", visible_alias = "si", about = "Get the selector for a function.")]
    Sig {
        #[clap(
            help = "The function signature, e.g. transfer(address,uint256).",
            value_name = "SIG"
        )]
        sig: Option<String>,
    },
    #[clap(
        name = "create2",
        visible_alias = "c2",
        about = "Generate a deterministic contract address using CREATE2"
    )]
    Create2(Create2Args),
    #[clap(
        name = "find-block",
        visible_alias = "f",
        about = "Get the block number closest to the provided timestamp."
    )]
    FindBlock(FindBlockArgs),
    #[clap(visible_alias = "com", about = "Generate shell completions script")]
    Completions {
        #[clap(value_enum)]
        shell: clap_complete::Shell,
    },
    #[clap(visible_alias = "fig", about = "Generate Fig autocompletion spec.")]
    GenerateFigSpec,
    #[clap(
        name = "run",
        visible_alias = "r",
        about = "Runs a published transaction in a local environment and prints the trace."
    )]
    Run(RunArgs),
    #[clap(name = "rpc")]
    #[clap(visible_alias = "rp")]
    #[clap(about = "Perform a raw JSON-RPC request")]
    Rpc(RpcArgs),
    #[clap(name = "--format-bytes32-string")]
    #[clap(about = "Formats a string into bytes32 encoding.")]
    FormatBytes32String {
        #[clap(value_name = "STRING")]
        string: Option<String>,
    },
    #[clap(name = "--parse-bytes32-string")]
    #[clap(about = "Parses a string from bytes32 encoding.")]
    ParseBytes32String {
        #[clap(value_name = "BYTES")]
        bytes: Option<String>,
    },
}

/// CLI arguments for `cast --to-base`.
#[derive(Debug, Parser)]
pub struct ToBaseArgs {
    #[clap(allow_hyphen_values = true, value_name = "VALUE")]
    pub value: Option<String>,
    #[clap(long = "base-in", short = 'i', help = "The input base")]
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
            _ => {
                unreachable!()
            }
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
