//! Tempo's RPC methods, registered through the public network extension boundary.

use crate::{Tempo, TempoConfigExt, tempo_eth::AnvilTempoEthApi, tempo_storage::TempoStorage};
use alloy_consensus::transaction::SignerRecoverable;
use alloy_eips::{Decodable2718, Encodable2718};
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_rlp::{Encodable as _, Header as RlpHeader, PayloadView};
use alloy_rpc_types_eth::erc4337::TransactionConditional;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use foundry_evm_core::tempo::PATH_USD_ADDRESS;
use jsonrpsee::{
    RpcModule,
    core::{RpcResult, async_trait},
    proc_macros::rpc,
    types::{
        ErrorObjectOwned,
        error::{INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE},
    },
};
use reth_anvil::{
    AnvilApiServer, AnvilComponents, AnvilRpc, AnvilRpcOf, EthExtApiServer, api::PoolRefresh,
    evm::AnvilNextBlockEnv, network::NodeOf, state_dump::StateDump,
};
use reth_ethereum::{
    evm::primitives::ConfigureEvm,
    pool::{CanonicalStateUpdate, PoolUpdateKind, TransactionPool, TransactionPoolExt},
    provider::{BlockNumReader, BlockReader, StateProviderFactory, TransactionVariant},
};
use reth_rpc_eth_api::{
    RpcNodeCore,
    helpers::config::{EthConfigApiServer, EthConfigHandler},
};
use tempo_alloy::rpc::{TempoTransactionReceipt, TempoTransactionRequest};
use tempo_chainspec::TempoChainSpec;
use tempo_hardfork::TempoHardfork;
use tempo_node::rpc::TempoEthApiBounds;
use tempo_precompiles::{
    storage::{Handler, StorageCtx},
    tip_fee_manager::{IFeeManager, TipFeeManager},
    tip20::{ITIP20, TIP20Token},
    tip20_factory::TIP20Factory,
};
use tempo_primitives::{
    TEMPO_TX_TYPE_ID, TempoTxEnvelope, transaction::FEE_PAYER_SIGNATURE_MARKER,
};
use tempo_transaction_pool::validator::DEFAULT_AA_VALID_AFTER_MAX_SECS;

