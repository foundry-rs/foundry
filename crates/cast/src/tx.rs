use crate::traces::identifier::SignaturesIdentifier;
use alloy_consensus::{SidecarBuilder, SimpleCoder};
use alloy_dyn_abi::ErrorExt;
use alloy_ens::NameOrAddress;
use alloy_json_abi::Function;
use alloy_network::{Network, ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, TxHash, TxKind, U64, U256, hex};
use alloy_provider::{PendingTransactionBuilder, Provider};
use alloy_rpc_types::{AccessList, Authorization, TransactionInputKind};
use alloy_signer::Signer;
use alloy_transport::TransportError;
use clap::Args;
use eyre::{Result, WrapErr};
use foundry_cli::{
    opts::{CliAuthorizationList, EthereumOpts, TempoOpts, TransactionOpts},
    utils::{self, apply_gas_estimate_multiplier, parse_function_args},
};
use foundry_common::{
    FoundryTransactionBuilder, TransactionReceiptWithRevertReason,
    fmt::*,
    get_pretty_receipt_w_reason_attr,
    provider::fee::{estimate_eip1559_fees, resolve_broadcast_eip1559_fees},
    shell,
};
use foundry_config::{Chain, Config, Eip1559FeeEstimatePreset};
use foundry_wallets::{BrowserWalletOpts, TempoAccountsWallet, WalletOpts, WalletSigner};
use itertools::Itertools;
use std::{fmt::Write, marker::PhantomData, str::FromStr, time::Duration};

#[derive(Debug, Clone, Args)]
pub struct SendTxOpts {
    /// Only print the transaction hash and exit immediately.
    #[arg(id = "async", long = "async", alias = "cast-async", env = "CAST_ASYNC")]
    pub cast_async: bool,

    /// Wait for transaction receipt synchronously instead of polling.
    /// Note: uses `eth_sendTransactionSync` or `eth_sendRawTransactionSync`, which may not be
    /// supported by all clients.
    #[arg(long, conflicts_with = "async")]
    pub sync: bool,

    /// The number of confirmations until the receipt is fetched.
    #[arg(long, default_value = "1")]
    pub confirmations: u64,

    /// Timeout for sending the transaction.
    #[arg(long, env = "ETH_TIMEOUT")]
    pub timeout: Option<u64>,

    /// Polling interval for transaction receipts (in seconds).
    #[arg(long, alias = "poll-interval", env = "ETH_POLL_INTERVAL")]
    pub poll_interval: Option<u64>,

    /// Ethereum options
    #[command(flatten)]
    pub eth: EthereumOpts,

    /// Browser wallet options
    #[command(flatten)]
    pub browser: BrowserWalletOpts,
}

/// Applies the `--poll-interval` (in seconds) to a provider's receipt polling when one was given.
pub(crate) fn apply_poll_interval<N: Network>(provider: &impl Provider<N>, interval: Option<u64>) {
    if let Some(interval) = interval {
        provider.client().set_poll_interval(Duration::from_secs(interval));
    }
}

/// Transaction options shared across cast commands that submit on-chain transactions.
#[derive(Debug, Clone, Args)]
#[command(next_help_heading = "Transaction options")]
pub struct TxParams {
    /// Gas limit for the transaction.
    #[arg(long, env = "ETH_GAS_LIMIT")]
    pub gas_limit: Option<U256>,

    /// Gas price for legacy transactions, or max fee per gas for EIP1559 transactions.
    #[arg(long, env = "ETH_GAS_PRICE")]
    pub gas_price: Option<U256>,

    /// Max priority fee per gas for EIP1559 transactions.
    #[arg(long, env = "ETH_PRIORITY_GAS_PRICE")]
    pub priority_gas_price: Option<U256>,

    /// Nonce for the transaction.
    #[arg(long)]
    pub nonce: Option<U64>,

    #[command(flatten)]
    pub tempo: TempoOpts,
}

impl TxParams {
    pub(crate) fn apply<N: Network>(&self, tx: &mut N::TransactionRequest, legacy: bool)
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        self.clone().into_transaction_opts().apply::<N>(tx, legacy);
    }

    /// Converts the shared compact transaction options into the full `cast send` options.
    pub(crate) fn into_transaction_opts(self) -> TransactionOpts {
        TransactionOpts {
            gas_limit: self.gas_limit,
            gas_price: self.gas_price,
            priority_gas_price: self.priority_gas_price,
            value: None,
            nonce: self.nonce,
            legacy: false,
            blob: false,
            eip4844: false,
            blob_gas_price: None,
            auth: Vec::new(),
            access_list: None,
            tempo: self.tempo,
        }
    }
}

