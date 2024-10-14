use alloy_consensus::{SidecarBuilder, SimpleCoder};
use alloy_json_abi::Function;
use alloy_network::{
    AnyNetwork, TransactionBuilder, TransactionBuilder4844, TransactionBuilder7702,
};
use alloy_primitives::{hex, Address, Bytes, TxKind, U256};
use alloy_provider::Provider;
use alloy_rlp::Decodable;
use alloy_rpc_types::{AccessList, Authorization, TransactionInput, TransactionRequest};
use alloy_serde::WithOtherFields;
use alloy_signer::Signer;
use alloy_transport::Transport;
use cast::revm::primitives::SignedAuthorization;
use eyre::{Result, WrapErr};
use foundry_cli::{
    opts::TransactionOpts,
    utils::{self, parse_function_args},
};
use foundry_common::ens::NameOrAddress;
use foundry_config::{Chain, Config};
use foundry_wallets::{WalletOpts, WalletSigner};
use serde_json;

/// Different sender kinds used by [`CastTxBuilder`].
pub enum SenderKind<'a> {
    /// An address without signer. Used for read-only calls and transactions sent through unlocked
    /// accounts.
    Address(Address),
    /// A refersnce to a signer.
    Signer(&'a WalletSigner),
    /// An owned signer.
    OwnedSigner(WalletSigner),
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
            Ok(Self::OwnedSigner(signer))
        } else {
            Ok(Address::ZERO.into())
        }
    }

    /// Returns the signer if available.
    pub fn as_signer(&self) -> Option<&WalletSigner> {
        match self {
            Self::Signer(signer) => Some(signer),
            Self::OwnedSigner(signer) => Some(signer),
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
        Self::OwnedSigner(signer)
    }
}

/// Prevents a misconfigured hwlib from sending a transaction that defies user-specified --from
pub fn validate_from_address(
    specified_from: Option<Address>,
    signer_address: Address,
) -> Result<()> {
    if let Some(specified_from) = specified_from {
        if specified_from != signer_address {
            eyre::bail!(
                "\
The specified sender via CLI/env vars does not match the sender configured via
the hardware wallet's HD Path.
Please use the `--hd-path <PATH>` parameter to specify the BIP32 Path which
corresponds to the sender, or let foundry automatically detect it by not specifying any sender address."
            )
        }
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

/// Builder type constructing [TransactionRequest] from cast send/mktx inputs.
///
/// It is implemented as a stateful builder with expected state transition of [InitState] ->
/// [TxKindState] -> [InputState].
#[derive(Debug)]
pub struct CastTxBuilder<T, P, S> {
    provider: P,
    tx: WithOtherFields<TransactionRequest>,
    legacy: bool,
    blob: bool,
    auth: Option<String>,
    chain: Chain,
    etherscan_api_key: Option<String>,
    access_list: Option<Option<String>>,
    state: S,
    _t: std::marker::PhantomData<T>,
}

impl<T, P> CastTxBuilder<T, P, InitState>
where
    P: Provider<T, AnyNetwork>,
    T: Transport + Clone,
{
    /// Creates a new instance of [CastTxBuilder] filling transaction with fields present in
    /// provided [TransactionOpts].
    pub async fn new(provider: P, tx_opts: TransactionOpts, config: &Config) -> Result<Self> {
        let mut tx = WithOtherFields::<TransactionRequest>::default();

        let chain = utils::get_chain(config.chain, &provider).await?;
        let etherscan_api_key = config.get_etherscan_api_key(Some(chain));
        let legacy = tx_opts.legacy || chain.is_legacy();

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

        if !legacy {
            if let Some(priority_fee) = tx_opts.priority_gas_price {
                tx.set_max_priority_fee_per_gas(priority_fee.to());
            }
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
            _t: std::marker::PhantomData,
        })
    }

    /// Sets [TxKind] for this builder and changes state to [TxKindState].
    pub async fn with_to(self, to: Option<NameOrAddress>) -> Result<CastTxBuilder<T, P, ToState>> {
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
            _t: self._t,
        })
    }
}

impl<T, P> CastTxBuilder<T, P, ToState>
where
    P: Provider<T, AnyNetwork>,
    T: Transport + Clone,
{
    /// Accepts user-provided code, sig and args params and constructs calldata for the transaction.
    /// If code is present, input will be set to code + encoded constructor arguments. If no code is
    /// present, input is set to just provided arguments.
    pub async fn with_code_sig_and_args(
        self,
        code: Option<String>,
        sig: Option<String>,
        args: Vec<String>,
    ) -> Result<CastTxBuilder<T, P, InputState>> {
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
            let has_value = self.tx.value.map_or(false, |v| !v.is_zero());
            let has_auth = self.auth.is_some();
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
            _t: self._t,
        })
    }
}

