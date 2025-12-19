use crate::traces::identifier::SignaturesIdentifier;
use alloy_consensus::{SidecarBuilder, SignableTransaction, SimpleCoder};
use alloy_dyn_abi::ErrorExt;
use alloy_ens::NameOrAddress;
use alloy_json_abi::Function;
use alloy_network::{
    AnyNetwork, AnyTypedTransaction, TransactionBuilder, TransactionBuilder4844,
    TransactionBuilder7702,
};
use alloy_primitives::{Address, Bytes, TxHash, TxKind, U256, hex};
use alloy_provider::{PendingTransactionBuilder, Provider};
use alloy_rpc_types::{AccessList, Authorization, TransactionInputKind, TransactionRequest};
use alloy_serde::WithOtherFields;
use alloy_signer::Signer;
use alloy_transport::TransportError;
use clap::Args;
use eyre::{Result, WrapErr};
use foundry_cli::{
    opts::{CliAuthorizationList, EthereumOpts, TransactionOpts},
    utils::{self, LoadConfig, get_provider_builder, parse_function_args},
};
use foundry_common::{
    TransactionReceiptWithRevertReason, fmt::*, get_pretty_tx_receipt_attr,
    provider::RetryProviderWithSigner, shell,
};
use foundry_config::{Chain, Config};
use foundry_wallets::{WalletOpts, WalletSigner};
use itertools::Itertools;
use serde_json::value::RawValue;
use std::{fmt::Write, str::FromStr, time::Duration};

#[derive(Debug, Clone, Args)]
pub struct SendTxOpts {
    /// Only print the transaction hash and exit immediately.
    #[arg(id = "async", long = "async", alias = "cast-async", env = "CAST_ASYNC")]
    pub cast_async: bool,

    /// Wait for transaction receipt synchronously instead of polling.
    /// Note: uses `eth_sendTransactionSync` which may not be supported by all clients.
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
    /// This function prefers the `from` field and may return a different address from the
    /// configured signer
    /// If from is specified, returns it
    /// If from is not specified, but there is a signer configured, returns the signer's address
    /// If from is not specified and there is no signer configured, returns zero address
    pub async fn from_wallet_opts(opts: WalletOpts) -> Result<Self> {
        if let Some(from) = opts.from {
            Ok(from.into())
        } else if let Ok(signer) = opts.signer().await {
            Ok(Self::OwnedSigner(Box::new(signer)))
        } else {
            Ok(Address::ZERO.into())
        }
    }

