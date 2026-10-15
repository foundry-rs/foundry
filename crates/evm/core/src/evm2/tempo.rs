//! Tempo on evm2, using Tempo's own evm2 integration (tempoxyz/tempo#7871).

use super::{
    EvmEnv, FoundryBlockExt, FoundryEvmExt, FoundryEvmFactory, FoundryEvmNetwork,
    FoundryTxResultExt, TransactionRequestFor, TxEnvelopeFor, TxPayload, placeholder_signature,
};
use crate::{
    constants::{CALLER, SYSTEM_PRECOMPILE_STUB, TEST_CONTRACT_ADDRESS},
    evm::TempoEvmNetwork,
    tempo::{ALPHA_USD_ADDRESS, BETA_USD_ADDRESS, PATH_USD_ADDRESS, THETA_USD_ADDRESS},
};
use alloy_consensus::{SignableTransaction, transaction::Recovered};
use alloy_eips::{Decodable2718, Encodable2718};
use alloy_network::{NetworkTransactionBuilder, TransactionBuilder};
use alloy_primitives::{Address, Bytes, U256};
use evm2::{Evm, TxResult, Version, evm::precompile::PrecompileProvider, registry::TxRegistry};
use foundry_evm_networks::TEMPO_PRECOMPILE_ADDRESSES;
use tempo_alloy::TempoNetwork;
use tempo_contracts::{
    ARACHNID_CREATE2_FACTORY_ADDRESS, CREATEX_ADDRESS, CreateX, MULTICALL3_ADDRESS, Multicall3,
    PERMIT2_ADDRESS, Permit2, SAFE_DEPLOYER_ADDRESS, SafeDeployer,
    contracts::ARACHNID_CREATE2_FACTORY_BYTECODE, precompiles::VALIDATOR_CONFIG_ADDRESS,
};
use tempo_evm2::{
    TempoBlockExt, TempoEvmExt, TempoEvmFactory, TempoEvmTypes, TempoTxEnv, TempoTxResultExt,
    tempo_execution_config, tempo_tx_registry,
};
use tempo_hardfork_evm2::TempoHardfork;
use tempo_precompiles_evm2::{
    TempoPrecompiles,
    error::TempoPrecompileError,
    storage::StorageCtx,
    tip20::{ITIP20, TIP20Token},
    tip20_factory::TIP20Factory,
    validator_config,
};
use tempo_primitives_evm2::TempoTxEnvelope;

impl FoundryBlockExt for TempoBlockExt {
    fn timestamp_millis_part(&self) -> u64 {
        self.timestamp_millis_part
    }

    fn set_timestamp_millis_part(&mut self, millis: u64) {
        self.timestamp_millis_part = millis;
    }
}

/// Tempo envelope of Foundry's RPC types.
type TempoNetworkTxEnvelope = TxEnvelopeFor<TempoEvmNetwork>;

impl FoundryEvmExt<TempoTxEnv> for TempoEvmExt {}

impl FoundryTxResultExt for TempoTxResultExt {}

impl FoundryEvmNetwork for TempoEvmNetwork {
    type Network = TempoNetwork;
    type EvmFactory = TempoEvmFactory;

    fn evm_tx(
        envelope: TempoNetworkTxEnvelope,
        sender: Address,
        payload: TxPayload,
    ) -> eyre::Result<Recovered<TempoTxEnv>> {
        tempo_tx(envelope, sender, payload, None)
    }

    fn tx_from_request(
        request: TransactionRequestFor<Self>,
    ) -> eyre::Result<Recovered<TempoTxEnv>> {
        let sender =
            request.from().ok_or_else(|| eyre::eyre!("transaction request has no sender"))?;
        // The access key only prices simulations; the signed envelope carries its own key.
        let key_id = request.key_id;
        let unsigned =
            request.build_unsigned().map_err(|err| eyre::eyre!(err.error.to_string()))?;
        let envelope = TempoNetworkTxEnvelope::from(unsigned.into_signed(placeholder_signature()));
        tempo_tx(envelope, sender, TxPayload::Simulated, key_id)
    }
}

/// Converts a Tempo network envelope into Tempo's evm2 transaction.
///
/// Foundry's RPC types and Tempo's evm2 integration are pinned to different Tempo revisions, so the
/// envelope crosses over as its exact EIP-2718 encoding.
fn tempo_tx(
    envelope: TempoNetworkTxEnvelope,
    sender: Address,
    payload: TxPayload,
    key_id: Option<Address>,
) -> eyre::Result<Recovered<TempoTxEnv>> {
    let encoded = envelope.encoded_2718();
    let envelope = TempoTxEnvelope::decode_2718_exact(&encoded)?;
    let mut tx = TempoTxEnv::from(Recovered::new_unchecked(envelope, sender));
    if payload.is_simulated() {
        let id = tx.unique_tx_identifier();
        tx = tx.with_simulation_overrides(id, None, key_id);
    }
    Ok(Recovered::new_unchecked(tx, sender))
}