/// Different sender kinds used by [`CastTxBuilder`].
pub enum SenderKind<'a> {
    /// An address without signer. Used for read-only calls and transactions sent through unlocked
    /// accounts.
    Address(Address),
    /// A reference to a signer.
    Signer(&'a WalletSigner),
    /// An owned signer.
    OwnedSigner(Box<WalletSigner>),
}

impl SenderKind<'_> {
    /// Resolves the name to an Ethereum Address.
    pub fn address(&self) -> Address {
        match self {
            Self::Address(addr) => *addr,
            Self::Signer(signer) => signer.address(),
            Self::OwnedSigner(signer) => signer.address(),
        }
    }

    /// Resolves the sender from the wallet options.
    ///
    /// Prefers a configured signer (or Tempo wallet account) over `from`, and falls back to the
    /// zero address when neither is available.
    pub async fn from_wallet_opts(mut opts: WalletOpts) -> Result<Self> {
        let from = opts.from.take();
        let (signer, tempo_wallet) = opts.maybe_signer().await?;
        Ok(if let Some(signer) = signer {
            signer.into()
        } else if let Some(tempo_wallet) = tempo_wallet {
            tempo_wallet.account().into()
        } else {
            from.unwrap_or_default().into()
        })
    }

    /// Returns the signer if available.
    pub fn as_signer(&self) -> Option<&WalletSigner> {
        match self {
            Self::Signer(signer) => Some(signer),
            Self::OwnedSigner(signer) => Some(signer.as_ref()),
            Self::Address(_) => None,
        }
    }
}

impl From<Address> for SenderKind<'_> {
    fn from(addr: Address) -> Self {
        Self::Address(addr)
    }
}

impl<'a> From<&'a WalletSigner> for SenderKind<'a> {
    fn from(signer: &'a WalletSigner) -> Self {
        Self::Signer(signer)
    }
}

impl From<WalletSigner> for SenderKind<'_> {
    fn from(signer: WalletSigner) -> Self {
        Self::OwnedSigner(Box::new(signer))
    }
}

/// Resolves the sender of a read-only request: the browser wallet address when `--browser` is
/// set, otherwise the wallet options. Also returns whether the browser wallet was used.
pub(crate) async fn read_only_sender<N: Network>(
    browser: &BrowserWalletOpts,
    wallet: WalletOpts,
) -> Result<(SenderKind<'static>, bool)> {
    Ok(match browser.run::<N>().await? {
        Some(browser) => (browser.address().into(), true),
        None => (SenderKind::from_wallet_opts(wallet).await?, false),
    })
}

/// Validates that `sender` can resolve every EIP-7702 authorization.
pub(crate) fn validate_authorizations(
    authorizations: &[CliAuthorizationList],
    sender: &SenderKind<'_>,
) -> Result<()> {
    let address_auth_count = authorizations
        .iter()
        .filter(|auth| matches!(auth, CliAuthorizationList::Address(_)))
        .count();
    if address_auth_count > 1 {
        eyre::bail!(
            "Multiple address-based authorizations provided. Only one address can be specified; \
             use pre-signed authorizations (hex-encoded) for multiple authorizations."
        );
    }
    if address_auth_count == 1 && sender.as_signer().is_none() {
        eyre::bail!(
            "No signer available to sign authorization. \
             Provide a pre-signed authorization (hex-encoded) instead."
        );
    }

    Ok(())
}

/// Resolves the sending signer, falling back to the wallet options, and validates it against an
/// explicit `--from`.
pub(crate) async fn resolve_send_signer(
    pre_resolved: Option<WalletSigner>,
    eth: &EthereumOpts,
) -> Result<(WalletSigner, Address)> {
    let signer = match pre_resolved {
        Some(signer) => signer,
        None => eth.wallet.signer().await?,
    };
    let from = signer.address();
    validate_from_address(eth.wallet.from, from)?;
    Ok((signer, from))
}

/// Prevents a misconfigured hwlib from sending a transaction that defies user-specified --from
pub fn validate_from_address(
    specified_from: Option<Address>,
    signer_address: Address,
) -> Result<()> {
    if let Some(specified_from) = specified_from
        && specified_from != signer_address
    {
        eyre::bail!(
                "\
The specified sender via CLI/env vars does not match the sender configured via
the hardware wallet's HD Path.
Please use the `--hd-path <PATH>` parameter to specify the BIP32 Path which
corresponds to the sender, or let foundry automatically detect it by not specifying any sender address."
            );
    }
    Ok(())
}

/// Initial state.
#[derive(Debug)]
pub struct InitState;

/// State with known [TxKind].
#[derive(Debug)]
pub struct ToState {
    to: Option<Address>,
}