    /// Returns the signer if available.
    pub fn as_signer(&self) -> Option<&WalletSigner> {
        match self {
            Self::Signer(signer) => Some(signer),
            Self::OwnedSigner(signer) => Some(signer.as_ref()),
            _ => None,
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
            )
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

pub struct CastTxSender<P> {
    provider: P,
}

impl<P: Provider<AnyNetwork>> CastTxSender<P> {
    /// Creates a new Cast instance responsible for sending transactions.
    pub fn new(provider: P) -> Self {
        Self { provider }
    }

    /// Sends a transaction and waits for receipt synchronously
    pub async fn send_sync(&self, tx: WithOtherFields<TransactionRequest>) -> Result<String> {
        let mut receipt: TransactionReceiptWithRevertReason =
            self.provider.send_transaction_sync(tx).await?.into();

        // Allow to fail silently
        let _ = receipt.update_revert_reason(&self.provider).await;

        self.format_receipt(receipt, None)
    }

    /// Sends a transaction to the specified address
    ///
    /// # Example
    ///
    /// ```
    /// use cast::tx::CastTxSender;
    /// use alloy_primitives::{Address, U256, Bytes};
    /// use alloy_serde::WithOtherFields;
    /// use alloy_rpc_types::{TransactionRequest};
    /// use alloy_provider::{RootProvider, ProviderBuilder, network::AnyNetwork};
    /// use std::str::FromStr;
    /// use alloy_sol_types::{sol, SolCall};    ///
    ///
    /// sol!(
    ///     function greet(string greeting) public;
    /// );
    ///
    /// # async fn foo() -> eyre::Result<()> {
    /// let provider = ProviderBuilder::<_,_, AnyNetwork>::default().connect("http://localhost:8545").await?;;
    /// let from = Address::from_str("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045")?;
    /// let to = Address::from_str("0xB3C95ff08316fb2F2e3E52Ee82F8e7b605Aa1304")?;
    /// let greeting = greetCall { greeting: "hello".to_string() }.abi_encode();
    /// let bytes = Bytes::from_iter(greeting.iter());
    /// let gas = U256::from_str("200000").unwrap();
    /// let value = U256::from_str("1").unwrap();
    /// let nonce = U256::from_str("1").unwrap();
    /// let tx = TransactionRequest::default().to(to).input(bytes.into()).from(from);
    /// let tx = WithOtherFields::new(tx);
    /// let cast = CastTxSender::new(provider);
    /// let data = cast.send(tx).await?;
    /// println!("{:#?}", data);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send(
        &self,
        tx: WithOtherFields<TransactionRequest>,
    ) -> Result<PendingTransactionBuilder<AnyNetwork>> {
        let res = self.provider.send_transaction(tx).await?;

        Ok(res)
    }

    /// # Example
    ///
    /// ```
    /// use alloy_provider::{ProviderBuilder, RootProvider, network::AnyNetwork};
    /// use cast::tx::CastTxSender;
    ///
    /// async fn foo() -> eyre::Result<()> {
    /// let provider =
    ///     ProviderBuilder::<_, _, AnyNetwork>::default().connect("http://localhost:8545").await?;
    /// let cast = CastTxSender::new(provider);
    /// let tx_hash = "0xf8d1713ea15a81482958fb7ddf884baee8d3bcc478c5f2f604e008dc788ee4fc";
    /// let receipt = cast.receipt(tx_hash.to_string(), None, 1, None, false).await?;
    /// println!("{}", receipt);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn receipt(
        &self,
        tx_hash: String,
        field: Option<String>,
        confs: u64,
        timeout: Option<u64>,
        cast_async: bool,
    ) -> Result<String> {
        let tx_hash = TxHash::from_str(&tx_hash).wrap_err("invalid tx hash")?;

        let mut receipt: TransactionReceiptWithRevertReason =
            match self.provider.get_transaction_receipt(tx_hash).await? {
                Some(r) => r,
                None => {
                    // if the async flag is provided, immediately exit if no tx is found, otherwise
                    // try to poll for it
                    if cast_async {
                        eyre::bail!("tx not found: {:?}", tx_hash)
                    } else {
                        PendingTransactionBuilder::new(self.provider.root().clone(), tx_hash)
                            .with_required_confirmations(confs)
                            .with_timeout(timeout.map(Duration::from_secs))
                            .get_receipt()
                            .await?
                    }
                }
            }
            .into();

        // Allow to fail silently
        let _ = receipt.update_revert_reason(&self.provider).await;

        self.format_receipt(receipt, field)
    }

    /// Helper method to format transaction receipts consistently
    fn format_receipt(
        &self,
        receipt: TransactionReceiptWithRevertReason,
        field: Option<String>,
    ) -> Result<String> {
        Ok(if let Some(ref field) = field {
            get_pretty_tx_receipt_attr(&receipt, field)
                .ok_or_else(|| eyre::eyre!("invalid receipt field: {}", field))?
        } else if shell::is_json() {
            // to_value first to sort json object keys
            serde_json::to_value(&receipt)?.to_string()
        } else {
            receipt.pretty()
        })
    }
}

/// Builder type constructing [TransactionRequest] from cast send/mktx inputs.
///
/// It is implemented as a stateful builder with expected state transition of [InitState] ->
/// [ToState] -> [InputState].
#[derive(Debug)]
pub struct CastTxBuilder<P, S> {
    provider: P,
    tx: WithOtherFields<TransactionRequest>,
    /// Whether the transaction should be sent as a legacy transaction.
    legacy: bool,
    blob: bool,
    auth: Vec<CliAuthorizationList>,
    chain: Chain,
    etherscan_api_key: Option<String>,
    access_list: Option<Option<AccessList>>,
    state: S,
}

