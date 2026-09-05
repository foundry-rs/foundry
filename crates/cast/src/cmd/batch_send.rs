//! `cast batch-send` command implementation.
//!
//! Sends a batch of calls as a single Tempo transaction using native call batching.
//! Unlike upstream Foundry's sequential transactions, this uses a single type 0x76
//! transaction with multiple calls executed atomically.

use crate::{
    call_spec::CallSpec,
    cmd::{
        auth::{confirm_and_build, confirm_and_build_with_tempo_wallet},
        send::{SendOptions, cast_send, cast_send_with_tempo_wallet},
    },
    tempo,
    tx::{self, CastTxBuilder, InitState, InputState, SendTxOpts, apply_poll_interval},
};
use alloy_network::EthereumWallet;
use alloy_provider::{Provider, ProviderBuilder as AlloyProviderBuilder};
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::TransactionOpts,
    utils::{self, resolve_lane},
};
use tempo_alloy::TempoNetwork;

/// CLI arguments for `cast batch-send`.
///
/// Sends multiple calls as a single atomic Tempo transaction.
#[derive(Debug, Parser)]
pub struct BatchSendArgs {
    /// Call specifications in format: `to[:<value>][:<sig>[:<args>]]` or `to[:<value>][:<0xdata>]`
    ///
    /// Examples:
    ///   --call "0x123:0.1ether" (ETH transfer)
    ///   --call "0x456::transfer(address,uint256):0x789,1000" (ERC20 transfer)
    ///   --call "0xabc::0x123def" (raw calldata)
    ///   --call "0x123:1ether:deposit()" (value + function call)
    #[arg(long = "call", value_name = "SPEC", required = true)]
    pub calls: Vec<String>,

    #[command(flatten)]
    pub send_tx: SendTxOpts,

    #[command(flatten)]
    pub tx: TransactionOpts,

    /// Skip the EIP-7702 authorization disclosure confirmation.
    #[arg(long)]
    pub force: bool,

    /// Send via `eth_sendTransaction` using the `--from` argument or $ETH_FROM as sender
    #[arg(long, requires = "from")]
    pub unlocked: bool,
}

impl BatchSendArgs {
    pub async fn run(self) -> Result<()> {
        let Self { calls, send_tx, mut tx, force, unlocked } = self;
        // Tempo sessions must sign with the session key; these modes route signing through a
        // node-managed account or browser wallet instead.
        if tx.tempo.session_id()?.is_some() && unlocked {
            eyre::bail!("--tempo.session/TEMPO_SESSION_ID cannot be combined with --unlocked");
        }
        tempo::ensure_session_not_browser(&tx.tempo, send_tx.browser.browser)?;

        let expires_at = tx.tempo.resolve_expires();

        let (config, provider) = tempo::tempo_provider(&send_tx.eth)?;
        let resolved_lane = resolve_lane(&mut tx.tempo, &config.root)?;
        let lane = resolved_lane.as_ref();

        apply_poll_interval(&provider, send_tx.poll_interval);

        let chain = utils::get_chain(config.chain, &provider).await?;
        let (signer, tempo_access_key) =
            tempo::resolve_session_or_wallet_signer(&tx.tempo, &send_tx.eth.wallet, chain.id())
                .await?;

        // Preserve key_id for modes that do not call build_with_tempo_wallet, such as unlocked.
        if let Some(access_key) = &tempo_access_key {
            tx.tempo.key_id = Some(access_key.key_id()?);
        }

        let builder = CastTxBuilder::<TempoNetwork, _, _>::new(&provider, tx, &config).await?;
        let builder = with_batch_calls(&calls, builder, &provider).await?;
        tempo::print_expires(expires_at)?;

        let send_opts =
            SendOptions::new(&send_tx, &config).resolving_fee_token(Some(chain), &config);

        if unlocked {
            let Some(tx) = confirm_and_build(builder, config.sender, force, lane, false).await?
            else {
                return Ok(());
            };
            cast_send(provider, tx, &send_opts).await?;
        } else if let Some(access_key) = &tempo_access_key {
            let Some((tx_request, prepared)) =
                confirm_and_build_with_tempo_wallet(builder, access_key, force, lane).await?
            else {
                return Ok(());
            };
            cast_send_with_tempo_wallet(&provider, tx_request, &prepared, &send_opts).await?;
        } else {
            let (signer, _) = tx::resolve_send_signer(signer, &send_tx.eth).await?;
            let Some(tx_request) = confirm_and_build(builder, &signer, force, lane, false).await?
            else {
                return Ok(());
            };
            let provider = AlloyProviderBuilder::<_, _, TempoNetwork>::default()
                .wallet(EthereumWallet::from(signer))
                .connect_provider(&provider);
            cast_send(provider, tx_request, &send_opts).await?;
        }

        Ok(())
    }
}

/// Parses the `--call` specs, resolves them against the builder's chain and sets them as the
/// batch calls of the transaction.
pub(super) async fn with_batch_calls<P: Provider<TempoNetwork>>(
    calls: &[String],
    mut builder: CastTxBuilder<TempoNetwork, P, InitState>,
    provider: &impl Provider<TempoNetwork>,
) -> Result<CastTxBuilder<TempoNetwork, P, InputState>> {
    let specs = calls.iter().map(|s| CallSpec::parse(s)).collect::<Result<Vec<_>>>()?;
    let (etherscan_api_key, etherscan_api_url) = builder.etherscan_api();
    let mut tempo_calls = Vec::with_capacity(specs.len());
    for (i, spec) in specs.iter().enumerate() {
        tempo_calls.push(
            spec.resolve(i, builder.chain(), provider, etherscan_api_key, etherscan_api_url)
                .await?,
        );
    }
    sh_status!("Building batch transaction with {} call(s)...", tempo_calls.len())?;
    builder.tx_mut().calls = tempo_calls;

    // The builder requires a `to`; `build_aa` uses the calls instead, so point it at the first
    // call's target.
    builder
        .with_to(specs.first().map(|spec| spec.to.into()))
        .await?
        .with_code_sig_and_args(None, None, vec![])
        .await
}