impl<T, P> CastTxBuilder<T, P, InputState>
where
    P: Provider<T, AnyNetwork>,
    T: Transport + Clone,
{
    /// Builds [TransactionRequest] and fiils missing fields. Returns a transaction which is ready
    /// to be broadcasted.
    pub async fn build(
        self,
        sender: impl Into<SenderKind<'_>>,
    ) -> Result<(WithOtherFields<TransactionRequest>, Option<Function>)> {
        self._build(sender, true).await
    }

    /// Builds [TransactionRequest] without filling missing fields. Used for read-only calls such as
    /// eth_call, eth_estimateGas, etc
    pub async fn build_raw(
        self,
        sender: impl Into<SenderKind<'_>>,
    ) -> Result<(WithOtherFields<TransactionRequest>, Option<Function>)> {
        self._build(sender, false).await
    }

    async fn _build(
        mut self,
        sender: impl Into<SenderKind<'_>>,
        fill: bool,
    ) -> Result<(WithOtherFields<TransactionRequest>, Option<Function>)> {
        let sender = sender.into();
        let from = sender.address();

        self.tx.set_kind(self.state.kind);

        // we set both fields to the same value because some nodes only accept the legacy `data` field: <https://github.com/foundry-rs/foundry/issues/7764#issuecomment-2210453249>
        let input = Bytes::copy_from_slice(&self.state.input);
        self.tx.input = TransactionInput { input: Some(input.clone()), data: Some(input) };

        self.tx.set_from(from);
        self.tx.set_chain_id(self.chain.id());

        if !fill {
            return Ok((self.tx, self.state.func));
        }

        if let Some(access_list) = match self.access_list {
            None => None,
            // --access-list provided with no value, call the provider to create it
            Some(None) => Some(self.provider.create_access_list(&self.tx).await?.access_list),
            // Access list provided as a string, attempt to parse it
            Some(Some(ref s)) => Some(
                serde_json::from_str::<AccessList>(s)
                    .map(AccessList::from)
                    .wrap_err("Failed to parse access list from string")?,
            ),
        } {
            self.tx.set_access_list(access_list);
        }

        if self.legacy && self.tx.gas_price.is_none() {
            self.tx.gas_price = Some(self.provider.get_gas_price().await?);
        }

        if self.blob && self.tx.max_fee_per_blob_gas.is_none() {
            self.tx.max_fee_per_blob_gas = Some(self.provider.get_blob_base_fee().await?)
        }

        if !self.legacy &&
            (self.tx.max_fee_per_gas.is_none() || self.tx.max_priority_fee_per_gas.is_none())
        {
            let estimate = self.provider.estimate_eip1559_fees(None).await?;

            if !self.legacy {
                if self.tx.max_fee_per_gas.is_none() {
                    self.tx.max_fee_per_gas = Some(estimate.max_fee_per_gas);
                }

                if self.tx.max_priority_fee_per_gas.is_none() {
                    self.tx.max_priority_fee_per_gas = Some(estimate.max_priority_fee_per_gas);
                }
            }
        }

        let nonce = if let Some(nonce) = self.tx.nonce {
            nonce
        } else {
            let nonce = self.provider.get_transaction_count(from).await?;
            self.tx.nonce = Some(nonce);
            nonce
        };

        self.resolve_auth(sender, nonce).await?;

        if self.tx.gas.is_none() {
            self.tx.gas = Some(self.provider.estimate_gas(&self.tx).await?);
        }

        Ok((self.tx, self.state.func))
    }

    /// Parses the passed --auth value and sets the authorization list on the transaction.
    async fn resolve_auth(&mut self, sender: SenderKind<'_>, nonce: u64) -> Result<()> {
        let Some(auth) = &self.auth else { return Ok(()) };

        let auth = hex::decode(auth)?;
        let auth = if let Ok(address) = Address::try_from(auth.as_slice()) {
            let auth =
                Authorization { chain_id: U256::from(self.chain.id()), nonce: nonce + 1, address };

            let Some(signer) = sender.as_signer() else {
                eyre::bail!("No signer available to sign authorization");
            };
            let signature = signer.sign_hash(&auth.signature_hash()).await?;

            auth.into_signed(signature)
        } else if let Ok(auth) = SignedAuthorization::decode(&mut auth.as_ref()) {
            auth
        } else {
            eyre::bail!("Failed to decode authorization");
        };
        self.tx.set_authorization_list(vec![auth]);

        Ok(())
    }
}

impl<T, P, S> CastTxBuilder<T, P, S>
where
    P: Provider<T, AnyNetwork>,
    T: Transport + Clone,
{
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