/// State with known input for the transaction.
#[derive(Debug)]
pub struct InputState {
    kind: TxKind,
    input: Vec<u8>,
    func: Option<Function>,
}

pub struct CastTxSender<N, P> {
    provider: P,
    _phantom: PhantomData<N>,
}

impl<N: Network, P: Provider<N>> CastTxSender<N, P>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
    N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
{
    /// Creates a new Cast instance responsible for sending transactions.
    pub const fn new(provider: P) -> Self {
        Self { provider, _phantom: PhantomData }
    }

    /// Sends a transaction and waits for receipt synchronously
    pub async fn send_sync(&self, tx: N::TransactionRequest) -> Result<(B256, String)> {
        let receipt = self.provider.send_transaction_sync(tx).await?;
        self.finish(receipt, None).await
    }

    /// Sends a transaction and returns the pending transaction handle.
    pub async fn send(&self, tx: N::TransactionRequest) -> Result<PendingTransactionBuilder<N>> {
        Ok(self.provider.send_transaction(tx).await?)
    }

    /// Sends a raw RLP-encoded transaction and waits for its receipt synchronously.
    pub async fn send_raw_sync(&self, raw_tx: &[u8]) -> Result<(B256, String)> {
        let receipt = self.provider.send_raw_transaction_sync(raw_tx).await?;
        self.finish(receipt, None).await
    }

    /// Prints the transaction hash (if async) or waits for the receipt and prints it.
    ///
    /// This is the shared "output" path used by both the normal send flow and the browser wallet
    /// flow (which sends the transaction out-of-band and only has a tx hash).
    pub async fn print_tx_result(
        &self,
        tx_hash: B256,
        cast_async: bool,
        confs: u64,
        timeout: u64,
    ) -> Result<()> {
        if cast_async {
            sh_println!("{tx_hash:#x}")?;
        } else {
            let receipt =
                self.receipt(format!("{tx_hash:#x}"), None, confs, Some(timeout), false).await?;
            sh_println!("{receipt}")?;
        }
        Ok(())
    }

    /// Fetches the receipt of `tx_hash`, polling for it unless `cast_async` is set, and formats
    /// it (or the requested `field`).
    pub async fn receipt(
        &self,
        tx_hash: String,
        field: Option<String>,
        confs: u64,
        timeout: Option<u64>,
        cast_async: bool,
    ) -> Result<String> {
        let tx_hash = TxHash::from_str(&tx_hash).wrap_err("invalid tx hash")?;
        let receipt = match self.provider.get_transaction_receipt(tx_hash).await? {
            Some(receipt) => receipt,
            None if cast_async => eyre::bail!("tx not found: {tx_hash:?}"),
            None => {
                PendingTransactionBuilder::<N>::new(self.provider.root().clone(), tx_hash)
                    .with_required_confirmations(confs)
                    .with_timeout(timeout.map(Duration::from_secs))
                    .get_receipt()
                    .await?
            }
        };
        Ok(self.finish(receipt, field).await?.1)
    }

    /// Attaches the revert reason (best effort) and formats the receipt.
    async fn finish(
        &self,
        receipt: N::ReceiptResponse,
        field: Option<String>,
    ) -> Result<(B256, String)> {
        let mut receipt = TransactionReceiptWithRevertReason::<N> { receipt, revert_reason: None };
        let tx_hash = receipt.receipt.transaction_hash();
        let _ = receipt.update_revert_reason(&self.provider).await;
        let formatted = if let Some(field) = field {
            get_pretty_receipt_w_reason_attr(&receipt, &field)
                .ok_or_else(|| eyre::eyre!("invalid receipt field: {field}"))?
        } else if shell::is_json() {
            // to_value first to sort json object keys
            serde_json::to_value(&receipt)?.to_string()
        } else {
            receipt.pretty()
        };
        Ok((tx_hash, formatted))
    }
}

/// Builder type constructing generic TransactionRequest from cast send/mktx inputs.
///
/// It is implemented as a stateful builder with expected state transition of [InitState] ->
/// [ToState] -> [InputState].
#[derive(Debug)]
pub struct CastTxBuilder<N: Network, P, S> {
    inner: TxBuilderInner<N, P>,
    state: S,
}