impl<P: Provider<AnyNetwork>> CastTxBuilder<P, InitState> {
    /// Creates a new instance of [CastTxBuilder] filling transaction with fields present in
    /// provided [TransactionOpts].
    pub async fn new(provider: P, tx_opts: TransactionOpts, config: &Config) -> Result<Self> {
        let mut tx = WithOtherFields::<TransactionRequest>::default();

        let chain = utils::get_chain(config.chain, &provider).await?;
        let etherscan_api_key = config.get_etherscan_api_key(Some(chain));
        // mark it as legacy if requested or the chain is legacy and no 7702 is provided.
        let legacy = tx_opts.legacy || (chain.is_legacy() && tx_opts.auth.is_empty());

        if let Some(gas_limit) = tx_opts.gas_limit {
            tx.set_gas_limit(gas_limit.to());
        }

        if let Some(value) = tx_opts.value {
            tx.set_value(value);
        }

        if let Some(gas_price) = tx_opts.gas_price {
            if legacy {
                tx.set_gas_price(gas_price.to());
            } else {
                tx.set_max_fee_per_gas(gas_price.to());
            }
        }

        if !legacy && let Some(priority_fee) = tx_opts.priority_gas_price {
            tx.set_max_priority_fee_per_gas(priority_fee.to());
        }

        if let Some(max_blob_fee) = tx_opts.blob_gas_price {
            tx.set_max_fee_per_blob_gas(max_blob_fee.to())
        }

        if let Some(nonce) = tx_opts.nonce {
            tx.set_nonce(nonce.to());
        }

        Ok(Self {
            provider,
            tx,
            legacy,
            blob: tx_opts.blob,
            chain,
            etherscan_api_key,
            auth: tx_opts.auth,
            access_list: tx_opts.access_list,
            state: InitState,
        })
    }

    /// Sets [TxKind] for this builder and changes state to [ToState].
    pub async fn with_to(self, to: Option<NameOrAddress>) -> Result<CastTxBuilder<P, ToState>> {
        let to = if let Some(to) = to { Some(to.resolve(&self.provider).await?) } else { None };
        Ok(CastTxBuilder {
            provider: self.provider,
            tx: self.tx,
            legacy: self.legacy,
            blob: self.blob,
            chain: self.chain,
            etherscan_api_key: self.etherscan_api_key,
            auth: self.auth,
            access_list: self.access_list,
            state: ToState { to },
        })
    }
}

impl<P: Provider<AnyNetwork>> CastTxBuilder<P, ToState> {
    /// Accepts user-provided code, sig and args params and constructs calldata for the transaction.
    /// If code is present, input will be set to code + encoded constructor arguments. If no code is
    /// present, input is set to just provided arguments.
    pub async fn with_code_sig_and_args(
        self,
        code: Option<String>,
        sig: Option<String>,
        args: Vec<String>,
    ) -> Result<CastTxBuilder<P, InputState>> {
        let (mut args, func) = if let Some(sig) = sig {
            parse_function_args(
                &sig,
                args,
                self.state.to,
                self.chain,
                &self.provider,
                self.etherscan_api_key.as_deref(),
            )
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

        if self.state.to.is_none() && code.is_none() {
            let has_value = self.tx.value.is_some_and(|v| !v.is_zero());
            let has_auth = !self.auth.is_empty();
            // We only allow user to omit the recipient address if transaction is an EIP-7702 tx
            // without a value.
            if !has_auth || has_value {
                eyre::bail!("Must specify a recipient address or contract code to deploy");
            }
        }

        Ok(CastTxBuilder {
            provider: self.provider,
            tx: self.tx,
            legacy: self.legacy,
            blob: self.blob,
            chain: self.chain,
            etherscan_api_key: self.etherscan_api_key,
            auth: self.auth,
            access_list: self.access_list,
            state: InputState { kind: self.state.to.into(), input, func },
        })
    }
}

impl<P: Provider<AnyNetwork>> CastTxBuilder<P, InputState> {
    /// Builds [TransactionRequest] and fills missing fields. Returns a transaction which is ready
    /// to be broadcasted.
    pub async fn build(
        self,
        sender: impl Into<SenderKind<'_>>,
    ) -> Result<(WithOtherFields<TransactionRequest>, Option<Function>)> {
        self._build(sender, true, false).await
    }

