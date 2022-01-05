use std::str::FromStr;

use ethers::types::{Address, BlockId, BlockNumber, NameOrAddress, H256};
use structopt::StructOpt;

use super::{EthereumOpts, Wallet};

#[derive(Debug, StructOpt)]
#[structopt(about = "Perform Ethereum RPC calls from the comfort of your command line.")]
pub enum Subcommands {
    #[structopt(name = "--max-int")]
    #[structopt(about = "maximum i256 value")]
    MaxInt,
    #[structopt(name = "--min-int")]
    #[structopt(about = "minimum i256 value")]
    MinInt,
    #[structopt(name = "--max-uint")]
    #[structopt(about = "maximum u256 value")]
    MaxUint,
    #[structopt(aliases = &["--from-ascii"])]
    #[structopt(name = "--from-utf8")]
    #[structopt(about = "convert text data into hexdata")]
    FromUtf8 { text: Option<String> },
    #[structopt(name = "--to-hex")]
    #[structopt(about = "convert a decimal number into hex")]
    ToHex { decimal: Option<String> },
    #[structopt(name = "--to-hexdata")]
    #[structopt(about = r#"[<hex>|</path>|<@tag>]
    Output lowercase, 0x-prefixed hex, converting from the
    input, which can be:
      - mixed case hex with or without 0x prefix
      - 0x prefixed hex, concatenated with a ':'
      - absolute path to file
      - @tag, where $TAG is defined in environment variables
    "#)]
    ToHexdata { input: Option<String> },
    #[structopt(aliases = &["--to-checksum"])] // Compatibility with dapptools' cast
    #[structopt(name = "--to-checksum-address")]
    #[structopt(about = "convert an address to a checksummed format (EIP-55)")]
    ToCheckSumAddress { address: Option<Address> },
    #[structopt(name = "--to-ascii")]
    #[structopt(about = "convert hex data to text data")]
    ToAscii { hexdata: Option<String> },
    #[structopt(name = "--to-bytes32")]
    #[structopt(about = "left-pads a hex bytes string to 32 bytes)")]
    ToBytes32 { bytes: Option<String> },
    #[structopt(name = "--to-dec")]
    #[structopt(about = "convert hex value into decimal number")]
    ToDec { hexvalue: Option<String> },
    #[structopt(name = "--to-fix")]
    #[structopt(about = "convert integers into fixed point with specified decimals")]
    ToFix { decimals: Option<u128>, value: Option<String> },
    #[structopt(name = "--to-uint256")]
    #[structopt(about = "convert a number into uint256 hex string with 0x prefix")]
    ToUint256 { value: Option<String> },
    #[structopt(name = "--to-wei")]
    #[structopt(about = "convert an ETH amount into wei")]
    ToWei { value: Option<String>, unit: Option<String> },
    #[structopt(name = "--from-wei")]
    #[structopt(about = "convert wei into an ETH amount")]
    FromWei { value: Option<String>, unit: Option<String> },
    #[structopt(name = "block")]
    #[structopt(
        about = "Prints information about <block>. If <field> is given, print only the value of that field"
    )]
    Block {
        #[structopt(help = "the block you want to query, can also be earliest/latest/pending", parse(try_from_str = parse_block_id))]
        block: BlockId,
        #[structopt(long, env = "CAST_FULL_BLOCK")]
        full: bool,
        field: Option<String>,
        #[structopt(long = "--json", short = "-j")]
        to_json: bool,
        #[structopt(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[structopt(name = "block-number")]
    #[structopt(about = "Prints latest block number")]
    BlockNumber {
        #[structopt(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[structopt(name = "call")]
    #[structopt(about = "Perform a local call to <to> without publishing a transaction.")]
    Call {
        #[structopt(help = "the address you want to query", parse(try_from_str = parse_name_or_address))]
        address: NameOrAddress,
        sig: String,
        args: Vec<String>,
        #[structopt(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[structopt(about = "Pack a signature and an argument list into hexadecimal calldata.")]
    Calldata {
        #[structopt(
            help = r#"When called with <sig> of the form <name>(<types>...), then perform ABI encoding to produce the hexadecimal calldata.
        If the value given—containing at least one slash character—then treat it as a file name to read, and proceed as if the contents were passed as hexadecimal data.
        Given data, ensure it is hexadecimal calldata starting with 0x and normalize it to lowercase.
        "#
        )]
        sig: String,
        args: Vec<String>,
    },
    #[structopt(name = "chain")]
    #[structopt(about = "Prints symbolic name of current blockchain by checking genesis hash")]
    Chain {
        #[structopt(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[structopt(name = "chain-id")]
    #[structopt(about = "returns ethereum chain id")]
    ChainId {
        #[structopt(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[structopt(name = "namehash")]
    #[structopt(about = "returns ENS namehash of provided name")]
    Namehash { name: String },
    #[structopt(name = "tx")]
    #[structopt(about = "Show information about the transaction <tx-hash>")]
    Tx {
        hash: String,
        field: Option<String>,
        #[structopt(long = "--json", short = "-j")]
        to_json: bool,
        #[structopt(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[structopt(name = "send")]
    #[structopt(about = "Publish a transaction signed by <from> to call <to> with <data>")]
    SendTx {
        #[structopt(help = "the address you want to transact with", parse(try_from_str = parse_name_or_address))]
        to: NameOrAddress,
        #[structopt(help = "the function signature you want to call")]
        sig: String,
        #[structopt(help = "the list of arguments you want to call the function with")]
        args: Vec<String>,
        #[structopt(long, env = "CAST_ASYNC")]
        cast_async: bool,
        #[structopt(flatten)]
        eth: EthereumOpts,
    },
    #[structopt(name = "estimate")]
    #[structopt(about = "Estimate the gas cost of a transaction from <from> to <to> with <data>")]
    Estimate {
        #[structopt(help = "the address you want to transact with", parse(try_from_str = parse_name_or_address))]
        to: NameOrAddress,
        #[structopt(help = "the function signature you want to call")]
        sig: String,
        #[structopt(help = "the list of arguments you want to call the function with")]
        args: Vec<String>,
        #[structopt(flatten)]
        eth: EthereumOpts,
    },
    #[structopt(name = "--calldata-decode")]
    #[structopt(
        about = "Decode ABI-encoded hex input data. Use `--abi-decode` to decode output data"
    )]
    CalldataDecode {
        #[structopt(
            help = "the function signature you want to decode, in the format `<name>(<in-types>)(<out-types>)`"
        )]
        sig: String,
        #[structopt(help = "the encoded calladata, in hex format")]
        calldata: String,
    },
    #[structopt(name = "--abi-decode")]
    #[structopt(
        about = "Decode ABI-encoded hex output data. Pass --input to decode as input, or use `--calldata-decode`"
    )]
    AbiDecode {
        #[structopt(
            help = "the function signature you want to decode, in the format `<name>(<in-types>)(<out-types>)`"
        )]
        sig: String,
        #[structopt(help = "the encoded calladata, in hex format")]
        calldata: String,
        #[structopt(long, short, help = "the encoded output, in hex format")]
        input: bool,
    },
    #[structopt(name = "abi-encode")]
    #[structopt(
        help = "ABI encodes the given arguments with the function signature, excluidng the selector"
    )]
    AbiEncode {
        #[structopt(help = "the function signature")]
        sig: String,
        #[structopt(help = "the list of function arguments")]
        args: Vec<String>,
    },
    #[structopt(name = "4byte")]
    #[structopt(about = "Fetches function signatures given the selector from 4byte.directory")]
    FourByte {
        #[structopt(help = "the function selector")]
        selector: String,
    },
    #[structopt(name = "4byte-decode")]
    #[structopt(
        about = "Decodes transaction calldata by fetching the signature using 4byte.directory"
    )]
    FourByteDecode {
        #[structopt(help = "the ABI-encoded calldata")]
        calldata: String,
        #[structopt(long, help = "the 4byte selector id to use, can also be earliest/latest")]
        id: Option<String>,
    },
    #[structopt(name = "age")]
    #[structopt(about = "Prints the timestamp of a block")]
    Age {
        #[structopt(global = true, help = "the block you want to query, can also be earliest/latest/pending", parse(try_from_str = parse_block_id))]
        block: Option<BlockId>,
        #[structopt(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[structopt(name = "balance")]
    #[structopt(about = "Print the balance of <account> in wei")]
    Balance {
        #[structopt(long, short, help = "the block you want to query, can also be earliest/latest/pending", parse(try_from_str = parse_block_id))]
        block: Option<BlockId>,
        #[structopt(help = "the account you want to query", parse(try_from_str = parse_name_or_address))]
        who: NameOrAddress,
        #[structopt(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[structopt(name = "basefee")]
    #[structopt(about = "Print the basefee of a block")]
    BaseFee {
        #[structopt(global = true, help = "the block you want to query, can also be earliest/latest/pending", parse(try_from_str = parse_block_id))]
        block: Option<BlockId>,
        #[structopt(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[structopt(name = "code")]
    #[structopt(about = "Prints the bytecode at <address>")]
    Code {
        #[structopt(long, short, help = "the block you want to query, can also be earliest/latest/pending", parse(try_from_str = parse_block_id))]
        block: Option<BlockId>,
        #[structopt(help = "the address you want to query", parse(try_from_str = parse_name_or_address))]
        who: NameOrAddress,
        #[structopt(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[structopt(name = "gas-price")]
    #[structopt(about = "Prints current gas price of target chain")]
    GasPrice {
        #[structopt(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[structopt(name = "keccak")]
    #[structopt(about = "Keccak-256 hashes arbitrary data")]
    Keccak { data: String },
    #[structopt(name = "resolve-name")]
    #[structopt(about = "Returns the address the provided ENS name resolves to")]
    ResolveName {
        #[structopt(help = "the account you want to resolve")]
        who: Option<String>,
        #[structopt(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
        #[structopt(
            long,
            short,
            help = "do a forward resolution to ensure the ENS name is correct"
        )]
        verify: bool,
    },
    #[structopt(name = "lookup-address")]
    #[structopt(about = "Returns the name the provided address resolves to")]
    LookupAddress {
        #[structopt(help = "the account you want to resolve")]
        who: Option<Address>,
        #[structopt(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
        #[structopt(
            long,
            short,
            help = "do a forward resolution to ensure the address is correct"
        )]
        verify: bool,
    },
    #[structopt(name = "storage", about = "Show the raw value of a contract's storage slot")]
    Storage {
        #[structopt(help = "the contract address", parse(try_from_str = parse_name_or_address))]
        address: NameOrAddress,
        #[structopt(help = "the storage slot number (hex or number)", parse(try_from_str = parse_slot))]
        slot: H256,
        #[structopt(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
        #[structopt(
            long,
            short,
            help = "the block you want to query, can also be earliest/latest/pending",
            parse(try_from_str = parse_block_id)
        )]
        block: Option<BlockId>,
    },
    #[structopt(name = "nonce")]
    #[structopt(about = "Prints the number of transactions sent from <address>")]
    Nonce {
        #[structopt(long, short = "-B", help = "the block you want to query, can also be earliest/latest/pending", parse(try_from_str = parse_block_id))]
        block: Option<BlockId>,
        #[structopt(help = "the address you want to query", parse(try_from_str = parse_name_or_address))]
        who: NameOrAddress,
        #[structopt(short, long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[structopt(name = "wallet", about = "Set of wallet management utilities")]
    Wallet {
        #[structopt(subcommand)]
        command: WalletSubcommands,
    },
}

