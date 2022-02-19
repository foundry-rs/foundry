use std::{path::PathBuf, str::FromStr};

use clap::{Parser, Subcommand};
use ethers::{
    abi::token::{LenientTokenizer, Tokenizer},
    types::{Address, BlockId, BlockNumber, NameOrAddress, H256, U256},
};

use super::{ClapChain, EthereumOpts, Wallet};
use crate::utils::parse_u256;

#[derive(Debug, Subcommand)]
#[clap(about = "Perform Ethereum RPC calls from the comfort of your command line.")]
pub enum Subcommands {
    #[clap(name = "--max-int")]
    #[clap(about = "Maximum i256 value")]
    MaxInt,
    #[clap(name = "--min-int")]
    #[clap(about = "Minimum i256 value")]
    MinInt,
    #[clap(name = "--max-uint")]
    #[clap(about = "Maximum u256 value")]
    MaxUint,
    #[clap(aliases = &["--from-ascii"])]
    #[clap(name = "--from-utf8")]
    #[clap(about = "Convert text data into hexdata")]
    FromUtf8 { text: Option<String> },
    #[clap(name = "--to-hex")]
    #[clap(about = "Convert a decimal number into hex")]
    ToHex { decimal: Option<String> },
    #[clap(name = "--to-hexdata")]
    #[clap(about = r#"[<hex>|</path>|<@tag>]
    Output lowercase, 0x-prefixed hex, converting from the
    input, which can be:
      - mixed case hex with or without 0x prefix
      - 0x prefixed hex, concatenated with a ':'
      - absolute path to file
      - @tag, where $TAG is defined in environment variables
    "#)]
    ToHexdata { input: Option<String> },
    #[clap(aliases = &["--to-checksum"])] // Compatibility with dapptools' cast
    #[clap(name = "--to-checksum-address")]
    #[clap(about = "Convert an address to a checksummed format (EIP-55)")]
    ToCheckSumAddress { address: Option<Address> },
    #[clap(name = "--to-ascii")]
    #[clap(about = "Convert hex data to text data")]
    ToAscii { hexdata: Option<String> },
    #[clap(name = "--from-fix")]
    #[clap(about = "Convert fixed point into specified number of decimals")]
    FromFix {
        decimals: Option<u128>,
        #[clap(allow_hyphen_values = true)] // negative values not yet supported internally
        value: Option<String>,
    },
    #[clap(name = "--to-bytes32")]
    #[clap(about = "Left-pads a hex bytes string to 32 bytes")]
    ToBytes32 { bytes: Option<String> },
    #[clap(name = "--to-dec")]
    #[clap(about = "Convert hex value into decimal number")]
    ToDec { hexvalue: Option<String> },
    #[clap(name = "--to-fix")]
    #[clap(about = "Convert integers into fixed point with specified decimals")]
    ToFix {
        decimals: Option<u128>,
        #[clap(allow_hyphen_values = true)] // negative values not yet supported internally
        value: Option<String>,
    },
    #[clap(name = "--to-uint256")]
    #[clap(about = "Convert a number into uint256 hex string with 0x prefix")]
    ToUint256 { value: Option<String> },
    #[clap(name = "--to-int256")]
    #[clap(about = "Convert a number into int256 hex string with 0x prefix")]
    ToInt256 { value: Option<String> },
    #[clap(name = "--to-unit")]
    #[clap(
        about = r#"Convert an ETH amount into a specified unit: ether, gwei or wei (default: wei).
    Usage:
      - 1ether wei     | converts 1 ether to wei
      - "1 ether" wei  | converts 1 ether to wei
      - 1ether         | converts 1 ether to wei
      - 1 gwei         | converts 1 wei to gwei
      - 1gwei ether    | converts 1 gwei to ether
    "#
    )]
    ToUnit { value: Option<String>, unit: Option<String> },
    #[clap(name = "--to-wei")]
    #[clap(about = "Convert an ETH amount into wei. Consider using --to-unit.")]
    ToWei {
        #[clap(allow_hyphen_values = true)] // negative values not yet supported internally
        value: Option<String>,
        unit: Option<String>,
    },
    #[clap(name = "--from-wei")]
    #[clap(about = "Convert wei into an ETH amount. Consider using --to-unit.")]
    FromWei {
        #[clap(allow_hyphen_values = true)] // negative values not yet supported internally
        value: Option<String>,
        unit: Option<String>,
    },
    #[clap(name = "block")]
    #[clap(
        about = "Prints information about <block>. If <field> is given, print only the value of that field"
    )]
    Block {
        #[clap(help = "the block you want to query, can also be earliest/latest/pending", parse(try_from_str = parse_block_id))]
        block: BlockId,
        #[clap(long, env = "CAST_FULL_BLOCK")]
        full: bool,
        field: Option<String>,
        #[clap(long = "json", short = 'j')]
        to_json: bool,
        #[clap(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[clap(name = "block-number")]
    #[clap(about = "Prints latest block number")]
    BlockNumber {
        #[clap(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[clap(name = "call")]
    #[clap(about = "Perform a local call to <to> without publishing a transaction.")]
    Call {
        #[clap(help = "the address you want to query", parse(try_from_str = parse_name_or_address))]
        address: NameOrAddress,
        sig: String,
        args: Vec<String>,
        #[clap(long, short, help = "the block you want to query, can also be earliest/latest/pending", parse(try_from_str = parse_block_id))]
        block: Option<BlockId>,
        #[clap(flatten)]
        eth: EthereumOpts,
    },
    #[clap(about = "Pack a signature and an argument list into hexadecimal calldata.")]
    Calldata {
        #[clap(
            help = r#"When called with <sig> of the form <name>(<types>...), then perform ABI encoding to produce the hexadecimal calldata.
        If the value given—containing at least one slash character—then treat it as a file name to read, and proceed as if the contents were passed as hexadecimal data.
        Given data, ensure it is hexadecimal calldata starting with 0x and normalize it to lowercase.
        "#
        )]
        sig: String,
        #[clap(allow_hyphen_values = true)] // negative values not yet supported internally
        args: Vec<String>,
    },
    #[clap(name = "chain")]
    #[clap(about = "Prints symbolic name of current blockchain by checking genesis hash")]
    Chain {
        #[clap(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[clap(name = "chain-id")]
    #[clap(about = "Returns ethereum chain id")]
    ChainId {
        #[clap(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[clap(name = "client")]
    #[clap(about = "Returns the current client version")]
    Client {
        #[clap(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[clap(name = "namehash")]
    #[clap(about = "Returns ENS namehash of provided name")]
    Namehash { name: String },
    #[clap(name = "tx")]
    #[clap(about = "Show information about the transaction <tx-hash>")]
    Tx {
        hash: String,
        field: Option<String>,
        #[clap(long = "json", short = 'j')]
        to_json: bool,
        #[clap(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[clap(name = "receipt")]
    #[clap(about = "Print information about the transaction receipt for <tx-hash>")]
    Receipt {
        hash: String,
        field: Option<String>,
        #[clap(
            short,
            long,
            help = "the number of confirmations until the receipt is fetched",
            default_value = "1"
        )]
        confirmations: usize,
        #[clap(long, env = "CAST_ASYNC")]
        cast_async: bool,
        #[clap(long = "json", short = 'j')]
        to_json: bool,
        #[clap(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[clap(name = "send")]
    #[clap(about = "Publish a transaction signed by <from> to call <to> with <data>")]
    SendTx {
        #[clap(help = "the address you want to transact with", parse(try_from_str = parse_name_or_address))]
        to: NameOrAddress,
        #[clap(help = "the function signature or name you want to call")]
        sig: Option<String>,
        #[clap(help = "the list of arguments you want to call the function with")]
        args: Vec<String>,
        #[clap(long, help = "gas quantity for the transaction", parse(try_from_str = parse_u256))]
        gas: Option<U256>,
        #[clap(long = "gas-price", help = "gas price for the transaction", env = "ETH_GAS_PRICE", parse(try_from_str = parse_ether_value))]
        gas_price: Option<U256>,
        #[clap(long, help = "ether value (in wei or string with unit type e.g. 1ether, 10gwei, 0.01ether) for the transaction", parse(try_from_str = parse_ether_value))]
        value: Option<U256>,
        #[clap(long, help = "nonce for the transaction", parse(try_from_str = parse_u256))]
        nonce: Option<U256>,
        #[clap(long, env = "CAST_ASYNC")]
        cast_async: bool,
        #[clap(flatten)]
        eth: EthereumOpts,
        #[clap(
            long,
            help = "use legacy transactions instead of EIP1559 ones. this is auto-enabled for common networks without EIP1559"
        )]
        legacy: bool,
        #[clap(
            short,
            long,
            help = "the number of confirmations until the receipt is fetched",
            default_value = "1"
        )]
        confirmations: usize,
        #[clap(long = "json", short = 'j')]
        to_json: bool,
    },
    #[clap(name = "publish")]
    #[clap(about = "Publish a raw transaction to the network")]
    PublishTx {
        #[clap(help = "the raw transaction you want to publish")]
        raw_tx: String,
        #[clap(long, env = "CAST_ASYNC")]
        cast_async: bool,
        #[clap(flatten)]
        eth: EthereumOpts,
    },
    #[clap(name = "estimate")]
    #[clap(about = "Estimate the gas cost of a transaction from <from> to <to> with <data>")]
    Estimate {
        #[clap(help = "the address you want to transact with", parse(try_from_str = parse_name_or_address))]
        to: NameOrAddress,
        #[clap(help = "the function signature or name you want to call")]
        sig: String,
        #[clap(help = "the list of arguments you want to call the function with")]
        args: Vec<String>,
        #[clap(long, help = "value for tx estimate (in wei)")]
        value: Option<U256>,
        #[clap(flatten)]
        eth: EthereumOpts,
    },
    #[clap(name = "--calldata-decode")]
    #[clap(about = "Decode ABI-encoded hex input data. Use `--abi-decode` to decode output data")]
    CalldataDecode {
        #[clap(
            help = "the function signature you want to decode, in the format `<name>(<in-types>)(<out-types>)`"
        )]
        sig: String,
        #[clap(help = "the encoded calladata, in hex format")]
        calldata: String,
    },
    #[clap(name = "--abi-decode")]
    #[clap(
        about = "Decode ABI-encoded hex output data. Pass --input to decode as input, or use `--calldata-decode`"
    )]
    AbiDecode {
        #[clap(
            help = "the function signature you want to decode, in the format `<name>(<in-types>)(<out-types>)`"
        )]
        sig: String,
        #[clap(help = "the encoded calladata, in hex format")]
        calldata: String,
        #[clap(long, short, help = "the encoded output, in hex format")]
        input: bool,
    },
    #[clap(name = "abi-encode")]
    #[clap(
        about = "ABI encodes the given arguments with the function signature, excluidng the selector"
    )]
    AbiEncode {
        #[clap(help = "the function signature")]
        sig: String,
        #[clap(help = "the list of function arguments")]
        #[clap(allow_hyphen_values = true)]
        args: Vec<String>,
    },
    #[clap(name = "index")]
    #[clap(
        about = "Get storage slot of value from mapping type, mapping slot number and input value"
    )]
    Index {
        #[clap(help = "mapping key type")]
        from_type: String,
        #[clap(help = "mapping value type")]
        to_type: String,
        #[clap(help = "the value")]
        from_value: String,
        #[clap(help = "storage slot of the mapping")]
        slot_number: String,
    },
    #[clap(name = "4byte")]
    #[clap(about = "Fetches function signatures given the selector from 4byte.directory")]
    FourByte {
        #[clap(help = "the function selector")]
        selector: String,
    },
    #[clap(name = "4byte-decode")]
    #[clap(about = "Decodes transaction calldata by fetching the signature using 4byte.directory")]
    FourByteDecode {
        #[clap(help = "the ABI-encoded calldata")]
        calldata: String,
        #[clap(long, help = "the 4byte selector id to use, can also be earliest/latest")]
        id: Option<String>,
    },
    #[clap(name = "4byte-event")]
    #[clap(
        about = "Takes a 32 byte topic and prints the response from querying 4byte.directory for that topic"
    )]
    FourByteEvent {
        #[clap(help = "the 32 byte topic")]
        topic: String,
    },
    #[clap(name = "pretty-calldata")]
    #[clap(about = "Pretty prints calldata, if available gets signature from 4byte.directory")]
    PrettyCalldata {
        #[clap(help = "Hex encoded calldata")]
        calldata: String,
        #[clap(long, short, help = "Skip the 4byte directory lookup.")]
        offline: bool,
    },

    #[clap(name = "age")]
    #[clap(about = "Prints the timestamp of a block")]
    Age {
        #[clap(global = true, help = "the block you want to query, can also be earliest/latest/pending", parse(try_from_str = parse_block_id))]
        block: Option<BlockId>,
        #[clap(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[clap(name = "balance")]
    #[clap(about = "Print the balance of <account> in wei")]
    Balance {
        #[clap(long, short, help = "the block you want to query, can also be earliest/latest/pending", parse(try_from_str = parse_block_id))]
        block: Option<BlockId>,
        #[clap(help = "the account you want to query", parse(try_from_str = parse_name_or_address))]
        who: NameOrAddress,
        #[clap(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[clap(name = "basefee")]
    #[clap(about = "Print the basefee of a block")]
    BaseFee {
        #[clap(global = true, help = "the block you want to query, can also be earliest/latest/pending", parse(try_from_str = parse_block_id))]
        block: Option<BlockId>,
        #[clap(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[clap(name = "code")]
    #[clap(about = "Prints the bytecode at <address>")]
    Code {
        #[clap(long, short, help = "the block you want to query, can also be earliest/latest/pending", parse(try_from_str = parse_block_id))]
        block: Option<BlockId>,
        #[clap(help = "the address you want to query", parse(try_from_str = parse_name_or_address))]
        who: NameOrAddress,
        #[clap(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[clap(name = "gas-price")]
    #[clap(about = "Prints current gas price of target chain")]
    GasPrice {
        #[clap(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[clap(name = "keccak")]
    #[clap(about = "Keccak-256 hashes arbitrary data")]
    Keccak { data: String },
    #[clap(name = "resolve-name")]
    #[clap(about = "Returns the address the provided ENS name resolves to")]
    ResolveName {
        #[clap(help = "the account you want to resolve")]
        who: Option<String>,
        #[clap(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
        #[clap(long, short, help = "do a forward resolution to ensure the ENS name is correct")]
        verify: bool,
    },
    #[clap(name = "lookup-address")]
    #[clap(about = "Returns the name the provided address resolves to")]
    LookupAddress {
        #[clap(help = "the account you want to resolve")]
        who: Option<Address>,
        #[clap(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
        #[clap(long, short, help = "do a forward resolution to ensure the address is correct")]
        verify: bool,
    },
    #[clap(name = "storage", about = "Show the raw value of a contract's storage slot")]
    Storage {
        #[clap(help = "the contract address", parse(try_from_str = parse_name_or_address))]
        address: NameOrAddress,
        #[clap(help = "the storage slot number (hex or number)", parse(try_from_str = parse_slot))]
        slot: H256,
        #[clap(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
        #[clap(
            long,
            short,
            help = "the block you want to query, can also be earliest/latest/pending",
            parse(try_from_str = parse_block_id)
        )]
        block: Option<BlockId>,
    },
    #[clap(name = "proof", about = "Generate a storage proof for a given slot")]
    Proof {
        #[clap(help = "the contract address", parse(try_from_str = parse_name_or_address))]
        address: NameOrAddress,
        #[clap(help = "the storage slot numbers (hex or number)", parse(try_from_str = parse_slot))]
        slots: Vec<H256>,
        #[clap(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
        #[clap(
            long,
            short,
            help = "the block you want to query, can also be earliest/latest/pending",
            parse(try_from_str = parse_block_id)
        )]
        block: Option<BlockId>,
    },
    #[clap(name = "nonce")]
    #[clap(about = "Prints the number of transactions sent from <address>")]
    Nonce {
        #[clap(long, short = 'B', help = "the block you want to query, can also be earliest/latest/pending", parse(try_from_str = parse_block_id))]
        block: Option<BlockId>,
        #[clap(help = "the address you want to query", parse(try_from_str = parse_name_or_address))]
        who: NameOrAddress,
        #[clap(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[clap(name = "etherscan-source")]
    #[clap(about = "Prints the source code of a contract from Etherscan")]
    EtherscanSource {
        #[clap(flatten)]
        chain: ClapChain,
        #[clap(help = "the contract address")]
        address: String,
        #[clap(long, env = "ETHERSCAN_API_KEY")]
        etherscan_api_key: String,
    },
    #[clap(name = "wallet", about = "Set of wallet management utilities")]
    Wallet {
        #[clap(subcommand)]
        command: WalletSubcommands,
    },
    #[clap(
        name = "interface",
        about = "Generate contract's interface from ABI. Currently it doesn't support ABI encoder V2"
    )]
    Interface {
        #[clap(help = "The contract address or path to ABI file")]
        path_or_address: String,
        #[clap(long, short, default_value = "^0.8.10", help = "pragma version")]
        pragma: String,
        #[clap(short, help = "Path to output file. Defaults to stdout")]
        output_location: Option<PathBuf>,
        #[clap(short, env = "ETHERSCAN_API_KEY", help = "etherscan API key")]
        etherscan_api_key: Option<String>,
        #[clap(flatten)]
        chain: ClapChain,
    },
    #[clap(name = "sig", about = "Print a function's 4-byte selector")]
    Sig {
        #[clap(help = "The human-readable function signature, e.g. 'transfer(address,uint256)'")]
        sig: String,
    },
    #[clap(
        name = "find-block",
        about = "Prints the block number closes to the provided timestamp"
    )]
    FindBlock {
        #[clap(help = "The UNIX timestamp to search for (in seconds)")]
        timestamp: u64,
        #[clap(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[clap(about = "Generate shell completions script")]
    Completions {
        #[clap(arg_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Debug, Parser)]
pub enum WalletSubcommands {
    #[clap(name = "new", about = "Create and output a new random keypair")]
    New {
        #[clap(help = "If provided, then keypair will be written to encrypted json keystore")]
        path: Option<String>,
        #[clap(
            long,
            short,
            help = "Triggers a hidden password prompt for the json keystore",
            conflicts_with = "unsafe-password",
            requires = "path"
        )]
        password: bool,
        #[clap(
            long,
            help = "Password for json keystore in cleartext. This is UNSAFE to use and we recommend using the --password parameter",
            requires = "path",
            env = "CAST_PASSWORD"
        )]
        unsafe_password: Option<String>,
    },
    #[clap(name = "vanity", about = "Generate a vanity address")]
    Vanity {
        #[clap(long, help = "Prefix for vanity address", required_unless_present = "ends-with")]
        starts_with: Option<String>,
        #[clap(long, help = "Suffix for vanity address")]
        ends_with: Option<String>,
    },
    #[clap(name = "address", about = "Convert a private key to an address")]
    Address {
        #[clap(flatten)]
        wallet: Wallet,
    },
    #[clap(name = "sign", about = "Sign the message with provided private key")]
    Sign {
        #[clap(help = "message to sign")]
        message: String,
        #[clap(flatten)]
        wallet: Wallet,
    },
    #[clap(name = "verify", about = "Verify the signature on the message")]
    Verify {
        #[clap(help = "original message")]
        message: String,
        #[clap(help = "signature to verify")]
        signature: String,
        #[clap(long, short, help = "pubkey of message signer")]
        address: String,
    },
}