/// Tempo methods and replacements for standard methods that accept Tempo requests.
#[rpc(server)]
pub trait TempoApi {
    /// Sets the balance of an account in a TIP-20 token. Tempo only.
    #[method(name = "anvil_dealTIP20")]
    async fn anvil_deal_tip20(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> RpcResult<()>;

    /// Sets the token an account pays fees with. Tempo only.
    #[method(name = "anvil_setFeeToken")]
    async fn anvil_set_fee_token(&self, user: Address, token: Address) -> RpcResult<()>;

    /// Sets the token a validator receives fees in. Tempo only.
    #[method(name = "anvil_setValidatorFeeToken")]
    async fn anvil_set_validator_fee_token(
        &self,
        validator: Address,
        token: Address,
    ) -> RpcResult<()>;

    /// Adds Fee AMM liquidity for a token pair. Tempo only.
    #[method(name = "anvil_setFeeAmmLiquidity")]
    async fn anvil_set_fee_amm_liquidity(
        &self,
        user_token: Address,
        validator_token: Address,
        amount: U256,
    ) -> RpcResult<()>;

    /// Sets a TIP-20 or ERC-20 balance.
    #[method(name = "anvil_setERC20Balance", aliases = ["anvil_dealERC20", "hardhat_dealERC20"])]
    async fn anvil_deal_erc20(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> RpcResult<()>;

    /// Signs a request with fees required by Tempo's transaction format.
    #[method(name = "eth_signTransaction")]
    async fn eth_sign_transaction(&self, request: TempoTransactionRequest) -> RpcResult<Bytes>;

    /// Sponsors a sender-signed fee-payer request without broadcasting it.
    #[method(name = "eth_signRawTransaction")]
    async fn eth_sign_raw_transaction(&self, tx: Bytes) -> RpcResult<Bytes>;

    /// Sponsors and submits a raw transaction.
    #[method(name = "eth_sendRawTransaction")]
    async fn eth_send_raw_transaction(&self, tx: Bytes) -> RpcResult<B256>;

    /// Sponsors and submits a raw transaction, then waits for its receipt.
    #[method(name = "eth_sendRawTransactionSync")]
    async fn eth_send_raw_transaction_sync(
        &self,
        tx: Bytes,
        timeout_ms: Option<u64>,
    ) -> RpcResult<TempoTransactionReceipt>;

    /// Sponsors and submits a raw transaction with the accepted condition.
    #[method(name = "eth_sendRawTransactionConditional")]
    async fn eth_send_raw_transaction_conditional(
        &self,
        tx: Bytes,
        condition: TransactionConditional,
    ) -> RpcResult<B256>;

    /// Sends a dev-account transaction and reports fee-token shortfalls.
    #[method(name = "eth_sendTransaction")]
    async fn eth_send_transaction(&self, request: TempoTransactionRequest) -> RpcResult<B256>;

    /// Sends a dev-account transaction and waits for its receipt.
    #[method(name = "eth_sendTransactionSync")]
    async fn eth_send_transaction_sync(
        &self,
        request: TempoTransactionRequest,
    ) -> RpcResult<TempoTransactionReceipt>;

    /// Sends an unsigned transaction with the impersonation controls.
    #[method(name = "eth_sendUnsignedTransaction")]
    async fn eth_send_unsigned_transaction(
        &self,
        request: TempoTransactionRequest,
    ) -> RpcResult<B256>;
}

/// Tempo RPC state, with the node's canonical shared components.
#[derive(Clone)]
pub struct TempoRpc<N: TempoEthApiBounds = NodeOf<Tempo>> {
    rpc: AnvilRpc<N::Pool, N::Provider, AnvilTempoEthApi<N>, TempoChainSpec, Tempo>,
    anvil: AnvilComponents,
    hardfork: TempoHardfork,
    fee_payer: Option<PrivateKeySigner>,
}

impl<N: TempoEthApiBounds> TempoRpc<N>
where
    N::Provider: StateDump,
    N::Pool: TransactionPoolExt,
    N::Evm: ConfigureEvm<NextBlockEnvCtx: AnvilNextBlockEnv<Tempo>>,
{
    /// Creates the extension with the configured dev fee payer.
    pub fn new(
        rpc: AnvilRpc<N::Pool, N::Provider, AnvilTempoEthApi<N>, TempoChainSpec, Tempo>,
        anvil: &AnvilComponents,
    ) -> eyre::Result<Self> {
        let hardfork = anvil.config.get_tempo_hardfork()?;
        let fee_payer = anvil.config.tempo_fee_payer_address().and_then(|address| {
            anvil.config.signer_accounts.iter().find(|signer| signer.address() == address).cloned()
        });
        Ok(Self { rpc, anvil: anvil.clone(), hardfork, fee_payer })
    }

    fn prepare_raw_transaction(&self, tx: Bytes) -> RpcResult<Bytes> {
        let tx = self.sponsor_raw_transaction(&tx, false)?;
        if let Ok(TempoTxEnvelope::AA(transaction)) = TempoTxEnvelope::decode_2718(&mut tx.as_ref())
        {
            let max_allowed = self
                .anvil
                .time
                .current_call_timestamp()
                .saturating_add(DEFAULT_AA_VALID_AFTER_MAX_SECS);
            transaction
                .tx()
                .ensure_valid_after(max_allowed)
                .map_err(|error| ErrorObjectOwned::owned(-32003, error.to_string(), None::<()>))?;
        }
        Ok(tx)
    }

    /// Sets the balance of an account in a TIP-20 token, and returns whether the token is a
    /// TIP-20 token. Tempo only.
    fn try_set_tip20_balance(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> RpcResult<bool> {
        self.with_tempo_storage(|| {
            if !TIP20Factory::new().is_tip20(token_address)? {
                return Ok(false);
            }
            TIP20Token::from_address(token_address)?.balances[address].write(balance)?;
            Ok(true)
        })
    }

    /// Sets the token an account pays fees with. Tempo only.
    fn set_fee_token(&self, user: Address, token: Address) -> RpcResult<()> {
        self.with_tempo_storage(|| {
            TipFeeManager::new().set_user_token(user, IFeeManager::setUserTokenCall { token })
        })
    }

    /// Sets the token a validator receives fees in. Tempo only.
    fn set_validator_fee_token(&self, validator: Address, token: Address) -> RpcResult<()> {
        // The zero beneficiary passes the check that the validator is not the beneficiary.
        self.with_tempo_storage(|| {
            TipFeeManager::new().set_validator_token(
                validator,
                IFeeManager::setValidatorTokenCall { token },
                Address::ZERO,
            )
        })
    }

    /// Mints both tokens to a helper account and adds them as Fee AMM liquidity for the pair.
    /// Tempo only.
    fn set_fee_amm_liquidity(
        &self,
        user_token: Address,
        validator_token: Address,
        amount: U256,
    ) -> RpcResult<()> {
        // From T3 on, liquidity cannot go to the zero address.
        let admin = Address::repeat_byte(0x11);
        self.with_tempo_storage(|| {
            for token in [user_token, validator_token] {
                let mut token = TIP20Token::from_address(token)?;
                token.grant_role_internal(admin, TIP20Token::issuer_role())?;
                token.mint(admin, ITIP20::mintCall { to: admin, amount })?;
            }
            TipFeeManager::new().mint(admin, user_token, validator_token, amount, admin)?;
            Ok(())
        })
    }

    /// Runs Tempo precompile logic over the latest state, and applies its storage and code
    /// writes as anvil state writes for the next block.
    fn with_tempo_storage<R>(
        &self,
        f: impl FnOnce() -> tempo_precompiles::error::Result<R>,
    ) -> RpcResult<R> {
        self.run_tempo_storage(f, true)
    }

    /// Runs Tempo precompile logic over the latest state, and applies its writes when `apply`
    /// is set.
    fn run_tempo_storage<R>(
        &self,
        f: impl FnOnce() -> tempo_precompiles::error::Result<R>,
        apply: bool,
    ) -> RpcResult<R> {
        let base = self
            .rpc
            .eth_api()
            .provider()
            .latest()
            .map_err(|error| internal_error(error.to_string()))?;
        let mut storage = TempoStorage::new(
            Some(&*base),
            self.anvil.config.get_chain_id(),
            self.rpc
                .eth_api()
                .provider()
                .best_block_number()
                .map_err(|error| internal_error(error.to_string()))?
                + 1,
            self.anvil.time.current_call_timestamp(),
            self.hardfork,
        );
        let result = StorageCtx::enter(&mut storage, f)
            .map_err(|error| internal_error(error.to_string()))?;
        if !apply {
            return Ok(result);
        }
        self.rpc.update_state(|state| {
            for (address, writes) in storage.into_writes() {
                if let Some(code) = writes.code {
                    state.set_code(address, reth_ethereum::primitives::Bytecode(code));
                }
                for (slot, value) in writes.storage {
                    state.set_storage_at(address, slot.into(), value);
                }
            }
        });
        Ok(result)
    }
    /// Fee-payer signs a raw Tempo transaction that carries the sponsorship placeholder, as
    /// Tempo's fee payer service does: the node's fee payer picks the fee token when the sender
    /// left it open, and signs. With `sign_only`, the transaction must ask for sponsorship;
    /// otherwise a transaction that does not ask for it comes back as is.
    fn sponsor_raw_transaction(&self, raw: &Bytes, sign_only: bool) -> RpcResult<Bytes> {
        // Fee payer service clients send the transaction with a `0x00` placeholder in the
        // fee payer signature field.
        let normalized = normalize_fee_payer_service_encoding(raw);
        let mut data = normalized.as_deref().unwrap_or(raw);
        let transaction = match TempoTxEnvelope::decode_2718(&mut data) {
            Ok(TempoTxEnvelope::AA(transaction)) => transaction,
            Ok(_) if sign_only => {
                return Err(invalid_params(
                    "only Tempo (0x76) transactions can be fee-payer signed",
                ));
            }
            Err(_) if sign_only => {
                return Err(invalid_params("failed to decode signed transaction"));
            }
            _ => return Ok(raw.clone()),
        };
        match transaction.tx().fee_payer_signature {
            Some(FEE_PAYER_SIGNATURE_MARKER) => {}
            _ if !sign_only => return Ok(raw.clone()),
            Some(_) => {
                return Err(invalid_params("transaction is already fee-payer signed"));
            }
            None => {
                return Err(invalid_params(
                    "transaction does not request sponsorship; sign it with the fee payer \
                     signature placeholder",
                ));
            }
        }
        let sender = transaction.recover_signer().map_err(|_| {
            invalid_params("transaction must be signed by the sender before fee-payer signing")
        })?;
        let Some(signer) = &self.fee_payer else {
            return Err(invalid_params("no Tempo fee payer account available"));
        };
        let sponsor = signer.address();
        if sponsor == sender {
            return Err(invalid_params(format!(
                "Tempo fee payer {sponsor} must not equal the transaction sender"
            )));
        }
        let (mut tx, sender_signature, _) = transaction.into_parts();
        // The fee payer signature commits to the fee token, so the token comes first.
        if tx.fee_token.is_none() {
            let token = self
                .run_tempo_storage(
                    || {
                        TipFeeManager::new()
                            .user_tokens(IFeeManager::userTokensCall { user: sponsor })
                    },
                    false,
                )
                .unwrap_or_default();
            tx.fee_token = Some(if token.is_zero() { PATH_USD_ADDRESS } else { token });
        }
        let digest = tx.fee_payer_signature_hash(sender);
        tx.fee_payer_signature = Some(
            signer.sign_hash_sync(&digest).map_err(|error| internal_error(error.to_string()))?,
        );
        Ok(TempoTxEnvelope::AA(tx.into_signed(sender_signature)).encoded_2718().into())
    }
}

#[async_trait]
impl<N: TempoEthApiBounds> TempoApiServer for TempoRpc<N>
where
    N::Provider: StateDump,
    N::Pool: TransactionPoolExt,
    N::Evm: ConfigureEvm<NextBlockEnvCtx: AnvilNextBlockEnv<Tempo>>,
{
    async fn anvil_deal_tip20(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> RpcResult<()> {
        if self.try_set_tip20_balance(address, token_address, balance)? {
            Ok(())
        } else {
            Err(internal_error(format!("address {token_address} is not a deployed TIP-20 token")))
        }
    }

    async fn anvil_set_fee_token(&self, user: Address, token: Address) -> RpcResult<()> {
        self.set_fee_token(user, token)
    }

    async fn anvil_set_validator_fee_token(
        &self,
        validator: Address,
        token: Address,
    ) -> RpcResult<()> {
        self.set_validator_fee_token(validator, token)
    }

    async fn anvil_set_fee_amm_liquidity(
        &self,
        user_token: Address,
        validator_token: Address,
        amount: U256,
    ) -> RpcResult<()> {
        self.set_fee_amm_liquidity(user_token, validator_token, amount)
    }
    async fn anvil_deal_erc20(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> RpcResult<()> {
        if self.try_set_tip20_balance(address, token_address, balance)? {
            return Ok(());
        }
        AnvilApiServer::anvil_deal_erc20(&self.rpc, address, token_address, balance).await
    }

    async fn eth_sign_transaction(&self, mut request: TempoTransactionRequest) -> RpcResult<Bytes> {
        self.rpc.fill_fees(&mut request).await?;
        EthExtApiServer::eth_sign_transaction(&self.rpc, request).await
    }

    async fn eth_sign_raw_transaction(&self, tx: Bytes) -> RpcResult<Bytes> {
        if tx.is_empty() {
            return Err(invalid_params("empty transaction data"));
        }
        self.sponsor_raw_transaction(&tx, true)
    }

    async fn eth_send_raw_transaction(&self, tx: Bytes) -> RpcResult<B256> {
        let tx = self.prepare_raw_transaction(tx)?;
        EthExtApiServer::eth_send_raw_transaction(&self.rpc, tx).await.map_err(pool_error)
    }

    async fn eth_send_raw_transaction_sync(
        &self,
        tx: Bytes,
        timeout_ms: Option<u64>,
    ) -> RpcResult<TempoTransactionReceipt> {
        let tx = self.prepare_raw_transaction(tx)?;
        EthExtApiServer::eth_send_raw_transaction_sync(&self.rpc, tx, timeout_ms)
            .await
            .map_err(pool_error)
    }

    async fn eth_send_raw_transaction_conditional(
        &self,
        tx: Bytes,
        condition: TransactionConditional,
    ) -> RpcResult<B256> {
        let tx = self.prepare_raw_transaction(tx)?;
        EthExtApiServer::eth_send_raw_transaction_conditional(&self.rpc, tx, condition)
            .await
            .map_err(pool_error)
    }

    async fn eth_send_transaction(&self, request: TempoTransactionRequest) -> RpcResult<B256> {
        EthExtApiServer::eth_send_transaction(&self.rpc, request).await.map_err(pool_error)
    }

    async fn eth_send_transaction_sync(
        &self,
        request: TempoTransactionRequest,
    ) -> RpcResult<TempoTransactionReceipt> {
        EthExtApiServer::eth_send_transaction_sync(&self.rpc, request).await.map_err(pool_error)
    }

    async fn eth_send_unsigned_transaction(
        &self,
        request: TempoTransactionRequest,
    ) -> RpcResult<B256> {
        EthExtApiServer::eth_send_unsigned_transaction(&self.rpc, request).await.map_err(pool_error)
    }
}

fn normalize_fee_payer_service_encoding(raw: &[u8]) -> Option<Vec<u8>> {
    let (tx_type, mut encoded_fields) = raw.split_first()?;
    if *tx_type != TEMPO_TX_TYPE_ID {
        return None;
    }
    let PayloadView::List(fields) = RlpHeader::decode_raw(&mut encoded_fields).ok()? else {
        return None;
    };
    if !encoded_fields.is_empty() {
        return None;
    }
    // The fee payer signature is the twelfth field of a Tempo transaction.
    if fields.get(11).is_none_or(|field| *field != [0x00]) {
        return None;
    }

    // The standard encoding of the placeholder signature, as Tempo encodes it.
    let marker = FEE_PAYER_SIGNATURE_MARKER;
    let mut marker_field = Vec::new();
    RlpHeader { list: true, payload_length: marker.rlp_rs_len() + marker.v().length() }
        .encode(&mut marker_field);
    marker.write_rlp_vrs(&mut marker_field, marker.v());

    let mut payload = Vec::new();
    for (index, field) in fields.into_iter().enumerate() {
        if index == 11 {
            payload.extend_from_slice(&marker_field);
        } else {
            payload.extend_from_slice(field);
        }
    }
    let mut normalized = vec![TEMPO_TX_TYPE_ID];
    RlpHeader { list: true, payload_length: payload.len() }.encode(&mut normalized);
    normalized.extend_from_slice(&payload);
    Some(normalized)
}

fn internal_error(message: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, message.into(), None::<()>)
}

fn invalid_params(message: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INVALID_PARAMS_CODE, message.into(), None::<()>)
}

/// Configures the Tempo pool cache and registers Tempo's RPC handlers.
pub fn extend_rpc(
    rpc: AnvilRpcOf<Tempo>,
    anvil: &AnvilComponents,
) -> eyre::Result<(AnvilRpcOf<Tempo>, RpcModule<()>)> {
    let pool = rpc.eth_api().pool().clone();
    let provider = rpc.eth_api().provider().clone();
    let refresh = PoolRefresh::new(move || {
        let Ok(Some(block)) = provider
            .best_block_number()
            .and_then(|number| provider.recovered_block(number.into(), TransactionVariant::NoHash))
        else {
            return;
        };
        let info = pool.block_info();
        pool.on_canonical_state_change(CanonicalStateUpdate {
            new_tip: block.sealed_block(),
            pending_block_base_fee: info.pending_basefee,
            pending_block_blob_fee: info.pending_blob_fee,
            changed_accounts: Vec::new(),
            mined_transactions: Vec::new(),
            update_kind: PoolUpdateKind::Commit,
        });
    });
    let rpc = rpc.with_pool_refresh(Some(refresh));
    let mut module = TempoApiServer::into_rpc(TempoRpc::<NodeOf<Tempo>>::new(rpc.clone(), anvil)?)
        .remove_context();
    let precompiles =
        anvil.config.networks.precompiles(Some(anvil.config.get_tempo_hardfork()?.into()));
    let mut config_module = RpcModule::new(EthConfigHandler::new(
        rpc.eth_api().provider().clone(),
        rpc.eth_api().evm_config().clone(),
    ));
    config_module.register_method("eth_config", move |_, handler, _| {
        let mut config = EthConfigApiServer::config(handler)?;
        config.current.precompiles.extend(precompiles.clone());
        RpcResult::Ok(config)
    })?;
    module.merge(config_module)?;
    Ok((rpc, module))
}

/// Keeps anvil's fee-token shortfall message for Tempo's native-funds pool error.
fn pool_error(error: ErrorObjectOwned) -> ErrorObjectOwned {
    const INSUFFICIENT_FUNDS: &str = "insufficient funds for gas * price + value: have ";
    if let Some(amounts) = error.message().strip_prefix(INSUFFICIENT_FUNDS)
        && let Some((balance, required)) = amounts.split_once(" want ")
    {
        return ErrorObjectOwned::owned(
            error.code(),
            format!("insufficient fee token balance: have {balance}, need {required}"),
            None::<()>,
        );
    }
    error
}