/// The state-independent part of [`CastTxBuilder`].
#[derive(Debug)]
struct TxBuilderInner<N: Network, P> {
    provider: P,
    tx: N::TransactionRequest,
    /// Whether the transaction should be sent as a legacy transaction.
    legacy: bool,
    blob: bool,
    /// Whether the blob transaction should use EIP-4844 (legacy) format instead of EIP-7594.
    eip4844: bool,
    /// Whether to fill gas, fees and nonce. Set to `false` for read-only calls
    /// (eth_call, eth_estimateGas, eth_createAccessList).
    fill: bool,
    /// Whether the filled transaction will be submitted through a browser wallet.
    browser: bool,
    /// The preset used when estimating EIP-1559 fees.
    eip1559_fee_estimate: Eip1559FeeEstimatePreset,
    /// Optional percentage applied to provider gas estimates.
    gas_estimate_multiplier: Option<u64>,
    auth: Vec<CliAuthorizationList>,
    chain: Chain,
    etherscan_api_key: Option<String>,
    etherscan_api_url: Option<String>,
    access_list: Option<Option<AccessList>>,
}

impl<N: Network, P, S> CastTxBuilder<N, P, S> {
    /// Returns the resolved chain for this builder.
    pub const fn chain(&self) -> Chain {
        self.inner.chain
    }

    /// Returns the Etherscan API key and URL resolved for this builder's chain.
    pub(crate) fn etherscan_api(&self) -> (Option<&str>, Option<&str>) {
        (self.inner.etherscan_api_key.as_deref(), self.inner.etherscan_api_url.as_deref())
    }

    /// Returns the transaction request being built.
    pub(crate) const fn tx_mut(&mut self) -> &mut N::TransactionRequest {
        &mut self.inner.tx
    }

    /// Marks this transaction as destined for browser wallet submission.
    pub const fn with_browser_wallet(mut self) -> Self {
        self.inner.browser = true;
        self
    }

    /// Applies a percentage multiplier to provider gas estimates.
    pub const fn with_gas_estimate_multiplier(mut self, multiplier: Option<u64>) -> Self {
        self.inner.gas_estimate_multiplier = multiplier;
        self
    }

    /// Returns whether this builder contains any EIP-7702 authorizations.
    pub(crate) const fn has_auth(&self) -> bool {
        !self.inner.auth.is_empty()
    }

    /// Validates that the configured sender can resolve all EIP-7702 authorizations.
    pub(crate) fn validate_auth(&self, sender: &SenderKind<'_>) -> Result<()> {
        validate_authorizations(&self.inner.auth, sender)
    }

    /// Returns whether building this request will disclose an EIP-7702 authorization to an RPC
    /// endpoint.
    pub(crate) fn will_disclose_auth_during_build(&self) -> bool {
        // Generating an access list or estimating gas sends the authorization-bearing request
        // to the RPC.
        self.has_auth()
            && (matches!(self.inner.access_list, Some(None))
                || (self.inner.fill && self.inner.tx.gas_limit().is_none()))
    }

    fn with_state<S2>(self, state: S2) -> CastTxBuilder<N, P, S2> {
        CastTxBuilder { inner: self.inner, state }
    }
}

impl<N: Network, P: Provider<N>> CastTxBuilder<N, P, InitState>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    /// Creates a new instance of [CastTxBuilder] filling transaction with fields present in
    /// provided [TransactionOpts].
    pub async fn new(provider: P, tx_opts: TransactionOpts, config: &Config) -> Result<Self> {
        let mut tx = N::TransactionRequest::default();

        let chain = utils::get_chain(config.chain, &provider).await?;
        let etherscan_config = config.get_etherscan_config_with_chain(Some(chain)).ok().flatten();
        let etherscan_api_key = etherscan_config.as_ref().map(|c| c.key.clone());
        let etherscan_api_url = etherscan_config.map(|c| c.api_url);
        // mark it as legacy if requested or the chain is legacy and no 7702 is provided.
        let legacy = tx_opts.legacy || (chain.is_legacy() && tx_opts.auth.is_empty());

        tx_opts.apply::<N>(&mut tx, legacy);

        Ok(Self {
            inner: TxBuilderInner {
                provider,
                tx,
                legacy,
                blob: tx_opts.blob,
                eip4844: tx_opts.eip4844,
                fill: true,
                browser: false,
                eip1559_fee_estimate: config.eip1559_fee_estimate,
                gas_estimate_multiplier: None,
                chain,
                etherscan_api_key,
                etherscan_api_url,
                auth: tx_opts.auth,
                access_list: tx_opts.access_list,
            },
            state: InitState,
        })
    }

    /// Sets [TxKind] for this builder and changes state to [ToState].
    pub async fn with_to(self, to: Option<NameOrAddress>) -> Result<CastTxBuilder<N, P, ToState>> {
        let to = match to {
            Some(to) => Some(to.resolve(&self.inner.provider).await?),
            None => None,
        };
        Ok(self.with_state(ToState { to }))
    }
}