fn parse_name_or_address(s: &str) -> eyre::Result<NameOrAddress> {
    Ok(if s.starts_with("0x") {
        NameOrAddress::Address(s.parse::<Address>()?)
    } else {
        NameOrAddress::Name(s.into())
    })
}

fn parse_block_id(s: &str) -> eyre::Result<BlockId> {
    Ok(match s {
        "earliest" => BlockId::Number(BlockNumber::Earliest),
        "latest" => BlockId::Number(BlockNumber::Latest),
        "pending" => BlockId::Number(BlockNumber::Pending),
        s if s.starts_with("0x") => BlockId::Hash(H256::from_str(s)?),
        s => BlockId::Number(BlockNumber::Number(u64::from_str(s)?.into())),
    })
}

fn parse_slot(s: &str) -> eyre::Result<H256> {
    Ok(if s.starts_with("0x") {
        let padded = format!("{:0>64}", s.strip_prefix("0x").unwrap());
        H256::from_str(&padded)?
    } else {
        H256::from_low_u64_be(u64::from_str(s)?)
    })
}

fn parse_ether_value(value: &str) -> eyre::Result<U256> {
    Ok(if value.starts_with("0x") {
        U256::from_str(value)?
    } else {
        U256::from(LenientTokenizer::tokenize_uint(value)?)
    })
}

#[derive(Debug, Parser)]
#[clap(name = "cast", version = crate::utils::VERSION_MESSAGE)]
pub struct Opts {
    #[clap(subcommand)]
    pub sub: Subcommands,
}
