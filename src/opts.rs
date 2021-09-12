use ethers::types::{Address, U256};
use ethers::{prelude::*, signers::coins_bip39::English};
use std::convert::TryFrom;
use std::str::FromStr;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(about = "Perform Ethereum RPC calls from the comfort of your command line.")]
pub enum Subcommands {
    #[structopt(name = "--from-ascii")]
    #[structopt(about = "convert text data into hexdata")]
    FromAscii { text: String },
    #[structopt(name = "--to-checksum-address")]
    #[structopt(about = "convert an address to a checksummed format (EIP-55)")]
    ToCheckSumAddress { address: Address },
    #[structopt(name = "--to-bytes32")]
    #[structopt(about = "left-pads a hex bytes string to 32 bytes)")]
    ToBytes32 { bytes: String },
    #[structopt(name = "block")]
    #[structopt(
        about = "Prints information about <block>. If <field> is given, print only the value of that field"
    )]
    Block {
        #[structopt(help = "the block you want to query, can also be earliest/latest/pending", parse(try_from_str = parse_block_id))]
        block: BlockId,
        #[structopt(long, env = "SETH_FULL_BLOCK")]
        full: bool,
        field: Option<String>,
        #[structopt(long = "--json", short = "-j")]
        to_json: bool,
        #[structopt(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[structopt(name = "call")]
    #[structopt(about = "Perform a local call to <to> without publishing a transaction.")]
    Call {
        #[structopt(help = "the address you want to query")]
        address: Address,
        sig: String,
        args: Vec<String>,
        #[structopt(long, env = "ETH_RPC_URL")]
        rpc_url: String,
    },
    #[structopt(name = "send")]
    #[structopt(about = "Publish a transaction signed by <from> to call <to> with <data>")]
    SendTx {
        #[structopt(help = "the address you want to query")]
        address: Address,
        sig: String,
        args: Vec<String>,
        #[structopt(long, env = "ETH_RPC_URL")]
        rpc_url: String,
        #[structopt(long, env = "ETH_FROM")]
        from: Address,
        #[structopt(long, env = "SETH_ASYNC")]
        seth_async: bool,
    },
}

fn parse_block_id(s: &str) -> eyre::Result<BlockId> {
    Ok(match s {
        "earliest" => BlockId::Number(BlockNumber::Earliest),
        "latest" => BlockId::Number(BlockNumber::Latest),
        s if s.starts_with("0x") => BlockId::Hash(H256::from_str(s)?),
        s => BlockId::Number(BlockNumber::Number(U64::from_str(s)?)),
    })
}

#[derive(Debug, StructOpt)]
pub struct Opts {
    #[structopt(subcommand)]
    pub sub: Subcommands,
}

#[derive(StructOpt, Debug, Clone)]
pub struct EthereumOpts {
    #[structopt(long = "eth.url", short, help = "The tracing / archival node's URL")]
    pub url: String,

    #[structopt(long = "eth.private_key", help = "Your private key string")]
    pub private_key: Option<String>,

    #[structopt(long = "eth.mnemonic", help = "Path to your mnemonic file")]
    pub mnemonic_path: Option<String>,

    #[structopt(
        long = "eth.hd_index",
        help = "your index in the standard hd path",
        default_value = "0"
    )]
    pub index: u32,
}

// TODO: Improve these so that we return a middleware trait object
use std::sync::Arc;
impl EthereumOpts {
    pub fn provider(&self) -> eyre::Result<Arc<Provider<Http>>> {
        Ok(Arc::new(Provider::try_from(self.url.as_str())?))
    }

    /// Returns a [`LocalWallet`] corresponding to the provided private key or mnemonic
    pub fn signer(&self) -> eyre::Result<LocalWallet> {
        if let Some(ref private_key) = self.private_key {
            Ok(LocalWallet::from_str(private_key)?)
        } else if let Some(ref mnemonic_path) = self.mnemonic_path {
            let mnemonic = std::fs::read_to_string(mnemonic_path)?.replace("\n", "");
            Ok(MnemonicBuilder::<English>::default()
                .phrase(mnemonic.as_str())
                .index(self.index)?
                .build()?)
        } else {
            panic!("Expected mnemonic or private key");
        }
    }
}

#[derive(StructOpt, Debug, Clone)]
pub struct FlashBotsOpts {
    #[structopt(
        long = "flashbots.bribe_receiver",
        help = "The address that will receive the bribe. Ideally it should be a smart contract with a block.coinbase transfer"
    )]
    pub bribe_receiver: Option<Address>,

    // #[structopt(long = "flashbots.bribe", parse(from_str = parse_u256), help = "The amount to be sent to the miner")]
    pub bribe: Option<U256>,
}