    /// Builds [TransactionRequest] without filling missing fields. Used for read-only calls such as
    /// eth_call, eth_estimateGas, etc
    pub async fn build_raw(
        self,
        sender: impl Into<SenderKind<'_>>,
    ) -> Result<(WithOtherFields<TransactionRequest>, Option<Function>)> {
        self._build(sender, false, false).await
    }

    /// Builds an unsigned RLP-encoded raw transaction.
    ///
    /// Returns the hex encoded string representation of the transaction.
    pub async fn build_unsigned_raw(self, from: Address) -> Result<String> {
        let (tx, _) = self._build(SenderKind::Address(from), true, true).await?;
        let tx = tx.build_unsigned()?;
        match tx {
            AnyTypedTransaction::Ethereum(t) => Ok(hex::encode_prefixed(t.encoded_for_signing())),
            _ => eyre::bail!("Cannot generate unsigned transaction for non-Ethereum transactions"),
        }
    }

    async fn _build(
        mut self,
        sender: impl Into<SenderKind<'_>>,
        fill: bool,
        unsigned: bool,
    ) -> Result<(WithOtherFields<TransactionRequest>, Option<Function>)> {
        let sender = sender.into();
        let from = sender.address();

        self.tx.set_kind(self.state.kind);

        // we set both fields to the same value because some nodes only accept the legacy `data` field: <https://github.com/foundry-rs/foundry/issues/7764#issuecomment-2210453249>
        self.tx.set_input_kind(self.state.input.clone(), TransactionInputKind::Both);

        self.tx.set_from(from);
        self.tx.set_chain_id(self.chain.id());

        let tx_nonce = if let Some(nonce) = self.tx.nonce {
            nonce
        } else {
            let nonce = self.provider.get_transaction_count(from).await?;
            if fill {
                self.tx.nonce = Some(nonce);
            }
            nonce
        };

        if !unsigned {
            self.resolve_auth(sender, tx_nonce).await?;
        } else if !self.auth.is_empty() {
            let mut signed_auths = Vec::with_capacity(self.auth.len());
            for auth in std::mem::take(&mut self.auth) {
                let CliAuthorizationList::Signed(signed_auth) = auth else {
                    eyre::bail!(
                        "SignedAuthorization needs to be provided for generating unsigned 7702 txs"
                    )
                };
                signed_auths.push(signed_auth);
            }

            self.tx.set_authorization_list(signed_auths);
        }

        if let Some(access_list) = match self.access_list.take() {
            None => None,
            // --access-list provided with no value, call the provider to create it
            Some(None) => Some(self.provider.create_access_list(&self.tx).await?.access_list),
            // Access list provided as a string, attempt to parse it
            Some(Some(access_list)) => Some(access_list),
        } {
            self.tx.set_access_list(access_list);
        }

        if !fill {
            return Ok((self.tx, self.state.func));
        }

        if self.legacy && self.tx.gas_price.is_none() {
            self.tx.gas_price = Some(self.provider.get_gas_price().await?);
        }

        if self.blob && self.tx.max_fee_per_blob_gas.is_none() {
            self.tx.max_fee_per_blob_gas = Some(self.provider.get_blob_base_fee().await?)
        }

        if !self.legacy
            && (self.tx.max_fee_per_gas.is_none() || self.tx.max_priority_fee_per_gas.is_none())
        {
            let estimate = self.provider.estimate_eip1559_fees().await?;

            if self.tx.max_fee_per_gas.is_none() {
                self.tx.max_fee_per_gas = Some(estimate.max_fee_per_gas);
            }

            if self.tx.max_priority_fee_per_gas.is_none() {
                self.tx.max_priority_fee_per_gas = Some(estimate.max_priority_fee_per_gas);
            }
        }

        if self.tx.gas.is_none() {
            self.estimate_gas().await?;
        }

        Ok((self.tx, self.state.func))
    }