impl<N: Network, P: Provider<N>> CastTxBuilder<N, P, ToState>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    /// Accepts user-provided code, sig and args params and constructs calldata for the transaction.
    /// If code is present, input will be set to code + encoded constructor arguments. If no code is
    /// present, input is set to just provided arguments.
    pub async fn with_code_sig_and_args(
        self,
        code: Option<String>,
        sig: Option<String>,
        args: Vec<String>,
    ) -> Result<CastTxBuilder<N, P, InputState>> {
        let to = self.state.to;
        let (mut args, func) = if let Some(sig) = sig {
            let (key, url) = self.etherscan_api();
            parse_function_args(&sig, args, to, self.chain(), &self.inner.provider, key, url)
                .await?
        } else {
            (Vec::new(), None)
        };

        let input = if let Some(code) = &code {
            let mut code = hex::decode(code)?;
            code.append(&mut args);
            code
        } else {
            args
        };

        // We only allow user to omit the recipient address if transaction is an EIP-7702 tx
        // without a value.
        if to.is_none()
            && code.is_none()
            && (!self.has_auth() || self.inner.tx.value().is_some_and(|v| !v.is_zero()))
        {
            eyre::bail!("Must specify a recipient address or contract code to deploy");
        }

        Ok(self.with_state(InputState { kind: to.into(), input, func }))
    }
}

impl<N: Network, P: Provider<N>> CastTxBuilder<N, P, InputState>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    /// Builds the TransactionRequest. Fills gas, fees and nonce unless [`raw`](Self::raw) was
    /// called.
    pub async fn build(
        self,
        sender: impl Into<SenderKind<'_>>,
    ) -> Result<(N::TransactionRequest, Option<Function>)> {
        self.inner.build(self.state, &sender.into(), None).await
    }

    /// Builds a transaction that will be signed by a Tempo access key.
    ///
    /// The access-key id is set before gas estimation. If the access key needs on-chain
    /// provisioning, its authorization is embedded before access-list/gas estimation and before
    /// any sponsor digest can be computed.
    pub async fn build_with_tempo_wallet(
        self,
        wallet: &TempoAccountsWallet,
    ) -> Result<(N::TransactionRequest, Option<Function>, TempoAccountsWallet)> {
        let mut prepared = wallet.clone();
        let (tx, func) =
            self.inner.build(self.state, &wallet.account().into(), Some(&mut prepared)).await?;
        Ok((tx, func, prepared))
    }

    /// Populates the blob sidecar for the transaction if any blob data was provided.
    pub fn with_blob_data(mut self, blob_data: Option<Vec<u8>>) -> Result<Self> {
        let Some(blob_data) = blob_data else { return Ok(self) };

        let mut coder = SidecarBuilder::<SimpleCoder>::default();
        coder.ingest(&blob_data);
        if self.inner.eip4844 {
            self.inner.tx.set_blob_sidecar_4844(coder.build_4844()?);
        } else {
            self.inner.tx.set_blob_sidecar_7594(coder.build_7594()?);
        }
        Ok(self)
    }

    /// Skips gas, fee and nonce filling. Use for read-only calls
    /// (eth_call, eth_estimateGas, eth_createAccessList).
    pub const fn raw(mut self) -> Self {
        self.inner.fill = false;
        self
    }
}

