//! Tempo on evm2, using Tempo's own evm2 integration (tempoxyz/tempo#7871).

use super::{
    EvmEnv, FoundryBlockExt, FoundryEvmExt, FoundryEvmFactory, FoundryEvmNetwork, FoundryTxEnv,
    FoundryTxExt, FoundryTxResultExt, build_eth_tx,
};
use crate::{
    constants::{CALLER, SYSTEM_PRECOMPILE_STUB, TEST_CONTRACT_ADDRESS},
    evm::TempoEvmNetwork,
    tempo::{ALPHA_USD_ADDRESS, BETA_USD_ADDRESS, PATH_USD_ADDRESS, THETA_USD_ADDRESS},
};
use alloy_consensus::{Signed, TxEip1559, TxEip2930, TxEip7702, TxLegacy, transaction::Recovered};
use alloy_primitives::{Address, Bytes, Signature, TxKind, U256};
use evm2::{
    Evm, TxResult, Version, ethereum::TxEnvelope, evm::precompile::PrecompileProvider,
    registry::TxRegistry,
};
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
use tempo_primitives_evm2::{
    AASigned, TempoTransaction, TempoTxEnvelope,
    transaction::{Call, PrimitiveSignature, TempoSignature},
};

/// Tempo-only fields of a [`FoundryTxEnv`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TempoTxExt {
    /// Fee token for Tempo AA transactions.
    pub fee_token: Option<Address>,
    /// Sponsored fee payer: `Some(Some(payer))` sponsors, `Some(None)` pays from the sender.
    pub fee_payer: Option<Option<Address>>,
    /// Tempo AA transaction template. Its first call, gas, nonce and fees follow the flat fields.
    pub aa: Option<Box<TempoTransaction>>,
}

impl FoundryTxExt for TempoTxExt {
    fn fee_token(&self) -> Option<Address> {
        self.fee_token
    }

    fn set_fee_token(&mut self, token: Option<Address>) {
        self.fee_token = token;
    }

    fn fee_payer(&self) -> Option<Option<Address>> {
        self.fee_payer
    }

    fn set_fee_payer(&mut self, payer: Option<Option<Address>>) {
        self.fee_payer = payer;
    }

    fn call_count(&self) -> usize {
        self.aa.as_ref().map_or(1, |aa| aa.calls.len())
    }

    fn on_call_change(&mut self, kind: TxKind, value: U256, data: &Bytes) {
        if let Some(call) = self.aa.as_mut().and_then(|aa| aa.calls.first_mut()) {
            *call = Call { to: kind, value, input: data.clone() };
        }
    }
}

impl FoundryBlockExt for TempoBlockExt {
    fn timestamp_millis_part(&self) -> u64 {
        self.timestamp_millis_part
    }

    fn set_timestamp_millis_part(&mut self, millis: u64) {
        self.timestamp_millis_part = millis;
    }
}

impl FoundryEvmExt<TempoTxExt> for TempoEvmExt {}

impl FoundryTxResultExt for TempoTxResultExt {}

impl FoundryEvmNetwork for TempoEvmNetwork {
    type Network = TempoNetwork;
    type EvmFactory = TempoEvmFactory;
}

impl FoundryEvmFactory for TempoEvmFactory {
    type Types = TempoEvmTypes;
    type TxExt = TempoTxExt;

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

    fn build_tx(&self, tx: &FoundryTxEnv<TempoTxExt>) -> eyre::Result<Recovered<TempoTxEnv>> {
        let envelope = match &tx.ext.aa {
            Some(aa) => TempoTxEnvelope::AA(AASigned::new_unhashed(
                TempoTransaction {
                    chain_id: tx.chain_id.unwrap_or(aa.chain_id),
                    fee_token: tx.ext.fee_token.or(aa.fee_token),
                    max_priority_fee_per_gas: tx.gas_priority_fee.unwrap_or_default(),
                    max_fee_per_gas: tx.gas_price,
                    gas_limit: tx.gas_limit,
                    access_list: tx.access_list.clone(),
                    nonce: tx.nonce,
                    ..(**aa).clone()
                },
                TempoSignature::Primitive(PrimitiveSignature::Secp256k1(
                    Signature::test_signature(),
                )),
            )),
            None => eth_to_tempo_envelope(build_eth_tx(tx)?)?,
        };
        let mut env = TempoTxEnv::from(Recovered::new_unchecked(envelope, tx.caller));
        if let Some(Some(payer)) = tx.ext.fee_payer {
            let id = env.unique_tx_identifier();
            env = env.with_simulation_overrides(id, Some(payer), None);
        }
        Ok(Recovered::new_unchecked(env, tx.caller))
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

/// Wraps an Ethereum transaction in Tempo's envelope with a placeholder signature.
fn eth_to_tempo_envelope(tx: TxEnvelope) -> eyre::Result<TempoTxEnvelope> {
    fn signed<T>(tx: T) -> Signed<T> {
        Signed::new_unhashed(tx, Signature::test_signature())
    }
    Ok(match tx {
        TxEnvelope::Legacy(tx) => TempoTxEnvelope::Legacy(signed::<TxLegacy>(tx)),
        TxEnvelope::Eip2930(tx) => TempoTxEnvelope::Eip2930(signed::<TxEip2930>(tx)),
        TxEnvelope::Eip1559(tx) => TempoTxEnvelope::Eip1559(signed::<TxEip1559>(tx)),
        TxEnvelope::Eip7702(tx) => {
            let authorization_list = tx
                .authorization_list
                .iter()
                .map(|auth| match auth {
                    evm2::ethereum::LazyAuthorization::Signed(auth) => auth.clone(),
                    auth => auth.inner().clone().into_signed(Signature::test_signature()),
                })
                .collect();
            TempoTxEnvelope::Eip7702(signed::<TxEip7702>(TxEip7702 {
                chain_id: tx.chain_id,
                nonce: tx.nonce,
                gas_limit: tx.gas_limit,
                max_fee_per_gas: tx.max_fee_per_gas,
                max_priority_fee_per_gas: tx.max_priority_fee_per_gas,
                to: tx.to,
                value: tx.value,
                access_list: tx.access_list,
                authorization_list,
                input: tx.input,
            }))
        }
        TxEnvelope::Eip4844(_) => eyre::bail!("Tempo does not support blob transactions"),
    })
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