    /// Estimate tx gas from provider call. Tries to decode custom error if execution reverted.
    async fn estimate_gas(&mut self) -> Result<()> {
        match self.provider.estimate_gas(self.tx.clone()).await {
            Ok(estimated) => {
                self.tx.gas = Some(estimated);
                Ok(())
            }
            Err(err) => {
                if let TransportError::ErrorResp(payload) = &err {
                    // If execution reverted with code 3 during provider gas estimation then try
                    // to decode custom errors and append it to the error message.
                    if payload.code == 3
                        && let Some(data) = &payload.data
                        && let Ok(Some(decoded_error)) = decode_execution_revert(data).await
                    {
                        eyre::bail!("Failed to estimate gas: {}: {}", err, decoded_error)
                    }
                }
                eyre::bail!("Failed to estimate gas: {}", err)
            }
        }
    }

    /// Parses the passed --auth values and sets the authorization list on the transaction.
    async fn resolve_auth(&mut self, sender: SenderKind<'_>, tx_nonce: u64) -> Result<()> {
        if self.auth.is_empty() {
            return Ok(());
        }

        let auths = std::mem::take(&mut self.auth);

        // Validate that at most one address-based auth is provided (multiple addresses are
        // almost always unintended).
        let address_auth_count =
            auths.iter().filter(|a| matches!(a, CliAuthorizationList::Address(_))).count();
        if address_auth_count > 1 {
            eyre::bail!(
                "Multiple address-based authorizations provided. Only one address can be specified; \
                use pre-signed authorizations (hex-encoded) for multiple authorizations."
            );
        }

        let mut signed_auths = Vec::with_capacity(auths.len());

        for auth in auths {
            let signed_auth = match auth {
                CliAuthorizationList::Address(address) => {
                    let auth = Authorization {
                        chain_id: U256::from(self.chain.id()),
                        nonce: tx_nonce + 1,
                        address,
                    };

                    let Some(signer) = sender.as_signer() else {
                        eyre::bail!("No signer available to sign authorization");
                    };
                    let signature = signer.sign_hash(&auth.signature_hash()).await?;

                    auth.into_signed(signature)
                }
                CliAuthorizationList::Signed(auth) => auth,
            };
            signed_auths.push(signed_auth);
        }

        self.tx.set_authorization_list(signed_auths);

        Ok(())
    }
}

impl<P, S> CastTxBuilder<P, S>
where
    P: Provider<AnyNetwork>,
{
    /// Populates the blob sidecar for the transaction if any blob data was provided.
    pub fn with_blob_data(mut self, blob_data: Option<Vec<u8>>) -> Result<Self> {
        let Some(blob_data) = blob_data else { return Ok(self) };

        let mut coder = SidecarBuilder::<SimpleCoder>::default();
        coder.ingest(&blob_data);
        let sidecar = coder.build()?;

        self.tx.set_blob_sidecar(sidecar);
        self.tx.populate_blob_hashes();

        Ok(self)
    }
}

/// Helper function that tries to decode custom error name and inputs from error payload data.
async fn decode_execution_revert(data: &RawValue) -> Result<Option<String>> {
    let err_data = serde_json::from_str::<Bytes>(data.get())?;
    let Some(selector) = err_data.get(..4) else { return Ok(None) };
    if let Some(known_error) =
        SignaturesIdentifier::new(false)?.identify_error(selector.try_into().unwrap()).await
    {
        let mut decoded_error = known_error.name.clone();
        if !known_error.inputs.is_empty()
            && let Ok(error) = known_error.decode_error(&err_data)
        {
            write!(decoded_error, "({})", format_tokens(&error.body).format(", "))?;
        }
        return Ok(Some(decoded_error));
    }
    Ok(None)
}

/// Creates a provider with wallet for signing transactions locally.
pub(crate) async fn signing_provider(
    tx_opts: &SendTxOpts,
) -> eyre::Result<RetryProviderWithSigner> {
    let config = tx_opts.eth.load_config()?;
    let signer = tx_opts.eth.wallet.signer().await?;
    let wallet = alloy_network::EthereumWallet::from(signer);
    let provider = get_provider_builder(&config)?.build_with_wallet(wallet)?;
    if let Some(interval) = tx_opts.poll_interval {
        provider.client().set_poll_interval(Duration::from_secs(interval))
    }
    Ok(provider)
}