impl<N: Network, P: Provider<N>> TxBuilderInner<N, P>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    async fn build(
        mut self,
        state: InputState,
        sender: &SenderKind<'_>,
        tempo_wallet: Option<&mut TempoAccountsWallet>,
    ) -> Result<(N::TransactionRequest, Option<Function>)> {
        let fill = self.fill;
        let from = sender.address();

        self.tx.set_kind(state.kind);
        // We set both fields to the same value because some nodes only accept the legacy
        // `data` field: https://github.com/foundry-rs/foundry/issues/7764#issuecomment-2210453249
        self.tx.set_input_kind(state.input, TransactionInputKind::Both);
        if !from.is_zero() {
            self.tx.set_from(from);
        }
        self.tx.set_chain_id(self.chain.id());
        // For batch transactions with calls, clear `to` and `value` so the node correctly
        // identifies this as an AA batch transaction. The `calls` field determines the actual
        // targets. If `to` is set, `build_aa()` would add a spurious extra call.
        self.tx.clear_batch_to();

        // Read-only calls do not need a nonce unless it is required to sign an authorization.
        // Avoid an otherwise unused `eth_getTransactionCount` request for raw transactions.
        let resolve_in_parallel =
            fill && self.auth.is_empty() && tempo_wallet.is_none() && !self.chain.is_tempo();
        let tx_nonce = if resolve_in_parallel {
            let fees_are_complete = if self.legacy {
                self.tx.gas_price().is_some()
            } else {
                matches!(
                    (self.tx.max_fee_per_gas(), self.tx.max_priority_fee_per_gas()),
                    (Some(max_fee), Some(priority_fee)) if priority_fee <= max_fee
                )
            } && (!self.blob || self.tx.max_fee_per_blob_gas().is_some());
            let gas_request =
                (fees_are_complete && self.access_list.is_none() && self.tx.gas_limit().is_none())
                    .then(|| self.tx.clone());
            let (blob, legacy, browser, eip1559_fee_estimate, gas_estimate_multiplier) = (
                self.blob,
                self.legacy,
                self.browser,
                self.eip1559_fee_estimate,
                self.gas_estimate_multiplier,
            );
            let Self { provider, tx, .. } = &mut self;
            let provider = &*provider;
            let (tx_nonce, (), gas_limit) = tokio::try_join!(
                Self::resolve_nonce(provider, from, tx.nonce()),
                Self::fill_fees(provider, tx, blob, legacy, browser, eip1559_fee_estimate),
                async {
                    match gas_request {
                        Some(request) => {
                            Self::estimate_gas(provider, request, gas_estimate_multiplier)
                                .await
                                .map(Some)
                        }
                        None => Ok(None),
                    }
                },
            )?;
            if let Some(gas_limit) = gas_limit {
                self.tx.set_gas_limit(gas_limit);
            }
            Some(tx_nonce)
        } else if fill || !self.auth.is_empty() {
            Some(Self::resolve_nonce(&self.provider, from, self.tx.nonce()).await?)
        } else {
            None
        };
        if let Some(tx_nonce) = tx_nonce {
            if fill {
                self.tx.set_nonce(tx_nonce);
            }
            self.resolve_auth(sender, tx_nonce).await?;
        }
        if let Some(wallet) = tempo_wallet {
            *wallet = self.tx.prepare_with_tempo_wallet(&self.provider, wallet).await?;
        }
        if fill && !resolve_in_parallel {
            Self::fill_fees(
                &self.provider,
                &mut self.tx,
                self.blob,
                self.legacy,
                self.browser,
                self.eip1559_fee_estimate,
            )
            .await?;
        }
        // Fetch the access list from the provider if `--access-list` was passed without a value.
        let access_list = match self.access_list.take() {
            None => None,
            Some(None) => Some(self.provider.create_access_list(&self.tx).await?.access_list),
            Some(Some(access_list)) => Some(access_list),
        };
        if let Some(access_list) = access_list {
            self.tx.set_access_list(access_list);
        }
        if fill && self.tx.gas_limit().is_none() {
            let request = if self.browser && self.chain.is_tempo() {
                self.tx.browser_wallet_gas_estimation_request()
            } else {
                self.tx.clone()
            };
            let estimated =
                Self::estimate_gas(&self.provider, request, self.gas_estimate_multiplier).await?;
            self.tx.set_gas_limit(estimated);
        }

        Ok((self.tx, state.func))
    }

    /// Resolves the transaction nonce. Returns the existing nonce or fetches one from the provider.
    async fn resolve_nonce(provider: &P, from: Address, nonce: Option<u64>) -> Result<u64> {
        match nonce {
            Some(nonce) => Ok(nonce),
            None => Ok(provider.get_transaction_count(from).await?),
        }
    }

    /// Parses the passed --auth values and sets the authorization list on the transaction.
    ///
    /// If a signer is available in `sender`, address-based auths will be signed.
    /// If no signer is available, all auths must be pre-signed.
    async fn resolve_auth(&mut self, sender: &SenderKind<'_>, tx_nonce: u64) -> Result<()> {
        if self.auth.is_empty() {
            return Ok(());
        }
        validate_authorizations(&self.auth, sender)?;

        let mut signed_auths = Vec::with_capacity(self.auth.len());
        for auth in std::mem::take(&mut self.auth) {
            signed_auths.push(match auth {
                CliAuthorizationList::Address(address) => {
                    let auth = Authorization {
                        chain_id: U256::from(self.chain.id()),
                        nonce: tx_nonce + 1,
                        address,
                    };
                    let signer =
                        sender.as_signer().expect("address-based authorization requires a signer");
                    let signature = signer.sign_hash(&auth.signature_hash()).await?;
                    auth.into_signed(signature)
                }
                CliAuthorizationList::Signed(auth) => auth,
            });
        }
        self.tx.set_authorization_list(signed_auths);
        Ok(())
    }

    /// Fills gas price, EIP-1559 fees, and blob fees from the provider.
    ///
    /// Only fills values that haven't been explicitly set by the user.
    async fn fill_fees(
        provider: &P,
        tx: &mut N::TransactionRequest,
        blob: bool,
        legacy: bool,
        browser: bool,
        eip1559_fee_estimate: Eip1559FeeEstimatePreset,
    ) -> Result<()> {
        if blob && tx.max_fee_per_blob_gas().is_none() {
            tx.set_max_fee_per_blob_gas(provider.get_blob_base_fee().await?)
        }
        fill_transaction_gas_fees(provider, tx, legacy, browser, eip1559_fee_estimate).await
    }

    /// Estimate tx gas from provider call. Tries to decode custom error if execution reverted.
    async fn estimate_gas(
        provider: &P,
        request: N::TransactionRequest,
        multiplier: Option<u64>,
    ) -> Result<u64> {
        let err = match provider.estimate_gas(request).await {
            Ok(estimated) => return apply_gas_estimate_multiplier(estimated, multiplier),
            Err(err) => err,
        };
        // If execution reverted with code 3 during provider gas estimation then try to decode
        // custom errors and append it to the error message.
        if let TransportError::ErrorResp(payload) = &err
            && payload.code == 3
            && let Some(data) = &payload.data
            && let Ok(data) = serde_json::from_str::<Bytes>(data.get())
            && let Ok(Some(decoded_error)) = decode_custom_error(&data).await
        {
            eyre::bail!("Failed to estimate gas: {err}: {decoded_error}");
        }
        eyre::bail!("Failed to estimate gas: {err}");
    }
}