#[derive(Debug, StructOpt)]
pub enum WalletSubcommands {
    #[structopt(name = "new", about = "Create and output a new random keypair")]
    New {
        #[structopt(help = "If provided, then keypair will be written to encrypted json keystore")]
        path: Option<String>,
        #[structopt(
            long,
            short,
            help = "Triggers a hidden password prompt for the json keystore",
            conflicts_with = "unsafe-password",
            requires = "path"
        )]
        password: bool,
        #[structopt(
            long,
            help = "Password for json keystore in cleartext. This is UNSAFE to use and we recommend using the --password parameter",
            requires = "path",
            env = "CAST_PASSWORD"
        )]
        unsafe_password: Option<String>,
    },
    #[structopt(name = "vanity", about = "Generate a vanity address")]
    Vanity {
        #[structopt(long, help = "Prefix for vanity address", required_unless = "ends-with")]
        starts_with: Option<String>,
        #[structopt(long, help = "Suffix for vanity address")]
        ends_with: Option<String>,
    },
    #[structopt(name = "address", about = "Convert a private key to an address")]
    Address {
        #[structopt(flatten)]
        wallet: Wallet,
    },
    #[structopt(name = "sign", about = "Sign the message with provided private key")]
    Sign {
        #[structopt(help = "message to sign")]
        message: String,
        #[structopt(flatten)]
        wallet: Wallet,
    },
    #[structopt(name = "verify", about = "Verify the signature on the message")]
    Verify {
        #[structopt(help = "original message")]
        message: String,
        #[structopt(help = "signature to verify")]
        signature: String,
        #[structopt(long, short, help = "pubkey of message signer")]
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

#[derive(Debug, StructOpt)]
pub struct Opts {
    #[structopt(subcommand)]
    pub sub: Subcommands,
}
