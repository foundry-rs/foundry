use ethers::types::{Address, U256};
use ethers::{prelude::*, signers::coins_bip39::English};
use std::convert::TryFrom;
use std::path::PathBuf;
use std::str::FromStr;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(about = "Choose what NFT subcommand you want to execute")]
pub enum Subcommands {
    Buy(BuyOpts),
    Deploy(DeployOpts),
    Prices(PricesOpts),
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

    #[structopt(long = "flashbots.bribe", parse(from_str = parse_u256), help = "The amount to be sent to the miner")]
    pub bribe: Option<U256>,
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Get OpenSea orderbook information about the token")]
pub struct PricesOpts {
    #[structopt(flatten)]
    pub nft: NftOpts,
}

#[derive(StructOpt, Debug, Clone)]
pub struct NftOpts {
    #[structopt(
        long = "nft.erc1155",
        short,
        help = "Whether the token you chose is an ERC1155 token, so that we make the correct ownership call check"
    )]
    pub erc1155: bool,

    #[structopt(long = "nft.address", short, help = "The NFT address you want to buy")]
    pub address: Address,

    #[structopt(long = "nft.ids", help = "The NFT id(s) you want to buy", parse(from_str = parse_u256))]
    pub ids: Vec<U256>,

    #[structopt(
        long = "nft.ids_path",
        help = "The file containing the NFT id(s) you want to buy"
    )]
    pub ids_path: Option<PathBuf>,
}

use std::fs::File;
use std::io::BufRead;
impl NftOpts {
    /// Returns a vector of token ids and quantities to check for
    pub fn tokens(&self) -> eyre::Result<(Vec<U256>, Vec<usize>)> {
        // read from a csv if a file is given
        Ok(if let Some(ref ids_path) = self.ids_path {
            let file = File::open(ids_path)?;
            let lines = std::io::BufReader::new(file).lines();
            let mut ids = Vec::new();
            let mut quantities = Vec::new();
            for line in lines {
                let line = line?;
                let mut line = line.split(',');
                let id = line.next().expect("no id found");
                let id = U256::from_dec_str(id)?;
                let quantity = match line.next() {
                    Some(inner) => usize::from_str(inner).unwrap_or(1),
                    None => 1,
                };
                ids.push(id);
                quantities.push(quantity);
            }
            (ids, quantities)
        } else {
            // assume 1 copy of each token if given via the cli
            (self.ids.clone(), vec![1; self.ids.len()])
        })
    }
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(
    about = "Deploy the Ethereum contract for doing consistency checks inside a Flashbots bundle"
)]
pub struct DeployOpts {
    #[structopt(flatten)]
    pub eth: EthereumOpts,
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Purchase 1 or more NFTs, with optional Flashbots support")]
pub struct BuyOpts {
    #[structopt(flatten)]
    pub eth: EthereumOpts,

    #[structopt(flatten)]
    pub flashbots: FlashBotsOpts,

    #[structopt(flatten)]
    pub nft: NftOpts,

    #[structopt(
        long,
        help = "Whether you're buying an ERC721 or an ERC1155 (true for 1155)"
    )]
    #[structopt(long, help = "Create and log the transactions without submitting them")]
    pub dry_run: bool,
}

fn parse_u256(s: &str) -> U256 {
    U256::from_dec_str(s).unwrap()
}