/// Fills gas price or EIP-1559 fee fields from the provider and validates the final pair.
pub(crate) async fn fill_transaction_gas_fees<N: Network, P: Provider<N>>(
    provider: &P,
    tx: &mut N::TransactionRequest,
    legacy: bool,
    browser: bool,
    eip1559_fee_estimate: Eip1559FeeEstimatePreset,
) -> Result<()>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    if legacy {
        if tx.gas_price().is_none() {
            tx.set_gas_price(provider.get_gas_price().await?);
        }
        return Ok(());
    }

    if tx.max_fee_per_gas().is_none() || tx.max_priority_fee_per_gas().is_none() {
        let estimate = estimate_eip1559_fees(provider, eip1559_fee_estimate).await?;

        // Only honor the browser-suggested tip when the user has not pinned a
        // priority fee; `resolve_broadcast_eip1559_fees` ignores a lower tip.
        let browser_suggested_tip = if browser && tx.max_priority_fee_per_gas().is_none() {
            provider.get_max_priority_fee_per_gas().await.ok()
        } else {
            None
        };

        // User `--gas-price`/`--priority-gas-price` overrides are already applied
        // to `tx`; pass `None` so they are not double-applied here.
        let estimate = resolve_broadcast_eip1559_fees(estimate, None, None, browser_suggested_tip)?;

        if tx.max_fee_per_gas().is_none() {
            tx.set_max_fee_per_gas(estimate.max_fee_per_gas);
        }

        if tx.max_priority_fee_per_gas().is_none() {
            tx.set_max_priority_fee_per_gas(estimate.max_priority_fee_per_gas);
        }
    }

    if let (Some(max_fee), Some(priority)) = (tx.max_fee_per_gas(), tx.max_priority_fee_per_gas()) {
        eyre::ensure!(
            priority <= max_fee,
            "max priority fee per gas ({priority}) cannot exceed max fee per gas ({max_fee})"
        );
    }

    Ok(())
}

