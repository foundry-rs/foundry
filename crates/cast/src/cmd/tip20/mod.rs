use crate::{
    cmd::send::SendTxArgs,
    tempo,
    tx::{SendTxOpts, TxParams},
};
use alloy_ens::NameOrAddress;
use alloy_primitives::{Address, B256};
use clap::Parser;
use eyre::Result;
use foundry_cli::utils::get_chain;
use std::str::FromStr;
use tempo_alloy::TempoNetwork;

mod create;
pub(crate) use create::iso4217_warning_message;
pub(crate) mod logo;
pub(crate) mod mine;

const MINE_REGISTER_MESSAGES: mine::RegisterMessages = mine::RegisterMessages {
    no_signer: "--register requires a signer or Tempo keychain identity (for example --private-key or --from)",
    mismatch: "registration sender mismatch: mined salt is for",
    submitting: " on Tempo...",
};

/// TIP-20 token operations (Tempo).
#[derive(Debug, Parser, Clone)]
pub enum Tip20Subcommand {
    /// Create a new TIP-20 token via the TIP20Factory.
    #[command(visible_alias = "c")]
    Create {
        /// The token name (e.g. "US Dollar Coin").
        name: String,

        /// The token symbol (e.g. "USDC").
        symbol: String,

        /// The ISO 4217 currency code (e.g. "USD", "EUR", "GBP").
        /// This field is IMMUTABLE after creation and affects fee payment
        /// eligibility, DEX routing, and quote token pairing.
        currency: String,

        /// The TIP-20 quote token address used for exchange pricing.
        #[arg(value_parser = NameOrAddress::from_str)]
        quote_token: NameOrAddress,

        /// The admin address to receive DEFAULT_ADMIN_ROLE on the new token.
        #[arg(value_parser = NameOrAddress::from_str)]
        admin: NameOrAddress,

        /// A unique salt for deterministic address derivation (hex-encoded bytes32).
        salt: B256,

        /// Optional T5 logo URI for the token.
        #[arg(long, value_name = "URI")]
        logo_uri: Option<String>,

        /// Skip the ISO 4217 currency code validation warning.
        #[arg(long)]
        force: bool,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },

    /// Validate a TIP-20 logo URI offline against Tempo T5 constraints.
    LogoCheck {
        /// The logo URI to validate. Empty string is valid.
        #[arg(value_name = "URI")]
        logo_uri: String,
    },

    /// Update a TIP-20 token logo URI.
    LogoSet {
        /// The TIP-20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The new logo URI. Empty string clears the on-chain value.
        #[arg(value_name = "URI")]
        logo_uri: String,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },

    /// Mine a TIP-1022 salt for virtual address' master registration on Tempo.
    #[command(visible_alias = "m")]
    Mine {
        /// Address that will call `registerVirtualMaster(bytes32)`.
        #[arg(value_name = "ADDRESS")]
        master: Address,

        /// Salt to validate directly instead of mining one.
        #[arg(long, conflicts_with_all = ["seed", "no_random"], value_name = "HEX")]
        salt: Option<B256>,

        /// Number of threads to use. Specifying 0 defaults to the number of logical cores.
        #[arg(global = true, long, short = 'j', visible_alias = "jobs")]
        threads: Option<usize>,

        /// The random number generator's seed, used to initialize the salt search.
        #[arg(long, value_name = "HEX")]
        seed: Option<B256>,

        /// Don't initialize the salt with a random value, and instead use the default value of 0.
        #[arg(long, conflicts_with = "seed")]
        no_random: bool,

        /// Submit `registerVirtualMaster(bytes32)` on Tempo after finding or validating the salt.
        #[arg(long, conflicts_with_all = ["seed", "no_random"])]
        register: bool,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },
}

impl Tip20Subcommand {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::Create {
                name,
                symbol,
                currency,
                quote_token,
                admin,
                salt,
                logo_uri,
                force,
                send_tx,
                tx,
            } => {
                create::run(
                    name,
                    symbol,
                    currency,
                    quote_token,
                    admin,
                    salt,
                    logo_uri,
                    force,
                    send_tx,
                    tx,
                )
                .await
            }
            Self::LogoCheck { logo_uri } => logo::check(&logo_uri),
            Self::LogoSet { token, logo_uri, send_tx, tx } => {
                logo::set(token, logo_uri, send_tx, tx).await
            }
            Self::Mine { master, salt, threads, seed, no_random, register, send_tx, tx } => {
                let output = mine::run(master, salt, threads, seed, no_random)?;
                if register {
                    let msgs = &MINE_REGISTER_MESSAGES;
                    mine::register_virtual_master(master, output.salt, send_tx, tx, false, msgs)
                        .await?;
                }
                Ok(())
            }
        }
    }
}

/// Sends pre-encoded `data` to the Tempo contract at `to` through the `cast send` flow, signing
/// with the selected Tempo session, access key, or wallet.
pub(crate) async fn send_tip20_transaction(
    to: Address,
    data: Vec<u8>,
    send_tx: SendTxOpts,
    tx: TxParams,
) -> Result<()> {
    tempo::ensure_session_not_browser(&tx.tempo, send_tx.browser.browser)?;
    let (config, provider) = tempo::tempo_provider(&send_tx.eth.rpc)?;
    let chain = get_chain(config.chain, &provider).await?;
    let (signer, access_key) =
        tempo::resolve_session_or_wallet_signer(&tx.tempo, &send_tx.eth.wallet, chain.id()).await?;
    SendTxArgs::contract_call(NameOrAddress::Address(to), data, send_tx, tx)
        .run_generic::<TempoNetwork>(signer, access_key)
        .await
}