impl FoundryEvmFactory for TempoEvmFactory {
    type Types = TempoEvmTypes;

    fn version(&self, spec: TempoHardfork, chain_id: u64) -> Version {
        *tempo_execution_config(spec, chain_id).version()
    }

    fn tx_registry(
        &self,
        spec: TempoHardfork,
    ) -> TxRegistry<TempoEvmTypes, TxResult<TempoEvmTypes>> {
        tempo_tx_registry(spec.into())
    }

    fn precompiles(
        &self,
        env: &EvmEnv<TempoEvmTypes>,
        ext: &TempoEvmExt,
    ) -> Box<dyn PrecompileProvider<TempoEvmTypes>> {
        Box::new(TempoPrecompiles::new(
            env.spec,
            ext.actions.clone(),
            ext.non_creditable_slots.clone(),
        ))
    }

    fn initialize_evm(&self, evm: &mut Evm<'_, TempoEvmTypes>, is_forked: bool) {
        let result = StorageCtx::enter_evm(evm, || {
            let mut ctx = StorageCtx;
            if is_forked {
                // Precompiles have no on-chain bytecode; seed sentinels so forks do not refetch
                // them. Failed loads are retried, and surface, on later access.
                for address in TEMPO_PRECOMPILE_ADDRESSES.iter().chain([&PATH_USD_ADDRESS]) {
                    let _ = ctx.set_code(*address, Bytes::from_static(SYSTEM_PRECOMPILE_STUB));
                }
                Ok(())
            } else {
                initialize_test_genesis(&mut ctx, TEST_CONTRACT_ADDRESS, CALLER)
            }
        });
        result.expect("tempo genesis initialization failed");
        evm.state_mut().commit_transaction();
    }
}

/// Seeds Forge's local Tempo genesis: precompile sentinels, fee tokens and standard contracts.
///
/// evm2 twin of [`crate::tempo::initialize_tempo_test_genesis_inner`]; the two merge once the
/// REVM-based Tempo pin is dropped.
fn initialize_test_genesis(
    ctx: &mut StorageCtx,
    admin: Address,
    recipient: Address,
) -> Result<(), TempoPrecompileError> {
    if TIP20Factory::new().is_tip20(PATH_USD_ADDRESS)? {
        return Ok(());
    }
    for precompile in TEMPO_PRECOMPILE_ADDRESSES {
        ctx.set_code(*precompile, Bytes::from_static(SYSTEM_PRECOMPILE_STUB))?;
    }
    let path_usd =
        create_and_mint_token(PATH_USD_ADDRESS, "PathUSD", Address::ZERO, admin, recipient)?;
    for (address, name) in [
        (ALPHA_USD_ADDRESS, "AlphaUSD"),
        (BETA_USD_ADDRESS, "BetaUSD"),
        (THETA_USD_ADDRESS, "ThetaUSD"),
    ] {
        create_and_mint_token(address, name, path_usd, admin, recipient)?;
    }
    ctx.sstore(VALIDATOR_CONFIG_ADDRESS, validator_config::slots::OWNER, admin.into_word().into())?;
    for (address, code) in [
        (MULTICALL3_ADDRESS, Multicall3::DEPLOYED_BYTECODE.clone()),
        (CREATEX_ADDRESS, CreateX::DEPLOYED_BYTECODE.clone()),
        (SAFE_DEPLOYER_ADDRESS, SafeDeployer::DEPLOYED_BYTECODE.clone()),
        (PERMIT2_ADDRESS, Permit2::DEPLOYED_BYTECODE.clone()),
        (ARACHNID_CREATE2_FACTORY_ADDRESS, ARACHNID_CREATE2_FACTORY_BYTECODE),
    ] {
        ctx.set_code(address, code)?;
    }
    Ok(())
}

/// Creates a USD TIP20 token at a reserved address and mints `u64::MAX` to `recipient` and `admin`.
fn create_and_mint_token(
    address: Address,
    name: &str,
    quote_token: Address,
    admin: Address,
    recipient: Address,
) -> Result<Address, TempoPrecompileError> {
    let token_address = TIP20Factory::new().create_token_reserved_address(
        address,
        name,
        name,
        "USD",
        quote_token,
        admin,
    )?;
    let mut token = TIP20Token::from_address(token_address)?;
    token.grant_role_internal(admin, TIP20Token::issuer_role())?;
    let amount = U256::from(u64::MAX);
    token.mint(admin, ITIP20::mintCall { to: recipient, amount })?;
    if admin != recipient {
        token.mint(admin, ITIP20::mintCall { to: admin, amount })?;
    }
    Ok(token_address)
}