/// Tries to decode a custom error name and inputs from revert data.
pub(crate) async fn decode_custom_error(data: &[u8]) -> Result<Option<String>> {
    let Some(selector) = data.get(..4) else { return Ok(None) };
    let Some(known_error) =
        SignaturesIdentifier::new(false)?.identify_error(selector.try_into().unwrap()).await
    else {
        return Ok(None);
    };
    let mut decoded_error = known_error.name.clone();
    if !known_error.inputs.is_empty()
        && let Ok(error) = known_error.decode_error(data)
    {
        write!(decoded_error, "({})", format_tokens(&error.body).format(", "))?;
    }
    Ok(Some(decoded_error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_json_rpc::{RequestPacket, ResponsePacket};
    use alloy_network::Ethereum;
    use alloy_provider::{ProviderBuilder, mock::Asserter};
    use alloy_rpc_client::RpcClient;
    use alloy_transport::{TransportFut, mock::MockTransport};
    use clap::Parser;
    use std::{
        sync::{Arc, Mutex},
        task::{Context, Poll},
    };
    use tokio::{sync::Barrier, time::timeout};
    use tower::Service;

    #[derive(Clone)]
    struct BarrierTransport {
        inner: MockTransport,
        barrier: Arc<Barrier>,
        fill_methods: Arc<Mutex<Vec<String>>>,
    }

    impl Service<RequestPacket> for BarrierTransport {
        type Response = ResponsePacket;
        type Error = TransportError;
        type Future = TransportFut<'static>;

        fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.inner.poll_ready(cx)
        }

        fn call(&mut self, req: RequestPacket) -> Self::Future {
            let fill_method = match &req {
                RequestPacket::Single(req)
                    if matches!(
                        req.method(),
                        "eth_getTransactionCount" | "eth_gasPrice" | "eth_estimateGas"
                    ) =>
                {
                    Some(req.method().to_string())
                }
                _ => None,
            };
            let Some(fill_method) = fill_method else {
                return self.inner.call(req);
            };
            self.fill_methods.lock().unwrap().push(fill_method);

            let barrier = self.barrier.clone();
            let mut inner = self.inner.clone();
            Box::pin(async move {
                barrier.wait().await;
                inner.call(req).await
            })
        }
    }

    const TO: Address = Address::repeat_byte(0x11);

    /// Builds a transaction to [`TO`] from the given `cast send` style arguments.
    async fn builder<P: Provider<Ethereum>>(
        provider: P,
        args: &[&str],
    ) -> CastTxBuilder<Ethereum, P, InputState> {
        let config = Config { chain: Some(Chain::mainnet()), ..Default::default() };
        let opts = TransactionOpts::parse_from([&["test"], args].concat());
        CastTxBuilder::new(provider, opts, &config)
            .await
            .unwrap()
            .with_to(Some(TO.into()))
            .await
            .unwrap()
            .with_code_sig_and_args(None, None, Vec::new())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn filled_build_applies_multiplier_to_concurrent_gas_estimate() {
        let asserter = Asserter::new();
        for _ in 0..2 {
            asserter.push_success(&U64::from(100));
        }
        let fill_methods = Arc::new(Mutex::new(Vec::new()));
        let transport = BarrierTransport {
            inner: MockTransport::new(asserter),
            barrier: Arc::new(Barrier::new(2)),
            fill_methods: fill_methods.clone(),
        };
        let provider = ProviderBuilder::new_with_network::<Ethereum>()
            .connect_client(RpcClient::new(transport, true));

        let builder = builder(&provider, &["--legacy", "--gas-price", "1"])
            .await
            .with_gas_estimate_multiplier(Some(150));
        let (tx, _) = timeout(Duration::from_secs(1), builder.build(Address::repeat_byte(0x22)))
            .await
            .expect("nonce and gas requests were not in flight together")
            .unwrap();

        assert_eq!(tx.nonce, Some(100));
        assert_eq!(tx.gas_price, Some(1));
        assert_eq!(tx.gas, Some(150));
        let mut fill_methods = fill_methods.lock().unwrap().clone();
        fill_methods.sort();
        assert_eq!(fill_methods, ["eth_estimateGas", "eth_getTransactionCount"]);
    }

    #[tokio::test]
    async fn raw_build_skips_nonce_request() {
        // No responses are queued, so any RPC request would fail this test. In particular, this
        // guards against restoring the unused `eth_getTransactionCount` request.
        let provider =
            ProviderBuilder::new_with_network::<Ethereum>().connect_mocked_client(Asserter::new());
        builder(&provider, &[]).await.raw().build(Address::repeat_byte(0x22)).await.unwrap();
    }

    #[tokio::test]
    async fn detects_auth_rpc_disclosure() {
        let provider =
            ProviderBuilder::new_with_network::<Ethereum>().connect_mocked_client(Asserter::new());
        let auth = TO.to_string();

        let no_auth = builder(&provider, &["--gas-limit", "21000"]).await.raw();
        assert!(!no_auth.has_auth());
        assert!(!no_auth.will_disclose_auth_during_build());

        let rpc_call = builder(&provider, &["--auth", &auth, "--gas-limit", "21000"]).await.raw();
        assert!(rpc_call.has_auth());
        assert!(!rpc_call.will_disclose_auth_during_build());

        let generated_access_list = builder(&provider, &["--auth", &auth, "--access-list"]).await;
        assert!(generated_access_list.raw().will_disclose_auth_during_build());

        let explicit_access_list =
            builder(&provider, &["--auth", &auth, "--access-list", "[]"]).await;
        assert!(!explicit_access_list.raw().will_disclose_auth_during_build());

        let estimated_gas = builder(&provider, &["--auth", &auth]).await;
        assert!(estimated_gas.will_disclose_auth_during_build());
    }
}
