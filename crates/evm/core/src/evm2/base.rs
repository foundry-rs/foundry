//! Base on evm2, using `base-evm2`.

use super::{
    DepositTxExt, EvmEnv, FoundryBlockExt, FoundryDatabase, FoundryEvmExt, FoundryEvmFactory,
    FoundryEvmNetwork, FoundryTxEnv, FoundryTxResultExt, build_eth_tx,
};
use crate::{constants::SYSTEM_PRECOMPILE_STUB, evm::BaseEvmNetwork};
use alloy_consensus::transaction::Recovered;
use alloy_primitives::{Bytes, KECCAK256_EMPTY};
use base_common_network::Base;
use base_evm2::{
    BaseBlockEnvExt, BaseEvmExt, BaseEvmFactory, BaseEvmTypes, BaseSpecId, BaseTxEnvelope,
    BaseTxResultExt, TxDeposit, base_tx_registry, base_version,
    precompiles::activation_admin_address_for_upgrade_by_chain_id,
};
use evm2::{
    Evm, TxResult, Version, bytecode::Bytecode, evm::precompile::PrecompileProvider,
    registry::TxRegistry,
};
use foundry_evm_networks::{BASE_CODE_SENTINEL_ADDRESSES, is_base_precompile_active_at};

impl FoundryBlockExt for BaseBlockEnvExt {}

impl FoundryEvmExt<DepositTxExt> for BaseEvmExt {
    fn clear_transaction_fee_cache(&mut self) {
        self.clear_tx_l1_cost();
    }
}

impl FoundryTxResultExt for BaseTxResultExt {
    fn is_failed_deposit(&self) -> bool {
        self.failed_deposit
    }
}

impl FoundryEvmNetwork for BaseEvmNetwork {
    type Network = Base;
    type EvmFactory = BaseEvmFactory;
}

impl FoundryEvmFactory for BaseEvmFactory {
    type Types = BaseEvmTypes;
    type TxExt = DepositTxExt;

    fn version(&self, spec: BaseSpecId, chain_id: u64) -> Version {
        base_version(spec, chain_id)
    }

    fn tx_registry(&self, spec: BaseSpecId) -> TxRegistry<BaseEvmTypes, TxResult<BaseEvmTypes>> {
        base_tx_registry(spec)
    }

    fn precompiles(
        &self,
        env: &EvmEnv<BaseEvmTypes>,
        _ext: &BaseEvmExt,
    ) -> Box<dyn PrecompileProvider<BaseEvmTypes>> {
        Box::new(Self::precompiles(self, env.spec))
    }

    fn configure_for<D: FoundryDatabase + ?Sized>(
        &self,
        db: &D,
        env: &EvmEnv<BaseEvmTypes>,
    ) -> Self {
        // Nested EVMs share the fork database, so execution chain-ID overrides must not change the
        // source chain's activation registry admin.
        let chain_id = db.active_fork_source_chain_id().unwrap_or(env.chain_id());
        let admin = self.activation_admin_address().or_else(|| {
            activation_admin_address_for_upgrade_by_chain_id(chain_id, env.spec.upgrade())
        });
        self.clone().with_activation_admin_address(admin)
    }

    fn build_tx(&self, tx: &FoundryTxEnv<DepositTxExt>) -> eyre::Result<Recovered<BaseTxEnvelope>> {
        let envelope = if tx.is_deposit() {
            BaseTxEnvelope::deposit(TxDeposit {
                source_hash: tx.ext.source_hash,
                from: tx.caller,
                to: tx.kind,
                mint: tx.ext.mint.unwrap_or_default(),
                value: tx.value,
                gas_limit: tx.gas_limit,
                is_system_transaction: tx.ext.is_system_transaction,
                input: tx.data.clone(),
            })
        } else {
            BaseTxEnvelope::standard(
                build_eth_tx(tx)?,
                tx.ext.enveloped_tx.clone().unwrap_or_default(),
            )
        };
        Ok(Recovered::new_unchecked(envelope, tx.caller))
    }

    fn initialize_evm(&self, evm: &mut Evm<'_, BaseEvmTypes>, _is_forked: bool) {
        // Base plants a one-byte sentinel on code-less stateful precompiles, so Solidity's
        // `extcodesize` check on high-level calls passes. Existing code, including fork code, wins.
        let upgrade = evm.config_spec_id().upgrade();
        let sentinel = Bytecode::new_legacy(Bytes::from_static(SYSTEM_PRECOMPILE_STUB));
        for address in BASE_CODE_SENTINEL_ADDRESSES
            .iter()
            .filter(|address| is_base_precompile_active_at(**address, upgrade))
        {
            if let Ok(mut account) = evm.state_mut().account(address)
                && account.code_hash() == KECCAK256_EMPTY
            {
                account.set_code_slow(sentinel.clone());
            }
        }
        evm.state_mut().commit_transaction();
    }
}
